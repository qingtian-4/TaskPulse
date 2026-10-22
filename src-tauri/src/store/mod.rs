//! TaskStore — the single source of truth for what the UI shows.
//!
//! Responsibilities:
//! * Merge provider snapshots by stable task `id`.
//! * Resolve conflicting observations with confidence ordering
//!   (EXACT > HOOK > INFERRED) — a `tp-run` exit code always beats a
//!   process-tree guess for the same pane.
//! * Keep finished tasks in a RECENT list for a TTL (30 min default) instead
//!   of deleting them the moment they finish (AgentTray's wrap behavior).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource};
use crate::providers::sort_tasks;

pub const DEFAULT_RECENT_TTL: Duration = Duration::from_secs(30 * 60);

pub struct TaskStore {
    active: HashMap<String, TaskSnapshot>,
    recent: Vec<TaskSnapshot>,
    recent_ttl: Duration,
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new(DEFAULT_RECENT_TTL)
    }
}

impl TaskStore {
    pub fn retain(&mut self, mut predicate: impl FnMut(&TaskSnapshot) -> bool) {
        self.active.retain(|_, task| predicate(task));
        self.recent.retain(|task| predicate(task));
    }

    pub fn merge_instances(
        &mut self,
        incoming: Vec<TaskSnapshot>,
        authoritative: &HashSet<String>,
        now: SystemTime,
    ) -> Vec<TaskSnapshot> {
        let ids: HashSet<_> = incoming.iter().map(|task| task.id.as_str()).collect();
        let vanished = self
            .active
            .iter()
            .filter(|(id, task)| {
                authoritative.contains(&crate::monitoring::source_id(task))
                    && !ids.contains(id.as_str())
            })
            .map(|(id, _)| id.clone())
            .collect();
        self.reconcile_vanished(vanished, now);
        self.merge(incoming, now)
    }
    pub fn new(recent_ttl: Duration) -> Self {
        Self {
            active: HashMap::new(),
            recent: Vec::new(),
            recent_ttl,
        }
    }

    /// Merge a provider batch and return the full display list
    /// (active + recent, sorted by attention priority).
    pub fn merge(&mut self, incoming: Vec<TaskSnapshot>, now: SystemTime) -> Vec<TaskSnapshot> {
        for snap in incoming {
            let id = snap.id.clone();
            if self
                .active
                .get(&id)
                .or_else(|| self.recent.iter().find(|r| r.id == id))
                .is_some_and(|previous| snap.replays_integration(previous))
            {
                if let Some(previous) = self
                    .active
                    .get_mut(&id)
                    .or_else(|| self.recent.iter_mut().find(|r| r.id == id))
                {
                    previous.merge_integration_health(&snap);
                }
                continue;
            }
            if let Some(existing) = self.active.get_mut(&id) {
                merge_into(existing, snap);
            } else if let Some(pos) = self.recent.iter().position(|r| r.id == id) {
                // The same pane/agent started a NEW run: drop the old RECENT
                // entry and treat this as an active task again.
                self.recent.remove(pos);
                self.active.insert(id, snap);
            } else {
                self.active.insert(id, snap);
            }
        }

        // Promote finished tasks to RECENT.
        let finished: Vec<String> = self
            .active
            .iter()
            .filter(|(_, t)| t.status.is_terminal() || t.finished_at.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            if let Some(mut t) = self.active.remove(&id) {
                t.recent = true;
                if t.finished_at.is_none() {
                    t.finished_at = Some(now);
                }
                self.recent.push(t);
            }
        }

        // Evict RECENT entries past TTL.
        self.recent.retain(|t| {
            t.finished_at
                .map(|f| now.duration_since(f).unwrap_or_default() <= self.recent_ttl)
                .unwrap_or(true)
        });
        self.recent
            .sort_by_key(|t| std::cmp::Reverse(t.finished_at.unwrap_or(now)));

        let mut out: Vec<TaskSnapshot> = self.active.values().cloned().collect();
        out.extend(self.recent.iter().cloned());
        sort_tasks(&mut out);
        out
    }

    /// Reconcile one complete local/remote observation partition.
    ///
    /// A successful provider cycle is authoritative for its partition: tasks
    /// that were previously active but are absent now must not remain as
    /// ghosts forever. A vanished RUNNING task is retained in RECENT as an
    /// inferred finish; waiting/idle tasks simply disappear. Remote SSH
    /// failures are safe because RemoteTmuxProvider re-emits its last tasks
    /// with `stale=true` rather than returning an empty successful snapshot.
    pub fn merge_partition(
        &mut self,
        incoming: Vec<TaskSnapshot>,
        remote: bool,
        now: SystemTime,
    ) -> Vec<TaskSnapshot> {
        let incoming_ids: std::collections::HashSet<&str> =
            incoming.iter().map(|t| t.id.as_str()).collect();
        let vanished: Vec<String> = self
            .active
            .iter()
            .filter(|(id, task)| task.remote == remote && !incoming_ids.contains(id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();

        self.reconcile_vanished(vanished, now);

        self.merge(incoming, now)
    }

    /// Reconcile only the local sources whose provider polls were complete.
    /// Degraded providers may still contribute stale snapshots, but their
    /// missing task IDs are deliberately non-authoritative.
    pub fn merge_sources(
        &mut self,
        incoming: Vec<TaskSnapshot>,
        authoritative_sources: &HashSet<TaskSource>,
        now: SystemTime,
    ) -> Vec<TaskSnapshot> {
        let incoming_ids: HashSet<&str> = incoming.iter().map(|task| task.id.as_str()).collect();
        let vanished: Vec<String> = self
            .active
            .iter()
            .filter(|(id, task)| {
                !task.remote
                    && authoritative_sources.contains(&task.source)
                    && !incoming_ids.contains(id.as_str())
            })
            .map(|(id, _)| id.clone())
            .collect();

        self.reconcile_vanished(vanished, now);
        self.merge(incoming, now)
    }

    fn reconcile_vanished(&mut self, vanished: Vec<String>, _now: SystemTime) {
        for id in vanished {
            let Some(mut task) = self.active.remove(&id) else {
                continue;
            };
            if task.status == crate::domain::TaskStatus::Running && task.is_agent_session() {
                // Missing native metadata or an expired discovery window is
                // not an end event. Keep it uncertain until a real signal.
                task.stale = true;
                self.active.insert(id, task);
                continue;
            }
            if task.status == crate::domain::TaskStatus::Running {
                task.status = crate::domain::TaskStatus::Unknown;
                task.finished_at = None;
                task.exit_code = None;
                task.stale = true;
                self.active.insert(id, task);
            }
        }
    }

    /// Active (non-finished) tasks only.
    #[allow(dead_code)]
    pub fn active(&self) -> Vec<TaskSnapshot> {
        let mut out: Vec<TaskSnapshot> = self
            .active
            .values()
            .filter(|t| !t.status.is_terminal() && t.finished_at.is_none())
            .cloned()
            .collect();
        sort_tasks(&mut out);
        out
    }

    /// RECENT completed tasks.
    #[allow(dead_code)]
    pub fn recent(&self) -> Vec<TaskSnapshot> {
        let mut out = self.recent.clone();
        out.sort_by_key(|t| std::cmp::Reverse(t.finished_at.unwrap_or(SystemTime::UNIX_EPOCH)));
        out
    }

    /// Restore only valid, non-expired recent entries from the on-disk cache.
    /// Active snapshots are intentionally ignored and must be rediscovered by
    /// their providers after startup.
    pub fn restore_recent(&mut self, tasks: Vec<TaskSnapshot>, now: SystemTime) {
        for mut task in tasks {
            let Some(finished_at) = task.finished_at else {
                continue;
            };
            if now.duration_since(finished_at).unwrap_or_default() > self.recent_ttl {
                continue;
            }
            task.recent = true;
            // A restart restores display history, not current source evidence.
            // Otherwise the first merge could revalidate stale attention from
            // the same disk cache before the provider has even responded.
            task.stale = true;
            self.recent.retain(|existing| existing.id != task.id);
            self.recent.push(task);
        }
        self.recent
            .sort_by_key(|task| std::cmp::Reverse(task.finished_at.unwrap_or(now)));
    }
}

/// Merge a new observation into an existing task.  Rules:
/// 1. Higher confidence wins outright (EXACT beats HOOK beats INFERRED).
/// 2. At equal confidence, a fresh observation from the same provider/source
///    wins even when its native activity timestamp did not change. Lifecycle
///    metadata (for example a released Hermes turn lease) can change without
///    appending another message.
/// 3. A stale observation never replaces a non-stale one. Cross-source ties
///    still use the provider timestamp and prefer active state on an exact tie.
fn merge_into(existing: &mut TaskSnapshot, new: TaskSnapshot) {
    if existing.is_agent_session()
        && new.source == existing.source
        && new.source_instance == existing.source_instance
        && new.session_id == existing.session_id
        && new.updated_at < existing.updated_at
    {
        if new.stale {
            existing.stale = true;
        }
        return;
    }
    // Native adapters already reconcile their event/lease/hook evidence. A
    // newer same-session observation must not be frozen behind the previous
    // turn's stronger hook confidence. Cross-provider/process guesses retain
    // confidence ordering (especially exact tmux exit codes).
    let native_successor = existing.source == new.source
        && existing.source_instance == new.source_instance
        && new.session_id.is_some()
        && existing.session_id == new.session_id
        && !new.stale
        && new.updated_at >= existing.updated_at;
    let existing_beats = if native_successor {
        false
    } else {
        match confidence_cmp(existing.confidence, new.confidence) {
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => {
                if existing.stale != new.stale {
                    !existing.stale
                } else if existing.source == new.source {
                    // This is the next poll from the same authority. Accept its
                    // lifecycle state even if updated_at still points at the last
                    // message/tool event.
                    false
                } else {
                    // Conflicting observations from different sources retain the
                    // old safety rule.
                    if new.updated_at > existing.updated_at {
                        false
                    } else if new.updated_at < existing.updated_at {
                        true
                    } else {
                        !new.status.is_active()
                    }
                }
            }
        }
    };

    if existing_beats {
        if new.stale && existing.source == new.source {
            // Preserve the trusted lifecycle, but expose loss of freshness.
            // A failed probe must not leave the UI claiming live coverage.
            existing.stale = true;
        }
        return;
    }

    let was_terminal = existing.status.is_terminal() || existing.finished_at.is_some();
    let new_is_terminal = new.status.is_terminal() || new.finished_at.is_some();

    // Preserve start time and identity metadata across observations.
    let mut merged = new;
    if merged.started_at.is_none() {
        merged.started_at = existing.started_at;
    }
    if was_terminal && !new_is_terminal {
        // A new run started in the same pane/agent.
        merged.finished_at = None;
        merged.recent = false;
    } else {
        merged.finished_at = merged.finished_at.or(existing.finished_at);
    }
    *existing = merged;
}

fn confidence_cmp(a: DetectionConfidence, b: DetectionConfidence) -> std::cmp::Ordering {
    a.cmp(&b)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_old_native_turn_never_replaces_newer_running_observation() {
        let mut current = t0("codex:s", TaskStatus::Running, DetectionConfidence::Hook);
        current.source = TaskSource::Codex;
        current.session_id = Some("s".into());
        current.turn_key = Some("new".into());
        let mut old = current.clone();
        old.updated_at -= Duration::from_secs(10);
        old.turn_key = Some("old".into());
        old.status = TaskStatus::WaitingInput;
        old.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        merge_into(&mut current, old);
        assert_eq!(current.status, TaskStatus::Running);
        assert_eq!(current.turn_key.as_deref(), Some("new"));
    }
    #[test]
    fn native_retry_and_tool_phase_can_replace_prior_hook_evidence() {
        let now = SystemTime::now();
        let mut store = TaskStore::default();
        let mut prior = TaskSnapshot::new(
            "claude:s",
            "title",
            TaskSource::ClaudeCode,
            crate::domain::TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
            now,
        );
        prior.session_id = Some("s".into());
        prior.turn_key = Some("old".into());
        store.merge(vec![prior.clone()], now);
        let mut next = prior;
        next.confidence = DetectionConfidence::Inferred;
        next.turn_key = Some("new".into());
        next.updated_at = now + Duration::from_secs(1);
        next.status = crate::domain::TaskStatus::Running;
        next.phase = Some(crate::domain::ExecutionPhase::Thinking);
        let out = store.merge(vec![next.clone()], next.updated_at);
        assert_eq!(out[0], next);
    }
    #[test]
    fn wsl_authority_cannot_reconcile_native_desktop_tasks() {
        let now = SystemTime::now();
        let mut store = TaskStore::default();
        let mut native = TaskSnapshot::new(
            "native",
            "native",
            TaskSource::DeepSeekHarness,
            crate::domain::TaskStatus::Running,
            DetectionConfidence::Hook,
            now,
        );
        native.source_instance = Some("desktop".into());
        store.merge_instances(vec![native], &HashSet::from(["desktop".into()]), now);
        let out = store.merge_instances(vec![], &HashSet::from(["wsl".into()]), now);
        assert_eq!(out[0].status, crate::domain::TaskStatus::Running);
    }
    use crate::domain::{TaskSource, TaskStatus, TerminalInfo};

    fn t0(id: &str, status: TaskStatus, conf: DetectionConfidence) -> TaskSnapshot {
        TaskSnapshot::new(
            id,
            id,
            TaskSource::RemoteTmux,
            status,
            conf,
            SystemTime::now(),
        )
    }

    fn secs(now: SystemTime, s: u64) -> SystemTime {
        now + Duration::from_secs(s)
    }

    #[test]
    fn exact_beats_inferred_for_same_pane() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();

        // Inferred says IDLE (process tree lost the workload)…
        let inferred_idle = t0(
            "ssh:Server3:%3",
            TaskStatus::Idle,
            DetectionConfidence::Inferred,
        )
        .with_finished_at(now);
        // …but tp-run knows it's still RUNNING.
        let exact_running = t0(
            "ssh:Server3:%3",
            TaskStatus::Running,
            DetectionConfidence::Exact,
        )
        .with_started_at(now);

        let out = store.merge(vec![inferred_idle], now);
        assert_eq!(out.len(), 1);
        let out = store.merge(vec![exact_running], now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, TaskStatus::Running);
        assert_eq!(out[0].confidence, DetectionConfidence::Exact);
        assert!(out[0].finished_at.is_none());
    }

    #[test]
    fn inferred_idle_does_not_false_complete_exact_running() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();

        store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Running,
                DetectionConfidence::Exact,
            )
            .with_started_at(now)],
            now,
        );
        // A stale inferred observation arrives later.
        let out = store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Idle,
                DetectionConfidence::Inferred,
            )
            .with_finished_at(secs(now, 5))],
            secs(now, 5),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, TaskStatus::Running);
    }

    #[test]
    fn same_pane_dedup_single_entry() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let out = store.merge(
            vec![
                t0(
                    "ssh:Server3:%3",
                    TaskStatus::Running,
                    DetectionConfidence::Inferred,
                ),
                t0(
                    "ssh:Server3:%3",
                    TaskStatus::Running,
                    DetectionConfidence::Inferred,
                )
                .with_message("dup"),
            ],
            now,
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn running_to_succeeded_exact_moves_to_recent() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();

        store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Running,
                DetectionConfidence::Exact,
            )
            .with_started_at(now)],
            now,
        );
        let out = store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Succeeded,
                DetectionConfidence::Exact,
            )
            .with_exit_code(0)
            .with_finished_at(secs(now, 10))],
            secs(now, 10),
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].recent);
        assert_eq!(out[0].exit_code, Some(0));
        assert_eq!(out[0].status, TaskStatus::Succeeded);
        assert!(store.active().is_empty());
        assert_eq!(store.recent().len(), 1);
    }

    #[test]
    fn running_to_failed_exact() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Running,
                DetectionConfidence::Exact,
            )],
            now,
        );
        let out = store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Failed,
                DetectionConfidence::Exact,
            )
            .with_exit_code(2)
            .with_finished_at(secs(now, 5))],
            secs(now, 5),
        );
        assert_eq!(out[0].status, TaskStatus::Failed);
        assert_eq!(out[0].exit_code, Some(2));
    }

    #[test]
    fn new_run_in_same_pane_drops_old_recent() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();

        store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Succeeded,
                DetectionConfidence::Exact,
            )
            .with_exit_code(0)
            .with_finished_at(now)],
            now,
        );
        assert_eq!(store.recent().len(), 1);

        // Same pane starts again.
        let out = store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Running,
                DetectionConfidence::Exact,
            )
            .with_started_at(secs(now, 60))],
            secs(now, 60),
        );
        assert_eq!(store.recent().len(), 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, TaskStatus::Running);
        assert!(!out[0].recent);
    }

    #[test]
    fn recent_ttl_eviction() {
        let mut store = TaskStore::new(Duration::from_secs(30));
        let now = SystemTime::now();
        store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Succeeded,
                DetectionConfidence::Exact,
            )
            .with_exit_code(0)
            .with_finished_at(now)],
            now,
        );
        assert_eq!(store.recent().len(), 1);
        // After TTL + epsilon the entry is gone.
        let late = secs(now, 31);
        let out = store.merge(Vec::new(), late);
        assert!(out.is_empty());
        assert!(store.recent().is_empty());
    }

    #[test]
    fn restore_recent_ignores_active_and_expired_snapshots() {
        let mut store = TaskStore::new(Duration::from_secs(30));
        let now = SystemTime::now();
        let active = t0(
            "codex:active",
            TaskStatus::Running,
            DetectionConfidence::Hook,
        );
        let recent = t0(
            "codex:recent",
            TaskStatus::Succeeded,
            DetectionConfidence::Exact,
        )
        .with_finished_at(secs(now, 0));
        let expired = t0(
            "codex:expired",
            TaskStatus::Succeeded,
            DetectionConfidence::Exact,
        )
        .with_finished_at(now - Duration::from_secs(31));

        store.restore_recent(vec![active, recent, expired], now);
        let restored = store.recent();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, "codex:recent");
        assert!(restored[0].recent);
    }

    #[test]
    fn running_kept_when_provider_silent() {
        // An SSH failure means NO snapshots for that host — the store must
        // preserve the running task, not complete it.
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        store.merge(
            vec![t0(
                "ssh:Server3:%3",
                TaskStatus::Running,
                DetectionConfidence::Inferred,
            )
            .with_started_at(now)],
            now,
        );
        // Next poll: provider emits nothing (host unreachable).
        let out = store.merge(Vec::new(), secs(now, 30));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, TaskStatus::Running);
        assert!(store.active().len() == 1);
    }

    #[test]
    fn successful_partition_reconciliation_marks_missing_work_uncertain() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut running = t0(
            "codex:local-1",
            TaskStatus::Running,
            DetectionConfidence::Hook,
        );
        running.remote = false;
        store.merge_partition(vec![running], false, now);

        let out = store.merge_partition(Vec::new(), false, secs(now, 5));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, TaskStatus::Unknown);
        assert_eq!(out[0].confidence, DetectionConfidence::Hook);
        assert!(out[0].stale);
        assert!(out[0].finished_at.is_none());
        assert_eq!(store.active().len(), 1);
    }

    #[test]
    fn successful_partition_reconciliation_drops_vanished_waiting_task() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut waiting = t0(
            "hermes:local-1",
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
        );
        waiting.remote = false;
        store.merge_partition(vec![waiting], false, now);

        let out = store.merge_partition(Vec::new(), false, secs(now, 5));
        assert!(out.is_empty());
        assert!(store.active().is_empty());
    }

    #[test]
    fn partition_reconciliation_preserves_other_partition() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut local = t0(
            "codex:local",
            TaskStatus::Running,
            DetectionConfidence::Hook,
        );
        local.remote = false;
        let mut remote = t0(
            "ssh:Server3:%0",
            TaskStatus::Running,
            DetectionConfidence::Inferred,
        );
        remote.remote = true;
        store.merge(vec![local, remote], now);

        let out = store.merge_partition(Vec::new(), false, secs(now, 5));
        assert!(out.iter().any(|task| task.id == "ssh:Server3:%0"));
    }

    #[test]
    fn source_reconciliation_preserves_degraded_provider_tasks() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut hermes = t0(
            "hermes:local-1",
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
        );
        hermes.source = TaskSource::Hermes;
        store.merge(vec![hermes], now);

        let authoritative = HashSet::from([TaskSource::Codex]);
        let out = store.merge_sources(Vec::new(), &authoritative, secs(now, 5));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "hermes:local-1");
    }

    #[test]
    fn source_reconciliation_removes_missing_task_from_healthy_provider() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut hermes = t0(
            "hermes:local-1",
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
        );
        hermes.source = TaskSource::Hermes;
        store.merge(vec![hermes], now);

        let authoritative = HashSet::from([TaskSource::Hermes]);
        let out = store.merge_sources(Vec::new(), &authoritative, secs(now, 5));
        assert!(out.is_empty());
    }

    #[test]
    fn local_waiting_input_survives_merge() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut waiting = t0(
            "file:codex",
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
        );
        waiting.source = TaskSource::Codex;
        store.merge(vec![waiting], now);
        let out = store.merge(Vec::new(), secs(now, 5));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, TaskStatus::WaitingInput);
    }

    #[test]
    fn same_provider_can_finish_turn_without_activity_timestamp_change() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let observed_at = SystemTime::now();
        let mut running = t0(
            "hermes:session-1",
            TaskStatus::Running,
            DetectionConfidence::Hook,
        );
        running.source = TaskSource::Hermes;
        running.updated_at = observed_at;
        store.merge(vec![running], observed_at);

        // Hermes releases its turn lease after writing the final assistant
        // message, so status changes but last_activity_at remains identical.
        let mut waiting = t0(
            "hermes:session-1",
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
        );
        waiting.source = TaskSource::Hermes;
        waiting.updated_at = observed_at;
        let out = store.merge(vec![waiting], secs(observed_at, 5));

        assert_eq!(out[0].status, TaskStatus::WaitingInput);
        assert_eq!(out[0].updated_at, observed_at);
    }

    #[test]
    fn stale_same_provider_observation_cannot_replace_fresh_state() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut waiting = t0(
            "hermes:session-1",
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
        );
        waiting.source = TaskSource::Hermes;
        store.merge(vec![waiting], now);

        let mut stale_running = t0(
            "hermes:session-1",
            TaskStatus::Running,
            DetectionConfidence::Hook,
        );
        stale_running.source = TaskSource::Hermes;
        stale_running.stale = true;
        let out = store.merge(vec![stale_running], secs(now, 5));

        assert_eq!(out[0].status, TaskStatus::WaitingInput);
        assert!(out[0].stale);
    }

    #[test]
    fn display_sorted_by_attention_priority() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut idle = t0("a", TaskStatus::Idle, DetectionConfidence::Hook);
        idle.source = TaskSource::LocalFile;
        let mut running = t0("b", TaskStatus::Running, DetectionConfidence::Hook);
        running.source = TaskSource::LocalFile;
        let mut waiting = t0("c", TaskStatus::WaitingInput, DetectionConfidence::Hook);
        waiting.source = TaskSource::LocalFile;

        let out = store.merge(vec![idle, running, waiting], now);
        let statuses: Vec<TaskStatus> = out.iter().map(|t| t.status).collect();
        assert_eq!(
            statuses,
            vec![
                TaskStatus::WaitingInput,
                TaskStatus::Running,
                TaskStatus::Idle
            ]
        );
    }

    #[test]
    fn native_task_missing_from_snapshot_is_uncertain_not_completed() {
        let mut store = TaskStore::default();
        let now = SystemTime::now();
        let mut native = t0(
            "codex:native",
            TaskStatus::Running,
            DetectionConfidence::Hook,
        );
        native.source = TaskSource::Codex;
        store.merge(vec![native], now);
        let out = store.merge_sources(vec![], &HashSet::from([TaskSource::Codex]), now);
        assert_eq!(out[0].status, TaskStatus::Running);
        assert!(out[0].stale);
        assert!(out[0].finished_at.is_none());
    }

    #[test]
    fn terminal_info_preserved_from_legacy() {
        let mut store = TaskStore::new(DEFAULT_RECENT_TTL);
        let now = SystemTime::now();
        let mut t = t0("file:agent", TaskStatus::Running, DetectionConfidence::Hook);
        t.source = TaskSource::LocalFile;
        t.terminal = Some(TerminalInfo {
            kind: "windows_native".into(),
            focus_id: "1234".into(),
            outer_id: String::new(),
            label: "Windows Terminal".into(),
            window_title: None,
        });
        t.can_focus = true;
        store.merge(vec![t], now);
        let out = store.merge(Vec::new(), secs(now, 3));
        assert_eq!(out[0].can_focus, true);
        assert_eq!(out[0].terminal.as_ref().unwrap().label, "Windows Terminal");
    }
}
