<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <img src="assets/brand/wordmark-light.svg" width="320" alt="TaskPulse">
</picture>

# TaskPulse

**Stop watching your agents. Come back when they need you.**

Let your coding agents work in the background. Keep your attention on what you are doing.

[Install](#install) · [Get started](#get-started) · [Supported agents](#supported-agents) · [简体中文](README.md)

<p><img alt="Windows 10 / 11" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4?style=flat-square"> <img alt="macOS 13.3+ · Apple Silicon / Intel" src="https://img.shields.io/badge/macOS-13.3%2B%20%7C%20Apple%20Silicon%20%2F%20Intel-555?style=flat-square"> <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/License-MIT-76DFCD?style=flat-square"></a> <img alt="Rust / Tauri 2" src="https://img.shields.io/badge/Rust-Tauri%202-CE422B?style=flat-square&amp;logo=rust"></p>

</div>

TaskPulse is a lightweight tray app for local Codex and Claude Code tasks. Come back when they need an answer, approval or a review of the latest turn's result. Stay focused instead of constantly checking their progress.

## Install

Choose an installer for your system in [TaskPulse v0.0.1](https://github.com/qingtian-4/TaskPulse/releases/tag/v0.0.1).

| System | Installer |
| --- | --- |
| Windows 10 / 11 · x64 | `TaskPulse_0.0.1_windows_x64_setup.exe` |
| Windows 11 · ARM64 | `TaskPulse_0.0.1_windows_arm64_setup.exe` |
| macOS 13.3+ · Apple Silicon | `TaskPulse_0.0.1_macos_arm64.dmg` |
| macOS 13.3+ · Intel | `TaskPulse_0.0.1_macos_x64.dmg` |

### Install from the command line

**Windows · PowerShell**

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

**macOS · Terminal**

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

The scripts select the matching architecture and verify SHA-256. [Installation, OS prompts and removal (Chinese)](docs/INSTALL.md)

## Get started

1. **Choose your sources.** On first launch, confirm agents under Sources → Local and start monitoring. Local data is discovered automatically; add SSH servers when needed.
2. **Work as usual.** Start a task in your agent. The tray panel keeps running tasks, requests and unread results together. On Windows, press `Ctrl+Shift+A` to open it.
3. **Come back when needed.** Answer or approve in the original agent. A matching source event clears the request. Mark a result as viewed after reading it.

Dismissing a reminder, opening details or focusing a host does not mark results as read or approve anything. Adjust notifications, sound, project scopes, text size and updates in Settings.

## Features

- **Focus without waiting:** run several agents without watching every terminal.
- **Know when to return:** reminders for explicit input, approvals, results and failures supported by the source.
- **See what needs attention:** running tasks, requests and unread results together in the tray.
- **Keep important results:** unread results, pins and unresolved requests survive restarts. Read failures preserve the last trusted state.
- **Connect on your terms:** discover local sources and manage multiple SSH servers and project scopes.
- **Keep conversations local:** no TaskPulse cloud account or conversation uploads. Agent hooks and approval policies are never changed automatically.

## Task panel

<p align="center"><a href="assets/screenshots/dashboard-en.png"><img src="assets/screenshots/dashboard-en.png" width="368" alt="TaskPulse v0.0.1 showing running tasks, input, approval and unread results"></a></p>

<p align="center"><img src="assets/promo/teaser-8s.gif" width="768" alt="An eight-second introduction to TaskPulse: return for input, approval or results"></p>

## Task reminders

<p align="center">
  <a href="assets/screenshots/notice-input-en.png"><img src="assets/screenshots/notice-input-en.png" width="32%" alt="Input request"></a>
  <a href="assets/screenshots/notice-approval-en.png"><img src="assets/screenshots/notice-approval-en.png" width="32%" alt="Approval request"></a>
  <a href="assets/screenshots/notice-completed-en.png"><img src="assets/screenshots/notice-completed-en.png" width="32%" alt="Turn completed"></a>
</p>

## Supported agents

| Agent or source | Connection and capabilities |
| --- | --- |
| Codex | Discover locally or connect a separate WSL or SSH source. Reads sessions, models, turn state and explicit questions. Approvals require an [explicit event](docs/CODEX_APPROVAL_OBSERVER.md). |
| Claude Code | Discover locally. Optional official hooks provide explicit approvals and lifecycle events. Native questions and SSH are unsupported. |
| DeepSeek Harness | Read local Desktop or CLI data, or connect WSL. Supports reported models, reasoning effort and explicit requests. |
| Hermes | Reads local sessions, models and running state. |
| OpenCode | Reads local sessions, execution phases and explicit turn outcomes. |
| WorkBuddy | Reads local tasks and turn results. Cloud tasks are outside this source. |
| ZCode | Reads local tasks, models and turn results. Standard Events and MCP are optional. |
| Doubao | Reads desktop tasks and execution state. |
| Kimi Code / kimi-cli | Reads local main sessions and explicit requests. Standard Events and MCP are optional. |
| Qoder | Reads local Desktop turns and IDE or CLI journals. Native approvals and questions are unsupported. |
| TRAE | Discovers public workspace sessions. Task events can come from Standard Events or MCP. |
| Gemini CLI | Discovers running processes. Task lifecycle requires an event connection. |
| Other agents | Connect through Standard Events, MCP or compatible status files. |
| Shell / remote tmux | `tp-run` records the foreground command exit code. An ordinary tmux shell return only provides an inferred result. |

Reminders depend on the events each source provides. See [compatibility and setup details (Chinese)](docs/AGENT_COMPATIBILITY.md).

## How it works

<p align="center"><a href="assets/architecture/architecture-en.svg"><img src="assets/architecture/architecture-en.svg" width="900" alt="TaskPulse architecture: read-only sources, trusted observations, durable attention and desktop reminders"></a></p>

## Roadmap

- [ ] Broaden explicit native events and agent-version coverage.
- [ ] Improve cross-platform return, multiple displays and sleep recovery.
- [ ] Complete signing, notarization and installer/upgrade runtime coverage.
- [ ] Explore mobile and wearable notifications.

[Full roadmap](ROADMAP.md) · [Usage and troubleshooting](docs/README.md)

## Develop locally

Install Bun, Rust 1.90+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then run:

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run dev:desktop
```

The development app uses an isolated profile and preserves installed-app data. [CONTRIBUTING.md](CONTRIBUTING.md) covers verification and maintenance conventions.

## Build from source

After installing the prerequisites above, run from the repository root:

```sh
bun run tauri build
```

Installers are written to `src-tauri/target/release/bundle/` by default.

For Windows ARM64, install the MSVC C++ ARM64 build tools and Windows SDK first, then run:

```powershell
rustup target add aarch64-pc-windows-msvc
bun run build:windows:arm64
```

Output defaults to `src-tauri/target/aarch64-pc-windows-msvc/release/bundle/nsis/`. [Build prerequisites](https://v2.tauri.app/distribute/windows-installer/#building-for-32-bit-or-arm)

## Contributing and license

[Report an issue](https://github.com/qingtian-4/TaskPulse/issues) · [Contributing](CONTRIBUTING.md) · [Security reports](SECURITY.md) · [Code of conduct](CODE_OF_CONDUCT.md)

Built on [AgentTray](https://github.com/sprklai/agenttray), preserving copyright and contribution history. Licensed under the [MIT License](LICENSE); dependency licenses are in [licenses](licenses/README.md).
