//! Unified task model (replaces AgentTray's `AgentStatus`).
//!
//! One snapshot type feeds every source — local agent hooks, process scans,
//! and remote tmux panes — so the store, transition engine, tray, and UI all
//! speak one language.  Semantic rules enforced by the model:
//!
//! * `WAITING_INPUT` means "agent finished its current turn, waiting for the
//!   user" — never `SUCCEEDED` (no exit code involved).
//! * `SUCCEEDED`/`FAILED` are only ever derived from a real command exit code.
//! * `DetectionConfidence` distinguishes exact (tp-run exit code), hook
//!   (deterministic hook event), and inferred (process-tree heuristics).

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Terminal focus metadata (moved here from `watcher::TerminalInfo` so the
/// domain owns its own types; `watcher` re-exports it for compatibility).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalInfo {
    pub kind: String,
    pub focus_id: String,
    pub outer_id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
}

/// Lifecycle state of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// A local agent is mid-turn, or a remote workload process is running.
    Running,
    /// Agent finished its current turn and is waiting for user input.
    WaitingInput,
    /// Finished with exit code == 0 (exact knowledge only).
    Succeeded,
    /// Finished with exit code != 0 (exact knowledge only).
    Failed,
    /// Shell at a prompt with no workload, or an inferred-finished task.
    Idle,
    /// Host unreachable after the failure debounce threshold.
    Disconnected,
    /// No signal at all.
    Unknown,
}

impl TaskStatus {
    /// Attention priority: lower = more urgent.
    /// FAILED > WAITING_INPUT > DISCONNECTED > RUNNING > SUCCEEDED/IDLE > UNKNOWN
    pub fn priority(self) -> u8 {
        match self {
            TaskStatus::Failed => 0,
            TaskStatus::WaitingInput => 1,
            TaskStatus::Disconnected => 2,
            TaskStatus::Running => 3,
            TaskStatus::Succeeded | TaskStatus::Idle => 4,
            TaskStatus::Unknown => 5,
        }
    }

    /// Terminal states (a finished run).
    pub fn is_terminal(self) -> bool {
        matches!(self, TaskStatus::Succeeded | TaskStatus::Failed)
    }

    /// Actively working / waiting states shown in the ACTIVE section.
    pub fn is_active(self) -> bool {
        matches!(self, TaskStatus::Running | TaskStatus::WaitingInput)
    }

    /// Human-readable label for UI chips.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Running => "running",
            TaskStatus::WaitingInput => "waiting input",
            TaskStatus::Succeeded => "succeeded",
            TaskStatus::Failed => "failed",
            TaskStatus::Idle => "idle",
            TaskStatus::Disconnected => "disconnected",
            TaskStatus::Unknown => "unknown",
        }
    }
}

/// Which system produced the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    Codex,
    Hermes,
    #[serde(rename = "deepseek_harness")]
    DeepSeekHarness,
    RemoteTmux,
    /// Generic local wrapper/hook file (`~/.agent-monitor` / `~/.taskpulse`).
    LocalFile,
    ClaudeCode,
    Gemini,
}

impl TaskSource {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            TaskSource::Codex => "Codex",
            TaskSource::Hermes => "Hermes",
            TaskSource::DeepSeekHarness => "DeepSeek Harness",
            TaskSource::RemoteTmux => "Remote Tmux",
            TaskSource::LocalFile => "Local",
            TaskSource::ClaudeCode => "Claude Code",
            TaskSource::Gemini => "Gemini",
        }
    }
}

/// How certain we are about a status.  Never let inference masquerade as fact.
/// Merge order: EXACT > HOOK > INFERRED.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionConfidence {
    /// Real exit code (tp-run) or equivalent authoritative signal.
    Exact,
    /// Deterministic hook event / wrapper write.
    Hook,
    /// Process-tree / heuristic inference.
    Inferred,
}

impl DetectionConfidence {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            DetectionConfidence::Exact => "exact",
            DetectionConfidence::Hook => "hook",
            DetectionConfidence::Inferred => "inferred",
        }
    }
}

/// Epoch-millis serialization for `SystemTime`.
mod epoch_ms {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Default value for fields that may be absent in older payloads.
    pub fn default() -> SystemTime {
        UNIX_EPOCH
    }

    pub fn serialize<S>(t: &SystemTime, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let ms = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        ser.serialize_u64(ms)
    }

    pub fn deserialize<'de, D>(de: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ms = u64::deserialize(de)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_millis(ms))
    }
}

mod opt_epoch_ms {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(t: &Option<SystemTime>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match t {
            Some(t) => {
                let ms = t
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                ser.serialize_some(&ms)
            }
            None => ser.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(de: D) -> Result<Option<SystemTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ms = Option::<u64>::deserialize(de)?;
        Ok(ms.map(|ms| UNIX_EPOCH + std::time::Duration::from_millis(ms)))
    }
}

/// The unified task snapshot.
///
/// `id` is the stable identity for dedup and notification tracking:
/// `file:<stem>`, `scan:<tty>`, `ssh:<host>:<pane_id>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub id: String,
    pub name: String,
    pub source: TaskSource,
    pub status: TaskStatus,
    /// Remote host alias, e.g. "Server3".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// cwd basename when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(
        default,
        with = "opt_epoch_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub started_at: Option<SystemTime>,
    #[serde(default = "epoch_ms::default", with = "epoch_ms")]
    pub updated_at: SystemTime,
    #[serde(
        default,
        with = "opt_epoch_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<SystemTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Only ever set from a real command exit code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub confidence: DetectionConfidence,
    #[serde(default)]
    pub remote: bool,
    /// Host is stale (recent SSH failure) but last state is preserved.
    #[serde(default)]
    pub stale: bool,
    /// Shown in the RECENT completed section.
    #[serde(default)]
    pub recent: bool,
    // ---- legacy passthrough (focus / scanner extras) ----
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalInfo>,
    #[serde(default)]
    pub can_focus: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Codex 7-day window usage percent (0-100) from token_count rate_limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_used_pct: Option<f64>,
    /// Unix epoch seconds the Codex usage window resets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_resets_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_matcher: Option<String>,
}

impl TaskSnapshot {
    /// Small constructor for tests and providers.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        source: TaskSource,
        status: TaskStatus,
        confidence: DetectionConfidence,
        now: SystemTime,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            source,
            status,
            host: None,
            project: None,
            message: None,
            started_at: None,
            updated_at: now,
            finished_at: None,
            pid: None,
            exit_code: None,
            confidence,
            remote: false,
            stale: false,
            recent: false,
            terminal: None,
            can_focus: false,
            cpu: None,
            cli: None,
            session_id: None,
            hook_event: None,
            hook_matcher: None,
            quota_used_pct: None,
            quota_resets_at: None,
        }
    }

    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_pid(mut self, pid: u32) -> Self {
        self.pid = Some(pid);
        self
    }

    #[allow(dead_code)]
    pub fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }

    pub fn with_started_at(mut self, t: SystemTime) -> Self {
        self.started_at = Some(t);
        self
    }

    pub fn with_finished_at(mut self, t: SystemTime) -> Self {
        self.finished_at = Some(t);
        self
    }

    pub fn with_remote(mut self) -> Self {
        self.remote = true;
        self
    }

    #[allow(dead_code)]
    pub fn with_terminal(mut self, t: TerminalInfo) -> Self {
        self.can_focus = !t.focus_id.is_empty() && t.focus_id != "0";
        self.terminal = Some(t);
        self
    }

    #[allow(dead_code)]
    pub fn with_cpu(mut self, cpu: f64) -> Self {
        self.cpu = Some(cpu);
        self
    }
}

/// Confidence ordering used by the store: EXACT > HOOK > INFERRED.
#[allow(dead_code)]
pub fn confidence_beats(a: DetectionConfidence, b: DetectionConfidence) -> bool {
    a < b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_priority_order() {
        assert!(TaskStatus::Failed.priority() < TaskStatus::WaitingInput.priority());
        assert!(TaskStatus::WaitingInput.priority() < TaskStatus::Disconnected.priority());
        assert!(TaskStatus::Disconnected.priority() < TaskStatus::Running.priority());
        assert!(TaskStatus::Running.priority() < TaskStatus::Succeeded.priority());
        assert!(TaskStatus::Succeeded.priority() <= TaskStatus::Idle.priority());
        assert!(TaskStatus::Idle.priority() < TaskStatus::Unknown.priority());
    }

    #[test]
    fn confidence_ordering() {
        assert!(DetectionConfidence::Exact < DetectionConfidence::Hook);
        assert!(DetectionConfidence::Hook < DetectionConfidence::Inferred);
        assert!(confidence_beats(
            DetectionConfidence::Exact,
            DetectionConfidence::Inferred
        ));
        assert!(!confidence_beats(
            DetectionConfidence::Inferred,
            DetectionConfidence::Hook
        ));
    }

    #[test]
    fn terminal_states() {
        assert!(TaskStatus::Succeeded.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
    }

    #[test]
    fn snapshot_serializes_with_epoch_ms() {
        let now = SystemTime::now();
        let t = TaskSnapshot::new(
            "ssh:Server3:%3",
            "python eval.py",
            TaskSource::RemoteTmux,
            TaskStatus::Running,
            DetectionConfidence::Inferred,
            now,
        )
        .with_host("Server3")
        .with_remote();
        let json = serde_json::to_string(&t).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["id"], "ssh:Server3:%3");
        assert_eq!(v["status"], "running");
        assert_eq!(v["source"], "remote_tmux");
        assert_eq!(v["confidence"], "inferred");
        assert_eq!(v["host"], "Server3");
        assert!(v["updated_at"].as_u64().is_some());
        assert_eq!(v["recent"], false);
    }

    #[test]
    fn legacy_statuses_map_to_new_enum() {
        // Every legacy AgentTray status string has a TaskStatus home.
        let cases = [
            ("needs-input", TaskStatus::WaitingInput),
            ("working", TaskStatus::Running),
            ("starting", TaskStatus::Running),
            ("error", TaskStatus::Failed),
            ("idle", TaskStatus::Idle),
            ("offline", TaskStatus::Unknown),
        ];
        for (legacy, expected) in cases {
            assert_eq!(
                crate::watcher::status_from_legacy(legacy),
                expected,
                "{legacy}"
            );
        }
    }
}
