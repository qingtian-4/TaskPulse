<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/wordmark-light.svg">
  <img src="assets/brand/wordmark-light.svg" width="320" alt="TaskPulse">
</picture>

**Multiple agents running. Know which one needs you.**

One tray dashboard for tasks on your computer, in WSL, and over SSH.<br>See what's running, what needs an answer, and which results you haven't reviewed.

[简体中文](README.md) · [Install](#install) · [Features](#features) · [Supported agents](#supported-agents)

<p><img alt="Windows 10 / 11" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4?style=flat-square"> <img alt="macOS experimental" src="https://img.shields.io/badge/macOS-experimental-555?style=flat-square"> <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/License-MIT-76DFCD?style=flat-square"></a></p>

</div>

## Install

> **0.2.0-alpha.8 is not publicly released.** [Download test packages](docs/CANDIDATE_TESTING.md). One-line installation below requires a public stable release.

**Windows 10 / 11 (x64)**, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

**macOS (Apple Silicon / Intel, experimental)**, in Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

The script downloads the package for your platform, verifies its SHA-256 checksum, and installs it. You can also download a Windows EXE/MSI or Mac DMG from [Releases](https://github.com/qingtian-4/TaskPulse/releases). No Bun, Rust, or source checkout is needed.

On first launch, select the agents and projects you want to follow. Open the dashboard from the tray, or press `Ctrl+Shift+A` on Windows.

<details>
<summary>Updates, alpha releases, and installation options</summary>

Exit TaskPulse from the tray before running the command again to update. Windows opens the standard setup wizard; macOS installs to `~/Applications` by default. Current candidates have no publisher signing or Apple notarization, so the OS may ask for confirmation.

Opt into a public alpha with `-Prerelease` on Windows or `--prerelease` on macOS. See [installation](docs/INSTALL.md) and [CLI options](docs/CLI_INSTALL.md) for version selection, download-only mode, uninstalling, and retained settings.

</details>

## Features

- **One view across environments.** Find tasks by project or agent across local, WSL, and configured SSH sources, without checking every window.
- **Reminders when you need to act.** Alerts for questions, approvals, failures, and finished turns tell you when to return to the original agent.
- **Results stay until you've reviewed them.** Unviewed results remain in your attention list; pins and unread state survive restarts.
- **Models and elapsed time at a glance.** See the task title, project, source-reported model, and turn elapsed time together. Search by model name too.
- **Thinking still means running.** Tool calls and model-response waits don't become false completions. Unavailable sources are shown as uncertain.

## Supported agents

| Source | Integration |
| --- | --- |
| Codex | Local native sessions, independent WSL sources, configured SSH sessions |
| Claude Code | Local read-only records; explicit opt-in official hooks for precise events |
| Hermes | Local native database and runtime state |
| DeepSeek Harness | Local Desktop/CLI plus a separate WSL source |
| Other agents and command-line tasks | Standard Events, MCP, compatibility hooks, and `tp-run` |

Doubao and some additional adapters are experimental. Event coverage varies by source; see the [compatibility matrix](docs/AGENT_COMPATIBILITY.md) or the [integration guide](docs/AGENT_INTEGRATION_GUIDE.md).

## Data and privacy

TaskPulse reads task state from local records and configured sources, and stores it locally. It needs no TaskPulse cloud account and does not upload conversations to its own cloud. SSH connects only to configured servers. It never installs Agent hooks automatically or approves requests for you. [How it works](docs/ARCHITECTURE.md) · [Data details](docs/INSTALL.md#数据与隐私)

## Contributing and license

[Report an issue](https://github.com/qingtian-4/TaskPulse/issues), improve the docs, or contribute an adapter. [Contributing](CONTRIBUTING.md) · [Roadmap](docs/ROADMAP.md) · [Security](SECURITY.md)

Derived from [AgentTray](https://github.com/sprklai/agenttray), with original copyright and contribution history preserved. [MIT License](LICENSE).

## Build from source

Install Bun, Rust 1.89+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri build
```

Packages are written to `src-tauri/target/release/bundle/`. Use `bun run tauri dev` during development. See the [development guide](docs/DEVELOPMENT.md).
