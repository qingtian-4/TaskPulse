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
    #[serde(skip)]
    dirty: bool,
    cursor: Cursor,
    state: Session,
    outbox: Vec<DurableObservation>,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    rebuilding: bool,
    #[serde(default)]
    quarantine: Vec<RejectedRecord>,
    #[serde(default)]
    rejected_count: u64,
    #[serde(default)]
    uncertain_turns: std::collections::BTreeSet<String>,
    #[serde(default)]
    recovered_gaps: std::collections::BTreeSet<String>,
}

fn save_checkpoint(path: &Path, cp: &Checkpoint) -> Result<(), String> {
    journal::serialized_size(&cp.state, 10 * 1024 * 1024)?;
    journal::serialized_size(&cp.outbox, journal::OUTBOX_LIMIT)?;
    journal::atomic_bounded(path, cp, journal::CHECKPOINT_LIMIT)?;
    journal::checkpoint_written(
        std::fs::metadata(path)
            .map_err(|_| "checkpoint_stat_failed")?
            .len(),
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct RejectedRecord {
    epoch: String,
    sequence: u64,
    event_key: String,
    code: String,
}
impl Checkpoint {
    fn reject(&mut self, envelope: &integrations::schema::Envelope, code: String) {
        self.rejected_count = self.rejected_count.saturating_add(1);
        if self.quarantine.len() >= 64 {
            self.quarantine.remove(0);
        }
        self.quarantine.push(RejectedRecord {
            epoch: envelope.journal_epoch.clone(),
            sequence: envelope.sequence,
            event_key: integrations::key(&envelope.event.event_id),
            code,
        });
        if let Some(turn) = envelope
            .event
            .turn_id
            .as_ref()
            .filter(|turn| self.state.has_turn(turn))
        {
            self.uncertain_turns.insert(turn.clone());
        }
        // Original facts remain in the append-only journal. The checkpoint
        // retains a bounded diagnostic index, never raw prompts or credentials.
    }
}

fn read_checkpoint(path: &Path) -> Result<Checkpoint, String> {
    match std::fs::metadata(path) {
        // Legacy recovery is separate from the normal writer budget. Preserve
        // exact old bytes first; stream-deserialize rather than allocating a
        // second complete file. New writes remain capped at the original16MiB.
        Ok(meta) if meta.len() > 128 * 1024 * 1024 => {
            Err("legacy_checkpoint_export_required".into())
        }
        Ok(meta) => {
            if meta.len() > journal::CHECKPOINT_LIMIT as u64 {
                journal::backup(path, "oversized.bak")?;
            }
            match serde_json::from_reader::<_, Checkpoint>(std::io::BufReader::new(
                std::fs::File::open(path).map_err(|_| "checkpoint_read_failed")?,
            )) {
                Ok(mut cp) => {
                    if !cp.state.supported_encoding() {
                        return Err("unsupported_checkpoint_encoding".into());
                    }
                    if cp.state.normalize_identities() {
                        journal::backup(path, "pre-budget.bak")?;
                        cp.dirty = true;
                    }
                    Ok(cp)
                }
                Err(_) => {
                    // Preserve the damaged bytes before rebuilding from the
                    // append-only journal. Never silently overwrite evidence.
                    let backup = path.with_extension("corrupt.bak");
                    if !backup.exists() {
                        std::fs::copy(path, backup).map_err(|_| "checkpoint_backup_failed")?;
                    }
                    Ok(Checkpoint {
                        dirty: true,
                        rebuilding: true,
                        owner: "checkpoint-recovery".into(),
                        ..Default::default()
                    })
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Checkpoint {
            dirty: true,
            ..Default::default()
        }),
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
    let before = cp.outbox.len();
    cp.outbox
        .retain(|e| e.epoch != receipt.epoch || e.sequence > receipt.sequence);
    if cp.dirty || before != cp.outbox.len() {
        save_checkpoint(&path, &cp)?;
    }
    Ok(())
}

pub struct GenericEventProvider {
    source: crate::monitoring::SourceConfig,
    home: PathBuf,
    owner: String,
    next: usize,
    latest: BTreeMap<String, TaskSnapshot>,
    started: u64,
    received_at: u64,
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
            received_at: 0,
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
            let result = (|| -> Result<(), String> {
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
                let gaps = integrations::health::load(&self.home, &self.source.id, stem)?;
                let old_gaps = cp.recovered_gaps.len();
                cp.recovered_gaps
                    .retain(|id| gaps.iter().any(|g| &g.id == id));
                cp.dirty |= old_gaps != cp.recovered_gaps.len();
                degraded |= cp.rebuilding;
                let recovered = !cp.owner.is_empty() && cp.owner != self.owner;
                // Do not ingest beyond the durable bounded outbox until the UI
                // consumer has persisted attention + notification dedup and acked.
                if cp.outbox.is_empty() {
                    let batch = journal::read_batch(path, &cp.cursor)?;
                    cp.dirty |= batch.cursor != cp.cursor || !batch.events.is_empty();
                    let mut outbox_bytes = 2usize;
                    let mut paused = false;
                    degraded |= batch.corrupt > 0
                        || batch.backlog
                        || batch.generation_changed
                        || batch.partial;
                    for (event_index, envelope) in batch.events.iter().enumerate() {
                        // Reserve for the largest legal snapshot before
                        // mutating the reducer. Pause exactly at this fact's
                        // boundary; never advance past an undelivered end.
                        if !cp.outbox.is_empty()
                            && outbox_bytes + 256 * 1024 > journal::OUTBOX_LIMIT
                        {
                            cp.cursor = batch.event_cursors[event_index - 1].clone();
                            paused = true;
                            degraded = true;
                            break;
                        }
                        let valid = if envelope.source_instance_id != self.source.id
                            || integrations::key(&envelope.event.session_id) != stem
                        {
                            Err("event_identity_mismatch".into())
                        } else {
                            envelope.event.validate(&integration.agent_id)
                        };
                        if let Err(code) = valid {
                            cp.reject(envelope, code);
                            degraded = true;
                            continue;
                        }
                        if envelope.received_at < integration.accept_since_ms {
                            continue;
                        }
                        let applied = match cp.state.apply(envelope, &self.source.name) {
                            Ok(task) => task,
                            Err(code) => {
                                cp.reject(envelope, code);
                                degraded = true;
                                continue;
                            }
                        };
                        for gap in &gaps {
                            if integrations::health::recovers(
                                gap,
                                envelope,
                                applied.as_ref().is_some_and(|t| t.new_execution),
                            ) {
                                cp.recovered_gaps.insert(gap.id.clone());
                            }
                        }
                        if matches!(
                            envelope.event.kind,
                            integrations::schema::Kind::Completed
                                | integrations::schema::Kind::Failed
                                | integrations::schema::Kind::Interrupted
                        ) {
                            if let Some(turn) = &envelope.event.turn_id {
                                cp.uncertain_turns.remove(turn);
                            }
                        }
                        if applied.as_ref().is_some_and(|t| t.new_execution) {
                            cp.uncertain_turns
                                .retain(|turn| Some(turn) == envelope.event.turn_id.as_ref());
                        }
                        if let Some(mut task) = applied {
                            task.stale |= gaps.iter().any(|g| !cp.recovered_gaps.contains(&g.id));
                            task.native_unread =
                                (recovered && task.finished_at.is_some()).then_some(true);
                            let observation = DurableObservation {
                                task,
                                epoch: envelope.journal_epoch.clone(),
                                sequence: envelope.sequence,
                                received_at: envelope.received_at,
                                owner: if cp.rebuilding {
                                    "checkpoint-recovery".into()
                                } else {
                                    self.owner.clone()
                                },
                            };
                            outbox_bytes += journal::serialized_size(&observation, 256 * 1024)? + 1;
                            cp.outbox.push(observation);
                        }
                    }
                    if !paused {
                        cp.cursor = batch.cursor;
                    }
                    let rebuilding = cp.rebuilding && batch.backlog;
                    cp.dirty |= cp.rebuilding != rebuilding || cp.owner != self.owner;
                    cp.rebuilding = rebuilding;
                    cp.owner = self.owner.clone();
                }
                if cp.dirty {
                    save_checkpoint(&state_path, &cp)?;
                }
                let has_gap = gaps.iter().any(|g| !cp.recovered_gaps.contains(&g.id));
                degraded |= !cp.uncertain_turns.is_empty() || has_gap;
                if let Some(task) = &cp.state.task {
                    let mut task = task.clone();
                    task.stale |= !cp.uncertain_turns.is_empty() || has_gap;
                    self.latest.insert(task.id.clone(), task);
                }
                self.received_at = self.received_at.max(cp.state.last_received_at);
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
                Ok(())
            })();
            if let Err(code) = result {
                // A locked/corrupt/unwritable session is not a provider-wide
                // abort. Its cursor stays retryable; healthy peers still drain.
                log::warn!("integration session deferred: {code}");
                degraded = true;
            }
            self.next = (index + 1) % count;
        }
        let receive_error = std::fs::read(dir.join("last-error.json"))
            .ok()
            .filter(|b| b.len() <= 1024)
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .filter(|v| v["at"].as_u64().is_some_and(|at| at >= self.received_at))
            .and_then(|v| {
                v["code"].as_str().map(|s| {
                    s.chars()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .take(96)
                        .collect::<String>()
                })
            });
        // Transport health can recover independently. Invalid producer input
        // is diagnostic only; durable per-session gaps above decide trust.
        let receive_error = receive_error.filter(|code| integrations::health::operational(code));
        degraded |= receive_error.is_some();
        let tasks: Vec<_> = self.latest.values().cloned().collect();
        Ok(ProviderOutput {
            tasks: tasks.clone(),
            catalog: tasks,
            ordered: observations,
            authoritative: !degraded,
            error: if let Some(code) = receive_error {
                Some(format!("接收事件失败：{code}；保留任务，不推断结束"))
            } else {
                degraded.then(|| "事件积压/日志世代变化/坏行：保留任务，继续有界恢复".into())
            },
            ..Default::default()
        })
    }
}
/// Bounded, payload-free operator evidence for quarantined accepted records.
pub fn quarantine_summary(home: &Path, source: &str) -> serde_json::Value {
    let mut bytes_left = 16 * 1024 * 1024;
    let mut rejected = 0u64;
    let mut indexed = 0usize;
    let mut complete = true;
    let mut unresolved_gaps = 0usize;
    if let Ok(entries) = std::fs::read_dir(journal::root(home, source).join("state")) {
        let paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .take(257)
            .collect();
        if paths.len() > 256 {
            complete = false;
        }
        for path in paths.into_iter().take(256) {
            let size = std::fs::metadata(&path)
                .map(|m| m.len())
                .unwrap_or(u64::MAX);
            if size > bytes_left {
                complete = false;
                continue;
            }
            bytes_left -= size;
            let session = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            match std::fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Checkpoint>(&b).ok())
            {
                Some(cp) => {
                    rejected = rejected.saturating_add(cp.rejected_count);
                    indexed += cp.quarantine.len();
                    match integrations::health::load(home, source, &session) {
                        Ok(gaps) => {
                            unresolved_gaps += gaps
                                .iter()
                                .filter(|g| !cp.recovered_gaps.contains(&g.id))
                                .count()
                        }
                        Err(_) => complete = false,
                    }
                }
                None => complete = false,
            }
        }
    }
    let (writes, bytes) = journal::checkpoint_stats();
    serde_json::json!({"rejected_records":rejected,"retained_index_records":indexed,"unresolved_fact_gaps":unresolved_gaps,"bounded_scan_complete":complete,"original_facts_retained_in_journal":true,"process_checkpoint_writes":writes,"process_checkpoint_bytes":bytes})
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

#[cfg(test)]
mod budget_tests {
    use super::*;
    use integrations::schema::{Envelope, Event, Kind, Payload};
    #[test]
    fn f03_one_hundred_idle_polls_write_nothing_and_ack_is_idempotent() {
        let home = tempfile::tempdir().unwrap();
        let src: crate::monitoring::SourceConfig = serde_json::from_value(serde_json::json!({"id":"source","name":"Fixture","kind":"generic_event","enabled":true,"integration":{"agent_id":"a","adapter_id":"standard-events","transport":"local","accept_since_ms":0}})).unwrap();
        let e: Event = serde_json::from_value(serde_json::json!({"schema_version":1,"agent_id":"a","session_id":"s","event_id":"start","turn_id":"turn","kind":"turn.started","payload":{}})).unwrap();
        journal::append(home.path(), "source", e, "self_reported", "1").unwrap();
        let mut p = GenericEventProvider::with_home(src.clone(), home.path().into());
        let first = p.poll(SystemTime::now());
        let receipt = first.ordered.last().unwrap().receipt.clone();
        acknowledge(home.path(), &receipt).unwrap();
        p.poll(SystemTime::now());
        let before = journal::local_checkpoint_stats();
        for _ in 0..100 {
            assert!(p.poll(SystemTime::now()).ordered.is_empty());
        }
        acknowledge(home.path(), &receipt).unwrap();
        let after = journal::local_checkpoint_stats();
        eprintln!(
            "idle100+duplicate_ack: writes={}, bytes={}",
            after.0 - before.0,
            after.1 - before.1
        );
        assert_eq!(after, before);
        let mut restarted = GenericEventProvider::with_home(src, home.path().into());
        assert!(restarted.poll(SystemTime::now()).ordered.is_empty());
        let taken = journal::local_checkpoint_stats();
        assert_eq!(taken.0, before.0 + 1); // New owner persisted once, no history replay.
        restarted.poll(SystemTime::now());
        assert_eq!(journal::local_checkpoint_stats(), taken);
    }
    #[test]
    fn f02_6800_maximum_field_events_do_not_create_an_unreadable_checkpoint() {
        let mut cp = Checkpoint::default();
        let mut rejected = 0;
        for n in 0..6800 {
            let e = Event {
                schema_version: 1,
                agent_id: "a".into(),
                session_id: "s".into(),
                event_id: format!("{n:08}{}", "x".repeat(504)),
                turn_id: Some("turn".into()),
                call_id: (n > 0).then(|| format!("{n:08}{}", "c".repeat(504))),
                request_id: None,
                occurred_at: None,
                kind: if n == 0 {
                    Kind::Started
                } else {
                    Kind::ToolStarted
                },
                payload: Payload {
                    activity: Some("a".repeat(2048)),
                    ..Default::default()
                },
            };
            e.validate("a").unwrap();
            let env = Envelope {
                version: 1,
                source_instance_id: "source".into(),
                sequence: n + 1,
                journal_epoch: "epoch".into(),
                received_at: n + 1,
                adapter_version: "1".into(),
                evidence: "self_reported".into(),
                event: e,
                context: None,
            };
            rejected += usize::from(cp.state.apply(&env, "Agent").is_err());
        }
        assert_eq!(cp.state.calls.len(), 128);
        assert_eq!(rejected, 6671);
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("state.json");
        journal::atomic(&path, &cp).unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        eprintln!("6800 maximum records checkpoint bytes={size}");
        assert!(size <= 16 * 1024 * 1024);
        assert!(read_checkpoint(&path).is_ok());
    }

    #[test]
    fn f02_legacy_oversized_checkpoint_keeps_evidence_and_pending_and_can_end() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("legacy.json");
        let mut old = serde_json::to_value(Checkpoint::default()).unwrap();
        old["state"]
            .as_object_mut()
            .unwrap()
            .remove("identity_encoding");
        old["state"]["seen"] = serde_json::json!((0..6800)
            .map(|n| format!("{n:08}{}", "x".repeat(504)))
            .collect::<Vec<_>>());
        old["state"]["turns"] = serde_json::json!(["turn"]);
        let mut task = TaskSnapshot::new(
            "legacy",
            "Legacy",
            crate::domain::TaskSource::GenericEvent,
            crate::domain::TaskStatus::WaitingInput,
            crate::domain::DetectionConfidence::Inferred,
            SystemTime::now(),
        );
        task.turn_key = Some("turn".into());
        task.unread = true;
        task.pinned = true;
        old["state"]["task"] = serde_json::to_value(task).unwrap();
        let pending_key = serde_json::to_string(&("turn", "request")).unwrap();
        old["state"]["pending"] = serde_json::json!({pending_key:{"turn":"turn","request":"request","call":null,"reason":"approval","confirmed_at":1}});
        old["state"]["calls"] = serde_json::json!((0..6800)
            .map(|n| (format!("{n:08}{}", "c".repeat(504)), "a".repeat(2048)))
            .collect::<BTreeMap<_, _>>());
        journal::atomic(&path, &old).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(before.len() > journal::CHECKPOINT_LIMIT);
        let cp = read_checkpoint(&path).unwrap();
        assert_eq!(cp.state.seen.len(), 6800);
        assert_eq!(cp.state.calls.len(), 6800);
        assert_eq!(cp.state.pending.len(), 1);
        assert!(cp.state.task.as_ref().unwrap().unread);
        assert!(cp.state.task.as_ref().unwrap().pinned);
        assert_eq!(
            std::fs::read(path.with_extension("oversized.bak")).unwrap(),
            before
        );
        save_checkpoint(&path, &cp).unwrap();
        let after = std::fs::metadata(&path).unwrap().len();
        assert!(after < journal::CHECKPOINT_LIMIT as u64);
        let mut cp = read_checkpoint(&path).unwrap();
        assert_eq!(cp.state.seen.len(), 6800);
        assert!(cp.state.has_turn("turn"));
        assert_eq!(cp.state.calls.len(), 6800);
        let end:Envelope=serde_json::from_value(serde_json::json!({"version":1,"source_instance_id":"source","sequence":6801,"journal_epoch":"epoch","received_at":6801,"adapter_version":"1","evidence":"self_reported","event":{"schema_version":1,"agent_id":"a","session_id":"s","event_id":"end","turn_id":"turn","kind":"turn.completed","payload":{}}})).unwrap();
        assert!(cp
            .state
            .apply(&end, "Agent")
            .unwrap()
            .unwrap()
            .finished_at
            .is_some());
        assert!(cp.state.pending.is_empty());
        assert!(cp.state.calls.is_empty());
        save_checkpoint(&path, &cp).unwrap();
        assert!(read_checkpoint(&path).unwrap().state.task.unwrap().unread);
        eprintln!("legacy checkpoint before={} after={after}", before.len());
    }

    #[test]
    fn f02_bounded_writer_preserves_previous_bytes_on_overflow() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("state.json");
        journal::atomic(&path, &serde_json::json!({"keep":true})).unwrap();
        let old = std::fs::read(&path).unwrap();
        assert!(journal::atomic_bounded(&path, &"x".repeat(1024), 128).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old);
    }

    #[test]
    fn f02_large_pending_outbox_pauses_at_fact_boundary_and_drains_after_restart() {
        let home = tempfile::tempdir().unwrap();
        let src:crate::monitoring::SourceConfig=serde_json::from_value(serde_json::json!({"id":"source","name":"Fixture","kind":"generic_event","enabled":true,"location":"","projects":["*"],"excluded_projects":[],"tmux_sessions":[],"tmux_panes":[],"poll_interval_ms":1000,"integration":{"agent_id":"a","adapter_id":"standard-events","transport":"local","accept_since_ms":0}})).unwrap();
        let mut lines = Vec::new();
        for n in 0..260 {
            let kind = if n == 0 {
                Kind::Started
            } else if n <= 128 {
                Kind::Requested
            } else if n == 259 {
                Kind::Completed
            } else {
                Kind::Phase
            };
            let e = Event {
                schema_version: 1,
                agent_id: "a".into(),
                session_id: "s".into(),
                event_id: format!("e-{n}"),
                turn_id: Some("turn".into()),
                call_id: None,
                request_id: (kind == Kind::Requested).then(|| format!("{n:08}{}", "r".repeat(504))),
                occurred_at: None,
                kind,
                payload: Payload {
                    title: Some("t".repeat(2048)),
                    project: Some("p".repeat(2048)),
                    project_path: Some("x".repeat(2048)),
                    model: Some("m".repeat(2048)),
                    request_kind: (kind == Kind::Requested)
                        .then_some(crate::domain::WaitingReason::Approval),
                    ..Default::default()
                },
            };
            e.validate("a").unwrap();
            let env = Envelope {
                version: 1,
                source_instance_id: "source".into(),
                sequence: n + 1,
                journal_epoch: "epoch".into(),
                received_at: integrations::now_ms(),
                adapter_version: "1".into(),
                evidence: "self_reported".into(),
                event: e,
                context: None,
            };
            serde_json::to_writer(&mut lines, &env).unwrap();
            lines.push(b'\n');
        }
        let dir = journal::root(home.path(), "source").join("events");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{}.jsonl", integrations::key("s"))), lines).unwrap();
        let mut seen = 0;
        let mut end = false;
        for _ in 0..30 {
            let mut p = GenericEventProvider::with_home(src.clone(), home.path().into());
            let out = p.poll(SystemTime::now());
            seen += out.ordered.len();
            end |= out.ordered.iter().any(|o| o.task.finished_at.is_some());
            if let Some(last) = out.ordered.last() {
                acknowledge(home.path(), &last.receipt).unwrap();
            }
            let path = journal::root(home.path(), "source")
                .join("state")
                .join(format!("{}.json", integrations::key("s")));
            assert!(std::fs::metadata(&path).unwrap().len() <= journal::CHECKPOINT_LIMIT as u64);
            assert!(read_checkpoint(&path).is_ok());
            if end {
                break;
            }
        }
        assert!(end);
        assert_eq!(seen, 260);
    }
}
