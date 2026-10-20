use super::{Provider, ProviderOutput};
use crate::{
    domain::TaskSnapshot,
    integrations::{
        self,
        journal::{self, Cursor},
        reducer::Session,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableObservation {
    pub task: TaskSnapshot,
    pub epoch: String,
    pub sequence: u64,
    pub received_at: u64,
    #[serde(default)]
    pub owner: String,
}
#[derive(Debug, Clone)]
pub struct OrderedObservation {
    pub task: TaskSnapshot,
    pub receipt: Receipt,
    pub notify: bool,
}
#[derive(Debug, Clone)]
pub struct Receipt {
    pub source: String,
    pub session_key: String,
    pub epoch: String,
    pub sequence: u64,
}
#[derive(Default, Serialize, Deserialize)]
struct Checkpoint {
    cursor: Cursor,
    state: Session,
    outbox: Vec<DurableObservation>,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    rebuilding: bool,
}

fn read_checkpoint(path: &Path) -> Result<Checkpoint, String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > 16 * 1024 * 1024 => Err("checkpoint_too_large".into()),
        Ok(_) => match serde_json::from_slice(
            &std::fs::read(path).map_err(|_| "checkpoint_read_failed")?,
        ) {
            Ok(cp) => Ok(cp),
            Err(_) => {
                // Preserve the damaged bytes before rebuilding from the
                // append-only journal. Never silently overwrite evidence.
                let backup = path.with_extension("corrupt.bak");
                if !backup.exists() {
                    std::fs::copy(path, backup).map_err(|_| "checkpoint_backup_failed")?;
                }
                Ok(Checkpoint {
                    rebuilding: true,
                    owner: "checkpoint-recovery".into(),
                    ..Default::default()
                })
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Checkpoint::default()),
        Err(_) => Err("checkpoint_read_failed".into()),
    }
}
pub fn acknowledge(home: &Path, receipt: &Receipt) -> Result<(), String> {
    if receipt.session_key.len() != 64
        || !receipt.session_key.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err("invalid_receipt".into());
    }
    let dir = journal::root(home, &receipt.source).join("state");
    let _guard = journal::lock(
        &dir.join(format!("{}.lock", receipt.session_key)),
        Duration::from_millis(500),
    )?;
    let path = dir.join(format!("{}.json", receipt.session_key));
    let mut cp = read_checkpoint(&path)?;
    cp.outbox
        .retain(|e| e.epoch != receipt.epoch || e.sequence > receipt.sequence);
    journal::atomic(&path, &cp)
}

pub struct GenericEventProvider {
    source: crate::monitoring::SourceConfig,
    home: PathBuf,
    owner: String,
    next: usize,
    latest: BTreeMap<String, TaskSnapshot>,
    started: u64,
}
impl GenericEventProvider {
    pub fn new(source: crate::monitoring::SourceConfig) -> Self {
        Self::with_home(source, integrations::home())
    }
    pub fn with_home(source: crate::monitoring::SourceConfig, home: PathBuf) -> Self {
        Self {
            source,
            home,
            owner: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                format!(
                    "{}:{}:{}",
                    std::process::id(),
                    integrations::now_ms(),
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                )
            },
            next: 0,
            latest: BTreeMap::new(),
            started: integrations::now_ms(),
        }
    }
    fn sample(&mut self) -> Result<ProviderOutput, String> {
        let integration = self
            .source
            .integration
            .as_ref()
            .ok_or("integration_not_configured")?;
        let dir = journal::root(&self.home, &self.source.id);
        let mut paths: Vec<_> = match std::fs::read_dir(dir.join("events")) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(_) => return Err("event_directory_unreadable".into()),
        };
        paths.sort();
        let mut observations = vec![];
        let mut degraded = false;
        let mut budget = 128usize;
        let count = paths.len();
        let start = self.next;
        for n in 0..count.min(256) {
            if budget == 0 {
                degraded = true;
                break;
            }
            let index = (start + n) % count;
            let path = &paths[index];
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("invalid_journal_path")?;
            let state_dir = dir.join("state");
            let _guard = journal::lock(
                &state_dir.join(format!("{stem}.lock")),
                Duration::from_millis(500),
            )?;
            let state_path = state_dir.join(format!("{stem}.json"));
            let mut cp = read_checkpoint(&state_path)?;
            degraded |= cp.rebuilding;
            let recovered = !cp.owner.is_empty() && cp.owner != self.owner;
            // Do not ingest beyond the durable bounded outbox until the UI
            // consumer has persisted attention + notification dedup and acked.
            if cp.outbox.is_empty() {
                let batch = journal::read_batch(path, &cp.cursor)?;
                degraded |=
                    batch.corrupt > 0 || batch.backlog || batch.generation_changed || batch.partial;
                for envelope in &batch.events {
                    if envelope.source_instance_id != self.source.id
                        || integrations::key(&envelope.event.session_id) != stem
                    {
                        return Err("event_identity_mismatch".into());
                    }
                    envelope.event.validate(&integration.agent_id)?;
                    if envelope.received_at < integration.accept_since_ms {
                        continue;
                    }
                    if let Some(mut task) = cp.state.apply(envelope, &self.source.name)? {
                        task.native_unread =
                            (recovered && task.finished_at.is_some()).then_some(true);
                        cp.outbox.push(DurableObservation {
                            task,
                            epoch: envelope.journal_epoch.clone(),
                            sequence: envelope.sequence,
                            received_at: envelope.received_at,
                            owner: if cp.rebuilding {
                                "checkpoint-recovery".into()
                            } else {
                                self.owner.clone()
                            },
                        });
                    }
                }
                cp.cursor = batch.cursor;
                cp.rebuilding &= batch.backlog;
                cp.owner = self.owner.clone();
                journal::atomic(&state_path, &cp)?;
            }
            if let Some(task) = &cp.state.task {
                self.latest.insert(task.id.clone(), task.clone());
            }
            for item in cp.outbox.iter().take(budget) {
                let notify = item.owner == self.owner
                    && item.received_at >= self.started
                    && integrations::now_ms().saturating_sub(item.received_at) < 5 * 60_000;
                let mut task = item.task.clone();
                if !notify && task.finished_at.is_some() {
                    task.native_unread = Some(true);
                }
                observations.push(OrderedObservation {
                    task,
                    notify,
                    receipt: Receipt {
                        source: self.source.id.clone(),
                        session_key: stem.into(),
                        epoch: item.epoch.clone(),
                        sequence: item.sequence,
                    },
                });
                budget -= 1;
            }
            self.next = (index + 1) % count;
        }
        let tasks: Vec<_> = self.latest.values().cloned().collect();
        Ok(ProviderOutput {
            tasks: tasks.clone(),
            catalog: tasks,
            ordered: observations,
            authoritative: !degraded,
            error: degraded.then(|| "事件积压/日志世代变化/坏行：保留任务，继续有界恢复".into()),
            ..Default::default()
        })
    }
}
impl Provider for GenericEventProvider {
    fn name(&self) -> &'static str {
        "generic-events"
    }
    fn poll(&mut self, _now: SystemTime) -> ProviderOutput {
        self.sample().unwrap_or_else(|error| {
            ProviderOutput::degraded(
                self.latest
                    .values()
                    .cloned()
                    .map(|mut t| {
                        t.stale = true;
                        t
                    })
                    .collect(),
                error,
            )
        })
    }
}
