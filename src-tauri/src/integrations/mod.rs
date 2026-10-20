//! Versioned, local-only observational integrations. No plugin code execution.
pub mod api;
pub mod bridge;
pub mod configuration;
pub mod journal;
pub mod manifest;
pub mod reducer;
pub mod schema;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IntegrationSource {
    pub agent_id: String,
    pub adapter_id: String,
    #[serde(default = "local_transport")]
    pub transport: String,
    #[serde(default)]
    pub host_version: Option<String>,
    #[serde(default)]
    pub accept_since_ms: u64,
}
fn local_transport() -> String {
    "local".into()
}

pub fn key(value: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn home() -> std::path::PathBuf {
    crate::providers::remote_tmux::config::config_dir().join("integrations")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
