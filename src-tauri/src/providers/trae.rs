//! Trae workspace metadata. The task database is only checked for its file
//! signature; workspace discovery does not imply task lifecycle support.
use super::{Provider, ProviderOutput};
use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
pub fn default_root() -> PathBuf {
    dirs_next::config_dir().unwrap_or_default().join("Trae")
}
pub fn available(root: &Path) -> bool {
    root.join("ModularData/ai-agent/database.db").is_file()
        || root.join("User/workspaceStorage").is_dir()
}
pub struct TraeProvider {
    root: PathBuf,
    last: Vec<TaskSnapshot>,
}
impl TraeProvider {
    pub fn new(root: PathBuf) -> Self {
        Self { root, last: vec![] }
    }
    fn read(&self, now: SystemTime) -> Result<(Vec<TaskSnapshot>, bool), String> {
        let primary = self.root.join("ModularData/ai-agent/database.db");
        let non_sqlite = if primary.exists() {
            let mut magic = [0; 16];
            std::fs::File::open(primary)
                .map_err(|_| "agent_store_read")?
                .read_exact(&mut magic)
                .map_err(|_| "agent_store_header")?;
            &magic != b"SQLite format 3\0"
        } else {
            false
        };
        let base = self.root.join("User/workspaceStorage");
        let mut rows = vec![];
        let mut total = 0usize;
        if base.exists() {
            for (index, entry) in std::fs::read_dir(base)
                .map_err(|_| "workspace_listing")?
                .enumerate()
            {
                if index >= 256 {
                    return Err("workspace_budget".into());
                }
                let entry = entry.map_err(|_| "workspace_listing")?;
                if !entry
                    .file_type()
                    .is_ok_and(|t| t.is_dir() && !t.is_symlink())
                {
                    continue;
                }
                let path = entry.path().join("state.vscdb");
                if !path.is_file() {
                    continue;
                }
                let db = Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|_| "workspace_read")?;
                db.busy_timeout(Duration::from_millis(250))
                    .map_err(|_| "workspace_busy")?;
                let mut stmt=db.prepare("SELECT length(value),value FROM ItemTable WHERE key='memento/icube-ai-agent-storage'").map_err(|_|"workspace_schema")?;
                let mut entries = stmt.query([]).map_err(|_| "workspace_read")?;
                if let Some(row) = entries.next().map_err(|_| "workspace_read")? {
                    let len: usize = row.get(0).map_err(|_| "workspace_read")?;
                    total = total.saturating_add(len);
                    if len > 1024 * 1024 || total > 4 * 1024 * 1024 {
                        return Err("metadata_budget".into());
                    }
                    let raw: String = row.get(1).map_err(|_| "workspace_read")?;
                    let data: Value = serde_json::from_str(&raw).map_err(|_| "workspace_json")?;
                    for session in data
                        .get("list")
                        .and_then(Value::as_array)
                        .ok_or("workspace_schema")?
                    {
                        if !session
                            .get("messages")
                            .and_then(Value::as_array)
                            .is_some_and(|m| !m.is_empty())
                        {
                            continue;
                        }
                        let Some(id) = session.get("sessionId").and_then(Value::as_str) else {
                            continue;
                        };
                        let mut task = TaskSnapshot::new(
                            format!("trae:{id}"),
                            session
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or("Trae"),
                            TaskSource::NativeAgent,
                            TaskStatus::Unknown,
                            DetectionConfidence::Inferred,
                            now,
                        );
                        task.agent_id = Some("trae".into());
                        task.agent_name = Some("Trae".into());
                        task.session_id = Some(id.into());
                        task.stale = true;
                        rows.push(task);
                    }
                }
            }
        }
        Ok((rows, non_sqlite))
    }
}
impl Provider for TraeProvider {
    fn name(&self) -> &'static str {
        "trae"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match self.read(now) {
            Ok((rows, non_sqlite)) => {
                self.last = rows.clone();
                ProviderOutput {
                    catalog: rows,
                    ..ProviderOutput::degraded(
                        vec![],
                        if non_sqlite {
                            "Trae 当前会话库格式不可直接读取；仅公开元数据可读，任务状态读取受限"
                        } else {
                            "Trae 目前只读取公开会话元数据；任务状态尚未适配"
                        },
                    )
                }
            }
            Err(code) => ProviderOutput {
                catalog: self.last.clone(),
                ..ProviderOutput::degraded(
                    vec![],
                    format!("Trae 公开元数据暂不可读（{code}）；保留上次状态"),
                )
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_sqlite_store_is_not_healthy_empty_or_a_fake_task() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ModularData/ai-agent");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("database.db"), [0x91; 32]).unwrap();
        let mut p = TraeProvider::new(dir.path().into());
        let output = p.poll(SystemTime::now());
        assert!(!output.authoritative);
        assert!(output.tasks.is_empty());
        assert!(output.error.as_deref().unwrap().contains("格式"));
    }
    #[test]
    #[ignore = "read-only installed TRAE; no private sessions printed"]
    fn live_trae_storage_probe() {
        let p = TraeProvider::new(default_root());
        let (rows, non_sqlite) = p.read(SystemTime::now()).unwrap();
        println!("TRAE public metadata sessions={} primary_not_sqlite={non_sqlite}; no lifecycle acceptance",rows.len());
    }
}
