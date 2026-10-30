//! The UI discovers supported connections from data, not per-brand UI branches.
use crate::monitoring::SourceKind;
use serde::Serialize;

#[derive(Serialize)]
pub struct SourceDefinition {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub category: String,
    pub description: String,
    pub location_kind: String,
    pub platforms: Vec<String>,
    pub adapter_id: Option<String>,
    pub agent_id: Option<String>,
    pub capabilities: serde_json::Value,
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
            SourceKind::Opencode => "只读本机 OpenCode 会话、模型和运行记录；明确本轮结束才提醒，不从工具返回推断完成。提问与审批事件尚不提供。",
            SourceKind::Workbuddy => "只读 WorkBuddy 本地数据库与会话记录；关注新执行，不导入空闲历史，不包含云端任务。提问与审批事件尚不提供。",
            SourceKind::Zcode => "自动读取本机 ZCode 会话、项目和模型；工具执行完成不会当作整轮结束。",
            SourceKind::Doubao => "读取豆包原生执行检查点；已验证 Windows 2.26.10。检查点移除只标为待更新，不推断完成；审批提醒尚未适配。",
            SourceKind::Codex => "原生标题、项目与 rollout 执行阶段；仅明确结束事件提醒。命令审批需可选的官方观察 Hook，安装后还需在 Codex /hooks 中信任。",
            SourceKind::CodexWsl => "独立读取已运行发行版的 Codex 数据；不唤醒 WSL，不借用 Windows 额度。命令审批需在该发行版安装并信任观察 Hook。",
            SourceKind::RemoteCodex => "读取已配置 SSH 主机的 Codex 会话；依赖主机可达、权限与远端数据格式。",
            SourceKind::ClaudeCode => "原生会话读取为尽力推断；精确生命周期需自愿配置官方 Hook，与代理模型无关。",
            SourceKind::Hermes => "原生标题、项目和运行租约；没有明确结束证据时不猜测任务完成。",
            SourceKind::DeepseekDesktop => "读取原生 Desktop / CLI 存储，支持旧版与 v4 日志；状态能力取决于宿主记录。",
            SourceKind::DeepseekWsl => "独立读取已启用 WSL 安装，不与 Windows 桌面会话混合。",
            SourceKind::Tmux => "普通 shell 返回仅为推断结束，退出码未知；前台 tp-run 可报告直接子进程退出。",
            SourceKind::Hooks => "读取兼容状态文件；状态准确性由写入方决定，不自动安装宿主 Hook。",
            SourceKind::Scanner => "只提供受限进程证据；不能用 CPU、静默日志或进程消失证明本轮结束。",
            SourceKind::GenericEvent => "通过已连接的 Agent 事件跟踪任务；具体支持的状态见接入说明。",
        }.into(),
        location_kind: location.into(),
        platforms: vec!["windows".into(), "macos".into(), "linux".into()],
        adapter_id: None,
        agent_id: None,
        capabilities: serde_json::json!({}),
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
        definition("zcode", "ZCode", Zcode, "local", "file"),
        definition("hermes", "Hermes", Hermes, "local", "file"),
        definition("doubao", "豆包", Doubao, "local", "directory"),
        definition(
            "scanner",
            "进程识别（含 Gemini CLI）",
            Scanner,
            "local",
            "none",
        ),
        definition("codex_wsl", "Codex · WSL", CodexWsl, "remote", "distro"),
        definition(
            "deepseek_harness",
            "DeepSeek Harness · WSL",
            DeepseekWsl,
            "remote",
            "distro",
        ),
        definition("remote_codex", "Codex · SSH", RemoteCodex, "remote", "host"),
        definition("tmux", "tmux · SSH", Tmux, "remote", "host"),
        definition("local_file", "兼容状态文件", Hooks, "custom", "none"),
    ];
    for entry in &mut entries {
        if matches!(entry.kind, CodexWsl | DeepseekWsl) {
            entry.platforms = vec!["windows".into()];
        }
        if entry.kind == Scanner {
            entry.capabilities = serde_json::json!({"discovery":"process","turn_end":"unsupported","approval":"unsupported"});
        }
    }
    for manifest in crate::integrations::api::integration_catalog() {
        // Existing legacy event sources remain editable; new WorkBuddy uses its native reader.
        if manifest.adapter_id == "workbuddy-candidate" {
            continue;
        }
        entries.push(SourceDefinition {
            id: format!("integration:{}", manifest.adapter_id),
            name: manifest.display_name,
            kind: GenericEvent,
            category: "custom".into(),
            description: "通过已连接的 Agent 事件跟踪任务；具体支持的状态见接入说明。".into(),
            location_kind: "none".into(),
            platforms: manifest.systems,
            adapter_id: Some(manifest.adapter_id),
            agent_id: Some(manifest.agent_id),
            capabilities: serde_json::to_value(manifest.capabilities).unwrap_or_default(),
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
        assert_eq!(process.capabilities["turn_end"], "unsupported");
        assert_eq!(entries.iter().filter(|e| e.category == "local").count(), 9);
        let ids: std::collections::HashSet<_> = entries.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), entries.len());
    }
}
