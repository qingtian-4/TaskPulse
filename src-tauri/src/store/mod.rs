//! TaskStore — the single source of truth for what the UI shows.
//!
//! Responsibilities:
//! * Merge provider snapshots by stable task `id`.
//! * Resolve conflicting observations with confidence ordering
//!   (EXACT > HOOK > INFERRED) — a `tp-run` exit code always beats a
//!   process-tree guess for the same pane.
//! * Keep finished tasks in a RECENT list for a TTL (30 min default) instead
//!   of deleting them the moment they finish (AgentTray's wrap behavior).

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use crate::domain::{DetectionConfidence, TaskSnapshot};
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
            if let Some(existing) = self.active.get_mut(&id) {
                merge_into(existing, snap, now);
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

        for id in vanished {
            let Some(mut task) = self.active.remove(&id) else {
                continue;
            };
            if task.status == crate::domain::TaskStatus::Running {
                task.status = crate::domain::TaskStatus::Idle;
                task.confidence = DetectionConfidence::Inferred;
                task.updated_at = now;
                task.finished_at = Some(now);
                task.exit_code = None;
                task.stale = false;
                task.recent = true;
                self.recent.retain(|recent| recent.id != id);
                self.recent.push(task);
            }
        }

        self.merge(incoming, now)
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
}

/// Merge a new observation into an existing task.  Rules:
/// 1. Higher confidence wins outright (EXACT beats HOOK beats INFERRED).
/// 2. Tie → the more recent observation wins.
/// 3. Tie → prefer an active status over a terminal/idle one (never let a
///    stale observation falsely complete a task).
fn merge_into(existing: &mut TaskSnapshot, new: TaskSnapshot, now: SystemTime) {
    let existing_beats = match confidence_cmp(existing.confidence, new.confidence) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => {
            // Same confidence: prefer the newer update.
            if new.updated_at > existing.updated_at {
                false
            } else if new.updated_at < existing.updated_at {
                true
            } else {
                // Tie on time: prefer active over terminal/idle.
                !new.status.is_active()
            }
        }
    };

    if existing_beats {
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
    merged.updated_at = now;
    *existing = merged;
}

fn confidence_cmp(a: DetectionConfidence, b: DetectionConfidence) -> std::cmp::Ordering {
    a.cmp(&b)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn successful_partition_reconciliation_finishes_vanished_running_task() {
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
        assert_eq!(out[0].status, TaskStatus::Idle);
        assert_eq!(out[0].confidence, DetectionConfidence::Inferred);
        assert!(out[0].recent);
        assert!(out[0].finished_at.is_some());
        assert!(store.active().is_empty());
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
