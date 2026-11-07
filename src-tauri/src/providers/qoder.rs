//! Qoder Desktop turn projections and IDE/CLI transcripts, read-only.
//! Per-tool completion and blockable Stop hooks never finish a task.
use super::{
    kimi::{stamp, text},
    native_sessions::{self, Sessions},
    Provider, ProviderOutput,
};
use crate::domain::{ExecutionPhase, TaskSnapshot, TaskStatus, WaitingReason};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn default_roots() -> Vec<PathBuf> {
    let home = dirs_next::home_dir().unwrap_or_default();
    let mut paths = vec![];
    if let Some(data) = dirs_next::config_dir() {
        paths.push(data.join("com.qodercn.app.stable"));
        paths.push(data.join("com.qoder.app.stable"));
    }
    paths.push(
        std::env::var_os("QODER_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".qoder")),
    );
    paths
}
pub fn available(root: &Path) -> bool {
    root.join("main.sqlite").is_file() || root.join("projects").is_dir()
}
pub struct QoderProvider {
    roots: Vec<PathBuf>,
    sessions: Sessions,
    models: HashMap<(String, String), String>,
    transcripts: super::qoder_transcript::Transcripts,
}
impl QoderProvider {
    pub fn new(location: Option<PathBuf>) -> Self {
        Self {
            roots: location.map(|p| vec![p]).unwrap_or_else(default_roots),
            sessions: Default::default(),
            models: HashMap::new(),
            transcripts: Default::default(),
        }
    }
    pub fn resume(&mut self, tasks: Vec<TaskSnapshot>) {
        for task in &tasks {
            if task.agent_id.as_deref() == Some("qoder-desktop") {
                if let (Some(id), Some(turn), Some(model)) =
                    (&task.session_id, &task.turn_key, &task.model)
                {
                    self.models
                        .insert((id.clone(), turn.clone()), model.clone());
                }
            }
        }
        for agent in ["qoder-desktop", "qoder-transcript"] {
            self.sessions.resume(
                agent,
                tasks
                    .iter()
                    .filter(|t| t.agent_id.as_deref() == Some(agent))
                    .cloned(),
            );
        }
    }
    fn desktop(&mut self, path: &Path) -> Result<Vec<TaskSnapshot>, String> {
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| "database_open")?;
        db.busy_timeout(Duration::from_millis(250))
            .map_err(|_| "database_busy")?;
        let tx = db.transaction().map_err(|_| "database_snapshot")?;
        let mut query=tx.prepare("SELECT session_id,coalesce(title,''),coalesce(cwd,''),model FROM chat_sessions WHERE deleted_at IS NULL AND archived=0 AND owner_session_id IS NULL ORDER BY updated_at DESC LIMIT 401").map_err(|_|"unsupported_desktop_schema")?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|_| "desktop_read")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "desktop_read")?;
        if rows.len() > 400 {
            return Err("session_budget".into());
        }
        let mut out = vec![];
        let mut bytes = 0usize;
        for (id, title, cwd, model) in rows {
            let mut q=tx.prepare("SELECT turn_id,status,source,created_at,length(payload_json),payload_json,updated_at FROM chat_session_messages WHERE session_id=?1 ORDER BY sequence DESC LIMIT 129").map_err(|_|"message_schema")?;
            let mut r = q.query([&id]).map_err(|_| "message_read")?;
            let mut messages = vec![];
            while let Some(row) = r.next().map_err(|_| "message_read")? {
                let len: usize = row.get(4).map_err(|_| "message_size")?;
                bytes = bytes.saturating_add(len);
                if len > 4 * 1024 * 1024 || bytes > 16 * 1024 * 1024 {
                    return Err("message_budget".into());
                }
                let json: String = row.get(5).map_err(|_| "message_read")?;
                let value: Value = serde_json::from_str(&json).map_err(|_| "message_json")?;
                messages.push((
                    row.get::<_, String>(0).map_err(|_| "turn_id")?,
                    row.get::<_, String>(1).map_err(|_| "message_status")?,
                    row.get::<_, String>(2).map_err(|_| "message_source")?,
                    row.get::<_, String>(3).map_err(|_| "message_timestamp")?,
                    row.get::<_, String>(6).map_err(|_| "message_timestamp")?,
                    value,
                ));
                if messages.last().is_some_and(|(_, _, source, _, _, v)| {
                    text(v, "role") == Some("user")
                        && text(v, "runtimeInputState") != Some("not-delivered")
                        && source != "fork-inherited"
                        && source != "cli-import"
                }) {
                    break;
                }
            }
            let user = messages.iter().find(|(_, _, source, _, _, v)| {
                text(v, "role") == Some("user")
                    && text(v, "runtimeInputState") != Some("not-delivered")
                    && source != "fork-inherited"
                    && source != "cli-import"
            });
            let Some((turn, _, _, user_at, _, user_value)) = user else {
                continue;
            };
            let assistant = messages.iter().find(|(t, _, source, _, _, v)| {
                t == turn
                    && text(v, "role") == Some("assistant")
                    && source != "fork-inherited"
                    && source != "cli-import"
                    && v.get("backgroundTaskId").is_none_or(Value::is_null)
            });
            let started = assistant
                .and_then(|(_, _, _, _, _, v)| v.get("turnStartedAt"))
                .and_then(stamp)
                .or_else(|| super::codex::parse_rfc3339(user_at))
                .ok_or("turn_timestamp")?;
            let Some(mut task) = native_sessions::task(
                ("qoder-desktop", "Qoder Desktop"),
                &id,
                &title,
                &cwd,
                turn,
                started
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                started
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ) else {
                return Err("turn_timestamp".into());
            };
            if task.name.is_empty() {
                task.name = text(user_value, "text")
                    .unwrap_or("Qoder")
                    .chars()
                    .take(72)
                    .collect();
            }
            task.phase = Some(ExecutionPhase::AwaitingModel);
            task.cli = Some("qoder".into());
            if let Some((_, status, _, _, update, value)) = assistant {
                if text(value, "turnId").is_some_and(|id| id != turn) {
                    return Err("desktop_turn_mismatch".into());
                }
                let duration = value
                    .pointer("/turnMetrics/durationMs")
                    .and_then(Value::as_u64);
                let terminal = matches!(status.as_str(), "completed" | "failed" | "interrupted")
                    && duration.is_some()
                    && value
                        .pointer("/turnMetrics/turnCount")
                        .and_then(Value::as_u64)
                        .is_some();
                if terminal {
                    task.finished_at =
                        started.checked_add(Duration::from_millis(duration.unwrap()));
                    task.updated_at = task.finished_at.ok_or("duration_range")?;
                    task.phase = None;
                    task.status = if status == "failed" {
                        TaskStatus::Failed
                    } else {
                        TaskStatus::WaitingInput
                    };
                    task.waiting_reason = (task.status == TaskStatus::WaitingInput)
                        .then_some(WaitingReason::TurnEnded);
                    if status == "interrupted" {
                        task.outcome = Some("interrupted".into());
                    }
                } else if status != "streaming" {
                    task.stale = true;
                    task.status = TaskStatus::Unknown;
                    task.phase = None;
                } else {
                    task.updated_at = super::codex::parse_rfc3339(update)
                        .or_else(|| value.get("timestamp").and_then(stamp))
                        .unwrap_or(started);
                    if value
                        .get("tools")
                        .and_then(Value::as_array)
                        .is_some_and(|tools| {
                            tools.iter().any(|t| text(t, "status") == Some("running"))
                        })
                    {
                        task.phase = Some(ExecutionPhase::Tool);
                    }
                }
            }
            // This is the host's selected model, captured while the turn is active.
            // Never rewrite completed history from a later idle model selection.
            let key = (id.clone(), turn.clone());
            if task.status == TaskStatus::Running {
                if let Some(model) = model.as_deref().and_then(super::reported_model) {
                    self.models.entry(key.clone()).or_insert(model);
                }
            }
            task.model = self.models.get(&key).cloned();
            out.push(task);
        }
        Ok(out)
    }
    fn read(&mut self) -> Result<Vec<TaskSnapshot>, String> {
        let mut rows = vec![];
        for root in self.roots.clone() {
            let db = root.join("main.sqlite");
            if db.exists() {
                rows.extend(self.desktop(&db)?);
            } else if root.join("projects").exists() {
                rows.extend(self.transcripts.read(&root)?);
            }
        }
        let live: std::collections::HashSet<_> = rows
            .iter()
            .filter_map(|t| Some((t.session_id.clone()?, t.turn_key.clone()?)))
            .collect();
        let mut ids = std::collections::HashSet::new();
        if rows.iter().any(|t| !ids.insert(t.id.clone())) {
            return Err("duplicate_session_identity".into());
        }
        self.models.retain(|key, _| live.contains(key));
        Ok(rows)
    }
}
impl Provider for QoderProvider {
    fn name(&self) -> &'static str {
        "qoder"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match self.read() {
            Ok(rows) => self.sessions.observe(rows, now),
            Err(code) => self
                .sessions
                .degraded(&format!("Qoder 会话暂不可读（{code}）；保留上次状态")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, Connection, QoderProvider) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE chat_sessions(session_id TEXT,title TEXT,cwd TEXT,model TEXT,deleted_at TEXT,archived INTEGER,owner_session_id TEXT,updated_at TEXT);CREATE TABLE chat_session_messages(session_id TEXT,message_id TEXT,turn_id TEXT,sequence INTEGER,payload_json TEXT,status TEXT,source TEXT,created_at TEXT,updated_at TEXT);INSERT INTO chat_sessions VALUES('s','Native title','C:/public/project','native-model',NULL,0,NULL,'2026-11-07T00:00:00Z');").unwrap();
        let provider = QoderProvider::new(Some(dir.path().into()));
        (dir, db, provider)
    }
    fn message(db: &Connection, id: &str, turn: &str, seq: i64, status: &str, v: Value) {
        db.execute("INSERT INTO chat_session_messages VALUES('s',?1,?2,?3,?4,?5,'host-projection','2026-11-07T00:00:00Z','2026-11-07T00:00:00Z')",rusqlite::params![id,turn,seq,v.to_string(),status]).unwrap();
    }
    #[test]
    fn only_full_current_turn_metrics_finish_the_desktop_turn() {
        let (dir, db, mut provider) = fixture();
        message(
            &db,
            "u",
            "t",
            1,
            "completed",
            json!({"id":"u","role":"user","text":"Fixture"}),
        );
        message(
            &db,
            "a",
            "t",
            2,
            "streaming",
            json!({"id":"a","role":"assistant","turnId":"t","tools":[{"status":"running"}],"timestamp":"2026-11-07T00:00:01Z"}),
        );
        let first = provider.desktop(&dir.path().join("main.sqlite")).unwrap();
        assert_eq!(first[0].status, TaskStatus::Running);
        assert_eq!(first[0].model.as_deref(), Some("native-model"));
        assert_eq!(first[0].phase, Some(ExecutionPhase::Tool));
        db.execute(
            "UPDATE chat_session_messages SET status='completed' WHERE message_id='a'",
            [],
        )
        .unwrap();
        let unknown = provider.desktop(&dir.path().join("main.sqlite")).unwrap();
        assert!(unknown[0].stale && unknown[0].finished_at.is_none());
        db.execute("UPDATE chat_session_messages SET payload_json=?1 WHERE message_id='a'",[json!({"id":"a","role":"assistant","turnId":"t","turnMetrics":{"durationMs":61000,"turnCount":3}}).to_string()]).unwrap();
        let ended = provider.desktop(&dir.path().join("main.sqlite")).unwrap();
        assert_eq!(ended[0].waiting_reason, Some(WaitingReason::TurnEnded));
        assert_eq!(
            ended[0]
                .finished_at
                .unwrap()
                .duration_since(ended[0].started_at.unwrap())
                .unwrap(),
            Duration::from_secs(61)
        );
        db.execute(
            "UPDATE chat_sessions SET title='Renamed',model='next-model'",
            [],
        )
        .unwrap();
        let renamed = provider.desktop(&dir.path().join("main.sqlite")).unwrap();
        assert_eq!(renamed[0].id, ended[0].id);
        assert_eq!(renamed[0].finished_at, ended[0].finished_at);
        assert_eq!(renamed[0].model, ended[0].model);
    }
    #[test]
    fn inherited_history_and_undelivered_prompts_are_not_new_turns() {
        let (dir, db, mut p) = fixture();
        message(
            &db,
            "u",
            "t",
            1,
            "completed",
            json!({"role":"user","runtimeInputState":"not-delivered"}),
        );
        assert!(p
            .desktop(&dir.path().join("main.sqlite"))
            .unwrap()
            .is_empty());
        db.execute(
            "UPDATE chat_session_messages SET source='fork-inherited',payload_json=?1",
            [json!({"role":"user"}).to_string()],
        )
        .unwrap();
        assert!(p
            .desktop(&dir.path().join("main.sqlite"))
            .unwrap()
            .is_empty());
    }
    #[test]
    #[ignore = "read-only installed Qoder Desktop; no conversations/credentials printed"]
    fn live_qoder_desktop_schema() {
        let root = default_roots()
            .into_iter()
            .find(|r| r.join("main.sqlite").exists())
            .expect("installed Qoder Desktop required");
        let mut p = QoderProvider::new(Some(root.clone()));
        let rows = p.desktop(&root.join("main.sqlite")).unwrap();
        println!(
            "Qoder readable native sessions={}; empty is not task acceptance",
            rows.len()
        );
    }
}
