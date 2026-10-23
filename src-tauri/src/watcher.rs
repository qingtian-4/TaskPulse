//! Watcher — the orchestration loop.
//!
//! Pipeline: Provider → Scope / Normalize → Store → Attention → Transition → UI.
//!
//! * Native, WSL, scanner and Generic Event sources have independent workers.
//! * `LocalFileProvider` reads compatible hook/wrapper files in this loop.
//! * `RemoteTmuxProvider` polls configured SSH hosts on a separate thread.
//! * Generic Event ordered facts retain their durable outboxes until the
//!   attention and transition batch is flushed; UI snapshots can coalesce.
//!
//! The `agents-updated` event contract, tray icon updates, and the popup
//! commands are preserved from AgentTray.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::Manager;

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

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
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
            integration_epoch: None,
            integration_sequence: 0,
            agent_id: None,
            agent_name: None,
            evidence_channel: None,
            outcome: None,
            pending_confirmed_at: None,
            id: a.id,
            name: a.name,
            source,
            status,
            source_instance: None,
            model: None,
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
            host: None,
            project: None,
            message: (!a.message.is_empty()).then_some(a.message),
            activity: None,
            phase: None,
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
    if std::env::var_os("TASKPULSE_HOME").is_some() {
        return Some(config::config_dir().join("legacy-status"));
    }
    dirs_next::home_dir().map(|h| h.join(".agent-monitor"))
}

/// Cached latest merged task list for on-demand popup refresh.
static LATEST_TASKS: Mutex<Vec<TaskSnapshot>> = Mutex::new(Vec::new());
static SELECTED_TASK: Mutex<Option<TaskSnapshot>> = Mutex::new(None);
static LATEST_HOSTS: Mutex<Vec<HostHealth>> = Mutex::new(Vec::new());
static LATEST_PROVIDERS: Mutex<Vec<ProviderHealth>> = Mutex::new(Vec::new());
static ATTENTION_TRACKER: OnceLock<Mutex<AttentionTracker>> = OnceLock::new();
static MANUAL_REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);

fn reconcile_attention(tasks: Vec<TaskSnapshot>) -> Vec<TaskSnapshot> {
    reconcile_attention_with_persistence(tasks, true)
}

// Kept separate so mixed-source replay tests exercise the production dispatch
// boundary, not a copy of the watch loop's notification branch.
pub(crate) fn ordered_transition_events(
    engine: &mut TransitionEngine,
    tracked: &[TaskSnapshot],
    hosts: &[HostHealth],
    observation: &crate::providers::generic_event::OrderedObservation,
    now: SystemTime,
) -> Vec<crate::transition::TaskEvent> {
    engine.apply_ordered(tracked, hosts, observation, now)
}
fn reconcile_attention_with_persistence(
    tasks: Vec<TaskSnapshot>,
    persist: bool,
) -> Vec<TaskSnapshot> {
    let mut tasks = tasks;
    let settings = crate::monitoring::get_monitoring_settings();
    for task in &mut tasks {
        if task.source_instance.is_none() {
            task.source_instance =
                crate::monitoring::source_for(&settings, task).map(|s| s.id.clone());
        }
    }
    crate::providers::codex::clear_legacy_quota(&mut tasks);
    let Some(tracker) = ATTENTION_TRACKER.get() else {
        return tasks;
    };
    let mut tracker = tracker.lock().unwrap_or_else(|error| {
        log::warn!("ATTENTION_TRACKER mutex poisoned, recovering");
        error.into_inner()
    });
    tracker.set_recent_days(settings.recent_days);
    tracker.normalize_source_instances(|task| {
        crate::monitoring::source_for(&settings, task).map(|s| s.id.clone())
    });
    let mut display = if persist {
        tracker.reconcile(tasks)
    } else {
        tracker.reconcile_unflushed(tasks)
    };
    display.retain(crate::monitoring::allows);
    display
}

#[tauri::command]
pub fn get_archived_attention() -> Vec<TaskSnapshot> {
    ATTENTION_TRACKER
        .get()
        .map(|tracker| tracker.lock().unwrap_or_else(|e| e.into_inner()).archived())
        .unwrap_or_default()
        .into_iter()
        .filter(crate::monitoring::allows)
        .collect()
}

pub fn known_sessions() -> Vec<TaskSnapshot> {
    ATTENTION_TRACKER
        .get()
        .map(|tracker| tracker.lock().unwrap_or_else(|e| e.into_inner()).known())
        .unwrap_or_default()
}

pub fn attention_reason(id: &str) -> String {
    ATTENTION_TRACKER
        .get()
        .map(|tracker| tracker.lock().unwrap_or_else(|e| e.into_inner()).reason(id))
        .unwrap_or_else(|| "首次采集中".into())
}

#[tauri::command]
pub fn get_selected_task() -> Option<TaskSnapshot> {
    SELECTED_TASK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn select_task(app: &AppHandle, task: TaskSnapshot) {
    *SELECTED_TASK.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.clone());
    crate::tray::show_popup(app);
    let _ = app.emit("task-selected", task);
}

#[tauri::command]
pub fn open_task(app: AppHandle, id: String) -> Result<String, String> {
    let task = crate::monitoring::find_session(&id).ok_or("未发现该任务")?;
    if task.can_focus {
        if let Some(terminal) = &task.terminal {
            if crate::focusers::dispatch(&terminal.kind, &terminal.focus_id, &terminal.outer_id)
                .is_ok()
            {
                // A generic terminal/window focus is not proof that this session was read.
                return Ok("window_focused".into());
            }
        }
    }
    select_task(&app, task);
    Ok("details".into())
}

/// Update one task's TaskPulse-owned attention state.
#[tauri::command]
pub fn update_task_attention(
    app: AppHandle,
    id: String,
    action: String,
    pinned: Option<bool>,
) -> Result<Vec<TaskSnapshot>, String> {
    // Resolve before locking the tracker: catalog fallback reads the same tracker.
    let discovered = crate::monitoring::find_session(&id);
    if matches!(action.as_str(), "follow" | "restore" | "pin") {
        if let Some(task) = &discovered {
            if !crate::monitoring::allows(task) {
                return Err("请先启用来源并将项目加入监控范围".into());
            }
        }
    }
    let tracker = ATTENTION_TRACKER
        .get()
        .ok_or_else(|| "attention tracker is still starting".to_string())?;
    let tasks = {
        let mut tracker = tracker.lock().unwrap_or_else(|error| error.into_inner());
        match action.as_str() {
            "follow" => tracker.follow(discovered.ok_or("未发现该会话")?)?,
            "ignore" => {
                if !tracker.known().iter().any(|t| t.id == id) {
                    tracker.follow(discovered.ok_or("未发现该会话")?)?;
                }
                tracker.set_ignored(&id, true)?
            }
            "unignore" => tracker.set_ignored(&id, false)?,
            "viewed" => tracker.mark_viewed(&id)?,
            "hide" => tracker.hide(&id)?,
            "restore" => tracker.restore(&id)?,
            "pin" => tracker.set_pinned(&id, pinned.unwrap_or(true))?,
            _ => return Err(format!("unsupported attention action: {action}")),
        }
    };
    let tasks: Vec<_> = tasks
        .into_iter()
        .filter(crate::monitoring::allows)
        .collect();
    let hosts = get_hosts();
    let providers = get_providers();
    if matches!(action.as_str(), "viewed" | "hide" | "ignore") {
        crate::task_notice::remove_task(&app, &id);
    }
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
    let _ = app.emit(
        "pending-prompts-updated",
        crate::task_notice::get_pending_task_prompts(),
    );
    let _ = app.emit("agents-updated", tasks);
    static ARCHIVED: Mutex<Vec<TaskSnapshot>> = Mutex::new(Vec::new());
    let archived = get_archived_attention();
    let mut previous = ARCHIVED.lock().unwrap_or_else(|e| e.into_inner());
    if *previous != archived {
        let _ = app.emit("archived-attention-updated", &archived);
        *previous = archived;
    }
    let _ = app.emit("hosts-updated", hosts);
    let _ = app.emit("providers-updated", providers);
    let _ = app.emit(
        "codex-quota-updated",
        crate::providers::codex::get_codex_quota(),
    );
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
pub fn request_refresh() -> u64 {
    let requested_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    crate::providers::codex::request_quota_refresh();
    MANUAL_REFRESH_REQUESTED.store(true, Ordering::Release);
    requested_at
}

/// Tauri command: install or uninstall AgentTray/TaskPulse hooks for a CLI
/// tool. `cli` = "claude", "codex", "gemini", or "all"; `uninstall` removes.
#[tauri::command]
pub fn install_hooks(app: AppHandle, cli: String, uninstall: bool) -> Result<String, String> {
    let base_dir = app
        .path()
        .resource_dir()
        .ok()
        .filter(|d| d.join("scripts/hooks").exists())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .and_then(|dir| {
                    dir.ancestors()
                        .find(|d| d.join("scripts").exists())
                        .map(|d| d.to_path_buf())
                        .or(Some(dir))
                })
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
        // A project window can contain many native sessions. Its PID is not
        // session identity and must not deduplicate them by evidence strength.
        if t.session_id.is_some() {
            continue;
        }
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

fn dedup_claude_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    if tasks.iter().any(|t| {
        t.source == TaskSource::ClaudeCode
            && (t.id.starts_with("claude:") || t.id.contains(":claude:"))
    }) {
        tasks.retain(|t| {
            !((t.source_instance.as_deref() == Some("scanner") || t.id.starts_with("scan:"))
                && t.source == TaskSource::ClaudeCode)
        });
    }
}

struct LocalBatch {
    ordered: Vec<crate::providers::generic_event::OrderedObservation>,
    tasks: Vec<TaskSnapshot>,
    authoritative_sources: HashSet<TaskSource>,
    authoritative_instances: HashSet<String>,
}

struct LocalProviders {
    ordered: Vec<crate::providers::generic_event::OrderedObservation>,
    file: LocalFileProvider,
    workers: Vec<(String, Vec<TaskSource>, crate::providers::worker::Worker)>,
    latest: std::collections::HashMap<String, ProviderOutput>,
    observed_at: std::collections::HashMap<String, Instant>,
    started_at: Instant,
    health: Vec<ProviderHealth>,
    configured: Vec<crate::monitoring::SourceConfig>,
}

/// Scope is applied at consumption time. Do not throw away native incremental
/// parsers on a project-filter/label change (notably WSL continuity). Generic
/// reducers capture integration policy and their fallback display name.
fn same_collector(
    old: &crate::monitoring::SourceConfig,
    next: &crate::monitoring::SourceConfig,
) -> bool {
    old.id == next.id
        && old.kind == next.kind
        && old.location == next.location
        && old.integration == next.integration
        && (old.kind != crate::monitoring::SourceKind::GenericEvent || old.name == next.name)
}

impl LocalProviders {
    fn new(_monitoring: MonitoringConfig) -> Self {
        let mut providers = Self {
            ordered: vec![],
            file: LocalFileProvider::new(),
            workers: vec![],
            latest: Default::default(),
            observed_at: Default::default(),
            started_at: Instant::now(),
            health: vec![],
            configured: vec![],
        };
        providers.configure(&crate::monitoring::get_monitoring_settings());
        providers
    }

    fn configure(&mut self, settings: &crate::monitoring::DashboardSettings) {
        use crate::monitoring::SourceKind;
        use crate::providers::worker::Worker;
        let desired: Vec<_> = settings
            .sources
            .iter()
            .filter(|s| {
                settings.onboarding_complete
                    && s.enabled
                    && !matches!(
                        s.kind,
                        SourceKind::RemoteCodex | SourceKind::Tmux | SourceKind::Hooks
                    )
            })
            .cloned()
            .collect();
        let live = self.retain_collectors(&desired);
        for source in &desired {
            if live.contains(&source.id) {
                continue;
            }
            let mut provider: Box<dyn Provider> = match source.kind {
                SourceKind::GenericEvent => Box::new(
                    crate::providers::generic_event::GenericEventProvider::new(source.clone()),
                ),
                SourceKind::ClaudeCode => {
                    Box::new(crate::providers::claude_code::ClaudeCodeProvider::new(
                        if source.location.is_empty() {
                            crate::providers::claude_code::default_root()
                        } else {
                            PathBuf::from(&source.location)
                        },
                    ))
                }
                SourceKind::Codex => Box::new(CodexProvider::new()),
                SourceKind::CodexWsl => {
                    Box::new(crate::providers::codex_wsl::WslCodexProvider::new(
                        source.location.clone(),
                        source.id.clone(),
                    ))
                }
                SourceKind::Hermes => Box::new(if source.location.is_empty() {
                    HermesProvider::new()
                } else {
                    HermesProvider::with_db_path(PathBuf::from(&source.location))
                }),
                SourceKind::DeepseekDesktop => Box::new(if source.location.is_empty() {
                    DeepSeekHarnessProvider::desktop()
                } else {
                    DeepSeekHarnessProvider::with_native_root(PathBuf::from(&source.location))
                }),
                SourceKind::DeepseekWsl => Box::new(DeepSeekHarnessProvider::with_distro(
                    source.location.clone(),
                )),
                SourceKind::Scanner => {
                    let mut scanner = Scanner::new();
                    let id = source.id.clone();
                    self.workers.push((
                        id.clone(),
                        vec![TaskSource::ClaudeCode, TaskSource::Gemini],
                        Worker::spawn(Duration::from_secs(2), move |_| {
                            ProviderOutput::tasks(
                                scanner.scan().into_iter().map(Into::into).collect(),
                            )
                        }),
                    ));
                    self.health.push(ProviderHealth {
                        provider: id,
                        status: ProviderStatus::Degraded,
                        last_success: None,
                        last_attempt: None,
                        message: Some("首次采集中".into()),
                    });
                    continue;
                }
                _ => continue,
            };
            let source = source.clone();
            let id = source.id.clone();
            let interval = if matches!(source.kind, SourceKind::DeepseekWsl | SourceKind::CodexWsl)
            {
                2
            } else {
                1
            };
            self.workers.push((
                id.clone(),
                vec![],
                Worker::spawn(Duration::from_secs(interval), move |_| {
                    let mut output = provider.poll(SystemTime::now());
                    for task in output.tasks.iter_mut().chain(output.catalog.iter_mut()) {
                        task.source_instance = Some(source.id.clone());
                        if !matches!(source.kind, SourceKind::CodexWsl | SourceKind::GenericEvent)
                            && !matches!(
                                source.id.as_str(),
                                "codex"
                                    | "claude_code"
                                    | "hermes"
                                    | "deepseek_harness"
                                    | "deepseek_harness_desktop"
                            )
                        {
                            task.id = format!("{}:{}", source.id, task.id);
                        }
                    }
                    output
                }),
            ));
            self.health.push(ProviderHealth {
                provider: id,
                status: ProviderStatus::Degraded,
                last_success: None,
                last_attempt: None,
                message: Some("首次采集中".into()),
            });
        }
        self.configured = settings
            .sources
            .iter()
            .filter(|s| s.enabled && !matches!(s.kind, SourceKind::RemoteCodex | SourceKind::Tmux))
            .cloned()
            .collect();
    }

    fn retain_collectors(
        &mut self,
        desired: &[crate::monitoring::SourceConfig],
    ) -> HashSet<String> {
        self.workers.retain(|(id, _, _)| {
            desired.iter().any(|next| {
                &next.id == id && self.configured.iter().any(|old| same_collector(old, next))
            })
        });
        // Each worker owns a distinct receiver: dropping it revokes this
        // generation's authority, including a poll that finishes later. Worker
        // closures must never mutate the shared discovered-session catalog.
        let live: HashSet<_> = self.workers.iter().map(|(id, _, _)| id.clone()).collect();
        self.latest.retain(|id, _| live.contains(id));
        self.observed_at.retain(|id, _| live.contains(id));
        self.health.retain(|h| live.contains(&h.provider));
        // These facts are still durable in their unacknowledged outboxes.
        self.ordered
            .retain(|fact| live.contains(&fact.receipt.source));
        live
    }

    fn watch_paths(&self) -> Vec<(PathBuf, RecursiveMode)> {
        use crate::monitoring::SourceKind;
        let mut paths = Vec::new();
        if self.configured.iter().any(|s| s.kind == SourceKind::Hooks) {
            paths.extend(
                self.file
                    .status_dirs()
                    .into_iter()
                    .map(|p| (p, RecursiveMode::NonRecursive)),
            );
        }
        for source in &self.configured {
            if source.kind == SourceKind::ClaudeCode {
                let root = if source.location.is_empty() {
                    crate::providers::claude_code::default_root()
                } else {
                    PathBuf::from(&source.location)
                };
                paths.push((root.join("projects"), RecursiveMode::Recursive));
                paths.push((root.join("sessions"), RecursiveMode::NonRecursive));
                if let Some(home) = dirs_next::home_dir() {
                    paths.push((
                        home.join(".taskpulse/claude-events"),
                        RecursiveMode::NonRecursive,
                    ));
                }
            }
            if source.kind == SourceKind::Codex {
                if let Some(home) = dirs_next::home_dir() {
                    paths.push((home.join(".codex/sessions"), RecursiveMode::Recursive));
                    paths.push((home.join(".codex"), RecursiveMode::NonRecursive));
                }
            }
            if source.kind == SourceKind::Hermes {
                let db = if source.location.is_empty() {
                    crate::providers::hermes::discover_db()
                } else {
                    Some(PathBuf::from(&source.location))
                };
                if let Some(parent) = db.as_ref().and_then(|p| p.parent()) {
                    paths.push((parent.to_path_buf(), RecursiveMode::NonRecursive));
                }
            }
            if source.kind == SourceKind::DeepseekDesktop {
                let root = if source.location.is_empty() {
                    DeepSeekHarnessProvider::desktop_root()
                } else {
                    PathBuf::from(&source.location)
                };
                paths.push((root.join("sessions"), RecursiveMode::Recursive));
                paths.push((root.join("storages"), RecursiveMode::Recursive));
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
            if self.configured.iter().any(|s| {
                &s.id == name
                    && matches!(
                        s.kind,
                        crate::monitoring::SourceKind::Codex
                            | crate::monitoring::SourceKind::ClaudeCode
                            | crate::monitoring::SourceKind::Hermes
                            | crate::monitoring::SourceKind::DeepseekDesktop
                    )
            }) {
                worker.request_refresh();
            }
        }
    }

    fn collect_ready(&mut self) -> bool {
        let mut changed = false;
        for (name, _, worker) in &self.workers {
            while let Ok(mut observation) = worker.observations.try_recv() {
                crate::monitoring::observe(&observation.output.catalog);
                self.ordered.append(&mut observation.output.ordered);
                log::debug!("provider {name}: {} ms", observation.duration_ms);
                if self
                    .configured
                    .iter()
                    .any(|s| s.id == *name && s.kind == crate::monitoring::SourceKind::CodexWsl)
                {
                    crate::providers::resources::continuity(
                        "worker_delivery",
                        name,
                        &observation.output.tasks,
                        serde_json::json!({
                            "duration_ms":observation.duration_ms,"delivery_delay_ms":SystemTime::now().duration_since(observation.completed_at).unwrap_or_default().as_millis(),
                            "authoritative":observation.output.authoritative
                        }),
                    );
                }
                update_provider_health(
                    &mut self.health,
                    name,
                    &observation.output,
                    observation.completed_at,
                );
                self.latest.insert(name.clone(), observation.output);
                self.observed_at.insert(name.clone(), Instant::now());
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
        self.configure(&crate::monitoring::get_monitoring_settings());
        self.collect_ready();
        let file = if crate::monitoring::get_monitoring_settings()
            .sources
            .iter()
            .any(|s| s.enabled && s.kind == crate::monitoring::SourceKind::Hooks)
        {
            self.file.poll(now)
        } else {
            ProviderOutput::default()
        };
        if crate::monitoring::get_monitoring_settings()
            .sources
            .iter()
            .any(|s| s.enabled && s.kind == crate::monitoring::SourceKind::Hooks)
        {
            update_provider_health(&mut self.health, "local_file", &file, now);
        }
        let mut authoritative_sources = HashSet::new();
        let mut authoritative_instances = HashSet::new();
        if file.authoritative {
            authoritative_sources.insert(TaskSource::LocalFile);
            authoritative_instances.insert("local_file".into());
        }
        let mut tasks = file.tasks;
        for (name, sources, _) in &self.workers {
            if let Some(output) = self.latest.get(name) {
                if output.authoritative {
                    authoritative_sources.extend(sources.iter().copied());
                    authoritative_instances.insert(name.clone());
                }
                if name != "scanner" {
                    tasks.extend(output.tasks.clone());
                }
            }
        }
        let has_claude_adapter = crate::monitoring::get_monitoring_settings()
            .sources
            .iter()
            .any(|s| s.kind == crate::monitoring::SourceKind::ClaudeCode);
        let scanned: Vec<_> = self
            .latest
            .get("scanner")
            .map(|out| out.tasks.as_slice())
            .unwrap_or(&[])
            .iter()
            // An explicit native-source switch must not be bypassed by the
            // generic process scanner (even before the first native result).
            .filter(|t| !(has_claude_adapter && t.source == TaskSource::ClaudeCode))
            .cloned()
            .collect();
        crate::heuristics::cross_validate_tasks(&mut tasks, &scanned);
        tasks.extend_from_slice(&scanned);
        dedup_claude_over_scan(&mut tasks);
        dedup_by_terminal(&mut tasks);
        dedup_codex_over_scan(&mut tasks);
        dedup_hermes_over_scan(&mut tasks);
        dedup_dsh_over_scan(&mut tasks);
        crate::monitoring::observe(&tasks);
        let wsl_before: Vec<_> = self
            .configured
            .iter()
            .filter(|s| s.kind == crate::monitoring::SourceKind::CodexWsl)
            .map(|s| {
                (
                    s.id.clone(),
                    tasks
                        .iter()
                        .filter(|t| t.source_instance.as_ref() == Some(&s.id))
                        .count(),
                )
            })
            .collect();
        tasks.retain(crate::monitoring::allows);
        for (source, before) in wsl_before {
            let scoped: Vec<_> = tasks
                .iter()
                .filter(|t| t.source_instance.as_ref() == Some(&source))
                .cloned()
                .collect();
            crate::providers::resources::continuity(
                "watcher_scope",
                &source,
                &scoped,
                serde_json::json!({"before_scope_count":before,"after_scope_count":scoped.len()}),
            );
        }
        LocalBatch {
            ordered: std::mem::take(&mut self.ordered),
            tasks,
            authoritative_sources,
            authoritative_instances,
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
            last_attempt: Some(now),
            message: None,
        }
    } else {
        ProviderHealth {
            provider: provider.to_string(),
            status: ProviderStatus::Degraded,
            last_success: previous_success,
            last_attempt: Some(now),
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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

fn append_state_health(
    health: &mut Vec<ProviderHealth>,
    attention_error: Option<&str>,
    notification_error: Option<&str>,
    now: SystemTime,
) {
    for (provider, error) in [
        ("state:attention", attention_error),
        ("state:notifications", notification_error),
    ] {
        if let Some(error) = error {
            health.push(ProviderHealth {
                provider: provider.into(),
                status: ProviderStatus::Degraded,
                last_success: None,
                last_attempt: Some(now),
                message: Some(error.into()),
            });
        }
    }
}

/// Ignore read/access events and our own atomic state writes. Otherwise each
/// attention save wakes every collector, which saves attention again.
fn meaningful_file_event(event: &notify::Event) -> bool {
    if !matches!(
        event.kind,
        notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|path| {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.')
            || name.ends_with("-shm")
            || matches!(
                name,
                "attention.json"
                    | "recent.json"
                    | "notifications.json"
                    | "notification-log.json"
                    | "config.toml"
                    | "config.toml.pre-dashboard.bak"
            )
        {
            return false;
        }
        name.ends_with(".status")
            || name.ends_with(".json")
            || name.ends_with(".jsonl")
            || name.ends_with(".zstd")
            || name.ends_with(".db")
            || name.ends_with(".db-wal")
            || name.ends_with(".sqlite")
            || name.ends_with(".sqlite-wal")
    })
}

pub fn watch(app: AppHandle) {
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Some(status) = status_dir() {
        let _ = std::fs::create_dir_all(status);
    }
    let settings = crate::monitoring::initialize();
    let _ = ATTENTION_TRACKER.set(Mutex::new(AttentionTracker::open(
        dir.join("attention.json"),
    )));
    if let Some(tracker) = ATTENTION_TRACKER.get() {
        tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset_baselines(
                &settings
                    .sources
                    .iter()
                    .filter(|s| s.enabled)
                    .map(|s| s.id.clone())
                    .collect(),
            );
    }
    let (mut history, restored) = HistoryStore::open(dir.join("recent.json"));
    let mut store = TaskStore::default();
    store.restore_recent(restored, SystemTime::now());
    let mut engine = TransitionEngine::open(dir.join("notifications.json"));
    let mut providers = LocalProviders::new(MonitoringConfig::default());
    let (remote_tx, remote_rx) = std::sync::mpsc::sync_channel::<(u64, ProviderOutput)>(4);
    let (refresh_tx, refresh_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let remote_revision = crate::monitoring::revision();
    std::thread::spawn(move || {
        let mut revision = remote_revision;
        let mut remote = RemoteTmuxProvider::new(
            crate::monitoring::remote_config(&settings),
            Arc::new(SystemSshExecutor::new()),
        );
        loop {
            let current = crate::monitoring::revision();
            if current != revision {
                remote.reconfigure(crate::monitoring::remote_config(
                    &crate::monitoring::get_monitoring_settings(),
                ));
                revision = current;
            }
            let started = Instant::now();
            let mut output = remote.poll(SystemTime::now());
            let settings = crate::monitoring::get_monitoring_settings();
            for task in output.tasks.iter_mut().chain(output.catalog.iter_mut()) {
                if let Some(source) = crate::monitoring::source_for(&settings, task) {
                    task.source_instance = Some(source.id.clone());
                }
            }
            if remote_tx.send((revision, output)).is_err() {
                break;
            }
            match refresh_rx
                .recv_timeout(remote_interval_ms(&remote).saturating_sub(started.elapsed()))
            {
                Ok(()) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    let mut hosts = Vec::new();
    let mut remote_attempts = std::collections::HashMap::new();
    let mut remote_source_health = std::collections::HashMap::new();
    let mut remote_source_success = std::collections::HashMap::new();
    let mut revision = crate::monitoring::revision();
    let mut active_settings = crate::monitoring::get_monitoring_settings();
    let mut watcher: Option<RecommendedWatcher> = None;
    let (watch_tx, watch_rx) = std::sync::mpsc::channel();
    let mut watch_revision = None;
    let mut last_scan = Instant::now() - Duration::from_secs(2);
    loop {
        let current = crate::monitoring::revision();
        if revision != current {
            let next = crate::monitoring::get_monitoring_settings();
            let affected: HashSet<_> = active_settings
                .sources
                .iter()
                .chain(next.sources.iter())
                .filter(|source| {
                    active_settings.sources.iter().find(|s| s.id == source.id)
                        != next.sources.iter().find(|s| s.id == source.id)
                })
                .map(|source| source.id.clone())
                .collect();
            if let Some(tracker) = ATTENTION_TRACKER.get() {
                tracker
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .reset_baselines(&affected);
            }
            let removed: HashSet<_> = known_sessions()
                .iter()
                .filter(|task| !crate::monitoring::allows(task))
                .map(|task| task.id.clone())
                .collect();
            engine.forget(&removed);
            store.retain(crate::monitoring::allows);
            providers.configure(&next);
            hosts.retain(|host: &HostHealth| {
                next.sources
                    .iter()
                    .any(|s| s.enabled && s.location == host.host)
            });
            active_settings = next;
            revision = current;
            let _ = refresh_tx.try_send(());
            last_scan = Instant::now() - Duration::from_secs(2);
        }
        if watch_revision != Some(revision) {
            watcher.take();
            match RecommendedWatcher::new(watch_tx.clone(), Config::default()) {
                Ok(mut next) => {
                    for (path, mode) in providers.watch_paths() {
                        if path.exists() {
                            let _ = next.watch(&path, mode);
                        }
                    }
                    watcher = Some(next);
                }
                Err(error) => {
                    log::warn!("file watcher unavailable; polling remains active: {error}")
                }
            }
            watch_revision = Some(revision);
        }
        let mut force = MANUAL_REFRESH_REQUESTED.swap(false, Ordering::AcqRel);
        while let Ok(event) = watch_rx.try_recv() {
            if event.as_ref().is_ok_and(meaningful_file_event) {
                providers.request_native_refresh();
                force = true;
            }
        }
        if force {
            providers.request_refresh();
            let _ = refresh_tx.try_send(());
        }
        let mut changed = false;
        while let Ok((generation, output)) = remote_rx.try_recv() {
            if generation != revision {
                continue;
            }
            hosts = output.hosts;
            for health in output.remote_health {
                for source in active_settings
                    .sources
                    .iter()
                    .filter(|source| source.enabled && source.location == health.host)
                {
                    let matches = matches!(
                        (source.kind, health.source),
                        (
                            crate::monitoring::SourceKind::RemoteCodex,
                            TaskSource::Codex
                        ) | (crate::monitoring::SourceKind::Tmux, TaskSource::RemoteTmux)
                    );
                    if matches && health.authoritative {
                        remote_source_success.insert(source.id.clone(), SystemTime::now());
                    }
                }
                remote_source_health.insert((health.host.clone(), health.source), health);
            }
            for host in &hosts {
                remote_attempts.insert(host.host.clone(), SystemTime::now());
            }
            crate::monitoring::observe(&output.catalog);
            crate::monitoring::observe(&output.tasks);
            let tasks: Vec<_> = output
                .tasks
                .into_iter()
                .filter(crate::monitoring::allows)
                .collect();
            let authoritative = active_settings
                .sources
                .iter()
                .filter(|source| source.enabled)
                .filter(|source| {
                    let kind = match source.kind {
                        crate::monitoring::SourceKind::RemoteCodex => TaskSource::Codex,
                        crate::monitoring::SourceKind::Tmux => TaskSource::RemoteTmux,
                        _ => return false,
                    };
                    remote_source_health
                        .get(&(source.location.clone(), kind))
                        .is_some_and(|health| health.authoritative)
                })
                .map(|source| source.id.clone())
                .collect();
            store.merge_instances(tasks, &authoritative, SystemTime::now());
            changed = true;
        }
        if providers.collect_ready() || last_scan.elapsed() >= Duration::from_secs(2) {
            let batch = providers.poll(SystemTime::now());
            let mut receipts = std::collections::BTreeMap::new();
            let mut ordered_events = vec![];
            let previous_engine = engine.clone();
            for observation in batch.ordered {
                let receipt_key = (
                    observation.receipt.source.clone(),
                    observation.receipt.session_key.clone(),
                    observation.receipt.epoch.clone(),
                );
                if crate::monitoring::allows(&observation.task) && !is_ignored(&observation.task.id)
                {
                    // Every ordered fact follows scope/store/attention/transition.
                    crate::monitoring::observe(std::slice::from_ref(&observation.task));
                    let merged = store.merge(vec![observation.task.clone()], SystemTime::now());
                    reconcile_attention_with_persistence(merged, false);
                    let mut tracked = known_sessions();
                    // Attention may already contain a newer durable observation
                    // after a crash. Transition still consumes this ordered fact,
                    // using its turn identity; its own ledger deduplicates it.
                    if let Some(current) = tracked.iter_mut().find(|t| t.id == observation.task.id)
                    {
                        let unread = current.unread;
                        let pinned = current.pinned;
                        *current = observation.task.clone();
                        current.unread = unread;
                        current.pinned = pinned;
                    }
                    tracked.retain(|task| crate::monitoring::allows(task) && !is_ignored(&task.id));
                    ordered_events.extend(ordered_transition_events(
                        &mut engine,
                        &tracked,
                        &hosts,
                        &observation,
                        SystemTime::now(),
                    ));
                }
                receipts.insert(receipt_key, observation.receipt);
            }
            if !receipts.is_empty() {
                let attention_durable = ATTENTION_TRACKER.get().is_some_and(|tracker| {
                    tracker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .flush()
                        .is_ok()
                });
                if attention_durable && engine.flush() {
                    let tracked = known_sessions();
                    // Do not show a human blocker that was explicitly resolved
                    // later in this same ordered batch. End facts remain events.
                    ordered_events.retain(|event| match event {
                        crate::transition::TaskEvent::WaitingInput { task, .. }
                            if task.requires_action() =>
                        {
                            tracked.iter().any(|latest| {
                                latest.id == task.id
                                    && latest.turn_key == task.turn_key
                                    && latest.requires_action()
                                    && task
                                        .input_request_id
                                        .as_ref()
                                        .is_some_and(|id| latest.input_request_ids.contains(id))
                            })
                        }
                        _ => true,
                    });
                    crate::notifier::handle_events(&app, &ordered_events, &tracked);
                    let mut acknowledged_sources = HashSet::new();
                    for receipt in receipts.values() {
                        if let Err(code) = crate::providers::generic_event::acknowledge(
                            &crate::integrations::home(),
                            receipt,
                        ) {
                            log::warn!("integration acknowledgment deferred: {code}");
                        } else {
                            acknowledged_sources.insert(receipt.source.as_str());
                        }
                    }
                    // Ack is the backpressure boundary: wake only successfully
                    // committed sources to drain the next bounded batch, rather
                    // than waiting a full polling interval for every 128 facts.
                    // An empty poll has no receipts and cannot self-spin.
                    for (id, _, worker) in &providers.workers {
                        if acknowledged_sources.contains(id.as_str()) {
                            worker.request_refresh();
                        }
                    }
                } else {
                    engine = previous_engine;
                    for receipt in receipts.values() {
                        if let Some(health) = providers
                            .health
                            .iter_mut()
                            .find(|p| p.provider == receipt.source)
                        {
                            health.status = ProviderStatus::Degraded;
                            health.message =
                                Some("状态持久化失败；待消费事件保留，未确认丢弃".into());
                        }
                    }
                    log::warn!("integration batch persistence failed; durable outboxes retained");
                }
            }
            store.retain(crate::monitoring::allows);
            store.merge_instances(
                batch.tasks,
                &batch.authoritative_instances,
                SystemTime::now(),
            );
            last_scan = Instant::now();
            changed = true;
        }
        if changed {
            let mut merged = store.merge(Vec::new(), SystemTime::now());
            crate::monitoring::refresh_known_metadata(&mut merged);
            let display = reconcile_attention(merged);
            let mut tracked = known_sessions();
            tracked.retain(|task| crate::monitoring::allows(task) && !is_ignored(&task.id));
            let excluded: HashSet<_> = known_sessions()
                .iter()
                .filter(|task| !crate::monitoring::allows(task) || is_ignored(&task.id))
                .map(|task| task.id.clone())
                .collect();
            engine.forget(&excluded);
            let events = engine.apply_snapshots(&tracked, &hosts, SystemTime::now());
            crate::notifier::handle_events(&app, &events, &tracked);
            if let Err(error) = history.save_if_changed(store.recent()) {
                log::warn!("recent history: {error}");
            }
            let mut health = providers.health.clone();
            let attention_error = ATTENTION_TRACKER.get().and_then(|tracker| {
                tracker
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .persistence_error()
            });
            append_state_health(
                &mut health,
                attention_error,
                engine.persistence_error(),
                SystemTime::now(),
            );
            for source in active_settings.sources.iter().filter(|s| {
                s.enabled
                    && matches!(
                        s.kind,
                        crate::monitoring::SourceKind::RemoteCodex
                            | crate::monitoring::SourceKind::Tmux
                    )
            }) {
                if let Some(host) = hosts.iter().find(|h| h.host == source.location) {
                    let kind = if source.kind == crate::monitoring::SourceKind::RemoteCodex {
                        TaskSource::Codex
                    } else {
                        TaskSource::RemoteTmux
                    };
                    let segment = remote_source_health.get(&(source.location.clone(), kind));
                    health.push(ProviderHealth {
                        provider: source.id.clone(),
                        status: if host.status == crate::providers::HostStatus::Connected
                            && segment.is_some_and(|health| health.authoritative)
                        {
                            ProviderStatus::Connected
                        } else {
                            ProviderStatus::Degraded
                        },
                        last_success: remote_source_success.get(&source.id).copied(),
                        last_attempt: remote_attempts.get(&host.host).copied(),
                        message: segment.and_then(|health| health.error.clone()),
                    });
                }
            }
            emit_agents(&app, &display, &hosts, &health);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn is_ignored(id: &str) -> bool {
    ATTENTION_TRACKER.get().is_some_and(|tracker| {
        tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .ignored(id)
    })
}

/// Read-only, bounded sampling of the same workers used by the tray. This
/// reports native task titles/status and cadence, without prompts or commands.
pub fn diagnose() {
    crate::monitoring::initialize();
    let mut providers = LocalProviders::new(MonitoringConfig::default());
    let started = Instant::now();
    let seconds = std::env::args()
        .find_map(|arg| {
            arg.strip_prefix("--diagnose-seconds=")
                .and_then(|value| value.parse::<u64>().ok())
        })
        .unwrap_or(10)
        .clamp(1, 60);
    let force_quota_refresh = std::env::args().any(|arg| arg == "--refresh-quota");
    let mut requested_quota_refresh = false;
    let mut quota_observations = Vec::new();
    let mut last_quota_at = None;
    let mut samples = std::collections::HashMap::new();
    let mut first_ready = std::collections::HashMap::new();
    let mut durations = std::collections::HashMap::new();
    let mut latest = LocalBatch {
        ordered: vec![],
        tasks: Vec::new(),
        authoritative_sources: HashSet::new(),
        authoritative_instances: HashSet::new(),
    };
    let mut store = TaskStore::default();
    let mut engine = TransitionEngine::new();
    let mut event_count = 0;
    while started.elapsed() < Duration::from_secs(seconds) {
        if force_quota_refresh
            && !requested_quota_refresh
            && started.elapsed() >= Duration::from_secs(5)
        {
            crate::providers::codex::request_quota_refresh();
            requested_quota_refresh = true;
        }
        let quota = crate::providers::codex::get_codex_quota();
        let observed_at = quota.observed_at;
        if observed_at.is_some() && observed_at != last_quota_at {
            last_quota_at = observed_at;
            quota_observations.push(quota);
        }
        for (name, _, worker) in &providers.workers {
            while let Ok(observation) = worker.observations.try_recv() {
                *samples.entry(name.clone()).or_insert(0u32) += 1;
                first_ready
                    .entry(name.clone())
                    .or_insert(started.elapsed().as_millis());
                durations.insert(name.clone(), observation.duration_ms);
                update_provider_health(
                    &mut providers.health,
                    name,
                    &observation.output,
                    observation.completed_at,
                );
                providers.latest.insert(name.clone(), observation.output);
                providers.observed_at.insert(name.clone(), Instant::now());
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
                "phase": task.phase, "turn": task.turn_key,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({
            "sample_window_ms": started.elapsed().as_millis(), "samples": samples,
            "first_ready_ms": first_ready, "last_poll_ms": durations,
            "providers": providers.health, "tasks": tasks, "transition_count": event_count,
            "codex_account_quota": crate::providers::codex::get_codex_quota(),
            "quota_observations": quota_observations, "requested_quota_refresh": requested_quota_refresh,
            "resources": crate::providers::resources::snapshot(),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r02_persistence_failures_are_visible_as_degraded_health() {
        let mut health = vec![];
        append_state_health(&mut health, None, None, SystemTime::now());
        assert!(health.is_empty());
        let message = crate::state_file::StateError::Invalid.message();
        append_state_health(&mut health, Some(message), Some(message), SystemTime::now());
        assert_eq!(health.len(), 2);
        assert!(health
            .iter()
            .all(|item| item.status == ProviderStatus::Degraded && item.last_success.is_none()));
        assert_eq!(health[0].provider, "state:attention");
        assert_eq!(health[1].provider, "state:notifications");
    }

    fn r03_source() -> crate::monitoring::SourceConfig {
        serde_json::from_value(serde_json::json!({
            "id":"r03-source","name":"Source","kind":"generic_event","enabled":true,
            "location":"","integration":{"agent_id":"fixture-agent","adapter_id":"fixture-adapter",
            "transport":"local","accept_since_ms":0}
        }))
        .unwrap()
    }

    #[test]
    fn r03_collector_policy_changes_invalidate_but_live_scope_changes_keep_parsers() {
        let old = r03_source();
        for key in [
            "agent_id",
            "adapter_id",
            "transport",
            "host_version",
            "accept_since_ms",
        ] {
            let mut next = serde_json::to_value(&old).unwrap();
            next["integration"][key] = if key == "accept_since_ms" {
                10.into()
            } else {
                "changed".into()
            };
            assert!(
                !same_collector(&old, &serde_json::from_value(next).unwrap()),
                "{key}"
            );
        }
        let mut native = old.clone();
        native.kind = crate::monitoring::SourceKind::CodexWsl;
        native.integration = None;
        native.location = "RunningDistro".into();
        let mut scoped = native.clone();
        scoped.projects = vec!["/new-project".into()];
        scoped.excluded_projects = vec!["/ignored".into()];
        scoped.name = "Display rename".into();
        assert!(same_collector(&native, &scoped));
        scoped.location = "OtherDistro".into();
        assert!(!same_collector(&native, &scoped));
    }

    #[test]
    fn r03_late_obsolete_worker_cannot_publish_catalog_snapshot_or_ordered_facts() {
        use crate::providers::worker::Worker;
        use std::sync::mpsc;
        struct Exit(mpsc::Sender<()>);
        impl Drop for Exit {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let old = r03_source();
        let mut next = old.clone();
        next.integration.as_mut().unwrap().accept_since_ms = 10;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let exit = Exit(exit_tx);
        let old_task = TaskSnapshot::new(
            "r03-obsolete",
            "Obsolete",
            TaskSource::GenericEvent,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            SystemTime::now(),
        );
        let fact = crate::providers::generic_event::OrderedObservation {
            task: old_task.clone(),
            notify: true,
            receipt: crate::providers::generic_event::Receipt {
                source: old.id.clone(),
                session_key: "session".into(),
                epoch: "epoch".into(),
                sequence: 1,
            },
        };
        let late_task = old_task.clone();
        let late_fact = fact.clone();
        let worker = Worker::spawn(Duration::from_secs(60), move |_| {
            let _keep_until_thread_exits = &exit;
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            ProviderOutput {
                tasks: vec![late_task.clone()],
                catalog: vec![late_task.clone()],
                ordered: vec![late_fact.clone()],
                ..Default::default()
            }
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut providers = LocalProviders {
            ordered: vec![fact],
            file: LocalFileProvider::new(),
            workers: vec![(old.id.clone(), vec![], worker)],
            latest: [(old.id.clone(), ProviderOutput::tasks(vec![old_task]))].into(),
            observed_at: [(old.id.clone(), Instant::now())].into(),
            started_at: Instant::now(),
            health: vec![],
            configured: vec![old],
        };
        assert!(providers
            .retain_collectors(std::slice::from_ref(&next))
            .is_empty());
        providers.configured = vec![next.clone()];
        assert!(
            providers.latest.is_empty()
                && providers.ordered.is_empty()
                && providers.observed_at.is_empty()
        );
        release_tx.send(()).unwrap();
        exit_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!providers.collect_ready());
        assert!(crate::monitoring::find_session("r03-obsolete").is_none());
        let fresh = TaskSnapshot::new(
            "r03-current",
            "Current",
            TaskSource::GenericEvent,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            SystemTime::now(),
        );
        providers.workers.push((
            next.id.clone(),
            vec![],
            Worker::spawn(Duration::from_secs(60), move |_| ProviderOutput {
                catalog: vec![fresh.clone()],
                tasks: vec![fresh.clone()],
                ..Default::default()
            }),
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !providers.collect_ready() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(providers.latest[&next.id].tasks[0].id, "r03-current");
        assert!(crate::monitoring::find_session("r03-current").is_some());
        assert!(crate::monitoring::find_session("r03-obsolete").is_none());
    }
    use crate::domain::{DetectionConfidence, TaskStatus};

    #[test]
    fn watcher_ignores_its_own_state_files_and_reads_but_keeps_native_writes() {
        use notify::{
            event::{AccessKind, ModifyKind},
            Event, EventKind,
        };
        let write = |name: &str| {
            Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Any,
            )))
            .add_path(PathBuf::from(name))
        };
        for path in [
            "attention.json",
            "recent.json",
            "notifications.json",
            "notification-log.json",
            ".tmpAbcd",
            "state_5.sqlite-shm",
        ] {
            assert!(!meaningful_file_event(&write(path)));
        }
        for path in [
            "task.status",
            "state_5.sqlite-wal",
            "state.db-wal",
            "rollout.jsonl",
            "session.v4.jsonl.zstd",
            "session-1.json",
        ] {
            assert!(meaningful_file_event(&write(path)));
        }
        assert!(!meaningful_file_event(
            &Event::new(EventKind::Access(AccessKind::Any)).add_path(PathBuf::from("state.db"))
        ));
    }

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
    fn native_claude_sessions_survive_one_window_and_suppress_scanner_duplicates() {
        let mut tasks = vec![
            task_with_terminal(
                "claude:one",
                TaskStatus::Running,
                DetectionConfidence::Inferred,
                "123",
                None,
            ),
            task_with_terminal(
                "claude:two",
                TaskStatus::WaitingInput,
                DetectionConfidence::Hook,
                "123",
                None,
            ),
            task_with_terminal(
                "scan:claude",
                TaskStatus::Running,
                DetectionConfidence::Inferred,
                "123",
                Some("claude-code"),
            ),
        ];
        for (i, task) in tasks.iter_mut().enumerate() {
            task.source = TaskSource::ClaudeCode;
            if i < 2 {
                task.session_id = Some(format!("session-{i}"));
            }
        }
        dedup_claude_over_scan(&mut tasks);
        dedup_by_terminal(&mut tasks);
        assert_eq!(tasks.len(), 2);
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
        assert_eq!(health[0].last_attempt, Some(now));

        let later = now + Duration::from_secs(5);
        update_provider_health(
            &mut health,
            "hermes",
            &ProviderOutput::degraded(Vec::new(), "database locked"),
            later,
        );
        assert_eq!(health[0].status, ProviderStatus::Degraded);
        assert_eq!(health[0].last_success, Some(now));
        assert_eq!(health[0].last_attempt, Some(later));
        assert_eq!(health[0].message.as_deref(), Some("database locked"));

        let recovered = later + Duration::from_secs(5);
        update_provider_health(&mut health, "hermes", &ProviderOutput::default(), recovered);
        assert_eq!(health[0].status, ProviderStatus::Connected);
        assert_eq!(health[0].last_success, Some(recovered));
        assert!(health[0].message.is_none());
    }
}
