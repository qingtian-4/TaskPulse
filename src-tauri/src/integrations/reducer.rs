use super::schema::{Envelope, Kind};
use crate::domain::{
    DetectionConfidence, ExecutionPhase, TaskSnapshot, TaskSource, TaskStatus, WaitingReason,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub turn: String,
    pub request: String,
    pub call: Option<String>,
    pub reason: WaitingReason,
    pub confirmed_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    identity_encoding: u8,
    pub task: Option<TaskSnapshot>,
    pub seen: BTreeSet<String>,
    pub turns: BTreeSet<String>,
    pub terminal: BTreeSet<String>,
    pub calls: BTreeMap<String, String>,
    pub pending: BTreeMap<String, Pending>,
    pub last_received_at: u64,
}

fn request_key(turn: &str, request: &str) -> String {
    serde_json::to_string(&(turn, request)).unwrap_or_default()
}

impl Session {
    pub fn supported_encoding(&self) -> bool {
        self.identity_encoding <= 1
    }
    pub fn normalize_identities(&mut self) -> bool {
        if self.identity_encoding == 0 {
            self.seen = std::mem::take(&mut self.seen)
                .into_iter()
                .map(|id| super::key(&id))
                .collect();
            self.turns = std::mem::take(&mut self.turns)
                .into_iter()
                .map(|id| super::key(&id))
                .collect();
            self.terminal = std::mem::take(&mut self.terminal)
                .into_iter()
                .map(|id| super::key(&id))
                .collect();
            self.calls = std::mem::take(&mut self.calls)
                .into_iter()
                .map(|(id, label)| (super::key(&id), label.chars().take(256).collect()))
                .collect();
            self.identity_encoding = 1;
            return true;
        }
        false
    }
    pub fn has_turn(&self, id: &str) -> bool {
        self.turns.contains(&if self.identity_encoding == 0 {
            id.into()
        } else {
            super::key(id)
        })
    }
    pub fn apply(
        &mut self,
        envelope: &Envelope,
        display_name: &str,
    ) -> Result<Option<TaskSnapshot>, String> {
        let e = &envelope.event;
        if !self.supported_encoding() {
            return Err("unsupported_checkpoint_encoding".into());
        }
        self.normalize_identities();
        let event_key = super::key(&e.event_id);
        if self.seen.contains(&event_key) {
            return Ok(None);
        }
        // Retention must not silently erase dedup or unresolved state.
        let recovery = matches!(
            e.kind,
            Kind::Resolved
                | Kind::ToolFinished
                | Kind::Completed
                | Kind::Failed
                | Kind::Interrupted
        );
        if self.seen.len() >= 100_000 && !recovery
            || e.kind == Kind::Started
                && self.turns.len() >= 4096
                && e.turn_id.as_ref().is_none_or(|t| !self.has_turn(t))
            || e.kind == Kind::Requested
                && self.pending.len() >= 128
                && !self.pending.contains_key(&request_key(
                    e.turn_id.as_deref().unwrap_or(""),
                    e.request_id.as_deref().unwrap_or(""),
                ))
            || e.kind == Kind::ToolStarted
                && self.calls.len() >= 128
                && e.call_id
                    .as_ref()
                    .is_none_or(|id| !self.calls.contains_key(&super::key(id)))
        {
            return Err("session_state_limit_record_quarantined".into());
        }
        // Recovery facts are naturally idempotent (matching pending/call IDs,
        // terminal-turn set). Never let dedup saturation prevent their cleanup,
        // and never erase the old ledger to make room for new admissions.
        if self.seen.len() < 100_000 {
            self.seen.insert(event_key);
        }
        self.last_received_at = envelope.received_at;
        if e.kind == Kind::Test {
            return Ok(None);
        }
        let metadata_only = matches!(
            e.kind,
            Kind::Discovered
                | Kind::Renamed
                | Kind::Heartbeat
                | Kind::Closed
                | Kind::Submitted
                | Kind::Unassociated
        );
        if !metadata_only
            && e.kind != Kind::Started
            && self
                .task
                .as_ref()
                .is_none_or(|t| t.turn_key.as_deref() != e.turn_id.as_deref())
        {
            // Old-turn cleanup is causally associated by explicit turn/request
            // IDs. Release its retained resources without changing the current
            // task or notifying a completion for the current turn.
            if e.kind == Kind::Resolved {
                let key = request_key(
                    e.turn_id.as_deref().unwrap_or(""),
                    e.request_id.as_deref().unwrap_or(""),
                );
                if self
                    .pending
                    .get(&key)
                    .is_some_and(|p| p.call.is_none() || e.call_id.is_none() || p.call == e.call_id)
                {
                    self.pending.remove(&key);
                }
            } else if matches!(e.kind, Kind::Completed | Kind::Failed | Kind::Interrupted) {
                if let Some(turn) = e.turn_id.as_ref().filter(|turn| self.has_turn(turn)) {
                    self.pending.retain(|_, p| p.turn != *turn);
                    self.terminal.insert(super::key(turn));
                }
            }
            return Ok(None);
        }
        if !metadata_only
            && e.turn_id.as_ref().is_some_and(|turn| {
                self.terminal.contains(&super::key(turn))
                    || (e.kind == Kind::Started && self.has_turn(turn))
            })
        {
            return Ok(None);
        }
        let source = &envelope.source_instance_id;
        let at = e
            .occurred_at
            .unwrap_or(envelope.received_at)
            .min(envelope.received_at);
        let now = UNIX_EPOCH + Duration::from_millis(at);
        let task = self.task.get_or_insert_with(|| {
            let mut task = TaskSnapshot::new(
                format!(
                    "integration:{}:{}:{}",
                    super::key(source),
                    super::key(&e.agent_id),
                    super::key(&e.session_id)
                ),
                format!(
                    "未命名任务 · {}",
                    super::key(&e.session_id)
                        .chars()
                        .take(8)
                        .collect::<String>()
                ),
                TaskSource::GenericEvent,
                TaskStatus::Unknown,
                DetectionConfidence::Inferred,
                now,
            );
            task.source_instance = Some(source.clone());
            task.agent_id = Some(e.agent_id.clone());
            task.agent_name = Some(display_name.into());
            task.session_id = Some(e.session_id.clone());
            task
        });
        task.evidence_channel = Some(envelope.evidence.clone());
        // Hook transport provenance is observational, not a cryptographic proof
        // of product identity. Imported/self-reported channels never claim Hook.
        task.confidence = if envelope.evidence == "documented_hook" {
            DetectionConfidence::Hook
        } else {
            DetectionConfidence::Inferred
        };
        // Current native envelopes may carry a renamed sidebar title on a
        // heartbeat. Metadata does not renew execution activity. Old-turn facts
        // are rejected above before they can roll the title/project backwards.
        if e.kind != Kind::Unassociated
            && (e.turn_id.is_none()
                || e.kind == Kind::Started
                || task.turn_key.as_deref() == e.turn_id.as_deref())
        {
            if let Some(value) = &e.payload.title {
                task.name = value.clone();
            }
            if let Some(value) = &e.payload.project {
                task.project = Some(value.clone());
            }
            if let Some(value) = &e.payload.project_path {
                task.project_path = Some(value.clone());
                if task.project.is_none() {
                    task.project = value
                        .trim_end_matches(['/', '\\'])
                        .rsplit(['/', '\\'])
                        .next()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                }
            }
            if let Some(value) = &e.payload.model {
                task.model = Some(value.clone());
            }
        }
        task.new_execution = false;
        if matches!(
            e.kind,
            Kind::Discovered
                | Kind::Renamed
                | Kind::Heartbeat
                | Kind::Closed
                | Kind::Submitted
                | Kind::Unassociated
        ) {
            if e.kind == Kind::Unassociated {
                task.message =
                    Some("宿主事件缺少可靠轮次/调用关联；保留原状态，不推断结束或授权".into());
            }
            task.integration_epoch = Some(envelope.journal_epoch.clone());
            task.integration_sequence = envelope.sequence;
            return Ok(Some(task.clone()));
        }
        let turn = e.turn_id.as_deref().ok_or("turn_required")?;
        if e.kind == Kind::Started {
            if self.turns.contains(&super::key(turn)) {
                return Ok(None);
            }
            self.turns.insert(super::key(turn));
            task.turn_key = Some(turn.into());
            task.status = TaskStatus::Running;
            task.started_at = Some(now);
            task.finished_at = None;
            task.phase = Some(e.payload.phase.unwrap_or(ExecutionPhase::Thinking));
            task.activity = e.payload.activity.clone();
            task.waiting_reason = None;
            task.message = None;
            task.outcome = None;
            task.new_execution = true;
            task.updated_at = now;
            self.calls.clear();
            if let Some(call) = &e.call_id {
                self.calls.insert(
                    super::key(call),
                    task.activity.clone().unwrap_or_else(|| "工具执行".into()),
                );
            }
        } else if task.turn_key.as_deref() != Some(turn)
            || self.terminal.contains(&super::key(turn))
        {
            // Receipt order is not causal order. Late old-turn events cannot
            // overwrite a new turn or resurrect a terminal turn.
            return Ok(None);
        } else {
            match e.kind {
                Kind::Phase => {
                    task.phase = e.payload.phase;
                    task.activity = e.payload.activity.clone();
                }
                Kind::ToolStarted => {
                    self.calls.insert(
                        super::key(e.call_id.as_ref().unwrap()),
                        e.payload
                            .activity
                            .as_ref()
                            .map(|s| s.chars().take(256).collect())
                            .unwrap_or_else(|| "工具执行".into()),
                    );
                    task.phase = Some(ExecutionPhase::Tool);
                    task.activity = self.calls.values().next().cloned();
                }
                Kind::ToolFinished => {
                    self.calls.remove(&super::key(e.call_id.as_ref().unwrap()));
                    task.activity = self.calls.values().next().cloned();
                    task.phase = Some(if self.calls.is_empty() {
                        ExecutionPhase::Thinking
                    } else {
                        ExecutionPhase::Tool
                    });
                    // A tool result (including failure) never resolves an input
                    // request or ends a turn on its own.
                }
                Kind::Requested => {
                    let request = e.request_id.clone().unwrap();
                    self.pending.insert(
                        request_key(turn, &request),
                        Pending {
                            turn: turn.into(),
                            request,
                            call: e.call_id.clone(),
                            reason: e.payload.request_kind.unwrap(),
                            confirmed_at: envelope.received_at,
                        },
                    );
                }
                Kind::Resolved => {
                    if self
                        .pending
                        .get(&request_key(turn, e.request_id.as_ref().unwrap()))
                        .is_some_and(|p| {
                            p.call.is_some() && e.call_id.is_some() && p.call != e.call_id
                        })
                    {
                        return Ok(None);
                    }
                    let pending = self
                        .pending
                        .remove(&request_key(turn, e.request_id.as_ref().unwrap()));
                    if e.payload.phase.is_some()
                        && pending
                            .as_ref()
                            .is_some_and(|p| p.call.is_some() && p.call == e.call_id)
                    {
                        self.calls.remove(&super::key(e.call_id.as_ref().unwrap()));
                        task.activity = self.calls.values().next().cloned();
                        task.phase = Some(if self.calls.is_empty() {
                            ExecutionPhase::Thinking
                        } else {
                            ExecutionPhase::Tool
                        });
                    }
                }
                Kind::Completed | Kind::Failed | Kind::Interrupted => {
                    self.terminal.insert(super::key(turn));
                    self.calls.clear();
                    self.pending.retain(|_, p| p.turn != turn);
                    task.status = if e.kind == Kind::Failed {
                        TaskStatus::Failed
                    } else {
                        TaskStatus::WaitingInput
                    };
                    task.waiting_reason = Some(WaitingReason::TurnEnded);
                    task.finished_at = Some(now);
                    task.activity = None;
                    task.phase = None;
                    task.outcome = Some(
                        if e.kind == Kind::Interrupted {
                            "interrupted"
                        } else if e.kind == Kind::Failed {
                            "failed"
                        } else {
                            "turn_ended"
                        }
                        .into(),
                    );
                    task.message = Some(
                        if e.kind == Kind::Interrupted {
                            "本轮已中断，非成功"
                        } else if e.kind == Kind::Failed {
                            "来源明确报告本轮失败"
                        } else {
                            "本轮结束，尚未查看"
                        }
                        .into(),
                    );
                }
                Kind::StopRequested => {
                    task.integration_epoch = Some(envelope.journal_epoch.clone());
                    task.integration_sequence = envelope.sequence;
                    return Ok(Some(task.clone()));
                }
                _ => {}
            }
            // Only observed execution advances the activity window. Metadata,
            // heartbeat, stop intent and request refresh do not renew it.
            if !matches!(e.kind, Kind::Requested | Kind::Resolved) {
                task.updated_at = now;
            }
        }
        task.integration_epoch = Some(envelope.journal_epoch.clone());
        task.integration_sequence = envelope.sequence;
        task.input_request_ids = self
            .pending
            .values()
            .filter(|p| p.turn == turn)
            .map(|p| p.request.clone())
            .collect();
        task.input_request_id = task.input_request_ids.first().cloned();
        task.pending_confirmed_at = None;
        if let Some(p) = self.pending.values().find(|p| p.turn == turn) {
            task.status = TaskStatus::WaitingInput;
            task.waiting_reason = Some(p.reason);
            task.pending_confirmed_at = Some(p.confirmed_at);
        } else if !self.terminal.contains(&super::key(turn)) {
            task.status = TaskStatus::Running;
            task.waiting_reason = None;
        }
        Ok(Some(task.clone()))
    }
}
