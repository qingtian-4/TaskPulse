//! TaskPulse domain model: the unified task snapshot shared by every provider,
//! the store, the transition engine, and the UI.  This is the replacement for
//! AgentTray's stringly-typed `AgentStatus.status` field.

pub mod task;

pub use task::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus, TerminalInfo};
