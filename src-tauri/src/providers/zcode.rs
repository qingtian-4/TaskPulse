//! Read-only ZCode native sessions. Tool completion is never turn completion.
use super::{Provider, ProviderOutput};
use crate::domain::{
    DetectionConfidence, ExecutionPhase, TaskSnapshot, TaskSource, TaskStatus, WaitingReason,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn default_db() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_default()
        .join(".zcode/cli/db/db.sqlite")
}

pub struct ZcodeProvider {
    path: PathBuf,
    previous: HashMap<String, TaskSnapshot>,
    followed: HashSet<String>,
    baseline: Option<SystemTime>,
}
impl ZcodeProvider {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            previous: HashMap::new(),
            followed: HashSet::new(),
            baseline: None,
        }
    }
    pub fn resume(&mut self, known: impl IntoIterator<Item = TaskSnapshot>) {
        for mut task in known {
            if let Some(session) = task.session_id.as_ref() {
                task.id = format!("zcode:{session}");
                task.new_execution = false;
                self.followed.insert(task.id.clone());
                self.previous.insert(task.id.clone(), task);
            }
        }
    }
    fn read(path: &Path) -> rusqlite::Result<Vec<TaskSnapshot>> {
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(250))?;
        let tx = db.transaction()?;
        let mut sessions = tx.prepare(
            "SELECT id, title, directory, last_outcome FROM session WHERE time_deleted IS NULL AND time_archived IS NULL ORDER BY time_updated DESC LIMIT 400"
        )?;
        let rows = sessions.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, title, project, outcome) = row?;
            // Runtime reminders also have role=user. Only an actual user prompt
            // creates a turn, and it must remain discoverable through long tool loops.
            let user: Option<(String,u64,i64)>=tx.query_row(
                "SELECT id,json_extract(data,'$.time.created'),sequence FROM message WHERE session_id=?1 AND json_extract(data,'$.role')='user' AND coalesce(json_extract(data,'$.synthetic'),0)=0 AND (json_extract(data,'$.semantics.origin') IS NULL OR json_extract(data,'$.semantics.origin')='real_user') ORDER BY sequence DESC LIMIT 1",
                [&id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))
            ).optional()?;
            let Some((user_id, started, user_sequence)) = user else {
                continue;
            };
            let mut messages = tx.prepare("SELECT id, json_object('role',json_extract(data,'$.role'),'created',json_extract(data,'$.time.created'),'completed',json_extract(data,'$.time.completed'),'parent',json_extract(data,'$.parentID'),'model',coalesce(json_extract(data,'$.modelId'),json_extract(data,'$.modelID')),'finish',json_extract(data,'$.finish'),'error_code',json_extract(data,'$.error.data.code'),'turn_result',json_extract(data,'$.error.data.turnResult'),'error',json_type(data,'$.error') NOT IN ('null') AND json_type(data,'$.error') IS NOT NULL) FROM message WHERE session_id=?1 AND sequence>=?2 AND (json_extract(data,'$.role')='assistant' OR id=?3) ORDER BY sequence DESC LIMIT 64")?;
            let messages = messages
                .query_map(rusqlite::params![id, user_sequence, user_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let messages: Vec<_> = messages
                .into_iter()
                .filter_map(|(id, json)| serde_json::from_str::<Value>(&json).ok().map(|v| (id, v)))
                .collect();
            let Some((message_id, latest)) = messages.first() else {
                continue;
            };
            let activity = latest["completed"]
                .as_u64()
                .or_else(|| latest["created"].as_u64())
                .unwrap_or(0);
            if started == 0 || started > 4_102_444_800_000 || activity > 4_102_444_800_000 {
                continue;
            }
            let mut task = TaskSnapshot::new(
                format!("zcode:{id}"),
                title,
                TaskSource::NativeAgent,
                TaskStatus::Running,
                DetectionConfidence::Inferred,
                UNIX_EPOCH + Duration::from_millis(activity.max(started)),
            );
            task.agent_id = Some("zcode".into());
            task.agent_name = Some("ZCode".into());
            task.evidence_channel = Some("native".into());
            task.session_id = Some(id.clone());
            task.project_path = (!project.is_empty()).then_some(project.clone());
            task.project = Path::new(&project)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned());
            task.turn_key = Some(user_id.clone());
            task.started_at = Some(UNIX_EPOCH + Duration::from_millis(started));
            task.model = messages
                .iter()
                .take_while(|(id, _)| id != &user_id)
                .find_map(|(_, v)| v["model"].as_str().and_then(super::reported_model));
            // Native outcome alone can belong to an older turn. Require the latest
            // assistant to explicitly reference this user turn and finish it.
            let current_assistant = latest["role"] == "assistant"
                && latest["parent"].as_str() == Some(user_id.as_str())
                && latest["completed"].as_u64().is_some();
            if current_assistant
                && latest["finish"] != "tool-calls"
                && (latest["error_code"] == "model_request_cancelled"
                    || latest["turn_result"] == "cancelled")
            {
                task.status = TaskStatus::WaitingInput;
                task.waiting_reason = Some(WaitingReason::TurnEnded);
                task.outcome = Some("interrupted".into());
            } else if current_assistant && latest["error"].as_i64() == Some(1) {
                task.status = TaskStatus::Failed;
            } else if current_assistant
                && outcome.as_deref() == Some("completed")
                && latest["finish"] == "stop"
            {
                task.status = TaskStatus::Succeeded;
            }
            if task.status == TaskStatus::Running {
                let mut parts = tx.prepare("SELECT json_extract(data,'$.type'), json_extract(data,'$.state.status') FROM part WHERE message_id=?1 ORDER BY sequence DESC LIMIT 64")?;
                let parts = parts
                    .query_map([message_id], |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                task.phase = Some(
                    if parts.iter().any(|(kind, status)| {
                        kind.as_deref() == Some("tool")
                            && matches!(status.as_deref(), Some("pending" | "running"))
                    }) {
                        ExecutionPhase::Tool
                    } else if parts
                        .iter()
                        .any(|(kind, _)| kind.as_deref() == Some("reasoning"))
                    {
                        ExecutionPhase::Thinking
                    } else {
                        ExecutionPhase::AwaitingModel
                    },
                );
            } else {
                task.finished_at = Some(task.updated_at);
            }
            out.push(task);
        }
        Ok(out)
    }
    fn observe(&mut self, rows: Vec<TaskSnapshot>, now: SystemTime) -> ProviderOutput {
        let mut current = HashMap::new();
        for mut task in rows {
            let old = self.previous.get(&task.id);
            task.new_execution = old.is_some_and(|old| old.turn_key != task.turn_key)
                || old.is_none()
                    && self.baseline.is_some_and(|baseline| {
                        task.started_at.is_some_and(|started| started >= baseline)
                    });
            let fresh =
                now.duration_since(task.updated_at).unwrap_or_default() < Duration::from_secs(120);
            let resumed_activity = old.is_some_and(|old| task.updated_at > old.updated_at)
                && self
                    .baseline
                    .is_some_and(|baseline| task.updated_at >= baseline);
            if task.new_execution
                || resumed_activity
                || old.is_none() && task.status == TaskStatus::Running && fresh
            {
                self.followed.insert(task.id.clone());
            }
            // Silence affects freshness only, never lifecycle or turn identity.
            task.stale = task.status == TaskStatus::Running && !fresh;
            current.insert(task.id.clone(), task);
        }
        for (id, old) in &self.previous {
            if self.followed.contains(id)
                && !current.contains_key(id)
                && old.status == TaskStatus::Running
            {
                let mut task = old.clone();
                task.stale = true;
                task.new_execution = false;
                current.insert(id.clone(), task);
            }
        }
        if current.len() > 1000 {
            self.followed.retain(|id| self.previous.contains_key(id));
            return ProviderOutput::degraded(
                self.previous
                    .values()
                    .filter(|t| self.followed.contains(&t.id))
                    .cloned()
                    .map(|mut task| {
                        task.stale = true;
                        task.new_execution = false;
                        task
                    })
                    .collect(),
                "ZCode 原生会话数量超过读取预算；保留上次状态",
            );
        }
        self.followed.retain(|id| current.contains_key(id));
        let tasks = current
            .values()
            .filter(|t| self.followed.contains(&t.id))
            .cloned()
            .collect();
        let catalog = current.values().cloned().collect();
        self.previous = current;
        self.baseline.get_or_insert(now);
        ProviderOutput {
            tasks,
            catalog,
            ..Default::default()
        }
    }
}
impl Provider for ZcodeProvider {
    fn name(&self) -> &'static str {
        "zcode"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match Self::read(&self.path) {
            Ok(rows) => self.observe(rows, now),
            Err(_) => ProviderOutput::degraded(
                self.previous
                    .values()
                    .filter(|t| self.followed.contains(&t.id))
                    .cloned()
                    .map(|mut t| {
                        t.stale = true;
                        t.new_execution = false;
                        t
                    })
                    .collect(),
                "ZCode 原生数据库暂不可读；保留上次任务状态",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(path: &Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch("CREATE TABLE session(id TEXT,title TEXT,directory TEXT,last_outcome TEXT,time_deleted INTEGER,time_archived INTEGER,time_updated INTEGER); CREATE TABLE message(id TEXT,session_id TEXT,sequence INTEGER,data TEXT); CREATE TABLE part(id TEXT,message_id TEXT,sequence INTEGER,data TEXT); INSERT INTO session VALUES('s','Real title','D:/project','completed',NULL,NULL,3000);").unwrap();
        message(&db, "u1", 1, json!({"role":"user","time":{"created":1000}}));
        message(
            &db,
            "a1",
            2,
            json!({"role":"assistant","parentID":"u1","modelId":"model-a","time":{"created":1100,"completed":2000},"finish":"stop"}),
        );
        db
    }
    fn message(db: &Connection, id: &str, seq: u32, v: Value) {
        db.execute(
            "INSERT INTO message VALUES(?1,'s',?2,?3)",
            rusqlite::params![id, seq, v.to_string()],
        )
        .unwrap();
    }
    #[test]
    fn completed_cache_entries_expire_without_inventing_running_completion() {
        let mut provider = ZcodeProvider::new(PathBuf::new());
        let now = SystemTime::now();
        let mut terminal = TaskSnapshot::new(
            "zcode:done",
            "Done",
            TaskSource::NativeAgent,
            TaskStatus::Succeeded,
            DetectionConfidence::Inferred,
            now,
        );
        terminal.session_id = Some("done".into());
        let mut running = terminal.clone();
        running.id = "zcode:running".into();
        running.session_id = Some("running".into());
        running.status = TaskStatus::Running;
        provider.resume([terminal, running]);
        let output = provider.observe(vec![], now);
        assert_eq!(output.tasks.len(), 1);
        assert_eq!(output.tasks[0].status, TaskStatus::Running);
        assert!(output.tasks[0].stale);
        assert_eq!(provider.followed.len(), 1);
        assert_eq!(provider.previous.len(), 1);
    }

    #[test]
    fn restart_resumes_only_known_tasks_and_stale_interruption_cannot_end_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = fixture(&path);
        let original = ZcodeProvider::read(&path).unwrap().pop().unwrap();
        let mut provider = ZcodeProvider::new(path.clone());
        provider.resume([original]);
        let resumed = provider.poll(UNIX_EPOCH + Duration::from_millis(900000));
        assert_eq!(resumed.tasks.len(), 1);
        assert_eq!(resumed.tasks[0].status, TaskStatus::Succeeded);
        message(
            &db,
            "u2",
            3,
            json!({"role":"user","time":{"created":910000}}),
        );
        db.execute("UPDATE session SET last_outcome='interrupted'", [])
            .unwrap();
        message(
            &db,
            "a2",
            4,
            json!({"role":"assistant","parentID":"u2","time":{"created":911000,"completed":912000},"finish":"tool-calls"}),
        );
        let running = provider.poll(UNIX_EPOCH + Duration::from_millis(913000));
        assert_eq!(running.tasks[0].status, TaskStatus::Running);
        let mut restarted = ZcodeProvider::new(path);
        restarted.resume(running.tasks);
        assert_eq!(
            restarted
                .poll(UNIX_EPOCH + Duration::from_millis(1200000))
                .tasks[0]
                .status,
            TaskStatus::Running
        );
        db.execute("UPDATE session SET last_outcome='completed'", [])
            .unwrap();
        message(
            &db,
            "end",
            5,
            json!({"role":"assistant","parentID":"u2","time":{"created":1210000,"completed":1220000},"finish":"stop"}),
        );
        assert_eq!(
            restarted
                .poll(UNIX_EPOCH + Duration::from_millis(1230000))
                .tasks[0]
                .status,
            TaskStatus::Succeeded
        );
    }

    #[test]
    fn internal_reminders_long_turns_and_short_new_tasks_keep_real_execution_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = fixture(&path);
        let mut provider = ZcodeProvider::new(path);
        provider.poll(UNIX_EPOCH + Duration::from_millis(2500));
        message(
            &db,
            "u2",
            3,
            json!({"role":"user","time":{"created":3000},"semantics":{"origin":"real_user"}}),
        );
        for n in 4..104 {
            message(
                &db,
                &format!("step-{n}"),
                n,
                json!({"role":"assistant","parentID":"u2","modelId":"model-long","time":{"created":3100,"completed":3200},"finish":"tool-calls"}),
            );
        }
        message(
            &db,
            "internal",
            104,
            json!({"role":"user","synthetic":true,"semantics":{"origin":"agent_runtime"},"time":{"created":3300}}),
        );
        let running = provider.poll(UNIX_EPOCH + Duration::from_millis(3500));
        assert_eq!(running.tasks[0].turn_key.as_deref(), Some("u2"));
        assert!(running.tasks[0].new_execution);
        assert_eq!(running.tasks[0].status, TaskStatus::Running);
        message(
            &db,
            "answer",
            105,
            json!({"role":"assistant","parentID":"u2","time":{"created":3400,"completed":4000},"finish":"stop"}),
        );
        let complete = provider.poll(UNIX_EPOCH + Duration::from_millis(5000));
        assert_eq!(complete.tasks[0].status, TaskStatus::Succeeded);
        assert!(!complete.tasks[0].new_execution);
        db.execute("INSERT INTO session VALUES('short','Short real task','D:/project','completed',NULL,NULL,6500)",[]).unwrap();
        for (id, seq, data) in [
            ("short-u", 1, json!({"role":"user","time":{"created":6000}})),
            (
                "short-a",
                2,
                json!({"role":"assistant","parentID":"short-u","time":{"created":6100,"completed":6500},"finish":"stop"}),
            ),
        ] {
            db.execute(
                "INSERT INTO message VALUES(?1,'short',?2,?3)",
                rusqlite::params![id, seq, data.to_string()],
            )
            .unwrap();
        }
        let short = provider.poll(UNIX_EPOCH + Duration::from_millis(7000));
        let task = short
            .tasks
            .iter()
            .find(|t| t.session_id.as_deref() == Some("short"))
            .unwrap();
        assert!(task.new_execution);
        assert_eq!(task.status, TaskStatus::Succeeded);
        db.execute(
            "UPDATE session SET last_outcome='interrupted' WHERE id='short'",
            [],
        )
        .unwrap();
        db.execute("UPDATE message SET data=?1 WHERE id='short-a'",[json!({"role":"assistant","parentID":"short-u","time":{"created":6100,"completed":6500},"error":{"name":"TransportError","data":{"code":"model_request_cancelled","turnResult":"cancelled"}}}).to_string()]).unwrap();
        let interrupted = provider.poll(UNIX_EPOCH + Duration::from_millis(8000));
        let task = interrupted
            .tasks
            .iter()
            .find(|t| t.session_id.as_deref() == Some("short"))
            .unwrap();
        assert_eq!(task.outcome.as_deref(), Some("interrupted"));
        assert_eq!(task.waiting_reason, Some(WaitingReason::TurnEnded));
    }

    #[test]
    fn native_turn_identity_tool_phases_and_outcomes_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = fixture(&path);
        let mut provider = ZcodeProvider::new(path.clone());
        let now = UNIX_EPOCH + Duration::from_millis(5000);
        let output = provider.poll(now);
        assert!(output.authoritative);
        assert!(output.tasks.is_empty());
        assert_eq!(output.catalog[0].status, TaskStatus::Succeeded);
        message(&db, "u2", 3, json!({"role":"user","time":{"created":3000}}));
        let output = provider.poll(now);
        assert_eq!(output.tasks[0].status, TaskStatus::Running);
        assert!(output.tasks[0].new_execution);
        assert_eq!(output.tasks[0].turn_key.as_deref(), Some("u2"));
        message(
            &db,
            "a2",
            4,
            json!({"role":"assistant","parentID":"u2","modelId":"model-b","time":{"created":3100,"completed":3200},"finish":"tool-calls"}),
        );
        db.execute(
            "INSERT INTO part VALUES('p','a2',1,?1)",
            [json!({"type":"tool","state":{"status":"running"}}).to_string()],
        )
        .unwrap();
        let output = provider.poll(now);
        assert_eq!(output.tasks[0].phase, Some(ExecutionPhase::Tool));
        assert_eq!(output.tasks[0].status, TaskStatus::Running);
        assert_eq!(output.tasks[0].model.as_deref(), Some("model-b"));
        assert!(!output.tasks[0].new_execution);
        db.execute(
            "UPDATE part SET data=?1",
            [json!({"type":"tool","state":{"status":"completed"}}).to_string()],
        )
        .unwrap();
        assert_eq!(provider.poll(now).tasks[0].status, TaskStatus::Running);
        message(
            &db,
            "a3",
            5,
            json!({"role":"assistant","parentID":"u2","time":{"created":3300,"completed":4000},"finish":"stop"}),
        );
        let output = provider.poll(now);
        assert_eq!(output.tasks[0].status, TaskStatus::Succeeded);
        let activity = output.tasks[0].updated_at;
        db.execute("UPDATE session SET title='Renamed',time_updated=99999", [])
            .unwrap();
        let renamed = provider.poll(now);
        assert_eq!(renamed.tasks[0].name, "Renamed");
        assert_eq!(renamed.tasks[0].updated_at, activity);
        assert_eq!(renamed.tasks[0].turn_key.as_deref(), Some("u2"));
        assert!(!renamed.tasks[0].new_execution);
        message(&db, "u3", 6, json!({"role":"user","time":{"created":4500}}));
        provider.poll(now);
        message(
            &db,
            "a4",
            7,
            json!({"role":"assistant","parentID":"u3","time":{"created":4600,"completed":4700},"error":{"name":"Failure"}}),
        );
        assert_eq!(provider.poll(now).tasks[0].status, TaskStatus::Failed);
        drop(db);
        std::fs::remove_file(path).unwrap();
        let missing = provider.poll(now);
        assert!(!missing.authoritative);
        assert_eq!(missing.tasks[0].status, TaskStatus::Failed);
        assert!(missing.tasks[0].stale);
    }
    #[test]
    #[ignore = "read-only native ZCode database; reports only counts and model presence"]
    fn live_zcode_native_metadata() {
        let mut provider = ZcodeProvider::new(default_db());
        let output = provider.poll(SystemTime::now());
        assert!(output.authoritative, "{:?}", output.error);
        assert!(!output.catalog.is_empty());
        let models = output.catalog.iter().filter(|t| t.model.is_some()).count();
        println!(
            "ZCode sessions={} active={} models={models}",
            output.catalog.len(),
            output.tasks.len()
        );
        assert!(models > 0);
    }
}
