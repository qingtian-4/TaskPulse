//! Visible, non-activating task alerts. Queue entries stay separate; closing
//! an alert NEVER marks its task viewed. Lifecycle dedup belongs to TransitionEngine.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::domain::{TaskSnapshot, TaskSource};

const NOTICE_WIDTH: f64 = 400.0;
const NOTICE_HEIGHT: f64 = 180.0;
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
    fn dismiss(&mut self, id: u64) {
        // A late timer/click for the previous alert must not dismiss the next.
        if self.entries.front().is_some_and(|e| e.notice.id == id) {
            self.entries.pop_front();
        }
    }
}

static QUEUE: LazyLock<Mutex<NoticeQueue>> = LazyLock::new(|| Mutex::new(NoticeQueue::default()));
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static CREATING: AtomicBool = AtomicBool::new(false);

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
    // Tests may exercise presentation metadata, but never point to a real task.
    let task = task.filter(|_| !test);
    let notice = TaskNotice {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        title,
        body,
        created_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        task_id: task.as_ref().map(|t| t.id.clone()),
        test,
        context,
    };
    QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Entry { notice, task });
    publish(app);
}

#[tauri::command]
pub fn get_task_notice() -> NoticeState {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).state()
}

#[tauri::command]
pub fn dismiss_task_notice(app: AppHandle, id: u64) -> NoticeState {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).dismiss(id);
    publish(&app);
    get_task_notice()
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
    if let Some(task) = entry.task {
        let current = crate::monitoring::find_session(&task.id).unwrap_or(task);
        crate::watcher::select_task(&app, current);
    } else {
        crate::tray::show_popup(&app);
    }
    dismiss_task_notice(app, id);
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
    let excluded: std::collections::HashSet<_> = entries
        .iter()
        .filter(|entry| {
            entry
                .task
                .as_ref()
                .is_some_and(|t| !crate::monitoring::allows(t) || crate::watcher::is_ignored(&t.id))
        })
        .map(|entry| entry.notice.id)
        .collect();
    QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .retain(|e| !excluded.contains(&e.notice.id));
    publish(app);
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
            let _ = win.show();
        }
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
                .background_color(tauri::webview::Color(255, 255, 255, 255))
                .inner_size(NOTICE_WIDTH, NOTICE_HEIGHT)
                .decorations(false)
                .shadow(true)
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
                    let _ = win.show();
                }
            }
            Err(error) => crate::notifier::report_notification_error(
                &app,
                format!("任务弹窗创建失败：{error}"),
            ),
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
        queue.dismiss(1);
        queue.dismiss(1);
        assert_eq!(queue.state().current.unwrap().id, 2);
        assert_eq!(queue.state().pending, 98);
        for id in 2..=100 {
            queue.dismiss(id);
        }
        assert!(queue.state().current.is_none());
    }
    #[test]
    fn placement_respects_work_area_dpi_and_negative_monitor_origin() {
        assert_eq!(corner_position(0, 0, 1920, 1040, 1.0), (1504, 844));
        assert_eq!(corner_position(-1920, 0, 1920, 1040, 1.5), (-624, 746));
        assert_eq!(corner_position(0, 0, 320, 160, 1.0), (0, 0));
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
