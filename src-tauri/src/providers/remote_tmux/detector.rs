//! Workload detection for remote panes.
//!
//! The pane structure is `tmux pane → bash → python eval.py`.  `pane_dead`
//! cannot tell us whether the python finished — the pane survives as an idle
//! shell.  So we detect workload by:
//! 1. `pane_current_command` directly naming a workload (python, pytest, …),
//! 2. otherwise walking the process descendants of `pane_pid` (the pane's
//!    foreground shell) for workload processes — this catches
//!    `bash run_eval.sh → python eval.py`.
//!
//! Priority mirrors tmux-agent: process metadata > content > title.
//! We never read pane content (no `capture-pane`) — terminal output may hold
//! tokens and private data.

use std::collections::HashMap;

use super::parser::{PaneInfo, ProcInfo};

/// Shells that sit at a prompt when idle.
pub const SHELLS: &[&str] = &[
    "bash", "sh", "zsh", "fish", "dash", "ksh", "csh", "tcsh", "ash", "mksh",
];

/// Commands that mean "a workload is running" when seen as the pane's
/// foreground process (or as a descendant of the pane shell).
pub const WORKLOADS: &[&str] = &[
    "python",
    "python3",
    "python2",
    "pypy",
    "pytest",
    "uv",
    "uvx",
    "torchrun",
    "accelerate",
    "node",
    "npm",
    "npx",
    "yarn",
    "pnpm",
    "bun",
    "make",
    "cmake",
    "ninja",
    "cargo",
    "go",
    "java",
    "ruby",
    "perl",
    "php",
    "R",
    "julia",
    "matlab",
    "octave",
    "mpirun",
    "sbatch",
    "srun",
    "singularity",
    "docker",
    "pip",
    "pip3",
    "conda",
    "mamba",
    "gcc",
    "g++",
    "clang",
    "rustc",
    "swift",
];

/// Prefixes that also count as workload (python3.11, nodejs, …).
const WORKLOAD_PREFIXES: &[&str] = &[
    "python",
    "pytest",
    "torchrun",
    "accelerate",
    "node",
    "npm",
    "uv",
];

pub fn is_shell(comm: &str) -> bool {
    SHELLS.contains(&comm)
}

pub fn is_workload(comm: &str, args: &str) -> bool {
    let comm = comm.trim();
    if WORKLOADS.contains(&comm) {
        return true;
    }
    if WORKLOAD_PREFIXES.iter().any(|p| comm.starts_with(p)) {
        return true;
    }
    // A shell whose args name a script or workload (bash run_eval.sh,
    // sh -c "python ...", bash train.sh).
    if is_shell(comm) {
        let tokens: Vec<&str> = args.split_whitespace().collect();
        for t in tokens {
            if t.ends_with(".sh") {
                return true;
            }
            if WORKLOADS.contains(&t) || t.starts_with("python") || t.starts_with("node") {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneClass {
    /// A workload process is running in this pane.
    Running,
    /// Shell at a prompt (or a non-workload interactive program).
    Idle,
    /// No usable signal.
    Unknown,
}

/// The workload process that makes a pane "running" — a workload descendant
/// of the pane shell, or the pane's own foreground process when it is one.
/// Note: tmux 3.0a `pane_pid` stays the SHELL pid even while a workload runs,
/// so we must walk descendants rather than trust `pane_pid` directly.
pub fn workload_process<'a>(
    pane: &PaneInfo,
    procs: &'a HashMap<u32, ProcInfo>,
) -> Option<&'a ProcInfo> {
    let pane_is_workload = is_workload(&pane.pane_current_command, "");
    if is_shell(&pane.pane_current_command) || pane_is_workload {
        for p in descendants(pane.pane_pid, procs, MAX_DEPTH) {
            if is_workload(&p.comm, &p.args) {
                return Some(p);
            }
        }
    }
    if pane_is_workload {
        procs
            .get(&pane.pane_pid)
            .filter(|p| is_workload(&p.comm, &p.args))
    } else {
        None
    }
}

const MAX_DEPTH: usize = 6;

/// BFS descendants of `pid` via the ppid map, depth-limited.
pub fn descendants(pid: u32, procs: &HashMap<u32, ProcInfo>, max_depth: usize) -> Vec<&ProcInfo> {
    let mut out = Vec::new();
    let mut frontier: Vec<u32> = vec![pid];
    for _ in 0..max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for f in &frontier {
            for p in procs.values() {
                if p.ppid == *f {
                    out.push(p);
                    next.push(p.pid);
                }
            }
        }
        frontier = next;
    }
    out
}

/// Classify a pane using pane metadata + process tree.
pub fn classify_pane(pane: &PaneInfo, procs: &HashMap<u32, ProcInfo>) -> PaneClass {
    let cmd = pane.pane_current_command.trim();
    if cmd.is_empty() || pane.pane_pid == 0 {
        return PaneClass::Unknown;
    }
    if is_workload(cmd, "") {
        return PaneClass::Running;
    }
    if is_shell(cmd) {
        let found = descendants(pane.pane_pid, procs, MAX_DEPTH)
            .iter()
            .any(|p| is_workload(&p.comm, &p.args));
        return if found {
            PaneClass::Running
        } else {
            PaneClass::Idle
        };
    }
    // Non-shell, non-workload foreground program (vim, git log, htop…):
    // not a monitored task.
    PaneClass::Idle
}

/// Human-readable workload label: `python eval.py` (args), else comm.
pub fn workload_label(pane: &PaneInfo, procs: &HashMap<u32, ProcInfo>) -> String {
    if let Some(p) = workload_process(pane, procs) {
        let args = p.args.trim();
        if args.is_empty() {
            return p.comm.clone();
        }
        // Keep the args readable but bounded.
        let mut s = args.to_string();
        if s.chars().count() > 80 {
            s = s.chars().take(80).collect::<String>() + "…";
        }
        return s;
    }
    if !pane.pane_current_command.is_empty() {
        return pane.pane_current_command.clone();
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(cmd: &str, pid: u32) -> PaneInfo {
        PaneInfo {
            session_name: "s".into(),
            window_index: "0".into(),
            pane_index: "0".into(),
            pane_id: "%0".into(),
            pane_pid: pid,
            pane_tty: "pts/0".into(),
            pane_current_path: "/home/user/proj".into(),
            pane_current_command: cmd.into(),
            pane_title: String::new(),
            window_name: String::new(),
        }
    }

    fn procs(entries: &[(u32, u32, &str, &str)]) -> HashMap<u32, ProcInfo> {
        entries
            .iter()
            .map(|(pid, ppid, comm, args)| {
                (
                    *pid,
                    ProcInfo {
                        pid: *pid,
                        ppid: *ppid,
                        comm: comm.to_string(),
                        args: args.to_string(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn idle_shell_is_idle() {
        let p = pane("bash", 100);
        let ps = procs(&[(100, 1, "bash", "bash")]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Idle);
    }

    #[test]
    fn shell_to_python_is_running() {
        // bash → python eval.py
        let p = pane("bash", 100);
        let ps = procs(&[
            (100, 1, "bash", "bash"),
            (200, 100, "python", "python eval.py"),
        ]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Running);
    }

    #[test]
    fn python_to_shell_back_to_idle() {
        let p = pane("bash", 100);
        // python gone → only bash remains
        let ps = procs(&[(100, 1, "bash", "bash")]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Idle);
    }

    #[test]
    fn bash_script_with_python_child() {
        // bash run_eval.sh → python eval.py
        let p = pane("bash", 100);
        let ps = procs(&[
            (100, 1, "bash", "bash run_eval.sh"),
            (150, 100, "bash", "bash"),
            (200, 150, "python", "python eval.py"),
        ]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Running);
    }

    #[test]
    fn direct_python_foreground() {
        let p = pane("python", 300);
        let ps = procs(&[(300, 100, "python", "python -c time.sleep(30)")]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Running);
    }

    #[test]
    fn python_with_version_suffix() {
        let p = pane("python3.11", 300);
        let ps = procs(&[(300, 100, "python3.11", "python3.11 train.py")]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Running);
    }

    #[test]
    fn vim_is_not_a_workload() {
        let p = pane("vim", 400);
        let ps = procs(&[(400, 100, "vim", "vim main.rs")]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Idle);
    }

    #[test]
    fn unknown_when_no_command() {
        let p = pane("", 0);
        let ps = HashMap::new();
        assert_eq!(classify_pane(&p, &ps), PaneClass::Unknown);
    }

    #[test]
    fn workload_label_uses_args() {
        let p = pane("bash", 100);
        let ps = procs(&[
            (100, 1, "bash", "bash"),
            (200, 100, "python", "python stage4.py"),
        ]);
        assert_eq!(workload_label(&p, &ps), "python stage4.py");
    }

    #[test]
    fn foreground_workload_with_shell_pane_pid() {
        // tmux 3.0a keeps pane_pid = shell pid even while python runs in the
        // foreground.  Detection and labeling must use the descendant.
        let p = pane("python", 901112);
        let ps = procs(&[
            (901112, 108442, "bash", "-bash"),
            (
                903955,
                901112,
                "python",
                "python -c import time; time.sleep(60)",
            ),
        ]);
        assert_eq!(classify_pane(&p, &ps), PaneClass::Running);
        assert_eq!(workload_process(&p, &ps).map(|x| x.pid), Some(903955));
        assert_eq!(
            workload_label(&p, &ps),
            "python -c import time; time.sleep(60)"
        );
    }

    #[test]
    fn descendants_depth_limited() {
        // chain of 10 shells — depth cap prevents runaway
        let mut entries = Vec::new();
        for i in 0..10 {
            entries.push((1000 + i, if i == 0 { 1 } else { 999 + i }, "bash", "bash"));
        }
        let ps = procs(&entries);
        let d = descendants(1000, &ps, 6);
        assert!(d.len() <= 6);
    }
}
