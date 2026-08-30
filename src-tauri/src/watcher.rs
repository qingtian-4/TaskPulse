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
    codex: CodexProvider,
    hermes: HermesProvider,
    dsh_rx: std::sync::mpsc::Receiver<ProviderOutput>,
    dsh_latest: Option<ProviderOutput>,
    scanner: Scanner,
    health: Vec<ProviderHealth>,
    monitoring: MonitoringConfig,
}

impl LocalProviders {
    fn new(monitoring: MonitoringConfig) -> Self {
        // WSL startup can take tens of seconds on Windows. Keep the DeepSeek
        // probe off the local refresh path so Codex/Hermes appear immediately.
        let (dsh_tx, dsh_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut dsh = DeepSeekHarnessProvider::new();
            loop {
                let output = dsh.poll(SystemTime::now());
                if dsh_tx.send(output).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        Self {
            file: LocalFileProvider::new(),
            codex: CodexProvider::new(),
            hermes: HermesProvider::new(),
            dsh_rx,
            dsh_latest: None,
            scanner: Scanner::new(),
            health: Vec::new(),
            monitoring,
        }
    }

    fn primary_dir(&self) -> PathBuf {
        self.file.primary_dir()
    }

    fn poll(&mut self, now: SystemTime) -> LocalBatch {
        let file = self.file.poll(now);
        update_provider_health(&mut self.health, "local_file", &file, now);
        let codex = self.codex.poll(now);
        update_provider_health(&mut self.health, "codex", &codex, now);
        let hermes = self.hermes.poll(now);
        update_provider_health(&mut self.health, "hermes", &hermes, now);
        while let Ok(output) = self.dsh_rx.try_recv() {
            self.dsh_latest = Some(output);
        }
        if let Some(dsh) = &self.dsh_latest {
            update_provider_health(&mut self.health, "deepseek_harness", dsh, now);
        }

        let mut authoritative_sources = HashSet::from([TaskSource::ClaudeCode, TaskSource::Gemini]);
        if file.authoritative {
            authoritative_sources.insert(TaskSource::LocalFile);
        }
        if codex.authoritative {
            authoritative_sources.insert(TaskSource::Codex);
        }
        if hermes.authoritative {
            authoritative_sources.insert(TaskSource::Hermes);
        }
        if self
            .dsh_latest
            .as_ref()
            .is_some_and(|output| output.authoritative)
        {
            authoritative_sources.insert(TaskSource::DeepSeekHarness);
        }

        let mut tasks = file.tasks;
        tasks.extend(codex.tasks);
        tasks.extend(hermes.tasks);
        if let Some(dsh) = &self.dsh_latest {
            tasks.extend(dsh.tasks.clone());
        }
        let scanned_agents = self.scanner.scan();
        let scanned: Vec<TaskSnapshot> = scanned_agents.iter().cloned().map(Into::into).collect();
        crate::heuristics::cross_validate_tasks(&mut tasks, &scanned);
        tasks.extend(scanned);
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
            std::thread::sleep(remote_interval);
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

    let dir = local_providers.primary_dir();

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

        if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
            log::error!("Failed to watch {:?}: {e}", dir);
            return;
        }

        log::info!(
            "Watching {:?} + scanning processes + polling remote hosts",
            dir
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

            // Periodic local process scan.
            if last_scan.elapsed() >= scan_interval {
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
