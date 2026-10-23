use super::*;
use crate::{
    attention::AttentionTracker,
    domain::{DetectionConfidence, TaskSource, TaskStatus, WaitingReason},
    store::TaskStore,
    transition::TransitionEngine,
};
use std::collections::HashSet;
use std::time::UNIX_EPOCH;

fn running(id: &str, source: &str, now: SystemTime) -> TaskSnapshot {
    let mut t = TaskSnapshot::new(
        format!("{source}:codex:{id}"),
        "private title",
        TaskSource::Codex,
        TaskStatus::Running,
        DetectionConfidence::Hook,
        now,
    );
    t.source_instance = Some(source.into());
    t.session_id = Some(id.into());
    t.turn_key = Some("stable-turn".into());
    t
}

#[test]
fn wsl_continuity_failures_partial_recovery_end_restart_and_peer_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let source = "codex-wsl:test";
    let mut provider = WslCodexProvider::new("-invalid".into(), source.into());
    let one = running("one", source, now);
    let two = running("two", source, now);
    let peer = running("peer", "codex", now);
    let mut store = TaskStore::default();
    let mut attention = AttentionTracker::open(dir.path().join("attention.json"));
    let mut engine = TransitionEngine::open(dir.path().join("notifications.json"));
    let mut output = ProviderOutput::tasks(vec![one.clone(), two.clone()]);
    provider.accept_sample(&mut output);
    let mut merge = |output: ProviderOutput,
                     attention: &mut AttentionTracker,
                     engine: &mut TransitionEngine| {
        let sources = if output.authoritative {
            HashSet::from([source.to_string(), "codex".into()])
        } else {
            HashSet::from(["codex".into()])
        };
        let mut tasks = output.tasks;
        tasks.push(peer.clone());
        let merged = store.merge_instances(tasks, &sources, SystemTime::now());
        let display = attention.reconcile(merged);
        let events = engine.apply(&display, &[], SystemTime::now());
        (display, events)
    };
    let (display, events) = merge(output, &mut attention, &mut engine);
    assert_eq!(display.len(), 3);
    assert!(events.is_empty());
    attention.set_pinned(&one.id, true).unwrap();
    // One and two consecutive metadata failures cannot clear either session,
    // promote an end, erase preferences, or degrade an independent source.
    for _ in 0..2 {
        let failed = provider.poll(now);
        assert!(!failed.authoritative);
        assert_eq!(failed.tasks.len(), 2);
        let (display, events) = merge(failed, &mut attention, &mut engine);
        assert!(events.is_empty());
        assert_eq!(display.iter().filter(|t| t.stale).count(), 2);
        assert!(display
            .iter()
            .find(|t| t.id == peer.id)
            .is_some_and(|t| !t.stale));
        assert!(display
            .iter()
            .find(|t| t.id == one.id)
            .is_some_and(|t| t.pinned && t.unread && t.status == TaskStatus::Running));
    }
    let mut partial = ProviderOutput::degraded(vec![], "[wsl:rollout_read:catch_up]");
    provider.accept_sample(&mut partial);
    assert_eq!(provider.last_authoritative_tasks.len(), 2);
    assert_eq!(partial.tasks.len(), 2);
    assert!(merge(partial, &mut attention, &mut engine).1.is_empty());
    // Partial success for one session is not staleness for its fresh peer.
    let mut partial = ProviderOutput::degraded(vec![two.clone()], "[wsl:rollout_read:io]");
    provider.accept_sample(&mut partial);
    let (display, events) = merge(partial, &mut attention, &mut engine);
    assert!(events.is_empty());
    assert!(display.iter().find(|t| t.id == one.id).unwrap().stale);
    assert!(!display.iter().find(|t| t.id == two.id).unwrap().stale);
    let mut recovered = ProviderOutput::tasks(vec![one.clone(), two.clone()]);
    provider.accept_sample(&mut recovered);
    let (display, events) = merge(recovered, &mut attention, &mut engine);
    assert!(events.is_empty());
    assert!(display.iter().all(|t| !t.stale));
    // A complete per-session terminal in a globally partial batch supersedes
    // Running for that session; later faults must not resurrect Running.
    let mut ended = one.clone();
    ended.status = TaskStatus::WaitingInput;
    ended.waiting_reason = Some(WaitingReason::TurnEnded);
    ended.updated_at = now + Duration::from_secs(1);
    let mut partial = ProviderOutput::degraded(
        vec![ended.clone(), two.clone()],
        "[wsl:rollout_read:catch_up]",
    );
    provider.accept_sample(&mut partial);
    let (_, events) = merge(partial, &mut attention, &mut engine);
    assert_eq!(events.len(), 1);
    let failed = provider.poll(now);
    assert_eq!(
        failed
            .tasks
            .iter()
            .find(|t| t.id == one.id)
            .unwrap()
            .waiting_reason,
        Some(WaitingReason::TurnEnded)
    );
    assert!(merge(failed, &mut attention, &mut engine).1.is_empty());
    let mut older = ProviderOutput::tasks(vec![one.clone(), two.clone()]);
    provider.accept_sample(&mut older);
    assert!(!older.authoritative);
    assert_eq!(
        older
            .tasks
            .iter()
            .find(|t| t.id == one.id)
            .unwrap()
            .waiting_reason,
        Some(WaitingReason::TurnEnded)
    );
    assert!(merge(older, &mut attention, &mut engine).1.is_empty());
    let mut recovered = ProviderOutput::tasks(vec![ended, two]);
    provider.accept_sample(&mut recovered);
    let (_, events) = merge(recovered, &mut attention, &mut engine);
    assert!(events.is_empty());
    drop(merge);
    let restored = AttentionTracker::open(dir.path().join("attention.json"));
    let one = restored
        .known()
        .into_iter()
        .find(|t| t.id == one.id)
        .unwrap();
    assert!(one.unread && one.pinned && one.stale);
    assert_eq!(one.turn_key.as_deref(), Some("stable-turn"));
    let mut restarted = TransitionEngine::open(dir.path().join("notifications.json"));
    assert!(restarted
        .apply(&attention.display(), &[], SystemTime::now())
        .is_empty());
}

#[test]
fn wsl_continuity_real_parser_pin_corruption_missing_and_explicit_end() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("home/test/.codex");
    std::fs::create_dir_all(root.join("sessions")).unwrap();
    let path = root.join("sessions/rollout-session.jsonl");
    let timestamp = "2026-10-23T00:00:00Z";
    let now = crate::providers::codex::parse_rfc3339(timestamp).unwrap();
    let start = format!(
        "{}\n{}\n",
        serde_json::json!({"type":"session_meta","payload":{"id":"one"}}),
        serde_json::json!({"timestamp":timestamp,"payload":{"type":"task_started","turn_id":"turn"}})
    );
    std::fs::write(&path, &start).unwrap();
    let linux_path = "/home/test/.codex/sessions/rollout-session.jsonl";
    let row=serde_json::from_value(serde_json::json!({"id":"one","is_pinned":true,"rollout_path":linux_path,"updated_at":now.duration_since(UNIX_EPOCH).unwrap().as_secs()})).unwrap();
    let mut inner = CodexProvider::with_wsl_root(root, dir.path().into(), "codex-wsl:test");
    inner.set_external_metadata(&[row]);
    let stamp = |version| super::super::jsonl_tail::LinuxFileStamp {
        dev: 1,
        ino: 1,
        len: path.metadata().unwrap().len(),
        modified_ns: now.duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64 + version,
        changed_ns: now.duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64 + version,
    };
    let set = |inner: &mut CodexProvider, version| {
        inner.set_linux_rollouts(vec![LinuxRollout {
            path: linux_path.into(),
            stamp: stamp(version),
        }])
    };
    set(&mut inner, 1);
    let first = inner.poll(now);
    assert!(first.authoritative);
    assert_eq!(first.tasks[0].status, TaskStatus::Running);
    // A transient UNC open/size failure makes tail untrusted; the native pin
    // cannot manufacture a fresh Idle fallback and erase the running count.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{bad}\n")
        .unwrap();
    set(&mut inner, 2);
    let broken = inner.poll(now);
    assert!(!broken.authoritative);
    assert_eq!(broken.tasks[0].status, TaskStatus::Running);
    assert!(broken.tasks[0].stale);
    assert!(broken.error.unwrap().contains("rollout_decode:gap"));
    inner.set_linux_rollouts(vec![]);
    let missing = inner.poll(now);
    assert!(!missing.authoritative);
    assert!(missing.tasks[0].stale);
    // Genuine replacement, not an assumed end: parse the explicit lifecycle.
    std::fs::write(&path,format!("{start}{}\n",serde_json::json!({"timestamp":timestamp,"payload":{"type":"task_complete","turn_id":"turn"}}))).unwrap();
    set(&mut inner, 3);
    let end = inner.poll(now);
    assert!(end.authoritative);
    assert_eq!(end.tasks[0].waiting_reason, Some(WaitingReason::TurnEnded));
    assert!(!end.tasks[0].stale);
    std::fs::remove_file(&path).unwrap();
    let missing = inner.poll(now);
    assert!(!missing.authoritative);
    assert_ne!(missing.tasks[0].status, TaskStatus::Running);
}

#[test]
fn wsl_continuity_incomplete_resumed_file_cannot_publish_an_old_rollout_end() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("home/test/.codex");
    std::fs::create_dir_all(root.join("sessions")).unwrap();
    let current = root.join("sessions/rollout-current.jsonl");
    let old = root.join("sessions/rollout-old.jsonl");
    let now = crate::providers::codex::parse_rfc3339("2026-10-23T00:00:00Z").unwrap();
    let meta = serde_json::json!({"type":"session_meta","payload":{"id":"one"}});
    let start = serde_json::json!({"timestamp":"2026-10-23T00:00:00Z","payload":{"type":"task_started","turn_id":"current"}});
    std::fs::write(&current, format!("{meta}\n{start}\n")).unwrap();
    let mut inner = CodexProvider::with_wsl_root(root, dir.path().into(), "codex-wsl:test");
    let first = inner.poll(now);
    assert!(first.authoritative);
    assert_eq!(first.tasks[0].status, TaskStatus::Running);
    let end = serde_json::json!({"timestamp":"2026-10-22T23:59:59Z","payload":{"type":"task_complete","turn_id":"old"}});
    std::fs::write(&old, format!("{meta}\n{end}\n")).unwrap();
    let huge = serde_json::json!({"type":"compacted","payload":{"text":"x".repeat(18*1024*1024)}});
    std::fs::OpenOptions::new()
        .append(true)
        .open(&current)
        .unwrap()
        .write_all(format!("{huge}\n").as_bytes())
        .unwrap();
    let partial = inner.poll(now);
    assert!(!partial.authoritative);
    assert_eq!(partial.tasks.len(), 1);
    assert!(partial.tasks[0].stale);
    assert_eq!(
        partial.tasks[0].status,
        TaskStatus::Running,
        "an older trusted file is not authoritative for an incompletely read resumed session"
    );
    let recovered = inner.poll(now);
    assert!(recovered.authoritative);
    assert_eq!(recovered.tasks[0].status, TaskStatus::Running);
}

#[test]
#[ignore = "read-only existing WSL logs; never starts a distro or Agent task"]
fn live_wsl_continuity_diagnostics() {
    let distro = std::env::var("TASKPULSE_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu".into());
    assert!(
        running_distros()
            .unwrap()
            .iter()
            .any(|d| d.eq_ignore_ascii_case(&distro)),
        "distro stopped: do not wake"
    );
    let dir = tempfile::tempdir().unwrap();
    let mut provider = WslCodexProvider::new(distro, "codex-wsl:diagnostic".into());
    let mut store = TaskStore::default();
    let mut attention = AttentionTracker::open(dir.path().join("attention.json"));
    let mut engine = TransitionEngine::new();
    let mut cycles = vec![];
    let mut hot = 0;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(60) && hot < 4 {
        let began = std::time::Instant::now();
        let output = provider.poll(SystemTime::now());
        let source = HashSet::from(["codex-wsl:diagnostic".to_string()]);
        let empty = HashSet::new();
        let merged = store.merge_instances(
            output.tasks,
            if output.authoritative {
                &source
            } else {
                &empty
            },
            SystemTime::now(),
        );
        let display = attention.reconcile(merged);
        let events = engine.apply(&display, &[], SystemTime::now());
        let parsers = super::super::resources::snapshot()["parser_instances"].clone();
        cycles.push(serde_json::json!({"poll_ms":began.elapsed().as_millis(),"authoritative":output.authoritative,"error":output.error,
            "display_count":display.len(),"ongoing":display.iter().filter(|t|t.status==TaskStatus::Running && !t.stale).count(),
            "unconfirmed_running":display.iter().filter(|t|t.status==TaskStatus::Running && t.stale).count(),"event_count":events.len(),"parsers":parsers}));
        if output.authoritative {
            hot += 1;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let evidence = serde_json::json!({"read_only":true,"cycles":cycles,"continuity":super::super::resources::continuity_snapshot()});
    let folder =
        PathBuf::from(std::env::var("CARGO_TARGET_DIR").expect("use an isolated Cargo target"))
            .join("ui-smoke/evidence");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(format!(
        "wsl-continuity-{}.json",
        super::super::resources::epoch_ms(SystemTime::now())
    ));
    std::fs::write(&path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    let summary:Vec<_>=cycles.iter().map(|cycle|serde_json::json!({"poll_ms":cycle["poll_ms"],"authoritative":cycle["authoritative"],"error":cycle["error"],"ongoing":cycle["ongoing"],"unconfirmed_running":cycle["unconfirmed_running"],"event_count":cycle["event_count"],"parsers":cycle["parsers"].as_object().unwrap().values().map(|p|serde_json::json!({"entries":p["entries"],"bytes":p["cumulative_rollout_bytes_read"],"poll_bytes":p["poll_rollout_bytes_read"],"resets":p["reset_count"],"validation_bytes":p["validation_bytes"]})).collect::<Vec<_>>()})).collect();
    println!("{}", serde_json::json!({"evidence":path,"cycles":summary}));
    assert_eq!(
        hot, 4,
        "did not reach complete authoritative collection; inspect safe stage codes"
    );
}
