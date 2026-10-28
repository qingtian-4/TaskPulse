<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/wordmark-light.svg">
  <img alt="TaskPulse: Your agents, one glance." src="assets/brand/wordmark-light.svg" width="410">
</picture>

# TaskPulse

**Stay focused on your code. Let TaskPulse watch your coding agents.**

![Windows first](https://img.shields.io/badge/Windows-first-76DFCD?style=flat-square)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?style=flat-square)
[![MIT](https://img.shields.io/badge/License-MIT-blue?style=flat-square)](LICENSE)

[中文](README.md) · [Quick start](#quick-start) · [Supported agents](#supported-agents) · [Roadmap](#roadmap) · [Issues](https://github.com/qingtian-4/TaskPulse/issues)

</div>

TaskPulse brings coding-agent tasks from local, WSL and SSH environments into one desktop tray. Get a reminder when an agent **needs an answer, requests approval or finishes a turn**, without repeatedly checking every window.

> **0.2.0-alpha.8**, not publicly released. [Candidate downloads and testing](docs/CANDIDATE_TESTING.md). Windows first; macOS/Linux are experimental. A mobile companion is planned.

## See it in action

<p align="center"><img src="assets/screenshots/dashboard-current.png" alt="TaskPulse dashboard: running, needs action and results to review" width="412"></p>

| Answer needed | Approval needed |
|---|---|
| ![Input request reminder](assets/screenshots/notice-question-current.png) | ![Approval reminder](assets/screenshots/notice-approval-current.png) |

*Actual Tauri/WebView2 screenshots with isolated synthetic standard events. Respond in the original agent; dismissing a reminder does not approve or resolve its request. [Media and reproduction](assets/README.md).*

<details>
<summary>Message center and source management</summary>

![Message center](assets/screenshots/message-center-current.png)

![Source management](assets/screenshots/sources-current.png)

</details>

## Quick start

Packaged installs need no Bun, Rust or source checkout. **The candidate is not publicly released**; use [candidate downloads](docs/CANDIDATE_TESTING.md) for authenticated Actions artifacts at the exact source SHA. PR previews are test attachments, not public releases.

| Platform | Package | Command-line entry | Availability |
|---|---|---|---|
| Windows 10/11 x64 | NSIS EXE / MSI | [`install.ps1`](docs/CLI_INSTALL.md#windows-1011-x64) | Primary platform; current PR packages need independent acceptance |
| macOS Apple Silicon | arm64 DMG | [`install.sh`](docs/CLI_INSTALL.md#macos-apple-silicon-and-intel) | Experimental, ad-hoc signed, not notarized; current PR and physical Mac checks pending |
| macOS Intel | x64 DMG | [`install.sh`](docs/CLI_INSTALL.md#macos-apple-silicon-and-intel) | Experimental; separate native build and physical Mac checks pending |

CLI entries consume public release packages after the scripts reach main and a reviewed version is published. Stable is the default; alpha requires explicit selection. Drafts and PR previews are excluded. Linux remains experimental through [source development](docs/DEVELOPMENT.md).

Open **source management → select an agent and projects → save and apply**, then start work in the original agent. Open the dashboard with `Ctrl+Shift+A`.

[Installation and upgrades](docs/INSTALL.md) · [CLI options](docs/CLI_INSTALL.md) · [WSL](docs/WSL_CODEX.md) · [Mac](docs/MACOS_DEVELOPMENT.md)

## Features

- **One attention dashboard** for running tasks, pending requests, failures and unviewed results, grouped by agent, project or source.
- **Aggregate tray status** prioritizes requests over errors, unviewed results and running work across concurrent agents.
- **Explicit event reminders** keep thinking and tools running; silence, low CPU or collection failure never imply completion.
- **Recoverable attention** retains unviewed results and pins across restarts. Message seen, result viewed and request resolved stay separate.

## Supported agents

| Source | Current integration |
|---|---|
| Codex | Local native sessions, independent WSL sources and configured SSH sessions |
| Claude Code | Local read-only records and explicitly enabled official hooks |
| Hermes / DeepSeek Harness | Local native data; DeepSeek Harness also has a separate WSL source |
| Doubao Desktop (experimental) | Native Windows running checkpoints; approval unsupported, end reminders not guaranteed |
| Gemini CLI / SSH tmux / `tp-run` | Compatibility hooks, process scanning or run wrappers |
| Kimi / TRAE / ZCode | Partial event mappings, fixture-tested; real-host acceptance pending |
| Other agents | Standard Events, declarative manifests and active reporting from hosts supporting local MCP |

Environment coverage, event support and validation differ by source. See the [compatibility matrix](docs/AGENT_COMPATIBILITY.md) and [integration guide](docs/AGENT_INTEGRATION_GUIDE.md).

## Models and elapsed time

Codex reports models from native metadata and rollout context; Claude Code from assistant records; DeepSeek Harness from session/log state; Hermes from SQLite session/messages. Missing, empty or unsupported values remain unknown. A reported or proxy model does not identify the underlying service, and changing it never changes task identity, unread state or notification deduplication.

Turn elapsed time covers thinking, tools and model-response waits. It is not pure model computation time; missing start times are not invented. Source freshness is separate from execution time. Only explicit requests establish an approval wait. [Source details and validation](docs/AGENT_COMPATIBILITY.md#模型信息与本轮计时).

![Synthetic model example](assets/screenshots/models-synthetic.png)

*Native WebView2 capture from isolated synthetic transcripts, showing a reported model and a missing-model task. This is not real-host certification.*

## How it works

![TaskPulse architecture](assets/diagrams/taskpulse-architecture.svg)

**Collect evidence → reconcile tasks → track attention → update the tray, dashboard and event reminders.**

Built with Rust, Tauri 2 and Svelte 5. State stays local without a TaskPulse cloud account. Source failures remain uncertain, and approvals stay in the original agent. [Architecture details](docs/ARCHITECTURE.md).

## Privacy

TaskPulse reads configured local records and explicitly enabled WSL/SSH sources. Provider records may contain prompts, titles and paths; TaskPulse does not upload conversations to its own cloud or require a cloud account. GitHub installation downloads, system notifications and configured SSH still cross their respective network/system boundaries. TaskPulse keeps settings, attention and notification deduplication in `~/.taskpulse/`; review diagnostics before sharing. It never installs Agent hooks automatically or approves requests. [Data boundaries](docs/INSTALL.md#数据与隐私).

## Source development

Developers need Bun, Rust 1.89+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/):

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri dev
```

See [development and troubleshooting](docs/DEVELOPMENT.md). The installer does not require these tools. README and installer output support Simplified Chinese and English; full UI localization is outside this change.

## Roadmap

- [ ] **Mobile companion** to view tasks on paired computers and receive question, approval, failure and completion alerts.
- [ ] **More coding agents**, using official events, native data and MCP, with real-host validation for each integration.
- [ ] **Cross-device experience** with pairing, offline visibility, notification deduplication and reading-state sync.
- [ ] **Desktop reliability** across upgrades, mixed-DPI monitors and extended use.

These are plans: mobile apps and cross-device notifications are not implemented. [Scope and milestones](docs/ROADMAP.md).

## Contributing & license

Issues, documentation and adapters are welcome. Use **feature branch → Pull Request → checks and review → merge into main**. [Contributing](CONTRIBUTING.md) · [Merge workflow](docs/GIT_WORKFLOW.md) · [Security](SECURITY.md).

Derived from [sprklai/agenttray](https://github.com/sprklai/agenttray), with upstream credits and history preserved. [MIT License](LICENSE).
