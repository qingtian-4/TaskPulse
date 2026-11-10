//! The UI discovers supported connections from data, not per-brand UI branches.
use crate::integrations::manifest::Manifest;
use crate::monitoring::{SourceConfig, SourceKind};
use serde::Serialize;

#[derive(Serialize)]
pub struct SourceDefinition {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub category: String,
    pub description: String,
    pub location_kind: String,
    pub multiple_instances: bool,
    pub platforms: Vec<String>,
    pub adapter_id: Option<String>,
    pub agent_id: Option<String>,
    pub capabilities: Capabilities,
    pub verified_platforms: Vec<String>,
    pub default_source: Option<SourceConfig>,
    pub auto_enable_on_discovery: bool,
}
/// Catalog claims describe what a reader can observe, never authorize action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Unsupported,
    Explicit,
    Inferred,
    Hook,
    Producer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnTarget {
    Details,
    HostWindow,
}

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub discovery: Evidence,
    pub model: Evidence,
    pub turn_start: Evidence,
    pub turn_end: Evidence,
    pub approval: Evidence,
    pub question: Evidence,
    pub return_target: ReturnTarget,
}

fn native_capabilities(kind: SourceKind) -> Capabilities {
    use Evidence::*;
    use SourceKind::*;
    let (discovery, model, turn_start, turn_end, approval, question, return_target) = match kind {
        KimiCode => (
            Explicit,
            Explicit,
            Explicit,
            Explicit,
            Explicit,
            Explicit,
            ReturnTarget::Details,
        ),
        Qoder => (
            Explicit,
            Inferred,
            Explicit,
            Explicit,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
        Trae => (
            Explicit,
            Unsupported,
            Unsupported,
            Unsupported,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
        Codex | CodexWsl => (
            Explicit,
            Explicit,
            Explicit,
            Explicit,
            Hook,
            Explicit,
            ReturnTarget::Details,
        ),
        RemoteCodex => (
            Explicit,
            Explicit,
            Inferred,
            Inferred,
            Unsupported,
            Explicit,
            ReturnTarget::Details,
        ),
        RemoteClaudeCode => (
            Explicit,
            Explicit,
            Inferred,
            Explicit,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
        ClaudeCode => (
            Explicit,
            Explicit,
            Inferred,
            Explicit,
            Hook,
            Unsupported,
            ReturnTarget::Details,
        ),
        Hermes => (
            Explicit,
            Explicit,
            Inferred,
            Unsupported,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
        DeepseekDesktop | DeepseekWsl => (
            Explicit,
            Explicit,
            Inferred,
            Explicit,
            Explicit,
            Explicit,
            ReturnTarget::Details,
        ),
        Opencode | Workbuddy | Zcode => (
            Explicit,
            Explicit,
            Inferred,
            Explicit,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
        Doubao => (
            Explicit,
            Unsupported,
            Inferred,
            Explicit,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
        Tmux => (
            Inferred,
            Unsupported,
            Inferred,
            Inferred,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
        Hooks => (
            Producer,
            Unsupported,
            Producer,
            Producer,
            Producer,
            Producer,
            ReturnTarget::Details,
        ),
        Scanner => (
            Inferred,
            Unsupported,
            Unsupported,
            Unsupported,
            Unsupported,
            Unsupported,
            ReturnTarget::HostWindow,
        ),
        GenericEvent => (
            Unsupported,
            Unsupported,
            Unsupported,
            Unsupported,
            Unsupported,
            Unsupported,
            ReturnTarget::Details,
        ),
    };
    Capabilities {
        discovery,
        model,
        turn_start,
        turn_end,
        approval,
        question,
        return_target,
    }
}

fn event_capabilities(manifest: &Manifest) -> Capabilities {
    let claims = &manifest.capabilities;
    let get = |key| match claims.get(key).map(|c| c.support.as_str()) {
        Some("explicit") => Evidence::Producer,
        Some("inferred") => Evidence::Inferred,
        _ => Evidence::Unsupported,
    };
    Capabilities {
        discovery: get("session_discovery"),
        model: if claims.contains_key("model") {
            get("model")
        } else if manifest.adapter_id == "standard-events" || manifest.fields.contains_key("model")
        {
            Evidence::Producer
        } else {
            Evidence::Unsupported
        },
        turn_start: get("turn_start"),
        turn_end: get("final_end"),
        approval: get("approval_request"),
        question: get("question"),
        // A self-reported event has no authenticated window or session target.
        return_target: ReturnTarget::Details,
    }
}
fn definition(
    id: &str,
    name: &str,
    kind: SourceKind,
    category: &str,
    location: &str,
) -> SourceDefinition {
    SourceDefinition {
        id: id.into(),
        name: name.into(),
        kind,
        category: category.into(),
        description: match kind {
            SourceKind::KimiCode => "只读 Kimi Code 和旧版 kimi-cli 会话。明确轮次结束、审批和提问按请求 ID 关联；模型与推理强度仅显示日志提供的值。尚需对应版本实机验收。",
            SourceKind::Qoder => "只读 Qoder Desktop 轮次投影和 IDE/CLI 转录。完整投影或明确结束字段才结束任务；原生审批与提问尚未支持。",
            SourceKind::Trae => "公开会话发现；当前非标准会话库的任务状态读取受限。审批、提问和完成提醒仍需独立事件接入。",
            SourceKind::Opencode => "只读本机 OpenCode 会话、模型和运行记录；明确本轮结束才提醒，不从工具返回推断完成。提问与审批事件尚不提供。",
            SourceKind::Workbuddy => "只读 WorkBuddy 本地数据库与会话记录；关注新执行，不导入空闲历史，不包含云端任务。提问与审批事件尚不提供。",
            SourceKind::Zcode => "自动读取本机 ZCode 会话、项目和模型；工具执行完成不会当作整轮结束。",
            SourceKind::Doubao => "暂停支持。",
            SourceKind::Codex => "原生标题、项目与 rollout 执行阶段；仅明确结束事件提醒。命令审批需可选的官方观察 Hook，安装后还需在 Codex /hooks 中信任。",
            SourceKind::CodexWsl => "独立读取已运行发行版的 Codex 数据；不唤醒 WSL，不借用 Windows 额度。命令审批需在该发行版安装并信任观察 Hook。",
            SourceKind::RemoteCodex => "读取已配置 SSH 主机的 Codex 会话；依赖主机可达、权限与远端数据格式。",
            SourceKind::RemoteClaudeCode => "只读 SSH 账号的 Claude Code 会话记录；明确结束才提醒，不读取审批或提问。",
            SourceKind::ClaudeCode => "原生会话读取为尽力推断。自愿接入 Hook 后可观察审批；提问接口目前无法同时确认对话框和匹配解除。",
            SourceKind::Hermes => "原生标题、项目和运行租约；没有明确结束证据时不猜测任务完成。",
            SourceKind::DeepseekDesktop => "读取原生 Desktop / CLI 存储，支持旧版与 v4 日志；状态能力取决于宿主记录。",
            SourceKind::DeepseekWsl => "独立读取已启用 WSL 安装，不与 Windows 桌面会话混合。",
            SourceKind::Tmux => "普通 shell 返回仅为推断结束，退出码未知；前台 tp-run 可报告直接子进程退出。",
            SourceKind::Hooks => "读取兼容状态文件；状态准确性由写入方决定，不自动安装宿主 Hook。",
            SourceKind::Scanner => "只提供受限进程证据；不能用 CPU、静默日志或进程消失证明本轮结束。",
            SourceKind::GenericEvent => "通过已连接的 Agent 事件跟踪任务；具体支持的状态见接入说明。",
        }.into(),
        location_kind: location.into(),
        multiple_instances: matches!(kind, SourceKind::CodexWsl | SourceKind::DeepseekWsl | SourceKind::RemoteCodex | SourceKind::RemoteClaudeCode | SourceKind::Tmux | SourceKind::GenericEvent),
        platforms: ["windows", "macos", "linux"].into_iter()
            .filter(|platform| kind.supported_on(platform))
            .map(str::to_owned)
            .collect(),
        adapter_id: None,
        agent_id: None,
        capabilities: native_capabilities(kind),
        verified_platforms: vec![],
        default_source: None,
        auto_enable_on_discovery: crate::monitoring::automatic_local(kind),
    }
}
#[tauri::command]
pub fn get_source_catalog() -> Vec<SourceDefinition> {
    use SourceKind::*;
    let mut entries = vec![
        definition("codex", "Codex", Codex, "local", "none"),
        definition(
            "claude_code",
            "Claude Code",
            ClaudeCode,
            "local",
            "directory",
        ),
        definition(
            "deepseek_harness_desktop",
            "DeepSeek Harness",
            DeepseekDesktop,
            "local",
            "directory",
        ),
        definition("opencode", "OpenCode", Opencode, "local", "file"),
        definition("workbuddy", "WorkBuddy", Workbuddy, "local", "directory"),
        definition("kimi_code", "Kimi Code", KimiCode, "local", "directory"),
        definition("qoder", "Qoder", Qoder, "local", "directory"),
        definition("trae", "Trae", Trae, "local", "directory"),
        definition("zcode", "ZCode", Zcode, "local", "file"),
        definition("hermes", "Hermes", Hermes, "local", "file"),
        definition("scanner", "进程检测", Scanner, "local", "none"),
        definition("codex_wsl", "Codex · WSL", CodexWsl, "remote", "distro"),
        definition(
            "deepseek_harness",
            "DeepSeek Harness · WSL",
            DeepseekWsl,
            "remote",
            "distro",
        ),
        definition("remote_codex", "Codex · SSH", RemoteCodex, "remote", "host"),
        definition(
            "remote_claude_code",
            "Claude Code · SSH",
            RemoteClaudeCode,
            "remote",
            "host",
        ),
        definition("tmux", "tmux · SSH", Tmux, "remote", "host"),
        definition("local_file", "兼容状态文件", Hooks, "custom", "none"),
    ];
    let defaults = crate::monitoring::from_legacy(&Default::default(), false).sources;
    for entry in &mut entries {
        entry.default_source = defaults.iter().find(|s| s.kind == entry.kind).cloned();
        if matches!(entry.kind, CodexWsl | DeepseekWsl) {
            entry.platforms = vec!["windows".into()];
        }
    }
    for manifest in crate::integrations::api::integration_catalog() {
        // Existing legacy event sources remain editable; new WorkBuddy uses its native reader.
        if manifest.adapter_id == "workbuddy-candidate" {
            continue;
        }
        let capabilities = event_capabilities(&manifest);
        entries.push(SourceDefinition {
            id: format!("integration:{}", manifest.adapter_id),
            name: manifest.display_name,
            kind: GenericEvent,
            category: "custom".into(),
            description: "通过已连接的 Agent 事件跟踪任务；具体支持的状态见接入说明。".into(),
            location_kind: "none".into(),
            multiple_instances: true,
            platforms: manifest.systems,
            adapter_id: Some(manifest.adapter_id),
            agent_id: Some(manifest.agent_id),
            capabilities,
            verified_platforms: vec![],
            default_source: None,
            auto_enable_on_discovery: false,
        });
    }
    entries
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_metadata_keeps_process_detection_separate_from_lifecycle() {
        let entries = get_source_catalog();
        let process = entries
            .iter()
            .find(|e| e.kind == SourceKind::Scanner)
            .unwrap();
        assert_eq!(process.capabilities.turn_end, Evidence::Unsupported);
        assert_eq!(process.capabilities.approval, Evidence::Unsupported);
        assert!(!entries.iter().any(|e| e.kind == SourceKind::Doubao));
        assert!(entries
            .iter()
            .all(|e| e.verified_platforms.iter().all(|p| e.platforms.contains(p))));
        let event = entries
            .iter()
            .find(|e| e.id == "integration:standard-events")
            .unwrap();
        assert_eq!(event.capabilities.approval, Evidence::Producer);
        assert_eq!(event.capabilities.return_target, ReturnTarget::Details);
        assert!(event.verified_platforms.is_empty());
        assert_eq!(entries.iter().filter(|e| e.category == "local").count(), 11);
        assert!(entries
            .iter()
            .filter(|e| e.category == "local")
            .all(|e| !e.multiple_instances));
        assert!(entries
            .iter()
            .filter(|e| e.category == "remote")
            .all(|e| e.multiple_instances));
        let ids: std::collections::HashSet<_> = entries.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), entries.len());
    }
}
