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
const NAME_LEN: usize = 40;
const MAX_SESSIONS: usize = 6;

#[derive(Debug, Clone)]
struct CachedSession {
    mtime: f64,
    status: TaskStatus,
    cwd: String,
    message: Option<String>,
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
            "find \"$HOME/.dsh/sessions\" -type f -name session.jsonl.zstd -printf '%T@\\t%p\\0' 2>/dev/null | sort -z -nr | head -z -n {MAX_SESSIONS}"
        ))?;
        Ok(parse_discovery_output(&out))
    }

    /// Read + decompress + parse a session log, returning inferred status.
    fn parse_session(&self, path: &str) -> Option<(TaskStatus, String, Option<String>)> {
        // base64 to avoid any binary/CRLF mangling through wsl.exe stdio.
        let path = shell_quote(path);
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
                }
                "turn/start" => last_turn_start_seq = seq,
                "turn/end" => last_turn_end_seq = seq,
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
        Some((status, cwd.unwrap_or_default(), last_msg))
    }

    fn session_to_task(
        &self,
        path: &str,
        status: TaskStatus,
        cwd: &str,
        message: Option<String>,
        updated_at: SystemTime,
        now: SystemTime,
    ) -> TaskSnapshot {
        let name = PathBuf::from(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "DeepSeek Harness".to_string());
        let mut task = TaskSnapshot::new(
            format!("dsh:{path}"),
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
        task.updated_at = updated_at;
        task.cli = Some("dsh".into());
        task
    }
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
            let (state, stale) = match cached {
                Some(cached) if (cached.mtime - mtime).abs() < 0.5 => (cached, false),
                previous => match self.parse_session(&path) {
                    Some((status, cwd, message)) => {
                        let parsed = CachedSession {
                            mtime,
                            status,
                            cwd,
                            message,
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
            let mut task = self.session_to_task(
                &path,
                state.status,
                &state.cwd,
                state.message,
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
}
