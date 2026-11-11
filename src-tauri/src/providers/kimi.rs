//! Read-only Kimi Code and legacy kimi-cli wire journals. No config/credentials.
use super::{
    jsonl_tail::JsonlTail,
    native_sessions::{self, BatchRead, DirectorySweep, Sessions},
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

fn valid_storage_component(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', ':'])
}

struct Journal {
    tail: JsonlTail,
    state: WireState,
    invalid: bool,
    catching_up: bool,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            tail: JsonlTail::newline_committed(),
            state: WireState::default(),
            invalid: false,
            catching_up: false,
        }
    }
}
pub struct KimiProvider {
    roots: Vec<PathBuf>,
    logs: HashMap<PathBuf, Journal>,
    sessions: Sessions,
    sweep: DirectorySweep,
    pending: Vec<(String, PathBuf, PathBuf)>,
    complete_page: bool,
    priority_offset: usize,
}
impl KimiProvider {
    pub fn new(location: Option<PathBuf>) -> Self {
        Self {
            roots: location.map(|p| vec![p]).unwrap_or_else(default_roots),
            logs: HashMap::new(),
            sessions: Default::default(),
            sweep: Default::default(),
            pending: Vec::new(),
            complete_page: true,
            priority_offset: 0,
        }
    }
    pub fn resume(&mut self, tasks: impl IntoIterator<Item = TaskSnapshot>) {
        self.sessions.resume("kimi-code", tasks);
    }
    fn read(&mut self) -> (Vec<TaskSnapshot>, bool) {
        let started = Instant::now();
        let mut incomplete = false;
        let mut workdirs = vec![];
        for root in &self.roots {
            let meta = root.join("kimi.json");
            if !meta.exists() {
                continue;
            }
            let Ok(meta) = read_json(&meta) else {
                incomplete = true;
                continue;
            };
            if let Some(dirs) = meta.get("work_dirs").and_then(Value::as_array) {
                for wd in dirs {
                    if let (Some(id), Some(path)) = (text(wd, "last_session_id"), text(wd, "path"))
                    {
                        workdirs.push((id.to_string(), path.to_string()));
                    }
                }
            }
        }
        let protected = self.sessions.protected_ids("kimi-code");
        if self.pending.is_empty() {
            let roots = self
                .roots
                .iter()
                .map(|root| root.join("sessions"))
                .collect::<Vec<_>>();
            let (pending, status) = self.sweep.page(&roots, 768, started, |entry, depth| {
                if !entry
                    .file_type()
                    .is_ok_and(|t| t.is_dir() && !t.is_symlink())
                {
                    return (None, None);
                }
                if depth == 1 {
                    return (Some(entry.path()), None);
                }
                let dir = entry.path();
                let modern = dir.join("agents/main/wire.jsonl");
                let path = if modern.is_file() {
                    modern
                } else {
                    dir.join("wire.jsonl")
                };
                let candidate = path
                    .is_file()
                    .then(|| (entry.file_name().to_string_lossy().into_owned(), path, dir));
                (None, candidate)
            });
            self.pending = pending;
            self.complete_page = status == BatchRead::Complete && self.pending.len() <= 128;
            self.pending.sort_by_cached_key(|(id, path, _)| {
                (
                    protected.contains(id),
                    path.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH),
                )
            });
        }
        let mut files: Vec<_> = self
            .pending
            .drain(self.pending.len().saturating_sub(128)..)
            .collect();
        // Protected paths already seen in earlier pages remain polled while
        // rotating through dormant history. Never treat a missing path as end.
        let mut priority: Vec<_> = self
            .logs
            .iter()
            .filter_map(|(path, log)| {
                let id = log.state.task.as_ref()?.session_id.as_ref()?;
                if !protected.contains(id) {
                    return None;
                }
                let dir = if path.parent().is_some_and(|p| p.ends_with("agents/main")) {
                    path.ancestors().nth(3)?
                } else {
                    path.parent()?
                };
                Some((id.clone(), path.clone(), dir.to_path_buf()))
            })
            .collect();
        priority.sort();
        if priority.len() > 128 {
            let len = priority.len();
            priority.rotate_left(self.priority_offset % len);
            self.priority_offset = self.priority_offset.wrapping_add(128);
            priority.truncate(128);
            incomplete = true;
        }
        for item in priority {
            if !files.iter().any(|(_, p, _)| p == &item.1) {
                files.push(item);
            }
        }
        for (path, _) in self.logs.iter().filter(|(_, log)| log.catching_up).take(8) {
            if files.iter().any(|(_, p, _)| p == path) {
                continue;
            }
            let dir = if path.parent().is_some_and(|p| p.ends_with("agents/main")) {
                path.ancestors().nth(3)
            } else {
                path.parent()
            };
            if let Some(dir) = dir {
                if let Some(id) = dir.file_name().and_then(|s| s.to_str()) {
                    files.push((id.into(), path.clone(), dir.to_path_buf()));
                }
            }
        }
        files.sort_by_cached_key(|(id, path, _)| {
            std::cmp::Reverse((
                protected.contains(id),
                path.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH),
            ))
        });
        incomplete |= !self.complete_page || !self.pending.is_empty();
        let workspace_by_last: HashMap<_, _> = workdirs.into_iter().collect();
        let mut buckets = HashMap::new();
        for (id, _, dir) in &files {
            if let Some(parent) = dir.parent() {
                if let Some(cwd) = workspace_by_last.get(id) {
                    buckets.insert(parent.to_path_buf(), cwd.clone());
                }
            }
        }
        // A page may not contain the metadata's last-session row. Recover the
        // bounded set of remaining workspace associations on demand.
        let mut workspace_checks = 256usize;
        let mut tasks = vec![];
        let mut bytes = 0usize;
        let mut metadata_bytes = 0u64;
        let mut live = HashSet::new();
        let mut seen = HashSet::new();
        let mut duplicates = HashSet::new();
        let mut files = files.into_iter();
        while let Some((id, path, dir)) = files.next() {
            if bytes >= 8 * 1024 * 1024 || started.elapsed() > Duration::from_millis(500) {
                let deferred = std::iter::once((id, path, dir))
                    .chain(files)
                    .filter(|(_, p, _)| !self.pending.iter().any(|(_, existing, _)| existing == p))
                    .collect::<Vec<_>>();
                self.pending.splice(0..0, deferred);
                incomplete = true;
                break;
            }
            if !seen.insert(id.clone()) {
                duplicates.insert(id);
                incomplete = true;
                continue;
            }
            live.insert(path.clone());
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
            let (caught_up, gap) = 'read: {
                let Ok(batch) = log.tail.read(&path) else {
                    break 'read (false, true);
                };
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
                (batch.caught_up, batch.gap)
            };
            log.catching_up = !caught_up && !gap && !log.invalid;
            if !caught_up || gap || log.invalid || log.state.uncertain {
                incomplete = true;
                continue;
            }
            if !log.state.recognized && log.tail.bytes_read_total > 0 {
                incomplete = true;
                continue;
            }
            if let Some(mut task) = log.state.task.clone() {
                let metadata = dir.join("state.json");
                if metadata.exists() {
                    let Ok(stat) = metadata.metadata() else {
                        incomplete = true;
                        continue;
                    };
                    metadata_bytes = metadata_bytes.saturating_add(stat.len());
                    if metadata_bytes > 4 * 1024 * 1024 {
                        incomplete = true;
                        continue;
                    }
                    let Ok(meta) = read_json(&metadata) else {
                        incomplete = true;
                        continue;
                    };
                    if text(&meta, "id").is_some_and(|value| value != id) {
                        incomplete = true;
                        continue;
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
                task.native_storage = Some(native_sessions::storage_key(&dir));
                if task.project_path.is_none() {
                    if let Some(parent) = dir.parent() {
                        if !buckets.contains_key(parent) && workspace_checks > 0 {
                            for (last, cwd) in workspace_by_last
                                .iter()
                                .filter(|(last, _)| valid_storage_component(last))
                            {
                                if workspace_checks == 0 {
                                    break;
                                }
                                workspace_checks -= 1;
                                if parent
                                    .join(last)
                                    .symlink_metadata()
                                    .is_ok_and(|m| m.is_dir())
                                {
                                    buckets.insert(parent.to_path_buf(), cwd.clone());
                                    break;
                                }
                            }
                        }
                        task.project_path = buckets.get(parent).cloned();
                    }
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
        tasks.retain(|task| {
            task.session_id
                .as_ref()
                .is_none_or(|id| !duplicates.contains(id))
        });
        let mut catchup = 32;
        self.logs.retain(|path, log| {
            if log.catching_up && catchup > 0 {
                catchup -= 1;
                return true;
            }
            live.contains(path)
                || log
                    .state
                    .task
                    .as_ref()
                    .and_then(|t| t.session_id.as_ref())
                    .is_some_and(|id| protected.contains(id))
        });
        (tasks, incomplete)
    }
}
impl Provider for KimiProvider {
    fn name(&self) -> &'static str {
        "kimi-code"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let (tasks, incomplete) = self.read();
        self.sessions.observe_batch(
            tasks,
            now,
            incomplete.then(|| "Kimi 会话分批读取或部分记录不可读；保留上次状态".into()),
        )
    }
}

#[cfg(test)]
mod tests;
