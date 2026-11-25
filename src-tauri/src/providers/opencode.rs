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
struct ModelSelection {
    id: String,
    variant: Option<String>,
}

impl ModelSelection {
    fn from_value(value: &Value, id_key: &str) -> Option<Self> {
        let field = |key: &str| value.get(key)?.as_str().and_then(super::reported_model);
        Some(Self {
            id: field(id_key)?,
            variant: field("variant"),
        })
    }

    fn from_json(json: Option<&str>, id_key: &str) -> Option<Self> {
        Self::from_value(&serde_json::from_str(json?).ok()?, id_key)
    }
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
                "SELECT id,time_created,seq,CAST(json_extract(data,'$.metadata.model') AS TEXT)
                 FROM session_message WHERE session_id=?1 AND type='user'
                   AND coalesce(json_extract(data,'$.synthetic'),0)=0 ORDER BY seq DESC LIMIT 1"
            } else {
                "SELECT id,json_extract(data,'$.time.created'),time_created,
                        json_object('modelID',json_extract(data,'$.model.modelID'),
                                    'variant',coalesce(json_extract(data,'$.variant'),json_extract(data,'$.model.variant')))
                 FROM message WHERE session_id=?1 AND json_extract(data,'$.role')='user'
                   AND coalesce(json_extract(data,'$.synthetic'),0)=0 ORDER BY time_created DESC,id DESC LIMIT 1"
            };
            let user: Option<(String, u64, i64, Option<String>)> = tx
                .query_row(user_sql, [&id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()?;
            let Some((turn, started, sequence, selected_model)) = user else {
                continue;
            };
            let latest_sql = if v2 {
                "SELECT type,data,CASE WHEN type='idle' THEN time_created ELSE time_updated END
                 FROM session_message WHERE session_id=?1 AND seq>=?2
                   AND (id=?3 OR type IN ('assistant','idle')) ORDER BY seq DESC LIMIT 1"
            } else {
                "SELECT json_extract(data,'$.role'),data,time_updated FROM message
                 WHERE session_id=?1 AND time_created>=?2 AND (id=?3 OR (json_extract(data,'$.role')='assistant'
                   AND json_extract(data,'$.parentID')=?3)) ORDER BY time_created DESC,id DESC LIMIT 1"
            };
            let latest: Option<(String, String, u64)> = tx
                .query_row(latest_sql, rusqlite::params![id, sequence, turn], |r| {
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
            // For idle records the query uses creation time, so later storage edits cannot extend a result.
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
            // An idle row has no model. Read only the model object from the
            // last assistant in this turn, even on the first poll after restart.
            let response_model = match (v2, kind.as_str()) {
                (true, "idle") => {
                    let last: Option<String> = tx.query_row(
                        "SELECT CAST(json_extract(data,'$.model') AS TEXT) FROM session_message
                         WHERE session_id=?1 AND seq>=?2 AND type='assistant' ORDER BY seq DESC LIMIT 1",
                        rusqlite::params![id, sequence],
                        |r| r.get(0),
                    ).optional()?.flatten();
                    ModelSelection::from_json(last.as_deref(), "id")
                }
                (true, "assistant") => data
                    .get("model")
                    .and_then(|m| ModelSelection::from_value(m, "id")),
                (false, "assistant") => ModelSelection::from_value(&data, "modelID"),
                _ => None,
            };
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
            // Session settings can change after a result, independently of
            // the execution. They are a fallback for an active turn only.
            let selected = response_model
                .or_else(|| ModelSelection::from_json(selected_model.as_deref(), "modelID"))
                .or_else(|| {
                    let config = model
                        .as_deref()
                        .filter(|_| task.status == TaskStatus::Running)?;
                    let value = serde_json::from_str(config).ok()?;
                    ModelSelection::from_value(&value, "id")
                        .or_else(|| ModelSelection::from_value(&value, "modelID"))
                });
            if let Some(selection) = selected {
                task.model = Some(selection.id);
                task.model_variant = selection.variant;
            }
            if task.status == TaskStatus::Running {
                // v2 embeds streaming parts in the assistant message itself.
                let content = data.get("content").and_then(Value::as_array);
                task.phase = Some(
                    if content.is_some_and(|c| {
                        c.iter().any(|p| {
                            p.get("type").and_then(Value::as_str) == Some("tool")
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
mod tests;
