//! Parsers for the one-shot remote snapshot:
//! `tmux list-panes -a -F <fmt>` + `ps` process table, emitted by a single
//! SSH roundtrip (see `snapshot_command` in `mod.rs`).

use std::collections::HashMap;

/// Marker separating the tmux section from the ps section.
pub const PS_MARKER: &str = "__TP_PS__";

#[derive(Debug, Clone, PartialEq)]
pub struct PaneInfo {
    pub session_name: String,
    pub window_index: String,
    pub pane_index: String,
    /// tmux pane id, e.g. "%3" — stable identity anchor.
    pub pane_id: String,
    pub pane_pid: u32,
    pub pane_tty: String,
    pub pane_current_path: String,
    pub pane_current_command: String,
    pub pane_title: String,
    pub window_name: String,
}

impl PaneInfo {
    /// `session:window.pane` label used only for display, never for identity.
    pub fn target(&self) -> String {
        format!(
            "{}:{}.{}",
            self.session_name, self.window_index, self.pane_index
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub args: String,
}

/// Split the combined snapshot output into (panes_section, procs_section).
#[cfg_attr(not(test), allow(dead_code))]
pub fn split_snapshot(output: &str) -> (String, String) {
    match output.split_once(PS_MARKER) {
        Some((panes, procs)) => (panes.to_string(), procs.to_string()),
        None => (output.to_string(), String::new()),
    }
}

const PANE_FIELDS: usize = 10;

/// Parse `tmux list-panes -a -F` tab-delimited output.
/// Malformed lines (wrong field count) are skipped, so a weird `pane_title`
/// cannot corrupt the whole snapshot.
pub fn parse_panes(output: &str) -> Vec<PaneInfo> {
    let mut panes = Vec::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != PANE_FIELDS {
            log::debug!(
                "parse_panes: skipping malformed line ({} fields): {line}",
                fields.len()
            );
            continue;
        }
        let Some(pane_pid) = fields[4].trim().parse::<u32>().ok() else {
            continue;
        };
        if fields[3].trim().is_empty() {
            continue;
        }
        panes.push(PaneInfo {
            session_name: fields[0].to_string(),
            window_index: fields[1].to_string(),
            pane_index: fields[2].to_string(),
            pane_id: fields[3].trim().to_string(),
            pane_pid,
            pane_tty: fields[5].to_string(),
            pane_current_path: fields[6].to_string(),
            pane_current_command: fields[7].to_string(),
            pane_title: fields[8].to_string(),
            window_name: fields[9].to_string(),
        });
    }
    panes
}

/// Parse `ps -eo pid=,ppid=,comm=,args= --no-headers` output into a pid map.
pub fn parse_procs(output: &str) -> HashMap<u32, ProcInfo> {
    let mut procs = HashMap::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        let line = line.trim_start(); // ps right-aligns numeric columns
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(pid_s), Some(ppid_s), Some(comm)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let args: String = parts.collect::<Vec<_>>().join(" ");
        let (Ok(pid), Ok(ppid)) = (pid_s.parse::<u32>(), ppid_s.parse::<u32>()) else {
            continue;
        };
        procs.insert(
            pid,
            ProcInfo {
                pid,
                ppid,
                comm: comm.to_string(),
                args: if args.is_empty() {
                    comm.to_string()
                } else {
                    args
                },
            },
        );
    }
    procs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_panes_with_spaces_and_unicode() {
        let out = concat!(
            "hyper mem\t0\t0\t%0\t1234\tpts/0\t/home/ai/项目 HyperMem\tbash\tMy Session\twin0\n",
            "eval\t2\t0\t%3\t5678\tpts/1\t/home/ai/tests\tpython\tEvaluating…\twin2\n",
        );
        let panes = parse_panes(out);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].pane_id, "%0");
        assert_eq!(panes[0].session_name, "hyper mem"); // space in session name
        assert_eq!(panes[0].pane_current_path, "/home/ai/项目 HyperMem"); // unicode
        assert_eq!(panes[1].pane_current_command, "python");
        assert_eq!(panes[1].pane_title, "Evaluating…");
        assert_eq!(panes[1].target(), "eval:2.0");
    }

    #[test]
    fn skips_malformed_lines() {
        let out = "a\tb\tc\n%0\t1234\tbash\t/ok\n";
        let panes = parse_panes(out);
        assert!(panes.is_empty());
    }

    #[test]
    fn parses_proc_table_with_spaces_in_args() {
        let out =
            "  1234  1000 bash bash run_eval.sh\n  5678  1234 python python eval.py --lr 1e-4\n";
        let procs = parse_procs(out);
        assert_eq!(procs.len(), 2);
        assert_eq!(procs[&5678].ppid, 1234);
        assert_eq!(procs[&5678].comm, "python");
        assert_eq!(procs[&5678].args, "python eval.py --lr 1e-4");
    }

    #[test]
    fn split_snapshot_sections() {
        let out = "pane line\n__TP_PS__\n  1 1 bash bash\n";
        let (panes, procs) = split_snapshot(out);
        assert_eq!(panes, "pane line\n");
        assert!(procs.contains("bash"));
    }

    #[test]
    fn missing_marker_means_no_procs() {
        let (panes, procs) = split_snapshot("just panes\n");
        assert_eq!(panes, "just panes\n");
        assert_eq!(procs, "");
    }
}
