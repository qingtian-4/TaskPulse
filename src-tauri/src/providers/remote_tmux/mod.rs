//! Remote Tmux Provider — TaskPulse's core new capability.
//!
//! One SSH snapshot per host per poll (`tmux list-panes -a -F` + `ps` in a
//! single roundtrip), executed in parallel across hosts so Server1 can never
//! block Server3.
//!
//! Semantics:
//! * Workload discovered by process metadata + process tree (never
//!   `pane_dead`, never pane content).
//! * `shell → workload`  ⇒  RUNNING / INFERRED (started_at = detected_at).
//! * `workload → shell` (or pane destroyed, observed during a *successful*
//!   poll)  ⇒  IDLE / INFERRED finish — NO exit code, so never SUCCEEDED.
//! * SSH failure preserves the last known tasks (`stale = true`); after
//!   `DISCONNECT_THRESHOLD` consecutive failures the host is DISCONNECTED.
//! * Recovery re-polls and clears the stale/disconnected flags.

pub mod config;
pub mod detector;
pub mod parser;
pub mod runfile;
pub mod ssh;
pub mod state;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus};
use crate::providers::{HostHealth, Provider, ProviderOutput};

use config::{RemoteHostConfig, TaskPulseConfig};
use parser::{PaneInfo, ProcInfo};
use ssh::SshExecutor;
use state::HostPollState;

pub const SSH_TIMEOUT: Duration = Duration::from_secs(5);

/// One remote poll result, produced on a per-host worker thread.
struct HostPollResult {
    host_id: String,
    ssh_host: String,
    outcome: Result<RemoteSnapshot, String>,
}

/// Parsed remote snapshot for one host.
struct RemoteSnapshot {
    panes: Vec<PaneInfo>,
    procs: HashMap<u32, ProcInfo>,
    runs: Vec<runfile::RunFile>,
}

pub struct RemoteTmuxProvider {
    config: TaskPulseConfig,
    executor: Arc<dyn SshExecutor>,
    hosts: HashMap<String, HostPollState>,
    /// First-seen time per pane (used as detected_at when start is unknown).
    detected: HashMap<String, SystemTime>,
}

impl RemoteTmuxProvider {
    pub fn new(config: TaskPulseConfig, executor: Arc<dyn SshExecutor>) -> Self {
        let mut hosts = HashMap::new();
        for h in &config.remote_hosts {
            hosts.insert(h.id.clone(), HostPollState::default());
        }
        Self {
            config,
            executor,
            hosts,
            detected: HashMap::new(),
        }
    }

    pub fn from_config(executor: Arc<dyn SshExecutor>) -> Self {
        Self::new(config::load_config(), executor)
    }

    /// Enabled host configs (used by the watcher to derive poll cadence).
    pub fn config_for_poll(&self) -> Vec<RemoteHostConfig> {
        self.config.enabled_hosts()
    }

    /// The single remote command that returns panes + process table + recent
    /// tp-run run files (one SSH roundtrip per host per poll).
    /// Trailing `; true` guarantees exit 0 even when `find` finds no run dir
    /// (a non-zero exit would otherwise look like an SSH failure).
    pub fn snapshot_command() -> String {
        // REAL tab characters (not "\\t") — tmux -F does not interpret
        // backslash escapes, so the remote output must contain actual tabs.
        let fmt = "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_id}\t#{pane_pid}\t#{pane_tty}\t#{pane_current_path}\t#{pane_current_command}\t#{pane_title}\t#{window_name}";
        format!(
            "tmux list-panes -a -F \"{fmt}\" 2>&1; echo '{}'; ps -eo pid=,ppid=,comm=,args= --no-headers 2>&1; echo '{}'; find \"$HOME/.local/state/taskpulse/runs\" -maxdepth 1 -name '*.json' -mmin -180 -exec cat {{}} \\; 2>/dev/null; true",
            parser::PS_MARKER,
            runfile::RUNS_MARKER
        )
    }

    /// Convert a successful remote snapshot into tasks + host health,
    /// handling finish detection.
    fn process_snapshot(
        &mut self,
        ssh_host: &str,
        host_id: &str,
        panes: &[PaneInfo],
        procs: &HashMap<u32, ProcInfo>,
        runs: &[runfile::RunFile],
        now: SystemTime,
    ) -> ProviderOutput {
        let state = self.hosts.get_mut(host_id).expect("host state exists");

        // Tmux-session whitelist: only monitor the sessions the user asked for
        // (exact name or `*` glob). Empty whitelist = every session.
        let whitelist = self
            .config
            .remote_hosts
            .iter()
            .find(|h| h.id == host_id)
            .map(|h| h.tmux_sessions.clone())
            .unwrap_or_default();
        let panes: Vec<PaneInfo> = if whitelist.is_empty() {
            panes.to_vec()
        } else {
            panes
                .iter()
                .filter(|p| session_allowed(&p.session_name, &whitelist))
                .cloned()
                .collect()
        };

        let mut tasks: Vec<TaskSnapshot> = Vec::new();
        let mut seen_panes: std::collections::HashSet<String> = std::collections::HashSet::new();

        for pane in &panes {
            let class = detector::classify_pane(pane, procs);
            match class {
                detector::PaneClass::Running => {
                    seen_panes.insert(pane.pane_id.clone());
                    state.running_panes.insert(pane.pane_id.clone());
                    let detected_at = *self.detected.entry(pane.pane_id.clone()).or_insert(now);
                    let workload = detector::workload_process(pane, procs);
                    let label = detector::workload_label(pane, procs);
                    let pid = workload.map(|p| p.pid).unwrap_or(pane.pane_pid);
                    let project = std::path::Path::new(&pane.pane_current_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| pane.pane_current_path.clone());
                    let message = if pane.pane_title.trim().is_empty() {
                        pane.target()
                    } else {
                        pane.pane_title.clone()
                    };
                    let task = TaskSnapshot::new(
                        format!("ssh:{ssh_host}:{}", pane.pane_id),
                        label,
                        TaskSource::RemoteTmux,
                        TaskStatus::Running,
                        DetectionConfidence::Inferred,
                        now,
                    )
                    .with_remote()
                    .with_host(ssh_host)
                    .with_project(project)
                    .with_message(message)
                    .with_pid(pid)
                    .with_started_at(detected_at);
                    tasks.push(task);
                }
                detector::PaneClass::Idle => {
                    seen_panes.insert(pane.pane_id.clone());
                    // Was this pane running before?  Then the workload just
                    // finished (inferred — no exit code available).
                    if state.running_panes.remove(&pane.pane_id) {
                        let label = detector::workload_label(pane, procs);
                        let project = std::path::Path::new(&pane.pane_current_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| pane.pane_current_path.clone());
                        let mut task = TaskSnapshot::new(
                            format!("ssh:{ssh_host}:{}", pane.pane_id),
                            label,
                            TaskSource::RemoteTmux,
                            TaskStatus::Idle,
                            DetectionConfidence::Inferred,
                            now,
                        )
                        .with_remote()
                        .with_host(ssh_host)
                        .with_project(project)
                        .with_started_at(self.detected.get(&pane.pane_id).copied().unwrap_or(now))
                        .with_finished_at(now)
                        .with_message("finished (inferred — exit code unknown)");
                        task.recent = true;
                        tasks.push(task);
                        log::info!(
                            "Remote pane {} on {} finished (inferred, no exit code)",
                            pane.pane_id,
                            ssh_host
                        );
                    }
                }
                detector::PaneClass::Unknown => {}
            }
        }

        // Panes we saw running that are no longer in the snapshot at all
        // (window/session destroyed) — inferred finish.
        let gone: Vec<String> = state
            .running_panes
            .iter()
            .filter(|p| !seen_panes.contains(*p))
            .cloned()
            .collect();
        for pane_id in gone {
            state.running_panes.remove(&pane_id);
            let detected_at = self.detected.get(&pane_id).copied();
            let mut task = TaskSnapshot::new(
                format!("ssh:{ssh_host}:{pane_id}"),
                "tmux pane destroyed",
                TaskSource::RemoteTmux,
                TaskStatus::Idle,
                DetectionConfidence::Inferred,
                now,
            )
            .with_remote()
            .with_host(ssh_host)
            .with_started_at(detected_at.unwrap_or(now))
            .with_finished_at(now)
            .with_message("pane gone — finished (inferred)");
            task.recent = true;
            tasks.push(task);
            log::info!(
                "Remote pane {pane_id} on {ssh_host} destroyed while running → inferred finish"
            );
        }

        // Prune detected-at cache for panes that are gone.
        self.detected
            .retain(|p, _| seen_panes.contains(p) || state.running_panes.contains(p));

        // EXACT mode: tp-run run files override inference for the same pane
        // (store merges by id, EXACT > HOOK > INFERRED).
        let exact = runfile::tasks_from_run_files(ssh_host, runs, procs, &panes, now);
        if !exact.is_empty() {
            log::debug!("{}: {} exact tp-run task(s)", ssh_host, exact.len());
        }
        tasks.extend(exact);

        state.last_tasks = tasks.clone();
        ProviderOutput {
            tasks,
            hosts: vec![HostHealth::connected(ssh_host.to_string(), now)],
        }
    }

    /// Merge a failed poll result into host state.  Never emits finishes.
    fn handle_failure(
        &mut self,
        host_id: &str,
        ssh_host: &str,
        err: String,
        now: SystemTime,
        output: &mut ProviderOutput,
    ) {
        let state = self.hosts.entry(host_id.to_string()).or_default();
        let crossed = state.record_failure(now);
        log::warn!(
            "Remote host {host_id} ({ssh_host}) poll failed ({}/{}): {err}",
            state.consecutive_failures,
            state::DISCONNECT_THRESHOLD
        );
        if crossed {
            log::error!("Remote host {host_id} ({ssh_host}) DISCONNECTED");
        }
        // Preserve last known tasks with stale=true; an SSH timeout is not
        // task completion.
        let tasks: Vec<TaskSnapshot> = state
            .last_tasks
            .iter()
            .map(|t| {
                let mut t = t.clone();
                t.stale = true;
                t.updated_at = now;
                t
            })
            .collect();
        output.tasks.extend(tasks);
        if state.is_connected() {
            output.hosts.push(HostHealth::connected(
                ssh_host.to_string(),
                state.last_seen.unwrap_or(now),
            ));
        } else {
            output.hosts.push(HostHealth::disconnected(
                ssh_host.to_string(),
                state.last_seen,
            ));
        }
    }
}

impl Provider for RemoteTmuxProvider {
    fn name(&self) -> &'static str {
        "remote_tmux"
    }

    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let hosts = self.config.enabled_hosts();
        if hosts.is_empty() {
            return ProviderOutput::default();
        }

        // Parallel per-host polling: one thread per host, each doing exactly
        // ONE ssh roundtrip.  A stuck host cannot delay the others.
        let executor = self.executor.clone();
        let results: Vec<HostPollResult> = std::thread::scope(|scope| {
            let handles: Vec<_> = hosts
                .iter()
                .map(|h| {
                    let executor = executor.clone();
                    let ssh_host = h.ssh_host.clone();
                    let id = h.id.clone();
                    scope.spawn(move || {
                        let command = RemoteTmuxProvider::snapshot_command();
                        match executor.run(&ssh_host, &command, SSH_TIMEOUT) {
                            Ok(output) => {
                                let (panes_section, procs_section, runs_section) =
                                    runfile::split_snapshot_sections(&output);
                                HostPollResult {
                                    host_id: id,
                                    ssh_host,
                                    outcome: Ok(RemoteSnapshot {
                                        panes: parser::parse_panes(&panes_section),
                                        procs: parser::parse_procs(&procs_section),
                                        runs: runfile::parse_run_files(&runs_section),
                                    }),
                                }
                            }
                            Err(e) => HostPollResult {
                                host_id: id,
                                ssh_host,
                                outcome: Err(e),
                            },
                        }
                    })
                })
                .collect();

            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| HostPollResult {
                        host_id: String::new(),
                        ssh_host: String::new(),
                        outcome: Err("poll thread panicked".into()),
                    })
                })
                .collect()
        });

        // Merge results back into provider state (single-threaded).
        let mut output = ProviderOutput::default();
        for r in results {
            match r.outcome {
                Ok(snapshot) => {
                    let id = r.host_id.clone();
                    let state = self.hosts.entry(id.clone()).or_default();
                    let just_recovered = state.status == crate::providers::HostStatus::Disconnected;
                    state.record_success(now);
                    if just_recovered {
                        log::info!("Remote host {id} ({}) recovered", r.ssh_host);
                    }
                    let host_out = self.process_snapshot(
                        &r.ssh_host,
                        &id,
                        &snapshot.panes,
                        &snapshot.procs,
                        &snapshot.runs,
                        now,
                    );
                    output.tasks.extend(host_out.tasks);
                    output.hosts.extend(host_out.hosts);
                }
                Err(e) => self.handle_failure(&r.host_id, &r.ssh_host, e, now, &mut output),
            }
        }
        output
    }
}

/// Is `session` in the watch-list? Empty list = allow all. Entries may be an
/// exact tmux session name or a `*` glob (e.g. `train*`, `worker*`).
fn session_allowed(session: &str, whitelist: &[String]) -> bool {
    whitelist.iter().any(|pattern| glob_match(pattern, session))
}

/// Minimal `*` glob: `*` matches any run of characters, everything else is a
/// literal. Full match against the whole string.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // DP match with `*` wildcard only.
    let mut dp = vec![false; t.len() + 1];
    dp[0] = true;
    for &pc in &p {
        if pc == '*' {
            for i in 0..t.len() {
                if dp[i] {
                    dp[i + 1] = true;
                }
            }
        } else {
            for i in (0..t.len()).rev() {
                dp[i + 1] = dp[i] && t[i] == pc;
            }
            dp[0] = false;
        }
    }
    dp[t.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh::FakeSshExecutor;
    use std::time::Instant;

    fn pane_line(session: &str, pane_id: &str, pid: u32, cmd: &str, path: &str) -> String {
        format!("{session}\t0\t0\t{pane_id}\t{pid}\tpts/0\t{path}\t{cmd}\t\ttitle")
    }

    fn ps_line(pid: u32, ppid: u32, comm: &str, args: &str) -> String {
        format!("{pid:>8} {ppid:>8} {comm} {args}")
    }

    fn snapshot_body(panes: &[String], procs: &[String]) -> String {
        snapshot_body_with_runs(panes, procs, "")
    }

    fn snapshot_body_with_runs(panes: &[String], procs: &[String], runs: &str) -> String {
        let mut s = panes.join("\n");
        s.push('\n');
        s.push_str(parser::PS_MARKER);
        s.push('\n');
        s.push_str(&procs.join("\n"));
        s.push('\n');
        s.push_str(runfile::RUNS_MARKER);
        s.push('\n');
        s.push_str(runs);
        s
    }

    fn running_snapshot() -> String {
        snapshot_body(
            &[pane_line("demo", "%3", 500, "python", "/home/user/proj")],
            &[ps_line(500, 400, "python", "python eval.py")],
        )
    }

    fn idle_snapshot() -> String {
        snapshot_body(
            &[pane_line("demo", "%3", 400, "bash", "/home/user/proj")],
            &[ps_line(400, 300, "bash", "bash")],
        )
    }

    fn empty_snapshot() -> String {
        snapshot_body(&[], &[])
    }

    fn config_with_hosts() -> TaskPulseConfig {
        // Two hosts for parallelism tests.
        TaskPulseConfig {
            remote_hosts: vec![
                RemoteHostConfig {
                    id: "server1".into(),
                    ssh_host: "Server1".into(),
                    enabled: true,
                    poll_interval_ms: 3000,
                    tmux_sessions: vec![],
                },
                RemoteHostConfig {
                    id: "server3".into(),
                    ssh_host: "Server3".into(),
                    enabled: true,
                    poll_interval_ms: 3000,
                    tmux_sessions: vec![],
                },
            ],
        }
    }

    fn config_single_host() -> TaskPulseConfig {
        TaskPulseConfig {
            remote_hosts: vec![RemoteHostConfig {
                id: "server3".into(),
                ssh_host: "Server3".into(),
                enabled: true,
                poll_interval_ms: 3000,
                tmux_sessions: vec![],
            }],
        }
    }

    /// Single-host provider (Server3) — most tests only exercise one host.
    fn provider_with(executor: Arc<dyn SshExecutor>) -> RemoteTmuxProvider {
        RemoteTmuxProvider::new(config_single_host(), executor)
    }

    /// Two-host provider (Server1 + Server3) — parallelism tests.
    fn provider_with_two_hosts(executor: Arc<dyn SshExecutor>) -> RemoteTmuxProvider {
        RemoteTmuxProvider::new(config_with_hosts(), executor)
    }

    #[test]
    fn session_allowed_matches_exact_and_glob() {
        let wl: Vec<String> = vec!["train".into(), "worker*".into()];
        assert!(session_allowed("train", &wl));
        assert!(session_allowed("worker1", &wl));
        assert!(session_allowed("worker-abc", &wl));
        assert!(!session_allowed("other", &wl));
        assert!(!session_allowed("trainx", &wl)); // exact, not prefix
    }

    #[test]
    fn whitelist_filters_other_sessions() {
        // Only session "allowed" is watched; "other" must produce no task.
        let cfg = TaskPulseConfig {
            remote_hosts: vec![RemoteHostConfig {
                id: "server3".into(),
                ssh_host: "Server3".into(),
                enabled: true,
                poll_interval_ms: 3000,
                tmux_sessions: vec!["allowed".into()],
            }],
        };
        let fake = FakeSshExecutor::new()
            .with(
                "Server3",
                &snapshot_body(
                    &[
                        pane_line("allowed", "%1", 100, "bash", "/home/user/proj"),
                        pane_line("other", "%2", 300, "bash", "/home/user/proj"),
                    ],
                    &[
                        ps_line(100, 90, "bash", "bash"),
                        ps_line(200, 100, "python", "python train.py"),
                        ps_line(300, 290, "bash", "bash"),
                        ps_line(400, 300, "python", "python other.py"),
                    ],
                ),
            )
            .arc();
        let mut provider = RemoteTmuxProvider::new(cfg, fake);
        let out = provider.poll(SystemTime::now());
        // Only the "allowed" pane's python workload is shown.
        let ids: Vec<&str> = out.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), 1, "expected exactly one task, got {ids:?}");
        assert!(
            ids[0].ends_with(":%1"),
            "task should be from allowed pane %1, got {ids:?}"
        );
    }

    #[test]
    fn already_running_task_discovered_on_first_poll() {
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_snapshot())
            .arc();
        let mut provider = provider_with(fake);
        let out = provider.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        let t = &out.tasks[0];
        assert_eq!(t.status, TaskStatus::Running);
        assert_eq!(t.confidence, DetectionConfidence::Inferred);
        assert_eq!(t.id, "ssh:Server3:%3");
        assert_eq!(t.host.as_deref(), Some("Server3"));
        assert!(t.started_at.is_some()); // detected_at, never fabricated
        assert!(out.hosts.len() == 1);
        assert!(out.hosts[0].is_connected());
    }

    #[test]
    fn workload_to_shell_is_inferred_finish() {
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_snapshot())
            .with("Server3", &idle_snapshot())
            .arc();
        let mut provider = provider_with(fake);
        let now = SystemTime::now();

        let out = provider.poll(now);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);

        let out = provider.poll(now + Duration::from_secs(30));
        assert_eq!(out.tasks.len(), 1);
        let t = &out.tasks[0];
        assert_eq!(t.status, TaskStatus::Idle);
        assert_eq!(t.confidence, DetectionConfidence::Inferred);
        assert!(t.finished_at.is_some());
        assert!(t.recent);
        assert_eq!(t.exit_code, None); // never fabricate an exit code
    }

    #[test]
    fn pane_destroyed_while_running_is_inferred_finish() {
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_snapshot())
            .with("Server3", &empty_snapshot())
            .arc();
        let mut provider = provider_with(fake);
        let now = SystemTime::now();
        provider.poll(now);
        let out = provider.poll(now + Duration::from_secs(5));
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::Idle);
        assert!(out.tasks[0].finished_at.is_some());
    }

    #[test]
    fn ssh_timeout_does_not_complete_tasks() {
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_snapshot())
            .failing("Server3")
            .arc();
        let mut provider = provider_with(fake);
        let now = SystemTime::now();

        let out = provider.poll(now);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);

        // First failure: task preserved, stale, host still "connected".
        let out = provider.poll(now + Duration::from_secs(3));
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);
        assert!(out.tasks[0].stale);
        assert!(out.hosts[0].is_connected());
    }

    #[test]
    fn three_failures_marks_host_disconnected_but_tasks_preserved() {
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_snapshot())
            .failing("Server3")
            .failing("Server3")
            .failing("Server3")
            .arc();
        let mut provider = provider_with(fake);
        let now = SystemTime::now();

        provider.poll(now);
        provider.poll(now + Duration::from_secs(3));
        provider.poll(now + Duration::from_secs(6));
        let out = provider.poll(now + Duration::from_secs(9));

        assert_eq!(
            out.hosts[0].status,
            crate::providers::HostStatus::Disconnected
        );
        // The running task is preserved — never converted to finished.
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);
        assert!(out.tasks[0].stale);
    }

    #[test]
    fn recovery_clears_disconnected_and_stale() {
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_snapshot())
            .failing("Server3")
            .failing("Server3")
            .failing("Server3")
            .with("Server3", &running_snapshot())
            .arc();
        let mut provider = provider_with(fake);
        let now = SystemTime::now();

        let _ = provider.poll(now); // 1: success (running)
        let _ = provider.poll(now + Duration::from_secs(3)); // 2: fail #1
        let _ = provider.poll(now + Duration::from_secs(6)); // 3: fail #2
        let out = provider.poll(now + Duration::from_secs(9)); // 4: fail #3 → DISCONNECTED
        assert_eq!(
            out.hosts[0].status,
            crate::providers::HostStatus::Disconnected
        );

        // Recovery: host responds again, still running.
        let out = provider.poll(now + Duration::from_secs(12));
        assert!(out.hosts[0].is_connected());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);
        assert!(!out.tasks[0].stale);
    }

    #[test]
    fn hosts_polled_in_parallel() {
        let fake = FakeSshExecutor::new()
            .with("Server1", &running_snapshot())
            .with("Server3", &running_snapshot())
            .with_delay(200) // each fake call sleeps 200ms
            .arc();
        let mut provider = provider_with_two_hosts(fake);

        let start = Instant::now();
        let out = provider.poll(SystemTime::now());
        let elapsed = start.elapsed();

        // Two hosts × 200ms serial ≈ 400ms; parallel should be well under.
        assert_eq!(out.tasks.len(), 2);
        assert!(elapsed < Duration::from_millis(350), "elapsed={elapsed:?}");
    }

    #[test]
    fn one_host_failing_does_not_block_other() {
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_snapshot())
            .failing("Server1")
            .arc();
        let mut provider = provider_with_two_hosts(fake);
        let out = provider.poll(SystemTime::now());
        assert!(out
            .tasks
            .iter()
            .any(|t| t.host.as_deref() == Some("Server3")));
        // Server1's first failure keeps it "connected" (below threshold).
        assert!(out
            .hosts
            .iter()
            .any(|h| h.host == "Server1" && h.is_connected()));
    }

    #[test]
    fn distinct_hosts_keep_distinct_tasks() {
        let fake = FakeSshExecutor::new()
            .with("Server1", &running_snapshot())
            .with("Server3", &running_snapshot())
            .arc();
        let mut provider = provider_with_two_hosts(fake);
        let out = provider.poll(SystemTime::now());
        let ids: Vec<&str> = out.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["ssh:Server1:%3", "ssh:Server3:%3"]);
    }

    #[test]
    fn spaces_and_unicode_in_cwd_and_session() {
        let panes = vec![pane_line(
            "实验 session",
            "%9",
            700,
            "python",
            "/home/user/项目 HyperMem",
        )];
        let body = snapshot_body(&panes, &[ps_line(700, 600, "python", "python 训练.py")]);
        let fake = FakeSshExecutor::new().with("Server3", &body).arc();
        let mut provider = provider_with(fake);
        let out = provider.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].project.as_deref(), Some("项目 HyperMem"));
        assert_eq!(out.tasks[0].name, "python 训练.py");
    }

    #[test]
    fn bash_script_chain_detected_via_process_tree() {
        // bash run_eval.sh → bash → python eval.py
        let panes = vec![pane_line("eval", "%5", 800, "bash", "/home/user/proj")];
        let body = snapshot_body(
            &panes,
            &[
                ps_line(800, 700, "bash", "bash run_eval.sh"),
                ps_line(801, 800, "bash", "bash"),
                ps_line(802, 801, "python", "python eval.py"),
            ],
        );
        let fake = FakeSshExecutor::new().with("Server3", &body).arc();
        let mut provider = provider_with(fake);
        let out = provider.poll(SystemTime::now());
        assert_eq!(out.tasks.len(), 1);
        assert_eq!(out.tasks[0].status, TaskStatus::Running);
        assert_eq!(out.tasks[0].name, "python eval.py");
    }

    // ---- exact mode (tp-run) integration ----

    fn run_file(label: &str, pane: &str, status: &str, exit: i64, started: i64) -> String {
        format!(
            r#"{{"schema_version":1,"run_id":"r-{label}","label":"{label}","hostname":"Server3","pane_id":"{pane}","cwd":"/home/user/proj","command":"python eval.py","pid":500,"started_at":{started},"finished_at":{started},"status":"{status}","exit_code":{exit}}}"#
        )
    }

    #[test]
    fn exact_succeeded_beats_inferred_idle_for_same_pane() {
        let now = SystemTime::now();
        let started = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 - 100;
        // Pane is now idle (python finished), but tp-run recorded exit 0.
        let body = snapshot_body_with_runs(
            &[pane_line("demo", "%3", 400, "bash", "/home/user/proj")],
            &[ps_line(400, 300, "bash", "bash")],
            &run_file("LoCoMo", "%3", "succeeded", 0, started),
        );
        // First poll: inferred RUNNING baseline, then exact SUCCEEDED wins.
        let running_body = snapshot_body(
            &[pane_line("demo", "%3", 500, "python", "/home/user/proj")],
            &[ps_line(500, 400, "python", "python eval.py")],
        );
        let fake = FakeSshExecutor::new()
            .with("Server3", &running_body)
            .with("Server3", &body)
            .arc();
        let mut provider = provider_with(fake);
        let out1 = provider.poll(now);
        assert_eq!(out1.tasks[0].status, TaskStatus::Running);

        let out2 = provider.poll(now + Duration::from_secs(30));
        // The provider emits both the inferred IDLE and the exact SUCCEEDED
        // for the same pane; the store merges them (EXACT wins).  Assert the
        // exact observation is present here.
        let task = out2
            .tasks
            .iter()
            .find(|t| t.id == "ssh:Server3:%3" && t.confidence == DetectionConfidence::Exact)
            .unwrap();
        assert_eq!(task.status, TaskStatus::Succeeded);
        assert_eq!(task.exit_code, Some(0));
        assert!(task.recent);
    }

    #[test]
    fn exact_failed_with_exit_2() {
        let now = SystemTime::now();
        let started = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 - 100;
        let body = snapshot_body_with_runs(
            &[pane_line("demo", "%3", 400, "bash", "/home/user/proj")],
            &[ps_line(400, 300, "bash", "bash")],
            &run_file("failure-demo", "%3", "failed", 2, started),
        );
        let fake = FakeSshExecutor::new().with("Server3", &body).arc();
        let mut provider = provider_with(fake);
        let out = provider.poll(now);
        let task = out
            .tasks
            .iter()
            .find(|t| t.id == "ssh:Server3:%3" && t.confidence == DetectionConfidence::Exact)
            .unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.exit_code, Some(2));
    }

    #[test]
    fn exact_running_overrides_inferred_idle_no_false_completion() {
        // tp-run says still running (pid alive); process tree lost the
        // workload.  EXACT must win: the task stays RUNNING.
        let now = SystemTime::now();
        let started = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 - 100;
        let body = snapshot_body_with_runs(
            &[pane_line("demo", "%3", 400, "bash", "/home/user/proj")],
            &[
                ps_line(400, 300, "bash", "bash"),
                ps_line(500, 400, "python", "python eval.py"),
            ],
            &run_file("still-running", "%3", "running", 0, started),
        );
        let fake = FakeSshExecutor::new().with("Server3", &body).arc();
        let mut provider = provider_with(fake);
        let out = provider.poll(now);
        let task = out
            .tasks
            .iter()
            .find(|t| t.id == "ssh:Server3:%3" && t.confidence == DetectionConfidence::Exact)
            .unwrap();
        assert_eq!(task.status, TaskStatus::Running);
    }

    #[test]
    fn stale_running_run_file_ignored_when_pid_dead() {
        // Wrapper killed -9: file says "running" but pid is gone and pane is
        // an idle shell → must NOT pin the task to RUNNING.
        let now = SystemTime::now();
        let started = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 - 100;
        let body = snapshot_body_with_runs(
            &[pane_line("demo", "%3", 400, "bash", "/home/user/proj")],
            &[ps_line(400, 300, "bash", "bash")],
            &run_file("dead", "%3", "running", 0, started),
        );
        let fake = FakeSshExecutor::new().with("Server3", &body).arc();
        let mut provider = provider_with(fake);
        let out = provider.poll(now);
        // No task: neither exact (dead) nor inferred (idle shell, never ran).
        assert!(out.tasks.is_empty());
    }

    // ---- live smoke test against the user's real servers ----
    // Requires ~/.taskpulse/config.toml with reachable hosts.  Run explicitly:
    //   cargo test --bin agent-tray live_remote -- --ignored --nocapture
    #[test]
    #[ignore = "requires real SSH access to Server1/Server3"]
    fn live_remote_snapshot_against_real_hosts() {
        let mut provider =
            RemoteTmuxProvider::from_config(std::sync::Arc::new(ssh::SystemSshExecutor::new()));
        let out = provider.poll(SystemTime::now());
        println!("hosts: {:?}", out.hosts);
        for t in &out.tasks {
            println!(
                "  task: id={} name={:?} status={:?} conf={:?} host={:?}",
                t.id, t.name, t.status, t.confidence, t.host
            );
        }
        // Whatever the servers report, the snapshot must parse without panics
        // and every task must carry a stable id.
        for t in &out.tasks {
            assert!(!t.id.is_empty());
        }
    }
}
