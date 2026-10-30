//! Only application-owned copy is translated. Agent-provided text is untouched.
use std::sync::LazyLock;
use tauri::{AppHandle, Manager};

static EN: LazyLock<std::collections::HashMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../locales/en.json"))
        .expect("bundled English translations")
});
pub struct TrayLabels(pub Vec<tauri::menu::MenuItem<tauri::Wry>>);
pub fn english() -> bool {
    let p = crate::monitoring::get_monitoring_settings().preferences;
    p.language == "en"
        || p.language == "system"
            && !sys_locale::get_locale()
                .unwrap_or_default()
                .to_lowercase()
                .starts_with("zh")
}
pub fn text(value: &str) -> String {
    if english() {
        if let Some(status) = value.strip_prefix("TaskPulse · ") {
            return format!(
                "TaskPulse · {}",
                EN.get(status).map(String::as_str).unwrap_or(status)
            );
        }
        EN.get(value).cloned().unwrap_or_else(|| value.into())
    } else {
        value.into()
    }
}
pub fn refresh_native(app: &AppHandle) {
    if let Some(labels) = app.try_state::<TrayLabels>() {
        for (item, label) in labels.0.iter().zip([
            "显示任务面板",
            "固定任务面板",
            "隐藏任务面板",
            "发送测试提醒",
            "退出 TaskPulse",
        ]) {
            if let Err(error) = item.set_text(text(label)) {
                log::warn!("Cannot update tray label: {error}");
            }
        }
    }
    if let Some(window) = app.get_webview_window("management") {
        let _ = window.set_title(&format!("TaskPulse · {}", text("设置")));
    }
}
