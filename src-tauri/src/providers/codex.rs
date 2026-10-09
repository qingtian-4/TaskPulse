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

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use crate::providers::{Provider, ProviderOutput};
use std::collections::HashMap;

/// A session whose last event is `task_complete` is WAITING_INPUT for this
/// long before we consider it dormant/uninteresting.
const WAITING_TTL: Duration = Duration::from_secs(24 * 3600);
/// An actively-running session must have been written recently, else we treat
/// the transcript as stale (a crashed/abandoned turn).
const RUNNING_TTL: Duration = Duration::from_secs(10 * 60);
/// Metadata discovery horizon, not a UI retention limit. New turns update the
/// thread/rollout timestamp and immediately become eligible again.
const DISCOVERY_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// Native titles change rarely, and starting app-server is materially more
/// expensive than reading an appended rollout. Keep runtime state polling
/// fast while still synchronizing sidebar renames promptly.
const APP_SERVER_REFRESH: Duration = Duration::from_secs(15);
const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(4);
const PROJECT_LEN: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CodexQuota {
    pub used_5h_pct: f64,
    pub resets_5h_at: Option<i64>,
    pub used_1w_pct: f64,
    pub resets_1w_at: Option<i64>,
}

static NATIVE_QUOTA: OnceLock<RwLock<Option<CodexQuota>>> = OnceLock::new();

pub struct CodexProvider {
    sessions_root: PathBuf,
    state_db: Option<PathBuf>,
    native_rx: Option<mpsc::Receiver<Result<NativeCodexSnapshot, String>>>,
    /// session id → last-seen mtime, so we can emit a terminal snapshot when a
    /// session stops being updated (it finished).
    seen: std::collections::HashMap<String, SystemTime>,
    /// path → (mtime, size, parsed state). Rollouts are append-only, so an
    /// unchanged signature can reuse the previous parse instead of rereading
    /// multi-megabyte JSONL files every two seconds.
    cache: std::collections::HashMap<PathBuf, (SystemTime, u64, Option<SessionState>)>,
    native_metadata: HashMap<String, AppThreadMeta>,
    last_tasks: Vec<TaskSnapshot>,
}

impl CodexProvider {
    pub fn new() -> Self {
        let home = dirs_next::home_dir().unwrap_or_default();
        let mut provider = Self::with_root(home.join(".codex/sessions"));
        let db = home.join(".codex/state_5.sqlite");
        provider.state_db = db.exists().then_some(db);
        let (tx, rx) = mpsc::sync_channel(1);
        provider.native_rx = Some(rx);
        std::thread::spawn(move || loop {
            if tx.send(query_codex_app_server()).is_err() {
                break;
            }
            std::thread::sleep(APP_SERVER_REFRESH);
        });
        provider
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self {
            sessions_root: root,
            state_db: None,
            native_rx: None,
            seen: Default::default(),
            cache: Default::default(),
            native_metadata: Default::default(),
            last_tasks: Vec::new(),
        }
    }

    fn thread_metadata(&mut self, now: SystemTime) -> Result<HashMap<String, ThreadMeta>, String> {
        self.refresh_native_metadata(now);
        let Some(path) = &self.state_db else {
            return Ok(self
                .native_metadata
                .values()
                .map(ThreadMeta::from_app_server)
                .map(|meta| (meta.id.clone(), meta))
                .collect());
        };
        let conn = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("open state_5.sqlite failed: {error}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, title, first_user_message, cwd, rollout_path, updated_at, is_pinned \
                 FROM threads WHERE archived = 0",
            )
            .map_err(|error| format!("query Codex thread metadata failed: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ThreadMeta {
                    id: row.get(0)?,
                    manual_name: row.get(1)?,
                    native_name: None,
                    title: row.get(2)?,
                    first_user_message: row.get(3)?,
                    cwd: row.get(4)?,
                    rollout_path: row.get(5)?,
                    updated_at: row.get(6)?,
                    pinned: row.get(7)?,
                })
            })
            .map_err(|error| format!("read Codex thread metadata failed: {error}"))?;
        let mut metadata: HashMap<String, ThreadMeta> = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("parse Codex thread metadata failed: {error}"))
            .map(|rows| rows.into_iter().map(|row| (row.id.clone(), row)).collect())?;
        for native in self.native_metadata.values() {
            if let Some(meta) = metadata.get_mut(&native.id) {
                meta.native_name = native.name.clone();
                if meta.cwd.as_deref().is_none_or(str::is_empty) {
                    meta.cwd = native.cwd.clone();
                }
                if meta.rollout_path.as_deref().is_none_or(str::is_empty) {
                    meta.rollout_path = native.path.clone();
                }
                meta.updated_at = meta.updated_at.max(native.updated_at);
            } else {
                let meta = ThreadMeta::from_app_server(native);
                metadata.insert(meta.id.clone(), meta);
            }
        }
        Ok(metadata)
    }

    fn refresh_native_metadata(&mut self, _now: SystemTime) {
        let Some(rx) = &self.native_rx else { return };
        while let Ok(result) = rx.try_recv() {
            match result {
                Ok(snapshot) => {
                    self.native_metadata = snapshot
                        .threads
                        .into_iter()
                        .map(|row| (row.id.clone(), row))
                        .collect();
                    if let Some(quota) = snapshot.quota {
                        *NATIVE_QUOTA
                            .get_or_init(|| RwLock::new(None))
                            .write()
                            .unwrap_or_else(|error| error.into_inner()) = Some(quota);
                    }
                }
                Err(error) => {
                    // app-server is optional. Keep the last successful title map,
                    // then fall back through state DB and rollout metadata.
                    log::debug!("codex: native app-server metadata unavailable: {error}");
                }
            }
        }
    }

    fn discover(&self) -> std::io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for path in walk(&self.sessions_root)? {
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("rollout-") {
                    files.push(path);
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
        Ok(files)
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

    fn cached_session_state(&mut self, path: &Path, now: SystemTime) -> Option<SessionState> {
        let metadata = path.metadata().ok()?;
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        let len = metadata.len();
        if let Some((cached_mtime, cached_len, state)) = self.cache.get(path) {
            if *cached_mtime == modified && *cached_len == len {
                return state.clone().filter(|state| session_is_live(state, now));
            }
        }

        let state = self.parse_session_state(path, now);
        self.cache
            .insert(path.to_path_buf(), (modified, len, state.clone()));
        state
    }

    fn session_to_task(
        &self,
        host: &SessionState,
        meta: Option<&ThreadMeta>,
        now: SystemTime,
    ) -> TaskSnapshot {
        let name = meta
            .map(thread_task_name)
            .or_else(|| host.first_user_message.as_deref().map(summarize_prompt))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| unnamed_task(&host.session_id));
        let mut task = TaskSnapshot::new(
            format!("codex:{}", host.session_id),
            name,
            TaskSource::Codex,
            host.status,
            DetectionConfidence::Hook,
            now,
        );
        task.project = meta
            .and_then(|meta| project_from_cwd(meta.cwd.as_deref()))
            .or_else(|| host.project.clone());
        task.cli = Some("codex".into());
        task.session_id = Some(host.session_id.clone());
        task.quota_5h_used_pct = host.quota_5h_used_pct;
        task.quota_5h_resets_at = host.quota_5h_resets_at;
        task.quota_1w_used_pct = host.quota_1w_used_pct;
        task.quota_1w_resets_at = host.quota_1w_resets_at;
        task.started_at = host.started_at;
        task.updated_at = host.updated_at;
        task.message = host.last_message.clone();
        task.activity = host.activity.clone();
        task.turn_key = host.turn_key.clone();
        task.pinned = meta.is_some_and(|meta| meta.pinned);
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

fn normalize_windows_path(path: &str) -> PathBuf {
    PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(path))
}

/// Derived live state of one Codex session.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub status: TaskStatus,
    pub started_at: Option<SystemTime>,
    pub updated_at: SystemTime,
    pub project: Option<String>,
    pub last_message: Option<String>,
    pub activity: Option<String>,
    pub session_id: String,
    pub turn_key: Option<String>,
    pub first_user_message: Option<String>,
    pub quota_5h_used_pct: Option<f64>,
    pub quota_5h_resets_at: Option<i64>,
    pub quota_1w_used_pct: Option<f64>,
    pub quota_1w_resets_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct ThreadMeta {
    id: String,
    manual_name: Option<String>,
    native_name: Option<String>,
    title: Option<String>,
    first_user_message: Option<String>,
    cwd: Option<String>,
    rollout_path: Option<String>,
    updated_at: i64,
    pinned: bool,
}

/// Metadata exposed by Codex's own app-server. Its `name` field is the same
/// title shown by the native sidebar, including user renames.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppThreadMeta {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    updated_at: i64,
}

#[derive(Debug)]
struct NativeCodexSnapshot {
    threads: Vec<AppThreadMeta>,
    quota: Option<CodexQuota>,
}

fn query_codex_app_server() -> Result<NativeCodexSnapshot, String> {
    let executable = find_codex_executable()
        .ok_or_else(|| "Codex app-server executable was not found".to_string())?;
    let mut child = Command::new(executable)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("start app-server failed: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "app-server stdin was unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "app-server stdout was unavailable".to_string())?;

    let initialize = serde_json::json!({
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "taskpulse",
                "title": "TaskPulse",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": { "experimentalApi": true }
        }
    });
    let list = serde_json::json!({
        "id": 2,
        "method": "thread/list",
        "params": {
            "limit": 100,
            "sortKey": "updated_at",
            "sortDirection": "desc",
            "archived": false,
            "useStateDbOnly": false
        }
    });
    let rate_limits = serde_json::json!({
        "id": 3,
        "method": "account/rateLimits/read",
        "params": null
    });
    for message in [
        initialize,
        serde_json::json!({"method": "initialized", "params": {}}),
        list,
        rate_limits,
    ] {
        writeln!(stdin, "{message}")
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("write app-server request failed: {error}"))?;
    }

    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if matches!(value.get("id").and_then(|id| id.as_i64()), Some(2 | 3)) {
                let _ = sender.send(value);
            }
        }
    });

    let deadline = Instant::now() + APP_SERVER_TIMEOUT;
    let mut list_response = None;
    let mut quota_response = None;
    while Instant::now() < deadline && (list_response.is_none() || quota_response.is_none()) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(response) = receiver.recv_timeout(remaining) else {
            break;
        };
        match response.get("id").and_then(|id| id.as_i64()) {
            Some(2) => list_response = Some(response),
            Some(3) => quota_response = Some(response),
            _ => {}
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let threads = parse_app_server_thread_list(
        list_response.ok_or_else(|| "app-server thread/list timed out".to_string())?,
    )?;
    let quota = quota_response.and_then(|response| parse_app_server_rate_limits(response).ok());
    Ok(NativeCodexSnapshot { threads, quota })
}

fn parse_app_server_thread_list(value: serde_json::Value) -> Result<Vec<AppThreadMeta>, String> {
    if let Some(error) = value.get("error") {
        return Err(format!("app-server thread/list error: {error}"));
    }
    serde_json::from_value(
        value
            .get("result")
            .and_then(|result| result.get("data"))
            .cloned()
            .ok_or_else(|| "app-server thread/list returned no data".to_string())?,
    )
    .map_err(|error| format!("parse app-server thread/list failed: {error}"))
}

fn parse_app_server_rate_limits(value: serde_json::Value) -> Result<CodexQuota, String> {
    if let Some(error) = value.get("error") {
        return Err(format!("app-server rate limits error: {error}"));
    }
    let limits = value
        .get("result")
        .and_then(|result| result.get("rateLimits"))
        .ok_or_else(|| "app-server rate limits returned no data".to_string())?;
    let primary = limits
        .get("primary")
        .ok_or_else(|| "app-server rate limits returned no 5-hour window".to_string())?;
    let secondary = limits
        .get("secondary")
        .ok_or_else(|| "app-server rate limits returned no weekly window".to_string())?;
    Ok(CodexQuota {
        used_5h_pct: primary
            .get("usedPercent")
            .and_then(|value| value.as_f64())
            .ok_or_else(|| "5-hour usedPercent was missing".to_string())?,
        resets_5h_at: primary.get("resetsAt").and_then(|value| value.as_i64()),
        used_1w_pct: secondary
            .get("usedPercent")
            .and_then(|value| value.as_f64())
            .ok_or_else(|| "weekly usedPercent was missing".to_string())?,
        resets_1w_at: secondary.get("resetsAt").and_then(|value| value.as_i64()),
    })
}

/// Apply the account-level app-server quota to every Codex task. Rollout
/// token events are per-thread and may be stale, while this endpoint is the
/// same authoritative account source used by Codex Desktop.
pub(crate) fn apply_authoritative_quota(tasks: &mut [TaskSnapshot]) {
    let quota = NATIVE_QUOTA
        .get()
        .and_then(|quota| *quota.read().unwrap_or_else(|error| error.into_inner()));
    let Some(quota) = quota else { return };
    for task in tasks
        .iter_mut()
        .filter(|task| task.source == TaskSource::Codex)
    {
        task.quota_5h_used_pct = Some(quota.used_5h_pct);
        task.quota_5h_resets_at = quota.resets_5h_at;
        task.quota_1w_used_pct = Some(quota.used_1w_pct);
        task.quota_1w_resets_at = quota.resets_1w_at;
    }
}

fn find_codex_executable() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("TASKPULSE_CODEX_EXECUTABLE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let root = dirs_next::data_local_dir()?.join("OpenAI/Codex/bin");
    walk(&root)
        .ok()?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
        })
        .max_by_key(|path| {
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH)
        })
}

impl ThreadMeta {
    fn from_app_server(meta: &AppThreadMeta) -> Self {
        Self {
            id: meta.id.clone(),
            manual_name: None,
            native_name: meta.name.clone(),
            title: None,
            first_user_message: meta.preview.clone(),
            cwd: meta.cwd.clone(),
            rollout_path: meta.path.clone(),
            updated_at: meta.updated_at,
            pinned: false,
        }
    }
}

fn project_from_cwd(cwd: Option<&str>) -> Option<String> {
    cwd.and_then(|cwd| {
        Path::new(cwd)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    })
}

fn thread_task_name(meta: &ThreadMeta) -> String {
    if let Some(name) = meta
        .manual_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    {
        return clean_task_name(name);
    }
    if let Some(name) = meta
        .native_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    {
        return clean_task_name(name);
    }
    let title = meta
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty());
    let first = meta
        .first_user_message
        .as_deref()
        .filter(|message| !message.trim().is_empty());
    match (title, first) {
        // Current Codex builds sometimes persist the raw first prompt as
        // `title`; treat that as fallback content, not a formal sidebar name.
        (Some(title), Some(first)) if title.trim() == first.trim() => summarize_prompt(first),
        (Some(title), _) => clean_task_name(title),
        (None, Some(first)) => summarize_prompt(first),
        _ => unnamed_task(&meta.id),
    }
}

fn unnamed_task(session_id: &str) -> String {
    let short: String = session_id.chars().take(8).collect();
    format!("未命名任务 · {short}")
}

fn summarize_prompt(prompt: &str) -> String {
    let cleaned = prompt
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with('<')
                && !line.starts_with("Files mentioned")
        })
        .unwrap_or(prompt);
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let first_clause = collapsed
        .split(['。', '！', '？', ';', '；', '\n'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(&collapsed)
        .trim();
    clean_task_name(first_clause)
}

fn session_is_live(state: &SessionState, now: SystemTime) -> bool {
    let age = now.duration_since(state.updated_at).unwrap_or_default();
    match state.status {
        TaskStatus::Running => age <= RUNNING_TTL,
        TaskStatus::WaitingInput => age <= WAITING_TTL,
        _ => false,
    }
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
    let mut started_at = None;
    let mut updated_at = now;
    let mut last_message: Option<String> = None;
    let mut activity: Option<String> = None;
    let mut session_id = default_session_id.to_string();
    let mut turn_key: Option<String> = None;
    let mut first_user_message: Option<String> = None;
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
            "task_started" => {
                status = TaskStatus::Running;
                started_at = ts;
                activity = None;
                if let Some(t) = ts {
                    updated_at = t;
                }
                turn_key = payload
                    .get("turn_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| value_timestamp_key(ts));
            }
            "user_message" => {
                if status != TaskStatus::Running {
                    started_at = ts;
                    turn_key = value_timestamp_key(ts);
                    activity = None;
                }
                status = TaskStatus::Running;
                if let Some(t) = ts {
                    updated_at = t;
                }
                if first_user_message.is_none() {
                    first_user_message = payload
                        .get("message")
                        .and_then(|value| value.as_str())
                        .map(str::to_string);
                }
                if turn_key.is_none() {
                    turn_key = value_timestamp_key(ts);
                }
            }
            "agent_message" => {
                if status != TaskStatus::WaitingInput {
                    status = TaskStatus::Running;
                }
                if let Some(t) = ts {
                    updated_at = t;
                }
                if let Some(msg) = payload.get("message").and_then(|v| v.as_str()) {
                    last_message = Some(truncate(msg, 120));
                }
            }
            "token_count" => {
                if status != TaskStatus::WaitingInput {
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
                if status != TaskStatus::WaitingInput {
                    status = TaskStatus::Running;
                }
                if let Some(t) = ts {
                    updated_at = t;
                }
            }
            "function_call" | "custom_tool_call" => {
                status = TaskStatus::Running;
                if let Some(t) = ts {
                    updated_at = t;
                }
                activity = summarize_tool_activity(payload);
            }
            "task_complete" => {
                status = TaskStatus::WaitingInput;
                activity = None;
                if let Some(t) = payload
                    .get("completed_at")
                    .and_then(parse_rfc3339_value)
                    .or(ts)
                {
                    updated_at = t;
                }
                if let Some(msg) = payload.get("last_agent_message").and_then(|v| v.as_str()) {
                    last_message = Some(truncate(msg, 120));
                }
            }
            "turn_aborted" | "thread_rolled_back" => {
                status = TaskStatus::WaitingInput;
                activity = None;
                if let Some(t) = ts {
                    updated_at = t;
                }
            }
            _ => {}
        }
    }

    let state = SessionState {
        status,
        started_at,
        updated_at,
        project,
        last_message,
        activity,
        session_id,
        turn_key,
        first_user_message,
        quota_5h_used_pct,
        quota_5h_resets_at,
        quota_1w_used_pct,
        quota_1w_resets_at,
    };
    session_is_live(&state, now).then_some(state)
}

fn summarize_tool_activity(payload: &serde_json::Value) -> Option<String> {
    let name = payload
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("tool");
    let arguments = payload
        .get("arguments")
        .or_else(|| payload.get("input"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let command = serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("command")
                .or_else(|| value.get("cmd"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .or_else(|| extract_js_string(arguments, "cmd"))
        .or_else(|| extract_js_string(arguments, "command"));
    let label = match name {
        "exec" | "exec_command" | "shell_command" => "执行命令",
        "apply_patch" => "应用代码修改",
        "update_plan" => "更新任务计划",
        "web_search" | "search_query" => "搜索资料",
        other => other,
    };
    Some(match command {
        Some(command) if !command.trim().is_empty() => {
            format!(
                "{label} · {}",
                truncate(&collapse_whitespace(&command), 180)
            )
        }
        _ => label.to_string(),
    })
}

fn extract_js_string(input: &str, key: &str) -> Option<String> {
    let start = input.find(&format!("{key}:"))? + key.len() + 1;
    let value = input[start..].trim_start();
    if !value.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (index, ch) in value[1..].char_indices() {
        if ch == '"' && !escaped {
            let quoted = &value[..index + 2];
            return serde_json::from_str::<String>(quoted).ok();
        }
        escaped = ch == '\\' && !escaped;
        if ch != '\\' {
            escaped = false;
        }
    }
    None
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn value_timestamp_key(timestamp: Option<SystemTime>) -> Option<String> {
    timestamp.map(|timestamp| {
        let millis = timestamp
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("turn:{millis}")
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
    let mut titles: HashMap<String, (String, String, bool)> = HashMap::new();
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("META\t") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
                if let (Some(id), Some(t)) = (
                    v.get("id").and_then(|x| x.as_str()),
                    v.get("t").and_then(|x| x.as_str()),
                ) {
                    if !t.trim().is_empty() {
                        let cwd = v
                            .get("cwd")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        let pinned = v.get("p").and_then(|x| x.as_bool()).unwrap_or(false);
                        titles.insert(id.to_string(), (t.to_string(), cwd, pinned));
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
                &buf,
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
        &buf,
        now,
        ssh_host,
        host_id,
        &titles,
        &mut out,
    );
    let emitted: std::collections::HashSet<String> = out
        .iter()
        .filter_map(|task| task.session_id.clone())
        .collect();
    for (id, (title, cwd, pinned)) in &titles {
        if !*pinned || emitted.contains(id) || title.trim().is_empty() {
            continue;
        }
        let mut task = TaskSnapshot::new(
            format!("codex-remote:{host_id}:{id}"),
            clean_task_name(title),
            TaskSource::Codex,
            TaskStatus::Idle,
            DetectionConfidence::Hook,
            now,
        )
        .with_host(ssh_host.to_string())
        .with_remote();
        task.project = project_from_cwd(Some(cwd));
        task.session_id = Some(id.clone());
        task.pinned = true;
        out.push(task);
    }
    // Metadata is lookup-only. Do not synthesize an Idle task for every
    // non-archived thread in state_5.sqlite: that list can contain hundreds
    // of dormant sessions and bypasses both the rollout limit and lifecycle
    // TTL. Only the recent rollout bodies above are eligible for display.
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
        || t.starts_with("<environment_context")
        || t.starts_with("hello") && t.len() < 20
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

#[allow(clippy::too_many_arguments)]
fn flush_codex(
    slug: &Option<String>,
    uuid: &Option<String>,
    buf: &str,
    now: SystemTime,
    ssh_host: &str,
    host_id: &str,
    titles: &HashMap<String, (String, String, bool)>,
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
        let title = titles.get(&sid).map(|(t, _, _)| t.as_str());
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
        // Remote tails usually do not include the file's early session_meta
        // record. The authoritative cwd travels with META; use it before the
        // date-directory slug from the rollout header, otherwise project
        // scoping can incorrectly filter a valid task (for example HyperMem).
        t.project = titles
            .get(&sid)
            .and_then(|(_, cwd, _)| project_from_cwd(Some(cwd)))
            .or(state.project)
            .or_else(|| Some(slug.clone()));
        t.updated_at = state.updated_at;
        t.started_at = state.started_at;
        t.message = state.last_message;
        t.activity = state.activity;
        t.cli = Some("codex".into());
        t.session_id = Some(sid);
        t.turn_key = state.turn_key;
        t.pinned = titles
            .get(t.session_id.as_deref().unwrap_or(""))
            .is_some_and(|(_, _, pinned)| *pinned);
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
        let discovered = match self.discover() {
            Ok(files) => files,
            Err(error) => {
                let message = format!("session discovery failed: {error}");
                log::warn!("codex: {message}");
                let tasks = self
                    .last_tasks
                    .iter()
                    .cloned()
                    .map(|mut task| {
                        task.stale = true;
                        task
                    })
                    .collect();
                return ProviderOutput::degraded(tasks, message);
            }
        };
        let metadata = match self.thread_metadata(now) {
            Ok(metadata) => metadata,
            Err(error) => {
                log::warn!("codex: {error}; falling back to rollout metadata");
                self.native_metadata
                    .values()
                    .map(ThreadMeta::from_app_server)
                    .map(|meta| (meta.id.clone(), meta))
                    .collect()
            }
        };
        let now_secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        let eligible_paths: std::collections::HashSet<PathBuf> = metadata
            .values()
            .filter(|meta| {
                meta.pinned
                    || now_secs.saturating_sub(meta.updated_at) <= DISCOVERY_TTL.as_secs() as i64
            })
            .filter_map(|meta| meta.rollout_path.as_deref())
            .map(normalize_windows_path)
            .collect();
        let files: Vec<PathBuf> = if eligible_paths.is_empty() {
            discovered
        } else {
            discovered
                .into_iter()
                .filter(|path| eligible_paths.contains(path))
                .collect()
        };
        let mut tasks = Vec::new();
        let mut live: std::collections::HashMap<String, SystemTime> = Default::default();
        for path in &files {
            if let Some(state) = self.cached_session_state(path, now) {
                live.insert(state.session_id.clone(), state.updated_at);
                tasks.push(self.session_to_task(&state, metadata.get(&state.session_id), now));
            }
        }
        let emitted: std::collections::HashSet<String> = tasks
            .iter()
            .filter_map(|task| task.session_id.clone())
            .collect();
        for meta in metadata.values().filter(|meta| meta.pinned) {
            if emitted.contains(&meta.id) {
                continue;
            }
            let mut task = TaskSnapshot::new(
                format!("codex:{}", meta.id),
                thread_task_name(meta),
                TaskSource::Codex,
                TaskStatus::Idle,
                DetectionConfidence::Hook,
                now,
            );
            task.project = project_from_cwd(meta.cwd.as_deref());
            task.session_id = Some(meta.id.clone());
            task.pinned = true;
            tasks.push(task);
        }
        let selected: std::collections::HashSet<PathBuf> = files.into_iter().collect();
        self.cache.retain(|path, _| selected.contains(path));
        propagate_freshest_quota(&mut tasks);

        // Sessions we previously knew but that vanished / went stale → no
        // terminal snapshot needed (the store's TTL handles the drop). We only
        // track them so a re-appearing session keeps its identity.
        self.seen = live;
        self.last_tasks = tasks.clone();
        ProviderOutput::tasks(tasks)
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

    #[test]
    fn post_completion_usage_event_does_not_restart_turn_or_reset_elapsed_time() {
        let start = recent_rfc3339(90);
        let end = recent_rfc3339(5);
        let body = format!(
            "{}\n{}\n{}\n{}\n",
            header("/project"),
            serde_json::json!({"timestamp":start,"payload":{"type":"task_started","turn_id":"turn-1"}}),
            serde_json::json!({"timestamp":end,"payload":{"type":"task_complete"}}),
            serde_json::json!({"timestamp":end,"payload":{"type":"token_count"}})
        );
        let state = parse_codex_rollout(&body, SystemTime::now(), "id").unwrap();
        assert_eq!(state.status, TaskStatus::WaitingInput);
        assert_eq!(
            state.started_at,
            parse_rfc3339_value(&serde_json::json!(start))
        );
    }

    #[test]
    fn worker_to_attention_turn_completion_and_resume() {
        use crate::attention::AttentionTracker;
        use crate::providers::worker::Worker;
        use crate::store::TaskStore;
        use crate::transition::{TaskEvent, TransitionEngine};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-live.jsonl");
        let event = |kind: &str, turn: &str| {
            format!(
                "{}\n",
                serde_json::json!({
                    "timestamp": recent_rfc3339(0), "payload": {"type": kind, "turn_id": turn}
                })
            )
        };
        std::fs::write(
            &path,
            format!("{}{}", header("/project"), event("task_started", "turn-1")),
        )
        .unwrap();
        let mut provider = CodexProvider::with_root(root.path().to_path_buf());
        let worker = Worker::spawn(Duration::from_secs(60), move |_| {
            provider.poll(SystemTime::now())
        });
        let mut store = TaskStore::default();
        let mut engine = TransitionEngine::new();
        let mut attention = AttentionTracker::open(root.path().join("attention.json"));
        let sources = std::collections::HashSet::from([TaskSource::Codex]);
        let first = worker
            .observations
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let merged = store.merge_sources(first.output.tasks, &sources, SystemTime::now());
        assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
        assert_eq!(attention.reconcile(merged)[0].status, TaskStatus::Running);

        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(event("task_complete", "turn-1").as_bytes())
            .unwrap();
        worker.request_refresh();
        let completed = worker
            .observations
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let mut merged = store.merge_sources(completed.output.tasks, &sources, SystemTime::now());
        let events = engine.apply(&merged, &[], SystemTime::now());
        assert!(matches!(
            events.as_slice(),
            [TaskEvent::WaitingInput { .. }]
        ));
        assert!(attention.reconcile(merged.clone())[0].unread);
        assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
        merged[0].name = "Native renamed task".into();
        assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
        let renamed = attention.reconcile(merged);
        assert_eq!(renamed[0].name, "Native renamed task");
        assert!(attention.mark_viewed(&renamed[0].id).unwrap().is_empty());

        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(event("task_started", "turn-2").as_bytes())
            .unwrap();
        worker.request_refresh();
        let resumed = worker
            .observations
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let merged = store.merge_sources(resumed.output.tasks, &sources, SystemTime::now());
        assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
        let display = attention.reconcile(merged);
        assert_eq!(display[0].status, TaskStatus::Running);
        assert_eq!(display[0].id, renamed[0].id);
    }

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
    fn latest_agent_message_is_displayed() {
        let ts = recent_rfc3339(60);
        let body = format!(
            "{}{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"first\"}}}}\n{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"latest\"}}}}\n",
            header("/home/user/proj")
        );
        let (_dir, path) = temp_rollout(&body);
        let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
        let state = provider
            .parse_session_state(&path, SystemTime::now())
            .unwrap();
        assert_eq!(state.last_message.as_deref(), Some("latest"));
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
        let files = provider.discover().unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn unchanged_rollout_reuses_cached_parse() {
        let root = tempfile::tempdir().unwrap();
        let body = format!(
            "{}{{\"timestamp\":\"{}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t1\"}}}}\n",
            header("/home/user/original-project"),
            recent_rfc3339(60)
        );
        let path = root.path().join("rollout-cache.jsonl");
        std::fs::write(&path, body).unwrap();
        let mut provider = CodexProvider::with_root(root.path().to_path_buf());

        let first = provider.poll(SystemTime::now());
        assert_eq!(first.tasks[0].project.as_deref(), Some("original-project"));
        provider
            .cache
            .get_mut(&path)
            .unwrap()
            .2
            .as_mut()
            .unwrap()
            .project = Some("cached-project".into());

        let second = provider.poll(SystemTime::now());
        assert_eq!(second.tasks[0].project.as_deref(), Some("cached-project"));
    }

    #[test]
    fn codex_section_builds_remote_tasks() {
        let section = format!(
            "META\t{{\"id\":\"sess-1\",\"t\":\"研究 Game 记忆机制\",\"cwd\":\"/home/ai/projects/Game\"}}\n===S:08===F:rollout-2026-08-27T23-12-16-sess-1.jsonl===\n{}\n{}\n",
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
        assert_eq!(t.project.as_deref(), Some("Game"));
    }

    #[test]
    fn metadata_only_remote_threads_are_not_synthesized_as_idle_tasks() {
        let section =
            "META\t{\"id\":\"old-session\",\"t\":\"Dormant task\",\"cwd\":\"/tmp/project\"}\n";
        let tasks = tasks_from_codex_section(section, "Server3", "server3", SystemTime::now());
        assert!(tasks.is_empty());
    }

    #[test]
    fn app_server_name_has_priority_over_prompt_title() {
        let meta = ThreadMeta {
            id: "01abcdef".into(),
            manual_name: None,
            native_name: Some("评审 AgentTray 监控界面".into()),
            title: Some("请你 review 一下这个项目并指出问题".into()),
            first_user_message: Some("请你 review 一下这个项目并指出问题".into()),
            cwd: Some(r"D:\Project\agenttray".into()),
            rollout_path: None,
            updated_at: 0,
            pinned: false,
        };
        assert_eq!(thread_task_name(&meta), "评审 AgentTray 监控界面");
    }

    #[test]
    fn parses_native_app_server_thread_list() {
        let response = serde_json::json!({
            "id": 2,
            "result": {
                "data": [{
                    "id": "01abcdef",
                    "name": "Implement TaskPulse Windows MVP",
                    "preview": "Long first user prompt",
                    "cwd": "D:\\Project\\agenttray",
                    "path": "C:\\Users\\tester\\.codex\\sessions\\rollout.jsonl",
                    "updatedAt": 1_788_000_000
                }],
                "nextCursor": null
            }
        });
        let rows = parse_app_server_thread_list(response).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].name.as_deref(),
            Some("Implement TaskPulse Windows MVP")
        );
        assert_eq!(rows[0].cwd.as_deref(), Some(r"D:\Project\agenttray"));
    }

    #[test]
    fn parses_native_rate_limits_as_used_percent() {
        let quota = parse_app_server_rate_limits(serde_json::json!({
            "id": 3,
            "result": {
                "rateLimits": {
                    "primary": { "usedPercent": 3, "resetsAt": 1788110671 },
                    "secondary": { "usedPercent": 31, "resetsAt": 1788660994 }
                }
            }
        }))
        .unwrap();
        assert_eq!(quota.used_5h_pct, 3.0);
        assert_eq!(quota.used_1w_pct, 31.0);
        assert_eq!(quota.resets_5h_at, Some(1788110671));
        assert_eq!(quota.resets_1w_at, Some(1788660994));
    }

    #[test]
    fn custom_exec_call_is_exposed_as_current_activity() {
        let ts = recent_rfc3339(1);
        let body = format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"custom_tool_call\",\"name\":\"exec\",\"arguments\":\"const r=await tools.exec_command({{cmd:\\\"git diff --check\\\",workdir:\\\"/tmp/project\\\"}});\"}}}}\n"
        );
        let state = parse_codex_rollout(&body, SystemTime::now(), "session").unwrap();
        assert_eq!(state.status, TaskStatus::Running);
        assert_eq!(
            state.activity.as_deref(),
            Some("执行命令 · git diff --check")
        );
    }
}
