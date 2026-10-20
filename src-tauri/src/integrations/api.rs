use super::{bridge, configuration, journal, key, manifest, now_ms};
use serde_json::{json, Value};

#[tauri::command]
pub fn integration_environment() -> Value {
    json!({"isolated":std::env::var_os("TASKPULSE_HOME").is_some(),"state_directory":crate::providers::remote_tmux::config::config_dir(),"executable":std::env::current_exe().ok(),"version":env!("CARGO_PKG_VERSION")})
}

#[tauri::command]
pub fn integration_catalog() -> Vec<manifest::Manifest> {
    let mut ids = vec![
        "standard-events".to_string(),
        "kimi-code-cli".into(),
        "trae-cn-ide".into(),
        "zcode-desktop".into(),
        "workbuddy-candidate".into(),
    ];
    if let Ok(entries) = std::fs::read_dir(super::home().join("manifests")) {
        for path in entries.take(128).filter_map(Result::ok).map(|e| e.path()) {
            if std::fs::metadata(&path).is_ok_and(|m| m.len() < 64 * 1024) {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if let Ok(m) = serde_json::from_str::<manifest::Manifest>(&text) {
                        if m.validate().is_ok() {
                            ids.push(m.adapter_id);
                        }
                    }
                }
            }
        }
    }
    ids.into_iter()
        .filter_map(|id| manifest::load(&id).ok())
        .collect()
}

#[tauri::command]
pub fn integration_import(text: String) -> Result<manifest::Manifest, String> {
    if text.len() > 64 * 1024 {
        return Err("manifest_too_large".into());
    }
    let mut m: manifest::Manifest =
        serde_json::from_str(&text).map_err(|_| "manifest_parse_error")?;
    m.validate()?;
    if manifest::builtin(&m.adapter_id).is_some() {
        return Err("builtin_manifest_cannot_be_overwritten".into());
    }
    // Untrusted manifests cannot promote themselves to official/native-tested.
    for c in m.capabilities.values_mut() {
        c.validation = "documented".into();
        c.tested_versions.clear();
        c.channel = "self-reported".into();
    }
    if let Ok(previous) = manifest::load(&m.adapter_id) {
        if previous.agent_id != m.agent_id {
            return Err("manifest_update_cannot_change_agent_identity".into());
        }
    }
    journal::atomic(
        &super::home()
            .join("manifests")
            .join(format!("{}.json", key(&m.adapter_id))),
        &m,
    )?;
    Ok(m)
}

#[tauri::command]
pub async fn integration_preview(
    source_id: String,
    remove: bool,
) -> Result<configuration::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || configuration::prepare(&source_id, remove))
        .await
        .map_err(|_| "preview_worker_failed")?
}
#[tauri::command]
pub async fn integration_apply(token: String, confirmed: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || configuration::apply(&token, confirmed))
        .await
        .map_err(|_| "configuration_worker_failed")??;
    Ok("已仅更新 TaskPulse 自有 Hook；请在宿主中重新加载并验证。".into())
}
#[tauri::command]
pub async fn integration_diagnose(source_id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut report = bridge::diagnose_source(&source_id)?;
        if let Some(health) = crate::watcher::get_providers()
            .into_iter()
            .find(|p| p.provider == source_id)
        {
            report["provider_health"] =
                serde_json::to_value(health).map_err(|_| "diagnostic_serialize_failed")?;
        }
        Ok(report)
    })
    .await
    .map_err(|_| "diagnostic_worker_failed")?
}
#[tauri::command]
pub fn integration_test(source_id: String) -> Result<Value, String> {
    let source = bridge::configured_source(&source_id)?;
    let config = source
        .integration
        .as_ref()
        .ok_or("integration_not_configured")?;
    let event = json!({"schema_version":1,"agent_id":config.agent_id,"session_id":"taskpulse-test-channel","event_id":format!("test:{}",now_ms()),"kind":"connection.test","payload":{}});
    bridge::receive(
        &source_id,
        &config.adapter_id,
        false,
        &serde_json::to_vec(&event).map_err(|_| "test_serialize_failed")?,
    )?;
    Ok(
        json!({"test":true,"accepted":true,"message":"仅测试接收连接；没有创建监控任务，也没有验证真实宿主事件。"}),
    )
}
#[tauri::command]
pub fn integration_export(source_id: String) -> Result<Value, String> {
    let source = bridge::configured_source(&source_id)?;
    let config = source
        .integration
        .as_ref()
        .ok_or("integration_not_configured")?;
    let exe = std::env::current_exe().map_err(|_| "executable_unavailable")?;
    Ok(
        json!({"transport":"local","argv":[exe,"--integration-event","--source",source_id,"--adapter",config.adapter_id],
        "sample":{"schema_version":1,"agent_id":config.agent_id,"session_id":"producer-stable-session","turn_id":"producer-stable-turn","event_id":"producer-stable-event","kind":"turn.started","payload":{}},
        "evidence":"self_reported","notice":"示例不是执行命令；生产者须传递 JSON 到 stdin。请使用长期保留的安装路径（不要引用 target/debug 或临时目录）。WSL/SSH 通用事件传输尚未实现。"}),
    )
}
