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
mod parser_cache;
pub mod remote_tmux;
pub mod resources;
pub mod worker;

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
