//! Notification Seen is independent of Task Unread and Request Pending.
//! Only deduplicated explicit lifecycle events enter this bounded journal.
//! Persist hashes/categories, never titles, prompts, tool inputs or replies.
use crate::domain::{DetectionConfidence, TaskSnapshot, WaitingReason};
use crate::transition::TaskEvent;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Emitter};

const LIMIT: usize = 200;
const BYTE_LIMIT: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Approval,
    Question,
    Quota,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Serialize, Deserialize)]
struct Message {
    id: String,
    task_key: String,
    turn_key: String,
    request_key: Option<String>,
    kind: MessageKind,
    at: u64,
    seen: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Persisted {
    version: u8,
    revision: u64,
    entries: VecDeque<Message>,
}
impl Default for Persisted {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            entries: VecDeque::new(),
        }
    }
}

struct MessageStore {
    path: PathBuf,
    state: Persisted,
    blocked: bool,
    dirty: bool,
    error: Option<String>,
}

impl MessageStore {
    fn open(path: PathBuf) -> Self {
        let loaded = (|| {
            if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > BYTE_LIMIT) {
                return Err(crate::state_file::StateError::Invalid);
            }
            crate::state_file::read(&path, 1, false)?
                .map(|(value, _)| {
                    let state: Persisted = serde_json::from_value(value)
                        .map_err(|_| crate::state_file::StateError::Invalid)?;
                    let hash =
                        |key: &str| key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit());
                    let mut ids = std::collections::HashSet::new();
                    if state.entries.len() > LIMIT
                        || state.entries.iter().any(|e| {
                            !hash(&e.id)
                                || !hash(&e.task_key)
                                || !hash(&e.turn_key)
                                || e.request_key.as_deref().is_some_and(|key| !hash(key))
                                || !ids.insert(&e.id)
                        })
                    {
                        return Err(crate::state_file::StateError::Invalid);
                    }
                    Ok(state)
                })
                .transpose()
        })();
        let (state, error) = match loaded {
            Ok(state) => (state.unwrap_or_default(), None),
            Err(error) => (Persisted::default(), Some(error.message().to_owned())),
        };
        Self {
            path,
            state,
            blocked: error.is_some(),
            dirty: false,
            error,
        }
    }

    fn flush(&mut self) -> Result<(), String> {
        if self.blocked {
            return Err(self.error.clone().unwrap());
        }
        if !self.dirty {
            return Ok(());
        }
        let save = (|| -> std::io::Result<()> {
            let bytes = serde_json::to_vec(&self.state)?;
            if bytes.len() as u64 > BYTE_LIMIT {
                return Err(std::io::Error::other("message budget"));
            }
            let parent = self
                .path
                .parent()
                .ok_or_else(|| std::io::Error::other("message directory"))?;
            std::fs::create_dir_all(parent)?;
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            file.persist(&self.path).map_err(|error| error.error)?;
            Ok(())
        })();
        self.error = save
            .err()
            .map(|_| "消息阅读状态未保存；保留内存记录，请检查状态目录".into());
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        self.dirty = false;
        Ok(())
    }

    fn record(&mut self, event: &TaskEvent, at: u64) {
        if self.blocked {
            return;
        }
        let Some((task, kind)) = message_event(event) else {
            return;
        };
        let request = matches!(kind, MessageKind::Approval | MessageKind::Question)
            .then_some(task.input_request_id.as_deref())
            .flatten();
        let task_key = crate::integrations::key(&task.id);
        // Native stable turn identity; command snapshots without turns fall
        // back to execution start/end, never rename or poll time.
        let turn_key = crate::integrations::key(
            &serde_json::to_string(&(
                &task.turn_key,
                task.turn_key
                    .is_none()
                    .then_some((task.started_at, task.finished_at)),
            ))
            .unwrap(),
        );
        let request_key = request.map(crate::integrations::key);
        let id = crate::integrations::key(
            &serde_json::to_string(&(&task_key, &turn_key, &request_key, kind)).unwrap(),
        );
        if self.state.entries.iter().any(|entry| entry.id == id) {
            return;
        }
        self.state.entries.push_back(Message {
            id,
            task_key,
            turn_key,
            request_key,
            kind,
            at,
            seen: false,
        });
        while self.state.entries.len() > LIMIT {
            self.state.entries.pop_front();
        }
        self.state.revision = self.state.revision.saturating_add(1);
        self.dirty = true;
        let _ = self.flush();
    }

    fn mark_seen(&mut self, ids: &[String]) -> Result<(), String> {
        self.flush()?;
        let before = self.state.clone();
        for entry in &mut self.state.entries {
            if ids.contains(&entry.id) && !entry.seen {
                entry.seen = true;
                self.dirty = true;
            }
        }
        if !self.dirty {
            return Ok(());
        }
        self.state.revision = self.state.revision.saturating_add(1);
        if let Err(error) = self.flush() {
            self.state = before;
            self.dirty = false;
            return Err(error);
        }
        Ok(())
    }

    fn view(&self, tasks: &[TaskSnapshot], prompts: &[TaskSnapshot]) -> MessageCenter {
        let tasks: std::collections::HashMap<_, _> = tasks
            .iter()
            .map(|task| (crate::integrations::key(&task.id), task))
            .collect();
        let mut entries = vec![];
        for entry in self.state.entries.iter().rev() {
            let Some(task) = tasks.get(&entry.task_key).copied() else {
                continue;
            };
            let pending = matches!(
                entry.kind,
                MessageKind::Approval | MessageKind::Question | MessageKind::Quota
            );
            let prompt = prompts.iter().find(|prompt| prompt.id == task.id);
            let current_request = prompt.and_then(|task| {
                task.input_request_ids
                    .iter()
                    .chain(task.input_request_id.iter())
                    .find(|id| entry.request_key.as_ref() == Some(&crate::integrations::key(id)))
                    .cloned()
            });
            let same_turn = crate::integrations::key(
                &serde_json::to_string(&(
                    &task.turn_key,
                    task.turn_key
                        .is_none()
                        .then_some((task.started_at, task.finished_at)),
                ))
                .unwrap(),
            ) == entry.turn_key;
            let still_pending = same_turn
                && prompt.is_some_and(|prompt| match entry.kind {
                    MessageKind::Quota => {
                        prompt.waiting_reason == Some(WaitingReason::QuotaExhausted)
                    }
                    _ => current_request.is_some(),
                });
            let state = if !pending {
                "result"
            } else if still_pending {
                if prompt.is_some_and(|p| !p.stale && p.confidence != DetectionConfidence::Inferred)
                {
                    "pending"
                } else {
                    "unconfirmed"
                }
            } else if task.stale {
                "unconfirmed"
            } else {
                "resolved"
            };
            entries.push(MessageView {
                id: entry.id.clone(),
                at: entry.at,
                seen: entry.seen,
                kind: entry.kind,
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                source: task.source,
                agent_name: task.agent_name.clone(),
                project: task.project.clone(),
                state: state.into(),
                request_id: current_request,
                turn_key: task.turn_key.clone(),
                can_reopen: state == "pending",
            });
        }
        MessageCenter {
            revision: self.state.revision,
            unread: entries.iter().filter(|entry| !entry.seen).count(),
            entries,
            error: self.error.clone(),
        }
    }
}

fn message_event(event: &TaskEvent) -> Option<(&TaskSnapshot, MessageKind)> {
    let (task, kind) = match event {
        TaskEvent::WaitingInput { task, .. } => (
            task,
            match task.waiting_reason? {
                WaitingReason::Approval if task.input_request_id.is_some() => MessageKind::Approval,
                WaitingReason::Question if task.input_request_id.is_some() => MessageKind::Question,
                WaitingReason::QuotaExhausted => MessageKind::Quota,
                WaitingReason::TurnEnded if task.outcome.as_deref() == Some("interrupted") => {
                    MessageKind::Interrupted
                }
                WaitingReason::TurnEnded => MessageKind::Completed,
                _ => return None,
            },
        ),
        TaskEvent::Succeeded { task } => (task, MessageKind::Completed),
        TaskEvent::Failed { task } => (task, MessageKind::Failed),
        _ => return None, // Host flaps and inferred shell-return are not inbox messages.
    };
    // Standard Events uses Inferred for producer provenance, but a validated
    // explicit end with a durable epoch/sequence is not an inferred shell end.
    let explicit = task.confidence != DetectionConfidence::Inferred
        || task.source == crate::domain::TaskSource::GenericEvent
            && task.evidence_channel.as_deref() == Some("self_reported")
            && task.integration_epoch.is_some()
            && task.integration_sequence > 0;
    (!task.stale && !task.internal && explicit).then_some((task, kind))
}

#[derive(Clone, Serialize)]
pub struct MessageView {
    id: String,
    at: u64,
    seen: bool,
    kind: MessageKind,
    task_id: String,
    task_name: String,
    source: crate::domain::TaskSource,
    agent_name: Option<String>,
    project: Option<String>,
    state: String,
    request_id: Option<String>,
    can_reopen: bool,
    turn_key: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct MessageCenter {
    revision: u64,
    unread: usize,
    entries: Vec<MessageView>,
    error: Option<String>,
}

static STORE: LazyLock<Mutex<MessageStore>> = LazyLock::new(|| {
    Mutex::new(MessageStore::open(
        crate::providers::remote_tmux::config::config_dir().join("messages.json"),
    ))
});

pub fn record(event: &TaskEvent) {
    STORE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .record(event, crate::integrations::now_ms());
}

#[tauri::command]
pub fn get_message_center() -> MessageCenter {
    // Resolve scope/Attention before taking STORE, so no opposite lock order.
    let tasks: Vec<_> = crate::watcher::known_sessions()
        .into_iter()
        .filter(|task| {
            !task.internal
                && crate::monitoring::allows(task)
                && !crate::watcher::is_ignored(&task.id)
        })
        .collect();
    let prompts = crate::task_notice::get_pending_task_prompts();
    let mut store = STORE.lock().unwrap_or_else(|error| error.into_inner());
    let _ = store.flush();
    store.view(&tasks, &prompts)
}

#[tauri::command]
pub fn mark_messages_seen(app: AppHandle, ids: Vec<String>) -> Result<MessageCenter, String> {
    if ids.len() > LIMIT || ids.iter().any(|id| id.len() != 64) {
        return Err("无效消息标识".into());
    }
    STORE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .mark_seen(&ids)?;
    let center = get_message_center();
    let _ = app.emit("messages-updated", &center);
    Ok(center)
}

pub fn publish(app: &AppHandle) {
    let _ = app.emit("messages-updated", get_message_center());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{TaskSource, TaskStatus};
    fn task(id: &str) -> TaskSnapshot {
        let mut task = TaskSnapshot::new(
            id,
            "private-title",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            std::time::SystemTime::now(),
        );
        task.source_instance = Some("source".into());
        task.turn_key = Some("turn".into());
        task.session_id = Some(id.into());
        task.unread = true;
        task
    }
    fn requested(id: &str, request: &str) -> TaskSnapshot {
        let mut task = task(id);
        task.status = TaskStatus::WaitingInput;
        task.waiting_reason = Some(WaitingReason::Approval);
        task.input_request_id = Some(request.into());
        task.input_request_ids = vec![request.into()];
        task
    }
    #[test]
    fn completed_message_seen_is_durable_and_does_not_view_task_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("messages.json");
        let mut store = MessageStore::open(path.clone());
        let mut task = task("agent-a");
        task.status = TaskStatus::Succeeded;
        let event = TaskEvent::Succeeded { task: task.clone() };
        store.record(&event, 1);
        let center = store.view(std::slice::from_ref(&task), &[]);
        assert_eq!(center.unread, 1);
        store.mark_seen(&[center.entries[0].id.clone()]).unwrap();
        assert!(task.unread);
        assert_eq!(store.view(std::slice::from_ref(&task), &[]).unread, 0);
        let mut restored = MessageStore::open(path);
        restored.record(&event, 2);
        assert_eq!(restored.state.entries.len(), 1);
        assert!(restored.state.entries[0].seen);
    }
    #[test]
    fn pending_parallel_requests_have_separate_read_state_and_match_only_their_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MessageStore::open(dir.path().join("messages.json"));
        let mut task = requested("a", "q1");
        task.input_request_ids.push("q2".into());
        for request in ["q1", "q2"] {
            let mut event_task = task.clone();
            event_task.input_request_id = Some(request.into());
            store.record(
                &TaskEvent::WaitingInput {
                    task: event_task,
                    reason: None,
                },
                1,
            );
        }
        let center = store.view(std::slice::from_ref(&task), std::slice::from_ref(&task));
        assert_eq!(center.unread, 2);
        assert!(center.entries.iter().all(|entry| entry.state == "pending"));
        store
            .mark_seen(
                &center
                    .entries
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert!(task.requires_action() && task.unread);
        task.input_request_ids = vec!["q2".into()];
        task.input_request_id = Some("q2".into());
        let center = store.view(std::slice::from_ref(&task), std::slice::from_ref(&task));
        assert_eq!(center.unread, 0);
        assert_eq!(
            center
                .entries
                .iter()
                .filter(|entry| entry.state == "pending")
                .count(),
            1
        );
        assert_eq!(
            center
                .entries
                .iter()
                .filter(|entry| entry.state == "resolved")
                .count(),
            1
        );
        task.status = TaskStatus::Running;
        task.waiting_reason = None;
        task.input_request_ids.clear();
        task.input_request_id = None;
        assert!(store
            .view(&[task], &[])
            .entries
            .iter()
            .all(|entry| entry.state == "resolved"));
    }
    #[test]
    fn stale_history_is_unconfirmed_not_a_new_message_and_scope_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MessageStore::open(dir.path().join("messages.json"));
        let task = requested("a", "q");
        // Attention recovery alone does not populate or migrate old messages.
        assert!(store
            .view(std::slice::from_ref(&task), std::slice::from_ref(&task))
            .entries
            .is_empty());
        store.record(
            &TaskEvent::WaitingInput {
                task: task.clone(),
                reason: None,
            },
            1,
        );
        let mut stale = task.clone();
        stale.stale = true;
        store.record(
            &TaskEvent::WaitingInput {
                task: stale.clone(),
                reason: None,
            },
            2,
        );
        let center = store.view(&[task], &[stale]);
        assert_eq!(center.entries.len(), 1);
        assert_eq!(center.entries[0].state, "unconfirmed");
        assert!(!center.entries[0].can_reopen);
        assert!(store.view(&[], &[]).entries.is_empty());
    }
    #[test]
    fn event_identity_cannot_cross_agents_turns_or_requests_and_renames_do_not_reset_seen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MessageStore::open(dir.path().join("messages.json"));
        for id in ["a", "b"] {
            store.record(
                &TaskEvent::WaitingInput {
                    task: requested(id, "q"),
                    reason: None,
                },
                1,
            );
        }
        let ids: Vec<_> = store
            .state
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        assert_ne!(ids[0], ids[1]);
        store.mark_seen(&ids).unwrap();
        let mut renamed = requested("a", "q");
        renamed.name = "other private title".into();
        store.record(
            &TaskEvent::WaitingInput {
                task: renamed.clone(),
                reason: None,
            },
            2,
        );
        assert_eq!(store.state.entries.len(), 2);
        assert!(store.state.entries.iter().all(|entry| entry.seen));
        renamed.turn_key = Some("next".into());
        store.record(
            &TaskEvent::WaitingInput {
                task: renamed,
                reason: None,
            },
            3,
        );
        assert_eq!(store.state.entries.len(), 3);
        assert!(!store.state.entries[2].seen);
    }
    #[test]
    fn all_seen_only_acknowledges_the_rendered_snapshot_not_a_racing_new_event() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MessageStore::open(dir.path().join("messages.json"));
        store.record(
            &TaskEvent::WaitingInput {
                task: requested("a", "q1"),
                reason: None,
            },
            1,
        );
        let rendered = vec![store.state.entries[0].id.clone()];
        store.record(
            &TaskEvent::WaitingInput {
                task: requested("a", "q2"),
                reason: None,
            },
            2,
        );
        store.mark_seen(&rendered).unwrap();
        assert!(store.state.entries[0].seen);
        assert!(!store.state.entries[1].seen);
    }
    #[test]
    fn bounded_store_contains_no_payloads_and_ignores_inferred_internal_and_health_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("messages.json");
        let mut store = MessageStore::open(path.clone());
        for n in 0..(LIMIT + 30) {
            store.record(
                &TaskEvent::WaitingInput {
                    task: requested(&format!("private-id-{n}"), "q"),
                    reason: Some("private-reason".into()),
                },
                n as u64,
            );
        }
        assert_eq!(store.state.entries.len(), LIMIT);
        assert!(std::fs::metadata(&path).unwrap().len() <= BYTE_LIMIT);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("private"));
        let mut task = requested("a", "q");
        task.internal = true;
        store.record(
            &TaskEvent::WaitingInput {
                task: task.clone(),
                reason: None,
            },
            1,
        );
        task.internal = false;
        task.confidence = DetectionConfidence::Inferred;
        store.record(&TaskEvent::Failed { task: task.clone() }, 1);
        store.record(&TaskEvent::InferredFinished { task }, 1);
        store.record(
            &TaskEvent::HostReconnected {
                host: "private-host".into(),
            },
            1,
        );
        assert_eq!(store.state.entries.len(), LIMIT);
    }
    #[test]
    fn corrupt_future_and_oversized_read_state_is_preserved_and_write_failure_never_fakes_seen() {
        for bytes in [
            b"bad".to_vec(),
            b"{\"version\":99}".to_vec(),
            vec![b' '; BYTE_LIMIT as usize + 1],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("messages.json");
            std::fs::write(&path, &bytes).unwrap();
            let mut store = MessageStore::open(path.clone());
            assert!(store.error.is_some());
            assert!(store.mark_seen(&[]).is_err());
            store.record(
                &TaskEvent::WaitingInput {
                    task: requested("a", "q"),
                    reason: None,
                },
                1,
            );
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("messages.json");
        let mut store = MessageStore::open(path.clone());
        store.record(
            &TaskEvent::WaitingInput {
                task: requested("a", "q"),
                reason: None,
            },
            1,
        );
        let id = store.state.entries[0].id.clone();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(store.mark_seen(&[id]).is_err());
        assert!(!store.state.entries[0].seen);
        assert!(path.is_dir());
    }
    #[test]
    fn transition_replay_and_internal_subthreads_cannot_resurrect_seen_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let mut engine = crate::transition::TransitionEngine::open(path.clone());
        let mut task = task("codex:stable");
        engine.apply_snapshots(&[task.clone()], &[], std::time::SystemTime::now());
        task.status = crate::domain::TaskStatus::Succeeded;
        let mut store = MessageStore::open(dir.path().join("messages.json"));
        for event in engine.apply_snapshots(&[task.clone()], &[], std::time::SystemTime::now()) {
            store.record(&event, 1);
        }
        let ids: Vec<_> = store
            .state
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        assert_eq!(ids.len(), 1);
        store.mark_seen(&ids).unwrap();
        let mut restored = crate::transition::TransitionEngine::open(path);
        assert!(restored
            .apply_snapshots(&[task.clone()], &[], std::time::SystemTime::now())
            .is_empty());
        task.internal = true;
        task.id = "codex:child".into();
        task.new_execution = true;
        assert!(restored
            .apply_snapshots(&[task], &[], std::time::SystemTime::now())
            .is_empty());
        assert!(store.state.entries[0].seen);
    }

    #[test]
    fn validated_standard_end_is_an_explicit_fact_not_an_inferred_shell_return() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MessageStore::open(dir.path().join("messages.json"));
        let mut task = task("standard");
        task.source = TaskSource::GenericEvent;
        task.confidence = DetectionConfidence::Inferred;
        task.status = TaskStatus::Succeeded;
        task.evidence_channel = Some("self_reported".into());
        store.record(&TaskEvent::Succeeded { task: task.clone() }, 1);
        assert!(store.state.entries.is_empty());
        task.integration_epoch = Some("durable-epoch".into());
        task.integration_sequence = 2;
        store.record(&TaskEvent::Succeeded { task: task.clone() }, 2);
        assert_eq!(store.state.entries.len(), 1);
        store.record(&TaskEvent::InferredFinished { task }, 3);
        assert_eq!(store.state.entries.len(), 1);
    }
}
