//! `tp-run` run-file parsing (exact mode).
//!
//! `tp-run` (scripts/tp-run) writes `<run_id>.json` to
//! `~/.local/state/taskpulse/runs/` on the remote host.  Each file carries the
//! real command exit code, so a run file is the ONLY source of EXACT
//! SUCCEEDED/FAILED for remote tasks. Active records have no age cutoff;
//! terminal discovery retains a 3 h window, separate from unread UI retention.
//!
//! Identity is host + run_id; pane is only a locator. A verified wrapper
//! suppresses inference for its own descendants, not another pane workload.
//!
//! Staleness guard: a run file that still says "running" is only honored if
//! its PID/start-ticks/boot identity matches and heartbeat is fresh. A killed
//! wrapper stays visible as stale, never fabricates success or disappears.

use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};

use super::parser::{PaneInfo, ProcInfo};

/// Marker separating the runs section in the snapshot output.
#[cfg(test)]
pub const RUNS_MARKER: &str = "__TP_RUNS__";
#[cfg(test)]
pub const CODEX_MARKER: &str = "__TP_CODEX__";

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)] // schema fields are read selectively
pub struct RunFile {
    #[serde(default)]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub pane_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub pid_start_ticks: Option<String>,
    #[serde(default)]
    pub observed_pid_start_ticks: Option<String>,
    #[serde(default)]
    pub boot_id: Option<String>,
    #[serde(default)]
    pub observed_boot_id: Option<String>,
    #[serde(default)]
    pub heartbeat_at: Option<i64>,
    #[serde(default)]
    pub execution_activity_at: Option<i64>,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub heartbeat_at_ms: Option<u64>,
    #[serde(default)]
    pub execution_activity_at_ms: Option<u64>,
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub finished_at: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
}

/// Split the combined snapshot output into its three sections.
///
/// The markers are echoed by the snapshot command, but the SAME tokens also
/// appear in the `ps` output (the command's own shell shows its full command
/// line in `args`).  So while the panes section ends at the FIRST ps marker,
/// the runs section must start at the LAST runs marker — the real `echo`
/// precedes the `find`/`cat` and is the final occurrence.
#[cfg(test)]
pub fn split_snapshot_sections(output: &str) -> (String, String, String, String) {
    let (head, rest) = match output.split_once(super::parser::PS_MARKER) {
        Some((panes, rest)) => (panes.to_string(), rest.to_string()),
        None => (output.to_string(), String::new()),
    };
    let (procs, runs) = match rest.rsplit_once(RUNS_MARKER) {
        Some((procs, runs)) => (procs.to_string(), runs.to_string()),
        None => (rest, String::new()),
    };
    // The Codex section's real marker is the LAST one (re-emitted after ps,
    // whose args echo the marker text too) — split from the right.
    let (runs, codex) = match runs.rsplit_once(CODEX_MARKER) {
        Some((runs, codex)) => (runs.to_string(), codex.to_string()),
        None => (runs, String::new()),
    };
    (head, procs, runs, codex)
}

/// Parse concatenated run-file JSON documents.
#[cfg(test)]
pub fn parse_run_files(output: &str) -> Vec<RunFile> {
    parse_run_files_checked(output).unwrap_or_default()
}

pub fn parse_run_files_checked(output: &str) -> Result<Vec<RunFile>, String> {
    serde_json::Deserializer::from_str(output)
        .into_iter::<RunFile>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid run JSON".into())
}

fn epoch_to_time(secs: i64) -> Option<SystemTime> {
    if secs <= 0 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64))
}

fn epoch_ms(ms: Option<u64>) -> Option<SystemTime> {
    ms.filter(|ms| *ms > 0)
        .and_then(|ms| UNIX_EPOCH.checked_add(Duration::from_millis(ms)))
}

/// Convert a run file into a TaskSnapshot (EXACT confidence).
///
/// `alive_pids` is the set of live process pids from the ps table.
/// `pane_has_workload` reports whether the bound pane currently shows a
/// workload — used as a secondary liveness signal for "running" run files.
pub fn run_file_to_task(
    host: &str,
    run: &RunFile,
    alive_pids: &HashMap<u32, ProcInfo>,
    _pane_has_workload: bool,
    now: SystemTime,
) -> Option<TaskSnapshot> {
    let run_id = run.run_id.as_deref().filter(|id| !id.trim().is_empty())?;

    let status_str = run.status.as_deref().unwrap_or("running");
    let status = match status_str {
        "running" => TaskStatus::Running,
        "succeeded" if run.exit_code == Some(0) => TaskStatus::Succeeded,
        "failed" | "interrupted"
            if run
                .exit_code
                .is_some_and(|code| code != 0 && i32::try_from(code).is_ok()) =>
        {
            TaskStatus::Failed
        }
        _ => TaskStatus::Unknown,
    };

    // An unrelated pane workload or reused PID must never revive a dead run.
    // Legacy records without identity/heartbeat remain visible as uncertain.
    let identity_alive = run.pid.is_some_and(|pid| alive_pids.contains_key(&pid))
        && run.pid_start_ticks.is_some()
        && run.pid_start_ticks == run.observed_pid_start_ticks
        && run.boot_id.is_some()
        && run.boot_id == run.observed_boot_id;
    let observed = epoch_ms(run.observed_at_ms).unwrap_or(now);
    let heartbeat_fresh = epoch_ms(run.heartbeat_at_ms)
        .or_else(|| run.heartbeat_at.and_then(epoch_to_time))
        .is_some_and(|at| {
            observed.duration_since(at).unwrap_or_default() <= Duration::from_secs(30)
        });

    let name = run
        .label
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| {
            run.command
                .clone()
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "tp-run".to_string())
        });

    let project = run
        .cwd
        .as_deref()
        .and_then(|c| std::path::Path::new(c).file_name())
        .map(|n| n.to_string_lossy().into_owned());

    let exit_code = run.exit_code.and_then(|c| i32::try_from(c).ok());

    // A pane is a locator, never the identity of successive command runs.
    let id = format!("ssh:{host}:run:{run_id}");

    let mut task = TaskSnapshot::new(
        id,
        name,
        TaskSource::RemoteTmux,
        status,
        DetectionConfidence::Exact,
        now,
    )
    .with_remote()
    .with_host(host)
    .with_project(project.unwrap_or_default())
    .with_message(run.command.clone().unwrap_or_default())
    .with_started_at(
        epoch_ms(run.started_at_ms)
            .or_else(|| run.started_at.and_then(epoch_to_time))
            .unwrap_or(now),
    );

    task.stale = status == TaskStatus::Unknown
        || status == TaskStatus::Running && (!identity_alive || !heartbeat_fresh);
    task.updated_at = epoch_ms(
        run.execution_activity_at_ms
            .or(run.finished_at_ms)
            .or(run.started_at_ms),
    )
    .or_else(|| {
        run.execution_activity_at
            .or(run.finished_at)
            .or(run.started_at)
            .and_then(epoch_to_time)
    })
    .unwrap_or(now);
    if let Some(finished) = epoch_ms(run.finished_at_ms)
        .or_else(|| run.finished_at.and_then(epoch_to_time))
        .filter(|_| status.is_terminal())
    {
        task.finished_at = Some(finished);
    }
    task.project_path = run.cwd.clone();
    task.tmux_pane = run.pane_id.clone();
    task.turn_key = run.run_id.clone();
    task.activity = (status == TaskStatus::Running)
        .then(|| run.command.clone())
        .flatten();
    if let Some(pid) = run.pid {
        task.pid = Some(pid);
    }
    if let Some(code) = exit_code {
        task.exit_code = Some(code);
    }
    if status.is_terminal() || task.finished_at.is_some() {
        task.recent = true;
    }
    Some(task)
}

/// Build exact tasks from run files, resolving liveness against the live
/// process table and panes.
pub fn tasks_from_run_files(
    host: &str,
    runs: &[RunFile],
    procs: &HashMap<u32, ProcInfo>,
    panes: &[PaneInfo],
    now: SystemTime,
) -> Vec<TaskSnapshot> {
    let pane_workload: std::collections::HashSet<String> = panes
        .iter()
        .filter(|p| {
            let cmd = p.pane_current_command.trim();
            super::detector::is_workload(cmd, "")
                || super::detector::descendants(p.pane_pid, procs, 6)
                    .iter()
                    .any(|d| super::detector::is_workload(&d.comm, &d.args))
        })
        .map(|p| p.pane_id.clone())
        .collect();

    runs.iter()
        .filter_map(|run| {
            let pane_id = run.pane_id.as_deref().unwrap_or("");
            let has_workload = !pane_id.is_empty() && pane_workload.contains(pane_id);
            let mut task = run_file_to_task(host, run, procs, has_workload, now)?;
            if let Some(pane) = panes.iter().find(|p| p.pane_id == pane_id) {
                task.tmux_session = Some(pane.session_name.clone());
                task.tmux_target = Some(pane.target());
            }
            Some(task)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_runs_use_identity_and_heartbeat_without_renewing_execution_activity() {
        let now = SystemTime::now();
        let ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let mut run: RunFile =
            serde_json::from_value(serde_json::json!({"schema_version":2,"run_id":"long",
            "status":"running","pid":500,"pid_start_ticks":"123","observed_pid_start_ticks":"123",
            "boot_id":"boot","observed_boot_id":"boot","started_at_ms":ms-4*3600*1000,
            "execution_activity_at_ms":ms-4*3600*1000,"heartbeat_at_ms":ms,"observed_at_ms":ms}))
            .unwrap();
        let task = run_file_to_task("host", &run, &alive(500), false, now).unwrap();
        assert!(!task.stale);
        assert_eq!(task.updated_at, task.started_at.unwrap());
        run.observed_pid_start_ticks = Some("different process".into());
        assert!(
            run_file_to_task("host", &run, &alive(500), true, now)
                .unwrap()
                .stale
        );
        run.observed_pid_start_ticks = run.pid_start_ticks.clone();
        run.observed_boot_id = Some("reboot".into());
        assert!(
            run_file_to_task("host", &run, &alive(500), true, now)
                .unwrap()
                .stale
        );
        run.observed_boot_id = run.boot_id.clone();
        run.heartbeat_at_ms = Some(ms - 60_000);
        let missed = run_file_to_task("host", &run, &alive(500), true, now).unwrap();
        assert!(missed.stale && missed.finished_at.is_none());
    }

    #[test]
    fn successive_runs_in_the_same_pane_never_collide_regardless_of_file_order() {
        let a: RunFile = serde_json::from_value(
            serde_json::json!({"run_id":"a","pane_id":"%3","status":"succeeded","exit_code":0}),
        )
        .unwrap();
        let b: RunFile = serde_json::from_value(
            serde_json::json!({"run_id":"b","pane_id":"%3","status":"running","pid":500}),
        )
        .unwrap();
        for runs in [vec![a.clone(), b.clone()], vec![b.clone(), a.clone()]] {
            let tasks = tasks_from_run_files("host", &runs, &alive(500), &[], SystemTime::now());
            let mut store = crate::store::TaskStore::default();
            let merged = store.merge(tasks, SystemTime::now());
            assert_eq!(merged.len(), 2);
            assert_eq!(
                merged
                    .iter()
                    .find(|t| t.id == "ssh:host:run:a")
                    .unwrap()
                    .status,
                TaskStatus::Succeeded
            );
            assert_eq!(
                merged
                    .iter()
                    .find(|t| t.id == "ssh:host:run:b")
                    .unwrap()
                    .status,
                TaskStatus::Running
            );
        }
    }

    fn run_file_json(label: &str, pane: &str, status: &str, exit: i64, started: i64) -> String {
        format!(
            r#"{{"schema_version":1,"run_id":"r1","label":"{label}","hostname":"Server3","pane_id":"{pane}","cwd":"/home/user/proj","command":"python eval.py","pid":500,"started_at":{started},"finished_at":{started},"status":"{status}","exit_code":{exit}}}"#
        )
    }

    fn alive(pid: u32) -> HashMap<u32, ProcInfo> {
        let mut m = HashMap::new();
        m.insert(
            pid,
            ProcInfo {
                pid,
                ppid: 1,
                comm: "python".into(),
                args: "python eval.py".into(),
            },
        );
        m
    }

    #[test]
    fn exit_zero_is_succeeded_exact() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "%3", "succeeded", 0, 1700000000))
                .unwrap();
        let t = run_file_to_task("Server3", &run, &alive(500), true, SystemTime::now()).unwrap();
        assert_eq!(t.status, TaskStatus::Succeeded);
        assert_eq!(t.confidence, DetectionConfidence::Exact);
        assert_eq!(t.exit_code, Some(0));
        assert!(t.recent);
        assert_eq!(t.id, "ssh:Server3:run:r1");
    }

    #[test]
    fn exit_two_is_failed_exact() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "%3", "failed", 2, 1700000000)).unwrap();
        let t = run_file_to_task("Server3", &run, &alive(500), true, SystemTime::now()).unwrap();
        assert_eq!(t.status, TaskStatus::Failed);
        assert_eq!(t.exit_code, Some(2));
    }

    #[test]
    fn running_honored_when_pid_alive() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "%3", "running", 0, 1700000000)).unwrap();
        let t = run_file_to_task("Server3", &run, &alive(500), false, SystemTime::now()).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        assert_eq!(t.confidence, DetectionConfidence::Exact);
    }

    #[test]
    fn dead_run_is_retained_as_uncertain_without_fabricating_a_finish() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "%3", "running", 0, 1700000000)).unwrap();
        let t = run_file_to_task("Server3", &run, &HashMap::new(), false, SystemTime::now());
        let task = t.unwrap();
        assert!(task.stale);
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.finished_at.is_none());
    }

    #[test]
    fn unrelated_pane_workload_does_not_prove_an_old_run_is_alive() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "%3", "running", 0, 1700000000)).unwrap();
        let t = run_file_to_task("Server3", &run, &HashMap::new(), true, SystemTime::now());
        assert!(t.unwrap().stale);
    }

    #[test]
    fn no_pane_id_uses_run_scoped_id() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "", "succeeded", 0, 1700000000)).unwrap();
        let t = run_file_to_task("Server3", &run, &alive(500), true, SystemTime::now()).unwrap();
        assert_eq!(t.id, "ssh:Server3:run:r1");
    }

    #[test]
    fn parses_concatenated_run_files() {
        let body = format!(
            "{}\n{}",
            run_file_json("a", "%3", "succeeded", 0, 1700000000),
            run_file_json("b", "%4", "failed", 1, 1700000001)
        );
        let runs = parse_run_files(&body);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].label.as_deref(), Some("a"));
        assert_eq!(runs[1].exit_code, Some(1));
    }

    #[test]
    fn split_three_sections() {
        let out =
            "pane line\n__TP_PS__\n  1 1 bash bash\n__TP_RUNS__\n{\"status\":\"succeeded\"}\n";
        let (panes, procs, runs, _) = split_snapshot_sections(out);
        assert!(panes.contains("pane line"));
        assert!(procs.contains("bash"));
        assert!(runs.contains("succeeded"));
    }

    #[test]
    fn split_survives_markers_repeated_in_ps_args() {
        // The snapshot command's own shell appears in `ps` with the marker
        // tokens as part of its args; the runs marker is the LAST occurrence.
        let ps_cmd = "bash -c tmux list-panes -a -F '...' 2>&1; echo __TP_PS__; ps -eo pid=,ppid=,comm=,args= --no-headers 2>&1; echo __TP_RUNS__; find ... ; true";
        let out = format!(
            "demo\t0\t0\t%4\t901112\tpts/0\t/home/user\tbash\t\ttitle\n__TP_PS__\n  901112  1 bash -bash\n  915238  915230 bash {ps_cmd}\n__TP_RUNS__\n{{\"schema_version\":1,\"status\":\"succeeded\",\"exit_code\":0,\"pane_id\":\"%4\",\"label\":\"success-demo\"}}\n"
        );
        let (panes, procs, runs, _) = split_snapshot_sections(&out);
        assert!(panes.contains("%4"), "panes: {panes}");
        assert!(procs.contains("915238"), "procs: {procs}");
        assert!(runs.contains("success-demo"), "runs: {runs}");
        let parsed = parse_run_files(&runs);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].label.as_deref(), Some("success-demo"));
    }
}
