use super::*;
use serde_json::json;
fn fixture() -> (tempfile::TempDir, Connection, OpenCodeProvider) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("opencode.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY,title TEXT,slug TEXT,directory TEXT,model TEXT,time_archived INTEGER,time_updated INTEGER);CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,seq INTEGER,time_created INTEGER,time_updated INTEGER,data TEXT);
    INSERT INTO session_v2 VALUES('s','Actual native title','native-slug','C:/project','{\"id\":\"source-model\"}',NULL,100000);").unwrap();
    let provider = OpenCodeProvider::new(path);
    (dir, db, provider)
}
fn message(db: &Connection, id: &str, kind: &str, seq: i64, time: u64, data: Value) {
    db.execute(
        "INSERT INTO session_message VALUES(?1,'s',?2,?3,?4,?4,?5)",
        rusqlite::params![id, kind, seq, time, data.to_string()],
    )
    .unwrap();
}
#[test]
fn v2_busy_tools_and_older_idle_cannot_finish_the_current_turn() {
    let (_dir, db, mut p) = fixture();
    message(
        &db,
        "u1",
        "user",
        1,
        100000,
        json!({"time":{"created":100000}}),
    );
    message(
        &db,
        "idle1",
        "idle",
        2,
        101000,
        json!({"time":{"created":101000},"outcome":"failed"}),
    );
    message(
        &db,
        "u2",
        "user",
        3,
        102000,
        json!({"time":{"created":102000}}),
    );
    message(
        &db,
        "a2",
        "assistant",
        4,
        103000,
        json!({"time":{"created":103000,"completed":103500},"finish":"tool-calls","content":[{"type":"tool","state":{"status":"running"}}]}),
    );
    let rows = p.read().unwrap();
    assert_eq!(rows[0].status, TaskStatus::Running);
    assert_eq!(rows[0].turn_key.as_deref(), Some("u2"));
    assert_eq!(rows[0].phase, Some(ExecutionPhase::Tool));
    assert_eq!(rows[0].model.as_deref(), Some("source-model"));
    p.sessions.resume("opencode", rows);
    message(
        &db,
        "idle2",
        "idle",
        5,
        104000,
        json!({"time":{"created":104000},"outcome":"succeeded"}),
    );
    message(
        &db,
        "config",
        "model-switched",
        6,
        105000,
        json!({"time":{"created":105000}}),
    );
    let ended = p.poll(native_sessions::time(106000).unwrap());
    assert_eq!(ended.tasks.len(), 1);
    assert_eq!(ended.tasks[0].status, TaskStatus::Succeeded);
    assert_eq!(ended.tasks[0].turn_key.as_deref(), Some("u2"));
    db.execute(
        "UPDATE session_v2 SET title='Renamed',time_updated=900000",
        [],
    )
    .unwrap();
    let renamed = p.poll(native_sessions::time(900000).unwrap());
    assert_eq!(renamed.tasks[0].name, "Renamed");
    assert!(!renamed.tasks[0].new_execution);
    assert_eq!(renamed.tasks[0].updated_at, ended.tasks[0].updated_at);
}
#[test]
fn model_variant_and_duration_belong_to_the_turn_on_a_cold_read() {
    let (_dir, db, p) = fixture();
    message(
        &db,
        "u",
        "user",
        1,
        100000,
        json!({"metadata":{"model":{"modelID":"requested-model","variant":"high"}}}),
    );
    let row = p.read().unwrap().remove(0);
    assert_eq!(row.model.as_deref(), Some("requested-model"));
    assert_eq!(row.model_variant.as_deref(), Some("high"));

    message(
        &db,
        "a",
        "assistant",
        2,
        101000,
        json!({"model":{"id":"actual-model","variant":"custom-preset"}}),
    );
    message(
        &db,
        "end",
        "idle",
        3,
        102000,
        json!({"outcome":"completed"}),
    );
    db.execute(
        "UPDATE session_message SET time_updated=900000 WHERE id='end'",
        [],
    )
    .unwrap();
    let row = p.read().unwrap().remove(0);
    assert_eq!(row.model.as_deref(), Some("actual-model"));
    assert_eq!(row.model_variant.as_deref(), Some("custom-preset"));
    assert!(row.reasoning_effort.is_none());
    assert_eq!(row.finished_at, native_sessions::time(102000));

    message(&db, "new", "user", 4, 910000, json!({}));
    message(
        &db,
        "end-again",
        "idle",
        5,
        920000,
        json!({"outcome":"failed"}),
    );
    let row = p.read().unwrap().remove(0);
    assert!(row.model.is_none() && row.model_variant.is_none());
}

#[test]
fn idle_history_is_catalog_only_unknown_outcomes_and_io_failures_do_not_invent_success() {
    let (_dir, db, mut p) = fixture();
    message(
        &db,
        "u",
        "user",
        1,
        100000,
        json!({"time":{"created":100000}}),
    );
    message(
        &db,
        "idle",
        "idle",
        2,
        101000,
        json!({"time":{"created":101000},"outcome":"succeeded"}),
    );
    let historical = p.poll(native_sessions::time(200000).unwrap());
    assert!(historical.tasks.is_empty());
    assert_eq!(historical.catalog.len(), 1);
    p.resume(historical.catalog);
    db.execute(
        "UPDATE session_message SET data='{\"time\":{\"created\":101000}}' WHERE id='idle'",
        [],
    )
    .unwrap();
    let missing = p.poll(native_sessions::time(200001).unwrap());
    assert_eq!(missing.tasks[0].status, TaskStatus::Unknown);
    assert!(missing.tasks[0].finished_at.is_none());

    db.execute("UPDATE session_message SET data='{\"outcome\":\"unknown\",\"time\":{\"created\":101000}}' WHERE id='idle'",[]).unwrap();
    let uncertain = p.poll(native_sessions::time(201000).unwrap());
    assert_eq!(uncertain.tasks[0].status, TaskStatus::Unknown);
    assert!(uncertain.tasks[0].finished_at.is_none());
    db.execute_batch("DROP TABLE session_message").unwrap();
    let degraded = p.poll(native_sessions::time(202000).unwrap());
    assert_eq!(degraded.tasks[0].status, TaskStatus::Unknown);
    assert!(degraded.tasks[0].stale);
}
#[test]
fn legacy_message_finish_requires_current_parent_and_an_explicit_final_finish() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE session(id TEXT,title TEXT,directory TEXT,time_archived INTEGER,time_updated INTEGER);CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);INSERT INTO session VALUES('s','Old format','/project',NULL,100000);INSERT INTO message VALUES('u','s',100000,100000,'{\"role\":\"user\",\"time\":{\"created\":100000}}');").unwrap();
    let p = OpenCodeProvider::new(path);
    for (parent, finish, status) in [
        ("other", "stop", TaskStatus::Running),
        ("u", "tool-calls", TaskStatus::Running),
        ("u", "stop", TaskStatus::Succeeded),
    ] {
        db.execute("DELETE FROM message WHERE id='a'", []).unwrap();
        db.execute("INSERT INTO message VALUES('a','s',101000,102000,?1)",[json!({"id":"assistant-message-id","role":"assistant","parentID":parent,"modelID":"legacy-model","variant":"high","time":{"created":101000,"completed":102000},"finish":finish}).to_string()]).unwrap();
        let task = p.read().unwrap().remove(0);
        assert_eq!(task.status, status);
        assert_eq!(
            task.model.as_deref(),
            (parent == "u").then_some("legacy-model")
        );
        assert_eq!(
            task.model_variant.as_deref(),
            (parent == "u").then_some("high")
        );
    }
}
#[test]
#[ignore = "read-only real local OpenCode database"]
fn live_opencode_snapshot() {
    let p = OpenCodeProvider::new(default_db());
    let rows = p.read().expect("native database must be readable");
    assert!(!rows.is_empty(), "run one real OpenCode task first");
    assert!(rows
        .iter()
        .all(|t| t.agent_id.as_deref() == Some("opencode")
            && t.turn_key.is_some()
            && t.started_at.is_some()));
    println!(
        "OpenCode native rows={} running={} ended={} models={} variants={}",
        rows.len(),
        rows.iter()
            .filter(|t| t.status == TaskStatus::Running)
            .count(),
        rows.iter().filter(|t| t.finished_at.is_some()).count(),
        rows.iter().filter(|t| t.model.is_some()).count(),
        rows.iter().filter(|t| t.model_variant.is_some()).count()
    );
}
