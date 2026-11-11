//! Qoder Desktop turn projections and IDE/CLI transcripts, read-only.
//! Per-tool completion and blockable Stop hooks never finish a task.
use super::{
    kimi::{stamp, text},
    native_sessions::{self, BatchRead, Sessions},
    Provider, ProviderOutput,
};
use crate::domain::{ExecutionPhase, TaskSnapshot, TaskStatus, WaitingReason};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
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
type DesktopCursor = (String, String);

pub struct QoderProvider {
    roots: Vec<PathBuf>,
    sessions: Sessions,
    models: HashMap<(String, String), String>,
    transcripts: super::qoder_transcript::Transcripts,
    cursors: HashMap<PathBuf, DesktopCursor>,
    next_head: HashMap<PathBuf, String>,
}
impl QoderProvider {
    pub fn new(location: Option<PathBuf>) -> Self {
        Self {
            roots: location.map(|p| vec![p]).unwrap_or_else(default_roots),
            sessions: Default::default(),
            models: HashMap::new(),
            transcripts: Default::default(),
            cursors: HashMap::new(),
            next_head: HashMap::new(),
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
    fn desktop(&mut self, path: &Path) -> Result<(Vec<TaskSnapshot>, BatchRead), String> {
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| "database_open")?;
        db.busy_timeout(Duration::from_millis(250))
            .map_err(|_| "database_busy")?;
        let tx = db.transaction().map_err(|_| "database_snapshot")?;
        let mut protected: Vec<_> = self
            .sessions
            .protected_ids("qoder-desktop")
            .into_iter()
            .collect();
        protected.sort();
        let mut status = if protected.len() > 128 {
            BatchRead::Paginating
        } else {
            BatchRead::Complete
        };
        protected.truncate(128);
        let priority = serde_json::to_string(&protected).map_err(|_| "session_ids")?;
        let cursor = self.cursors.get(path).cloned();
        let (updated, last_id) = cursor
            .as_ref()
            .map(|(a, b)| (Some(a.as_str()), b.as_str()))
            .unwrap_or((None, ""));
        const SELECT: &str = "SELECT session_id,coalesce(title,''),coalesce(cwd,''),model,coalesce(updated_at,'')
            FROM chat_sessions WHERE deleted_at IS NULL AND archived=0 AND owner_session_id IS NULL";
        let row = |r: &rusqlite::Row<'_>| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        };
        let mut heads = tx.prepare(&format!("{SELECT} AND (session_id IN(SELECT value FROM json_each(?1))
            OR session_id IN (SELECT session_id FROM chat_sessions WHERE deleted_at IS NULL AND archived=0 AND owner_session_id IS NULL
                              ORDER BY coalesce(updated_at,'') DESC, session_id LIMIT 32))
            ORDER BY session_id IN(SELECT value FROM json_each(?1)) DESC, coalesce(updated_at,'') DESC, session_id LIMIT 160"))
            .map_err(|_| "unsupported_desktop_schema")?;
        let mut rows = heads
            .query_map([priority], row)
            .map_err(|_| "desktop_read")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "desktop_read")?;
        if let Some(index) = self
            .next_head
            .get(path)
            .and_then(|id| rows.iter().position(|row| &row.0 == id))
        {
            rows.rotate_left(index);
        }
        let mut page = tx
            .prepare(&format!(
                "{SELECT} AND (?1 IS NULL OR coalesce(updated_at,'')<?1
            OR (coalesce(updated_at,'')=?1 AND session_id>?2))
            ORDER BY coalesce(updated_at,'') DESC, session_id LIMIT 129"
            ))
            .map_err(|_| "unsupported_desktop_schema")?;
        let mut entries = page
            .query_map(rusqlite::params![updated, last_id], row)
            .map_err(|_| "desktop_read")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "desktop_read")?;
        let more = entries.len() > 128;
        entries.truncate(128);
        let next_cursor = entries
            .last()
            .map(|(id, _, _, _, at)| (at.clone(), id.clone()));
        if more || cursor.is_some() {
            status = BatchRead::Paginating;
        }
        if more {
            if let Some(next) = next_cursor {
                self.cursors.insert(path.to_path_buf(), next);
            }
        } else {
            self.cursors.remove(path);
        }
        let head_ids: HashSet<_> = rows.iter().map(|row| row.0.clone()).collect();
        rows.extend(entries);
        let mut seen = HashSet::new();
        rows.retain(|(id, _, _, _, _)| seen.insert(id.clone()));
        let storage_key = native_sessions::storage_key(path);
        let mut out = vec![];
        let mut bytes = 0usize;
        let mut next_head = None;
        for (id, title, cwd, model, _) in rows {
            let is_head = head_ids.contains(&id);
            if is_head && next_head.is_some() {
                continue;
            }
            let limit = if is_head {
                12 * 1024 * 1024
            } else {
                16 * 1024 * 1024
            };
            if bytes >= 16 * 1024 * 1024 {
                status = status.max(BatchRead::Paginating);
                break;
            }
            let used_before_session = bytes;
            let task = (|| -> Result<Option<TaskSnapshot>, &'static str> {
                let mut q=tx.prepare("SELECT turn_id,status,source,created_at,length(CAST(payload_json AS BLOB)),payload_json,updated_at FROM chat_session_messages WHERE session_id=?1 ORDER BY sequence DESC LIMIT 129").map_err(|_|"message_schema")?;
                let mut r = q.query([&id]).map_err(|_| "message_read")?;
                let mut messages = vec![];
                while let Some(row) = r.next().map_err(|_| "message_read")? {
                    let len: usize = row.get(4).map_err(|_| "message_size")?;
                    if len > 4 * 1024 * 1024 {
                        return Err("message_budget");
                    }
                    if bytes.saturating_add(len) > limit {
                        return Err("poll_budget");
                    }
                    bytes += len;
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
                    return if messages.len() >= 129 {
                        Err("message_budget")
                    } else {
                        Ok(None)
                    };
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
                    return Err("turn_timestamp");
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
                task.native_storage = Some(storage_key.clone());
                if let Some((_, status, _, _, update, value)) = assistant {
                    if text(value, "turnId").is_some_and(|id| id != turn) {
                        return Err("desktop_turn_mismatch");
                    }
                    let duration = value
                        .pointer("/turnMetrics/durationMs")
                        .and_then(Value::as_u64);
                    let terminal =
                        matches!(status.as_str(), "completed" | "failed" | "interrupted")
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
                Ok(Some(task))
            })();
            match task {
                Ok(Some(task)) => out.push(task),
                Ok(None) => {}
                Err("poll_budget") => {
                    // Start the next poll at the first deferred head. A session
                    // that alone exhausts its budget cannot pin the cursor.
                    if used_before_session == 0 {
                        status = BatchRead::Failed;
                    } else {
                        if is_head {
                            next_head = Some(id);
                        }
                        status = status.max(BatchRead::Paginating);
                    }
                }
                Err(_) => status = BatchRead::Failed,
            }
        }
        if let Some(id) = next_head {
            self.next_head.insert(path.to_path_buf(), id);
        } else {
            self.next_head.remove(path);
        }
        Ok((out, status))
    }
    fn read(&mut self) -> (Vec<TaskSnapshot>, BatchRead) {
        let mut rows = vec![];
        let mut status = BatchRead::Complete;
        let protected = self.sessions.protected_ids("qoder-transcript");
        for root in self.roots.clone() {
            let db = root.join("main.sqlite");
            let result = if db.exists() {
                self.desktop(&db)
            } else if root.join("projects").exists() {
                Ok(self.transcripts.read(&root, &protected))
            } else {
                continue;
            };
            match result {
                Ok((items, batch_status)) => {
                    rows.extend(items);
                    status = status.max(batch_status);
                }
                Err(_) => status = BatchRead::Failed,
            }
        }
        let live: std::collections::HashSet<_> = rows
            .iter()
            .filter_map(|t| Some((t.session_id.clone()?, t.turn_key.clone()?)))
            .collect();
        let mut ids = HashSet::new();
        let mut duplicate = HashSet::new();
        for task in &rows {
            if !ids.insert(task.id.clone()) {
                duplicate.insert(task.id.clone());
            }
        }
        if !duplicate.is_empty() {
            status = BatchRead::Failed;
        }
        rows.retain(|task| !duplicate.contains(&task.id));
        let protected = self.sessions.protected_ids("qoder-desktop");
        self.models
            .retain(|key, _| live.contains(key) || protected.contains(&key.0));
        (rows, status)
    }
}
impl Provider for QoderProvider {
    fn name(&self) -> &'static str {
        "qoder"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let (rows, status) = self.read();
        let mut output = self.sessions.observe_batch(
            rows,
            now,
            (status == BatchRead::Failed).then(|| "Qoder 记录读取失败；保留上次状态".into()),
        );
        output.authoritative &= status == BatchRead::Complete;
        output
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
        let first = provider.desktop(&dir.path().join("main.sqlite")).unwrap().0;
        assert_eq!(first[0].status, TaskStatus::Running);
        assert_eq!(first[0].model.as_deref(), Some("native-model"));
        assert_eq!(first[0].phase, Some(ExecutionPhase::Tool));
        db.execute(
            "UPDATE chat_session_messages SET status='completed' WHERE message_id='a'",
            [],
        )
        .unwrap();
        let unknown = provider.desktop(&dir.path().join("main.sqlite")).unwrap().0;
        assert!(unknown[0].stale && unknown[0].finished_at.is_none());
        db.execute("UPDATE chat_session_messages SET payload_json=?1 WHERE message_id='a'",[json!({"id":"a","role":"assistant","turnId":"t","turnMetrics":{"durationMs":61000,"turnCount":3}}).to_string()]).unwrap();
        let ended = provider.desktop(&dir.path().join("main.sqlite")).unwrap().0;
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
        let renamed = provider.desktop(&dir.path().join("main.sqlite")).unwrap().0;
        assert_eq!(renamed[0].id, ended[0].id);
        assert_eq!(renamed[0].finished_at, ended[0].finished_at);
        assert_eq!(renamed[0].model, ended[0].model);
    }
    #[test]
    fn qoder_capacity_retains_followed_turn_and_paginates_old_sessions() {
        let (_dir, db, mut provider) = fixture();
        let now = super::super::codex::parse_rfc3339("2026-11-07T00:00:03Z").unwrap();
        message(
            &db,
            "u",
            "t",
            1,
            "completed",
            json!({"role":"user","text":"Fixture"}),
        );
        message(
            &db,
            "a",
            "t",
            2,
            "streaming",
            json!({"role":"assistant","turnId":"t"}),
        );
        let first = provider.poll(now);
        assert!(first.authoritative && first.tasks.len() == 1);
        for n in 0..401 {
            let id = format!("old-{n:04}");
            db.execute("INSERT INTO chat_sessions VALUES(?1,'History','/older',NULL,NULL,0,NULL,'2026-11-08')",[&id]).unwrap();
            db.execute("INSERT INTO chat_session_messages VALUES(?1,'u','o',1,?2,'completed','host-projection','2026-11-01T00:00:00Z','2026-11-01T00:00:00Z')",rusqlite::params![id,json!({"role":"user"}).to_string()]).unwrap();
        }
        let active = provider.poll(now);
        assert!(!active.authoritative);
        assert_eq!(active.tasks.len(), 1);
        assert!(!active.tasks[0].stale);
        db.execute("UPDATE chat_session_messages SET status='completed',payload_json=?1 WHERE message_id='a'",
            [json!({"role":"assistant","turnId":"t","turnMetrics":{"durationMs":5000,"turnCount":1}}).to_string()]).unwrap();
        let finished = provider.poll(now + Duration::from_secs(6));
        assert_eq!(
            finished.tasks[0].waiting_reason,
            Some(WaitingReason::TurnEnded)
        );
        assert!(!finished.tasks[0].stale);
        assert!(finished
            .catalog
            .iter()
            .any(|task| task.session_id.as_deref() == Some("old-0200")));
    }

    #[test]
    fn desktop_message_budget_rotates_five_followed_sessions() {
        let (_dir, db, mut provider) = fixture();
        db.execute("DELETE FROM chat_sessions", []).unwrap();
        let now = super::super::codex::parse_rfc3339("2026-11-07T00:00:03Z").unwrap();
        for index in 0..5 {
            let id = format!("session-{index}");
            db.execute("INSERT INTO chat_sessions VALUES(?1,'Fixture','/project',NULL,NULL,0,NULL,'2026-11-07')",[&id]).unwrap();
            for (message, seq, value, status) in [
                ("u", 1, json!({"role":"user"}), "completed"),
                (
                    "a",
                    2,
                    json!({"role":"assistant","turnId":"t"}),
                    "streaming",
                ),
            ] {
                db.execute("INSERT INTO chat_session_messages VALUES(?1,?2,'t',?3,?4,?5,'host-projection','2026-11-07T00:00:00Z','2026-11-07T00:00:00Z')",
                    rusqlite::params![id,message,seq,value.to_string(),status]).unwrap();
            }
        }
        assert_eq!(provider.poll(now).tasks.len(), 5);
        let filler = "x".repeat(3 * 1024 * 1024 - 512);
        db.execute(
            "UPDATE chat_session_messages SET payload_json=?1 WHERE message_id='a'",
            [json!({"role":"assistant","turnId":"t","text":filler}).to_string()],
        )
        .unwrap();
        db.execute("UPDATE chat_session_messages SET status='completed',payload_json=?1 WHERE session_id='session-4' AND message_id='a'",
            [json!({"role":"assistant","turnId":"t","text":filler,"turnMetrics":{"durationMs":5000,"turnCount":1}}).to_string()]).unwrap();

        let first = provider.poll(now + Duration::from_secs(6));
        assert!(!first.authoritative);
        assert!(
            first.error.is_none(),
            "Budget-limited reads are not I/O errors"
        );
        let mut finished = first.tasks.iter().any(|task| {
            task.session_id.as_deref() == Some("session-4")
                && task.waiting_reason == Some(WaitingReason::TurnEnded)
        });
        for _ in 0..3 {
            let batch = provider.poll(now + Duration::from_secs(6));
            finished |= batch.tasks.iter().any(|task| {
                task.session_id.as_deref() == Some("session-4")
                    && task.waiting_reason == Some(WaitingReason::TurnEnded)
            });
        }
        assert!(
            finished,
            "Head budget exhaustion must not starve the last followed session"
        );
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
            .0
            .is_empty());
        db.execute(
            "UPDATE chat_session_messages SET source='fork-inherited',payload_json=?1",
            [json!({"role":"user"}).to_string()],
        )
        .unwrap();
        assert!(p
            .desktop(&dir.path().join("main.sqlite"))
            .unwrap()
            .0
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
        let rows = p.desktop(&root.join("main.sqlite")).unwrap().0;
        println!(
            "Qoder readable native sessions={}; empty is not task acceptance",
            rows.len()
        );
    }
}
