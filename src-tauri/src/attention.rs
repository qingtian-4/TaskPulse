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

const ATTENTION_VERSION: u8 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AttentionRecord {
    task: TaskSnapshot,
    unread: bool,
    pinned: bool,
    last_turn_key: Option<String>,
    last_status: TaskStatus,
    #[serde(default)]
    ignored: bool,
    #[serde(default)]
    hidden: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedAttention {
    version: u8,
    records: HashMap<String, AttentionRecord>,
    #[serde(default)]
    baselines: HashMap<String, u64>,
}

pub struct AttentionTracker {
    path: PathBuf,
    records: HashMap<String, AttentionRecord>,
    last_payload: Option<Vec<u8>>,
    baselines: HashMap<String, u64>,
}

impl AttentionTracker {
    pub fn open(path: PathBuf) -> Self {
        let payload = std::fs::read(&path).ok();
        let persisted = payload
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<PersistedAttention>(bytes).ok())
            .filter(|state| state.version <= ATTENTION_VERSION);
        let (records, baselines) = persisted
            .map(|state| (state.records, state.baselines))
            .unwrap_or_default();
        Self {
            path,
            records,
            last_payload: payload,
            baselines,
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
            let now = millis(SystemTime::now());
            let source = crate::monitoring::source_id(&task);
            let baseline = self.baselines.entry(source).or_insert(now);
            let short_finished = turn_key.is_some()
                && task.status != TaskStatus::Running
                && millis(task.updated_at) > *baseline;

            if let Some(record) = self.records.get_mut(&id) {
                let changed_turn = turn_key.is_some()
                    && turn_key != record.last_turn_key
                    && (task.status == TaskStatus::Running
                        || millis(task.updated_at) > *baseline
                        || native_unread
                        || record.last_status == TaskStatus::Running);
                let resumed =
                    task.status == TaskStatus::Running && record.last_status != TaskStatus::Running;
                let completed =
                    record.last_status == TaskStatus::Running && task.status != TaskStatus::Running;
                task.new_execution = !task.stale && (changed_turn || resumed || completed);
                if !task.stale && (changed_turn || resumed || completed || native_unread) {
                    record.unread = true;
                }
                if changed_turn || resumed {
                    record.hidden = false;
                }
                if provider_pinned {
                    record.pinned = true;
                }

                // Native title/project refreshes update the same record only.
                task.unread = record.unread;
                task.pinned = record.pinned;
                let previous_section = section(&record.task);
                task.attention_since = if section(&task) != previous_section {
                    now
                } else {
                    record.task.attention_since
                };
                record.task = task;
                if turn_key.is_some() {
                    record.last_turn_key = turn_key;
                }
                record.last_status = record.task.status;
                continue;
            }

            // Startup baseline: only active/native-unread/native-pinned rows
            // enter the attention list. This prevents bulk history import.
            let unread = task.status == TaskStatus::Running
                || native_unread
                || short_finished
                || task.requires_action();
            task.unread = unread;
            task.pinned = provider_pinned;
            task.attention_since = now;
            task.new_execution = short_finished;
            self.records.insert(
                id,
                AttentionRecord {
                    last_turn_key: turn_key,
                    last_status: task.status,
                    task,
                    unread,
                    pinned: provider_pinned,
                    ignored: false,
                    hidden: false,
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
        if record.task.status == TaskStatus::Running {
            return Err("运行中的任务不能隐藏；如需长期停用，请选择不再监控".into());
        }
        record.unread = false;
        record.pinned = false;
        record.hidden = true;
        record.task.unread = false;
        record.task.pinned = false;
        self.save_best_effort();
        Ok(self.display())
    }

    pub fn restore(&mut self, id: &str) -> Result<Vec<TaskSnapshot>, String> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.unread = true;
        record.hidden = false;
        record.task.unread = true;
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
                !record.ignored
                    && (record.task.status == TaskStatus::Running
                        || (!record.hidden && record.task.requires_action())
                        || record.unread
                        || record.pinned)
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

    #[allow(dead_code)]
    pub fn retain<F>(&mut self, mut predicate: F)
    where
        F: FnMut(&TaskSnapshot) -> bool,
    {
        self.records.retain(|_, record| predicate(&record.task));
        self.save_best_effort();
    }

    pub fn known(&self) -> Vec<TaskSnapshot> {
        self.records
            .values()
            .map(|record| record.task.clone())
            .collect()
    }

    pub fn reason(&self, id: &str) -> String {
        match self.records.get(id) {
            Some(record) if record.ignored => "已设为不再监控".into(),
            Some(record)
                if record.task.status == TaskStatus::Running
                    || (!record.hidden && record.task.requires_action())
                    || record.unread
                    || record.pinned =>
            {
                "正在关注".into()
            }
            Some(_) => "历史会话 / 已查看或隐藏本轮".into(),
            None => "历史会话，尚未加入关注".into(),
        }
    }

    pub fn ignored(&self, id: &str) -> bool {
        self.records.get(id).is_some_and(|r| r.ignored)
    }

    pub fn follow(&mut self, task: TaskSnapshot) -> Result<Vec<TaskSnapshot>, String> {
        let mut task = task;
        task.new_execution = false;
        let record = self
            .records
            .entry(task.id.clone())
            .or_insert(AttentionRecord {
                last_turn_key: effective_turn_key(&task),
                last_status: task.status,
                task: task.clone(),
                unread: true,
                pinned: false,
                ignored: false,
                hidden: false,
            });
        record.unread = true;
        record.task = task;
        record.ignored = false;
        record.hidden = false;
        record.task.unread = true;
        record.task.attention_since = millis(SystemTime::now());
        self.save_best_effort();
        Ok(self.display())
    }

    pub fn set_ignored(&mut self, id: &str, ignored: bool) -> Result<Vec<TaskSnapshot>, String> {
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.ignored = ignored;
        if !ignored {
            record.unread = true;
        }
        self.save_best_effort();
        Ok(self.display())
    }

    pub fn reset_baselines(&mut self, sources: &HashSet<String>) {
        let now = millis(SystemTime::now());
        for source in sources {
            self.baselines.insert(source.clone(), now);
        }
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
            baselines: self.baselines.clone(),
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

fn millis(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn section(task: &TaskSnapshot) -> u8 {
    if task.stale || task.status == TaskStatus::Failed || task.requires_action() {
        0
    } else if task.status == TaskStatus::Running {
        1
    } else if task.unread {
        2
    } else {
        3
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

    #[test]
    fn completed_short_task_enters_after_baseline_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        let mut old = task("old", TaskStatus::WaitingInput, Some("old-turn"));
        old.updated_at = SystemTime::UNIX_EPOCH;
        assert!(tracker.reconcile(vec![old]).is_empty());
        let mut done = task("short", TaskStatus::WaitingInput, Some("new-turn"));
        done.updated_at = SystemTime::now() + std::time::Duration::from_secs(1);
        let display = tracker.reconcile(vec![done.clone()]);
        assert_eq!(display.len(), 1);
        assert!(display[0].new_execution);
        let mut restored = AttentionTracker::open(path);
        restored.set_ignored("short", true).unwrap();
        done.status = TaskStatus::Running;
        done.turn_key = Some("another-turn".into());
        assert!(restored.reconcile(vec![done]).is_empty());
        assert_eq!(restored.set_ignored("short", false).unwrap().len(), 1);
    }

    #[test]
    fn viewing_a_gate_keeps_it_visible_but_hiding_this_turn_dismisses_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        let mut gate = task("gate", TaskStatus::WaitingInput, Some("turn"));
        gate.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        tracker.reconcile(vec![gate]);
        assert_eq!(tracker.mark_viewed("gate").unwrap().len(), 1);
        assert!(tracker.hide("gate").unwrap().is_empty());
        assert_eq!(tracker.restore("gate").unwrap().len(), 1);
    }

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
    fn adopting_a_new_provider_key_for_old_viewed_history_is_not_a_new_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        let mut historical = task("old", TaskStatus::WaitingInput, None);
        historical.updated_at = SystemTime::UNIX_EPOCH;
        tracker.reconcile(vec![historical.clone()]);
        tracker.mark_viewed("old").unwrap();
        historical.turn_key = Some("settled:old-activity".into());
        assert!(tracker.reconcile(vec![historical]).is_empty());
        assert!(!tracker.known()[0].new_execution);
    }

    #[test]
    fn new_turn_reopens_hidden_task() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        tracker.reconcile(vec![task("a", TaskStatus::Running, Some("t1"))]);
        assert!(tracker.hide("a").is_err());
        tracker.reconcile(vec![task("a", TaskStatus::WaitingInput, Some("t1"))]);
        tracker.hide("a").unwrap();
        let reopened = tracker.reconcile(vec![task("a", TaskStatus::Running, Some("t2"))]);
        assert_eq!(reopened.len(), 1);
        assert!(reopened[0].unread);
    }

    #[test]
    fn hidden_task_can_be_restored_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        tracker.reconcile(vec![task("a", TaskStatus::Running, Some("t1"))]);
        tracker.reconcile(vec![task("a", TaskStatus::WaitingInput, Some("t1"))]);
        assert!(tracker.hide("a").unwrap().is_empty());
        let restored = tracker.restore("a").unwrap();
        assert_eq!(restored.len(), 1);
        assert!(restored[0].unread);
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
