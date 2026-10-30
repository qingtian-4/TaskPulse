//! OpenCode read-only SQLite observer. v2 idle records are explicit turn facts.
//! Tool finishes and an older idle outcome never terminate a newer user turn.
use super::{
    native_sessions::{self, Sessions},
    Provider, ProviderOutput,
};
use crate::domain::{ExecutionPhase, TaskSnapshot, TaskStatus, WaitingReason};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

pub fn default_db() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs_next::home_dir()
                .unwrap_or_default()
                .join(".local/share")
        })
        .join("opencode/opencode.db")
}
pub struct OpenCodeProvider {
    path: PathBuf,
    sessions: Sessions,
}
impl OpenCodeProvider {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            sessions: Sessions::default(),
        }
    }
    pub fn resume(&mut self, tasks: impl IntoIterator<Item = TaskSnapshot>) {
        self.sessions.resume("opencode", tasks);
    }
    fn read(&self) -> rusqlite::Result<Vec<TaskSnapshot>> {
        let mut db = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(250))?;
        let tx = db.transaction()?;
        let v2: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_v2')",
            [],
            |r| r.get(0),
        )?;
        let sql = if v2 {
            "SELECT id,coalesce(nullif(title,''),slug),directory,model FROM session_v2 WHERE time_archived IS NULL ORDER BY time_updated DESC LIMIT 401"
        } else {
            "SELECT id,title,directory,NULL FROM session WHERE time_archived IS NULL ORDER BY time_updated DESC LIMIT 401"
        };
        let mut rows = tx.prepare(sql)?;
        let rows = rows
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > 400 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut out = Vec::new();
        for (id, title, project, model) in rows {
            let user_sql = if v2 {
                "SELECT id,time_created,seq FROM session_message WHERE session_id=?1 AND type='user' AND coalesce(json_extract(data,'$.synthetic'),0)=0 ORDER BY seq DESC LIMIT 1"
            } else {
                "SELECT id,json_extract(data,'$.time.created'),time_created FROM message WHERE session_id=?1 AND json_extract(data,'$.role')='user' AND coalesce(json_extract(data,'$.synthetic'),0)=0 ORDER BY time_created DESC,id DESC LIMIT 1"
            };
            let user: Option<(String, u64, i64)> = tx
                .query_row(user_sql, [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?;
            let Some((turn, started, sequence)) = user else {
                continue;
            };
            let latest_sql = if v2 {
                "SELECT type,data,time_updated FROM session_message WHERE session_id=?1 AND seq>=?2 AND type IN ('user','assistant','idle') ORDER BY seq DESC LIMIT 1"
            } else {
                "SELECT json_extract(data,'$.role'),data,time_updated FROM message WHERE session_id=?1 AND time_created>=?2 ORDER BY time_created DESC,id DESC LIMIT 1"
            };
            let latest: Option<(String, String, u64)> = tx
                .query_row(latest_sql, rusqlite::params![id, sequence], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            let Some((kind, json, updated)) = latest else {
                continue;
            };
            let data: Value =
                serde_json::from_str(&json).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let activity = data
                .pointer("/time/completed")
                .and_then(Value::as_u64)
                .or_else(|| data.pointer("/time/created").and_then(Value::as_u64))
                .unwrap_or(started)
                .max(started);
            // v2 time_updated is only for messages, not session title/config edits.
            let activity = if v2 { activity.max(updated) } else { activity };
            let Some(mut task) = native_sessions::task(
                ("opencode", "OpenCode"),
                &id,
                &title,
                &project,
                &turn,
                started,
                activity,
            ) else {
                continue;
            };
            task.model = data
                .pointer("/model/id")
                .or_else(|| data.get("modelID"))
                .and_then(Value::as_str)
                .and_then(super::reported_model)
                .or_else(|| {
                    model
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .and_then(|v| {
                            v.get("id")
                                .or_else(|| v.get("modelID"))
                                .and_then(Value::as_str)
                                .and_then(super::reported_model)
                        })
                });
            let outcome = if v2 && kind == "idle" {
                Some(
                    data.get("outcome")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                )
            } else {
                None
            };
            if let Some(outcome) = outcome {
                match outcome {
                    "completed" | "success" | "succeeded" => task.status = TaskStatus::Succeeded,
                    "failed" | "error" => task.status = TaskStatus::Failed,
                    "interrupted" | "cancelled" | "canceled" => {
                        task.status = TaskStatus::WaitingInput;
                        task.waiting_reason = Some(WaitingReason::TurnEnded);
                        task.outcome = Some("interrupted".into());
                    }
                    _ => {
                        task.status = TaskStatus::Unknown;
                        task.stale = true;
                    }
                }
            } else if !v2
                && kind == "assistant"
                && data.get("parentID").and_then(Value::as_str) == Some(turn.as_str())
                && data
                    .pointer("/time/completed")
                    .and_then(Value::as_u64)
                    .is_some()
            {
                if data.get("error").is_some_and(|e| !e.is_null()) {
                    task.status = TaskStatus::Failed;
                } else if data
                    .get("finish")
                    .and_then(Value::as_str)
                    .is_some_and(|f| matches!(f, "stop" | "end_turn"))
                {
                    task.status = TaskStatus::Succeeded;
                }
            }
            if task.status == TaskStatus::Running {
                // v2 embeds streaming parts in the assistant message itself.
                let content = data.get("content").and_then(Value::as_array);
                task.phase = Some(
                    if content.is_some_and(|c| {
                        c.iter().any(|p| {
                            p.get("type") == Some(&Value::String("tool".into()))
                                && matches!(
                                    p.pointer("/state/status").and_then(Value::as_str),
                                    Some("running" | "pending")
                                )
                        })
                    }) {
                        ExecutionPhase::Tool
                    } else if content.is_some_and(|c| {
                        c.iter()
                            .any(|p| p.get("type").and_then(Value::as_str) == Some("reasoning"))
                    }) {
                        ExecutionPhase::Thinking
                    } else {
                        ExecutionPhase::AwaitingModel
                    },
                );
            } else if !task.stale {
                task.finished_at = Some(task.updated_at);
            }
            out.push(task);
        }
        Ok(out)
    }
}
impl Provider for OpenCodeProvider {
    fn name(&self) -> &'static str {
        "opencode"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match self.read() {
            Ok(tasks) => self.sessions.observe(tasks, now),
            Err(_) => self
                .sessions
                .degraded("OpenCode 数据格式变化、读取受限或超过预算；保留上次任务状态"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, Connection, OpenCodeProvider) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY,title TEXT,slug TEXT,directory TEXT,model TEXT,time_archived INTEGER,time_updated INTEGER);CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,seq INTEGER,time_created INTEGER,time_updated INTEGER,data TEXT);
        INSERT INTO session_v2 VALUES('s','Actual native title','native-slug','C:/project','{\"id\":\"source-model\"}',NULL,100000);").unwrap();
        let provider = OpenCodeProvider::new(path);
        (dir, db, provider)
    }
    fn message(db: &Connection, id: &str, kind: &str, seq: i64, time: u64, data: Value) {
        db.execute(
            "INSERT INTO session_message VALUES(?1,'s',?2,?3,?4,?4,?5)",
            rusqlite::params![id, kind, seq, time, data.to_string()],
        )
        .unwrap();
    }
    #[test]
    fn v2_busy_tools_and_older_idle_cannot_finish_the_current_turn() {
        let (_dir, db, mut p) = fixture();
        message(
            &db,
            "u1",
            "user",
            1,
            100000,
            json!({"time":{"created":100000}}),
        );
        message(
            &db,
            "idle1",
            "idle",
            2,
            101000,
            json!({"time":{"created":101000},"outcome":"failed"}),
        );
        message(
            &db,
            "u2",
            "user",
            3,
            102000,
            json!({"time":{"created":102000}}),
        );
        message(
            &db,
            "a2",
            "assistant",
            4,
            103000,
            json!({"time":{"created":103000,"completed":103500},"finish":"tool-calls","content":[{"type":"tool","state":{"status":"running"}}]}),
        );
        let rows = p.read().unwrap();
        assert_eq!(rows[0].status, TaskStatus::Running);
        assert_eq!(rows[0].turn_key.as_deref(), Some("u2"));
        assert_eq!(rows[0].phase, Some(ExecutionPhase::Tool));
        assert_eq!(rows[0].model.as_deref(), Some("source-model"));
        p.sessions.resume("opencode", rows);
        message(
            &db,
            "idle2",
            "idle",
            5,
            104000,
            json!({"time":{"created":104000},"outcome":"succeeded"}),
        );
        message(
            &db,
            "config",
            "model-switched",
            6,
            105000,
            json!({"time":{"created":105000}}),
        );
        let ended = p.poll(native_sessions::time(106000).unwrap());
        assert_eq!(ended.tasks.len(), 1);
        assert_eq!(ended.tasks[0].status, TaskStatus::Succeeded);
        assert_eq!(ended.tasks[0].turn_key.as_deref(), Some("u2"));
        db.execute(
            "UPDATE session_v2 SET title='Renamed',time_updated=900000",
            [],
        )
        .unwrap();
        let renamed = p.poll(native_sessions::time(900000).unwrap());
        assert_eq!(renamed.tasks[0].name, "Renamed");
        assert!(!renamed.tasks[0].new_execution);
        assert_eq!(renamed.tasks[0].updated_at, ended.tasks[0].updated_at);
    }
    #[test]
    fn idle_history_is_catalog_only_unknown_outcomes_and_io_failures_do_not_invent_success() {
        let (_dir, db, mut p) = fixture();
        message(
            &db,
            "u",
            "user",
            1,
            100000,
            json!({"time":{"created":100000}}),
        );
        message(
            &db,
            "idle",
            "idle",
            2,
            101000,
            json!({"time":{"created":101000},"outcome":"succeeded"}),
        );
        let historical = p.poll(native_sessions::time(200000).unwrap());
        assert!(historical.tasks.is_empty());
        assert_eq!(historical.catalog.len(), 1);
        p.resume(historical.catalog);
        db.execute(
            "UPDATE session_message SET data='{\"time\":{\"created\":101000}}' WHERE id='idle'",
            [],
        )
        .unwrap();
        let missing = p.poll(native_sessions::time(200001).unwrap());
        assert_eq!(missing.tasks[0].status, TaskStatus::Unknown);
        assert!(missing.tasks[0].finished_at.is_none());

        db.execute("UPDATE session_message SET data='{\"outcome\":\"unknown\",\"time\":{\"created\":101000}}' WHERE id='idle'",[]).unwrap();
        let uncertain = p.poll(native_sessions::time(201000).unwrap());
        assert_eq!(uncertain.tasks[0].status, TaskStatus::Unknown);
        assert!(uncertain.tasks[0].finished_at.is_none());
        db.execute_batch("DROP TABLE session_message").unwrap();
        let degraded = p.poll(native_sessions::time(202000).unwrap());
        assert_eq!(degraded.tasks[0].status, TaskStatus::Unknown);
        assert!(degraded.tasks[0].stale);
    }
    #[test]
    fn legacy_message_finish_requires_current_parent_and_an_explicit_final_finish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE session(id TEXT,title TEXT,directory TEXT,time_archived INTEGER,time_updated INTEGER);CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);INSERT INTO session VALUES('s','Old format','/project',NULL,100000);INSERT INTO message VALUES('u','s',100000,100000,'{\"role\":\"user\",\"time\":{\"created\":100000}}');").unwrap();
        let p = OpenCodeProvider::new(path);
        for (parent, finish, status) in [
            ("other", "stop", TaskStatus::Running),
            ("u", "tool-calls", TaskStatus::Running),
            ("u", "stop", TaskStatus::Succeeded),
        ] {
            db.execute("DELETE FROM message WHERE id='a'", []).unwrap();
            db.execute("INSERT INTO message VALUES('a','s',101000,102000,?1)",[json!({"role":"assistant","parentID":parent,"time":{"created":101000,"completed":102000},"finish":finish}).to_string()]).unwrap();
            assert_eq!(p.read().unwrap()[0].status, status);
        }
    }
    #[test]
    #[ignore = "read-only real local OpenCode database"]
    fn live_opencode_snapshot() {
        let p = OpenCodeProvider::new(default_db());
        let rows = p.read().expect("native database must be readable");
        assert!(!rows.is_empty(), "run one real OpenCode task first");
        assert!(rows
            .iter()
            .all(|t| t.agent_id.as_deref() == Some("opencode")
                && t.turn_key.is_some()
                && t.started_at.is_some()));
        println!(
            "OpenCode native rows={} running={} ended={} models={}",
            rows.len(),
            rows.iter()
                .filter(|t| t.status == TaskStatus::Running)
                .count(),
            rows.iter().filter(|t| t.finished_at.is_some()).count(),
            rows.iter().filter(|t| t.model.is_some()).count()
        );
    }
}
