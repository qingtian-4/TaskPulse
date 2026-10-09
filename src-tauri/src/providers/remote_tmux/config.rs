//! Remote host configuration (`~/.taskpulse/config.toml`).
//!
//! ```toml
//! [[remote_hosts]]
//! id = "server1"
//! ssh_host = "Server1"   # alias in ~/.ssh/config
//! enabled = true
//! poll_interval_ms = 3000
//! ```
//!
//! TaskPulse never stores credentials — it shells out to the system `ssh.exe`
//! and relies on the user's own `~/.ssh/config` + ssh-agent / keys.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::{TaskSnapshot, TaskSource};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteHostConfig {
    pub id: String,
    pub ssh_host: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,
    /// Only monitor tasks inside these tmux session names (exact or `*` glob).
    /// Empty = all sessions.
    #[serde(default)]
    pub tmux_sessions: Vec<String>,
    /// Optional pane allowlist. Entries match `%pane_id` or
    /// `session:window.pane`, with `*` wildcards.
    #[serde(default)]
    pub tmux_panes: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_poll_ms() -> u64 {
    3000
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskPulseConfig {
    #[serde(default)]
    pub remote_hosts: Vec<RemoteHostConfig>,
    #[serde(default)]
    pub monitoring: MonitoringConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCodexScope {
    pub host: String,
    #[serde(default)]
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Empty means disabled; `*` means all projects. Missing legacy config is
    /// upgraded through Default to preserve the previous monitor-all behavior.
    #[serde(default = "wildcard_projects")]
    pub local_codex_projects: Vec<String>,
    #[serde(default = "wildcard_projects")]
    pub hermes_projects: Vec<String>,
    #[serde(default = "wildcard_projects")]
    pub deepseek_harness_projects: Vec<String>,
    #[serde(default = "wildcard_remote_codex")]
    pub remote_codex: Vec<RemoteCodexScope>,
}

fn wildcard_projects() -> Vec<String> {
    vec!["*".into()]
}

fn wildcard_remote_codex() -> Vec<RemoteCodexScope> {
    vec![RemoteCodexScope {
        host: "*".into(),
        projects: wildcard_projects(),
    }]
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            local_codex_projects: wildcard_projects(),
            hermes_projects: wildcard_projects(),
            deepseek_harness_projects: wildcard_projects(),
            remote_codex: wildcard_remote_codex(),
        }
    }
}

impl MonitoringConfig {
    #[allow(dead_code)]
    pub fn allows(&self, task: &TaskSnapshot) -> bool {
        match (task.source, task.remote) {
            (TaskSource::Codex, false) => {
                project_allowed(task.project.as_deref(), &self.local_codex_projects)
            }
            (TaskSource::Codex, true) => self.remote_codex.iter().any(|scope| {
                wildcard_match(&scope.host, task.host.as_deref().unwrap_or(""))
                    && project_allowed(task.project.as_deref(), &scope.projects)
            }),
            (TaskSource::Hermes, _) => {
                project_allowed(task.project.as_deref(), &self.hermes_projects)
            }
            (TaskSource::DeepSeekHarness, _) => {
                project_allowed(task.project.as_deref(), &self.deepseek_harness_projects)
            }
            _ => true,
        }
    }
}

#[allow(dead_code)]
fn project_allowed(project: Option<&str>, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        pattern == "*" || project.is_some_and(|project| wildcard_match(pattern, project))
    })
}

pub fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == value;
    }
    let mut remainder = value.as_str();
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let Some(position) = remainder.find(part) else {
            return false;
        };
        if index == 0 && !pattern.starts_with('*') && position != 0 {
            return false;
        }
        remainder = &remainder[position + part.len()..];
    }
    pattern.ends_with('*') || remainder.is_empty()
}

impl TaskPulseConfig {
    pub fn enabled_hosts(&self) -> Vec<RemoteHostConfig> {
        self.remote_hosts
            .iter()
            .filter(|h| h.enabled)
            .cloned()
            .collect()
    }
}

pub fn config_dir() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".taskpulse")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// First run enables local monitoring only. Remote hosts require explicit
/// configuration; existing user configs are preserved.
pub fn load_config() -> TaskPulseConfig {
    let path = config_path();
    if !path.exists() {
        let cfg = default_config();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string_pretty(&cfg) {
            let _ = atomic_write(&path, text.as_bytes());
        }
        return cfg;
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("Invalid config {}: {} — using defaults", path.display(), e);
            default_config()
        }),
        Err(_) => default_config(),
    }
}

pub fn default_config() -> TaskPulseConfig {
    TaskPulseConfig {
        remote_hosts: Vec::new(),
        monitoring: MonitoringConfig::default(),
    }
}

/// Atomic write (tmp + rename) so a reader never sees a half-written file.
pub fn atomic_write(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DetectionConfidence, TaskStatus};

    #[test]
    fn fresh_install_does_not_contact_personal_servers() {
        assert!(default_config().enabled_hosts().is_empty());
    }
    use std::time::SystemTime;

    #[test]
    fn parses_sample_config() {
        let text = r#"
[[remote_hosts]]
id = "server1"
ssh_host = "Server1"
enabled = true
poll_interval_ms = 3000

[[remote_hosts]]
id = "server3"
ssh_host = "Server3"
enabled = true
poll_interval_ms = 3000
"#;
        let cfg: TaskPulseConfig = toml::from_str(text).unwrap();
        assert_eq!(cfg.remote_hosts.len(), 2);
        assert_eq!(cfg.remote_hosts[0].ssh_host, "Server1");
        assert_eq!(cfg.remote_hosts[1].poll_interval_ms, 3000);
    }

    #[test]
    fn disabled_hosts_excluded() {
        let cfg: TaskPulseConfig = toml::from_str(
            r#"
[[remote_hosts]]
id = "a"
ssh_host = "A"
enabled = true

[[remote_hosts]]
id = "b"
ssh_host = "B"
enabled = false
"#,
        )
        .unwrap();
        assert_eq!(cfg.enabled_hosts().len(), 1);
    }

    #[test]
    fn defaults_applied_when_fields_missing() {
        let cfg: TaskPulseConfig = toml::from_str(
            r#"
[[remote_hosts]]
id = "a"
ssh_host = "A"
"#,
        )
        .unwrap();
        assert!(cfg.remote_hosts[0].enabled);
        assert_eq!(cfg.remote_hosts[0].poll_interval_ms, 3000);
    }

    #[test]
    fn parses_tmux_session_whitelist() {
        let cfg: TaskPulseConfig = toml::from_str(
            r#"
[[remote_hosts]]
id = "a"
ssh_host = "A"
tmux_sessions = ["train", "worker*"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.remote_hosts[0].tmux_sessions, vec!["train", "worker*"]);
    }

    #[test]
    fn monitoring_scope_filters_local_and_remote_projects() {
        let cfg: TaskPulseConfig = toml::from_str(
            r#"
[monitoring]
local_codex_projects = ["agenttray"]
hermes_projects = ["*"]
deepseek_harness_projects = []

[[monitoring.remote_codex]]
host = "Server3"
projects = ["Hyper*"]
"#,
        )
        .unwrap();
        let mut local = TaskSnapshot::new(
            "local",
            "Review UI",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            SystemTime::now(),
        );
        local.project = Some("agenttray".into());
        assert!(cfg.monitoring.allows(&local));
        local.project = Some("other".into());
        assert!(!cfg.monitoring.allows(&local));

        let mut remote = TaskSnapshot::new(
            "remote",
            "Memory research",
            TaskSource::Codex,
            TaskStatus::Running,
            DetectionConfidence::Hook,
            SystemTime::now(),
        )
        .with_remote()
        .with_host("server3");
        remote.project = Some("HyperMem".into());
        assert!(cfg.monitoring.allows(&remote));
        remote.host = Some("Server1".into());
        assert!(!cfg.monitoring.allows(&remote));
    }

    #[test]
    fn wildcard_match_is_case_insensitive_and_anchored() {
        assert!(wildcard_match("Hyper*", "hypermem"));
        assert!(wildcard_match("*Mem", "HyperMem"));
        assert!(!wildcard_match("Hyper", "HyperMem"));
    }
}
