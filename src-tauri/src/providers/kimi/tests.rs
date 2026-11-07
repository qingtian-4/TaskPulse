use crate::domain::{TaskStatus, WaitingReason};
#[test]
fn aliased_roots_do_not_duplicate_sessions_but_distinct_roots_remain_ambiguous() {
    let root = tempfile::tempdir().unwrap();
    let make = |base: &Path| {
        let dir = base.join("sessions/work/s");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wire.jsonl"), format!("{}\n{}\n",
            json!({"type":"turn.started","time":1794000000000u64,"turnId":"turn"}),
            json!({"type":"event.question.requested","time":1794000001000u64,"payload":{"question_id":"q"}})
        )).unwrap();
    };
    make(root.path());
    let mut p = KimiProvider::new(Some(root.path().into()));
    let now = native_sessions::time(1794000002000).unwrap();
    p.roots = distinct_roots(vec![root.path().into(), root.path().join(".")]);
    assert_eq!(p.roots.len(), 1);
    let output = p.poll(now);
    assert!(output.authoritative);
    assert_eq!(output.tasks.len(), 1);
    assert_eq!(output.tasks[0].input_request_id.as_deref(), Some("q"));
    let other = tempfile::tempdir().unwrap();
    make(other.path());
    p.roots.push(other.path().into());
    let ambiguous = p.poll(now);
    assert!(!ambiguous.authoritative);
    assert_eq!(ambiguous.tasks.len(), 1);
    assert_eq!(ambiguous.tasks[0].input_request_id.as_deref(), Some("q"));
}
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
