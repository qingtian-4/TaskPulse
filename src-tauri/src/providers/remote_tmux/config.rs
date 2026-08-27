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

/// Load the config, creating a starter file with the user's real SSH aliases
/// on first run (nothing is hardcoded in the core logic).
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

/// Atomic write (tmp + rename) so a reader never sees a half-written file.
pub fn atomic_write(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
