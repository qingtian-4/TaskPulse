//! Current observations refresh followed results after RECENT expires.
//! Attention owns unread/pin retention; the cache TTL is not extended.
use super::merge_into;
use crate::domain::{TaskSnapshot, WaitingReason};
use std::collections::{HashMap, HashSet};

pub fn refresh_retained_results(
    merged: &mut Vec<TaskSnapshot>,
    observed: Vec<TaskSnapshot>,
    retained: Vec<TaskSnapshot>,
) {
    let mut known: HashMap<_, _> = retained.into_iter().map(|t| (t.id.clone(), t)).collect();
    let mut included: HashSet<_> = merged.iter().map(|t| t.id.clone()).collect();
    for next in observed {
        if next.stale
            || !next.is_agent_session()
            || next.finished_at.is_none()
            || !(next.status.is_terminal() || next.waiting_reason == Some(WaitingReason::TurnEnded))
            || included.contains(&next.id)
        {
            continue;
        }
        let Some(previous) = known.get_mut(&next.id) else {
            continue;
        };
        if next.source != previous.source
            || next.source_instance != previous.source_instance
            || next.session_id != previous.session_id
            || next.updated_at < previous.updated_at
            || next
                .started_at
                .zip(previous.started_at)
                .is_some_and(|(a, b)| a < b)
            || next.turn_key != previous.turn_key
                && next.started_at.zip(previous.started_at).is_none()
        {
            continue;
        }
        if next.replays_integration(previous) {
            previous.merge_integration_health(&next);
        } else {
            merge_into(previous, next);
        }
        previous.new_execution = false;
        included.insert(previous.id.clone());
        merged.push(previous.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        attention::AttentionTracker,
        domain::{DetectionConfidence, TaskSource, TaskStatus, WaitingReason},
        store::TaskStore,
        transition::TransitionEngine,
    };
    use std::time::{Duration, SystemTime};

    #[test]
    fn old_results_refresh_after_restart_without_importing_history_or_renotifying() {
        for source in [
            TaskSource::Codex,
            TaskSource::ClaudeCode,
            TaskSource::DeepSeekHarness,
            TaskSource::NativeAgent,
            TaskSource::Hermes,
            TaskSource::GenericEvent,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let now = SystemTime::now();
            let end = now - Duration::from_secs(3600);
            let mut running = TaskSnapshot::new(
                "source:session",
                "Fixture",
                source,
                TaskStatus::Running,
                DetectionConfidence::Hook,
                end,
            );
            running.source_instance = Some("source".into());
            running.session_id = Some("session".into());
            running.turn_key = Some("turn".into());
            running.started_at = Some(end - Duration::from_secs(61));
            running.pinned = true;
            let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
            tracker.reconcile(vec![running.clone()]);
            let mut old = running.clone();
            old.status = TaskStatus::WaitingInput;
            old.waiting_reason = Some(WaitingReason::TurnEnded);
            tracker.reconcile(vec![old.clone()]);
            drop(tracker);
            let mut tracker = AttentionTracker::open(dir.path().join("attention.json"));
            let retained = tracker.reconcile(vec![old.clone()]);
            let mut observed = old.clone();
            observed.finished_at = Some(end);
            observed.reasoning_effort = Some("high".into());
            observed.model = Some("native-model".into());
            let mut cold = observed.clone();
            cold.id = "unfollowed-history".into();
            let mut store = TaskStore::new(Duration::from_secs(1800));
            let mut merged = store.merge(vec![observed.clone(), cold.clone()], now);
            assert!(
                merged.is_empty() && store.recent().is_empty(),
                "cache TTL still applies"
            );
            refresh_retained_results(&mut merged, vec![observed, cold], retained);
            assert_eq!(merged.len(), 1, "unfollowed idle history stays excluded");
            let tasks = tracker.reconcile(merged);
            assert_eq!(tasks[0].finished_at, Some(end));
            assert_eq!(tasks[0].reasoning_effort.as_deref(), Some("high"));
            assert!(tasks[0].unread && tasks[0].pinned && !tasks[0].stale);
            tracker.mark_viewed("source:session").unwrap();
            let mut refreshed = vec![];
            refresh_retained_results(&mut refreshed, tasks.clone(), tracker.known());
            assert!(
                !tracker.reconcile(refreshed)[0].unread,
                "re-observation cannot undo viewed state"
            );
            let mut engine = TransitionEngine::new();
            engine.apply(&[running], &[], now);
            engine.apply(&[old], &[], now);
            assert!(
                engine.apply(&tasks, &[], now).is_empty(),
                "backfilled metadata cannot notify again"
            );
        }
    }

    #[test]
    fn stale_rewound_and_wrong_source_results_cannot_clear_a_retained_request() {
        let now = SystemTime::now();
        let mut known = TaskSnapshot::new(
            "id",
            "Fixture",
            TaskSource::DeepSeekHarness,
            TaskStatus::WaitingInput,
            DetectionConfidence::Hook,
            now,
        );
        known.source_instance = Some("source".into());
        known.session_id = Some("session".into());
        known.turn_key = Some("turn".into());
        known.started_at = Some(now);
        known.waiting_reason = Some(WaitingReason::Question);
        known.input_request_id = Some("pending".into());
        let mut end = known.clone();
        end.waiting_reason = Some(WaitingReason::TurnEnded);
        end.input_request_id = None;
        end.finished_at = Some(now);
        let mut stale = end.clone();
        stale.stale = true;
        let mut rewind = end.clone();
        rewind.updated_at = now - Duration::from_secs(1);
        let mut foreign = end.clone();
        foreign.source_instance = Some("other".into());
        let mut contradictory = end.clone();
        contradictory.waiting_reason = Some(WaitingReason::Question);
        for invalid in [stale, rewind, foreign, contradictory] {
            let mut merged = vec![];
            refresh_retained_results(&mut merged, vec![invalid], vec![known.clone()]);
            assert!(merged.is_empty());
        }
    }
}
