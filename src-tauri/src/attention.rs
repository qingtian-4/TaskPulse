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

const ATTENTION_VERSION: u8 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AttentionRecord {
    task: TaskSnapshot,
    unread: bool,
    pinned: bool,
    /// Explicit TaskPulse preference wins over a provider's native pin.
    #[serde(default)]
    pin_override: Option<bool>,
    /// Execution activity, NOT a metadata refresh/rename or a successful poll.
    #[serde(default)]
    last_execution_at: u64,
    #[serde(default)]
    followed_at: u64,
    /// Legacy cold-history imports are retained in older attention, not
    /// acknowledged/deleted. Explicit follow/pin or a new turn restores them.
    #[serde(default)]
    historical_import: bool,
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
    recent_days: u16,
    load_error: Option<crate::state_file::StateError>,
}

impl AttentionTracker {
    pub fn open(path: PathBuf) -> Self {
        let loaded =
            crate::state_file::read(&path, ATTENTION_VERSION.into(), false).and_then(|loaded| {
                loaded
                    .map(|(value, bytes)| {
                        serde_json::from_value::<PersistedAttention>(value)
                            .map(|state| (state, bytes))
                            .map_err(|_| crate::state_file::StateError::Invalid)
                    })
                    .transpose()
            });
        let (persisted, payload, mut load_error) = match loaded {
            Ok(Some((state, bytes))) => (Some(state), Some(bytes), None),
            Ok(None) => (None, None, None),
            Err(error) => (None, None, Some(error)),
        };
        if persisted
            .as_ref()
            .is_some_and(|state| state.version < ATTENTION_VERSION)
        {
            let backup =
                path.with_extension(if persisted.as_ref().is_some_and(|s| s.version < 3) {
                    "json.pre-recency.bak"
                } else {
                    "json.pre-lifecycle.bak"
                });
            if let Err(error) = crate::state_file::backup(&backup, payload.as_deref().unwrap()) {
                load_error = Some(error);
            }
        }
        let migrate =
            load_error.is_none() && persisted.as_ref().is_some_and(|state| state.version < 4);
        let (mut records, baselines) = persisted
            .map(|state| (state.records, state.baselines))
            .unwrap_or_default();
        // Legacy pins are preserved, but their origin is unknown. Do not
        // silently turn old imported/native pins into permanent user choices.
        for record in records.values_mut() {
            if migrate
                && record.task.source == crate::domain::TaskSource::ClaudeCode
                && record.task.status == TaskStatus::Failed
                && record.followed_at == 0
                && record.pin_override != Some(true)
                && record.task.native_unread != Some(true)
                && millis(record.task.updated_at).saturating_add(30_000)
                    < record.task.attention_since
                && millis(record.task.updated_at).saturating_add(30_000) < record.last_execution_at
            {
                // v3 could bootstrap an old incomplete Claude transcript as
                // RUNNING, then "complete" it as FAILED hours after its error.
                // Preserve unread/pin preferences, conservatively archive it.
                record.historical_import = true;
                record.last_execution_at = millis(record.task.updated_at);
            }
            // This is an observation-edge flag, not durable lifecycle state.
            record.task.new_execution = false;
            // Persistent attention is not a fresh provider observation. Slow
            // startup collectors must not replay a cached end/request while
            // another source refreshes. Keep outcome and user preferences;
            // only a matching successful observation restores freshness.
            record.task.stale = true;
            if record.last_execution_at == 0 {
                record.last_execution_at = millis(record.task.updated_at);
            }
        }
        let legacy_keys: Vec<_> = records
            .iter()
            .filter(|(_, record)| {
                record.task.legacy_run_alias().as_deref() == Some(&record.task.id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        if !legacy_keys.is_empty() {
            let backup = path.with_extension("json.pre-run-identity.bak");
            let backed_up = load_error.is_none()
                && crate::state_file::backup(&backup, payload.as_deref().unwrap()).is_ok();
            if backed_up {
                for old in legacy_keys {
                    let mut record = records.remove(&old).unwrap();
                    record.task.normalize_run_identity();
                    // An existing canonical record owns the latest user choice.
                    records.entry(record.task.id.clone()).or_insert(record);
                }
            } else {
                load_error = Some(crate::state_file::StateError::Backup);
            }
        }
        Self {
            path,
            records,
            last_payload: payload,
            baselines,
            recent_days: 7,
            load_error,
        }
    }

    pub fn persistence_error(&self) -> Option<&'static str> {
        self.load_error.map(|error| error.message())
    }

    fn writable(&self) -> Result<(), String> {
        self.persistence_error()
            .map_or(Ok(()), |error| Err(error.into()))
    }

    pub fn set_recent_days(&mut self, days: u16) {
        self.recent_days = days;
    }

    pub fn normalize_source_instances<F>(&mut self, mut resolve: F)
    where
        F: FnMut(&TaskSnapshot) -> Option<String>,
    {
        for record in self.records.values_mut() {
            if record.task.source_instance.is_none() || record.task.id.starts_with("scan:") {
                record.task.source_instance = resolve(&record.task);
            }
        }
    }

    /// Reconcile current observations into the persistent attention state.
    /// New dormant/history rows are deliberately ignored. Existing records
    /// still receive metadata updates so native renames are synchronized.
    pub fn reconcile(&mut self, observed: Vec<TaskSnapshot>) -> Vec<TaskSnapshot> {
        self.reconcile_with_persistence(observed, true)
    }
    pub fn reconcile_unflushed(&mut self, observed: Vec<TaskSnapshot>) -> Vec<TaskSnapshot> {
        self.reconcile_with_persistence(observed, false)
    }
    fn reconcile_with_persistence(
        &mut self,
        observed: Vec<TaskSnapshot>,
        persist: bool,
    ) -> Vec<TaskSnapshot> {
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
                // Durable observation replay must not undo a user's later
                // viewed/pin action or renew execution activity after a crash.
                if task.replays_integration(&record.task) {
                    record.task.merge_integration_health(&task);
                    continue;
                }
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
                if task.new_execution {
                    record.last_execution_at = now;
                }
                if !task.stale && (changed_turn || resumed || completed || native_unread) {
                    record.unread = true;
                }
                if changed_turn || resumed {
                    record.hidden = false;
                    if !task.stale {
                        record.historical_import = false;
                    }
                }
                record.pinned = record
                    .pin_override
                    .unwrap_or(record.pinned || provider_pinned);

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
            let unread = task.status == TaskStatus::Running && !task.stale
                || native_unread
                || short_finished;
            task.unread = unread;
            task.pinned = provider_pinned;
            task.attention_since = now;
            task.new_execution = short_finished;
            let last_execution_at = now.min(millis(task.updated_at));
            self.records.insert(
                id,
                AttentionRecord {
                    last_turn_key: turn_key,
                    last_status: task.status,
                    task,
                    unread,
                    pinned: provider_pinned,
                    pin_override: None,
                    last_execution_at,
                    followed_at: 0,
                    historical_import: false,
                    ignored: false,
                    hidden: !unread && !provider_pinned,
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

        if persist {
            self.save_best_effort();
        }
        let display = self.display();
        let sources: HashSet<_> = self
            .records
            .values()
            .filter_map(|r| r.task.source_instance.as_ref())
            .filter(|s| s.starts_with("codex-wsl:"))
            .collect();
        for source in sources {
            let tasks: Vec<_> = display
                .iter()
                .filter(|t| t.source_instance.as_ref() == Some(source))
                .cloned()
                .collect();
            crate::providers::resources::continuity(
                "attention_display",
                source,
                &tasks,
                serde_json::json!({
                    "stored_count":self.records.values().filter(|r|r.task.source_instance.as_ref()==Some(source)).count(),
                    "observed_count":observed_ids.iter().filter(|id|self.records.get(*id).is_some_and(|r|r.task.source_instance.as_ref()==Some(source))).count(),
                    "hidden_count":self.records.values().filter(|r|r.task.source_instance.as_ref()==Some(source) && (r.hidden || r.ignored)).count(),
                    "archived_count":self.records.values().filter(|r|r.task.source_instance.as_ref()==Some(source) && self.expired(r,millis(SystemTime::now()))).count()
                }),
            );
        }
        display
    }

    pub fn mark_viewed(&mut self, id: &str) -> Result<Vec<TaskSnapshot>, String> {
        self.writable()?;
        let action_id = id.to_owned();
        let previous = self.records.get(&action_id).cloned();
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.unread = false;
        record.task.unread = false;
        self.persist_action(&action_id, previous)
    }

    pub fn hide(&mut self, id: &str) -> Result<Vec<TaskSnapshot>, String> {
        self.writable()?;
        let action_id = id.to_owned();
        let previous = self.records.get(&action_id).cloned();
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        if record.task.status == TaskStatus::Running {
            return Err("运行中的任务不能隐藏；如需长期停用，请选择不再监控".into());
        }
        record.unread = false;
        record.pinned = false;
        record.pin_override = Some(false);
        record.hidden = true;
        record.task.unread = false;
        record.task.pinned = false;
        self.persist_action(&action_id, previous)
    }

    pub fn restore(&mut self, id: &str) -> Result<Vec<TaskSnapshot>, String> {
        self.writable()?;
        let action_id = id.to_owned();
        let previous = self.records.get(&action_id).cloned();
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.unread = true;
        record.hidden = false;
        record.followed_at = millis(SystemTime::now());
        record.historical_import = false;
        record.task.unread = true;
        self.persist_action(&action_id, previous)
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<Vec<TaskSnapshot>, String> {
        self.writable()?;
        let action_id = id.to_owned();
        let previous = self.records.get(&action_id).cloned();
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.pinned = pinned;
        record.pin_override = Some(pinned);
        record.task.pinned = pinned;
        self.persist_action(&action_id, previous)
    }

    pub fn display(&self) -> Vec<TaskSnapshot> {
        self.list(false, millis(SystemTime::now()))
    }

    pub fn archived(&self) -> Vec<TaskSnapshot> {
        self.list(true, millis(SystemTime::now()))
    }

    fn active(record: &AttentionRecord) -> bool {
        !record.ignored
            && (record.task.status == TaskStatus::Running
                || (!record.hidden && record.task.requires_action())
                || record.unread
                || record.pinned)
    }

    fn expired(&self, record: &AttentionRecord, now: u64) -> bool {
        record.historical_import && record.pin_override != Some(true)
            || self.recent_days > 0
                && record.task.status != TaskStatus::Running
                && record.pin_override != Some(true)
                && now.saturating_sub(record.last_execution_at.max(record.followed_at))
                    > u64::from(self.recent_days) * 86_400_000
    }

    fn list(&self, archived: bool, now: u64) -> Vec<TaskSnapshot> {
        let mut tasks: Vec<TaskSnapshot> = self
            .records
            .values()
            .filter(|record| Self::active(record) && self.expired(record, now) == archived)
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
                if Self::active(record) && self.expired(record, millis(SystemTime::now())) =>
            {
                "较早关注 / 可重新加入".into()
            }
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
        self.writable()?;
        let action_id = task.id.clone();
        let previous = self.records.get(&action_id).cloned();
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
                pin_override: None,
                last_execution_at: millis(task.updated_at),
                followed_at: 0,
                historical_import: false,
                ignored: false,
                hidden: false,
            });
        record.unread = true;
        record.task = task;
        record.ignored = false;
        record.hidden = false;
        record.followed_at = millis(SystemTime::now());
        record.historical_import = false;
        record.task.unread = true;
        record.task.attention_since = millis(SystemTime::now());
        self.persist_action(&action_id, previous)
    }

    pub fn set_ignored(&mut self, id: &str, ignored: bool) -> Result<Vec<TaskSnapshot>, String> {
        self.writable()?;
        let action_id = id.to_owned();
        let previous = self.records.get(&action_id).cloned();
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| format!("unknown task id: {id}"))?;
        record.ignored = ignored;
        if !ignored {
            record.unread = true;
            record.task.unread = true;
            record.followed_at = millis(SystemTime::now());
        }
        self.persist_action(&action_id, previous)
    }

    pub fn reset_baselines(&mut self, sources: &HashSet<String>) {
        let now = millis(SystemTime::now());
        for source in sources {
            self.baselines.insert(source.clone(), now);
        }
        self.save_best_effort();
    }

    // User actions commit only after atomic replacement succeeds.
    fn persist_action(
        &mut self,
        id: &str,
        previous: Option<AttentionRecord>,
    ) -> Result<Vec<TaskSnapshot>, String> {
        if let Err(error) = self.save_if_changed() {
            match previous {
                Some(record) => {
                    self.records.insert(id.to_owned(), record);
                }
                None => {
                    self.records.remove(id);
                }
            }
            return Err(format!(
                "Failed to save attention state; please retry: {error}"
            ));
        }
        Ok(self.display())
    }

    fn save_best_effort(&mut self) {
        if let Err(error) = self.save_if_changed() {
            log::warn!("failed to persist attention list: {error}");
        }
    }

    fn save_if_changed(&mut self) -> std::io::Result<()> {
        if let Some(error) = self.persistence_error() {
            return Err(std::io::Error::other(error));
        }
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
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        self.last_payload = Some(payload);
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.save_if_changed()
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
    #[test]
    fn failed_manual_save_rolls_back_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        tracker
            .follow(task("a", TaskStatus::Succeeded, Some("turn")))
            .unwrap();
        let persisted = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(tracker.mark_viewed("a").is_err());
        assert!(tracker.set_pinned("a", true).is_err());
        assert!(tracker.hide("a").is_err());
        assert!(tracker.set_ignored("a", true).is_err());
        assert!(tracker
            .follow(task("b", TaskStatus::Running, Some("turn")))
            .is_err());
        assert_eq!(tracker.records.len(), 1);
        let record = &tracker.records["a"];
        assert!(record.unread && !record.hidden && !record.ignored && !record.pinned);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, persisted).unwrap();
        tracker.mark_viewed("a").unwrap();
        assert!(!AttentionTracker::open(path).records["a"].unread);
    }
    #[test]
    fn r02_invalid_or_future_attention_never_overwrites_original_across_restarts() {
        for original in [
            b"{truncated".as_slice(),
            b"{}",
            b"{\"version\":99,\"records\":{}}",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("attention.json");
            std::fs::write(&path, original).unwrap();
            for _ in 0..2 {
                let mut tracker = AttentionTracker::open(path.clone());
                assert!(tracker.persistence_error().is_some());
                tracker.reconcile(vec![task("fresh", TaskStatus::Running, Some("turn"))]);
                assert!(tracker.set_pinned("fresh", true).is_err());
                assert!(tracker.mark_viewed("fresh").is_err());
                assert!(tracker.flush().is_err());
                assert_eq!(std::fs::read(&path).unwrap(), original);
            }
        }
    }

    #[test]
    fn r02_attention_read_error_is_not_a_new_installation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        std::fs::create_dir(&path).unwrap();
        let mut tracker = AttentionTracker::open(path.clone());
        assert!(tracker.persistence_error().is_some());
        assert!(tracker.flush().is_err());
        assert!(path.is_dir());
    }

    #[test]
    fn r02_migration_requires_a_complete_matching_regular_backup() {
        for backup_is_directory in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("attention.json");
            let mut tracker = AttentionTracker::open(path.clone());
            tracker.reconcile(vec![task("a", TaskStatus::Running, Some("turn"))]);
            tracker.set_pinned("a", true).unwrap();
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            value["version"] = 3.into();
            let original = serde_json::to_vec(&value).unwrap();
            std::fs::write(&path, &original).unwrap();
            let backup = path.with_extension("json.pre-lifecycle.bak");
            if backup_is_directory {
                std::fs::create_dir(&backup).unwrap();
            } else {
                std::fs::write(&backup, b"partial").unwrap();
            }
            let mut restored = AttentionTracker::open(path.clone());
            assert!(restored.persistence_error().is_some());
            assert!(restored.known()[0].unread && restored.known()[0].pinned);
            assert!(restored.flush().is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert!(backup.is_dir() || std::fs::read(backup).unwrap() == b"partial");
        }
    }

    #[test]
    fn r02_missing_attention_initializes_and_valid_state_can_be_repaired_before_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        assert!(tracker.persistence_error().is_none());
        tracker.reconcile(vec![task("a", TaskStatus::Running, Some("turn"))]);
        let valid = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"bad").unwrap();
        let mut failed = AttentionTracker::open(path.clone());
        std::fs::write(&path, &valid).unwrap();
        assert!(failed.flush().is_err()); // No implicit retry overwriting a repair.
        let mut repaired = AttentionTracker::open(path);
        assert!(repaired.persistence_error().is_none());
        assert!(repaired.known()[0].unread);
        assert!(repaired.flush().is_ok());
    }
    #[test]
    fn lifecycle_restart_caches_do_not_replay_old_turn_during_other_source_refresh() {
        use crate::store::TaskStore;
        use crate::transition::{TaskEvent, TransitionEngine};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let ledger = dir.path().join("notifications.json");
        let now = SystemTime::now();
        let mut old = task("slow", TaskStatus::Running, Some("old"));
        let mut tracker = AttentionTracker::open(path.clone());
        tracker.reconcile(vec![old.clone()]);
        tracker.set_pinned("slow", true).unwrap();
        old.status = TaskStatus::WaitingInput;
        old.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        old.finished_at = Some(now);
        tracker.reconcile(vec![old.clone()]);
        let mut current = task("slow", TaskStatus::Running, Some("current"));
        let mut fast = task("fast", TaskStatus::Running, Some("fast-turn"));
        fast.source_instance = Some("other-source".into());
        let mut engine = TransitionEngine::open(ledger.clone());
        assert!(engine
            .apply_snapshots(&[current.clone(), fast.clone()], &[], now)
            .is_empty());

        // A crash can leave attention/history older than the durable ledger.
        // A fast provider must still notify, not replay the slow cached end.
        let mut restored = AttentionTracker::open(path);
        let retained = restored
            .known()
            .into_iter()
            .find(|t| t.id == "slow")
            .unwrap();
        assert!(retained.unread && retained.pinned);
        let mut store = TaskStore::default();
        store.restore_recent(vec![old], now);
        let mut engine = TransitionEngine::open(ledger);
        fast.status = TaskStatus::Succeeded;
        restored.reconcile(store.merge(vec![fast], now));
        let events = engine.apply_snapshots(&restored.known(), &[], now);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], TaskEvent::Succeeded { task } if task.id == "fast"));
        assert!(
            restored
                .known()
                .iter()
                .find(|t| t.id == "slow")
                .unwrap()
                .stale
        );

        restored.reconcile(store.merge(vec![current.clone()], now));
        assert!(engine
            .apply_snapshots(&restored.known(), &[], now)
            .is_empty());
        current.status = TaskStatus::WaitingInput;
        current.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
        restored.reconcile(store.merge(vec![current], now));
        let events = engine.apply_snapshots(&restored.known(), &[], now);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], TaskEvent::WaitingInput { task, .. } if task.id == "slow" && task.turn_key.as_deref() == Some("current"))
        );
    }

    #[test]
    fn pane_scoped_exact_records_migrate_with_backup_without_losing_unread_or_manual_pin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        let mut old = task("ssh:host:%3", TaskStatus::Running, Some("run-a"));
        old.source = TaskSource::RemoteTmux;
        old.confidence = DetectionConfidence::Exact;
        old.host = Some("host".into());
        old.tmux_pane = Some("%3".into());
        tracker.reconcile(vec![old.clone()]);
        tracker.set_pinned(&old.id, true).unwrap();
        old.status = TaskStatus::Succeeded;
        tracker.reconcile(vec![old]);
        let restored = AttentionTracker::open(path.clone());
        assert!(path.with_extension("json.pre-run-identity.bak").exists());
        let tasks = restored.display();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "ssh:host:run:run-a");
        assert!(tasks[0].unread && tasks[0].pinned);
        assert_eq!(tasks[0].turn_key.as_deref(), Some("run-a"));
    }
    #[test]
    fn legacy_false_failure_import_is_archived_without_acknowledging_or_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        let mut old = task("claude:old", TaskStatus::Running, Some("old-turn"));
        old.source = crate::domain::TaskSource::ClaudeCode;
        old.updated_at = SystemTime::now() - std::time::Duration::from_secs(3600);
        tracker.reconcile(vec![old.clone()]);
        old.status = TaskStatus::Failed;
        tracker.reconcile(vec![old.clone()]);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["version"] = 3.into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut upgraded = AttentionTracker::open(path.clone());
        assert!(upgraded.reconcile(vec![old.clone()]).is_empty());
        assert!(upgraded.archived()[0].unread);
        assert_eq!(upgraded.known().len(), 1);
        assert!(path.with_extension("json.pre-lifecycle.bak").exists());
        old.status = TaskStatus::Running;
        old.turn_key = Some("new-turn".into());
        old.updated_at = SystemTime::now();
        assert_eq!(upgraded.reconcile(vec![old]).len(), 1);
    }

    #[test]
    fn cold_failed_and_blocked_history_do_not_become_current_attention() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        let failed = task("old-failure", TaskStatus::Failed, Some("old"));
        let mut approval = task("old-gate", TaskStatus::WaitingInput, Some("old"));
        approval.waiting_reason = Some(crate::domain::WaitingReason::Approval);
        assert!(tracker.reconcile(vec![failed.clone(), approval]).is_empty());
        let mut unread = failed;
        unread.native_unread = Some(true);
        assert_eq!(tracker.reconcile(vec![unread]).len(), 1);
    }
    use crate::domain::{DetectionConfidence, TaskSource};

    #[test]
    fn legacy_migration_backs_up_and_normalizes_identity_without_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut legacy = AttentionTracker::open(path.clone());
        legacy.reconcile(vec![task("a", TaskStatus::Running, Some("t1"))]);
        legacy.reconcile(vec![task("a", TaskStatus::WaitingInput, Some("t1"))]);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["version"] = 2.into();
        let record = value["records"]["a"].as_object_mut().unwrap();
        record.remove("last_execution_at");
        record.remove("pin_override");
        record.remove("followed_at");
        let original = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &original).unwrap();
        let mut restored = AttentionTracker::open(path.clone());
        assert_eq!(
            std::fs::read(path.with_extension("json.pre-recency.bak")).unwrap(),
            original
        );
        restored.normalize_source_instances(|_| Some("remote-codex:server3".into()));
        assert_eq!(
            restored.display()[0].source_instance.as_deref(),
            Some("remote-codex:server3")
        );
        assert_eq!(restored.display()[0].id, "a");
        assert!(!restored.display()[0].new_execution);
    }

    #[test]
    fn obsolete_approval_hint_loads_without_losing_attention_preferences_or_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        let mut running = task("codex:s", TaskStatus::Running, Some("turn"));
        running.source = TaskSource::Codex;
        running.session_id = Some("s".into());
        tracker.reconcile(vec![running]);
        tracker.set_pinned("codex:s", true).unwrap();
        let expected = tracker.known()[0].clone();
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let record = &mut legacy["records"]["codex:s"];
        record["task"]["approval_hint"] =
            serde_json::json!({"request_id":"c1","requested_at":1,"check_due":true});
        record["task"]["phase"] = "awaiting_tool".into();
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let restored = AttentionTracker::open(path);
        let tasks = restored.display();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, expected.id);
        assert_eq!(tasks[0].turn_key, expected.turn_key);
        assert_eq!(tasks[0].attention_since, expected.attention_since);
        assert_eq!(tasks[0].unread, expected.unread);
        assert!(tasks[0].pinned);
        assert!(!tasks[0].requires_action());
        assert_eq!(
            tasks[0].phase,
            Some(crate::domain::ExecutionPhase::AwaitingTool)
        );
        assert!(serde_json::to_value(&tasks[0])
            .unwrap()
            .get("approval_hint")
            .is_none());
        assert!(crate::transition::TransitionEngine::new()
            .apply(&tasks, &[], SystemTime::now())
            .is_empty());
    }

    #[test]
    fn old_attention_is_archived_without_losing_unread_or_native_pins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        let mut old = task("old", TaskStatus::WaitingInput, Some("old-turn"));
        old.native_unread = Some(true);
        old.pinned = true;
        old.updated_at = SystemTime::now() - std::time::Duration::from_secs(20 * 86400);
        assert!(tracker.reconcile(vec![old.clone()]).is_empty());
        assert_eq!(tracker.archived().len(), 1);
        assert!(tracker.archived()[0].unread && tracker.archived()[0].pinned);
        old.name = "Renamed old conversation".into();
        old.updated_at = SystemTime::now(); // Metadata rename is not execution.
        assert!(tracker.reconcile(vec![old.clone()]).is_empty());
        assert!(!tracker.known()[0].new_execution);
        let mut restored = AttentionTracker::open(path);
        assert!(restored.display().is_empty());
        old.status = TaskStatus::Running;
        old.turn_key = Some("new-turn".into());
        assert_eq!(restored.reconcile(vec![old]).len(), 1);
        assert!(restored.archived().is_empty());
        assert_eq!(
            restored
                .reconcile(vec![task(
                    "old",
                    TaskStatus::WaitingInput,
                    Some("new-turn")
                )])
                .len(),
            1
        );
    }

    #[test]
    fn manual_pin_and_follow_restore_archived_tasks_and_unpin_wins_over_native() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attention.json");
        let mut tracker = AttentionTracker::open(path.clone());
        let mut old = task("old", TaskStatus::Idle, Some("t1"));
        old.pinned = true;
        old.updated_at = SystemTime::UNIX_EPOCH;
        tracker.reconcile(vec![old.clone()]);
        assert!(tracker.display().is_empty());
        assert_eq!(tracker.set_pinned("old", true).unwrap().len(), 1);
        assert_eq!(AttentionTracker::open(path.clone()).display().len(), 1);
        tracker.set_pinned("old", false).unwrap();
        assert!(tracker.reconcile(vec![old.clone()]).is_empty());
        let mut restored = AttentionTracker::open(path);
        restored.reconcile(vec![old.clone()]);
        assert!(!restored.known()[0].pinned);
        assert_eq!(restored.follow(old).unwrap().len(), 1);
    }

    #[test]
    fn recent_window_never_limits_running_tasks_and_zero_disables_archival() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
        let mut running = task("running", TaskStatus::Running, Some("t1"));
        running.updated_at = SystemTime::UNIX_EPOCH;
        assert_eq!(tracker.reconcile(vec![running]).len(), 1);
        let mut old = task("old", TaskStatus::Failed, Some("t2"));
        old.native_unread = Some(true);
        old.updated_at = SystemTime::UNIX_EPOCH;
        assert_eq!(tracker.reconcile(vec![old]).len(), 1);
        tracker.set_recent_days(0);
        assert_eq!(tracker.display().len(), 2);
        assert!(tracker.archived().is_empty());
    }

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
        gate.native_unread = Some(true);
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
