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
const APP_SERVER_REFRESH: Duration = Duration::from_secs(10);
const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(12);
const PROJECT_LEN: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub(crate) struct CodexQuota {
    pub used_5h_pct: f64,
    pub resets_5h_at: Option<i64>,
    pub used_1w_pct: f64,
    pub resets_1w_at: Option<i64>,
}

/// Account quota is not a task property: remote SSH users can have different
/// accounts, and idle rollout logs are not a live usage endpoint.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub(crate) struct AccountQuota {
    quota: Option<CodexQuota>,
    pub(crate) observed_at: Option<u64>,
    error: Option<String>,
    stale: bool,
    refreshing: bool,
}

static NATIVE_QUOTA: OnceLock<RwLock<AccountQuota>> = OnceLock::new();
static NATIVE_REFRESH: OnceLock<std::sync::Mutex<Option<mpsc::SyncSender<()>>>> = OnceLock::new();

#[tauri::command]
pub(crate) fn get_codex_quota() -> AccountQuota {
    let state = NATIVE_QUOTA
        .get_or_init(|| RwLock::new(AccountQuota::default()))
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    quota_at(state, SystemTime::now())
}

fn quota_at(mut state: AccountQuota, now: SystemTime) -> AccountQuota {
    let now = epoch_millis(now);
    state.stale = state.error.is_some()
        || state
            .observed_at
            .is_none_or(|at| now.saturating_sub(at) > 60_000);
    state
}

pub(crate) fn request_quota_refresh() {
    if let Some(slot) = NATIVE_REFRESH.get() {
        if let Some(sender) = slot.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = sender.try_send(());
        }
    }
}

fn epoch_millis(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn record_quota(state: &mut AccountQuota, result: Result<CodexQuota, String>, now: SystemTime) {
    state.refreshing = false;
    match result {
        Ok(quota) => {
            state.quota = Some(quota);
            state.observed_at = Some(epoch_millis(now));
            state.error = None;
        }
        Err(error) => state.error = Some(error),
    }
}

pub struct CodexProvider {
    sessions_root: PathBuf,
    identity_prefix: String,
    linux_share: Option<PathBuf>,
    external_metadata: Option<HashMap<String, ThreadMeta>>,
    state_db: Option<PathBuf>,
    native_rx: Option<mpsc::Receiver<Result<NativeCodexSnapshot, String>>>,
    /// session id → last-seen mtime, so we can emit a terminal snapshot when a
    /// session stops being updated (it finished).
    seen: std::collections::HashMap<String, SystemTime>,
    /// Append-only rollout cursors plus folded state. Unchanged files reuse
    /// state; changed files read new complete lines, with reset on replacement.
    cache: std::collections::HashMap<PathBuf, RolloutCache>,
    cache_policy: super::parser_cache::Policy,
    read_bytes: u64,
    native_metadata: HashMap<String, AppThreadMeta>,
    title_index: super::jsonl_tail::JsonlTail,
    indexed_titles: HashMap<String, String>,
    last_tasks: Vec<TaskSnapshot>,
}

struct RolloutCache {
    tail: super::jsonl_tail::JsonlTail,
    state: Option<SessionState>,
}
impl Default for RolloutCache {
    fn default() -> Self {
        Self {
            tail: super::jsonl_tail::JsonlTail::ignoring_envelopes(&["compacted", "world_state"]),
            state: None,
        }
    }
}

impl CodexProvider {
    /// WSL transport supplies metadata/path mapping, not a second lifecycle.
    pub(super) fn with_wsl_root(root: PathBuf, share: PathBuf, instance: &str) -> Self {
        let mut provider = Self::with_root(root.join("sessions"));
        provider.identity_prefix = format!("{instance}:codex");
        provider.linux_share = Some(share);
        provider.external_metadata = Some(Default::default());
        provider
    }
    pub(super) fn set_external_metadata(&mut self, rows: &[super::codex_wsl::WslThread]) {
        self.external_metadata = Some(
            rows.iter()
                .map(|row| {
                    (
                        row.id.clone(),
                        ThreadMeta {
                            id: row.id.clone(),
                            manual_name: row.name.clone(),
                            native_name: row.sidebar_title.clone(),
                            title: row.title.clone(),
                            first_user_message: row.first_user_message.clone(),
                            cwd: row.cwd.clone(),
                            rollout_path: row.rollout_path.clone(),
                            updated_at: row.updated_at,
                            pinned: row.is_pinned,
                            internal: internal_guardian(&row.source),
                        },
                    )
                })
                .collect(),
        );
    }
    fn task_id(&self, id: &str) -> String {
        format!("{}:{id}", self.identity_prefix)
    }
    fn resolve_rollout_path(&self, path: &str) -> PathBuf {
        if let Some(share) = &self.linux_share {
            if path.starts_with('/') {
                return share.join(path.trim_start_matches('/'));
            }
        }
        normalize_windows_path(path)
    }

    pub fn new() -> Self {
        let home = dirs_next::home_dir().unwrap_or_default();
        let mut provider = Self::with_root(home.join(".codex/sessions"));
        let db = home.join(".codex/state_5.sqlite");
        provider.state_db = db.exists().then_some(db);
        let (tx, rx) = mpsc::sync_channel(1);
        let (refresh_tx, refresh_rx) = mpsc::sync_channel(1);
        *NATIVE_REFRESH
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(refresh_tx);
        provider.native_rx = Some(rx);
        std::thread::spawn(move || loop {
            if !crate::monitoring::get_monitoring_settings()
                .sources
                .iter()
                .any(|s| s.enabled && s.kind == crate::monitoring::SourceKind::Codex)
            {
                break;
            }
            let started = Instant::now();
            NATIVE_QUOTA
                .get_or_init(|| RwLock::new(AccountQuota::default()))
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .refreshing = true;
            let result = query_codex_app_server();
            let quota = match &result {
                Ok(snapshot) => snapshot.quota.clone(),
                Err(error) => Err(error.clone()),
            };
            record_quota(
                &mut NATIVE_QUOTA
                    .get_or_init(|| RwLock::new(AccountQuota::default()))
                    .write()
                    .unwrap_or_else(|error| error.into_inner()),
                quota,
                SystemTime::now(),
            );
            if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(result) {
                break;
            }
            let _ = refresh_rx.recv_timeout(APP_SERVER_REFRESH.saturating_sub(started.elapsed()));
            if matches!(refresh_rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
                break;
            }
            while refresh_rx.try_recv().is_ok() {}
        });
        provider
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self {
            sessions_root: root,
            identity_prefix: "codex".into(),
            linux_share: None,
            external_metadata: None,
            state_db: None,
            native_rx: None,
            seen: Default::default(),
            cache: Default::default(),
            cache_policy: Default::default(),
            read_bytes: 0,
            native_metadata: Default::default(),
            title_index: Default::default(),
            indexed_titles: Default::default(),
            last_tasks: Vec::new(),
        }
    }

    fn thread_metadata(&mut self, now: SystemTime) -> Result<HashMap<String, ThreadMeta>, String> {
        if let Some(metadata) = &self.external_metadata {
            return Ok(metadata.clone());
        }
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
                "SELECT id, name, title, first_user_message, cwd, rollout_path, updated_at, is_pinned, source \
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
                    internal: internal_guardian(
                        &serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(8)?)
                            .unwrap_or_default(),
                    ),
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
                    if let Ok(threads) = snapshot.threads {
                        self.native_metadata = threads
                            .into_iter()
                            .map(|row| (row.id.clone(), row))
                            .collect();
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

    fn refresh_title_index(&mut self) {
        // Desktop can preserve a sidebar name here while threads.name is null
        // and threads.title is still the original prompt. WSL supplies its own
        // index through external_metadata; do not read that file twice.
        if self.external_metadata.is_some() {
            return;
        }
        let Some(root) = self.sessions_root.parent() else {
            return;
        };
        let Ok(batch) = self.title_index.read(&root.join("session_index.jsonl")) else {
            return; // Optional metadata failure must not stop turn discovery.
        };
        if batch.reset {
            self.indexed_titles.clear();
        }
        for line in batch.body.lines() {
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if let (Some(id), Some(name)) = (
                row.get("id").and_then(|v| v.as_str()),
                row.get("thread_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty()),
            ) {
                self.indexed_titles.insert(id.to_owned(), name.to_owned());
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

    #[cfg(test)]
    fn parse_session_state(&self, path: &Path, now: SystemTime) -> Option<SessionState> {
        let body = std::fs::read_to_string(path).ok()?;
        parse_codex_rollout_unfiltered(&body, now, path.file_stem()?.to_str()?)
    }

    fn cached_session_state(&mut self, path: &Path, now: SystemTime) -> Option<SessionState> {
        let cache = self.cache.entry(path.to_path_buf()).or_default();
        let trusted = cache
            .tail
            .drain(path, |batch| {
                self.read_bytes = self.read_bytes.saturating_add(batch.bytes_read as u64);
                if batch.reset {
                    cache.state = None;
                }
                if !batch.body.trim().is_empty() {
                    let sid = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    cache.state = apply_codex_rollout(&batch.body, now, sid, cache.state.take());
                }
            })
            .ok()?;
        trusted.then(|| cache.state.clone()).flatten()
    }

    fn session_to_task(
        &self,
        host: &SessionState,
        meta: Option<&ThreadMeta>,
        now: SystemTime,
    ) -> TaskSnapshot {
        let name = meta
            .map(thread_task_name)
            .or_else(|| {
                self.indexed_titles
                    .get(&host.session_id)
                    .map(|name| clean_task_name(name))
            })
            .or_else(|| host.first_user_message.as_deref().map(summarize_prompt))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| unnamed_task(&host.session_id));
        let mut task = TaskSnapshot::new(
            self.task_id(&host.session_id),
            name,
            TaskSource::Codex,
            host.status,
            DetectionConfidence::Hook,
            now,
        );
        task.project = meta
            .and_then(|meta| project_from_cwd(meta.cwd.as_deref()))
            .or_else(|| host.project.clone());
        task.project_path = meta
            .and_then(|meta| meta.cwd.clone())
            .or_else(|| host.cwd.clone());
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
        task.phase = host.phase;
        task.turn_key = host.turn_key.clone();
        task.waiting_reason = host.waiting_reason;
        task.input_request_id = host.input_request_id.clone();
        task.pinned = meta.is_some_and(|meta| meta.pinned);
        task.internal = host.internal || meta.is_some_and(|meta| meta.internal);
        // A completed turn means the agent is waiting on the user — keep the
        // timestamp reflecting the moment the turn finished.
        if host.status == TaskStatus::WaitingInput {
            task.updated_at = host.updated_at;
            task.waiting_reason = host
                .waiting_reason
                .or(Some(crate::domain::WaitingReason::TurnEnded));
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
#[derive(Debug, Clone, PartialEq)]
pub struct SessionState {
    pub status: TaskStatus,
    pub waiting_reason: Option<crate::domain::WaitingReason>,
    pub input_request_id: Option<String>,
    pub started_at: Option<SystemTime>,
    pub updated_at: SystemTime,
    pub project: Option<String>,
    pub cwd: Option<String>,
    pub internal: bool,
    pub last_message: Option<String>,
    pub activity: Option<String>,
    pub phase: Option<crate::domain::ExecutionPhase>,
    active_tools: std::collections::BTreeMap<String, String>,
    confirmed_approvals: std::collections::BTreeMap<String, Option<String>>,
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
    internal: bool,
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
    #[serde(default)]
    source: serde_json::Value,
}

fn internal_guardian(source: &serde_json::Value) -> bool {
    // SQLite may carry JSON text while app-server / rollout carry an object.
    // Classify the native origin, never a task name or prompt substring.
    let parsed;
    let source = if let Some(raw) = source.as_str() {
        parsed = serde_json::from_str::<serde_json::Value>(raw).unwrap_or_default();
        &parsed
    } else {
        source
    };
    source.pointer("/subagent/other").and_then(|v| v.as_str()) == Some("guardian")
}

#[derive(Debug)]
struct NativeCodexSnapshot {
    threads: Result<Vec<AppThreadMeta>, String>,
    quota: Result<CodexQuota, String>,
}

fn query_codex_app_server() -> Result<NativeCodexSnapshot, String> {
    let executable = find_codex_executable()
        .ok_or_else(|| "Codex app-server executable was not found".to_string())?;
    let mut command = Command::new(executable);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: never disturb editor focus.
    }
    let mut child = command
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
    let threads = list_response
        .ok_or_else(|| "app-server thread/list timed out".to_string())
        .and_then(parse_app_server_thread_list);
    let quota = quota_response
        .ok_or_else(|| "app-server account/rateLimits/read timed out".to_string())
        .and_then(parse_app_server_rate_limits);
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
        .and_then(|result| {
            result
                .get("rateLimitsByLimitId")
                .and_then(|limits| limits.get("codex"))
                .or_else(|| result.get("rateLimits"))
        })
        .ok_or_else(|| "app-server rate limits returned no data".to_string())?;
    let primary = limits
        .get("primary")
        .ok_or_else(|| "app-server rate limits returned no 5-hour window".to_string())?;
    let secondary = limits
        .get("secondary")
        .ok_or_else(|| "app-server rate limits returned no weekly window".to_string())?;
    if limits
        .get("limitId")
        .and_then(|value| value.as_str())
        .is_some_and(|id| id != "codex")
    {
        return Err("app-server returned a different model's quota bucket".into());
    }
    for (window, expected) in [(primary, 300), (secondary, 10080)] {
        if window
            .get("windowDurationMins")
            .and_then(|value| value.as_i64())
            .is_some_and(|minutes| minutes != expected)
        {
            return Err("app-server quota window is not 5 hours / 7 days".into());
        }
    }
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

/// Remove legacy per-task quota from the UI. The account-level endpoint is
/// presented separately with its own successful-read timestamp and errors.
pub(crate) fn clear_legacy_quota(tasks: &mut [TaskSnapshot]) {
    for task in tasks
        .iter_mut()
        .filter(|task| task.source == TaskSource::Codex)
    {
        task.quota_5h_used_pct = None;
        task.quota_5h_resets_at = None;
        task.quota_1w_used_pct = None;
        task.quota_1w_resets_at = None;
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
            internal: internal_guardian(&meta.source),
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
                && !line.starts_with("Distinguish instructions in attached documents")
        })
        .unwrap_or(prompt);
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let first_clause = collapsed
        .split(['。', '！', '？', ';', '；', '\n'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(&collapsed)
        .trim();
    truncate(first_clause, 56)
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
    let state = parse_codex_rollout_unfiltered(body, now, default_session_id)?;
    session_is_live(&state, now).then_some(state)
}

fn parse_codex_rollout_unfiltered(
    body: &str,
    now: SystemTime,
    default_session_id: &str,
) -> Option<SessionState> {
    apply_codex_rollout(body, now, default_session_id, None)
}

fn apply_codex_rollout(
    body: &str,
    now: SystemTime,
    default_session_id: &str,
    previous: Option<SessionState>,
) -> Option<SessionState> {
    if body.trim().is_empty() {
        return previous;
    }
    let SessionState {
        mut project,
        mut cwd,
        mut internal,
        mut status,
        mut waiting_reason,
        mut input_request_id,
        mut started_at,
        mut updated_at,
        mut last_message,
        mut activity,
        mut phase,
        mut active_tools,
        mut confirmed_approvals,
        mut session_id,
        mut turn_key,
        mut first_user_message,
        mut quota_5h_used_pct,
        mut quota_5h_resets_at,
        mut quota_1w_used_pct,
        mut quota_1w_resets_at,
    } = previous.unwrap_or_else(|| SessionState {
        project: None,
        cwd: None,
        internal: false,
        status: TaskStatus::Unknown,
        waiting_reason: None,
        input_request_id: None,
        started_at: None,
        updated_at: now,
        last_message: None,
        activity: None,
        phase: None,
        active_tools: Default::default(),
        confirmed_approvals: Default::default(),
        session_id: default_session_id.into(),
        turn_key: None,
        first_user_message: None,
        quota_5h_used_pct: None,
        quota_5h_resets_at: None,
        quota_1w_used_pct: None,
        quota_1w_resets_at: None,
    });

    for line in body.lines() {
        let line = line.trim_start();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ts = value.get("timestamp").and_then(parse_rfc3339_value);
        // Exact public app-server messages, only when the source records them.
        if let (Some(method), Some(params)) = (
            value.get("method").and_then(|v| v.as_str()),
            value.get("params"),
        ) {
            if params.get("threadId").and_then(|v| v.as_str()) == Some(session_id.as_str()) {
                if params
                    .get("turnId")
                    .and_then(|v| v.as_str())
                    .zip(turn_key.as_deref())
                    .is_some_and(|(event, current)| event != current)
                {
                    continue;
                }
                let request = |v: &serde_json::Value| {
                    v.as_str()
                        .map(str::to_owned)
                        .or_else(|| v.as_i64().map(|n| n.to_string()))
                };
                match method {
                    "error" if explicit_quota_error(params) => {
                        status = TaskStatus::WaitingInput;
                        waiting_reason = Some(crate::domain::WaitingReason::QuotaExhausted);
                        input_request_id = turn_key.as_ref().map(|turn| format!("quota:{turn}"));
                        last_message = Some("额度已用完，请在原 Agent 检查额度后继续".into());
                        activity = None;
                        active_tools.clear();
                        phase = None;
                        if let Some(t) = ts {
                            updated_at = t;
                        }
                    }
                    "item/commandExecution/requestApproval"
                    | "item/fileChange/requestApproval"
                    | "item/permissions/requestApproval" => {
                        if let Some(id) = value.get("id").and_then(request) {
                            confirmed_approvals.insert(
                                id.clone(),
                                params
                                    .get("itemId")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_owned),
                            );
                            status = TaskStatus::WaitingInput;
                            waiting_reason = Some(crate::domain::WaitingReason::Approval);
                            input_request_id = Some(id);
                            phase = None;
                            if let Some(t) = ts {
                                updated_at = t;
                            }
                        }
                    }
                    "serverRequest/resolved" => {
                        if let Some(id) = params.get("requestId").and_then(request) {
                            if confirmed_approvals.remove(&id).is_some() {
                                if confirmed_approvals.is_empty() {
                                    status = TaskStatus::Running;
                                    waiting_reason = None;
                                    input_request_id = None;
                                    phase = Some(crate::domain::ExecutionPhase::AwaitingTool);
                                }
                                if let Some(t) = ts {
                                    updated_at = t;
                                }
                            }
                        }
                    }
                    _ => {}
                }
                if let Some(id) = confirmed_approvals.keys().next() {
                    status = TaskStatus::WaitingInput;
                    waiting_reason = Some(crate::domain::WaitingReason::Approval);
                    input_request_id = Some(id.clone());
                    phase = None;
                }
            }
            continue;
        }
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
        if cwd.is_none() {
            cwd = payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
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
            internal |= payload.get("source").is_some_and(internal_guardian);
        }

        let Some(ptype) = payload.get("type").and_then(|v| v.as_str()) else {
            continue;
        };

        // Asynchronous callbacks belong to their original turn. Never let an
        // old completion/abort/approval mutate whichever turn is now active.
        if ptype != "task_started"
            && payload
                .get("turn_id")
                .and_then(|v| v.as_str())
                .zip(turn_key.as_deref())
                .is_some_and(|(event, current)| event != current)
        {
            continue;
        }

        match ptype {
            "task_started" => {
                status = TaskStatus::Running;
                waiting_reason = None;
                input_request_id = None;
                started_at = ts;
                activity = None;
                active_tools.clear();
                confirmed_approvals.clear();
                phase = Some(crate::domain::ExecutionPhase::AwaitingModel);
                if let Some(t) = ts {
                    updated_at = t;
                }
                turn_key = payload
                    .get("turn_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| value_timestamp_key(ts));
            }
            "user_message" | "message"
                if ptype == "user_message"
                    || payload.get("role").and_then(|v| v.as_str()) == Some("user") =>
            {
                if status != TaskStatus::Running
                    && confirmed_approvals.is_empty()
                    && !matches!(
                        waiting_reason,
                        Some(
                            crate::domain::WaitingReason::Approval
                                | crate::domain::WaitingReason::Question
                                | crate::domain::WaitingReason::QuotaExhausted
                        )
                    )
                {
                    started_at = ts;
                    turn_key = value_timestamp_key(ts);
                    waiting_reason = None;
                    input_request_id = None;
                    status = TaskStatus::Running;
                    activity = None;
                    active_tools.clear();
                    phase = Some(crate::domain::ExecutionPhase::AwaitingModel);
                }
                if status != TaskStatus::WaitingInput {
                    status = TaskStatus::Running;
                }
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
            "agent_message" | "message"
                if ptype == "agent_message"
                    || payload.get("role").and_then(|v| v.as_str()) == Some("assistant") =>
            {
                if status != TaskStatus::WaitingInput {
                    status = TaskStatus::Running;
                }
                if let Some(t) = ts {
                    updated_at = t;
                }
                if let Some(msg) = payload.get("message").and_then(|v| v.as_str()).or_else(|| {
                    payload
                        .get("content")
                        .and_then(|v| v.as_array())
                        .and_then(|items| {
                            items
                                .iter()
                                .find_map(|item| item.get("text").and_then(|v| v.as_str()))
                        })
                }) {
                    last_message = Some(truncate(msg, 120));
                }
                if active_tools.is_empty() && status == TaskStatus::Running {
                    phase = Some(crate::domain::ExecutionPhase::Responding);
                }
            }
            "reasoning" | "agent_reasoning" | "agent_reasoning_raw_content" => {
                if status == TaskStatus::Running && active_tools.is_empty() {
                    phase = Some(crate::domain::ExecutionPhase::Thinking);
                    activity = None;
                    if let Some(t) = ts {
                        updated_at = t;
                    }
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
                if status != TaskStatus::WaitingInput {
                    status = TaskStatus::Running;
                }
                if let Some(t) = ts {
                    updated_at = t;
                }
                activity = summarize_tool_activity(payload);
                if let Some(id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    active_tools.insert(
                        id.to_owned(),
                        activity.clone().unwrap_or_else(|| "工具".into()),
                    );
                }
                phase = Some(crate::domain::ExecutionPhase::Tool);
                if payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|name| {
                        name == "request_user_input" || name.ends_with(".request_user_input")
                    })
                {
                    status = TaskStatus::WaitingInput;
                    waiting_reason = Some(crate::domain::WaitingReason::Question);
                    input_request_id = payload
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                }
            }
            "function_call_output" | "custom_tool_call_output" => {
                if let Some(id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    active_tools.remove(id);
                    activity = active_tools.values().next_back().cloned();
                    phase = (status == TaskStatus::Running).then_some(if active_tools.is_empty() {
                        crate::domain::ExecutionPhase::AwaitingModel
                    } else {
                        crate::domain::ExecutionPhase::Tool
                    });
                }
                if let Some(t) = ts {
                    updated_at = t;
                }
                if input_request_id.is_some()
                    && payload.get("call_id").and_then(|v| v.as_str())
                        == input_request_id.as_deref()
                {
                    status = TaskStatus::Running;
                    waiting_reason = None;
                    input_request_id = None;
                    phase = Some(if active_tools.is_empty() {
                        crate::domain::ExecutionPhase::AwaitingModel
                    } else {
                        crate::domain::ExecutionPhase::Tool
                    });
                }
            }
            "task_complete" => {
                if waiting_reason == Some(crate::domain::WaitingReason::QuotaExhausted) {
                    continue;
                }
                status = TaskStatus::WaitingInput;
                waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
                input_request_id = None;
                activity = None;
                active_tools.clear();
                confirmed_approvals.clear();
                phase = None;
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
                waiting_reason = Some(crate::domain::WaitingReason::Unknown);
                input_request_id = None;
                activity = None;
                active_tools.clear();
                confirmed_approvals.clear();
                phase = None;
                if let Some(t) = ts {
                    updated_at = t;
                }
            }
            "error" if explicit_quota_error(payload) => {
                status = TaskStatus::WaitingInput;
                waiting_reason = Some(crate::domain::WaitingReason::QuotaExhausted);
                input_request_id = turn_key.as_ref().map(|turn| format!("quota:{turn}"));
                last_message =
                    Some("额度已用完，请在原 Agent 检查账号、重置时间或切换可用额度后继续".into());
                activity = None;
                active_tools.clear();
                phase = None;
                if let Some(t) = ts {
                    updated_at = t;
                }
            }
            _ => {}
        }
        if let Some(id) = confirmed_approvals.keys().next() {
            status = TaskStatus::WaitingInput;
            waiting_reason = Some(crate::domain::WaitingReason::Approval);
            input_request_id = Some(id.clone());
            phase = None;
        }
    }

    let state = SessionState {
        status,
        waiting_reason,
        input_request_id,
        started_at,
        updated_at,
        project,
        cwd,
        internal,
        last_message,
        activity,
        phase,
        active_tools,
        confirmed_approvals,
        session_id,
        turn_key,
        first_user_message,
        quota_5h_used_pct,
        quota_5h_resets_at,
        quota_1w_used_pct,
        quota_1w_resets_at,
    };
    Some(state)
}

fn explicit_quota_error(value: &serde_json::Value) -> bool {
    if value
        .get("will_retry")
        .or_else(|| value.get("willRetry"))
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        return false;
    }
    let known = |v: &serde_json::Value| {
        v.as_str().is_some_and(|code| {
            matches!(
                code,
                "usage_limit_reached"
                    | "insufficient_quota"
                    | "usageLimitExceeded"
                    | "UsageLimitExceeded"
            )
        })
    };
    [
        value,
        value.get("error").unwrap_or(&serde_json::Value::Null),
    ]
    .iter()
    .any(|v| {
        ["code", "codex_error_info", "codexErrorInfo"]
            .iter()
            .any(|key| v.get(*key).is_some_and(known))
            || v.get("message")
                .and_then(|v| v.as_str())
                .is_some_and(|message| {
                    let message = message.to_ascii_lowercase();
                    message.starts_with("you've hit your usage limit")
                        || message.starts_with("you have hit your usage limit")
                })
    })
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

/// Remote metadata is origin evidence as well as a sidebar title lookup.
#[derive(serde::Deserialize)]
struct RemoteThreadMeta {
    id: String,
    #[serde(default, rename = "t")]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default, rename = "p")]
    pinned: bool,
    #[serde(default, rename = "u")]
    updated_at: u64,
    #[serde(default)]
    source: serde_json::Value,
    #[serde(skip)]
    internal: bool,
}

fn remote_thread_metadata(section: &str) -> HashMap<String, RemoteThreadMeta> {
    let mut metadata = HashMap::new();
    let mut internal_origins = std::collections::HashSet::new();
    for line in section.lines() {
        if let Some(raw) = line.strip_prefix("META\t") {
            if let Ok(mut meta) = serde_json::from_str::<RemoteThreadMeta>(raw) {
                if !meta.id.trim().is_empty() {
                    meta.internal = internal_guardian(&meta.source);
                    // Retain origin even if the native title is unavailable.
                    metadata.insert(meta.id.clone(), meta);
                }
            }
        } else if let Ok(row) = serde_json::from_str::<serde_json::Value>(line) {
            if row.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
                if let Some(payload) = row.get("payload") {
                    if payload.get("source").is_some_and(internal_guardian) {
                        if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
                            internal_origins.insert(id.to_owned());
                        }
                    }
                }
            }
        }
    }
    // Old DBs may lack source, but the supplied rollout header still proves
    // origin. Catalog and execution paths must agree on that classification.
    for meta in metadata.values_mut() {
        meta.internal |= internal_origins.contains(&meta.id);
    }
    metadata
}

/// Parse remote rollouts using the shared lifecycle parser, preserving native
/// origin even when logs are truncated or metadata has no title yet.
pub fn tasks_from_codex_section(
    section: &str,
    ssh_host: &str,
    host_id: &str,
    now: SystemTime,
) -> Vec<TaskSnapshot> {
    let titles = remote_thread_metadata(section);

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
    for (id, meta) in &titles {
        if !meta.pinned || emitted.contains(id) || meta.title.trim().is_empty() {
            continue;
        }
        let mut task = TaskSnapshot::new(
            format!("codex-remote:{host_id}:{id}"),
            clean_task_name(&meta.title),
            TaskSource::Codex,
            TaskStatus::Idle,
            DetectionConfidence::Hook,
            now,
        )
        .with_host(ssh_host.to_string())
        .with_remote();
        task.project = project_from_cwd(Some(&meta.cwd));
        task.project_path = Some(meta.cwd.clone());
        task.session_id = Some(id.clone());
        task.pinned = true;
        task.internal = meta.internal;
        out.push(task);
    }
    // Metadata is lookup-only. Do not synthesize an Idle task for every
    // non-archived thread in state_5.sqlite: that list can contain hundreds
    // of dormant sessions and bypasses both the rollout limit and lifecycle
    // TTL. Only the recent rollout bodies above are eligible for display.
    out
}

/// Native metadata is a discovery directory, not a request to import idle history.
pub fn catalog_from_codex_section(
    section: &str,
    ssh_host: &str,
    host_id: &str,
) -> Vec<TaskSnapshot> {
    remote_thread_metadata(section)
        .into_values()
        .filter_map(|meta| {
            let id = &meta.id;
            let title = &meta.title;
            if is_synthetic_title(title) {
                return None;
            }
            let mut task = TaskSnapshot::new(
                format!("codex-remote:{host_id}:{id}"),
                if title.trim().is_empty() {
                    unnamed_task(id)
                } else {
                    clean_task_name(title)
                },
                TaskSource::Codex,
                TaskStatus::Unknown,
                DetectionConfidence::Hook,
                UNIX_EPOCH
                    .checked_add(Duration::from_secs(meta.updated_at))
                    .unwrap_or(UNIX_EPOCH),
            )
            .with_host(ssh_host)
            .with_remote();
            task.project_path = (!meta.cwd.is_empty()).then_some(meta.cwd);
            task.project = project_from_cwd(task.project_path.as_deref());
            task.session_id = Some(id.to_string());
            task.internal = meta.internal;
            Some(task)
        })
        .collect()
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

fn is_synthetic_title(t: &str) -> bool {
    t.starts_with("The following is the Codex agent history")
        || t.starts_with("# Files mentioned by the user")
        || t.starts_with("PLEASE IMPLEMENT THIS PLAN")
        || t.starts_with("<environment_context")
        || t.starts_with("hello") && t.len() < 20
}

/// Preserve the complete native title; row ellipsis is presentation-only.
fn clean_task_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[allow(clippy::too_many_arguments)]
fn flush_codex(
    slug: &Option<String>,
    uuid: &Option<String>,
    buf: &str,
    now: SystemTime,
    ssh_host: &str,
    host_id: &str,
    titles: &HashMap<String, RemoteThreadMeta>,
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
        let meta = titles.get(&sid);
        let title = meta.map(|m| m.title.as_str());
        // Drop explicit synthetic sub-agent boilerplate, but never lose an
        // actual live task just because its title is temporarily unavailable.
        if title.is_some_and(is_synthetic_title) {
            return;
        }
        let name = title
            .filter(|title| !title.trim().is_empty())
            .map(clean_task_name)
            .or_else(|| state.first_user_message.as_deref().map(summarize_prompt))
            .unwrap_or_else(|| unnamed_task(&sid));
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
        t.internal = state.internal || meta.is_some_and(|m| m.internal);
        // Remote tails usually do not include the file's early session_meta
        // record. The authoritative cwd travels with META; use it before the
        // date-directory slug from the rollout header, otherwise project
        // scoping can incorrectly filter a valid task (for example HyperMem).
        t.project = titles
            .get(&sid)
            .and_then(|m| project_from_cwd(Some(&m.cwd)))
            .or(state.project)
            .or_else(|| Some(slug.clone()));
        t.updated_at = state.updated_at;
        t.project_path = meta
            .map(|m| m.cwd.clone())
            .filter(|cwd| !cwd.is_empty())
            .or(state.cwd);
        t.waiting_reason = state.waiting_reason;
        t.input_request_id = state.input_request_id;
        t.started_at = state.started_at;
        t.message = state.last_message;
        t.activity = state.activity;
        t.phase = state.phase;
        t.cli = Some("codex".into());
        t.session_id = Some(sid);
        t.turn_key = state.turn_key;
        t.pinned = titles
            .get(t.session_id.as_deref().unwrap_or(""))
            .is_some_and(|m| m.pinned);
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
        self.refresh_title_index();
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
        let mut metadata = match self.thread_metadata(now) {
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
        for meta in metadata.values_mut() {
            if meta
                .native_name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
            {
                meta.native_name = self.indexed_titles.get(&meta.id).cloned();
            }
        }
        let now_secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        let eligible_paths: std::collections::HashSet<PathBuf> = metadata
            .values()
            .filter(|meta| {
                meta.pinned
                    || self
                        .last_tasks
                        .iter()
                        .any(|t| t.session_id.as_deref() == Some(&meta.id) && parser_protected(t))
                    || crate::watcher::known_sessions().iter().any(|t| {
                        t.id == self.task_id(&meta.id)
                            && (t.unread || t.pinned || t.requires_action())
                    })
                    || now_secs.saturating_sub(meta.updated_at) <= DISCOVERY_TTL.as_secs() as i64
            })
            .filter_map(|meta| meta.rollout_path.as_deref())
            .map(|path| self.resolve_rollout_path(path))
            .collect();
        // Metadata is an enrichment/discovery hint, never an allowlist of
        // executions. A new/resumed rollout may precede the DB or app-server
        // update, or the DB may retain the previous rollout path.
        let files: Vec<PathBuf> = discovered
            .into_iter()
            .filter(|path| {
                let protected = self.cache.get(path).is_some_and(|c| {
                    c.state.as_ref().is_some_and(|s| {
                        s.status == TaskStatus::Running || !s.confirmed_approvals.is_empty()
                    })
                });
                self.cache_policy
                    .eligible(path, protected, self.cache.contains_key(path))
                    && (eligible_paths.contains(path)
                        || protected
                        || path
                            .metadata()
                            .and_then(|m| m.modified())
                            .is_ok_and(|modified| {
                                now.duration_since(modified).unwrap_or_default() <= DISCOVERY_TTL
                            }))
            })
            .collect();
        let mut tasks = Vec::new();
        let mut live: std::collections::HashMap<String, SystemTime> = Default::default();
        let mut sessions: HashMap<String, (PathBuf, SessionState)> = HashMap::new();
        for path in &files {
            if let Some(state) = self.cached_session_state(path, now) {
                // A header-only new file is not a lifecycle observation.
                if state.status == TaskStatus::Unknown {
                    continue;
                }
                let known = self.seen.contains_key(&state.session_id)
                    || metadata.get(&state.session_id).is_some_and(|m| m.pinned)
                    || crate::watcher::known_sessions().iter().any(|t| {
                        t.id == self.task_id(&state.session_id)
                            && (t.unread || t.pinned || t.requires_action())
                    });
                if !session_is_live(&state, now)
                    && !known
                    && !matches!(
                        state.waiting_reason,
                        Some(
                            crate::domain::WaitingReason::Approval
                                | crate::domain::WaitingReason::Question
                        )
                    )
                {
                    continue;
                }
                // Resuming a thread can create another rollout with the SAME
                // native ID. Emit one observation, not conflicting old/new
                // turns whose final state depends on file enumeration order.
                let replace = sessions
                    .get(&state.session_id)
                    .is_none_or(|(old_path, old)| {
                        state.updated_at > old.updated_at
                            || state.updated_at == old.updated_at
                                && metadata
                                    .get(&state.session_id)
                                    .and_then(|m| m.rollout_path.as_deref())
                                    .is_some_and(|canonical| {
                                        self.resolve_rollout_path(canonical) == *path
                                            && old_path != path
                                    })
                    });
                if replace {
                    sessions.insert(state.session_id.clone(), (path.clone(), state));
                }
            }
        }
        for (_, state) in sessions.into_values() {
            live.insert(state.session_id.clone(), state.updated_at);
            let mut task = self.session_to_task(&state, metadata.get(&state.session_id), now);
            if state.status == TaskStatus::Running
                && now.duration_since(state.updated_at).unwrap_or_default() > RUNNING_TTL
            {
                task.stale = true;
            }
            tasks.push(task);
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
                self.task_id(&meta.id),
                thread_task_name(meta),
                TaskSource::Codex,
                TaskStatus::Idle,
                DetectionConfidence::Hook,
                now,
            );
            task.project = project_from_cwd(meta.cwd.as_deref());
            task.session_id = Some(meta.id.clone());
            task.pinned = true;
            task.internal = meta.internal;
            tasks.push(task);
        }
        let selected: std::collections::HashSet<PathBuf> = files.into_iter().collect();
        self.cache.retain(|path, _| selected.contains(path));
        let keep = self.cache_policy.keep(
            self.cache
                .iter()
                .map(|(p, c)| {
                    let protected = !c.tail.trusted()
                        || c.state.as_ref().is_some_and(|s| {
                            s.status == TaskStatus::Running || !s.confirmed_approvals.is_empty()
                        });
                    let bytes = c.tail.retained_bytes()
                        + c.state.as_ref().map_or(0, |s| {
                            s.active_tools
                                .iter()
                                .map(|(k, v)| k.len() + v.len())
                                .sum::<usize>()
                                + s.first_user_message.as_ref().map_or(0, String::len)
                                + 2048
                        });
                    (p.clone(), protected, bytes)
                })
                .collect(),
            now,
        );
        self.cache.retain(|p, _| keep.contains(p));

        // Sessions we previously knew but that vanished / went stale → no
        // terminal snapshot needed (the store's TTL handles the drop). We only
        // track them so a re-appearing session keeps its identity.
        self.seen = live;
        // Compact business observations survive rebuildable buffer eviction.
        // Attention/pins/dedup remain owned by their existing persistent stores.
        for previous in &self.last_tasks {
            if !tasks.iter().any(|t| t.id == previous.id) && !parser_protected(previous) {
                let mut previous = previous.clone();
                if let Some(meta) = previous.session_id.as_ref().and_then(|id| metadata.get(id)) {
                    previous.name = thread_task_name(meta);
                }
                tasks.push(previous);
            }
        }
        let incomplete = self.cache.values().any(|cache| !cache.tail.trusted());
        if incomplete {
            for previous in &self.last_tasks {
                if !tasks.iter().any(|t| t.id == previous.id) {
                    let mut previous = previous.clone();
                    previous.stale = true;
                    tasks.push(previous);
                }
            }
        }
        log::debug!(
            "codex parser entries={} retained_tail_bytes={}",
            self.cache.len(),
            self.cache
                .values()
                .map(|c| c.tail.retained_bytes())
                .sum::<usize>()
        );
        self.last_tasks = tasks.clone();
        super::resources::record(
            "codex",
            &self.sessions_root,
            serde_json::json!({"entries":self.cache.len(),"protected_entries":self.cache.values().filter(|c|!c.tail.trusted() || c.state.as_ref().is_some_and(|s|s.status==TaskStatus::Running || !s.confirmed_approvals.is_empty())).count(),"tail_capacity_bytes":self.cache.values().map(|c|c.tail.retained_bytes()).sum::<usize>(),"cumulative_rollout_bytes_read":self.read_bytes,"inactive_entry_budget":super::parser_cache::INACTIVE_ENTRIES,"inactive_estimated_byte_budget":super::parser_cache::INACTIVE_BYTES}),
        );
        let catalog = metadata
            .values()
            .map(|meta| {
                let mut task = TaskSnapshot::new(
                    self.task_id(&meta.id),
                    thread_task_name(meta),
                    TaskSource::Codex,
                    TaskStatus::Unknown,
                    DetectionConfidence::Hook,
                    UNIX_EPOCH + Duration::from_secs(meta.updated_at.max(0) as u64),
                );
                task.project = project_from_cwd(meta.cwd.as_deref());
                task.project_path = meta.cwd.clone();
                task.session_id = Some(meta.id.clone());
                task.internal = meta.internal;
                task
            })
            .collect();
        ProviderOutput {
            catalog,
            authoritative: !incomplete,
            error: incomplete.then(|| "Codex 日志追赶中或存在坏行；不发布中间终态".into()),
            ..ProviderOutput::tasks(tasks)
        }
    }
}

fn parser_protected(t: &TaskSnapshot) -> bool {
    t.status == TaskStatus::Running
        || matches!(
            t.waiting_reason,
            Some(crate::domain::WaitingReason::Approval | crate::domain::WaitingReason::Question)
        )
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

pub(crate) fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    // Accept "2026-07-06T08:22:01.075Z" and "+00:00" offsets.
    let cleaned = s.replace('Z', "+00:00");
    let dt = chrono_like_parse(&cleaned)?;
    // Preserve sub-second event order. Resumed rollouts and very short turns
    // can both change within one second; truncating precision lets the old
    // turn override the new one or gives two executions the same fallback ID.
    let fraction = cleaned.get(19..).and_then(|tail| tail.strip_prefix('.'));
    let mut nanos = 0;
    let mut place = 100_000_000;
    if let Some(fraction) = fraction {
        for digit in fraction.bytes().take_while(u8::is_ascii_digit).take(9) {
            nanos += u32::from(digit - b'0') * place;
            place /= 10;
        }
    }
    Some(UNIX_EPOCH + Duration::new(dt, nanos))
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
    #[test]
    fn lifecycle_schema_error_is_scoped_and_retry_or_rate_limit_is_not_exhaustion() {
        let now = SystemTime::now();
        let body = format!(
            "{}{}",
            header("/project"),
            turn_event("task_started", "current", 1)
        );
        let original = parse_codex_rollout_unfiltered(&body, now, "s").unwrap();
        for (turn, info, retry, expected) in [
            ("current", "usageLimitExceeded", false, true),
            ("old", "usageLimitExceeded", false, false),
            ("current", "usageLimitExceeded", true, false),
            ("current", "rateLimitExceeded", false, false),
            ("current", "serverOverloaded", false, false),
        ] {
            let event = serde_json::json!({"timestamp":recent_rfc3339(0),"method":"error","params":{"threadId":"sess-1","turnId":turn,"willRetry":retry,"error":{"codexErrorInfo":info}}}).to_string();
            let state = apply_codex_rollout(&event, now, "s", Some(original.clone())).unwrap();
            assert_eq!(
                state.waiting_reason == Some(crate::domain::WaitingReason::QuotaExhausted),
                expected
            );
        }
    }
    #[test]
    fn lifecycle_large_compaction_does_not_poison_new_running_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-large.jsonl");
        let compacted = serde_json::json!({"type":"compacted","payload":{"replacement_history":"x".repeat(super::super::jsonl_tail::LINE_BUDGET + 1024)}});
        std::fs::write(
            &path,
            format!(
                "{}{}\n{}",
                header("/project"),
                compacted,
                turn_event("task_started", "current", 0)
            ),
        )
        .unwrap();
        let mut provider = CodexProvider::with_root(dir.path().into());
        let mut parsed = None;
        for _ in 0..30 {
            parsed = provider.cached_session_state(&path, SystemTime::now());
            if parsed.is_some() {
                break;
            }
        }
        let parsed = parsed
            .expect("irrelevant oversized compaction must not permanently hide lifecycle events");
        assert_eq!(parsed.status, TaskStatus::Running);
        assert_eq!(parsed.turn_key.as_deref(), Some("current"));
        assert!(provider.cache[&path].tail.retained_bytes() < 1024);
    }

    #[test]
    fn lifecycle_late_old_turn_end_cannot_end_current_turn() {
        let body = format!(
            "{}{}{}",
            header("/project"),
            turn_event("task_started", "current", 2),
            turn_event("task_complete", "previous", 0)
        );
        let state = parse_codex_rollout_unfiltered(&body, SystemTime::now(), "s").unwrap();
        assert_eq!(state.status, TaskStatus::Running);
        assert_eq!(state.turn_key.as_deref(), Some("current"));
    }

    #[test]
    fn lifecycle_new_user_message_reopens_ended_but_not_an_explicit_gate() {
        let now = SystemTime::now();
        let body = format!(
            "{}{}{}",
            header("/project"),
            turn_event("task_started", "old", 3),
            turn_event("task_complete", "old", 2)
        );
        let old = parse_codex_rollout_unfiltered(&body, now, "s").unwrap();
        let message = serde_json::json!({"timestamp":recent_rfc3339(0),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}}).to_string();
        let next = apply_codex_rollout(&message, now, "s", Some(old.clone())).unwrap();
        assert_eq!(next.status, TaskStatus::Running);
        assert_ne!(next.turn_key, old.turn_key);
        let mut blocked = old;
        blocked.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        blocked
            .confirmed_approvals
            .insert("request".into(), Some("call".into()));
        assert_eq!(
            apply_codex_rollout(&message, now, "s", Some(blocked))
                .unwrap()
                .waiting_reason,
            Some(crate::domain::WaitingReason::Approval)
        );
    }

    #[test]
    fn lifecycle_explicit_quota_error_is_actionable_not_a_successful_end() {
        let now = SystemTime::now();
        let start = format!(
            "{}{}",
            header("/project"),
            turn_event("task_started", "current", 1)
        );
        let error = serde_json::json!({"timestamp":recent_rfc3339(0),"payload":{"type":"error","turn_id":"current","code":"usage_limit_reached","will_retry":false}}).to_string();
        let state = parse_codex_rollout_unfiltered(
            &format!(
                "{start}{error}\n{}",
                turn_event("task_complete", "current", 0)
            ),
            now,
            "s",
        )
        .unwrap();
        assert_eq!(state.status, TaskStatus::WaitingInput);
        assert_eq!(
            serde_json::to_value(state.waiting_reason).unwrap(),
            "quota_exhausted"
        );
        let retry = error.replace("false", "true");
        let state = parse_codex_rollout_unfiltered(&format!("{start}{retry}"), now, "s").unwrap();
        assert_eq!(state.status, TaskStatus::Running);
    }
    use std::io::Write;

    fn escalation_log(id: &str) -> String {
        format!(
            "{}\n{}\n{}\n",
            serde_json::json!({"timestamp":"2026-10-12T09:00:00Z","type":"session_meta","payload":{"id":"s","cwd":"/project"}}),
            serde_json::json!({"timestamp":"2026-10-12T09:00:00Z","payload":{"type":"task_started","turn_id":"turn"}}),
            serde_json::json!({"timestamp":"2026-10-12T09:00:01Z","payload":{"type":"custom_tool_call","name":"exec","call_id":id,"input":"const r = await tools.exec_command({cmd: \"gpu-check\", sandbox_permissions: \"require_escalated\"});"}})
        )
    }

    /// Replay a real, read-only SSH transcript at a caller-selected historical
    /// cutoff. Never prompts an Agent, executes its commands, or edits a log.
    #[test]
    #[ignore = "requires TASKPULSE_APPROVAL_SSH_HOST, THREAD_ID and REPLAY_UNTIL"]
    fn live_remote_approval_replay() {
        use base64::Engine;
        let host = std::env::var("TASKPULSE_APPROVAL_SSH_HOST").expect("Set SSH host");
        let id = std::env::var("TASKPULSE_APPROVAL_THREAD_ID").expect("Set thread ID");
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
        let until = std::env::var("TASKPULSE_APPROVAL_REPLAY_UNTIL").expect("Set RFC3339 cutoff");
        let now = parse_rfc3339(&until).expect("Valid cutoff");
        let script = format!(
            r#"import glob,json,os
from datetime import datetime
thread={id:?}
cutoff=datetime.fromisoformat({until:?}.replace('Z','+00:00'))
for path in glob.glob(os.path.expanduser('~/.codex/sessions/**/rollout-*'+thread+'*.jsonl'),recursive=True):
 with open(path,encoding='utf-8') as stream:
  for line in stream:
   try:
    row=json.loads(line)
    if datetime.fromisoformat(row['timestamp'].replace('Z','+00:00'))<=cutoff: print(line,end='')
   except (ValueError,KeyError): pass
"#
        );
        let encoded = base64::engine::general_purpose::STANDARD.encode(script);
        let output = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=6",
                &host,
                &format!("echo {encoded} | base64 -d | python3"),
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "SSH read failed");
        let body = String::from_utf8(output.stdout).unwrap();
        let state = parse_codex_rollout(&body, now, &id).expect("Real rollout available");
        assert!(
            !state.active_tools.is_empty(),
            "Unresolved native tool at cutoff"
        );
        assert_eq!(state.phase, Some(crate::domain::ExecutionPhase::Tool));
        assert_eq!(state.status, TaskStatus::Running);
        assert_eq!(state.waiting_reason, None);
        assert_eq!(state.session_id, id);
        let mut task = TaskSnapshot::new(
            format!("replay:{id}"),
            "Read-only replay (not monitored)",
            TaskSource::Codex,
            state.status,
            DetectionConfidence::Hook,
            now,
        );
        task.phase = state.phase;
        task.turn_key = state.turn_key;
        let mut engine = crate::transition::TransitionEngine::new();
        assert!(engine.apply(&[task.clone()], &[], now).is_empty());
        assert!(engine
            .apply(&[task], &[], now + Duration::from_secs(7200))
            .is_empty());
        println!("Read-only replay passed: unresolved native tool, state RUNNING, no inferred approval alert or completion");
    }

    #[test]
    fn long_escalated_calls_never_infer_approval_or_emit_notifications() {
        let started = parse_rfc3339("2026-10-12T09:00:00Z").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, escalation_log("c1")).unwrap();
        let mut provider = CodexProvider::with_root(dir.path().to_path_buf());
        let mut engine = crate::transition::TransitionEngine::new();
        // Both the first parse and unchanged incremental cache must stay running.
        for seconds in [10, 22, 600, 7200] {
            let now = started + Duration::from_secs(seconds);
            let state = provider.cached_session_state(&path, now).unwrap();
            assert_eq!(state.status, TaskStatus::Running);
            assert_eq!(state.waiting_reason, None);
            assert_eq!(state.input_request_id, None);
            assert_eq!(state.phase, Some(crate::domain::ExecutionPhase::Tool));
            assert!(state.active_tools.contains_key("c1"));
            let snapshot = provider.session_to_task(&state, None, now);
            assert!(!snapshot.requires_action());
            assert!(engine.apply(&[snapshot], &[], now).is_empty());
        }
        let now = started + Duration::from_secs(7201);
        let returned = serde_json::json!({"timestamp":"2026-10-12T11:00:01Z","payload":{"type":"custom_tool_call_output","call_id":"c1","output":"Process running with session ID 12"}}).to_string();
        let state = provider.cached_session_state(&path, now).unwrap();
        let returned = apply_codex_rollout(&returned, now, "s", Some(state)).unwrap();
        assert!(returned.active_tools.is_empty());
        assert_eq!(returned.status, TaskStatus::Running);
        assert_eq!(
            returned.phase,
            Some(crate::domain::ExecutionPhase::AwaitingModel)
        );
        assert!(engine
            .apply(&[provider.session_to_task(&returned, None, now)], &[], now)
            .is_empty());
        let ended = serde_json::json!({"timestamp":"2026-10-12T11:00:02Z","payload":{"type":"task_complete","turn_id":"turn"}}).to_string();
        let ended = apply_codex_rollout(&ended, now, "s", Some(returned)).unwrap();
        let snapshot = provider.session_to_task(&ended, None, now);
        assert!(
            matches!(engine.apply(&[snapshot.clone()], &[], now).as_slice(),
            [crate::transition::TaskEvent::WaitingInput { task, .. }]
            if task.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded))
        );
        assert!(engine.apply(&[snapshot], &[], now).is_empty());
    }

    #[test]
    fn unrelated_tool_results_preserve_the_active_command_without_an_approval_guess() {
        let now = parse_rfc3339("2026-10-12T09:00:25Z").unwrap();
        let state = parse_codex_rollout(&escalation_log("c1"), now, "s").unwrap();
        let other = serde_json::json!({"timestamp":"2026-10-12T09:00:05Z","payload":{"type":"function_call_output","call_id":"c-other"}}).to_string();
        let running = apply_codex_rollout(&other, now, "s", Some(state)).unwrap();
        assert!(running.active_tools.contains_key("c1"));
        assert_eq!(running.status, TaskStatus::Running);
        assert_eq!(running.phase, Some(crate::domain::ExecutionPhase::Tool));
        assert_eq!(running.waiting_reason, None);
        let ended = serde_json::json!({"timestamp":"2026-10-12T09:00:30Z","payload":{"type":"task_complete","turn_id":"turn"}}).to_string();
        let ended = apply_codex_rollout(&ended, now, "s", Some(running)).unwrap();
        assert_eq!(
            ended.waiting_reason,
            Some(crate::domain::WaitingReason::TurnEnded)
        );
        assert!(ended.active_tools.is_empty());
    }

    #[test]
    fn command_permissions_are_not_evidence_of_a_pending_approval() {
        let now = parse_rfc3339("2026-10-12T09:01:00Z").unwrap();
        for (name, arguments) in [
            (
                "exec_command",
                r#"{"cmd":"gpu-check","sandbox_permissions":"require_escalated"}"#,
            ),
            (
                "functions.exec_command",
                r#"{"sandbox_permissions":"require_escalated","justification":"Check GPU"}"#,
            ),
            (
                "exec_command",
                r#"{"cmd":"long-command","sandbox_permissions":"use_default"}"#,
            ),
        ] {
            let call = serde_json::json!({"timestamp":"2026-10-12T09:00:02Z","payload":{"type":"function_call","name":name,"call_id":"c2","arguments":arguments}}).to_string();
            let state = parse_codex_rollout(&(escalation_log("c1") + &call), now, "s").unwrap();
            assert_eq!(state.status, TaskStatus::Running);
            assert_eq!(state.phase, Some(crate::domain::ExecutionPhase::Tool));
            assert_eq!(state.waiting_reason, None);
            assert_eq!(state.input_request_id, None);
        }
    }

    #[test]
    fn explicit_approval_events_wait_until_the_matching_request_is_resolved() {
        let now = parse_rfc3339("2026-10-12T09:00:25Z").unwrap();
        let base = parse_codex_rollout(&escalation_log("c1"), now, "s").unwrap();
        let requested=serde_json::json!({"timestamp":"2026-10-12T09:00:02Z","id":42,"method":"item/commandExecution/requestApproval","params":{"threadId":"s","turnId":"turn","itemId":"c1"}}).to_string();
        let state = apply_codex_rollout(&requested, now, "s", Some(base)).unwrap();
        assert_eq!(
            state.waiting_reason,
            Some(crate::domain::WaitingReason::Approval)
        );
        assert_eq!(state.input_request_id.as_deref(), Some("42"));
        let wrong=serde_json::json!({"method":"serverRequest/resolved","params":{"threadId":"s","requestId":43}}).to_string();
        let still = apply_codex_rollout(&wrong, now, "s", Some(state)).unwrap();
        assert_eq!(still.status, TaskStatus::WaitingInput);
        let resolved=serde_json::json!({"timestamp":"2026-10-12T09:00:05Z","method":"serverRequest/resolved","params":{"threadId":"s","requestId":42}}).to_string();
        let resumed = apply_codex_rollout(&resolved, now, "s", Some(still)).unwrap();
        assert_eq!(resumed.status, TaskStatus::Running);
        assert_eq!(resumed.waiting_reason, None);
        assert_eq!(resumed.input_request_id, None);
        assert_eq!(
            resumed.phase,
            Some(crate::domain::ExecutionPhase::AwaitingTool)
        );
    }

    #[test]
    fn remote_snapshots_never_infer_approval_or_invent_completion() {
        let now = parse_rfc3339("2026-10-12T09:00:25Z").unwrap();
        let section=format!("===S:project===F:rollout-2026-10-12T09-00-00-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl===\n{}",escalation_log("c1"));
        let tasks = tasks_from_codex_section(&section, "test-host", "host", now);
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].remote);
        assert_eq!(tasks[0].status, TaskStatus::Running);
        assert_eq!(tasks[0].phase, Some(crate::domain::ExecutionPhase::Tool));
        assert!(!tasks[0].requires_action());
        assert!(crate::transition::TransitionEngine::new()
            .apply(&tasks, &[], now)
            .is_empty());
        assert_eq!(
            tasks[0].session_id.as_deref(),
            Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
    }

    fn native_db(dir: &Path, path: &Path, updated: i64) -> PathBuf {
        let db = dir.join("state.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE threads (id TEXT, name TEXT, title TEXT, first_user_message TEXT, cwd TEXT, rollout_path TEXT, updated_at INTEGER, is_pinned INTEGER, source TEXT, archived INTEGER);").unwrap();
        conn.execute("INSERT INTO threads VALUES ('sess-1',NULL,'Old prompt','Old prompt','C:/old-project',?, ?,0,'appServer',0)", (path.to_string_lossy().as_ref(),updated)).unwrap();
        db
    }

    fn turn_event(kind: &str, turn: &str, ago: u64) -> String {
        format!(
            "{}\n",
            serde_json::json!({"timestamp":recent_rfc3339(ago),"type":"event_msg","payload":{"type":kind,"turn_id":turn}})
        )
    }

    #[test]
    fn desktop_sidebar_index_renames_without_lifecycle_or_activity_changes() {
        use crate::attention::AttentionTracker;
        use crate::transition::TransitionEngine;
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let path = sessions.join("rollout-test.jsonl");
        std::fs::write(
            &path,
            format!(
                "{}{}",
                header("/project"),
                turn_event("task_started", "current-turn", 1)
            ),
        )
        .unwrap();
        let index = dir.path().join("session_index.jsonl");
        std::fs::write(
            &index,
            "{\"id\":\"sess-1\",\"thread_name\":\"Native sidebar name\"}\ninvalid future row\n",
        )
        .unwrap();
        let mut provider = CodexProvider::with_root(sessions);
        provider.state_db = Some(native_db(dir.path(), &path, 0));
        let first = provider.poll(SystemTime::now());
        assert_eq!(first.tasks[0].name, "Native sidebar name");
        let before = first.tasks[0].clone();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        let mut engine = TransitionEngine::new();
        engine.apply(&first.tasks, &[], SystemTime::now());
        tracker.reconcile(first.tasks);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&index)
            .unwrap()
            .write_all(b"{\"id\":\"sess-1\",\"thread_name\":\"Renamed in Desktop\"}\n")
            .unwrap();
        let renamed = provider.poll(SystemTime::now()).tasks;
        assert_eq!(renamed[0].name, "Renamed in Desktop");
        assert_eq!(renamed[0].id, before.id);
        assert_eq!(renamed[0].turn_key, before.turn_key);
        assert_eq!(renamed[0].updated_at, before.updated_at);
        assert!(engine.apply(&renamed, &[], SystemTime::now()).is_empty());
        assert!(!tracker.reconcile(renamed)[0].new_execution);
        let conn = rusqlite::Connection::open(provider.state_db.as_ref().unwrap()).unwrap();
        conn.execute("UPDATE threads SET name='Manual override'", [])
            .unwrap();
        assert_eq!(
            provider.poll(SystemTime::now()).tasks[0].name,
            "Manual override"
        );
    }

    #[test]
    fn new_project_rollout_is_detected_before_metadata_and_title_are_available() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("rollout-existing.jsonl");
        std::fs::write(
            &old,
            format!(
                "{}{}",
                header("/old-project"),
                turn_event("task_complete", "old-turn", 60)
            ),
        )
        .unwrap();
        let mut provider = CodexProvider::with_root(dir.path().to_owned());
        provider.state_db = Some(native_db(
            dir.path(),
            &old,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        ));
        provider.poll(SystemTime::now());
        let fresh = dir.path().join("rollout-brand-new.jsonl");
        std::fs::write(&fresh,format!("{}\n{}",serde_json::json!({"type":"session_meta","payload":{"id":"new-id","cwd":"D:/never-monitored-before"}}),turn_event("task_started","new-turn",0))).unwrap();
        let tasks = provider.poll(SystemTime::now()).tasks;
        let task = tasks
            .iter()
            .find(|t| t.session_id.as_deref() == Some("new-id"))
            .expect("new execution is independent of metadata allowlist");
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.project.as_deref(), Some("never-monitored-before"));
        assert_eq!(
            task.project_path.as_deref(),
            Some("D:/never-monitored-before")
        );
        assert_eq!(task.name, "未命名任务 · new-id");
        let id = task.id.clone();
        let mut tracker =
            crate::attention::AttentionTracker::open(dir.path().join("attention.json"));
        assert!(tracker.reconcile(tasks).iter().any(|t| t.id == id));
    }

    #[test]
    fn resumed_thread_uses_newest_events_not_old_metadata_path_or_file_order() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("rollout-old.jsonl");
        let resumed = dir.path().join("rollout-resumed.jsonl");
        let now = SystemTime::now();
        let secs = now.duration_since(UNIX_EPOCH).unwrap().as_secs() - 1;
        let stamp = format_unix(secs);
        let old_stamp = stamp.replace(".000Z", ".100Z");
        let new_stamp = stamp.replace(".000Z", ".900Z");
        let body = |kind: &str, turn: &str, ts: &str| {
            format!(
                "{}{}\n",
                header("/project"),
                serde_json::json!({"timestamp":ts,"payload":{"type":kind,"turn_id":turn}})
            )
        };
        std::fs::write(&resumed, body("task_started", "new-turn", &new_stamp)).unwrap();
        // Write the old file LAST: discovery mtime order must not choose it.
        std::fs::write(&old, body("task_complete", "old-turn", &old_stamp)).unwrap();
        let mut provider = CodexProvider::with_root(dir.path().to_owned());
        provider.state_db = Some(native_db(dir.path(), &old, secs as i64));
        let tasks = provider.poll(now).tasks;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "codex:sess-1");
        assert_eq!(tasks[0].status, TaskStatus::Running);
        assert_eq!(tasks[0].turn_key.as_deref(), Some("new-turn"));
    }

    #[test]
    fn unknown_metadata_guardian_is_marked_internal_from_rollout_source() {
        let body = format!(
            "{}\n{}",
            serde_json::json!({"type":"session_meta","payload":{"id":"internal","source":{"subagent":{"other":"guardian"}},"cwd":"/project"}}),
            turn_event("task_started", "turn", 0)
        );
        let (dir, _) = temp_rollout(&body);
        let task = CodexProvider::with_root(dir.path().to_owned())
            .poll(SystemTime::now())
            .tasks
            .remove(0);
        assert!(task.internal);
    }

    fn remote_origin_fixture(
        id: &str,
        title: &str,
        database_source: Option<serde_json::Value>,
        rollout_source: Option<serde_json::Value>,
        event: &str,
        turn: &str,
    ) -> String {
        let metadata = serde_json::json!({"id":id,"t":title,"cwd":"/project","source":database_source,"p":true});
        let header = rollout_source
            .map(|source| {
                serde_json::json!({"type":"session_meta",
            "payload":{"id":id,"source":source,"cwd":"/project"}})
                .to_string()
                    + "\n"
            })
            .unwrap_or_default();
        let event = serde_json::json!({"timestamp":recent_rfc3339(1),"type":"event_msg",
            "payload":{"type":event,"turn_id":turn}});
        format!("META\t{metadata}\n===S:10===F:rollout-2026-10-13T13-00-00-{id}.jsonl===\n{header}{event}\n")
    }

    #[test]
    fn remote_guardian_origin_survives_full_truncated_and_legacy_metadata_paths() {
        let id = "11111111-1111-7111-8111-111111111111";
        let origin = serde_json::json!({"subagent":{"other":"guardian"}});
        for (db, rollout, title) in [
            (
                Some(origin.clone()),
                Some(origin.clone()),
                "Guardian review",
            ),
            (Some(origin.clone()), None, "Renamed internal service"),
            (None, Some(origin.clone()), "Guardian review"),
            (
                Some(serde_json::Value::String(origin.to_string())),
                None,
                "",
            ),
        ] {
            let section =
                remote_origin_fixture(id, title, db, rollout, "task_complete", "child-turn");
            let tasks = tasks_from_codex_section(&section, "Server1", "server1", SystemTime::now());
            assert_eq!(tasks.len(), 1);
            assert!(tasks[0].internal, "origin was lost for {title:?}");
            assert_eq!(tasks[0].session_id.as_deref(), Some(id));
            let catalog = catalog_from_codex_section(&section, "Server1", "server1");
            assert_eq!(catalog.len(), 1);
            assert!(
                catalog[0].internal,
                "catalog disagreed with execution origin"
            );
        }
    }

    #[test]
    fn guardian_completions_never_notify_while_user_parent_is_running() {
        use crate::domain::WaitingReason;
        let dir = tempfile::tempdir().unwrap();
        let mut engine =
            crate::transition::TransitionEngine::open(dir.path().join("notifications.json"));
        let child = "11111111-1111-7111-8111-111111111111";
        let parent = "22222222-2222-7222-8222-222222222222";
        let now = SystemTime::now();
        // A real user can name their task Guardian review; it must still notify.
        let parent_running = remote_origin_fixture(
            parent,
            "Guardian review",
            Some(serde_json::json!("appServer")),
            None,
            "task_started",
            "parent-turn",
        );
        let initial = format!(
            "{parent_running}{}",
            remote_origin_fixture(
                child,
                "Guardian review",
                Some(serde_json::json!({"subagent":{"other":"guardian"}})),
                None,
                "task_started",
                "child-0"
            )
        );
        assert!(engine
            .apply(
                &tasks_from_codex_section(&initial, "Server1", "server1", now),
                &[],
                now
            )
            .is_empty());
        for round in 0..3 {
            let section = format!(
                "{parent_running}{}",
                remote_origin_fixture(
                    child,
                    "Guardian review",
                    Some(serde_json::json!({"subagent":{"other":"guardian"}})),
                    None,
                    "task_complete",
                    &format!("child-{round}")
                )
            );
            let tasks = tasks_from_codex_section(&section, "Server1", "server1", now);
            assert_eq!(tasks.iter().filter(|t| !t.internal).count(), 1);
            assert_eq!(
                tasks.iter().find(|t| !t.internal).unwrap().status,
                TaskStatus::Running
            );
            // Even a direct engine caller cannot bypass the internal guard.
            assert!(engine.apply(&tasks, &[], now).is_empty());
        }
        let section = remote_origin_fixture(
            parent,
            "Guardian review",
            Some(serde_json::json!("appServer")),
            None,
            "task_complete",
            "parent-turn",
        );
        let tasks = tasks_from_codex_section(&section, "Server1", "server1", now);
        assert!(!tasks[0].internal);
        assert_eq!(tasks[0].waiting_reason, Some(WaitingReason::TurnEnded));
        assert_eq!(engine.apply(&tasks, &[], now).len(), 1);
        assert!(engine.apply(&tasks, &[], now).is_empty());
    }

    #[test]
    fn metadata_only_pins_do_not_override_native_internal_origin() {
        let section = "META\t{\"id\":\"internal\",\"t\":\"Renamed service\",\"cwd\":\"/project\",\"p\":true,\"source\":{\"subagent\":{\"other\":\"guardian\"}}}\nMETA\t{\"id\":\"user\",\"t\":\"Guardian review\",\"cwd\":\"/project\",\"p\":true,\"source\":\"appServer\"}\n";
        let tasks = tasks_from_codex_section(section, "Server1", "server1", SystemTime::now());
        assert!(
            tasks
                .iter()
                .find(|t| t.session_id.as_deref() == Some("internal"))
                .unwrap()
                .internal
        );
        assert!(
            !tasks
                .iter()
                .find(|t| t.session_id.as_deref() == Some("user"))
                .unwrap()
                .internal
        );
    }

    #[test]
    fn sidebar_index_tolerates_partial_appends_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("session_index.jsonl");
        let mut provider = CodexProvider::with_root(dir.path().join("sessions"));
        std::fs::write(
            &index,
            "{\"id\":\"one\",\"thread_name\":\"First\"}\n{\"id\":\"two\"",
        )
        .unwrap();
        provider.refresh_title_index();
        assert_eq!(provider.indexed_titles.len(), 1);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&index)
            .unwrap()
            .write_all(b",\"thread_name\":\"Second\"}\n")
            .unwrap();
        provider.refresh_title_index();
        assert_eq!(
            provider.indexed_titles.get("two").map(String::as_str),
            Some("Second")
        );
        std::fs::write(&index, "{\"id\":\"one\",\"thread_name\":\"Replaced\"}\n").unwrap();
        provider.refresh_title_index();
        assert_eq!(provider.indexed_titles.len(), 1);
        assert_eq!(provider.indexed_titles["one"], "Replaced");
    }

    #[test]
    fn wsl_mapping_sidebar_rename_and_incremental_turn_share_native_rules() {
        use crate::attention::AttentionTracker;
        use crate::transition::{TaskEvent, TransitionEngine};
        let dir = tempfile::tempdir().unwrap();
        let linux_home = "home/user/.codex";
        let root = dir.path().join(linux_home);
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        let path = root.join("sessions/rollout-stable.jsonl");
        let ts = recent_rfc3339(0);
        let event = |kind: &str| {
            format!(
                "{}\n",
                serde_json::json!({"timestamp":ts,"payload":{"type":kind,"turn_id":"turn"}})
            )
        };
        let session_meta = serde_json::json!({"type":"session_meta","payload":{"id":"stable","cwd":"/home/user/projects/E-commerce Agent"}});
        std::fs::write(&path, format!("{session_meta}\n{}", event("task_started"))).unwrap();
        let mut provider =
            CodexProvider::with_wsl_root(root, dir.path().into(), "codex-wsl:ubuntu");
        let mut row:super::super::codex_wsl::WslThread=serde_json::from_value(serde_json::json!({
            "id":"stable","title":"long first prompt, not the sidebar", "sidebar_title":"原生 WSL 标题",
            "cwd":"/home/user/projects/E-commerce Agent","rollout_path":"/home/user/.codex/sessions/rollout-stable.jsonl",
            "updated_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
        })).unwrap();
        provider.set_external_metadata(&[row]);
        let first = provider.poll(SystemTime::now());
        assert!(first.authoritative);
        assert_eq!(first.tasks.len(), 1);
        let task = &first.tasks[0];
        assert_eq!(task.id, "codex-wsl:ubuntu:codex:stable");
        assert_eq!(task.name, "原生 WSL 标题");
        assert_eq!(task.project.as_deref(), Some("E-commerce Agent"));
        assert_eq!(
            task.project_path.as_deref(),
            Some("/home/user/projects/E-commerce Agent")
        );
        assert_eq!(task.status, TaskStatus::Running);
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        let mut engine = TransitionEngine::new();
        assert!(engine
            .apply(&first.tasks, &[], SystemTime::now())
            .is_empty());
        assert_eq!(tracker.reconcile(first.tasks.clone()).len(), 1);
        row=serde_json::from_value(serde_json::json!({"id":"stable","name":"手动新标题","rollout_path":"/home/user/.codex/sessions/rollout-stable.jsonl"})).unwrap();
        provider.set_external_metadata(&[row]);
        let renamed = provider.poll(SystemTime::now());
        assert_eq!(renamed.tasks[0].name, "手动新标题");
        assert_eq!(renamed.tasks[0].id, task.id);
        assert_eq!(renamed.tasks[0].turn_key, task.turn_key);
        assert!(engine
            .apply(&renamed.tasks, &[], SystemTime::now())
            .is_empty());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(event("task_complete").as_bytes())
            .unwrap();
        let ended = provider.poll(SystemTime::now());
        assert_eq!(
            ended.tasks[0].waiting_reason,
            Some(crate::domain::WaitingReason::TurnEnded)
        );
        assert_eq!(ended.tasks[0].status, TaskStatus::WaitingInput);
        assert!(matches!(
            engine
                .apply(&ended.tasks, &[], SystemTime::now())
                .as_slice(),
            [TaskEvent::WaitingInput { .. }]
        ));
        assert!(engine
            .apply(&ended.tasks, &[], SystemTime::now())
            .is_empty());
        let attention = tracker.reconcile(ended.tasks.clone());
        assert!(attention[0].unread);
        assert!(tracker.hide(&task.id).unwrap().is_empty());
        let mut fresh = AttentionTracker::open(dir.path().join("fresh-attention.json"));
        assert!(fresh.reconcile(ended.tasks).is_empty());
        let next_turn = serde_json::json!({"timestamp":ts,"payload":{"type":"task_started","turn_id":"next-turn"}});
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap(),
            "{next_turn}"
        )
        .unwrap();
        let resumed = provider.poll(SystemTime::now()).tasks;
        assert!(engine.apply(&resumed, &[], SystemTime::now()).is_empty());
        let attention = tracker.reconcile(resumed);
        assert_eq!(attention.len(), 1);
        assert_eq!(attention[0].id, task.id);
        assert_eq!(attention[0].status, TaskStatus::Running);
        assert_eq!(attention[0].turn_key.as_deref(), Some("next-turn"));
    }

    #[test]
    fn tool_outputs_clear_only_matching_calls_and_reasoning_keeps_turn_running() {
        use crate::domain::ExecutionPhase;
        let now = SystemTime::now();
        let mut state = None;
        for payload in [
            serde_json::json!({"type":"task_started","turn_id":"stable-turn"}),
            serde_json::json!({"type":"custom_tool_call","call_id":"a","name":"exec","input":"{\"cmd\":\"cargo test\"}"}),
            serde_json::json!({"type":"function_call","call_id":"b","name":"shell_command","arguments":"{\"command\":\"git status\"}"}),
            serde_json::json!({"type":"custom_tool_call_output","call_id":"a"}),
        ] {
            state = apply_codex_rollout(
                &format!("{}\n", serde_json::json!({"payload":payload})),
                now,
                "s",
                state,
            );
        }
        assert_eq!(state.as_ref().unwrap().phase, Some(ExecutionPhase::Tool));
        assert!(state
            .as_ref()
            .unwrap()
            .activity
            .as_ref()
            .unwrap()
            .contains("git status"));
        state = apply_codex_rollout(
            "{\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"b\"}}\n",
            now,
            "s",
            state,
        );
        assert_eq!(state.as_ref().unwrap().activity, None);
        assert_eq!(
            state.as_ref().unwrap().phase,
            Some(ExecutionPhase::AwaitingModel)
        );
        state = apply_codex_rollout("{\"payload\":{\"type\":\"reasoning\"}}\n", now, "s", state);
        let thinking = state.unwrap();
        assert_eq!(thinking.status, TaskStatus::Running);
        assert_eq!(thinking.phase, Some(ExecutionPhase::Thinking));
        assert_eq!(thinking.turn_key.as_deref(), Some("stable-turn"));
        let resumed = apply_codex_rollout(
            "{\"payload\":{\"type\":\"task_complete\"}}\n",
            now,
            "s",
            Some(thinking),
        )
        .unwrap();
        assert_eq!(resumed.phase, None);
        assert_eq!(
            resumed.waiting_reason,
            Some(crate::domain::WaitingReason::TurnEnded)
        );
    }

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
    fn stale_running_state_is_retained_but_not_treated_as_fresh_discovery() {
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
        // Preserve raw events for already tracked sessions. Fresh discovery
        // still applies TTL, while known sessions can display uncertainty.
        let fut = SystemTime::now() + Duration::from_secs(3600 * 30);
        let state = provider.parse_session_state(&p, fut).unwrap();
        assert_eq!(state.status, TaskStatus::Running);
        assert!(!session_is_live(&state, fut));
        assert!(parse_codex_rollout(&body, fut, "test").is_none());
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
    fn incremental_rollout_matches_full_parse_across_turns() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("stable.jsonl");
        let now = SystemTime::now();
        let mut body = header("/tmp/project");
        std::fs::write(&path, &body).unwrap();
        let mut provider = CodexProvider::with_root(root.path().into());
        for (kind, payload) in [
            (
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"first"}),
            ),
            (
                "event_msg",
                serde_json::json!({"type":"user_message","message":"keep this title"}),
            ),
            (
                "response_item",
                serde_json::json!({"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cargo test\"}"}),
            ),
            (
                "event_msg",
                serde_json::json!({"type":"task_complete","last_agent_message":"finished"}),
            ),
            (
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"second"}),
            ),
        ] {
            let line = format!(
                "{}\n",
                serde_json::json!({"type":kind,"timestamp":recent_rfc3339(1),"payload":payload})
            );
            body.push_str(&line);
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(line.as_bytes())
                .unwrap();
            assert_eq!(
                provider.cached_session_state(&path, now),
                parse_codex_rollout_unfiltered(&body, now, "stable")
            );
        }
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
            .state
            .as_mut()
            .unwrap()
            .project = Some("cached-project".into());

        let second = provider.poll(SystemTime::now());
        assert_eq!(second.tasks[0].project.as_deref(), Some("cached-project"));
    }

    #[test]
    fn f04_cold_cache_expires_without_replaying_or_losing_reactivation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-cold.jsonl");
        let body=format!("{}{{\"timestamp\":\"{}\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"old\"}}}}\n",header("/tmp/project"),recent_rfc3339(1));
        std::fs::write(&path, &body).unwrap();
        let mut p = CodexProvider::with_root(root.path().into());
        let now = SystemTime::now();
        assert!(!p.poll(now).tasks.is_empty());
        p.poll(now + DISCOVERY_TTL + Duration::from_secs(10));
        assert!(
            p.cache.is_empty(),
            "completed cached files cannot select themselves forever"
        );
        // A native writer touching an old project makes it discoverable again.
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{{\"timestamp\":\"{}\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"new\"}}}}\n",recent_rfc3339(0)).as_bytes()).unwrap();
        let out = p.poll(now);
        assert!(out
            .tasks
            .iter()
            .any(|t| t.turn_key.as_deref() == Some("new") && t.status == TaskStatus::Running));
    }

    #[test]
    fn f04_partial_catchup_cannot_publish_a_historical_terminal() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-budget.jsonl");
        let at = recent_rfc3339(1);
        let mut body = header("/tmp/project");
        body.push_str(&format!("{{\"timestamp\":\"{at}\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"old\"}}}}\n"));
        body.push_str(&format!("{{\"ignored\":\"{}\"}}\n", "x".repeat(1009)).repeat(18 * 1024));
        body.push_str(&format!("{{\"timestamp\":\"{at}\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"current\"}}}}\n"));
        std::fs::write(&path, body).unwrap();
        let mut provider = CodexProvider::with_root(root.path().into());
        let first = provider.poll(SystemTime::now());
        assert!(!first.authoritative);
        assert!(first.tasks.is_empty());
        let mut out = first;
        for _ in 0..10 {
            out = provider.poll(SystemTime::now());
            if out.authoritative {
                break;
            }
        }
        assert!(out.authoritative, "{:?}", out.error);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);
        assert_eq!(out.tasks[0].turn_key.as_deref(), Some("current"));
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
        let catalog = catalog_from_codex_section(section, "Server3", "server3");
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id, "codex-remote:server3:old-session");
        assert_eq!(catalog[0].status, TaskStatus::Unknown);
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
            internal: false,
        };
        assert_eq!(thread_task_name(&meta), "评审 AgentTray 监控界面");
    }

    #[test]
    fn guardian_is_identified_by_native_source_not_its_title() {
        let rows = parse_app_server_thread_list(serde_json::json!({
            "result": {"data": [
                {"id":"internal", "name":"Renamed service", "source":{"subagent":{"other":"guardian"}}},
                {"id":"user", "name":"Guardian review", "source":"appServer"},
                {"id":"worker", "source":{"subagent":{"other":"coding"}}}
            ]}
        })).unwrap();
        assert!(ThreadMeta::from_app_server(&rows[0]).internal);
        assert!(!ThreadMeta::from_app_server(&rows[1]).internal);
        assert!(!ThreadMeta::from_app_server(&rows[2]).internal);
    }

    #[test]
    fn native_database_marks_guardian_catalog_without_hiding_user_title_matches() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE threads (id TEXT, name TEXT, title TEXT, first_user_message TEXT, cwd TEXT, rollout_path TEXT, updated_at INTEGER, is_pinned INTEGER, source TEXT, archived INTEGER);").unwrap();
        for (id, source) in [
            ("internal", r#"{"subagent":{"other":"guardian"}}"#),
            ("user", "appServer"),
        ] {
            conn.execute("INSERT INTO threads VALUES (?, NULL, 'Guardian review', NULL, 'C:/demo', NULL, 0, 0, ?, 0)", (id, source)).unwrap();
        }
        let mut provider = CodexProvider::with_root(dir.path().join("sessions"));
        provider.state_db = Some(db);
        let catalog = provider.poll(SystemTime::now()).catalog;
        assert!(
            catalog
                .iter()
                .find(|t| t.session_id.as_deref() == Some("internal"))
                .unwrap()
                .internal
        );
        assert!(
            !catalog
                .iter()
                .find(|t| t.session_id.as_deref() == Some("user"))
                .unwrap()
                .internal
        );
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
    fn quota_selects_codex_bucket_not_another_model() {
        let quota = parse_app_server_rate_limits(serde_json::json!({"result": {
            "rateLimits": {"primary":{"usedPercent":99},"secondary":{"usedPercent":99}},
            "rateLimitsByLimitId": {"codex": {
                "primary":{"usedPercent":7},"secondary":{"usedPercent":75}
            }}
        }}))
        .unwrap();
        assert_eq!(quota.used_5h_pct, 7.0);
        assert_eq!(quota.used_1w_pct, 75.0);
    }

    #[test]
    fn quota_rejects_wrong_bucket_or_window_instead_of_mislabeling() {
        for limits in [
            serde_json::json!({"limitId":"another-model","primary":{"usedPercent":1},"secondary":{"usedPercent":2}}),
            serde_json::json!({"primary":{"usedPercent":1,"windowDurationMins":60},"secondary":{"usedPercent":2}}),
            serde_json::json!({"primary":{"usedPercent":1},"secondary":{"usedPercent":2,"windowDurationMins":1440}}),
        ] {
            assert!(parse_app_server_rate_limits(
                serde_json::json!({"result":{"rateLimits":limits}})
            )
            .is_err());
        }
    }

    #[test]
    fn failed_quota_refresh_preserves_timestamp_and_marks_cache_stale() {
        let now = SystemTime::now();
        let quota = CodexQuota {
            used_5h_pct: 7.0,
            used_1w_pct: 75.0,
            resets_5h_at: None,
            resets_1w_at: None,
        };
        let mut state = AccountQuota::default();
        record_quota(&mut state, Ok(quota), now);
        assert!(!quota_at(state.clone(), now).stale);
        assert!(quota_at(state.clone(), now + Duration::from_secs(61)).stale);
        record_quota(
            &mut state,
            Err("endpoint unavailable".into()),
            now + Duration::from_secs(2),
        );
        assert!(quota_at(state.clone(), now + Duration::from_secs(2)).stale);
        assert_eq!(state.observed_at, Some(epoch_millis(now)));
        assert_eq!(state.quota, Some(quota));
        record_quota(&mut state, Ok(quota), now + Duration::from_secs(3));
        assert!(state.error.is_none());
        assert!(!quota_at(state, now + Duration::from_secs(3)).stale);
    }

    #[test]
    fn legacy_task_quota_is_not_presented_as_live_account_usage() {
        let now = SystemTime::now();
        let mut tasks = vec![
            TaskSnapshot::new(
                "local",
                "local",
                TaskSource::Codex,
                TaskStatus::Idle,
                DetectionConfidence::Hook,
                now,
            ),
            TaskSnapshot::new(
                "remote",
                "remote",
                TaskSource::Codex,
                TaskStatus::Running,
                DetectionConfidence::Hook,
                now,
            )
            .with_remote()
            .with_host("different-account"),
        ];
        for task in &mut tasks {
            task.quota_5h_used_pct = Some(0.0);
            task.quota_1w_used_pct = Some(5.0);
        }
        clear_legacy_quota(&mut tasks);
        assert!(tasks
            .iter()
            .all(|task| task.quota_5h_used_pct.is_none() && task.quota_1w_used_pct.is_none()));
    }

    #[test]
    fn remote_session_without_sidebar_title_still_has_stable_id_and_project() {
        let section = format!("===S:10===F:rollout-2026-10-09T20-12-14-01a12094-0536-7b01-bbb0-c3dc63696fac.jsonl===\n{}\n{}\n",
            header("/home/user/YOLO-RDD"), serde_json::json!({"timestamp":recent_rfc3339(0),"payload":{"type":"task_started"}}));
        let tasks =
            tasks_from_codex_section(&section, "Server1_wanghy", "server1", SystemTime::now());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::Running);
        assert_eq!(tasks[0].project.as_deref(), Some("YOLO-RDD"));
        assert_eq!(
            tasks[0].session_id.as_deref(),
            Some("01a12094-0536-7b01-bbb0-c3dc63696fac")
        );
        assert!(tasks[0].name.starts_with("未命名任务"));
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
