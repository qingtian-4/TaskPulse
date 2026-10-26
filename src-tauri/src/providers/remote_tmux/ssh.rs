//! SSH execution abstraction.
//!
//! TaskPulse shells out to the system `ssh.exe` with `BatchMode=yes` so it
//! never prompts and never handles credentials — keys, agents, and
//! `~/.ssh/config` are the user's own.  The trait exists so unit tests can
//! drive the whole remote provider with a fake executor (no real servers).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub trait SshExecutor: Send + Sync {
    /// Run one remote command.  `Err` on spawn failure, timeout, or
    /// non-zero exit — callers must treat any `Err` as "no data this round",
    /// never as "tasks finished".
    fn run(&self, host: &str, command: &str, timeout: Duration) -> Result<String, String>;
}

/// Real executor: `ssh -o BatchMode=yes -o ConnectTimeout=<s> <host> <cmd>`.
pub struct SystemSshExecutor;

impl SystemSshExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SystemSshExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SshExecutor for SystemSshExecutor {
    fn run(&self, host: &str, command: &str, timeout: Duration) -> Result<String, String> {
        let connect_timeout = timeout.as_secs().clamp(1, 10).to_string();
        // stderr is directed to the null device: no unbounded reader and no
        // remote command/credential text leaked through diagnostics. stdout
        // must be complete UTF-8 within the per-snapshot byte budget.
        let connect = format!("ConnectTimeout={connect_timeout}");
        let result = crate::process::output(
            "ssh",
            &[
                "-o",
                "BatchMode=yes",
                "-o",
                "LogLevel=ERROR",
                "-o",
                &connect,
                "--",
                host,
                command,
            ],
            timeout,
            16 * 1024 * 1024,
        )
        .map_err(|error| format!("ssh {host} probe failed: {error:?}"))?;
        if !result.status.success() {
            return Err(format!("ssh {host} exited {:?}", result.status.code()));
        }
        String::from_utf8(result.stdout).map_err(|_| format!("ssh {host} returned invalid UTF-8"))
    }
}

/// Fake executor for tests: per-host scripted response queues.
///
/// Each `script()` call appends one step (`Ok(output)` or `Err(reason)`).
/// Calls consume steps in order, so a test can replay "running → timeout →
/// timeout → running".  Interior mutability (Mutex) lets the same fake be
/// shared across the provider's parallel poll threads.
#[cfg_attr(not(test), allow(dead_code))]
pub struct FakeSshExecutor {
    sequences: Mutex<HashMap<String, VecDeque<Result<String, String>>>>,
    delay_ms: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl FakeSshExecutor {
    pub fn new() -> Self {
        Self {
            sequences: Mutex::new(HashMap::new()),
            delay_ms: 0,
        }
    }

    /// Append a scripted step for `host`.
    pub fn script(self, host: &str, step: Result<&str, &str>) -> Self {
        self.sequences
            .lock()
            .unwrap()
            .entry(host.to_string())
            .or_default()
            .push_back(step.map(|s| s.to_string()).map_err(|e| e.to_string()));
        self
    }

    /// Convenience: one successful output step.
    pub fn with(self, host: &str, output: &str) -> Self {
        self.script(host, Ok(output))
    }

    /// Convenience: one failure step.
    pub fn failing(self, host: &str) -> Self {
        self.script(host, Err("Connection timed out (fake)"))
    }

    /// Artificial per-call delay to simulate slow SSH (parallelism tests).
    pub fn with_delay(mut self, ms: u64) -> Self {
        self.delay_ms = ms;
        self
    }

    pub fn arc(self) -> Arc<dyn SshExecutor> {
        Arc::new(self)
    }
}

impl Default for FakeSshExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SshExecutor for FakeSshExecutor {
    fn run(&self, host: &str, _command: &str, _timeout: Duration) -> Result<String, String> {
        if self.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.delay_ms));
        }
        let mut seq = self.sequences.lock().unwrap();
        match seq.get_mut(host) {
            Some(queue) => match queue.pop_front() {
                Some(step) => step,
                None => Err(format!("ssh {host}: no more scripted responses (fake)")),
            },
            None => Err(format!("ssh {host}: no scripted responses (fake)")),
        }
    }
}
