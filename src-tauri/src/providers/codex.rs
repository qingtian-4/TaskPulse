//! Codex provider — session-metadata tier for Codex Desktop / CLI.
//!
//! Priority per the TaskPulse design: native hook > session/app-server
//! metadata > process scan.  On this machine Codex is the Desktop (electron)
//! app; it doesn't expose a CLI hook we can reuse non-invasively, so we read
//! its session transcripts: `~/.codex/sessions/**/rollout-*.jsonl`.  On the
//! remote hosts the same transcripts live under the server user's home, read
//! via the remote-tmux snapshot (`__TP_CODEX__` section) — this module shares
//! one event parser for both.
//!
//! State inference (from the agent's own emitted events — more reliable than
//! process scanning, so confidence = HOOK):
//!   `event_msg.task_started` / `user_message`          → RUNNING
//!   `event_msg.agent_message` / `token_count` / tool
//!     `*_end` events (activity)                       → RUNNING
//!   `event_msg.task_complete`                          → WAITING_INPUT
//!   `event_msg.turn_aborted` / `thread_rolled_back`    → WAITING_INPUT
//!
//! The `notify = ["...", "turn-ended"]` config key exists in Codex Desktop;
//! wiring TaskPulse's own receiver there would give EXACT turn-end events, but
//! that mutates the user's Codex config — deferred.  Reading the transcript is
//! non-invasive and stays behind this adapter, so a Codex format change only
//! touches this file (plus the shared parses below).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use crate::providers::{Provider, ProviderOutput};
use std::collections::HashMap;

/// A session whose last event is `task_complete` is WAITING_INPUT for this
/// long before we consider it dormant/uninteresting.
const WAITING_TTL: Duration = Duration::from_secs(24 * 3600);
/// An actively-running session must have been written recently, else we treat
/// the transcript as stale (a crashed/abandoned turn).
const RUNNING_TTL: Duration = Duration::from_secs(10 * 60);
/// Cap on how many recent sessions we surface at once.
const MAX_SESSIONS: usize = 6;
const PROJECT_LEN: usize = 40;

pub struct CodexProvider {
    sessions_root: PathBuf,
    /// session id → last-seen mtime, so we can emit a terminal snapshot when a
    /// session stops being updated (it finished).
    seen: std::collections::HashMap<String, SystemTime>,
}

impl CodexProvider {
    pub fn new() -> Self {
        Self::with_root(
            dirs_next::home_dir()
                .map(|h| h.join(".codex/sessions"))
                .unwrap_or_default(),
        )
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self {
            sessions_root: root,
            seen: Default::default(),
        }
    }

    fn discover(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Ok(entries) = walk(&self.sessions_root) {
            for path in entries {
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.starts_with("rollout-") {
                        files.push(path);
                    }
                }
            }
        }
        // newest first by mtime
        files.sort_by(|a, b| {
            let am = a
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            let bm = b
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            bm.cmp(&am)
        });
        files
    }

    fn parse_session_state(&self, path: &Path, now: SystemTime) -> Option<SessionState> {
        let body = std::fs::read_to_string(path).ok()?;
        let sid = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        parse_codex_rollout(&body, now, &sid)
    }

    fn session_to_task(&self, host: &SessionState, now: SystemTime) -> TaskSnapshot {
        let name = host.project.clone().unwrap_or_else(|| "Codex".to_string());
        let mut task = TaskSnapshot::new(
            format!("codex:{}", host.session_id),
            name,
            TaskSource::Codex,
            host.status,
            DetectionConfidence::Hook,
            now,
        );
        task.project = host.project.clone();
        task.cli = Some("codex".into());
        task.session_id = Some(host.session_id.clone());
        task.quota_5h_used_pct = host.quota_5h_used_pct;
        task.quota_5h_resets_at = host.quota_5h_resets_at;
        task.quota_1w_used_pct = host.quota_1w_used_pct;
        task.quota_1w_resets_at = host.quota_1w_resets_at;
        task.started_at = Some(host.updated_at);
        task.message = host.last_message.clone();
        // A completed turn means the agent is waiting on the user — keep the
        // timestamp reflecting the moment the turn finished.
        if host.status == TaskStatus::WaitingInput {
            task.updated_at = host.updated_at;
        }
        task
    }
}

fn walk(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if root.is_dir() {
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                out.extend(walk(&p)?);
            } else {
                out.push(p);
            }
        }
    }
    Ok(out)
}

/// Derived live state of one Codex session.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub status: TaskStatus,
    pub updated_at: SystemTime,
    pub project: Option<String>,
    pub last_message: Option<String>,
    pub session_id: String,
    pub quota_5h_used_pct: Option<f64>,
    pub quota_5h_resets_at: Option<i64>,
    pub quota_1w_used_pct: Option<f64>,
    pub quota_1w_resets_at: Option<i64>,
}

/// Shared Codex rollout → session-state parser.
///
/// Used by the local `CodexProvider` and by the remote-tmux snapshot parser
/// (which reads remote `~/.codex/sessions` rollouts). Keeps the Codex event
/// semantics in ONE place behind an adapter boundary.
pub fn parse_codex_rollout(
    body: &str,
    now: SystemTime,
    default_session_id: &str,
) -> Option<SessionState> {
    if body.trim().is_empty() {
        return None;
    }
    let mut project: Option<String> = None;
    let mut status = TaskStatus::Unknown;
    let mut updated_at = now;
    let mut last_message: Option<String> = None;
    let mut session_id = default_session_id.to_string();
    let mut quota_5h_used_pct: Option<f64> = None;
    let mut quota_5h_resets_at: Option<i64> = None;
    let mut quota_1w_used_pct: Option<f64> = None;
    let mut quota_1w_resets_at: Option<i64> = None;

    for line in body.lines() {
        let line = line.trim_start();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ts = value.get("timestamp").and_then(parse_rfc3339_value);
        let Some(payload) = value.get("payload") else {
            continue;
        };

        // Extract project (cwd) + session id on ANY line that carries them
        // — session_meta / turn_context have no `payload.type`, so this
        // must run before the type guard below.
        if project.is_none() {
            if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                let base = Path::new(cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| cwd.to_string());
                project = Some(truncate(&base, PROJECT_LEN));
            }
        }
        // Codex emits session_meta with `type` in different places depending
        // on host form: local Desktop puts it in payload.type, headless
        // app-server (Server3) puts it at the top level. Accept both.
        let is_session_meta = value.get("type").and_then(|v| v.as_str()) == Some("session_meta")
            || payload.get("type").and_then(|v| v.as_str()) == Some("session_meta");
        if is_session_meta {
            if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
                session_id = id.to_string();
            }
        }

        let Some(ptype) = payload.get("type").and_then(|v| v.as_str()) else {
            continue;
        };

        match ptype {
            "task_started" | "user_message" => {
                status = TaskStatus::Running;
                if let Some(t) = ts {
                    updated_at = t;
                }
            }
            "agent_message" => {
                status = TaskStatus::Running;
                if let Some(t) = ts {
                    updated_at = t;
                }
                if last_message.is_none() {
                    if let Some(msg) = payload.get("message").and_then(|v| v.as_str()) {
                        last_message = Some(truncate(msg, 120));
                    }
                }
            }
            "token_count" => {
                if status != TaskStatus::Running {
                    status = TaskStatus::Running;
                }
                if let Some(t) = ts {
                    updated_at = t;
                }
                // Codex rate-limit quota: primary = 5h window, secondary = 1 week.
                if let Some(rl) = payload.get("rate_limits") {
                    if let Some(primary) = rl.get("primary") {
                        quota_5h_used_pct = primary.get("used_percent").and_then(|v| v.as_f64());
                        quota_5h_resets_at = primary.get("resets_at").and_then(|v| v.as_i64());
                    }
                    if let Some(secondary) = rl.get("secondary") {
                        quota_1w_used_pct = secondary.get("used_percent").and_then(|v| v.as_f64());
                        quota_1w_resets_at = secondary.get("resets_at").and_then(|v| v.as_i64());
                    }
                }
            }
            "mcp_tool_call_end" | "web_search_end" | "patch_apply_end" | "item_completed" => {
                // activity — confirms the turn is running
                if status != TaskStatus::Running {
                    status = TaskStatus::Running;
                }
                if let Some(t) = ts {
                    updated_at = t;
                }
            }
            "task_complete" => {
                status = TaskStatus::WaitingInput;
                if let Some(t) = payload
                    .get("completed_at")
                    .and_then(parse_rfc3339_value)
                    .or(ts)
                {
                    updated_at = t;
                }
                if last_message.is_none() {
                    if let Some(msg) = payload.get("last_agent_message").and_then(|v| v.as_str()) {
                        last_message = Some(truncate(msg, 120));
                    }
                }
            }
            "turn_aborted" | "thread_rolled_back" => {
                status = TaskStatus::WaitingInput;
                if let Some(t) = ts {
                    updated_at = t;
                }
            }
            _ => {}
        }
    }

    // Lifecycle gate.
    let age = now.duration_since(updated_at).unwrap_or_default();
    match status {
        TaskStatus::Running => {
            if age > RUNNING_TTL {
                // no recent write while "running" → stale transcript; drop.
                return None;
            }
        }
        TaskStatus::WaitingInput => {
            if age > WAITING_TTL {
                return None;
            }
        }
        _ => return None,
    }

    Some(SessionState {
        status,
        updated_at,
        project,
        last_message,
        session_id,
        quota_5h_used_pct,
        quota_5h_resets_at,
        quota_1w_used_pct,
        quota_1w_resets_at,
    })
}

/// Build remote Codex tasks from the snapshot's `__TP_CODEX__` section.
///
/// Each remote rollout is prefixed by a `===S:<slug>===` header (the workspace
/// slug the rollout lives under). We parse each rollout body with the shared
/// `parse_codex_rollout` and emit a task per live session.
pub fn tasks_from_codex_section(
    section: &str,
    ssh_host: &str,
    host_id: &str,
    now: SystemTime,
) -> Vec<TaskSnapshot> {
    let mut titles: HashMap<String, String> = HashMap::new();
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("META\t") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
                if let (Some(id), Some(t)) = (
                    v.get("id").and_then(|x| x.as_str()),
                    v.get("t").and_then(|x| x.as_str()),
                ) {
                    if !t.trim().is_empty() {
                        titles.insert(id.to_string(), t.to_string());
                    }
                }
            }
        }
    }

    let mut out = Vec::new();
    let mut current_slug: Option<String> = None;
    let mut current_uuid: Option<String> = None;
    let mut buf = String::new();

    for line in section.lines() {
        let line = line.trim();
        if line.starts_with("META\t") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("===S:") {
            let rest = rest.strip_suffix("===").unwrap_or(rest);
            // rest = "<slug>===F:<basename>"
            let (slug, fname) = match rest.split_once("===F:") {
                Some((s, f)) => (s, Some(f)),
                None => (rest, None),
            };
            flush_codex(
                &current_slug,
                &current_uuid,
                &mut buf,
                now,
                ssh_host,
                host_id,
                &titles,
                &mut out,
            );
            current_slug = Some(slug.to_string());
            current_uuid = fname.and_then(uuid_from_filename);
            buf.clear();
            continue;
        }
        buf.push_str(line);
        buf.push('\n');
    }
    flush_codex(
        &current_slug,
        &current_uuid,
        &mut buf,
        now,
        ssh_host,
        host_id,
        &titles,
        &mut out,
    );
    propagate_freshest_quota(&mut out);
    out
}

/// Extract the Codex thread id (uuid) from a rollout filename like
/// `rollout-2026-08-27T23-12-16-01a0457e-da94-7711-a20f-9738009992b8.jsonl`.
/// The tail we read for remote sessions does not include session_meta, so we
/// recover the id from the filename instead.
fn uuid_from_filename(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    let stem = stem.strip_prefix("rollout-")?;
    let parts: Vec<&str> = stem.split('-').collect();
    // parts = [YYYY, MM, DDTHH, MM, SS, <uuid 5 groups>]  (10 total)
    if parts.len() < 10 {
        return None;
    }
    Some(parts[5..].join("-"))
}

/// Copy the freshest rate-limit quota (from the most-recently-updated codex
/// session) onto every codex task, so the displayed `5h / 1周` usage reflects
/// the newest codex self-report instead of a stale session's cached value.
pub(crate) fn propagate_freshest_quota(tasks: &mut [TaskSnapshot]) {
    let Some(newest) = tasks
        .iter()
        .filter(|t| t.source == TaskSource::Codex)
        .max_by_key(|t| t.updated_at)
    else {
        return;
    };
    let (h5, r5, h1, r1) = (
        newest.quota_5h_used_pct,
        newest.quota_5h_resets_at,
        newest.quota_1w_used_pct,
        newest.quota_1w_resets_at,
    );
    for t in tasks.iter_mut() {
        if t.source == TaskSource::Codex {
            t.quota_5h_used_pct = h5;
            t.quota_5h_resets_at = r5;
            t.quota_1w_used_pct = h1;
            t.quota_1w_resets_at = r1;
        }
    }
}

fn is_synthetic_title(t: &str) -> bool {
    t.starts_with("The following is the Codex agent history")
        || t.starts_with("# Files mentioned by the user")
        || t.starts_with("PLEASE IMPLEMENT THIS PLAN")
}

/// Collapse whitespace/newlines (titles can be multi-line prompts) and clamp
/// to a readable length for a row label.
fn clean_task_name(s: &str) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= 56 {
        collapsed
    } else {
        let mut t: String = collapsed.chars().take(56).collect();
        t.push('…');
        t
    }
}

fn flush_codex(
    slug: &Option<String>,
    uuid: &Option<String>,
    buf: &mut String,
    now: SystemTime,
    ssh_host: &str,
    host_id: &str,
    titles: &HashMap<String, String>,
    out: &mut Vec<TaskSnapshot>,
) {
    if buf.trim().is_empty() {
        return;
    }
    let Some(slug) = slug else { return };
    if let Some(state) = parse_codex_rollout(buf, now, slug) {
        // The tail we read may not carry session_meta (it's at the top of the
        // file), so prefer the thread id extracted from the rollout filename.
        let sid = uuid
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| state.session_id.clone());
        let title = titles.get(&sid).map(|s| s.as_str());
        // A1: only surface sessions that carry a readable task title; drop the
        // synthetic sub-agent boilerplate so the view stays clean.
        if title.is_none_or(|t| t.trim().is_empty() || is_synthetic_title(t)) {
            return;
        }
        let name = title.map(clean_task_name).unwrap_or_else(|| slug.clone());
        let mut t = TaskSnapshot::new(
            format!("codex-remote:{host_id}:{sid}"),
            name,
            TaskSource::Codex,
            state.status,
            DetectionConfidence::Hook,
            now,
        )
        .with_host(ssh_host.to_string())
        .with_remote();
        t.project = state.project.or_else(|| Some(slug.clone()));
        t.updated_at = state.updated_at;
        t.message = state.last_message;
        t.cli = Some("codex".into());
        t.session_id = Some(sid);
        t.quota_5h_used_pct = state.quota_5h_used_pct;
        t.quota_5h_resets_at = state.quota_5h_resets_at;
        t.quota_1w_used_pct = state.quota_1w_used_pct;
        t.quota_1w_resets_at = state.quota_1w_resets_at;
        out.push(t);
    }
}

impl Provider for CodexProvider {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let files = self.discover();
        let mut tasks = Vec::new();
        let mut live: std::collections::HashMap<String, SystemTime> = Default::default();

        for path in files.iter().take(MAX_SESSIONS) {
            if let Some(state) = self.parse_session_state(path, now) {
                live.insert(state.session_id.clone(), state.updated_at);
                tasks.push(self.session_to_task(&state, now));
            }
        }
        propagate_freshest_quota(&mut tasks);

        // Sessions we previously knew but that vanished / went stale → no
        // terminal snapshot needed (the store's TTL handles the drop). We only
        // track them so a re-appearing session keeps its identity.
        self.seen = live;
        ProviderOutput {
            tasks,
            hosts: vec![],
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

fn parse_rfc3339_value(v: &serde_json::Value) -> Option<SystemTime> {
    let s = v.as_str()?;
    parse_rfc3339(s)
}

fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    // Accept "2026-07-06T08:22:01.075Z" and "+00:00" offsets.
    let cleaned = s.replace('Z', "+00:00");
    let dt = chrono_like_parse(&cleaned)?;
    Some(UNIX_EPOCH + Duration::from_secs(dt))
}

/// Minimal RFC3339 → unix-seconds parser (no external chrono dependency in the
/// MVP; handles the Codex timestamp shape exactly).
fn chrono_like_parse(s: &str) -> Option<u64> {
    // 2026-07-06T08:22:01.075+00:00
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let year = num(&s[0..4])?;
    let month = num(&s[5..7])?;
    let day = num(&s[8..10])?;
    let hour = num(&s[11..13])?;
    let min = num(&s[14..16])?;
    let sec = num(&s[17..19])?;
    let days = days_from_civil(year as i64, month as i64, day as i64);
    let secs = days * 86_400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64;
    if secs >= 0 {
        Some(secs as u64)
    } else {
        None
    }
}

fn num(s: &str) -> Option<u64> {
    s.parse::<u64>().ok()
}

/// Days since 1970-01-01 (Howard Hinnant's civil-days algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_rollout(body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rollout-test.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        (dir, p)
    }

    fn header(cwd: &str) -> String {
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"session_meta\",\"id\":\"sess-1\",\"cwd\":\"{cwd}\",\"originator\":\"Codex Desktop\"}}}}\n",
            ts = recent_rfc3339(120)
        )
    }

    /// RFC3339 string for `now - secs_ago`, so fixtures are always "recent"
    /// relative to SystemTime::now() (otherwise the 24h TTL gate drops them).
    fn recent_rfc3339(secs_ago: u64) -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(secs_ago);
        format_unix(secs)
    }

    fn format_unix(secs: u64) -> String {
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        let (y, m, d) = civil_from_days(days);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
            rem / 3600,
            (rem % 3600) / 60,
            rem % 60
        )
    }

    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as i64;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        (if m <= 2 { y + 1 } else { y }, m, d)
    }

    #[test]
    fn completed_turn_is_waiting_input() {
        let body = format!(
            "{}{}",
            header("/mnt/c/projects/my-app"),
            format!(
                "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t1\",\"last_agent_message\":\"done\",\"completed_at\":\"{ts}\"}}}}\n",
                ts = recent_rfc3339(60)
            )
        );
        let (_d, p) = temp_rollout(&body);
        let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
        let now = SystemTime::now();
        let st = provider.parse_session_state(&p, now).unwrap();
        assert_eq!(st.status, TaskStatus::WaitingInput);
        assert_eq!(st.project.as_deref(), Some("my-app"));
    }

    #[test]
    fn active_turn_is_running() {
        let body = format!(
            "{}{}",
            header("/home/user/proj"),
            format!(
                "{{\"timestamp\":\"{ts0}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"t2\"}}}}\n{{\"timestamp\":\"{ts1}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"working on it\"}}}}\n",
                ts0 = recent_rfc3339(120),
                ts1 = recent_rfc3339(90)
            )
        );
        let (_d, p) = temp_rollout(&body);
        let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
        let st = provider.parse_session_state(&p, SystemTime::now()).unwrap();
        assert_eq!(st.status, TaskStatus::Running);
    }

    #[test]
    fn aborted_turn_returns_to_waiting() {
        let body = format!(
            "{}{}",
            header("/home/user/proj"),
            format!(
                "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"turn_aborted\",\"turn_id\":\"t3\"}}}}\n",
                ts = recent_rfc3339(60)
            )
        );
        let (_d, p) = temp_rollout(&body);
        let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
        let st = provider.parse_session_state(&p, SystemTime::now()).unwrap();
        assert_eq!(st.status, TaskStatus::WaitingInput);
    }

    #[test]
    fn stale_running_session_dropped() {
        let body = format!(
            "{}{}",
            header("/home/user/proj"),
            format!(
                "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"t4\"}}}}\n",
                ts = recent_rfc3339(60)
            )
        );
        let (_d, p) = temp_rollout(&body);
        let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
        // now is far in the future → the running session is stale → dropped
        let fut = SystemTime::now() + Duration::from_secs(3600 * 30);
        assert!(provider.parse_session_state(&p, fut).is_none());
    }

    #[test]
    fn project_with_unicode_and_spaces() {
        let body = format!(
            "{}{}",
            header("/home/user/实验 数据/My Project"),
            format!(
                "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t5\",\"completed_at\":\"{ts}\"}}}}\n",
                ts = recent_rfc3339(60)
            )
        );
        let (_d, p) = temp_rollout(&body);
        let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
        let st = provider.parse_session_state(&p, SystemTime::now()).unwrap();
        assert_eq!(st.project.as_deref(), Some("My Project"));
    }

    #[test]
    fn discover_finds_rollouts_recursively() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("2026/08/27");
        let mut f = std::fs::File::create(a.join("rollout-x.jsonl")).unwrap_or_else(|_| {
            std::fs::create_dir_all(&a).unwrap();
            std::fs::File::create(a.join("rollout-x.jsonl")).unwrap()
        });
        f.write_all(b"{}").unwrap();
        let provider = CodexProvider::with_root(root.path().to_path_buf());
        let files = provider.discover();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn codex_section_builds_remote_tasks() {
        let section = format!(
            "META\t{{\"id\":\"sess-1\",\"t\":\"研究 Game 记忆机制\"}}\n===S:--home-ai-projects-Game--===\n{}\n{}\n",
            header("/home/ai/projects/Game"),
            format!(
                "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t1\",\"completed_at\":\"{ts}\"}}}}\n",
                ts = recent_rfc3339(60)
            )
        );
        let now = SystemTime::now();
        let tasks = tasks_from_codex_section(&section, "Server3", "server3", now);
        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];
        assert_eq!(t.source, TaskSource::Codex);
        assert_eq!(t.status, TaskStatus::WaitingInput);
        assert_eq!(t.host.as_deref(), Some("Server3"));
        assert!(t.remote);
        assert_eq!(t.name, "研究 Game 记忆机制");
    }
}
