//! Read-only source discovery, separate from registry persistence and CAS.
use super::{
    from_legacy, get_monitoring_settings, source, DashboardSettings, SourceConfig, SourceKind,
    TaskPulseConfig,
};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Serialize)]
pub struct SourceDiscovery {
    pub id: String,
    pub location: String,
    pub data_location_detected: Option<bool>,
    pub detail: String,
    pub candidate: Option<SourceConfig>,
}
pub(super) fn discover_source_rows() -> Vec<SourceDiscovery> {
    let settings = get_monitoring_settings();
    if std::env::var_os("TASKPULSE_HOME").is_some() {
        return settings
            .sources
            .iter()
            .map(|s| SourceDiscovery {
                id: s.id.clone(),
                location: s.location.clone(),
                data_location_detected: None,
                detail: "隔离状态目录；未探测真实 Agent 数据".into(),
                candidate: None,
            })
            .collect();
    }
    let wsl = crate::providers::codex_wsl::discover_codex_distros().unwrap_or_default();
    discover_source_rows_with_wsl(&settings, &wsl)
}

pub(super) fn discover_source_rows_with_wsl(
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
        .filter(|source| source.kind.supported_on(std::env::consts::OS))
        .map(|source| {
            let (detected, detail) = match source.kind {
                SourceKind::KimiCode => {let roots=if source.location.is_empty(){crate::providers::kimi::default_roots()}else{vec![PathBuf::from(&source.location)]};(roots.iter().any(|p|crate::providers::kimi::available(p)),"Kimi Code / kimi-cli 原生 wire 日志；不修改 Hook".into())},
                SourceKind::Qoder => {let roots=if source.location.is_empty(){crate::providers::qoder::default_roots()}else{vec![PathBuf::from(&source.location)]};(roots.iter().any(|p|crate::providers::qoder::available(p)),"Qoder Desktop 数据库 / IDE、CLI 会话记录；支持范围取决于格式".into())},
                SourceKind::Trae => {let root=if source.location.is_empty(){crate::providers::trae::default_root()}else{PathBuf::from(&source.location)};(crate::providers::trae::available(&root),"Trae 会话目录；暂不读取任务进度".into())},
                SourceKind::Opencode => {
                    let path=if source.location.is_empty(){crate::providers::opencode::default_db()}else{PathBuf::from(&source.location)};
                    (path.is_file(),format!("{} · 原生任务；不提供独立提问/审批事件",path.display()))
                }
                SourceKind::Workbuddy => {
                    let root=if source.location.is_empty(){crate::providers::workbuddy::default_root()}else{PathBuf::from(&source.location)};
                    (root.join("workbuddy.db").is_file() && root.join("projects").is_dir(),format!("{} · 本机任务；不包含云端任务或独立提问/审批事件",root.display()))
                }
                SourceKind::Zcode => {
                    let path=if source.location.is_empty() {crate::providers::zcode::default_db()} else {PathBuf::from(&source.location)};
                    let index = crate::providers::zcode::desktop_index(&path).filter(|index| index.is_file());
                    (path.is_file() || index.is_some(), match index {
                        Some(index) => format!("{} · 桌面项目：{}", path.display(), index.display()),
                        None => path.display().to_string(),
                    })
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
                SourceKind::GenericEvent | SourceKind::Hooks | SourceKind::Scanner => (false, "启用后读取数据".into()),
            };
            SourceDiscovery {
                id: source.id.clone(),
                location: source.location.clone(),
                data_location_detected: if matches!(source.kind,
                    SourceKind::DeepseekWsl | SourceKind::RemoteCodex | SourceKind::Tmux
                    | SourceKind::GenericEvent | SourceKind::Hooks | SourceKind::Scanner
                ) { None } else { Some(detected) },
                detail,
                candidate: (!settings.sources.iter().any(|s| s.id == source.id))
                    .then(|| source.clone()),
            }
        })
        .collect()
}
