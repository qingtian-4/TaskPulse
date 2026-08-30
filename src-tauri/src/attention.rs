//! Persistent dynamic attention list.
//!
//! Providers describe what exists; this module decides what the popup keeps
//! visible. Historical sessions are not imported on startup. A task enters the
//! list when a new turn is observed, remains while running or unread, and can
//! also be kept explicitly pinned. Dismissing a task does not delete its
//! record, because the next distinct turn must be able to re-open it.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::domain::{TaskSnapshot, TaskStatus};
use crate::providers::sort_tasks;

const ATTENTION_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AttentionRecord {
    task: TaskSnapshot,
    unread: bool,
    pinned: bool,
    last_turn_key: Option<String>,
    last_status: TaskStatus,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedAttention {
    version: u8,
    records: HashMap<String, AttentionRecord>,
}

pub struct AttentionTracker {
    path: PathBuf,
    records: HashMap<String, AttentionRecord>,
    last_payload: Option<Vec<u8>>,
}

impl AttentionTracker {
    pub fn open(path: PathBuf) -> Self {
        let payload = std::fs::read(&path).ok();
        let records = payload
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<PersistedAttention>(bytes).ok())
            .filter(|state| state.version == ATTENTION_VERSION)
            .map(|state| state.records)
            .unwrap_or_default();
        Self {
            path,
            records,
            last_payload: payload,
        }
    }

    /// Reconcile current observations into the persistent attention state.
    /// New dormant/history rows are deliberately ignored. Existing records
    /// still receive metadata updates so native renames are synchronized.
    pub fn reconcile(&mut self, observed: Vec<TaskSnapshot>) -> Vec<TaskSnapshot> {
        let observed_ids: HashSet<String> = observed.iter().map(|task| task.id.clone()).collect();

        for mut task in observed {
            let id = task.id.clone();
            let provider_pinned = task.pinned;
            let native_unread = task.native_unread == Some(true);
            let turn_key = effective_turn_key(&task);

            if let Some(record) = self.records.get_mut(&id) {
                let changed_turn = turn_key.is_some() && turn_key != record.last_turn_key;
                let resumed =
                    task.status == TaskStatus::Running && record.last_status != TaskStatus::Running;
                let completed =
                    record.last_status == TaskStatus::Running && task.status != TaskStatus::Running;
                if changed_turn || resumed || completed || native_unread {
                    record.unread = true;
                }
                if provider_pinned {
                    record.pinned = true;
                }

                // Native title/project refreshes update the same record only.
                task.unread = record.unread;
                task.pinned = record.pinned;
                record.task = task;
                if turn_key.is_some() {
                    record.last_turn_key = turn_key;
                }
                record.last_status = record.task.status;
                continue;
            }

            // Startup baseline: only active/native-unread/native-pinned rows
            // enter the attention list. This prevents bulk history import.
            if task.status != TaskStatus::Running && !native_unread && !provider_pinned {
                continue;
            }
            let unread = task.status == TaskStatus::Running || native_unread;
            task.unread = unread;
            task.pinned = provider_pinned;
            self.records.insert(
                id,
                AttentionRecord {
                    last_turn_key: turn_key,
                    last_status: task.status,
                    task,
                    unread,
                    pinned: provider_pinned,
                },
            );
        }

        // A provider can stop returning an old conversation. Keep the last
        // snapshot only when the user still owes it attention or pinned it.
        for (id, record) in &mut self.records {
            if !observed_ids.contains(id)
                && record.task.status == TaskStatus::Running
                && (record.unread || record.pinned)
            {
                record.task.stale = true;
                record.task.status = TaskStatus::Idle;
                record.task.finished_at = Some(SystemTime::now());
                record.task.updated_at = SystemTime::now();
            }
        }

        self.save_best_effort();
        self.display()
    }

    pub fn mark_viewed(&mut self, id: &str) -> Result<Vec<TaskSnapshot>, String> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.unread = false;
        record.task.unread = false;
        self.save_best_effort();
        Ok(self.display())
    }

    pub fn hide(&mut self, id: &str) -> Result<Vec<TaskSnapshot>, String> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.unread = false;
        record.pinned = false;
        record.task.unread = false;
        record.task.pinned = false;
        self.save_best_effort();
        Ok(self.display())
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<Vec<TaskSnapshot>, String> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.pinned = pinned;
        record.task.pinned = pinned;
        self.save_best_effort();
        Ok(self.display())
    }

    pub fn display(&self) -> Vec<TaskSnapshot> {
        let mut tasks: Vec<TaskSnapshot> = self
            .records
            .values()
            .filter(|record| {
                record.task.status == TaskStatus::Running || record.unread || record.pinned
            })
            .map(|record| {
                let mut task = record.task.clone();
                task.unread = record.unread;
                task.pinned = record.pinned;
                task
            })
            .collect();
        sort_tasks(&mut tasks);
        tasks
    }

    pub fn retain<F>(&mut self, mut predicate: F)
    where
        F: FnMut(&TaskSnapshot) -> bool,
    {
        self.records.retain(|_, record| predicate(&record.task));
        self.save_best_effort();
    }

    fn save_best_effort(&mut self) {
        if let Err(error) = self.save_if_changed() {
            log::warn!("failed to persist attention list: {error}");
        }
    }

    fn save_if_changed(&mut self) -> std::io::Result<()> {
        let payload = serde_json::to_vec_pretty(&PersistedAttention {
            version: ATTENTION_VERSION,
            records: self.records.clone(),
        })?;
        if self.last_payload.as_deref() == Some(payload.as_slice()) {
            return Ok(());
        }
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&payload)?;
        temporary.flush()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        self.last_payload = Some(payload);
        Ok(())
    }
}

fn effective_turn_key(task: &TaskSnapshot) -> Option<String> {
    task.turn_key.clone().or_else(|| {
        (task.status == TaskStatus::Running).then(|| {
            let at = task.started_at.unwrap_or(task.updated_at);
            let millis = at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            format!("started:{millis}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DetectionConfidence, TaskSource};

    fn task(id: &str, status: TaskStatus, turn: Option<&str>) -> TaskSnapshot {
        let mut task = TaskSnapshot::new(
            id,
            "Native title",
            TaskSource::Codex,
            status,
            DetectionConfidence::Hook,
            SystemTime::now(),
        );
        task.turn_key = turn.map(str::to_string);
        task
    }

    #[test]
    fn baseline_dormant_history_is_not_imported() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        assert!(tracker
            .reconcile(vec![task("old", TaskStatus::WaitingInput, Some("t1"))])
            .is_empty());
    }

    #[test]
    fn running_then_waiting_stays_until_viewed() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        assert_eq!(
            tracker
                .reconcile(vec![task("a", TaskStatus::Running, Some("t1"))])
                .len(),
            1
        );
        let waiting = tracker.reconcile(vec![task("a", TaskStatus::WaitingInput, Some("t1"))]);
        assert!(waiting[0].unread);
        assert!(tracker.mark_viewed("a").unwrap().is_empty());
    }

    #[test]
    fn completion_becomes_unread_after_viewing_while_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        tracker.reconcile(vec![task("live", TaskStatus::Running, Some("t1"))]);
        tracker.mark_viewed("live").unwrap();

        let display = tracker.reconcile(vec![task("live", TaskStatus::WaitingInput, Some("t1"))]);
        assert_eq!(display.len(), 1);
        assert!(display[0].unread);
    }

    #[test]
    fn scope_change_removes_persisted_records() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        tracker.reconcile(vec![task("outside", TaskStatus::Running, Some("t1"))]);
        tracker.retain(|task| task.id != "outside");
        assert!(tracker.display().is_empty());
    }

    #[test]
    fn renamed_task_updates_without_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        tracker.reconcile(vec![task("a", TaskStatus::Running, Some("t1"))]);
        tracker.reconcile(vec![task("a", TaskStatus::WaitingInput, Some("t1"))]);
        tracker.mark_viewed("a").unwrap();
        let mut renamed = task("a", TaskStatus::WaitingInput, Some("t1"));
        renamed.name = "Renamed".into();
        assert!(tracker.reconcile(vec![renamed]).is_empty());
        assert_eq!(tracker.records["a"].task.name, "Renamed");
    }

    #[test]
    fn new_turn_reopens_hidden_task() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        tracker.reconcile(vec![task("a", TaskStatus::Running, Some("t1"))]);
        tracker.hide("a").unwrap();
        let reopened = tracker.reconcile(vec![task("a", TaskStatus::Running, Some("t2"))]);
        assert_eq!(reopened.len(), 1);
        assert!(reopened[0].unread);
    }

    #[test]
    fn pinned_task_survives_view_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        tracker.reconcile(vec![task("a", TaskStatus::Running, Some("t1"))]);
        tracker.set_pinned("a", true).unwrap();
        tracker.mark_viewed("a").unwrap();
        let restored = AttentionTracker::open(path);
        assert_eq!(restored.display().len(), 1);
        assert!(restored.display()[0].pinned);
    }
}
