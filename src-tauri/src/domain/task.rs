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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitingReason {
    TurnEnded,
    Approval,
    Question,
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
    GenericEvent,
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
            TaskSource::GenericEvent => "其他 Agent",
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

/// Observed activity, independent of lifecycle and source freshness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Thinking,
    Tool,
    Responding,
    AwaitingModel,
    /// Waiting for a tool response, never evidence of pending human approval.
    /// Retained for persisted snapshots from older versions.
    AwaitingTool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Refreshed only by an explicit matching pending-request observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_confirmed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_epoch: Option<String>,
    #[serde(default)]
    pub integration_sequence: u64,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_instance: Option<String>,
    /// Model reported by the agent/response, never part of session identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider-identified internal service work, not a user coding task.
    #[serde(default)]
    pub internal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_reason: Option<WaitingReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_request_id: Option<String>,
    /// All outstanding explicit requests; the scalar above is the primary one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_request_ids: Vec<String>,
    #[serde(default)]
    pub attention_since: u64,
    #[serde(default)]
    pub new_execution: bool,
    /// Remote host alias, e.g. "Server3".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// cwd basename when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Concrete tool/command activity observed during the current agent turn.
    /// Kept separate from `message`, which is the latest agent prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Subphase of an active turn. Tool completion is NOT turn completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ExecutionPhase>,
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
    /// TaskPulse attention state. These fields are owned by the attention
    /// tracker, not by providers, and are persisted by stable task id.
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub pinned: bool,
    /// Stable identity of the latest observed turn. A changed key re-opens a
    /// dismissed task without changing the session/task id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_key: Option<String>,
    /// Best-effort unread signal from the native agent. `None` means the
    /// provider cannot determine it reliably.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_unread: Option<bool>,
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
    /// Codex 5-hour window usage percent (0-100) from token_count rate_limits.primary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_5h_used_pct: Option<f64>,
    /// Unix epoch seconds when the 5-hour window resets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_5h_resets_at: Option<i64>,
    /// Codex 1-week window usage percent (0-100) from rate_limits.secondary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_1w_used_pct: Option<f64>,
    /// Unix epoch seconds when the 1-week window resets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_1w_resets_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_matcher: Option<String>,
}

impl TaskSnapshot {
    pub fn replays_integration(&self, previous: &Self) -> bool {
        self.source == TaskSource::GenericEvent
            && self.integration_epoch.is_some()
            && self.integration_epoch == previous.integration_epoch
            && self.integration_sequence <= previous.integration_sequence
    }

    /// A failed/recovered collector can change uncertainty without a new host
    /// fact. Permit only the exact current version to update this flag; never
    /// let an older replay overwrite lifecycle, title or user attention state.
    pub fn merge_integration_health(&mut self, incoming: &Self) {
        if incoming.source == TaskSource::GenericEvent
            && incoming.integration_epoch.is_some()
            && incoming.integration_epoch == self.integration_epoch
            && incoming.integration_sequence == self.integration_sequence
        {
            self.stale = incoming.stale;
        }
    }
    /// A conversation can be thinking on a server while its local process is
    /// idle. Process/CPU guesses must never end a structured agent turn.
    pub fn is_agent_session(&self) -> bool {
        self.session_id.is_some()
            || matches!(
                self.source,
                TaskSource::Codex
                    | TaskSource::Hermes
                    | TaskSource::DeepSeekHarness
                    | TaskSource::ClaudeCode
                    | TaskSource::Gemini
            )
    }

    pub fn allows_inferred_finish(&self) -> bool {
        self.source == TaskSource::RemoteTmux
            && self.session_id.is_none()
            && self.finished_at.is_some()
    }
    pub fn requires_action(&self) -> bool {
        self.status == TaskStatus::WaitingInput
            && matches!(
                self.waiting_reason,
                Some(WaitingReason::Approval | WaitingReason::Question)
            )
    }
    /// Compatibility alias for a legacy pane-scoped exact run.
    pub fn legacy_run_alias(&self) -> Option<String> {
        if self.source != TaskSource::RemoteTmux || self.confidence != DetectionConfidence::Exact {
            return None;
        }
        let host = self.host.as_deref()?;
        let pane = self.tmux_pane.as_deref().filter(|p| !p.is_empty())?;
        let run = self.turn_key.as_deref().filter(|r| !r.is_empty())?;
        let legacy = format!("ssh:{host}:{pane}");
        let current = format!("ssh:{host}:run:{run}");
        (self.id == legacy || self.id == current).then_some(legacy)
    }

    pub fn normalize_run_identity(&mut self) -> bool {
        if self.legacy_run_alias().as_deref() == Some(&self.id) {
            self.id = format!(
                "ssh:{}:run:{}",
                self.host.as_deref().unwrap(),
                self.turn_key.as_deref().unwrap()
            );
            return true;
        }
        false
    }

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
            agent_id: None,
            agent_name: None,
            evidence_channel: None,
            outcome: None,
            pending_confirmed_at: None,
            integration_epoch: None,
            integration_sequence: 0,
            status,
            source_instance: None,
            model: None,
            phase: None,
            internal: false,
            project_path: None,
            tmux_session: None,
            tmux_pane: None,
            tmux_target: None,
            waiting_reason: None,
            input_request_id: None,
            input_request_ids: vec![],
            attention_since: 0,
            new_execution: false,
            host: None,
            project: None,
            message: None,
            activity: None,
            started_at: None,
            updated_at: now,
            finished_at: None,
            pid: None,
            exit_code: None,
            confidence,
            remote: false,
            stale: false,
            recent: false,
            unread: false,
            pinned: false,
            turn_key: None,
            native_unread: None,
            terminal: None,
            can_focus: false,
            cpu: None,
            cli: None,
            session_id: None,
            hook_event: None,
            hook_matcher: None,
            quota_5h_used_pct: None,
            quota_5h_resets_at: None,
            quota_1w_used_pct: None,
            quota_1w_resets_at: None,
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
