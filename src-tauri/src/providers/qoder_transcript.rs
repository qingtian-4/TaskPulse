//! Qoder IDE/CLI JSONL. A blockable Stop or an assistant's model response is
//! not a final query result. Unknown or incomplete journals remain uncertain.
use super::{
    jsonl_tail::JsonlTail,
    kimi::{stamp, text},
    native_sessions,
};
use crate::domain::{ExecutionPhase, TaskSnapshot, TaskStatus, WaitingReason};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

struct Journal {
    tail: JsonlTail,
    task: Option<TaskSnapshot>,
    cwd: String,
    invalid: bool,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            tail: JsonlTail::newline_committed(),
            task: None,
            cwd: String::new(),
            invalid: false,
        }
    }
}
impl Journal {
    fn apply(&mut self, v: &Value, id: &str) -> Result<(), String> {
        if text(v, "sessionId")
            .or_else(|| text(v, "session_id"))
            .is_some_and(|s| s != id)
        {
            return Err("session_mismatch".into());
        }
        if v.get("parent_tool_use_id").is_some_and(|v| !v.is_null())
            || v.get("isSidechain") == Some(&Value::Bool(true))
        {
            return Ok(());
        }
        let kind = text(v, "type").unwrap_or("");
        let at = v.get("timestamp").and_then(stamp);
        if let Some(cwd) = text(v, "cwd") {
            self.cwd = cwd.into();
        }
        if kind == "user" && v.pointer("/message/content").is_some_and(Value::is_string) {
            let at = at.ok_or("user_timestamp")?;
            let ms = at
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_err(|_| "user_timestamp")?
                .as_millis() as u64;
            let turn = text(v, "uuid").ok_or("user_id")?;
            let title = v
                .pointer("/message/content")
                .and_then(Value::as_str)
                .unwrap_or("Qoder");
            self.task = native_sessions::task(
                ("qoder-transcript", "Qoder IDE / CLI"),
                id,
                &title.chars().take(72).collect::<String>(),
                &self.cwd,
                turn,
                ms,
                ms,
            );
        }
        let Some(task) = self.task.as_mut() else {
            return Ok(());
        };
        if let Some(at) = at {
            task.updated_at = task.updated_at.max(at);
        }
        if let Some(model) = v
            .pointer("/message/model")
            .and_then(Value::as_str)
            .and_then(super::reported_model)
        {
            task.model = Some(model);
        }
        if kind == "assistant" && task.finished_at.is_none() {
            let tool = v
                .pointer("/message/content")
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|p| text(p, "type") == Some("tool_use")));
            task.phase = Some(if tool {
                ExecutionPhase::Tool
            } else {
                ExecutionPhase::Responding
            });
        }
        if kind == "result" {
            if text(v, "turn_id").is_some_and(|t| Some(t) != task.turn_key.as_deref()) {
                return Err("result_turn_mismatch".into());
            }
            let status = text(v, "subtype").ok_or("result_subtype")?;
            if status != "success" && !status.starts_with("error_") && status != "interrupted" {
                return Err("result_subtype".into());
            }
            task.status =
                if v.get("is_error") == Some(&Value::Bool(true)) || status.starts_with("error_") {
                    TaskStatus::Failed
                } else {
                    TaskStatus::WaitingInput
                };
            task.waiting_reason =
                (task.status == TaskStatus::WaitingInput).then_some(WaitingReason::TurnEnded);
            task.finished_at = at.or_else(|| {
                task.started_at?
                    .checked_add(Duration::from_millis(v.get("duration_ms")?.as_u64()?))
            });
            if task.finished_at.is_none() {
                return Err("result_timestamp".into());
            }
            task.updated_at = task.finished_at.unwrap();
            task.phase = None;
            if status == "interrupted" {
                task.outcome = Some("interrupted".into());
            }
        }
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Transcripts {
    logs: HashMap<PathBuf, Journal>,
}
impl Transcripts {
    pub fn read(&mut self, root: &Path) -> Result<Vec<TaskSnapshot>, String> {
        let began = Instant::now();
        let mut files = vec![];
        for p in std::fs::read_dir(root.join("projects")).map_err(|_| "project_listing")? {
            let p = p.map_err(|_| "project_listing")?;
            if !p.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                continue;
            }
            let sub = p.path().join("transcript");
            let sub = if sub.is_dir() { sub } else { p.path() };
            for f in std::fs::read_dir(sub).map_err(|_| "transcript_listing")? {
                let f = f.map_err(|_| "transcript_listing")?;
                if f.file_type().is_ok_and(|t| t.is_file() && !t.is_symlink())
                    && f.path().extension().is_some_and(|s| s == "jsonl")
                {
                    files.push(f.path());
                    if files.len() > 400 {
                        return Err("transcript_admission_budget".into());
                    }
                }
            }
        }
        files.sort();
        let mut rows = vec![];
        let mut bytes = 0;
        let mut partial = false;
        let mut live = HashSet::new();
        for path in files {
            live.insert(path.clone());
            if bytes >= 8 * 1024 * 1024 || began.elapsed() > Duration::from_millis(500) {
                partial = true;
                break;
            }
            let id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if !self.logs.contains_key(&path)
                && self
                    .logs
                    .values()
                    .filter(|l| l.tail.has_partial_line())
                    .count()
                    >= 8
            {
                partial = true;
                continue;
            }
            let journal = self.logs.entry(path.clone()).or_default();
            let (caught_up, gap) = loop {
                let batch = journal.tail.read(&path).map_err(|_| "transcript_read")?;
                bytes += batch.bytes_read;
                if batch.reset {
                    journal.task = None;
                    journal.invalid = false;
                    journal.cwd.clear();
                }
                for line in batch.body.lines() {
                    match serde_json::from_str::<Value>(line) {
                        Ok(v) => {
                            if journal.apply(&v, &id).is_err() {
                                journal.invalid = true;
                            }
                        }
                        Err(_) => journal.invalid = true,
                    }
                }
                journal.tail.trim_read_buffer();
                if batch.caught_up
                    || batch.bytes_read == 0
                    || bytes >= 8 * 1024 * 1024
                    || began.elapsed() > Duration::from_millis(500)
                {
                    break (batch.caught_up, batch.gap);
                }
            };
            if !caught_up || gap || journal.invalid {
                partial = true;
                continue;
            }
            if let Some(task) = &journal.task {
                rows.push(task.clone());
            }
        }
        if partial {
            return Err("transcript_catching_up_or_unsupported".into());
        }
        self.logs.retain(|p, _| live.contains(p));
        Ok(rows)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn assistant_end_and_stop_hooks_cannot_finish_query() {
        let mut p = Journal::default();
        p.apply(&json!({"type":"user","sessionId":"s","uuid":"u","timestamp":"2026-11-07T00:00:00Z","message":{"content":"Fixture"}}),"s").unwrap();
        for v in [
            json!({"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}),
            json!({"type":"progress","data":{"hookEvent":"Stop"}}),
        ] {
            p.apply(&v, "s").unwrap();
            assert!(p.task.as_ref().unwrap().finished_at.is_none());
        }
        p.apply(
            &json!({"type":"result","session_id":"s","subtype":"success","duration_ms":61000}),
            "s",
        )
        .unwrap();
        assert_eq!(
            p.task.unwrap().waiting_reason,
            Some(WaitingReason::TurnEnded)
        );
    }
}
