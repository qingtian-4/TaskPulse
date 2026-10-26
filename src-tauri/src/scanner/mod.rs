use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

/// Number of CPU samples to average for state smoothing (~8 s at the 2-second scan interval).
const CPU_SMOOTHING_WINDOW: usize = 4;
/// CPU% threshold for marking a process as recently active (used for `last_active` tracking).
const CPU_ACTIVE_THRESHOLD: f64 = 2.0;

use crate::watcher::{AgentStatus, TerminalInfo};

pub mod strategies;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct ProcessEntry {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub command_line: String,
    pub process_instance: Option<String>,
    pub user_mode_time: u64,
    pub kernel_mode_time: u64,
}

pub(crate) struct ProcessSnapshot {
    pub ppid_map: HashMap<u32, u32>,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub processes: HashMap<u32, ProcessEntry>,
    pub collected_at: Instant,
    pub succeeded: bool,
}

pub(crate) struct PlatformScan<'a> {
    pub matched: Vec<(ProcInfo, &'a dyn strategies::CliStrategy)>,
    pub snapshot: ProcessSnapshot,
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use self::linux as platform;
#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(target_os = "windows")]
use self::windows as platform;

struct CpuSnapshot {
    total_ticks: u64,
    when: Instant,
    /// Last time this process had significant CPU activity.
    last_active: Option<Instant>,
    /// Rolling window of recent raw CPU% samples for smoothing.
    cpu_history: VecDeque<f64>,
}

/// Per-terminal-PID cache for window IDs (shared across platforms).
type WindowCache = HashMap<u32, Option<String>>;

/// Scans for live CLI agent processes across platforms.
pub struct Scanner {
    prev: HashMap<u32, CpuSnapshot>,
    window_cache: WindowCache,
    /// Per-PID cache for session IDs read from process environment.
    /// Session IDs are stable for a process's lifetime; caching avoids
    /// re-reading on every scan (important on Windows where env reading
    /// spawns a PowerShell process).
    session_id_cache: HashMap<u32, Option<String>>,
    /// Per-PID cache for multiplexer/CLI-terminal focus info detected from
    /// process environment variables (KITTY_WINDOW_ID, WEZTERM_PANE, etc.).
    /// Cached because env vars are stable for a process's lifetime and
    /// reading them can be expensive (macOS spawns a `ps` process).
    multiplexer_cache: HashMap<u32, Option<(String, String, String)>>,
    /// Opaque lifetime tokens prevent PID reuse from inheriting cached state.
    process_lifetimes: HashMap<u32, Option<String>>,
    strategies: Vec<Box<dyn strategies::CliStrategy>>,
}

pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    /// Platform-provided process instance identity when available. This is
    /// intentionally opaque to the common scanner (for example, Windows
    /// CIM creation time), so PID reuse cannot silently reuse a task ID.
    pub process_instance: Option<String>,
    pub cwd: PathBuf,
    pub tty_label: String,
    pub utime: u64,
    pub stime: u64,
    /// Pre-computed CPU% (macOS `ps` gives this directly).
    pub instant_cpu: Option<f64>,
    /// Terminal window title (populated by platform terminal_info).
    pub window_title: Option<String>,
    /// Last time this process had significant CPU activity (carried from snapshot).
    pub last_active: Option<Instant>,
}

impl ProcInfo {
    /// Format the working directory as a home-relative path (e.g. `~/project`)
    /// when possible, falling back to the absolute path.
    pub fn cwd_display(&self) -> String {
        if let Some(home) = std::env::var_os("HOME") {
            if let Ok(rel) = self.cwd.strip_prefix(&home) {
                return format!("~/{}", rel.display());
            }
        }
        self.cwd.display().to_string()
    }
}

impl Scanner {
    pub fn new() -> Self {
        Self {
            prev: HashMap::new(),
            window_cache: HashMap::new(),
            session_id_cache: HashMap::new(),
            multiplexer_cache: HashMap::new(),
            process_lifetimes: HashMap::new(),
            strategies: strategies::all_strategies(),
        }
    }

    /// Scan the local process table. A platform sampling failure is returned
    /// separately from a successful scan with no matching agents so callers
    /// can preserve the last authoritative observation.
    pub fn scan_result(&mut self) -> Result<Vec<AgentStatus>, String> {
        self.scan_with(platform::find_cli_processes)
    }

    fn scan_with(
        &mut self,
        collect: impl for<'a> FnOnce(&'a [Box<dyn strategies::CliStrategy>]) -> PlatformScan<'a>,
    ) -> Result<Vec<AgentStatus>, String> {
        let batch = collect(&self.strategies);
        log::trace!(
            "scanner batch age: {:?}",
            batch.snapshot.collected_at.elapsed()
        );
        if !batch.snapshot.succeeded {
            return Err("local process snapshot unavailable".to_string());
        }
        let procs = batch.matched;
        // Parentage and terminal sharing alone do not establish session identity.
        // Only the exact scoped session/process key below may merge observations.
        self.window_cache.clear();

        let mut agents: Vec<AgentStatus> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // Track only identities proven to represent the same scanner session.
        // A missing tty or terminal must never collapse independent agents.
        let mut seen_ids: HashMap<String, usize> = HashMap::new();

        for (mut p, strategy) in procs {
            p.ppid = batch
                .snapshot
                .ppid_map
                .get(&p.pid)
                .copied()
                .unwrap_or(p.ppid);
            seen.insert(p.pid);
            let token = p.process_instance.clone();
            if !same_process_lifetime(self.process_lifetimes.get(&p.pid), token.as_deref()) {
                self.prev.remove(&p.pid);
                self.session_id_cache.remove(&p.pid);
                self.multiplexer_cache.remove(&p.pid);
            }
            self.process_lifetimes.insert(p.pid, token);

            // Compute raw instantaneous CPU%, then smooth with a rolling window
            // to avoid state flipping on transient spikes or dips.
            let instant_cpu = self.cpu_pct(&p);
            let mut history = self
                .prev
                .get(&p.pid)
                .map(|s| s.cpu_history.clone())
                .unwrap_or_default();
            history.push_back(instant_cpu);
            if history.len() > CPU_SMOOTHING_WINDOW {
                history.pop_front();
            }
            let cpu_pct = history.iter().copied().sum::<f64>() / history.len() as f64;
            let is_active = cpu_pct > CPU_ACTIVE_THRESHOLD;

            // Carry forward last_active from previous snapshot.
            // New processes start with last_active = now so they default
            // to "needs-input" rather than "idle" on first scan.
            let is_new = !self.prev.contains_key(&p.pid);
            let prev_last_active = self.prev.get(&p.pid).and_then(|s| s.last_active);
            let last_active = if is_active || is_new {
                Some(Instant::now())
            } else {
                prev_last_active
            };
            p.last_active = last_active;

            let project = p
                .cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            // Get terminal info and populate window_title on ProcInfo
            let mut terminal = platform::terminal_info(&mut self.window_cache, &p, &batch.snapshot);
            if let Some(ref t) = terminal {
                if p.window_title.is_none() {
                    p.window_title = t.window_title.clone();
                }
            }

            // Override kind/focus_id when the agent is running inside a terminal or
            // multiplexer with a CLI focus API (Kitty, WezTerm, tmux, Zellij, screen).
            // Env vars are stable for a process's lifetime, so results are cached.
            if let Some(ref mut t) = terminal {
                let mux = self
                    .multiplexer_cache
                    .entry(p.pid)
                    .or_insert_with(|| detect_multiplexer_focus(p.pid));
                if let Some((ref kind, ref focus_id, ref outer_id)) = *mux {
                    t.kind = kind.clone();
                    t.focus_id = focus_id.clone();
                    t.outer_id = outer_id.clone();
                }
            }

            // For tmux panes: read the pane title set by the inner process via OSC
            // escape sequences (e.g. Codex sets "Waiting...", "Thinking...", etc.).
            // This overrides any outer terminal title from xdotool, which reflects
            // tmux's own formatted title when set-titles is off — not the process title.
            #[cfg(not(target_os = "windows"))]
            if let Some(ref t) = terminal {
                if t.kind == "tmux" && !t.focus_id.is_empty() {
                    if let Some(pane_title) = read_tmux_pane_title(&t.focus_id) {
                        p.window_title = Some(pane_title);
                    }
                }
            }

            // Count direct child processes, excluding known background services
            // (MCP servers, worker services) that are always present.
            let child_count =
                count_children(p.pid, strategy.excluded_substrings(), &batch.snapshot);

            // Delegate state detection to the strategy
            let detected = strategy.detect_state(&p, cpu_pct, child_count);

            // Build display name from terminal label + tty (not window title, which
            // may contain transient status text like "Claude Code - Waiting for approval").
            let suffix = if let Some(ref t) = terminal {
                if !t.label.is_empty() && !p.tty_label.is_empty() {
                    format!("{} · {}", t.label, p.tty_label)
                } else if !t.label.is_empty() {
                    t.label.clone()
                } else {
                    p.tty_label.clone()
                }
            } else {
                p.tty_label.clone()
            };

            let name = if suffix.is_empty() {
                project.to_string()
            } else {
                format!("{} · {}", project, suffix)
            };

            let can_focus = terminal
                .as_ref()
                .map(|t| !t.focus_id.is_empty())
                .unwrap_or(false);

            // Read session ID from the process environment when the strategy
            // exposes an env var name (e.g. CLAUDE_SESSION_ID for Claude Code).
            // Cached per-PID: session IDs are stable for a process's lifetime,
            // and env reads can be expensive on Windows (spawns PowerShell).
            let session_id = strategy.session_env_var().and_then(|key| {
                self.session_id_cache
                    .entry(p.pid)
                    .or_insert_with(|| platform::read_proc_env(p.pid, key))
                    .clone()
            });
            let id = scanner_identity(
                &p,
                strategy.cli_name(),
                terminal.as_ref(),
                session_id.as_deref(),
            );

            let agent = AgentStatus {
                id: id.clone(),
                name,
                status: detected.status,
                message: detected.message,
                terminal,
                can_focus,
                cpu: Some(cpu_pct),
                source: Some("scan".into()),
                cli: Some(strategy.cli_name().to_string()),
                session_id,
                hook_event: None,
                hook_matcher: None,
                mtime: None,
            };

            // Dedup only by stable session/process identity; keep the higher
            // priority (lower numeric) status for multiple records of one
            // confirmed Agent.
            if let Some(&prev_idx) = seen_ids.get(&id) {
                let prev_prio = crate::watcher::status_priority_num(&agents[prev_idx].status);
                let cur_prio = crate::watcher::status_priority_num(&agent.status);
                if cur_prio < prev_prio {
                    agents[prev_idx] = agent;
                }
            } else {
                seen_ids.insert(id, agents.len());
                agents.push(agent);
            }

            self.prev.insert(
                p.pid,
                CpuSnapshot {
                    total_ticks: p.utime + p.stime,
                    when: Instant::now(),
                    last_active,
                    cpu_history: history,
                },
            );
        }

        self.process_lifetimes.retain(|pid, _| seen.contains(pid));
        self.prev.retain(|pid, _| seen.contains(pid));
        self.session_id_cache.retain(|pid, _| seen.contains(pid));
        self.multiplexer_cache.retain(|pid, _| seen.contains(pid));
        self.window_cache.retain(|pid, _| seen.contains(pid));
        Ok(agents)
    }

    /// Compatibility wrapper for callers that only need a best-effort list.
    /// Provider workers use `scan_result` so a failed sample cannot reconcile
    /// away tasks that were observed by the previous successful sample.
    #[allow(dead_code)]
    pub fn scan(&mut self) -> Vec<AgentStatus> {
        self.scan_result().unwrap_or_default()
    }

    fn cpu_pct(&self, p: &ProcInfo) -> f64 {
        if let Some(instant) = p.instant_cpu {
            return instant;
        }
        let Some(prev) = self.prev.get(&p.pid) else {
            return 0.0;
        };
        let dt = prev.when.elapsed().as_secs_f64();
        if dt < 0.01 {
            return 0.0;
        }
        let ticks_now = p.utime + p.stime;
        let delta = ticks_now.saturating_sub(prev.total_ticks) as f64;
        let clk_tck = clk_tck() as f64;
        (delta / clk_tck / dt) * 100.0
    }
}

fn same_process_lifetime(previous: Option<&Option<String>>, current: Option<&str>) -> bool {
    matches!((previous, current), (Some(Some(old)), Some(new)) if !new.is_empty() && old == new)
}

fn scanner_identity(
    process: &ProcInfo,
    cli: &str,
    _terminal: Option<&TerminalInfo>,
    session_id: Option<&str>,
) -> String {
    use sha2::{Digest, Sha256};
    // This scanner samples the local host. Provider workers namespace their
    // observations separately; no Windows handles or CIM syntax enter this key.
    // Hash structured components to avoid delimiter collisions and path disclosure.
    let components = if let Some(session) = session_id.filter(|s| !s.is_empty()) {
        serde_json::json!(["local", cli, "session", session])
    } else {
        serde_json::json!([
            "local",
            cli,
            "process",
            process.pid,
            process.process_instance.as_deref().unwrap_or("")
        ])
    };
    format!(
        "scan:{cli}:{:x}",
        Sha256::digest(components.to_string().as_bytes())
    )
}

/// Count direct child processes of a given PID, excluding children whose
/// command line contains any of the `excluded` substrings (e.g. "mcp-server",
/// "worker-service"). This prevents persistent background services from
/// inflating the child count and causing false "working" status.
#[cfg(target_os = "linux")]
fn count_children(pid: u32, excluded: &[&str], _snapshot: &ProcessSnapshot) -> u32 {
    let task_dir = format!("/proc/{}/task/{}/children", pid, pid);
    let children_str = match std::fs::read_to_string(&task_dir) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    if excluded.is_empty() {
        return children_str.split_whitespace().count() as u32;
    }

    children_str
        .split_whitespace()
        .filter(|child_pid_str| {
            let Ok(child_pid) = child_pid_str.parse::<u32>() else {
                return true;
            };
            let cmdline_path = format!("/proc/{}/cmdline", child_pid);
            let Ok(cmdline_bytes) = std::fs::read(&cmdline_path) else {
                return true;
            };
            let cmdline = String::from_utf8_lossy(&cmdline_bytes);
            !excluded.iter().any(|exc| cmdline.contains(exc))
        })
        .count() as u32
}

#[cfg(target_os = "macos")]
fn count_children(pid: u32, excluded: &[&str], _snapshot: &ProcessSnapshot) -> u32 {
    let output = match std::process::Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    {
        Ok(o) => o,
        Err(_) => return 0,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let child_pids: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();

    if excluded.is_empty() || child_pids.is_empty() {
        return child_pids.len() as u32;
    }

    // Batch all child PIDs into a single `ps` call instead of one per child.
    let pid_list = child_pids.join(",");
    let ps_out = match std::process::Command::new("ps")
        .args(["-o", "pid=,command=", "-p", &pid_list])
        .output()
    {
        Ok(o) => o,
        Err(_) => return child_pids.len() as u32,
    };
    let ps_text = String::from_utf8_lossy(&ps_out.stdout);

    // ps output: one line per matched PID ("  PID command...")
    // Count lines whose command does NOT match any excluded substring.
    ps_text
        .lines()
        .filter(|line| {
            let line = line.trim();
            if line.is_empty() {
                return false;
            }
            // Skip the PID field; everything after the first whitespace is the command.
            let cmd = line.splitn(2, char::is_whitespace).nth(1).unwrap_or(line);
            !excluded.iter().any(|exc| cmd.contains(exc))
        })
        .count() as u32
}

#[cfg(target_os = "windows")]
fn count_children(pid: u32, excluded: &[&str], snapshot: &ProcessSnapshot) -> u32 {
    platform::count_children(pid, excluded, snapshot)
}

/// Returns the kernel clock ticks per second (CLK_TCK).
/// Reads from /proc on Linux; defaults to 100 on other platforms.
fn clk_tck() -> u64 {
    use std::sync::OnceLock;
    static CLK_TCK: OnceLock<u64> = OnceLock::new();
    *CLK_TCK.get_or_init(|| {
        #[cfg(target_os = "linux")]
        {
            // getconf CLK_TCK is the portable way without libc dependency
            std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
                .unwrap_or(100)
        }
        #[cfg(not(target_os = "linux"))]
        {
            100
        }
    })
}

// ===========================================================================
// Known terminal emulators (cross-platform lookup table).
// To add a new terminal, add one line here — no other changes needed.
// ===========================================================================

fn known_terminal(exe_name: &str) -> Option<&'static str> {
    match exe_name {
        // Cross-platform
        "warp" | "warp-terminal" | "Warp.exe" => Some("Warp"),
        "kitty" => Some("Kitty"),
        "alacritty" => Some("Alacritty"),
        "ghostty" | "Ghostty" => Some("Ghostty"),
        "WezTerm" | "wezterm" | "wezterm-gui" => Some("WezTerm"),
        "hyper" | "Hyper" => Some("Hyper"),
        "tabby" | "Tabby" => Some("Tabby"),
        // IDE terminals (process names)
        "Code" | "code" | "Electron" => Some("Visual Studio Code"),
        // Linux
        "gnome-terminal-server" | "gnome-terminal" => Some("GNOME Terminal"),
        "konsole" => Some("Konsole"),
        "xterm" => Some("XTerm"),
        "tilix" => Some("Tilix"),
        "terminator" => Some("Terminator"),
        "xfce4-terminal" => Some("XFCE Terminal"),
        "mate-terminal" => Some("MATE Terminal"),
        "lxterminal" => Some("LXTerminal"),
        "foot" => Some("Foot"),
        "st" => Some("st"),
        "urxvt" | "urxvtd" => Some("URxvt"),
        // macOS
        "Terminal" => Some("Terminal"),
        "iTerm2" => Some("iTerm2"),
        // Windows
        "WindowsTerminal.exe" | "WindowsTerminal" => Some("Windows Terminal"),
        "ConEmuC.exe" | "ConEmuC64.exe" | "ConEmu.exe" => Some("ConEmu"),
        "cmd.exe" | "cmd" => Some("CMD"),
        "powershell.exe" | "powershell" | "pwsh.exe" | "pwsh" => Some("PowerShell"),
        _ => None,
    }
}

// ===========================================================================
// Multiplexer / CLI-terminal focus detection via process environment variables.
// Checks env vars in priority order; first match wins.
// ===========================================================================

/// Returns `Some((kind, focus_id, outer_id))` if the agent process (identified by
/// `pid`) is running inside a terminal or multiplexer that has a CLI-based focus
/// API. Returns `None` if only the generic platform focuser should be used.
fn detect_multiplexer_focus(pid: u32) -> Option<(String, String, String)> {
    // Kitty: KITTY_WINDOW_ID is the integer window ID for `kitty @ focus-window`.
    if let Some(id) = platform::read_proc_env(pid, "KITTY_WINDOW_ID") {
        if !id.is_empty() {
            return Some(("kitty".into(), id, String::new()));
        }
    }
    // WezTerm: WEZTERM_PANE is the pane ID for `wezterm cli activate-pane`.
    if let Some(id) = platform::read_proc_env(pid, "WEZTERM_PANE") {
        if !id.is_empty() {
            return Some(("wezterm".into(), id, String::new()));
        }
    }
    // tmux: TMUX_PANE is the pane reference (e.g. "%3"); tmux select-pane accepts this.
    if let Some(id) = platform::read_proc_env(pid, "TMUX_PANE") {
        if !id.is_empty() {
            return Some(("tmux".into(), id, String::new()));
        }
    }
    // Zellij: ZELLIJ_SESSION_NAME is the session name for `zellij attach`.
    if let Some(id) = platform::read_proc_env(pid, "ZELLIJ_SESSION_NAME") {
        if !id.is_empty() {
            return Some(("zellij".into(), id, String::new()));
        }
    }
    // GNU Screen: STY is the session identifier; WINDOW is the window number.
    if let Some(sty) = platform::read_proc_env(pid, "STY") {
        if !sty.is_empty() {
            let win = platform::read_proc_env(pid, "WINDOW").unwrap_or_default();
            return Some(("screen".into(), sty, win));
        }
    }
    None
}

/// Read the title of a tmux pane as set by the inner process via OSC escape
/// sequences. Returns None if tmux is unavailable or the pane has no title.
#[cfg(not(target_os = "windows"))]
fn read_tmux_pane_title(pane_id: &str) -> Option<String> {
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-t", pane_id, "-p", "#{pane_title}"])
        .output()
        .ok()?;
    let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_terminal_matches() {
        assert_eq!(known_terminal("kitty"), Some("Kitty"));
        assert_eq!(known_terminal("warp"), Some("Warp"));
        assert_eq!(known_terminal("iTerm2"), Some("iTerm2"));
        assert_eq!(known_terminal("bash"), None);
    }

    #[test]
    fn scanner_returns_vec() {
        let mut s = Scanner::new();
        let _agents = s.scan();
    }

    fn identity_process(pid: u32, instance: &str) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: 1,
            process_instance: Some(instance.into()),
            cwd: PathBuf::from(r"C:\project"),
            tty_label: String::new(),
            utime: 0,
            stime: 0,
            instant_cpu: Some(0.0),
            window_title: None,
            last_active: None,
        }
    }

    fn identity_terminal(focus_id: &str) -> TerminalInfo {
        TerminalInfo {
            kind: "windows_native".into(),
            focus_id: focus_id.into(),
            outer_id: String::new(),
            label: "Windows Terminal".into(),
            window_title: None,
        }
    }

    #[test]
    fn scanner_identity_is_scoped_and_does_not_collapse_ambiguous_processes() {
        let first = identity_process(10, "start-a");
        let second = identity_process(11, "start-b");
        assert_ne!(
            scanner_identity(&first, "codex", Some(&identity_terminal("term-a")), None),
            scanner_identity(&second, "codex", Some(&identity_terminal("term-b")), None)
        );

        let mut session_a = identity_process(12, "start-c");
        session_a.process_instance = None;
        let mut session_b = identity_process(13, "start-d");
        session_b.process_instance = None;
        let terminal = identity_terminal("term-shared");
        assert_ne!(
            scanner_identity(&session_a, "codex", Some(&terminal), Some("session-a")),
            scanner_identity(&session_b, "codex", Some(&terminal), Some("session-b"))
        );

        assert_eq!(
            scanner_identity(&session_a, "codex", Some(&terminal), Some("session-a")),
            scanner_identity(&session_b, "codex", Some(&terminal), Some("session-a"))
        );
        assert_ne!(
            scanner_identity(&session_a, "codex", None, None),
            scanner_identity(&session_a, "claude-code", None, None)
        );

        let mut reused = identity_process(10, "start-new");
        reused.process_instance = Some("start-new".into());
        assert_ne!(
            scanner_identity(&first, "codex", None, None),
            scanner_identity(&reused, "codex", None, None)
        );
    }

    #[test]
    fn independent_processes_share_neither_terminal_nor_birth_identity() {
        let mut first = identity_process(10, "same-start");
        let mut second = identity_process(11, "same-start");
        assert_ne!(
            scanner_identity(&first, "codex", None, None),
            scanner_identity(&second, "codex", None, None)
        );
        first.process_instance = None;
        second.process_instance = None;
        let terminal = identity_terminal("same-terminal");
        assert_ne!(
            scanner_identity(&first, "codex", Some(&terminal), None),
            scanner_identity(&second, "codex", Some(&terminal), None)
        );
    }

    #[test]
    fn process_cache_requires_matching_lifetime_not_just_pid() {
        let old = Some("old-start".to_string());
        assert!(same_process_lifetime(Some(&old), Some("old-start")));
        assert!(!same_process_lifetime(Some(&old), Some("new-start")));
        assert!(!same_process_lifetime(Some(&None), None));
        assert!(!same_process_lifetime(None, Some("old-start")));
    }

    #[test]
    fn identity_survives_scanner_restart_and_presentation_changes() {
        let mut process = identity_process(10, "start");
        let before = scanner_identity(&process, "codex", None, Some("session"));
        process.window_title = Some("renamed title".into());
        process.tty_label = "another label".into();
        process.pid = 99;
        assert_eq!(
            before,
            scanner_identity(&process, "codex", None, Some("session"))
        );
        assert_ne!(
            before,
            scanner_identity(&process, "claude-code", None, Some("session"))
        );
    }

    #[test]
    fn batch_identity_retains_independent_nested_sessions_and_survives_restart() {
        fn collect(strategies: &[Box<dyn strategies::CliStrategy>]) -> PlatformScan<'_> {
            let codex = strategies.iter().find(|s| s.cli_name() == "codex").unwrap();
            let mut child = identity_process(11, "same-start");
            child.ppid = 10;
            PlatformScan {
                matched: vec![
                    (identity_process(10, "same-start"), codex.as_ref()),
                    (child, codex.as_ref()),
                    (identity_process(12, "same-start"), codex.as_ref()),
                ],
                snapshot: ProcessSnapshot {
                    ppid_map: [(10, 1), (11, 10), (12, 1)].into(),
                    processes: HashMap::new(),
                    succeeded: true,
                    collected_at: Instant::now(),
                },
            }
        }
        let first = Scanner::new().scan_with(collect).unwrap();
        let second = Scanner::new().scan_with(collect).unwrap();
        assert_eq!(first.len(), 3);
        let ids: std::collections::HashSet<_> = first.iter().map(|a| &a.id).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(
            first.iter().map(|a| &a.id).collect::<Vec<_>>(),
            second.iter().map(|a| &a.id).collect::<Vec<_>>()
        );
        for agent in first {
            let task: crate::domain::TaskSnapshot = agent.into();
            assert_eq!(
                task.confidence,
                crate::domain::DetectionConfidence::Inferred
            );
            assert!(task.id.starts_with("scan:"));
        }
    }

    #[test]
    fn confirmed_session_records_merge_but_pid_reuse_drops_cached_session() {
        fn collect(strategies: &[Box<dyn strategies::CliStrategy>]) -> PlatformScan<'_> {
            let strategy = strategies
                .iter()
                .find(|s| s.cli_name() == "claude-code")
                .unwrap();
            PlatformScan {
                matched: vec![
                    (identity_process(10, "birth"), strategy.as_ref()),
                    (identity_process(11, "birth"), strategy.as_ref()),
                ],
                snapshot: ProcessSnapshot {
                    ppid_map: HashMap::new(),
                    processes: HashMap::new(),
                    succeeded: true,
                    collected_at: Instant::now(),
                },
            }
        }
        let mut scanner = Scanner::new();
        for pid in [10, 11] {
            scanner.process_lifetimes.insert(pid, Some("birth".into()));
            scanner
                .session_id_cache
                .insert(pid, Some("same-session".into()));
        }
        assert_eq!(scanner.scan_with(collect).unwrap().len(), 1);
        // The production path must invalidate cached session/environment state
        // even if no intervening scan observed a reused PID disappear.
        let before = scanner.scan_with(collect).unwrap()[0].id.clone();
        // Use Codex (no session-env lookup) for the reused process, avoiding any
        // query against an unrelated host process at this synthetic PID.
        let reused = scanner
            .scan_with(|strategies| {
                let strategy = strategies.iter().find(|s| s.cli_name() == "codex").unwrap();
                PlatformScan {
                    matched: vec![(identity_process(10, "new-birth"), strategy.as_ref())],
                    snapshot: ProcessSnapshot {
                        ppid_map: HashMap::new(),
                        processes: HashMap::new(),
                        succeeded: true,
                        collected_at: Instant::now(),
                    },
                }
            })
            .unwrap();
        assert_ne!(before, reused[0].id);
        assert!(!scanner.session_id_cache.contains_key(&10));
    }

    #[test]
    fn all_strategies_contains_claude() {
        let strats = strategies::all_strategies();
        assert!(!strats.is_empty());
        assert!(strats[0].process_names().contains(&"claude"));
    }
}
