//! Read-only Kimi Code and legacy kimi-cli wire journals. No config/credentials.
use super::{
    jsonl_tail::JsonlTail,
    native_sessions::{self, Sessions},
    Provider, ProviderOutput,
};
use crate::domain::TaskSnapshot;
mod wire;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use wire::WireState;

pub fn default_roots() -> Vec<PathBuf> {
    let home = dirs_next::home_dir().unwrap_or_default();
    distinct_roots(vec![
        std::env::var_os("KIMI_CODE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".kimi-code")),
        std::env::var_os("KIMI_SHARE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".kimi")),
    ])
}
fn distinct_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    roots
        .into_iter()
        .filter(|root| seen.insert(root.canonicalize().unwrap_or_else(|_| root.clone())))
        .collect()
}
pub fn available(root: &Path) -> bool {
    root.join("sessions").is_dir()
}
pub(super) fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)?.as_str().filter(|s| !s.is_empty())
}
pub(super) fn stamp(v: &Value) -> Option<SystemTime> {
    if let Some(s) = v.as_str() {
        return super::codex::parse_rfc3339(s);
    }
    let n = v.as_f64()?;
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    let ms = if n < 1e11 { n * 1000.0 } else { n };
    native_sessions::time(ms as u64)
}
pub(super) fn read_json(path: &Path) -> Result<Value, String> {
    let mut f = std::fs::File::open(path).map_err(|_| "metadata_read")?;
    use std::io::Read;
    let mut b = vec![];
    f.by_ref()
        .take(256 * 1024 + 1)
        .read_to_end(&mut b)
        .map_err(|_| "metadata_read")?;
    if b.len() > 256 * 1024 {
        return Err("metadata_budget".into());
    }
    serde_json::from_slice(&b).map_err(|_| "metadata_json".into())
}

struct Journal {
    tail: JsonlTail,
    state: WireState,
    invalid: bool,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            tail: JsonlTail::newline_committed(),
            state: WireState::default(),
            invalid: false,
        }
    }
}
pub struct KimiProvider {
    roots: Vec<PathBuf>,
    logs: HashMap<PathBuf, Journal>,
    sessions: Sessions,
}
impl KimiProvider {
    pub fn new(location: Option<PathBuf>) -> Self {
        Self {
            roots: location.map(|p| vec![p]).unwrap_or_else(default_roots),
            logs: HashMap::new(),
            sessions: Default::default(),
        }
    }
    pub fn resume(&mut self, tasks: impl IntoIterator<Item = TaskSnapshot>) {
        self.sessions.resume("kimi-code", tasks);
    }
    fn read(&mut self) -> Result<Vec<TaskSnapshot>, String> {
        let started = Instant::now();
        let mut files = vec![];
        let mut seen = HashSet::new();
        let mut visited = 0usize;
        let mut workdirs = vec![];
        for root in &self.roots {
            let meta = root.join("kimi.json");
            if meta.exists() {
                let meta = read_json(&meta)?;
                if let Some(dirs) = meta.get("work_dirs").and_then(Value::as_array) {
                    for wd in dirs {
                        if let (Some(id), Some(path)) =
                            (text(wd, "last_session_id"), text(wd, "path"))
                        {
                            workdirs.push((id.to_string(), path.to_string()));
                        }
                    }
                }
            }
            let base = root.join("sessions");
            if !base.try_exists().map_err(|_| "session_listing")? {
                continue;
            }
            for w in std::fs::read_dir(base).map_err(|_| "session_listing")? {
                visited += 1;
                if visited > 4096 || started.elapsed() > Duration::from_millis(500) {
                    return Err("session_listing_budget".into());
                }
                let w = w.map_err(|_| "session_listing")?;
                if !w.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                    continue;
                }
                for s in std::fs::read_dir(w.path()).map_err(|_| "session_listing")? {
                    visited += 1;
                    if visited > 4096 || started.elapsed() > Duration::from_millis(500) {
                        return Err("session_listing_budget".into());
                    }
                    let s = s.map_err(|_| "session_listing")?;
                    if !s.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                        continue;
                    }
                    let id = s.file_name().to_string_lossy().into_owned();
                    let modern = s.path().join("agents/main/wire.jsonl");
                    let path = if modern.is_file() {
                        modern
                    } else {
                        s.path().join("wire.jsonl")
                    };
                    if path.is_file() {
                        if !seen.insert(id.clone()) {
                            return Err("duplicate_session_id".into());
                        }
                        files.push((id, path, s.path()));
                        if files.len() > 512 {
                            return Err("session_admission_budget".into());
                        }
                    }
                }
            }
        }
        let mut buckets = HashMap::new();
        for (last, cwd) in workdirs {
            if let Some((_, _, dir)) = files.iter().find(|(id, _, _)| id == &last) {
                if let Some(parent) = dir.parent() {
                    buckets.insert(parent.to_path_buf(), cwd);
                }
            }
        }
        files.sort_by_key(|(_, p, _)| {
            std::cmp::Reverse(
                p.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH),
            )
        });
        let mut tasks = vec![];
        let mut bytes = 0usize;
        let mut metadata_bytes = 0u64;
        let mut live = HashSet::new();
        let mut incomplete = false;
        for (id, path, dir) in files {
            live.insert(path.clone());
            if bytes >= 8 * 1024 * 1024 || started.elapsed() > Duration::from_millis(500) {
                incomplete = true;
                break;
            }
            if !self.logs.contains_key(&path)
                && self
                    .logs
                    .values()
                    .filter(|l| l.tail.has_partial_line())
                    .count()
                    >= 8
            {
                incomplete = true;
                continue;
            }
            let log = self.logs.entry(path.clone()).or_default();
            let (caught_up, gap) = loop {
                let batch = log.tail.read(&path).map_err(|_| "wire_read")?;
                bytes += batch.bytes_read;
                if batch.reset {
                    log.state = Default::default();
                    log.invalid = false;
                }
                for line in batch.body.lines() {
                    match serde_json::from_str::<Value>(line) {
                        Ok(v) => {
                            if let Err(code) = log.state.apply(&v, &id) {
                                if code.starts_with("wire_") {
                                    log.state.uncertain = true;
                                } else {
                                    log.invalid = true;
                                }
                            }
                        }
                        Err(_) => log.invalid = true,
                    }
                }
                log.tail.trim_read_buffer();
                if batch.caught_up
                    || batch.bytes_read == 0
                    || bytes >= 8 * 1024 * 1024
                    || started.elapsed() > Duration::from_millis(500)
                {
                    break (batch.caught_up, batch.gap);
                }
            };
            if !caught_up || gap || log.invalid || log.state.uncertain {
                incomplete = true;
                continue;
            }
            if !log.state.recognized && log.tail.bytes_read_total > 0 {
                return Err("unsupported_wire_format".into());
            }
            if let Some(mut task) = log.state.task.clone() {
                let metadata = dir.join("state.json");
                if metadata.exists() {
                    metadata_bytes = metadata_bytes
                        .saturating_add(metadata.metadata().map_err(|_| "metadata_read")?.len());
                    if metadata_bytes > 4 * 1024 * 1024 {
                        return Err("metadata_round_budget".into());
                    }
                    let meta = read_json(&metadata)?;
                    if text(&meta, "id").is_some_and(|value| value != id) {
                        return Err("metadata_session_mismatch".into());
                    }
                    if let Some(title) =
                        text(&meta, "title").or_else(|| text(&meta, "custom_title"))
                    {
                        task.name = title.chars().take(1024).collect();
                    }
                    if let Some(cwd) = text(&meta, "cwd") {
                        task.project_path = Some(cwd.into());
                    }
                }
                if task.project_path.is_none() {
                    task.project_path = dir.parent().and_then(|p| buckets.get(p)).cloned();
                }
                if task.name.is_empty() {
                    task.name = format!("Kimi · {}", id.chars().take(8).collect::<String>());
                }
                task.project = task
                    .project_path
                    .as_ref()
                    .and_then(|p| Path::new(p).file_name())
                    .map(|p| p.to_string_lossy().into_owned());
                tasks.push(task);
            }
        }
        if incomplete {
            return Err("wire_catching_up_or_invalid".into());
        }
        self.logs.retain(|p, _| live.contains(p));
        Ok(tasks)
    }
}
impl Provider for KimiProvider {
    fn name(&self) -> &'static str {
        "kimi-code"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        match self.read() {
            Ok(tasks) => self.sessions.observe(tasks, now),
            Err(reason) => self
                .sessions
                .degraded(&format!("Kimi 会话暂不可读（{reason}）；保留上次状态")),
        }
    }
}

#[cfg(test)]
mod tests;
