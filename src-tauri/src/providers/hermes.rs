//! Hermes provider — session / turn metadata tier.
//!
//! Priority per design: lifecycle hooks > session metadata > process scan.
//! Hermes (this agent) stores per-session state in a SQLite DB; we read the
//! `sessions` table for live session metadata and `session_turn_leases` for
//! turn state — a held (unexpired) turn lease means the agent is actively
//! processing (RUNNING); otherwise the agent is waiting for the user
//! (WAITING_INPUT).
//!
//! DB discovery order:
//!   1. `$HERMES_STATE_DB` env override
//!   2. `~/.hermes/state.db`
//!   3. `$LOCALAPPDATA/hermes/state.db`   (this machine)
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
const NAME_LEN: usize = 40;

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
    #[cfg(test)]
    pub fn with_db_path(path: PathBuf) -> Self {
        Self {
            db_path: Some(path),
            last_tasks: Vec::new(),
        }
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
fn discover_db() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HERMES_STATE_DB").map(PathBuf::from) {
        if p.exists() {
            return Some(p);
        }
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

fn row_to_snapshot(row: &SessionRow, lease: Option<TurnLease>, now: SystemTime) -> TaskSnapshot {
    let name = row
        .name
        .clone()
        .or_else(|| row.title.clone())
        .unwrap_or_else(|| {
            let short: String = row.id.chars().take(8).collect();
            format!("未命名任务 · {short}")
        });
    let name = truncate(&name, NAME_LEN);
    let status = if lease.is_some_and(|lease| lease.live) {
        TaskStatus::Running
    } else if row.ended_at.and_then(epoch_to_time).is_some() {
        let failed = row.end_reason.as_deref().is_some_and(|reason| {
            let reason = reason.to_ascii_lowercase();
            reason.contains("fail")
                || reason.contains("error")
                || reason.contains("panic")
                || reason.contains("crash")
        });
        if failed {
            TaskStatus::Failed
        } else {
            TaskStatus::Idle
        }
    } else {
        TaskStatus::WaitingInput
    };
    let at = row.last_activity_at.and_then(epoch_to_time).unwrap_or(now);
    let project = row
        .cwd
        .as_deref()
        .or(row.git_repo_root.as_deref())
        .and_then(|c| {
            Path::new(c)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        });

    let mut task = TaskSnapshot::new(
        format!("hermes:{}", row.id),
        name,
        TaskSource::Hermes,
        status,
        DetectionConfidence::Hook,
        now,
    );
    task.project = project;
    task.cli = Some("hermes".into());
    task.session_id = Some(row.id.clone());
    task.started_at = row.started_at.and_then(epoch_to_time).or(Some(at));
    task.updated_at = at;
    task.finished_at = row.ended_at.and_then(epoch_to_time);
    task.turn_key = lease.map(|lease| format!("lease:{:.6}", lease.acquired_at));
    task.native_unread = row
        .last_read_at
        .map(|read| row.last_activity_at.is_some_and(|activity| activity > read));
    task.pinned = row.pinned;
    if let Some(d) = &row.description {
        task.message = Some(truncate(d, 120));
    }
    task
}

fn epoch_to_time(secs: f64) -> Option<SystemTime> {
    if secs > 0.0 {
        Some(UNIX_EPOCH + Duration::from_secs_f64(secs))
    } else {
        None
    }
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
        let conn = match rusqlite::Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                let error = format!("open state.db failed: {e}");
                log::warn!("hermes: {error}");
                return self.degraded(error);
            }
        };

        // Turn leases → the set of conversation_ids currently in-flight.
        let leases = match read_turn_leases(&conn, now) {
            Ok(leases) => leases,
            Err(error) => {
                log::warn!("hermes: {error}");
                return self.degraded(error);
            }
        };

        // Active sessions, newest activity first.
        let mut stmt = match conn.prepare(
            "SELECT id, display_name, title, last_activity_at, started_at, cwd, git_repo_root, \
             ended_at, end_reason, archived, hidden, pinned, last_read_at, \
             last_activity_description FROM sessions ORDER BY last_activity_at DESC",
        ) {
            Ok(stmt) => stmt,
            Err(error) => {
                let error = format!("query sessions failed: {error}");
                log::warn!("hermes: {error}");
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
            })
        }) {
            Ok(mapped) => match mapped.collect::<rusqlite::Result<Vec<_>>>() {
                Ok(rows) => rows,
                Err(error) => {
                    let error = format!("parse sessions failed: {error}");
                    log::warn!("hermes: {error}");
                    return self.degraded(error);
                }
            },
            Err(error) => {
                let error = format!("read sessions failed: {error}");
                log::warn!("hermes: {error}");
                return self.degraded(error);
            }
        };

        let mut tasks = Vec::new();
        for row in rows {
            if row.archived {
                continue;
            }
            let Some(at) = row.last_activity_at.and_then(epoch_to_time) else {
                continue;
            };
            let lease = leases.get(&row.id).copied();
            let running = lease.is_some_and(|lease| lease.live);
            // A native-hidden session stays hidden until a new turn starts.
            if row.hidden && !running {
                continue;
            }
            let age = now.duration_since(at).unwrap_or_default();
            if !running && age > DISCOVERY_TTL && !row.pinned {
                continue;
            }
            tasks.push(row_to_snapshot(&row, lease, now));
        }

        self.last_tasks = tasks.clone();
        ProviderOutput::tasks(tasks)
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
    fn expired_lease_is_waiting_input() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s2", "Other", now - 30.0, "/home/user/proj", false, 0.0)],
            &[("s2", now - 60.0)], // expired lease → done, waiting for user
        );
        let mut provider = HermesProvider::with_db_path(db);
        let out = provider.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::WaitingInput);
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
        assert_eq!(out.tasks[0].status, TaskStatus::Idle);
        assert!(out.tasks[0].finished_at.is_some());
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
}
