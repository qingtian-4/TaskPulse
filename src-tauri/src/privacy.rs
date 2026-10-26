//! Allowlisted export/log views; private task content stays in local UI state.
use crate::domain::TaskSnapshot;
use crate::providers::ProviderHealth;
#[cfg(test)]
use crate::transition::TaskEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub fn fingerprint(value: &str) -> String {
    crate::integrations::key(value)[..16].to_owned()
}

pub fn config_root_key(root: &Path) -> String {
    let normalized = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let normalized = normalized.to_string_lossy();
    let normalized = if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized.into_owned()
    };
    crate::integrations::key(&normalized)
}

pub fn tool_activity(input: &Value) -> &'static str {
    match input.get("tool_name").and_then(Value::as_str) {
        Some("Bash" | "Shell" | "PowerShell") => "执行命令",
        Some("Read" | "ReadFile") => "读取文件",
        Some("Write" | "Edit" | "MultiEdit") => "编辑文件",
        Some("Glob" | "Grep" | "Search") => "搜索文件",
        Some("WebSearch" | "WebFetch") => "查询网络",
        _ => "执行工具",
    }
}

pub fn task_summary(task: &TaskSnapshot) -> Value {
    json!({"task_hash":fingerprint(&task.id),
        "source_hash":task.source_instance.as_deref().map(fingerprint),
        "session_hash":task.session_id.as_deref().map(fingerprint),
        "turn_hash":task.turn_key.as_deref().map(fingerprint),
        "source":task.source,"status":task.status,"phase":task.phase,
        "waiting_reason":task.waiting_reason,"pending_requests":task.input_request_ids.len(),
        "has_activity":task.activity.is_some(),"stale":task.stale,
        "unread":task.unread,"pinned":task.pinned})
}

pub fn provider_summary(provider: &ProviderHealth) -> Value {
    json!({"source_hash":fingerprint(&provider.provider),"status":provider.status,
        "last_success":provider.last_success,"last_attempt":provider.last_attempt,
        "has_error":provider.message.is_some()})
}

pub fn keyed_metrics<T: serde::Serialize>(metrics: &HashMap<String, T>) -> Value {
    Value::Object(
        metrics
            .iter()
            .map(|(key, value)| {
                (
                    fingerprint(key),
                    serde_json::to_value(value).unwrap_or(Value::Null),
                )
            })
            .collect(),
    )
}

/// Event kind alone is enough for application logs. No Debug task dumps.
#[cfg(test)]
fn event_kind(event: &TaskEvent) -> &'static str {
    match event {
        TaskEvent::WaitingInput { .. } => "waiting_input",
        TaskEvent::Succeeded { .. } => "succeeded",
        TaskEvent::Failed { .. } => "failed",
        TaskEvent::InferredFinished { .. } => "inferred_finished",
        TaskEvent::HostDisconnected { .. } => "host_disconnected",
        TaskEvent::HostReconnected { .. } => "host_reconnected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r04_export_allowlists_omit_arbitrary_payloads_and_provider_errors() {
        use crate::domain::{DetectionConfidence, TaskSource, TaskStatus};
        let secret = "private-token-command-path-title-reason";
        let mut task = TaskSnapshot::new(
            secret,
            secret,
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        task.source_instance = Some(secret.into());
        task.session_id = Some(secret.into());
        task.turn_key = Some(secret.into());
        task.message = Some(secret.into());
        task.activity = Some(secret.into());
        task.project = Some(secret.into());
        task.project_path = Some(secret.into());
        task.model = Some(secret.into());
        let provider = ProviderHealth {
            provider: secret.into(),
            status: crate::providers::ProviderStatus::Degraded,
            last_success: None,
            last_attempt: None,
            message: Some(secret.into()),
        };
        let event = TaskEvent::WaitingInput {
            task: task.clone(),
            reason: Some(secret.into()),
        };
        let metrics = [(secret.to_owned(), 5u64)].into();
        let output = json!({"task":task_summary(&task),"provider":provider_summary(&provider),
            "event":event_kind(&event),"metrics":keyed_metrics(&metrics)});
        assert!(!output.to_string().contains(secret));
        assert_eq!(output["task"]["status"], "running");
        assert_eq!(output["provider"]["has_error"], true);
        assert_eq!(output["event"], "waiting_input");
        let mut renamed = task.clone();
        renamed.name = "Different title".into();
        assert_eq!(
            task_summary(&task)["task_hash"],
            task_summary(&renamed)["task_hash"]
        );
    }
}
