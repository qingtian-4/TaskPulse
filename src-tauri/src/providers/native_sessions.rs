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
    pub fn observe(&mut self, rows: Vec<TaskSnapshot>, now: SystemTime) -> ProviderOutput {
        let baseline = *self.baseline.get_or_insert(now);
        let mut current = HashMap::new();
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
                if older || reopened {
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
            let executing = task.status == TaskStatus::Running && fresh;
            if task.new_execution || executing {
                self.followed.insert(task.id.clone());
            }
            task.stale |= task.status == TaskStatus::Running && !fresh;
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
        let catalog = current.values().cloned().collect();
        self.previous = current;
        ProviderOutput {
            tasks,
            catalog,
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
