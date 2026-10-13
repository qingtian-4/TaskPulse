# TaskPulse

**A tray dashboard for AI-agent sessions and code runs, with alerts when work needs attention or produces a result.**

[中文](README.md) · [Contributing](CONTRIBUTING.md) · [Architecture](docs/ARCHITECTURE.md) · [Implementation review](docs/PROJECT_REVIEW_2026-10.md)

TaskPulse is an **alpha, Windows-first** desktop application derived from
[AgentTray](https://github.com/sprklai/agenttray). The executable and package are
named `agent-tray`.

## Features

- Real session titles, separate project names, and observable execution activity.
- Explicit source selection and project scopes for local, supported WSL, and SSH sources.
- Independent task popups, system notifications, and persistent notification deduplication.
- Unviewed results, manual pins, and a configurable execution-activity window.
- Remote tmux process observation and `tp-run` exit-code records.
- Stale-source and disconnected-host indicators without inventing task completion.

## Support matrix

WSL means monitoring Linux data from the Windows application. These are code
capabilities, not a guarantee covering every installed Agent version.

| Source | Local | WSL | SSH |
|---|---|---|---|
| Codex | Native adapter | Dedicated adapter | Dedicated collection, with bounded log snapshots |
| Claude Code | Native records; opt-in hooks | No dedicated adapter | No dedicated adapter |
| Hermes | SQLite and turn leases | No dedicated adapter | No dedicated adapter |
| DeepSeek Harness | Desktop/CLI adapter | Dedicated adapter | No dedicated adapter |
| Gemini CLI | Legacy hooks/process scanning | No dedicated adapter | No dedicated adapter |
| ZCode, Trae, WorkBuddy | No dedicated adapter | No dedicated adapter | No dedicated adapter |

Seeing an Agent process inside remote tmux does not provide its conversation
lifecycle or approval events. Native undocumented formats are best-effort.
Hermes lease expiry currently needs stronger completion semantics.

Remote tmux code runs can be observed without a wrapper. `tp-run` adds real exit
codes and also writes records outside tmux, provided the configured source does
not restrict sessions/panes. Collection currently only reads run files modified
within three hours, and the wrapper has no heartbeat. Long runs are a known gap.
Arbitrary local, WSL, and unwrapped non-tmux remote runs do not yet have a complete
monitoring workflow. Legacy local wrappers should not be treated as reliable job supervisors.

## Run from source

Install Rust, Bun and [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
The standard Windows setup includes Microsoft C++ Build Tools and WebView2.
Linux/macOS compatibility remains in the code but Windows is the primary validation target.

```powershell
git clone https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri dev
```

Use `bun run tauri build` to build desktop bundles. `bun run build` builds only
the frontend. See [development notes](docs/DEVELOPMENT.md).

First launch asks you to confirm monitoring sources. Later use the tray menu or
`Ctrl+Shift+A`. WSL Codex is a separate opt-in source; see [WSL setup](docs/WSL_CODEX.md).
SSH monitoring uses your system SSH client and your own host aliases/authentication.

For command exit codes, copy `scripts/tp-run` to the server and run from the repository:

```bash
mkdir -p ~/.local/bin
install -m 755 scripts/tp-run ~/.local/bin/tp-run
~/.local/bin/tp-run "Training experiment" -- python train.py
```

Enable the server's tmux source and include the project path. The long-run
limitation above still applies.

## State and notifications

The current dashboard orders actionable tasks, running tasks, then unviewed
results, with pinned idle and older-attention views. A running-first layout is a
proposal, not an implemented change.

Thinking, tool execution and model-response waits remain running. An Agent turn
ending is not proof that its project is complete. Only real exit codes establish
command success. Missing logs and failed probes never establish completion.

Completion popups normally close after 15 seconds, pausing while hovered.
Confirmed approval/question popups do not time out. Closing a popup or opening details
does not mark a result viewed. Approval is performed in the original Agent.
Codex escalation parameters and slow commands are not evidence of pending human
approval. Only explicit source approval events trigger authorization reminders;
sources without these events may miss approval waits. Timed approval guesses are
not used.

## Data and privacy

Settings, attention state, recent results and notification deduplication live in
`~/.taskpulse/`. Delivery evidence is bounded to 500 entries and is not proof of
user reading. Generic `.status` files are accepted in `~/.taskpulse/` and the
legacy `~/.agent-monitor/` directory.

There is no TaskPulse cloud account or telemetry service. TaskPulse does not read
Claude `*.key` files or capture tmux pane content. Native logs, process arguments,
run files and activity summaries may contain commands or paths; review diagnostic
material before sharing it.

Claude hooks require explicit installation, preserve other settings, and can be
removed. CC Switch model/provider changes do not change the Agent identity.
Local Codex quota is queried through Codex app-server; there is currently no
independent quota toggle, and these values do not represent WSL/SSH accounts.

## Development and roadmap

```text
Provider → scope/normalization/dedup → TaskStore → AttentionTracker
         → TransitionEngine → tray / UI / notifications
```

While the Windows tray is running, use the isolated verification target:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend
bun run test:ui-model
```

Real Agent, SSH, WSL and native WebView2 acceptance checks are opt-in and separate
from unit tests. See [AGENTS.md](AGENTS.md) and [contributing](CONTRIBUTING.md).

Planned priorities are reliable cross-environment code-run monitoring, missing
Claude/Hermes transports, running-first presentation with recoverable alerts,
a generic Agent event contract, and optional quota display/querying. These are
not current capabilities. There is no general third-party plugin loader yet.

## License

[MIT](LICENSE). Derived from [AgentTray](https://github.com/sprklai/agenttray);
retain the original copyright, license and attribution when renaming or redistributing.
