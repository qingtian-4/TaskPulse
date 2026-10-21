//! Receive-channel errors and durable fact gaps are different evidence. A peer
//! success can restore transport, never a missing end/request result elsewhere.
use super::{
    journal, key, now_ms,
    schema::{Event, Kind},
};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub id: String,
    pub at: u64,
    pub turn: Option<String>,
    pub request: Option<String>,
    pub call: Option<String>,
    pub kind: Option<Kind>,
}
#[derive(Default, Serialize, Deserialize)]
struct Gaps {
    items: Vec<Gap>,
    #[serde(default)]
    unknown_before: u64,
}

pub fn operational(code: &str) -> bool {
    !matches!(
        code,
        "invalid_json"
            | "event_schema_error"
            | "unsupported_schema"
            | "invalid_identity"
            | "invalid_correlation_id"
            | "turn_id_required"
            | "request_id_required"
            | "call_id_required"
            | "explicit_request_kind_required"
            | "invalid_display_field"
            | "invalid_timestamp"
            | "agent_binding_mismatch"
            | "adapter_or_transport_mismatch"
            | "adapter_argument_required"
            | "source_not_configured"
            | "source_disabled"
            | "source_not_confirmed"
            | "invalid_source_id"
            | "input_too_large"
            | "host_session_missing"
            | "host_event_missing"
            | "invalid_session_id"
            | "late_host_turn_ignored"
    )
}

pub fn record(home: &Path, source: &str, code: &str, event: Option<&Event>) -> Result<(), String> {
    if !operational(code) {
        return Ok(());
    }
    let scope = event
        .map(|e| key(&e.session_id))
        .unwrap_or_else(|| "unassociated".into());
    let dir = journal::root(home, source).join("gaps");
    let _guard = journal::lock(
        &dir.join(format!("{scope}.lock")),
        Duration::from_millis(500),
    )?;
    let path = dir.join(format!("{scope}.json"));
    let mut gaps = read(&path)?;
    let at = now_ms();
    let nonce = tempfile::NamedTempFile::new().map_err(|_| "gap_nonce_failed")?;
    let gap = Gap {
        id: key(&format!("{at}:{}", nonce.path().display())),
        at,
        turn: event.and_then(|e| e.turn_id.as_deref()).map(key),
        request: event.and_then(|e| e.request_id.as_deref()).map(key),
        call: event.and_then(|e| e.call_id.as_deref()).map(key),
        kind: event.map(|e| e.kind),
    };
    if gaps.items.len() >= 64 {
        gaps.unknown_before = gaps.unknown_before.max(gaps.items.remove(0).at);
    }
    gaps.items.push(gap);
    journal::atomic(&path, &gaps)
}
fn read(path: &Path) -> Result<Gaps, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > 64 * 1024 => Err("gap_state_too_large".into()),
        Ok(_) => serde_json::from_slice(&std::fs::read(path).map_err(|_| "gap_read_failed")?)
            .map_err(|_| "gap_state_invalid".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Gaps::default()),
        Err(_) => Err("gap_read_failed".into()),
    }
}
pub fn load(home: &Path, source: &str, session: &str) -> Result<Vec<Gap>, String> {
    let dir = journal::root(home, source);
    let mut all = vec![];
    for scope in ["unassociated", session] {
        let gaps = read(&dir.join("gaps").join(format!("{scope}.json")))?;
        all.extend(gaps.items);
        if gaps.unknown_before > 0 {
            all.push(Gap {
                id: format!("overflow:{scope}:{}", gaps.unknown_before),
                at: gaps.unknown_before,
                turn: None,
                request: None,
                call: None,
                kind: None,
            });
        }
    }
    // Old source-only errors must remain conservative across upgrades. They
    // cannot be "repaired" by the source's max received timestamp.
    if let Ok(bytes) = std::fs::read(dir.join("last-error.json")) {
        if bytes.len() <= 1024 {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if v.get("gap_recorded").is_none() && v["code"].as_str().is_some_and(operational) {
                    if let Some(at) = v["at"].as_u64() {
                        let legacy = Gap {
                            id: format!("legacy:{at}"),
                            at,
                            turn: None,
                            request: None,
                            call: None,
                            kind: None,
                        };
                        if !all.iter().any(|g| g.id == legacy.id) {
                            let path = dir.join("gaps/unassociated.json");
                            let _guard = journal::lock(
                                &path.with_extension("lock"),
                                Duration::from_millis(500),
                            )?;
                            let mut saved = read(&path)?;
                            if !saved.items.iter().any(|g| g.id == legacy.id) {
                                if saved.items.len() >= 64 {
                                    saved.unknown_before =
                                        saved.unknown_before.max(saved.items.remove(0).at);
                                }
                                saved.items.push(legacy.clone());
                                journal::atomic(&path, &saved)?;
                            }
                            all.push(legacy);
                        }
                    }
                }
            }
        }
    }
    Ok(all)
}
pub fn recovers(gap: &Gap, e: &super::schema::Envelope, new_turn: bool) -> bool {
    if e.received_at <= gap.at {
        return false;
    }
    let event = &e.event;
    let turn = event.turn_id.as_deref().map(key);
    if new_turn && event.kind == Kind::Started && (gap.turn.is_none() || gap.turn != turn) {
        return true;
    }
    if gap.turn.is_some() && gap.turn == turn {
        if matches!(
            event.kind,
            Kind::Completed | Kind::Failed | Kind::Interrupted
        ) {
            return true;
        }
        return gap.kind == Some(event.kind)
            && matches!(event.kind, Kind::Resolved | Kind::ToolFinished)
            && gap.request == event.request_id.as_deref().map(key)
            && gap.call == event.call_id.as_deref().map(key);
    }
    false
}
