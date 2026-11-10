//! Hermes provider — session / turn metadata tier.
//!
//! Priority per design: lifecycle hooks > session metadata > process scan.
//! Hermes (this agent) stores per-session state in a SQLite DB; we read the
//! `sessions` table for live session metadata and `session_turn_leases` for
//! turn state — a held (unexpired) turn lease means the agent is actively
//! processing (RUNNING). Missing/expired leases prove neither turn completion
//! nor a user-input gate; retain the prior lifecycle with stale evidence.
//!
//! DB discovery order:
//!   1. `$HERMES_STATE_DB` env override
//!   2. `$HERMES_HOME/state.db`
//!   3. `~/.hermes/state.db`
//!   4. `$LOCALAPPDATA/hermes/state.db`
//!
//! Adapter boundary lives here; a Hermes schema change only touches this file.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use crate::providers::{Provider, ProviderOutput};

/// Discovery horizon only. It is not a UI retention rule: unread/pinned tasks
/// are persisted by the attention tracker, while an old session becomes
/// eligible again as soon as Hermes updates its activity or turn lease.
const DISCOVERY_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

pub struct HermesProvider {
    db_path: Option<PathBuf>,
    last_tasks: Vec<TaskSnapshot>,
}

impl HermesProvider {
    pub fn new() -> Self {
        Self {
            db_path: discover_db(),
            last_tasks: Vec::new(),
        }
    }

    /// Constructor for tests: point at a specific DB.
    pub fn with_db_path(path: PathBuf) -> Self {
        Self {
            db_path: Some(path),
            last_tasks: Vec::new(),
        }
    }

    pub fn resume(&mut self, tasks: impl IntoIterator<Item = TaskSnapshot>) {
        self.last_tasks = tasks
            .into_iter()
            .filter(|task| {
                lease_turn_start(task).is_some()
                    && (followed_turn(task) || confirmed_turn_end(task))
            })
            .collect();
    }

    fn snapshot(
        row: &SessionRow,
        lease: Option<TurnLease>,
        now: SystemTime,
        previous: Option<&TaskSnapshot>,
        reply_end: Option<SystemTime>,
    ) -> TaskSnapshot {
        let mut task = row_to_snapshot(row, lease, now);
        if let Some(previous) = previous {
            let known_reply = previous.finished_at.filter(|_| {
                previous.confidence == DetectionConfidence::Exact
                    && confirmed_turn_end(previous)
                    && lease_matches(previous, lease)
            });
            if let Some(ended_at) = reply_end.or(known_reply) {
                // The native transcript proves the reply was finished without
                // closing the chat session. Keep the already-observed lease ID.
                task.status = TaskStatus::WaitingInput;
                task.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
                task.finished_at = Some(ended_at);
                task.updated_at = ended_at.max(previous.updated_at);
                task.confidence = DetectionConfidence::Exact;
                task.turn_key = previous.turn_key.clone();
                task.started_at = previous.started_at;
                task.native_unread = None; // TaskPulse owns the result's viewed state.
                task.stale = false;
            } else if task.finished_at.is_some() && lease.is_none() {
                // A native session close must not change the observed turn ID.
                task.turn_key = previous.turn_key.clone();
                task.started_at = previous.started_at;
            } else if task.status == TaskStatus::Unknown {
                // Lease loss and metadata updates cannot create a lifecycle event.
                task.status = previous.status;
                task.waiting_reason = previous.waiting_reason;
                task.input_request_id = previous.input_request_id.clone();
                task.turn_key = previous.turn_key.clone();
                task.started_at = previous.started_at;
                task.finished_at = previous.finished_at;
                task.phase = previous.phase;
                task.activity = previous.activity.clone();
                task.updated_at = previous.updated_at;
                if confirmed_turn_end(previous) && lease_matches(previous, lease) {
                    task.stale = previous.stale;
                    task.confidence = previous.confidence;
                    task.native_unread = None;
                }
            }
        }
        task
    }

    fn degraded(&self, error: impl Into<String>) -> ProviderOutput {
        let tasks = self
            .last_tasks
            .iter()
            .cloned()
            .map(|mut task| {
                task.stale = true;
                task
            })
            .collect();
        ProviderOutput::degraded(tasks, error)
    }
}

/// Find the Hermes state DB across candidate locations.
pub fn discover_db() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HERMES_STATE_DB").map(PathBuf::from) {
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("HERMES_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(home).join("state.db");
        return path.exists().then_some(path);
    }
    if let Some(home) = dirs_next::home_dir() {
        let p = home.join(".hermes/state.db");
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let p = local.join("hermes/state.db");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// One active session pulled from the DB.
struct SessionRow {
    model: Option<String>,
    id: String,
    name: Option<String>,
    title: Option<String>,
    last_activity_at: Option<f64>,
    started_at: Option<f64>,
    cwd: Option<String>,
    git_repo_root: Option<String>,
    ended_at: Option<f64>,
    end_reason: Option<String>,
    archived: bool,
    hidden: bool,
    pinned: bool,
    last_read_at: Option<f64>,
    description: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct TurnLease {
    acquired_at: f64,
    live: bool,
}

// Read only metadata, and only for turns we already observed running.
const TURN_TAIL_ROWS: usize = 256;
const TURN_END_PROBES_PER_POLL: usize = 32;
const TURN_TAIL_SQL: &str = "SELECT role, timestamp, finish_reason,
    CASE WHEN role='assistant' AND finish_reason='stop' THEN tool_call_id IS NULL AND
        (tool_calls IS NULL OR (length(tool_calls) <= 16 AND trim(tool_calls)='[]'))
        ELSE 0 END,
    display_kind IS NULL, coalesce(active,1)=1 AND coalesce(compacted,0)=0
    FROM messages WHERE session_id=?1 ORDER BY id DESC LIMIT ?2";

fn lease_turn_start(task: &TaskSnapshot) -> Option<SystemTime> {
    let start = task
        .turn_key
        .as_deref()?
        .strip_prefix("lease:")?
        .parse()
        .ok()?;
    epoch_to_time(start)
}

fn lease_matches(task: &TaskSnapshot, lease: Option<TurnLease>) -> bool {
    lease.is_none_or(|lease| {
        !lease.live && task.turn_key.as_deref() == Some(&format!("lease:{:.6}", lease.acquired_at))
    })
}

fn followed_turn(task: &TaskSnapshot) -> bool {
    task.status == TaskStatus::Running || task.requires_action()
}

fn confirmed_turn_end(task: &TaskSnapshot) -> bool {
    task.status == TaskStatus::WaitingInput
        && task.waiting_reason == Some(crate::domain::WaitingReason::TurnEnded)
        && task.finished_at.is_some()
}

fn final_reply_at(
    statement: &mut rusqlite::Statement<'_>,
    session_id: &str,
    start: SystemTime,
    now: SystemTime,
) -> rusqlite::Result<Option<SystemTime>> {
    let mut rows = statement.query(rusqlite::params![session_id, TURN_TAIL_ROWS])?;
    let mut count = 0;
    let end = loop {
        let Some(tail) = rows.next()? else {
            return Ok(None);
        };
        count += 1;
        if !tail.get::<_, bool>(5)? {
            continue;
        }
        let Some(end) = tail.get::<_, Option<f64>>(1)?.and_then(epoch_to_time) else {
            return Ok(None);
        };
        if end < start || end > now || !tail.get::<_, bool>(3)? || !tail.get::<_, bool>(4)? {
            return Ok(None);
        }
        break end;
    };

    // Require exactly one ordinary user message in the observed lease window.
    // A newer turn or a clipped tail cannot supply this turn's completion.
    let mut user_seen = false;
    while let Some(row) = rows.next()? {
        count += 1;
        if !row.get::<_, bool>(5)? {
            continue;
        }
        let Some(at) = row.get::<_, Option<f64>>(1)?.and_then(epoch_to_time) else {
            return Ok(None);
        };
        if at < start {
            return Ok(user_seen.then_some(end));
        }
        if at > end {
            return Ok(None);
        }
        if row.get::<_, String>(0)? == "user" && row.get::<_, bool>(4)? {
            if user_seen {
                return Ok(None);
            }
            user_seen = true;
        }
    }
    Ok((user_seen && count < TURN_TAIL_ROWS).then_some(end))
}

fn row_to_snapshot(row: &SessionRow, lease: Option<TurnLease>, now: SystemTime) -> TaskSnapshot {
    let name = row
        .name
        .clone()
        .or_else(|| row.title.clone())
        .unwrap_or_else(|| {
            let short: String = row.id.chars().take(8).collect();
            format!("未命名任务 · {short}")
        });
    let ended = row.ended_at.and_then(epoch_to_time).filter(|ended| {
        lease
            .and_then(|l| epoch_to_time(l.acquired_at))
            .is_none_or(|start| *ended >= start)
    });
    let waiting_for_user = lease.is_some_and(|l| l.live)
        && row
            .description
            .as_deref()
            .is_some_and(description_waits_for_user)
        && ended.is_none();
    let status = if waiting_for_user {
        // Hermes can keep the turn lease while an approval/input gate is
        // open. The native activity description is more specific than the
        // coarse lease and represents a user-blocked turn.
        TaskStatus::WaitingInput
    } else if ended.is_some() {
        let failed = row.end_reason.as_deref().is_some_and(|reason| {
            let reason = reason.to_ascii_lowercase();
            matches!(
                reason.as_str(),
                "failed" | "failure" | "error" | "panic" | "crash" | "crashed"
            )
        });
        if failed {
            TaskStatus::Failed
        } else {
            TaskStatus::WaitingInput
        }
    } else if lease.is_some_and(|lease| lease.live) {
        TaskStatus::Running
    } else {
        TaskStatus::Unknown
    };
    let at = row.last_activity_at.and_then(epoch_to_time).unwrap_or(now);
    let project_path = row
        .cwd
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .or_else(|| {
            row.git_repo_root
                .as_deref()
                .filter(|path| !path.trim().is_empty())
        });
    let project = project_path.and_then(|c| {
        Path::new(c)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    });

    let mut task = TaskSnapshot::new(
        format!("hermes:{}", row.id),
        name,
        TaskSource::Hermes,
        status,
        if ended.is_some() {
            DetectionConfidence::Hook
        } else {
            DetectionConfidence::Inferred
        },
        now,
    );
    task.stale = status == TaskStatus::Unknown;
    task.project = project;
    task.model = row.model.as_deref().and_then(super::reported_model);
    task.project_path = project_path.map(str::to_owned);
    if task.status == TaskStatus::WaitingInput {
        task.waiting_reason = Some(if waiting_for_user {
            if row
                .description
                .as_deref()
                .is_some_and(|d| d.to_ascii_lowercase().contains("approval"))
            {
                crate::domain::WaitingReason::Approval
            } else {
                crate::domain::WaitingReason::Question
            }
        } else {
            crate::domain::WaitingReason::TurnEnded
        });
        if waiting_for_user {
            task.input_request_id = row.description.as_deref().map(|description| {
                format!(
                    "{}:{}",
                    lease.map(|l| l.acquired_at.to_string()).unwrap_or_default(),
                    description.split(" (").next().unwrap_or(description)
                )
            });
        }
    }
    task.cli = Some("hermes".into());
    task.session_id = Some(row.id.clone());
    task.started_at = lease
        .filter(|l| l.live)
        .and_then(|l| epoch_to_time(l.acquired_at))
        .or_else(|| row.started_at.and_then(epoch_to_time))
        .or(Some(at));
    task.updated_at = at;
    task.finished_at = ended;
    task.turn_key = Some(
        lease
            .map(|lease| format!("lease:{:.6}", lease.acquired_at))
            .unwrap_or_else(|| format!("session:{:.6}", row.started_at.unwrap_or(0.0))),
    );
    task.native_unread = row
        .last_read_at
        .map(|read| row.last_activity_at.is_some_and(|activity| activity > read));
    task.pinned = row.pinned;
    if let Some(d) = &row.description {
        task.message = Some(truncate(d, 120));
        if status == TaskStatus::Running {
            task.activity = Some(d.clone());
        }
    }
    task
}

fn description_waits_for_user(description: &str) -> bool {
    let description = description.trim().to_ascii_lowercase();
    [
        "waiting for user approval",
        "waiting for approval",
        "awaiting user approval",
        "awaiting approval",
        "waiting for user input",
        "awaiting user input",
        "needs user input",
        "requires user input",
        "tool running: clarify",
        "tool running: ask_user",
        "tool running: request_user_input",
    ]
    .iter()
    .any(|marker| description.contains(marker))
}

fn epoch_to_time(secs: f64) -> Option<SystemTime> {
    if secs > 0.0 {
        Duration::try_from_secs_f64(secs)
            .ok()
            .and_then(|d| UNIX_EPOCH.checked_add(d))
    } else {
        None
    }
}

fn session_columns(
    conn: &rusqlite::Connection,
) -> rusqlite::Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('sessions')")?;
    let names = stmt.query_map([], |row| row.get(0))?.collect();
    names
}

/// The older CLI schema tracks session creation/end, not the current turn.
fn legacy_session_catalog(
    conn: &rusqlite::Connection,
    columns: &std::collections::HashSet<String>,
    now: SystemTime,
) -> rusqlite::Result<Option<Vec<TaskSnapshot>>> {
    if columns.contains("last_activity_at")
        || ![
            "id",
            "source",
            "model",
            "title",
            "started_at",
            "message_count",
        ]
        .iter()
        .all(|name| columns.contains(*name))
    {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT id, substr(title, 1, 256), substr(model, 1, 128), started_at
         FROM sessions WHERE typeof(id) = 'text' AND length(id) BETWEEN 1 AND 256
         ORDER BY started_at DESC LIMIT 400",
    )?;
    let catalog = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let model: Option<String> = row.get(2)?;
            let started_at = row.get::<_, Option<f64>>(3)?.and_then(epoch_to_time);
            let mut task = TaskSnapshot::new(
                format!("hermes:{id}"),
                title.unwrap_or_else(|| {
                    format!("未命名任务 · {}", id.chars().take(8).collect::<String>())
                }),
                TaskSource::Hermes,
                TaskStatus::Unknown,
                DetectionConfidence::Inferred,
                started_at.unwrap_or(now),
            );
            task.stale = true;
            task.session_id = Some(id);
            task.started_at = started_at;
            task.model = model.as_deref().and_then(super::reported_model);
            Ok(task)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(catalog))
}

fn read_turn_leases(
    conn: &rusqlite::Connection,
    now: SystemTime,
) -> Result<std::collections::HashMap<String, TurnLease>, String> {
    let mut stmt = match conn
        .prepare("SELECT conversation_id, acquired_at, expires_at FROM session_turn_leases")
    {
        Ok(stmt) => stmt,
        Err(error) if error.to_string().contains("no such table") => return Ok(Default::default()),
        Err(error) => return Err(format!("query turn leases failed: {error}")),
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(|error| format!("read turn leases failed: {error}"))?;
    let mut leases = std::collections::HashMap::new();
    for row in rows {
        let (conversation, acquired, expires) =
            row.map_err(|error| format!("parse turn lease failed: {error}"))?;
        leases.insert(
            conversation,
            TurnLease {
                acquired_at: acquired,
                live: epoch_to_time(expires).is_some_and(|expires_at| expires_at > now),
            },
        );
    }
    Ok(leases)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

impl Provider for HermesProvider {
    fn name(&self) -> &'static str {
        "hermes"
    }

    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let Some(db_path) = self.db_path.clone() else {
            return ProviderOutput::default();
        };
        let conn = match rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(e) => {
                let error = format!("open state.db failed: {e}");
                log::warn!("hermes: database open failed");
                return self.degraded(error);
            }
        };
        if let Err(error) = conn.busy_timeout(Duration::from_millis(200)) {
            return self.degraded(error.to_string());
        }

        // Read the lease, session, and transcript in one SQLite snapshot so
        // a new turn cannot appear between the ownership and reply checks.
        let conn = match conn.unchecked_transaction() {
            Ok(snapshot) => snapshot,
            Err(error) => return self.degraded(error.to_string()),
        };
        let leases = match read_turn_leases(&conn, now) {
            Ok(leases) => leases,
            Err(error) => {
                log::warn!("hermes: runtime lease query failed");
                return self.degraded(error);
            }
        };

        let columns = match session_columns(&conn) {
            Ok(columns) => columns,
            Err(error) => return self.degraded(format!("read Hermes schema failed: {error}")),
        };
        let optional = |column: &'static str, missing| {
            if columns.contains(column) {
                column
            } else {
                missing
            }
        };
        let model_column = optional("model", "NULL");
        let hidden_column = optional("hidden", "0");
        let read_column = optional("last_read_at", "NULL");
        // Active sessions, newest activity first.
        let mut stmt = match conn.prepare(&format!(
            "SELECT id, display_name, title, last_activity_at, started_at, cwd, git_repo_root, \
             ended_at, end_reason, archived, {hidden_column}, pinned, {read_column}, \
             last_activity_description, {model_column} FROM sessions ORDER BY last_activity_at DESC",
        )) {
            Ok(stmt) => stmt,
            Err(error) => {
                if let Ok(Some(catalog)) = legacy_session_catalog(&conn, &columns, now) {
                    return ProviderOutput {
                        catalog,
                        ..self.degraded("Hermes 旧版数据库仅提供会话信息，无法确认当前执行状态")
                    };
                }
                let error = format!("query sessions failed: {error}");
                log::warn!("hermes: session query failed");
                return self.degraded(error);
            }
        };
        let rows: Vec<SessionRow> = match stmt.query_map([], |r| {
            Ok(SessionRow {
                id: r.get(0)?,
                name: r.get(1)?,
                title: r.get(2)?,
                last_activity_at: r.get(3)?,
                started_at: r.get(4)?,
                cwd: r.get(5)?,
                git_repo_root: r.get(6)?,
                ended_at: r.get(7)?,
                end_reason: r.get(8)?,
                archived: r.get(9)?,
                hidden: r.get(10)?,
                pinned: r.get(11)?,
                last_read_at: r.get(12)?,
                description: r.get(13)?,
                model: r.get(14)?,
            })
        }) {
            Ok(mapped) => match mapped.collect::<rusqlite::Result<Vec<_>>>() {
                Ok(rows) => rows,
                Err(error) => {
                    let error = format!("parse sessions failed: {error}");
                    log::warn!("hermes: session parse failed");
                    return self.degraded(error);
                }
            },
            Err(error) => {
                let error = format!("read sessions failed: {error}");
                log::warn!("hermes: session read failed");
                return self.degraded(error);
            }
        };

        let mut transcript = conn.prepare(TURN_TAIL_SQL).ok();
        let mut remaining = TURN_END_PROBES_PER_POLL;
        let mut catalog = Vec::new();
        let mut tasks = Vec::new();
        for row in rows {
            if row.archived {
                continue;
            }
            let lease = leases.get(&row.id).copied();
            let previous = self
                .last_tasks
                .iter()
                .find(|task| task.session_id.as_deref() == Some(row.id.as_str()));
            let reply_end = previous
                .filter(|task| {
                    remaining > 0
                        && followed_turn(task)
                        && row.ended_at.and_then(epoch_to_time).is_none()
                        && lease_matches(task, lease)
                })
                .and_then(lease_turn_start)
                .and_then(|start| {
                    let statement = transcript.as_mut()?;
                    remaining -= 1;
                    final_reply_at(statement, &row.id, start, now).unwrap_or(None)
                });
            let task = Self::snapshot(&row, lease, now, previous, reply_end);
            catalog.push(task.clone());

            let Some(at) = row.last_activity_at.and_then(epoch_to_time) else {
                continue;
            };
            let running = lease.is_some_and(|lease| lease.live);
            // A native-hidden session stays hidden until a new turn starts.
            if row.hidden && !running {
                continue;
            }
            let age = now.duration_since(at).unwrap_or_default();
            let known_running = previous.is_some_and(followed_turn);
            if !running && !known_running && age > DISCOVERY_TTL && !row.pinned {
                continue;
            }
            tasks.push(task);
        }

        self.last_tasks = tasks.clone();
        ProviderOutput {
            catalog,
            ..ProviderOutput::tasks(tasks)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(
        sessions: &[(&str, &str, f64, &str, bool, f64)],
        leases: &[(&str, f64)],
    ) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.db");
        let conn = rusqlite::Connection::open(&p).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT, display_name TEXT, title TEXT, last_activity_at REAL, \
             started_at REAL, cwd TEXT, git_repo_root TEXT, ended_at REAL, end_reason TEXT, \
             archived INTEGER, hidden INTEGER, pinned INTEGER, last_read_at REAL, \
             last_activity_description TEXT); \
             CREATE TABLE session_turn_leases (conversation_id TEXT, holder TEXT, acquired_at REAL, expires_at REAL);",
        )
        .unwrap();
        for (id, name, act, cwd, archived, ended) in sessions {
            conn.execute(
                "INSERT INTO sessions (id, display_name, title, last_activity_at, started_at, cwd, \
                 git_repo_root, ended_at, end_reason, archived, hidden, pinned, last_read_at, last_activity_description) \
                 VALUES (?1, ?2, ?2, ?3, ?3, ?4, NULL, ?6, NULL, ?5, 0, 0, NULL, 'last activity')",
                rusqlite::params![id, name, act, cwd, archived, ended],
            )
            .unwrap();
        }
        for (conv, exp) in leases {
            conn.execute(
                "INSERT INTO session_turn_leases (conversation_id, holder, acquired_at, expires_at) VALUES (?1, 'h', 0.0, ?2)",
                rusqlite::params![conv, exp],
            )
            .unwrap();
        }
        (dir, p)
    }

    fn now_secs() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }

    #[test]
    fn optional_session_model_survives_schema_compatibility_and_switch() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s1", "Session", now, "/project", false, 0.0)],
            &[("s1", now + 300.0)],
        );
        let mut provider = HermesProvider::with_db_path(db.clone());
        let before = provider.poll(SystemTime::now()).tasks.remove(0);
        assert!(before.model.is_none());
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute_batch(
            "ALTER TABLE sessions ADD COLUMN model TEXT; UPDATE sessions SET model='proxy-model';",
        )
        .unwrap();
        let after = provider.poll(SystemTime::now()).tasks.remove(0);
        assert_eq!(after.model.as_deref(), Some("proxy-model"));
        assert_eq!(before.id, after.id);
        assert_eq!(before.status, after.status);
        assert_eq!(before.turn_key, after.turn_key);
        assert_eq!(before.started_at, after.started_at);
        conn.execute("UPDATE sessions SET model='next-model'", [])
            .unwrap();
        assert_eq!(
            provider.poll(SystemTime::now()).tasks[0].model.as_deref(),
            Some("next-model")
        );
    }

    #[test]
    fn active_turn_lease_is_running() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s1", "My Session", now - 5.0, "/home/user/proj", false, 0.0)],
            &[("s1", now + 300.0)], // unexpired lease → running
        );
        let mut provider = HermesProvider::with_db_path(db);
        let out = provider.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);
        assert_eq!(out.tasks[0].project.as_deref(), Some("proj"));
        assert_eq!(out.tasks[0].cli.as_deref(), Some("hermes"));
    }

    #[test]
    fn expired_lease_without_history_is_unknown_not_a_completed_turn() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s2", "Other", now - 30.0, "/home/user/proj", false, 0.0)],
            &[("s2", now - 60.0)], // expired lease is not an end event
        );
        let mut provider = HermesProvider::with_db_path(db);
        let out = provider.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::Unknown);
        assert!(out.tasks[0].stale);
        assert_eq!(out.tasks[0].waiting_reason, None);
    }

    #[test]
    fn live_lease_waiting_for_approval_is_waiting_input() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s-approval", "Approval", now, "/home/user/proj", false, 0.0)],
            &[("s-approval", now + 300.0)],
        );
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE sessions SET last_activity_description = 'waiting for user approval (20s elapsed)' WHERE id = 's-approval'",
            [],
        )
        .unwrap();

        let mut provider = HermesProvider::with_db_path(db);
        let out = provider.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::WaitingInput);
        assert_eq!(
            out.tasks[0].message.as_deref(),
            Some("waiting for user approval (20s elapsed)")
        );
    }

    #[test]
    fn clarify_tool_is_a_user_input_gate() {
        assert!(description_waits_for_user("tool running: clarify"));
        assert!(description_waits_for_user(
            "waiting for user approval (20s elapsed)"
        ));
        assert!(!description_waits_for_user("terminal command running"));
        assert!(!description_waits_for_user("tool running: read_file"));
    }

    #[test]
    fn archived_session_skipped() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s3", "Old", now - 5.0, "/x", true, 0.0)],
            &[("s3", now + 300.0)],
        );
        let mut provider = HermesProvider::with_db_path(db);
        let out = provider.poll(SystemTime::now());
        assert!(out.tasks.is_empty());
    }

    #[test]
    fn ended_session_is_reported_as_finished_metadata() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s-ended", "ended", now - 5.0, "/tmp/proj", false, now)],
            &[],
        );
        let mut p = HermesProvider::with_db_path(db);
        let out = p.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::WaitingInput);
        assert_eq!(
            out.tasks[0].waiting_reason,
            Some(crate::domain::WaitingReason::TurnEnded)
        );
        assert!(out.tasks[0].finished_at.is_some());
    }

    #[test]
    fn lease_expiry_and_missing_row_preserve_running_without_end_notifications() {
        let seconds = now_secs();
        let start = UNIX_EPOCH + Duration::from_secs_f64(seconds);
        let (_dir, db) = make_db(
            &[("s", "original", seconds, "/proj", false, 0.0)],
            &[("s", seconds + 10.0)],
        );
        let mut provider = HermesProvider::with_db_path(db.clone());
        let mut engine = crate::transition::TransitionEngine::new();
        let first = provider.poll(start).tasks;
        assert_eq!(first[0].confidence, DetectionConfidence::Inferred);
        assert!(engine.apply(&first, &[], start).is_empty());
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE sessions SET display_name='renamed'", [])
            .unwrap();
        let expired = provider.poll(start + Duration::from_secs(3600)).tasks;
        assert_eq!(expired[0].status, TaskStatus::Running);
        assert!(expired[0].stale);
        assert_eq!(expired[0].name, "renamed");
        assert_eq!(expired[0].turn_key, first[0].turn_key);
        assert!(engine.apply(&expired, &[], start).is_empty());
        conn.execute("DELETE FROM session_turn_leases", []).unwrap();
        let missing = provider.poll(start + Duration::from_secs(7200)).tasks;
        assert_eq!(missing[0].status, TaskStatus::Running);
        assert_eq!(missing[0].turn_key, first[0].turn_key);
        assert!(engine.apply(&missing, &[], start).is_empty());
        conn.execute(
            "UPDATE sessions SET ended_at=?1, end_reason='completed'",
            [seconds + 7201.0],
        )
        .unwrap();
        let ended = provider.poll(start + Duration::from_secs(7202)).tasks;
        assert!(!ended[0].stale);
        assert_eq!(
            ended[0].waiting_reason,
            Some(crate::domain::WaitingReason::TurnEnded)
        );
        // Removing the lease must not split the same execution identity.
        assert_eq!(engine.apply(&ended, &[], start).len(), 1);
        assert!(engine.apply(&ended, &[], start).is_empty());
    }

    fn turn_fixture(seconds: f64) -> (tempfile::TempDir, PathBuf, rusqlite::Connection) {
        let (dir, db) = make_db(
            &[("s", "Working session", seconds, "/project", false, 0.0)],
            &[("s", seconds + 30.0)],
        );
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT,
             timestamp REAL, finish_reason TEXT, tool_calls TEXT, tool_call_id TEXT,
             active INTEGER DEFAULT 1, compacted INTEGER DEFAULT 0, display_kind TEXT);
             CREATE INDEX messages_by_session ON messages(session_id, id);",
        )
        .unwrap();
        conn.execute("UPDATE session_turn_leases SET acquired_at=?1", [seconds])
            .unwrap();
        conn.execute(
            "INSERT INTO messages(session_id,role,timestamp) VALUES('s','user',?1)",
            [seconds + 0.01],
        )
        .unwrap();
        (dir, db, conn)
    }

    #[test]
    fn explicit_final_reply_completes_a_turn_without_closing_the_session() {
        let seconds = now_secs();
        let start = UNIX_EPOCH + Duration::from_secs_f64(seconds);
        let (_dir, db, conn) = turn_fixture(seconds);
        let mut provider = HermesProvider::with_db_path(db);
        let mut engine = crate::transition::TransitionEngine::new();
        let first = provider.poll(start + Duration::from_secs(1)).tasks;
        assert_eq!(first[0].status, TaskStatus::Running);
        assert!(engine.apply(&first, &[], start).is_empty());
        conn.execute(
            "INSERT INTO messages(session_id,role,timestamp,finish_reason)
             VALUES('s','assistant',?1,'stop')",
            [seconds + 2.0],
        )
        .unwrap();
        // Hermes writes the answer just before leaving the turn loop. Do not
        // end a task while its turn lease is still live.
        let live = provider.poll(start + Duration::from_secs(3)).tasks;
        assert_eq!(live[0].status, TaskStatus::Running);
        assert!(engine.apply(&live, &[], start).is_empty());
        conn.execute("DELETE FROM session_turn_leases", []).unwrap();
        let end = provider.poll(start + Duration::from_secs(4)).tasks;
        assert_eq!(
            end[0].waiting_reason,
            Some(crate::domain::WaitingReason::TurnEnded)
        );
        assert_eq!(end[0].finished_at, Some(start + Duration::from_secs(2)));
        assert_eq!(end[0].turn_key, first[0].turn_key);
        assert!(!end[0].stale);
        assert_eq!(engine.apply(&end, &[], start).len(), 1);
        let again = provider.poll(start + Duration::from_secs(5)).tasks;
        assert_eq!(again[0].finished_at, end[0].finished_at);
        assert!(!again[0].stale);
        assert!(engine.apply(&again, &[], start).is_empty());
    }

    #[test]
    fn hermes_p1_tracked_pending_completes_across_restart() {
        use crate::domain::WaitingReason;
        for (description, reason) in [
            ("Waiting for user approval", WaitingReason::Approval),
            ("Waiting for user input", WaitingReason::Question),
        ] {
            let seconds = now_secs();
            let start = UNIX_EPOCH + Duration::from_secs_f64(seconds);
            let (_dir, db, conn) = turn_fixture(seconds);
            conn.execute(
                "UPDATE sessions SET last_activity_description=?1",
                [description],
            )
            .unwrap();
            let mut provider = HermesProvider::with_db_path(db.clone());
            let first = provider
                .poll(start + Duration::from_secs(1))
                .tasks
                .remove(0);
            assert_eq!(first.waiting_reason, Some(reason));
            assert!(first.input_request_id.is_some());
            conn.execute(
                "INSERT INTO messages(session_id,role,timestamp,finish_reason) VALUES('s','assistant',?1,'stop')",
                [seconds + 2.0],
            ).unwrap();
            assert_eq!(
                provider.poll(start + Duration::from_secs(3)).tasks[0].waiting_reason,
                Some(reason)
            );
            conn.execute("DELETE FROM session_turn_leases", []).unwrap();
            let mut restarted = HermesProvider::with_db_path(db);
            restarted.resume([first.clone()]);
            for provider in [&mut provider, &mut restarted] {
                let done = provider
                    .poll(start + Duration::from_secs(4))
                    .tasks
                    .remove(0);
                assert_eq!(done.waiting_reason, Some(WaitingReason::TurnEnded));
                assert_eq!(done.turn_key, first.turn_key);
                assert!(done.input_request_id.is_none() && !done.stale);
                assert_eq!(done.finished_at, Some(start + Duration::from_secs(2)));
            }
        }
    }

    #[test]
    fn only_the_observed_turn_can_supply_a_final_reply() {
        for (role, finish, offset, calls, extra_user) in [
            ("assistant", "stop", -1.0, None, false),
            ("tool", "stop", 2.0, None, false),
            ("assistant", "stop", 2.0, Some("[{}]"), false),
            ("assistant", "verification_required", 2.0, None, false),
            ("assistant", "stop", 2.0, None, true),
        ] {
            let seconds = now_secs();
            let start = UNIX_EPOCH + Duration::from_secs_f64(seconds);
            let (_dir, db, conn) = turn_fixture(seconds);
            let mut provider = HermesProvider::with_db_path(db);
            let first = provider.poll(start + Duration::from_secs(1)).tasks;
            if extra_user {
                conn.execute(
                    "INSERT INTO messages(session_id,role,timestamp) VALUES('s','user',?1)",
                    [seconds + 1.0],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO messages(session_id,role,timestamp,finish_reason,tool_calls)
                 VALUES('s',?1,?2,?3,?4)",
                rusqlite::params![role, seconds + offset, finish, calls],
            )
            .unwrap();
            conn.execute("DELETE FROM session_turn_leases", []).unwrap();
            let latest = provider.poll(start + Duration::from_secs(4)).tasks;
            assert_eq!(latest[0].turn_key, first[0].turn_key);
            assert_eq!(latest[0].status, TaskStatus::Running);
            assert!(latest[0].stale && latest[0].finished_at.is_none());
        }
    }

    #[test]
    fn restored_followed_turn_can_finish_but_idle_history_is_not_imported() {
        let seconds = now_secs();
        let start = UNIX_EPOCH + Duration::from_secs_f64(seconds);
        let (_dir, db, conn) = turn_fixture(seconds);
        let mut provider = HermesProvider::with_db_path(db.clone());
        let first = provider.poll(start + Duration::from_secs(1)).tasks;
        conn.execute(
            "INSERT INTO messages(session_id,role,timestamp,finish_reason)
             VALUES('s','assistant',?1,'stop')",
            [seconds + 2.0],
        )
        .unwrap();
        conn.execute("DELETE FROM session_turn_leases", []).unwrap();

        let mut cold = HermesProvider::with_db_path(db.clone());
        let idle = cold.poll(start + Duration::from_secs(3));
        assert!(idle.tasks[0].finished_at.is_none());

        let mut resumed = HermesProvider::with_db_path(db);
        resumed.resume(first.clone());
        let end = resumed.poll(start + Duration::from_secs(3)).tasks;
        assert_eq!(
            end[0].waiting_reason,
            Some(crate::domain::WaitingReason::TurnEnded)
        );
        assert_eq!(end[0].turn_key, first[0].turn_key);
        assert!(!end[0].stale);
    }

    #[test]
    fn stale_waiting_session_dropped() {
        let now = now_secs();
        // last activity long ago, no lease → beyond discovery horizon → dropped
        let (_dir, db) = make_db(
            &[("s4", "Old", now - 8.0 * 24.0 * 3600.0, "/x", false, 0.0)],
            &[],
        );
        let mut provider = HermesProvider::with_db_path(db);
        let out = provider.poll(SystemTime::now());
        assert!(out.tasks.is_empty());
    }

    #[test]
    fn missing_db_is_empty() {
        let mut provider = HermesProvider::with_db_path(PathBuf::from("/nonexistent/state.db"));
        let out = provider.poll(SystemTime::now());
        assert!(out.tasks.is_empty());
        assert!(!out.authoritative);
    }

    #[test]
    fn desktop_schema_without_hidden_or_read_time_still_reads_sessions() {
        let (_dir, path) = make_db(
            &[("old-desktop", "Example", 5000.0, "/example", false, 0.0)],
            &[],
        );
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("ALTER TABLE sessions DROP COLUMN hidden; ALTER TABLE sessions DROP COLUMN last_read_at; DROP TABLE session_turn_leases;").unwrap();
        let mut provider = HermesProvider::with_db_path(path);
        let result = provider.poll(UNIX_EPOCH + Duration::from_secs(5001));
        assert!(result.authoritative);
        assert_eq!(result.catalog.len(), 1);
        let task = &result.catalog[0];
        assert_eq!(task.id, "hermes:old-desktop");
        assert_eq!(task.status, TaskStatus::Unknown);
        assert_eq!(task.native_unread, None);
        assert!(task.stale && task.finished_at.is_none());
    }
    #[test]
    fn legacy_cli_sessions_are_discovered_without_runtime_or_completion_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE sessions(id TEXT, source TEXT, model TEXT, title TEXT,
             started_at REAL, ended_at REAL, end_reason TEXT, message_count INTEGER);
             INSERT INTO sessions VALUES('cli','cli','model-a','Old session',5000,6000,'error',2);",
        )
        .unwrap();
        let mut provider = HermesProvider::with_db_path(path);
        let result = provider.poll(UNIX_EPOCH + Duration::from_secs(6001));
        assert!(!result.authoritative);
        assert!(result.tasks.is_empty());
        assert_eq!(result.catalog.len(), 1);
        let session = &result.catalog[0];
        assert_eq!(session.id, "hermes:cli");
        assert_eq!(session.status, TaskStatus::Unknown);
        assert!(session.stale && !session.new_execution && session.finished_at.is_none());
    }
    #[test]
    fn schema_error_is_degraded_not_authoritative_empty() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        rusqlite::Connection::open(&db).unwrap();
        let mut provider = HermesProvider::with_db_path(db);
        let out = provider.poll(SystemTime::now());
        assert!(out.tasks.is_empty());
        assert!(!out.authoritative);
        assert!(out
            .error
            .as_deref()
            .is_some_and(|error| error.contains("sessions")));
    }

    #[test]
    #[ignore = "requires a real local Hermes state database; read-only, counts only"]
    fn live_hermes_snapshot_against_real_db() {
        let mut provider = HermesProvider::new();
        let out = provider.poll(SystemTime::now());
        assert!(out.authoritative);
        assert!(!out.catalog.is_empty());
        assert!(out.catalog.iter().all(|task| !task.id.is_empty()));
        println!(
            "Hermes native: {} discovered sessions, {} without project paths, {} active leases",
            out.catalog.len(),
            out.catalog
                .iter()
                .filter(|task| task.project_path.is_none())
                .count(),
            out.tasks
                .iter()
                .filter(|task| !task.stale && task.status == TaskStatus::Running)
                .count(),
        );
    }
}
