//! Tray lifecycle: aggregate attention state → icon, popup window show/hide/
//! pin, position restore.  Preserved from AgentTray, with the aggregate
//! priority updated to TaskPulse semantics:
//! ACTION > ERROR/UNCERTAIN > UNVIEWED RESULT > RUNNING > VERIFIED IDLE

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::tray::TrayIconId;
use tauri::webview::Color;
use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager, WebviewUrl};

use tauri_plugin_window_state::{AppHandleExt, StateFlags};

use crate::domain::{TaskSnapshot, TaskStatus, WaitingReason};
use crate::providers::{HostStatus, ProviderStatus};
use crate::watcher::DashboardSnapshot;

static LAST_PRESENTATION: Mutex<AppliedPresentation> = Mutex::new(AppliedPresentation {
    revision: 0,
    icon: None,
    tooltip: None,
});

#[derive(Default)]
struct AppliedPresentation {
    revision: u64,
    icon: Option<&'static str>,
    tooltip: Option<String>,
}

impl AppliedPresentation {
    fn accepts(&self, revision: u64) -> bool {
        revision >= self.revision
    }
    fn needs_icon(&self, state: &str) -> bool {
        self.icon != Some(state)
    }
    fn needs_tooltip(&self, tooltip: &str) -> bool {
        self.tooltip.as_deref() != Some(tooltip)
    }
}
static PINNED: AtomicBool = AtomicBool::new(false);
static POPUP_CREATING: AtomicBool = AtomicBool::new(false);

// Keep text readable; the narrow CSS layout reduces spacing instead of fonts.
// Logical size preference is independent of the monitor's Retina/DPI scale.
const DASHBOARD_WIDTH: f64 = 368.0;
const DASHBOARD_MIN_WIDTH: f64 = 320.0;
const DASHBOARD_MIN_HEIGHT: u32 = 360;
const DASHBOARD_MAX_HEIGHT: u32 = 840;
static DASHBOARD_HEIGHT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(DASHBOARD_MAX_HEIGHT);

/// Presentation only: consumes the same scoped, reconciled attention snapshot
/// as the Dashboard. Never changes lifecycle, unread state or notifications.
#[derive(Debug, PartialEq, Eq)]
struct TrayPresentation {
    icon: &'static str,
    tooltip: String,
}

fn result_to_review(task: &TaskSnapshot) -> bool {
    task.unread
        && !task.requires_action()
        && (matches!(task.status, TaskStatus::Succeeded | TaskStatus::Failed)
            || task.status == TaskStatus::WaitingInput
                && task.waiting_reason == Some(WaitingReason::TurnEnded)
            || !task.is_agent_session()
                && task.source == crate::domain::TaskSource::RemoteTmux
                && task.confidence == crate::domain::DetectionConfidence::Inferred
                && task.status == TaskStatus::Idle
                && task.finished_at.is_some()
                && task.exit_code.is_none())
}

fn aggregate(snapshot: &DashboardSnapshot) -> TrayPresentation {
    let mut needs = 0;
    let mut running = 0;
    let mut unread = 0;
    let mut failed = 0;
    let mut uncertain = 0;
    let unhealthy = snapshot
        .providers
        .iter()
        .filter(|p| p.status == ProviderStatus::Degraded || p.last_success.is_none())
        .count();
    let disconnected = snapshot
        .hosts
        .iter()
        .filter(|h| h.status == HostStatus::Disconnected)
        .count();
    for task in &snapshot.tasks {
        let source_unhealthy = task.source_instance.as_ref().is_some_and(|id| {
            snapshot
                .providers
                .iter()
                .find(|p| &p.provider == id)
                .is_none_or(|p| {
                    (p.status == ProviderStatus::Degraded || p.last_success.is_none())
                        && !p.verifies_task(task, std::time::SystemTime::now())
                })
        }) || task.host.as_ref().is_some_and(|host| {
            snapshot
                .hosts
                .iter()
                .any(|h| &h.host == host && h.status == HostStatus::Disconnected)
        });
        let stale = task.stale || source_unhealthy;
        if stale
            || matches!(task.status, TaskStatus::Unknown | TaskStatus::Disconnected)
            || task.status == TaskStatus::WaitingInput
                && !task.requires_action()
                && task.waiting_reason != Some(WaitingReason::TurnEnded)
        {
            uncertain += 1;
        }
        if stale {
            continue;
        }
        if task.requires_action() {
            needs += 1;
        }
        if task.status == TaskStatus::Running || task.requires_action() {
            running += 1;
        }
        if result_to_review(task) {
            unread += 1;
        }
        // A viewed result retained only by a pin does not demand attention again.
        if task.status == TaskStatus::Failed && task.unread {
            failed += 1;
        }
    }
    let starting = snapshot.revision == 0;
    let icon = if needs > 0 {
        "needs-input"
    } else if failed > 0 || unhealthy > 0 || disconnected > 0 || uncertain > 0 || starting {
        "error"
    } else if unread > 0 {
        "review"
    } else if running > 0 {
        "working"
    } else {
        "idle"
    };
    // Keep this comfortably below Windows' 127 UTF-16-unit tooltip limit.
    let tooltip = format!(
        "TaskPulse · 进行中 {running}（待处理 {needs}）· 待查看 {unread} · 任务失败 {failed} · 来源异常 {unhealthy} / 断连 {disconnected} · 状态待更新 {}",
        uncertain + usize::from(starting)
    );
    TrayPresentation { icon, tooltip }
}

fn icon_bytes(state: &str) -> &'static [u8] {
    match state {
        "needs-input" => include_bytes!("../icons/tray-needs-input.png"),
        "working" => include_bytes!("../icons/tray-working.png"),
        "review" => include_bytes!("../icons/tray-review.png"),
        "idle" => include_bytes!("../icons/tray-idle.png"),
        _ => include_bytes!("../icons/tray-error.png"),
    }
}

pub fn update_icon(app: &AppHandle, snapshot: &DashboardSnapshot) {
    let next = aggregate(snapshot);
    let revision = snapshot.revision;
    let handle = app.clone();
    // Tauri setters wait for the main thread. Acquire the cache lock there,
    // never on a worker: popup-open can also update the tray on the main thread.
    // Otherwise it could wait for the worker lock while that worker waits for it.
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(&TrayIconId::new("main")) else {
            return;
        };
        // Reject late snapshots; cache only successful native writes. Failed
        // writes retry on an existing watcher update, without any new timer.
        let mut last = LAST_PRESENTATION.lock().unwrap_or_else(|e| e.into_inner());
        if !last.accepts(revision) {
            return;
        }
        last.revision = revision;
        if last.needs_icon(next.icon) {
            match Image::from_bytes(icon_bytes(next.icon))
                .and_then(|icon| tray.set_icon(Some(icon)))
            {
                Ok(()) => {
                    last.icon = Some(next.icon);
                    log::debug!("Tray icon updated to: {}", next.icon);
                }
                Err(error) => log::warn!("Tray icon update failed: {error}"),
            }
        }
        if last.needs_tooltip(&next.tooltip) {
            match tray.set_tooltip(Some(&next.tooltip)) {
                Ok(()) => last.tooltip = Some(next.tooltip),
                Err(error) => log::warn!("Tray tooltip update failed: {error}"),
            }
        }
    }) {
        log::warn!("Tray update scheduling failed: {error}");
    }
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
        let _ = fit_popup(&win, DASHBOARD_HEIGHT.load(Ordering::SeqCst), true);
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
        let _ = fit_popup(&win, DASHBOARD_HEIGHT.load(Ordering::SeqCst), true);
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
        .inner_size(DASHBOARD_WIDTH, f64::from(DASHBOARD_MAX_HEIGHT))
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
            let _ = fit_popup(&win, DASHBOARD_MAX_HEIGHT, false);
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
pub async fn open_management(app: AppHandle, section: Option<String>) -> Result<(), String> {
    let section = management_section(section.as_deref());
    if let Some(win) = app.get_webview_window("management") {
        if let Some(section) = section {
            app.emit_to("management", "management-section", section)
                .map_err(|e| e.to_string())?;
        }
        fit_management(&win)?;
        win.show().map_err(|e| e.to_string())?;
        return win.set_focus().map_err(|e| e.to_string());
    }
    let builder = WebviewWindowBuilder::new(
        &app,
        "management",
        WebviewUrl::App(
            section
                .map_or_else(|| "manage/".into(), |s| format!("manage/?section={s}"))
                .into(),
        ),
    );
    let builder = if std::env::var_os("TASKPULSE_HOME").is_some() {
        builder.data_directory(crate::providers::remote_tmux::config::config_dir().join("webview2"))
    } else {
        builder
    };
    let window = builder
        .title(format!("TaskPulse · {}", crate::localization::text("设置")))
        .inner_size(960.0, 704.0)
        .resizable(false)
        .maximizable(false)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    let area = management_area(&window)?;
    fit_management_in(&window, area)?;
    window.show().map_err(|e| e.to_string())?;
    let fitted = window.clone();
    let fit_state = Arc::new(Mutex::new(ManagementFitState {
        area: Some(area),
        queued: false,
        force: false,
    }));
    window.on_window_event(move |event| {
        if management_geometry_event(event) {
            let queue = {
                let mut state = fit_state.lock().unwrap_or_else(|e| e.into_inner());
                state.force |= matches!(event, tauri::WindowEvent::ScaleFactorChanged { .. });
                if state.queued {
                    false
                } else {
                    state.queued = true;
                    true
                }
            };
            if queue {
                let dispatcher = fitted.clone();
                let target = fitted.clone();
                let state = fit_state.clone();
                // A worker queues this after the current native DPI callback.
                // Fitting inside that callback races Windows' suggested resize
                // and can apply scaling twice. No timing delay is required.
                tauri::async_runtime::spawn(async move {
                    let callback_state = state.clone();
                    let result = dispatcher.run_on_main_thread(move || {
                        let area = management_area(&target);
                        let should_fit = {
                            let mut state =
                                callback_state.lock().unwrap_or_else(|e| e.into_inner());
                            state.queued = false;
                            let force = std::mem::take(&mut state.force);
                            area.as_ref().is_ok_and(|area| {
                                management_area_changed(&mut state.area, *area) || force
                            })
                        };
                        if let Ok(area) = area {
                            if should_fit {
                                if let Err(error) = fit_management_in(&target, area) {
                                    log::warn!("Could not fit settings window: {error}");
                                    callback_state
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .area = None;
                                }
                            }
                        }
                    });
                    if let Err(error) = result {
                        state.lock().unwrap_or_else(|e| e.into_inner()).queued = false;
                        log::warn!("Could not queue settings window fit: {error}");
                    }
                });
            }
        }
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = app.emit("management-closed", ());
        }
    });
    Ok(())
}

fn management_section(section: Option<&str>) -> Option<&str> {
    section.filter(|s| {
        matches!(
            *s,
            "general" | "panel" | "sources" | "notifications" | "updates" | "about"
        )
    })
}

fn management_geometry_event(event: &tauri::WindowEvent) -> bool {
    matches!(
        event,
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
    )
}

#[derive(Clone, Copy, PartialEq)]
struct ManagementArea {
    bounds: (i32, i32, u32, u32),
    scale: f64,
}

struct ManagementFitState {
    area: Option<ManagementArea>,
    queued: bool,
    force: bool,
}

fn management_area_changed(previous: &mut Option<ManagementArea>, area: ManagementArea) -> bool {
    if *previous == Some(area) {
        return false;
    }
    *previous = Some(area);
    true
}

fn management_geometry(
    area: (i32, i32, u32, u32),
    scale: f64,
    decoration: (u32, u32),
    position: (i32, i32),
) -> (u32, u32, i32, i32) {
    let (left, top, available_width, available_height) = area;
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let dimension = |available: u32, border: u32, preferred: f64| {
        let usable = available.saturating_sub(border).max(1);
        let logical = (f64::from(usable) / scale - 32.0).clamp(1.0, preferred);
        ((logical * scale).round() as u32).min(usable).max(1)
    };
    let width = dimension(available_width, decoration.0, 960.0);
    let height = dimension(available_height, decoration.1, 704.0);
    let max_x =
        (i64::from(left) + i64::from(available_width) - i64::from(width) - i64::from(decoration.0))
            .max(i64::from(left));
    let max_y = (i64::from(top) + i64::from(available_height)
        - i64::from(height)
        - i64::from(decoration.1))
    .max(i64::from(top));
    (
        width,
        height,
        i64::from(position.0).clamp(i64::from(left), max_x) as i32,
        i64::from(position.1).clamp(i64::from(top), max_y) as i32,
    )
}

fn fit_management(window: &tauri::WebviewWindow) -> Result<(), String> {
    fit_management_in(window, management_area(window)?)
}

fn management_area(window: &tauri::WebviewWindow) -> Result<ManagementArea, String> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("Cannot read monitor work area")?;
    let area = monitor.work_area();
    Ok(ManagementArea {
        bounds: (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        ),
        scale: monitor.scale_factor().max(0.1),
    })
}

fn fit_management_in(window: &tauri::WebviewWindow, area: ManagementArea) -> Result<(), String> {
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let outer = window.outer_size().map_err(|e| e.to_string())?;
    let decoration = (
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    );
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let (width, height, x, y) = management_geometry(
        area.bounds,
        area.scale,
        decoration,
        (position.x, position.y),
    );
    // Setters emit window events themselves. Only change an actual difference,
    // so the follow-up Moved event converges without resizing or moving again.
    if (inner.width, inner.height) != (width, height) {
        window
            .set_size(tauri::PhysicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
    }
    if (position.x, position.y) != (x, y) {
        window
            .set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
    }
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
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let margin = (8.0 * scale).round() as i32;
    let logical_width =
        (f64::from(available_width) / scale * 0.32).clamp(DASHBOARD_MIN_WIDTH, DASHBOARD_WIDTH);
    // Expanded task lists can use more of the screen; keep the idle view compact.
    let work_area_fraction = if height > 400 { 0.92 } else { 0.82 };
    let logical_height = f64::from(height.clamp(DASHBOARD_MIN_HEIGHT, DASHBOARD_MAX_HEIGHT))
        .min(f64::from(available_height) / scale * work_area_fraction);
    let width = ((logical_width * scale).round() as u32)
        .min(available_width.saturating_sub((margin * 2) as u32).max(1));
    let height = ((logical_height * scale).round() as u32)
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
    DASHBOARD_HEIGHT.store(height, Ordering::SeqCst);
    fit_popup(&window, height, true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn management_routes_all_sections_and_rejects_url_injection() {
        for section in [
            "general",
            "panel",
            "sources",
            "notifications",
            "updates",
            "about",
        ] {
            assert_eq!(super::management_section(Some(section)), Some(section));
        }
        for section in ["", "unknown", "sources?section=updates", "../notice"] {
            assert_eq!(super::management_section(Some(section)), None);
        }
        assert_eq!(super::management_section(None), None);
    }

    #[test]
    fn management_move_without_dpi_change_is_refitted_and_converges() {
        let first = super::ManagementArea {
            bounds: (0, 0, 1920, 1040),
            scale: 1.0,
        };
        let mut previous = None;
        assert!(super::management_area_changed(&mut previous, first));
        // Moving near/over the same display edge must not snap back.
        assert!(!super::management_area_changed(&mut previous, first));
        let smaller = super::ManagementArea {
            bounds: (-1280, 0, 1280, 720),
            scale: 1.0,
        };
        assert!(super::management_area_changed(&mut previous, smaller));
        assert!(!super::management_area_changed(&mut previous, smaller));
        assert!(super::management_area_changed(
            &mut previous,
            super::ManagementArea {
                bounds: smaller.bounds,
                scale: 1.5
            }
        ));
        assert!(super::management_geometry_event(
            &tauri::WindowEvent::Moved(tauri::PhysicalPosition::new(-1200, 100))
        ));
        assert!(!super::management_geometry_event(
            &tauri::WindowEvent::Focused(true)
        ));
        let decoration = (16, 39);
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let large =
                super::management_geometry((0, 0, 3840, 2160), scale, decoration, (100, 100));
            let small = (-1280, 40, 1280, 720);
            let next = super::management_geometry(small, scale, decoration, (-500, 400));
            assert!(next.0 <= large.0 && next.1 <= large.1);
            assert!(next.2 >= -1280 && next.3 >= 40);
            assert!(i64::from(next.2) + i64::from(next.0 + decoration.0) <= 0);
            assert!(i64::from(next.3) + i64::from(next.1 + decoration.1) <= 760);
            assert_eq!(
                next,
                super::management_geometry(small, scale, decoration, (next.2, next.3))
            );
        }
    }

    #[test]
    fn four_task_panel_can_grow_without_widening_or_overlapping_the_work_area() {
        let large = super::popup_geometry((0, 0, 1920, 1040), 1.0, 840, None);
        assert_eq!((large.0, large.1), (368, 840));
        let retina = super::popup_geometry((0, 0, 3840, 2080), 2.0, 840, None);
        assert_eq!((retina.0, retina.1), (736, 1680));
        let small = super::popup_geometry((0, 0, 1366, 728), 1.5, 840, None);
        assert_eq!(small.0, 480);
        assert!(small.1 <= 671 && small.3 + small.1 as i32 <= 728);
    }

    use super::*;
    use std::time::SystemTime;

    #[test]
    fn popup_fits_work_area_at_scaled_and_negative_monitor_coordinates() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for (aw, ah) in [(1920, 1000), (1366, 728), (1536, 824), (2560, 1400)] {
                let (w, h, x, y) =
                    popup_geometry((-1920, 40, aw, ah), scale, 620, Some((-9999, 9999)));
                assert!(f64::from(w) / scale <= DASHBOARD_WIDTH + 1.0);
                assert!(f64::from(h) <= f64::from(ah) * 0.92 + 1.0);
                assert!(x >= -1920 && y >= 40);
                assert!(x + w as i32 <= -1920 + aw as i32 && y + h as i32 <= 40 + ah as i32);
            }
        }
        let large = popup_geometry((0, 0, 2560, 1400), 1.0, 620, None);
        assert_eq!((large.0, large.1), (368, 620));
        let laptop = popup_geometry((0, 0, 1536, 824), 1.5, 620, None);
        assert_eq!(laptop.0, 492);
        assert!(laptop.1 <= 759);
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

    fn snapshot(tasks: Vec<TaskSnapshot>) -> DashboardSnapshot {
        DashboardSnapshot {
            revision: 1,
            tasks,
            hosts: vec![],
            providers: vec![],
        }
    }

    fn pending() -> TaskSnapshot {
        let mut t = task(TaskStatus::WaitingInput);
        t.source = crate::domain::TaskSource::ClaudeCode;
        t.waiting_reason = Some(WaitingReason::Approval);
        t.input_request_id = Some("request-1".into());
        t.input_request_ids = vec!["request-1".into()];
        t
    }

    fn completed(status: TaskStatus) -> TaskSnapshot {
        let mut t = task(status);
        t.unread = true;
        t
    }

    fn health(status: ProviderStatus) -> crate::providers::ProviderHealth {
        crate::providers::ProviderHealth {
            task_observations: Default::default(),
            provider: "fixture-source".into(),
            status,
            last_success: Some(SystemTime::now()),
            last_attempt: Some(SystemTime::now()),
            message: None,
        }
    }

    #[test]
    fn partial_codex_observation_updates_tray_without_hiding_source_warning() {
        let now = SystemTime::now();
        let mut current = task(TaskStatus::Running);
        current.source = crate::domain::TaskSource::Codex;
        current.source_instance = Some("fixture-source".into());
        let mut provider = health(ProviderStatus::Degraded);
        provider.last_success = None;
        provider.last_attempt = Some(now);
        provider.task_observations.insert(
            current.id.clone(),
            crate::providers::TaskObservation {
                observed_at: crate::providers::resources::epoch_ms(now),
                state: crate::providers::task_observation_state(&current),
            },
        );
        let mut state = snapshot(vec![current]);
        state.providers.push(provider);
        let shown = aggregate(&state);
        assert!(shown.tooltip.contains("进行中 1"));
        assert!(shown.tooltip.contains("来源异常 1"));
        state.tasks[0].turn_key = Some("different-turn".into());
        assert!(aggregate(&state).tooltip.contains("进行中 0"));
    }

    #[test]
    fn tray_basic_states_and_unknown_startup_are_distinct() {
        for (tasks, expected) in [
            (vec![], "idle"),
            (vec![task(TaskStatus::Idle)], "idle"),
            (vec![task(TaskStatus::Running)], "working"),
            (vec![pending()], "needs-input"),
            (vec![completed(TaskStatus::Failed)], "error"),
            (vec![completed(TaskStatus::Succeeded)], "review"),
            (vec![task(TaskStatus::Unknown)], "error"),
            (vec![task(TaskStatus::Disconnected)], "error"),
        ] {
            assert_eq!(aggregate(&snapshot(tasks)).icon, expected);
        }
        let mut first = snapshot(vec![]);
        first.revision = 0;
        assert_eq!(aggregate(&first).icon, "error");
        assert!(aggregate(&first).tooltip.contains("状态待更新 1"));
    }

    #[test]
    fn tray_multi_agent_priority_is_independent_of_observation_order() {
        let mut codex = task(TaskStatus::Running);
        codex.source = crate::domain::TaskSource::Codex;
        let mut hermes = completed(TaskStatus::Succeeded);
        hermes.source = crate::domain::TaskSource::Hermes;
        let failure = completed(TaskStatus::Failed);
        let tasks = [codex.clone(), hermes.clone(), failure.clone(), pending()];
        for shift in 0..tasks.len() {
            let mut mixed = tasks.to_vec();
            mixed.rotate_left(shift);
            let state = aggregate(&snapshot(mixed));
            assert_eq!(state.icon, "needs-input");
            assert!(state.tooltip.contains("待处理 1"));
            assert!(state.tooltip.contains("任务失败 1"));
        }
        assert_eq!(
            aggregate(&snapshot(vec![codex.clone(), hermes.clone(), failure])).icon,
            "error"
        );
        assert_eq!(aggregate(&snapshot(vec![codex, hermes])).icon, "review");
    }

    #[test]
    fn tray_source_failure_and_catchup_never_imply_idle_or_completion() {
        let mut live = task(TaskStatus::Running);
        live.source_instance = Some("fixture-source".into());
        let mut state = snapshot(vec![live]);
        state.providers = vec![health(ProviderStatus::Connected)];
        assert_eq!(aggregate(&state).icon, "working");
        state.providers[0].status = ProviderStatus::Degraded;
        assert_eq!(aggregate(&state).icon, "error");
        assert!(aggregate(&state).tooltip.contains("来源异常 1"));
        assert!(aggregate(&state).tooltip.contains("任务失败 0"));
        state.tasks[0].stale = true;
        state.providers[0].status = ProviderStatus::Connected;
        assert_eq!(aggregate(&state).icon, "error");
        state.tasks[0].stale = false;
        assert_eq!(aggregate(&state).icon, "working");
        state.tasks.clear();
        state.providers[0].status = ProviderStatus::Degraded;
        assert_eq!(aggregate(&state).icon, "error");
        state.providers[0].status = ProviderStatus::Connected;
        state.providers[0].last_success = None;
        assert_eq!(aggregate(&state).icon, "error");
    }

    #[test]
    fn tray_remote_loss_and_stale_requests_are_uncertainty() {
        let mut state = snapshot(vec![task(TaskStatus::Running)]);
        state.hosts.push(crate::providers::HostHealth::disconnected(
            "fixture-host",
            None,
        ));
        assert_eq!(aggregate(&state).icon, "error");
        state.tasks.push(pending());
        assert_eq!(aggregate(&state).icon, "needs-input");
        state.tasks[1].stale = true;
        assert_eq!(aggregate(&state).icon, "error");
        state.tasks[1].stale = false;
        state.tasks[1].source_instance = Some("not-yet-observed".into());
        assert_eq!(aggregate(&state).icon, "error");
    }

    #[test]
    fn tray_turn_end_review_is_not_a_request_and_viewing_does_not_resolve_requests() {
        let mut ended = completed(TaskStatus::WaitingInput);
        ended.waiting_reason = Some(WaitingReason::TurnEnded);
        assert_eq!(aggregate(&snapshot(vec![ended.clone()])).icon, "review");
        ended.unread = false;
        assert_eq!(aggregate(&snapshot(vec![ended])).icon, "idle");
        let mut request = pending();
        for unread in [true, false] {
            request.unread = unread;
            assert_eq!(
                aggregate(&snapshot(vec![request.clone()])).icon,
                "needs-input"
            );
        }
        let mut viewed_failure = completed(TaskStatus::Failed);
        viewed_failure.unread = false;
        viewed_failure.pinned = true;
        assert_eq!(aggregate(&snapshot(vec![viewed_failure])).icon, "idle");
    }

    #[test]
    fn tray_tool_and_model_waits_are_not_human_blockers() {
        for phase in [
            crate::domain::ExecutionPhase::Thinking,
            crate::domain::ExecutionPhase::Tool,
            crate::domain::ExecutionPhase::Responding,
            crate::domain::ExecutionPhase::AwaitingModel,
            crate::domain::ExecutionPhase::AwaitingTool,
        ] {
            let mut live = task(TaskStatus::Running);
            live.phase = Some(phase);
            assert_eq!(aggregate(&snapshot(vec![live])).icon, "working");
        }
        let legacy = task(TaskStatus::WaitingInput);
        assert_eq!(aggregate(&snapshot(vec![legacy])).icon, "error");
    }

    #[test]
    fn tray_inferred_shell_return_is_review_but_never_an_agent_finish() {
        let mut shell = completed(TaskStatus::Idle);
        shell.source = crate::domain::TaskSource::RemoteTmux;
        shell.confidence = crate::domain::DetectionConfidence::Inferred;
        shell.finished_at = Some(SystemTime::now());
        assert_eq!(aggregate(&snapshot(vec![shell.clone()])).icon, "review");
        shell.session_id = Some("agent-session".into());
        assert!(!result_to_review(&shell));
    }

    #[test]
    fn tray_attention_restart_restores_unviewed_result_and_view_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = crate::attention::AttentionTracker::open(path.clone());
        let live = task(TaskStatus::Running);
        let id = live.id.clone();
        assert_eq!(
            aggregate(&snapshot(tracker.reconcile(vec![live.clone()]))).icon,
            "working"
        );
        let mut ended = live;
        ended.status = TaskStatus::Succeeded;
        let display = tracker.reconcile(vec![ended.clone()]);
        assert_eq!(aggregate(&snapshot(display)).icon, "review");
        tracker.flush().unwrap();
        drop(tracker);
        let mut restored = crate::attention::AttentionTracker::open(path);
        let display = restored.reconcile(vec![ended.clone()]);
        assert_eq!(aggregate(&snapshot(display)).icon, "review");
        restored.mark_viewed(&id).unwrap();
        assert_eq!(
            aggregate(&snapshot(restored.reconcile(vec![ended]))).icon,
            "idle"
        );
    }

    #[test]
    fn tray_repeated_updates_and_old_snapshots_do_not_rewrite_icons() {
        let mut applied = AppliedPresentation::default();
        let next = aggregate(&snapshot(vec![task(TaskStatus::Running)]));
        assert!(applied.needs_icon(next.icon));
        // A failed native write leaves the cache untouched and can retry.
        assert!(applied.needs_icon(next.icon));
        applied.revision = 2;
        applied.icon = Some(next.icon);
        applied.tooltip = Some(next.tooltip.clone());
        assert!(!applied.needs_icon(next.icon));
        assert!(!applied.needs_tooltip(&next.tooltip));
        assert!(!applied.accepts(1));
        assert!(applied.accepts(2));
        assert!(applied.accepts(3));
        let more = aggregate(&snapshot(vec![
            task(TaskStatus::Running),
            task(TaskStatus::Running),
        ]));
        assert!(!applied.needs_icon(more.icon));
        assert!(applied.needs_tooltip(&more.tooltip));
        assert!(more.tooltip.encode_utf16().count() < 128);
    }

    #[test]
    fn tray_embedded_assets_decode_with_transparency_and_distinct_palettes() {
        let mut pixels = vec![];
        for state in ["needs-input", "error", "review", "working", "idle"] {
            let image = Image::from_bytes(icon_bytes(state)).unwrap();
            assert_eq!((image.width(), image.height()), (32, 32));
            assert_eq!(image.rgba()[3], 0);
            // TP-03's white T extends left of the stem. The old plain-P export
            // has only tile pixels here, so a mixed runtime asset must fail.
            let t_bar = &image.rgba()[(9 * 32 + 6) * 4..(9 * 32 + 6) * 4 + 4];
            assert!(t_bar[0] > 220 && t_bar[1] > 235 && t_bar[2] > 230);
            assert_eq!(t_bar[3], 255);
            assert!(image.rgba().chunks_exact(4).any(|p| p[3] == 255));
            assert!(!pixels.contains(&image.rgba().to_vec()));
            pixels.push(image.rgba().to_vec());
        }
    }

    #[test]
    fn tray_store_silence_and_recovery_preserve_lifecycle_and_notification_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("notifications.json");
        let mut engine = crate::transition::TransitionEngine::open(ledger.clone());
        let mut store = crate::store::TaskStore::default();
        let now = SystemTime::now();
        let mut live = task(TaskStatus::Running);
        live.source = crate::domain::TaskSource::Codex;
        live.source_instance = Some("fixture-source".into());
        live.turn_key = Some("turn-1".into());
        let mut state = snapshot(store.merge(vec![live.clone()], now));
        state.providers.push(health(ProviderStatus::Connected));
        assert!(engine.apply_snapshots(&state.tasks, &[], now).is_empty());
        assert_eq!(aggregate(&state).icon, "working");
        // No new log line is not an authoritative removal.
        state.tasks = store.merge_instances(vec![], &Default::default(), now);
        assert_eq!(aggregate(&state).icon, "working");
        assert!(engine.apply_snapshots(&state.tasks, &[], now).is_empty());
        // Even an authoritative disappearance of an Agent is stale, not done.
        state.tasks = store.merge_instances(vec![], &["fixture-source".into()].into(), now);
        assert_eq!(aggregate(&state).icon, "error");
        assert!(engine.apply_snapshots(&state.tasks, &[], now).is_empty());
        state.tasks = store.merge(vec![live.clone()], now);
        assert_eq!(aggregate(&state).icon, "working");
        assert!(engine.apply_snapshots(&state.tasks, &[], now).is_empty());
        live.status = TaskStatus::Succeeded;
        live.unread = true;
        state.tasks = store.merge(vec![live], now);
        assert_eq!(engine.apply_snapshots(&state.tasks, &[], now).len(), 1);
        let bytes = std::fs::read(&ledger).unwrap();
        for _ in 0..5 {
            assert_eq!(aggregate(&state).icon, "review");
        }
        assert_eq!(std::fs::read(&ledger).unwrap(), bytes);
        assert!(engine.apply_snapshots(&state.tasks, &[], now).is_empty());
        let mut restarted = crate::transition::TransitionEngine::open(ledger);
        assert!(restarted.apply_snapshots(&state.tasks, &[], now).is_empty());
    }
}
