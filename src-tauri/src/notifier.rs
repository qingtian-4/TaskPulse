//! Notification plumbing.
//!
//! The Transition Engine decides WHAT happened; this module decides how it
//! is surfaced: a one-shot system sound and native desktop notification.

use std::sync::{LazyLock, Mutex};

use tauri::{AppHandle, Emitter};

use crate::domain::TaskSnapshot;
use crate::transition::TaskEvent;

/// A concrete notification to surface.
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    WaitingInput {
        task: TaskSnapshot,
    },
    Succeeded {
        task: TaskSnapshot,
    },
    Failed {
        task: TaskSnapshot,
    },
    InferredFinished {
        task: TaskSnapshot,
    },
    HostDisconnected {
        host: String,
        last_seen_label: Option<String>,
    },
    HostReconnected {
        host: String,
    },
    Test,
}

/// Swappable notification backend.
pub trait Notifier: Send + Sync {
    fn notify(&self, event: &NotifyEvent, app: Option<&AppHandle>);
}

/// Plays a platform system sound via shell commands.
pub struct SystemBeepNotifier;

impl Notifier for SystemBeepNotifier {
    fn notify(&self, event: &NotifyEvent, _app: Option<&AppHandle>) {
        // Windows toast controls sound itself; don't play a second alert.
        let settings = crate::monitoring::get_monitoring_settings();
        if !settings.sound || (cfg!(target_os = "windows") && settings.desktop_notifications()) {
            return;
        }
        if let Some(kind) = beep_kind(event) {
            log::info!("notification sound: {kind}");
            play_system_beep();
        }
    }
}

fn beep_kind(event: &NotifyEvent) -> Option<&'static str> {
    match event {
        NotifyEvent::WaitingInput { .. } => Some("waiting_input"),
        NotifyEvent::Succeeded { .. } => Some("succeeded"),
        NotifyEvent::InferredFinished { .. } => Some("inferred_finished"),
        NotifyEvent::Failed { .. } => Some("failed"),
        NotifyEvent::HostDisconnected { .. } => Some("host_disconnected"),
        NotifyEvent::Test => Some("test"),
        _ => None,
    }
}

/// Send on a separate thread, preserving the native API's real error result.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&self, event: &NotifyEvent, app: Option<&AppHandle>) {
        if !crate::monitoring::get_monitoring_settings().desktop_notifications() {
            return;
        }
        let Some(app) = app else {
            log::warn!("DesktopNotifier: no AppHandle, skipping notification");
            return;
        };

        let (title, body) = notification_copy(event);

        let identifier = app.config().identifier.clone();
        let app = app.clone();
        let task = match event {
            NotifyEvent::WaitingInput { task, .. }
            | NotifyEvent::Succeeded { task }
            | NotifyEvent::Failed { task }
            | NotifyEvent::InferredFinished { task } => Some(task.clone()),
            _ => None,
        };
        std::thread::spawn(move || {
            let result = send_actionable_notification(
                &title,
                &body,
                &identifier,
                Some(app.clone()),
                task.clone(),
            );
            crate::notification_log::record(
                task.as_ref(),
                "system",
                if result.is_ok() {
                    "api_accepted"
                } else {
                    "failed"
                },
                &title,
                None,
                false,
            );
            if let Err(error) = result {
                report_notification_error(&app, error);
            }
        });
    }
}

pub struct PopupNotifier;
impl Notifier for PopupNotifier {
    fn notify(&self, event: &NotifyEvent, app: Option<&AppHandle>) {
        if !crate::monitoring::get_monitoring_settings().popup_notifications() {
            return;
        }
        if let Some(app) = app {
            let (title, body) = notification_copy(event);
            let task = match event {
                NotifyEvent::WaitingInput { task, .. }
                | NotifyEvent::Succeeded { task }
                | NotifyEvent::Failed { task }
                | NotifyEvent::InferredFinished { task } => Some(task.clone()),
                _ => None,
            };
            crate::task_notice::enqueue(app, title, body, task, matches!(event, NotifyEvent::Test));
        }
    }
}

fn send_desktop_notification(title: &str, body: &str, identifier: &str) -> Result<(), String> {
    send_actionable_notification(title, body, identifier, None, None)
}

static LAST_NOTIFICATION_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn report_notification_error(app: &AppHandle, error: String) {
    log::error!("Task notification failed; details available in local application state");
    *LAST_NOTIFICATION_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(error.clone());
    let _ = app.emit("notification-error", error);
}

#[tauri::command]
pub fn get_notification_error() -> Option<String> {
    LAST_NOTIFICATION_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[cfg(target_os = "windows")]
fn send_actionable_notification(
    title: &str,
    body: &str,
    identifier: &str,
    app: Option<AppHandle>,
    task: Option<TaskSnapshot>,
) -> Result<(), String> {
    use winrt_notification::{Sound, Toast};
    let id = if cfg!(debug_assertions) {
        Toast::POWERSHELL_APP_ID
    } else {
        identifier
    };
    let mut toast = Toast::new(id).title(title).text1(body).sound(
        crate::monitoring::get_monitoring_settings()
            .sound
            .then_some(Sound::Default),
    );
    if let Some(icon) = notification_icon_path() {
        toast = toast.icon(
            std::path::Path::new(&icon),
            winrt_notification::IconCrop::Square,
            "TaskPulse",
        );
    }
    if let Some(app) = app {
        toast = toast.on_activated(move |_| {
            let handle = app.clone();
            let task = task.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(task) = task {
                    // Metadata may have changed since the toast was sent.
                    let current = crate::monitoring::find_session(&task.id).unwrap_or(task);
                    crate::watcher::select_task(&handle, current);
                } else {
                    crate::tray::show_popup(&handle);
                }
            });
            Ok(())
        });
    }
    toast.show().map_err(|e| e.to_string())
}

#[cfg(not(target_os = "windows"))]
fn send_actionable_notification(
    title: &str,
    body: &str,
    identifier: &str,
    _app: Option<AppHandle>,
    _task: Option<TaskSnapshot>,
) -> Result<(), String> {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body);
    if let Some(icon) = notification_icon_path() {
        notification.icon(&icon);
    }
    #[cfg(target_os = "macos")]
    let _ = notify_rust::set_application(if cfg!(debug_assertions) {
        "com.apple.Terminal"
    } else {
        identifier
    });
    let _ = identifier;
    notification
        .show()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn notification_copy(event: &NotifyEvent) -> (String, String) {
    match event {
        NotifyEvent::WaitingInput { task, .. } => (
            match task.waiting_reason {
                Some(crate::domain::WaitingReason::TurnEnded)
                    if task.outcome.as_deref() == Some("interrupted") =>
                {
                    "TaskPulse · 本轮已中断，非成功"
                }
                Some(crate::domain::WaitingReason::TurnEnded) => {
                    "TaskPulse · 本轮结束，等待后续输入"
                }
                Some(crate::domain::WaitingReason::Approval) => "TaskPulse · 需处理：需要授权",
                Some(crate::domain::WaitingReason::Question) => "TaskPulse · 需处理：等待输入",
                Some(crate::domain::WaitingReason::QuotaExhausted) => {
                    "TaskPulse · 需处理：额度已用完"
                }
                _ => "TaskPulse · 等待输入",
            }
            .to_string(),
            task_notification_label(task),
        ),
        NotifyEvent::Succeeded { task } => (
            "TaskPulse · 任务已完成".to_string(),
            task_notification_label(task),
        ),
        NotifyEvent::Failed { task } => {
            let mut body = task_notification_label(task);
            if let Some(code) = task.exit_code {
                body.push_str(&format!(" · 退出码 {code}"));
            }
            ("TaskPulse · 任务失败，需要处理".to_string(), body)
        }
        NotifyEvent::InferredFinished { task } => (
            "TaskPulse · 任务可能已结束".to_string(),
            format!("{} · 状态来自推断，请确认", task_notification_label(task)),
        ),
        NotifyEvent::HostDisconnected {
            host,
            last_seen_label,
        } => (
            "TaskPulse · 远程主机已断开".to_string(),
            match last_seen_label {
                Some(last_seen) => format!("{host} · 最后在线 {last_seen}"),
                None => host.clone(),
            },
        ),
        NotifyEvent::HostReconnected { host } => {
            ("TaskPulse · 远程主机已恢复".to_string(), host.clone())
        }
        NotifyEvent::Test => (
            "TaskPulse · 通知测试成功".to_string(),
            "任务完成或需要你输入时，将在这里提醒。".to_string(),
        ),
    }
}

fn task_notification_label(task: &TaskSnapshot) -> String {
    let mut context = Vec::with_capacity(4);
    if let Some(host) = task
        .host
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        context.push(host);
    }
    context.push(task.source.label());
    if let Some(project) = task
        .project
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        context.push(project);
    }
    context.push(&task.name);
    context.join(" · ")
}

/// Returns an absolute path to the notification icon, writing it to a cache
/// file on first call (embedded app icon, so dev mode works too).
fn notification_icon_path() -> Option<String> {
    let cache_dir = dirs_next::cache_dir()?.join("taskpulse");
    let icon_path = cache_dir.join("notification-icon.png");
    if !icon_path.exists() {
        std::fs::create_dir_all(&cache_dir).ok()?;
        std::fs::write(&icon_path, include_bytes!("../icons/128x128.png")).ok()?;
    }
    Some(icon_path.to_string_lossy().into_owned())
}

/// Fires configured reminder channels (sound, app popup, system banner).
pub struct CompositeNotifier {
    backends: Vec<Box<dyn Notifier>>,
}

impl CompositeNotifier {
    pub fn new(backends: Vec<Box<dyn Notifier>>) -> Self {
        Self { backends }
    }
}

impl Notifier for CompositeNotifier {
    fn notify(&self, event: &NotifyEvent, app: Option<&AppHandle>) {
        for backend in &self.backends {
            backend.notify(event, app);
        }
    }
}

static NOTIFIER: LazyLock<CompositeNotifier> = LazyLock::new(|| {
    CompositeNotifier::new(vec![
        Box::new(SystemBeepNotifier),
        Box::new(PopupNotifier),
        Box::new(DesktopNotifier),
    ])
});

fn play_system_beep() {
    std::thread::spawn(|| {
        if let Err(e) = platform_beep() {
            log::warn!("Failed to play notification sound: {e}");
        }
    });
}

#[cfg(target_os = "linux")]
fn platform_beep() -> Result<(), String> {
    use std::process::Command;

    if Command::new("canberra-gtk-play")
        .args(["--id", "bell"])
        .output()
        .map_or(false, |o| o.status.success())
    {
        return Ok(());
    }
    if Command::new("paplay")
        .args(["--property", "media.role=event"])
        .arg("/usr/share/sounds/freedesktop/stereo/bell.oga")
        .output()
        .map_or(false, |o| o.status.success())
    {
        return Ok(());
    }
    Command::new("sh")
        .args(["-c", "printf '\\a'"])
        .output()
        .map(|_| ())
        .map_err(|e| format!("beep failed: {e}"))
}

#[cfg(target_os = "macos")]
fn platform_beep() -> Result<(), String> {
    use std::process::Command;
    Command::new("osascript")
        .args(["-e", "beep"])
        .output()
        .map(|_| ())
        .map_err(|e| format!("osascript beep failed: {e}"))
}

#[cfg(target_os = "windows")]
fn platform_beep() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    Command::new("powershell")
        .creation_flags(0x08000000) // CREATE_NO_WINDOW: never flash a console.
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[System.Media.SystemSounds]::Exclamation.Play()",
        ])
        .output()
        .map(|_| ())
        .map_err(|e| format!("powershell beep failed: {e}"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_beep() -> Result<(), String> {
    log::debug!("No platform beep implementation for this OS");
    Ok(())
}

/// Drive one-shot notifications from transition-engine events.
pub fn handle_events(app: &AppHandle, events: &[TaskEvent], _tasks: &[TaskSnapshot]) {
    crate::task_notice::retain_in_scope(app);
    crate::task_notice::reconcile(app, _tasks);
    for event in events {
        let task = match event {
            TaskEvent::WaitingInput { task, .. }
            | TaskEvent::Succeeded { task }
            | TaskEvent::Failed { task }
            | TaskEvent::InferredFinished { task } => Some(task),
            _ => None,
        };
        if task.is_some_and(|task| {
            task.internal
                || crate::monitoring::find_session(&task.id).is_some_and(|latest| latest.internal)
        }) {
            continue;
        }
        // Audit the event category, not arbitrary native titles/prompt fallbacks.
        let title = match event {
            TaskEvent::WaitingInput { task, .. }
                if task.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded) =>
            {
                "本轮结束"
            }
            TaskEvent::WaitingInput { task, .. }
                if task.waiting_reason == Some(crate::domain::WaitingReason::QuotaExhausted) =>
            {
                "额度已用完"
            }
            TaskEvent::WaitingInput { .. } => "等待输入/授权",
            TaskEvent::Succeeded { .. } => "任务已完成",
            TaskEvent::Failed { .. } => "任务失败",
            TaskEvent::InferredFinished { .. } => "非 Agent 命令推断结束",
            TaskEvent::HostDisconnected { .. } => "服务器断连",
            TaskEvent::HostReconnected { .. } => "服务器恢复",
        };
        crate::notification_log::record(task, "lifecycle", "detected", title, None, false);
        if !crate::monitoring::get_monitoring_settings().notifications {
            crate::notification_log::record(task, "all", "suppressed_disabled", title, None, false);
            continue;
        }
        if task.is_some_and(|t| !crate::monitoring::allows(t) || crate::watcher::is_ignored(&t.id))
        {
            crate::notification_log::record(task, "all", "suppressed_scope", title, None, false);
            continue;
        }
        crate::message_center::record(event);
        let notify_event = match event {
            TaskEvent::WaitingInput { task, .. } => {
                NotifyEvent::WaitingInput { task: task.clone() }
            }
            TaskEvent::Succeeded { task } => NotifyEvent::Succeeded { task: task.clone() },
            TaskEvent::Failed { task } => NotifyEvent::Failed { task: task.clone() },
            TaskEvent::InferredFinished { task } => {
                NotifyEvent::InferredFinished { task: task.clone() }
            }
            TaskEvent::HostDisconnected { host, last_seen } => NotifyEvent::HostDisconnected {
                host: host.clone(),
                last_seen_label: last_seen.map(fmt_last_seen),
            },
            TaskEvent::HostReconnected { host } => {
                NotifyEvent::HostReconnected { host: host.clone() }
            }
        };
        NOTIFIER.notify(&notify_event, Some(app));
    }
}

/// Verify configured reminder channels without waiting for a real task.
#[tauri::command]
pub async fn test_notification(app: AppHandle, kind: Option<String>) -> Result<(), String> {
    let settings = crate::monitoring::get_monitoring_settings();
    if !settings.notifications {
        return Err("请先启用任务提醒并保存配置".into());
    }
    let event = test_event(kind.as_deref().unwrap_or("test"))?;
    if settings.popup_notifications() {
        let (title, body) = notification_copy(&event);
        // Explicit test: no fake task enters attention/history or Agent data.
        let task = match &event {
            NotifyEvent::WaitingInput { task, .. }
            | NotifyEvent::Succeeded { task }
            | NotifyEvent::Failed { task }
            | NotifyEvent::InferredFinished { task } => Some(task.clone()),
            _ => None,
        };
        crate::task_notice::enqueue(&app, title, body, task, true);
    }
    SystemBeepNotifier.notify(&NotifyEvent::Test, Some(&app));
    if !settings.desktop_notifications() {
        return Ok(());
    }
    let identifier = app.config().identifier.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (title, body) = notification_copy(&event);
        send_actionable_notification(&title, &body, &identifier, Some(app), None)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn test_event(kind: &str) -> Result<NotifyEvent, String> {
    use crate::domain::{DetectionConfidence, TaskSource, TaskStatus, WaitingReason};
    if kind == "test" {
        return Ok(NotifyEvent::Test);
    }
    let mut task = TaskSnapshot::new(
        "notification-test",
        "任务提醒验收（测试，不是真实任务）",
        TaskSource::LocalFile,
        TaskStatus::WaitingInput,
        DetectionConfidence::Exact,
        std::time::SystemTime::now(),
    );
    match kind {
        "quota_exhausted" => {
            task.waiting_reason = Some(WaitingReason::QuotaExhausted);
            Ok(NotifyEvent::WaitingInput { task })
        }
        "turn_ended" => {
            task.waiting_reason = Some(WaitingReason::TurnEnded);
            Ok(NotifyEvent::WaitingInput { task })
        }
        "approval" => {
            task.waiting_reason = Some(WaitingReason::Approval);
            Ok(NotifyEvent::WaitingInput { task })
        }
        "failed" => {
            task.status = TaskStatus::Failed;
            Ok(NotifyEvent::Failed { task })
        }
        "long_title" => {
            task.name = "长标题排版验收（测试）· Verify multilingual task titles 与很长的任务名称在消息卡片中正确换行，且不挤压操作按钮".into();
            task.source = TaskSource::Codex;
            task.project = Some("长项目名称排版验收（测试）· Native project".into());
            task.host = Some("Test-server".into());
            task.waiting_reason = Some(WaitingReason::TurnEnded);
            Ok(NotifyEvent::WaitingInput { task })
        }
        _ => Err("无效的提醒测试类型".into()),
    }
}

pub fn send_test_notification(identifier: &str) -> Result<(), String> {
    let (title, body) = notification_copy(&NotifyEvent::Test);
    send_desktop_notification(&title, &body, identifier)
}

fn fmt_last_seen(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let ago = (now - secs).max(0);
    if ago < 60 {
        format!("{ago}s ago")
    } else if ago < 3600 {
        format!("{}m ago", ago / 60)
    } else {
        format!("{}h ago", ago / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r04_notification_log_labels_do_not_include_task_reason_or_host_payloads() {
        let secret = "private-notification-sentinel";
        let task = crate::domain::TaskSnapshot::new(
            secret,
            secret,
            crate::domain::TaskSource::Codex,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        let event = NotifyEvent::WaitingInput { task };
        assert_eq!(beep_kind(&event), Some("waiting_input"));
        assert_eq!(
            beep_kind(&NotifyEvent::HostDisconnected {
                host: secret.into(),
                last_seen_label: Some(secret.into())
            }),
            Some("host_disconnected")
        );
    }
    use crate::domain::{DetectionConfidence, TaskSource, TaskStatus};
    use std::time::{Duration, SystemTime};

    #[test]
    fn completion_transition_has_real_title_and_only_notifies_once_per_turn() {
        let mut engine = crate::transition::TransitionEngine::new();
        let mut task = TaskSnapshot::new(
            "codex:stable-id",
            "Native renamed task",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Exact,
            SystemTime::now(),
        );
        task.project = Some("real-project".into());
        task.turn_key = Some("turn-1".into());
        task.unread = true;
        assert!(engine
            .apply(&[task.clone()], &[], SystemTime::now())
            .is_empty());
        task.status = TaskStatus::WaitingInput;
        task.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        let events = engine.apply(&[task.clone()], &[], SystemTime::now());
        assert_eq!(events.len(), 1);
        if let TaskEvent::WaitingInput { task, .. } = &events[0] {
            let (title, body) =
                notification_copy(&NotifyEvent::WaitingInput { task: task.clone() });
            assert_eq!(title, "TaskPulse · 本轮结束，等待后续输入");
            assert_eq!(body, "Codex · real-project · Native renamed task");
            assert_eq!(task.id, "codex:stable-id");
        } else {
            panic!("expected turn completion");
        }
        task.name = "Renamed after completion".into();
        assert!(engine
            .apply(&[task.clone()], &[], SystemTime::now())
            .is_empty());
        task.status = TaskStatus::Running;
        task.turn_key = Some("turn-2".into());
        engine.apply(&[task.clone()], &[], SystemTime::now());
        task.status = TaskStatus::WaitingInput;
        assert_eq!(engine.apply(&[task], &[], SystemTime::now()).len(), 1);
    }

    #[test]
    fn test_alerts_are_explicit_and_invalid_kinds_rejected() {
        for kind in ["turn_ended", "approval", "failed"] {
            let (title, body) = notification_copy(&test_event(kind).unwrap());
            assert!(body.contains("测试，不是真实任务"));
            assert!(title.contains("TaskPulse"));
        }
        if let NotifyEvent::WaitingInput { task, .. } = test_event("long_title").unwrap() {
            assert!(task.name.contains("（测试）"));
            assert!(task.project.unwrap().contains("（测试）"));
        } else {
            panic!("expected long title reminder");
        }
        assert!(test_event("anything").is_err());
        assert!(test_event("approval_check").is_err());
    }

    #[test]
    fn task_label_preserves_source_project_and_native_title() {
        let mut task = TaskSnapshot::new(
            "codex:thread-1",
            "研究 HyperMem 记忆机制",
            TaskSource::Codex,
            TaskStatus::WaitingInput,
            DetectionConfidence::Exact,
            SystemTime::now(),
        );
        task.host = Some("Server3".into());
        task.project = Some("HyperMem".into());
        assert_eq!(
            task_notification_label(&task),
            "Server3 · Codex · HyperMem · 研究 HyperMem 记忆机制"
        );
    }

    #[test]
    fn waiting_notification_does_not_invent_completion() {
        let mut task = TaskSnapshot::new(
            "hermes:session-1",
            "Install image-to-editable-ppt skill",
            TaskSource::Hermes,
            TaskStatus::WaitingInput,
            DetectionConfidence::Exact,
            SystemTime::now(),
        );
        let (title, body) = notification_copy(&NotifyEvent::WaitingInput { task: task.clone() });
        assert_eq!(title, "TaskPulse · 等待输入");
        assert_eq!(body, "Hermes · Install image-to-editable-ppt skill");
        task.waiting_reason = Some(crate::domain::WaitingReason::Question);
        assert_eq!(
            notification_copy(&NotifyEvent::WaitingInput { task: task.clone() }).0,
            "TaskPulse · 需处理：等待输入"
        );
        task.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        assert_eq!(
            notification_copy(&NotifyEvent::WaitingInput { task }).0,
            "TaskPulse · 本轮结束，等待后续输入"
        );
    }

    #[test]
    fn fmt_last_seen_recent() {
        let t = std::time::SystemTime::now() - Duration::from_secs(30);
        assert_eq!(fmt_last_seen(t), "30s ago");
        let t = std::time::SystemTime::now() - Duration::from_secs(420);
        assert_eq!(fmt_last_seen(t), "7m ago");
    }
}
