//! Qoder IDE/CLI JSONL. A blockable Stop or an assistant's model response is
//! not a final query result. Unknown or incomplete journals remain uncertain.
use super::{
    jsonl_tail::JsonlTail,
    kimi::{stamp, text},
    native_sessions::{self, BatchRead, DirectorySweep},
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
    unfinished: bool,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            tail: JsonlTail::newline_committed(),
            task: None,
            cwd: String::new(),
            invalid: false,
            unfinished: false,
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
struct TranscriptPage {
    sweep: DirectorySweep,
    pending: Vec<PathBuf>,
    status: BatchRead,
}
#[derive(Default)]
pub(super) struct Transcripts {
    logs: HashMap<PathBuf, Journal>,
    pages: HashMap<PathBuf, TranscriptPage>,
}
impl Transcripts {
    pub fn read(
        &mut self,
        root: &Path,
        protected: &HashSet<String>,
    ) -> (Vec<TaskSnapshot>, BatchRead) {
        let began = Instant::now();
        let projects = root.join("projects");
        let page = self.pages.entry(projects.clone()).or_default();
        let is_protected = |path: &Path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|id| protected.contains(id))
        };
        if page.pending.is_empty() {
            (page.pending, page.status) = page.sweep.page(
                std::slice::from_ref(&projects),
                768,
                began,
                |entry, depth| {
                    let Ok(kind) = entry.file_type() else {
                        return (None, None);
                    };
                    if depth == 1 && kind.is_dir() {
                        let sub = entry.path().join("transcript");
                        let dir = if sub.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                            sub
                        } else {
                            entry.path()
                        };
                        (Some(dir), None)
                    } else if depth == 2
                        && kind.is_file()
                        && entry.path().extension().is_some_and(|s| s == "jsonl")
                    {
                        (None, Some(entry.path()))
                    } else {
                        (None, None)
                    }
                },
            );
            if page.pending.len() > 128 {
                page.status = page.status.max(BatchRead::Paginating);
            }
            page.pending.sort_by_cached_key(|p| {
                (
                    is_protected(p),
                    p.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH),
                )
            });
        }
        let mut files: Vec<_> = page.pending.iter().rev().take(128).cloned().collect();
        for (path, journal) in &self.logs {
            if path.starts_with(&projects)
                && (journal.unfinished || is_protected(path))
                && !files.contains(path)
            {
                files.push(path.clone());
            }
        }
        files.sort_by_cached_key(|path| {
            std::cmp::Reverse((
                self.logs
                    .get(path)
                    .is_some_and(|journal| journal.unfinished),
                is_protected(path),
                path.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH),
            ))
        });
        let mut rows = vec![];
        let mut bytes = 0;
        let mut status = page.status;
        let mut visited = HashSet::new();
        let mut unfinished = self.logs.values().filter(|log| log.unfinished).count();
        for path in files {
            if bytes >= 8 * 1024 * 1024 || began.elapsed() > Duration::from_millis(500) {
                status = status.max(BatchRead::Paginating);
                break;
            }
            let id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if unfinished >= 8 && !self.logs.get(&path).is_some_and(|log| log.unfinished) {
                status = status.max(BatchRead::Paginating);
                continue;
            }
            visited.insert(path.clone());
            let journal = self.logs.entry(path.clone()).or_default();
            let (caught_up, gap) = loop {
                let Ok(batch) = journal.tail.read(&path) else {
                    break (false, true);
                };
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
            unfinished -= usize::from(journal.unfinished);
            journal.unfinished = !caught_up && !gap && !journal.invalid;
            unfinished += usize::from(journal.unfinished);
            if gap || journal.invalid {
                status = BatchRead::Failed;
                continue;
            }
            if !caught_up {
                status = status.max(BatchRead::Paginating);
                continue;
            }
            if let Some(task) = &journal.task {
                let mut task = task.clone();
                task.native_storage = Some(native_sessions::storage_key(&path));
                rows.push(task);
            }
        }
        page.pending.retain(|path| !visited.contains(path));
        if !page.pending.is_empty() {
            status = status.max(BatchRead::Paginating);
        }
        self.logs.retain(|p, journal| {
            !p.starts_with(&projects)
                || visited.contains(p)
                || journal.unfinished
                || is_protected(p)
        });
        (rows, status)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn transcript_larger_than_one_poll_survives_the_history_page() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects/fixture/transcript");
        std::fs::create_dir_all(&project).unwrap();
        for index in 0..128 {
            let file =
                std::fs::File::create(project.join(format!("history-{index:03}.jsonl"))).unwrap();
            file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
                .unwrap();
        }
        let active = project.join("active.jsonl");
        let mut file = std::fs::File::create(&active).unwrap();
        writeln!(file, "{}", json!({"type":"user","sessionId":"active","uuid":"u","timestamp":"2026-11-07T00:00:00Z","message":{"content":"Fixture"}})).unwrap();
        let progress = json!({"type":"progress","data":"x".repeat(128 * 1024)}).to_string();
        for _ in 0..72 {
            writeln!(file, "{progress}").unwrap();
        }
        writeln!(
            file,
            "{}",
            json!({"type":"result","sessionId":"active","subtype":"success","duration_ms":61000})
        )
        .unwrap();
        drop(file);

        let mut reader = Transcripts::default();
        let protected = HashSet::new();
        let (first, status) = reader.read(dir.path(), &protected);
        assert!(status == BatchRead::Paginating && first.is_empty());
        let mut finished = false;
        for _ in 0..4 {
            let (rows, _) = reader.read(dir.path(), &protected);
            finished |= rows.iter().any(|task| {
                task.session_id.as_deref() == Some("active")
                    && task.waiting_reason == Some(WaitingReason::TurnEnded)
            });
        }
        assert!(
            finished,
            "The next page must retain and resume the unfinished transcript"
        );
    }

    #[test]
    fn malformed_transcript_reports_failure_instead_of_pagination() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects/fixture");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("bad.jsonl"), b"{broken json}\n").unwrap();
        let (rows, status) = Transcripts::default().read(dir.path(), &HashSet::new());
        assert!(rows.is_empty() && status == BatchRead::Failed);
    }

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
