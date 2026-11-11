//! Shared attention admission for read-only native adapters, independent of brand.
use super::ProviderOutput;
use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn time(ms: u64) -> Option<SystemTime> {
    (ms > 0 && ms < 4_102_444_800_000).then(|| UNIX_EPOCH + Duration::from_millis(ms))
}
pub fn task(
    identity: (&str, &str),
    session: &str,
    title: &str,
    project: &str,
    turn: &str,
    started: u64,
    activity: u64,
) -> Option<TaskSnapshot> {
    let (agent, name) = identity;
    let mut task = TaskSnapshot::new(
        format!("{agent}:{session}"),
        title,
        TaskSource::NativeAgent,
        TaskStatus::Running,
        DetectionConfidence::Inferred,
        time(activity.max(started))?,
    );
    task.agent_id = Some(agent.into());
    task.agent_name = Some(name.into());
    task.session_id = Some(session.into());
    task.evidence_channel = Some("native".into());
    task.turn_key = Some(turn.into());
    task.started_at = Some(time(started)?);
    task.project_path = (!project.is_empty()).then(|| project.into());
    // Windows paths also remain meaningful in Unix fixture/compatibility runs.
    task.project = Path::new(project.trim_end_matches(['/', '\\']))
        .file_name()
        .map(|s| {
            s.to_string_lossy()
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .into()
        });
    Some(task)
}
#[derive(Default)]
pub struct Sessions {
    previous: HashMap<String, TaskSnapshot>,
    followed: HashSet<String>,
    baseline: Option<SystemTime>,
}
impl Sessions {
    pub fn resume(&mut self, agent: &str, tasks: impl IntoIterator<Item = TaskSnapshot>) {
        for mut task in tasks {
            if let Some(session) = &task.session_id {
                task.id = format!("{agent}:{session}");
                task.new_execution = false;
                self.followed.insert(task.id.clone());
                self.previous.insert(task.id.clone(), task);
            }
        }
    }
    pub fn protected_ids(&self, agent: &str) -> HashSet<String> {
        self.previous
            .values()
            .filter(|task| {
                task.agent_id.as_deref() == Some(agent)
                    && self.followed.contains(&task.id)
                    && (task.finished_at.is_none() || task.pinned)
            })
            .filter_map(|task| task.session_id.clone())
            .collect()
    }

    pub fn observe(&mut self, rows: Vec<TaskSnapshot>, now: SystemTime) -> ProviderOutput {
        self.observe_inner(rows, now, None, false)
    }

    pub fn observe_batch(
        &mut self,
        rows: Vec<TaskSnapshot>,
        now: SystemTime,
        incomplete: Option<String>,
    ) -> ProviderOutput {
        self.observe_inner(rows, now, incomplete, true)
    }

    fn observe_inner(
        &mut self,
        rows: Vec<TaskSnapshot>,
        now: SystemTime,
        mut incomplete: Option<String>,
        bounded: bool,
    ) -> ProviderOutput {
        let baseline = *self.baseline.get_or_insert(now);
        let mut current = HashMap::new();
        let mut catalog = Vec::new();
        for mut task in rows {
            let old = self.previous.get(&task.id);
            if let Some(old) = old {
                let older = task
                    .started_at
                    .zip(old.started_at)
                    .is_some_and(|(next, previous)| next < previous);
                let reopened = task.turn_key == old.turn_key
                    && old.finished_at.is_some()
                    && task.status == TaskStatus::Running;
                let different_storage = old
                    .native_storage
                    .as_ref()
                    .zip(task.native_storage.as_ref())
                    .is_some_and(|(a, b)| a != b);
                if different_storage {
                    incomplete
                        .get_or_insert_with(|| "存在多个同名会话记录；保留已确认的任务".into());
                }
                if older || reopened || different_storage {
                    let mut retained = old.clone();
                    retained.stale = true;
                    retained.new_execution = false;
                    current.insert(retained.id.clone(), retained);
                    continue;
                }
            }
            task.new_execution = old.is_some_and(|old| old.turn_key != task.turn_key)
                || old.is_none() && task.started_at.is_some_and(|start| start >= baseline);
            let fresh =
                now.duration_since(task.updated_at).unwrap_or_default() < Duration::from_secs(120);
            // Title/model changes cannot enroll old idle history or renew a turn.
            let executing = (task.status == TaskStatus::Running || task.requires_action()) && fresh;
            if (task.new_execution || executing) && !self.followed.contains(&task.id) {
                if bounded && self.followed.len() >= 512 {
                    incomplete
                        .get_or_insert_with(|| "活动会话已达本轮监控容量；保留已有任务".into());
                } else {
                    self.followed.insert(task.id.clone());
                }
            }
            task.stale |= task.status == TaskStatus::Running && !fresh;
            catalog.push(task.clone());
            current.insert(task.id.clone(), task);
        }
        for (id, old) in &self.previous {
            if self.followed.contains(id) && !current.contains_key(id) && old.finished_at.is_none()
            {
                let mut task = old.clone();
                task.stale = true;
                task.new_execution = false;
                current.insert(id.clone(), task);
            }
        }
        self.followed.retain(|id| current.contains_key(id));
        let tasks = current
            .values()
            .filter(|t| self.followed.contains(&t.id))
            .cloned()
            .collect();
        self.previous = current;
        ProviderOutput {
            tasks,
            catalog,
            authoritative: incomplete.is_none(),
            error: incomplete,
            ..Default::default()
        }
    }
    pub fn degraded(&self, message: &str) -> ProviderOutput {
        ProviderOutput::degraded(
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
            message,
        )
    }
}

pub(super) fn storage_key(path: &Path) -> String {
    // The configured roots are fixed for the lifetime of a worker. Hash the
    // storage path instead of exporting it or folding it into the task ID.
    let path = path.to_string_lossy();
    if cfg!(windows) {
        crate::integrations::key(&path.replace('/', "\\").to_ascii_lowercase())
    } else {
        crate::integrations::key(&path)
    }
}

#[derive(Default)]
pub(super) struct DirectorySweep {
    root: usize,
    stack: Vec<std::fs::ReadDir>,
    incomplete: bool,
    paged: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum BatchRead {
    #[default]
    Complete,
    Paginating,
    Failed,
}

impl DirectorySweep {
    pub fn page<T>(
        &mut self,
        roots: &[std::path::PathBuf],
        visit_limit: usize,
        start: std::time::Instant,
        mut item: impl FnMut(std::fs::DirEntry, usize) -> (Option<std::path::PathBuf>, Option<T>),
    ) -> (Vec<T>, BatchRead) {
        let mut rows = Vec::new();
        for _ in 0..visit_limit {
            if start.elapsed() > Duration::from_millis(350) {
                break;
            }
            let Some(dir) = self.stack.last_mut() else {
                let Some(root) = roots.get(self.root) else {
                    let status = if self.incomplete {
                        BatchRead::Failed
                    } else if self.paged {
                        BatchRead::Paginating
                    } else {
                        BatchRead::Complete
                    };
                    *self = Self::default();
                    return (rows, status);
                };
                self.root += 1;
                match std::fs::read_dir(root) {
                    Ok(dir) => self.stack.push(dir),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => self.incomplete = true,
                }
                continue;
            };
            match dir.next() {
                Some(Ok(entry)) => {
                    let (child, row) = item(entry, self.stack.len());
                    rows.extend(row);
                    if let Some(child) = child {
                        match std::fs::read_dir(child) {
                            Ok(dir) => self.stack.push(dir),
                            Err(_) => self.incomplete = true,
                        }
                    }
                }
                Some(Err(_)) => self.incomplete = true,
                None => {
                    self.stack.pop();
                }
            }
        }
        self.paged = true;
        (
            rows,
            if self.incomplete {
                BatchRead::Failed
            } else {
                BatchRead::Paginating
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(turn: &str, start: u64) -> TaskSnapshot {
        task(
            ("agent", "Agent"),
            "s",
            "Native title",
            "/project",
            turn,
            start,
            start,
        )
        .unwrap()
    }
    #[test]
    fn another_storage_record_cannot_complete_a_restored_followed_session() {
        let mut original = row("t", 100000);
        original.native_storage = Some("known-storage".into());
        let mut resumed = Sessions::default();
        resumed.resume("agent", [original.clone()]);
        let mut alternate = original.clone();
        alternate.native_storage = Some("other-storage".into());
        alternate.updated_at = time(105000).unwrap();
        alternate.finished_at = Some(alternate.updated_at);
        alternate.status = TaskStatus::Succeeded;
        let stale = resumed.observe_batch(vec![alternate], time(106000).unwrap(), None);
        assert!(!stale.authoritative && stale.catalog.is_empty());
        assert!(stale.tasks[0].stale && stale.tasks[0].finished_at.is_none());
        original.status = TaskStatus::Succeeded;
        original.updated_at = time(107000).unwrap();
        original.finished_at = Some(original.updated_at);
        let ended = resumed.observe_batch(vec![original], time(108000).unwrap(), None);
        assert!(
            ended.authoritative && !ended.tasks[0].stale && ended.tasks[0].finished_at.is_some()
        );
    }

    #[test]
    fn rename_old_history_restart_and_rewound_storage_keep_attention_bound_to_the_real_turn() {
        let mut s = Sessions::default();
        let mut old = row("old", 100000);
        old.status = TaskStatus::Succeeded;
        old.finished_at = Some(old.updated_at);
        assert!(s
            .observe(vec![old.clone()], time(200000).unwrap())
            .tasks
            .is_empty());
        old.name = "Renamed".into();
        assert!(s.observe(vec![old], time(201000).unwrap()).tasks.is_empty());
        let running = s.observe(vec![row("current", 202000)], time(203000).unwrap());
        assert!(running.tasks[0].new_execution);
        let mut restart = Sessions::default();
        restart.resume("agent", running.tasks);
        let resumed = restart.observe(vec![row("current", 202000)], time(204000).unwrap());
        assert!(!resumed.tasks[0].new_execution);
        let rewound = restart.observe(vec![row("old", 100000)], time(205000).unwrap());
        assert_eq!(rewound.tasks[0].turn_key.as_deref(), Some("current"));
        assert!(rewound.tasks[0].stale);
        assert!(!rewound.tasks[0].new_execution);
        let mut ended = row("current", 202000);
        ended.status = TaskStatus::Succeeded;
        ended.finished_at = time(206000);
        ended.updated_at = time(206000).unwrap();
        restart.observe(vec![ended], time(207000).unwrap());
        let flapped = restart.observe(vec![row("current", 202000)], time(208000).unwrap());
        assert_eq!(flapped.tasks[0].status, TaskStatus::Succeeded);
        assert!(!flapped.tasks[0].new_execution);
    }
}
