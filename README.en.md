<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/wordmark-light.svg">
  <img src="assets/brand/wordmark-light.svg" width="320" alt="TaskPulse">
</picture>

**Several agents running. Know which one needs you.**

See AI task status, model and elapsed time from your system tray.<br>Get notified when work finishes, fails or needs your attention.

[简体中文](README.md) · [Install](#install) · [Supported agents](#supported-agents) · [User guides](docs/README.md)

<p><img alt="Windows 10 / 11" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4?style=flat-square"> <img alt="macOS" src="https://img.shields.io/badge/macOS-Apple%20Silicon%20%2F%20Intel-555?style=flat-square"> <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/License-MIT-76DFCD?style=flat-square"></a></p>

</div>

## Features

- **Find your agents automatically** — confirm detected local sources once, then follow new tasks. Connect WSL and SSH sources when needed.
- **Status, model and time together** — see task titles, projects, reported models and turn duration, with search and grouping.
- **Know when to return** — separate notifications for completion, failure, questions and permission requests.
- **Keep results in view** — unread results remain available; pins and unread state survive restarts.
- **Stay out of the way** — a compact tray panel fits laptops and high-DPI displays. Thinking and tool execution remain running states.
- **Keep your data local** — no TaskPulse cloud account or conversation uploads. TaskPulse never approves Agent requests for you.

## Install

### Download an installer

Choose your platform on [GitHub Releases](https://github.com/qingtian-4/TaskPulse/releases):

| Platform | Package |
| --- | --- |
| Windows 10 / 11 · x64 | `.exe` setup wizard or `.msi` |
| macOS · Apple Silicon | `macos_arm64.dmg` |
| macOS · Intel | `macos_x64.dmg` |

### Install with one command

**Windows** — open PowerShell:

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

**macOS** — open Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

The scripts select your architecture, verify SHA-256 and install the package. No Bun, Rust or repository checkout is needed. On Mac, the default destination is `~/Applications`. See [installation details](docs/INSTALL.md) for system security prompts and version options.

On first launch, confirm the detected local agents and click **开始监控** (Start monitoring). Open the tray panel to see tasks; Windows also supports `Ctrl+Shift+A`.

TaskPulse automatically checks for new versions. In **Settings → Application updates**, click to download, verify the signature and install. You can turn automatic checks off or check manually. The application interface currently uses Simplified Chinese.

## Supported agents

| Agent / task source | Connection |
| --- | --- |
| Codex | Automatic local discovery; separate WSL and SSH sources |
| Claude Code | Automatic local discovery; optional official hooks for precise events |
| Hermes | Native local database and runtime state |
| DeepSeek Harness | Local Desktop / CLI; separate WSL source |
| ZCode | Native local sessions, projects, models and task state |
| Doubao | Native desktop execution state |
| Gemini CLI | Process discovery; task events through a generic integration |
| Kimi Code CLI | Configured Standard Events or MCP integration |
| WorkBuddy | Configured Standard Events or MCP integration |
| TRAE China IDE | Configured Standard Events or MCP integration |
| Other agents / command-line tasks | Standard Events, MCP, compatible hooks, `tp-run` and remote tmux |

Automatic discovery covers supported native data formats. WSL, SSH and event integrations need configuration. Available states vary by agent; missing models and permission requests are never guessed. [Compatibility](docs/AGENT_COMPATIBILITY.md) · [Integration guide](docs/AGENT_INTEGRATION_GUIDE.md)

## Roadmap

- [x] Local, WSL and remote tasks in one view
- [x] Model display, elapsed time, notifications and unread results
- [x] Windows / macOS packages and command-line installation
- [x] Automatic local discovery and in-app updates
- [ ] Native automatic connections for more agents
- [ ] Simplified Chinese / English interface switch

## Build from source

Install Bun, Rust 1.90+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri build
```

Packages are written to `src-tauri/target/release/bundle/`. Use `bun run tauri dev` to run locally.

## Contributing and license

[Report an issue](https://github.com/qingtian-4/TaskPulse/issues), improve the docs or contribute an Agent adapter. [Contributing guide](CONTRIBUTING.md) · [Security](SECURITY.md)

Built on [AgentTray](https://github.com/sprklai/agenttray), preserving its copyright and contribution history. Licensed under the [MIT License](LICENSE).
