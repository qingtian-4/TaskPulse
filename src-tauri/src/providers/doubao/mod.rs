//! Doubao Desktop 2.26.10: read-only native execution checkpoints (best effort).
//! Checkpoint removal is NOT completion. Only a whole-reply FINALLY state can
//! end a turn; tool/message completion and permission-mode settings cannot.
mod storage;
use super::{Provider, ProviderOutput};
use crate::domain::{
    DetectionConfidence, ExecutionPhase, TaskSnapshot, TaskSource, TaskStatus, WaitingReason,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn default_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Doubao/User Data");
    #[cfg(target_os = "macos")]
    let base = dirs_next::home_dir()
        .unwrap_or_default()
        .join("Library/Application Support/Doubao");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = dirs_next::home_dir()
        .unwrap_or_default()
        .join(".config/Doubao");
    base.join("Default/IndexedDB")
}
pub fn available(root: &std::path::Path) -> bool {
    root.join(storage::DATABASE).join("CURRENT").is_file()
}

pub struct DoubaoProvider {
    root: PathBuf,
    reader: storage::Reader,
    last: HashMap<String, TaskSnapshot>,
    progress: HashMap<String, Value>,
}
impl DoubaoProvider {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            reader: Default::default(),
            last: HashMap::new(),
            progress: HashMap::new(),
        }
    }
    fn observe(&mut self, rows: Value, now: SystemTime) -> storage::Result<ProviderOutput> {
        let rows = rows.as_array().ok_or("projection")?;
        let mut current = HashMap::new();
        let mut progress = self.progress.clone();
        for row in rows {
            let id = format!(
                "doubao:{}",
                row["conversation"].as_str().ok_or("conversation")?
            );
            let turn = row["session"].as_str().ok_or("session")?;
            let created = row["created"].as_u64().ok_or("created")?;
            let status = row["state"].as_str().ok_or("state")?;
            let previous = self.last.get(&id);
            // Never import terminal idle history on first observation.
            if status != "running" && !previous.is_some_and(|t| t.turn_key.as_deref() == Some(turn))
            {
                continue;
            }
            let mut t = TaskSnapshot::new(
                &id,
                row["name"].as_str().unwrap_or("豆包工作任务"),
                TaskSource::Doubao,
                if status == "ended" {
                    TaskStatus::WaitingInput
                } else {
                    TaskStatus::Running
                },
                DetectionConfidence::Inferred,
                now,
            );
            t.agent_id = Some("doubao".into());
            t.agent_name = Some("豆包".into());
            t.evidence_channel = Some("native_metadata".into());
            t.session_id = Some(row["conversation"].as_str().unwrap().into());
            t.turn_key = Some(turn.into());
            t.started_at = Some(UNIX_EPOCH + Duration::from_millis(created));
            t.updated_at = if self.progress.get(&id) == Some(&row["progress"]) {
                previous.map(|t| t.updated_at).unwrap_or(now)
            } else {
                now
            };
            if status == "ended" {
                t.waiting_reason = Some(WaitingReason::TurnEnded);
                t.finished_at = Some(t.updated_at);
            } else {
                t.phase = match row["phase"].as_str() {
                    Some("thinking") => Some(ExecutionPhase::Thinking),
                    Some("tool") => Some(ExecutionPhase::Tool),
                    Some("responding") => Some(ExecutionPhase::Responding),
                    _ => None,
                };
            }
            t.message = Some("豆包原生执行检查点 · 实验观测；审批提醒尚未适配".into());
            progress.insert(id.clone(), row["progress"].clone());
            current.insert(id, t);
        }
        for (id, old) in &self.last {
            if !current.contains_key(id) {
                let mut t = old.clone();
                t.stale = t.status == TaskStatus::Running;
                current.insert(id.clone(), t);
            }
        }
        if current.len() > 500 {
            return Err("retained task budget");
        }
        self.last = current;
        self.progress = progress;
        let tasks = self.last.values().cloned().collect();
        Ok(if self.last.values().any(|t| t.stale) {
            ProviderOutput::degraded(tasks, "豆包执行检查点已移除；未确认任务结束，保留上次状态")
        } else {
            ProviderOutput::tasks(tasks)
        })
    }
}
impl Provider for DoubaoProvider {
    fn name(&self) -> &'static str {
        "doubao"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let rows = if desktop_open() {
            self.reader.read(&self.root)
        } else {
            Err("desktop not running")
        };
        match rows.and_then(|rows| self.observe(rows, now)) {
            Ok(output) => output,
            Err(reason) => ProviderOutput::degraded(
                self.last
                    .values()
                    .cloned()
                    .map(|mut t| {
                        t.stale = true;
                        t
                    })
                    .collect(),
                format!("豆包原生数据暂不可读（{reason}）；不会推断结束或批准请求"),
            ),
        }
    }
}

// Process presence is only source health. It never creates or finishes a task.
#[cfg(target_os = "windows")]
fn desktop_open() -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    // SAFETY: initialize the documented size, keep the snapshot alive through
    // enumeration, and close it on the single post-acquisition return path.
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut row: PROCESSENTRY32W = std::mem::zeroed();
        row.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(handle, &mut row);
        let mut found = false;
        while ok != 0 {
            let length = row
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(row.szExeFile.len());
            if String::from_utf16_lossy(&row.szExeFile[..length]).eq_ignore_ascii_case("doubao.exe")
            {
                found = true;
                break;
            }
            ok = Process32NextW(handle, &mut row);
        }
        CloseHandle(handle);
        found
    }
}
#[cfg(not(target_os = "windows"))]
fn desktop_open() -> bool {
    true
}

fn text(v: &Value) -> Option<&str> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
}
fn objects(v: &Value) -> Vec<&Value> {
    v.as_object()
        .map(|o| o.values().collect())
        .unwrap_or_default()
}
/// Immediately discard request bodies, output text, account fields and children.
fn project_checkpoint(v: &Value) -> storage::Result<Value> {
    if v["version"].as_f64() != Some(2.0) {
        return Err("checkpoint version");
    }
    let main = v["state"]["mainTaskDataMap"]
        .as_object()
        .ok_or("checkpoint schema")?;
    if main.len() > 200 {
        return Err("task budget");
    }
    let mut rows = Vec::new();
    for (key, task) in main {
        let Some(session) = text(&task["sessionId"]).filter(|id| *id == key) else {
            return Err("session identity");
        };
        let created = task["createTime"]
            .as_f64()
            .filter(|t| t.is_finite() && *t > 0.0 && *t < 4_102_444_800_000.0)
            .ok_or("create time")? as u64;
        let messages = objects(&task["receivedMessages"]);
        let Some(message) = messages
            .iter()
            .max_by_key(|m| m["extra"]["index"].as_i64().unwrap_or(0))
        else {
            continue;
        };
        let Some(conversation) =
            text(&message["extra"]["conversation_id"]).or_else(|| text(&message["conversationId"]))
        else {
            continue;
        };
        if conversation.starts_with("local_") {
            continue;
        }
        let extra = &message["extra"];
        // FINALLY is emitted by ReplyFinish. COMPLETE alone can describe just
        // one message of an ongoing Agent turn (including delegated work).
        let ended = extra["stage"].as_i64() == Some(4);
        let streaming = message["contentStatus"].as_i64() == Some(2);
        if !ended && !streaming {
            continue;
        }
        let phase = objects(&extra["content_blocks_v2"])
            .into_iter()
            .rev()
            .find(|b| b["is_finish"] == false)
            .and_then(|b| b["block_type"].as_u64())
            .and_then(|kind| match kind {
                10040 => Some("thinking"),
                10000 => Some("responding"),
                _ => None,
            });
        let name = format!(
            "豆包工作任务 · {}",
            conversation
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
        rows.push(json!({"conversation":conversation,"session":session,"created":created,"name":name,"state":if ended {"ended"} else {"running"},"phase":phase,
            "progress":{"turn":session,"seq":extra["chunk_seq"],"stage":extra["stage"],"state":message["contentStatus"]}}));
    }
    rows.sort_by_key(|row| row["created"].as_u64().unwrap_or(0));
    Ok(Value::Array(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn checkpoint(stage: i64, status: i64) -> Value {
        json!({"version":2,"state":{"mainTaskDataMap":{"turn":{"sessionId":"turn","createTime":1793074433009u64,"receivedMessages":{"m":{"conversationId":"conversation","contentStatus":status,"extra":{"conversation_id":"conversation","stage":stage,"chunk_seq":3}}}}},"subTaskDataMap":{"ignored":{"password":"never copy"}}}})
    }
    #[test]
    fn active_checkpoint_does_not_copy_private_payloads_or_children() {
        let p = project_checkpoint(&checkpoint(2, 2)).unwrap();
        assert_eq!(p.as_array().unwrap().len(), 1);
        assert_eq!(p[0]["state"], "running");
        assert!(!p.to_string().contains("password"));
    }
    #[test]
    fn message_completion_is_not_turn_end() {
        assert!(project_checkpoint(&checkpoint(2, 1))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            project_checkpoint(&checkpoint(4, 1)).unwrap()[0]["state"],
            "ended"
        );
    }
    #[test]
    fn missing_checkpoint_keeps_stale_state_without_completion_or_approval() {
        let mut p = DoubaoProvider::new(PathBuf::new());
        let now = SystemTime::now();
        let active = project_checkpoint(&checkpoint(2, 2)).unwrap();
        let first = p.observe(active.clone(), now).unwrap();
        let again = p.observe(active, now + Duration::from_secs(2)).unwrap();
        assert_eq!(first.tasks[0].updated_at, again.tasks[0].updated_at);
        let missing = p.observe(json!([]), now).unwrap();
        assert!(!missing.authoritative);
        assert!(missing.tasks[0].stale);
        assert_eq!(missing.tasks[0].status, TaskStatus::Running);
        assert!(missing.tasks[0].input_request_id.is_none());
        let ended = p
            .observe(project_checkpoint(&checkpoint(4, 1)).unwrap(), now)
            .unwrap();
        assert_eq!(
            ended.tasks[0].waiting_reason,
            Some(WaitingReason::TurnEnded)
        );
        assert!(!ended.tasks[0].stale);
    }
    #[test]
    fn terminal_history_is_not_imported() {
        let mut p = DoubaoProvider::new(PathBuf::new());
        assert!(p
            .observe(
                project_checkpoint(&checkpoint(4, 1)).unwrap(),
                SystemTime::now()
            )
            .unwrap()
            .tasks
            .is_empty());
    }
    #[test]
    #[ignore = "read-only real Doubao Desktop data; opt in locally"]
    fn live_doubao_snapshot() {
        let mut p = DoubaoProvider::new(default_root());
        let out = p.poll(SystemTime::now());
        assert!(out.authoritative, "{:?}", out.error);
        println!(
            "doubao tasks={} running={}",
            out.tasks.len(),
            out.tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Running)
                .count()
        );
        assert!(out
            .tasks
            .iter()
            .all(|t| t.source == TaskSource::Doubao && t.session_id.is_some()));
    }
}
