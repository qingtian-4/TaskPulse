//! Watcher — the orchestration loop.
//!
//! Pipeline:  Provider → Normalize → Store → Transition Engine → Tray / UI
//!
//! * `LocalFileProvider`  — hook/wrapper status files (`~/.agent-monitor`,
//!   `~/.taskpulse`), watched for changes.
//! * `Scanner`           — local agent process scan (AgentTray's mature
//!   implementation, converted to `TaskSnapshot`).
//! * `RemoteTmuxProvider` — parallel SSH polling of Server1/Server3, on its
//!   own thread so a slow host never blocks local updates.
//!
//! The `agents-updated` event contract, tray icon updates, and the popup
//! commands are preserved from AgentTray.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::attention::AttentionTracker;
use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource};
use crate::history::HistoryStore;
use crate::providers::codex::CodexProvider;
use crate::providers::deepseek_harness::DeepSeekHarnessProvider;
use crate::providers::hermes::HermesProvider;
use crate::providers::local_file::LocalFileProvider;
use crate::providers::remote_tmux::{
    config::{self, MonitoringConfig},
    ssh::SystemSshExecutor,
    RemoteTmuxProvider,
};
use crate::providers::{HostHealth, Provider, ProviderHealth, ProviderOutput, ProviderStatus};
use crate::scanner::Scanner;
use crate::store::TaskStore;
use crate::transition::TransitionEngine;

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

// Backward-compat re-exports (scanner / heuristics / focusers reference these).
pub use crate::domain::TerminalInfo;
pub use crate::providers::local_file::status_from_legacy;

/// Legacy AgentTray status type — kept so the scanner stack and heuristics
/// keep working unchanged; converted to `TaskSnapshot` at the boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalInfo>,
    pub can_focus: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_matcher: Option<String>,
    #[serde(skip)]
    pub mtime: Option<SystemTime>,
}

impl From<AgentStatus> for TaskSnapshot {
    fn from(a: AgentStatus) -> Self {
        let status = status_from_legacy(&a.status);
        let confidence = match a.source.as_deref() {
            Some("hook") | Some("wrap") => DetectionConfidence::Hook,
            _ => DetectionConfidence::Inferred,
        };
        let source = match a.cli.as_deref() {
            Some("codex") => TaskSource::Codex,
            Some("claude-code") => TaskSource::ClaudeCode,
            Some("gemini") => TaskSource::Gemini,
            _ => TaskSource::LocalFile,
        };
        let now = SystemTime::now();
        TaskSnapshot {
            id: a.id,
            name: a.name,
            source,
            status,
            host: None,
            project: None,
            message: (!a.message.is_empty()).then_some(a.message),
            activity: None,
            started_at: None,
            updated_at: a.mtime.unwrap_or(now),
            finished_at: None,
            pid: None,
            exit_code: None,
            confidence,
            remote: false,
            stale: false,
            recent: false,
            unread: false,
            pinned: false,
            turn_key: None,
            native_unread: None,
            terminal: a.terminal,
            can_focus: a.can_focus,
            cpu: a.cpu,
            cli: a.cli,
            session_id: a.session_id,
            hook_event: a.hook_event,
            hook_matcher: a.hook_matcher,
            quota_5h_used_pct: None,
            quota_5h_resets_at: None,
            quota_1w_used_pct: None,
            quota_1w_resets_at: None,
        }
    }
}

/// Legacy numeric priority (scanner uses this for tty dedup).
pub fn status_priority_num(status: &str) -> u8 {
    match status {
        "needs-input" => 0,
        "error" => 1,
        "working" => 2,
        "starting" => 3,
        "idle" => 4,
        _ => 5,
    }
}

pub fn status_dir() -> Option<PathBuf> {
    dirs_next::home_dir().map(|h| h.join(".agent-monitor"))
}

/// Cached latest merged task list for on-demand popup refresh.
static LATEST_TASKS: Mutex<Vec<TaskSnapshot>> = Mutex::new(Vec::new());
static LATEST_HOSTS: Mutex<Vec<HostHealth>> = Mutex::new(Vec::new());
static LATEST_PROVIDERS: Mutex<Vec<ProviderHealth>> = Mutex::new(Vec::new());
static ATTENTION_TRACKER: OnceLock<Mutex<AttentionTracker>> = OnceLock::new();
static MANUAL_REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);

fn reconcile_attention(tasks: Vec<TaskSnapshot>) -> Vec<TaskSnapshot> {
    let mut tasks = tasks;
    crate::providers::codex::apply_authoritative_quota(&mut tasks);
    let Some(tracker) = ATTENTION_TRACKER.get() else {
        return tasks;
    };
    tracker
        .lock()
        .unwrap_or_else(|error| {
            log::warn!("ATTENTION_TRACKER mutex poisoned, recovering");
            error.into_inner()
        })
        .reconcile(tasks)
}

/// Update one task's TaskPulse-owned attention state.
#[tauri::command]
pub fn update_task_attention(
    app: AppHandle,
    id: String,
    action: String,
    pinned: Option<bool>,
) -> Result<Vec<TaskSnapshot>, String> {
    let tracker = ATTENTION_TRACKER
        .get()
        .ok_or_else(|| "attention tracker is still starting".to_string())?;
    let tasks = {
        let mut tracker = tracker.lock().unwrap_or_else(|error| error.into_inner());
        match action.as_str() {
            "viewed" => tracker.mark_viewed(&id)?,
            "hide" => tracker.hide(&id)?,
            "restore" => tracker.restore(&id)?,
            "pin" => tracker.set_pinned(&id, pinned.unwrap_or(true))?,
            _ => return Err(format!("unsupported attention action: {action}")),
        }
    };
    let hosts = get_hosts();
    let providers = get_providers();
    emit_agents(&app, &tasks, &hosts, &providers);
    Ok(tasks)
}

fn emit_agents(
    app: &AppHandle,
    tasks: &[TaskSnapshot],
    hosts: &[HostHealth],
    providers: &[ProviderHealth],
) {
    log::info!("emit_agents: {} tasks, {} hosts", tasks.len(), hosts.len());
    for t in tasks {
        log::debug!(
            "  task: name={:?} status={:?} conf={:?}",
            t.name,
            t.status,
            t.confidence
        );
    }
    {
        let mut cache = LATEST_TASKS.lock().unwrap_or_else(|e| {
            log::warn!("LATEST_TASKS mutex poisoned, recovering");
            e.into_inner()
        });
        *cache = tasks.to_vec();
    }
    {
        let mut cache = LATEST_HOSTS.lock().unwrap_or_else(|e| {
            log::warn!("LATEST_HOSTS mutex poisoned, recovering");
            e.into_inner()
        });
        *cache = hosts.to_vec();
    }
    {
        let mut cache = LATEST_PROVIDERS.lock().unwrap_or_else(|e| {
            log::warn!("LATEST_PROVIDERS mutex poisoned, recovering");
            e.into_inner()
        });
        *cache = providers.to_vec();
    }
    crate::tray::update_icon(app, tasks, hosts);
    let _ = app.emit("agents-updated", tasks);
    let _ = app.emit("hosts-updated", hosts);
    let _ = app.emit("providers-updated", providers);
}

/// Re-emit the last known task list (used when popup opens).
pub fn emit_latest(app: &AppHandle) {
    let tasks = LATEST_TASKS
        .lock()
        .unwrap_or_else(|e| {
            log::warn!("LATEST_TASKS mutex poisoned, recovering");
            e.into_inner()
        })
        .clone();
    let hosts = LATEST_HOSTS
        .lock()
        .unwrap_or_else(|e| {
            log::warn!("LATEST_HOSTS mutex poisoned, recovering");
            e.into_inner()
        })
        .clone();
    let providers = LATEST_PROVIDERS
        .lock()
        .unwrap_or_else(|e| {
            log::warn!("LATEST_PROVIDERS mutex poisoned, recovering");
            e.into_inner()
        })
        .clone();
    crate::tray::update_icon(app, &tasks, &hosts);
    let _ = app.emit("agents-updated", &tasks);
    let _ = app.emit("hosts-updated", &hosts);
    let _ = app.emit("providers-updated", &providers);
}

/// Tauri command: return the platform-appropriate status directory path.
#[tauri::command]
pub fn get_status_dir() -> String {
    status_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// Tauri command: return cached tasks (called by frontend on mount).
#[tauri::command]
pub fn get_agents() -> Vec<TaskSnapshot> {
    let tasks = LATEST_TASKS
        .lock()
        .unwrap_or_else(|e| {
            log::warn!("LATEST_TASKS mutex poisoned, recovering");
            e.into_inner()
        })
        .clone();
    log::info!("get_agents: returning {} tasks", tasks.len());
    tasks
}

/// Tauri command: return cached remote-host health with the cached tasks.
#[tauri::command]
pub fn get_hosts() -> Vec<HostHealth> {
    LATEST_HOSTS
        .lock()
        .unwrap_or_else(|e| {
            log::warn!("LATEST_HOSTS mutex poisoned, recovering");
            e.into_inner()
        })
        .clone()
}

/// Tauri command: return cached local-provider health.
#[tauri::command]
pub fn get_providers() -> Vec<ProviderHealth> {
    LATEST_PROVIDERS
        .lock()
        .unwrap_or_else(|e| {
            log::warn!("LATEST_PROVIDERS mutex poisoned, recovering");
            e.into_inner()
        })
        .clone()
}

/// Tauri command: ask every provider to poll again instead of merely returning
/// the last cached snapshot. The watcher owns provider state, so the command is
/// intentionally non-blocking and fresh results arrive through update events.
#[tauri::command]
pub fn request_refresh() {
    MANUAL_REFRESH_REQUESTED.store(true, Ordering::Release);
}

/// Tauri command: install or uninstall AgentTray/TaskPulse hooks for a CLI
/// tool. `cli` = "claude", "codex", "gemini", or "all"; `uninstall` removes.
#[tauri::command]
pub fn install_hooks(cli: String, uninstall: bool) -> Result<String, String> {
    let base_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .and_then(|dir| {
            dir.ancestors()
                .find(|d| d.join("scripts").exists())
                .map(|d| d.to_path_buf())
                .or(Some(dir))
        })
        .ok_or_else(|| "Could not locate scripts directory".to_string())?;

    let (program, script) = if cfg!(target_os = "windows") {
        let ps1 = base_dir.join("scripts/hooks/install-hooks.ps1");
        if ps1.exists() {
            ("powershell".to_string(), ps1)
        } else {
            let sh = base_dir.join("scripts/hooks/install-hooks.sh");
            ("bash".to_string(), sh)
        }
    } else {
        let sh = base_dir.join("scripts/hooks/install-hooks.sh");
        ("bash".to_string(), sh)
    };

    if !script.exists() {
        return Err(format!("Hook installer not found at: {}", script.display()));
    }

    let mut cmd = std::process::Command::new(&program);
    if program == "powershell" {
        cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
        cmd.arg(&script);
        cmd.args(["-Target", &cli]);
        if uninstall {
            cmd.arg("-Uninstall");
        }
    } else {
        cmd.arg(&script).arg(&cli);
        if uninstall {
            cmd.arg("--uninstall");
        }
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run installer: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        log::info!("Hook installation succeeded for '{cli}': {}", stdout.trim());
        Ok(stdout)
    } else {
        let msg = if stderr.is_empty() { &stdout } else { &stderr };
        log::error!("Hook installation failed for '{cli}': {}", msg.trim());
        Err(format!("Installation failed: {}", msg.trim()))
    }
}

/// Cross-provider dedup: when a hook/file task and a scan task share the same
/// terminal focus_id and CLI, keep the higher-confidence one (hook beats
/// scan) and drop the duplicate.  Mirrors AgentTray's focus_id budget logic
/// for the common case.
#[allow(clippy::ptr_arg)]
fn dedup_by_terminal(tasks: &mut Vec<TaskSnapshot>) {
    let mut keep = vec![true; tasks.len()];
    let mut best: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();

    for (i, t) in tasks.iter().enumerate() {
        let Some(term) = &t.terminal else { continue };
        if term.focus_id.is_empty() || term.focus_id == "0" {
            continue;
        }
        let cli = t.cli.clone().unwrap_or_default();
        let key = (term.focus_id.clone(), cli);
        match best.get(&key) {
            Some(&prev) => {
                let prev_conf = tasks[prev].confidence;
                let cur_conf = t.confidence;
                if cur_conf < prev_conf {
                    keep[prev] = false;
                    best.insert(key, i);
                } else if cur_conf == prev_conf {
                    // Same confidence: distinct sessions in one window — keep both.
                    continue;
                } else {
                    keep[i] = false;
                }
            }
            None => {
                best.insert(key, i);
            }
        }
    }

    let mut out = Vec::with_capacity(tasks.len());
    for (i, t) in tasks.drain(..).enumerate() {
        if keep[i] {
            out.push(t);
        }
    }
    *tasks = out;
}

/// Merge provider output into the store, run the transition engine, notify,
/// and emit to tray + UI.
/// Session-metadata (Codex) beats process-scan (Codex): if a real Codex
/// session task exists, drop the scanner's inferred scan entry for the same
/// CLI so we never show the same agent twice.
fn dedup_codex_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    let has_session = tasks.iter().any(|t| t.source == TaskSource::Codex);
    if has_session {
        tasks.retain(|t| {
            !(t.cli.as_deref() == Some("codex") && t.confidence == DetectionConfidence::Inferred)
        });
    }
}

fn dedup_hermes_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    let has_session = tasks.iter().any(|t| t.source == TaskSource::Hermes);
    if has_session {
        tasks.retain(|t| {
            !(t.cli.as_deref() == Some("hermes") && t.confidence == DetectionConfidence::Inferred)
        });
    }
}

fn dedup_dsh_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    let has_session = tasks
        .iter()
        .any(|t| t.source == TaskSource::DeepSeekHarness);
    if has_session {
        tasks.retain(|t| {
            !(t.cli.as_deref() == Some("dsh") && t.confidence == DetectionConfidence::Inferred)
        });
    }
}

struct LocalBatch {
    tasks: Vec<TaskSnapshot>,
    authoritative_sources: HashSet<TaskSource>,
}

struct LocalProviders {
    file: LocalFileProvider,
    workers: Vec<(
        &'static str,
        Vec<TaskSource>,
        crate::providers::worker::Worker,
    )>,
    latest: std::collections::HashMap<&'static str, ProviderOutput>,
    observed_at: std::collections::HashMap<&'static str, Instant>,
    started_at: Instant,
    health: Vec<ProviderHealth>,
    monitoring: MonitoringConfig,
}

impl LocalProviders {
    fn new(monitoring: MonitoringConfig) -> Self {
        use crate::providers::worker::Worker;
        let mut codex = CodexProvider::new();
        let mut hermes = HermesProvider::new();
        let mut dsh = DeepSeekHarnessProvider::new();
        let mut scanner = Scanner::new();
        Self {
            file: LocalFileProvider::new(),
            workers: vec![
                (
                    "codex",
                    vec![TaskSource::Codex],
                    Worker::spawn(Duration::from_secs(1), move |_| {
                        codex.poll(SystemTime::now())
                    }),
                ),
                (
                    "hermes",
                    vec![TaskSource::Hermes],
                    Worker::spawn(Duration::from_secs(1), move |_| {
                        hermes.poll(SystemTime::now())
                    }),
                ),
                (
                    "deepseek_harness",
                    vec![TaskSource::DeepSeekHarness],
                    Worker::spawn(Duration::from_secs(2), move |_| dsh.poll(SystemTime::now())),
                ),
                (
                    "scanner",
                    vec![TaskSource::ClaudeCode, TaskSource::Gemini],
                    Worker::spawn(Duration::from_secs(2), move |_| {
                        ProviderOutput::tasks(scanner.scan().into_iter().map(Into::into).collect())
                    }),
                ),
            ],
            latest: Default::default(),
            observed_at: Default::default(),
            started_at: Instant::now(),
            health: ["codex", "hermes", "deepseek_harness", "scanner"]
                .into_iter()
                .map(|provider| ProviderHealth {
                    provider: provider.to_string(),
                    status: ProviderStatus::Degraded,
                    last_success: None,
                    message: Some("首次采集中".into()),
                })
                .collect(),
            monitoring,
        }
    }

    fn watch_paths(&self) -> Vec<(PathBuf, RecursiveMode)> {
        let mut paths: Vec<_> = self
            .file
            .status_dirs()
            .into_iter()
            .map(|path| (path, RecursiveMode::NonRecursive))
            .collect();
        if let Some(home) = dirs_next::home_dir() {
            // Rollouts and SQLite WAL writes carry start/end/rename changes.
            paths.push((home.join(".codex/sessions"), RecursiveMode::Recursive));
            paths.push((home.join(".codex"), RecursiveMode::NonRecursive));
            paths.push((home.join(".hermes"), RecursiveMode::NonRecursive));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push((
                PathBuf::from(local).join("hermes"),
                RecursiveMode::NonRecursive,
            ));
        }
        if let Some(db) = std::env::var_os("HERMES_STATE_DB").map(PathBuf::from) {
            if let Some(parent) = db.parent() {
                paths.push((parent.to_path_buf(), RecursiveMode::NonRecursive));
            }
        }
        paths
    }

    fn request_refresh(&self) {
        for (_, _, worker) in &self.workers {
            worker.request_refresh();
        }
    }

    fn request_native_refresh(&self) {
        for (name, _, worker) in &self.workers {
            if *name == "codex" || *name == "hermes" {
                worker.request_refresh();
            }
        }
    }

    fn collect_ready(&mut self) -> bool {
        let mut changed = false;
        for (name, _, worker) in &self.workers {
            while let Ok(observation) = worker.observations.try_recv() {
                log::debug!("provider {name}: {} ms", observation.duration_ms);
                update_provider_health(
                    &mut self.health,
                    name,
                    &observation.output,
                    observation.completed_at,
                );
                self.latest.insert(name, observation.output);
                self.observed_at.insert(name, Instant::now());
                changed = true;
            }
            let last = self
                .observed_at
                .get(name)
                .copied()
                .unwrap_or(self.started_at);
            if last.elapsed() > Duration::from_secs(30) {
                let error = "采集超过 30 秒未返回；保留上次状态";
                if let Some(health) = self
                    .health
                    .iter_mut()
                    .find(|health| health.provider == *name)
                {
                    if health.message.as_deref() != Some(error) {
                        health.status = ProviderStatus::Degraded;
                        health.message = Some(error.into());
                        if let Some(output) = self.latest.get_mut(name) {
                            output.authoritative = false;
                            output.error = Some(error.into());
                            for task in &mut output.tasks {
                                task.stale = true;
                            }
                        }
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    fn poll(&mut self, now: SystemTime) -> LocalBatch {
        self.collect_ready();
        let file = self.file.poll(now);
        update_provider_health(&mut self.health, "local_file", &file, now);
        let mut authoritative_sources = HashSet::new();
        if file.authoritative {
            authoritative_sources.insert(TaskSource::LocalFile);
        }
        let mut tasks = file.tasks;
        for (name, sources, _) in &self.workers {
            if let Some(output) = self.latest.get(name) {
                if output.authoritative {
                    authoritative_sources.extend(sources.iter().copied());
                }
                if *name != "scanner" {
                    tasks.extend(output.tasks.clone());
                }
            }
        }
        let scanned = self
            .latest
            .get("scanner")
            .map(|out| out.tasks.as_slice())
            .unwrap_or(&[]);
        crate::heuristics::cross_validate_tasks(&mut tasks, scanned);
        tasks.extend_from_slice(scanned);
        dedup_by_terminal(&mut tasks);
        dedup_codex_over_scan(&mut tasks);
        dedup_hermes_over_scan(&mut tasks);
        dedup_dsh_over_scan(&mut tasks);
        tasks.retain(|task| self.monitoring.allows(task));
        LocalBatch {
            tasks,
            authoritative_sources,
        }
    }
}

fn update_provider_health(
    health: &mut Vec<ProviderHealth>,
    provider: &str,
    output: &ProviderOutput,
    now: SystemTime,
) {
    let previous_success = health
        .iter()
        .find(|item| item.provider == provider)
        .and_then(|item| item.last_success);
    let next = if output.authoritative {
        ProviderHealth {
            provider: provider.to_string(),
            status: ProviderStatus::Connected,
            last_success: Some(now),
            message: None,
        }
    } else {
        ProviderHealth {
            provider: provider.to_string(),
            status: ProviderStatus::Degraded,
            last_success: previous_success,
            message: output.error.clone(),
        }
    };
    if let Some(existing) = health.iter_mut().find(|item| item.provider == provider) {
        *existing = next;
    } else {
        health.push(next);
    }
}

#[allow(clippy::too_many_arguments)]
fn merge_and_emit(
    app: &AppHandle,
    store: &mut TaskStore,
    engine: &mut TransitionEngine,
    tasks: Vec<TaskSnapshot>,
    hosts: &[HostHealth],
    providers: &[ProviderHealth],
    history: &mut HistoryStore,
    now: SystemTime,
    remote: bool,
) {
    let merged = store.merge_partition(tasks, remote, now);
    let events = engine.apply(&merged, hosts, now);
    for e in &events {
        log::info!("transition event: {e:?}");
    }
    crate::notifier::handle_events(app, &events, &merged);
    if let Err(error) = history.save_if_changed(store.recent()) {
        log::warn!("failed to persist recent task history: {error}");
    }
    let display = reconcile_attention(merged);
    emit_agents(app, &display, hosts, providers);
}

/// Refresh local sources (file provider + Codex session metadata + scanner),
/// dedup, and emit.
fn refresh_local(
    app: &AppHandle,
    store: &mut TaskStore,
    engine: &mut TransitionEngine,
    providers: &mut LocalProviders,
    history: &mut HistoryStore,
    hosts: &[HostHealth],
    now: SystemTime,
) {
    let batch = providers.poll(now);
    let merged = store.merge_sources(batch.tasks, &batch.authoritative_sources, now);
    let events = engine.apply(&merged, hosts, now);
    for event in &events {
        log::info!("transition event: {event:?}");
    }
    crate::notifier::handle_events(app, &events, &merged);
    if let Err(error) = history.save_if_changed(store.recent()) {
        log::warn!("failed to persist recent task history: {error}");
    }
    let display = reconcile_attention(merged);
    emit_agents(app, &display, hosts, &providers.health);
}

fn remote_interval_ms(provider: &RemoteTmuxProvider) -> Duration {
    let cfg = provider.config_for_poll();
    let min_ms = cfg.iter().map(|h| h.poll_interval_ms).min().unwrap_or(3000);
    Duration::from_millis(min_ms.max(500))
}

pub fn watch(app: AppHandle) {
    // Ensure status directories exist.
    if let Some(dir) = status_dir() {
        let _ = std::fs::create_dir_all(&dir);
    }
    if let Some(dir) = dirs_next::home_dir().map(|h| h.join(".taskpulse")) {
        let _ = std::fs::create_dir_all(&dir);
    }

    let attention_path = dirs_next::home_dir()
        .map(|home| home.join(".taskpulse/attention.json"))
        .unwrap_or_else(|| PathBuf::from(".taskpulse/attention.json"));
    let _ = ATTENTION_TRACKER.set(Mutex::new(AttentionTracker::open(attention_path)));

    let history_path = dirs_next::home_dir()
        .map(|home| home.join(".taskpulse/recent.json"))
        .unwrap_or_else(|| PathBuf::from(".taskpulse/recent.json"));
    let (mut history, restored_recent) = HistoryStore::open(history_path);
    let mut store = TaskStore::default();
    store.restore_recent(restored_recent, SystemTime::now());
    let mut engine = TransitionEngine::new();
    let taskpulse_config = config::load_config();
    let monitoring = taskpulse_config.monitoring.clone();
    if let Some(tracker) = ATTENTION_TRACKER.get() {
        if let Ok(mut tracker) = tracker.lock() {
            tracker.retain(|task| monitoring.allows(task));
        }
    }
    let mut local_providers = LocalProviders::new(monitoring.clone());
    let scan_interval = Duration::from_secs(2);

    // Remote polling on its own thread (SSH can block for seconds).
    let mut remote = RemoteTmuxProvider::new(taskpulse_config, Arc::new(SystemSshExecutor::new()));
    let remote_interval = remote_interval_ms(&remote);
    let (remote_tx, remote_rx) = std::sync::mpsc::channel::<ProviderOutput>();
    let (remote_refresh_tx, remote_refresh_rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        log::info!("Remote tmux poller started (interval {remote_interval:?})");
        loop {
            let now = SystemTime::now();
            let mut out = remote.poll(now);
            out.tasks.retain(|task| monitoring.allows(task));
            if remote_tx.send(out).is_err() {
                log::warn!("Remote poller channel closed, exiting");
                break;
            }
            match remote_refresh_rx.recv_timeout(remote_interval) {
                Ok(()) => while remote_refresh_rx.try_recv().is_ok() {},
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    // Last known host health (kept so local refreshes don't lose it).
    let mut hosts: Vec<HostHealth> = Vec::new();

    // Initial full refresh.
    let now = SystemTime::now();
    refresh_local(
        &app,
        &mut store,
        &mut engine,
        &mut local_providers,
        &mut history,
        &hosts,
        now,
    );

    let watch_paths = local_providers.watch_paths();

    // Outer restart loop: recreates the file watcher whenever the channel
    // disconnects (inotify watch limit exhausted, OS resource pressure).
    loop {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match RecommendedWatcher::new(tx, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                log::error!("Failed to create file watcher: {e}");
                log::info!("Falling back to scan-only mode");
                loop {
                    std::thread::sleep(scan_interval);
                    if MANUAL_REFRESH_REQUESTED.swap(false, Ordering::AcqRel) {
                        local_providers.request_refresh();
                        let _ = remote_refresh_tx.send(());
                    }
                    let now = SystemTime::now();
                    refresh_local(
                        &app,
                        &mut store,
                        &mut engine,
                        &mut local_providers,
                        &mut history,
                        &hosts,
                        now,
                    );
                    while let Ok(out) = remote_rx.try_recv() {
                        hosts = out.hosts.clone();
                        merge_and_emit(
                            &app,
                            &mut store,
                            &mut engine,
                            out.tasks,
                            &out.hosts,
                            &local_providers.health,
                            &mut history,
                            now,
                            true,
                        );
                    }
                }
            }
        };

        for (path, mode) in &watch_paths {
            if path.exists() {
                if let Err(error) = watcher.watch(path, *mode) {
                    log::warn!(
                        "Failed to watch {}: {error}; polling remains active",
                        path.display()
                    );
                }
            }
        }

        log::info!(
            "Watching {:?} + scanning processes + polling remote hosts",
            watch_paths
        );

        let debounce = Duration::from_millis(50);
        let mut last_emit = Instant::now() - debounce;
        let mut last_scan = Instant::now();

        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(event)) => {
                    if matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    ) && last_emit.elapsed() >= debounce
                    {
                        local_providers.request_native_refresh();
                        let now = SystemTime::now();
                        refresh_local(
                            &app,
                            &mut store,
                            &mut engine,
                            &mut local_providers,
                            &mut history,
                            &hosts,
                            now,
                        );
                        last_emit = Instant::now();
                    }
                }
                Ok(Err(e)) => log::warn!("Watch error: {e}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    log::warn!("File watcher channel disconnected, restarting watcher in 2s");
                    std::thread::sleep(Duration::from_secs(2));
                    break;
                }
            }

            if MANUAL_REFRESH_REQUESTED.swap(false, Ordering::AcqRel) {
                local_providers.request_refresh();
                let _ = remote_refresh_tx.send(());
                let now = SystemTime::now();
                refresh_local(
                    &app,
                    &mut store,
                    &mut engine,
                    &mut local_providers,
                    &mut history,
                    &hosts,
                    now,
                );
                last_scan = Instant::now();
                last_emit = Instant::now();
            }

            // Drain remote poll results (parallel SSH polling thread).
            while let Ok(out) = remote_rx.try_recv() {
                let now = SystemTime::now();
                hosts = out.hosts.clone();
                merge_and_emit(
                    &app,
                    &mut store,
                    &mut engine,
                    out.tasks,
                    &out.hosts,
                    &local_providers.health,
                    &mut history,
                    now,
                    true,
                );
            }

            // Publish worker results immediately, even when another source is slow.
            if local_providers.collect_ready() || last_scan.elapsed() >= scan_interval {
                let now = SystemTime::now();
                refresh_local(
                    &app,
                    &mut store,
                    &mut engine,
                    &mut local_providers,
                    &mut history,
                    &hosts,
                    now,
                );
                last_scan = Instant::now();
                last_emit = Instant::now();
            }
        }
    }
}

/// Read-only, bounded sampling of the same workers used by the tray. This
/// reports native task titles/status and cadence, without prompts or commands.
pub fn diagnose() {
    let config = config::load_config();
    let mut providers = LocalProviders::new(config.monitoring.clone());
    let started = Instant::now();
    let mut samples = std::collections::HashMap::new();
    let mut first_ready = std::collections::HashMap::new();
    let mut durations = std::collections::HashMap::new();
    let mut latest = LocalBatch {
        tasks: Vec::new(),
        authoritative_sources: HashSet::new(),
    };
    let mut store = TaskStore::default();
    let mut engine = TransitionEngine::new();
    let mut event_count = 0;
    while started.elapsed() < Duration::from_secs(10) {
        for (name, _, worker) in &providers.workers {
            while let Ok(observation) = worker.observations.try_recv() {
                *samples.entry(*name).or_insert(0u32) += 1;
                first_ready
                    .entry(*name)
                    .or_insert(started.elapsed().as_millis());
                durations.insert(*name, observation.duration_ms);
                update_provider_health(
                    &mut providers.health,
                    name,
                    &observation.output,
                    observation.completed_at,
                );
                providers.latest.insert(name, observation.output);
                providers.observed_at.insert(name, Instant::now());
            }
        }
        latest = providers.poll(SystemTime::now());
        let merged = store.merge_sources(
            latest.tasks.clone(),
            &latest.authoritative_sources,
            SystemTime::now(),
        );
        event_count += engine.apply(&merged, &[], SystemTime::now()).len();
        std::thread::sleep(Duration::from_millis(100));
    }
    let tasks: Vec<_> = latest
        .tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "id": task.id, "name": task.name, "project": task.project,
                "source": task.source, "status": task.status, "stale": task.stale,
                "has_activity": task.activity.is_some(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({
            "sample_window_ms": started.elapsed().as_millis(), "samples": samples,
            "first_ready_ms": first_ready, "last_poll_ms": durations,
            "providers": providers.health, "tasks": tasks, "transition_count": event_count,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DetectionConfidence, TaskStatus};

    fn task_with_terminal(
        id: &str,
        status: TaskStatus,
        conf: DetectionConfidence,
        focus_id: &str,
        cli: Option<&str>,
    ) -> TaskSnapshot {
        let mut t = TaskSnapshot::new(
            id,
            id,
            TaskSource::LocalFile,
            status,
            conf,
            SystemTime::now(),
        );
        t.terminal = Some(TerminalInfo {
            kind: "windows_native".into(),
            focus_id: focus_id.into(),
            outer_id: String::new(),
            label: "Windows Terminal".into(),
            window_title: None,
        });
        t.cli = cli.map(|s| s.to_string());
        t
    }

    #[test]
    fn dedup_hook_beats_scan_same_focus_and_cli() {
        let mut tasks = vec![
            task_with_terminal(
                "file:codex",
                TaskStatus::WaitingInput,
                DetectionConfidence::Hook,
                "0x1234",
                Some("codex"),
            ),
            task_with_terminal(
                "scan:pts/1",
                TaskStatus::Running,
                DetectionConfidence::Inferred,
                "0x1234",
                Some("codex"),
            ),
        ];
        dedup_by_terminal(&mut tasks);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "file:codex");
    }

    #[test]
    fn dedup_keeps_distinct_sessions_same_focus() {
        // Two hook files in different tabs of one terminal window.
        let mut tasks = vec![
            task_with_terminal(
                "file:a",
                TaskStatus::Running,
                DetectionConfidence::Hook,
                "0x3800005",
                Some("claude-code"),
            ),
            task_with_terminal(
                "file:b",
                TaskStatus::WaitingInput,
                DetectionConfidence::Hook,
                "0x3800005",
                Some("claude-code"),
            ),
        ];
        dedup_by_terminal(&mut tasks);
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn dedup_ignores_different_cli() {
        let mut tasks = vec![
            task_with_terminal(
                "file:codex",
                TaskStatus::WaitingInput,
                DetectionConfidence::Hook,
                "0x1234",
                Some("codex"),
            ),
            task_with_terminal(
                "scan:pts/1",
                TaskStatus::Running,
                DetectionConfidence::Inferred,
                "0x1234",
                Some("gemini"),
            ),
        ];
        dedup_by_terminal(&mut tasks);
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn agent_status_converts_to_snapshot() {
        let a = AgentStatus {
            id: "scan:pts/1".into(),
            name: "proj · Windows Terminal".into(),
            status: "needs-input".into(),
            message: "permission".into(),
            terminal: None,
            can_focus: false,
            cpu: Some(1.5),
            source: Some("scan".into()),
            cli: Some("codex".into()),
            session_id: Some("abc".into()),
            hook_event: None,
            hook_matcher: None,
            mtime: None,
        };
        let t: TaskSnapshot = a.into();
        assert_eq!(t.status, TaskStatus::WaitingInput);
        assert_eq!(t.source, TaskSource::Codex);
        assert_eq!(t.confidence, DetectionConfidence::Inferred);
        assert_eq!(t.message.as_deref(), Some("permission"));
        assert_eq!(t.cpu, Some(1.5));
    }

    #[test]
    fn legacy_priority_order_preserved() {
        assert!(status_priority_num("needs-input") < status_priority_num("error"));
        assert!(status_priority_num("error") < status_priority_num("working"));
        assert!(status_priority_num("working") < status_priority_num("idle"));
    }

    #[test]
    fn provider_health_preserves_last_success_during_degradation() {
        let mut health = Vec::new();
        let now = SystemTime::now();
        update_provider_health(&mut health, "hermes", &ProviderOutput::default(), now);
        assert_eq!(health[0].status, ProviderStatus::Connected);
        assert_eq!(health[0].last_success, Some(now));

        let later = now + Duration::from_secs(5);
        update_provider_health(
            &mut health,
            "hermes",
            &ProviderOutput::degraded(Vec::new(), "database locked"),
            later,
        );
        assert_eq!(health[0].status, ProviderStatus::Degraded);
        assert_eq!(health[0].last_success, Some(now));
        assert_eq!(health[0].message.as_deref(), Some("database locked"));

        let recovered = later + Duration::from_secs(5);
        update_provider_health(&mut health, "hermes", &ProviderOutput::default(), recovered);
        assert_eq!(health[0].status, ProviderStatus::Connected);
        assert_eq!(health[0].last_success, Some(recovered));
        assert!(health[0].message.is_none());
    }
}
