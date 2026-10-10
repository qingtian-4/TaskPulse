//! Explicit, reversible installation of documented Claude hooks. Existing
//! hooks, statusLine, proxy/model settings and credentials are left intact.
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
const EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "StopFailure",
    "Notification",
];

fn own_hook(value: &Value) -> bool {
    value.get("tag").and_then(Value::as_str) == Some("taskpulse-claude-events")
        || value
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| command.ends_with(" --claude-event --taskpulse-hook"))
        || value.get("args") == Some(&json!(["--claude-event", "--taskpulse-hook"]))
}

fn merge_settings(mut settings: Value, command: &str, enabled: bool) -> Result<Value, String> {
    let object = settings
        .as_object_mut()
        .ok_or("Claude 配置不是 JSON 对象")?;
    if !object.contains_key("hooks") && !enabled {
        return Ok(settings);
    }
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Claude hooks 配置格式不正确")?;
    for event in EVENTS {
        if !enabled && !hooks.contains_key(*event) {
            continue;
        }
        let entries = hooks
            .entry(*event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("Claude Hook 事件格式不正确")?;
        let mut removed = false;
        entries.retain_mut(|entry| {
            if let Some(nested) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                let owned = nested.iter().any(own_hook);
                if owned {
                    removed = true;
                    nested.retain(|h| !own_hook(h));
                    return !nested.is_empty();
                }
            }
            true
        });
        if enabled {
            entries.push(json!({"matcher":"","hooks":[{"type":"command","command":command,"args":["--claude-event","--taskpulse-hook"],"timeout":5}]}));
        }
        if removed && entries.is_empty() {
            hooks.remove(*event);
        }
    }
    if hooks.is_empty() {
        object.remove("hooks");
    }
    Ok(settings)
}

#[tauri::command]
pub async fn configure_claude_events(
    source_id: String,
    enabled: bool,
    confirmed: bool,
) -> Result<String, String> {
    if !confirmed {
        return Err("请先确认修改 Claude 配置；此操作可撤销".into());
    }
    let settings = crate::monitoring::get_monitoring_settings();
    let source = settings
        .sources
        .iter()
        .find(|s| s.id == source_id && s.kind == crate::monitoring::SourceKind::ClaudeCode)
        .ok_or("Claude 来源不存在")?;
    let root = if source.location.is_empty() {
        crate::providers::claude_code::default_root()
    } else {
        source.location.clone().into()
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Use only documented Hook properties. Ownership lives in our command
    // marker, not an invented schema field that Claude could reject.
    // Exec form bypasses Git Bash/PowerShell quoting on Windows entirely.
    let command = exe.to_string_lossy().into_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let path = root.join("settings.json");
        let before = read_settings(&path)?;
        let existing = match &before {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|e| format!("Claude 配置解析失败；未修改：{e}"))?,
            None => json!({}),
        };
        let updated = merge_settings(existing, &command, enabled)?;
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let backup = root.join("settings.json.pre-taskpulse.bak");
        if before.is_some() && !backup.exists() {
            std::fs::copy(&path, backup).map_err(|e| e.to_string())?;
        }
        if read_settings(&path)? != before {
            return Err("Claude 配置已被其他程序更新，请重试".into());
        }
        let bytes = serde_json::to_vec_pretty(&updated).map_err(|e| e.to_string())?;
        atomic_write(&path, &bytes).map_err(|e| e.to_string())?;
        let message = if enabled {
            "已启用精确事件。请在 Claude 中重新加载 Hook 或重开会话；已有 statusLine 和代理设置保留。"
        } else {
            "已移除 TaskPulse 精确事件，其他 Claude 配置保留。"
        };
        Ok(message.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn read_settings(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Claude 配置不可读；未修改：{error}")),
    }
}

pub fn receive_event() -> Result<(), String> {
    let mut bytes = vec![];
    std::io::stdin()
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let input: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let id = input
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 160
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        })
        .ok_or("invalid session ID")?;
    let event = input
        .get("hook_event_name")
        .and_then(Value::as_str)
        .filter(|e| EVENTS.contains(e))
        .ok_or("unknown hook event")?;
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let dir = dirs_next::home_dir()
        .unwrap_or_default()
        .join(".taskpulse/claude-events");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{id}.json"));
    let previous: Value = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let turn = if event == "UserPromptSubmit" {
        format!("prompt:{at}")
    } else {
        previous
            .get("turn_key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let turn_started_at = if event == "UserPromptSubmit" {
        Some(at)
    } else {
        previous.get("turn_started_at").and_then(Value::as_u64)
    };
    let tool = input.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let command = input
        .pointer("/tool_input/command")
        .or_else(|| input.pointer("/tool_input/file_path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let payload = json!({"session_id":id,"hook_event_name":event,"observed_at":at,"turn_key":turn,"turn_started_at":turn_started_at,
        "config_dir":crate::providers::claude_code::default_root(),
        "cwd":input.get("cwd"),"tool_use_id":input.get("tool_use_id"),"notification_type":input.get("notification_type"),
        "activity":format!("{tool} · {}",command.chars().take(300).collect::<String>())});
    atomic_write(
        &path,
        &serde_json::to_vec(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn install_is_idempotent_and_uninstall_preserves_proxy_statusline_and_other_hooks() {
        let original = json!({"env":{"ANTHROPIC_BASE_URL":"https://example.invalid","ANTHROPIC_AUTH_TOKEN":"fixture"},"model":"deepseek", "statusLine":{"type":"command","command":"ccusage"},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"my-script"}]}]}});
        let installed = merge_settings(
            original.clone(),
            "C:\\Program Files\\TaskPulse\\agent-tray.exe",
            true,
        )
        .unwrap();
        assert_eq!(
            installed,
            merge_settings(
                installed.clone(),
                "C:\\Program Files\\TaskPulse\\agent-tray.exe",
                true
            )
            .unwrap()
        );
        assert_eq!(
            original,
            merge_settings(
                installed,
                "C:\\Program Files\\TaskPulse\\agent-tray.exe",
                false
            )
            .unwrap()
        );
    }
}
