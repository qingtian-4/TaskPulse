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

/// A session whose last activity is WAITING_INPUT stays visible this long.
const WAITING_TTL: Duration = Duration::from_secs(24 * 3600);
/// A session we believe is RUNNING must have had activity recently.
const RUNNING_TTL: Duration = Duration::from_secs(10 * 60);
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
    cwd: Option<String>,
    ended_at: Option<f64>,
    archived: bool,
    description: Option<String>,
}

fn row_to_snapshot(row: &SessionRow, running: bool, now: SystemTime) -> TaskSnapshot {
    let name = row
        .name
        .clone()
        .or_else(|| row.title.clone())
        .unwrap_or_else(|| "Hermes".to_string());
    let name = truncate(&name, NAME_LEN);
    let status = if running {
        TaskStatus::Running
    } else {
        TaskStatus::WaitingInput
    };
    let at = row.last_activity_at.and_then(epoch_to_time).unwrap_or(now);
    let project = row.cwd.as_deref().and_then(|c| {
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
    task.started_at = Some(at);
    task.updated_at = at;
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

fn read_live_leases(
    conn: &rusqlite::Connection,
    now: SystemTime,
) -> Result<std::collections::HashSet<String>, String> {
    let mut stmt = match conn.prepare("SELECT conversation_id, expires_at FROM session_turn_leases")
    {
        Ok(stmt) => stmt,
        Err(error) if error.to_string().contains("no such table") => return Ok(Default::default()),
        Err(error) => return Err(format!("query turn leases failed: {error}")),
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|error| format!("read turn leases failed: {error}"))?;
    let mut live = std::collections::HashSet::new();
    for row in rows {
        let (conversation, expires) =
            row.map_err(|error| format!("parse turn lease failed: {error}"))?;
        if epoch_to_time(expires).is_some_and(|expires_at| expires_at > now) {
            live.insert(conversation);
        }
    }
    Ok(live)
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
        let live = match read_live_leases(&conn, now) {
            Ok(live) => live,
            Err(error) => {
                log::warn!("hermes: {error}");
                return self.degraded(error);
            }
        };

        // Active sessions, newest activity first.
        let mut stmt = match conn.prepare(
            "SELECT id, display_name, title, last_activity_at, cwd, ended_at, archived, \
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
                cwd: r.get(4)?,
                ended_at: r.get(5)?,
                archived: r.get(6)?,
                description: r.get(7)?,
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
            // Ended conversations are not waiting for user input even when
            // Hermes has not archived them yet.
            if row.archived || row.ended_at.and_then(epoch_to_time).is_some() {
                continue;
            }
            let Some(at) = row.last_activity_at.and_then(epoch_to_time) else {
                continue;
            };
            let running = live.contains(&row.id);
            let age = now.duration_since(at).unwrap_or_default();
            // Lifecycle gate: running sessions must be recent; waiting sessions
            // must have been touched within the TTL.
            if running {
                if age > RUNNING_TTL {
                    continue;
                }
            } else if age > WAITING_TTL {
                continue;
            }
            tasks.push(row_to_snapshot(&row, running, now));
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
             cwd TEXT, ended_at REAL, archived INTEGER, last_activity_description TEXT); \
             CREATE TABLE session_turn_leases (conversation_id TEXT, holder TEXT, acquired_at REAL, expires_at REAL);",
        )
        .unwrap();
        for (id, name, act, cwd, archived, ended) in sessions {
            conn.execute(
                "INSERT INTO sessions (id, display_name, title, last_activity_at, cwd, ended_at, archived, last_activity_description) \
                 VALUES (?1, ?2, ?2, ?3, ?4, ?6, ?5, 'last activity')",
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
    fn ended_session_skipped_even_when_not_archived() {
        let now = now_secs();
        let (_dir, db) = make_db(
            &[("s-ended", "ended", now - 5.0, "/tmp/proj", false, now)],
            &[],
        );
        let mut p = HermesProvider::with_db_path(db);
        let out = p.poll(SystemTime::now());
        assert!(out.tasks.is_empty());
    }

    #[test]
    fn stale_waiting_session_dropped() {
        let now = now_secs();
        // last activity long ago, no lease → beyond WAITING_TTL → dropped
        let (_dir, db) = make_db(&[("s4", "Old", now - 48.0 * 3600.0, "/x", false, 0.0)], &[]);
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
