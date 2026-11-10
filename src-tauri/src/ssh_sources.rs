//! Management-only SSH discovery/probing. No credentials, writes or Agent execution.
use crate::providers::remote_tmux::ssh::{SshExecutor, SystemSshExecutor};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && !host.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn parse_hosts(text: &str) -> (Vec<String>, Vec<String>) {
    let mut hosts = Vec::new();
    let mut includes = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        if key.eq_ignore_ascii_case("host") {
            hosts.extend(
                parts
                    .map(|s| s.trim_matches(['\'', '"']))
                    .filter(|s| valid_host(s) && !s.contains(['*', '?', '!']))
                    .map(str::to_string),
            );
        } else if key.eq_ignore_ascii_case("include") {
            includes.extend(parts.map(|s| s.trim_matches(['\'', '"']).to_string()));
        }
    }
    (hosts, includes)
}

fn read_hosts(root: &Path) -> Vec<String> {
    let mut queue = vec![(root.join("config"), 0)];
    let mut visited = HashSet::new();
    let mut hosts = Vec::new();
    let mut bytes = 0usize;
    while let Some((path, depth)) = queue.pop() {
        if visited.len() >= 16 || bytes >= 1024 * 1024 {
            break;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        let Ok(meta) = path.metadata() else { continue };
        if !meta.is_file() || meta.len() > 128 * 1024 {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut text = String::new();
        if file.take(128 * 1024 + 1).read_to_string(&mut text).is_err() || text.len() > 128 * 1024 {
            continue;
        }
        bytes += text.len();
        let (found, includes) = parse_hosts(&text);
        for host in found {
            if !hosts.contains(&host) && hosts.len() < 128 {
                hosts.push(host);
            }
        }
        if depth >= 4 {
            continue;
        }
        for pattern in includes.into_iter().take(16) {
            let path = if let Some(rest) = pattern.strip_prefix("~/") {
                root.parent().unwrap_or(root).join(rest)
            } else if Path::new(&pattern).is_absolute() {
                PathBuf::from(pattern)
            } else {
                root.join(pattern)
            };
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.contains(['*', '?']) {
                if let Some(parent) = path.parent() {
                    if let Ok(entries) = parent.read_dir() {
                        for entry in entries.take(128).flatten() {
                            if crate::providers::remote_tmux::config::wildcard_match(
                                name,
                                &entry.file_name().to_string_lossy(),
                            ) {
                                queue.push((entry.path(), depth + 1));
                            }
                        }
                    }
                }
            } else {
                queue.push((path, depth + 1));
            }
        }
    }
    hosts.sort();
    hosts
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentProbe {
    pub state: String,
    pub data_detected: bool,
    pub projects: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SshProbe {
    pub host: String,
    pub checked_at: u64,
    pub connection: String,
    pub codex: AgentProbe,
    pub claude_code: AgentProbe,
    pub message: String,
}

fn probe(executor: &dyn SshExecutor, host: &str) -> Result<SshProbe, String> {
    if !valid_host(host) {
        return Err("请输入有效的 SSH Host".into());
    }
    let empty = || AgentProbe {
        state: "not_checked".into(),
        data_detected: false,
        projects: vec![],
    };
    let mut report = SshProbe {
        host: host.into(),
        checked_at: crate::integrations::now_ms(),
        connection: "disconnected".into(),
        codex: empty(),
        claude_code: empty(),
        message: "SSH 未连接，请检查 Host、密钥和系统终端连接".into(),
    };
    if executor
        .run(host, "printf TASKPULSE_SSH_READY", Duration::from_secs(5))
        .ok()
        .as_deref()
        != Some("TASKPULSE_SSH_READY")
    {
        return Ok(report);
    }
    report.connection = "connected".into();
    report.message = "SSH 已连接；Agent 数据读取能力单独检测".into();
    let payload =
        base64::engine::general_purpose::STANDARD.encode(include_str!("ssh_sources_probe.py"));
    let command = format!("echo {payload} | base64 -d | python3");
    report.codex.state = "read_error".into();
    if let Ok(output) = executor.run(host, &command, Duration::from_secs(5)) {
        if output.len() <= 512 * 1024 {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&output) {
                if value["schema_version"] == 1 {
                    if let Ok(agent) = serde_json::from_value::<AgentProbe>(value["codex"].clone())
                    {
                        if [
                            "readable",
                            "empty",
                            "missing",
                            "read_error",
                            "unsupported_format",
                            "partial",
                        ]
                        .contains(&agent.state.as_str())
                            && agent.projects.len() <= 128
                            && agent
                                .projects
                                .iter()
                                .all(|p| p.len() <= 4096 && !p.chars().any(char::is_control))
                        {
                            report.codex = agent;
                        }
                    }
                }
            }
        }
    }
    report.claude_code = crate::providers::claude_code::ssh::probe(executor, host);
    Ok(report)
}

#[tauri::command]
pub async fn discover_ssh_hosts() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(|| {
        dirs_next::home_dir()
            .map(|p| read_hosts(&p.join(".ssh")))
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub async fn probe_ssh_source(host: String) -> Result<SshProbe, String> {
    tauri::async_runtime::spawn_blocking(move || probe(&SystemSshExecutor::new(), &host))
        .await
        .map_err(|_| "SSH 检测未完成".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::remote_tmux::ssh::FakeSshExecutor;
    #[test]
    fn aliases_exclude_patterns_and_credentials() {
        let (hosts, includes) = parse_hosts("Host alpha beta\n User private\n IdentityFile ~/.ssh/key\nHost * !gamma\nInclude config.d/*.conf\n");
        assert_eq!(hosts, ["alpha", "beta"]);
        assert_eq!(includes, ["config.d/*.conf"]);
    }
    #[test]
    fn include_and_limits_are_read_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("config.d")).unwrap();
        std::fs::write(dir.path().join("config"), "Host a\nInclude config.d/*.conf").unwrap();
        std::fs::write(
            dir.path().join("config.d/one.conf"),
            "Host b\nInclude config",
        )
        .unwrap();
        assert_eq!(read_hosts(dir.path()), ["a", "b"]);
    }
    #[test]
    fn connection_is_not_agent_readability() {
        let ssh = FakeSshExecutor::new()
            .with("host", "TASKPULSE_SSH_READY")
            .script("host", Err("no python"));
        let result = probe(&ssh, "host").unwrap();
        assert_eq!(result.connection, "connected");
        assert_eq!(result.codex.state, "read_error");
        assert_eq!(result.claude_code.state, "read_error");
        let failed = probe(&FakeSshExecutor::new().failing("host"), "host").unwrap();
        assert_eq!(failed.codex.state, "not_checked");
        assert!(probe(&ssh, "-oProxyCommand=bad").is_err());
        assert!(probe(&ssh, "host\nother").is_err());
    }
    #[test]
    fn bounded_valid_data_is_separate_from_empty() {
        let text =
            r#"{"schema_version":1,"codex":{"state":"empty","data_detected":false,"projects":[]}}"#;
        let ssh = FakeSshExecutor::new()
            .with("host", "TASKPULSE_SSH_READY")
            .with("host", text);
        let result = probe(&ssh, "host").unwrap();
        assert_eq!(result.codex.state, "empty");
        assert!(!result.codex.data_detected);
    }
}
