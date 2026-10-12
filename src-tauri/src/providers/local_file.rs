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

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus, TerminalInfo};
use crate::providers::{Provider, ProviderOutput};

/// How long a legacy status file can sit untouched before we hide it
/// (unchanged from AgentTray).
const STALE_TTL: Duration = Duration::from_secs(10 * 60);
/// Same rule for v2 files that never reported a finish.
const V2_STALE_TTL: Duration = Duration::from_secs(30 * 60);

pub struct LocalFileProvider {
    dirs: Vec<PathBuf>,
    last_tasks: Vec<TaskSnapshot>,
}

impl LocalFileProvider {
    pub fn new() -> Self {
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
        let mut tasks = Vec::new();
        let mut errors = Vec::new();
        for dir in &self.dirs {
            match read_dir_tasks(dir, now) {
                Ok(found) => tasks.extend(found),
                Err(error) => errors.push(format!("{}: {error}", dir.display())),
            }
        }
        if errors.is_empty() {
            self.last_tasks = tasks.clone();
            return ProviderOutput::tasks(tasks);
        }

        let mut fallback = self
            .last_tasks
            .iter()
            .cloned()
            .map(|mut task| {
                task.stale = true;
                task
            })
            .collect::<Vec<_>>();
        for task in tasks {
            fallback.retain(|previous| previous.id != task.id);
            fallback.push(task);
        }
        ProviderOutput::degraded(fallback, errors.join("; "))
    }
}

/// Parse one status file into a `TaskSnapshot`.
pub fn parse_status_file(path: &Path, now: SystemTime) -> Option<TaskSnapshot> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    let name = path.file_stem()?.to_str()?.to_string();

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        // Version gate: v1 (AgentTray) or v2 (TaskPulse); anything else skips.
        let version = val.get("v").and_then(|v| v.as_u64()).unwrap_or(1);
        if version != 1 && version != 2 {
            return None;
        }

        let status_str = val.get("status")?.as_str()?;
        let status = status_from_legacy(status_str);
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
            .map(|t| !t.focus_id.is_empty() && t.focus_id != "0")
            .unwrap_or(false);

        // v2-only fields
        let started_at =
            parse_epoch_ms(val.get("started_at")).or_else(|| parse_rfc3339(val.get("started_at")));
        let finished_at = parse_epoch_ms(val.get("finished_at"))
            .or_else(|| parse_rfc3339(val.get("finished_at")));
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

        let display_name = match (cwd_project.as_deref(), terminal.as_ref()) {
            (Some(proj), Some(t)) if !t.label.is_empty() => format!("{proj} · {}", t.label),
            (Some(proj), _) => proj.to_string(),
            _ => name.clone(),
        };

        let mut snapshot = TaskSnapshot {
            source_instance: None,
            model: None,
            internal: false,
            project_path: None,
            tmux_session: None,
            tmux_pane: None,
            tmux_target: None,
            waiting_reason: None,
            input_request_id: None,
            attention_since: 0,
            new_execution: false,
            id: format!("file:{name}"),
            name: display_name,
            source: TaskSource::LocalFile,
            status,
            host: None,
            project: cwd_project,
            message: (!message.is_empty()).then_some(message),
            activity: None,
            phase: None,
            started_at: started_at.or_else(|| file_mtime(path)),
            updated_at: file_mtime(path).unwrap_or(now),
            finished_at,
            pid: None,
            exit_code,
            confidence,
            remote: false,
            stale: false,
            recent: false,
            unread: false,
            pinned: false,
            turn_key: None,
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
            snapshot.waiting_reason = val
                .get("waiting_reason")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .or(Some(crate::domain::WaitingReason::Unknown));
            snapshot.input_request_id = val
                .get("input_request_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
        Some(snapshot)
    } else {
        // Legacy pipe format: "status|message"
        let (status, message) = match trimmed.split_once('|') {
            Some((s, m)) => (status_from_legacy(s), Some(m.to_string())),
            None => (status_from_legacy(trimmed), None),
        };
        Some(TaskSnapshot {
            source_instance: None,
            model: None,
            internal: false,
            project_path: None,
            tmux_session: None,
            tmux_pane: None,
            tmux_target: None,
            waiting_reason: None,
            input_request_id: None,
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
            started_at: file_mtime(path),
            updated_at: file_mtime(path).unwrap_or(now),
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

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

fn parse_epoch_ms(v: Option<&serde_json::Value>) -> Option<SystemTime> {
    let ms = v?.as_i64()?;
    if ms <= 0 {
        return None;
    }
    Some(std::time::UNIX_EPOCH + Duration::from_millis(ms as u64))
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
    let year: i64 = date[0].parse().ok()?;
    let month: u32 = date[1].parse().ok()?;
    let day: u32 = date[2].parse().ok()?;
    let hour: u32 = hms[0].parse().ok()?;
    let min: u32 = hms[1].parse().ok()?;
    let sec: u32 = hms[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = days * 86_400 + hour as i64 * 3_600 + min as i64 * 60 + sec as i64;
    Some(std::time::UNIX_EPOCH + Duration::from_secs(secs as u64))
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

/// Read all `*.status` files in a directory, cleaning up stale ones.
fn read_dir_tasks(dir: &Path, now: SystemTime) -> std::io::Result<Vec<TaskSnapshot>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(dir)?;

    let mut tasks = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("status") {
            continue;
        }
        let Some(mut task) = parse_status_file(&path, now) else {
            continue;
        };

        // Stale-file cleanup.
        // Legacy dir: AgentTray semantics — hide (and delete) untouched files.
        // v2 dir: only delete files that never reported a finish.
        let is_v2 = task.confidence == DetectionConfidence::Exact
            || task.finished_at.is_some()
            || path
                .parent()
                .map(|p| p.ends_with(".taskpulse"))
                .unwrap_or(false);
        let ttl = if is_v2 { V2_STALE_TTL } else { STALE_TTL };
        let mtime = task.updated_at;
        if now.duration_since(mtime).unwrap_or_default() > ttl {
            let _ = std::fs::remove_file(&path);
            continue;
        }

        if task.status.is_terminal() || task.finished_at.is_some() {
            task.recent = true;
        }
        tasks.push(task);
    }
    Ok(tasks)
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
