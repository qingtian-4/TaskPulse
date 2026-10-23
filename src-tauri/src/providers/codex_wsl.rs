//! WSL is a local, isolated Codex instance, not an SSH host or Windows folder.
//! Read SQLite inside Linux (UNC WAL locking is unsafe), and incrementally read
//! rollout logs through the WSL share using the shared Codex parser.
use super::{codex::CodexProvider, Provider, ProviderOutput};
use crate::domain::TaskSnapshot;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[derive(Debug, Deserialize)]
pub(super) struct WslThread {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sidebar_title: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub first_user_message: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub rollout_path: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub source: serde_json::Value,
}
#[derive(Deserialize)]
struct Metadata {
    #[serde(default)]
    root: String,
    #[serde(default)]
    threads: Vec<WslThread>,
    #[serde(default)]
    rollouts: Vec<LinuxRollout>,
    error: Option<MetadataFault>,
}
#[derive(Deserialize)]
pub(super) struct LinuxRollout {
    pub path: String,
    pub stamp: super::jsonl_tail::LinuxFileStamp,
}
#[derive(Deserialize)]
struct MetadataFault {
    stage: String,
    code: String,
}

pub fn validate_distro(distro: &str) -> Result<(), String> {
    if distro.trim().is_empty()
        || distro.starts_with('-')
        || distro
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        return Err("请输入有效的 WSL 发行版名称".into());
    }
    Ok(())
}

pub fn running_distros() -> Result<Vec<String>, String> {
    let bytes = command(&["--list", "--running", "--quiet"], Duration::from_secs(3))?;
    let text = if bytes.contains(&0) {
        String::from_utf16_lossy(
            &bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect::<Vec<_>>(),
        )
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(text
        .lines()
        .map(|s| s.trim().trim_start_matches('\u{feff}').to_owned())
        .filter(|s| !s.is_empty())
        .collect())
}

#[derive(Debug, Clone)]
pub struct WslCodexDiscovery {
    pub distro: String,
    pub root: Option<String>,
    pub detail: String,
}

fn infrastructure_distro(distro: &str) -> bool {
    matches!(
        distro.to_ascii_lowercase().as_str(),
        "docker-desktop" | "docker-desktop-data" | "rancher-desktop" | "rancher-desktop-data"
    )
}

fn discovery_root(body: &[u8]) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct Probe {
        root: String,
        evidence: Option<String>,
    }
    let probe: Probe = serde_json::from_slice(body).map_err(|_| "WSL 发现结果不可读")?;
    if !probe.root.starts_with('/') || probe.root.split('/').any(|s| s == "..") {
        return Err("WSL Codex 目录无效".into());
    }
    match probe.evidence.as_deref() {
        None => Ok(None),
        Some("thread_schema" | "session_index" | "rollout_metadata") => Ok(Some(probe.root)),
        Some(_) => Err("WSL Codex 证据类型不可识别".into()),
    }
}

fn discover_in_running_distros(
    running: &[String],
    mut probe: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Vec<WslCodexDiscovery> {
    running
        .iter()
        .map(|distro| {
            // A WSL runtime is not an Agent. Do not even enter infrastructure
            // distros for auto-discovery. Explicit saved sources stay untouched.
            let result = if infrastructure_distro(distro) {
                Ok(None)
            } else {
                validate_distro(distro).and_then(|_| probe(distro))
            };
            let (root, detail) = match result {
                Ok(Some(root)) => {
                    let detail = format!(
                        "WSL {distro} · 已验证 Codex 会话数据：{root}；来源可用不代表任务正在运行"
                    );
                    (Some(root), detail)
                }
                Ok(None) if infrastructure_distro(distro) => (
                    None,
                    format!("WSL {distro} · 内部运行环境，不自动识别为 Codex 来源"),
                ),
                Ok(None) => (
                    None,
                    format!("WSL {distro} · 未发现 Codex 会话数据；仅有 WSL 环境不算 Agent 来源"),
                ),
                Err(_) => (
                    None,
                    format!("WSL {distro} · 暂未验证 Codex 数据，请检查 Python 3、权限或连接；可手动配置"),
                ),
            };
            WslCodexDiscovery {
                distro: distro.clone(),
                root,
                detail,
            }
        })
        .collect()
}

pub fn discover_codex_distros() -> Result<Vec<WslCodexDiscovery>, String> {
    use base64::Engine;
    let running = running_distros()?;
    let program =
        base64::engine::general_purpose::STANDARD.encode(include_bytes!("codex_wsl_discovery.py"));
    let script = format!("import base64;exec(base64.b64decode('{program}'))");
    // Only the running snapshot is probed; never enumerate all installed distros
    // with --distribution, which would wake stopped environments.
    Ok(discover_in_running_distros(&running, |distro| {
        let body = command(
            &["--distribution", distro, "--exec", "python3", "-c", &script],
            Duration::from_secs(3),
        )?;
        discovery_root(&body)
    }))
}

#[cfg(windows)]
fn command(args: &[&str], timeout: Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let mut child = Command::new("wsl.exe")
        .args(args)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("[wsl:process:spawn_{}]", e.raw_os_error().unwrap_or(0)))?;
    let (tx, rx) = std::sync::mpsc::channel();
    for (err, pipe) in [
        (
            false,
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
        (
            true,
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            let _ = tx.send((err, bytes));
        });
    }
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|_| "[wsl:process:wait]")? {
            Some(s) => break s,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("[wsl:process:timeout] WSL 采集超时；保留原任务状态".into());
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    for _ in 0..2 {
        let (err, body) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "[wsl:process:output_timeout]")?;
        if err {
            stderr = body;
        } else {
            stdout = body;
        }
    }
    if !status.success() {
        let _ = stderr; // Never expose Python tracebacks, metadata or paths.
        return Err(format!(
            "[wsl:process:exit_{}]",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(stdout)
}
#[cfg(not(windows))]
fn command(_: &[&str], _: Duration) -> Result<Vec<u8>, String> {
    Err("WSL 来源仅适用于 Windows".into())
}

pub struct WslCodexProvider {
    distro: String,
    instance: String,
    root: Option<String>,
    inner: Option<CodexProvider>,
    last_authoritative_tasks: Vec<TaskSnapshot>,
    partial_tasks: Vec<TaskSnapshot>,
    last_success: Option<SystemTime>,
}
impl WslCodexProvider {
    pub fn new(distro: String, instance: String) -> Self {
        Self {
            distro,
            instance,
            root: None,
            inner: None,
            last_authoritative_tasks: vec![],
            partial_tasks: vec![],
            last_success: None,
        }
    }
    fn sample(&mut self, now: SystemTime) -> Result<ProviderOutput, String> {
        use base64::Engine;
        validate_distro(&self.distro).map_err(|_| "[wsl:configuration:invalid_distro]")?;
        if !running_distros()
            .map_err(|e| format!("[wsl:running_query] {e}"))?
            .iter()
            .any(|d| d.eq_ignore_ascii_case(&self.distro))
        {
            return Err("[wsl:running_query:stopped] 不会自动唤醒发行版".into());
        }
        let program = base64::engine::general_purpose::STANDARD
            .encode(include_bytes!("codex_wsl_metadata.py"));
        let script = format!("import base64;exec(base64.b64decode('{program}'))");
        let body = command(
            &[
                "--distribution",
                &self.distro,
                "--exec",
                "python3",
                "-c",
                &script,
            ],
            Duration::from_secs(10),
        )
        .map_err(|e| format!("[wsl:python_launch] {e}"))?;
        let meta: Metadata =
            serde_json::from_slice(&body).map_err(|_| "[wsl:metadata_decode:invalid_json]")?;
        if let Some(error) = meta.error {
            let safe = |s: &str| {
                s.len() <= 48 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            };
            return Err(if safe(&error.stage) && safe(&error.code) {
                format!("[wsl:{}:{}]", error.stage, error.code)
            } else {
                "[wsl:metadata_decode:invalid_error]".into()
            });
        }
        if !meta.root.starts_with('/') || meta.root.split('/').any(|s| s == "..") {
            return Err("[wsl:metadata_decode:invalid_root]".into());
        }
        if self.root.as_deref() != Some(&meta.root) {
            let share = PathBuf::from(format!(r"\\wsl.localhost\{}", self.distro));
            let root = share.join(meta.root.trim_start_matches('/'));
            self.inner = Some(CodexProvider::with_wsl_root(root, share, &self.instance));
            self.root = Some(meta.root);
        }
        let inner = self.inner.as_mut().ok_or("[wsl:initialization:missing]")?;
        inner.set_external_metadata(&meta.threads);
        inner.set_linux_rollouts(meta.rollouts);
        let mut output = inner.poll(now);
        for t in output.tasks.iter_mut().chain(output.catalog.iter_mut()) {
            t.source_instance = Some(self.instance.clone());
            t.remote = false;
            t.host = None;
            // Windows account quota remains independent from this Linux root.
            t.quota_5h_used_pct = None;
            t.quota_5h_resets_at = None;
            t.quota_1w_used_pct = None;
            t.quota_1w_resets_at = None;
        }
        self.accept_sample(&mut output);
        Ok(output)
    }
    fn accept_sample(&mut self, output: &mut ProviderOutput) {
        let mut regressed = false;
        for task in &mut output.tasks {
            if let Some(latest) = self
                .partial_tasks
                .iter()
                .chain(self.last_authoritative_tasks.iter())
                .filter(|old| old.id == task.id)
                .max_by_key(|old| old.updated_at)
            {
                if !task.stale && task.updated_at < latest.updated_at {
                    *task = latest.clone();
                    task.stale = true;
                    regressed = true;
                }
            }
        }
        if regressed {
            output.authoritative = false;
            output.error = Some("[wsl:snapshot:regressed] 未使用倒退的生命周期观测".into());
        }
        if output.authoritative {
            self.last_authoritative_tasks = output.tasks.clone();
            self.partial_tasks.clear();
        } else {
            // Fresh, completely parsed sessions may advance even when a
            // different file is catching up. Never resurrect an older turn or
            // Running state over such a fresh explicit terminal observation.
            for task in output.tasks.iter().filter(|t| !t.stale) {
                if let Some(old) = self.partial_tasks.iter_mut().find(|t| t.id == task.id) {
                    if task.updated_at >= old.updated_at {
                        *old = task.clone();
                    }
                } else {
                    self.partial_tasks.push(task.clone());
                }
            }
            let mut retained = self.retained_tasks();
            for task in &output.tasks {
                if let Some(old) = retained.iter_mut().find(|t| t.id == task.id) {
                    if !task.stale && task.updated_at >= old.updated_at {
                        *old = task.clone();
                    }
                } else {
                    retained.push(task.clone());
                }
            }
            output.tasks = retained;
        }
    }
    fn retained_tasks(&self) -> Vec<TaskSnapshot> {
        let mut tasks = self.last_authoritative_tasks.clone();
        for task in &self.partial_tasks {
            if let Some(old) = tasks.iter_mut().find(|t| t.id == task.id) {
                if task.updated_at >= old.updated_at {
                    *old = task.clone();
                }
            } else {
                tasks.push(task.clone());
            }
        }
        for task in &mut tasks {
            task.stale = true;
        }
        tasks
    }
}
impl Provider for WslCodexProvider {
    fn name(&self) -> &'static str {
        "codex-wsl"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        let began = std::time::Instant::now();
        let started = SystemTime::now();
        let previous_root = self.root.clone();
        let before_bytes = self
            .inner
            .as_ref()
            .and_then(CodexProvider::diagnostics)
            .and_then(|m| m["cumulative_rollout_bytes_read"].as_u64())
            .unwrap_or(0);
        let result = self.sample(now);
        let mut metadata_ok = result.is_ok();
        let mut output =
            result.unwrap_or_else(|error| ProviderOutput::degraded(self.retained_tasks(), error));
        let retried = output
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("[wsl:rollout_read:size_race]"));
        if retried {
            // One transport consistency retry; this delay never infers a turn
            // end or a human wait. Each sample rechecks the running distro.
            std::thread::sleep(Duration::from_millis(40));
            let result = self.sample(now);
            metadata_ok = result.is_ok();
            output = result
                .unwrap_or_else(|error| ProviderOutput::degraded(self.retained_tasks(), error));
        }
        if output.authoritative {
            self.last_success = Some(SystemTime::now());
        }
        let code = output
            .error
            .as_deref()
            .and_then(|e| e.find(']').map(|end| &e[..=end]))
            .unwrap_or("none");
        let stage = code
            .trim_matches(['[', ']'])
            .split(':')
            .nth(1)
            .unwrap_or("none");
        let error_codes: Vec<_> = output
            .error
            .as_deref()
            .unwrap_or("")
            .split_inclusive(']')
            .map(str::trim)
            .filter(|s| {
                s.starts_with("[wsl:")
                    && s.ends_with(']')
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"[]:_-".contains(&b))
            })
            .take(4)
            .collect();
        let rollout_metrics = self.inner.as_ref().and_then(CodexProvider::diagnostics);
        let after_bytes = rollout_metrics
            .as_ref()
            .and_then(|m| m["cumulative_rollout_bytes_read"].as_u64())
            .unwrap_or(0);
        let cycle_bytes = if previous_root == self.root {
            after_bytes.saturating_sub(before_bytes)
        } else {
            after_bytes
        };
        super::resources::continuity(
            "provider_poll",
            &self.instance,
            &output.tasks,
            serde_json::json!({
                "poll_start_ms":super::resources::epoch_ms(started),"poll_end_ms":super::resources::epoch_ms(SystemTime::now()),
                "duration_ms":began.elapsed().as_millis(),"authoritative":output.authoritative,"degraded":!output.authoritative,
                "error_stage":stage,"error_code":code,"error_codes":error_codes,"size_race_retried":retried,"last_success_ms":self.last_success.map(super::resources::epoch_ms),
                "authoritative_baseline_count":self.last_authoritative_tasks.len(),"partial_count":self.partial_tasks.len(),
                "cycle_rollout_bytes":cycle_bytes,"rollout_observed_this_cycle":metadata_ok,"rollout_metrics":rollout_metrics
            }),
        );
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wsl_continuity_partial_empty_does_not_erase_the_trusted_running_session() {
        let mut provider = WslCodexProvider::new("-invalid".into(), "codex-wsl:test".into());
        let task = TaskSnapshot::new(
            "codex-wsl:test:codex:s",
            "native",
            crate::domain::TaskSource::Codex,
            crate::domain::TaskStatus::Running,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        provider.accept_sample(&mut ProviderOutput::tasks(vec![task]));
        let mut partial = ProviderOutput::degraded(vec![], "rollout catch-up");
        provider.accept_sample(&mut partial);
        assert_eq!(
            partial.tasks.len(),
            1,
            "partial observations must carry the known task as stale"
        );
        let failed = provider.poll(SystemTime::now());
        assert_eq!(failed.tasks.len(), 1);
        assert!(failed.tasks[0].stale);
        assert_eq!(failed.tasks[0].status, crate::domain::TaskStatus::Running);
    }
    #[test]
    fn wsl_discovery_probes_only_user_distros_in_the_running_snapshot() {
        let running: Vec<String> = [
            "Ubuntu",
            "DOCKER-DESKTOP",
            "docker-desktop-data",
            "rancher-desktop",
            "rancher-desktop-data",
            "Debian",
            "Ubuntu-Docker",
            "Fedora",
            "-invalid",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let mut probed = vec![];
        let found = discover_in_running_distros(&running, |name| {
            probed.push(name.to_string());
            match name {
                "Ubuntu" => Ok(Some("/home/user/.codex".into())),
                "Fedora" => Err("Python missing or timed out".into()),
                _ => Ok(None),
            }
        });
        assert_eq!(probed, ["Ubuntu", "Debian", "Ubuntu-Docker", "Fedora"]);
        assert!(!probed.iter().any(|d| d == "Stopped-Ubuntu"));
        let available: Vec<_> = found.iter().filter(|f| f.root.is_some()).collect();
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].distro, "Ubuntu");
        assert!(available[0].detail.contains("不代表任务正在运行"));
        assert!(found[1].detail.contains("内部运行环境"));
        assert!(found[5].detail.contains("未发现 Codex"));
        assert!(found[7].detail.contains("暂未验证"));
    }

    #[test]
    fn wsl_discovery_requires_recognized_session_evidence_and_a_linux_root() {
        assert_eq!(
            discovery_root(br#"{"root":"/home/user/.codex","evidence":null}"#).unwrap(),
            None
        );
        for evidence in ["thread_schema", "session_index", "rollout_metadata"] {
            let body = serde_json::json!({"root":"/custom/codex", "evidence":evidence});
            assert_eq!(
                discovery_root(&serde_json::to_vec(&body).unwrap()).unwrap(),
                Some("/custom/codex".into())
            );
        }
        for body in [
            r#"{"root":"/home/user/.codex","evidence":"installed_executable"}"#,
            r#"{"root":"relative","evidence":"thread_schema"}"#,
            r#"{"root":"/home/../etc","evidence":"thread_schema"}"#,
            r#"{"evidence":"thread_schema"}"#,
            "not JSON",
        ] {
            assert!(discovery_root(body.as_bytes()).is_err());
        }
    }

    #[test]
    fn distro_is_argument_not_shell_or_unc_path() {
        assert!(validate_distro("Ubuntu").is_ok());
        assert!(validate_distro("Ubuntu Test").is_ok());
        for name in ["", "-d", "../Ubuntu", "Ubuntu\\home", "Ubuntu\n"] {
            assert!(validate_distro(name).is_err());
        }
    }
    #[test]
    fn failed_or_stopped_wsl_preserves_running_turn_and_does_not_finish_it() {
        let mut provider = WslCodexProvider::new("-invalid".into(), "codex-wsl:test".into());
        let mut task = TaskSnapshot::new(
            "codex-wsl:test:codex:s",
            "native title",
            crate::domain::TaskSource::Codex,
            crate::domain::TaskStatus::Running,
            crate::domain::DetectionConfidence::Hook,
            SystemTime::now(),
        );
        task.session_id = Some("s".into());
        provider.last_authoritative_tasks = vec![task];
        let output = provider.poll(SystemTime::now());
        assert!(!output.authoritative);
        assert_eq!(output.tasks[0].status, crate::domain::TaskStatus::Running);
        assert!(output.tasks[0].stale);
        assert!(output.tasks[0].finished_at.is_none());
    }
    #[test]
    #[ignore = "read-only WSL discovery smoke; requires a running Codex distro"]
    fn live_wsl_codex_discovery() {
        let distro = std::env::var("TASKPULSE_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu".into());
        let started = std::time::Instant::now();
        let found = discover_codex_distros().unwrap();
        let source = found
            .iter()
            .find(|source| source.distro.eq_ignore_ascii_case(&distro))
            .expect("expected distro is not running; smoke never starts it");
        assert!(source.root.is_some(), "{}", source.detail);
        assert!(found
            .iter()
            .filter(|source| infrastructure_distro(&source.distro))
            .all(|source| source.root.is_none()));
        println!(
            "{}",
            serde_json::json!({
                "discovery_ms": started.elapsed().as_millis(),
                "sources": found.iter().map(|source| serde_json::json!({
                    "distro":source.distro, "codex_verified":source.root.is_some()
                })).collect::<Vec<_>>()
            })
        );
    }

    #[test]
    #[ignore = "read-only WSL Codex smoke; requires a running installed distro"]
    fn live_wsl_codex_snapshot() {
        let distro = std::env::var("TASKPULSE_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu".into());
        let mut provider = WslCodexProvider::new(
            distro.clone(),
            format!("codex-wsl:{}", distro.to_lowercase()),
        );
        let started = std::time::Instant::now();
        let mut output = provider.poll(SystemTime::now());
        let first_ms = started.elapsed().as_millis();
        while !output.authoritative
            && started.elapsed() < Duration::from_secs(30)
            && output.error.as_deref().is_some_and(|e| e.contains("追赶"))
        {
            // Large real logs legitimately need multiple bounded worker turns.
            // Keep the normal worker cadence; failures/gaps still fail below.
            std::thread::sleep(Duration::from_secs(1));
            output = provider.poll(SystemTime::now());
        }
        assert!(output.authoritative, "{:?}", output.error);
        assert!(!output.catalog.is_empty());
        for task in &output.catalog {
            assert!(task.session_id.is_some());
            assert!(!task.remote);
            assert!(task.id.starts_with("codex-wsl:"));
        }
        println!(
            "{}",
            serde_json::json!({"distro":distro,"first_poll_ms":first_ms,"caught_up_ms":started.elapsed().as_millis(),"catalog":output.catalog.iter().map(|t|serde_json::json!({"id":t.id,"name":t.name,"project":t.project,"path":t.project_path})).collect::<Vec<_>>(),"tasks":output.tasks.iter().map(|t|serde_json::json!({"id":t.id,"name":t.name,"status":t.status,"phase":t.phase})).collect::<Vec<_>>()})
        );
        let started = std::time::Instant::now();
        let next = provider.poll(SystemTime::now());
        assert!(next.authoritative, "{:?}", next.error);
        assert_eq!(next.catalog.len(), output.catalog.len());
        println!("second_ms={}", started.elapsed().as_millis());
    }
}

#[cfg(test)]
mod continuity_tests;
