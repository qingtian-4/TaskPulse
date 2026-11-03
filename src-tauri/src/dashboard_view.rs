//! One presentation projection for the popup and tray. Keep these fields
//! runtime-only; they do not change lifecycle, attention or source settings.
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus, WaitingReason};
use crate::monitoring::{collector_for, DashboardSettings, SourceEvidenceSummary, SourceKind};
use crate::providers::{resources::epoch_ms, ProviderHealth, ProviderStatus};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadState {
    Disabled,
    Waiting,
    Healthy,
    Partial,
    Failed,
    Stale,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceSnapshot {
    pub id: String,
    // Compare the observed connection with the current draft/applied row.
    // A saved source can retain its ID while changing its location.
    pub kind: SourceKind,
    pub location: String,
    pub enabled: bool,
    pub read_state: ReadState,
    /// Collector evidence, independent of local discovery or connection health.
    pub data_detected: Option<bool>,
    /// Unique public sessions observed by this source in this app run.
    pub discovered_sessions: usize,
    /// Verified ongoing tasks in the monitored attention scope.
    pub current_tasks: usize,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub ongoing: usize,
    pub action: usize,
    pub results: usize,
    pub failed: usize,
    pub uncertain: usize,
}

#[derive(Clone, Default, Serialize)]
pub struct DashboardView {
    #[serde(skip)]
    pub expires_at: Option<SystemTime>,
    pub counts: Counts,
    pub sources: Vec<SourceSnapshot>,
    pub unconfirmed_task_ids: Vec<String>,
}

impl DashboardView {
    pub const fn empty() -> Self {
        Self {
            counts: Counts {
                ongoing: 0,
                action: 0,
                results: 0,
                failed: 0,
                uncertain: 0,
            },
            expires_at: None,
            sources: Vec::new(),
            unconfirmed_task_ids: Vec::new(),
        }
    }
}

fn consider_expiry(view: &mut DashboardView, now: SystemTime, observed_ms: u64, ttl_ms: u64) {
    let expiry = UNIX_EPOCH.checked_add(Duration::from_millis(
        observed_ms.saturating_add(ttl_ms).saturating_add(1),
    ));
    if let Some(expiry) = expiry.filter(|&at| at > now) {
        view.expires_at = Some(
            view.expires_at
                .map_or(expiry, |current| current.min(expiry)),
        );
    }
}

pub fn read_fresh(health: Option<&ProviderHealth>, now: SystemTime, poll_interval_ms: u64) -> bool {
    let Some(health) = health else { return false };
    let now = epoch_ms(now);
    health.status == ProviderStatus::Connected
        && health.last_attempt.is_some()
        && health.last_success.map(epoch_ms).is_some_and(|at| {
            now >= at && now - at <= poll_interval_ms.saturating_mul(3).max(30_000)
        })
}

pub fn has_current_request(task: &TaskSnapshot, now: SystemTime) -> bool {
    let generic = task.source == TaskSource::GenericEvent;
    let credible = task.confidence != DetectionConfidence::Inferred
        || generic
            && task.evidence_channel.as_deref() == Some("self_reported")
            && task
                .integration_epoch
                .as_deref()
                .is_some_and(|epoch| !epoch.is_empty())
            && task.integration_sequence > 0;
    credible
        && (task.waiting_reason == Some(WaitingReason::QuotaExhausted)
            || task
                .input_request_id
                .iter()
                .chain(&task.input_request_ids)
                .any(|id| !id.trim().is_empty()))
        && (!generic
            || task.pending_confirmed_at.is_some_and(|at| {
                let now = epoch_ms(now);
                now >= at && now - at <= 30_000
            }))
}

/// The same read evidence gates headline counts and manual prompt reopening.
pub fn task_read_fresh(
    task: &TaskSnapshot,
    source_kind: SourceKind,
    provider: Option<&ProviderHealth>,
    now: SystemTime,
    interval: u64,
) -> bool {
    if matches!(source_kind, SourceKind::Codex | SourceKind::CodexWsl) {
        provider.is_some_and(|h| {
            h.verifies_task(task, now)
                || read_fresh(Some(h), now, interval) && h.matches_latest_observation(task, now)
        })
    } else {
        read_fresh(provider, now, interval)
    }
}

pub fn task_candidate(task: &TaskSnapshot) -> bool {
    !task.internal
        && (task.pinned
            || !(task.confidence == DetectionConfidence::Inferred
                && (task.source_instance.as_deref() == Some("scanner")
                    || task.id.starts_with("scan:"))
                && [&task.session_id, &task.project, &task.project_path]
                    .iter()
                    .all(|v| v.as_deref().is_none_or(|s| s.trim().is_empty()))
                && (task.name.trim().is_empty()
                    || task.name.trim().eq_ignore_ascii_case("unknown"))))
}

pub fn result_to_review(task: &TaskSnapshot) -> bool {
    task.unread
        && !task.requires_action()
        && (matches!(task.status, TaskStatus::Succeeded | TaskStatus::Failed)
            || task.status == TaskStatus::WaitingInput
                && task.waiting_reason == Some(WaitingReason::TurnEnded)
            || task.allows_inferred_finish()
                && task.confidence == DetectionConfidence::Inferred
                && task.status == TaskStatus::Idle
                && task.exit_code.is_none())
}

pub fn project(
    tasks: &[TaskSnapshot],
    providers: &[ProviderHealth],
    settings: &DashboardSettings,
    evidence: &SourceEvidenceSummary,
    now: SystemTime,
) -> DashboardView {
    let mut health = HashMap::new();
    // Real provider rows precede compatibility persistence diagnostics.
    for provider in providers {
        health.entry(provider.provider.as_str()).or_insert(provider);
    }
    let mut view = DashboardView::default();
    let mut indices = HashMap::new();
    for source in &settings.sources {
        let enabled = settings.onboarding_complete && source.enabled;
        let provider = health.get(source.id.as_str()).copied();
        let (data_seen, discovered_sessions) =
            evidence.get(&source.id).copied().unwrap_or_default();
        let read_state = match provider {
            _ if !enabled => ReadState::Disabled,
            None => ReadState::Waiting,
            Some(h)
                if h.last_attempt.is_none()
                    && h.message.as_deref() != Some(crate::providers::PROVIDER_TIMEOUT) =>
            {
                ReadState::Waiting
            }
            Some(h) if h.status == ProviderStatus::Degraded => {
                if matches!(source.kind, SourceKind::Codex | SourceKind::CodexWsl)
                    && h.has_fresh_observations(now)
                {
                    ReadState::Partial
                } else {
                    ReadState::Failed
                }
            }
            Some(h) if read_fresh(Some(h), now, source.poll_interval_ms) => ReadState::Healthy,
            Some(_) => ReadState::Stale,
        };
        if let Some(h) = provider {
            if read_state == ReadState::Healthy {
                if let Some(at) = h.last_success {
                    consider_expiry(
                        &mut view,
                        now,
                        epoch_ms(at),
                        source.poll_interval_ms.saturating_mul(3).max(30_000),
                    );
                }
            }
            if matches!(read_state, ReadState::Healthy | ReadState::Partial)
                && h.has_fresh_observations(now)
            {
                if let Some(at) = h.last_attempt {
                    consider_expiry(&mut view, now, epoch_ms(at), 30_000);
                }
            }
        }
        indices.insert(source.id.as_str(), view.sources.len());
        view.sources.push(SourceSnapshot {
            id: source.id.clone(),
            kind: source.kind,
            location: source.location.clone(),
            enabled,
            read_state,
            data_detected: if data_seen {
                Some(true)
            } else if read_state == ReadState::Healthy {
                Some(false)
            } else {
                None
            },
            discovered_sessions,
            current_tasks: 0,
        });
    }
    for task in tasks.iter().filter(|t| task_candidate(t)) {
        let action = task.requires_action();
        let ongoing = task.status == TaskStatus::Running || action;
        let mut stale = task.stale;
        if ongoing && !stale {
            stale = true;
            if let Some(source) = collector_for(settings, task) {
                let row = &mut view.sources[indices[source.id.as_str()]];
                if row.enabled {
                    let fresh = task_read_fresh(
                        task,
                        source.kind,
                        health.get(source.id.as_str()).copied(),
                        now,
                        source.poll_interval_ms,
                    );
                    stale = !fresh || action && !has_current_request(task, now);
                    if !stale {
                        row.current_tasks += 1;
                        if action && task.source == TaskSource::GenericEvent {
                            if let Some(at) = task.pending_confirmed_at {
                                consider_expiry(&mut view, now, at, 30_000);
                            }
                        }
                    }
                }
            }
            if stale {
                view.unconfirmed_task_ids.push(task.id.clone());
            }
        }
        if stale
            || matches!(task.status, TaskStatus::Unknown | TaskStatus::Disconnected)
            || task.status == TaskStatus::WaitingInput
                && !action
                && task.waiting_reason != Some(WaitingReason::TurnEnded)
        {
            view.counts.uncertain += 1;
        } else if ongoing {
            view.counts.ongoing += 1;
            view.counts.action += usize::from(action);
        } else if result_to_review(task) {
            view.counts.results += 1;
            view.counts.failed += usize::from(task.status == TaskStatus::Failed);
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::SourceKind;
    use crate::providers::{task_observation_state, TaskObservation};
    use std::time::{Duration, UNIX_EPOCH};

    fn settings() -> DashboardSettings {
        let mut settings = crate::monitoring::from_legacy(&Default::default(), true);
        settings.onboarding_complete = true;
        settings.sources.retain(|s| s.kind == SourceKind::Codex);
        settings.sources[0].enabled = true;
        settings
    }

    fn task(id: &str, now: SystemTime) -> TaskSnapshot {
        let mut task = TaskSnapshot::new(
            id,
            "public fixture",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Exact,
            now,
        );
        task.source_instance = Some("codex".into());
        task.turn_key = Some("turn".into());
        task
    }

    fn provider(now: SystemTime, tasks: &[TaskSnapshot]) -> ProviderHealth {
        ProviderHealth {
            provider: "codex".into(),
            status: ProviderStatus::Degraded,
            last_attempt: Some(now),
            last_success: None,
            message: None,
            task_observations: tasks
                .iter()
                .map(|task| {
                    (
                        task.id.clone(),
                        TaskObservation {
                            observed_at: epoch_ms(now),
                            state: task_observation_state(task),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn partial_codex_keeps_only_exact_fresh_tasks_and_blockers() {
        let now = UNIX_EPOCH + Duration::from_secs(100);
        let running = task("running", now);
        let mut blocker = task("blocker", now);
        blocker.status = TaskStatus::WaitingInput;
        blocker.waiting_reason = Some(WaitingReason::Approval);
        blocker.input_request_id = Some("request-1".into());
        let mut empty = blocker.clone();
        empty.id = "empty".into();
        empty.input_request_id = Some(" ".into());
        let mut rewind = task("rewind", now);
        let mut result = task("result", now);
        result.status = TaskStatus::Failed;
        result.unread = true;
        let mut proof = provider(
            now,
            &[
                running.clone(),
                blocker.clone(),
                rewind.clone(),
                empty.clone(),
            ],
        );
        rewind.turn_key = Some("newer-turn".into());
        let tasks = [running, blocker, rewind, empty, result];
        let settings = settings();
        let observed = HashMap::from([("codex".into(), (true, 8))]);
        let view = project(&tasks, &[proof.clone()], &settings, &observed, now);
        assert_eq!(
            view.counts,
            Counts {
                ongoing: 2,
                action: 1,
                results: 1,
                failed: 1,
                uncertain: 2
            }
        );
        assert_eq!(view.unconfirmed_task_ids, ["rewind", "empty"]);
        assert_eq!(view.sources[0].read_state, ReadState::Partial);
        assert_eq!(
            project(&[], &[proof.clone()], &settings, &observed, now).sources[0].read_state,
            ReadState::Partial
        );
        let healthy = ProviderHealth {
            status: ProviderStatus::Connected,
            last_success: Some(now),
            ..proof.clone()
        };
        assert_eq!(
            project(&tasks, &[healthy], &settings, &observed, now)
                .counts
                .ongoing,
            2
        );
        assert_eq!(view.sources[0].discovered_sessions, 8);
        assert_eq!(view.sources[0].current_tasks, 2);
        let expired = project(
            &tasks,
            &[proof.clone()],
            &settings,
            &observed,
            now + Duration::from_millis(30_001),
        );
        assert_eq!(expired.counts.ongoing, 0);
        assert_eq!(expired.counts.action, 0);
        assert_eq!(expired.sources[0].read_state, ReadState::Failed);
        assert_eq!(expired.sources[0].data_detected, Some(true));
        proof.last_attempt = Some(now + Duration::from_millis(1));
        assert_eq!(
            project(&tasks, &[proof], &settings, &observed, now)
                .counts
                .ongoing,
            0
        );
    }

    #[test]
    fn enabled_empty_waiting_and_failed_sources_do_not_imply_data_or_current_tasks() {
        let now = UNIX_EPOCH + Duration::from_secs(100);
        let mut settings = settings();
        for (id, kind) in [
            ("ssh", SourceKind::RemoteCodex),
            ("wsl", SourceKind::CodexWsl),
            ("events", SourceKind::GenericEvent),
        ] {
            let mut source = settings.sources[0].clone();
            source.id = id.into();
            source.kind = kind;
            settings.sources.push(source);
        }
        let mut empty = provider(now, &[]);
        empty.provider = "events".into();
        empty.status = ProviderStatus::Connected;
        empty.last_success = Some(now);
        let mut failed = provider(now, &[]);
        failed.provider = "wsl".into();
        let view = project(
            &[],
            &[empty.clone(), failed],
            &settings,
            &Default::default(),
            now,
        );
        let source = |id| view.sources.iter().find(|s| s.id == id).unwrap();
        assert_eq!(source("ssh").read_state, ReadState::Waiting);
        assert_eq!(source("ssh").data_detected, None);
        assert_eq!(source("wsl").read_state, ReadState::Failed);
        assert_eq!(source("wsl").data_detected, None);
        assert_eq!(source("events").read_state, ReadState::Healthy);
        assert_eq!(source("events").data_detected, Some(false));
        assert_eq!(view.counts, Counts::default());
        settings.onboarding_complete = false;
        let disabled = project(&[], &[empty], &settings, &Default::default(), now);
        assert!(disabled
            .sources
            .iter()
            .all(|s| !s.enabled && s.read_state == ReadState::Disabled));
    }

    #[test]
    fn current_counts_exclude_internal_and_unverified_requests_even_on_healthy_sources() {
        let now = UNIX_EPOCH + Duration::from_secs(100);
        let mut settings = settings();
        settings.sources[0].kind = SourceKind::GenericEvent;
        let mut connected = provider(now, &[]);
        connected.status = ProviderStatus::Connected;
        connected.last_success = Some(now);
        let mut active = task("active", now);
        active.source = TaskSource::GenericEvent;
        active.status = TaskStatus::WaitingInput;
        active.waiting_reason = Some(WaitingReason::Question);
        active.integration_epoch = Some("epoch".into());
        active.integration_sequence = 1;
        active.evidence_channel = Some("self_reported".into());
        active.confidence = DetectionConfidence::Inferred;
        active.input_request_id = Some("request".into());
        active.pending_confirmed_at = Some(epoch_ms(now));
        let mut expired = active.clone();
        expired.id = "expired".into();
        expired.pending_confirmed_at = Some(epoch_ms(now) - 30_001);
        let mut guessed = active.clone();
        guessed.id = "guessed".into();
        guessed.evidence_channel = None;
        let mut internal = task("internal", now);
        internal.internal = true;
        let view = project(
            &[active, expired, guessed, internal],
            &[connected],
            &settings,
            &Default::default(),
            now,
        );
        assert_eq!(
            view.counts,
            Counts {
                ongoing: 1,
                action: 1,
                results: 0,
                failed: 0,
                uncertain: 2
            }
        );
        assert_eq!(view.unconfirmed_task_ids, ["expired", "guessed"]);
    }
}
