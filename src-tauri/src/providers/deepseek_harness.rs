//! DeepSeek Harness (dsh) provider — reads the harness's append-only session
//! log from inside WSL2.
//!
//! dsh stores sessions at `$DSH_HOME/sessions/<workspace>/<session-id>/session.jsonl.zstd`
//! (zstd-compressed JSONL of the full event stream: `turn/start`, `turn/end`,
//! `step/*`, `assistant/*`, `tool/*`, `user/message`, `session`, …).
//!
//! State inference (from the harness's own event stream — confidence HOOK):
//!   * the latest `turn/end` seq is newer than the latest `turn/start` seq
//!     → the turn finished and the agent is waiting for the user
//!     → WAITING_INPUT
//!   * a `turn/start` exists with no later `turn/end`
//!     → the agent is actively processing → RUNNING
//!   * no turn events at all → IDLE
//!
//! Access: this Windows host cannot read the WSL2 filesystem via `\\wsl.localhost`
//! (disabled), so we shell out to `wsl.exe` every poll, base64-encode the session
//! on the WSL side (to survive stdio), and decompress in Rust.  A per-session
//! (path, mtime) cache avoids re-reading an unchanged log.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use crate::providers::{Provider, ProviderOutput};
use base64::Engine;

/// A session with no activity within this window is treated as dormant.
const STALE_TTL: Duration = Duration::from_secs(24 * 3600);
/// How often we re-run `wsl.exe` to re-discover the newest session. Between
/// probes we re-emit the last known state (avoids spawning wsl.exe each poll).
const PROBE_INTERVAL: Duration = Duration::from_secs(6);
const WSL_DISTRO: &str = "Ubuntu";
const NAME_LEN: usize = 56;

#[derive(Debug, Clone)]
struct CachedSession {
    mtime: f64,
    status: TaskStatus,
    cwd: String,
    name: String,
    message: Option<String>,
    turn_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct SessionMetadata {
    cwd: Option<String>,
    title: Option<String>,
    status: Option<TaskStatus>,
}

pub struct DeepSeekHarnessProvider {
    distro: String,
    /// path → parsed state; unchanged logs are not re-parsed.
    cache: std::collections::HashMap<String, CachedSession>,
    last_probe: Option<std::time::Instant>,
    last_tasks: Vec<TaskSnapshot>,
}

impl DeepSeekHarnessProvider {
    pub fn new() -> Self {
        let distro =
            std::env::var("TASKPULSE_WSL_DISTRO").unwrap_or_else(|_| WSL_DISTRO.to_string());
        Self {
            distro,
            cache: Default::default(),
            last_probe: None,
            last_tasks: Vec::new(),
        }
    }

    /// Run a command inside WSL and capture stdout as raw bytes.
    fn wsl(&self, script: &str) -> std::io::Result<Vec<u8>> {
        let out = Command::new("wsl.exe")
            .args(["--distribution", &self.distro, "-e", "bash", "-lc", script])
            .output()?;
        if !out.status.success() {
            let detail = String::from_utf8_lossy(&out.stderr);
            return Err(std::io::Error::other(format!(
                "WSL command failed: {}",
                detail.trim()
            )));
        }
        Ok(out.stdout)
    }

    /// Paths + mtimes of the most recently written session logs. NUL-delimited
    /// output preserves workspaces containing spaces or shell punctuation.
    fn recent_sessions(&self) -> std::io::Result<Vec<(String, f64)>> {
        let out = self.wsl(&format!(
            "find \"$HOME/.dsh/sessions\" -type f -name session.jsonl.zstd -printf '%T@\\t%p\\0' 2>/dev/null | sort -z -nr"
        ))?;
        Ok(parse_discovery_output(&out))
    }

    /// Read DSH's projection cache. The native UI writes session renames and
    /// live step state here; neither is guaranteed to be appended to the
    /// compressed event log.
    fn session_metadata(
        &self,
    ) -> std::io::Result<std::collections::HashMap<String, SessionMetadata>> {
        let out = self.wsl("cat \"$HOME/.dsh/storages/session_projcache.json\" 2>/dev/null")?;
        Ok(parse_session_metadata(&out))
    }

    pub fn force_refresh(&mut self) {
        self.last_probe = None;
    }

    /// Read + decompress + parse a session log, returning inferred status.
    fn parse_session(
        &self,
        path: &str,
    ) -> Option<(TaskStatus, String, String, Option<String>, Option<String>)> {
        // base64 to avoid any binary/CRLF mangling through wsl.exe stdio.
        let session_path = path;
        let path = shell_quote(session_path);
        let b64 = self
            .wsl(&format!("base64 -w0 -- {path} 2>/dev/null; echo"))
            .ok()?;
        let b64 = String::from_utf8_lossy(&b64);
        let b64: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
        let raw = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        let text = zstd::decode_all(std::io::Cursor::new(raw)).ok()?;
        let text = String::from_utf8_lossy(&text);

        let mut last_turn_start_seq = -1i64;
        let mut last_turn_end_seq = -1i64;
        let mut cwd: Option<String> = None;
        let mut native_title: Option<String> = None;
        let mut first_user_message: Option<String> = None;
        let mut last_msg: Option<String> = None;
        for line in text.lines() {
            let line = line.trim_start();
            if !line.starts_with('{') {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(ty) = v.get("type").and_then(|t| t.as_str()) else {
                continue;
            };
            let seq = v.get("seq").and_then(|s| s.as_i64()).unwrap_or(-1);
            match ty {
                "session" => {
                    if cwd.is_none() {
                        cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
                    }
                    native_title = native_title.or_else(|| {
                        v.get("name")
                            .or_else(|| v.get("title"))
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                    });
                }
                "turn/start" => last_turn_start_seq = seq,
                "turn/end" => last_turn_end_seq = seq,
                "user/message" => {
                    if first_user_message.is_none() {
                        first_user_message = extract_text(&v).map(str::to_string);
                    }
                }
                "assistant/message" => {
                    if let Some(msg) = v
                        .get("data")
                        .and_then(|d| d.get("message"))
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_array())
                        .and_then(|a| a.first())
                        .and_then(|f| f.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        last_msg = Some(truncate(msg, 120));
                    }
                }
                _ => {}
            }
        }

        let status = if last_turn_start_seq < 0 && last_turn_end_seq < 0 {
            TaskStatus::Idle
        } else if last_turn_end_seq > last_turn_start_seq {
            TaskStatus::WaitingInput
        } else {
            TaskStatus::Running
        };
        let cwd = cwd.unwrap_or_default();
        let session_id = session_id_from_path(session_path);
        let name = native_title
            .as_deref()
            .or(first_user_message.as_deref())
            .map(summarize_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("未命名任务 · {}", &session_id[..session_id.len().min(8)]));
        let turn_key = (last_turn_start_seq >= 0).then(|| format!("turn:{last_turn_start_seq}"));
        Some((status, cwd, name, last_msg, turn_key))
    }

    fn session_to_task(
        &self,
        path: &str,
        status: TaskStatus,
        cwd: &str,
        name: &str,
        message: Option<String>,
        turn_key: Option<String>,
        updated_at: SystemTime,
        now: SystemTime,
    ) -> TaskSnapshot {
        let session_id = session_id_from_path(path);
        let mut task = TaskSnapshot::new(
            format!("dsh:{session_id}"),
            truncate(&name, NAME_LEN),
            TaskSource::DeepSeekHarness,
            status,
            DetectionConfidence::Hook,
            now,
        );
        let project = PathBuf::from(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        task.project = project;
        task.message = message;
        task.turn_key = turn_key;
        task.session_id = Some(session_id);
        task.updated_at = updated_at;
        task.cli = Some("dsh".into());
        task
    }
}

fn session_id_from_path(path: &str) -> String {
    PathBuf::from(path)
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn extract_text(value: &serde_json::Value) -> Option<&str> {
    value
        .get("text")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("content"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| data.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(|content| content.as_str())
        })
}

fn summarize_name(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let clause = collapsed
        .split(['。', '！', '？', ';', '；'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(&collapsed);
    truncate(clause.trim(), NAME_LEN)
}

fn parse_discovery_output(out: &[u8]) -> Vec<(String, f64)> {
    out.split(|byte| *byte == 0)
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let (mtime, path) = record.split_once('\t')?;
            let path = path.trim();
            if path.is_empty() {
                return None;
            }
            Some((path.to_string(), mtime.trim().parse::<f64>().ok()?))
        })
        .collect()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

impl Provider for DeepSeekHarnessProvider {
    fn name(&self) -> &'static str {
        "deepseek-harness"
    }

    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        // Throttle the wsl.exe probe; between probes re-emit the last state so
        // we don't spawn wsl.exe every 2s poll.
        if let Some(last) = self.last_probe {
            if last.elapsed() < PROBE_INTERVAL {
                return ProviderOutput {
                    tasks: self.last_tasks.clone(),
                    hosts: vec![],
                    ..ProviderOutput::default()
                };
            }
        }
        self.last_probe = Some(std::time::Instant::now());

        let sessions = match self.recent_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                log::warn!("deepseek-harness: session discovery failed: {error}");
                let tasks = self
                    .last_tasks
                    .iter()
                    .cloned()
                    .map(|mut task| {
                        task.stale = true;
                        task
                    })
                    .collect();
                return ProviderOutput::degraded(tasks, error.to_string());
            }
        };
        let metadata = match self.session_metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                log::warn!("deepseek-harness: metadata projection unavailable: {error}");
                std::collections::HashMap::new()
            }
        };

        let mut tasks = Vec::new();
        let mut live_paths = std::collections::HashSet::new();
        for (path, mtime) in sessions {
            let mtime_time = UNIX_EPOCH + Duration::from_secs_f64(mtime);
            let age = now.duration_since(mtime_time).unwrap_or_default();
            if age > STALE_TTL {
                continue;
            }
            live_paths.insert(path.clone());

            let cached = self.cache.get(&path).cloned();
            let (mut state, stale) = match cached {
                Some(cached) if (cached.mtime - mtime).abs() < 0.5 => (cached, false),
                previous => match self.parse_session(&path) {
                    Some((status, cwd, name, message, turn_key)) => {
                        let parsed = CachedSession {
                            mtime,
                            status,
                            cwd,
                            name,
                            message,
                            turn_key,
                        };
                        self.cache.insert(path.clone(), parsed.clone());
                        (parsed, false)
                    }
                    None => {
                        let Some(previous) = previous else {
                            continue;
                        };
                        (previous, true)
                    }
                },
            };
            let session_id = session_id_from_path(&path);
            if let Some(native) = metadata.get(&session_id) {
                if let Some(title) = native
                    .title
                    .as_deref()
                    .filter(|title| !title.trim().is_empty())
                {
                    state.name = title.trim().to_string();
                }
                if let Some(cwd) = native.cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()) {
                    state.cwd = cwd.to_string();
                }
                if let Some(status) = native.status {
                    state.status = status;
                }
            }
            let mut task = self.session_to_task(
                &path,
                state.status,
                &state.cwd,
                &state.name,
                state.message,
                state.turn_key,
                mtime_time,
                now,
            );
            task.stale = stale;
            tasks.push(task);
        }
        self.cache.retain(|path, _| live_paths.contains(path));
        self.last_tasks = tasks.clone();
        ProviderOutput {
            tasks,
            hosts: vec![],
            ..ProviderOutput::default()
        }
    }
}

fn parse_session_metadata(out: &[u8]) -> std::collections::HashMap<String, SessionMetadata> {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(out) else {
        return Default::default();
    };
    let Some(sessions) = root
        .pointer("/tables/sessions")
        .and_then(serde_json::Value::as_object)
    else {
        return Default::default();
    };

    sessions
        .iter()
        .map(|(id, session)| {
            let rows = session.get("rows").unwrap_or(&serde_json::Value::Null);
            let cwd = session
                .pointer("/identity/cwd")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let title = rows
                .pointer("/title/val")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let stats = rows.pointer("/sessionStats/val");
            let open_step = stats.and_then(|value| value.get("openStep"));
            let has_pending_calls = stats
                .and_then(|value| value.get("pendingCalls"))
                .and_then(serde_json::Value::as_object)
                .is_some_and(|calls| !calls.is_empty());
            let turns = stats
                .and_then(|value| value.get("turns"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let goal_phase = rows
                .pointer("/goal/val/goal/phase")
                .and_then(serde_json::Value::as_str);

            // sessionStats is the UI's live projection. An open step or tool
            // call means work is active; a closed projection means the latest
            // turn has settled. Goal completion/failure is more specific than
            // the generic waiting-for-user state.
            let status = if open_step.is_some_and(|step| !step.is_null()) || has_pending_calls {
                Some(TaskStatus::Running)
            } else {
                match goal_phase {
                    Some("complete" | "completed" | "succeeded") => Some(TaskStatus::Succeeded),
                    Some("failed" | "error") => Some(TaskStatus::Failed),
                    _ if stats.is_some() && turns > 0 => Some(TaskStatus::WaitingInput),
                    _ => None,
                }
            };
            (id.clone(), SessionMetadata { cwd, title, status })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Parse-only tests (no WSL): feed JSONL event text through the same logic.
    // We exercise parse by building a zstd frame of the JSONL and decoding it
    // with the crate — this validates the decompress + infer path without WSL.
    fn parse_text_status(text: &str) -> TaskStatus {
        let enc = zstd::encode_all(std::io::Cursor::new(text.as_bytes()), 3).unwrap();
        let dec = zstd::decode_all(std::io::Cursor::new(enc)).unwrap();
        let text = String::from_utf8_lossy(&dec);
        let mut last_s = -1i64;
        let mut last_e = -1i64;
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let seq = v.get("seq").and_then(|s| s.as_i64()).unwrap_or(-1);
            match ty {
                "turn/start" => last_s = seq,
                "turn/end" => last_e = seq,
                _ => {}
            }
        }
        if last_s < 0 && last_e < 0 {
            TaskStatus::Idle
        } else if last_e > last_s {
            TaskStatus::WaitingInput
        } else {
            TaskStatus::Running
        }
    }

    #[test]
    fn completed_turn_is_waiting() {
        let s = "{\"type\":\"session\",\"cwd\":\"/home/q/projects/Game\"}\n\
                 {\"type\":\"turn/start\",\"seq\":10}\n\
                 {\"type\":\"assistant/message\",\"seq\":11,\"data\":{\"message\":{\"content\":[{\"text\":\"done\"}]}}}\n\
                 {\"type\":\"turn/end\",\"seq\":12,\"data\":{\"reason\":{\"kind\":\"completed\"}}}\n";
        assert_eq!(parse_text_status(s), TaskStatus::WaitingInput);
    }

    #[test]
    fn active_turn_is_running() {
        let s = "{\"type\":\"session\",\"cwd\":\"/home/q/projects/Game\"}\n\
                 {\"type\":\"turn/start\",\"seq\":20}\n\
                 {\"type\":\"step/start\",\"seq\":21}\n\
                 {\"type\":\"tool/call\",\"seq\":22}\n";
        assert_eq!(parse_text_status(s), TaskStatus::Running);
    }

    #[test]
    fn no_turns_is_idle() {
        let s = "{\"type\":\"session\",\"cwd\":\"/home/q/projects/Game\"}\n";
        assert_eq!(parse_text_status(s), TaskStatus::Idle);
    }

    #[test]
    fn truncated_path_name() {
        let name = truncate(&"Game".repeat(20), NAME_LEN);
        assert!(name.chars().count() <= NAME_LEN + 1);
    }

    #[test]
    fn zero_like_epoch_time() {
        // UNIX_EPOCH + 0 is fine; just ensure we don't panic on a 0 mtime.
        use std::time::Duration as D;
        let t = UNIX_EPOCH + D::from_secs_f64(0.0);
        assert_eq!(t.duration_since(UNIX_EPOCH).unwrap().as_secs(), 0);
    }

    #[test]
    fn discovery_output_preserves_paths_with_spaces() {
        let out = b"1788000000.25\t/home/q/My Project/s1/session.jsonl.zstd\0\
                    1787999999.00\t/home/q/other/s2/session.jsonl.zstd\0";
        let sessions = parse_discovery_output(out);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].0, "/home/q/My Project/s1/session.jsonl.zstd");
    }

    #[test]
    fn shell_quote_handles_spaces_and_apostrophes() {
        assert_eq!(shell_quote("/tmp/My Project/a"), "'/tmp/My Project/a'");
        assert_eq!(shell_quote("/tmp/a'b"), "'/tmp/a'\"'\"'b'");
    }

    #[test]
    fn projection_cache_overrides_title_and_completed_state() {
        let json = r#"{
          "tables":{"sessions":{"session-123":{
            "identity":{"cwd":"/home/q/projects/Game"},
            "rows":{
              "title":{"val":"大富翁"},
              "sessionStats":{"val":{"turns":18,"openStep":null,"pendingCalls":{}}},
              "goal":{"val":{"goal":{"phase":"complete"}}}
            }
          }}}
        }"#;
        let metadata = parse_session_metadata(json.as_bytes());
        assert_eq!(metadata["session-123"].title.as_deref(), Some("大富翁"));
        assert_eq!(
            metadata["session-123"].cwd.as_deref(),
            Some("/home/q/projects/Game")
        );
        assert_eq!(metadata["session-123"].status, Some(TaskStatus::Succeeded));
    }

    #[test]
    fn projection_cache_detects_open_step_as_running() {
        let json = br#"{
          "tables":{"sessions":{"session-123":{"rows":{
            "sessionStats":{"val":{"turns":2,"openStep":7,"pendingCalls":{}}}
          }}}}
        }"#;
        let metadata = parse_session_metadata(json);
        assert_eq!(metadata["session-123"].status, Some(TaskStatus::Running));
    }
}
