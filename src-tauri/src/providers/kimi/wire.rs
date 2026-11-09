//! Pure Kimi wire-event reduction; no filesystem or host actions.
use super::{stamp, text};
use crate::domain::{
    DetectionConfidence, ExecutionPhase, TaskSnapshot, TaskSource, TaskStatus, WaitingReason,
};
use crate::providers::reported_model;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};
fn native_id(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|value| {
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_u64().map(|n| n.to_string()))
    })
}
#[derive(Default)]
pub(super) struct WireState {
    pub(super) task: Option<TaskSnapshot>,
    pub(super) pending: BTreeMap<String, WaitingReason>,
    pub(super) tools: HashSet<String>,
    pub(super) settled: HashSet<String>,
    pub(super) line: u64,
    pub(super) prompt_start: bool,
    pub(super) recognized: bool,
    pub(super) uncertain: bool,
    pub(super) native_turn: Option<String>,
    pub(super) request_calls: HashMap<String, String>,
    pub(super) cwd: Option<String>,
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
    pub(super) fn apply(&mut self, v: &Value, id: &str) -> Result<(), String> {
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
                .and_then(reported_model)
            {
                task.model = Some(model);
            }
            task.reasoning_effort = text(data, "thinkingEffort").and_then(reported_model);
        }
        if matches!(
            kind,
            "context.append_message" | "agent.message.appended" | "human.agent.message.appended"
        ) {
            if let Some(model) = data
                .pointer("/message/meta/model/model")
                .or_else(|| data.pointer("/message/message/model"))
                .and_then(Value::as_str)
                .and_then(reported_model)
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
