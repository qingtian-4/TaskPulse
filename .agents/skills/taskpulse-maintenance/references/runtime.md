# Runtime and data contracts

Read only when the change touches this area. User scope takes precedence.

## Architecture

Collection and durable attention:

`Provider -> scope filtering/normalization/dedup -> TaskStore -> AttentionTracker`

The attention display plus source health feed dashboard projection/snapshots,
Svelte and the tray. Tracked sessions separately feed `TransitionEngine` and
notification delivery. Scoped user commands control window visibility and
verified host return; snapshot delivery never triggers focus or approval.

Ownership and file locations are maintained in the [architecture map](../../../../docs/ARCHITECTURE.md).

Stable session/thread IDs are task identity. A title rename updates presentation only; it must not create a new task, reset lifecycle state, or send another notification. Project and task title remain separate model fields.

Execution `phase` is independent of lifecycle: thinking, tool execution, responding and waiting for a model response all remain RUNNING. Clear tool activity by matching call ID on tool results; never infer an Agent turn end from low CPU, silent logs, missing files or a tool return. Only explicit lifecycle events notify; inferred finish is limited to non-Agent tmux shell-return snapshots. Same stable turn status flapping cannot reset notification dedup. Attention v4 conservatively archives v3 false Claude history imports without acknowledging/deleting them; back up to `attention.json.pre-lifecycle.bak` before migration.

The main view defaults to a configurable 7-day execution-activity window (`dashboard.recent_days`, zero disables archival). Inactive attention moves to a separate older-attention view without deleting unread/pin state. Running tasks and explicit TaskPulse manual pins never expire. Legacy pin origin is unknown and does not exempt old records from archival; preserve their flag and allow explicit re-pinning. Polling/renames must not renew execution activity. Preserve existing project allowlists unless the user changes them; new sources default to all projects' new executions.

## Data and configuration

Codex command permission parameters, slow tool results, silent logs and CPU activity are never proof of pending human approval. Do not infer authorization waits or emit timed approval-check reminders. Only an explicit source approval request sets WAITING_INPUT / approval, cleared by its matching request ID. Preserve RUNNING tool/thinking phases; legacy awaiting_tool snapshots remain readable and are not human blockers. Never execute Code Mode snippets or install hooks/change approval policies automatically. Confirmed blocking popups have no automatic timer, may rotate without dismissal, and reconcile only against fresh matching observations. Delivery evidence in `~/.taskpulse/notification-log.json` is bounded to 500 records, excluded from collection/watcher triggers, and never asserts user-read or includes commands/prompts/credentials. See `docs/PRODUCT_DASHBOARD.md` for source limitations.

- TaskPulse state: `~/.taskpulse/attention.json` and `~/.taskpulse/recent.json`.
- Notification dedup: `~/.taskpulse/notifications.json` (must survive restarts).
- Monitoring/remote configuration: `~/.taskpulse/config.toml`.
- Source registry uses `[dashboard]` in that config; once saved it supersedes legacy scopes. UI saves apply immediately; preserve unknown config fields and create the one-time migration backup. Fresh installs require explicit onboarding confirmation.
- Legacy/generic hook input: `~/.agent-monitor/` and `~/.taskpulse/` status files. Compatibility records have a per-file and shared read budget; incomplete listings are non-authoritative. A malformed record cannot replace a trusted task.
- Codex: native thread/app-server metadata plus rollout logs; local and configured remote projects are supported.
- Codex WSL: independent opt-in `codex_wsl` sources per running distro. Read metadata inside Linux using Python 3 / read-only SQLite plus `session_index.jsonl`; never open WSL SQLite/WAL through Windows UNC. Reuse the Codex incremental rollout parser via `\\wsl.localhost\<distro>`. IDs are `<source-instance>:codex:<thread-id>` and Linux project paths stay separate from Windows / SSH. Never auto-wake a stopped distro or query Windows account quota for WSL tasks. See `docs/WSL_CODEX.md`.
- Claude Code: read-only `CLAUDE_CONFIG_DIR` (default `~/.claude`) projects/*.jsonl, sessions/*.json and IDE *.lock evidence; never read *.key. Native formats are best-effort/inferred. Explicit, reversible official hooks write a minimal event envelope to `~/.taskpulse/claude-events/`. Agent identity is independent from the reported/proxy model. Never auto-install hooks or overwrite statusLine/other hooks/proxy settings.
  Hook envelopes use an OS-locked per-session JSONL journal with monotonic sequence; latest JSON is a compatibility checkpoint. PermissionRequest has no native call ID: associate only a unique preceding tool/input signature, preserve all explicit pending request IDs, and resolve only a matching call or explicit end. Native thinking/busy/idle metadata cannot clear an approval or prove turn end. Admission is bounded across sessions; only the current journal may compact, and a rewind or corruption cannot overwrite a newer checkpoint. A v3 checkpoint without its journal supplies only stale recovery evidence, and the writer cannot rebuild the missing history from it. Stale recovery cannot clear a durable pending request. File locking requires Rust 1.89+; the signed updater raises the project minimum to Rust 1.90.
- OpenCode: read-only `XDG_DATA_HOME/opencode/opencode.db` (default `~/.local/share/opencode`). v2 `session_message` user/assistant/idle records and legacy message tables are distinct schemas; only a current-turn explicit idle outcome/final assistant ends a turn. A `tool-calls` finish, older idle outcome or model/title update never does. No approval/question state is inferred from local storage.
- WorkBuddy: read-only `~/.workbuddy/workbuddy.db` plus bounded incremental `projects/*/*.jsonl`. Match native session IDs, latest real user message and current request; require current-turn transcript evidence with database terminal state. Partial/corrupt/rewound logs preserve uncertainty. `pending` is not proof of approval; cloud and WorkBuddy AI sessions are outside this adapter. Neither adapter installs hooks or imports idle history.
- Hermes: native SQLite session metadata and runtime leases. A followed lease can finish without closing the chat when the bounded active transcript has one matching user prompt and an explicit terminal assistant reply; lease loss, tool results and old history do not complete a turn.
- DeepSeek Harness: native Desktop/CLI `$DSH_HOME` (default `~/.dsh`, override with `TASKPULSE_DSH_HOME`) plus an independent WSL worker. Support both `session.jsonl.zstd` and `session.v4.jsonl.zstd`. Native v4 titles/state use `storages/session_projcache/sessions/<id>.json` (`version`/`record` envelope); legacy versions use `storages/session_projcache.json`. Projection event seq must not override a newer log state. Native IDs are `dsh-native:<session-id>`; existing WSL IDs stay `dsh:<session-id>`.
- Remote: configured SSH hosts, projects, and tmux sessions/panes.
