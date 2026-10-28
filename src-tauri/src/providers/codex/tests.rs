use super::*;

fn question_call(name: &str, id: &str, count: usize) -> String {
    let questions: Vec<_> = (0..count)
        .map(|i| serde_json::json!({"title":format!("Question {i}"),"options":["Yes","No"]}))
        .collect();
    format!(
        "{}\n",
        serde_json::json!({"timestamp":recent_rfc3339(0),"type":"response_item",
        "payload":{"type":"function_call","name":name,"call_id":id,"arguments":serde_json::json!({"questions":questions}).to_string()}})
    )
}

fn question_ack(id: &str, accepted: bool) -> String {
    format!(
        "{}\n",
        serde_json::json!({"timestamp":recent_rfc3339(0),"type":"response_item",
        "payload":{"type":"function_call_output","call_id":id,"output":[{"type":"input_text","text":serde_json::json!({"accepted":accepted}).to_string()}]}})
    )
}

fn question_reply(id: &str, index: usize, role: &str) -> String {
    let item_id = serde_json::json!(["request_user_input_async", id, index]).to_string();
    let answers =
        serde_json::json!([{"questionItemId":item_id,"question":"Question","answer":"Yes"}]);
    format!(
        "{}\n",
        serde_json::json!({"timestamp":recent_rfc3339(0),"type":"response_item",
        "payload":{"type":"message","role":role,"content":[{"type":"input_text","text":format!("<send_user_message_question_reply>\n{answers}\n</send_user_message_question_reply>")}]}})
    )
}

#[test]
fn async_question_acknowledgement_is_not_an_answer_or_turn_end() {
    use crate::domain::{ExecutionPhase, WaitingReason};
    let now = SystemTime::now();
    let mut state = parse_codex_rollout_unfiltered(
        &format!(
            "{}{}",
            header("/project"),
            turn_event("task_started", "current", 1)
        ),
        now,
        "s",
    )
    .unwrap();
    for line in [
        question_call("request_user_input_async", "question", 1),
        question_ack("question", true),
    ] {
        state = apply_codex_rollout(&line, now, "s", Some(state)).unwrap();
    }
    assert_eq!(state.waiting_reason, Some(WaitingReason::Question));
    assert_eq!(state.input_request_id.as_deref(), Some("question"));
    let started = state.started_at;
    for line in [
        serde_json::json!({"payload":{"type":"function_call","name":"exec_command","call_id":"work","arguments":"{\"cmd\":\"read-only-check\"}"}}).to_string(),
        serde_json::json!({"payload":{"type":"token_count"}}).to_string(),
        question_reply("unrelated",0,"user"), question_reply("question",0,"assistant"),
    ] {
        state = apply_codex_rollout(&line,now,"s",Some(state)).unwrap();
        assert_eq!(state.waiting_reason, Some(WaitingReason::Question));
    }
    state = apply_codex_rollout(
        &question_reply("question", 0, "user"),
        now,
        "s",
        Some(state),
    )
    .unwrap();
    assert_eq!(state.status, TaskStatus::Running);
    assert_eq!(state.phase, Some(ExecutionPhase::Tool));
    assert_eq!(state.waiting_reason, None);
    assert_eq!(state.turn_key.as_deref(), Some("current"));
    assert_eq!(state.started_at, started);
}

#[test]
fn async_question_end_does_not_hide_unanswered_items_or_parallel_requests() {
    use crate::domain::WaitingReason;
    let now = SystemTime::now();
    let body = format!(
        "{}{}{}{}{}{}{}",
        header("/project"),
        turn_event("task_started", "current", 1),
        question_call("functions.request_user_input_async", "a", 2),
        question_ack("a", true),
        question_call("request_user_input_async", "b", 1),
        question_ack("b", true),
        turn_event("task_complete", "current", 0)
    );
    let mut state = parse_codex_rollout_unfiltered(&body, now, "s").unwrap();
    let provider = CodexProvider::with_root(PathBuf::new());
    assert_eq!(state.waiting_reason, Some(WaitingReason::Question));
    assert_eq!(
        provider
            .session_to_task(&state, None, now)
            .input_request_ids,
        vec!["a", "b"]
    );
    for (id, index, left) in [("a", 0, 2), ("b", 0, 1)] {
        state =
            apply_codex_rollout(&question_reply(id, index, "user"), now, "s", Some(state)).unwrap();
        assert_eq!(state.waiting_reason, Some(WaitingReason::Question));
        assert_eq!(
            provider
                .session_to_task(&state, None, now)
                .input_request_ids
                .len(),
            left
        );
    }
    state = apply_codex_rollout(&question_reply("a", 1, "user"), now, "s", Some(state)).unwrap();
    assert_eq!(state.waiting_reason, Some(WaitingReason::TurnEnded));
    assert_eq!(state.status, TaskStatus::WaitingInput);
    assert!(!provider
        .session_to_task(&state, None, now)
        .requires_action());
}

#[test]
fn async_question_rejected_dispatch_and_old_replies_do_not_block_new_turns() {
    use crate::domain::WaitingReason;
    let now = SystemTime::now();
    let start = format!(
        "{}{}",
        header("/project"),
        turn_event("task_started", "old", 1)
    );
    let failed = format!(
        "{start}{}{}",
        question_call("request_user_input_async", "rejected", 1),
        question_ack("rejected", false)
    );
    let mut state = parse_codex_rollout_unfiltered(&failed, now, "s").unwrap();
    assert_eq!(state.status, TaskStatus::Running);
    assert_eq!(state.waiting_reason, None);
    let next = format!(
        "{}{}{}{}",
        turn_event("task_started", "new", 0),
        question_call("request_user_input_async", "new-question", 1),
        question_ack("new-question", true),
        question_reply("rejected", 0, "user")
    );
    state = apply_codex_rollout(&next, now, "s", Some(state)).unwrap();
    assert_eq!(state.waiting_reason, Some(WaitingReason::Question));
    assert_eq!(state.input_request_id.as_deref(), Some("new-question"));
    assert_eq!(state.turn_key.as_deref(), Some("new"));
}

#[test]
fn async_question_pipeline_notifies_once_then_resumes_and_notifies_real_end() {
    use crate::{attention::AttentionTracker, store::TaskStore, transition::TransitionEngine};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollout-question.jsonl");
    let ledger = root.path().join("notifications.json");
    let now = SystemTime::now();
    std::fs::write(
        &path,
        format!(
            "{}{}",
            header("/project"),
            turn_event("task_started", "current", 1)
        ),
    )
    .unwrap();
    let mut provider = CodexProvider::with_root(root.path().into());
    let mut engine = TransitionEngine::open(ledger.clone());
    let mut store = TaskStore::default();
    let mut attention = AttentionTracker::open(root.path().join("attention.json"));
    let initial = attention.reconcile(store.merge(provider.poll(now).tasks, now));
    assert!(engine.apply(&initial, &[], now).is_empty());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(
            format!(
                "{}{}",
                question_call("request_user_input_async", "q", 1),
                question_ack("q", true)
            )
            .as_bytes(),
        )
        .unwrap();
    let pending = attention.reconcile(store.merge(provider.poll(now).tasks, now));
    assert!(pending[0].requires_action());
    assert_eq!(engine.apply(&pending, &[], now).len(), 1);
    engine = TransitionEngine::open(ledger.clone());
    provider = CodexProvider::with_root(root.path().into());
    let replay = attention.reconcile(store.merge(provider.poll(now).tasks, now));
    assert!(replay[0].requires_action());
    assert!(engine.apply(&replay, &[], now).is_empty());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(question_reply("q", 0, "user").as_bytes())
        .unwrap();
    let answered = attention.reconcile(store.merge(provider.poll(now).tasks, now));
    assert_eq!(answered[0].status, TaskStatus::Running);
    assert!(engine.apply(&answered, &[], now).is_empty());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(turn_event("task_complete", "current", 0).as_bytes())
        .unwrap();
    let completed = attention.reconcile(store.merge(provider.poll(now).tasks, now));
    assert!(!completed[0].requires_action());
    assert_eq!(engine.apply(&completed, &[], now).len(), 1);
    engine = TransitionEngine::open(ledger);
    assert!(engine.apply(&completed, &[], now).is_empty());
}

#[test]
#[ignore = "read-only native async question replay; explicit file and call ID required"]
fn live_codex_async_question_replay() {
    let file = std::env::var("TASKPULSE_CODEX_QUESTION_REPLAY_FILE")
        .expect("explicit rollout file required");
    let request = std::env::var("TASKPULSE_CODEX_QUESTION_CALL_ID")
        .expect("explicit request call ID required");
    let body = std::fs::read_to_string(file).unwrap();
    let mut state = None;
    let mut accepted = false;
    let mut answered = false;
    let mut pending_observations = 0;
    let mut identity = None;
    let mut started = None;
    for line in body.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let before = state.clone();
        state = apply_codex_rollout(line, SystemTime::now(), "replay", state);
        let current = state.as_ref().unwrap();
        if current.questions.active_ids().contains(&request) {
            accepted = true;
            assert_eq!(
                current.waiting_reason,
                Some(crate::domain::WaitingReason::Question)
            );
            if identity.is_none() {
                identity = current.turn_key.clone();
                started = current.started_at;
            }
            assert_eq!(current.turn_key, identity);
            pending_observations += 1;
        } else if accepted {
            // Confirm an actual structured user reply resolved the known
            // request, not a tool result, timeout, reasoning or completion.
            let mut questions = before.unwrap().questions;
            assert_eq!(questions.replied(&value["payload"]), Some(true));
            assert_eq!(current.status, TaskStatus::Running);
            assert_eq!(current.turn_key, identity);
            assert_eq!(current.started_at, started);
            answered = true;
            break;
        }
    }
    assert!(
        accepted && answered && pending_observations > 1,
        "real accepted request plus matching reply required"
    );
    println!("read_only_async_replay: pending_observations={pending_observations}; accepted ACK remains Question, matching user reply resumes RUNNING; no host/state writes");
}

fn fork_meta(id: &str, source: serde_json::Value, cwd: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({"timestamp":recent_rfc3339(5),
        "type":"session_meta","payload":{"id":id,"cwd":cwd,"source":source}})
    )
}

fn spawned_source(parent: &str) -> serde_json::Value {
    serde_json::json!({"subagent":{"thread_spawn":{"parent_thread_id":parent,
        "depth":1,"agent_path":"/root/worker"}}})
}

#[test]
fn fork_identity_cannot_be_overwritten_by_inherited_metadata_across_batches() {
    let now = SystemTime::now();
    for source in [spawned_source("parent"), serde_json::json!("vscode")] {
        let own = fork_meta("child", source.clone(), "/child-project");
        let state = apply_codex_rollout(&own, now, "filename-fallback", None).unwrap();
        let history = fork_meta("parent", serde_json::json!("vscode"), "/parent-project");
        let state = apply_codex_rollout(&history, now, "filename-fallback", Some(state)).unwrap();
        assert_eq!(
            state.session_id, "child",
            "copied parent metadata is not the file owner"
        );
        assert_eq!(state.cwd.as_deref(), Some("/child-project"));
        assert_eq!(state.internal, source != serde_json::json!("vscode"));
        // Repeated metadata for the actual owner remains compatible.
        let same = apply_codex_rollout(&own, now, "filename-fallback", Some(state)).unwrap();
        assert_eq!(same.session_id, "child");
    }
}

#[test]
fn forked_workers_cannot_end_running_parent_through_store_attention_and_restart() {
    use crate::attention::AttentionTracker;
    use crate::store::TaskStore;
    use crate::transition::TransitionEngine;
    for wsl in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let parent_path = sessions.join("rollout-parent.jsonl");
        let parent_header = fork_meta("parent", serde_json::json!("vscode"), "/project");
        std::fs::write(
            &parent_path,
            format!(
                "{parent_header}{}",
                turn_event("task_started", "parent-turn", 4)
            ),
        )
        .unwrap();
        let mut provider = if wsl {
            CodexProvider::with_wsl_root(dir.path().into(), dir.path().into(), "codex-wsl:ubuntu")
        } else {
            CodexProvider::with_root(sessions.clone())
        };
        let ledger = dir.path().join("notifications.json");
        let mut engine = TransitionEngine::open(ledger.clone());
        let mut store = TaskStore::default();
        let mut attention = AttentionTracker::open(dir.path().join("attention.json"));
        let now = SystemTime::now();
        let initial = provider.poll(now);
        let merged = store.merge(initial.tasks, now);
        assert!(engine.apply(&merged, &[], now).is_empty());
        attention.reconcile(merged);
        let parent_id = provider.task_id("parent");
        // Native thread_spawn rollouts begin with their own metadata and
        // then replay a parent's metadata/history before their own turn.
        for i in 0..2 {
            let child = format!("child-{i}");
            let child_turn = format!("child-turn-{i}");
            let path = sessions.join(format!("rollout-{child}.jsonl"));
            let prefix = format!(
                "{}{}{}{}",
                fork_meta(&child, spawned_source("parent"), "/project"),
                parent_header,
                turn_event("task_complete", "old-parent-turn", 3),
                turn_event("task_started", "parent-turn", 2)
            );
            std::fs::write(&path, &prefix).unwrap();
            for suffix in [
                turn_event("task_started", &child_turn, 1),
                turn_event("task_complete", &child_turn, 0),
            ] {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(suffix.as_bytes())
                    .unwrap();
                let output = provider.poll(now);
                let parent = output.tasks.iter().find(|t| t.id == parent_id).unwrap();
                assert_eq!(
                    parent.status,
                    TaskStatus::Running,
                    "a child must never replace its parent (WSL={wsl})"
                );
                assert_eq!(parent.turn_key.as_deref(), Some("parent-turn"));
                let child_task = output
                    .tasks
                    .iter()
                    .find(|t| t.session_id.as_deref() == Some(&child))
                    .unwrap();
                assert!(
                    child_task.internal,
                    "native spawned workers are not independent user turns"
                );
                let merged = store.merge(
                    output.tasks.into_iter().filter(|t| !t.internal).collect(),
                    now,
                );
                assert!(engine.apply(&merged, &[], now).is_empty());
                let visible = attention.reconcile(merged);
                assert_eq!(visible.len(), 1);
                assert_eq!(visible[0].status, TaskStatus::Running);
                engine = TransitionEngine::open(ledger.clone());
            }
        }
        // Real parent completion must still notify once, including after
        // restart; do not reset the persisted notification ledger to fix it.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&parent_path)
            .unwrap()
            .write_all(turn_event("task_complete", "parent-turn", 0).as_bytes())
            .unwrap();
        let tasks = provider
            .poll(now)
            .tasks
            .into_iter()
            .filter(|t| !t.internal)
            .collect();
        let merged = store.merge(tasks, now);
        assert_eq!(engine.apply(&merged, &[], now).len(), 1);
        engine = TransitionEngine::open(ledger);
        assert!(engine.apply(&merged, &[], now).is_empty());
        assert!(attention.reconcile(merged)[0].unread);
    }
}

#[test]
fn fork_spawn_origin_survives_remote_full_and_truncated_rollouts() {
    let child = "11111111-1111-7111-8111-111111111111";
    for source in [
        spawned_source("parent"),
        serde_json::Value::String(spawned_source("parent").to_string()),
    ] {
        for with_header in [false, true] {
            let inherited = if with_header {
                format!(
                    "{}{}",
                    fork_meta(child, source.clone(), "/project"),
                    fork_meta("parent", serde_json::json!("vscode"), "/parent-project")
                )
            } else {
                String::new()
            };
            let section = format!(
                "META\t{}\n===S:10===F:rollout-2026-10-22T12-00-00-{child}.jsonl===\n{inherited}{}",
                serde_json::json!({"id":child,"t":"Worker","cwd":"/project","source":source}),
                turn_event("task_complete", "worker-turn", 0)
            );
            let tasks = tasks_from_codex_section(&section, "Server", "server", SystemTime::now());
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].session_id.as_deref(), Some(child));
            assert!(tasks[0].internal);
            assert!(catalog_from_codex_section(&section, "Server", "server")[0].internal);
            assert!(crate::transition::TransitionEngine::new()
                .apply(&tasks, &[], SystemTime::now())
                .is_empty());
        }
    }
    assert!(
        !internal_codex_source(&serde_json::json!({"subagent":{"thread_spawn":{}}})),
        "unknown origin is not proof of a spawned worker"
    );
}

#[test]
fn user_fork_does_not_inherit_service_classification_from_copied_history() {
    let body = format!(
        "{}{}{}",
        fork_meta("user", serde_json::json!("vscode"), "/project"),
        fork_meta(
            "service",
            serde_json::json!({"subagent":{"other":"guardian"}}),
            "/service"
        ),
        turn_event("task_started", "user-turn", 0)
    );
    let state = parse_codex_rollout(&body, SystemTime::now(), "fallback").unwrap();
    assert_eq!(state.session_id, "user");
    assert!(!state.internal);
    let section = format!("META\t{{\"id\":\"service\",\"t\":\"User title\",\"source\":\"vscode\"}}\n===S:10===F:rollout.jsonl===\n{body}");
    assert!(!remote_thread_metadata(&section)["service"].internal);
}

#[test]
#[ignore = "read-only real rollout replay; requires TASKPULSE_CODEX_REPLAY_ROOT"]
fn live_codex_fork_identity_replay() {
    let root = PathBuf::from(
        std::env::var("TASKPULSE_CODEX_REPLAY_ROOT").expect("explicit sessions root required"),
    );
    let now = SystemTime::now();
    let mut files = 0;
    let mut spawned = 0;
    let mut provider = CodexProvider::with_root(root.clone());
    for path in walk(&root)
        .unwrap()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|s| s == "jsonl"))
    {
        let body = std::fs::read_to_string(&path).unwrap();
        let first: serde_json::Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
        let Some(owner) = first.pointer("/payload/id").and_then(|v| v.as_str()) else {
            continue;
        };
        let state = parse_codex_rollout_unfiltered(&body, now, "fallback").unwrap();
        assert_eq!(state.session_id, owner, "rollout header identity changed");
        if first
            .pointer("/payload/source/subagent/thread_spawn/parent_thread_id")
            .is_some()
        {
            spawned += 1;
            assert!(state.internal);
        }
        // The same native file must keep its identity in the bounded
        // incremental path too, not just when parsed as one string.
        let mut caught = None;
        for _ in 0..30 {
            caught = provider.cached_session_state(&path, now);
            if caught.is_some() {
                break;
            }
        }
        let caught = caught.expect("bounded reader did not catch up");
        assert_eq!(caught.session_id, owner);
        assert_eq!(caught.internal, state.internal);
        assert_eq!(caught.turn_key, state.turn_key);
        if std::env::var("TASKPULSE_CODEX_REPLAY_SESSION")
            .ok()
            .as_deref()
            == Some(owner)
        {
            println!(
                "target_session={owner} status={:?} phase={:?} turn={:?}",
                caught.status, caught.phase, caught.turn_key
            );
        }
        files += 1;
    }
    assert!(
        files > 0 && spawned > 0,
        "real spawned rollout evidence required"
    );
    println!("read_only_replay: files={files} native_spawned_workers={spawned}; no Agent tasks or notification state changed");
}

#[test]
fn lifecycle_schema_error_is_scoped_and_retry_or_rate_limit_is_not_exhaustion() {
    let now = SystemTime::now();
    let body = format!(
        "{}{}",
        header("/project"),
        turn_event("task_started", "current", 1)
    );
    let original = parse_codex_rollout_unfiltered(&body, now, "s").unwrap();
    for (turn, info, retry, expected) in [
        ("current", "usageLimitExceeded", false, true),
        ("old", "usageLimitExceeded", false, false),
        ("current", "usageLimitExceeded", true, false),
        ("current", "rateLimitExceeded", false, false),
        ("current", "serverOverloaded", false, false),
    ] {
        let event = serde_json::json!({"timestamp":recent_rfc3339(0),"method":"error","params":{"threadId":"sess-1","turnId":turn,"willRetry":retry,"error":{"codexErrorInfo":info}}}).to_string();
        let state = apply_codex_rollout(&event, now, "s", Some(original.clone())).unwrap();
        assert_eq!(
            state.waiting_reason == Some(crate::domain::WaitingReason::QuotaExhausted),
            expected
        );
    }
}
#[test]
fn lifecycle_large_compaction_does_not_poison_new_running_turn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout-large.jsonl");
    let compacted = serde_json::json!({"type":"compacted","payload":{"replacement_history":"x".repeat(super::super::jsonl_tail::LINE_BUDGET + 1024)}});
    std::fs::write(
        &path,
        format!(
            "{}{}\n{}",
            header("/project"),
            compacted,
            turn_event("task_started", "current", 0)
        ),
    )
    .unwrap();
    let mut provider = CodexProvider::with_root(dir.path().into());
    let mut parsed = None;
    for _ in 0..30 {
        parsed = provider.cached_session_state(&path, SystemTime::now());
        if parsed.is_some() {
            break;
        }
    }
    let parsed =
        parsed.expect("irrelevant oversized compaction must not permanently hide lifecycle events");
    assert_eq!(parsed.status, TaskStatus::Running);
    assert_eq!(parsed.turn_key.as_deref(), Some("current"));
    assert!(provider.cache[&path].tail.retained_bytes() < 1024);
}

#[test]
fn lifecycle_late_old_turn_end_cannot_end_current_turn() {
    let body = format!(
        "{}{}{}",
        header("/project"),
        turn_event("task_started", "current", 2),
        turn_event("task_complete", "previous", 0)
    );
    let state = parse_codex_rollout_unfiltered(&body, SystemTime::now(), "s").unwrap();
    assert_eq!(state.status, TaskStatus::Running);
    assert_eq!(state.turn_key.as_deref(), Some("current"));
}

#[test]
fn lifecycle_new_user_message_reopens_ended_but_not_an_explicit_gate() {
    let now = SystemTime::now();
    let body = format!(
        "{}{}{}",
        header("/project"),
        turn_event("task_started", "old", 3),
        turn_event("task_complete", "old", 2)
    );
    let old = parse_codex_rollout_unfiltered(&body, now, "s").unwrap();
    let message = serde_json::json!({"timestamp":recent_rfc3339(0),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}}).to_string();
    let next = apply_codex_rollout(&message, now, "s", Some(old.clone())).unwrap();
    assert_eq!(next.status, TaskStatus::Running);
    assert_ne!(next.turn_key, old.turn_key);
    let mut blocked = old;
    blocked.waiting_reason = Some(crate::domain::WaitingReason::Approval);
    blocked
        .confirmed_approvals
        .insert("request".into(), Some("call".into()));
    assert_eq!(
        apply_codex_rollout(&message, now, "s", Some(blocked))
            .unwrap()
            .waiting_reason,
        Some(crate::domain::WaitingReason::Approval)
    );
}

#[test]
fn lifecycle_explicit_quota_error_is_actionable_not_a_successful_end() {
    let now = SystemTime::now();
    let start = format!(
        "{}{}",
        header("/project"),
        turn_event("task_started", "current", 1)
    );
    let error = serde_json::json!({"timestamp":recent_rfc3339(0),"payload":{"type":"error","turn_id":"current","code":"usage_limit_reached","will_retry":false}}).to_string();
    let state = parse_codex_rollout_unfiltered(
        &format!(
            "{start}{error}\n{}",
            turn_event("task_complete", "current", 0)
        ),
        now,
        "s",
    )
    .unwrap();
    assert_eq!(state.status, TaskStatus::WaitingInput);
    assert_eq!(
        serde_json::to_value(state.waiting_reason).unwrap(),
        "quota_exhausted"
    );
    let retry = error.replace("false", "true");
    let state = parse_codex_rollout_unfiltered(&format!("{start}{retry}"), now, "s").unwrap();
    assert_eq!(state.status, TaskStatus::Running);
}
use std::io::Write;

fn escalation_log(id: &str) -> String {
    format!(
        "{}\n{}\n{}\n",
        serde_json::json!({"timestamp":"2026-10-12T09:00:00Z","type":"session_meta","payload":{"id":"s","cwd":"/project"}}),
        serde_json::json!({"timestamp":"2026-10-12T09:00:00Z","payload":{"type":"task_started","turn_id":"turn"}}),
        serde_json::json!({"timestamp":"2026-10-12T09:00:01Z","payload":{"type":"custom_tool_call","name":"exec","call_id":id,"input":"const r = await tools.exec_command({cmd: \"gpu-check\", sandbox_permissions: \"require_escalated\"});"}})
    )
}

/// Replay a real, read-only SSH transcript at a caller-selected historical
/// cutoff. Never prompts an Agent, executes its commands, or edits a log.
#[test]
#[ignore = "requires TASKPULSE_APPROVAL_SSH_HOST, THREAD_ID and REPLAY_UNTIL"]
fn live_remote_approval_replay() {
    use base64::Engine;
    let host = std::env::var("TASKPULSE_APPROVAL_SSH_HOST").expect("Set SSH host");
    let id = std::env::var("TASKPULSE_APPROVAL_THREAD_ID").expect("Set thread ID");
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
    let until = std::env::var("TASKPULSE_APPROVAL_REPLAY_UNTIL").expect("Set RFC3339 cutoff");
    let now = parse_rfc3339(&until).expect("Valid cutoff");
    let script = format!(
        r#"import glob,json,os
from datetime import datetime
thread={id:?}
cutoff=datetime.fromisoformat({until:?}.replace('Z','+00:00'))
for path in glob.glob(os.path.expanduser('~/.codex/sessions/**/rollout-*'+thread+'*.jsonl'),recursive=True):
 with open(path,encoding='utf-8') as stream:
  for line in stream:
   try:
row=json.loads(line)
if datetime.fromisoformat(row['timestamp'].replace('Z','+00:00'))<=cutoff: print(line,end='')
   except (ValueError,KeyError): pass
"#
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(script);
    let output = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=6",
            &host,
            &format!("echo {encoded} | base64 -d | python3"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "SSH read failed");
    let body = String::from_utf8(output.stdout).unwrap();
    let state = parse_codex_rollout(&body, now, &id).expect("Real rollout available");
    assert!(
        !state.active_tools.is_empty(),
        "Unresolved native tool at cutoff"
    );
    assert_eq!(state.phase, Some(crate::domain::ExecutionPhase::Tool));
    assert_eq!(state.status, TaskStatus::Running);
    assert_eq!(state.waiting_reason, None);
    assert_eq!(state.session_id, id);
    let mut task = TaskSnapshot::new(
        format!("replay:{id}"),
        "Read-only replay (not monitored)",
        TaskSource::Codex,
        state.status,
        DetectionConfidence::Hook,
        now,
    );
    task.phase = state.phase;
    task.turn_key = state.turn_key;
    let mut engine = crate::transition::TransitionEngine::new();
    assert!(engine.apply(&[task.clone()], &[], now).is_empty());
    assert!(engine
        .apply(&[task], &[], now + Duration::from_secs(7200))
        .is_empty());
    println!("Read-only replay passed: unresolved native tool, state RUNNING, no inferred approval alert or completion");
}

#[test]
fn long_escalated_calls_never_infer_approval_or_emit_notifications() {
    let started = parse_rfc3339("2026-10-12T09:00:00Z").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout.jsonl");
    std::fs::write(&path, escalation_log("c1")).unwrap();
    let mut provider = CodexProvider::with_root(dir.path().to_path_buf());
    let mut engine = crate::transition::TransitionEngine::new();
    // Both the first parse and unchanged incremental cache must stay running.
    for seconds in [10, 22, 600, 7200] {
        let now = started + Duration::from_secs(seconds);
        let state = provider.cached_session_state(&path, now).unwrap();
        assert_eq!(state.status, TaskStatus::Running);
        assert_eq!(state.waiting_reason, None);
        assert_eq!(state.input_request_id, None);
        assert_eq!(state.phase, Some(crate::domain::ExecutionPhase::Tool));
        assert!(state.active_tools.contains_key("c1"));
        let snapshot = provider.session_to_task(&state, None, now);
        assert!(!snapshot.requires_action());
        assert!(engine.apply(&[snapshot], &[], now).is_empty());
    }
    let now = started + Duration::from_secs(7201);
    let returned = serde_json::json!({"timestamp":"2026-10-12T11:00:01Z","payload":{"type":"custom_tool_call_output","call_id":"c1","output":"Process running with session ID 12"}}).to_string();
    let state = provider.cached_session_state(&path, now).unwrap();
    let returned = apply_codex_rollout(&returned, now, "s", Some(state)).unwrap();
    assert!(returned.active_tools.is_empty());
    assert_eq!(returned.status, TaskStatus::Running);
    assert_eq!(
        returned.phase,
        Some(crate::domain::ExecutionPhase::AwaitingModel)
    );
    assert!(engine
        .apply(&[provider.session_to_task(&returned, None, now)], &[], now)
        .is_empty());
    let ended = serde_json::json!({"timestamp":"2026-10-12T11:00:02Z","payload":{"type":"task_complete","turn_id":"turn"}}).to_string();
    let ended = apply_codex_rollout(&ended, now, "s", Some(returned)).unwrap();
    let snapshot = provider.session_to_task(&ended, None, now);
    assert!(
        matches!(engine.apply(&[snapshot.clone()], &[], now).as_slice(),
        [crate::transition::TaskEvent::WaitingInput { task, .. }]
        if task.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded))
    );
    assert!(engine.apply(&[snapshot], &[], now).is_empty());
}

#[test]
fn unrelated_tool_results_preserve_the_active_command_without_an_approval_guess() {
    let now = parse_rfc3339("2026-10-12T09:00:25Z").unwrap();
    let state = parse_codex_rollout(&escalation_log("c1"), now, "s").unwrap();
    let other = serde_json::json!({"timestamp":"2026-10-12T09:00:05Z","payload":{"type":"function_call_output","call_id":"c-other"}}).to_string();
    let running = apply_codex_rollout(&other, now, "s", Some(state)).unwrap();
    assert!(running.active_tools.contains_key("c1"));
    assert_eq!(running.status, TaskStatus::Running);
    assert_eq!(running.phase, Some(crate::domain::ExecutionPhase::Tool));
    assert_eq!(running.waiting_reason, None);
    let ended = serde_json::json!({"timestamp":"2026-10-12T09:00:30Z","payload":{"type":"task_complete","turn_id":"turn"}}).to_string();
    let ended = apply_codex_rollout(&ended, now, "s", Some(running)).unwrap();
    assert_eq!(
        ended.waiting_reason,
        Some(crate::domain::WaitingReason::TurnEnded)
    );
    assert!(ended.active_tools.is_empty());
}

#[test]
fn command_permissions_are_not_evidence_of_a_pending_approval() {
    let now = parse_rfc3339("2026-10-12T09:01:00Z").unwrap();
    for (name, arguments) in [
        (
            "exec_command",
            r#"{"cmd":"gpu-check","sandbox_permissions":"require_escalated"}"#,
        ),
        (
            "functions.exec_command",
            r#"{"sandbox_permissions":"require_escalated","justification":"Check GPU"}"#,
        ),
        (
            "exec_command",
            r#"{"cmd":"long-command","sandbox_permissions":"use_default"}"#,
        ),
    ] {
        let call = serde_json::json!({"timestamp":"2026-10-12T09:00:02Z","payload":{"type":"function_call","name":name,"call_id":"c2","arguments":arguments}}).to_string();
        let state = parse_codex_rollout(&(escalation_log("c1") + &call), now, "s").unwrap();
        assert_eq!(state.status, TaskStatus::Running);
        assert_eq!(state.phase, Some(crate::domain::ExecutionPhase::Tool));
        assert_eq!(state.waiting_reason, None);
        assert_eq!(state.input_request_id, None);
    }
}

#[test]
fn explicit_approval_events_wait_until_the_matching_request_is_resolved() {
    let now = parse_rfc3339("2026-10-12T09:00:25Z").unwrap();
    let base = parse_codex_rollout(&escalation_log("c1"), now, "s").unwrap();
    let requested=serde_json::json!({"timestamp":"2026-10-12T09:00:02Z","id":42,"method":"item/commandExecution/requestApproval","params":{"threadId":"s","turnId":"turn","itemId":"c1"}}).to_string();
    let state = apply_codex_rollout(&requested, now, "s", Some(base)).unwrap();
    assert_eq!(
        state.waiting_reason,
        Some(crate::domain::WaitingReason::Approval)
    );
    assert_eq!(state.input_request_id.as_deref(), Some("42"));
    let wrong=serde_json::json!({"method":"serverRequest/resolved","params":{"threadId":"s","requestId":43}}).to_string();
    let still = apply_codex_rollout(&wrong, now, "s", Some(state)).unwrap();
    assert_eq!(still.status, TaskStatus::WaitingInput);
    let resolved=serde_json::json!({"timestamp":"2026-10-12T09:00:05Z","method":"serverRequest/resolved","params":{"threadId":"s","requestId":42}}).to_string();
    let resumed = apply_codex_rollout(&resolved, now, "s", Some(still)).unwrap();
    assert_eq!(resumed.status, TaskStatus::Running);
    assert_eq!(resumed.waiting_reason, None);
    assert_eq!(resumed.input_request_id, None);
    assert_eq!(
        resumed.phase,
        Some(crate::domain::ExecutionPhase::AwaitingTool)
    );
}

#[test]
fn remote_snapshots_never_infer_approval_or_invent_completion() {
    let now = parse_rfc3339("2026-10-12T09:00:25Z").unwrap();
    let section=format!("===S:project===F:rollout-2026-10-12T09-00-00-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl===\n{}",escalation_log("c1"));
    let tasks = tasks_from_codex_section(&section, "test-host", "host", now);
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].remote);
    assert_eq!(tasks[0].status, TaskStatus::Running);
    assert_eq!(tasks[0].phase, Some(crate::domain::ExecutionPhase::Tool));
    assert!(!tasks[0].requires_action());
    assert!(crate::transition::TransitionEngine::new()
        .apply(&tasks, &[], now)
        .is_empty());
    assert_eq!(
        tasks[0].session_id.as_deref(),
        Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
    );
}

fn native_db(dir: &Path, path: &Path, updated: i64) -> PathBuf {
    let db = dir.join("state.sqlite");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE threads (id TEXT, name TEXT, title TEXT, first_user_message TEXT, cwd TEXT, rollout_path TEXT, updated_at INTEGER, is_pinned INTEGER, source TEXT, archived INTEGER);").unwrap();
    conn.execute("INSERT INTO threads VALUES ('sess-1',NULL,'Old prompt','Old prompt','C:/old-project',?, ?,0,'appServer',0)", (path.to_string_lossy().as_ref(),updated)).unwrap();
    db
}

fn turn_event(kind: &str, turn: &str, ago: u64) -> String {
    format!(
        "{}\n",
        serde_json::json!({"timestamp":recent_rfc3339(ago),"type":"event_msg","payload":{"type":kind,"turn_id":turn}})
    )
}

#[test]
fn desktop_sidebar_index_renames_without_lifecycle_or_activity_changes() {
    use crate::attention::AttentionTracker;
    use crate::transition::TransitionEngine;
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let path = sessions.join("rollout-test.jsonl");
    std::fs::write(
        &path,
        format!(
            "{}{}",
            header("/project"),
            turn_event("task_started", "current-turn", 1)
        ),
    )
    .unwrap();
    let index = dir.path().join("session_index.jsonl");
    std::fs::write(
        &index,
        "{\"id\":\"sess-1\",\"thread_name\":\"Native sidebar name\"}\ninvalid future row\n",
    )
    .unwrap();
    let mut provider = CodexProvider::with_root(sessions);
    provider.state_db = Some(native_db(dir.path(), &path, 0));
    let first = provider.poll(SystemTime::now());
    assert_eq!(first.tasks[0].name, "Native sidebar name");
    let before = first.tasks[0].clone();
    let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
    let mut engine = TransitionEngine::new();
    engine.apply(&first.tasks, &[], SystemTime::now());
    tracker.reconcile(first.tasks);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .unwrap()
        .write_all(b"{\"id\":\"sess-1\",\"thread_name\":\"Renamed in Desktop\"}\n")
        .unwrap();
    let renamed = provider.poll(SystemTime::now()).tasks;
    assert_eq!(renamed[0].name, "Renamed in Desktop");
    assert_eq!(renamed[0].id, before.id);
    assert_eq!(renamed[0].turn_key, before.turn_key);
    assert_eq!(renamed[0].updated_at, before.updated_at);
    assert!(engine.apply(&renamed, &[], SystemTime::now()).is_empty());
    assert!(!tracker.reconcile(renamed)[0].new_execution);
    let conn = rusqlite::Connection::open(provider.state_db.as_ref().unwrap()).unwrap();
    conn.execute("UPDATE threads SET name='Manual override'", [])
        .unwrap();
    assert_eq!(
        provider.poll(SystemTime::now()).tasks[0].name,
        "Manual override"
    );
}

#[test]
fn new_project_rollout_is_detected_before_metadata_and_title_are_available() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("rollout-existing.jsonl");
    std::fs::write(
        &old,
        format!(
            "{}{}",
            header("/old-project"),
            turn_event("task_complete", "old-turn", 60)
        ),
    )
    .unwrap();
    let mut provider = CodexProvider::with_root(dir.path().to_owned());
    provider.state_db = Some(native_db(
        dir.path(),
        &old,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    ));
    provider.poll(SystemTime::now());
    let fresh = dir.path().join("rollout-brand-new.jsonl");
    std::fs::write(&fresh,format!("{}\n{}",serde_json::json!({"type":"session_meta","payload":{"id":"new-id","cwd":"D:/never-monitored-before"}}),turn_event("task_started","new-turn",0))).unwrap();
    let tasks = provider.poll(SystemTime::now()).tasks;
    let task = tasks
        .iter()
        .find(|t| t.session_id.as_deref() == Some("new-id"))
        .expect("new execution is independent of metadata allowlist");
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.project.as_deref(), Some("never-monitored-before"));
    assert_eq!(
        task.project_path.as_deref(),
        Some("D:/never-monitored-before")
    );
    assert_eq!(task.name, "未命名任务 · new-id");
    let id = task.id.clone();
    let mut tracker = crate::attention::AttentionTracker::open(dir.path().join("attention.json"));
    assert!(tracker.reconcile(tasks).iter().any(|t| t.id == id));
}

#[test]
fn resumed_thread_uses_newest_events_not_old_metadata_path_or_file_order() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("rollout-old.jsonl");
    let resumed = dir.path().join("rollout-resumed.jsonl");
    let now = SystemTime::now();
    let secs = now.duration_since(UNIX_EPOCH).unwrap().as_secs() - 1;
    let stamp = format_unix(secs);
    let old_stamp = stamp.replace(".000Z", ".100Z");
    let new_stamp = stamp.replace(".000Z", ".900Z");
    let body = |kind: &str, turn: &str, ts: &str| {
        format!(
            "{}{}\n",
            header("/project"),
            serde_json::json!({"timestamp":ts,"payload":{"type":kind,"turn_id":turn}})
        )
    };
    std::fs::write(&resumed, body("task_started", "new-turn", &new_stamp)).unwrap();
    // Write the old file LAST: discovery mtime order must not choose it.
    std::fs::write(&old, body("task_complete", "old-turn", &old_stamp)).unwrap();
    let mut provider = CodexProvider::with_root(dir.path().to_owned());
    provider.state_db = Some(native_db(dir.path(), &old, secs as i64));
    let tasks = provider.poll(now).tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "codex:sess-1");
    assert_eq!(tasks[0].status, TaskStatus::Running);
    assert_eq!(tasks[0].turn_key.as_deref(), Some("new-turn"));
}

#[test]
fn unknown_metadata_guardian_is_marked_internal_from_rollout_source() {
    let body = format!(
        "{}\n{}",
        serde_json::json!({"type":"session_meta","payload":{"id":"internal","source":{"subagent":{"other":"guardian"}},"cwd":"/project"}}),
        turn_event("task_started", "turn", 0)
    );
    let (dir, _) = temp_rollout(&body);
    let task = CodexProvider::with_root(dir.path().to_owned())
        .poll(SystemTime::now())
        .tasks
        .remove(0);
    assert!(task.internal);
}

fn remote_origin_fixture(
    id: &str,
    title: &str,
    database_source: Option<serde_json::Value>,
    rollout_source: Option<serde_json::Value>,
    event: &str,
    turn: &str,
) -> String {
    let metadata =
        serde_json::json!({"id":id,"t":title,"cwd":"/project","source":database_source,"p":true});
    let header = rollout_source
        .map(|source| {
            serde_json::json!({"type":"session_meta",
        "payload":{"id":id,"source":source,"cwd":"/project"}})
            .to_string()
                + "\n"
        })
        .unwrap_or_default();
    let event = serde_json::json!({"timestamp":recent_rfc3339(1),"type":"event_msg",
        "payload":{"type":event,"turn_id":turn}});
    format!("META\t{metadata}\n===S:10===F:rollout-2026-10-13T13-00-00-{id}.jsonl===\n{header}{event}\n")
}

#[test]
fn remote_guardian_origin_survives_full_truncated_and_legacy_metadata_paths() {
    let id = "11111111-1111-7111-8111-111111111111";
    let origin = serde_json::json!({"subagent":{"other":"guardian"}});
    for (db, rollout, title) in [
        (
            Some(origin.clone()),
            Some(origin.clone()),
            "Guardian review",
        ),
        (Some(origin.clone()), None, "Renamed internal service"),
        (None, Some(origin.clone()), "Guardian review"),
        (
            Some(serde_json::Value::String(origin.to_string())),
            None,
            "",
        ),
    ] {
        let section = remote_origin_fixture(id, title, db, rollout, "task_complete", "child-turn");
        let tasks = tasks_from_codex_section(&section, "Server1", "server1", SystemTime::now());
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].internal, "origin was lost for {title:?}");
        assert_eq!(tasks[0].session_id.as_deref(), Some(id));
        let catalog = catalog_from_codex_section(&section, "Server1", "server1");
        assert_eq!(catalog.len(), 1);
        assert!(
            catalog[0].internal,
            "catalog disagreed with execution origin"
        );
    }
}

#[test]
fn guardian_completions_never_notify_while_user_parent_is_running() {
    use crate::domain::WaitingReason;
    let dir = tempfile::tempdir().unwrap();
    let mut engine =
        crate::transition::TransitionEngine::open(dir.path().join("notifications.json"));
    let child = "11111111-1111-7111-8111-111111111111";
    let parent = "22222222-2222-7222-8222-222222222222";
    let now = SystemTime::now();
    // A real user can name their task Guardian review; it must still notify.
    let parent_running = remote_origin_fixture(
        parent,
        "Guardian review",
        Some(serde_json::json!("appServer")),
        None,
        "task_started",
        "parent-turn",
    );
    let initial = format!(
        "{parent_running}{}",
        remote_origin_fixture(
            child,
            "Guardian review",
            Some(serde_json::json!({"subagent":{"other":"guardian"}})),
            None,
            "task_started",
            "child-0"
        )
    );
    assert!(engine
        .apply(
            &tasks_from_codex_section(&initial, "Server1", "server1", now),
            &[],
            now
        )
        .is_empty());
    for round in 0..3 {
        let section = format!(
            "{parent_running}{}",
            remote_origin_fixture(
                child,
                "Guardian review",
                Some(serde_json::json!({"subagent":{"other":"guardian"}})),
                None,
                "task_complete",
                &format!("child-{round}")
            )
        );
        let tasks = tasks_from_codex_section(&section, "Server1", "server1", now);
        assert_eq!(tasks.iter().filter(|t| !t.internal).count(), 1);
        assert_eq!(
            tasks.iter().find(|t| !t.internal).unwrap().status,
            TaskStatus::Running
        );
        // Even a direct engine caller cannot bypass the internal guard.
        assert!(engine.apply(&tasks, &[], now).is_empty());
    }
    let section = remote_origin_fixture(
        parent,
        "Guardian review",
        Some(serde_json::json!("appServer")),
        None,
        "task_complete",
        "parent-turn",
    );
    let tasks = tasks_from_codex_section(&section, "Server1", "server1", now);
    assert!(!tasks[0].internal);
    assert_eq!(tasks[0].waiting_reason, Some(WaitingReason::TurnEnded));
    assert_eq!(engine.apply(&tasks, &[], now).len(), 1);
    assert!(engine.apply(&tasks, &[], now).is_empty());
}

#[test]
fn metadata_only_pins_do_not_override_native_internal_origin() {
    let section = "META\t{\"id\":\"internal\",\"t\":\"Renamed service\",\"cwd\":\"/project\",\"p\":true,\"source\":{\"subagent\":{\"other\":\"guardian\"}}}\nMETA\t{\"id\":\"user\",\"t\":\"Guardian review\",\"cwd\":\"/project\",\"p\":true,\"source\":\"appServer\"}\n";
    let tasks = tasks_from_codex_section(section, "Server1", "server1", SystemTime::now());
    assert!(
        tasks
            .iter()
            .find(|t| t.session_id.as_deref() == Some("internal"))
            .unwrap()
            .internal
    );
    assert!(
        !tasks
            .iter()
            .find(|t| t.session_id.as_deref() == Some("user"))
            .unwrap()
            .internal
    );
}

#[test]
fn sidebar_index_tolerates_partial_appends_and_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("session_index.jsonl");
    let mut provider = CodexProvider::with_root(dir.path().join("sessions"));
    std::fs::write(
        &index,
        "{\"id\":\"one\",\"thread_name\":\"First\"}\n{\"id\":\"two\"",
    )
    .unwrap();
    provider.refresh_title_index();
    assert_eq!(provider.indexed_titles.len(), 1);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .unwrap()
        .write_all(b",\"thread_name\":\"Second\"}\n")
        .unwrap();
    provider.refresh_title_index();
    assert_eq!(
        provider.indexed_titles.get("two").map(String::as_str),
        Some("Second")
    );
    std::fs::write(&index, "{\"id\":\"one\",\"thread_name\":\"Replaced\"}\n").unwrap();
    provider.refresh_title_index();
    assert_eq!(provider.indexed_titles.len(), 1);
    assert_eq!(provider.indexed_titles["one"], "Replaced");
}

#[test]
fn wsl_mapping_sidebar_rename_and_incremental_turn_share_native_rules() {
    use crate::attention::AttentionTracker;
    use crate::transition::{TaskEvent, TransitionEngine};
    let dir = tempfile::tempdir().unwrap();
    let linux_home = "home/user/.codex";
    let root = dir.path().join(linux_home);
    std::fs::create_dir_all(root.join("sessions")).unwrap();
    let path = root.join("sessions/rollout-stable.jsonl");
    let ts = recent_rfc3339(0);
    let event = |kind: &str| {
        format!(
            "{}\n",
            serde_json::json!({"timestamp":ts,"payload":{"type":kind,"turn_id":"turn"}})
        )
    };
    let session_meta = serde_json::json!({"type":"session_meta","payload":{"id":"stable","cwd":"/home/user/projects/E-commerce Agent"}});
    std::fs::write(&path, format!("{session_meta}\n{}", event("task_started"))).unwrap();
    let mut provider = CodexProvider::with_wsl_root(root, dir.path().into(), "codex-wsl:ubuntu");
    let mut row:super::super::codex_wsl::WslThread=serde_json::from_value(serde_json::json!({
        "id":"stable","title":"long first prompt, not the sidebar", "sidebar_title":"原生 WSL 标题",
        "cwd":"/home/user/projects/E-commerce Agent","rollout_path":"/home/user/.codex/sessions/rollout-stable.jsonl",
        "updated_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    })).unwrap();
    provider.set_external_metadata(&[row]);
    let first = provider.poll(SystemTime::now());
    assert!(first.authoritative);
    assert_eq!(first.tasks.len(), 1);
    let task = &first.tasks[0];
    assert_eq!(task.id, "codex-wsl:ubuntu:codex:stable");
    assert_eq!(task.name, "原生 WSL 标题");
    assert_eq!(task.project.as_deref(), Some("E-commerce Agent"));
    assert_eq!(
        task.project_path.as_deref(),
        Some("/home/user/projects/E-commerce Agent")
    );
    assert_eq!(task.status, TaskStatus::Running);
    let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
    let mut engine = TransitionEngine::new();
    assert!(engine
        .apply(&first.tasks, &[], SystemTime::now())
        .is_empty());
    assert_eq!(tracker.reconcile(first.tasks.clone()).len(), 1);
    row=serde_json::from_value(serde_json::json!({"id":"stable","name":"手动新标题","rollout_path":"/home/user/.codex/sessions/rollout-stable.jsonl"})).unwrap();
    provider.set_external_metadata(&[row]);
    let renamed = provider.poll(SystemTime::now());
    assert_eq!(renamed.tasks[0].name, "手动新标题");
    assert_eq!(renamed.tasks[0].id, task.id);
    assert_eq!(renamed.tasks[0].turn_key, task.turn_key);
    assert!(engine
        .apply(&renamed.tasks, &[], SystemTime::now())
        .is_empty());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(event("task_complete").as_bytes())
        .unwrap();
    let ended = provider.poll(SystemTime::now());
    assert_eq!(
        ended.tasks[0].waiting_reason,
        Some(crate::domain::WaitingReason::TurnEnded)
    );
    assert_eq!(ended.tasks[0].status, TaskStatus::WaitingInput);
    assert!(matches!(
        engine
            .apply(&ended.tasks, &[], SystemTime::now())
            .as_slice(),
        [TaskEvent::WaitingInput { .. }]
    ));
    assert!(engine
        .apply(&ended.tasks, &[], SystemTime::now())
        .is_empty());
    let attention = tracker.reconcile(ended.tasks.clone());
    assert!(attention[0].unread);
    assert!(tracker.hide(&task.id).unwrap().is_empty());
    let mut fresh = AttentionTracker::open(dir.path().join("fresh-attention.json"));
    assert!(fresh.reconcile(ended.tasks).is_empty());
    let next_turn =
        serde_json::json!({"timestamp":ts,"payload":{"type":"task_started","turn_id":"next-turn"}});
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap(),
        "{next_turn}"
    )
    .unwrap();
    let resumed = provider.poll(SystemTime::now()).tasks;
    assert!(engine.apply(&resumed, &[], SystemTime::now()).is_empty());
    let attention = tracker.reconcile(resumed);
    assert_eq!(attention.len(), 1);
    assert_eq!(attention[0].id, task.id);
    assert_eq!(attention[0].status, TaskStatus::Running);
    assert_eq!(attention[0].turn_key.as_deref(), Some("next-turn"));
}

#[test]
fn tool_outputs_clear_only_matching_calls_and_reasoning_keeps_turn_running() {
    use crate::domain::ExecutionPhase;
    let now = SystemTime::now();
    let mut state = None;
    for payload in [
        serde_json::json!({"type":"task_started","turn_id":"stable-turn"}),
        serde_json::json!({"type":"custom_tool_call","call_id":"a","name":"exec","input":"{\"cmd\":\"cargo test\"}"}),
        serde_json::json!({"type":"function_call","call_id":"b","name":"shell_command","arguments":"{\"command\":\"git status\"}"}),
        serde_json::json!({"type":"custom_tool_call_output","call_id":"a"}),
    ] {
        state = apply_codex_rollout(
            &format!("{}\n", serde_json::json!({"payload":payload})),
            now,
            "s",
            state,
        );
    }
    assert_eq!(state.as_ref().unwrap().phase, Some(ExecutionPhase::Tool));
    assert!(state
        .as_ref()
        .unwrap()
        .activity
        .as_ref()
        .unwrap()
        .contains("git status"));
    state = apply_codex_rollout(
        "{\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"b\"}}\n",
        now,
        "s",
        state,
    );
    assert_eq!(state.as_ref().unwrap().activity, None);
    assert_eq!(
        state.as_ref().unwrap().phase,
        Some(ExecutionPhase::AwaitingModel)
    );
    state = apply_codex_rollout("{\"payload\":{\"type\":\"reasoning\"}}\n", now, "s", state);
    let thinking = state.unwrap();
    assert_eq!(thinking.status, TaskStatus::Running);
    assert_eq!(thinking.phase, Some(ExecutionPhase::Thinking));
    assert_eq!(thinking.turn_key.as_deref(), Some("stable-turn"));
    let resumed = apply_codex_rollout(
        "{\"payload\":{\"type\":\"task_complete\"}}\n",
        now,
        "s",
        Some(thinking),
    )
    .unwrap();
    assert_eq!(resumed.phase, None);
    assert_eq!(
        resumed.waiting_reason,
        Some(crate::domain::WaitingReason::TurnEnded)
    );
}

#[test]
fn post_completion_usage_event_does_not_restart_turn_or_reset_elapsed_time() {
    let start = recent_rfc3339(90);
    let end = recent_rfc3339(5);
    let body = format!(
        "{}\n{}\n{}\n{}\n",
        header("/project"),
        serde_json::json!({"timestamp":start,"payload":{"type":"task_started","turn_id":"turn-1"}}),
        serde_json::json!({"timestamp":end,"payload":{"type":"task_complete"}}),
        serde_json::json!({"timestamp":end,"payload":{"type":"token_count"}})
    );
    let state = parse_codex_rollout(&body, SystemTime::now(), "id").unwrap();
    assert_eq!(state.status, TaskStatus::WaitingInput);
    assert_eq!(
        state.started_at,
        parse_rfc3339_value(&serde_json::json!(start))
    );
}

#[test]
fn worker_to_attention_turn_completion_and_resume() {
    use crate::attention::AttentionTracker;
    use crate::providers::worker::Worker;
    use crate::store::TaskStore;
    use crate::transition::{TaskEvent, TransitionEngine};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollout-live.jsonl");
    let event = |kind: &str, turn: &str| {
        format!(
            "{}\n",
            serde_json::json!({
                "timestamp": recent_rfc3339(0), "payload": {"type": kind, "turn_id": turn}
            })
        )
    };
    std::fs::write(
        &path,
        format!("{}{}", header("/project"), event("task_started", "turn-1")),
    )
    .unwrap();
    let mut provider = CodexProvider::with_root(root.path().to_path_buf());
    let worker = Worker::spawn(Duration::from_secs(60), move |_| {
        provider.poll(SystemTime::now())
    });
    let mut store = TaskStore::default();
    let mut engine = TransitionEngine::new();
    let mut attention = AttentionTracker::open(root.path().join("attention.json"));
    let sources = std::collections::HashSet::from([TaskSource::Codex]);
    let first = worker
        .observations
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let merged = store.merge_sources(first.output.tasks, &sources, SystemTime::now());
    assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
    assert_eq!(attention.reconcile(merged)[0].status, TaskStatus::Running);

    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(event("task_complete", "turn-1").as_bytes())
        .unwrap();
    worker.request_refresh();
    let completed = worker
        .observations
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let mut merged = store.merge_sources(completed.output.tasks, &sources, SystemTime::now());
    let events = engine.apply(&merged, &[], SystemTime::now());
    assert!(matches!(
        events.as_slice(),
        [TaskEvent::WaitingInput { .. }]
    ));
    assert!(attention.reconcile(merged.clone())[0].unread);
    assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
    merged[0].name = "Native renamed task".into();
    assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
    let renamed = attention.reconcile(merged);
    assert_eq!(renamed[0].name, "Native renamed task");
    assert!(attention.mark_viewed(&renamed[0].id).unwrap().is_empty());

    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(event("task_started", "turn-2").as_bytes())
        .unwrap();
    worker.request_refresh();
    let resumed = worker
        .observations
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let merged = store.merge_sources(resumed.output.tasks, &sources, SystemTime::now());
    assert!(engine.apply(&merged, &[], SystemTime::now()).is_empty());
    let display = attention.reconcile(merged);
    assert_eq!(display[0].status, TaskStatus::Running);
    assert_eq!(display[0].id, renamed[0].id);
}

fn temp_rollout(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("rollout-test.jsonl");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    (dir, p)
}

fn header(cwd: &str) -> String {
    format!(
        "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"session_meta\",\"id\":\"sess-1\",\"cwd\":\"{cwd}\",\"originator\":\"Codex Desktop\"}}}}\n",
        ts = recent_rfc3339(120)
    )
}

/// RFC3339 string for `now - secs_ago`, so fixtures are always "recent"
/// relative to SystemTime::now() (otherwise the 24h TTL gate drops them).
fn recent_rfc3339(secs_ago: u64) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(secs_ago);
    format_unix(secs)
}

fn format_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[test]
fn completed_turn_is_waiting_input() {
    let body = format!(
        "{}{}",
        header("/mnt/c/projects/my-app"),
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t1\",\"last_agent_message\":\"done\",\"completed_at\":\"{ts}\"}}}}\n",
            ts = recent_rfc3339(60)
        )
    );
    let (_d, p) = temp_rollout(&body);
    let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
    let now = SystemTime::now();
    let st = provider.parse_session_state(&p, now).unwrap();
    assert_eq!(st.status, TaskStatus::WaitingInput);
    assert_eq!(st.project.as_deref(), Some("my-app"));
}

#[test]
fn active_turn_is_running() {
    let body = format!(
        "{}{}",
        header("/home/user/proj"),
        format!(
            "{{\"timestamp\":\"{ts0}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"t2\"}}}}\n{{\"timestamp\":\"{ts1}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"working on it\"}}}}\n",
            ts0 = recent_rfc3339(120),
            ts1 = recent_rfc3339(90)
        )
    );
    let (_d, p) = temp_rollout(&body);
    let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
    let st = provider.parse_session_state(&p, SystemTime::now()).unwrap();
    assert_eq!(st.status, TaskStatus::Running);
}

#[test]
fn latest_agent_message_is_displayed() {
    let ts = recent_rfc3339(60);
    let body = format!(
        "{}{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"first\"}}}}\n{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"latest\"}}}}\n",
        header("/home/user/proj")
    );
    let (_dir, path) = temp_rollout(&body);
    let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
    let state = provider
        .parse_session_state(&path, SystemTime::now())
        .unwrap();
    assert_eq!(state.last_message.as_deref(), Some("latest"));
}

#[test]
fn aborted_turn_returns_to_waiting() {
    let body = format!(
        "{}{}",
        header("/home/user/proj"),
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"turn_aborted\",\"turn_id\":\"t3\"}}}}\n",
            ts = recent_rfc3339(60)
        )
    );
    let (_d, p) = temp_rollout(&body);
    let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
    let st = provider.parse_session_state(&p, SystemTime::now()).unwrap();
    assert_eq!(st.status, TaskStatus::WaitingInput);
}

#[test]
fn stale_running_state_is_retained_but_not_treated_as_fresh_discovery() {
    let body = format!(
        "{}{}",
        header("/home/user/proj"),
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"t4\"}}}}\n",
            ts = recent_rfc3339(60)
        )
    );
    let (_d, p) = temp_rollout(&body);
    let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
    // Preserve raw events for already tracked sessions. Fresh discovery
    // still applies TTL, while known sessions can display uncertainty.
    let fut = SystemTime::now() + Duration::from_secs(3600 * 30);
    let state = provider.parse_session_state(&p, fut).unwrap();
    assert_eq!(state.status, TaskStatus::Running);
    assert!(!session_is_live(&state, fut));
    assert!(parse_codex_rollout(&body, fut, "test").is_none());
}

#[test]
fn project_with_unicode_and_spaces() {
    let body = format!(
        "{}{}",
        header("/home/user/实验 数据/My Project"),
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t5\",\"completed_at\":\"{ts}\"}}}}\n",
            ts = recent_rfc3339(60)
        )
    );
    let (_d, p) = temp_rollout(&body);
    let provider = CodexProvider::with_root(PathBuf::from("/nonexistent"));
    let st = provider.parse_session_state(&p, SystemTime::now()).unwrap();
    assert_eq!(st.project.as_deref(), Some("My Project"));
}

#[test]
fn discover_finds_rollouts_recursively() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("2026/08/27");
    let mut f = std::fs::File::create(a.join("rollout-x.jsonl")).unwrap_or_else(|_| {
        std::fs::create_dir_all(&a).unwrap();
        std::fs::File::create(a.join("rollout-x.jsonl")).unwrap()
    });
    f.write_all(b"{}").unwrap();
    let provider = CodexProvider::with_root(root.path().to_path_buf());
    let files = provider.discover().unwrap();
    assert_eq!(files.len(), 1);
}

#[test]
fn incremental_rollout_matches_full_parse_across_turns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("stable.jsonl");
    let now = SystemTime::now();
    let mut body = header("/tmp/project");
    std::fs::write(&path, &body).unwrap();
    let mut provider = CodexProvider::with_root(root.path().into());
    for (kind, payload) in [
        (
            "event_msg",
            serde_json::json!({"type":"task_started","turn_id":"first"}),
        ),
        (
            "event_msg",
            serde_json::json!({"type":"user_message","message":"keep this title"}),
        ),
        (
            "response_item",
            serde_json::json!({"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cargo test\"}"}),
        ),
        (
            "event_msg",
            serde_json::json!({"type":"task_complete","last_agent_message":"finished"}),
        ),
        (
            "event_msg",
            serde_json::json!({"type":"task_started","turn_id":"second"}),
        ),
    ] {
        let line = format!(
            "{}\n",
            serde_json::json!({"type":kind,"timestamp":recent_rfc3339(1),"payload":payload})
        );
        body.push_str(&line);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(line.as_bytes())
            .unwrap();
        assert_eq!(
            provider.cached_session_state(&path, now),
            parse_codex_rollout_unfiltered(&body, now, "stable")
        );
    }
}

#[test]
fn unchanged_rollout_reuses_cached_parse() {
    let root = tempfile::tempdir().unwrap();
    let body = format!(
        "{}{{\"timestamp\":\"{}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t1\"}}}}\n",
        header("/home/user/original-project"),
        recent_rfc3339(60)
    );
    let path = root.path().join("rollout-cache.jsonl");
    std::fs::write(&path, body).unwrap();
    let mut provider = CodexProvider::with_root(root.path().to_path_buf());

    let first = provider.poll(SystemTime::now());
    assert_eq!(first.tasks[0].project.as_deref(), Some("original-project"));
    provider
        .cache
        .get_mut(&path)
        .unwrap()
        .state
        .as_mut()
        .unwrap()
        .project = Some("cached-project".into());

    let second = provider.poll(SystemTime::now());
    assert_eq!(second.tasks[0].project.as_deref(), Some("cached-project"));
}

#[test]
fn f04_cold_cache_expires_without_replaying_or_losing_reactivation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollout-cold.jsonl");
    let body=format!("{}{{\"timestamp\":\"{}\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"old\"}}}}\n",header("/tmp/project"),recent_rfc3339(1));
    std::fs::write(&path, &body).unwrap();
    let mut p = CodexProvider::with_root(root.path().into());
    let now = SystemTime::now();
    assert!(!p.poll(now).tasks.is_empty());
    p.poll(now + DISCOVERY_TTL + Duration::from_secs(10));
    assert!(
        p.cache.is_empty(),
        "completed cached files cannot select themselves forever"
    );
    // A native writer touching an old project makes it discoverable again.
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{{\"timestamp\":\"{}\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"new\"}}}}\n",recent_rfc3339(0)).as_bytes()).unwrap();
    let out = p.poll(now);
    assert!(out
        .tasks
        .iter()
        .any(|t| t.turn_key.as_deref() == Some("new") && t.status == TaskStatus::Running));
}

#[test]
fn f04_partial_catchup_cannot_publish_a_historical_terminal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollout-budget.jsonl");
    let at = recent_rfc3339(1);
    let mut body = header("/tmp/project");
    body.push_str(&format!("{{\"timestamp\":\"{at}\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"old\"}}}}\n"));
    body.push_str(&format!("{{\"ignored\":\"{}\"}}\n", "x".repeat(1009)).repeat(18 * 1024));
    body.push_str(&format!("{{\"timestamp\":\"{at}\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"current\"}}}}\n"));
    std::fs::write(&path, body).unwrap();
    let mut provider = CodexProvider::with_root(root.path().into());
    let first = provider.poll(SystemTime::now());
    assert!(!first.authoritative);
    assert!(first.tasks.is_empty());
    let mut out = first;
    for _ in 0..10 {
        out = provider.poll(SystemTime::now());
        if out.authoritative {
            break;
        }
    }
    assert!(out.authoritative, "{:?}", out.error);
    assert_eq!(out.tasks[0].status, TaskStatus::Running);
    assert_eq!(out.tasks[0].turn_key.as_deref(), Some("current"));
}

#[test]
fn codex_section_builds_remote_tasks() {
    let section = format!(
        "META\t{{\"id\":\"sess-1\",\"t\":\"研究 Game 记忆机制\",\"cwd\":\"/home/ai/projects/Game\"}}\n===S:08===F:rollout-2026-08-27T23-12-16-sess-1.jsonl===\n{}\n{}\n",
        header("/home/ai/projects/Game"),
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t1\",\"completed_at\":\"{ts}\"}}}}\n",
            ts = recent_rfc3339(60)
        )
    );
    let now = SystemTime::now();
    let tasks = tasks_from_codex_section(&section, "Server3", "server3", now);
    assert_eq!(tasks.len(), 1);
    let t = &tasks[0];
    assert_eq!(t.source, TaskSource::Codex);
    assert_eq!(t.status, TaskStatus::WaitingInput);
    assert_eq!(t.host.as_deref(), Some("Server3"));
    assert!(t.remote);
    assert_eq!(t.name, "研究 Game 记忆机制");
    assert_eq!(t.project.as_deref(), Some("Game"));
}

#[test]
fn metadata_only_remote_threads_are_not_synthesized_as_idle_tasks() {
    let section =
        "META\t{\"id\":\"old-session\",\"t\":\"Dormant task\",\"cwd\":\"/tmp/project\"}\n";
    let tasks = tasks_from_codex_section(section, "Server3", "server3", SystemTime::now());
    assert!(tasks.is_empty());
    let catalog = catalog_from_codex_section(section, "Server3", "server3");
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].id, "codex-remote:server3:old-session");
    assert_eq!(catalog[0].status, TaskStatus::Unknown);
}

#[test]
fn app_server_name_has_priority_over_prompt_title() {
    let meta = ThreadMeta {
        id: "01abcdef".into(),
        manual_name: None,
        native_name: Some("评审 AgentTray 监控界面".into()),
        title: Some("请你 review 一下这个项目并指出问题".into()),
        first_user_message: Some("请你 review 一下这个项目并指出问题".into()),
        cwd: Some(r"D:\Project\agenttray".into()),
        rollout_path: None,
        updated_at: 0,
        pinned: false,
        internal: false,
    };
    assert_eq!(thread_task_name(&meta), "评审 AgentTray 监控界面");
}

#[test]
fn guardian_is_identified_by_native_source_not_its_title() {
    let rows = parse_app_server_thread_list(serde_json::json!({
        "result": {"data": [
            {"id":"internal", "name":"Renamed service", "source":{"subagent":{"other":"guardian"}}},
            {"id":"user", "name":"Guardian review", "source":"appServer"},
            {"id":"worker", "source":{"subagent":{"other":"coding"}}}
        ]}
    }))
    .unwrap();
    assert!(ThreadMeta::from_app_server(&rows[0]).internal);
    assert!(!ThreadMeta::from_app_server(&rows[1]).internal);
    assert!(!ThreadMeta::from_app_server(&rows[2]).internal);
}

#[test]
fn native_database_marks_guardian_catalog_without_hiding_user_title_matches() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("state.sqlite");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE threads (id TEXT, name TEXT, title TEXT, first_user_message TEXT, cwd TEXT, rollout_path TEXT, updated_at INTEGER, is_pinned INTEGER, source TEXT, archived INTEGER);").unwrap();
    for (id, source) in [
        ("internal", r#"{"subagent":{"other":"guardian"}}"#),
        ("user", "appServer"),
    ] {
        conn.execute("INSERT INTO threads VALUES (?, NULL, 'Guardian review', NULL, 'C:/demo', NULL, 0, 0, ?, 0)", (id, source)).unwrap();
    }
    let mut provider = CodexProvider::with_root(dir.path().join("sessions"));
    provider.state_db = Some(db);
    let catalog = provider.poll(SystemTime::now()).catalog;
    assert!(
        catalog
            .iter()
            .find(|t| t.session_id.as_deref() == Some("internal"))
            .unwrap()
            .internal
    );
    assert!(
        !catalog
            .iter()
            .find(|t| t.session_id.as_deref() == Some("user"))
            .unwrap()
            .internal
    );
}

#[test]
fn parses_native_app_server_thread_list() {
    let response = serde_json::json!({
        "id": 2,
        "result": {
            "data": [{
                "id": "01abcdef",
                "name": "Implement TaskPulse Windows MVP",
                "preview": "Long first user prompt",
                "cwd": "D:\\Project\\agenttray",
                "path": "C:\\Users\\tester\\.codex\\sessions\\rollout.jsonl",
                "updatedAt": 1_788_000_000
            }],
            "nextCursor": null
        }
    });
    let rows = parse_app_server_thread_list(response).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].name.as_deref(),
        Some("Implement TaskPulse Windows MVP")
    );
    assert_eq!(rows[0].cwd.as_deref(), Some(r"D:\Project\agenttray"));
}

#[test]
fn parses_native_rate_limits_as_used_percent() {
    let quota = parse_app_server_rate_limits(serde_json::json!({
        "id": 3,
        "result": {
            "rateLimits": {
                "primary": { "usedPercent": 3, "resetsAt": 1788110671 },
                "secondary": { "usedPercent": 31, "resetsAt": 1788660994 }
            }
        }
    }))
    .unwrap();
    assert_eq!(quota.used_5h_pct, 3.0);
    assert_eq!(quota.used_1w_pct, 31.0);
    assert_eq!(quota.resets_5h_at, Some(1788110671));
    assert_eq!(quota.resets_1w_at, Some(1788660994));
}

#[test]
fn quota_selects_codex_bucket_not_another_model() {
    let quota = parse_app_server_rate_limits(serde_json::json!({"result": {
        "rateLimits": {"primary":{"usedPercent":99},"secondary":{"usedPercent":99}},
        "rateLimitsByLimitId": {"codex": {
            "primary":{"usedPercent":7},"secondary":{"usedPercent":75}
        }}
    }}))
    .unwrap();
    assert_eq!(quota.used_5h_pct, 7.0);
    assert_eq!(quota.used_1w_pct, 75.0);
}

#[test]
fn quota_rejects_wrong_bucket_or_window_instead_of_mislabeling() {
    for limits in [
        serde_json::json!({"limitId":"another-model","primary":{"usedPercent":1},"secondary":{"usedPercent":2}}),
        serde_json::json!({"primary":{"usedPercent":1,"windowDurationMins":60},"secondary":{"usedPercent":2}}),
        serde_json::json!({"primary":{"usedPercent":1},"secondary":{"usedPercent":2,"windowDurationMins":1440}}),
    ] {
        assert!(
            parse_app_server_rate_limits(serde_json::json!({"result":{"rateLimits":limits}}))
                .is_err()
        );
    }
}

#[test]
fn failed_quota_refresh_preserves_timestamp_and_marks_cache_stale() {
    let now = SystemTime::now();
    let quota = CodexQuota {
        used_5h_pct: 7.0,
        used_1w_pct: 75.0,
        resets_5h_at: None,
        resets_1w_at: None,
    };
    let mut state = AccountQuota::default();
    record_quota(&mut state, Ok(quota), now);
    assert!(!quota_at(state.clone(), now).stale);
    assert!(quota_at(state.clone(), now + Duration::from_secs(61)).stale);
    record_quota(
        &mut state,
        Err("endpoint unavailable".into()),
        now + Duration::from_secs(2),
    );
    assert!(quota_at(state.clone(), now + Duration::from_secs(2)).stale);
    assert_eq!(state.observed_at, Some(epoch_millis(now)));
    assert_eq!(state.quota, Some(quota));
    record_quota(&mut state, Ok(quota), now + Duration::from_secs(3));
    assert!(state.error.is_none());
    assert!(!quota_at(state, now + Duration::from_secs(3)).stale);
}

#[test]
fn legacy_task_quota_is_not_presented_as_live_account_usage() {
    let now = SystemTime::now();
    let mut tasks = vec![
        TaskSnapshot::new(
            "local",
            "local",
            TaskSource::Codex,
            TaskStatus::Idle,
            DetectionConfidence::Hook,
            now,
        ),
        TaskSnapshot::new(
            "remote",
            "remote",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            now,
        )
        .with_remote()
        .with_host("different-account"),
    ];
    for task in &mut tasks {
        task.quota_5h_used_pct = Some(0.0);
        task.quota_1w_used_pct = Some(5.0);
    }
    clear_legacy_quota(&mut tasks);
    assert!(tasks
        .iter()
        .all(|task| task.quota_5h_used_pct.is_none() && task.quota_1w_used_pct.is_none()));
}

#[test]
fn remote_session_without_sidebar_title_still_has_stable_id_and_project() {
    let section = format!("===S:10===F:rollout-2026-10-09T20-12-14-01a12094-0536-7b01-bbb0-c3dc63696fac.jsonl===\n{}\n{}\n",
        header("/home/user/YOLO-RDD"), serde_json::json!({"timestamp":recent_rfc3339(0),"payload":{"type":"task_started"}}));
    let tasks = tasks_from_codex_section(&section, "Server1_wanghy", "server1", SystemTime::now());
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Running);
    assert_eq!(tasks[0].project.as_deref(), Some("YOLO-RDD"));
    assert_eq!(
        tasks[0].session_id.as_deref(),
        Some("01a12094-0536-7b01-bbb0-c3dc63696fac")
    );
    assert!(tasks[0].name.starts_with("未命名任务"));
}

#[test]
fn custom_exec_call_is_exposed_as_current_activity() {
    let ts = recent_rfc3339(1);
    let body = format!(
        "{{\"timestamp\":\"{ts}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"custom_tool_call\",\"name\":\"exec\",\"arguments\":\"const r=await tools.exec_command({{cmd:\\\"git diff --check\\\",workdir:\\\"/tmp/project\\\"}});\"}}}}\n"
    );
    let state = parse_codex_rollout(&body, SystemTime::now(), "session").unwrap();
    assert_eq!(state.status, TaskStatus::Running);
    assert_eq!(
        state.activity.as_deref(),
        Some("执行命令 · git diff --check")
    );
}

#[test]
fn model_context_updates_presentation_without_lifecycle_or_clock_changes() {
    let now = SystemTime::now();
    let initial = parse_codex_rollout_unfiltered(
        &format!(
            "{}{}",
            header("/project"),
            turn_event("task_started", "current", 1)
        ),
        now,
        "s",
    )
    .unwrap();
    let context = |model: &str, turn: &str| {
        serde_json::json!({
        "type":"turn_context", "timestamp":recent_rfc3339(0),
        "payload":{"model":model,"turn_id":turn}})
        .to_string()
    };
    let mut state = initial.clone();
    for model in ["gpt-5.4", "custom/provider-model"] {
        state = apply_codex_rollout(&context(model, "current"), now, "s", Some(state)).unwrap();
        assert_eq!(state.model.as_deref(), Some(model));
        let mut without_model = state.clone();
        without_model.model = None;
        assert_eq!(without_model, initial);
        let task = CodexProvider::with_root(PathBuf::new()).session_to_task(&state, None, now);
        assert_eq!(task.model, state.model);
        assert_eq!(task.source, TaskSource::Codex);
    }
    for (model, turn) in [
        ("stale-model", "older"),
        ("<synthetic>", "current"),
        ("  ", "current"),
        ("bad\nmodel", "current"),
    ] {
        let before = state.clone();
        state = apply_codex_rollout(&context(model, turn), now, "s", Some(state)).unwrap();
        assert_eq!(state, before);
    }
}

#[test]
#[ignore = "read-only real model evidence; requires TASKPULSE_CODEX_REPLAY_ROOT"]
fn live_codex_model_replay() {
    let root = PathBuf::from(
        std::env::var_os("TASKPULSE_CODEX_REPLAY_ROOT").expect("explicit rollout root"),
    );
    let mut paths = walk(&root).unwrap();
    paths.retain(|p| p.extension().is_some_and(|e| e == "jsonl"));
    paths.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let mut found = 0;
    for path in paths.into_iter().rev().take(30) {
        let mut provider = CodexProvider::with_root(root.clone());
        if let Some(state) = provider.cached_session_state(&path, SystemTime::now()) {
            if let Some(model) = &state.model {
                println!("reported_model={model} status={:?}", state.status);
                found += 1;
            }
        }
    }
    assert!(found > 0, "No real model metadata found");
}

#[test]
fn remote_model_context_is_optional_and_preserves_task_identity() {
    let id = "11111111-1111-7111-8111-111111111111";
    let section = remote_origin_fixture(id, "Native title", None, None, "task_started", "t1");
    let now = SystemTime::now();
    let unknown = tasks_from_codex_section(&section, "Server3", "server3", now);
    let context = serde_json::json!({"type":"turn_context","payload":{"turn_id":"t1","model":"remote-reported-model"}});
    let known =
        tasks_from_codex_section(&format!("{section}{context}\n"), "Server3", "server3", now);
    assert_eq!(known.len(), 1);
    assert!(unknown[0].model.is_none());
    assert_eq!(known[0].model.as_deref(), Some("remote-reported-model"));
    assert_eq!(known[0].id, unknown[0].id);
    assert_eq!(known[0].turn_key, unknown[0].turn_key);
    assert_eq!(known[0].started_at, unknown[0].started_at);
    assert_eq!(known[0].updated_at, unknown[0].updated_at);
}
