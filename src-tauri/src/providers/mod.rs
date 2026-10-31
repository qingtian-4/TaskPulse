//! Provider layer: every data source implements `Provider` and emits
//! normalized `TaskSnapshot`s.  Providers never talk to the tray, the store,
//! or the UI — they only observe and normalize.

pub mod claude_code;
pub mod codex;
pub mod codex_wsl;
pub mod deepseek_harness;
pub mod doubao;
pub mod generic_event;
pub mod hermes;
pub mod jsonl_tail;
pub mod local_file;
mod native_sessions;
pub mod opencode;
mod parser_cache;
pub mod remote_tmux;
pub mod resources;
pub mod workbuddy;
pub mod worker;
pub mod zcode;

use std::time::SystemTime;

use crate::domain::TaskSnapshot;

/// Health of a remote host as seen by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    Connected,
    Disconnected,
}

/// Per-host health summary (empty for local providers).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostHealth {
    pub host: String,
    pub status: HostStatus,
    /// Last time we successfully polled this host.
    #[serde(
        default,
        with = "opt_epoch_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_seen: Option<SystemTime>,
}

mod opt_epoch_ms {
    use serde::Serializer;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(t: &Option<SystemTime>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match t {
            Some(t) => {
                let ms = t
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                ser.serialize_some(&ms)
            }
            None => ser.serialize_none(),
        }
    }
}

impl HostHealth {
    pub fn connected(host: impl Into<String>, last_seen: SystemTime) -> Self {
        Self {
            host: host.into(),
            status: HostStatus::Connected,
            last_seen: Some(last_seen),
        }
    }

    pub fn disconnected(host: impl Into<String>, last_seen: Option<SystemTime>) -> Self {
        Self {
            host: host.into(),
            status: HostStatus::Disconnected,
            last_seen,
        }
    }

    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self.status == HostStatus::Connected
    }
}

/// Output of one provider poll: normalized tasks plus (for remote providers)
/// per-host health.
#[derive(Debug, Clone)]
pub struct RemoteSourceHealth {
    pub host: String,
    pub source: crate::domain::TaskSource,
    pub authoritative: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct ProviderOutput {
    /// Durable ordered integration facts; never coalesce into latest snapshots.
    pub ordered: Vec<generic_event::OrderedObservation>,
    pub tasks: Vec<TaskSnapshot>,
    pub catalog: Vec<TaskSnapshot>,
    pub hosts: Vec<HostHealth>,
    pub remote_health: Vec<RemoteSourceHealth>,
    /// Whether this poll is a complete, authoritative view of the provider.
    /// A degraded poll may re-emit stale tasks, but must never reconcile away
    /// tasks that are absent from its partial result.
    pub authoritative: bool,
    pub error: Option<String>,
}

impl Default for ProviderOutput {
    fn default() -> Self {
        Self {
            ordered: vec![],
            tasks: Vec::new(),
            catalog: Vec::new(),
            hosts: Vec::new(),
            remote_health: Vec::new(),
            authoritative: true,
            error: None,
        }
    }
}

impl ProviderOutput {
    pub fn tasks(tasks: Vec<TaskSnapshot>) -> Self {
        Self {
            ordered: vec![],
            tasks,
            catalog: vec![],
            hosts: Vec::new(),
            remote_health: Vec::new(),
            authoritative: true,
            error: None,
        }
    }

    pub fn degraded(tasks: Vec<TaskSnapshot>, error: impl Into<String>) -> Self {
        Self {
            ordered: vec![],
            tasks,
            catalog: vec![],
            hosts: Vec::new(),
            remote_health: Vec::new(),
            authoritative: false,
            error: Some(error.into()),
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty() && self.hosts.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Connected,
    Degraded,
}

/// Health shown for local data sources. Remote availability remains modeled
/// per host through `HostHealth`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProviderHealth {
    pub provider: String,
    pub status: ProviderStatus,
    /// Runtime-only evidence from the latest completed Codex collection. A bad
    /// rollout must not invalidate a different, fully verified session. This
    /// map is replaced on every attempt and never stored in task checkpoints.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub task_observations: std::collections::HashMap<String, TaskObservation>,
    #[serde(
        default,
        with = "opt_epoch_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_success: Option<SystemTime>,
    /// Most recent completed attempt, including failures. Never a cached read.
    #[serde(
        default,
        with = "opt_epoch_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_attempt: Option<SystemTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TaskObservation {
    pub observed_at: u64,
    pub state: String,
}

/// Match the exact observed lifecycle, not just a stable session ID. Store
/// reconciliation may retain a newer turn when a rollout has been rewound.
pub fn task_observation_state(task: &TaskSnapshot) -> String {
    let mut requests = task.input_request_ids.clone();
    if let Some(id) = &task.input_request_id {
        requests.push(id.clone());
    }
    // Match JavaScript's UTF-16 string ordering for the shared wire key.
    requests.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    requests.dedup();
    serde_json::json!([
        task.session_id,
        task.turn_key,
        task.status,
        task.waiting_reason,
        resources::epoch_ms(task.updated_at),
        task.finished_at.map(resources::epoch_ms),
        requests
    ])
    .to_string()
}

impl ProviderHealth {
    pub fn verifies_task(&self, task: &TaskSnapshot, now: SystemTime) -> bool {
        let now = resources::epoch_ms(now);
        task.source == crate::domain::TaskSource::Codex
            && !task.stale
            && !task.internal
            && task.confidence != crate::domain::DetectionConfidence::Inferred
            && (!task.requires_action()
                || task.waiting_reason == Some(crate::domain::WaitingReason::QuotaExhausted)
                || task
                    .input_request_id
                    .as_deref()
                    .is_some_and(|id| !id.trim().is_empty())
                || task
                    .input_request_ids
                    .iter()
                    .any(|id| !id.trim().is_empty()))
            && task.source_instance.as_deref() == Some(&self.provider)
            && self.status == ProviderStatus::Degraded
            && self.task_observations.get(&task.id).is_some_and(|proof| {
                Some(proof.observed_at) == self.last_attempt.map(resources::epoch_ms)
                    && now >= proof.observed_at
                    && now - proof.observed_at <= 30_000
                    && proof.state == task_observation_state(task)
            })
    }
}

#[cfg(test)]
mod partial_observation_tests {
    use super::*;
    #[test]
    fn proof_is_bound_to_source_turn_request_and_observed_lifecycle() {
        use crate::domain::{DetectionConfidence, TaskSource, TaskStatus, WaitingReason};
        let now = SystemTime::now();
        let mut task = TaskSnapshot::new(
            "codex:session",
            "native",
            TaskSource::Codex,
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
            now,
        );
        task.source_instance = Some("codex".into());
        task.session_id = Some("session".into());
        task.turn_key = Some("turn".into());
        task.waiting_reason = Some(WaitingReason::Approval);
        task.input_request_id = Some("request".into());
        let mut health = ProviderHealth {
            provider: "codex".into(),
            status: ProviderStatus::Degraded,
            last_success: None,
            last_attempt: Some(now),
            message: Some("unrelated gap".into()),
            task_observations: [(
                task.id.clone(),
                TaskObservation {
                    observed_at: resources::epoch_ms(now),
                    state: task_observation_state(&task),
                },
            )]
            .into(),
        };
        assert!(health.verifies_task(&task, now));
        let mut changed = task.clone();
        changed.turn_key = Some("newer-turn".into());
        assert!(!health.verifies_task(&changed, now));
        changed = task.clone();
        changed.input_request_id = Some("different-request".into());
        assert!(!health.verifies_task(&changed, now));
        changed = task.clone();
        changed.updated_at += std::time::Duration::from_secs(1);
        assert!(!health.verifies_task(&changed, now));
        changed = task.clone();
        changed.stale = true;
        assert!(!health.verifies_task(&changed, now));
        changed = task.clone();
        changed.confidence = DetectionConfidence::Inferred;
        assert!(!health.verifies_task(&changed, now));
        changed = task.clone();
        changed.source_instance = Some("another-source".into());
        assert!(!health.verifies_task(&changed, now));
        assert!(!health.verifies_task(&task, now + std::time::Duration::from_secs(31)));
        health.last_attempt = Some(now + std::time::Duration::from_secs(1));
        assert!(!health.verifies_task(&task, now));
    }
}

/// A pollable data source.
pub trait Provider: Send {
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    /// Poll once and return normalized snapshots.  Implementations must be
    /// side-effect-free with respect to task state: a failed poll must NOT
    /// emit "finished" snapshots (see the remote provider's failure rules).
    fn poll(&mut self, now: SystemTime) -> ProviderOutput;
}

/// Sort a list of snapshots by attention priority then name (stable display
/// ordering used by the store and the UI).
pub fn sort_tasks(tasks: &mut [TaskSnapshot]) {
    tasks.sort_by(|a, b| {
        a.status
            .priority()
            .cmp(&b.status.priority())
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Keep source-reported model identifiers verbatim, with bounded display data.
/// Never derive an Agent identity or lifecycle signal from a model name.
pub(crate) fn reported_model(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && !value.starts_with('<')
        && value.chars().count() <= 200
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

#[cfg(test)]
mod model_evidence_tests {
    use super::*;
    #[test]
    #[ignore = "read-only real native Claude, Hermes and DSH data; no commands or prompts logged"]
    fn live_native_model_evidence() {
        let providers: Vec<Box<dyn Provider>> = vec![
            Box::new(claude_code::ClaudeCodeProvider::new(
                claude_code::default_root(),
            )),
            Box::new(hermes::HermesProvider::new()),
            Box::new(deepseek_harness::DeepSeekHarnessProvider::desktop()),
        ];
        let mut observed_sources = 0;
        for mut provider in providers {
            let out = provider.poll(SystemTime::now());
            assert!(out.authoritative, "{}: {:?}", provider.name(), out.error);
            let models: std::collections::BTreeSet<_> = out
                .tasks
                .iter()
                .chain(out.catalog.iter())
                .filter_map(|task| task.model.clone())
                .collect();
            println!(
                "provider={} tasks={} models={models:?}",
                provider.name(),
                out.tasks.len()
            );
            if out.tasks.is_empty() && out.catalog.is_empty() {
                println!(
                    "{}: no eligible recent sessions; not validated",
                    provider.name()
                );
                continue;
            }
            observed_sources += 1;
            assert!(
                !models.is_empty(),
                "{} had no native model evidence",
                provider.name()
            );
        }
        assert!(observed_sources > 0, "No real sources were validated");
    }
}
