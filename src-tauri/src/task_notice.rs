//! Visible, non-activating task alerts. Queue entries stay separate; closing
//! an alert NEVER marks its task viewed. Lifecycle dedup belongs to TransitionEngine.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::domain::{TaskSnapshot, TaskSource};

const NOTICE_WIDTH: f64 = 320.0;
const NOTICE_HEIGHT: f64 = 132.0;
const NOTICE_MARGIN: f64 = 16.0;

/// Presentation fields are separate: native task titles may themselves contain ` · `.
#[derive(Debug, Clone, Serialize)]
pub struct NoticeContext {
    pub task_name: String,
    pub source: TaskSource,
    pub project: Option<String>,
    pub host: Option<String>,
}

impl From<&TaskSnapshot> for NoticeContext {
    fn from(task: &TaskSnapshot) -> Self {
        Self {
            task_name: task.name.clone(),
            source: task.source,
            project: task.project.clone(),
            host: task.host.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskNotice {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub created_at: u64,
    pub task_id: Option<String>,
    pub test: bool,
    pub context: Option<NoticeContext>,
    pub auto_dismiss_ms: Option<u64>,
}

#[derive(Default, Clone, Serialize)]
pub struct NoticeState {
    pub current: Option<TaskNotice>,
    pub pending: usize,
}

#[derive(Clone)]
struct Entry {
    notice: TaskNotice,
    task: Option<TaskSnapshot>,
}

#[derive(Default)]
struct NoticeQueue {
    entries: VecDeque<Entry>,
}
impl NoticeQueue {
    fn state(&self) -> NoticeState {
        NoticeState {
            current: self.entries.front().map(|e| e.notice.clone()),
            pending: self.entries.len().saturating_sub(1),
        }
    }
    fn push(&mut self, entry: Entry) {
        self.entries.push_back(entry);
    }
    fn dismiss(&mut self, id: u64) -> Option<Entry> {
        // A late timer/click for the previous alert must not dismiss the next.
        if self.entries.front().is_some_and(|e| e.notice.id == id) {
            return self.entries.pop_front();
        }
        None
    }
    fn rotate(&mut self, id: u64) {
        if self.entries.len() > 1 && self.entries.front().is_some_and(|e| e.notice.id == id) {
            let mut entry = self.entries.pop_front().unwrap();
            entry.notice.id = next_id(); // Late callbacks belong to the old presentation.
            self.entries.push_back(entry);
        }
    }
    fn reconcile(&mut self, tasks: &[TaskSnapshot]) -> Vec<Entry> {
        let mut removed = Vec::new();
        self.entries.retain(|entry| {
            let Some(previous) = entry.task.as_ref() else {
                return true;
            };
            if entry.notice.auto_dismiss_ms.is_some() {
                return true;
            }
            let Some(current) = tasks.iter().find(|t| t.id == previous.id && !t.stale) else {
                return true;
            };
            let same_request = previous.requires_action()
                && current.requires_action()
                && current.waiting_reason == previous.waiting_reason
                && (current.input_request_id == previous.input_request_id
                    || previous
                        .input_request_id
                        .as_ref()
                        .is_some_and(|id| current.input_request_ids.contains(id)))
                && current.turn_key == previous.turn_key;
            if !same_request {
                removed.push(entry.clone());
            }
            same_request
        });
        removed
    }
}

static QUEUE: LazyLock<Mutex<NoticeQueue>> = LazyLock::new(|| Mutex::new(NoticeQueue::default()));
static NEXT_ID: LazyLock<AtomicU64> = LazyLock::new(|| {
    AtomicU64::new(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            * 1000,
    )
});
static CREATING: AtomicBool = AtomicBool::new(false);
fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
fn audit(entry: &Entry, stage: &str) {
    crate::notification_log::record(
        entry.task.as_ref(),
        "popup",
        stage,
        &entry.notice.title,
        Some(entry.notice.id),
        entry.notice.test,
    );
}

pub fn enqueue(
    app: &AppHandle,
    title: String,
    body: String,
    task: Option<TaskSnapshot>,
    test: bool,
) {
    if !crate::monitoring::get_monitoring_settings().popup_notifications() {
        return;
    }
    let context = task.as_ref().map(NoticeContext::from);
    let auto_dismiss_ms = if task.as_ref().is_some_and(|t| t.requires_action()) {
        None
    } else {
        Some(15_000)
    };
    // Tests may exercise presentation metadata, but never point to a real task.
    let task = task.filter(|_| !test);
    let notice = TaskNotice {
        id: next_id(),
        title,
        body,
        created_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        task_id: task.as_ref().map(|t| t.id.clone()),
        test,
        context,
        auto_dismiss_ms,
    };
    let entry = Entry { notice, task };
    audit(&entry, "queued");
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push(entry);
    publish(app);
}

#[tauri::command]
pub fn get_task_notice() -> NoticeState {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).state()
}

#[tauri::command]
pub fn dismiss_task_notice(app: AppHandle, id: u64, reason: Option<String>) -> NoticeState {
    let entry = {
        let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        if reason.as_deref() == Some("timeout")
            && queue
                .entries
                .front()
                .is_some_and(|e| e.notice.id == id && e.notice.auto_dismiss_ms.is_none())
        {
            return queue.state();
        }
        queue.dismiss(id)
    };
    if let Some(entry) = entry {
        audit(
            &entry,
            if reason.as_deref() == Some("timeout") {
                "auto_dismissed"
            } else {
                "user_dismissed"
            },
        );
    }
    publish(&app);
    get_task_notice()
}

#[tauri::command]
pub fn next_task_notice(app: AppHandle, id: u64) -> NoticeState {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).rotate(id);
    publish(&app);
    get_task_notice()
}

#[tauri::command]
pub fn ack_task_notice(app: AppHandle, id: u64) -> bool {
    if !app
        .get_webview_window("task-notice")
        .is_some_and(|w| w.is_visible().unwrap_or(false))
    {
        return false;
    }
    let entry = QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .front()
        .filter(|e| e.notice.id == id)
        .cloned();
    if let Some(entry) = entry {
        audit(&entry, "rendered");
        return true;
    }
    false
}

pub fn reconcile(app: &AppHandle, tasks: &[TaskSnapshot]) {
    let removed = QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reconcile(tasks);
    if removed.is_empty() {
        return;
    }
    for entry in removed {
        audit(&entry, "request_resolved");
    }
    publish(app);
}

#[tauri::command]
pub async fn open_task_notice(app: AppHandle, id: u64) -> Result<(), String> {
    let entry = QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .front()
        .filter(|e| e.notice.id == id)
        .cloned()
        .ok_or("这条提醒已关闭")?;
    if let Some(task) = entry.task.clone() {
        let current = crate::monitoring::find_session(&task.id).unwrap_or(task);
        crate::watcher::select_task(&app, current);
    } else {
        crate::tray::show_popup(&app);
    }
    audit(&entry, "details_opened");
    dismiss_task_notice(app, id, Some("details".into()));
    Ok(())
}

pub fn clear(app: &AppHandle) {
    QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .clear();
    publish(app);
}

pub fn remove_task(app: &AppHandle, id: &str) {
    QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .retain(|e| e.notice.task_id.as_deref() != Some(id));
    publish(app);
}

pub fn retain_in_scope(app: &AppHandle) {
    // Resolve eligibility without holding either attention/runtime mutex.
    let entries: Vec<_> = QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .iter()
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }
    let excluded = excluded_notices(&entries, |t| {
        // Entries can predate a newly discovered native origin. Eligibility
        // uses the latest classification, not the old queued task snapshot.
        let current = crate::monitoring::find_session(&t.id).unwrap_or_else(|| t.clone());
        crate::monitoring::allows(&current) && !crate::watcher::is_ignored(&t.id)
    });
    // Polling must not re-publish unchanged presentations/reset UI timers.
    if excluded.is_empty() && crate::monitoring::get_monitoring_settings().popup_notifications() {
        return;
    }
    QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .retain(|e| !excluded.contains(&e.notice.id));
    publish(app);
}

fn excluded_notices(
    entries: &[Entry],
    mut eligible: impl FnMut(&TaskSnapshot) -> bool,
) -> std::collections::HashSet<u64> {
    entries
        .iter()
        .filter(|entry| entry.task.as_ref().is_some_and(|t| !eligible(t)))
        .map(|entry| entry.notice.id)
        .collect()
}

fn publish(app: &AppHandle) {
    if !crate::monitoring::get_monitoring_settings().popup_notifications() {
        QUEUE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .clear();
    }
    let state = get_task_notice();
    let _ = app.emit_to("task-notice", "task-notice-updated", &state);
    if state.current.is_none() {
        if let Some(win) = app.get_webview_window("task-notice") {
            let _ = win.hide();
        }
        return;
    }
    if let Some(win) = app.get_webview_window("task-notice") {
        // The window is non-focusable; never activate the user's current editor.
        if !win.is_visible().unwrap_or(false) {
            if let Err(error) = win.show() {
                audit_current("failed");
                crate::notifier::report_notification_error(
                    app,
                    format!("任务弹窗显示失败：{error}"),
                );
                return;
            }
        }
        audit_current("window_visible");
        return;
    }
    if CREATING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let result =
            WebviewWindowBuilder::new(&app, "task-notice", WebviewUrl::App("notice/".into()))
                .title("TaskPulse · 任务提醒")
                .transparent(true)
                .background_color(tauri::webview::Color(0, 0, 0, 0))
                .inner_size(NOTICE_WIDTH, NOTICE_HEIGHT)
                .decorations(false)
                // Native Windows shadows add a white frame to undecorated windows.
                // Match the dashboard's transparent, CSS-rounded dark surface instead.
                .shadow(false)
                .resizable(false)
                .skip_taskbar(true)
                .always_on_top(true)
                .focused(false)
                .focusable(false)
                .visible(false)
                .build();
        match result {
            Ok(win) => {
                if let Ok(Some(monitor)) = app.primary_monitor() {
                    let area = monitor.work_area();
                    let position = corner_position(
                        area.position.x,
                        area.position.y,
                        area.size.width,
                        area.size.height,
                        monitor.scale_factor(),
                    );
                    let _ = win.set_position(PhysicalPosition::new(position.0, position.1));
                }
                // Creation may overlap dismiss/disable; re-read, don't resurrect it.
                if get_task_notice().current.is_some() {
                    if let Err(error) = win.show() {
                        audit_current("failed");
                        crate::notifier::report_notification_error(
                            &app,
                            format!("任务弹窗显示失败：{error}"),
                        );
                    } else {
                        audit_current("window_visible");
                    }
                }
            }
            Err(error) => {
                audit_current("failed");
                crate::notifier::report_notification_error(
                    &app,
                    format!("任务弹窗创建失败：{error}"),
                );
            }
        }
        CREATING.store(false, Ordering::Release);
    });
}

fn corner_position(x: i32, y: i32, width: u32, height: u32, scale: f64) -> (i32, i32) {
    let left =
        i64::from(x) + i64::from(width) - ((NOTICE_WIDTH + NOTICE_MARGIN) * scale).ceil() as i64;
    let top =
        i64::from(y) + i64::from(height) - ((NOTICE_HEIGHT + NOTICE_MARGIN) * scale).ceil() as i64;
    (left.max(i64::from(x)) as i32, top.max(i64::from(y)) as i32)
}

fn audit_current(stage: &str) {
    let entry = QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .front()
        .cloned();
    if let Some(entry) = entry {
        audit(&entry, stage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: u64) -> Entry {
        Entry {
            notice: TaskNotice {
                id,
                title: "Task ended".into(),
                body: "Native title".into(),
                created_at: 1,
                task_id: Some(format!("task:{id}")),
                test: false,
                context: None,
                auto_dismiss_ms: Some(15_000),
            },
            task: None,
        }
    }
    #[test]
    fn alerts_are_separate_and_old_dismiss_cannot_close_next() {
        let mut queue = NoticeQueue::default();
        for id in 1..=100 {
            queue.push(entry(id));
        }
        assert_eq!(queue.state().current.unwrap().id, 1);
        assert_eq!(queue.state().pending, 99);
        let _ = queue.dismiss(1);
        let _ = queue.dismiss(1);
        assert_eq!(queue.state().current.unwrap().id, 2);
        assert_eq!(queue.state().pending, 98);
        for id in 2..=100 {
            let _ = queue.dismiss(id);
        }
        assert!(queue.state().current.is_none());
    }
    #[test]
    fn sticky_alerts_can_rotate_and_old_presentations_cannot_dismiss_them() {
        let mut queue = NoticeQueue::default();
        let mut sticky = entry(1);
        sticky.notice.auto_dismiss_ms = None;
        queue.push(sticky);
        queue.push(entry(2));
        queue.rotate(1);
        assert_eq!(queue.state().current.unwrap().id, 2);
        assert_eq!(queue.state().pending, 1);
        let _ = queue.dismiss(2);
        assert!(queue.dismiss(1).is_none());
        assert!(queue.state().current.unwrap().auto_dismiss_ms.is_none());
    }
    #[test]
    fn request_resolution_removes_only_its_sticky_notice_and_never_marks_viewed() {
        let mut task = TaskSnapshot::new(
            "codex:s",
            "native",
            TaskSource::Codex,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        task.unread = true;
        task.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        task.input_request_id = Some("q1".into());
        task.turn_key = Some("turn".into());
        let mut sticky = entry(1);
        sticky.notice.auto_dismiss_ms = None;
        sticky.task = Some(task.clone());
        let mut queue = NoticeQueue::default();
        queue.push(sticky);
        queue.push(entry(2));
        assert!(queue.reconcile(&[]).is_empty());
        let mut parallel = task.clone();
        parallel.input_request_id = Some("q2".into());
        parallel.input_request_ids = vec!["q2".into(), "q1".into()];
        assert!(queue.reconcile(&[parallel]).is_empty());
        let mut stale = task.clone();
        stale.stale = true;
        stale.status = crate::domain::TaskStatus::Running;
        stale.waiting_reason = None;
        stale.input_request_id = None;
        assert!(queue.reconcile(&[stale]).is_empty());
        task.status = crate::domain::TaskStatus::Running;
        task.waiting_reason = None;
        task.input_request_id = None;
        assert_eq!(queue.reconcile(&[task.clone()]).len(), 1);
        assert!(task.unread);
        assert_eq!(queue.state().current.unwrap().id, 2);
    }
    #[test]
    fn placement_respects_work_area_dpi_and_negative_monitor_origin() {
        assert_eq!(corner_position(0, 0, 1920, 1040, 1.0), (1584, 892));
        assert_eq!(corner_position(-1920, 0, 1920, 1040, 1.5), (-504, 818));
        assert_eq!(corner_position(0, 0, 300, 130, 1.0), (0, 0));
    }

    #[test]
    fn latest_origin_can_remove_an_old_guardian_notice_without_viewing_the_task() {
        let mut old = TaskSnapshot::new(
            "child",
            "Guardian review",
            TaskSource::Codex,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        old.unread = true;
        let mut latest = old.clone();
        latest.internal = true;
        let mut guardian = entry(1);
        guardian.task = Some(old.clone());
        let mut user = entry(2);
        let mut same_title = old.clone();
        same_title.id = "user".into();
        user.task = Some(same_title);
        let excluded = excluded_notices(&[guardian, user, entry(3)], |task| {
            let current = if task.id == latest.id { &latest } else { task };
            !current.internal
        });
        assert_eq!(excluded, std::collections::HashSet::from([1]));
        assert!(old.unread && latest.unread);
    }

    #[test]
    fn context_keeps_native_title_separate_from_project_and_host() {
        let mut task = TaskSnapshot::new(
            "codex:stable-id",
            "Task · native separator",
            TaskSource::Codex,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Exact,
            SystemTime::now(),
        );
        task.project = Some("Project · with separator".into());
        task.host = Some("Server1".into());
        let context = NoticeContext::from(&task);
        assert_eq!(context.task_name, "Task · native separator");
        assert_eq!(context.project.as_deref(), Some("Project · with separator"));
        assert_eq!(context.host.as_deref(), Some("Server1"));
        assert_eq!(context.source, TaskSource::Codex);
    }
}
