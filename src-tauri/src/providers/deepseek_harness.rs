//! DeepSeek Harness (dsh) provider — reads the harness's append-only session
//! logs from native desktop/CLI storage or the legacy WSL2 installation.
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
//! Native Desktop reads `$DSH_HOME` (normally `~/.dsh`) directly, including v4
//! logs and per-session projection shards. WSL is a separate worker using
//! `wsl.exe` and base64 transport. Both share parsing and lifecycle rules;
//! slow WSL probes cannot delay the desktop feed.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use crate::providers::{Provider, ProviderOutput};
use base64::Engine;

/// A session with no activity within this window is treated as dormant.
const WSL_TIMEOUT: Duration = Duration::from_secs(15);
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
    activity: Option<String>,
    last_seq: i64,
    started_at: Option<SystemTime>,
    updated_at: Option<SystemTime>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct SessionMetadata {
    cwd: Option<String>,
    title: Option<String>,
    status: Option<TaskStatus>,
    seq: Option<i64>,
    turn_start_seq: Option<i64>,
}

pub struct DeepSeekHarnessProvider {
    distro: String,
    native_root: Option<PathBuf>,
    /// path → parsed state; unchanged logs are not re-parsed.
    cache: std::collections::HashMap<String, CachedSession>,
    last_metadata: std::collections::HashMap<String, SessionMetadata>,
    last_tasks: Vec<TaskSnapshot>,
}

impl DeepSeekHarnessProvider {
    pub fn new() -> Self {
        let distro =
            std::env::var("TASKPULSE_WSL_DISTRO").unwrap_or_else(|_| WSL_DISTRO.to_string());
        Self {
            distro,
            native_root: None,
            cache: Default::default(),
            last_metadata: Default::default(),
            last_tasks: Vec::new(),
        }
    }

    pub fn desktop_root() -> PathBuf {
        std::env::var_os("TASKPULSE_DSH_HOME")
            .or_else(|| std::env::var_os("DSH_HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs_next::home_dir().unwrap_or_default().join(".dsh"))
    }

    pub fn desktop() -> Self {
        Self::with_native_root(Self::desktop_root())
    }

    pub fn with_native_root(root: PathBuf) -> Self {
        Self {
            native_root: Some(root),
            ..Self::new()
        }
    }

    pub fn with_distro(distro: String) -> Self {
        Self {
            distro,
            ..Self::new()
        }
    }

    /// Run a command inside WSL and capture stdout as raw bytes.
    fn wsl(&self, script: &str) -> std::io::Result<Vec<u8>> {
        let mut child = Command::new("wsl.exe")
            .args(["--distribution", &self.distro, "-e", "bash", "-lc", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Drain both pipes while the child runs, including large compressed logs.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = std::sync::mpsc::channel();
        for (is_stdout, stream) in [
            (
                true,
                stdout.map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
            ),
            (
                false,
                stderr.map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
            ),
        ] {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(mut stream) = stream {
                    let _ = stream.read_to_end(&mut bytes);
                }
                let _ = tx.send((is_stdout, bytes));
            });
        }
        let deadline = Instant::now() + WSL_TIMEOUT;
        let status = loop {
            match child.try_wait()? {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "WSL probe exceeded 15 seconds",
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        for _ in 0..2 {
            let (is_stdout, bytes) = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::TimedOut, "WSL output pipe timed out")
                })?;
            if is_stdout {
                out = bytes;
            } else {
                err = bytes;
            }
        }
        if !status.success() {
            let detail = String::from_utf8_lossy(&err);
            return Err(std::io::Error::other(format!(
                "WSL command failed: {}",
                detail.trim()
            )));
        }
        Ok(out)
    }

    /// Paths + mtimes of the most recently written session logs. NUL-delimited
    /// output preserves workspaces containing spaces or shell punctuation.
    fn recent_sessions(&self) -> std::io::Result<Vec<(String, f64)>> {
        if let Some(root) = &self.native_root {
            let mut files = std::collections::HashMap::new();
            collect_native_sessions(&root.join("sessions"), &mut files)?;
            let mut sessions = Vec::new();
            for path in files.into_values() {
                let modified = path
                    .metadata()?
                    .modified()?
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let id = session_id_from_path(&path.to_string_lossy());
                let projection_modified = root
                    .join("storages/session_projcache/sessions")
                    .join(format!("{id}.json"))
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|modified| modified.as_secs_f64())
                    .unwrap_or(0.0);
                sessions.push((
                    path.to_string_lossy().into_owned(),
                    modified.max(projection_modified),
                ));
            }
            sessions.sort_by(|a, b| b.1.total_cmp(&a.1));
            return Ok(sessions);
        }
        let out = self.wsl(
            "find \"${DSH_HOME:-$HOME/.dsh}/sessions\" -type f \\( -name session.jsonl.zstd -o -name session.v4.jsonl.zstd \\) -printf '%T@\\t%p\\0' 2>/dev/null | sort -z -nr",
        )?;
        let mut sessions = std::collections::HashMap::new();
        for (path, mtime) in parse_discovery_output(&out) {
            let id = session_id_from_path(&path);
            if !sessions.contains_key(&id) || path.ends_with("session.v4.jsonl.zstd") {
                sessions.insert(id, (path, mtime));
            }
        }
        Ok(sessions.into_values().collect())
    }

    /// Read DSH's projection cache. The native UI writes session renames and
    /// live step state here; neither is guaranteed to be appended to the
    /// compressed event log.
    fn session_metadata(
        &self,
    ) -> std::io::Result<std::collections::HashMap<String, SessionMetadata>> {
        if let Some(root) = &self.native_root {
            let shards = root.join("storages/session_projcache/sessions");
            if shards.is_dir() {
                let mut metadata = std::collections::HashMap::new();
                for entry in std::fs::read_dir(shards)? {
                    let path = entry?.path();
                    if path.extension().and_then(|value| value.to_str()) != Some("json") {
                        continue;
                    }
                    let bytes = std::fs::read(&path)?;
                    let root: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
                    let record = root.get("record").ok_or_else(|| {
                        std::io::Error::other("DSH projection shard has no record")
                    })?;
                    let id = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    metadata.insert(id, parse_metadata_record(record));
                }
                return Ok(metadata);
            }
            let legacy = root.join("storages/session_projcache.json");
            if !legacy.exists() && !root.join("sessions").exists() {
                return Ok(Default::default());
            }
            return parse_session_metadata(&std::fs::read(legacy)?);
        }
        let out = self.wsl("root=\"${DSH_HOME:-$HOME/.dsh}\"; if [ -d \"$root/storages/session_projcache/sessions\" ]; then find \"$root/storages/session_projcache/sessions\" -type f -name '*.json' -print0 | while IFS= read -r -d '' f; do printf '%s\\0' \"$f\"; cat -- \"$f\"; printf '\\0'; done; else cat \"$root/storages/session_projcache.json\"; fi")?;
        if out.first() == Some(&b'{') {
            return parse_session_metadata(&out);
        }
        let mut metadata = std::collections::HashMap::new();
        let records: Vec<_> = out.split(|byte| *byte == 0).collect();
        for pair in records.as_chunks::<2>().0 {
            let path = String::from_utf8_lossy(pair[0]);
            let id = PathBuf::from(path.as_ref())
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let root: serde_json::Value =
                serde_json::from_slice(pair[1]).map_err(std::io::Error::other)?;
            let record = root
                .get("record")
                .ok_or_else(|| std::io::Error::other("DSH projection shard has no record"))?;
            metadata.insert(id, parse_metadata_record(record));
        }
        Ok(metadata)
    }

    /// Read + decompress + parse a session log, returning inferred status.
    fn parse_session(&self, path: &str) -> Option<CachedSession> {
        // base64 to avoid any binary/CRLF mangling through wsl.exe stdio.
        let session_path = path;
        let raw = if self.native_root.is_some() {
            std::fs::read(session_path).ok()?
        } else {
            let quoted = shell_quote(session_path);
            let b64 = self
                .wsl(&format!("base64 -w0 -- {quoted} 2>/dev/null; echo"))
                .ok()?;
            let b64: String = String::from_utf8_lossy(&b64)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            base64::engine::general_purpose::STANDARD.decode(b64).ok()?
        };
        let text = zstd::decode_all(std::io::Cursor::new(raw)).ok()?;
        let text = String::from_utf8_lossy(&text);

        parse_session_text(&text, session_path)
    }

    #[allow(clippy::too_many_arguments)]
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
        let prefix = if self.native_root.is_some() {
            "dsh-native"
        } else {
            "dsh"
        };
        let mut task = TaskSnapshot::new(
            format!("{prefix}:{session_id}"),
            truncate(name, NAME_LEN),
            TaskSource::DeepSeekHarness,
            status,
            DetectionConfidence::Hook,
            now,
        );
        task.project = PathBuf::from(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        task.message = message;
        task.turn_key = turn_key;
        task.session_id = Some(session_id);
        task.updated_at = updated_at;
        task.cli = Some("dsh".into());
        task
    }
}

fn parse_session_text(text: &str, session_path: &str) -> Option<CachedSession> {
    let mut last_turn_start_seq = -1i64;
    let mut last_turn_end_seq = -1i64;
    let mut cwd: Option<String> = None;
    let mut native_title: Option<String> = None;
    let mut first_user_message: Option<String> = None;
    let mut last_msg: Option<String> = None;
    let mut activity = None;
    let mut last_seq = -1;
    let mut started_at = None;
    let mut updated_at = None;
    let mut failed = false;
    let mut parsed_event = false;
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
        parsed_event = true;
        let seq = v.get("seq").and_then(|s| s.as_i64()).unwrap_or(-1);
        last_seq = last_seq.max(seq);
        let time = v
            .get("time")
            .and_then(|value| value.as_u64())
            .and_then(|ms| UNIX_EPOCH.checked_add(Duration::from_millis(ms)));
        updated_at = time.or(updated_at);
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
            "turn/start" => {
                last_turn_start_seq = seq;
                started_at = time;
                failed = false;
                activity = Some("正在分析".into());
            }
            "turn/end" => {
                last_turn_end_seq = seq;
                failed = v
                    .pointer("/data/reason/kind")
                    .and_then(|value| value.as_str())
                    .is_some_and(|kind| matches!(kind, "failed" | "error"));
                activity = None;
            }
            "tool/call" => {
                let tool = v
                    .pointer("/data/name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("工具");
                let arguments = v.pointer("/data/arguments");
                let command = arguments
                    .and_then(|value| value.get("command").or_else(|| value.get("cmd")))
                    .and_then(|value| value.as_str());
                activity = Some(truncate(
                    &command
                        .map(|command| format!("{tool} · {command}"))
                        .unwrap_or_else(|| format!("正在调用 {tool}")),
                    160,
                ));
            }
            "tool/result" if last_turn_start_seq > last_turn_end_seq => {
                activity = Some("正在分析".into())
            }
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

    if !parsed_event {
        return None;
    }
    let status = if failed && last_turn_end_seq > last_turn_start_seq {
        TaskStatus::Failed
    } else if last_turn_start_seq < 0 && last_turn_end_seq < 0 {
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
        .unwrap_or_else(|| {
            let short: String = session_id
                .strip_prefix("session-")
                .unwrap_or(&session_id)
                .chars()
                .take(8)
                .collect();
            format!("未命名任务 · {short}")
        });
    let turn_key = (last_turn_start_seq >= 0).then(|| format!("turn:{last_turn_start_seq}"));
    Some(CachedSession {
        mtime: 0.0,
        status,
        cwd,
        name,
        message: last_msg,
        turn_key,
        activity,
        last_seq,
        started_at,
        updated_at,
    })
}

fn collect_native_sessions(
    root: &std::path::Path,
    files: &mut std::collections::HashMap<PathBuf, PathBuf>,
) -> std::io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_native_sessions(&path, files)?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "session.jsonl.zstd" | "session.v4.jsonl.zstd"))
        {
            let parent = path.parent().unwrap_or(root).to_path_buf();
            if !files.contains_key(&parent)
                || path
                    .file_name()
                    .is_some_and(|name| name == "session.v4.jsonl.zstd")
            {
                files.insert(parent, path);
            }
        }
    }
    Ok(())
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
        .or_else(|| {
            value
                .pointer("/data/content/0/text")
                .and_then(serde_json::Value::as_str)
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
            let modified = mtime.trim().parse::<f64>().ok()?;
            (modified.is_finite() && modified >= 0.0).then(|| (path.to_string(), modified))
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
        // Cadence is owned by the independent worker; every poll is a real probe.
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
        let mut metadata_error = None;
        let metadata = match self.session_metadata() {
            Ok(metadata) => {
                self.last_metadata = metadata.clone();
                metadata
            }
            Err(error) => {
                log::warn!("deepseek-harness: metadata projection unavailable: {error}");
                metadata_error = Some(error.to_string());
                self.last_metadata.clone()
            }
        };

        let mut tasks = Vec::new();
        let mut live_paths = std::collections::HashSet::new();
        for (path, mtime) in sessions {
            let mtime_time = UNIX_EPOCH + Duration::from_secs_f64(mtime);
            live_paths.insert(path.clone());

            let cached = self.cache.get(&path).cloned();
            let (mut state, stale) = match cached {
                Some(cached) if cached.mtime == mtime => (cached, false),
                previous => match self.parse_session(&path) {
                    Some(mut parsed) => {
                        parsed.mtime = mtime;
                        self.cache.insert(path.clone(), parsed.clone());
                        (parsed, false)
                    }
                    None => {
                        metadata_error
                            .get_or_insert_with(|| "会话日志暂不可读或压缩帧尚未写完".to_string());
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
                let current = native.seq.is_none_or(|seq| seq >= state.last_seq);
                if current {
                    if let Some(status) = native.status {
                        // A generic closed projection must not erase an
                        // explicit failed turn, nor an unversioned stale open
                        // step resurrect a turn whose end is in the log.
                        if !(state.status == TaskStatus::Failed
                            && status == TaskStatus::WaitingInput)
                            && !(native.seq.is_none()
                                && state.status != TaskStatus::Running
                                && status == TaskStatus::Running)
                        {
                            state.status = status;
                        }
                    }
                    if let Some(seq) = native.turn_start_seq {
                        state.turn_key = Some(format!("turn:{seq}"));
                    }
                }
            }
            let mut task = self.session_to_task(
                &path,
                state.status,
                &state.cwd,
                &state.name,
                state.message,
                state.turn_key,
                state.updated_at.unwrap_or(mtime_time),
                now,
            );
            task.started_at = state.started_at;
            task.project_path = (!state.cwd.is_empty()).then_some(state.cwd.clone());
            if task.status == TaskStatus::WaitingInput {
                task.waiting_reason = Some(crate::domain::WaitingReason::TurnEnded);
            }
            task.activity = if state.status == TaskStatus::Running {
                state.activity.or_else(|| Some("正在分析".into()))
            } else {
                None
            };
            task.stale = stale || metadata_error.is_some();
            tasks.push(task);
        }
        self.cache.retain(|path, _| live_paths.contains(path));
        self.last_tasks = tasks.clone();
        if let Some(error) = metadata_error {
            return ProviderOutput::degraded(tasks, format!("会话数据暂不可用: {error}"));
        }
        ProviderOutput {
            tasks,
            hosts: vec![],
            ..ProviderOutput::default()
        }
    }
}

fn parse_session_metadata(
    out: &[u8],
) -> std::io::Result<std::collections::HashMap<String, SessionMetadata>> {
    let root = serde_json::from_slice::<serde_json::Value>(out).map_err(std::io::Error::other)?;
    let Some(sessions) = root
        .pointer("/tables/sessions")
        .and_then(serde_json::Value::as_object)
    else {
        return Err(std::io::Error::other(
            "DSH projection has no sessions table",
        ));
    };

    Ok(sessions
        .iter()
        .map(|(id, session)| (id.clone(), parse_metadata_record(session)))
        .collect())
}

fn parse_metadata_record(session: &serde_json::Value) -> SessionMetadata {
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
    let boundary = rows.pointer("/turnBoundary/val");
    let turn_start_seq = boundary
        .and_then(|value| value.get("openTurnStartSeq"))
        .and_then(serde_json::Value::as_i64);
    let seq = rows
        .pointer("/turnBoundary/seq")
        .or_else(|| rows.pointer("/sessionStats/seq"))
        .and_then(serde_json::Value::as_i64);

    // sessionStats is the UI's live projection. An open step or tool
    // call means work is active; a closed projection means the latest
    // turn has settled. Goal completion/failure is more specific than
    // the generic waiting-for-user state.
    let status = if turn_start_seq.is_some()
        || open_step.is_some_and(|step| !step.is_null())
        || has_pending_calls
    {
        Some(TaskStatus::Running)
    } else {
        match goal_phase {
            Some("complete" | "completed" | "succeeded") => Some(TaskStatus::Succeeded),
            Some("failed" | "error") => Some(TaskStatus::Failed),
            _ if stats.is_some() && turns > 0 => Some(TaskStatus::WaitingInput),
            _ => None,
        }
    };
    SessionMetadata {
        cwd,
        title,
        status,
        seq,
        turn_start_seq,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop_fixture(root: &std::path::Path, ended: bool, title: &str, projection_seq: i64) {
        let dir = root.join("sessions/workspace/session-fixture");
        let cache = root.join("storages/session_projcache/sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut body = format!(
            "{}\n{}\n{}\n",
            serde_json::json!({"type":"session","id":"session-fixture","cwd":"/projects/Game","version":4}),
            serde_json::json!({"type":"turn/start","seq":10,"time":time,"data":{"turn":1}}),
            serde_json::json!({"type":"tool/call","seq":11,"time":time,"data":{"name":"Bash","arguments":{"command":"cargo test"}}})
        );
        if ended {
            body.push_str(&format!("{}\n", serde_json::json!({"type":"turn/end","seq":12,"time":time,"data":{"reason":{"kind":"completed"}}})));
        }
        let encoded = zstd::encode_all(std::io::Cursor::new(body.as_bytes()), 3).unwrap();
        std::fs::write(dir.join("session.v4.jsonl.zstd"), encoded).unwrap();
        let projection = serde_json::json!({"version":7,"record":{"identity":{"cwd":"/projects/Game"},"rows":{
            "title":{"seq":projection_seq,"val":title},
            "sessionStats":{"seq":projection_seq,"val":{"turns":1,"openStep":if ended && projection_seq >=12 { serde_json::Value::Null } else { serde_json::json!(1) },"pendingCalls":{}}},
            "turnBoundary":{"seq":projection_seq,"val":{"openTurnStartSeq":if ended && projection_seq>=12 { serde_json::Value::Null } else { serde_json::json!(10) }}}
        }}});
        std::fs::write(
            cache.join("session-fixture.json"),
            serde_json::to_vec(&projection).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn desktop_v4_start_commands_and_completed_turn() {
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), false, "Native title", 11);
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let first = provider.poll(SystemTime::now());
        assert!(first.authoritative);
        assert_eq!(first.tasks.len(), 1);
        let task = &first.tasks[0];
        assert_eq!(task.name, "Native title");
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.activity.as_deref(), Some("Bash · cargo test"));
        assert_eq!(task.session_id.as_deref(), Some("session-fixture"));
        assert!(task.started_at.is_some());
        desktop_fixture(root.path(), true, "Native title", 12);
        let completed = provider.poll(SystemTime::now());
        assert_eq!(completed.tasks[0].id, task.id);
        assert_eq!(completed.tasks[0].status, TaskStatus::WaitingInput);
        assert!(completed.tasks[0].activity.is_none());
    }

    #[test]
    fn desktop_rename_without_log_change_keeps_identity_and_turn() {
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), false, "Old title", 11);
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let first = provider.poll(SystemTime::now()).tasks.remove(0);
        let path = root
            .path()
            .join("storages/session_projcache/sessions/session-fixture.json");
        let mut projection: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        projection["record"]["rows"]["title"]["val"] = serde_json::json!("用户重命名的新任务");
        std::fs::write(path, serde_json::to_vec(&projection).unwrap()).unwrap();
        let renamed = provider.poll(SystemTime::now()).tasks.remove(0);
        assert_eq!(renamed.name, "用户重命名的新任务");
        assert_eq!(renamed.id, first.id);
        assert_eq!(renamed.turn_key, first.turn_key);
        assert_eq!(renamed.status, first.status);
    }

    #[test]
    fn stale_desktop_projection_cannot_resurrect_finished_turn() {
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), true, "Native title", 11);
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let task = provider.poll(SystemTime::now()).tasks.remove(0);
        assert_eq!(task.status, TaskStatus::WaitingInput);
    }

    #[test]
    fn damaged_projection_is_degraded_not_healthy_empty_data() {
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), false, "Native title", 11);
        std::fs::write(
            root.path()
                .join("storages/session_projcache/sessions/session-fixture.json"),
            b"{incomplete",
        )
        .unwrap();
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let output = provider.poll(SystemTime::now());
        assert!(!output.authoritative);
        assert!(output.error.is_some());
        assert!(output.tasks[0].stale);
    }

    #[test]
    fn desktop_prefers_v4_when_migrated_legacy_log_remains() {
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), true, "Native title", 12);
        std::fs::write(
            root.path()
                .join("sessions/workspace/session-fixture/session.jsonl.zstd"),
            b"invalid old log",
        )
        .unwrap();
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let output = provider.poll(SystemTime::now());
        assert_eq!(output.tasks.len(), 1);
        assert_eq!(output.tasks[0].status, TaskStatus::WaitingInput);
    }

    #[test]
    fn open_native_turn_between_steps_stays_running() {
        let record = serde_json::json!({"rows":{
            "sessionStats":{"seq":9,"val":{"turns":1,"openStep":null,"pendingCalls":{}}},
            "turnBoundary":{"seq":9,"val":{"openTurnStartSeq":2}}
        }});
        let metadata = parse_metadata_record(&record);
        assert_eq!(metadata.status, Some(TaskStatus::Running));
        assert_eq!(metadata.turn_start_seq, Some(2));
    }

    #[test]
    fn desktop_not_installed_is_empty_without_invoking_wsl() {
        let root = tempfile::tempdir().unwrap();
        let mut provider =
            DeepSeekHarnessProvider::with_native_root(root.path().join("not-installed"));
        let output = provider.poll(SystemTime::now());
        assert!(output.authoritative);
        assert!(output.tasks.is_empty());
    }

    #[test]
    fn projection_read_failure_retains_last_native_title() {
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), false, "用户的真实任务名", 11);
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let first = provider.poll(SystemTime::now()).tasks.remove(0);
        std::fs::write(
            root.path()
                .join("storages/session_projcache/sessions/session-fixture.json"),
            b"{partial",
        )
        .unwrap();
        let output = provider.poll(SystemTime::now());
        assert!(!output.authoritative);
        assert_eq!(output.tasks[0].name, first.name);
        assert_eq!(output.tasks[0].id, first.id);
        assert!(output.tasks[0].stale);
    }

    #[test]
    fn desktop_completion_notifies_once_and_new_turn_reopens_hidden_task() {
        use crate::attention::AttentionTracker;
        use crate::transition::{TaskEvent, TransitionEngine};
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), false, "Native task", 11);
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let mut tracker = AttentionTracker::open(root.path().join("attention.json"));
        let mut engine = TransitionEngine::new();
        let first = provider.poll(SystemTime::now()).tasks;
        assert!(engine.apply(&first, &[], SystemTime::now()).is_empty());
        assert_eq!(tracker.reconcile(first)[0].status, TaskStatus::Running);
        desktop_fixture(root.path(), true, "Native task", 12);
        let finished = provider.poll(SystemTime::now()).tasks;
        assert!(matches!(
            engine.apply(&finished, &[], SystemTime::now()).as_slice(),
            [TaskEvent::WaitingInput { .. }]
        ));
        assert!(engine.apply(&finished, &[], SystemTime::now()).is_empty());
        let attention = tracker.reconcile(finished);
        assert!(tracker.hide(&attention[0].id).unwrap().is_empty());
        let path = root
            .path()
            .join("sessions/workspace/session-fixture/session.v4.jsonl.zstd");
        let mut text =
            zstd::decode_all(std::io::Cursor::new(std::fs::read(&path).unwrap())).unwrap();
        text.extend_from_slice(
            format!(
                "{}\n",
                serde_json::json!({"type":"turn/start","seq":20,"time":1791500000000u64})
            )
            .as_bytes(),
        );
        std::fs::write(
            path,
            zstd::encode_all(std::io::Cursor::new(text), 3).unwrap(),
        )
        .unwrap();
        let resumed = provider.poll(SystemTime::now()).tasks;
        assert!(engine.apply(&resumed, &[], SystemTime::now()).is_empty());
        let attention = tracker.reconcile(resumed);
        assert_eq!(attention.len(), 1);
        assert_eq!(attention[0].status, TaskStatus::Running);
        assert_eq!(attention[0].turn_key.as_deref(), Some("turn:20"));
    }

    #[test]
    fn desktop_closed_history_is_not_imported_into_attention() {
        let root = tempfile::tempdir().unwrap();
        desktop_fixture(root.path(), true, "Old task", 12);
        let mut provider = DeepSeekHarnessProvider::with_native_root(root.path().to_path_buf());
        let mut attention =
            crate::attention::AttentionTracker::open(root.path().join("attention.json"));
        assert!(attention
            .reconcile(provider.poll(SystemTime::now()).tasks)
            .is_empty());
    }

    #[test]
    #[ignore = "requires real native DeepSeek Harness desktop data"]
    fn live_desktop_snapshot() {
        let mut provider = DeepSeekHarnessProvider::desktop();
        let started = Instant::now();
        let metadata = provider.session_metadata().unwrap();
        let output = provider.poll(SystemTime::now());
        println!(
            "desktop snapshot: {} ms, metadata={}, recent tasks={}, error={:?}",
            started.elapsed().as_millis(),
            metadata.len(),
            output.tasks.len(),
            output.error
        );
        assert!(output.authoritative);
        for task in &output.tasks {
            println!(
                "title={:?} project={:?} status={:?} session={:?}",
                task.name, task.project, task.status, task.session_id
            );
            assert!(task.id.starts_with("dsh-native:"));
        }
        if let Ok(title) = std::env::var("TASKPULSE_EXPECT_DSH_TITLE") {
            assert!(metadata
                .values()
                .any(|session| session.title.as_deref() == Some(title.as_str())));
            assert!(
                output.tasks.iter().any(|task| task.name == title),
                "expected desktop task was not emitted"
            );
        }
    }

    #[test]
    #[ignore = "read-only inspection of installed desktop session schema"]
    fn live_desktop_schema() {
        fn inspect(root: &std::path::Path) {
            let Ok(entries) = std::fs::read_dir(root) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    inspect(&path);
                    continue;
                }
                if path.file_name().and_then(|name| name.to_str()) != Some("session.v4.jsonl.zstd")
                {
                    continue;
                }
                let compressed = std::fs::read(&path).unwrap();
                let bytes = zstd::decode_all(std::io::Cursor::new(compressed)).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                let rows: Vec<serde_json::Value> = text
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();
                println!(
                    "session={} events={}",
                    session_id_from_path(&path.to_string_lossy()),
                    rows.len()
                );
                for row in rows.iter().take(4).chain(rows.iter().rev().take(12)) {
                    let keys: Vec<_> = row.as_object().unwrap().keys().collect();
                    let data_keys: Vec<_> = row
                        .get("data")
                        .and_then(|v| v.as_object())
                        .map(|v| v.keys().collect())
                        .unwrap_or_default();
                    println!(
                        "type={:?} seq={:?} keys={keys:?} data_keys={data_keys:?}",
                        row.get("type"),
                        row.get("seq")
                    );
                }
            }
        }
        inspect(&dirs_next::home_dir().unwrap().join(".dsh/sessions"));
    }

    // Parse-only tests (no WSL): feed JSONL event text through the same logic.
    // We exercise parse by building a zstd frame of the JSONL and decoding it
    // with the crate — this validates the decompress + infer path without WSL.
    fn parse_text_status(text: &str) -> TaskStatus {
        let enc = zstd::encode_all(std::io::Cursor::new(text.as_bytes()), 3).unwrap();
        let dec = zstd::decode_all(std::io::Cursor::new(enc)).unwrap();
        let text = String::from_utf8_lossy(&dec);
        parse_session_text(&text, "/workspace/session-test/session.jsonl.zstd")
            .unwrap()
            .status
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
        let metadata = parse_session_metadata(json.as_bytes()).unwrap();
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
        let metadata = parse_session_metadata(json).unwrap();
        assert_eq!(metadata["session-123"].status, Some(TaskStatus::Running));
    }
}
