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
static POPUP_CREATING: AtomicBool = AtomicBool::new(false);

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
            && (!t.is_agent_session() || t.requires_action())
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
        } else if t.is_agent_session()
            && t.status == TaskStatus::WaitingInput
            && !t.requires_action()
        {
            5
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
        TaskStatus::WaitingInput if best.is_agent_session() && !best.requires_action() => "offline",
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
    if let Some(tray) = app.tray_by_id(&TrayIconId::new("main")) {
        let needs = tasks
            .iter()
            .filter(|t| t.requires_action() && !t.stale)
            .count();
        let running = tasks
            .iter()
            .filter(|t| (t.status == TaskStatus::Running || t.requires_action()) && !t.stale)
            .count();
        let unread = tasks
            .iter()
            .filter(|t| {
                t.unread
                    && !t.requires_action()
                    && !t.stale
                    && (matches!(t.status, TaskStatus::Succeeded | TaskStatus::Failed)
                        || t.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded)
                        || t.source == crate::domain::TaskSource::RemoteTmux
                            && t.confidence == crate::domain::DetectionConfidence::Inferred
                            && t.status == TaskStatus::Idle
                            && t.finished_at.is_some()
                            && t.exit_code.is_none())
            })
            .count();
        let uncertain = tasks
            .iter()
            .filter(|t| {
                t.stale
                    || !t.requires_action()
                        && matches!(t.status, TaskStatus::Unknown | TaskStatus::Disconnected)
                    || !t.requires_action()
                        && t.status == TaskStatus::WaitingInput
                        && t.waiting_reason != Some(crate::domain::WaitingReason::TurnEnded)
            })
            .count();
        let _ = tray.set_tooltip(Some(format!(
            "TaskPulse · 进行中 {running}（其中待处理 {needs}）· 待查看 {unread} · 待更新 {uncertain}"
        )));
    }

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
    set_popup_always_on_top(app, true);
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
            set_popup_always_on_top(app, false);
            let _ = app.emit("pinned-changed", false);
            if std::env::var_os("TASKPULSE_HOME").is_none() {
                let _ = app.save_window_state(StateFlags::POSITION);
            }
            let _ = win.hide();
            return;
        }
        set_popup_always_on_top(app, PINNED.load(Ordering::SeqCst));
        let _ = win.show();
        let _ = app.emit("popup-opened", ());
        let _ = win.set_focus();
        emit_current_state(app);
        return;
    }

    show_popup(app);
}

/// Notification activation must show, never toggle/hide an existing popup.
pub fn show_popup(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("popup") {
        set_popup_always_on_top(app, PINNED.load(Ordering::SeqCst));
        let was_hidden = !win.is_visible().unwrap_or(false);
        let _ = win.show();
        if was_hidden {
            let _ = app.emit("popup-opened", ());
        }
        let _ = win.set_focus();
        emit_current_state(app);
        return;
    }

    // WebView2 window creation deadlocks inside synchronous IPC/event handlers.
    // Tray clicks and notification activation can reach this before a popup exists.
    if POPUP_CREATING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        create_popup(&app);
        POPUP_CREATING.store(false, Ordering::Release);
    });
}

fn create_popup(app: &AppHandle) {
    let ready = app.clone();
    app.once("popup-ready", move |_| {
        if let Some(win) = ready.get_webview_window("popup") {
            let _ = win.show();
            let _ = win.set_focus();
        }
        emit_current_state(&ready);
    });
    let builder = WebviewWindowBuilder::new(app, "popup", WebviewUrl::default());
    let builder = if std::env::var_os("TASKPULSE_HOME").is_some() {
        builder.data_directory(crate::providers::remote_tmux::config::config_dir().join("webview2"))
    } else {
        builder
    };
    match builder
        .title("TaskPulse")
        .inner_size(412.0, 620.0)
        .decorations(false)
        // Windows adds a white non-client frame when an undecorated window
        // keeps its native shadow. The transparent dashboard draws its own edge.
        .shadow(false)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .always_on_top(PINNED.load(Ordering::SeqCst))
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()
    {
        Ok(win) => {
            let _ = fit_popup(&win, 620, false);
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("wmctrl")
                    .args(["-r", "TaskPulse", "-b", "remove,sticky"])
                    .output();
                let _ = std::process::Command::new("wmctrl")
                    .args(["-r", "TaskPulse", "-b", "add,sticky"])
                    .output();
            }
        }
        Err(e) => log::error!("Failed to create popup window: {e}"),
    }
}

#[tauri::command]
pub async fn open_management(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("management") {
        win.show().map_err(|e| e.to_string())?;
        return win.set_focus().map_err(|e| e.to_string());
    }
    let builder = WebviewWindowBuilder::new(&app, "management", WebviewUrl::App("manage/".into()));
    let builder = if std::env::var_os("TASKPULSE_HOME").is_some() {
        builder.data_directory(crate::providers::remote_tmux::config::config_dir().join("webview2"))
    } else {
        builder
    };
    let window = builder
        .title("TaskPulse · 管理监控来源")
        .inner_size(900.0, 640.0)
        .min_inner_size(720.0, 520.0)
        .resizable(true)
        .build()
        .map_err(|e| e.to_string())?;
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = app.emit("management-closed", ());
        }
    });
    Ok(())
}

fn emit_current_state(app: &AppHandle) {
    crate::watcher::emit_latest(app);
}

#[tauri::command]
pub fn toggle_pin(app: AppHandle) {
    let pinned = if PINNED.load(Ordering::SeqCst) {
        PINNED.store(false, Ordering::SeqCst);
        false
    } else {
        PINNED.store(true, Ordering::SeqCst);
        true
    };
    set_popup_always_on_top(&app, pinned);
    let _ = app.emit("pinned-changed", pinned);
}

pub fn hide_popup(app: &AppHandle) {
    PINNED.store(false, Ordering::SeqCst);
    set_popup_always_on_top(app, false);
    let _ = app.emit("pinned-changed", false);
    if let Some(win) = app.get_webview_window("popup") {
        if std::env::var_os("TASKPULSE_HOME").is_none() {
            let _ = app.save_window_state(StateFlags::POSITION);
        }
        let _ = win.hide();
    }
}

#[tauri::command]
pub fn get_pinned() -> bool {
    PINNED.load(Ordering::SeqCst)
}

fn set_popup_always_on_top(app: &AppHandle, pinned: bool) {
    if let Some(win) = app.get_webview_window("popup") {
        if let Err(error) = win.set_always_on_top(pinned) {
            log::warn!("failed to synchronize popup pin state: {error}");
        }
    }
}

#[tauri::command]
pub fn close_popup(app: AppHandle) {
    hide_popup(&app);
}

// Size in logical pixels, fit against the actual monitor work area in physical
// pixels. This avoids WebView screen/ResizeObserver feedback and taskbar overlap.
fn popup_geometry(
    area: (i32, i32, u32, u32),
    scale: f64,
    height: u32,
    current: Option<(i32, i32)>,
) -> (u32, u32, i32, i32) {
    let (left, top, available_width, available_height) = area;
    let margin = (8.0 * scale).round() as i32;
    let width = ((412.0 * scale).round() as u32)
        .min(available_width.saturating_sub((margin * 2) as u32).max(1));
    let height = ((f64::from(height.clamp(400, 620)) * scale).round() as u32)
        .min(available_height.saturating_sub((margin * 2) as u32).max(1));
    let min_x = left + margin.min(available_width.saturating_sub(width) as i32);
    let min_y = top + margin.min(available_height.saturating_sub(height) as i32);
    let max_x = (left + available_width as i32 - width as i32 - margin).max(min_x);
    let max_y = (top + available_height as i32 - height as i32 - margin).max(min_y);
    let (x, y) = current.unwrap_or((max_x, top + (24.0 * scale).round() as i32));
    (width, height, x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

fn fit_popup(
    window: &tauri::WebviewWindow,
    height: u32,
    keep_position: bool,
) -> Result<(), String> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("无法读取窗口所在屏幕")?;
    let area = monitor.work_area();
    let current = keep_position
        .then(|| window.outer_position().ok())
        .flatten()
        .map(|p| (p.x, p.y));
    let (width, height, x, y) = popup_geometry(
        (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        ),
        monitor.scale_factor(),
        height,
        current,
    );
    window
        .set_size(tauri::PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resize_dashboard(app: AppHandle, height: u32) -> Result<(), String> {
    let window = app.get_webview_window("popup").ok_or("任务窗口尚未创建")?;
    fit_popup(&window, height, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn popup_fits_work_area_at_scaled_and_negative_monitor_coordinates() {
        for scale in [1.0, 1.5, 2.0] {
            let (w, h, x, y) =
                popup_geometry((-1920, 40, 1920, 1000), scale, 620, Some((-9999, 9999)));
            assert_eq!(w, (412.0 * scale) as u32);
            assert!(x >= -1920 && y >= 40);
            assert!(x + w as i32 <= 0 && y + h as i32 <= 1040);
        }
        let (w, h, x, y) = popup_geometry((0, 0, 640, 480), 2.0, 620, None);
        assert!(w <= 608 && h <= 448 && x >= 16 && y >= 16);
        let (w, h, x, y) = popup_geometry((0, 0, 10, 10), 1.0, 0, None);
        assert!(x >= 0 && y >= 0 && x + w as i32 <= 10 && y + h as i32 <= 10);
    }

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
