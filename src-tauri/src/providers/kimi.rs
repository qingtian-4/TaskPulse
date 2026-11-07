//! Read-only Kimi Code and legacy kimi-cli wire journals. No config/credentials.
use super::{
    jsonl_tail::JsonlTail,
    native_sessions::{self, Sessions},
    Provider, ProviderOutput,
};
use crate::domain::{
    DetectionConfidence, ExecutionPhase, TaskSnapshot, TaskSource, TaskStatus, WaitingReason,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn default_roots() -> Vec<PathBuf> {
    let home = dirs_next::home_dir().unwrap_or_default();
    vec![
        std::env::var_os("KIMI_CODE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".kimi-code")),
        std::env::var_os("KIMI_SHARE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".kimi")),
    ]
}
pub fn available(root: &Path) -> bool {
    root.join("sessions").is_dir()
}
pub(super) fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)?.as_str().filter(|s| !s.is_empty())
}
fn native_id(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|value| {
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_u64().map(|n| n.to_string()))
    })
}
pub(super) fn stamp(v: &Value) -> Option<SystemTime> {
    if let Some(s) = v.as_str() {
        return super::codex::parse_rfc3339(s);
    }
    let n = v.as_f64()?;
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    let ms = if n < 1e11 { n * 1000.0 } else { n };
    native_sessions::time(ms as u64)
}
pub(super) fn read_json(path: &Path) -> Result<Value, String> {
    let mut f = std::fs::File::open(path).map_err(|_| "metadata_read")?;
    use std::io::Read;
    let mut b = vec![];
    f.by_ref()
        .take(256 * 1024 + 1)
        .read_to_end(&mut b)
        .map_err(|_| "metadata_read")?;
    if b.len() > 256 * 1024 {
        return Err("metadata_budget".into());
    }
    serde_json::from_slice(&b).map_err(|_| "metadata_json".into())
}

#[derive(Default)]
struct WireState {
    task: Option<TaskSnapshot>,
    pending: BTreeMap<String, WaitingReason>,
    tools: HashSet<String>,
    settled: HashSet<String>,
    line: u64,
    prompt_start: bool,
    recognized: bool,
    uncertain: bool,
    native_turn: Option<String>,
    request_calls: HashMap<String, String>,
    cwd: Option<String>,
}
impl WireState {
    fn begin(&mut self, id: &str, key: String, at: SystemTime) {
        let mut task = TaskSnapshot::new(
            format!("kimi-code:{id}"),
            "",
            TaskSource::NativeAgent,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            at,
        );
        task.agent_id = Some("kimi-code".into());
        task.agent_name = Some("Kimi Code".into());
        task.session_id = Some(id.into());
        task.cli = Some("kimi".into());
        task.evidence_channel = Some("native_wire".into());
        task.started_at = Some(at);
        task.turn_key = Some(key);
        task.phase = Some(ExecutionPhase::AwaitingModel);
        if let Some(old) = &self.task {
            task.name = old.name.clone();
            task.project = old.project.clone();
            task.project_path = old.project_path.clone();
            task.model = old.model.clone();
            task.reasoning_effort = old.reasoning_effort.clone();
        }
        self.task = Some(task);
        if let Some(cwd) = &self.cwd {
            self.task.as_mut().unwrap().project_path = Some(cwd.clone());
        }
        self.pending.clear();
        self.tools.clear();
        self.settled.clear();
        self.request_calls.clear();
        self.uncertain = false;
    }
    fn apply(&mut self, v: &Value, id: &str) -> Result<(), String> {
        self.line += 1;
        if text(v, "sessionId")
            .or_else(|| text(v, "session_id"))
            .is_some_and(|s| s != id)
        {
            return Err("session_mismatch".into());
        }
        let event = if text(v, "type") == Some("context.append_loop_event") {
            v.get("event").unwrap_or(v)
        } else {
            v
        };
        let envelope = event
            .get("message")
            .filter(|m| m.get("payload").is_some() && m.get("type").is_some())
            .unwrap_or(event);
        let kind = text(envelope, "type").unwrap_or("");
        if kind == "metadata" {
            if text(envelope, "protocol_version").is_some_and(|version| {
                !matches!(version, "1.0" | "1.1" | "1.2" | "1.3" | "1.4" | "1.5")
            }) {
                return Err("unsupported_wire_version".into());
            }
            self.recognized = true;
            return Ok(());
        }
        if kind == "SubagentEvent" || text(v, "agentId").is_some_and(|a| a != "main") {
            return Ok(());
        }
        let data = envelope.get("payload").unwrap_or(envelope);
        if text(data, "agentId").is_some_and(|agent| agent != "main") {
            return Ok(());
        }
        if let Some(cwd) = data
            .pointer("/environmentDisclosure/cwd")
            .and_then(Value::as_str)
            .or_else(|| text(data, "cwd"))
        {
            if cwd.len() > 32768 {
                return Err("wire_path_budget".into());
            }
            self.cwd = Some(cwd.into());
        }
        if text(data, "source_kind") == Some("background_agent") {
            return Ok(());
        }
        let at = v.get("time").or_else(|| v.get("timestamp")).and_then(stamp);
        let critical = matches!(
            kind,
            "TurnBegin"
                | "TurnEnd"
                | "ApprovalRequest"
                | "ApprovalResponse"
                | "QuestionRequest"
                | "QuestionResponse"
                | "turn.prompt"
                | "turn.started"
                | "turn.ended"
                | "turn.cancel"
                | "event.approval.requested"
                | "event.approval.resolved"
                | "event.question.requested"
                | "event.question.answered"
                | "event.question.dismissed"
                | "interaction.request"
                | "interaction.resolved"
        );
        if critical {
            self.recognized = true;
        }
        if matches!(kind, "TurnBegin" | "turn.prompt" | "turn.started") {
            let at = at.ok_or("wire_timestamp")?;
            if kind == "turn.started" && self.prompt_start {
                self.prompt_start = false;
                self.native_turn = native_id(data, "turnId").or_else(|| native_id(data, "turn_id"));
                return Ok(());
            }
            let native = native_id(data, "turnId").or_else(|| native_id(data, "turn_id"));
            let key = native
                .as_ref()
                .map(|id| {
                    format!(
                        "turn:{id}:{}",
                        at.duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "wire:{:x}",
                        Sha256::digest(
                            serde_json::to_vec(&serde_json::json!([
                                kind,
                                v.get("time").or_else(|| v.get("timestamp")),
                                data.get("user_input")
                                    .or_else(|| data.get("prompt"))
                                    .or_else(|| data.get("input"))
                            ]))
                            .unwrap_or_default()
                        )
                    )
                });
            if self
                .task
                .as_ref()
                .is_some_and(|task| task.turn_key.as_deref() == Some(&key))
            {
                return Ok(());
            }
            self.begin(id, key, at);
            self.native_turn = native;
            self.prompt_start = kind == "turn.prompt";
            if let Some(prompt) = text(data, "user_input")
                .or_else(|| text(data, "prompt"))
                .or_else(|| {
                    data.get("user_input")
                        .and_then(Value::as_array)
                        .and_then(|parts| parts.iter().find_map(|p| text(p, "text")))
                })
                .or_else(|| {
                    data.get("input")
                        .and_then(Value::as_array)
                        .and_then(|parts| parts.iter().find_map(|p| text(p, "text")))
                })
            {
                if let Some(t) = self.task.as_mut() {
                    t.name = prompt
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(72)
                        .collect();
                }
            }
            return Ok(());
        }
        let Some(task) = self.task.as_mut() else {
            return if critical {
                Err("wire_missing_begin".into())
            } else {
                Ok(())
            };
        };
        if task.finished_at.is_some() {
            return Ok(());
        }
        if native_id(data, "turnId")
            .or_else(|| native_id(data, "turn_id"))
            .zip(self.native_turn.clone())
            .is_some_and(|(incoming, current)| incoming != current)
        {
            return Ok(());
        }
        if let Some(at) = at {
            task.updated_at = task.updated_at.max(at);
        }
        match kind {
            "TurnEnd" | "turn.ended" => {
                task.status = if text(data, "status") == Some("failed")
                    || text(data, "reason") == Some("failed")
                {
                    TaskStatus::Failed
                } else {
                    TaskStatus::WaitingInput
                };
                task.waiting_reason =
                    (task.status == TaskStatus::WaitingInput).then_some(WaitingReason::TurnEnded);
                task.finished_at = Some(at.ok_or("wire_end_timestamp")?);
                task.phase = None;
                self.pending.clear();
                self.tools.clear();
                self.request_calls.clear();
                self.settled.clear();
                self.prompt_start = false;
                self.uncertain = false;
                if text(data, "reason") == Some("cancelled") {
                    task.outcome = Some("interrupted".into());
                }
            }
            "ApprovalRequest"
            | "QuestionRequest"
            | "event.approval.requested"
            | "event.question.requested" => {
                let request = text(data, "id")
                    .or_else(|| text(data, "approval_id"))
                    .or_else(|| text(data, "question_id"))
                    .ok_or("wire_request_id")?;
                if request.trim().is_empty()
                    || request.chars().any(char::is_control)
                    || request.len() > 240
                    || self.pending.len() >= 256 && !self.pending.contains_key(request)
                {
                    return Err("wire_request_budget".into());
                }
                let reason = if matches!(kind, "ApprovalRequest" | "event.approval.requested") {
                    WaitingReason::Approval
                } else {
                    WaitingReason::Question
                };
                if !self.settled.contains(request) {
                    self.pending.insert(request.into(), reason);
                    if let Some(call) = text(data, "tool_call_id") {
                        if call.len() > 240 {
                            return Err("wire_tool_id".into());
                        }
                        self.request_calls.insert(request.into(), call.into());
                    }
                }
            }
            "ApprovalResponse"
            | "QuestionResponse"
            | "event.approval.resolved"
            | "event.question.answered"
            | "event.question.dismissed" => {
                let request = text(data, "request_id")
                    .or_else(|| text(data, "approval_id"))
                    .or_else(|| text(data, "question_id"))
                    .ok_or("wire_resolution_id")?;
                // Response records, never a tool return/queued ACK, settle native requests.
                let valid = match kind {
                    "ApprovalResponse" => matches!(
                        text(data, "response"),
                        Some("approve" | "approve_for_session" | "reject")
                    ),
                    "QuestionResponse" => data.get("answers").is_some_and(Value::is_object),
                    _ => true,
                };
                if valid && self.pending.remove(request).is_some() {
                    if self.settled.len() >= 4096 {
                        return Err("wire_request_budget".into());
                    }
                    self.settled.insert(request.into());
                    self.request_calls.remove(request);
                }
            }
            "interaction.request" => {
                let reason = match text(data, "kind") {
                    Some("approval") => WaitingReason::Approval,
                    Some("question") => WaitingReason::Question,
                    Some("user_tool") => return Ok(()),
                    _ => return Err("wire_interaction_kind".into()),
                };
                let id = text(data, "id")
                    .filter(|id| {
                        id.len() <= 240
                            && !id.trim().is_empty()
                            && !id.chars().any(char::is_control)
                    })
                    .ok_or("wire_request_id")?;
                if self.pending.len() >= 256 && !self.pending.contains_key(id) {
                    return Err("wire_request_budget".into());
                }
                if !self.settled.contains(id) {
                    self.pending.insert(id.into(), reason);
                    if let Some(call) = text(data, "toolCallId").filter(|s| s.len() <= 240) {
                        self.request_calls.insert(id.into(), call.into());
                    }
                }
            }
            "interaction.resolved" => {
                let id = text(data, "id").ok_or("wire_resolution_id")?;
                if self.pending.remove(id).is_some() {
                    if self.settled.len() >= 4096 {
                        return Err("wire_request_budget".into());
                    }
                    self.settled.insert(id.into());
                    self.request_calls.remove(id);
                }
            }
            "StepBegin" | "step.begin" | "turn.step.started" | "llm.request" => {
                if self.native_turn.is_none() {
                    self.native_turn =
                        native_id(data, "turnId").or_else(|| native_id(data, "turn_id"));
                }
                if !task.status.is_terminal() && task.finished_at.is_none() {
                    task.phase = Some(ExecutionPhase::AwaitingModel);
                }
            }
            "ToolCall" | "ToolCallRequest" | "tool.call" | "tool.call.started" => {
                if let Some(call) = text(data, "id")
                    .or_else(|| text(data, "tool_call_id"))
                    .or_else(|| text(data, "callId"))
                {
                    if call.len() > 240 || call.chars().any(char::is_control) {
                        return Err("wire_tool_id".into());
                    }
                    if self.tools.len() >= 512 && !self.tools.contains(call) {
                        return Err("wire_tool_budget".into());
                    }
                    self.tools.insert(call.into());
                    task.phase = Some(ExecutionPhase::Tool);
                }
            }
            "ToolResult" | "tool.result" => {
                if let Some(call) = text(data, "tool_call_id").or_else(|| text(data, "callId")) {
                    self.tools.remove(call);
                    if data
                        .pointer("/return_value/is_error")
                        .or_else(|| data.get("is_error"))
                        .and_then(Value::as_bool)
                        == Some(true)
                    {
                        let rejected: Vec<_> = self
                            .request_calls
                            .iter()
                            .filter(|(request, owner)| {
                                owner.as_str() == call
                                    && self.pending.get(*request) == Some(&WaitingReason::Question)
                            })
                            .map(|(request, _)| request.clone())
                            .collect();
                        for request in rejected {
                            if self.settled.len() >= 4096 {
                                return Err("wire_request_budget".into());
                            }
                            self.pending.remove(&request);
                            self.request_calls.remove(&request);
                            self.settled.insert(request);
                        }
                    }
                }
                if self.tools.is_empty() && task.finished_at.is_none() {
                    task.phase = Some(ExecutionPhase::AwaitingModel);
                }
            }
            _ => {}
        }
        if kind == "llm.request" && text(data, "kind") != Some("compaction") {
            if let Some(model) = text(data, "modelAlias")
                .or_else(|| text(data, "model"))
                .and_then(super::reported_model)
            {
                task.model = Some(model);
            }
            task.reasoning_effort = text(data, "thinkingEffort").and_then(super::reported_model);
        }
        if matches!(
            kind,
            "context.append_message" | "agent.message.appended" | "human.agent.message.appended"
        ) {
            if let Some(model) = data
                .pointer("/message/meta/model/model")
                .or_else(|| data.pointer("/message/message/model"))
                .and_then(Value::as_str)
                .and_then(super::reported_model)
            {
                task.model = Some(model);
            }
        }
        if let Some(cwd) = v
            .pointer("/environmentDisclosure/cwd")
            .and_then(Value::as_str)
        {
            if cwd.len() > 32768 {
                return Err("wire_path_budget".into());
            }
            task.project_path = Some(cwd.into());
        }
        if task.finished_at.is_none() {
            if self.pending.is_empty() {
                if task
                    .waiting_reason
                    .is_some_and(|r| matches!(r, WaitingReason::Question | WaitingReason::Approval))
                {
                    task.status = TaskStatus::Running;
                    task.waiting_reason = None;
                    task.phase = Some(if self.tools.is_empty() {
                        ExecutionPhase::AwaitingModel
                    } else {
                        ExecutionPhase::Tool
                    });
                }
            } else {
                task.status = TaskStatus::WaitingInput;
                task.waiting_reason = Some(
                    if self.pending.values().any(|r| *r == WaitingReason::Approval) {
                        WaitingReason::Approval
                    } else {
                        WaitingReason::Question
                    },
                );
                task.phase = None;
            }
        }
        task.input_request_ids = self.pending.keys().cloned().collect();
        task.input_request_id = task.input_request_ids.first().cloned();
        Ok(())
    }
}
struct Journal {
    tail: JsonlTail,
    state: WireState,
    invalid: bool,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            tail: JsonlTail::newline_committed(),
            state: WireState::default(),
            invalid: false,
        }
    }
}
pub struct KimiProvider {
    roots: Vec<PathBuf>,
    logs: HashMap<PathBuf, Journal>,
    sessions: Sessions,
}
impl KimiProvider {
    pub fn new(location: Option<PathBuf>) -> Self {
        Self {
            roots: location.map(|p| vec![p]).unwrap_or_else(default_roots),
            logs: HashMap::new(),
            sessions: Default::default(),
        }
    }
    pub fn resume(&mut self, tasks: impl IntoIterator<Item = TaskSnapshot>) {
        self.sessions.resume("kimi-code", tasks);
    }
    fn read(&mut self) -> Result<Vec<TaskSnapshot>, String> {
        let started = Instant::now();
        let mut files = vec![];
        let mut seen = HashSet::new();
        let mut visited = 0usize;
        let mut workdirs = vec![];
        for root in &self.roots {
            let meta = root.join("kimi.json");
            if meta.exists() {
                let meta = read_json(&meta)?;
                if let Some(dirs) = meta.get("work_dirs").and_then(Value::as_array) {
                    for wd in dirs {
                        if let (Some(id), Some(path)) =
                            (text(wd, "last_session_id"), text(wd, "path"))
                        {
                            workdirs.push((id.to_string(), path.to_string()));
                        }
                    }
                }
            }
            let base = root.join("sessions");
            if !base.try_exists().map_err(|_| "session_listing")? {
                continue;
            }
            for w in std::fs::read_dir(base).map_err(|_| "session_listing")? {
                visited += 1;
                if visited > 4096 || started.elapsed() > Duration::from_millis(500) {
                    return Err("session_listing_budget".into());
                }
                let w = w.map_err(|_| "session_listing")?;
                if !w.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                    continue;
                }
                for s in std::fs::read_dir(w.path()).map_err(|_| "session_listing")? {
                    visited += 1;
                    if visited > 4096 || started.elapsed() > Duration::from_millis(500) {
                        return Err("session_listing_budget".into());
                    }
                    let s = s.map_err(|_| "session_listing")?;
                    if !s.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                        continue;
                    }
                    let id = s.file_name().to_string_lossy().into_owned();
                    let modern = s.path().join("agents/main/wire.jsonl");
                    let path = if modern.is_file() {
                        modern
                    } else {
                        s.path().join("wire.jsonl")
                    };
                    if path.is_file() {
                        if !seen.insert(id.clone()) {
                            return Err("duplicate_session_id".into());
                        }
                        files.push((id, path, s.path()));
                        if files.len() > 512 {
                            return Err("session_admission_budget".into());
                        }
                    }
                }
            }
        }
        let mut buckets = HashMap::new();
        for (last, cwd) in workdirs {
            if let Some((_, _, dir)) = files.iter().find(|(id, _, _)| id == &last) {
                if let Some(parent) = dir.parent() {
                    buckets.insert(parent.to_path_buf(), cwd);
                }
            }
        }
        files.sort_by_key(|(_, p, _)| {
            std::cmp::Reverse(
                p.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH),
            )
        });
        let mut tasks = vec![];
        let mut bytes = 0usize;
        let mut metadata_bytes = 0u64;
        let mut live = HashSet::new();
        let mut incomplete = false;
        for (id, path, dir) in files {
            live.insert(path.clone());
            if bytes >= 8 * 1024 * 1024 || started.elapsed() > Duration::from_millis(500) {
                incomplete = true;
                break;
            }
            if !self.logs.contains_key(&path)
                && self
                    .logs
                    .values()
                    .filter(|l| l.tail.has_partial_line())
                    .count()
                    >= 8
            {
                incomplete = true;
                continue;
            }
            let log = self.logs.entry(path.clone()).or_default();
            let (caught_up, gap) = loop {
                let batch = log.tail.read(&path).map_err(|_| "wire_read")?;
                bytes += batch.bytes_read;
                if batch.reset {
                    log.state = Default::default();
                    log.invalid = false;
                }
                for line in batch.body.lines() {
                    match serde_json::from_str::<Value>(line) {
                        Ok(v) => {
                            if let Err(code) = log.state.apply(&v, &id) {
                                if code.starts_with("wire_") {
                                    log.state.uncertain = true;
                                } else {
                                    log.invalid = true;
                                }
                            }
                        }
                        Err(_) => log.invalid = true,
                    }
                }
                log.tail.trim_read_buffer();
                if batch.caught_up
                    || batch.bytes_read == 0
                    || bytes >= 8 * 1024 * 1024
                    || started.elapsed() > Duration::from_millis(500)
                {
                    break (batch.caught_up, batch.gap);
                }
            };
            if !caught_up || gap || log.invalid || log.state.uncertain {
                incomplete = true;
                continue;
            }
            if !log.state.recognized && log.tail.bytes_read_total > 0 {
                return Err("unsupported_wire_format".into());
            }
            if let Some(mut task) = log.state.task.clone() {
                let metadata = dir.join("state.json");
                if metadata.exists() {
                    metadata_bytes = metadata_bytes
                        .saturating_add(metadata.metadata().map_err(|_| "metadata_read")?.len());
                    if metadata_bytes > 4 * 1024 * 1024 {
                        return Err("metadata_round_budget".into());
                    }
                    let meta = read_json(&metadata)?;
                    if text(&meta, "id").is_some_and(|value| value != id) {
                        return Err("metadata_session_mismatch".into());
                    }
                    if let Some(title) =
                        text(&meta, "title").or_else(|| text(&meta, "custom_title"))
                    {
                        task.name = title.chars().take(1024).collect();
                    }
                    if let Some(cwd) = text(&meta, "cwd") {
                        task.project_path = Some(cwd.into());
                    }
                }
                if task.project_path.is_none() {
                    task.project_path = dir.parent().and_then(|p| buckets.get(p)).cloned();
                }
                if task.name.is_empty() {
                    task.name = format!("Kimi · {}", id.chars().take(8).collect::<String>());
                }
                task.project = task
                    .project_path
                    .as_ref()
                    .and_then(|p| Path::new(p).file_name())
                    .map(|p| p.to_string_lossy().into_owned());
                tasks.push(task);
            }
        }
        if incomplete {
            return Err("wire_catching_up_or_invalid".into());
        }
        self.logs.retain(|p, _| live.contains(p));
        Ok(tasks)
    }
}
impl Provider for KimiProvider {
    fn name(&self) -> &'static str {
        "kimi-code"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match self.read() {
            Ok(tasks) => self.sessions.observe(tasks, now),
            Err(reason) => self
                .sessions
                .degraded(&format!("Kimi 会话暂不可读（{reason}）；保留上次状态")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn official_v1_5_interactions_numeric_turns_and_cancel_intent() {
        let mut s = WireState::default();
        for record in [
            json!({"type":"metadata","protocol_version":"1.5","created_at":1794000000000u64}),
            json!({"type":"profile.bind","agentId":"main","environmentDisclosure":{"cwd":"C:/public/project"}}),
            json!({"type":"turn.prompt","agentId":"main","turnId":7,"input":[{"type":"text","text":"Native fixture"}],"origin":"user","time":1794000000000u64}),
            json!({"type":"interaction.request","agentId":"main","id":"approve","kind":"approval","toolCallId":"call","request":{},"time":1794000001000u64}),
            json!({"type":"interaction.request","agentId":"main","id":"question","kind":"question","request":{},"time":1794000002000u64}),
        ] {
            s.apply(&record, "s").unwrap();
        }
        assert_eq!(s.task.as_ref().unwrap().input_request_ids.len(), 2);
        s.apply(&json!({"type":"interaction.resolved","agentId":"main","id":"other","response":{},"time":1794000003000u64}),"s").unwrap();
        assert_eq!(s.pending.len(), 2);
        s.apply(&json!({"type":"interaction.resolved","agentId":"main","id":"approve","response":{"decision":"rejected"},"time":1794000004000u64}),"s").unwrap();
        assert_eq!(
            s.task.as_ref().unwrap().waiting_reason,
            Some(WaitingReason::Question)
        );
        s.apply(&json!({"type":"turn.cancel","agentId":"main","turnId":7,"target":"active","time":1794000005000u64}),"s").unwrap();
        assert!(s.task.as_ref().unwrap().finished_at.is_none());
        assert_eq!(s.pending.len(), 1);
        s.apply(&json!({"type":"turn.ended","agentId":"main","turnId":6,"reason":"completed","time":1794000006000u64}),"s").unwrap();
        assert!(s.task.as_ref().unwrap().finished_at.is_none());
        s.apply(&json!({"type":"turn.ended","agentId":"main","turnId":7,"reason":"cancelled","time":1794000061000u64}),"s").unwrap();
        assert!(s.pending.is_empty());
        assert_eq!(
            s.task.as_ref().unwrap().outcome.as_deref(),
            Some("interrupted")
        );
    }
    #[test]
    fn provider_preserves_pending_during_concurrent_write_and_recovers_at_matching_response() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("sessions/workspace/s");
        let main = session.join("agents/main");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::write(
            session.join("state.json"),
            json!({"id":"s","title":"Native title","cwd":"C:/public/project"}).to_string(),
        )
        .unwrap();
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let events = [
            json!({"type":"turn.started","time":at,"turnId":"t"}),
            json!({"type":"llm.request","time":at,"modelAlias":"native-model","thinkingEffort":"high"}),
            json!({"type":"event.question.requested","time":at,"payload":{"question_id":"q","turn_id":"t"}}),
        ];
        let log = main.join("wire.jsonl");
        std::fs::write(
            &log,
            events.iter().map(|v| format!("{v}\n")).collect::<String>(),
        )
        .unwrap();
        let mut p = KimiProvider::new(Some(dir.path().into()));
        let blocked = p.poll(SystemTime::now());
        assert!(blocked.authoritative);
        assert_eq!(blocked.tasks.len(), 1);
        assert_eq!(blocked.tasks[0].input_request_id.as_deref(), Some("q"));
        assert_eq!(blocked.tasks[0].project.as_deref(), Some("project"));
        let response=json!({"type":"event.question.answered","time":at+1,"payload":{"question_id":"q","turn_id":"t"}}).to_string();
        let mut writer = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writer.write_all(response.as_bytes()).unwrap();
        writer.flush().unwrap();
        let partial = p.poll(SystemTime::now());
        assert!(!partial.authoritative);
        assert!(partial.tasks[0].stale);
        assert_eq!(partial.tasks[0].input_request_id.as_deref(), Some("q"));
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        let resumed = p.poll(SystemTime::now());
        assert!(resumed.authoritative);
        assert_eq!(resumed.tasks[0].status, TaskStatus::Running);
        assert!(resumed.tasks[0].input_request_id.is_none());
        writer
            .write_all(
                format!(
                    "{}\n",
                    json!({"type":"turn.ended","time":at+61000,"payload":{"turn_id":"t"}})
                )
                .as_bytes(),
            )
            .unwrap();
        writer.flush().unwrap();
        let end = p.poll(SystemTime::now());
        assert_eq!(
            end.tasks[0]
                .finished_at
                .unwrap()
                .duration_since(end.tasks[0].started_at.unwrap())
                .unwrap(),
            Duration::from_secs(61)
        );
        assert_eq!(end.tasks[0].reasoning_effort.as_deref(), Some("high"));
    }
    #[test]
    fn start_identity_ignores_preceding_metadata_and_stale_other_turn_end() {
        let start = json!({"type":"turn.started","time":1794000000000u64,"turnId":"native-turn"});
        let mut a = WireState::default();
        let mut b = WireState::default();
        b.apply(&json!({"type":"metadata","protocol_version":"1.3"}), "s")
            .unwrap();
        a.apply(&start, "s").unwrap();
        b.apply(&start, "s").unwrap();
        assert_eq!(
            a.task.as_ref().unwrap().turn_key,
            b.task.as_ref().unwrap().turn_key
        );
        a.apply(
            &json!({"type":"turn.ended","time":1794000001000u64,"payload":{"turn_id":"older"}}),
            "s",
        )
        .unwrap();
        assert!(a.task.unwrap().finished_at.is_none());
    }
    #[test]
    fn tool_error_can_close_unsupported_question_but_cannot_decide_approval() {
        let mut s = WireState::default();
        let row = |kind: &str, payload: Value| json!({"timestamp":1794000000.0,"message":{"type":kind,"payload":payload}});
        for (kind, payload) in [
            ("TurnBegin", json!({"user_input":"Fixture"})),
            ("ApprovalRequest", json!({"id":"a","tool_call_id":"c"})),
            ("QuestionRequest", json!({"id":"q","tool_call_id":"c"})),
            (
                "ToolResult",
                json!({"tool_call_id":"c","return_value":{"is_error":true}}),
            ),
        ] {
            s.apply(&row(kind, payload), "s").unwrap();
        }
        assert_eq!(s.task.as_ref().unwrap().input_request_ids, vec!["a"]);
        assert_eq!(
            s.task.as_ref().unwrap().waiting_reason,
            Some(WaitingReason::Approval)
        );
    }
    #[test]
    fn native_requests_require_matching_resolution_and_end_is_explicit() {
        let mut s = WireState::default();
        let event = |kind: &str, payload: Value| json!({"timestamp":1794000000.0,"message":{"type":kind,"payload":payload}});
        s.apply(&event("TurnBegin", json!({"user_input":"Fixture"})), "s")
            .unwrap();
        s.apply(
            &event("QuestionRequest", json!({"id":"q1","tool_call_id":"c1"})),
            "s",
        )
        .unwrap();
        assert!(s.task.as_ref().unwrap().requires_action());
        s.apply(
            &event(
                "QuestionResponse",
                json!({"request_id":"other","answers":{}}),
            ),
            "s",
        )
        .unwrap();
        assert!(s.task.as_ref().unwrap().requires_action());
        s.apply(&event("StepBegin", json!({})), "s").unwrap();
        assert!(s.task.as_ref().unwrap().requires_action());
        s.apply(
            &event(
                "QuestionResponse",
                json!({"request_id":"q1","answers":{"Choice":"A"}}),
            ),
            "s",
        )
        .unwrap();
        assert_eq!(s.task.as_ref().unwrap().status, TaskStatus::Running);
        assert!(s.task.as_ref().unwrap().finished_at.is_none());
        s.apply(&event("TurnEnd", json!({})), "s").unwrap();
        assert!(s.task.as_ref().unwrap().finished_at.is_some());
    }
    #[test]
    fn modern_wire_modes_and_end_survive_tool_step_completion() {
        let mut s = WireState::default();
        for v in [
            json!({"type":"turn.prompt","time":1794000000000u64,"turnId":"t"}),
            json!({"type":"llm.request","time":1794000001000u64,"model":"kimi-for-coding","thinkingEffort":"high"}),
            json!({"type":"step.end","time":1794000002000u64}),
        ] {
            s.apply(&v, "s").unwrap();
        }
        assert_eq!(s.task.as_ref().unwrap().status, TaskStatus::Running);
        s.apply(&json!({"type":"turn.ended","time":1794000061000u64}), "s")
            .unwrap();
        let t = s.task.unwrap();
        assert_eq!(t.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(
            t.finished_at
                .unwrap()
                .duration_since(t.started_at.unwrap())
                .unwrap(),
            Duration::from_secs(61)
        );
    }
}
