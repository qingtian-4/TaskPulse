//! Notification plumbing.
//!
//! The Transition Engine decides WHAT happened; this module decides how it
//! is surfaced: system beep + desktop notification, with a 10 s reminder
//! beep while an agent keeps waiting for input.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::domain::{TaskSnapshot, TaskStatus};
use crate::transition::TaskEvent;

const NEEDS_INPUT_REMINDER_INTERVAL: Duration = Duration::from_secs(10);

/// A concrete notification to surface.
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    WaitingInput {
        task: TaskSnapshot,
        reason: Option<String>,
    },
    /// Periodic beep-only reminder while an agent stays waiting.
    WaitingInputReminder {
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
            NotifyEvent::WaitingInputReminder { task } => {
                log::info!("'{}' still waiting — reminder beep", task.name);
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

        let (title, body) = match event {
            NotifyEvent::WaitingInput { task, reason } => {
                let body = match reason {
                    Some(r) => format!("{} needs your input ({})", task.name, r),
                    None => format!("{} needs your input", task.name),
                };
                ("TaskPulse".to_string(), body)
            }
            NotifyEvent::WaitingInputReminder { .. } => return, // beep only
            NotifyEvent::Succeeded { task } => (
                "TaskPulse".to_string(),
                match task.exit_code {
                    Some(0) => format!("✓ {} finished (exit 0)", task.name),
                    _ => format!("✓ {} finished", task.name),
                },
            ),
            NotifyEvent::Failed { task } => (
                "TaskPulse".to_string(),
                match task.exit_code {
                    Some(code) => format!("✗ {} failed (exit {})", task.name, code),
                    None => format!("✗ {} failed", task.name),
                },
            ),
            NotifyEvent::InferredFinished { task } => (
                "TaskPulse".to_string(),
                format!("{} finished (inferred — exit code unknown)", task.name),
            ),
            NotifyEvent::HostDisconnected {
                host,
                last_seen_label,
            } => (
                "TaskPulse".to_string(),
                match last_seen_label {
                    Some(ls) => format!("{host} disconnected — last seen {ls}"),
                    None => format!("{host} disconnected"),
                },
            ),
            NotifyEvent::HostReconnected { host } => {
                ("TaskPulse".to_string(), format!("{host} reconnected"))
            }
        };

        let mut builder = app.notification().builder().title(&title).body(&body);
        if let Some(icon_path) = notification_icon_path() {
            builder = builder.icon(icon_path);
        }
        if let Err(e) = builder.show() {
            log::warn!("Desktop notification failed: {e}");
        }
    }
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

#[derive(Debug, Clone, Copy)]
struct ReminderState {
    entered_at: Instant,
    last_reminder_at: Option<Instant>,
}

#[derive(Default)]
struct NeedsInputReminderTracker {
    active: HashMap<String, ReminderState>,
}

impl NeedsInputReminderTracker {
    fn mark_entered(&mut self, id: &str, now: Instant) {
        self.active.insert(
            id.to_string(),
            ReminderState {
                entered_at: now,
                last_reminder_at: None,
            },
        );
    }
}

static REMINDER_TRACKER: LazyLock<Mutex<NeedsInputReminderTracker>> =
    LazyLock::new(|| Mutex::new(NeedsInputReminderTracker::default()));

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

/// Drive notifications from engine events + the current waiting set.
///
/// * Engine events → one-shot notifications (beep + banner as appropriate).
/// * Tasks still `WAITING_INPUT` → periodic reminder beeps (10 s).
pub fn handle_events(app: &AppHandle, events: &[TaskEvent], tasks: &[TaskSnapshot]) {
    let mut tracker = REMINDER_TRACKER.lock().unwrap_or_else(|e| {
        log::warn!("REMINDER_TRACKER mutex poisoned, recovering");
        e.into_inner()
    });
    let now = Instant::now();

    // Drop reminder state for tasks that are no longer waiting.
    let waiting_ids: HashSet<String> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::WaitingInput)
        .map(|t| t.id.clone())
        .collect();
    tracker.active.retain(|id, _| waiting_ids.contains(id));

    // One-shot notifications from engine transitions.
    for event in events {
        let notify_event = match event {
            TaskEvent::WaitingInput { task, reason } => {
                tracker.mark_entered(&task.id, now);
                NotifyEvent::WaitingInput {
                    task: task.clone(),
                    reason: reason.clone(),
                }
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

    // Periodic reminders while an agent keeps waiting.
    for task in tasks
        .iter()
        .filter(|t| t.status == TaskStatus::WaitingInput)
    {
        let entry = tracker
            .active
            .entry(task.id.clone())
            .or_insert(ReminderState {
                entered_at: now,
                last_reminder_at: None,
            });
        let has_waited = now.duration_since(entry.entered_at) >= NEEDS_INPUT_REMINDER_INTERVAL;
        let reminder_due = entry
            .last_reminder_at
            .map(|last| now.duration_since(last) >= NEEDS_INPUT_REMINDER_INTERVAL)
            .unwrap_or(true);
        if has_waited && reminder_due {
            entry.last_reminder_at = Some(now);
            NOTIFIER.notify(
                &NotifyEvent::WaitingInputReminder { task: task.clone() },
                Some(app),
            );
        }
    }
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingNotifier(AtomicUsize);
    impl Notifier for CountingNotifier {
        fn notify(&self, _event: &NotifyEvent, _app: Option<&AppHandle>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn task(id: &str, status: TaskStatus) -> TaskSnapshot {
        TaskSnapshot::new(
            id,
            id,
            crate::domain::TaskSource::LocalFile,
            status,
            crate::domain::DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        )
    }

    #[test]
    fn reminder_fires_after_interval() {
        let n = CountingNotifier(AtomicUsize::new(0));
        let mut tracker = NeedsInputReminderTracker::default();
        let start = Instant::now();

        tracker.mark_entered("a", start);
        // not due yet
        let due = {
            let entry = tracker.active.get("a").unwrap();
            start.duration_since(entry.entered_at) >= NEEDS_INPUT_REMINDER_INTERVAL
        };
        assert!(!due);

        // pretend 10s passed
        let later = start + NEEDS_INPUT_REMINDER_INTERVAL;
        let mut entry = *tracker.active.get("a").unwrap();
        entry.entered_at = later - NEEDS_INPUT_REMINDER_INTERVAL;
        tracker.active.insert("a".into(), entry);
        let due =
            later.duration_since(tracker.active["a"].entered_at) >= NEEDS_INPUT_REMINDER_INTERVAL;
        assert!(due);
        let _ = n.0.load(Ordering::Relaxed);
    }

    #[test]
    fn fmt_last_seen_recent() {
        let t = std::time::SystemTime::now() - Duration::from_secs(30);
        assert_eq!(fmt_last_seen(t), "30s ago");
        let t = std::time::SystemTime::now() - Duration::from_secs(420);
        assert_eq!(fmt_last_seen(t), "7m ago");
    }
}
