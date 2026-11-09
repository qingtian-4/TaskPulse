//! Pure observation deduplication. Terminal focus alone is not session identity.
use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource};

/// Cross-provider dedup: when a hook/file task and a scan task share the same
/// terminal focus_id and CLI, keep the higher-confidence one (hook beats
/// scan) and drop the duplicate.  Mirrors AgentTray's focus_id budget logic
/// for the common case.
#[allow(clippy::ptr_arg)]
pub(super) fn dedup_by_terminal(tasks: &mut Vec<TaskSnapshot>) {
    let mut keep = vec![true; tasks.len()];
    let mut best: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();

    for (i, t) in tasks.iter().enumerate() {
        // A project window can contain many native sessions. Its PID is not
        // session identity and must not deduplicate them by evidence strength.
        if is_scanner_observation(t) || t.session_id.is_some() {
            continue;
        }
        let Some(term) = &t.terminal else { continue };
        if term.focus_id.is_empty() || term.focus_id == "0" {
            continue;
        }
        let cli = t.cli.clone().unwrap_or_default();
        let key = (term.focus_id.clone(), cli);
        match best.get(&key) {
            Some(&prev) => {
                let prev_conf = tasks[prev].confidence;
                let cur_conf = t.confidence;
                if cur_conf < prev_conf {
                    keep[prev] = false;
                    best.insert(key, i);
                } else if cur_conf == prev_conf {
                    // Same confidence: distinct sessions in one window — keep both.
                    continue;
                } else {
                    keep[i] = false;
                }
            }
            None => {
                best.insert(key, i);
            }
        }
    }

    let mut out = Vec::with_capacity(tasks.len());
    for (i, t) in tasks.drain(..).enumerate() {
        if keep[i] {
            out.push(t);
        }
    }
    *tasks = out;
}

/// Merge provider output into the store, run the transition engine, notify,
/// and emit to tray + UI.
/// Session-metadata (Codex) beats process-scan (Codex): if a real Codex
/// session task exists, drop the scanner's inferred scan entry for the same
/// CLI so we never show the same agent twice.
pub(super) fn dedup_codex_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    dedup_scanner_observations(tasks, TaskSource::Codex, Some("codex"));
}

pub(super) fn is_scanner_observation(task: &TaskSnapshot) -> bool {
    task.source_instance.as_deref() == Some("scanner") || task.id.starts_with("scan:")
}

pub(super) fn dedup_hermes_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    // Native Hermes leases also use Inferred confidence. Evidence strength
    // does not identify a scanner row; only remove actual scanner output with
    // a matching stable identity.
    dedup_scanner_observations(tasks, TaskSource::Hermes, Some("hermes"));
}

pub(super) fn dedup_dsh_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    dedup_scanner_observations(tasks, TaskSource::DeepSeekHarness, Some("dsh"));
}

fn dedup_scanner_observations(
    tasks: &mut Vec<TaskSnapshot>,
    source: TaskSource,
    cli: Option<&str>,
) {
    let native: Vec<TaskSnapshot> = tasks
        .iter()
        .filter(|task| task.source == source && !is_scanner_observation(task))
        .cloned()
        .collect();
    if native.is_empty() {
        return;
    }
    tasks.retain(|task| {
        let scanner = is_scanner_observation(task)
            && cli.is_none_or(|name| task.cli.as_deref() == Some(name))
            && task.confidence == DetectionConfidence::Inferred;
        !(scanner
            && native
                .iter()
                .any(|candidate| same_observed_identity(candidate, task)))
    });
}

/// Only merge a scanner row when a stable identity proves it is the same
/// session. Ambiguous rows remain visible instead of deleting an independent
/// scanner observation from another Agent or terminal.
pub(super) fn same_observed_identity(a: &TaskSnapshot, b: &TaskSnapshot) -> bool {
    if a.source != b.source || a.host != b.host || a.remote != b.remote {
        return false;
    }
    // A terminal is a focus target, not proof that two observations describe
    // the same session. Unknown or mismatched session IDs must remain visible.
    matches!((&a.session_id, &b.session_id), (Some(a_id), Some(b_id))
        if !a_id.is_empty() && a_id == b_id)
}

pub(super) fn dedup_claude_over_scan(tasks: &mut Vec<TaskSnapshot>) {
    let native: Vec<TaskSnapshot> = tasks
        .iter()
        .filter(|task| {
            task.source == TaskSource::ClaudeCode
                && !is_scanner_observation(task)
                && (task.id.starts_with("claude:") || task.id.contains(":claude:"))
        })
        .cloned()
        .collect();
    if native.is_empty() {
        return;
    }
    tasks.retain(|task| {
        !(task.source == TaskSource::ClaudeCode
            && is_scanner_observation(task)
            && native
                .iter()
                .any(|candidate| same_observed_identity(candidate, task)))
    });
}
