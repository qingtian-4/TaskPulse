//! `tp-run` run-file parsing (exact mode).
//!
//! `tp-run` (scripts/tp-run) writes `<run_id>.json` to
//! `~/.local/state/taskpulse/runs/` on the remote host.  Each file carries the
//! real command exit code, so a run file is the ONLY source of EXACT
//! SUCCEEDED/FAILED for remote tasks.  The snapshot command cats the recent
//! run files (3 h window) so one SSH roundtrip still returns everything.
//!
//! Merge rule (store): EXACT > HOOK > INFERRED.  A run file for pane `%3`
//! overrides the process-tree inference for the same pane.
//!
//! Staleness guard: a run file that still says "running" is only honored if
//! its recorded pid is alive in the process table (or the pane shows a
//! workload).  A hard-killed wrapper (kill -9) leaves a lying "running" file;
//! we must not pin the task to RUNNING forever on the strength of it.

use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};

use super::parser::{PaneInfo, ProcInfo};

/// Marker separating the runs section in the snapshot output.
pub const RUNS_MARKER: &str = "__TP_RUNS__";

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
pub fn split_snapshot_sections(output: &str) -> (String, String, String) {
    let (head, rest) = match output.split_once(super::parser::PS_MARKER) {
        Some((panes, rest)) => (panes.to_string(), rest.to_string()),
        None => (output.to_string(), String::new()),
    };
    let (procs, runs) = match rest.rsplit_once(RUNS_MARKER) {
        Some((procs, runs)) => (procs.to_string(), runs.to_string()),
        None => (rest, String::new()),
    };
    (head, procs, runs)
}

/// Parse concatenated run-file JSON documents.
pub fn parse_run_files(output: &str) -> Vec<RunFile> {
    let mut runs = Vec::new();
    for chunk in output.split("}\n{") {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let doc = if chunk.starts_with('{') {
            chunk.to_string()
        } else {
            format!("{{{chunk}")
        };
        let doc = if doc.trim_end().ends_with('}') {
            doc
        } else {
            format!("{doc}}}")
        };
        if let Ok(r) = serde_json::from_str::<RunFile>(&doc) {
            runs.push(r);
        }
    }
    runs
}

fn epoch_to_time(secs: i64) -> Option<SystemTime> {
    if secs <= 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
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
    pane_has_workload: bool,
    now: SystemTime,
) -> Option<TaskSnapshot> {
    let run_id = run.run_id.as_deref().unwrap_or("unknown");
    let pane_id = run.pane_id.as_deref().unwrap_or("");

    let status_str = run.status.as_deref().unwrap_or("running");
    let status = match status_str {
        "running" => TaskStatus::Running,
        "succeeded" => TaskStatus::Succeeded,
        "failed" | "interrupted" => TaskStatus::Failed,
        _ => TaskStatus::Unknown,
    };

    // Staleness guard for "running" files.
    if status == TaskStatus::Running {
        let pid_alive = run
            .pid
            .map(|p| alive_pids.contains_key(&p))
            .unwrap_or(false);
        if !pid_alive && !pane_has_workload {
            log::debug!(
                "run {run_id} on {host} says running but pid {:?} is dead and pane idle — ignoring",
                run.pid
            );
            return None;
        }
    }

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

    let exit_code = run.exit_code.map(|c| c as i32);

    // Identity: bind to the pane when tp-run ran inside tmux; otherwise
    // fall back to a run-file-scoped id.
    let id = if pane_id.is_empty() {
        format!("ssh:{host}:run:{run_id}")
    } else {
        format!("ssh:{host}:{pane_id}")
    };

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
    .with_started_at(epoch_to_time(run.started_at.unwrap_or(0)).unwrap_or(now));

    if let Some(finished) = run.finished_at.and_then(epoch_to_time) {
        task.finished_at = Some(finished);
    }
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
            run_file_to_task(host, run, procs, has_workload, now)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(t.id, "ssh:Server3:%3");
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
    fn stale_running_ignored_when_pid_dead_and_pane_idle() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "%3", "running", 0, 1700000000)).unwrap();
        let t = run_file_to_task("Server3", &run, &HashMap::new(), false, SystemTime::now());
        assert!(t.is_none());
    }

    #[test]
    fn running_honored_with_pane_workload_even_if_pid_unknown() {
        let run: RunFile =
            serde_json::from_str(&run_file_json("LoCoMo", "%3", "running", 0, 1700000000)).unwrap();
        let t = run_file_to_task("Server3", &run, &HashMap::new(), true, SystemTime::now());
        assert!(t.is_some());
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
        let (panes, procs, runs) = split_snapshot_sections(out);
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
        let (panes, procs, runs) = split_snapshot_sections(&out);
        assert!(panes.contains("%4"), "panes: {panes}");
        assert!(procs.contains("915238"), "procs: {procs}");
        assert!(runs.contains("success-demo"), "runs: {runs}");
        let parsed = parse_run_files(&runs);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].label.as_deref(), Some("success-demo"));
    }
}
