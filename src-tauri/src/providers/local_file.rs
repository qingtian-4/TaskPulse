//! Local file provider — the AgentTray `hook > wrapper > scan` reliability
//! layer, preserved.
//!
//! Reads status files from two directories:
//! * `~/.agent-monitor/` — legacy AgentTray v1 JSON / pipe files (compat).
//! * `~/.taskpulse/` — v2 files with `started_at` / `finished_at` /
//!   `exit_code` / `confidence` (new recommended location).
//!
//! Wrappers and hooks must write atomically (tmp file + rename) so a watcher
//! never observes a half-written JSON blob; see `scripts/wrap.ps1` and
//! `scripts/hooks/`.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus, TerminalInfo};
use crate::providers::{Provider, ProviderOutput};

/// How long a legacy status file can sit untouched before we hide it
/// (unchanged from AgentTray).
const STALE_TTL: Duration = Duration::from_secs(10 * 60);
/// Same rule for v2 files that never reported a finish.
const V2_STALE_TTL: Duration = Duration::from_secs(30 * 60);

const STATUS_FILE_BYTES: u64 = 256 * 1024;
const STATUS_ROUND_BYTES: u64 = 4 * 1024 * 1024;
const STATUS_ROUND_ENTRIES: usize = 4096;

pub struct LocalFileProvider {
    dirs: Vec<PathBuf>,
    last_tasks: Vec<TaskSnapshot>,
}

impl LocalFileProvider {
    pub fn new() -> Self {
        if crate::runtime::isolated() {
            let root = crate::providers::remote_tmux::config::config_dir();
            return Self {
                dirs: vec![root.join("legacy-status"), root],
                last_tasks: vec![],
            };
        }
        let home = dirs_next::home_dir();
        let mut dirs = Vec::new();
        if let Some(h) = &home {
            dirs.push(h.join(".agent-monitor"));
            dirs.push(h.join(".taskpulse"));
        }
        Self {
            dirs,
            last_tasks: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn status_dirs(&self) -> Vec<PathBuf> {
        self.dirs.clone()
    }
}

impl Default for LocalFileProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for LocalFileProvider {
    fn name(&self) -> &'static str {
        "local_file"
    }

    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let mut round = StatusRound::default();
        let mut enumeration_incomplete = false;
        for dir in &self.dirs {
            if let Err(error) = round.collect(dir, now) {
                enumeration_incomplete = true;
                round.problem.get_or_insert(error);
            }
        }
        if enumeration_incomplete {
            // We may have missed a higher-priority same-ID file. Do not let
            // a partial directory listing advance any lifecycle.
            round.tasks.clear();
        }
        // A damaged copy cannot be replaced by a same-ID observation from
        // the other compatibility directory in this incomplete round.
        round
            .tasks
            .retain(|task| !round.failed_ids.contains(&task.id));
        // The last configured directory has precedence for a shared legacy ID.
        let mut seen = std::collections::HashSet::new();
        round.tasks.reverse();
        round.tasks.retain(|task| seen.insert(task.id.clone()));
        round.tasks.reverse();
        if let Some(error) = round.problem {
            // The store independently retains missing tasks on partial
            // rounds. Cache only a bounded working set here.
            let current: std::collections::HashSet<_> =
                round.tasks.iter().map(|task| task.id.as_str()).collect();
            round.tasks.extend(
                self.last_tasks
                    .iter()
                    .filter(|task| !current.contains(task.id.as_str()))
                    .take(STATUS_ROUND_ENTRIES)
                    .cloned()
                    .map(|mut task| {
                        task.stale = true;
                        task
                    })
                    .collect::<Vec<_>>(),
            );
            self.last_tasks = round
                .tasks
                .iter()
                .take(STATUS_ROUND_ENTRIES)
                .cloned()
                .collect();
            ProviderOutput::degraded(round.tasks, error.to_string())
        } else {
            self.last_tasks = round.tasks.clone();
            ProviderOutput::tasks(round.tasks)
        }
    }
}

struct StatusRound {
    remaining_bytes: u64,
    remaining_entries: usize,
    tasks: Vec<TaskSnapshot>,
    failed_ids: std::collections::HashSet<String>,
    problem: Option<io::Error>,
}

impl Default for StatusRound {
    fn default() -> Self {
        Self {
            remaining_bytes: STATUS_ROUND_BYTES,
            remaining_entries: STATUS_ROUND_ENTRIES,
            tasks: Vec::new(),
            failed_ids: Default::default(),
            problem: None,
        }
    }
}

fn invalid_status() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid compatibility status record",
    )
}

fn status_id(path: &Path) -> io::Result<String> {
    Ok(format!(
        "file:{}",
        path.file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(invalid_status)?
    ))
}

fn read_status(path: &Path, remaining: &mut u64) -> io::Result<(TaskSnapshot, SystemTime)> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err(invalid_status());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > STATUS_FILE_BYTES {
        return Err(io::Error::other(
            "Compatibility status file exceeds 256 KiB or is not a regular file",
        ));
    }
    if before.len() >= *remaining {
        return Err(io::Error::other(
            "Compatibility status read budget exhausted",
        ));
    }
    let mut reader = file.take((STATUS_FILE_BYTES + 1).min(*remaining));
    let mut bytes = Vec::new();
    let read = reader.read_to_end(&mut bytes);
    *remaining -= bytes.len() as u64;
    read?;
    let after = reader.get_ref().metadata()?;
    let mtime = before.modified()?;
    if bytes.len() as u64 > STATUS_FILE_BYTES
        || bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || after.modified()? != mtime
    {
        return Err(io::Error::other(
            "Compatibility status file changed during read or exceeds 256 KiB",
        ));
    }
    parse_status(
        path,
        std::str::from_utf8(&bytes).map_err(|_| invalid_status())?,
        mtime,
    )
    .map(|task| (task, mtime))
    .ok_or_else(invalid_status)
}

#[cfg(test)]
fn parse_status_file(path: &Path, _now: SystemTime) -> Option<TaskSnapshot> {
    let mut budget = STATUS_ROUND_BYTES;
    read_status(path, &mut budget).ok().map(|(task, _)| task)
}

fn parse_status(path: &Path, content: &str, mtime: SystemTime) -> Option<TaskSnapshot> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    let name = path.file_stem()?.to_str()?.to_string();

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        // Version gate: v1 (AgentTray) or v2 (TaskPulse); anything else skips.
        let version = val.get("v").map_or(Some(1), |v| v.as_u64())?;
        if version != 1 && version != 2 {
            return None;
        }

        let status_str = val.get("status")?.as_str()?;
        let status = checked_status(status_str)?;
        let waiting_reason = match val.get("waiting_reason") {
            Some(value) => Some(serde_json::from_value(value.clone()).ok()?),
            None => None,
        };
        let input_request_id = match val.get("input_request_id") {
            Some(value) => Some(value.as_str()?.to_owned()),
            None => None,
        };
        let message = val
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .chars()
            .take(500)
            .collect::<String>();

        let terminal: Option<TerminalInfo> = val
            .get("terminal")
            .and_then(|t| serde_json::from_value(t.clone()).ok());

        let can_focus = terminal
            .as_ref()
            .map(|t| t.kind != "windows_native" && !t.focus_id.is_empty() && t.focus_id != "0")
            .unwrap_or(false);

        // v2-only fields
        let started_at = status_time(val.get("started_at"))?;
        let finished_at = status_time(val.get("finished_at"))?;
        let activity_at = status_time(val.get("execution_activity_at"))?;
        let exit_code = val
            .get("exit_code")
            .and_then(|v| v.as_i64())
            .map(|c| c as i32);

        // Confidence: explicit v2 field wins; else an exit code is EXACT;
        // else a file write is HOOK (deterministic, not inference).
        let confidence = match val.get("confidence").and_then(|v| v.as_str()) {
            Some("exact") => DetectionConfidence::Exact,
            Some("hook") => DetectionConfidence::Hook,
            Some("inferred") => DetectionConfidence::Inferred,
            _ if exit_code.is_some() => DetectionConfidence::Exact,
            _ => DetectionConfidence::Hook,
        };

        let cwd_project = val
            .get("cwd")
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .and_then(|cwd| Path::new(cwd).file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        let display_name = val
            .get("name")
            .and_then(|name| name.as_str())
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| match (cwd_project.as_deref(), terminal.as_ref()) {
                (Some(proj), Some(t)) if !t.label.is_empty() => format!("{proj} · {}", t.label),
                (Some(proj), _) => proj.to_string(),
                _ => name.clone(),
            });

        let mut snapshot = TaskSnapshot {
            integration_epoch: None,
            integration_sequence: 0,
            agent_id: None,
            agent_name: None,
            evidence_channel: None,
            native_storage: None,
            outcome: None,
            pending_confirmed_at: None,
            source_instance: None,
            model: None,
            reasoning_effort: None,
            internal: false,
            project_path: None,
            tmux_session: None,
            tmux_pane: None,
            tmux_target: None,
            waiting_reason: None,
            input_request_id: None,
            input_request_ids: vec![],
            attention_since: 0,
            new_execution: false,
            id: format!("file:{name}"),
            name: display_name,
            source: TaskSource::LocalFile,
            status,
            host: None,
            project: cwd_project,
            message: (!message.is_empty()).then_some(message),
            activity: val
                .get("activity")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            phase: None,
            started_at: started_at.or(Some(mtime)),
            updated_at: activity_at.or(finished_at).unwrap_or(mtime),
            finished_at,
            pid: val
                .get("pid")
                .and_then(|v| v.as_u64())
                .and_then(|p| u32::try_from(p).ok()),
            exit_code,
            confidence,
            remote: false,
            stale: false,
            recent: false,
            unread: false,
            pinned: false,
            turn_key: val
                .get("turn_key")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            native_unread: None,
            terminal,
            can_focus,
            cpu: None,
            cli: val.get("cli").and_then(|s| s.as_str()).map(String::from),
            session_id: val
                .get("session_id")
                .and_then(|s| s.as_str())
                .map(String::from),
            hook_event: val
                .get("hook_event")
                .and_then(|s| s.as_str())
                .map(String::from),
            hook_matcher: val
                .get("hook_matcher")
                .and_then(|s| s.as_str())
                .map(String::from),
            quota_5h_used_pct: None,
            quota_5h_resets_at: None,
            quota_1w_used_pct: None,
            quota_1w_resets_at: None,
        };
        snapshot.project_path = val.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
        snapshot.source = match snapshot.cli.as_deref() {
            Some("codex") => TaskSource::Codex,
            Some("hermes") => TaskSource::Hermes,
            Some("dsh") => TaskSource::DeepSeekHarness,
            Some("claude-code") => TaskSource::ClaudeCode,
            Some("gemini") => TaskSource::Gemini,
            _ => TaskSource::LocalFile,
        };
        if snapshot.status == TaskStatus::WaitingInput {
            snapshot.waiting_reason =
                waiting_reason.or(Some(crate::domain::WaitingReason::Unknown));
            snapshot.input_request_id = input_request_id;
        }
        Some(snapshot)
    } else {
        // Legacy pipe format: "status|message"
        let (status, message) = match trimmed.split_once('|') {
            Some((s, m)) => (checked_status(s)?, Some(m.to_string())),
            None => (checked_status(trimmed)?, None),
        };
        Some(TaskSnapshot {
            integration_epoch: None,
            integration_sequence: 0,
            agent_id: None,
            agent_name: None,
            evidence_channel: None,
            native_storage: None,
            outcome: None,
            pending_confirmed_at: None,
            source_instance: None,
            model: None,
            reasoning_effort: None,
            internal: false,
            project_path: None,
            tmux_session: None,
            tmux_pane: None,
            tmux_target: None,
            waiting_reason: None,
            input_request_id: None,
            input_request_ids: vec![],
            attention_since: 0,
            new_execution: false,
            id: format!("file:{name}"),
            name,
            source: TaskSource::LocalFile,
            status,
            host: None,
            project: None,
            message,
            activity: None,
            phase: None,
            started_at: Some(mtime),
            updated_at: mtime,
            finished_at: None,
            pid: None,
            exit_code: None,
            confidence: DetectionConfidence::Hook,
            remote: false,
            stale: false,
            recent: false,
            unread: false,
            pinned: false,
            turn_key: None,
            native_unread: None,
            terminal: None,
            can_focus: false,
            cpu: None,
            cli: None,
            session_id: None,
            hook_event: None,
            hook_matcher: None,
            quota_5h_used_pct: None,
            quota_5h_resets_at: None,
            quota_1w_used_pct: None,
            quota_1w_resets_at: None,
        })
    }
}

fn status_time(v: Option<&serde_json::Value>) -> Option<Option<SystemTime>> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Some(None);
    };
    // Older wrappers can emit zero for an unset timestamp.
    if v.as_i64() == Some(0) {
        return Some(None);
    }
    parse_epoch_ms(Some(v))
        .or_else(|| parse_rfc3339(Some(v)))
        .map(Some)
}

fn parse_epoch_ms(v: Option<&serde_json::Value>) -> Option<SystemTime> {
    let ms = v?.as_i64()?;
    if ms <= 0 {
        return None;
    }
    std::time::UNIX_EPOCH.checked_add(Duration::from_millis(ms as u64))
}

fn parse_rfc3339(v: Option<&serde_json::Value>) -> Option<SystemTime> {
    let s = v?.as_str()?;
    // Parse "2026-08-27T13:00:00Z" style timestamps (naive UTC).
    let (date, time) = s.split_once('T')?;
    let date = date.split('-').collect::<Vec<_>>();
    if date.len() != 3 {
        return None;
    }
    let time = time.trim_end_matches('Z');
    let time = time.split('.').next().unwrap_or(time);
    let hms = time.split(':').collect::<Vec<_>>();
    if hms.len() != 3 {
        return None;
    }
    if date[0].len() != 4 || !date[0].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i64 = date[0].parse().ok()?;
    let month: u32 = date[1].parse().ok()?;
    let day: u32 = date[2].parse().ok()?;
    let hour: u32 = hms[0].parse().ok()?;
    let min: u32 = hms[1].parse().ok()?;
    let sec: u32 = hms[2].parse().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if !(1..=days_in_month).contains(&day) || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = days * 86_400 + hour as i64 * 3_600 + min as i64 * 60 + sec as i64;
    if secs < 0 {
        std::time::UNIX_EPOCH.checked_sub(Duration::from_secs(secs.unsigned_abs()))
    } else {
        std::time::UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64))
    }
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> Option<i64> {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Read status files without deleting producer-owned records. A missing
/// heartbeat is uncertainty, never an exit or permission event.
impl StatusRound {
    fn collect(&mut self, dir: &Path, now: SystemTime) -> io::Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            if self.remaining_entries == 0 {
                return Err(io::Error::other(
                    "Compatibility status directory entry budget exhausted",
                ));
            }
            self.remaining_entries -= 1;
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("status") {
                continue;
            }
            let (mut task, mtime) = match read_status(&path, &mut self.remaining_bytes) {
                Ok(task) => task,
                Err(error) => {
                    if let Ok(id) = status_id(&path) {
                        self.failed_ids.insert(id);
                    }
                    self.problem.get_or_insert(error);
                    continue;
                }
            };
            // Retention hides only old completed results. An old or missing
            // heartbeat never proves the end of a running or waiting task.
            let ttl = if task.confidence == DetectionConfidence::Exact
                || task.finished_at.is_some()
                || dir.ends_with(".taskpulse")
            {
                V2_STALE_TTL
            } else {
                STALE_TTL
            };
            // TTL uses the modification time of the exact handle we read.
            if now.duration_since(mtime).unwrap_or_default() > ttl {
                if task.status.is_terminal() || task.finished_at.is_some() {
                    continue;
                }
                task.stale = true;
            }
            if task.status.is_terminal() || task.finished_at.is_some() {
                task.recent = true;
            }
            self.tasks.push(task);
        }
        Ok(())
    }
}

#[cfg(test)]
fn read_dir_tasks(dir: &Path, now: SystemTime) -> io::Result<Vec<TaskSnapshot>> {
    let mut round = StatusRound::default();
    round.collect(dir, now)?;
    round.problem.map_or(Ok(round.tasks), Err)
}

fn checked_status(s: &str) -> Option<TaskStatus> {
    let status = status_from_legacy(s);
    (status != TaskStatus::Unknown || matches!(s, "offline" | "unknown")).then_some(status)
}

/// Map a legacy (or v2) status string to the unified enum.
pub fn status_from_legacy(s: &str) -> TaskStatus {
    match s {
        "needs-input" => TaskStatus::WaitingInput,
        "working" | "starting" => TaskStatus::Running,
        "error" => TaskStatus::Failed,
        "idle" => TaskStatus::Idle,
        "offline" => TaskStatus::Unknown,
        "running" => TaskStatus::Running,
        "waiting_input" | "waiting-input" => TaskStatus::WaitingInput,
        "succeeded" | "success" => TaskStatus::Succeeded,
        "failed" => TaskStatus::Failed,
        "disconnected" => TaskStatus::Disconnected,
        "unknown" => TaskStatus::Unknown,
        _ => TaskStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn invalid_request_fields_are_rejected_even_outside_waiting_status() {
        for (field, values) in [
            ("waiting_reason", vec!["\"future_reason\"", "3", "null"]),
            ("input_request_id", vec!["42", "[]", "null"]),
        ] {
            for value in values {
                for status in ["waiting_input", "running"] {
                    let body = format!(r#"{{"v":2,"status":"{status}","{field}":{value}}}"#);
                    assert!(
                        parse_status(Path::new("x.status"), &body, now()).is_none(),
                        "accepted {field}={value} with status {status}"
                    );
                }
            }
        }
        let legacy = r#"{"v":1,"status":"needs-input"}"#;
        let parsed = parse_status(Path::new("x.status"), legacy, now()).unwrap();
        assert_eq!(parsed.status, TaskStatus::WaitingInput);
        assert_eq!(
            parsed.waiting_reason,
            Some(crate::domain::WaitingReason::Unknown)
        );
        assert!(parsed.input_request_id.is_none());
    }

    #[test]
    fn invalid_request_fields_recovery_preserves_pinned_unread_and_delivery() {
        use crate::{attention::AttentionTracker, transition::TransitionEngine};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approval.status");
        let attention_path = dir.path().join("attention.json");
        let notifications_path = dir.path().join("notifications.json");
        let provider = || LocalFileProvider {
            dirs: vec![dir.path().into()],
            last_tasks: vec![],
        };
        let now = now();
        let running = r#"{"v":2,"status":"running","turn_key":"turn-a"}"#;
        let pending = r#"{"v":2,"status":"waiting_input","turn_key":"turn-a","waiting_reason":"approval","input_request_id":"req-1"}"#;
        let mut collector = provider();
        let mut attention = AttentionTracker::open(attention_path.clone());
        let mut notifications = TransitionEngine::open(notifications_path.clone());

        std::fs::write(&path, running).unwrap();
        assert!(notifications
            .apply(&attention.reconcile(collector.poll(now).tasks), &[], now)
            .is_empty());
        std::fs::write(&path, pending).unwrap();
        let active = attention.reconcile(collector.poll(now).tasks);
        assert!(active[0].unread && active[0].requires_action());
        assert_eq!(notifications.apply(&active, &[], now).len(), 1);
        attention.set_pinned("file:approval", true).unwrap();
        drop((attention, notifications));

        let bad_reason = pending.replace("\"approval\"", "\"bad-enum\"");
        let bad_request = pending.replace("\"req-1\"", "false");
        for bad in [&bad_reason, &bad_request] {
            std::fs::write(&path, bad).unwrap();
            let warm = collector.poll(now);
            let mut cold = provider();
            let degraded = cold.poll(now);
            let mut attention = AttentionTracker::open(attention_path.clone());
            let mut notifications = TransitionEngine::open(notifications_path.clone());
            let recovered = attention.reconcile(degraded.tasks);
            assert_eq!(recovered[0].input_request_id.as_deref(), Some("req-1"));
            assert!(recovered[0].requires_action());
            assert!(recovered[0].stale && recovered[0].pinned && recovered[0].unread);
            assert!(!degraded.authoritative && !warm.authoritative);
            assert!(warm.tasks[0].stale);
            assert!(notifications.apply(&recovered, &[], now).is_empty());

            std::fs::write(&path, pending).unwrap();
            let repaired = cold.poll(now);
            assert!(repaired.authoritative);
            let recovered = attention.reconcile(repaired.tasks);
            assert_eq!(recovered[0].input_request_id.as_deref(), Some("req-1"));
            assert!(recovered[0].pinned && recovered[0].unread && !recovered[0].stale);
            assert!(notifications.apply(&recovered, &[], now).is_empty());
        }

        // Compatibility producers resolve by updating the same task snapshot.
        std::fs::write(&path, running).unwrap();
        let mut attention = AttentionTracker::open(attention_path);
        let mut notifications = TransitionEngine::open(notifications_path);
        let resolved = attention.reconcile(provider().poll(now).tasks);
        assert_eq!(resolved[0].status, TaskStatus::Running);
        assert!(resolved[0].input_request_id.is_none());
        assert!(resolved[0].pinned && resolved[0].unread && !resolved[0].stale);
        assert!(notifications.apply(&resolved, &[], now).is_empty());
    }

    #[test]
    fn oversized_torn_and_unreadable_status_preserve_last_pending_until_repaired() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("approval.status");
        let peer = root.path().join("healthy.status");
        let mut provider = LocalFileProvider {
            dirs: vec![root.path().into()],
            last_tasks: vec![],
        };
        let pending = br#"{"v":2,"status":"waiting_input","waiting_reason":"approval","input_request_id":"req-1"}"#;
        std::fs::write(&path, pending).unwrap();
        assert!(provider.poll(now()).authoritative);
        std::fs::write(&peer, "working|healthy").unwrap();
        for (n, contents) in [
            vec![b'x'; STATUS_FILE_BYTES as usize + 1],
            b"{\"status\":\"failed\"".to_vec(),
            vec![255],
            br#"{"status":"failed","finished_at":"999999999999999999-01-01T00:00:00Z"}"#.to_vec(),
        ]
        .iter()
        .enumerate()
        {
            std::fs::write(&path, contents).unwrap();
            let out = provider.poll(now());
            assert!(!out.authoritative, "round {n}");
            let cached = out.tasks.iter().find(|t| t.id == "file:approval").unwrap();
            assert_eq!(cached.status, TaskStatus::WaitingInput);
            assert_eq!(cached.input_request_id.as_deref(), Some("req-1"));
            assert!(cached.stale && cached.finished_at.is_none());
            assert!(out.tasks.iter().any(|t| t.id == "file:healthy" && !t.stale));
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(!provider.poll(now()).authoritative);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, pending).unwrap();
        let recovered = provider.poll(now());
        assert!(recovered.authoritative);
        assert!(recovered.tasks.iter().all(|task| !task.stale));
    }

    #[test]
    fn concurrent_atomic_replacements_never_mix_status_and_request() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("active.status");
        let active = br#"{"status":"working","message":"running"}"#;
        let pending = br#"{"status":"needs-input","message":"pending","waiting_reason":"approval","input_request_id":"req"}"#;
        std::fs::write(&path, active).unwrap();
        let start = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                start.wait();
                for n in 0..32 {
                    let mut next = tempfile::NamedTempFile::new_in(root.path()).unwrap();
                    next.write_all(if n % 2 == 0 { pending } else { active })
                        .unwrap();
                    // Win32 replacement can lose a race with an open
                    // reader even when the handle shares deletion.
                    let deadline = std::time::Instant::now() + Duration::from_secs(1);
                    loop {
                        match next.persist(&path) {
                            Ok(_) => break,
                            Err(error)
                                if cfg!(windows)
                                    && error.error.kind() == io::ErrorKind::PermissionDenied
                                    && std::time::Instant::now() < deadline =>
                            {
                                next = error.file;
                                std::thread::sleep(Duration::from_millis(2));
                            }
                            Err(error) => panic!("status replacement failed: {error}"),
                        }
                    }
                }
            });
            let mut provider = LocalFileProvider {
                dirs: vec![root.path().into()],
                last_tasks: vec![],
            };
            start.wait();
            for _ in 0..64 {
                for task in provider.poll(now()).tasks {
                    match task.status {
                        TaskStatus::Running => {
                            assert_eq!(task.message.as_deref(), Some("running"));
                            assert!(task.input_request_id.is_none());
                        }
                        TaskStatus::WaitingInput => {
                            assert_eq!(task.message.as_deref(), Some("pending"));
                            assert_eq!(task.input_request_id.as_deref(), Some("req"));
                        }
                        other => panic!("unexpected status: {other:?}"),
                    }
                }
            }
            writer.join().unwrap();
            assert!(provider.poll(now()).authoritative);
        });
    }

    #[test]
    fn incomplete_listing_cannot_use_earlier_duplicate_to_clear_pending() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let lower = a.path().join("approval.status");
        let higher = b.path().join("newer");
        std::fs::create_dir(&higher).unwrap();
        std::fs::write(&lower, "idle").unwrap();
        std::fs::write(
            higher.join("approval.status"),
            br#"{"status":"needs-input","waiting_reason":"approval","input_request_id":"keep"}"#,
        )
        .unwrap();
        let mut provider = LocalFileProvider {
            dirs: vec![a.path().into(), higher.clone()],
            last_tasks: vec![],
        };
        assert!(provider.poll(now()).authoritative);
        std::fs::remove_dir_all(&higher).unwrap();
        std::fs::write(higher, "not a directory").unwrap();
        let out = provider.poll(now());
        assert!(!out.authoritative);
        assert!(out.tasks.iter().all(|task| task.stale));
        assert!(out
            .tasks
            .iter()
            .any(|task| task.input_request_id.as_deref() == Some("keep")));
    }

    #[test]
    fn status_byte_budget_is_shared_between_directories_and_never_truncates_records() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut body = b"working|".to_vec();
        body.resize(STATUS_FILE_BYTES as usize, b'a');
        for dir in [&a, &b] {
            for i in 0..10 {
                std::fs::write(dir.path().join(format!("{i}.status")), &body).unwrap();
            }
        }
        let mut round = StatusRound::default();
        round.collect(a.path(), now()).unwrap();
        round.collect(b.path(), now()).unwrap();
        assert!(round.problem.is_some());
        assert!(round.tasks.len() < 20 && round.tasks.len() >= 10);
        assert!(round
            .tasks
            .iter()
            .all(|task| task.message.as_ref().unwrap().len() == body.len() - 8));
        assert_eq!(
            STATUS_ROUND_BYTES - round.remaining_bytes,
            round.tasks.len() as u64 * STATUS_FILE_BYTES
        );
    }

    #[test]
    fn old_running_and_pending_files_survive_ttl_without_becoming_finished() {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in [
            (
                "quiet",
                r#"{"v":2,"status":"running","execution_activity_at":1000}"#,
            ),
            ("pending", r#"{"v":1,"status":"needs-input"}"#),
        ] {
            let path = dir.path().join(format!("{name}.status"));
            std::fs::write(&path, body).unwrap();
        }
        let at = SystemTime::now();
        let old = read_dir_tasks(dir.path(), at + Duration::from_secs(4 * 3600)).unwrap();
        assert_eq!(old.len(), 2);
        assert!(old
            .iter()
            .all(|task| task.stale && task.finished_at.is_none()));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        assert_eq!(
            old.iter()
                .find(|t| t.id.contains("quiet"))
                .unwrap()
                .updated_at,
            std::time::UNIX_EPOCH + Duration::from_millis(1000)
        );
    }

    #[test]
    fn expired_results_are_hidden_but_producer_owned_files_are_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.status");
        std::fs::write(&path, r#"{"v":2,"status":"succeeded","exit_code":0}"#).unwrap();
        assert!(
            read_dir_tasks(dir.path(), SystemTime::now() + Duration::from_secs(3600))
                .unwrap()
                .is_empty()
        );
        assert!(path.exists());
    }

    fn temp_file(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn parse_v1_json_status() {
        let f = temp_file(
            r#"{"v":1,"status":"working","message":"Running...","terminal":{"kind":"iterm2","focus_id":"w0t0p0:ABC","outer_id":"","label":"iTerm2"}}"#,
        );
        let t = parse_status_file(f.path(), now()).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        assert_eq!(t.message.as_deref(), Some("Running..."));
        assert_eq!(t.confidence, DetectionConfidence::Hook);
        assert!(t.can_focus);
        assert_eq!(t.source, TaskSource::LocalFile);
    }

    #[test]
    fn legacy_windows_status_is_readable_without_trusting_its_focus_metadata() {
        let f = temp_file(
            r#"{"v":1,"status":"working","terminal":{"kind":"windows_native","focus_id":"42","outer_id":"","label":"Terminal"}}"#,
        );
        let task = parse_status_file(f.path(), now()).unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.terminal.unwrap().focus_id, "42");
        assert!(!task.can_focus);
    }

    #[test]
    fn parse_v1_needs_input() {
        let f =
            temp_file(r#"{"v":1,"status":"needs-input","message":"permission","source":"hook"}"#);
        let t = parse_status_file(f.path(), now()).unwrap();
        assert_eq!(t.status, TaskStatus::WaitingInput);
        assert_eq!(t.confidence, DetectionConfidence::Hook);
    }

    #[test]
    fn parse_v2_with_exit_code_is_exact() {
        let f = temp_file(
            r#"{"v":2,"status":"succeeded","exit_code":0,"started_at":1700000000000,"finished_at":1700000010000,"confidence":"exact"}"#,
        );
        let t = parse_status_file(f.path(), now()).unwrap();
        assert_eq!(t.status, TaskStatus::Succeeded);
        assert_eq!(t.exit_code, Some(0));
        assert_eq!(t.confidence, DetectionConfidence::Exact);
        assert!(t.finished_at.is_some());
    }

    #[test]
    fn parse_v2_failed_with_nonzero_exit() {
        let f = temp_file(r#"{"v":2,"status":"failed","exit_code":2}"#);
        let t = parse_status_file(f.path(), now()).unwrap();
        assert_eq!(t.status, TaskStatus::Failed);
        assert_eq!(t.exit_code, Some(2));
    }

    #[test]
    fn parse_legacy_pipe_format() {
        let f = temp_file("working|Running tests...");
        let t = parse_status_file(f.path(), now()).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        assert_eq!(t.message.as_deref(), Some("Running tests..."));
        assert_eq!(t.confidence, DetectionConfidence::Hook);
    }

    #[test]
    fn parse_empty_returns_none() {
        let f = temp_file("   \n");
        assert!(parse_status_file(f.path(), now()).is_none());
    }

    #[test]
    fn parse_unknown_version_skipped() {
        let f = temp_file(r#"{"v":99,"status":"working"}"#);
        assert!(parse_status_file(f.path(), now()).is_none());
    }

    #[test]
    fn message_truncated_at_500_chars() {
        let long = "x".repeat(600);
        let f = temp_file(&format!(
            r#"{{"v":1,"status":"working","message":"{long}"}}"#
        ));
        let t = parse_status_file(f.path(), now()).unwrap();
        assert!(t.message.as_ref().unwrap().len() <= 500);
    }

    #[test]
    fn rfc3339_and_epoch_parsing() {
        let t1 = parse_rfc3339(Some(&serde_json::json!("2026-08-27T05:00:00Z"))).unwrap();
        let t2 = parse_epoch_ms(Some(&serde_json::json!(1787806800000u64))).unwrap();
        // Both should be ~the same instant (2026-08-27 05:00:00 UTC).
        let d1 = t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let d2 = t2.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(d1, d2, "d1={d1} d2={d2}");
    }

    #[test]
    fn status_mapping_covers_all_legacy_names() {
        assert_eq!(status_from_legacy("needs-input"), TaskStatus::WaitingInput);
        assert_eq!(status_from_legacy("working"), TaskStatus::Running);
        assert_eq!(status_from_legacy("starting"), TaskStatus::Running);
        assert_eq!(status_from_legacy("error"), TaskStatus::Failed);
        assert_eq!(status_from_legacy("idle"), TaskStatus::Idle);
        assert_eq!(status_from_legacy("offline"), TaskStatus::Unknown);
        assert_eq!(status_from_legacy("garbage"), TaskStatus::Unknown);
    }
}
