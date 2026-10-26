//! Minimal opt-in official hook state supplements missing rollout approvals.
use crate::domain::{DetectionConfidence, TaskSnapshot, TaskStatus, WaitingReason};
use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Deserialize)]
struct State {
    v: u8,
    session_id: String,
    turn_id: String,
    pending: Vec<Request>,
}
#[derive(Deserialize)]
struct Request {
    id: String,
}

pub(super) fn apply(root: &Path, task: &mut TaskSnapshot) -> std::io::Result<bool> {
    let Some(session) = task.session_id.as_deref().filter(|id| {
        !id.is_empty()
            && id.len() <= 160
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    }) else {
        return Ok(false);
    };
    let file = match std::fs::File::open(
        root.join("taskpulse-events")
            .join(format!("{session}.json")),
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 {
        return Err(std::io::Error::other("Codex observer state exceeds budget"));
    }
    let state: State = serde_json::from_slice(&bytes)?;
    if state.v != 1 {
        return Err(std::io::Error::other("Unsupported Codex observer state"));
    }
    if state.session_id != session || task.turn_key.as_deref() != Some(state.turn_id.as_str()) {
        return Ok(false);
    }
    if task.finished_at.is_some() || task.status != TaskStatus::Running || state.pending.is_empty()
    {
        return Ok(true);
    }
    if state
        .pending
        .iter()
        .any(|p| p.id.is_empty() || p.id.len() > 256)
    {
        return Err(std::io::Error::other("Invalid Codex observer request"));
    }
    task.status = TaskStatus::WaitingInput;
    task.waiting_reason = Some(WaitingReason::Approval);
    task.phase = None;
    task.confidence = DetectionConfidence::Hook;
    task.input_request_ids = state.pending.into_iter().map(|p| p.id).collect();
    task.input_request_id = task.input_request_ids.first().cloned();
    task.message = Some("等待命令批准，请在原 Codex 窗口处理".into());
    task.stale = false;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_matching_turn_only_and_closed_requests_resume_native_state() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("taskpulse-events")).unwrap();
        let file = root.path().join("taskpulse-events/s.json");
        let mut native = TaskSnapshot::new(
            "codex:s",
            "task",
            crate::domain::TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Exact,
            std::time::SystemTime::now(),
        );
        native.session_id = Some("s".into());
        native.turn_key = Some("turn".into());
        for (turn, pending, expected) in [
            ("old", r#"[{"id":"request"}]"#, TaskStatus::Running),
            ("turn", r#"[{"id":"request"}]"#, TaskStatus::WaitingInput),
            ("turn", "[]", TaskStatus::Running),
        ] {
            std::fs::write(
                &file,
                format!(r#"{{"v":1,"session_id":"s","turn_id":"{turn}","pending":{pending}}}"#),
            )
            .unwrap();
            let mut task = native.clone();
            apply(root.path(), &mut task).unwrap();
            assert_eq!(task.status, expected);
        }
        std::fs::write(file, b"{partial").unwrap();
        assert!(apply(root.path(), &mut native).is_err());
    }
}
