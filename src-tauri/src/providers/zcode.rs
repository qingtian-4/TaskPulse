//! Read-only ZCode native sessions. Tool completion is never turn completion.
use super::{Provider, ProviderOutput};
use crate::domain::{
    DetectionConfidence, ExecutionPhase, TaskSnapshot, TaskSource, TaskStatus, WaitingReason,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// Keep model selection on the real user row, outside the bounded assistant
// tail. Project only short metadata fields; never materialize a prompt.
const USER_TURN_SQL: &str = "
    WITH latest_user AS (
        SELECT id, sequence, data FROM message
        WHERE session_id=?1 AND json_extract(data,'$.role')='user'
          AND coalesce(json_extract(data,'$.synthetic'),0)=0
          AND (json_extract(data,'$.semantics.origin') IS NULL
               OR json_extract(data,'$.semantics.origin')='real_user')
        ORDER BY sequence DESC LIMIT 1
    ), model_metadata AS (
        SELECT id, sequence, json_extract(data,'$.time.created') AS created,
            CASE WHEN json_type(data,'$.modelSelection.modelId')='text' THEN data ->> '$.modelSelection.modelId' END AS selected_model,
            CASE WHEN json_type(data,'$.modelSelection.providerId')='text' THEN data ->> '$.modelSelection.providerId' END AS selected_provider,
            CASE WHEN json_type(data,'$.modelSelection.options.reasoningLevel')='text' THEN data ->> '$.modelSelection.options.reasoningLevel' END AS selected_effort,
            coalesce(
                CASE WHEN json_type(data,'$.model.modelID')='text' THEN data ->> '$.model.modelID' END,
                CASE WHEN json_type(data,'$.model.modelId')='text' THEN data ->> '$.model.modelId' END) AS legacy_model,
            coalesce(
                CASE WHEN json_type(data,'$.model.providerID')='text' THEN data ->> '$.model.providerID' END,
                CASE WHEN json_type(data,'$.model.providerId')='text' THEN data ->> '$.model.providerId' END) AS legacy_provider,
            CASE WHEN json_type(data,'$.model.variant')='text' THEN data ->> '$.model.variant' END AS legacy_effort
        FROM latest_user
    )
    SELECT id, created, sequence,
        CASE WHEN typeof(selected_model)='text' AND length(selected_model) <= 200 THEN selected_model END,
        CASE WHEN typeof(selected_provider)='text' AND length(selected_provider) <= 200 THEN selected_provider END,
        CASE WHEN typeof(selected_effort)='text' AND length(selected_effort) <= 64 THEN selected_effort END,
        CASE WHEN typeof(legacy_model)='text' AND length(legacy_model) <= 200 THEN legacy_model END,
        CASE WHEN typeof(legacy_provider)='text' AND length(legacy_provider) <= 200 THEN legacy_provider END,
        CASE WHEN typeof(legacy_effort)='text' AND length(legacy_effort) <= 64 THEN legacy_effort END
    FROM model_metadata";

#[derive(Default)]
struct ModelSelection {
    model: Option<String>,
    provider: Option<String>,
    effort: Option<String>,
}

impl ModelSelection {
    fn read(row: &rusqlite::Row<'_>, first: usize) -> rusqlite::Result<Self> {
        let field = |n| {
            Ok::<_, rusqlite::Error>(
                row.get::<_, Option<String>>(first + n)?
                    .as_deref()
                    .and_then(super::reported_model),
            )
        };
        Ok(Self {
            model: field(0)?,
            provider: field(1)?,
            effort: field(2)?,
        })
    }

    fn agrees_with(&self, model: &str, provider: Option<&str>) -> bool {
        self.model.as_deref() == Some(model)
            && self
                .provider
                .as_deref()
                .zip(provider)
                .is_none_or(|(a, b)| a == b)
    }
}

pub fn default_db() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_default()
        .join(".zcode/cli/db/db.sqlite")
}

/// The desktop index is a sibling of the native CLI DB in ZCode's standard layout.
pub fn desktop_index(runtime_db: &Path) -> Option<PathBuf> {
    if !runtime_db.ends_with("cli/db/db.sqlite") {
        return None;
    }
    let root = runtime_db.parent()?.parent()?.parent()?;
    Some(root.join("v2/tasks-index.sqlite"))
}

/// Read project paths only. A desktop project may have no readable runtime turn.
pub fn local_project_roots(runtime_db: &Path) -> Vec<String> {
    fn read(path: &Path) -> rusqlite::Result<Vec<String>> {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(200))?;
        let project_tables: u32 = db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN ('projects','project_roots')",
            [],
            |row| row.get(0),
        )?;
        let sql = if project_tables == 0 {
            // Earlier ZCode desktops attach tasks directly to local workspaces.
            // Legacy remote rows can lack workspace_identity; check the key too.
            "SELECT DISTINCT workspace_path FROM tasks
             WHERE deleted=0 AND typeof(workspace_path)='text'
               AND length(workspace_path) BETWEEN 1 AND 4096
               AND typeof(workspace_key)='text' AND workspace_key NOT GLOB 'remote:*'
               AND coalesce(workspace_identity,'') NOT GLOB 'remote:*'
             ORDER BY workspace_path LIMIT 1024"
        } else {
            "SELECT DISTINCT r.root_path FROM project_roots r JOIN projects p USING(project_id)
             WHERE p.host_id='local' AND p.deleted=0 AND typeof(r.root_path)='text'
               AND length(r.root_path) BETWEEN 1 AND 4096
             ORDER BY r.root_path LIMIT 1024"
        };
        let mut stmt = db.prepare(sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    desktop_index(runtime_db)
        .and_then(|path| read(&path).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|path| {
            let path = path.as_str();
            let windows = cfg!(windows);
            let literal = if windows {
                path.strip_prefix(r"\\?\").unwrap_or(path)
            } else {
                path
            };
            Path::new(path).is_absolute()
                && !literal.contains(['*', '?', '\0', '\r', '\n'])
                && !Path::new(path).ends_with(".zcode/workspace/default")
        })
        .collect()
}

pub struct ZcodeProvider {
    path: PathBuf,
    previous: HashMap<String, TaskSnapshot>,
    followed: HashSet<String>,
    baseline: Option<SystemTime>,
}
impl ZcodeProvider {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            previous: HashMap::new(),
            followed: HashSet::new(),
            baseline: None,
        }
    }
    pub fn resume(&mut self, known: impl IntoIterator<Item = TaskSnapshot>) {
        for mut task in known {
            if let Some(session) = task.session_id.as_ref() {
                task.id = format!("zcode:{session}");
                task.new_execution = false;
                self.followed.insert(task.id.clone());
                self.previous.insert(task.id.clone(), task);
            }
        }
    }
    fn read(path: &Path) -> rusqlite::Result<Vec<TaskSnapshot>> {
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(250))?;
        let tx = db.transaction()?;
        let (has_outcome, has_deleted): (bool, bool) = tx.query_row(
            "SELECT count(*) FILTER (WHERE name='last_outcome')>0,
                    count(*) FILTER (WHERE name='time_deleted')>0 FROM pragma_table_info('session')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        // Older desktops do not expose these two fields. Their absence
        // is not lifecycle evidence.
        let outcome_column = if has_outcome { "last_outcome" } else { "NULL" };
        let deleted = if has_deleted {
            "time_deleted IS NULL AND "
        } else {
            ""
        };
        let mut sessions = tx.prepare(&format!(
            "SELECT id, title, directory, {outcome_column} FROM session WHERE {deleted}time_archived IS NULL ORDER BY time_updated DESC LIMIT 400"
        ))?;
        let rows = sessions.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, title, project, outcome) = row?;
            // Runtime reminders also have role=user. Only an actual user prompt
            // creates a turn, and it must remain discoverable through long tool loops.
            let user = tx
                .query_row(USER_TURN_SQL, [&id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, i64>(2)?,
                        ModelSelection::read(row, 3)?,
                        ModelSelection::read(row, 6)?,
                    ))
                })
                .optional()?;
            let Some((user_id, started, user_sequence, selection, legacy)) = user else {
                continue;
            };
            let mut messages = tx.prepare("SELECT id, json_object('role',json_extract(data,'$.role'),'created',json_extract(data,'$.time.created'),'completed',json_extract(data,'$.time.completed'),'parent',json_extract(data,'$.parentID'),'model',coalesce(json_extract(data,'$.modelId'),json_extract(data,'$.modelID')),'provider',coalesce(json_extract(data,'$.providerId'),json_extract(data,'$.providerID')),'finish',json_extract(data,'$.finish'),'error_code',json_extract(data,'$.error.data.code'),'turn_result',json_extract(data,'$.error.data.turnResult'),'error',json_type(data,'$.error') NOT IN ('null') AND json_type(data,'$.error') IS NOT NULL) FROM message WHERE session_id=?1 AND sequence>=?2 AND (json_extract(data,'$.role')='assistant' OR id=?3) ORDER BY sequence DESC LIMIT 64")?;
            let messages = messages
                .query_map(rusqlite::params![id, user_sequence, user_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let messages: Vec<_> = messages
                .into_iter()
                .filter_map(|(id, json)| serde_json::from_str::<Value>(&json).ok().map(|v| (id, v)))
                .collect();
            let Some((message_id, latest)) = messages.first() else {
                continue;
            };
            let activity = latest["completed"]
                .as_u64()
                .or_else(|| latest["created"].as_u64())
                .unwrap_or(0);
            if started == 0 || started > 4_102_444_800_000 || activity > 4_102_444_800_000 {
                continue;
            }
            let mut task = TaskSnapshot::new(
                format!("zcode:{id}"),
                title,
                TaskSource::NativeAgent,
                TaskStatus::Running,
                DetectionConfidence::Inferred,
                UNIX_EPOCH + Duration::from_millis(activity.max(started)),
            );
            task.agent_id = Some("zcode".into());
            task.agent_name = Some("ZCode".into());
            task.evidence_channel = Some("native".into());
            task.session_id = Some(id.clone());
            task.project_path = (!project.is_empty()).then_some(project.clone());
            task.project = Path::new(&project)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned());
            task.turn_key = Some(user_id.clone());
            task.started_at = Some(UNIX_EPOCH + Duration::from_millis(started));
            let assistant_model = messages
                .iter()
                .filter(|(_, v)| v["parent"].as_str() == Some(user_id.as_str()))
                .find_map(|(_, v)| {
                    let model = v["model"].as_str().and_then(super::reported_model)?;
                    Some((model, v["provider"].as_str()))
                });
            let selected_model = selection
                .model
                .as_deref()
                .map(|model| (model, selection.provider.as_deref()))
                .or_else(|| {
                    legacy
                        .model
                        .as_deref()
                        .map(|model| (model, legacy.provider.as_deref()))
                });
            if let Some((model, provider)) = assistant_model
                .as_ref()
                .map(|(model, provider)| (model.as_str(), *provider))
                .or(selected_model)
            {
                task.model = Some(model.to_owned());
                // If the assistant actually used a different model or provider,
                // the user's requested reasoning level cannot describe it.
                if selection.agrees_with(model, provider) {
                    task.reasoning_effort = selection.effort.clone();
                }
                if task.reasoning_effort.is_none()
                    && selection
                        .model
                        .as_ref()
                        .is_none_or(|_| selection.agrees_with(model, provider))
                    && legacy.agrees_with(model, provider)
                {
                    task.reasoning_effort = legacy.effort;
                }
            }
            // Native outcome alone can belong to an older turn. Require the latest
            // assistant to explicitly reference this user turn and finish it.
            let current_assistant = latest["role"] == "assistant"
                && latest["parent"].as_str() == Some(user_id.as_str())
                && latest["completed"].as_u64().is_some();
            if current_assistant
                && latest["finish"] != "tool-calls"
                && (latest["error_code"] == "model_request_cancelled"
                    || latest["turn_result"] == "cancelled")
            {
                task.status = TaskStatus::WaitingInput;
                task.waiting_reason = Some(WaitingReason::TurnEnded);
                task.outcome = Some("interrupted".into());
            } else if current_assistant && latest["error"].as_i64() == Some(1) {
                task.status = TaskStatus::Failed;
            } else if current_assistant
                && outcome.as_deref() == Some("completed")
                && latest["finish"] == "stop"
            {
                task.status = TaskStatus::Succeeded;
            } else if !has_outcome && current_assistant && latest["finish"] == "stop" {
                // Older storage has no native outcome to corroborate a stop.
                task.status = TaskStatus::Unknown;
                task.stale = true;
            }
            if task.status == TaskStatus::Running {
                let mut parts = tx.prepare("SELECT json_extract(data,'$.type'), json_extract(data,'$.state.status') FROM part WHERE message_id=?1 ORDER BY sequence DESC LIMIT 64")?;
                let parts = parts
                    .query_map([message_id], |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                task.phase = Some(
                    if parts.iter().any(|(kind, status)| {
                        kind.as_deref() == Some("tool")
                            && matches!(status.as_deref(), Some("pending" | "running"))
                    }) {
                        ExecutionPhase::Tool
                    } else if parts
                        .iter()
                        .any(|(kind, _)| kind.as_deref() == Some("reasoning"))
                    {
                        ExecutionPhase::Thinking
                    } else {
                        ExecutionPhase::AwaitingModel
                    },
                );
            } else if task.status != TaskStatus::Unknown {
                task.finished_at = Some(task.updated_at);
            }
            out.push(task);
        }
        Ok(out)
    }
    fn observe(&mut self, rows: Vec<TaskSnapshot>, now: SystemTime) -> ProviderOutput {
        let mut current = HashMap::new();
        for mut task in rows {
            let old = self.previous.get(&task.id);
            task.new_execution = old.is_some_and(|old| old.turn_key != task.turn_key)
                || old.is_none()
                    && self.baseline.is_some_and(|baseline| {
                        task.started_at.is_some_and(|started| started >= baseline)
                    });
            let fresh =
                now.duration_since(task.updated_at).unwrap_or_default() < Duration::from_secs(120);
            let resumed_activity = old.is_some_and(|old| task.updated_at > old.updated_at)
                && self
                    .baseline
                    .is_some_and(|baseline| task.updated_at >= baseline);
            if task.new_execution
                || resumed_activity
                || old.is_none() && task.status == TaskStatus::Running && fresh
            {
                self.followed.insert(task.id.clone());
            }
            // Silence affects freshness only, never lifecycle or turn identity.
            task.stale |= task.status == TaskStatus::Running && !fresh;
            current.insert(task.id.clone(), task);
        }
        for (id, old) in &self.previous {
            if self.followed.contains(id)
                && !current.contains_key(id)
                && old.status == TaskStatus::Running
            {
                let mut task = old.clone();
                task.stale = true;
                task.new_execution = false;
                current.insert(id.clone(), task);
            }
        }
        if current.len() > 1000 {
            self.followed.retain(|id| self.previous.contains_key(id));
            return ProviderOutput::degraded(
                self.previous
                    .values()
                    .filter(|t| self.followed.contains(&t.id))
                    .cloned()
                    .map(|mut task| {
                        task.stale = true;
                        task.new_execution = false;
                        task
                    })
                    .collect(),
                "ZCode 原生会话数量超过读取预算；保留上次状态",
            );
        }
        self.followed.retain(|id| current.contains_key(id));
        let tasks = current
            .values()
            .filter(|t| self.followed.contains(&t.id))
            .cloned()
            .collect();
        let catalog = current.values().cloned().collect();
        self.previous = current;
        self.baseline.get_or_insert(now);
        ProviderOutput {
            tasks,
            catalog,
            ..Default::default()
        }
    }
}
impl Provider for ZcodeProvider {
    fn name(&self) -> &'static str {
        "zcode"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match Self::read(&self.path) {
            Ok(rows) => self.observe(rows, now),
            Err(error) => ProviderOutput::degraded(
                self.previous
                    .values()
                    .filter(|t| self.followed.contains(&t.id))
                    .cloned()
                    .map(|mut t| {
                        t.stale = true;
                        t.new_execution = false;
                        t
                    })
                    .collect(),
                match error.sqlite_error_code() {
                    Some(rusqlite::ErrorCode::CannotOpen)
                        if self
                            .path
                            .metadata()
                            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
                    {
                        "未找到 ZCode 任务数据库；桌面项目可单独读取，保留上次任务状态"
                    }
                    Some(
                        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked,
                    ) => "ZCode 任务数据库暂被占用；保留上次任务状态",
                    _ => "ZCode 任务数据库无法读取或格式不兼容；保留上次任务状态",
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(path: &Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch("CREATE TABLE session(id TEXT,title TEXT,directory TEXT,last_outcome TEXT,time_deleted INTEGER,time_archived INTEGER,time_updated INTEGER); CREATE TABLE message(id TEXT,session_id TEXT,sequence INTEGER,data TEXT); CREATE TABLE part(id TEXT,message_id TEXT,sequence INTEGER,data TEXT); INSERT INTO session VALUES('s','Real title','D:/project','completed',NULL,NULL,3000);").unwrap();
        message(&db, "u1", 1, json!({"role":"user","time":{"created":1000}}));
        message(
            &db,
            "a1",
            2,
            json!({"role":"assistant","parentID":"u1","modelId":"model-a","time":{"created":1100,"completed":2000},"finish":"stop"}),
        );
        db
    }
    fn message(db: &Connection, id: &str, seq: u32, v: Value) {
        db.execute(
            "INSERT INTO message VALUES(?1,'s',?2,?3)",
            rusqlite::params![id, seq, v.to_string()],
        )
        .unwrap();
    }
    #[test]
    fn native_turn_model_metadata_comes_from_the_real_user_even_after_long_tool_loop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let db = fixture(&path);
        let mut provider = ZcodeProvider::new(path);
        let now = UNIX_EPOCH + Duration::from_millis(5000);
        provider.poll(now);
        message(
            &db,
            "u2",
            3,
            json!({"role":"user","time":{"created":3000},
          "modelSelection":{"providerId":"proxy","modelId":"model-b","options":{"reasoningLevel":"high"}},
          "model":{"providerID":"proxy","modelID":"model-b","variant":"max"}}),
        );
        let task = provider.poll(now).tasks.remove(0);
        assert_eq!(task.model.as_deref(), Some("model-b"));
        assert_eq!(task.reasoning_effort.as_deref(), Some("high"));
        for n in 4..74 {
            message(
                &db,
                &format!("a{n}"),
                n,
                json!({"role":"assistant","parentID":"u2","providerId":"proxy","modelId":"model-b","time":{"created":3100,"completed":3200},"finish":"tool-calls"}),
            );
        }
        let task = provider.poll(now).tasks.remove(0);
        assert_eq!(task.reasoning_effort.as_deref(), Some("high"));
        message(
            &db,
            "fallback",
            74,
            json!({"role":"assistant","parentID":"u2","providerId":"fallback","modelId":"model-b","time":{"created":3300},"finish":"tool-calls"}),
        );
        assert!(provider.poll(now).tasks[0].reasoning_effort.is_none());
        message(
            &db,
            "u3",
            75,
            json!({"role":"user","time":{"created":4000},"model":{"modelID":"model-c","variant":"low"}}),
        );
        let task = provider.poll(now).tasks.remove(0);
        assert_eq!(task.model.as_deref(), Some("model-c"));
        assert_eq!(task.reasoning_effort.as_deref(), Some("low"));
        message(
            &db,
            "u4",
            76,
            json!({"role":"user","time":{"created":4100},"model":{"modelID":"model-c"}}),
        );
        assert!(provider.poll(now).tasks[0].reasoning_effort.is_none());
    }

    #[test]
    fn desktop_projects_are_independent_of_runtime_read_health() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = dir.path().join("cli/db/db.sqlite");
        let index = dir.path().join("v2/tasks-index.sqlite");
        std::fs::create_dir(index.parent().unwrap()).unwrap();
        let db = Connection::open(index).unwrap();
        db.execute_batch(
            "CREATE TABLE projects(project_id TEXT, host_id TEXT, deleted INTEGER);
             CREATE TABLE project_roots(project_id TEXT, root_path TEXT);
             INSERT INTO projects VALUES('active','local',0),('removed','local',1),('remote','ssh',0);
             INSERT INTO project_roots VALUES('active','relative-path');",
        )
        .unwrap();
        let local = dir.path().join("local-project");
        for (id, path) in [
            ("active", local.clone()),
            ("active", local.clone()),
            ("removed", dir.path().join("removed-project")),
            ("remote", dir.path().join("remote-project")),
        ] {
            db.execute(
                "INSERT INTO project_roots VALUES(?1,?2)",
                rusqlite::params![id, path.to_str().unwrap()],
            )
            .unwrap();
        }
        let tasks = ZcodeProvider::new(runtime.clone()).poll(SystemTime::now());
        assert!(!tasks.authoritative);
        assert!(tasks.tasks.is_empty() && tasks.catalog.is_empty());
        assert_eq!(local_project_roots(&runtime), [local.to_str().unwrap()]);
        assert!(local_project_roots(&dir.path().join("custom.sqlite")).is_empty());
    }
    #[test]
    fn legacy_desktop_index_keeps_only_local_project_paths() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = dir.path().join("cli/db/db.sqlite");
        let index = dir.path().join("v2/tasks-index.sqlite");
        std::fs::create_dir(index.parent().unwrap()).unwrap();
        let db = Connection::open(index).unwrap();
        db.execute_batch(
            "CREATE TABLE tasks(workspace_key TEXT, workspace_path TEXT, workspace_identity TEXT, deleted INTEGER);",
        )
        .unwrap();
        let project = dir.path().join("local-project");
        for (key, identity, deleted, path) in [
            ("local", None, 0, project.clone()),
            (
                "remote:ssh:example:22:user:/tmp",
                None,
                0,
                dir.path().join("remote-a"),
            ),
            (
                "legacy-remote",
                Some("remote:docker:example:/tmp"),
                0,
                dir.path().join("remote-b"),
            ),
            ("deleted", None, 1, dir.path().join("deleted")),
            (
                "default",
                None,
                0,
                dir.path().join(".zcode/workspace/default"),
            ),
        ] {
            db.execute(
                "INSERT INTO tasks VALUES(?1,?2,?3,?4)",
                rusqlite::params![key, path.to_str().unwrap(), identity, deleted],
            )
            .unwrap();
        }
        assert_eq!(local_project_roots(&runtime), [project.to_str().unwrap()]);
    }
    #[test]
    #[ignore = "requires real local ZCode Desktop project metadata; read-only, counts only"]
    fn live_desktop_project_roots() {
        let path = default_db();
        let roots = local_project_roots(&path);
        assert!(
            !roots.is_empty(),
            "Expected at least one real local ZCode project"
        );
        println!("ZCode Desktop: {} local project roots", roots.len());
    }
    #[test]
    fn legacy_runtime_schema_reads_new_turn_without_inventing_old_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let db = fixture(&path);
        db.execute_batch("ALTER TABLE session DROP COLUMN time_deleted; ALTER TABLE session DROP COLUMN last_outcome;").unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(10);
        let mut provider = ZcodeProvider::new(path);
        let first = provider.poll(now);
        assert!(first.authoritative);
        assert!(first.tasks.is_empty());
        assert_eq!(first.catalog[0].id, "zcode:s");
        assert_eq!(first.catalog[0].status, TaskStatus::Unknown);
        assert!(first.catalog[0].stale && first.catalog[0].finished_at.is_none());

        message(
            &db,
            "u2",
            3,
            json!({"role":"user","time":{"created":10_001}}),
        );
        let next = provider.poll(now + Duration::from_millis(2));
        assert!(next.authoritative);
        assert_eq!(next.tasks[0].id, "zcode:s");
        assert_eq!(next.tasks[0].status, TaskStatus::Running);
        assert!(next.tasks[0].new_execution && !next.tasks[0].stale);
    }
    #[test]
    fn completed_cache_entries_expire_without_inventing_running_completion() {
        let mut provider = ZcodeProvider::new(PathBuf::new());
        let now = SystemTime::now();
        let mut terminal = TaskSnapshot::new(
            "zcode:done",
            "Done",
            TaskSource::NativeAgent,
            TaskStatus::Succeeded,
            DetectionConfidence::Inferred,
            now,
        );
        terminal.session_id = Some("done".into());
        let mut running = terminal.clone();
        running.id = "zcode:running".into();
        running.session_id = Some("running".into());
        running.status = TaskStatus::Running;
        provider.resume([terminal, running]);
        let output = provider.observe(vec![], now);
        assert_eq!(output.tasks.len(), 1);
        assert_eq!(output.tasks[0].status, TaskStatus::Running);
        assert!(output.tasks[0].stale);
        assert_eq!(provider.followed.len(), 1);
        assert_eq!(provider.previous.len(), 1);
    }

    #[test]
    fn restart_resumes_only_known_tasks_and_stale_interruption_cannot_end_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = fixture(&path);
        let original = ZcodeProvider::read(&path).unwrap().pop().unwrap();
        let mut provider = ZcodeProvider::new(path.clone());
        provider.resume([original]);
        let resumed = provider.poll(UNIX_EPOCH + Duration::from_millis(900000));
        assert_eq!(resumed.tasks.len(), 1);
        assert_eq!(resumed.tasks[0].status, TaskStatus::Succeeded);
        message(
            &db,
            "u2",
            3,
            json!({"role":"user","time":{"created":910000}}),
        );
        db.execute("UPDATE session SET last_outcome='interrupted'", [])
            .unwrap();
        message(
            &db,
            "a2",
            4,
            json!({"role":"assistant","parentID":"u2","time":{"created":911000,"completed":912000},"finish":"tool-calls"}),
        );
        let running = provider.poll(UNIX_EPOCH + Duration::from_millis(913000));
        assert_eq!(running.tasks[0].status, TaskStatus::Running);
        let mut restarted = ZcodeProvider::new(path);
        restarted.resume(running.tasks);
        assert_eq!(
            restarted
                .poll(UNIX_EPOCH + Duration::from_millis(1200000))
                .tasks[0]
                .status,
            TaskStatus::Running
        );
        db.execute("UPDATE session SET last_outcome='completed'", [])
            .unwrap();
        message(
            &db,
            "end",
            5,
            json!({"role":"assistant","parentID":"u2","time":{"created":1210000,"completed":1220000},"finish":"stop"}),
        );
        assert_eq!(
            restarted
                .poll(UNIX_EPOCH + Duration::from_millis(1230000))
                .tasks[0]
                .status,
            TaskStatus::Succeeded
        );
    }

    #[test]
    fn internal_reminders_long_turns_and_short_new_tasks_keep_real_execution_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = fixture(&path);
        let mut provider = ZcodeProvider::new(path);
        provider.poll(UNIX_EPOCH + Duration::from_millis(2500));
        message(
            &db,
            "u2",
            3,
            json!({"role":"user","time":{"created":3000},"semantics":{"origin":"real_user"}}),
        );
        for n in 4..104 {
            message(
                &db,
                &format!("step-{n}"),
                n,
                json!({"role":"assistant","parentID":"u2","modelId":"model-long","time":{"created":3100,"completed":3200},"finish":"tool-calls"}),
            );
        }
        message(
            &db,
            "internal",
            104,
            json!({"role":"user","synthetic":true,"semantics":{"origin":"agent_runtime"},"time":{"created":3300}}),
        );
        let running = provider.poll(UNIX_EPOCH + Duration::from_millis(3500));
        assert_eq!(running.tasks[0].turn_key.as_deref(), Some("u2"));
        assert!(running.tasks[0].new_execution);
        assert_eq!(running.tasks[0].status, TaskStatus::Running);
        message(
            &db,
            "answer",
            105,
            json!({"role":"assistant","parentID":"u2","time":{"created":3400,"completed":4000},"finish":"stop"}),
        );
        let complete = provider.poll(UNIX_EPOCH + Duration::from_millis(5000));
        assert_eq!(complete.tasks[0].status, TaskStatus::Succeeded);
        assert!(!complete.tasks[0].new_execution);
        db.execute("INSERT INTO session VALUES('short','Short real task','D:/project','completed',NULL,NULL,6500)",[]).unwrap();
        for (id, seq, data) in [
            ("short-u", 1, json!({"role":"user","time":{"created":6000}})),
            (
                "short-a",
                2,
                json!({"role":"assistant","parentID":"short-u","time":{"created":6100,"completed":6500},"finish":"stop"}),
            ),
        ] {
            db.execute(
                "INSERT INTO message VALUES(?1,'short',?2,?3)",
                rusqlite::params![id, seq, data.to_string()],
            )
            .unwrap();
        }
        let short = provider.poll(UNIX_EPOCH + Duration::from_millis(7000));
        let task = short
            .tasks
            .iter()
            .find(|t| t.session_id.as_deref() == Some("short"))
            .unwrap();
        assert!(task.new_execution);
        assert_eq!(task.status, TaskStatus::Succeeded);
        db.execute(
            "UPDATE session SET last_outcome='interrupted' WHERE id='short'",
            [],
        )
        .unwrap();
        db.execute("UPDATE message SET data=?1 WHERE id='short-a'",[json!({"role":"assistant","parentID":"short-u","time":{"created":6100,"completed":6500},"error":{"name":"TransportError","data":{"code":"model_request_cancelled","turnResult":"cancelled"}}}).to_string()]).unwrap();
        let interrupted = provider.poll(UNIX_EPOCH + Duration::from_millis(8000));
        let task = interrupted
            .tasks
            .iter()
            .find(|t| t.session_id.as_deref() == Some("short"))
            .unwrap();
        assert_eq!(task.outcome.as_deref(), Some("interrupted"));
        assert_eq!(task.waiting_reason, Some(WaitingReason::TurnEnded));
    }

    #[test]
    fn native_turn_identity_tool_phases_and_outcomes_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = fixture(&path);
        let mut provider = ZcodeProvider::new(path.clone());
        let now = UNIX_EPOCH + Duration::from_millis(5000);
        let output = provider.poll(now);
        assert!(output.authoritative);
        assert!(output.tasks.is_empty());
        assert_eq!(output.catalog[0].status, TaskStatus::Succeeded);
        message(&db, "u2", 3, json!({"role":"user","time":{"created":3000}}));
        let output = provider.poll(now);
        assert_eq!(output.tasks[0].status, TaskStatus::Running);
        assert!(output.tasks[0].new_execution);
        assert_eq!(output.tasks[0].turn_key.as_deref(), Some("u2"));
        message(
            &db,
            "a2",
            4,
            json!({"role":"assistant","parentID":"u2","modelId":"model-b","time":{"created":3100,"completed":3200},"finish":"tool-calls"}),
        );
        db.execute(
            "INSERT INTO part VALUES('p','a2',1,?1)",
            [json!({"type":"tool","state":{"status":"running"}}).to_string()],
        )
        .unwrap();
        let output = provider.poll(now);
        assert_eq!(output.tasks[0].phase, Some(ExecutionPhase::Tool));
        assert_eq!(output.tasks[0].status, TaskStatus::Running);
        assert_eq!(output.tasks[0].model.as_deref(), Some("model-b"));
        assert!(!output.tasks[0].new_execution);
        db.execute(
            "UPDATE part SET data=?1",
            [json!({"type":"tool","state":{"status":"completed"}}).to_string()],
        )
        .unwrap();
        assert_eq!(provider.poll(now).tasks[0].status, TaskStatus::Running);
        message(
            &db,
            "a3",
            5,
            json!({"role":"assistant","parentID":"u2","time":{"created":3300,"completed":4000},"finish":"stop"}),
        );
        let output = provider.poll(now);
        assert_eq!(output.tasks[0].status, TaskStatus::Succeeded);
        let activity = output.tasks[0].updated_at;
        db.execute("UPDATE session SET title='Renamed',time_updated=99999", [])
            .unwrap();
        let renamed = provider.poll(now);
        assert_eq!(renamed.tasks[0].name, "Renamed");
        assert_eq!(renamed.tasks[0].updated_at, activity);
        assert_eq!(renamed.tasks[0].turn_key.as_deref(), Some("u2"));
        assert!(!renamed.tasks[0].new_execution);
        message(&db, "u3", 6, json!({"role":"user","time":{"created":4500}}));
        provider.poll(now);
        message(
            &db,
            "a4",
            7,
            json!({"role":"assistant","parentID":"u3","time":{"created":4600,"completed":4700},"error":{"name":"Failure"}}),
        );
        assert_eq!(provider.poll(now).tasks[0].status, TaskStatus::Failed);
        drop(db);
        std::fs::remove_file(path).unwrap();
        let missing = provider.poll(now);
        assert!(!missing.authoritative);
        assert_eq!(missing.tasks[0].status, TaskStatus::Failed);
        assert!(missing.tasks[0].stale);
    }
    #[test]
    #[ignore = "read-only native ZCode database; reports only counts and model presence"]
    fn live_zcode_native_metadata() {
        let mut provider = ZcodeProvider::new(default_db());
        let output = provider.poll(SystemTime::now());
        assert!(output.authoritative, "{:?}", output.error);
        assert!(!output.catalog.is_empty());
        let models = output.catalog.iter().filter(|t| t.model.is_some()).count();
        let efforts = output
            .catalog
            .iter()
            .filter(|t| t.reasoning_effort.is_some())
            .count();
        println!(
            "ZCode sessions={} active={} models={models} efforts={efforts}",
            output.catalog.len(),
            output.tasks.len()
        );
        if std::env::var_os("TASKPULSE_EXPECT_NATIVE_EFFORT").is_some() {
            assert!(efforts > 0);
        }
        assert!(models > 0);
    }
}
