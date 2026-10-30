<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/wordmark-light.svg">
  <img src="assets/brand/wordmark-light.svg" width="320" alt="TaskPulse">
</picture>

# TaskPulse

**多个 Agent 同时跑，先看哪个需要你。**

在托盘里查看 AI 任务的状态、模型与运行时间。<br>完成、失败或需要你时，及时提醒。

[English](README.en.md)

<p><img alt="Windows 10 / 11" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4?style=flat-square"> <img alt="macOS 13.3+ · Apple Silicon / Intel" src="https://img.shields.io/badge/macOS-13.3%2B%20%7C%20Apple%20Silicon%20%2F%20Intel-555?style=flat-square"> <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/License-MIT-76DFCD?style=flat-square"></a> <img alt="Rust / Tauri 2" src="https://img.shields.io/badge/Rust-Tauri%202-CE422B?style=flat-square&amp;logo=rust"></p>

</div>

<p align="center">
  <img src="assets/screenshots/dashboard.png" width="368" alt="TaskPulse 四任务面板：进行中、待输入、待批准和待查看，显示模型与耗时">
  <br><sub>Windows 界面 · 示例任务</sub>
</p>

## 功能

- **打开即关注**：自动发现本机 Agent，首次确认后跟踪新任务；也可接入 WSL 和 SSH 服务器。
- **状态、模型、时间**：任务标题、项目、来源上报的模型和本轮耗时同屏展示，支持搜索与分组。
- **需要你时提醒**：完成、失败、提问和授权请求分别提醒，点击后回到对应任务。
- **结果不会错过**：未查看的结果持续保留，固定任务与未读状态可跨重启恢复。
- **按你的习惯显示**：支持浅色、深色与跟随系统，可切换简体中文 / English，并设置登录启动、面板信息和通知方式。
- **数据留在本机**：无需 TaskPulse 云账号，不上传对话；不会代你批准 Agent 请求。

## 任务提醒

需要补充信息或授权时持续提醒，本轮结束后提醒查看结果。

<p align="center">
  <img src="assets/screenshots/notice-input.png" width="320" alt="待输入弹窗：确认导出文件格式">
  <br><sub>待输入：等待你补充信息</sub>
</p>

<p align="center">
  <img src="assets/screenshots/notice-approval.png" width="320" alt="待批准弹窗：运行项目测试需要授权">
  <br><sub>待批准：等待你确认执行请求</sub>
</p>

<p align="center">
  <img src="assets/screenshots/notice-completed.png" width="320" alt="任务完成弹窗：快速入门指南本轮结束">
  <br><sub>任务完成：回来查看本轮结果</sub>
</p>

## 安装

### 下载安装包

从 [GitHub Releases](https://github.com/qingtian-4/TaskPulse/releases) 选择对应系统：

| 系统 | 安装包 |
| --- | --- |
| Windows 10 / 11 · x64 | `.exe` 安装向导，或 `.msi` |
| macOS 13.3+ · Apple Silicon | `macos_arm64.dmg` |
| macOS 13.3+ · Intel | `macos_x64.dmg` |

### 一条命令安装

**Windows**：打开 PowerShell：

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

**macOS**：打开终端：

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

脚本选择对应架构、校验 SHA-256 并安装，无需 Bun、Rust 或克隆仓库。Mac 默认安装到 `~/Applications`。系统安全提示和指定版本等选项见[安装说明](docs/INSTALL.md)。

首次启动，确认发现的本机 Agent 并点击**开始监控**。之后从托盘查看任务；Windows 也可按 `Ctrl+Shift+A`。

默认面板展示运行中、明确待处理、结果未查看和手动固定的任务。来源暂未更新的任务可按需查看，未读状态会继续保留。

应用会自动检查新版本；在**设置 → 应用更新**中点击安装，下载和签名校验完成后重启。也可关闭自动检查或手动检查更新。

## 支持的 Agent

| Agent / 任务来源 | 接入方式 |
| --- | --- |
| Codex | 本机自动发现；另支持独立 WSL 与 SSH 来源 |
| Claude Code | 本机自动发现；可选官方 Hook 提供精确事件 |
| Hermes | 本机原生数据库与运行状态 |
| DeepSeek Harness | 本机 Desktop / CLI；另支持独立 WSL 来源 |
| ZCode | 本机原生会话、项目、模型与任务状态 |
| 豆包 | 本机桌面端原生执行状态 |
| Gemini CLI | 进程发现；任务事件可通过通用接入补充 |
| Kimi Code CLI | 配置标准事件或 MCP 接入 |
| WorkBuddy | 本机自动发现，读取本地任务、模型与执行记录 |
| OpenCode | 本机自动发现，读取会话数据库与明确结束记录 |
| TRAE 中国版 IDE | 配置标准事件或 MCP 接入 |
| 其他 Agent / 命令行任务 | 标准事件、MCP、兼容 Hook、`tp-run` 和远程 tmux |

自动发现适用于已适配的本机数据格式；WSL、SSH 和事件接入按需配置。不同 Agent 提供的状态不同，未提供的模型或审批信息不会猜测。[兼容说明](docs/AGENT_COMPATIBILITY.md) · [接入指南](docs/AGENT_INTEGRATION_GUIDE.md)

## 路线图

- [x] 本机、WSL 与远程任务统一查看
- [x] 模型显示、运行计时、提醒与未读保留
- [x] Windows / macOS 安装包与命令行安装
- [x] 自动发现本机来源、应用内更新
- [ ] 扩展更多 Agent 的原生自动接入
- [x] 浅色 / 深色主题、简体中文 / English 界面切换
- [ ] 桌面端稳定后推出 Android / iOS App，随时查看任务与提醒
- [ ] 支持在智能手表、手环上查看任务进度与提醒

## 从源码构建

需要 Bun、Rust 1.90+ 和 [Tauri 开发环境](https://v2.tauri.app/start/prerequisites/)。

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri build
```

安装包位于 `src-tauri/target/release/bundle/`。本地运行使用 `bun run tauri dev`。

## 贡献与许可

欢迎[反馈问题](https://github.com/qingtian-4/TaskPulse/issues)、改进文档或贡献 Agent 适配器。[贡献指南](CONTRIBUTING.md) · [安全反馈](SECURITY.md) · [行为准则](CODE_OF_CONDUCT.md)

基于 [AgentTray](https://github.com/sprklai/agenttray) 开发，保留原始版权与贡献历史，采用 [MIT License](LICENSE)。
