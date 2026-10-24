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
const MAX_QUEUED_NOTICES: usize = 128;

/// Presentation fields are separate: native task titles may themselves contain ` · `.
#[derive(Debug, Clone, Serialize)]
pub struct NoticeContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    pub task_name: String,
    pub source: TaskSource,
    pub project: Option<String>,
    pub host: Option<String>,
}

impl From<&TaskSnapshot> for NoticeContext {
    fn from(task: &TaskSnapshot) -> Self {
        Self {
            agent_name: task.agent_name.clone(),
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
    fn push(&mut self, entry: Entry) -> Option<Entry> {
        // Results remain unread in AttentionTracker even if their transient
        // presentation is evicted. Never grow the in-memory queue indefinitely.
        let dropped = if self.entries.len() >= MAX_QUEUED_NOTICES {
            let disposable = self
                .entries
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, e)| e.notice.auto_dismiss_ms.is_some())
                .map(|(i, _)| i);
            if let Some(index) = disposable {
                self.entries.remove(index)
            } else {
                // Do not remove the visible blocker or reset its callback ID.
                // Overflowed requests are still available in the durable inbox.
                return Some(entry);
            }
        } else {
            None
        };
        self.entries.push_back(entry);
        dropped
    }
    fn reopen(&mut self, entry: Entry) -> Option<Entry> {
        if let Some(index) = self.entries.iter().position(|old| {
            old.task
                .as_ref()
                .zip(entry.task.as_ref())
                .is_some_and(|(old, new)| {
                    old.id == new.id
                        && old.turn_key == new.turn_key
                        && old.input_request_id == new.input_request_id
                })
        }) {
            self.entries.remove(index);
        }
        // Explicit user interaction can put this presentation first, without
        // emitting a new lifecycle event or changing persisted notification dedup.
        let dropped = self.push(entry.clone());
        if dropped
            .as_ref()
            .is_some_and(|e| e.notice.id == entry.notice.id)
        {
            let displaced = self.entries.pop_back();
            self.entries.push_front(entry);
            return displaced;
        }
        let last = self.entries.pop_back().unwrap();
        self.entries.push_front(last);
        dropped
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
            let Some(current) = tasks.iter().find(|t| t.id == previous.id && !t.stale) else {
                return true;
            };
            if entry.notice.auto_dismiss_ms.is_some() {
                let relevant = current.turn_key == previous.turn_key
                    && current.status != crate::domain::TaskStatus::Running
                    && !current.requires_action();
                if !relevant {
                    removed.push(entry.clone());
                }
                return relevant;
            }
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
    let dropped = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push(entry);
    if let Some(entry) = dropped {
        audit(&entry, "queue_overflow");
    }
    publish(app);
}

#[tauri::command]
pub fn get_task_notice() -> NoticeState {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).state()
}

/// The inbox is reconstructed from durable attention state, not replayed
/// delivery records. A dismissed popup is neither read nor a resolved request.
#[tauri::command]
pub fn get_pending_task_prompts() -> Vec<TaskSnapshot> {
    let settings = crate::monitoring::get_monitoring_settings();
    let health = crate::watcher::get_providers();
    let now = SystemTime::now();
    let mut tasks: Vec<_> = crate::watcher::known_sessions()
        .into_iter()
        .filter(|t| {
            t.requires_action()
                && !t.internal
                && crate::monitoring::allows(t)
                && !crate::watcher::is_ignored(&t.id)
        })
        .map(|mut task| {
            let source = crate::monitoring::source_for(&settings, &task);
            let provider = source.and_then(|s| health.iter().find(|p| p.provider == s.id));
            let interval = source.map_or(3000, |s| s.poll_interval_ms);
            task.stale |= !prompt_source_fresh(provider, now, interval);
            if task.source == TaskSource::GenericEvent {
                task.stale |= task
                    .pending_confirmed_at
                    .is_none_or(|at| crate::integrations::now_ms().saturating_sub(at) > 30_000);
            }
            task
        })
        .collect();
    tasks.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    tasks
}

fn prompt_source_fresh(
    health: Option<&crate::providers::ProviderHealth>,
    now: SystemTime,
    interval: u64,
) -> bool {
    health.is_some_and(|h| {
        h.status == crate::providers::ProviderStatus::Connected
            && h.last_success
                .and_then(|at| now.duration_since(at).ok())
                .is_some_and(|age| {
                    age.as_millis() <= u128::from(interval.saturating_mul(3).max(30_000))
                })
    })
}

fn reopenable(task: &TaskSnapshot) -> bool {
    task.requires_action()
        && !task.stale
        && !task.internal
        && task.confidence != crate::domain::DetectionConfidence::Inferred
        && (task.source != TaskSource::GenericEvent
            || task
                .pending_confirmed_at
                .is_some_and(|at| crate::integrations::now_ms().saturating_sub(at) <= 30_000))
}

#[tauri::command]
pub async fn reopen_task_prompt(
    app: AppHandle,
    id: String,
    request_id: Option<String>,
    turn_key: Option<String>,
) -> Result<NoticeState, String> {
    if !crate::monitoring::get_monitoring_settings().popup_notifications() {
        return Err("请先启用 TaskPulse 弹窗提醒；请求仍可在面板查看".into());
    }
    let mut task = get_pending_task_prompts()
        .into_iter()
        .find(|t| t.id == id)
        .ok_or("该请求已处理或不在监控范围")?;
    if turn_key.is_some() && turn_key != task.turn_key {
        return Err("该消息所属轮次已变化，请查看当前任务".into());
    }
    if let Some(request) = request_id {
        if !task.input_request_ids.contains(&request)
            && task.input_request_id.as_ref() != Some(&request)
        {
            return Err("该消息的请求已处理，请查看当前任务".into());
        }
        task.input_request_id = Some(request);
    }
    if !reopenable(&task) {
        return Err("来源尚未确认最新状态，请刷新或打开 Agent 核对；不会重发过期请求".into());
    }
    let notice = TaskNotice {
        id: next_id(),
        title: if task.waiting_reason == Some(crate::domain::WaitingReason::QuotaExhausted) {
            "TaskPulse · 需处理：额度已用完".into()
        } else if task.waiting_reason == Some(crate::domain::WaitingReason::Approval) {
            "TaskPulse · 需处理：需要授权".into()
        } else if task.waiting_reason == Some(crate::domain::WaitingReason::Question) {
            "TaskPulse · 需处理：等待输入".into()
        } else {
            "TaskPulse · 需处理：需要你确认".into()
        },
        body: "未解决请求 · 手动重新打开，请在原 Agent 中处理".into(),
        created_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        task_id: Some(task.id.clone()),
        test: false,
        context: Some(NoticeContext::from(&task)),
        auto_dismiss_ms: None,
    };
    let entry = Entry {
        notice,
        task: Some(task),
    };
    audit(&entry, "reopened");
    let dropped = QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reopen(entry);
    if let Some(entry) = dropped {
        audit(&entry, "queue_overflow");
    }
    publish(&app);
    Ok(get_task_notice())
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
        let current = crate::monitoring::find_session(&task.id).unwrap_or_else(|| task.clone());
        if current.turn_key != task.turn_key {
            return Err(
                "这条提醒对应旧轮次；请从任务面板查看当前执行，不会将旧操作作用于新轮次。".into(),
            );
        }
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
        let builder =
            WebviewWindowBuilder::new(&app, "task-notice", WebviewUrl::App("notice/".into()));
        let builder = if std::env::var_os("TASKPULSE_HOME").is_some() {
            builder.data_directory(
                crate::providers::remote_tmux::config::config_dir().join("webview2"),
            )
        } else {
            builder
        };
        let result = builder
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
    #[test]
    fn lifecycle_queued_old_result_is_withdrawn_when_new_turn_is_running() {
        let mut queue = NoticeQueue::default();
        let mut task = TaskSnapshot::new(
            "codex:s",
            "native",
            TaskSource::Codex,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        task.turn_key = Some("old".into());
        task.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        let mut e = entry(1);
        e.task = Some(task.clone());
        e.notice.auto_dismiss_ms = Some(15000);
        queue.push(e);
        task.status = crate::domain::TaskStatus::Running;
        task.turn_key = Some("new".into());
        task.waiting_reason = None;
        assert_eq!(queue.reconcile(&[task]).len(), 1);
        assert!(queue.state().current.is_none());
    }
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
    fn unresolved_requests_are_recoverable_from_attention_after_viewing_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("attention.json");
        let mut tracker = crate::attention::AttentionTracker::open(path.clone());
        let mut task = TaskSnapshot::new(
            "session",
            "native",
            TaskSource::Codex,
            crate::domain::TaskStatus::Running,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        task.turn_key = Some("turn".into());
        tracker.reconcile(vec![task.clone()]);
        task.status = crate::domain::TaskStatus::WaitingInput;
        task.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        task.input_request_id = Some("q2".into());
        task.input_request_ids = vec!["q1".into(), "q2".into()];
        tracker.reconcile(vec![task]);
        tracker.mark_viewed("session").unwrap();
        drop(tracker);
        let restored = crate::attention::AttentionTracker::open(path).known();
        assert_eq!(restored.len(), 1);
        assert!(restored[0].requires_action());
        assert_eq!(restored[0].input_request_ids, vec!["q1", "q2"]);
        assert!(!restored[0].unread); // Viewed and resolved are distinct facts.
        assert!(!prompt_source_fresh(None, SystemTime::now(), 3000));
    }
    #[test]
    fn restored_prompts_require_a_fresh_success_of_their_own_source() {
        use crate::providers::{ProviderHealth, ProviderStatus};
        let now = SystemTime::now();
        let mut health = ProviderHealth {
            provider: "source".into(),
            status: ProviderStatus::Connected,
            last_success: Some(now),
            last_attempt: Some(now),
            message: None,
        };
        assert!(prompt_source_fresh(Some(&health), now, 3000));
        health.status = ProviderStatus::Degraded;
        assert!(!prompt_source_fresh(Some(&health), now, 3000));
        health.status = ProviderStatus::Connected;
        health.last_success = Some(now - std::time::Duration::from_secs(31));
        assert!(!prompt_source_fresh(Some(&health), now, 3000));
        assert!(prompt_source_fresh(Some(&health), now, 20_000));
        health.last_success = None;
        assert!(!prompt_source_fresh(Some(&health), now, 3000));
        assert!(!prompt_source_fresh(None, now, 3000));
    }
    #[test]
    fn queue_is_bounded_without_dismissing_the_visible_notice() {
        let mut queue = NoticeQueue::default();
        for id in 1..=1000 {
            let _ = queue.push(entry(id));
        }
        assert_eq!(queue.entries.len(), MAX_QUEUED_NOTICES);
        assert_eq!(queue.state().current.unwrap().id, 1);
        assert_eq!(queue.entries.back().unwrap().notice.id, 1000);
        assert!(queue.dismiss(2).is_none());
    }
    #[test]
    fn all_blocker_overflow_is_recoverable_and_reopen_does_not_acknowledge() {
        let mut queue = NoticeQueue::default();
        for id in 1..=MAX_QUEUED_NOTICES as u64 {
            let mut item = entry(id);
            item.notice.auto_dismiss_ms = None;
            let _ = queue.push(item);
        }
        let mut overflow = entry(1000);
        overflow.notice.auto_dismiss_ms = None;
        assert_eq!(queue.push(overflow.clone()).unwrap().notice.id, 1000);
        let _ = queue.reopen(overflow);
        assert_eq!(queue.entries.len(), MAX_QUEUED_NOTICES);
        assert_eq!(queue.state().current.unwrap().id, 1000);
        assert!(queue.dismiss(1).is_none());
    }
    #[test]
    fn reopen_deduplicates_the_same_request_but_invalidates_old_callbacks() {
        let mut task = TaskSnapshot::new(
            "session",
            "native",
            TaskSource::Codex,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        task.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        task.input_request_id = Some("q1".into());
        task.turn_key = Some("turn".into());
        task.unread = true;
        let mut old = entry(1);
        old.task = Some(task.clone());
        old.notice.auto_dismiss_ms = None;
        let mut new = old.clone();
        new.notice.id = 1000;
        let mut queue = NoticeQueue::default();
        let _ = queue.push(old);
        let _ = queue.push(entry(2));
        let _ = queue.reopen(new);
        assert_eq!(queue.entries.len(), 2);
        assert_eq!(queue.state().current.unwrap().id, 1000);
        assert!(queue.dismiss(1).is_none());
        assert!(task.unread);
        assert!(reopenable(&task));
        task.stale = true;
        assert!(!reopenable(&task));
        task.stale = false;
        task.internal = true;
        assert!(!reopenable(&task));
        task.internal = false;
        task.status = crate::domain::TaskStatus::Running;
        assert!(!reopenable(&task));
        assert!(!prompt_source_fresh(None, SystemTime::now(), 3000));
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

    #[test]
    fn generic_pending_cannot_be_reopened_by_a_fresh_heartbeat() {
        let mut task = TaskSnapshot::new(
            "generic",
            "synthetic",
            TaskSource::GenericEvent,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        task.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        task.input_request_id = Some("r".into());
        task.input_request_ids = vec!["r".into()];
        task.pending_confirmed_at = Some(crate::integrations::now_ms().saturating_sub(60_000));
        assert!(!reopenable(&task));
        task.pending_confirmed_at = Some(crate::integrations::now_ms());
        assert!(reopenable(&task));
        task.confidence = crate::domain::DetectionConfidence::Inferred;
        assert!(!reopenable(&task));
    }
}
