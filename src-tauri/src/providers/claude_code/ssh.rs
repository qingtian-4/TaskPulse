//! Remote Claude Code uses the same native turn parser, without local hooks or
//! access to prompt text, tool arguments, or the remote process/window handles.
use super::{apply_record, text, Transcript};
use crate::domain::{TaskSnapshot, TaskStatus};
use crate::providers::remote_tmux::ssh::{SshExecutor, SystemSshExecutor};
use crate::providers::{Provider, ProviderOutput};
use crate::ssh_sources::AgentProbe;
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, SystemTime};

const CAPACITY: usize = 128;
const INCOMPLETE: &str = "远端 Claude Code 记录未读完整，已保留上次状态";
const UNREADABLE: &str = "无法读取远端 Claude Code；请检查 SSH、Python 3.8 和数据目录权限";
const SSH_FAILED: &str = "远端 Claude Code 的 SSH 连接失败";
const MISSING: &str = "未找到远端 Claude Code 会话目录";

#[derive(Debug, Deserialize)]
struct Snapshot {
    schema_version: u8,
    state: ReadState,
    sessions: Vec<Session>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReadState {
    Missing,
    Empty,
    Readable,
    Partial,
}

#[derive(Debug, Deserialize)]
struct Session {
    id: String,
    live: bool,
    records: Vec<Value>,
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn snapshot(
    executor: &dyn SshExecutor,
    host: &str,
    probe: bool,
    tracked: impl Iterator<Item = String>,
) -> Result<Snapshot, &'static str> {
    if !crate::ssh_sources::valid_host(host) {
        return Err(UNREADABLE);
    }
    let payload =
        base64::engine::general_purpose::STANDARD.encode(include_str!("../ssh_claude.py"));
    let mode = if probe { "probe" } else { "poll" };
    let ids = tracked
        .filter(|id| valid_id(id))
        .take(CAPACITY)
        .collect::<Vec<_>>();
    let command = format!(
        "echo {payload} | base64 -d | python3 - {mode} {}",
        ids.join(" ")
    );
    let output = executor
        .run(host, &command, Duration::from_secs(8))
        .map_err(|error| {
            if crate::providers::remote_tmux::ssh::connection_failed(&error) {
                SSH_FAILED
            } else {
                UNREADABLE
            }
        })?;
    if output.len() > 2_200_000 {
        return Err(UNREADABLE);
    }
    let result: Snapshot = serde_json::from_str(&output).map_err(|_| UNREADABLE)?;
    let mut ids = BTreeSet::new();
    if result.schema_version != 1
        || result.sessions.len() > 64
        || result.sessions.iter().any(|s| {
            !valid_id(&s.id) || !ids.insert(&s.id) || s.records.is_empty() || s.records.len() > 256
        })
        || matches!(result.state, ReadState::Missing | ReadState::Empty)
            && !result.sessions.is_empty()
    {
        return Err(UNREADABLE);
    }
    Ok(result)
}

pub(crate) fn probe(executor: &dyn SshExecutor, host: &str) -> AgentProbe {
    let Ok(snapshot) = snapshot(executor, host, true, std::iter::empty()) else {
        return AgentProbe {
            state: "read_error".into(),
            data_detected: false,
            projects: vec![],
        };
    };
    let projects = snapshot
        .sessions
        .iter()
        .filter_map(|session| session.records.iter().rev().find_map(|r| text(r, "cwd")))
        .filter(|path| {
            path.starts_with('/') && path.len() <= 4096 && !path.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    AgentProbe {
        state: match snapshot.state {
            ReadState::Missing => "missing",
            ReadState::Empty => "empty",
            ReadState::Readable => "readable",
            ReadState::Partial => "partial",
        }
        .into(),
        data_detected: !snapshot.sessions.is_empty(),
        projects: projects.into_iter().take(128).collect(),
    }
}

pub struct RemoteClaudeProvider {
    host: String,
    ssh: Box<dyn SshExecutor>,
    last: HashMap<String, TaskSnapshot>,
}

impl RemoteClaudeProvider {
    pub fn new(host: String) -> Self {
        Self {
            host,
            ssh: Box::new(SystemSshExecutor::new()),
            last: HashMap::new(),
        }
    }

    pub fn resume(&mut self, previous: impl Iterator<Item = TaskSnapshot>) {
        for mut task in previous.take(CAPACITY) {
            if let Some(id) = task.session_id.clone().filter(|id| valid_id(id)) {
                task.id = format!("claude:{id}");
                self.last.insert(id, task);
            }
        }
    }

    fn sample(&mut self, now: SystemTime) -> ProviderOutput {
        let tracked = self
            .last
            .iter()
            .filter(|(_, task)| task.status == TaskStatus::Running);
        let data = snapshot(
            self.ssh.as_ref(),
            &self.host,
            false,
            tracked.map(|(id, _)| id.clone()),
        );
        let (mut error, sessions) = match data {
            Ok(snapshot) => (
                match snapshot.state {
                    ReadState::Missing => Some(MISSING),
                    ReadState::Partial => Some(INCOMPLETE),
                    _ => None,
                },
                snapshot.sessions,
            ),
            Err(error) => (Some(error), vec![]),
        };
        let mut tasks = Vec::new();
        let mut catalog = Vec::new();
        let mut observed = BTreeSet::new();
        for session in sessions {
            let Some(mut task) = parse_session(&session, &self.host, now) else {
                continue;
            };
            let previous = self.last.get(&session.id);
            let recent =
                now.duration_since(task.updated_at).unwrap_or_default() <= Duration::from_secs(30);
            if previous.is_some_and(|old| {
                old.updated_at > task.updated_at
                    || old.turn_key == task.turn_key
                        && old.finished_at.is_some()
                        && task.finished_at.is_none()
            }) {
                error.get_or_insert(INCOMPLETE);
                continue;
            }
            if task.status == TaskStatus::Running && !session.live && !recent {
                task.stale = true;
                if previous.is_none_or(|old| old.status != TaskStatus::Running) {
                    task.status = TaskStatus::Unknown;
                }
            }
            catalog.push(task.clone());
            if task.status == TaskStatus::Unknown {
                continue; // Don't turn old, incomplete history into work.
            }
            if previous.is_none() && self.last.len() >= CAPACITY {
                if let Some(old) = self
                    .last
                    .iter()
                    .filter(|(id, t)| !observed.contains(*id) && t.status != TaskStatus::Running)
                    .min_by_key(|(_, t)| t.updated_at)
                    .map(|(id, _)| id.clone())
                {
                    self.last.remove(&old);
                } else {
                    error.get_or_insert(INCOMPLETE);
                    continue;
                }
            }
            observed.insert(session.id.clone());
            self.last.insert(session.id, task.clone());
            tasks.push(task);
        }
        for (id, task) in &self.last {
            if task.status == TaskStatus::Running && !observed.contains(id) {
                let mut task = task.clone();
                task.stale = true;
                tasks.push(task);
                error.get_or_insert(INCOMPLETE);
            }
        }
        ProviderOutput {
            tasks,
            catalog,
            authoritative: error.is_none(),
            error: error.map(str::to_owned),
            ..Default::default()
        }
    }
}

fn parse_session(session: &Session, host: &str, now: SystemTime) -> Option<TaskSnapshot> {
    let mut transcript = Transcript::default();
    let mut latest = None;
    for row in &session.records {
        let kind = text(row, "type");
        let timestamp = text(row, "timestamp");
        if timestamp
            .is_some_and(|stamp| !stamp.is_ascii() || stamp.len() > 64 || !stamp.ends_with('Z'))
        {
            continue;
        }
        if matches!(kind, Some("custom-title" | "ai-title")) {
            apply_record(&mut transcript, row, &session.id, now);
            continue;
        }
        let Some(at) = timestamp.and_then(crate::providers::codex::parse_rfc3339) else {
            continue;
        };
        if at > now + Duration::from_secs(120) || latest.is_some_and(|last| at < last) {
            continue;
        }
        let start = kind == Some("user")
            && row.pointer("/message/content").and_then(Value::as_str) == Some("")
            && text(row, "promptId")
                .or_else(|| text(row, "uuid"))
                .is_some();
        if !start && latest.is_none() {
            continue; // A tail without its start cannot verify a completion.
        }
        apply_record(&mut transcript, row, &session.id, now);
        latest = Some(at);
    }
    let mut task = transcript.task?;
    task.name = transcript
        .manual_title
        .or(transcript.native_title)
        .unwrap_or_else(|| format!("Claude Code · {}", &session.id[..8]));
    if task.turn_key.is_none() {
        task.status = TaskStatus::Unknown;
    }
    task.remote = true;
    task.host = Some(host.into());
    task.message = None;
    task.evidence_channel = Some("native_transcript".into());
    Some(task)
}

impl Provider for RemoteClaudeProvider {
    fn name(&self) -> &'static str {
        "remote_claude_code"
    }

    fn poll(&mut self, now: SystemTime) -> ProviderOutput {
        self.sample(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ExecutionPhase, WaitingReason};
    use crate::providers::remote_tmux::ssh::FakeSshExecutor;
    use serde_json::json;

    const ID: &str = "11223344-1111-2222-3333-445566778899";
    const START: &str = "2026-11-10T12:00:00.100Z";
    const TOOL: &str = "2026-11-10T12:00:01.123Z";
    const RESULT: &str = "2026-11-10T12:00:02.456Z";
    const END: &str = "2026-11-10T12:00:04.789Z";

    fn at(stamp: &str) -> SystemTime {
        crate::providers::codex::parse_rfc3339(stamp).unwrap()
    }

    fn start() -> Value {
        json!({"type":"user","timestamp":START,"cwd":"/srv/project","uuid":"turn-one","message":{"content":""}})
    }

    fn assistant(timestamp: &str, message: Value) -> Value {
        json!({"type":"assistant","timestamp":timestamp,"message":message})
    }

    fn data(live: bool, state: &str, records: &[Value]) -> String {
        json!({"schema_version":1,"state":state,"sessions":[{"id":ID,"live":live,"records":records}]}).to_string()
    }

    #[test]
    fn remote_claude_tool_result_and_connection_loss_do_not_finish_a_turn() {
        let open = data(
            true,
            "readable",
            &[
                start(),
                assistant(
                    TOOL,
                    json!({"model":"claude-sonnet","content":[{"type":"tool_use","id":"tool-one","name":"Bash"}]}),
                ),
            ],
        );
        let tool_result = json!({"type":"user","timestamp":RESULT,"toolUseResult":true,
            "message":{"content":[{"type":"tool_result","tool_use_id":"tool-one"}]}});
        let returned = data(false, "readable", &[start(), tool_result.clone()]);
        let final_row = assistant(END, json!({"stop_reason":"end_turn","content":[]}));
        let finished = data(
            false,
            "readable",
            &[start(), tool_result, final_row.clone()],
        );
        let old = data(false, "readable", &[start()]);
        let ssh = FakeSshExecutor::new()
            .with("server", &open)
            .failing("server")
            .with("server", &returned)
            .with("server", &finished)
            .with("server", &old);
        let mut p = RemoteClaudeProvider::new("server".into());
        p.ssh = Box::new(ssh);
        let state = tempfile::tempdir().unwrap();
        let ledger = state.path().join("notifications.json");
        let mut engine = crate::transition::TransitionEngine::open(ledger.clone());
        let first = p.poll(at(END));
        assert!(engine
            .apply_snapshots(&first.tasks, &[], at(END))
            .is_empty());
        let task = &first.tasks[0];
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.phase, Some(ExecutionPhase::Tool));
        assert!(task.remote && task.pid.is_none() && !task.can_focus);
        assert_eq!(task.host.as_deref(), Some("server"));
        assert_eq!(task.project_path.as_deref(), Some("/srv/project"));
        assert_eq!(task.model.as_deref(), Some("claude-sonnet"));
        assert_eq!(task.updated_at, at(TOOL));
        let disconnected = p.poll(at(END));
        assert!(!disconnected.authoritative && disconnected.tasks[0].stale);
        assert_eq!(disconnected.tasks[0].status, TaskStatus::Running);
        assert!(disconnected.catalog.is_empty());
        assert!(engine
            .apply_snapshots(&disconnected.tasks, &[], at(END))
            .is_empty());
        let result = p.poll(at(END));
        assert_eq!(result.tasks[0].phase, Some(ExecutionPhase::AwaitingModel));
        assert!(!result.tasks[0].requires_action());
        assert!(result.tasks[0].finished_at.is_none());
        assert!(engine
            .apply_snapshots(&result.tasks, &[], at(END))
            .is_empty());
        let done = p.poll(at(END));
        assert_eq!(done.tasks[0].id, task.id);
        assert_eq!(done.tasks[0].turn_key, task.turn_key);
        assert_eq!(done.tasks[0].waiting_reason, Some(WaitingReason::TurnEnded));
        assert_eq!(done.tasks[0].finished_at, Some(at(END)));
        assert_eq!(engine.apply_snapshots(&done.tasks, &[], at(END)).len(), 1);
        assert!(engine.flush());
        let mut restarted = crate::transition::TransitionEngine::open(ledger);
        assert!(restarted
            .apply_snapshots(&done.tasks, &[], at(END))
            .is_empty());
        let rewind = p.poll(at(END));
        assert!(!rewind.authoritative && rewind.tasks.is_empty());
        // A bounded tail without a starting record cannot verify an end.
        let tail_only = data(false, "readable", &[final_row]);
        p.ssh = Box::new(FakeSshExecutor::new().with("server", &tail_only));
        assert!(p.poll(at(END)).tasks.is_empty());
    }

    #[test]
    fn remote_claude_p1_resume_uses_native_identity_on_first_failure() {
        use crate::{
            attention::AttentionTracker, monitoring::SourceConfig, transition::TransitionEngine,
        };
        let source: SourceConfig = serde_json::from_value(json!({
            "id":"remote-claude","name":"SSH fixture","kind":"remote_claude_code",
            "location":"server","enabled":true
        }))
        .unwrap();
        let open = data(true, "readable", &[start()]);
        let end = data(
            false,
            "readable",
            &[
                start(),
                assistant(END, json!({"stop_reason":"end_turn","content":[]})),
            ],
        );
        let mut provider = RemoteClaudeProvider::new("server".into());
        provider.ssh = Box::new(FakeSshExecutor::new().with("server", &open));
        let state = tempfile::tempdir().unwrap();
        let attention_path = state.path().join("attention.json");
        let notification_path = state.path().join("notifications.json");
        let mut tracker = AttentionTracker::open(attention_path.clone());
        let mut engine = TransitionEngine::open(notification_path.clone());
        let collect = |provider: &mut RemoteClaudeProvider, tracker: &mut AttentionTracker| {
            let mut tasks = provider.poll(at(END)).tasks;
            for task in &mut tasks {
                crate::watcher::bind_provider_identity(task, &source);
            }
            tracker.reconcile(tasks)
        };
        let first = collect(&mut provider, &mut tracker);
        let id = format!("remote-claude:claude:{ID}");
        assert_eq!(first[0].id, id);
        assert!(engine.apply_snapshots(&first, &[], at(END)).is_empty());
        tracker.set_pinned(&id, true).unwrap();
        tracker.flush().unwrap();
        assert!(engine.flush());
        let mut tracker = AttentionTracker::open(attention_path);
        let mut engine = TransitionEngine::open(notification_path);
        let mut restarted = RemoteClaudeProvider::new("server".into());
        restarted.ssh = Box::new(
            FakeSshExecutor::new()
                .failing("server")
                .with("server", &open)
                .with("server", &end)
                .with("server", &end),
        );
        restarted.resume(tracker.known().into_iter());
        let stale = collect(&mut restarted, &mut tracker);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, id);
        assert!(stale[0].pinned && stale[0].stale);
        assert!(engine.apply_snapshots(&stale, &[], at(END)).is_empty());
        let fresh = collect(&mut restarted, &mut tracker);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].id, id);
        assert!(fresh[0].pinned && !fresh[0].stale);
        assert!(engine.apply_snapshots(&fresh, &[], at(END)).is_empty());
        let finished = collect(&mut restarted, &mut tracker);
        assert!(finished[0].pinned && finished[0].unread);
        assert_eq!(engine.apply_snapshots(&finished, &[], at(END)).len(), 1);
        let repeated = collect(&mut restarted, &mut tracker);
        assert!(engine.apply_snapshots(&repeated, &[], at(END)).is_empty());
    }

    #[test]
    fn remote_claude_cold_stale_session_needs_a_verified_live_process() {
        let no_process = data(false, "readable", &[start()]);
        let live = data(true, "readable", &[start()]);
        let empty = r#"{"schema_version":1,"state":"empty","sessions":[]}"#;
        let mut p = RemoteClaudeProvider::new("server".into());
        p.ssh = Box::new(
            FakeSshExecutor::new()
                .with("server", &no_process)
                .with("server", &live)
                .with("server", empty),
        );
        let now = at(START) + Duration::from_secs(180);
        let cold = p.poll(now);
        assert!(cold.tasks.is_empty());
        assert_eq!(cold.catalog[0].status, TaskStatus::Unknown);
        let known = p.poll(now);
        assert_eq!(known.tasks[0].status, TaskStatus::Running);
        assert!(!known.tasks[0].stale);
        let missing = p.poll(now);
        assert_eq!(missing.tasks[0].status, TaskStatus::Running);
        assert!(missing.tasks[0].stale && !missing.authoritative);
        assert_eq!(missing.tasks[0].updated_at, at(START));
    }

    #[test]
    fn remote_claude_probe_reports_partial_data_without_claiming_full_health() {
        let data = data(false, "partial", &[start()]);
        let ssh = FakeSshExecutor::new().with("server", &data);
        let report = probe(&ssh, "server");
        assert!(report.data_detected);
        assert_eq!(report.state, "partial");
        assert_eq!(report.projects, ["/srv/project"]);
    }
}
