//! Notification plumbing.
//!
//! The Transition Engine decides WHAT happened; this module decides how it
//! is surfaced: a one-shot system sound and native desktop notification.

use std::sync::LazyLock;

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::domain::TaskSnapshot;
use crate::transition::TaskEvent;

/// A concrete notification to surface.
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    WaitingInput {
        task: TaskSnapshot,
        reason: Option<String>,
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
        match event {
            NotifyEvent::WaitingInput { task, reason } => {
                log::info!(
                    "'{}' needs input ({}) — playing alert",
                    task.name,
                    reason.as_deref().unwrap_or("unknown")
                );
                play_system_beep();
            }
            NotifyEvent::Succeeded { task } | NotifyEvent::InferredFinished { task } => {
                log::info!("'{}' finished — playing alert", task.name);
                play_system_beep();
            }
            NotifyEvent::Failed { task } => {
                log::info!("'{}' FAILED — playing alert", task.name);
                play_system_beep();
            }
            NotifyEvent::HostDisconnected { host, .. } => {
                log::info!("Host {host} disconnected — playing alert");
                play_system_beep();
            }
            NotifyEvent::Test => play_system_beep(),
            _ => {}
        }
    }
}

/// Sends native OS desktop notifications via tauri-plugin-notification.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&self, event: &NotifyEvent, app: Option<&AppHandle>) {
        let Some(app) = app else {
            log::warn!("DesktopNotifier: no AppHandle, skipping notification");
            return;
        };

        let (title, body) = notification_copy(event);

        let mut builder = app.notification().builder().title(&title).body(&body);
        if let Some(icon_path) = notification_icon_path() {
            builder = builder.icon(icon_path);
        }
        if let Err(e) = builder.show() {
            log::warn!("Desktop notification failed: {e}");
        }
    }
}

fn notification_copy(event: &NotifyEvent) -> (String, String) {
    match event {
        NotifyEvent::WaitingInput { task, reason } => {
            if let Some(reason) = reason {
                log::debug!("Waiting-input reason for '{}': {reason}", task.name);
            }
            (
                "TaskPulse · 任务已完成，等待输入".to_string(),
                task_notification_label(task),
            )
        }
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

/// Fires multiple notifiers in sequence (beep + desktop banner).
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
    use std::process::Command;
    Command::new("powershell")
        .args(["-c", "[System.Media.SystemSounds]::Exclamation.Play()"])
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
    for event in events {
        let notify_event = match event {
            TaskEvent::WaitingInput { task, reason } => NotifyEvent::WaitingInput {
                task: task.clone(),
                reason: reason.clone(),
            },
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

/// Let users verify Windows notification settings without waiting for a task.
#[tauri::command]
pub fn test_notification(app: AppHandle) {
    NOTIFIER.notify(&NotifyEvent::Test, Some(&app));
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
    use crate::domain::{DetectionConfidence, TaskSource, TaskStatus};
    use std::time::{Duration, SystemTime};

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
    fn waiting_notification_says_task_finished_and_needs_input() {
        let task = TaskSnapshot::new(
            "hermes:session-1",
            "Install image-to-editable-ppt skill",
            TaskSource::Hermes,
            TaskStatus::WaitingInput,
            DetectionConfidence::Exact,
            SystemTime::now(),
        );
        let (title, body) = notification_copy(&NotifyEvent::WaitingInput {
            task,
            reason: Some("provider event".into()),
        });
        assert_eq!(title, "TaskPulse · 任务已完成，等待输入");
        assert_eq!(body, "Hermes · Install image-to-editable-ppt skill");
    }

    #[test]
    fn fmt_last_seen_recent() {
        let t = std::time::SystemTime::now() - Duration::from_secs(30);
        assert_eq!(fmt_last_seen(t), "30s ago");
        let t = std::time::SystemTime::now() - Duration::from_secs(420);
        assert_eq!(fmt_last_seen(t), "7m ago");
    }
}
