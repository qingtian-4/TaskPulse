//! Per-host poll state: failure debounce, preserved last-known tasks, and
//! running-pane tracking for finish detection.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use crate::domain::TaskSnapshot;
use crate::providers::HostStatus;

/// Consecutive SSH failures before a host flips to DISCONNECTED.
pub const DISCONNECT_THRESHOLD: u32 = 3;

#[derive(Debug, Clone)]
pub struct HostPollState {
    pub status: HostStatus,
    pub consecutive_failures: u32,
    /// Last successful poll time (None until first success).
    pub last_seen: Option<SystemTime>,
    /// Last known tasks — preserved (with `stale = true`) while the host is
    /// unreachable so an SSH timeout never looks like "all tasks finished".
    pub last_tasks: Vec<TaskSnapshot>,
    /// Panes we have seen running.  A pane that leaves this set (observed
    /// idle or gone during a SUCCESSFUL poll) is an inferred finish.
    pub running_panes: HashSet<String>,
    pub pane_roots: HashMap<String, u32>,
}

impl Default for HostPollState {
    fn default() -> Self {
        Self {
            status: HostStatus::Connected,
            consecutive_failures: 0,
            last_seen: None,
            last_tasks: Vec::new(),
            running_panes: HashSet::new(),
            pane_roots: HashMap::new(),
        }
    }
}

impl HostPollState {
    pub fn record_success(&mut self, now: SystemTime) {
        self.consecutive_failures = 0;
        self.status = HostStatus::Connected;
        self.last_seen = Some(now);
    }

    /// Returns true when the host just crossed into DISCONNECTED.
    pub fn record_failure(&mut self, now: SystemTime) -> bool {
        self.consecutive_failures += 1;
        let _ = now;
        if self.consecutive_failures >= DISCONNECT_THRESHOLD {
            let was = self.status;
            self.status = HostStatus::Disconnected;
            was == HostStatus::Connected
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self.status == HostStatus::Connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_threshold_debounce() {
        let mut s = HostPollState::default();
        let now = SystemTime::now();
        for _ in 0..2 {
            assert!(!s.record_failure(now));
        }
        // 3rd failure crosses the threshold.
        assert!(s.record_failure(now));
        assert_eq!(s.status, HostStatus::Disconnected);
        // Further failures don't re-trigger.
        assert!(!s.record_failure(now));
    }

    #[test]
    fn success_resets_failures() {
        let mut s = HostPollState::default();
        let now = SystemTime::now();
        s.record_failure(now);
        s.record_failure(now);
        s.record_success(now);
        assert!(s.is_connected());
        assert_eq!(s.consecutive_failures, 0);
        assert!(s.last_seen.is_some());
    }

    #[test]
    fn recovery_after_disconnect() {
        let mut s = HostPollState::default();
        let now = SystemTime::now();
        for _ in 0..3 {
            s.record_failure(now);
        }
        assert_eq!(s.status, HostStatus::Disconnected);
        s.record_success(now);
        assert!(s.is_connected());
    }
}
