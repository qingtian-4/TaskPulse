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

mod observer;
mod questions;

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
    linux_rollouts: Option<HashMap<PathBuf, super::jsonl_tail::LinuxFileStamp>>,
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
    pub(super) fn diagnostics(&self) -> Option<serde_json::Value> {
        super::resources::parser_metrics("codex", &self.sessions_root)
    }
    pub(super) fn set_linux_rollouts(&mut self, rows: Vec<super::codex_wsl::LinuxRollout>) {
        self.linux_rollouts = Some(
            rows.into_iter()
                .filter(|r| r.path.starts_with('/') && !r.path.split('/').any(|s| s == ".."))
                .map(|r| (self.resolve_rollout_path(&r.path), r.stamp))
                .collect(),
        );
    }
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
                            internal: internal_codex_source(&row.source),
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
        let root = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        let mut provider = Self::with_root(root.join("sessions"));
        let db = root.join("state_5.sqlite");
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
            linux_rollouts: None,
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
                    internal: internal_codex_source(
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
                Err(_) => {
                    // app-server is optional. Keep the last successful title map,
                    // then fall back through state DB and rollout metadata.
                    log::debug!("codex: native app-server metadata unavailable");
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
        if let Some(rows) = &self.linux_rollouts {
            // Linux already enumerated and stat'ed these files. Avoid an
            // additional slow/inconsistent UNC recursive directory snapshot.
            let mut files: Vec<_> = rows
                .keys()
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("rollout-"))
                })
                .cloned()
                .collect();
            files.sort_by(|a, b| rows[b].modified_ns.cmp(&rows[a].modified_ns).then(a.cmp(b)));
            return Ok(files);
        }
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
        if let Some(stamp) = self.linux_rollouts.as_ref().and_then(|rows| rows.get(path)) {
            cache.tail.set_linux_stamp(stamp.clone());
        }
        let before_read = cache.tail.bytes_read_total;
        let trusted = cache.tail.drain(path, |batch| {
            if batch.reset {
                cache.state = None;
            }
            if !batch.body.trim().is_empty() {
                let sid = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                cache.state = apply_codex_rollout(&batch.body, now, sid, cache.state.take());
            }
        });
        self.read_bytes = self
            .read_bytes
            .saturating_add(cache.tail.bytes_read_total - before_read);
        let trusted = trusted.ok()?;
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
        task.model = host.model.clone();
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
        task.input_request_ids = host.pending_input_ids();
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
    pub model: Option<String>,
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
    questions: questions::Questions,
    turn_completed: bool,
    pub session_id: String,
    /// The first native header owns this rollout. Forked transcripts can embed
    /// another session_meta from the parent; that is history, not a new owner.
    metadata_session_id: Option<String>,
    pub turn_key: Option<String>,
    pub first_user_message: Option<String>,
    pub quota_5h_used_pct: Option<f64>,
    pub quota_5h_resets_at: Option<i64>,
    pub quota_1w_used_pct: Option<f64>,
    pub quota_1w_resets_at: Option<i64>,
}

impl SessionState {
    fn parser_protected(&self) -> bool {
        self.status == TaskStatus::Running
            || !self.confirmed_approvals.is_empty()
            || !self.internal && self.questions.has_active()
    }

    fn pending_input_ids(&self) -> Vec<String> {
        match self.waiting_reason {
            Some(crate::domain::WaitingReason::Question) => self.questions.active_ids(),
            Some(crate::domain::WaitingReason::Approval) => {
                self.confirmed_approvals.keys().cloned().collect()
            }
            _ => Vec::new(),
        }
    }
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

fn internal_codex_source(source: &serde_json::Value) -> bool {
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
        || source
            .pointer("/subagent/thread_spawn/parent_thread_id")
            .and_then(|v| v.as_str())
            .is_some_and(|parent| !parent.trim().is_empty())
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
            internal: internal_codex_source(&meta.source),
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
        mut model,
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
        mut questions,
        mut turn_completed,
        mut session_id,
        mut metadata_session_id,
        mut turn_key,
        mut first_user_message,
        mut quota_5h_used_pct,
        mut quota_5h_resets_at,
        mut quota_1w_used_pct,
        mut quota_1w_resets_at,
    } = previous.unwrap_or_else(|| SessionState {
        model: None,
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
        questions: Default::default(),
        turn_completed: false,
        session_id: default_session_id.into(),
        metadata_session_id: None,
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

        // A fork's first header describes the child, followed by a copied
        // parent header/history. Never let the latter re-key this file, replace
        // its project or classify a normal user fork as a parent's service.
        // A genuine file replacement resets SessionState in cached_session_state.
        let is_session_meta = value.get("type").and_then(|v| v.as_str()) == Some("session_meta")
            || payload.get("type").and_then(|v| v.as_str()) == Some("session_meta");
        if is_session_meta {
            if let Some(id) = payload
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|id| !id.is_empty())
            {
                if metadata_session_id
                    .as_deref()
                    .is_some_and(|owner| owner != id)
                {
                    continue;
                }
                metadata_session_id = Some(id.to_owned());
                session_id = id.to_owned();
            }
            internal |= payload.get("source").is_some_and(internal_codex_source);
        }

        // Model metadata is presentation only: no activity clock, turn or
        // notification changes. Reject late contexts belonging to other turns.
        let envelope = value.get("type").and_then(|v| v.as_str());
        if (envelope == Some("turn_context") || is_session_meta && model.is_none())
            && !payload
                .get("turn_id")
                .and_then(|v| v.as_str())
                .zip(turn_key.as_deref())
                .is_some_and(|(event, current)| event != current)
        {
            if let Some(reported) = payload
                .get("model")
                .and_then(|v| v.as_str())
                .and_then(super::reported_model)
            {
                model = Some(reported);
            }
        }

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
                questions.clear();
                turn_completed = false;
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
                if let Some(changed) = questions.replied(payload) {
                    if changed {
                        if waiting_reason == Some(crate::domain::WaitingReason::Question) {
                            status = if turn_completed {
                                TaskStatus::WaitingInput
                            } else {
                                TaskStatus::Running
                            };
                            waiting_reason =
                                turn_completed.then_some(crate::domain::WaitingReason::TurnEnded);
                            input_request_id = None;
                            phase = (!turn_completed).then_some(if active_tools.is_empty() {
                                crate::domain::ExecutionPhase::AwaitingModel
                            } else {
                                crate::domain::ExecutionPhase::Tool
                            });
                        }
                        if let Some(t) = ts {
                            updated_at = t;
                        }
                    }
                    continue;
                }
                if status != TaskStatus::Running
                    && confirmed_approvals.is_empty()
                    && !questions.has_active()
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
                    questions.clear();
                    turn_completed = false;
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
                questions.called(payload);
                if let Some(id) = questions.active_ids().first() {
                    status = TaskStatus::WaitingInput;
                    waiting_reason = Some(crate::domain::WaitingReason::Question);
                    input_request_id = Some(id.clone());
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
                questions.returned(payload);
                if waiting_reason == Some(crate::domain::WaitingReason::Question)
                    && !questions.has_active()
                {
                    status = if turn_completed {
                        TaskStatus::WaitingInput
                    } else {
                        TaskStatus::Running
                    };
                    waiting_reason =
                        turn_completed.then_some(crate::domain::WaitingReason::TurnEnded);
                    input_request_id = None;
                    phase = (!turn_completed).then_some(if active_tools.is_empty() {
                        crate::domain::ExecutionPhase::AwaitingModel
                    } else {
                        crate::domain::ExecutionPhase::Tool
                    });
                }
            }
            "task_complete" => {
                questions.finish_turn();
                turn_completed = true;
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
                questions.clear();
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

    // Human requests are independent of tool execution / dispatch success.
    // In particular an accepted async question can coexist with ongoing work
    // or an ordinary completed turn, and must never become a green end notice.
    if confirmed_approvals.is_empty()
        && waiting_reason != Some(crate::domain::WaitingReason::QuotaExhausted)
    {
        if let Some(id) = questions.active_ids().first() {
            status = TaskStatus::WaitingInput;
            waiting_reason = Some(crate::domain::WaitingReason::Question);
            input_request_id = Some(id.clone());
            phase = None;
        }
    }

    let state = SessionState {
        model,
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
        questions,
        turn_completed,
        session_id,
        metadata_session_id,
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
    let mut rollout_owner: Option<String> = None;
    for line in section.lines() {
        if line.starts_with("===S:") {
            rollout_owner = None;
        } else if let Some(raw) = line.strip_prefix("META\t") {
            if let Ok(mut meta) = serde_json::from_str::<RemoteThreadMeta>(raw) {
                if !meta.id.trim().is_empty() {
                    meta.internal = internal_codex_source(&meta.source);
                    // Retain origin even if the native title is unavailable.
                    metadata.insert(meta.id.clone(), meta);
                }
            }
        } else if let Ok(row) = serde_json::from_str::<serde_json::Value>(line) {
            if row.get("type").and_then(|v| v.as_str()) == Some("session_meta")
                || row.pointer("/payload/type").and_then(|v| v.as_str()) == Some("session_meta")
            {
                if let Some(payload) = row.get("payload") {
                    if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
                        if rollout_owner.as_deref().is_some_and(|owner| owner != id) {
                            continue;
                        }
                        rollout_owner = Some(id.to_owned());
                        if payload.get("source").is_some_and(internal_codex_source) {
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
        let pending_input_ids = state.pending_input_ids();
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
        t.input_request_ids = pending_input_ids;
        t.input_request_id = state.input_request_id;
        t.started_at = state.started_at;
        t.message = state.last_message;
        t.activity = state.activity;
        t.phase = state.phase;
        t.model = state.model;
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
        let poll_read_before = self.read_bytes;
        self.refresh_title_index();
        let discovered = match self.discover() {
            Ok(files) => files,
            Err(error) => {
                let message = format!("session discovery failed: {error}");
                log::warn!("codex: session discovery failed; preserving previous tasks");
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
            Err(_) => {
                log::warn!("codex: metadata unavailable; falling back to rollout metadata");
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
                let protected = self
                    .cache
                    .get(path)
                    .is_some_and(|c| c.state.as_ref().is_some_and(SessionState::parser_protected));
                let stamp = self.linux_rollouts.as_ref().and_then(|rows| rows.get(path));
                let eligible = if let Some(s) = stamp {
                    self.cache_policy.eligible_observed(
                        UNIX_EPOCH + Duration::from_nanos(s.modified_ns),
                        UNIX_EPOCH + Duration::from_nanos(s.changed_ns),
                        protected,
                        self.cache.contains_key(path),
                    )
                } else {
                    self.cache_policy
                        .eligible(path, protected, self.cache.contains_key(path))
                };
                let modified = stamp
                    .map(|s| UNIX_EPOCH + Duration::from_nanos(s.modified_ns))
                    .or_else(|| path.metadata().and_then(|m| m.modified()).ok());
                eligible
                    && (eligible_paths.contains(path)
                        || protected
                        || modified.is_some_and(|modified| {
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
            // A complete OLD rollout is not a complete observation of a
            // resumed session whose other rollout is still catching up or
            // unreadable. Partial evidence for another session is independent.
            if self.cache.iter().any(|(path, cache)| {
                !cache.tail.trusted()
                    && (cache
                        .state
                        .as_ref()
                        .is_some_and(|partial| partial.session_id == state.session_id)
                        || metadata
                            .get(&state.session_id)
                            .and_then(|m| m.rollout_path.as_deref())
                            .is_some_and(|p| self.resolve_rollout_path(p) == *path))
            }) {
                continue;
            }
            live.insert(state.session_id.clone(), state.updated_at);
            let mut task = self.session_to_task(&state, metadata.get(&state.session_id), now);
            if state.status == TaskStatus::Running
                && now.duration_since(state.updated_at).unwrap_or_default() > RUNNING_TTL
            {
                task.stale = true;
            }
            let observer_root = self.sessions_root.parent().unwrap_or(&self.sessions_root);
            if !matches!(observer::apply(observer_root, &mut task), Ok(true)) {
                if let Some(previous) = self.last_tasks.iter().find(|old| {
                    old.id == task.id
                        && old.turn_key == task.turn_key
                        && old
                            .input_request_id
                            .as_deref()
                            .is_some_and(|id| id.starts_with("hook:"))
                        && task.status == TaskStatus::Running
                }) {
                    // Lost/corrupt observer state is uncertainty, not approval resolution.
                    task = previous.clone();
                    task.stale = true;
                }
            }
            tasks.push(task);
        }
        let emitted: std::collections::HashSet<String> = tasks
            .iter()
            .filter_map(|task| task.session_id.clone())
            .collect();
        for meta in metadata.values().filter(|meta| meta.pinned) {
            if emitted.contains(&meta.id)
                || self
                    .last_tasks
                    .iter()
                    .any(|t| t.session_id.as_deref() == Some(&meta.id) && parser_protected(t))
            {
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
        let items = self
            .cache
            .iter()
            .map(|(p, c)| {
                let protected = !c.tail.trusted()
                    || c.state.as_ref().is_some_and(SessionState::parser_protected);
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
            .collect::<Vec<_>>();
        let keep = if let Some(rows) = &self.linux_rollouts {
            self.cache_policy.keep_observed(
                items
                    .into_iter()
                    .map(|(p, protected, bytes)| {
                        let at = rows.get(&p).map_or(UNIX_EPOCH, |s| {
                            UNIX_EPOCH + Duration::from_nanos(s.modified_ns.max(s.changed_ns))
                        });
                        (p, protected, bytes, at)
                    })
                    .collect(),
                now,
            )
        } else {
            self.cache_policy.keep(items, now)
        };
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
                if self.linux_rollouts.is_some() || previous.requires_action() {
                    previous.stale = true;
                }
                if let Some(meta) = previous.session_id.as_ref().and_then(|id| metadata.get(id)) {
                    previous.name = thread_task_name(meta);
                }
                tasks.push(previous);
            }
        }
        let missing_protected = self.linux_rollouts.is_some()
            && self.last_tasks.iter().any(|t| {
                parser_protected(t) && !tasks.iter().any(|new| new.id == t.id && !new.stale)
            });
        let incomplete =
            missing_protected || self.cache.values().any(|cache| !cache.tail.trusted());
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
        let tails:Vec<_> = self.cache.iter().take(128).map(|(p,c)| serde_json::json!({
            "file_hash": &crate::integrations::key(&p.to_string_lossy())[..16],
            "session_hash": c.state.as_ref().map(|s|crate::integrations::key(&s.session_id)[..16].to_string()),
            "turn_hash": c.state.as_ref().and_then(|s|s.turn_key.as_ref()).map(|s|crate::integrations::key(s)[..16].to_string()),
            "cursor": c.tail.diagnostics()
        })).collect();
        super::resources::record(
            "codex",
            &self.sessions_root,
            serde_json::json!({"entries":self.cache.len(),"protected_entries":self.cache.values().filter(|c|!c.tail.trusted() || c.state.as_ref().is_some_and(SessionState::parser_protected)).count(),"tail_capacity_bytes":self.cache.values().map(|c|c.tail.retained_bytes()).sum::<usize>(),"cumulative_rollout_bytes_read":self.read_bytes,"poll_rollout_bytes_read":self.read_bytes-poll_read_before,"reset_count":self.cache.values().map(|c|c.tail.resets).sum::<u64>(),"validation_bytes":self.cache.values().map(|c|c.tail.validation_bytes_total).sum::<u64>(),"inactive_entry_budget":super::parser_cache::INACTIVE_ENTRIES,"inactive_estimated_byte_budget":super::parser_cache::INACTIVE_BYTES,"tails_truncated":self.cache.len()>128,"tails":tails}),
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
            error: incomplete.then(|| {
                if self.linux_rollouts.is_some() {
                    if self.cache.values().any(|c| c.tail.size_raced()) {
                        "[wsl:rollout_read:size_race] 文件正在追加；等待一致长度的观测"
                    } else if self.cache.values().any(|c| c.tail.read_failed()) {
                        "[wsl:rollout_read:io] 读取失败；保留上次观测"
                    } else if self.cache.values().any(|c| c.tail.has_gap()) {
                        "[wsl:rollout_decode:gap] 日志存在坏行；保留上次观测"
                    } else if missing_protected && self.cache.values().all(|c| c.tail.trusted()) {
                        "[wsl:rollout_discovery:missing_session] 已知会话日志缺失；保留上次观测"
                    } else {
                        "[wsl:rollout_read:catch_up] 日志追赶中；不发布中间终态"
                    }
                } else {
                    "Codex 日志追赶中或存在坏行；不发布中间终态"
                }
                .into()
            }),
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
mod tests;
