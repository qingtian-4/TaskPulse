use super::{journal, manifest, reducer::Session, schema::*};
use crate::{
    domain::{ExecutionPhase, TaskStatus, WaitingReason},
    providers::{
        generic_event::{acknowledge, GenericEventProvider},
        Provider,
    },
    transition::TransitionEngine,
};
use serde_json::json;

fn event(kind: Kind, turn: &str, n: u64) -> Event {
    Event {
        schema_version: 1,
        agent_id: "fixture-agent".into(),
        session_id: "fixture-session".into(),
        event_id: format!("event-{n}"),
        kind,
        turn_id: Some(turn.into()),
        occurred_at: Some(1000 + n),
        request_id: None,
        call_id: None,
        payload: Payload::default(),
    }
}
fn envelope(event: Event, n: u64) -> Envelope {
    Envelope {
        version: 1,
        source_instance_id: "fixture-source".into(),
        sequence: n,
        journal_epoch: "epoch".into(),
        received_at: 2000 + n,
        adapter_version: "1".into(),
        evidence: "self_reported".into(),
        event,
        context: None,
    }
}
fn apply(s: &mut Session, e: Event, n: u64) -> crate::domain::TaskSnapshot {
    s.apply(&envelope(e, n), "Fixture").unwrap().unwrap()
}

#[test]
fn protocol_rejects_unknown_versions_unbound_identity_and_secret_fields() {
    let e = event(Kind::Started, "a", 1);
    assert!(e.validate("fixture-agent").is_ok());
    let mut wrong = e.clone();
    wrong.schema_version = 2;
    assert!(wrong.validate("fixture-agent").is_err());
    assert!(e.validate("other-agent").is_err());
    let mut value = serde_json::to_value(e).unwrap();
    value["payload"]["prompt"] = json!("SYNTHETIC-PRIVATE");
    assert!(serde_json::from_value::<Event>(value).is_err());
    let mut request = event(Kind::Requested, "a", 2);
    request.request_id = Some("id".into());
    assert!(request.validate("fixture-agent").is_err());
}

#[test]
fn latest_snapshot_cannot_bypass_or_consume_an_uncommitted_ordered_end() {
    let mut state = Session::default();
    let mut engine = TransitionEngine::new();
    let start = apply(&mut state, event(Kind::Started, "a", 1), 1);
    engine.apply_unflushed(&[start], &[], std::time::SystemTime::now());
    let mut end = apply(&mut state, event(Kind::Completed, "a", 2), 2);
    end.unread = true;
    assert!(engine
        .apply_snapshots(&[end.clone()], &[], std::time::SystemTime::now())
        .is_empty());
    assert_eq!(
        engine
            .apply_unflushed(&[end.clone()], &[], std::time::SystemTime::now())
            .len(),
        1
    );
    assert!(engine
        .apply_unflushed(&[end], &[], std::time::SystemTime::now())
        .is_empty());
}

#[test]
fn receiver_write_failures_degrade_source_without_finishing_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let src = source();
    let mut provider = GenericEventProvider::with_home(src.clone(), dir.path().into());
    journal::append(
        dir.path(),
        &src.id,
        event(Kind::Started, "a", 1),
        "self_reported",
        "v1",
    )
    .unwrap();
    let batch = provider.poll(std::time::SystemTime::now());
    for item in &batch.ordered {
        acknowledge(dir.path(), &item.receipt).unwrap();
    }
    let error = journal::root(dir.path(), &src.id).join("last-error.json");
    journal::atomic(
        &error,
        &json!({"at":super::now_ms()+1,"code":"journal_sync_failed"}),
    )
    .unwrap();
    let failed = provider.poll(std::time::SystemTime::now());
    assert!(!failed.authoritative);
    assert!(failed.tasks[0].stale);
    assert_eq!(failed.tasks[0].status, TaskStatus::Running);
    std::thread::sleep(std::time::Duration::from_millis(3));
    journal::append(
        dir.path(),
        &src.id,
        event(Kind::Heartbeat, "a", 2),
        "self_reported",
        "v1",
    )
    .unwrap();
    let recovered = provider.poll(std::time::SystemTime::now());
    assert!(recovered.authoritative);
    assert!(!recovered.tasks[0].stale);
    assert_eq!(recovered.tasks[0].status, TaskStatus::Running);
}

#[test]
fn pending_capacity_does_not_prevent_resolution_or_explicit_end() {
    let mut state = Session::default();
    apply(&mut state, event(Kind::Started, "a", 1), 1);
    for n in 2..130 {
        let mut request = event(Kind::Requested, "a", n);
        request.request_id = Some(format!("r-{n}"));
        request.payload.request_kind = Some(WaitingReason::Approval);
        apply(&mut state, request, n);
    }
    let mut resolved = event(Kind::Resolved, "a", 130);
    resolved.request_id = Some("r-2".into());
    assert_eq!(
        apply(&mut state, resolved, 130).input_request_ids.len(),
        127
    );
    assert!(apply(&mut state, event(Kind::Completed, "a", 131), 131)
        .input_request_ids
        .is_empty());
}

#[test]
fn ambiguous_native_tool_signature_cannot_resolve_an_unlinked_request() {
    let m = manifest::load("zcode-desktop").unwrap();
    let mut previous = None;
    let mut state = Session::default();
    for (n,raw) in [
        json!({"hook_event_name":"UserPromptSubmit","session_id":"s"}),
        json!({"hook_event_name":"PreToolUse","session_id":"s","tool_use_id":"a","tool_name":"Synthetic","tool_input":{"same":true}}),
        json!({"hook_event_name":"PreToolUse","session_id":"s","tool_use_id":"b","tool_name":"Synthetic","tool_input":{"same":true}}),
        json!({"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Synthetic","tool_input":{"same":true}}),
        json!({"hook_event_name":"PostToolUse","session_id":"s","tool_use_id":"b"})
    ].into_iter().enumerate(){
        let (event,context)=manifest::map(&raw,&m,previous.as_ref(),n as u64+1,"epoch").unwrap();
        if n==3{assert!(event.call_id.is_none());}
        if n==4{assert_eq!(event.kind,Kind::ToolFinished);}
        let mut envelope=envelope(event,n as u64+1);envelope.context=context;
        state.apply(&envelope,"Synthetic ZCode").unwrap();previous=Some(envelope);
    }
    assert_eq!(state.task.unwrap().input_request_ids.len(), 1);
}

#[test]
fn rename_heartbeat_and_tool_returns_do_not_end_or_renew_a_turn() {
    let mut state = Session::default();
    let started = apply(&mut state, event(Kind::Started, "a", 1), 1);
    let mut rename = event(Kind::Renamed, "a", 2);
    rename.payload.title = Some("native rename".into());
    let renamed = apply(&mut state, rename, 2);
    assert_eq!(started.id, renamed.id);
    assert_eq!(started.updated_at, renamed.updated_at);
    assert_eq!(
        apply(&mut state, event(Kind::Heartbeat, "a", 3), 3).updated_at,
        started.updated_at
    );
    let mut tool = event(Kind::ToolStarted, "a", 4);
    tool.call_id = Some("call-a".into());
    apply(&mut state, tool, 4);
    let mut end = event(Kind::ToolFinished, "a", 5);
    end.call_id = Some("call-unknown".into());
    let unmatched = apply(&mut state, end.clone(), 5);
    assert_eq!(unmatched.phase, Some(ExecutionPhase::Tool));
    end.event_id = "matching-tool-return".into();
    end.call_id = Some("call-a".into());
    let returned = apply(&mut state, end, 6);
    assert_eq!(returned.status, TaskStatus::Running);
    assert_eq!(returned.phase, Some(ExecutionPhase::Thinking));
    assert_eq!(
        apply(&mut state, event(Kind::StopRequested, "a", 7), 7).status,
        TaskStatus::Running
    );
    assert_eq!(
        apply(&mut state, event(Kind::Completed, "a", 8), 8).status,
        TaskStatus::WaitingInput
    );
    assert!(state
        .apply(&envelope(event(Kind::ToolStarted, "a", 9), 9), "Fixture")
        .unwrap()
        .is_none());
    let new = apply(&mut state, event(Kind::Started, "b", 10), 10);
    assert_eq!(new.id, started.id);
    assert!(state
        .apply(&envelope(event(Kind::Completed, "a", 11), 11), "Fixture")
        .unwrap()
        .is_none());
    assert_eq!(state.task.unwrap().status, TaskStatus::Running);
}

#[test]
fn independent_requests_survive_phase_changes_unmatched_results_and_restart() {
    let mut state = Session::default();
    apply(&mut state, event(Kind::Started, "a", 1), 1);
    for n in 2..=3 {
        let mut e = event(Kind::Requested, "a", n);
        e.request_id = Some(format!("request-{n}"));
        e.call_id = Some(format!("call-{n}"));
        e.payload.request_kind = Some(WaitingReason::Approval);
        apply(&mut state, e, n);
    }
    let mut phase = event(Kind::Phase, "a", 4);
    phase.payload.phase = Some(ExecutionPhase::Thinking);
    assert_eq!(apply(&mut state, phase, 4).input_request_ids.len(), 2);
    let mut tool = event(Kind::ToolFinished, "a", 5);
    tool.call_id = Some("call-2".into());
    assert_eq!(apply(&mut state, tool, 5).input_request_ids.len(), 2);
    let mut state: Session = serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
    let mut resolved = event(Kind::Resolved, "a", 6);
    resolved.request_id = Some("request-2".into());
    let task = apply(&mut state, resolved, 6);
    assert_eq!(task.status, TaskStatus::WaitingInput);
    assert_eq!(task.input_request_ids, ["request-3"]);
    let confirmed = task.pending_confirmed_at;
    assert_eq!(
        apply(&mut state, event(Kind::Heartbeat, "a", 7), 7).pending_confirmed_at,
        confirmed
    );
    let mut resolved = event(Kind::Resolved, "old-turn", 8);
    resolved.request_id = Some("request-3".into());
    assert!(state
        .apply(&envelope(resolved, 8), "Fixture")
        .unwrap()
        .is_none());
    assert_eq!(state.task.unwrap().input_request_ids, ["request-3"]);
}

fn source() -> crate::monitoring::SourceConfig {
    serde_json::from_value(json!({"id":"integration-fixture","name":"Fixture","kind":"generic_event","enabled":true,"location":"",
        "integration":{"agent_id":"fixture-agent","adapter_id":"fixture-adapter","transport":"local","accept_since_ms":0},
        "projects":["*"],"excluded_projects":[],"tmux_sessions":[],"tmux_panes":[],"poll_interval_ms":1000})).unwrap()
}

#[test]
fn ordered_short_turns_durable_outbox_and_turn_dedup_survive_crash_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let src = source();
    let mut provider = GenericEventProvider::with_home(src.clone(), dir.path().into());
    for (n, kind, turn) in [
        (1, Kind::Started, "a"),
        (2, Kind::Completed, "a"),
        (3, Kind::Started, "b"),
        (4, Kind::Completed, "b"),
    ] {
        journal::append(
            dir.path(),
            &src.id,
            event(kind, turn, n),
            "self_reported",
            "v1",
        )
        .unwrap();
    }
    let batch = provider.poll(std::time::SystemTime::now());
    assert!(batch.authoritative, "{:?}", batch.error);
    assert_eq!(batch.ordered.len(), 4);
    assert_eq!(batch.tasks[0].turn_key.as_deref(), Some("b"));
    let mut engine = TransitionEngine::open(dir.path().join("notifications.json"));
    let mut notices = 0;
    for observation in &batch.ordered {
        let mut task = observation.task.clone();
        task.unread = true;
        task.new_execution = true;
        notices += engine
            .apply(&[task], &[], std::time::SystemTime::now())
            .len();
    }
    assert_eq!(notices, 2);
    assert!(engine.flush());
    // Crash after reducer checkpoint + dedup, before outbox acknowledgement.
    drop(provider);
    drop(engine);
    let mut recovered = GenericEventProvider::with_home(src, dir.path().into());
    // Force a distinct receiver incarnation even on same-millisecond tests.
    std::thread::sleep(std::time::Duration::from_millis(2));
    let batch = recovered.poll(std::time::SystemTime::now());
    assert_eq!(batch.ordered.len(), 4);
    let mut engine = TransitionEngine::open(dir.path().join("notifications.json"));
    for observation in &batch.ordered {
        let mut task = observation.task.clone();
        task.unread = true;
        task.new_execution = true;
        assert!(engine
            .apply(&[task], &[], std::time::SystemTime::now())
            .is_empty());
        acknowledge(dir.path(), &observation.receipt).unwrap();
    }
    assert!(recovered
        .poll(std::time::SystemTime::now())
        .ordered
        .is_empty());
}

#[test]
fn identities_are_isolated_and_discovery_does_not_create_execution() {
    let mut a = Session::default();
    let mut b = Session::default();
    let mut e = envelope(event(Kind::Discovered, "a", 1), 1);
    let task = a.apply(&e, "A").unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Unknown);
    e.source_instance_id = "other-source".into();
    let other = b.apply(&e, "B").unwrap().unwrap();
    assert_ne!(task.id, other.id);
    let mut e = event(Kind::Started, "a", 2);
    e.payload.title = Some("Renamed".into());
    let started = apply(&mut a, e, 2);
    assert_eq!(task.id, started.id);
}

#[test]
fn builtin_manifests_are_restricted_and_stop_is_never_final() {
    for id in [
        "standard-events",
        "kimi-code-cli",
        "trae-cn-ide",
        "zcode-desktop",
        "workbuddy-candidate",
    ] {
        let m = manifest::load(id).unwrap();
        assert_ne!(m.events.get("Stop"), Some(&Kind::Completed));
        assert!(m
            .capabilities
            .values()
            .all(|c| c.validation != "native-tested" && c.tested_versions.is_empty()));
    }
    let mut value: serde_json::Value =
        serde_json::from_str(manifest::builtin("kimi-code-cli").unwrap()).unwrap();
    value["command"] = json!("not allowed");
    assert!(serde_json::from_value::<manifest::Manifest>(value).is_err());
    let mut m = manifest::load("kimi-code-cli").unwrap();
    m.events.insert("Stop".into(), Kind::Completed);
    assert!(m.validate().is_err());
}

#[test]
fn native_kimi_start_stop_and_explicit_failure_are_not_claude_semantics() {
    let m = manifest::load("kimi-code-cli").unwrap();
    let raw = json!({"hook_event_name":"TurnStarted","session_id":"s","turn_id":"turn-a","session_title":"synthetic native title","prompt":"SYNTHETIC PRIVATE"});
    let (event, context) = manifest::map(&raw, &m, None, 1, "epoch").unwrap();
    assert_eq!(event.kind, Kind::Started);
    assert!(!serde_json::to_string(&event).unwrap().contains("PRIVATE"));
    let mut previous = envelope(event, 1);
    previous.context = context;
    let (stop, _) = manifest::map(
        &json!({"hook_event_name":"Stop","session_id":"s"}),
        &m,
        Some(&previous),
        2,
        "epoch",
    )
    .unwrap();
    assert_eq!(stop.kind, Kind::StopRequested);
    assert_eq!(stop.turn_id.as_deref(), Some("turn-a"));
    let (failure, _) = manifest::map(
        &json!({"hook_event_name":"StopFailure","session_id":"s"}),
        &m,
        Some(&previous),
        3,
        "epoch",
    )
    .unwrap();
    assert_eq!(failure.kind, Kind::Failed);
}

#[test]
fn zcode_unique_call_mapping_and_ambiguous_calls_do_not_clear_the_last_request() {
    let m = manifest::load("zcode-desktop").unwrap();
    let mut previous = None;
    let mut send = |input: serde_json::Value, n| {
        let (event, context) = manifest::map(&input, &m, previous.as_ref(), n, "epoch").unwrap();
        let mut e = envelope(event.clone(), n);
        e.context = context;
        previous = Some(e);
        event
    };
    send(
        json!({"hook_event_name":"UserPromptSubmit","session_id":"s"}),
        1,
    );
    let started = send(
        json!({"hook_event_name":"PreToolUse","session_id":"s","tool_use_id":"a","tool_name":"Bash","tool_input":{"command":"synthetic-a"}}),
        2,
    );
    assert_eq!(started.kind, Kind::Started);
    send(
        json!({"hook_event_name":"PreToolUse","session_id":"s","tool_use_id":"b","tool_name":"Bash","tool_input":{"command":"synthetic-b"}}),
        3,
    );
    let a = send(
        json!({"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Bash","tool_input":{"command":"synthetic-a"}}),
        4,
    );
    let b = send(
        json!({"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Bash","tool_input":{"command":"synthetic-b"}}),
        5,
    );
    assert_eq!(a.call_id.as_deref(), Some("a"));
    assert_eq!(b.call_id.as_deref(), Some("b"));
    assert_ne!(a.request_id, b.request_id);
    let resolved = send(
        json!({"hook_event_name":"PostToolUse","session_id":"s","tool_use_id":"b"}),
        6,
    );
    assert_eq!(resolved.kind, Kind::Resolved);
    assert_eq!(resolved.request_id, b.request_id);
    assert!(!serde_json::to_string(&previous)
        .unwrap()
        .contains("synthetic-b"));
}

#[test]
fn replay_after_user_viewed_never_reopens_unread_or_rewinds_the_latest_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut attention = crate::attention::AttentionTracker::open(dir.path().join("attention.json"));
    let mut state = Session::default();
    let a = apply(&mut state, event(Kind::Started, "a", 1), 1);
    attention.reconcile(vec![a.clone()]);
    let end = apply(&mut state, event(Kind::Completed, "a", 2), 2);
    attention.reconcile(vec![end.clone()]);
    attention.mark_viewed(&end.id).unwrap();
    drop(attention);
    let mut restored = crate::attention::AttentionTracker::open(dir.path().join("attention.json"));
    restored.reconcile(vec![a]);
    restored.reconcile(vec![end]);
    assert!(restored
        .known()
        .iter()
        .all(|task| !task.unread && task.status != TaskStatus::Running));
}

#[cfg(windows)]
fn peak_working_set() -> usize {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
        paged_peak: usize,
        paged: usize,
        nonpaged_peak: usize,
        nonpaged: usize,
        pagefile: usize,
        pagefile_peak: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters: Counters = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<Counters>() as u32;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if ok != 0 {
        counters.peak
    } else {
        0
    }
}
#[cfg(not(windows))]
fn peak_working_set() -> usize {
    0
}

/// Called only by the opt-in benchmark controller, against an owned marker.
#[test]
#[ignore = "isolated multi-process benchmark worker"]
fn integration_benchmark_worker() {
    use std::time::Instant;
    let root = std::path::PathBuf::from(
        std::env::var_os("TASKPULSE_BENCH_ROOT").expect("owned benchmark root required"),
    );
    assert!(root.is_absolute() && root.join("benchmark-owned.json").is_file());
    let worker: usize = std::env::var("TASKPULSE_BENCH_WORKER")
        .unwrap()
        .parse()
        .unwrap();
    assert!(worker < 20);
    let source = std::env::var("TASKPULSE_BENCH_SOURCE").unwrap_or_else(|_| "stress-source".into());
    let agent = std::env::var("TASKPULSE_BENCH_AGENT").unwrap_or_else(|_| "fixture-agent".into());
    let home = root.join("integrations");
    let mut times = vec![];
    let mut retries = 0;
    for n in 0..500 {
        let turn = format!("synthetic-turn-{}", n / 10);
        let kind = match n % 10 {
            0 => Kind::Started,
            1 => Kind::ToolStarted,
            2 => Kind::Requested,
            3 => Kind::Resolved,
            4 => Kind::ToolFinished,
            5 => Kind::Phase,
            6 => Kind::StopRequested,
            7 => Kind::Completed,
            8 => Kind::Renamed,
            _ => Kind::Heartbeat,
        };
        let mut e = event(kind, &turn, (worker * 500 + n) as u64);
        e.agent_id = agent.clone();
        e.session_id = format!("synthetic-worker-{worker}");
        e.occurred_at = None;
        if matches!(
            kind,
            Kind::ToolStarted | Kind::ToolFinished | Kind::Requested | Kind::Resolved
        ) {
            e.call_id = Some(format!("call-{}", n / 10));
        }
        if matches!(kind, Kind::Requested | Kind::Resolved) {
            e.request_id = Some(format!("request-{}", n / 10));
            e.payload.request_kind = Some(WaitingReason::Approval);
        }
        if kind == Kind::Phase {
            e.payload.phase = Some(ExecutionPhase::Thinking);
        }
        let began = Instant::now();
        let mut attempts = 0;
        loop {
            match journal::append(&home, &source, e.clone(), "self_reported", "v1") {
                Ok(_) => break,
                Err(code) if code == "lock_timeout" && attempts < 3 => {
                    attempts += 1;
                    retries += 1;
                }
                Err(code) => panic!("benchmark append failed: {code}"),
            }
        }
        times.push(began.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    journal::atomic(&root.join(format!("worker-{worker}.json")),&json!({"worker":worker,"events":500,"retry_count":retries,"write_ms":times,"peak_working_set_bytes":peak_working_set()})).unwrap();
}

#[test]
#[ignore = "20 real processes, 10000 mixed events; isolated disk benchmark"]
fn integration_concurrent_stress() {
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let target = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
    let root = tempfile::Builder::new()
        .prefix("integration-stress-")
        .tempdir_in(target)
        .unwrap();
    journal::atomic(
        &root.path().join("benchmark-owned.json"),
        &json!({"synthetic":true}),
    )
    .unwrap();
    let mut src = source();
    src.id = "stress-source".into();
    let mut provider = GenericEventProvider::with_home(src, root.path().join("integrations"));
    let began = Instant::now();
    let mut children = vec![];
    for worker in 0..20 {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "integrations::tests::integration_benchmark_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("TASKPULSE_BENCH_ROOT", root.path())
        .env("TASKPULSE_BENCH_WORKER", worker.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        children.push(cmd.spawn().unwrap());
    }
    let deadline = Instant::now() + std::time::Duration::from_secs(180);
    while children
        .iter_mut()
        .any(|child| child.try_wait().unwrap().is_none())
    {
        if Instant::now() >= deadline {
            for child in &mut children {
                let _ = child.kill();
                let _ = child.wait();
            }
            panic!("owned benchmark children exceeded 180 seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let write_elapsed = began.elapsed().as_millis();
    let mut latencies = vec![];
    let mut peak = 0;
    let mut retries = 0;
    for worker in 0..20 {
        let value: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.path().join(format!("worker-{worker}.json"))).unwrap(),
        )
        .unwrap();
        latencies.extend(
            value["write_ms"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap()),
        );
        peak = peak.max(value["peak_working_set_bytes"].as_u64().unwrap());
        retries += value["retry_count"].as_u64().unwrap();
    }
    latencies.sort_by(f64::total_cmp);
    let mut processed = 0;
    let mut finished = 0;
    let mut polls = vec![];
    let mut engine = TransitionEngine::new();
    let mut tracked = std::collections::BTreeMap::new();
    let mut saw = std::collections::HashSet::new();
    for _ in 0..1000 {
        let start = Instant::now();
        let batch = provider.poll(std::time::SystemTime::now());
        polls.push(start.elapsed().as_secs_f64() * 1000.0);
        let mut receipts = std::collections::BTreeMap::new();
        for obs in batch.ordered {
            assert!(saw.insert((
                obs.receipt.session_key.clone(),
                obs.receipt.epoch.clone(),
                obs.receipt.sequence
            )));
            let mut task = obs.task;
            task.unread = true;
            task.new_execution = true;
            // Production TransitionEngine consumes the full known-task view,
            // not a one-row list that removes all peer lifecycle baselines.
            tracked.insert(task.id.clone(), task);
            finished+=engine.apply(&tracked.values().cloned().collect::<Vec<_>>(),&[],std::time::SystemTime::now()).iter().filter(|event|matches!(event,crate::transition::TaskEvent::WaitingInput{task,..} if task.finished_at.is_some())).count();
            receipts.insert(obs.receipt.session_key.clone(), obs.receipt);
            processed += 1;
        }
        for receipt in receipts.values() {
            acknowledge(&root.path().join("integrations"), receipt).unwrap();
        }
        if processed >= 10_000 {
            break;
        }
    }
    assert_eq!(processed, 10_000);
    assert_eq!(finished, 1000);
    polls.sort_by(f64::total_cmp);
    let result = json!({"processes":20,"events":processed,"turn_ends":finished,"write_total_ms":write_elapsed,"write_p50_ms":latencies[latencies.len()/2],"write_p95_ms":latencies[latencies.len()*95/100],"poll_p50_ms":polls[polls.len()/2],"poll_p95_ms":polls[polls.len()*95/100],"max_individual_peak_working_set_bytes":peak,"lock_retries":retries,"ui_latency_measured":false});
    journal::atomic(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/integration-stress-result.json"),
        &result,
    )
    .unwrap();
    println!("{result}");
}

#[test]
fn late_metadata_conflicting_call_and_heartbeat_rename_are_safe() {
    let mut state = Session::default();
    apply(&mut state, event(Kind::Started, "a", 1), 1);
    let b = apply(&mut state, event(Kind::Started, "b", 2), 2);
    let mut late = event(Kind::Started, "a", 3);
    late.payload.title = Some("old title".into());
    assert!(state
        .apply(&envelope(late, 3), "Fixture")
        .unwrap()
        .is_none());
    assert_eq!(state.task.as_ref().unwrap().name, b.name);
    let mut heartbeat = event(Kind::Heartbeat, "b", 4);
    heartbeat.payload.title = Some("renamed while idle".into());
    let renamed = apply(&mut state, heartbeat, 4);
    assert_eq!(renamed.name, "renamed while idle");
    assert_eq!(renamed.updated_at, b.updated_at);
    let mut request = event(Kind::Requested, "b", 5);
    request.request_id = Some("r".into());
    request.call_id = Some("correct".into());
    request.payload.request_kind = Some(WaitingReason::Approval);
    apply(&mut state, request, 5);
    let mut bad = event(Kind::Resolved, "b", 6);
    bad.request_id = Some("r".into());
    bad.call_id = Some("other".into());
    assert!(state.apply(&envelope(bad, 6), "Fixture").unwrap().is_none());
    assert_eq!(state.task.unwrap().input_request_ids, ["r"]);
}

#[test]
fn collector_uncertainty_updates_do_not_replay_lifecycle_or_user_attention() {
    let dir = tempfile::tempdir().unwrap();
    let mut attention = crate::attention::AttentionTracker::open(dir.path().join("attention.json"));
    let mut state = Session::default();
    let task = apply(&mut state, event(Kind::Started, "a", 1), 1);
    attention.reconcile(vec![task.clone()]);
    attention.mark_viewed(&task.id).unwrap();
    let mut degraded = task.clone();
    degraded.stale = true;
    degraded.name = "must not replace title".into();
    attention.reconcile(vec![degraded]);
    let retained = attention.known().pop().unwrap();
    assert!(retained.stale);
    assert!(!retained.unread);
    assert_eq!(retained.name, task.name);
    attention.reconcile(vec![task]);
    assert!(!attention.known()[0].stale);
    assert!(!attention.known()[0].unread);
}

#[test]
fn corrupt_checkpoint_replays_journal_without_replaying_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let src = source();
    let mut provider = GenericEventProvider::with_home(src.clone(), dir.path().into());
    journal::append(
        dir.path(),
        &src.id,
        event(Kind::Started, "a", 1),
        "self_reported",
        "v1",
    )
    .unwrap();
    journal::append(
        dir.path(),
        &src.id,
        event(Kind::Completed, "a", 2),
        "self_reported",
        "v1",
    )
    .unwrap();
    let batch = provider.poll(std::time::SystemTime::now());
    for item in batch.ordered {
        acknowledge(dir.path(), &item.receipt).unwrap();
    }
    let path = journal::root(dir.path(), &src.id)
        .join("state")
        .join(format!("{}.json", super::key("fixture-session")));
    std::fs::write(&path, b"{damaged checkpoint").unwrap();
    let mut provider = GenericEventProvider::with_home(src, dir.path().into());
    let batch = provider.poll(std::time::SystemTime::now());
    assert!(!batch.authoritative);
    assert!(path.with_extension("corrupt.bak").exists());
    assert!(batch.tasks[0].finished_at.is_some());
    assert_eq!(batch.ordered.len(), 2);
    assert!(batch.ordered.iter().all(|o| !o.notify));
}

#[test]
fn trae_structured_idle_is_end_only_after_execution_and_stop_is_not_end() {
    let m = manifest::load("trae-cn-ide").unwrap();
    assert!(manifest::map(&json!({"hook_event_name":"Notification","notification_type":"idle_prompt","session_id":"s"}),&m,None,1,"epoch").is_err());
    let (submitted, ctx) = manifest::map(
        &json!({"hook_event_name":"UserPromptSubmit","session_id":"s"}),
        &m,
        None,
        1,
        "epoch",
    )
    .unwrap();
    assert_eq!(submitted.kind, Kind::Submitted);
    let mut previous = envelope(submitted, 1);
    previous.context = ctx;
    let (started,ctx)=manifest::map(&json!({"hook_event_name":"PreToolUse","session_id":"s","tool_use_id":"c","tool_name":"SyntheticTool"}),&m,Some(&previous),2,"epoch").unwrap();
    assert_eq!(started.kind, Kind::Started);
    previous = envelope(started, 2);
    previous.context = ctx;
    let (stop, _) = manifest::map(
        &json!({"hook_event_name":"Stop","session_id":"s"}),
        &m,
        Some(&previous),
        3,
        "epoch",
    )
    .unwrap();
    assert_eq!(stop.kind, Kind::StopRequested);
    let (end,_)=manifest::map(&json!({"hook_event_name":"Notification","notification_type":"idle_prompt","session_id":"s"}),&m,Some(&previous),4,"epoch").unwrap();
    assert_eq!(end.kind, Kind::Completed);
    assert!(manifest::map(&json!({"hook_event_name":"Notification","notification_type":"arbitrary text","session_id":"s"}),&m,Some(&previous),5,"epoch").is_err());
}
