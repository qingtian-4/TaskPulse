//! Delivery evidence, separate from lifecycle dedup. "Rendered" is not "read".
//! No Agent commands, prompts, credentials or approval decisions are persisted.
use crate::domain::TaskSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const LIMIT: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryRecord {
    pub at: u64,
    pub channel: String,
    pub stage: String,
    pub notice_id: Option<u64>,
    pub task_id: Option<String>,
    pub request_id: Option<String>,
    pub title: String,
    pub test: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct DeliveryLog {
    entries: VecDeque<DeliveryRecord>,
    #[serde(skip)]
    path: Option<PathBuf>,
    #[serde(skip)]
    error: Option<String>,
}
impl DeliveryLog {
    fn open(path: PathBuf) -> Self {
        let mut log: Self = std::fs::read(&path)
            .ok()
            .and_then(|body| serde_json::from_slice(&body).ok())
            .unwrap_or_default();
        log.path = Some(path);
        log
    }
    fn push(&mut self, record: DeliveryRecord) {
        self.entries.push_back(record);
        while self.entries.len() > LIMIT {
            self.entries.pop_front();
        }
        let result = (|| -> Result<(), String> {
            use std::io::Write;
            let Some(path) = &self.path else {
                return Ok(());
            };
            let parent = path.parent().ok_or("Missing notification log directory")?;
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            file.write_all(&serde_json::to_vec(self).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            file.as_file().sync_all().map_err(|e| e.to_string())?;
            file.persist(path).map_err(|e| e.to_string())?;
            Ok(())
        })();
        self.error = result.err();
        if let Some(error) = &self.error {
            log::warn!("notification delivery log: {error}");
        }
    }
}
static LOG: LazyLock<Mutex<DeliveryLog>> = LazyLock::new(|| {
    Mutex::new(
        dirs_next::home_dir()
            .map(|h| DeliveryLog::open(h.join(".taskpulse/notification-log.json")))
            .unwrap_or_default(),
    )
});

pub fn record(
    task: Option<&TaskSnapshot>,
    channel: &str,
    stage: &str,
    title: &str,
    notice_id: Option<u64>,
    test: bool,
) {
    LOG.lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(DeliveryRecord {
            at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            channel: channel.into(),
            stage: stage.into(),
            notice_id,
            task_id: task.filter(|_| !test).map(|t| t.id.clone()),
            request_id: task
                .filter(|_| !test)
                .and_then(|t| t.input_request_id.clone()),
            title: title.into(),
            test,
        });
}

#[tauri::command]
pub fn get_notification_log() -> Result<Vec<DeliveryRecord>, String> {
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(error) = &log.error {
        return Err(format!("提醒投递记录无法保存：{error}"));
    }
    Ok(log.entries.iter().rev().take(100).cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_history_is_atomic_bounded_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notification-log.json");
        let mut log = DeliveryLog::open(path.clone());
        for n in 0..LIMIT + 1 {
            log.push(DeliveryRecord {
                at: n as u64,
                channel: "popup".into(),
                stage: "rendered".into(),
                notice_id: Some(n as u64),
                task_id: None,
                request_id: None,
                title: "Test".into(),
                test: true,
            });
        }
        let restored = DeliveryLog::open(path);
        assert_eq!(restored.entries.len(), LIMIT);
        assert_eq!(restored.entries.front().unwrap().at, 1);
        assert_eq!(restored.entries.back().unwrap().stage, "rendered");
        assert!(log.error.is_none());
    }
}
