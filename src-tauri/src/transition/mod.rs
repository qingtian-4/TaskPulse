//! Transition Engine — turns raw snapshot diffs into meaningful events.
//!
//! Rule: never notify on every poll.  Notify only on meaningful transitions:
//!   RUNNING → WAITING_INPUT
//!   RUNNING → SUCCEEDED (exact)
//!   RUNNING → FAILED (exact)
//!   RUNNING → IDLE with explicit tmux shell-return evidence (inferred finish)
//!   CONNECTED → DISCONNECTED (already debounced by the provider)
//!   DISCONNECTED → CONNECTED (recovery)
//!
//! Each (task, transition) is notified at most once; an inferred finish that
//! is later upgraded to an exact SUCCEEDED/FAILED does not double-notify.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use crate::domain::{TaskSnapshot, TaskStatus};
use crate::providers::{HostHealth, HostStatus};

/// A meaningful change worth surfacing (notification + UI emphasis).
#[derive(Debug, Clone, PartialEq)]
pub enum TaskEvent {
    /// Agent finished its turn and needs the user (local agents).
    WaitingInput {
        task: TaskSnapshot,
        reason: Option<String>,
    },
    /// Exact exit code 0.
    Succeeded { task: TaskSnapshot },
    /// Exact non-zero exit code.
    Failed { task: TaskSnapshot },
    /// Workload disappeared / shell returned — no exit code known.
    InferredFinished { task: TaskSnapshot },
    /// Host crossed the failure threshold.
    HostDisconnected {
        host: String,
        last_seen: Option<SystemTime>,
    },
    /// Host is reachable again.
    HostReconnected { host: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
enum EventKind {
    WaitingInput,
    TurnEnded,
    Succeeded,
    Failed,
    InferredFinished,
    HostDisconnected,
    HostReconnected,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TransitionEngine {
    #[serde(default)]
    integration_delivered: HashSet<String>,
    #[serde(skip)]
    integration_ledger_full: bool,
    #[serde(skip)]
    deferred: bool,
    prev: HashMap<String, TaskStatus>,
    prev_turns: HashMap<String, Option<String>>,
    #[serde(default)]
    prev_waits: HashMap<String, (Option<crate::domain::WaitingReason>, Option<String>)>,
    prev_hosts: HashMap<String, HostStatus>,
    notified: HashSet<(String, EventKind)>,
    #[serde(default)]
    input_notifications: HashSet<(String, String)>,
    #[serde(skip)]
    path: Option<std::path::PathBuf>,
}

impl TransitionEngine {
    pub fn forget(&mut self, ids: &HashSet<String>) {
        self.prev.retain(|id, _| !ids.contains(id));
        self.prev_turns.retain(|id, _| !ids.contains(id));
        self.prev_waits.retain(|id, _| !ids.contains(id));
        self.notified.retain(|(id, _)| !ids.contains(id));
        self.input_notifications.retain(|(id, _)| !ids.contains(id));
    }
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(path: std::path::PathBuf) -> Self {
        let mut engine: Self = std::fs::read(&path)
            .ok()
            .and_then(|body| serde_json::from_slice(&body).ok())
            .unwrap_or_default();
        engine.path = Some(path);
        engine
    }

    fn persist(&self) -> bool {
        use std::io::Write;
        let Some(path) = &self.path else {
            return true;
        };
        let result = (|| -> std::io::Result<()> {
            let body = serde_json::to_vec(&serde_json::to_value(self)?)?;
            if std::fs::read(path).ok().as_deref() == Some(body.as_slice()) {
                return Ok(());
            }
            let mut file = tempfile::NamedTempFile::new_in(
                path.parent().unwrap_or(std::path::Path::new(".")),
            )?;
            file.write_all(&body)?;
            file.as_file().sync_all()?;
            file.persist(path).map_err(|e| e.error)?;
            Ok(())
        })();
        if let Err(error) = result {
            log::warn!("notification state not persisted: {error}");
            return false;
        }
        !self.integration_ledger_full
    }

    pub fn flush(&self) -> bool {
        self.persist()
    }

    /// Compare the merged task list + host health against the previous
    /// observation and emit events.  First call establishes the baseline and
    /// emits nothing (no "already finished" notifications for tasks that were
    /// finished before we started watching).
    pub fn apply(
        &mut self,
        tasks: &[TaskSnapshot],
        hosts: &[HostHealth],
        _now: SystemTime,
    ) -> Vec<TaskEvent> {
        let mut events = Vec::new();

        // ---- task transitions ----
        let baseline = self.prev.is_empty();
        let mut current: HashMap<String, TaskStatus> = HashMap::new();
        let mut current_turns: HashMap<String, Option<String>> = HashMap::new();
        let mut current_waits = HashMap::new();

        for task in tasks {
            // Scope filtering normally happens upstream. Keep the lifecycle
            // boundary safe for direct callers and recovered old snapshots.
            if task.internal {
                continue;
            }
            let id = task.id.clone();
            // Legacy pane-scoped exact results move to their own run identity.
            // Match the run key before transferring dedup, never apply A's
            // memory to a new B run in the same pane.
            if let Some(alias) = task.legacy_run_alias().filter(|alias| alias != &id) {
                if self.prev_turns.get(&alias) == Some(&task.turn_key)
                    && !self.prev.contains_key(&id)
                {
                    if let Some(status) = self.prev.remove(&alias) {
                        self.prev.insert(id.clone(), status);
                    }
                    if let Some(turn) = self.prev_turns.remove(&alias) {
                        self.prev_turns.insert(id.clone(), turn);
                    }
                    if let Some(wait) = self.prev_waits.remove(&alias) {
                        self.prev_waits.insert(id.clone(), wait);
                    }
                    self.notified = self
                        .notified
                        .drain()
                        .map(|(key, kind)| (if key == alias { id.clone() } else { key }, kind))
                        .collect();
                    self.input_notifications = self
                        .input_notifications
                        .drain()
                        .map(|(key, request)| {
                            (if key == alias { id.clone() } else { key }, request)
                        })
                        .collect();
                }
            }
            if task.is_agent_session() {
                // v3 could infer an end during thinking. Do not let that false
                // event suppress the next genuine failure/completion.
                self.notified
                    .remove(&(id.clone(), EventKind::InferredFinished));
            }
            let status = task.status;
            current.insert(id.clone(), status);
            current_turns.insert(id.clone(), task.turn_key.clone());
            let wait = (task.waiting_reason, task.input_request_id.clone());
            current_waits.insert(id.clone(), wait.clone());
            let wait_changed = status == TaskStatus::WaitingInput
                && self
                    .prev_waits
                    .get(&id)
                    .is_some_and(|previous| previous != &wait);
            if wait_changed && task.requires_action() {
                self.notified.remove(&(id.clone(), EventKind::WaitingInput));
            }

            // A poll may coalesce several concurrent explicit requests. Dedup
            // every request independently, rather than only the primary gate.
            if !task.stale && task.requires_action() && !task.input_request_ids.is_empty() {
                for request in &task.input_request_ids {
                    let key =
                        serde_json::to_string(&(&task.turn_key, task.waiting_reason, request))
                            .unwrap_or_default();
                    if self.input_notifications.insert((id.clone(), key)) {
                        let mut requested = task.clone();
                        requested.input_request_id = Some(request.clone());
                        events.push(TaskEvent::WaitingInput {
                            task: requested,
                            reason: task.hook_matcher.clone(),
                        });
                    }
                }
                self.notified.insert((id.clone(), EventKind::WaitingInput));
            }

            let prev_status = match self.prev.get(&id).copied() {
                Some(status) => status,
                None if !task.stale
                    && (task.new_execution && task.turn_key.is_some()
                        || task.requires_action() && task.input_request_id.is_some()) =>
                {
                    TaskStatus::Running
                }
                None => continue,
            };
            let turn_changed = task.turn_key.is_some()
                && self
                    .prev_turns
                    .get(&id)
                    .is_some_and(|previous| previous != &task.turn_key);

            // A new run started in the same id — reset notification memory.
            if turn_changed
                || task.turn_key.is_none()
                    && (prev_status.is_terminal() || prev_status == TaskStatus::Idle)
                    && status.is_active()
            {
                reset_task_notifications(&mut self.notified, &id);
            }
            if prev_status == status && !turn_changed && !wait_changed {
                continue;
            }
            if !turn_changed
                && (prev_status.is_terminal() || prev_status == TaskStatus::Idle)
                && status.is_active()
            {
                continue;
            }

            if baseline && !task.unread && !task.requires_action() || task.stale {
                continue;
            }

            match status {
                TaskStatus::WaitingInput => {
                    let duplicate_request=task.input_request_id.as_ref().is_some_and(|request| {
                        let key=serde_json::to_string(&(&task.turn_key,task.waiting_reason,request)).unwrap_or_default();
                        !self.input_notifications.insert((id.clone(),key))
                    });
                    let kind = if task.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded) {
                        EventKind::TurnEnded
                    } else { EventKind::WaitingInput };
                    if !duplicate_request && !already(&mut self.notified, &id, kind) {
                        events.push(TaskEvent::WaitingInput {
                            task: task.clone(),
                            reason: task.hook_matcher.clone(),
                        });
                    }
                }
                TaskStatus::Succeeded => {
                    // If we already announced an inferred finish for this
                    // task, silently upgrade (no double notification).
                    if !already(&mut self.notified, &id, EventKind::InferredFinished)
                        && !already(&mut self.notified, &id, EventKind::Succeeded) {
                            events.push(TaskEvent::Succeeded { task: task.clone() });
                        }
                }
                TaskStatus::Failed => {
                    if !already(&mut self.notified, &id, EventKind::InferredFinished)
                        && !already(&mut self.notified, &id, EventKind::Failed) {
                            events.push(TaskEvent::Failed { task: task.clone() });
                        }
                }
                TaskStatus::Idle
                    // RUNNING → IDLE is an inferred finish (no exit code).
                    // WAITING_INPUT → IDLE and IDLE → IDLE are not events.
                    if task.allows_inferred_finish()
                        && (prev_status == TaskStatus::Running || prev_status == TaskStatus::Unknown)
                        && !already(&mut self.notified, &id, EventKind::InferredFinished) => {
                            events.push(TaskEvent::InferredFinished { task: task.clone() });
                        }
                _ => {}
            }
        }

        // Absence is never completion evidence. Source removal, discovery
        // windows and failed probes all remove rows without ending a turn.

        self.prev = current;
        self.prev_turns = current_turns;
        self.prev_waits = current_waits;

        // ---- host transitions ----
        for host in hosts {
            let id = format!("host:{}", host.host);
            let prev = self.prev_hosts.get(&id).copied();
            if prev.is_some_and(|status| status != host.status) {
                reset_task_notifications(&mut self.notified, &id);
            }
            match host.status {
                HostStatus::Connected => {
                    if prev == Some(HostStatus::Disconnected)
                        && !already(&mut self.notified, &id, EventKind::HostReconnected)
                    {
                        events.push(TaskEvent::HostReconnected {
                            host: host.host.clone(),
                        });
                    }
                    self.prev_hosts.insert(id, HostStatus::Connected);
                }
                HostStatus::Disconnected => {
                    if (prev == Some(HostStatus::Connected) || prev.is_none())
                        && !already(&mut self.notified, &id, EventKind::HostDisconnected)
                    {
                        events.push(TaskEvent::HostDisconnected {
                            host: host.host.clone(),
                            last_seen: host.last_seen,
                        });
                    }
                    self.prev_hosts.insert(id, HostStatus::Disconnected);
                }
            }
        }
        // Hosts no longer reported (provider removed them) are dropped.

        events.retain(|event| {
            let task = match event {
                TaskEvent::WaitingInput { task, .. }
                | TaskEvent::Succeeded { task }
                | TaskEvent::Failed { task }
                | TaskEvent::InferredFinished { task } => task,
                _ => return true,
            };
            if task.source != crate::domain::TaskSource::GenericEvent {
                return true;
            }
            let identity = serde_json::to_string(&(
                &task.id,
                &task.turn_key,
                task.status,
                task.waiting_reason,
                &task.input_request_id,
            ))
            .unwrap_or_default();
            if self.integration_delivered.contains(&identity) {
                return false;
            }
            if self.integration_delivered.len() >= 100_000 {
                self.integration_ledger_full = true;
                return false;
            }
            self.integration_delivered.insert(identity)
        });
        if !self.deferred {
            self.persist();
        }
        events
    }

    pub fn apply_unflushed(
        &mut self,
        tasks: &[TaskSnapshot],
        hosts: &[HostHealth],
        now: SystemTime,
    ) -> Vec<TaskEvent> {
        self.deferred = true;
        let events = self.apply(tasks, hosts, now);
        self.deferred = false;
        events
    }

    /// Generic integration lifecycle is consumed only through its durable
    /// ordered outbox. A latest snapshot must not bypass a failed batch commit
    /// or silently consume A's notification before the outbox is retried.
    pub fn apply_snapshots(
        &mut self,
        tasks: &[TaskSnapshot],
        hosts: &[HostHealth],
        now: SystemTime,
    ) -> Vec<TaskEvent> {
        let before = self.clone();
        let ids: HashSet<_> = tasks
            .iter()
            .filter(|t| t.source == crate::domain::TaskSource::GenericEvent)
            .map(|t| t.id.clone())
            .collect();
        let prev: Vec<_> = self
            .prev
            .iter()
            .filter(|(id, _)| ids.contains(*id))
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let turns: Vec<_> = self
            .prev_turns
            .iter()
            .filter(|(id, _)| ids.contains(*id))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let waits: Vec<_> = self
            .prev_waits
            .iter()
            .filter(|(id, _)| ids.contains(*id))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let snapshots: Vec<_> = tasks
            .iter()
            .filter(|t| t.source != crate::domain::TaskSource::GenericEvent)
            .cloned()
            .collect();
        let events = self.apply_unflushed(&snapshots, hosts, now);
        self.prev.extend(prev);
        self.prev_turns.extend(turns);
        self.prev_waits.extend(waits);
        if !self.flush() {
            *self = before;
            return vec![];
        }
        events
    }

    /// Forget everything (e.g. provider config reloaded).
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.prev.clear();
        self.prev_turns.clear();
        self.prev_waits.clear();
        self.prev_hosts.clear();
        self.notified.clear();
        self.input_notifications.clear();
    }
}

/// Returns true if this (id, kind) was ALREADY notified (dedup).
fn already(notified: &mut HashSet<(String, EventKind)>, id: &str, kind: EventKind) -> bool {
    !notified.insert((id.to_string(), kind))
}

fn reset_task_notifications(notified: &mut HashSet<(String, EventKind)>, id: &str) {
    notified.retain(|(nid, _)| nid != id);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reclassifying_a_previously_observed_service_never_emits_completion() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = TransitionEngine::open(dir.path().join("notifications.json"));
        let mut task = task_turn("codex:child", TaskStatus::Running, "child-turn");
        apply_once(&mut engine, &[task.clone()]);
        task.internal = true;
        task.status = TaskStatus::WaitingInput;
        task.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        assert!(apply_once(&mut engine, &[task.clone()]).is_empty());
        task.status = TaskStatus::Failed;
        assert!(apply_once(&mut engine, &[task]).is_empty());
    }
    #[test]
    fn coalesced_parallel_requests_notify_once_each_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let mut engine = TransitionEngine::open(path.clone());
        let mut task = task_turn("claude:s", TaskStatus::WaitingInput, "turn");
        task.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        task.input_request_id = Some("a".into());
        task.input_request_ids = vec!["a".into(), "b".into()];
        let now = SystemTime::now();
        assert_eq!(engine.apply(&[task.clone()], &[], now).len(), 2);
        assert!(engine.apply(&[task.clone()], &[], now).is_empty());
        task.input_request_ids = vec!["a".into()];
        let mut restarted = TransitionEngine::open(path);
        assert!(restarted.apply(&[task], &[], now).is_empty());
    }
    #[test]
    fn run_identity_migration_preserves_delivered_events_but_not_a_different_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let now = SystemTime::now();
        let mut engine = TransitionEngine::open(path.clone());
        let mut old = task_turn("ssh:host:%3", TaskStatus::Running, "a");
        old.source = crate::domain::TaskSource::RemoteTmux;
        old.confidence = crate::domain::DetectionConfidence::Exact;
        old.host = Some("host".into());
        old.tmux_pane = Some("%3".into());
        engine.apply(&[old.clone()], &[], now);
        old.status = TaskStatus::Succeeded;
        assert_eq!(engine.apply(&[old.clone()], &[], now).len(), 1);
        old.normalize_run_identity();
        let mut restarted = TransitionEngine::open(path);
        assert!(restarted.apply(&[old.clone()], &[], now).is_empty());
        let mut other = old.clone();
        other.id = "ssh:host:run:b".into();
        other.turn_key = Some("b".into());
        other.status = TaskStatus::Running;
        assert!(restarted
            .apply(&[old.clone(), other.clone()], &[], now)
            .is_empty());
        other.status = TaskStatus::Succeeded;
        assert_eq!(restarted.apply(&[old, other], &[], now).len(), 1);
    }
    #[test]
    fn confirmed_requests_dedup_by_request_across_flaps_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let mut engine = TransitionEngine::open(path.clone());
        let mut t = task_turn("codex:s", TaskStatus::Running, "turn");
        assert!(apply_once(&mut engine, &[t.clone()]).is_empty());
        let mut restart = TransitionEngine::open(path);
        assert!(apply_once(&mut restart, &[t.clone()]).is_empty());
        t.status = TaskStatus::WaitingInput;
        t.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        t.input_request_id = Some("q1".into());
        assert_eq!(apply_once(&mut restart, &[t.clone()]).len(), 1);
        t.status = TaskStatus::Running;
        t.waiting_reason = None;
        assert!(apply_once(&mut restart, &[t.clone()]).is_empty());
        t.status = TaskStatus::WaitingInput;
        t.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        assert!(apply_once(&mut restart, &[t.clone()]).is_empty());
        t.input_request_id = Some("q2".into());
        assert_eq!(apply_once(&mut restart, &[t]).len(), 1);
    }
    #[test]
    fn obsolete_approval_checks_do_not_reset_persisted_completion_or_request_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let mut engine = TransitionEngine::open(path.clone());
        let mut t = task_turn("codex:s", TaskStatus::Running, "turn");
        apply_once(&mut engine, &[t.clone()]);
        t.status = TaskStatus::WaitingInput;
        t.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        t.input_request_id = Some("q1".into());
        assert_eq!(apply_once(&mut engine, &[t.clone()]).len(), 1);
        t.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        t.input_request_id = None;
        assert_eq!(apply_once(&mut engine, &[t.clone()]).len(), 1);
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        legacy["approval_checks"] = serde_json::json!([["codex:s", "c1"]]);
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let mut restored = TransitionEngine::open(path.clone());
        assert!(apply_once(&mut restored, &[t.clone()]).is_empty());
        t.status = TaskStatus::Running;
        t.waiting_reason = None;
        assert!(apply_once(&mut restored, &[t.clone()]).is_empty());
        t.status = TaskStatus::WaitingInput;
        t.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        t.input_request_id = Some("q1".into());
        assert!(apply_once(&mut restored, &[t.clone()]).is_empty());
        t.input_request_id = Some("q2".into());
        assert_eq!(apply_once(&mut restored, &[t]).len(), 1);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(saved.get("approval_checks").is_none());
    }

    #[test]
    fn agent_cpu_idle_flapping_never_announces_end_or_resets_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let mut engine = TransitionEngine::open(path.clone());
        let mut snap = task_turn("codex:s", TaskStatus::Running, "same-turn");
        snap.source = crate::domain::TaskSource::Codex;
        snap.session_id = Some("s".into());
        apply_once(&mut engine, &[snap.clone()]);
        for _ in 0..60 {
            for status in [TaskStatus::Idle, TaskStatus::Running] {
                snap.status = status;
                assert!(apply_once(&mut engine, &[snap.clone()]).is_empty());
            }
        }
        snap.status = TaskStatus::WaitingInput;
        snap.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        assert_eq!(apply_once(&mut engine, &[snap.clone()]).len(), 1);
        snap.status = TaskStatus::Idle;
        assert!(apply_once(&mut engine, &[snap.clone()]).is_empty());
        snap.status = TaskStatus::Running;
        assert!(apply_once(&mut engine, &[snap.clone()]).is_empty());
        snap.status = TaskStatus::WaitingInput;
        assert!(apply_once(&mut engine, &[snap.clone()]).is_empty());
        let mut restarted = TransitionEngine::open(path);
        assert!(apply_once(&mut restarted, &[snap.clone()]).is_empty());
        snap.turn_key = Some("new-turn".into());
        assert_eq!(apply_once(&mut restarted, &[snap]).len(), 1);
    }
    use crate::domain::{DetectionConfidence, TaskSource};

    fn task(id: &str, status: TaskStatus) -> TaskSnapshot {
        TaskSnapshot::new(
            id,
            id,
            TaskSource::RemoteTmux,
            status,
            DetectionConfidence::Inferred,
            SystemTime::now(),
        )
    }

    fn task_turn(id: &str, status: TaskStatus, turn: &str) -> TaskSnapshot {
        let mut snapshot = task(id, status);
        snapshot.turn_key = Some(turn.to_string());
        snapshot
    }

    fn apply_once(engine: &mut TransitionEngine, tasks: &[TaskSnapshot]) -> Vec<TaskEvent> {
        engine.apply(tasks, &[], SystemTime::now())
    }

    #[test]
    fn baseline_no_events() {
        let mut engine = TransitionEngine::new();
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        assert!(events.is_empty());
    }

    #[test]
    fn question_then_end_and_restart_are_independent_one_shot_events() {
        use crate::domain::WaitingReason;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notifications.json");
        let mut engine = TransitionEngine::open(path.clone());
        let mut snapshot = task_turn("t", TaskStatus::Running, "turn-1");
        apply_once(&mut engine, &[snapshot.clone()]);
        snapshot.status = TaskStatus::WaitingInput;
        snapshot.waiting_reason = Some(WaitingReason::Question);
        snapshot.input_request_id = Some("q1".into());
        assert_eq!(apply_once(&mut engine, &[snapshot.clone()]).len(), 1);
        assert!(apply_once(&mut engine, &[snapshot.clone()]).is_empty());
        snapshot.input_request_id = Some("q2".into());
        assert_eq!(apply_once(&mut engine, &[snapshot.clone()]).len(), 1);
        snapshot.waiting_reason = Some(WaitingReason::TurnEnded);
        snapshot.input_request_id = None;
        assert_eq!(apply_once(&mut engine, &[snapshot.clone()]).len(), 1);
        let mut restarted = TransitionEngine::open(path);
        assert!(apply_once(&mut restarted, &[snapshot]).is_empty());
    }

    #[test]
    fn a_new_short_turn_completed_before_first_observation_notifies_once() {
        let mut engine = TransitionEngine::new();
        let mut short = task_turn("short", TaskStatus::WaitingInput, "new-turn");
        short.new_execution = true;
        short.unread = true;
        assert_eq!(apply_once(&mut engine, &[short.clone()]).len(), 1);
        assert!(apply_once(&mut engine, &[short]).is_empty());
    }

    #[test]
    fn running_to_waiting_input_emits_once() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::WaitingInput)]);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::WaitingInput { .. }));
        // Same state again → nothing.
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::WaitingInput)]);
        assert!(events.is_empty());
    }

    #[test]
    fn running_to_exact_succeeded() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::Succeeded)]);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::Succeeded { .. }));
    }

    #[test]
    fn running_to_exact_failed() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::Failed)]);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::Failed { .. }));
    }

    #[test]
    fn running_to_idle_is_inferred_finish() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        let mut ended = task("t1", TaskStatus::Idle);
        ended.source = crate::domain::TaskSource::RemoteTmux;
        ended.finished_at = Some(SystemTime::now());
        let events = apply_once(&mut engine, &[ended]);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::InferredFinished { .. }));
    }

    #[test]
    fn idle_to_idle_no_event() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Idle)]);
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::Idle)]);
        assert!(events.is_empty());
    }

    #[test]
    fn inferred_finish_then_exact_no_double_notify() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        let mut ended = task("t1", TaskStatus::Idle);
        ended.source = crate::domain::TaskSource::RemoteTmux;
        ended.finished_at = Some(SystemTime::now());
        let e1 = apply_once(&mut engine, &[ended]);
        assert_eq!(e1.len(), 1);
        // Upgraded to exact — no second notification.
        let e2 = apply_once(&mut engine, &[task("t1", TaskStatus::Succeeded)]);
        assert!(e2.is_empty());
    }

    #[test]
    fn new_run_resets_notification_memory() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        apply_once(&mut engine, &[task("t1", TaskStatus::Idle)]); // inferred finish notified
                                                                  // New run starts in the same pane.
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::WaitingInput)]);
        assert_eq!(events.len(), 1); // notified again for the new run
        assert!(matches!(events[0], TaskEvent::WaitingInput { .. }));
    }

    #[test]
    fn completed_turn_between_polls_notifies_again() {
        let mut engine = TransitionEngine::new();
        apply_once(
            &mut engine,
            &[task_turn("t1", TaskStatus::WaitingInput, "turn-1")],
        );
        // The next turn both started and completed between provider polls.
        let events = apply_once(
            &mut engine,
            &[task_turn("t1", TaskStatus::WaitingInput, "turn-2")],
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::WaitingInput { .. }));
        assert!(apply_once(
            &mut engine,
            &[task_turn("t1", TaskStatus::WaitingInput, "turn-2")]
        )
        .is_empty());
    }

    #[test]
    fn ssh_timeout_does_not_complete_task() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        // SSH failure: the provider preserves the task (stale=true) and the
        // store keeps it RUNNING — the merged list is unchanged, so no
        // finish event can fire.
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        assert!(events.is_empty());
        // Same again — still nothing.
        let events = apply_once(&mut engine, &[task("t1", TaskStatus::Running)]);
        assert!(events.is_empty());
    }

    #[test]
    fn host_disconnect_and_recovery() {
        let mut engine = TransitionEngine::new();
        let now = SystemTime::now();

        let connected = [HostHealth::connected("Server3", now)];
        engine.apply(&[], &connected, now);
        let disconnected = [HostHealth::disconnected("Server3", Some(now))];
        let events = engine.apply(&[], &disconnected, now);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::HostDisconnected { .. }));

        // Still disconnected → no repeat.
        let events = engine.apply(&[], &disconnected, now);
        assert!(events.is_empty());

        // Recovery.
        let events = engine.apply(&[], &connected, now);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TaskEvent::HostReconnected { .. }));
        assert_eq!(engine.apply(&[], &disconnected, now).len(), 1);
    }

    #[test]
    fn task_disappears_without_terminal_snapshot_never_notifies_completion() {
        let mut engine = TransitionEngine::new();
        apply_once(&mut engine, &[task("local-1", TaskStatus::Running)]);
        let events = apply_once(&mut engine, &[]);
        assert!(events.is_empty());
    }
}
