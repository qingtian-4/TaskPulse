//! Tray lifecycle: aggregate attention state → icon, popup window show/hide/
//! pin, position restore.  Preserved from AgentTray, with the aggregate
//! priority updated to TaskPulse semantics:
//! FAILED > WAITING_INPUT > DISCONNECTED > RUNNING > SUCCEEDED/IDLE > UNKNOWN

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::image::Image;
use tauri::tray::TrayIconId;
use tauri::webview::Color;
use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager, WebviewUrl};

use tauri_plugin_window_state::{AppHandleExt, StateFlags};

use crate::domain::{TaskSnapshot, TaskStatus};
use crate::providers::{HostHealth, HostStatus};

static LAST_STATE: Mutex<String> = Mutex::new(String::new());
static PINNED: AtomicBool = AtomicBool::new(false);

/// Aggregate attention state across tasks (and host health).
/// Returns an icon-name string used by `icon_bytes`.
fn aggregate_state(tasks: &[TaskSnapshot], hosts: &[HostHealth]) -> &'static str {
    // Task attention wins over infrastructure health, matching the public
    // priority contract: FAILED > WAITING_INPUT > DISCONNECTED > RUNNING.
    if tasks.iter().any(|t| t.status == TaskStatus::Failed) {
        return "error";
    }
    if tasks.iter().any(|t| {
        t.status == TaskStatus::WaitingInput
            && t.waiting_reason != Some(crate::domain::WaitingReason::TurnEnded)
    }) {
        return "needs-input";
    }
    if hosts.iter().any(|h| h.status == HostStatus::Disconnected)
        || tasks.iter().any(|t| t.status == TaskStatus::Disconnected)
    {
        return "disconnected";
    }

    let Some(best) = tasks.iter().min_by_key(|t| {
        if t.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded) {
            4
        } else {
            t.status.priority()
        }
    }) else {
        return "offline";
    };
    match best.status {
        TaskStatus::Failed => "error",
        TaskStatus::WaitingInput
            if best.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded) =>
        {
            "idle"
        }
        TaskStatus::WaitingInput => "needs-input",
        TaskStatus::Disconnected => "disconnected",
        TaskStatus::Running => "working",
        TaskStatus::Succeeded | TaskStatus::Idle => "idle",
        TaskStatus::Unknown => "offline",
    }
}

fn icon_bytes(state: &str) -> &'static [u8] {
    match state {
        "needs-input" => include_bytes!("../icons/tray-needs-input.png"),
        "error" => include_bytes!("../icons/tray-error.png"),
        "working" => include_bytes!("../icons/tray-working.png"),
        "disconnected" => include_bytes!("../icons/tray-offline.png"),
        "idle" => include_bytes!("../icons/tray-idle.png"),
        _ => include_bytes!("../icons/tray-offline.png"),
    }
}

pub fn update_icon(app: &AppHandle, tasks: &[TaskSnapshot], hosts: &[HostHealth]) {
    let state = aggregate_state(tasks, hosts);

    {
        let mut last = LAST_STATE.lock().unwrap_or_else(|e| {
            log::warn!("LAST_STATE mutex poisoned, recovering");
            e.into_inner()
        });
        if last.as_str() == state {
            return;
        }
        *last = state.to_string();
    }

    let bytes = icon_bytes(state);
    if let Ok(icon) = Image::from_bytes(bytes) {
        if let Some(tray) = app.tray_by_id(&TrayIconId::new("main")) {
            let _ = tray.set_icon(Some(icon));
        }
    }
    log::debug!("Tray icon updated to: {state}");
}

pub fn pin_popup(app: &AppHandle) {
    PINNED.store(true, Ordering::SeqCst);
    let _ = app.emit("pinned-changed", true);

    if let Some(win) = app.get_webview_window("popup") {
        if !win.is_visible().unwrap_or(false) {
            let _ = win.show();
            let _ = win.set_focus();
            emit_current_state(app);
        }
        return;
    }

    toggle_popup(app);
}

pub fn toggle_popup(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("popup") {
        if win.is_visible().unwrap_or(false) {
            PINNED.store(false, Ordering::SeqCst);
            let _ = app.emit("pinned-changed", false);
            let _ = app.save_window_state(StateFlags::POSITION);
            let _ = win.hide();
            return;
        }
        let _ = win.show();
        let _ = win.set_focus();
        emit_current_state(app);
        return;
    }

    show_popup(app);
}

/// Notification activation must show, never toggle/hide an existing popup.
pub fn show_popup(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("popup") {
        let _ = win.show();
        let _ = win.set_focus();
        emit_current_state(app);
        return;
    }

    match WebviewWindowBuilder::new(app, "popup", WebviewUrl::default())
        .title("TaskPulse")
        .inner_size(440.0, 180.0)
        .decorations(false)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()
    {
        Ok(win) => {
            let (default_x, default_y) = compute_default_position(app);
            let _ = win.set_position(tauri::PhysicalPosition::new(default_x, default_y));
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("wmctrl")
                    .args(["-r", "TaskPulse", "-b", "remove,sticky"])
                    .output();
                let _ = std::process::Command::new("wmctrl")
                    .args(["-r", "TaskPulse", "-b", "add,sticky"])
                    .output();
            }
            let _ = win.show();
            let _ = win.set_focus();
            let h = app.clone();
            app.once("popup-ready", move |_| {
                emit_current_state(&h);
            });
        }
        Err(e) => log::error!("Failed to create popup window: {e}"),
    }
}

#[tauri::command]
pub fn open_management(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("management") {
        win.show().map_err(|e| e.to_string())?;
        return win.set_focus().map_err(|e| e.to_string());
    }
    WebviewWindowBuilder::new(&app, "management", WebviewUrl::App("manage/".into()))
        .title("TaskPulse · 管理监控来源")
        .inner_size(900.0, 640.0)
        .min_inner_size(720.0, 520.0)
        .resizable(true)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn emit_current_state(app: &AppHandle) {
    crate::watcher::emit_latest(app);
}

#[tauri::command]
pub fn toggle_pin(app: AppHandle) {
    if PINNED.load(Ordering::SeqCst) {
        PINNED.store(false, Ordering::SeqCst);
        let _ = app.emit("pinned-changed", false);
    } else {
        PINNED.store(true, Ordering::SeqCst);
        let _ = app.emit("pinned-changed", true);
    }
}

pub fn hide_popup(app: &AppHandle) {
    PINNED.store(false, Ordering::SeqCst);
    let _ = app.emit("pinned-changed", false);
    if let Some(win) = app.get_webview_window("popup") {
        let _ = app.save_window_state(StateFlags::POSITION);
        let _ = win.hide();
    }
}

#[tauri::command]
pub fn get_pinned() -> bool {
    PINNED.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn close_popup(app: AppHandle) {
    hide_popup(&app);
}

fn compute_default_position(app: &AppHandle) -> (i32, i32) {
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| {
            app.available_monitors()
                .ok()
                .and_then(|m| m.into_iter().next())
        })
        .map(|m| {
            (
                m.position().x + m.size().width as i32 - (450.0 * m.scale_factor()) as i32,
                m.position().y + 32,
            )
        })
        .unwrap_or((100, 32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn task(status: TaskStatus) -> TaskSnapshot {
        TaskSnapshot::new(
            "test:0",
            "test",
            crate::domain::TaskSource::LocalFile,
            status,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        )
    }

    #[test]
    fn aggregate_empty_returns_offline() {
        assert_eq!(aggregate_state(&[], &[]), "offline");
    }

    #[test]
    fn aggregate_single_agent() {
        assert_eq!(
            aggregate_state(&[task(TaskStatus::Running)], &[]),
            "working"
        );
    }

    #[test]
    fn aggregate_waiting_input_beats_running() {
        let tasks = vec![
            task(TaskStatus::Idle),
            task(TaskStatus::Running),
            task(TaskStatus::WaitingInput),
        ];
        assert_eq!(aggregate_state(&tasks, &[]), "needs-input");
    }

    #[test]
    fn aggregate_failed_beats_waiting() {
        let tasks = vec![task(TaskStatus::WaitingInput), task(TaskStatus::Failed)];
        assert_eq!(aggregate_state(&tasks, &[]), "error");
    }

    #[test]
    fn aggregate_disconnected_host_beats_running() {
        let tasks = vec![task(TaskStatus::Running), task(TaskStatus::Running)];
        let hosts = vec![HostHealth::disconnected("Server3", None)];
        assert_eq!(aggregate_state(&tasks, &hosts), "disconnected");
    }

    #[test]
    fn aggregate_failed_beats_disconnected_host() {
        let tasks = vec![task(TaskStatus::Failed)];
        let hosts = vec![HostHealth::disconnected("Server3", None)];
        assert_eq!(aggregate_state(&tasks, &hosts), "error");
    }

    #[test]
    fn aggregate_waiting_beats_disconnected_host() {
        let tasks = vec![task(TaskStatus::WaitingInput)];
        let hosts = vec![HostHealth::disconnected("Server3", None)];
        assert_eq!(aggregate_state(&tasks, &hosts), "needs-input");
    }

    #[test]
    fn aggregate_succeeded_idle() {
        let tasks = vec![task(TaskStatus::Succeeded), task(TaskStatus::Idle)];
        assert_eq!(aggregate_state(&tasks, &[]), "idle");
    }

    #[test]
    fn aggregate_unknown() {
        assert_eq!(
            aggregate_state(&[task(TaskStatus::Unknown)], &[]),
            "offline"
        );
    }

    #[test]
    fn completed_turn_does_not_mask_a_running_task_as_blocked() {
        let mut ended = task(TaskStatus::WaitingInput);
        ended.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        assert_eq!(
            aggregate_state(&[ended.clone(), task(TaskStatus::Running)], &[]),
            "working"
        );
        assert_eq!(aggregate_state(&[ended], &[]), "idle");
    }
}
