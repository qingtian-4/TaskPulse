//! WorkBuddy native database + incremental local transcripts. Never reads keys,
//! user settings, prompts, tool arguments or tool output into the task model.
use super::{
    jsonl_tail::JsonlTail,
    native_sessions::{self, Sessions},
    Provider, ProviderOutput,
};
use crate::domain::{ExecutionPhase, TaskSnapshot, TaskStatus, WaitingReason};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};
pub fn default_root() -> PathBuf {
    dirs_next::home_dir().unwrap_or_default().join(".workbuddy")
}
#[derive(Default)]
struct Transcript {
    invalid: bool,
    tail: JsonlTail,
    turn: Option<String>,
    request: Option<String>,
    started: u64,
    activity: u64,
    assistant_completed: bool,
    calls: HashSet<String>,
    model: Option<String>,
    phase: Option<ExecutionPhase>,
}
impl Transcript {
    fn event(&mut self, d: &Value, session: &str) {
        if d.get("sessionId")
            .and_then(Value::as_str)
            .is_some_and(|id| id != session)
        {
            return;
        }
        let Some(stamp) = d
            .get("timestamp")
            .and_then(Value::as_u64)
            .filter(|n| native_sessions::time(*n).is_some())
        else {
            return;
        };
        let kind = d.get("type").and_then(Value::as_str).unwrap_or_default();
        if kind == "message" && d.get("role").and_then(Value::as_str) == Some("user") {
            if d.pointer("/providerData/startsNewUserRequest") == Some(&Value::Bool(false)) {
                return;
            }
            let Some(id) = d
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                return;
            };
            if stamp < self.started {
                return;
            }
            self.turn = Some(id.into());
            self.request = d
                .pointer("/providerData/conversationRequestId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            self.started = stamp;
            self.activity = stamp;
            self.assistant_completed = false;
            self.model = None;
            self.calls.clear();
            self.phase = Some(ExecutionPhase::AwaitingModel);
            return;
        }
        if self.turn.is_none() || stamp < self.started {
            return;
        }
        if !matches!(
            kind,
            "message" | "reasoning" | "function_call" | "function_call_result"
        ) {
            return;
        }
        if self
            .request
            .as_deref()
            .zip(
                d.pointer("/providerData/conversationRequestId")
                    .and_then(Value::as_str),
            )
            .is_some_and(|(current, incoming)| current != incoming)
        {
            return;
        }
        self.activity = self.activity.max(stamp);
        if let Some(model) = d
            .pointer("/providerData/requestModelId")
            .or_else(|| d.pointer("/providerData/model"))
            .and_then(Value::as_str)
            .and_then(super::reported_model)
        {
            self.model = Some(model);
        }
        match kind {
            "function_call" => {
                if let Some(call) = d.get("callId").and_then(Value::as_str) {
                    if self.calls.len() >= 512 && !self.calls.contains(call) {
                        self.invalid = true;
                        return;
                    }
                    self.calls.insert(call.into());
                }
                self.phase = Some(ExecutionPhase::Tool);
                self.assistant_completed = false;
            }
            "function_call_result" => {
                if let Some(call) = d.get("callId").and_then(Value::as_str) {
                    self.calls.remove(call);
                }
                if self.calls.is_empty() {
                    self.phase = Some(ExecutionPhase::AwaitingModel);
                }
            }
            "reasoning" => self.phase = Some(ExecutionPhase::Thinking),
            "message" if d.get("role").and_then(Value::as_str) == Some("assistant") => {
                self.assistant_completed =
                    d.get("status").and_then(Value::as_str) == Some("completed");
                self.phase = Some(ExecutionPhase::Responding);
            }
            _ => {}
        }
    }
    fn read(&mut self, path: &std::path::Path, session: &str) -> bool {
        // One bounded increment per session/poll. No old terminal may escape
        // while catching up, after a partial line, or across a replaced file.
        let batch = match self.tail.read(path) {
            Ok(b) => b,
            Err(_) => return false,
        };
        if batch.reset {
            self.invalid = false;
            self.turn = None;
            self.request = None;
            self.started = 0;
            self.activity = 0;
            self.assistant_completed = false;
            self.calls.clear();
            self.model = None;
            self.phase = None;
        }
        for line in batch.body.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                self.event(&value, session);
            } else {
                self.invalid = true;
                return false;
            }
        }
        !self.invalid && batch.caught_up && !batch.gap && self.tail.trusted()
    }
}
pub struct WorkBuddyProvider {
    root: PathBuf,
    sessions: Sessions,
    logs: HashMap<String, Transcript>,
    paths: HashMap<String, PathBuf>,
}
impl WorkBuddyProvider {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            sessions: Sessions::default(),
            logs: HashMap::new(),
            paths: HashMap::new(),
        }
    }
    pub fn resume(&mut self, tasks: impl IntoIterator<Item = TaskSnapshot>) {
        self.sessions.resume("workbuddy", tasks);
    }
    fn index(&mut self) -> rusqlite::Result<()> {
        let mut paths = HashMap::new();
        let mut visited = 0;
        for project in std::fs::read_dir(self.root.join("projects"))
            .map_err(|_| rusqlite::Error::InvalidQuery)?
        {
            let project = project.map_err(|_| rusqlite::Error::InvalidQuery)?;
            if !project
                .file_type()
                .is_ok_and(|t| t.is_dir() && !t.is_symlink())
            {
                continue;
            }
            for file in
                std::fs::read_dir(project.path()).map_err(|_| rusqlite::Error::InvalidQuery)?
            {
                visited += 1;
                if visited > 8000 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let file = file.map_err(|_| rusqlite::Error::InvalidQuery)?;
                if !file
                    .file_type()
                    .is_ok_and(|t| t.is_file() && !t.is_symlink())
                {
                    continue;
                }
                let path = file.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    let id = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    if paths.insert(id, path).is_some() {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                }
            }
        }
        self.paths = paths;
        Ok(())
    }
    fn read(&mut self) -> rusqlite::Result<Vec<TaskSnapshot>> {
        self.index()?;
        let db = Connection::open_with_flags(
            self.root.join("workbuddy.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(250))?;
        let mut rows=db.prepare("SELECT id,coalesce(nullif(custom_title,''),title,id),cwd,status,coalesce(last_activity_at,created_at),model FROM sessions WHERE deleted_at IS NULL AND coalesce(transport,'local')='local' ORDER BY last_activity_at DESC LIMIT 401")?;
        let rows = rows
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > 400 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut out = Vec::new();
        let began = Instant::now();
        let mut incomplete = false;
        let mut seen = HashSet::new();
        for (id, title, project, status, activity, model) in rows {
            seen.insert(id.clone());
            let Some(path) = self.paths.get(&id) else {
                continue;
            };
            if began.elapsed() > Duration::from_millis(500) {
                incomplete = true;
                break;
            }
            let log = self.logs.entry(id.clone()).or_default();
            if !log.read(path, &id) {
                incomplete = true;
                continue;
            }
            let Some(turn) = &log.turn else { continue };
            let Some(mut task) = native_sessions::task(
                ("workbuddy", "WorkBuddy"),
                &id,
                &title,
                &project,
                turn,
                log.started,
                log.activity,
            ) else {
                continue;
            };
            task.model = log
                .model
                .clone()
                .or_else(|| model.as_deref().and_then(super::reported_model));
            task.phase = log.phase;
            // DB terminal belongs to this turn only with matching transcript
            // evidence. updated_at includes renames and is deliberately unused.
            let current = activity >= log.activity && activity >= log.started;
            if current {
                match status.as_str() {
                    "completed" if log.assistant_completed && log.calls.is_empty() => {
                        task.status = TaskStatus::Succeeded
                    }
                    "completed" => task.stale = true,
                    "failed" | "error" => task.status = TaskStatus::Failed,
                    "terminated" => {
                        task.status = TaskStatus::WaitingInput;
                        task.waiting_reason = Some(WaitingReason::TurnEnded);
                        task.outcome = Some("interrupted".into());
                    }
                    "pending" | "Pending" | "idle" => {
                        task.status = TaskStatus::Unknown;
                        task.stale = true;
                    }
                    "working" | "planning" | "running" => {}
                    _ => {
                        task.status = TaskStatus::Unknown;
                        task.stale = true;
                    }
                }
            }
            if task.status != TaskStatus::Running && !task.stale {
                task.updated_at = native_sessions::time(activity).unwrap_or(task.updated_at);
                task.finished_at = Some(task.updated_at);
                task.phase = None;
            }
            out.push(task);
        }
        self.logs.retain(|id, _| seen.contains(id));
        if incomplete {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(out)
    }
}
impl Provider for WorkBuddyProvider {
    fn name(&self) -> &'static str {
        "workbuddy"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match self.read() {
            Ok(tasks) => self.sessions.observe(tasks, now),
            Err(_) => self
                .sessions
                .degraded("WorkBuddy 原生记录读取受限、追赶中或格式变化；保留上次任务状态"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    fn fixture() -> (tempfile::TempDir, Connection, WorkBuddyProvider, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("projects/p")).unwrap();
        let db = Connection::open(root.join("workbuddy.db")).unwrap();
        db.execute_batch("CREATE TABLE sessions(id TEXT,title TEXT,custom_title TEXT,cwd TEXT,status TEXT,last_activity_at INTEGER,created_at INTEGER,updated_at INTEGER,model TEXT,deleted_at INTEGER,transport TEXT);INSERT INTO sessions VALUES('s','Actual native task',NULL,'C:/project','working',100000,100000,100000,'source-model',NULL,'local');").unwrap();
        let log = root.join("projects/p/s.jsonl");
        std::fs::write(&log, "").unwrap();
        let p = WorkBuddyProvider::new(root.to_path_buf());
        (dir, db, p, log)
    }
    fn append(path: &std::path::Path, kind: &str, role: Option<&str>, stamp: u64, extra: Value) {
        let mut d =
            json!({"id":format!("e-{stamp}"),"sessionId":"s","type":kind,"timestamp":stamp});
        if let Some(role) = role {
            d["role"] = json!(role);
        }
        for (k, v) in extra.as_object().unwrap() {
            d[k] = v.clone();
        }
        writeln!(
            std::fs::OpenOptions::new().append(true).open(path).unwrap(),
            "{d}"
        )
        .unwrap();
    }
    #[test]
    fn tool_results_and_stale_terminal_are_not_a_turn_end_and_rename_preserves_identity() {
        let (_dir, db, mut p, log) = fixture();
        append(&log, "message", Some("user"), 100000, json!({}));
        append(&log, "function_call", None, 101000, json!({"callId":"a"}));
        append(&log, "function_call", None, 102000, json!({"callId":"b"}));
        append(
            &log,
            "function_call_result",
            None,
            103000,
            json!({"callId":"a","status":"completed"}),
        );
        let running = p.poll(native_sessions::time(104000).unwrap());
        assert_eq!(running.tasks[0].status, TaskStatus::Running);
        assert_eq!(running.tasks[0].model.as_deref(), Some("source-model"));
        assert_eq!(p.logs["s"].calls.len(), 1);
        db.execute(
            "UPDATE sessions SET status='completed',last_activity_at=103000",
            [],
        )
        .unwrap();
        assert_eq!(
            p.poll(native_sessions::time(105000).unwrap()).tasks[0].status,
            TaskStatus::Running
        );
        append(
            &log,
            "function_call_result",
            None,
            106000,
            json!({"callId":"b","status":"completed"}),
        );
        append(
            &log,
            "message",
            Some("assistant"),
            107000,
            json!({"status":"completed","providerData":{"requestModelId":"reported-model"}}),
        );
        db.execute(
            "UPDATE sessions SET status='completed',last_activity_at=107000",
            [],
        )
        .unwrap();
        let done = p.poll(native_sessions::time(108000).unwrap());
        assert_eq!(done.tasks[0].status, TaskStatus::Succeeded);
        assert_eq!(done.tasks[0].model.as_deref(), Some("reported-model"));
        db.execute(
            "UPDATE sessions SET custom_title='Renamed',updated_at=999000",
            [],
        )
        .unwrap();
        let rename = p.poll(native_sessions::time(999000).unwrap());
        assert_eq!(rename.tasks[0].id, done.tasks[0].id);
        assert_eq!(rename.tasks[0].turn_key, done.tasks[0].turn_key);
        assert_eq!(rename.tasks[0].updated_at, done.tasks[0].updated_at);
        assert!(!rename.tasks[0].new_execution);
        append(&log, "message", Some("user"), 1000000, json!({}));
        let new = p.poll(native_sessions::time(1000001).unwrap());
        assert_eq!(new.tasks[0].status, TaskStatus::Running);
        assert!(new.tasks[0].new_execution);
        assert_ne!(new.tasks[0].turn_key, done.tasks[0].turn_key);
    }
    #[test]
    fn initial_finished_history_is_not_imported_and_unknown_pending_is_not_an_approval() {
        let (_dir, db, mut p, log) = fixture();
        append(&log, "message", Some("user"), 100000, json!({}));
        append(
            &log,
            "message",
            Some("assistant"),
            101000,
            json!({"status":"completed"}),
        );
        db.execute(
            "UPDATE sessions SET status='completed',last_activity_at=101000",
            [],
        )
        .unwrap();
        let old = p.poll(native_sessions::time(200000).unwrap());
        assert!(old.tasks.is_empty());
        assert_eq!(old.catalog.len(), 1);
        p.resume(old.catalog);
        db.execute("UPDATE sessions SET status='pending'", [])
            .unwrap();
        let pending = p.poll(native_sessions::time(201000).unwrap());
        assert_eq!(pending.tasks[0].status, TaskStatus::Unknown);
        assert!(pending.tasks[0].input_request_id.is_none());
        std::fs::remove_file(log).unwrap();
        let missing = p.poll(native_sessions::time(202000).unwrap());
        assert!(missing.tasks[0].stale);
        assert!(missing.tasks[0].finished_at.is_none());
    }
    #[test]
    fn partial_malformed_replaced_transcript_never_publishes_a_historical_success() {
        let (_dir, _db, mut p, log) = fixture();
        append(&log, "message", Some("user"), 100000, json!({}));
        let running = p.poll(native_sessions::time(101000).unwrap());
        assert_eq!(running.tasks.len(), 1);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(b"{\"type\":")
            .unwrap();
        let partial = p.poll(native_sessions::time(102000).unwrap());
        assert!(partial.tasks[0].stale);
        assert_eq!(partial.tasks[0].status, TaskStatus::Running);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(b"!invalid}\n")
            .unwrap();
        for _ in 0..2 {
            assert!(p.poll(native_sessions::time(103000).unwrap()).tasks[0].stale);
        }
        std::fs::write(&log, "").unwrap();
        append(&log, "message", Some("user"), 104000, json!({}));
        let replaced = p.poll(native_sessions::time(105000).unwrap());
        assert_eq!(replaced.tasks[0].status, TaskStatus::Running);
        assert!(replaced.tasks[0].new_execution);
    }
    #[test]
    #[ignore = "read-only real local WorkBuddy database and native transcripts"]
    fn live_workbuddy_snapshot() {
        let mut p = WorkBuddyProvider::new(default_root());
        let mut result = None;
        for _ in 0..100 {
            if let Ok(rows) = p.read() {
                result = Some(rows);
                break;
            }
        }
        let rows = result.expect("native records must be readable and caught up");
        assert!(!rows.is_empty(), "run one real local WorkBuddy task first");
        assert!(rows
            .iter()
            .all(|t| t.agent_id.as_deref() == Some("workbuddy") && t.turn_key.is_some()));
        println!(
            "WorkBuddy native rows={} running={} ended={} models={}",
            rows.len(),
            rows.iter()
                .filter(|t| t.status == TaskStatus::Running)
                .count(),
            rows.iter().filter(|t| t.finished_at.is_some()).count(),
            rows.iter().filter(|t| t.model.is_some()).count()
        );
    }
}
