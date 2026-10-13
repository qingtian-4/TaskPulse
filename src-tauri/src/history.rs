use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::TaskSnapshot;

const HISTORY_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct PersistedHistory {
    version: u8,
    tasks: Vec<TaskSnapshot>,
}

/// Small, best-effort cache for recently completed tasks. Active tasks are
/// deliberately never persisted because providers must rediscover them after
/// every application restart.
pub struct HistoryStore {
    path: PathBuf,
    last_payload: Option<Vec<u8>>,
}

impl HistoryStore {
    pub fn open(path: PathBuf) -> (Self, Vec<TaskSnapshot>) {
        let payload = std::fs::read(&path).ok();
        let mut tasks = payload
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<PersistedHistory>(bytes).ok())
            .filter(|history| history.version == HISTORY_VERSION)
            .map(|history| history.tasks)
            .unwrap_or_default();
        let mut migrated = false;
        for task in &mut tasks {
            migrated |= task.normalize_run_identity();
        }
        if migrated {
            let backup = path.with_extension("json.pre-run-identity.bak");
            if !backup.exists() {
                if let Err(error) = std::fs::copy(&path, backup) {
                    log::warn!("recent run identity backup: {error}");
                }
            }
        }
        (
            Self {
                path,
                last_payload: payload,
            },
            tasks,
        )
    }

    pub fn save_if_changed(&mut self, tasks: Vec<TaskSnapshot>) -> std::io::Result<()> {
        let payload = serde_json::to_vec_pretty(&PersistedHistory {
            version: HISTORY_VERSION,
            tasks,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DetectionConfidence, TaskSource, TaskStatus};
    use std::time::SystemTime;

    #[test]
    fn history_round_trip_preserves_recent_task() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        let now = SystemTime::now();
        let task = TaskSnapshot::new(
            "codex:done",
            "done",
            TaskSource::Codex,
            TaskStatus::Succeeded,
            DetectionConfidence::Exact,
            now,
        )
        .with_finished_at(now);

        let (mut history, restored) = HistoryStore::open(path.clone());
        assert!(restored.is_empty());
        history.save_if_changed(vec![task.clone()]).unwrap();

        let (_, restored) = HistoryStore::open(path);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, task.id);
        assert_eq!(restored[0].status, task.status);
        assert!(restored[0].finished_at.is_some());
    }

    #[test]
    fn corrupt_history_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        std::fs::write(&path, b"not json").unwrap();
        let (_, restored) = HistoryStore::open(path);
        assert!(restored.is_empty());
    }

    #[test]
    fn history_atomically_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        let (mut history, _) = HistoryStore::open(path.clone());
        history.save_if_changed(Vec::new()).unwrap();

        let now = SystemTime::now();
        let task = TaskSnapshot::new(
            "remote:done",
            "done",
            TaskSource::RemoteTmux,
            TaskStatus::Failed,
            DetectionConfidence::Exact,
            now,
        )
        .with_finished_at(now);
        history.save_if_changed(vec![task.clone()]).unwrap();

        let (_, restored) = HistoryStore::open(path);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, task.id);
        assert_eq!(restored[0].status, task.status);
    }
}
