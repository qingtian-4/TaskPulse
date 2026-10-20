use super::{
    key,
    schema::{Envelope, Event, Kind, Payload},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub support: String,
    pub channel: String,
    pub validation: String,
    pub tested_versions: Vec<String>,
    pub limitations: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub event: String,
    pub field: String,
    pub equals: String,
    pub kind: Kind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u8,
    pub adapter_id: String,
    pub adapter_version: String,
    pub agent_id: String,
    pub display_name: String,
    pub product_form: String,
    pub systems: Vec<String>,
    pub supported_versions: Vec<String>,
    pub official_sources: Vec<String>,
    pub fields: BTreeMap<String, String>,
    pub events: BTreeMap<String, Kind>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub capabilities: BTreeMap<String, Capability>,
}
pub fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_".contains(c))
}
fn pointer(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 128
        && value.split('/').count() <= 8
        && !value.contains(['~', '*', '[', ']'])
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.manifest_version != 1
            || !slug(&self.adapter_id)
            || !slug(&self.agent_id)
            || self.adapter_version.is_empty()
            || self.adapter_version.len() > 40
        {
            return Err("invalid_manifest_identity_or_version".into());
        }
        if self.display_name.trim().is_empty()
            || self.display_name.len() > 120
            || self.display_name.chars().any(char::is_control)
        {
            return Err("invalid_manifest_display".into());
        }
        if self.fields.len() > 16
            || self.events.len() > 48
            || self.rules.len() > 32
            || self.capabilities.len() > 16
        {
            return Err("manifest_limit".into());
        }
        for (name, path) in &self.fields {
            if ![
                "event",
                "session",
                "turn",
                "event_id",
                "request",
                "call",
                "title",
                "project",
                "project_path",
                "model",
            ]
            .contains(&name.as_str())
                || !pointer(path)
            {
                return Err("restricted_field_mapping_only".into());
            }
        }
        if !self.fields.contains_key("session") || !self.fields.contains_key("event") {
            return Err("manifest_session_and_event_required".into());
        }
        for (name, kind) in &self.events {
            if name.len() > 80
                || matches!(name.as_str(), "Stop" | "UserPromptSubmit" | "SessionStart")
                    && matches!(kind, Kind::Completed | Kind::Started | Kind::Failed)
            {
                return Err("unsafe_host_lifecycle_mapping".into());
            }
        }
        for rule in &self.rules {
            if !pointer(&rule.field)
                || rule.equals.len() > 80
                || rule.event.len() > 80
                || matches!(
                    rule.event.as_str(),
                    "Stop" | "UserPromptSubmit" | "SessionStart"
                ) && matches!(rule.kind, Kind::Completed | Kind::Started | Kind::Failed)
            {
                return Err("unsafe_manifest_rule".into());
            }
        }
        for c in self.capabilities.values() {
            if !["explicit", "inferred", "unsupported"].contains(&c.support.as_str())
                || !["documented", "fixture-tested", "native-tested"]
                    .contains(&c.validation.as_str())
                || c.limitations.len() > 2048
            {
                return Err("invalid_capability".into());
            }
        }
        if self.official_sources.len() > 8
            || self
                .official_sources
                .iter()
                .any(|s| !s.starts_with("https://") || s.len() > 1024)
        {
            return Err("invalid_documentation_link".into());
        }
        Ok(())
    }
    pub fn text<'a>(&self, input: &'a Value, field: &str) -> Option<&'a str> {
        self.fields
            .get(field)
            .and_then(|path| input.pointer(path))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }
}

pub fn builtin(id: &str) -> Option<&'static str> {
    match id {
        "kimi-code-cli" => Some(include_str!("../../../integrations/kimi-code-cli.json")),
        "trae-cn-ide" => Some(include_str!("../../../integrations/trae-cn-ide.json")),
        "zcode-desktop" => Some(include_str!("../../../integrations/zcode-desktop.json")),
        "workbuddy-candidate" => Some(include_str!(
            "../../../integrations/workbuddy-candidate.json"
        )),
        "standard-events" => Some(include_str!("../../../integrations/standard-events.json")),
        _ => None,
    }
}
pub fn load(id: &str) -> Result<Manifest, String> {
    let text = if let Some(text) = builtin(id) {
        text.to_string()
    } else {
        let path = super::home()
            .join("manifests")
            .join(format!("{}.json", key(id)));
        let meta = std::fs::metadata(&path).map_err(|_| "adapter_not_found")?;
        if meta.len() > 64 * 1024 {
            return Err("manifest_too_large".into());
        }
        std::fs::read_to_string(path).map_err(|_| "manifest_unreadable")?
    };
    let mut m: Manifest = serde_json::from_str(&text).map_err(|_| "manifest_parse_error")?;
    m.validate()?;
    if m.adapter_id != id {
        return Err("adapter_id_mismatch".into());
    }
    if builtin(id).is_none() {
        for c in m.capabilities.values_mut() {
            c.validation = "documented".into();
            c.tested_versions.clear();
            c.channel = "self-reported".into();
        }
    }
    Ok(m)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Context {
    turn: Option<String>,
    submitted: Option<String>,
    calls: BTreeMap<String, String>,
    pending: BTreeMap<String, ContextRequest>,
    #[serde(default)]
    turns: BTreeSet<String>,
}
#[derive(Debug, Serialize, Deserialize)]
struct ContextRequest {
    turn: Option<String>,
    request: String,
    call: Option<String>,
}

pub fn map(
    input: &Value,
    m: &Manifest,
    previous: Option<&Envelope>,
    seq: u64,
    epoch: &str,
) -> Result<(Event, Option<Value>), String> {
    let name = m.text(input, "event").ok_or("host_event_missing")?;
    let mut kind = m
        .rules
        .iter()
        .find(|rule| {
            rule.event == name
                && input.pointer(&rule.field).and_then(Value::as_str) == Some(&rule.equals)
        })
        .map(|rule| rule.kind)
        .or_else(|| m.events.get(name).copied())
        .ok_or("host_event_not_supported")?;
    let session = m.text(input, "session").ok_or("host_session_missing")?;
    let event_id = m
        .text(input, "event_id")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("delivery:{epoch}:{seq}"));
    let mut ctx: Context = previous
        .and_then(|e| e.context.clone())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let mut call = m.text(input, "call").map(str::to_owned);
    let mut request = m.text(input, "request").map(str::to_owned);
    let mut phase = None;
    let mut activity = None;
    if kind == Kind::Submitted {
        ctx.submitted = Some(format!("submission:{event_id}"));
    }
    if kind != Kind::Started
        && m.text(input, "turn")
            .is_some_and(|t| ctx.turn.as_deref().is_some_and(|c| c != t))
    {
        return Err("late_host_turn_ignored".into());
    }
    if kind == Kind::Started {
        let turn = m.text(input, "turn").ok_or("native_turn_id_missing")?;
        if ctx.turns.len() >= 4096 && !ctx.turns.contains(turn) {
            return Err("host_correlation_limit".into());
        }
        if ctx.turns.insert(turn.into()) {
            ctx.turn = Some(turn.into());
            ctx.submitted = None;
            ctx.calls.clear();
        }
    }
    if kind == Kind::ToolStarted {
        if ctx.submitted.is_some() || ctx.turn.is_none() {
            if ctx.turns.len() >= 4096 {
                return Err("host_correlation_limit".into());
            }
            ctx.turn = ctx
                .submitted
                .take()
                .or_else(|| Some(format!("execution:{event_id}")));
            kind = Kind::Started;
            ctx.turns.insert(ctx.turn.clone().unwrap());
            ctx.calls.clear();
        }
        if let Some(call) = &call {
            if ctx.calls.len() >= 128 && !ctx.calls.contains_key(call) {
                return Err("host_correlation_limit".into());
            }
            let signature =
                key(
                    &serde_json::json!([input.get("tool_name"), input.get("tool_input")])
                        .to_string(),
                );
            ctx.calls.insert(call.clone(), signature);
        }
        phase = Some(crate::domain::ExecutionPhase::Tool);
        activity = input
            .get("tool_name")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(80).collect());
    }
    if kind == Kind::Requested {
        request = Some(request.unwrap_or_else(|| format!("request:{event_id}")));
        // PermissionRequest can lack a call ID. Associate only a UNIQUE
        // preceding signature, never the last tool or matching project/title.
        let linked = call.clone().or_else(|| {
            let signature =
                key(
                    &serde_json::json!([input.get("tool_name"), input.get("tool_input")])
                        .to_string(),
                );
            let matches: Vec<_> = ctx.calls.iter().filter(|(_, s)| **s == signature).collect();
            (matches.len() == 1).then(|| matches[0].0.clone())
        });
        call = linked.clone();
        let id = request.clone().unwrap();
        let pending_key =
            serde_json::to_string(&(&ctx.turn, &id)).map_err(|_| "correlation_error")?;
        if ctx.pending.len() >= 128 && !ctx.pending.contains_key(&pending_key) {
            return Err("host_correlation_limit".into());
        }
        ctx.pending.insert(
            pending_key,
            ContextRequest {
                turn: ctx.turn.clone(),
                request: id,
                call: linked,
            },
        );
    }
    if matches!(kind, Kind::ToolFinished | Kind::Resolved) {
        if request.is_none() {
            let matches: Vec<_> = ctx
                .pending
                .values()
                .filter(|p| p.turn == ctx.turn && call.is_some() && p.call == call)
                .map(|p| p.request.clone())
                .collect();
            if matches.len() == 1 {
                request = Some(matches[0].clone());
            }
        }
        let pending_key =
            serde_json::to_string(&(&ctx.turn, &request)).map_err(|_| "correlation_error")?;
        // Serialize the actual string, not Option, for the identity key.
        let pending_key = request
            .as_ref()
            .map(|r| serde_json::to_string(&(&ctx.turn, r)).unwrap_or_default())
            .unwrap_or(pending_key);
        if request.is_some() && ctx.pending.contains_key(&pending_key) {
            if ctx
                .pending
                .get(&pending_key)
                .is_some_and(|p| p.call.is_some() && call.is_some() && p.call != call)
            {
                return Err("permission_call_mismatch".into());
            }
            ctx.pending.remove(&pending_key);
            kind = Kind::Resolved;
            phase = Some(crate::domain::ExecutionPhase::Thinking);
        } else if kind == Kind::Resolved {
            return Err("ambiguous_permission_resolution".into());
        }
        if let Some(c) = &call {
            ctx.calls.remove(c);
        }
    }
    if matches!(kind, Kind::Completed | Kind::Failed | Kind::Interrupted) {
        ctx.pending.retain(|_, p| p.turn != ctx.turn);
        ctx.calls.clear();
    }
    let reason = if kind == Kind::Requested {
        Some(
            if input.get("notification_type").and_then(Value::as_str) == Some("ask_user_question") {
                crate::domain::WaitingReason::Question
            } else {
                crate::domain::WaitingReason::Approval
            },
        )
    } else {
        None
    };
    let event = Event {
        schema_version: 1,
        agent_id: m.agent_id.clone(),
        session_id: session.into(),
        event_id,
        kind,
        turn_id: m
            .text(input, "turn")
            .map(str::to_owned)
            .or_else(|| ctx.turn.clone()),
        occurred_at: None,
        request_id: request,
        call_id: call,
        payload: Payload {
            title: m.text(input, "title").map(str::to_owned),
            project: m.text(input, "project").map(str::to_owned),
            project_path: m.text(input, "project_path").map(str::to_owned),
            model: m.text(input, "model").map(str::to_owned),
            phase,
            activity,
            request_kind: reason,
        },
    };
    event.validate(&m.agent_id)?;
    Ok((
        event,
        Some(serde_json::to_value(ctx).map_err(|_| "adapter_context_failed")?),
    ))
}
