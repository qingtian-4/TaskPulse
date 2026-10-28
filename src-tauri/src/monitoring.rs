//! User-owned source registry. Configuration is separate from session identity.
use crate::domain::{TaskSnapshot, TaskSource};
use crate::providers::remote_tmux::config::{
    self, RemoteCodexScope, RemoteHostConfig, TaskPulseConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Doubao,
    Zcode,
    Codex,
    CodexWsl,
    ClaudeCode,
    Hermes,
    DeepseekDesktop,
    DeepseekWsl,
    RemoteCodex,
    Tmux,
    Hooks,
    Scanner,
    GenericEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<crate::integrations::IntegrationSource>,
    pub enabled: bool,
    #[serde(default)]
    pub location: String,
    #[serde(default = "all_projects")]
    pub projects: Vec<String>,
    #[serde(default)]
    pub excluded_projects: Vec<String>,
    #[serde(default)]
    pub tmux_sessions: Vec<String>,
    #[serde(default)]
    pub tmux_panes: Vec<String>,
    #[serde(default = "poll_default")]
    pub poll_interval_ms: u64,
}
fn all_projects() -> Vec<String> {
    vec!["*".into()]
}
fn poll_default() -> u64 {
    3000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSettings {
    pub version: u8,
    pub onboarding_complete: bool,
    /// Existing installs keep their explicit source choices until opted in.
    #[serde(default)]
    pub auto_discover_local: bool,
    #[serde(default)]
    pub auto_discovery_seen: Vec<String>,
    #[serde(default = "yes")]
    pub auto_check_updates: bool,
    pub sources: Vec<SourceConfig>,
    #[serde(default = "yes")]
    pub notifications: bool,
    #[serde(default = "notification_mode_default")]
    pub notification_mode: String,
    #[serde(default)]
    pub sound: bool,
    #[serde(default = "task_view")]
    pub default_view: String,
    /// Inactive attention is archived, never deleted. Zero shows all ages.
    #[serde(default = "recent_days_default")]
    pub recent_days: u16,
}
fn recent_days_default() -> u16 {
    7
}
fn notification_mode_default() -> String {
    "popup".into()
}
impl DashboardSettings {
    pub fn popup_notifications(&self) -> bool {
        self.notifications && matches!(self.notification_mode.as_str(), "popup" | "both")
    }
    pub fn desktop_notifications(&self) -> bool {
        self.notifications && matches!(self.notification_mode.as_str(), "system" | "both")
    }
}
fn yes() -> bool {
    true
}
fn task_view() -> String {
    "tasks".into()
}

#[derive(Default)]
struct Runtime {
    settings: Option<DashboardSettings>,
    revision: u64,
    catalog: HashMap<String, TaskSnapshot>,
    config_error: Option<String>,
}
static RUNTIME: OnceLock<Mutex<Runtime>> = OnceLock::new();
static SETTINGS_WRITER: Mutex<()> = Mutex::new(());
fn runtime() -> &'static Mutex<Runtime> {
    RUNTIME.get_or_init(|| Mutex::new(Runtime::default()))
}

pub fn from_legacy(cfg: &TaskPulseConfig, existing: bool) -> DashboardSettings {
    let mut sources = vec![
        source("codex", "Codex · 本机", SourceKind::Codex, ""),
        source(
            "codex-wsl:ubuntu",
            "Codex · WSL Ubuntu",
            SourceKind::CodexWsl,
            "Ubuntu",
        ),
        source(
            "claude_code",
            "Claude Code · 本机",
            SourceKind::ClaudeCode,
            "",
        ),
        source("hermes", "Hermes · 本机", SourceKind::Hermes, ""),
        source(
            "deepseek_harness_desktop",
            "DeepSeek · Windows",
            SourceKind::DeepseekDesktop,
            "",
        ),
        source(
            "deepseek_harness",
            "DeepSeek · WSL",
            SourceKind::DeepseekWsl,
            "Ubuntu",
        ),
        source("local_file", "通用 Hooks", SourceKind::Hooks, ""),
        source("scanner", "进程扫描", SourceKind::Scanner, ""),
        source("zcode", "ZCode · 本机", SourceKind::Zcode, ""),
        source("doubao", "豆包 · 本机", SourceKind::Doubao, ""),
    ];
    for item in &mut sources {
        item.projects = match item.kind {
            SourceKind::Codex => cfg.monitoring.local_codex_projects.clone(),
            SourceKind::Hermes => cfg.monitoring.hermes_projects.clone(),
            SourceKind::DeepseekDesktop | SourceKind::DeepseekWsl => {
                cfg.monitoring.deepseek_harness_projects.clone()
            }
            _ => all_projects(),
        };
        item.enabled = existing
            && !item.projects.is_empty()
            && !matches!(
                item.kind,
                SourceKind::ClaudeCode
                    | SourceKind::CodexWsl
                    | SourceKind::Doubao
                    | SourceKind::Zcode
            );
    }
    for host in &cfg.remote_hosts {
        let mut codex = source(
            &format!("remote-codex:{}", host.id),
            &format!("Codex · {}", host.ssh_host),
            SourceKind::RemoteCodex,
            &host.ssh_host,
        );
        codex.projects = cfg
            .monitoring
            .remote_codex
            .iter()
            .filter(|scope| config::wildcard_match(&scope.host, &host.ssh_host))
            .flat_map(|scope| scope.projects.clone())
            .collect();
        codex.enabled = existing && host.enabled && !codex.projects.is_empty();
        codex.poll_interval_ms = host.poll_interval_ms;
        sources.push(codex);
        let mut tmux = source(
            &format!("tmux:{}", host.id),
            &format!("tmux · {}", host.ssh_host),
            SourceKind::Tmux,
            &host.ssh_host,
        );
        tmux.enabled = existing && host.enabled;
        tmux.tmux_sessions = host.tmux_sessions.clone();
        tmux.tmux_panes = host.tmux_panes.clone();
        tmux.poll_interval_ms = host.poll_interval_ms;
        sources.push(tmux);
    }
    DashboardSettings {
        version: 2,
        onboarding_complete: existing,
        auto_discover_local: !existing,
        auto_discovery_seen: vec![],
        auto_check_updates: true,
        sources,
        notifications: true,
        notification_mode: notification_mode_default(),
        sound: false,
        default_view: task_view(),
        recent_days: recent_days_default(),
    }
}

fn source(id: &str, name: &str, kind: SourceKind, location: &str) -> SourceConfig {
    SourceConfig {
        id: id.into(),
        name: name.into(),
        kind,
        integration: None,
        enabled: false,
        location: location.into(),
        projects: all_projects(),
        excluded_projects: vec![],
        tmux_sessions: vec![],
        tmux_panes: vec![],
        poll_interval_ms: poll_default(),
    }
}

pub fn initialize() -> DashboardSettings {
    let mut runtime = runtime().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(settings) = &runtime.settings {
        return settings.clone();
    }
    let path = config::config_path();
    let existed = path.exists();
    let cfg = config::load_config();
    let parsed = (|| -> Result<DashboardSettings, String> {
        let body = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let value: toml::Value = toml::from_str(&body).map_err(|e| format!("配置无法解析：{e}"))?;
        let settings: DashboardSettings = if let Some(dashboard) = value.get("dashboard") {
            dashboard
                .clone()
                .try_into()
                .map_err(|e| format!("来源配置无效：{e}"))?
        } else {
            from_legacy(&cfg, existed)
        };
        validate(&settings)?;
        Ok(settings)
    })();
    let settings = match parsed {
        Ok(settings) => settings,
        Err(error) => {
            runtime.config_error = Some(format!(
                "{error}。已暂停所有来源，原文件未覆盖；请修复 {}",
                path.display()
            ));
            from_legacy(&TaskPulseConfig::default(), false)
        }
    };
    // A new config must persist its unconfirmed onboarding state. Otherwise
    // the next launch would mistake the generated file for a legacy opt-in.
    if !existed && runtime.config_error.is_none() {
        if let Err(error) = write_settings(&path, &settings) {
            runtime.config_error = Some(error);
        }
    }
    runtime.settings = Some(settings.clone());
    settings
}

#[tauri::command]
pub fn get_monitoring_error() -> Option<String> {
    runtime()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .config_error
        .clone()
}

#[tauri::command]
pub fn get_monitoring_settings() -> DashboardSettings {
    if let Some(settings) = runtime()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .settings
        .clone()
    {
        return settings;
    }
    initialize()
}

pub fn revision() -> u64 {
    runtime().lock().unwrap_or_else(|e| e.into_inner()).revision
}

fn validate(settings: &DashboardSettings) -> Result<(), String> {
    if settings.version != 2 {
        return Err("不支持的配置版本".into());
    }
    if !matches!(
        settings.notification_mode.as_str(),
        "popup" | "system" | "both"
    ) {
        return Err("无效的任务提醒方式".into());
    }
    if !matches!(settings.default_view.as_str(), "tasks" | "projects") {
        return Err("无效的默认视图".into());
    }
    if settings.recent_days > 365 {
        return Err("近期活动窗口需为 0–365 天；0 表示保留全部关注".into());
    }
    let mut ids = HashSet::new();
    let mut locations = HashSet::new();
    for source in &settings.sources {
        if source.id.trim().is_empty() || source.name.trim().is_empty() || !ids.insert(&source.id) {
            return Err("来源名称或 ID 无效/重复".into());
        }
        if !(500..=60_000).contains(&source.poll_interval_ms) {
            return Err("远程轮询周期需在 500–60000 毫秒之间".into());
        }
        if source.kind == SourceKind::CodexWsl {
            crate::providers::codex_wsl::validate_distro(&source.location)?;
        }
        if source.kind == SourceKind::GenericEvent {
            let integration = source
                .integration
                .as_ref()
                .ok_or("请先选择通用接入适配描述")?;
            let manifest = crate::integrations::manifest::load(&integration.adapter_id)?;
            if integration.transport != "local"
                || integration.agent_id != manifest.agent_id
                || !source.location.is_empty()
            {
                return Err("通用事件只支持本机且 Agent 身份必须匹配描述文件".into());
            }
        }
        if matches!(source.kind, SourceKind::RemoteCodex | SourceKind::Tmux)
            && (source.location.trim().is_empty()
                || source.location.starts_with('-')
                || source.location.chars().any(char::is_whitespace))
        {
            return Err("请输入有效的 SSH 别名".into());
        }
        let location = match source.kind {
            SourceKind::Zcode => Some(if source.location.is_empty() {
                crate::providers::zcode::default_db()
            } else {
                PathBuf::from(&source.location)
            }),
            SourceKind::Doubao => Some(if source.location.is_empty() {
                crate::providers::doubao::default_root()
            } else {
                PathBuf::from(&source.location)
            }),
            SourceKind::ClaudeCode => Some(if source.location.is_empty() {
                crate::providers::claude_code::default_root()
            } else {
                PathBuf::from(&source.location)
            }),
            SourceKind::DeepseekDesktop => Some(if source.location.is_empty() {
                crate::providers::deepseek_harness::DeepSeekHarnessProvider::desktop_root()
            } else {
                PathBuf::from(&source.location)
            }),
            SourceKind::Hermes => {
                if source.location.is_empty() {
                    crate::providers::hermes::discover_db()
                } else {
                    Some(PathBuf::from(&source.location))
                }
            }
            _ => None,
        };
        let location = location
            .map(|path| {
                path.canonicalize()
                    .unwrap_or(path)
                    .display()
                    .to_string()
                    .to_lowercase()
            })
            .unwrap_or_else(|| source.location.to_lowercase());
        if source.kind != SourceKind::GenericEvent
            && !locations.insert((format!("{:?}", source.kind), location))
        {
            return Err("相同运行环境只能添加一个同类来源".into());
        }
        if matches!(
            source.kind,
            SourceKind::Codex | SourceKind::Hooks | SourceKind::Scanner
        ) && !source.location.is_empty()
        {
            return Err("此来源使用默认本机配置，不支持自定义目录".into());
        }
        let builtin = match source.kind {
            SourceKind::Codex => Some("codex"),
            SourceKind::Hooks => Some("local_file"),
            SourceKind::Scanner => Some("scanner"),
            _ => None,
        };
        if builtin.is_some_and(|id| source.id != id) {
            return Err("本机 Codex、Hooks、扫描来源使用固定 ID，请启用已有来源".into());
        }
    }
    Ok(())
}

// Preserve fields owned by a newer version, without resurrecting deleted
// sources or merging user-edited project arrays by their array position.
fn preserve_unknown(old: &toml::Value, updated: &mut toml::Value) {
    if let (Some(old), Some(updated)) = (old.as_table(), updated.as_table_mut()) {
        for (key, value) in old {
            if let Some(current) = updated.get_mut(key) {
                if key == "sources" {
                    if let (Some(before), Some(after)) = (value.as_array(), current.as_array_mut())
                    {
                        for source in after {
                            if let Some(previous) = before.iter().find(|previous| {
                                source.get("id").and_then(toml::Value::as_str).is_some()
                                    && source.get("id") == previous.get("id")
                            }) {
                                preserve_unknown(previous, source);
                            }
                        }
                    }
                } else {
                    preserve_unknown(value, current);
                }
            } else {
                updated.insert(key.clone(), value.clone());
            }
        }
    }
}

pub fn write_settings(path: &Path, settings: &DashboardSettings) -> Result<(), String> {
    validate(settings)?;
    let old = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut value: toml::Value =
        toml::from_str(&old).map_err(|e| format!("原配置无法解析，未覆盖：{e}"))?;
    if value.get("dashboard").is_none() {
        let backup = path.with_extension("toml.pre-dashboard.bak");
        if !backup.exists() {
            std::fs::copy(path, &backup).map_err(|e| e.to_string())?;
        }
    }
    let mut dashboard = toml::Value::try_from(settings).map_err(|e| e.to_string())?;
    if let Some(previous) = value.get("dashboard") {
        preserve_unknown(previous, &mut dashboard);
    }
    value
        .as_table_mut()
        .ok_or("无效配置")?
        .insert("dashboard".into(), dashboard);
    let text = toml::to_string_pretty(&value).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("无效配置路径")?)
        .map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn save_monitoring_settings(
    app: AppHandle,
    mut settings: DashboardSettings,
    expected: Option<DashboardSettings>,
) -> Result<DashboardSettings, String> {
    let _writer = SETTINGS_WRITER.lock().unwrap_or_else(|e| e.into_inner());
    let previous = get_monitoring_settings();
    if expected
        .as_ref()
        .is_some_and(|expected| expected != &previous)
    {
        return Err("配置已更新，请重新加载后保存".into());
    }
    record_discovery_choices(&previous, &mut settings);
    for source in &mut settings.sources {
        if source.kind == SourceKind::GenericEvent {
            if let Some(integration) = &mut source.integration {
                let old = previous.sources.iter().find(|s| s.id == source.id);
                if let Some(value) = old.and_then(|s| s.integration.as_ref()) {
                    // Identity/adapter updates are not allowed to reinterpret an
                    // existing journal. Create a separate source instead.
                    if value.agent_id != integration.agent_id
                        || value.adapter_id != integration.adapter_id
                    {
                        return Err("改变 Agent/适配器请创建新来源，现有会话身份不会重写".into());
                    }
                    integration.accept_since_ms =
                        if old.is_some_and(|s| !s.enabled) && source.enabled {
                            crate::integrations::now_ms()
                        } else {
                            value.accept_since_ms
                        };
                } else {
                    integration.accept_since_ms = crate::integrations::now_ms();
                }
            }
        }
    }
    apply_settings(&app, settings)
}

fn apply_settings(
    app: &AppHandle,
    settings: DashboardSettings,
) -> Result<DashboardSettings, String> {
    write_settings(&config::config_path(), &settings)?;
    {
        let mut state = runtime().lock().unwrap_or_else(|e| e.into_inner());
        state.settings = Some(settings.clone());
        state.config_error = None;
        state.revision += 1;
    }
    crate::watcher::request_refresh();
    if !settings.popup_notifications() {
        crate::task_notice::clear(app);
    } else {
        crate::task_notice::retain_in_scope(app);
    }
    let _ = app.emit("monitoring-settings-updated", &settings);
    Ok(settings)
}

/// Only read-only native adapters are eligible. Remote/WSL, hooks and generic
/// event integrations always remain explicit user choices.
fn automatic_local(kind: SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::Codex
            | SourceKind::ClaudeCode
            | SourceKind::Hermes
            | SourceKind::DeepseekDesktop
            | SourceKind::Doubao
            | SourceKind::Zcode
    )
}

fn record_discovery_choices(previous: &DashboardSettings, next: &mut DashboardSettings) {
    let mut seen: HashSet<String> = previous.auto_discovery_seen.iter().cloned().collect();
    seen.extend(next.auto_discovery_seen.iter().cloned());
    for old in &previous.sources {
        let current = next.sources.iter().find(|s| s.id == old.id);
        if current.is_none()
            || current.is_some_and(|s| s != old)
            || (previous.onboarding_complete
                && !previous.auto_discover_local
                && next.auto_discover_local)
        {
            seen.insert(old.id.clone());
        }
    }
    for source in &next.sources {
        if !previous.sources.iter().any(|s| s.id == source.id) {
            seen.insert(source.id.clone());
        }
    }
    // First-run discovery selection is explicit, including deselected installs.
    if !previous.onboarding_complete
        && next.onboarding_complete
        && std::env::var_os("TASKPULSE_HOME").is_none()
    {
        for row in discover_source_rows_with_wsl(next, &[]) {
            if row.available {
                seen.insert(row.id);
            }
        }
    }
    next.auto_discovery_seen = seen.into_iter().collect();
    next.auto_discovery_seen.sort();
}

fn merge_discovered(settings: &mut DashboardSettings, discoveries: Vec<SourceDiscovery>) -> bool {
    if !settings.onboarding_complete || !settings.auto_discover_local {
        return false;
    }
    let before = settings.clone();
    for row in discoveries {
        if !row.available || settings.auto_discovery_seen.contains(&row.id) {
            continue;
        }
        let source = settings.sources.iter_mut().find(|s| s.id == row.id);
        if let Some(source) = source {
            if !automatic_local(source.kind) {
                continue;
            }
            source.enabled = true;
        } else if let Some(mut candidate) = row.candidate {
            if !automatic_local(candidate.kind)
                || settings.sources.iter().any(|s| s.kind == candidate.kind)
            {
                continue;
            }
            candidate.enabled = true;
            settings.sources.push(candidate);
        } else {
            continue;
        }
        settings.auto_discovery_seen.push(row.id);
    }
    settings.auto_discovery_seen.sort();
    *settings != before
}

pub fn start_auto_discovery(app: AppHandle) {
    if std::env::var_os("TASKPULSE_HOME").is_some() {
        return;
    }
    std::thread::spawn(move || loop {
        let previous = get_monitoring_settings();
        if previous.onboarding_complete
            && previous.auto_discover_local
            && get_monitoring_error().is_none()
        {
            // No WSL probing or process launch is needed for automatic local discovery.
            let rows = discover_source_rows_with_wsl(&previous, &[]);
            let mut next = previous.clone();
            if merge_discovered(&mut next, rows) {
                let _writer = SETTINGS_WRITER.lock().unwrap_or_else(|e| e.into_inner());
                // Do not overwrite a concurrent UI save or an external config edit.
                let disk = std::fs::read_to_string(config::config_path())
                    .ok()
                    .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
                    .and_then(|value| value.get("dashboard").cloned())
                    .and_then(|value| value.try_into::<DashboardSettings>().ok());
                if get_monitoring_settings() == previous && disk.as_ref() == Some(&previous) {
                    if let Err(error) = apply_settings(&app, next) {
                        log::warn!("自动发现来源保存失败: {error}");
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(60));
    });
}

pub fn source_id(task: &TaskSnapshot) -> String {
    if task.id.starts_with("scan:") && !task.remote {
        return "scanner".into();
    }
    task.source_instance
        .clone()
        .unwrap_or_else(|| match (task.source, task.remote) {
            (TaskSource::Doubao, _) => "doubao".into(),
            (TaskSource::Codex, true) => {
                format!("remote-codex:{}", task.host.as_deref().unwrap_or(""))
            }
            (TaskSource::RemoteTmux, _) => format!("tmux:{}", task.host.as_deref().unwrap_or("")),
            (TaskSource::Codex, false) => "codex".into(),
            (TaskSource::Hermes, _) => "hermes".into(),
            (TaskSource::DeepSeekHarness, _) if task.id.starts_with("dsh-native:") => {
                "deepseek_harness_desktop".into()
            }
            (TaskSource::DeepSeekHarness, _) => "deepseek_harness".into(),
            (TaskSource::ClaudeCode, _) if task.id.starts_with("claude:") => "claude_code".into(),
            (TaskSource::ClaudeCode | TaskSource::Gemini, _) => "scanner".into(),
            _ => "local_file".into(),
        })
}

pub fn source_for<'a>(
    settings: &'a DashboardSettings,
    task: &TaskSnapshot,
) -> Option<&'a SourceConfig> {
    if task.id.starts_with("scan:") && !task.remote {
        return settings.sources.iter().find(|s| s.id == "scanner");
    }
    if let Some(id) = &task.source_instance {
        return settings.sources.iter().find(|s| &s.id == id);
    }
    settings.sources.iter().find(|s| {
        if task.remote {
            s.location == task.host.as_deref().unwrap_or("")
                && matches!(
                    (s.kind, task.source),
                    (SourceKind::RemoteCodex, TaskSource::Codex)
                        | (SourceKind::Tmux, TaskSource::RemoteTmux)
                )
        } else {
            s.id == source_id(task)
        }
    })
}

fn project_matches(pattern: &str, task: &TaskSnapshot) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern == "@unassociated" {
        return task.project.is_none();
    }
    if pattern.contains('/') || pattern.contains('\\') {
        return task.project_path.as_deref().is_some_and(|path| {
            if path.starts_with('/') {
                path == pattern
            } else {
                path.eq_ignore_ascii_case(pattern)
            }
        });
    }
    task.project
        .as_deref()
        .is_some_and(|project| config::wildcard_match(pattern, project))
}

pub fn exclusion(settings: &DashboardSettings, task: &TaskSnapshot) -> Option<&'static str> {
    if task.internal {
        return Some("Agent 内部服务会话，不属于用户任务");
    }
    let Some(source) = source_for(settings, task) else {
        return Some("来源已移除");
    };
    if !source.enabled {
        return Some("来源未启用");
    }
    if task.id.starts_with("scan:") && !task.remote {
        let native = settings.sources.iter().find(|s| {
            matches!(
                (s.kind, task.source),
                (SourceKind::Codex, TaskSource::Codex)
                    | (SourceKind::ClaudeCode, TaskSource::ClaudeCode)
            )
        });
        if let Some(native) = native {
            if !native.enabled {
                return Some("来源未启用");
            }
            if native
                .excluded_projects
                .iter()
                .any(|p| project_matches(p, task))
                || !native.projects.iter().any(|p| project_matches(p, task))
            {
                return Some("项目不在监控范围");
            }
        }
    }
    if source.kind == SourceKind::Tmux
        && ((!source.tmux_sessions.is_empty()
            && !source.tmux_sessions.iter().any(|p| {
                task.tmux_session
                    .as_deref()
                    .is_some_and(|s| config::wildcard_match(p, s))
            }))
            || (!source.tmux_panes.is_empty()
                && !source.tmux_panes.iter().any(|p| {
                    task.tmux_pane
                        .as_deref()
                        .is_some_and(|s| config::wildcard_match(p, s))
                        || task
                            .tmux_target
                            .as_deref()
                            .is_some_and(|s| config::wildcard_match(p, s))
                })))
    {
        return Some("tmux 不在监控范围");
    }
    if source
        .excluded_projects
        .iter()
        .any(|p| project_matches(p, task))
        || !source.projects.iter().any(|p| project_matches(p, task))
    {
        return Some("项目不在监控范围");
    }
    None
}
pub fn allows(task: &TaskSnapshot) -> bool {
    exclusion(&get_monitoring_settings(), task).is_none()
}

pub fn observe(tasks: &[TaskSnapshot]) {
    let mut state = runtime().lock().unwrap_or_else(|e| e.into_inner());
    for task in tasks {
        let merged = catalog_observation(state.catalog.get(&task.id), task);
        state.catalog.insert(task.id.clone(), merged);
    }
}

fn catalog_observation(previous: Option<&TaskSnapshot>, incoming: &TaskSnapshot) -> TaskSnapshot {
    if let Some(previous) = previous.filter(|previous| incoming.replays_integration(previous)) {
        let mut retained = previous.clone();
        retained.merge_integration_health(incoming);
        return retained;
    }
    let mut merged = if incoming.status == crate::domain::TaskStatus::Unknown {
        let mut task = previous.cloned().unwrap_or_else(|| incoming.clone());
        task.name = incoming.name.clone();
        task.project = incoming.project.clone();
        task.project_path = incoming.project_path.clone();
        task
    } else {
        incoming.clone()
    };
    // Native session origin belongs to stable identity. A missing source in
    // a later partial snapshot must not reclassify a proven internal session.
    merged.internal = incoming.internal || previous.is_some_and(|t| t.internal);
    merged
}

pub fn refresh_known_metadata(tasks: &mut Vec<TaskSnapshot>) {
    let ids: HashSet<_> = tasks.iter().map(|t| t.id.clone()).collect();
    // Refresh known sessions without importing dormant catalog entries.
    let catalog = runtime()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .catalog
        .clone();
    for task in tasks.iter_mut() {
        if let Some(meta) = catalog.get(&task.id) {
            task.internal = meta.internal;
        }
    }
    for mut known in crate::watcher::known_sessions() {
        if ids.contains(&known.id) || !allows(&known) {
            continue;
        }
        if let Some(meta) = catalog.get(&known.id) {
            known.name = meta.name.clone();
            known.project = meta.project.clone();
            known.project_path = meta.project_path.clone();
            known.internal = meta.internal;
            known.new_execution = false;
            if known.status == crate::domain::TaskStatus::Running {
                known.stale = true;
            }
            tasks.push(known);
        }
    }
}

#[derive(Serialize)]
pub struct DiscoveredSession {
    pub task: TaskSnapshot,
    pub reason: String,
}
#[derive(Serialize)]
pub struct SessionPage {
    pub items: Vec<DiscoveredSession>,
    pub total: usize,
}

#[tauri::command]
pub fn list_discovered_sessions(
    source: Option<String>,
    query: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> SessionPage {
    let settings = get_monitoring_settings();
    let mut tasks: HashMap<String, TaskSnapshot> = crate::watcher::known_sessions()
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect();
    tasks.extend(
        runtime()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .catalog
            .clone(),
    );
    let query = query.unwrap_or_default().to_lowercase();
    let mut items: Vec<_> = tasks
        .into_values()
        .filter(|task| !task.internal)
        .filter(|task| {
            source
                .as_ref()
                .is_none_or(|s| source_for(&settings, task).is_some_and(|v| &v.id == s))
        })
        .filter(|task| {
            format!(
                "{} {} {}",
                task.name,
                task.project.as_deref().unwrap_or(""),
                task.project_path.as_deref().unwrap_or("")
            )
            .to_lowercase()
            .contains(&query)
        })
        .map(|task| {
            let reason = exclusion(&settings, &task)
                .map(str::to_string)
                .unwrap_or_else(|| crate::watcher::attention_reason(&task.id));
            DiscoveredSession { task, reason }
        })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.task.updated_at));
    let total = items.len();
    SessionPage {
        items: items
            .into_iter()
            .skip(offset.unwrap_or(0))
            .take(limit.unwrap_or(50).clamp(1, 100))
            .collect(),
        total,
    }
}

pub fn find_session(id: &str) -> Option<TaskSnapshot> {
    let catalog = runtime()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .catalog
        .get(id)
        .cloned();
    let known = crate::watcher::known_sessions()
        .into_iter()
        .find(|t| t.id == id);
    if let (Some(meta), Some(mut task)) = (&catalog, known.clone()) {
        if meta.status == crate::domain::TaskStatus::Unknown {
            task.name = meta.name.clone();
            task.project = meta.project.clone();
            task.project_path = meta.project_path.clone();
            task.internal = meta.internal;
            return Some(task);
        }
    }
    catalog.or(known)
}

#[derive(Serialize)]
pub struct SourceDiscovery {
    pub id: String,
    pub available: bool,
    pub detail: String,
    pub candidate: Option<SourceConfig>,
}
#[tauri::command]
pub async fn discover_sources() -> Vec<SourceDiscovery> {
    tauri::async_runtime::spawn_blocking(discover_source_rows)
        .await
        .unwrap_or_default()
}
fn discover_source_rows() -> Vec<SourceDiscovery> {
    let settings = get_monitoring_settings();
    if std::env::var_os("TASKPULSE_HOME").is_some() {
        return settings
            .sources
            .iter()
            .map(|s| SourceDiscovery {
                id: s.id.clone(),
                available: s.kind == SourceKind::GenericEvent,
                detail: "隔离状态目录；未探测真实 Agent 数据".into(),
                candidate: None,
            })
            .collect();
    }
    let wsl = crate::providers::codex_wsl::discover_codex_distros().unwrap_or_default();
    discover_source_rows_with_wsl(&settings, &wsl)
}

fn discover_source_rows_with_wsl(
    settings: &DashboardSettings,
    wsl: &[crate::providers::codex_wsl::WslCodexDiscovery],
) -> Vec<SourceDiscovery> {
    let mut sources = settings.sources.clone();
    for candidate in from_legacy(&TaskPulseConfig::default(), false).sources {
        if !sources.iter().any(|s| s.kind == candidate.kind) {
            sources.push(candidate);
        }
    }
    for found in wsl.iter().filter(|found| found.root.is_some()) {
        let distro = &found.distro;
        if !sources
            .iter()
            .any(|s| s.kind == SourceKind::CodexWsl && s.location.eq_ignore_ascii_case(distro))
        {
            sources.push(source(
                &format!("codex-wsl:{}", distro.to_lowercase()),
                &format!("Codex · WSL {distro}"),
                SourceKind::CodexWsl,
                distro,
            ));
        }
    }
    sources
        .iter()
        .map(|source| {
            let (available, detail) = match source.kind {
                SourceKind::Zcode => {
                    let path=if source.location.is_empty() {crate::providers::zcode::default_db()} else {PathBuf::from(&source.location)};
                    (path.is_file(), path.display().to_string())
                }
                SourceKind::Doubao => {
                    let root=if source.location.is_empty() {crate::providers::doubao::default_root()} else {PathBuf::from(&source.location)};
                    (crate::providers::doubao::available(&root),format!("{} · 原生执行状态；不提供审批提醒",root.display()))
                }
                SourceKind::ClaudeCode => {
                    let root = if source.location.is_empty() {
                        crate::providers::claude_code::default_root()
                    } else {
                        PathBuf::from(&source.location)
                    };
                    (
                        root.join("projects").exists() || root.join("sessions").exists(),
                        format!("{} · 原生会话观测；可启用精确事件", root.display()),
                    )
                }
                SourceKind::Codex => {
                    let root = std::env::var_os("CODEX_HOME").map(PathBuf::from)
                        .unwrap_or_else(|| dirs_next::home_dir().unwrap_or_default().join(".codex"));
                    (root.join("sessions").is_dir(), root.display().to_string())
                },
                SourceKind::CodexWsl => wsl
                        .iter()
                        .find(|found| found.distro.eq_ignore_ascii_case(&source.location))
                        .map(|found| (found.root.is_some(), found.detail.clone()))
                        .unwrap_or_else(|| (false, format!(
                            "WSL {} · 未运行或暂无法查询；不会自动唤醒，已保存配置仍保留；独立于 Windows",
                            source.location
                        ))),
                SourceKind::Hermes => {
                    let path = if source.location.is_empty() {
                        crate::providers::hermes::discover_db()
                    } else {
                        Some(PathBuf::from(&source.location)).filter(|p| p.exists())
                    };
                    (
                        path.is_some(),
                        path.map(|p| p.display().to_string())
                            .unwrap_or_else(|| "未发现 Hermes 状态数据库".into()),
                    )
                }
                SourceKind::DeepseekDesktop => {
                    let root = if source.location.is_empty() {
                        crate::providers::deepseek_harness::DeepSeekHarnessProvider::desktop_root()
                    } else {
                        PathBuf::from(&source.location)
                    };
                    (root.exists(), root.display().to_string())
                }
                SourceKind::DeepseekWsl => {
                    (false, format!("WSL {} · 启用后验证连接", source.location))
                }
                SourceKind::RemoteCodex | SourceKind::Tmux => {
                    (false, format!("SSH {} · 启用后验证连接", source.location))
                }
                _ => (true, "本机状态采集".into()),
            };
            SourceDiscovery {
                id: source.id.clone(),
                available,
                detail,
                candidate: (!settings.sources.iter().any(|s| s.id == source.id))
                    .then(|| source.clone()),
            }
        })
        .collect()
}

pub fn remote_config(settings: &DashboardSettings) -> TaskPulseConfig {
    let mut cfg = TaskPulseConfig::default();
    cfg.monitoring.remote_codex.clear();
    let mut hosts = HashSet::new();
    for source in settings
        .sources
        .iter()
        .filter(|s| s.enabled && matches!(s.kind, SourceKind::RemoteCodex | SourceKind::Tmux))
    {
        if hosts.insert(&source.location) {
            let tmux = settings
                .sources
                .iter()
                .find(|s| s.enabled && s.kind == SourceKind::Tmux && s.location == source.location);
            cfg.remote_hosts.push(RemoteHostConfig {
                id: settings
                    .sources
                    .iter()
                    .find(|s| s.location == source.location && s.kind == SourceKind::RemoteCodex)
                    .or_else(|| {
                        settings
                            .sources
                            .iter()
                            .find(|s| s.location == source.location && s.kind == SourceKind::Tmux)
                    })
                    .filter(|s| s.id.starts_with("remote-codex:") || s.id.starts_with("tmux:"))
                    .map(|s| s.id.as_str())
                    .unwrap_or(&source.location)
                    .split_once(':')
                    .map(|(_, id)| id.to_string())
                    .unwrap_or_else(|| source.location.clone()),
                ssh_host: source.location.clone(),
                enabled: true,
                poll_interval_ms: source.poll_interval_ms,
                tmux_sessions: tmux.map(|s| s.tmux_sessions.clone()).unwrap_or_default(),
                tmux_panes: tmux.map(|s| s.tmux_panes.clone()).unwrap_or_default(),
            });
        }
        if source.kind == SourceKind::RemoteCodex {
            cfg.monitoring.remote_codex.push(RemoteCodexScope {
                host: source.location.clone(),
                projects: source.projects.clone(),
            });
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn newly_saved_disabled_custom_sources_remain_disabled() {
        let mut previous = from_legacy(&TaskPulseConfig::default(), false);
        previous.onboarding_complete = true;
        let mut next = previous.clone();
        let mut custom = source(
            "custom-zcode",
            "ZCode custom",
            SourceKind::Zcode,
            "D:/custom.db",
        );
        custom.enabled = false;
        next.sources.push(custom);
        record_discovery_choices(&previous, &mut next);
        assert!(next.auto_discovery_seen.contains(&"custom-zcode".into()));
        assert!(!merge_discovered(
            &mut next,
            vec![SourceDiscovery {
                id: "custom-zcode".into(),
                available: true,
                detail: String::new(),
                candidate: None
            }]
        ));
        assert!(!next.sources.last().unwrap().enabled);
    }

    #[test]
    fn automatic_discovery_preserves_choices_and_only_enables_native_local_sources() {
        let mut settings = from_legacy(&TaskPulseConfig::default(), false);
        let rows = || {
            vec![
                SourceDiscovery {
                    id: "codex".into(),
                    available: true,
                    detail: String::new(),
                    candidate: None,
                },
                SourceDiscovery {
                    id: "codex-wsl:ubuntu".into(),
                    available: true,
                    detail: String::new(),
                    candidate: None,
                },
                SourceDiscovery {
                    id: "local_file".into(),
                    available: true,
                    detail: String::new(),
                    candidate: None,
                },
            ]
        };
        assert!(!merge_discovered(&mut settings, rows()));
        settings.onboarding_complete = true;
        settings.sources[0].projects = vec!["D:/chosen".into()];
        assert!(merge_discovered(&mut settings, rows()));
        assert!(settings.sources[0].enabled);
        assert_eq!(settings.sources[0].projects, vec!["D:/chosen"]);
        assert!(settings
            .sources
            .iter()
            .filter(|s| matches!(s.kind, SourceKind::CodexWsl | SourceKind::Hooks))
            .all(|s| !s.enabled));
        let previous = settings.clone();
        settings.sources[0].enabled = false;
        record_discovery_choices(&previous, &mut settings);
        assert!(!merge_discovered(&mut settings, rows()));
        let previous = settings.clone();
        settings.sources.retain(|s| s.id != "codex");
        record_discovery_choices(&previous, &mut settings);
        let candidate = source("codex", "Codex", SourceKind::Codex, "");
        assert!(!merge_discovered(
            &mut settings,
            vec![SourceDiscovery {
                id: "codex".into(),
                available: true,
                detail: String::new(),
                candidate: Some(candidate)
            }]
        ));
    }

    #[test]
    fn legacy_opt_in_keeps_existing_disabled_sources_and_scope() {
        let mut previous = from_legacy(&TaskPulseConfig::default(), true);
        previous.sources[0].enabled = false;
        previous.sources[0].projects = vec!["D:/explicit".into()];
        let mut next = previous.clone();
        next.auto_discover_local = true;
        record_discovery_choices(&previous, &mut next);
        assert!(next.auto_discovery_seen.contains(&"codex".into()));
        assert!(!next.sources[0].enabled);
        assert_eq!(next.sources[0].projects, previous.sources[0].projects);
        let mut absent = from_legacy(&TaskPulseConfig::default(), false);
        absent.onboarding_complete = true;
        assert!(merge_discovered(
            &mut absent,
            vec![SourceDiscovery {
                id: "zcode".into(),
                available: true,
                detail: String::new(),
                candidate: None
            }]
        ));
        assert!(
            absent
                .sources
                .iter()
                .find(|s| s.id == "zcode")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn wsl_discovery_requires_agent_evidence_and_preserves_saved_sources() {
        use crate::providers::codex_wsl::WslCodexDiscovery;
        let mut settings = from_legacy(&TaskPulseConfig::default(), true);
        settings.sources.retain(|s| s.kind != SourceKind::CodexWsl);
        let discoveries = vec![
            WslCodexDiscovery {
                distro: "Ubuntu".into(),
                root: Some("/home/user/.codex".into()),
                detail: "已验证 Codex 会话数据".into(),
            },
            WslCodexDiscovery {
                distro: "Debian".into(),
                root: None,
                detail: "未发现 Codex 会话数据".into(),
            },
            WslCodexDiscovery {
                distro: "docker-desktop".into(),
                root: None,
                detail: "内部运行环境".into(),
            },
        ];
        let rows = discover_source_rows_with_wsl(&settings, &discoveries);
        let candidates: Vec<_> = rows
            .iter()
            .filter(|r| r.available)
            .filter_map(|r| r.candidate.as_ref())
            .filter(|s| s.kind == SourceKind::CodexWsl)
            .collect();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].location, "Ubuntu");
        assert!(!candidates[0].enabled); // Discovery never opts users in.
        assert!(!rows.iter().any(|r| r.id.contains("docker-desktop")));

        let mut manual = source("manual-wsl", "My distro", SourceKind::CodexWsl, "UBUNTU");
        manual.enabled = true;
        manual.projects = vec!["/home/user/my-project".into()];
        settings.sources.push(manual.clone());
        let original = settings.clone();
        let rows = discover_source_rows_with_wsl(&settings, &discoveries);
        assert!(rows
            .iter()
            .any(|r| r.id == manual.id && r.available && r.candidate.is_none()));
        assert!(!rows.iter().any(|r| r.id == "codex-wsl:ubuntu"));
        // Stop/disconnect does not delete, disable or broaden a saved source.
        let stopped = discover_source_rows_with_wsl(&settings, &[]);
        let row = stopped.iter().find(|r| r.id == manual.id).unwrap();
        assert!(!row.available && row.candidate.is_none());
        assert_eq!(settings, original);

        let mut docker = source(
            "explicit-docker",
            "Manual",
            SourceKind::CodexWsl,
            "docker-desktop",
        );
        docker.enabled = true;
        settings.sources.push(docker.clone());
        let rows = discover_source_rows_with_wsl(&settings, &discoveries);
        assert!(rows
            .iter()
            .any(|r| r.id == docker.id && !r.available && r.candidate.is_none()));
        assert_eq!(settings.sources.last(), Some(&docker));
    }

    #[test]
    fn wsl_sources_are_opt_in_and_distro_scopes_never_alias_windows_or_ssh() {
        let mut settings = from_legacy(&TaskPulseConfig::default(), true);
        let wsl_source = settings
            .sources
            .iter_mut()
            .find(|s| s.kind == SourceKind::CodexWsl)
            .unwrap();
        assert!(!wsl_source.enabled);
        wsl_source.enabled = true;
        wsl_source.projects = vec!["/home/user/projects/Project".into()];
        let source_id = wsl_source.id.clone();
        let mut task = TaskSnapshot::new(
            "codex-wsl:ubuntu:codex:s",
            "native title",
            TaskSource::Codex,
            crate::domain::TaskStatus::Running,
            crate::domain::DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        task.source_instance = Some(source_id);
        task.project_path = Some("/home/user/projects/Project".into());
        assert!(exclusion(&settings, &task).is_none());
        task.project_path = Some("/home/user/projects/project".into());
        assert!(exclusion(&settings, &task).is_some());
        task.source_instance = Some("codex-wsl:other".into());
        assert_eq!(exclusion(&settings, &task), Some("来源已移除"));
        assert!(validate(&settings).is_ok());
        let another = source("wsl2", "Other", SourceKind::CodexWsl, "ubuntu");
        settings.sources.push(another);
        assert!(validate(&settings).is_err());
    }
    use crate::domain::{DetectionConfidence, TaskStatus};
    #[test]
    fn internal_service_sessions_are_outside_attention_and_notification_scope() {
        let mut settings = from_legacy(&TaskPulseConfig::default(), true);
        settings.sources[0].enabled = true;
        settings.sources[0].projects = vec!["*".into()];
        let mut task = TaskSnapshot::new(
            "codex:a",
            "Guardian review",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        assert!(exclusion(&settings, &task).is_none());
        task.internal = true;
        assert!(exclusion(&settings, &task).is_some());
    }

    #[test]
    fn native_internal_reclassification_preserves_lifecycle_and_partial_refresh_cannot_undo_it() {
        let mut old = TaskSnapshot::new(
            "codex:child",
            "Guardian review",
            TaskSource::Codex,
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        old.unread = true;
        old.pinned = true;
        old.turn_key = Some("child-turn".into());
        old.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        let mut metadata = old.clone();
        metadata.status = TaskStatus::Unknown;
        metadata.internal = true;
        metadata.name = "Renamed internal service".into();
        let classified = catalog_observation(Some(&old), &metadata);
        assert!(classified.internal);
        assert!(classified.unread && classified.pinned);
        assert_eq!(classified.status, old.status);
        assert_eq!(classified.turn_key, old.turn_key);
        assert_eq!(classified.updated_at, old.updated_at);
        metadata.internal = false; // source absent in a later partial snapshot
        assert!(catalog_observation(Some(&classified), &metadata).internal);
        metadata.status = TaskStatus::Running;
        assert!(catalog_observation(Some(&classified), &metadata).internal);
    }
    #[test]
    fn legacy_scopes_and_source_identity_are_preserved() {
        let mut cfg = TaskPulseConfig::default();
        cfg.monitoring.local_codex_projects = vec!["agenttray".into()];
        let settings = from_legacy(&cfg, true);
        assert_eq!(settings.sources[0].projects, vec!["agenttray"]);
        let mut task = TaskSnapshot::new(
            "dsh-native:a",
            "title",
            TaskSource::DeepSeekHarness,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        assert_eq!(
            source_for(&settings, &task).unwrap().kind,
            SourceKind::DeepseekDesktop
        );
        task.id = "dsh:a".into();
        assert_eq!(
            source_for(&settings, &task).unwrap().kind,
            SourceKind::DeepseekWsl
        );
    }
    #[test]
    fn recent_window_is_defaulted_without_resetting_old_explicit_project_scopes() {
        let mut settings = from_legacy(&TaskPulseConfig::default(), true);
        settings.sources[0].projects = vec!["my-project".into()];
        let mut value = serde_json::to_value(&settings).unwrap();
        value.as_object_mut().unwrap().remove("recent_days");
        value.as_object_mut().unwrap().remove("notification_mode");
        let restored: DashboardSettings = serde_json::from_value(value).unwrap();
        assert_eq!(restored.recent_days, 7);
        assert!(restored.popup_notifications());
        assert!(!restored.desktop_notifications());
        assert_eq!(restored.sources[0].projects, vec!["my-project"]);
        assert!(validate(&restored).is_ok());
        settings.recent_days = 366;
        assert!(validate(&settings).is_err());
        settings.recent_days = 0;
        assert!(validate(&settings).is_ok());
    }
    #[test]
    fn reminder_channels_respect_master_switch_and_validate_mode() {
        let mut settings = from_legacy(&TaskPulseConfig::default(), true);
        settings.notification_mode = "both".into();
        assert!(settings.popup_notifications() && settings.desktop_notifications());
        settings.notifications = false;
        assert!(!settings.popup_notifications() && !settings.desktop_notifications());
        settings.notification_mode = "unsupported".into();
        assert!(validate(&settings).is_err());
    }
    #[test]
    fn project_paths_and_exclusions_do_not_alias_same_names() {
        let mut settings = from_legacy(&TaskPulseConfig::default(), true);
        settings.sources[0].projects = vec!["C:\\work\\demo".into()];
        let mut task = TaskSnapshot::new(
            "codex:a",
            "title",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        task.project = Some("demo".into());
        task.project_path = Some("D:\\work\\demo".into());
        assert!(exclusion(&settings, &task).is_some());
        task.project_path = Some("c:\\work\\demo".into());
        assert!(exclusion(&settings, &task).is_none());
        settings.sources[0].excluded_projects = vec!["demo".into()];
        assert!(exclusion(&settings, &task).is_some());
    }

    #[test]
    fn scanner_cannot_bypass_disabled_native_source_and_host_identity_survives_disable() {
        let mut cfg = TaskPulseConfig::default();
        cfg.remote_hosts.push(RemoteHostConfig {
            id: "server3".into(),
            ssh_host: "Server3".into(),
            enabled: true,
            poll_interval_ms: 3000,
            tmux_sessions: vec![],
            tmux_panes: vec![],
        });
        let mut settings = from_legacy(&cfg, true);
        let task = TaskSnapshot::new(
            "scan:1",
            "process",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Inferred,
            std::time::SystemTime::now(),
        );
        settings.sources[0].enabled = false;
        assert_eq!(exclusion(&settings, &task), Some("来源未启用"));
        let remote_id = remote_config(&settings).remote_hosts[0].id.clone();
        settings
            .sources
            .iter_mut()
            .find(|s| s.kind == SourceKind::RemoteCodex)
            .unwrap()
            .enabled = false;
        assert_eq!(remote_config(&settings).remote_hosts[0].id, remote_id);
    }
    #[test]
    fn saving_preserves_unknown_config_and_backs_up_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "custom = 'keep'\n").unwrap();
        write_settings(&path, &from_legacy(&TaskPulseConfig::default(), true)).unwrap();
        assert!(path.with_extension("toml.pre-dashboard.bak").exists());
        let value: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["custom"].as_str(), Some("keep"));
    }

    #[test]
    fn nested_unknown_fields_follow_source_identity_not_position_or_deleted_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut settings = from_legacy(&TaskPulseConfig::default(), true);
        let mut old = toml::Value::try_from(&settings).unwrap();
        old.as_table_mut()
            .unwrap()
            .insert("future".into(), toml::Value::String("root".into()));
        old.as_table_mut().unwrap().insert(
            "future_table".into(),
            toml::toml! { nested = { enabled = true } }.into(),
        );
        let first_id = settings.sources[0].id.clone();
        old["sources"][0].as_table_mut().unwrap().insert(
            "future_source".into(),
            toml::Value::String(first_id.clone()),
        );
        let removed_id = settings.sources[1].id.clone();
        old["sources"][1].as_table_mut().unwrap().insert(
            "future_source".into(),
            toml::Value::String("do not resurrect".into()),
        );
        let outer = toml::toml! { custom = "keep" };
        let mut original = toml::Value::Table(outer);
        original
            .as_table_mut()
            .unwrap()
            .insert("dashboard".into(), old);
        std::fs::write(&path, toml::to_string(&original).unwrap()).unwrap();
        settings.sources.remove(1);
        settings.sources.reverse();
        settings
            .sources
            .iter_mut()
            .find(|s| s.id == first_id)
            .unwrap()
            .projects = vec!["changed".into()];
        settings.notifications = false;
        write_settings(&path, &settings).unwrap();
        let saved: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(saved["custom"].as_str(), Some("keep"));
        assert_eq!(saved["dashboard"]["future"].as_str(), Some("root"));
        assert_eq!(
            saved["dashboard"]["future_table"]["nested"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(saved["dashboard"]["notifications"].as_bool(), Some(false));
        let sources = saved["dashboard"]["sources"].as_array().unwrap();
        assert!(!sources
            .iter()
            .any(|s| s["id"].as_str() == Some(&removed_id)));
        let first = sources
            .iter()
            .find(|s| s["id"].as_str() == Some(&first_id))
            .unwrap();
        assert_eq!(first["future_source"].as_str(), Some(first_id.as_str()));
        assert_eq!(first["projects"][0].as_str(), Some("changed"));
    }
}
