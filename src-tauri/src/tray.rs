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
    // Host disconnected is a strong signal even with no tasks.
    if hosts.iter().any(|h| h.status == HostStatus::Disconnected) {
        return "disconnected";
    }

    let Some(best) = tasks.iter().min_by_key(|t| t.status.priority()) else {
        return "offline";
    };
    match best.status {
        TaskStatus::Failed => "error",
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

    match WebviewWindowBuilder::new(app, "popup", WebviewUrl::default())
        .title("TaskPulse")
        .inner_size(400.0, 120.0)
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
        .map(|m| (m.size().width as i32 - 410, 32))
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
}
