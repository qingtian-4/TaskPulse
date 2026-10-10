//! Non-invasive Claude Code discovery. Native session/log formats are optional,
//! version-tolerant evidence; documented hooks supply precise lifecycle events.
//! Only sessions/*.json is read. Credentials (*.key) are never opened.
use super::jsonl_tail::JsonlTail;
use crate::domain::{DetectionConfidence, TaskSnapshot, TaskSource, TaskStatus, WaitingReason};
use crate::providers::{Provider, ProviderOutput};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn default_root() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_next::home_dir().unwrap_or_default().join(".claude"))
}

#[derive(Default)]
struct Transcript {
    tail: JsonlTail,
    task: Option<TaskSnapshot>,
    manual_title: Option<String>,
    native_title: Option<String>,
    first_prompt: Option<String>,
}

pub struct ClaudeCodeProvider {
    root: PathBuf,
    events: PathBuf,
    transcripts: HashMap<PathBuf, Transcript>,
    last_tasks: Vec<TaskSnapshot>,
}

impl ClaudeCodeProvider {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            events: dirs_next::home_dir()
                .unwrap_or_default()
                .join(".taskpulse/claude-events"),
            transcripts: HashMap::new(),
            last_tasks: vec![],
        }
    }
    fn sample(&mut self, now: SystemTime) -> std::io::Result<ProviderOutput> {
        let mut live = HashMap::new();
        for path in json_files(&self.root.join("sessions"))? {
            let Ok(value) = read_json(&path) else {
                continue;
            };
            let Some(id) = text(&value, "sessionId") else {
                continue;
            };
            let Some(pid) = value
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|p| u32::try_from(p).ok())
            else {
                continue;
            };
            if process_alive(pid) {
                live.insert(id.to_string(), value);
            }
        }
        let mut logs = vec![];
        let projects = self.root.join("projects");
        if projects.exists() {
            for entry in std::fs::read_dir(projects)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                for file in std::fs::read_dir(entry.path())? {
                    let file = file?;
                    if !file.file_type()?.is_file()
                        || file.path().extension().and_then(|x| x.to_str()) != Some("jsonl")
                    {
                        continue;
                    }
                    let id = file
                        .path()
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let recent = file.metadata()?.modified().ok().is_some_and(|at| {
                        now.duration_since(at).unwrap_or_default() < Duration::from_secs(7 * 86400)
                    });
                    if recent
                        || live.contains_key(&id)
                        || self.transcripts.contains_key(&file.path())
                    {
                        logs.push(file.path());
                    }
                }
            }
        }
        let mut tasks = vec![];
        let mut failed = false;
        for path in &logs {
            let id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let transcript = self.transcripts.entry(path.clone()).or_default();
            match transcript.tail.read(path) {
                Ok(batch) => {
                    if batch.reset {
                        transcript.task = None;
                        transcript.manual_title = None;
                        transcript.native_title = None;
                        transcript.first_prompt = None;
                    }
                    for line in batch.body.lines() {
                        if let Ok(value) = serde_json::from_str::<Value>(line) {
                            apply_record(transcript, &value, &id, now);
                        }
                    }
                    if transcript.task.is_none() && path.metadata()?.len() > 0 {
                        failed = true;
                    }
                }
                Err(_) => failed = true,
            }
            let Some(mut task) = transcript.task.clone() else {
                continue;
            };
            task.name = transcript
                .manual_title
                .as_ref()
                .or(transcript.native_title.as_ref())
                .or(transcript.first_prompt.as_ref())
                .cloned()
                .unwrap_or_else(|| {
                    format!("未命名任务 · {}", id.chars().take(8).collect::<String>())
                });
            if let Some(lease) = live.get(&id) {
                task.pid = lease
                    .get("pid")
                    .and_then(Value::as_u64)
                    .and_then(|p| u32::try_from(p).ok());
                if let Some(cwd) = text(lease, "cwd") {
                    set_project(&mut task, cwd);
                }
                let at = millis(lease.get("updatedAt"));
                if at.is_some_and(|at| at >= task.updated_at) {
                    match text(lease, "status") {
                        Some("busy") => {
                            task.status = TaskStatus::Running;
                            task.waiting_reason = None;
                            task.confidence = DetectionConfidence::Inferred;
                        }
                        Some("idle")
                            if task.turn_key.is_some() && task.status == TaskStatus::Running =>
                        {
                            task.status = TaskStatus::WaitingInput;
                            task.waiting_reason = Some(WaitingReason::TurnEnded);
                            task.activity = None;
                            task.confidence = DetectionConfidence::Inferred;
                        }
                        _ => {}
                    }
                }
            }
            if let Ok(event) = read_json(&self.events.join(format!("{id}.json"))) {
                if text(&event, "session_id") == Some(id.as_str())
                    && event_belongs_to_root(&event, &self.root)
                {
                    reconcile_hook_turn(&mut task, &event);
                    if millis(event.get("observed_at")).is_some_and(|at| at >= task.updated_at) {
                        apply_hook(&mut task, &event);
                    }
                }
            }
            // Neither an old busy log nor a retained hook proves liveness.
            if task.status == TaskStatus::Running && !live.contains_key(&id) {
                task.stale = now.duration_since(task.updated_at).unwrap_or_default()
                    > Duration::from_secs(30);
            }
            task.terminal = ide_terminal(&self.root, task.project_path.as_deref());
            task.can_focus = task.terminal.is_some();
            // Do not bootstrap old truncated/incomplete history as live work.
            // Previously observed tasks retain their last state as stale.
            if task.status == TaskStatus::Running
                && task.stale
                && !self.last_tasks.iter().any(|previous| {
                    previous.id == task.id && previous.status == TaskStatus::Running
                })
            {
                task.status = TaskStatus::Unknown;
            }
            tasks.push(task);
        }
        // Hooks can precede the first transcript write.
        for path in json_files(&self.events)? {
            let Ok(event) = read_json(&path) else {
                continue;
            };
            if !event_belongs_to_root(&event, &self.root) {
                continue;
            }
            let Some(id) = text(&event, "session_id") else {
                continue;
            };
            if tasks.iter().any(|t| t.session_id.as_deref() == Some(id)) {
                continue;
            }
            let mut task = TaskSnapshot::new(
                format!("claude:{id}"),
                format!("未命名任务 · {}", id.chars().take(8).collect::<String>()),
                TaskSource::ClaudeCode,
                TaskStatus::Unknown,
                DetectionConfidence::Hook,
                now,
            );
            task.session_id = Some(id.into());
            apply_hook(&mut task, &event);
            if task.status == TaskStatus::Running && !live.contains_key(id) {
                task.stale = now.duration_since(task.updated_at).unwrap_or_default()
                    > Duration::from_secs(30);
                if task.stale
                    && !self.last_tasks.iter().any(|previous| {
                        previous.id == task.id && previous.status == TaskStatus::Running
                    })
                {
                    task.status = TaskStatus::Unknown;
                }
            }
            tasks.push(task);
        }
        if failed {
            for previous in &self.last_tasks {
                if !tasks.iter().any(|task| task.id == previous.id) {
                    let mut retained = previous.clone();
                    retained.stale = true;
                    tasks.push(retained);
                }
            }
        }
        self.transcripts.retain(|p, _| logs.contains(p));
        self.last_tasks = tasks.clone();
        let catalog = tasks.clone();
        Ok(ProviderOutput {
            tasks,
            catalog,
            authoritative: !failed,
            error: failed.then(|| "Claude 会话暂不可读；保留上次观测".into()),
            ..Default::default()
        })
    }
}

impl Provider for ClaudeCodeProvider {
    fn name(&self) -> &'static str {
        "claude-code"
    }
    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        self.sample(now).unwrap_or_else(|error| {
            ProviderOutput::degraded(
                self.last_tasks
                    .iter()
                    .cloned()
                    .map(|mut t| {
                        t.stale = true;
                        t
                    })
                    .collect(),
                error.to_string(),
            )
        })
    }
}

fn apply_record(state: &mut Transcript, value: &Value, id: &str, now: SystemTime) {
    if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return;
    }
    if !matches!(
        text(value, "type"),
        Some("user" | "assistant" | "custom-title" | "ai-title")
    ) && !(text(value, "type") == Some("system")
        && text(value, "subtype") == Some("turn_duration"))
    {
        return;
    }
    let task = state.task.get_or_insert_with(|| {
        let mut t = TaskSnapshot::new(
            format!("claude:{id}"),
            "",
            TaskSource::ClaudeCode,
            TaskStatus::Idle,
            DetectionConfidence::Inferred,
            now,
        );
        t.session_id = Some(id.into());
        t.updated_at = UNIX_EPOCH;
        t
    });
    if let Some(cwd) = text(value, "cwd") {
        set_project(task, cwd);
    }
    let at = text(value, "timestamp").and_then(super::codex::parse_rfc3339);
    match text(value, "type") {
        Some("custom-title") => state.manual_title = text(value, "customTitle").map(str::to_owned),
        Some("ai-title") => state.native_title = text(value, "aiTitle").map(str::to_owned),
        Some("user") if value.get("toolUseResult").is_none() => {
            let content = value.pointer("/message/content");
            let prompt = content
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    content.and_then(Value::as_array).and_then(|items| {
                        items
                            .iter()
                            .find_map(|v| text(v, "text").map(str::to_owned))
                    })
                });
            if let Some(prompt) = prompt {
                if state.first_prompt.is_none() {
                    state.first_prompt = Some(
                        prompt
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(72)
                            .collect(),
                    );
                }
                task.turn_key = text(value, "promptId")
                    .or_else(|| text(value, "uuid"))
                    .map(str::to_owned);
                task.started_at = at;
                task.updated_at = at.unwrap_or(task.updated_at);
                task.status = TaskStatus::Running;
                task.waiting_reason = None;
                task.stale = false;
                task.activity = None;
            }
        }
        Some("assistant") => {
            let api_error = value.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true);
            if api_error {
                task.status = TaskStatus::Failed;
                task.waiting_reason = None;
                task.activity = None;
                task.stale = false;
            } else if task.status == TaskStatus::Failed {
                task.status = TaskStatus::Running;
            }
            if let Some(model) = value
                .pointer("/message/model")
                .and_then(Value::as_str)
                .filter(|model| !model.starts_with('<'))
            {
                task.model = Some(model.into());
            }
            if let Some(at) = at {
                task.updated_at = at;
            }
            if let Some(items) = value.pointer("/message/content").and_then(Value::as_array) {
                for item in items {
                    if text(item, "type") == Some("tool_use") {
                        let name = text(item, "name").unwrap_or("工具");
                        let cmd = item
                            .pointer("/input/command")
                            .or_else(|| item.pointer("/input/file_path"))
                            .and_then(Value::as_str);
                        task.activity = Some(match cmd {
                            Some(cmd) => {
                                format!("{name} · {}", cmd.chars().take(300).collect::<String>())
                            }
                            None => name.into(),
                        });
                    }
                    if let Some(message) = text(item, "text") {
                        task.message = Some(message.chars().take(500).collect());
                    }
                }
            }
            if !api_error
                && value
                    .pointer("/message/stop_reason")
                    .and_then(Value::as_str)
                    == Some("end_turn")
            {
                task.status = TaskStatus::WaitingInput;
                task.waiting_reason = Some(WaitingReason::TurnEnded);
                task.activity = None;
            }
        }
        Some("system")
            if text(value, "subtype") == Some("turn_duration")
                && task.status != TaskStatus::Failed =>
        {
            task.status = TaskStatus::WaitingInput;
            task.waiting_reason = Some(WaitingReason::TurnEnded);
            task.activity = None;
            if let Some(at) = at {
                task.updated_at = at;
            }
        }
        _ => {}
    }
}

/// Hook input has no documented prompt UUID. Correlate the adjacent native
/// prompt timestamp, but never carry an old hook across a newer user prompt.
fn reconcile_hook_turn(task: &mut TaskSnapshot, event: &Value) {
    let (Some(native_start), Some(hook_start), Some(observed), Some(turn)) = (
        task.started_at,
        millis(event.get("turn_started_at")),
        millis(event.get("observed_at")),
        text(event, "turn_key"),
    ) else {
        return;
    };
    let distance = native_start
        .duration_since(hook_start)
        .or_else(|_| hook_start.duration_since(native_start))
        .unwrap_or_default();
    if distance <= Duration::from_secs(2) && observed >= native_start {
        task.turn_key = Some(turn.into());
        task.started_at = Some(hook_start);
    }
}

fn apply_hook(task: &mut TaskSnapshot, event: &Value) {
    let name = text(event, "hook_event_name").unwrap_or("");
    match name {
        "UserPromptSubmit" => {
            task.status = TaskStatus::Running;
            task.waiting_reason = None;
            task.started_at = millis(event.get("observed_at"));
            task.activity = None;
        }
        "PreToolUse" | "PostToolUse" => {
            task.status = TaskStatus::Running;
            task.waiting_reason = None;
            task.activity = text(event, "activity").map(str::to_owned);
        }
        "PermissionRequest" => {
            task.status = TaskStatus::WaitingInput;
            task.waiting_reason = Some(WaitingReason::Approval);
            task.input_request_id = text(event, "tool_use_id").map(str::to_owned);
        }
        "Stop" => {
            task.status = TaskStatus::WaitingInput;
            task.waiting_reason = Some(WaitingReason::TurnEnded);
            task.activity = None;
        }
        "StopFailure" => {
            task.status = TaskStatus::Failed;
            task.activity = None;
        }
        "Notification" if text(event, "notification_type") == Some("permission_prompt") => {
            task.status = TaskStatus::WaitingInput;
            task.waiting_reason = Some(WaitingReason::Approval);
        }
        _ => return,
    }
    if name == "UserPromptSubmit" || task.turn_key.is_none() {
        if let Some(turn) = text(event, "turn_key") {
            task.turn_key = Some(turn.into());
        }
    }
    if let Some(cwd) = text(event, "cwd") {
        set_project(task, cwd);
    }
    if let Some(at) = millis(event.get("observed_at")) {
        task.updated_at = at;
    }
    task.confidence = DetectionConfidence::Hook;
    task.stale = false;
    task.hook_event = Some(name.into());
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}
fn event_belongs_to_root(event: &Value, root: &Path) -> bool {
    let origin = text(event, "config_dir")
        .map(PathBuf::from)
        .unwrap_or_else(default_root);
    let origin = origin.canonicalize().unwrap_or(origin);
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if cfg!(windows) {
        origin
            .to_string_lossy()
            .eq_ignore_ascii_case(&root.to_string_lossy())
    } else {
        origin == root
    }
}
fn millis(value: Option<&Value>) -> Option<SystemTime> {
    value?
        .as_u64()
        .map(|ms| UNIX_EPOCH + Duration::from_millis(ms))
}
fn read_json(path: &Path) -> std::io::Result<Value> {
    serde_json::from_slice(&std::fs::read(path)?).map_err(std::io::Error::other)
}
fn json_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut files = vec![];
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|x| x.to_str()) == Some("json")
        {
            files.push(entry.path());
        }
    }
    Ok(files)
}
fn set_project(task: &mut TaskSnapshot, cwd: &str) {
    task.project_path = Some(cwd.into());
    task.project = cwd
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .map(str::to_owned);
}
fn ide_terminal(root: &Path, cwd: Option<&str>) -> Option<crate::watcher::TerminalInfo> {
    if !cfg!(windows) {
        return None;
    }
    let cwd = cwd?.replace('\\', "/").to_lowercase();
    let mut matches = vec![];
    for entry in std::fs::read_dir(root.join("ide")).ok()? {
        let entry = entry.ok()?;
        if entry.path().extension().and_then(|x| x.to_str()) != Some("lock") {
            continue;
        }
        let Ok(value) = read_json(&entry.path()) else {
            continue;
        };
        let Some(pid) = value
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
        else {
            continue;
        };
        if !process_alive(pid) {
            continue;
        }
        if text(&value, "ideName") != Some("Visual Studio Code") {
            continue;
        }
        if value
            .get("workspaceFolders")
            .and_then(Value::as_array)
            .is_some_and(|folders| {
                folders.iter().any(|v| {
                    v.as_str()
                        .is_some_and(|s| s.replace('\\', "/").to_lowercase() == cwd)
                })
            })
        {
            matches.push(pid);
        }
    }
    let unique: HashSet<_> = matches.into_iter().collect();
    if unique.len() != 1 {
        return None;
    }
    let pid = *unique.iter().next()?;
    Some(crate::watcher::TerminalInfo {
        kind: if cfg!(windows) {
            "windows_native"
        } else {
            "vscode"
        }
        .into(),
        focus_id: pid.to_string(),
        outer_id: String::new(),
        label: "Visual Studio Code（项目窗口）".into(),
        window_title: None,
    })
}

pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn GetExitCodeProcess(handle: *mut std::ffi::c_void, code: *mut u32) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        // Query-only process handle, released immediately after observing liveness.
        unsafe {
            let handle = OpenProcess(0x1000, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0;
            let alive = GetExitCodeProcess(handle, &mut code) != 0 && code == 259;
            CloseHandle(handle);
            alive
        }
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_old_logs_never_become_running_and_corruption_retains_last_state() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("projects/project");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stable.jsonl");
        std::fs::write(&path,b"{\"type\":\"user\",\"uuid\":\"turn\",\"timestamp\":\"2020-01-01T00:00:00Z\",\"message\":{\"content\":\"hello\"}}\n").unwrap();
        let mut provider = ClaudeCodeProvider::new(root.path().into());
        provider.events = root.path().join("events");
        for _ in 0..3 {
            let output = provider.poll(SystemTime::now());
            assert_eq!(output.tasks[0].status, TaskStatus::Unknown);
            assert!(output.tasks[0].stale);
        }
        std::fs::OpenOptions::new().append(true).open(&path).unwrap()
            .write_all(b"{\"type\":\"assistant\",\"isApiErrorMessage\":true,\"error\":\"authentication_failed\",\"message\":{\"model\":\"<synthetic>\"}}\n").unwrap();
        let output = provider.poll(SystemTime::now());
        assert_eq!(output.tasks[0].status, TaskStatus::Failed);
        assert_eq!(output.tasks[0].exit_code, None);
        assert_eq!(output.tasks[0].model, None);
        std::fs::write(&path, b"{\"type\":").unwrap();
        let broken = provider.poll(SystemTime::now());
        assert!(!broken.authoritative);
        assert_eq!(broken.tasks[0].status, TaskStatus::Failed);
        assert!(broken.tasks[0].stale);
    }

    #[test]
    fn unknown_records_do_not_invent_tasks_and_old_hooks_do_not_change_new_turn() {
        let mut state = Transcript::default();
        apply_record(
            &mut state,
            &serde_json::json!({"type":"future-format"}),
            "s",
            SystemTime::now(),
        );
        assert!(state.task.is_none());
        let mut task = TaskSnapshot::new(
            "claude:s",
            "title",
            TaskSource::ClaudeCode,
            TaskStatus::Running,
            DetectionConfidence::Inferred,
            SystemTime::now(),
        );
        task.started_at = Some(UNIX_EPOCH + Duration::from_millis(1000));
        task.turn_key = Some("native-prompt".into());
        let event =
            serde_json::json!({"turn_started_at":1020,"observed_at":2000,"turn_key":"prompt:1020"});
        reconcile_hook_turn(&mut task, &event);
        assert_eq!(task.turn_key.as_deref(), Some("prompt:1020"));
        task.started_at = Some(UNIX_EPOCH + Duration::from_millis(2100));
        task.turn_key = Some("new-native-prompt".into());
        reconcile_hook_turn(&mut task, &event);
        assert_eq!(task.turn_key.as_deref(), Some("new-native-prompt"));
    }

    #[test]
    #[ignore = "read-only smoke against an installed Claude Code data source"]
    fn live_claude_snapshot() {
        let mut provider = ClaudeCodeProvider::new(default_root());
        let output = provider.poll(SystemTime::now());
        assert!(output.authoritative, "{:?}", output.error);
        assert!(
            !output.tasks.is_empty(),
            "No recent Claude sessions; this smoke requires actual local data"
        );
        for task in &output.tasks {
            assert!(task.session_id.is_some());
            assert!(task.id.starts_with("claude:"));
            println!(
                "{}",
                serde_json::json!({"id":task.id,"title":task.name,"project":task.project,"model":task.model,"status":task.status,"confidence":task.confidence,"stale":task.stale})
            );
        }
        let next = provider.poll(SystemTime::now());
        assert_eq!(
            output.tasks.iter().map(|t| &t.id).collect::<Vec<_>>(),
            next.tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rename_and_model_switch_preserve_session_and_turn() {
        let mut state = Transcript::default();
        let now = SystemTime::now();
        for value in [
            serde_json::json!({"type":"user","uuid":"turn-1","message":{"content":"Fix tests"}}),
            serde_json::json!({"type":"ai-title","aiTitle":"Native title"}),
            serde_json::json!({"type":"custom-title","customTitle":"Manual title"}),
            serde_json::json!({"type":"assistant","message":{"model":"deepseek-flash","stop_reason":"end_turn"}}),
        ] {
            apply_record(&mut state, &value, "stable", now);
        }
        let before = state.task.clone().unwrap();
        apply_record(
            &mut state,
            &serde_json::json!({"type":"assistant","message":{"model":"another-model"}}),
            "stable",
            now,
        );
        let after = state.task.unwrap();
        assert_eq!(before.id, after.id);
        assert_eq!(before.turn_key, after.turn_key);
        assert_eq!(after.model.as_deref(), Some("another-model"));
        assert_eq!(state.manual_title.as_deref(), Some("Manual title"));
        assert_eq!(after.waiting_reason, Some(WaitingReason::TurnEnded));
    }
    #[test]
    fn credentials_and_dead_lease_do_not_create_running_task() {
        let root = tempfile::tempdir().unwrap();
        assert!(event_belongs_to_root(
            &serde_json::json!({"config_dir":root.path()}),
            root.path()
        ));
        let other = tempfile::tempdir().unwrap();
        assert!(!event_belongs_to_root(
            &serde_json::json!({"config_dir":other.path()}),
            root.path()
        ));
        std::fs::create_dir(root.path().join("sessions")).unwrap();
        std::fs::write(
            root.path().join("sessions/secret.key"),
            b"not JSON, never read",
        )
        .unwrap();
        std::fs::write(
            root.path().join("sessions/dead.json"),
            br#"{"sessionId":"old","pid":0,"status":"busy"}"#,
        )
        .unwrap();
        assert_eq!(json_files(&root.path().join("sessions")).unwrap().len(), 1);
        let mut provider = ClaudeCodeProvider::new(root.path().into());
        provider.events = root.path().join("events");
        assert!(provider.poll(SystemTime::now()).tasks.is_empty());
    }
    #[test]
    fn stop_is_turn_end_not_success_and_permission_is_blocker() {
        let mut task = TaskSnapshot::new(
            "claude:s",
            "name",
            TaskSource::ClaudeCode,
            TaskStatus::Running,
            DetectionConfidence::Inferred,
            SystemTime::now(),
        );
        apply_hook(
            &mut task,
            &serde_json::json!({"hook_event_name":"Stop","turn_key":"turn-1"}),
        );
        assert_eq!(task.status, TaskStatus::WaitingInput);
        assert_eq!(task.waiting_reason, Some(WaitingReason::TurnEnded));
        assert_eq!(task.exit_code, None);
        apply_hook(
            &mut task,
            &serde_json::json!({"hook_event_name":"PermissionRequest","turn_key":"turn-2"}),
        );
        assert!(task.requires_action());
    }
}
