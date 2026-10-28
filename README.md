<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/wordmark-light.svg">
  <img src="assets/brand/wordmark-light.svg" width="320" alt="TaskPulse">
</picture>

**多个 Agent 同时跑，先看哪个需要你。**

把本机、WSL 和 SSH 上的任务收进一个托盘面板。<br>谁在运行、谁等你处理、哪些结果还没看，一眼找到。

[English](README.en.md) · [安装](#安装) · [功能](#功能) · [支持的 Agent](#支持的-agent)

<p><img alt="Windows 10 / 11" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4?style=flat-square"> <img alt="macOS experimental" src="https://img.shields.io/badge/macOS-experimental-555?style=flat-square"> <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/License-MIT-76DFCD?style=flat-square"></a></p>

</div>

## 安装

> **0.2.0-alpha.8 尚未公开发布。** [下载测试包](docs/CANDIDATE_TESTING.md)。以下一行命令面向后续公开稳定版。

**Windows 10 / 11（x64）**，打开 PowerShell：

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

**macOS（Apple Silicon / Intel，实验支持）**，打开终端：

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

脚本下载对应平台的安装包，校验 SHA-256 后安装。也可以从 [Releases](https://github.com/qingtian-4/TaskPulse/releases) 手动下载 Windows EXE/MSI 或 Mac DMG，无需 Bun、Rust 或克隆源码。

首次启动后选择要关注的 Agent 和项目。日常从托盘打开面板，Windows 也可按 `Ctrl+Shift+A`。

<details>
<summary>升级、Alpha 与安装选项</summary>

升级前从托盘退出 TaskPulse，再运行安装命令。Windows 会打开标准安装向导；Mac 默认安装到 `~/Applications`。当前候选未做发行者签名/Apple 公证，系统可能要求确认。

公开 Alpha 使用 `-Prerelease`（Windows）或 `--prerelease`（Mac）显式选择。指定版本、仅下载、卸载和状态保留见[安装说明](docs/INSTALL.md)与[命令行选项](docs/CLI_INSTALL.md)。

</details>

## 功能

- **一个面板，多处任务**：汇总本机、WSL 和已配置的 SSH 来源，按项目或 Agent 查找，不用逐个切换窗口。
- **需要你时再提醒**：提问、批准、失败和本轮结束有独立提醒，提示你回到原 Agent 处理。
- **结果留到你看过**：结束但未查看的任务保留在关注列表，固定和未读状态可跨重启恢复。
- **模型与时间一起看**：任务标题、项目、来源上报的模型和本轮经过时间同屏展示，支持搜索模型名。
- **思考中，仍是运行中**：工具执行和等待模型响应不会被误判为完成；来源失联会显示状态待更新。

## 支持的 Agent

| 来源 | 接入方式 |
| --- | --- |
| Codex | 本机原生会话、独立 WSL 来源、已配置的 SSH 会话 |
| Claude Code | 本机只读记录；精确事件可显式启用官方 Hook |
| Hermes | 本机原生数据库与运行状态 |
| DeepSeek Harness | 本机 Desktop/CLI，另有独立 WSL 来源 |
| 其他 Agent 与命令行任务 | 标准事件、MCP、兼容 Hook 和 `tp-run` |

豆包及部分扩展适配器仍属实验支持。各来源的事件覆盖不同，详见[兼容矩阵](docs/AGENT_COMPATIBILITY.md)；开发适配器可读[接入指南](docs/AGENT_INTEGRATION_GUIDE.md)。

## 数据与隐私

TaskPulse 从本地记录和已配置的来源获取任务状态，保存在本机，不需要 TaskPulse 云账号，不向自有云上传对话。SSH 仅连接你配置的服务器；不会自动安装 Agent Hook，也不会替你批准请求。[工作原理](docs/ARCHITECTURE.md) · [数据说明](docs/INSTALL.md#数据与隐私)

## 贡献与许可

欢迎[反馈问题](https://github.com/qingtian-4/TaskPulse/issues)、改进文档或提交适配器。[贡献指南](CONTRIBUTING.md) · [路线图](docs/ROADMAP.md) · [安全反馈](SECURITY.md)

基于 [AgentTray](https://github.com/sprklai/agenttray) 开发，保留原始版权与贡献历史，采用 [MIT License](LICENSE)。

## 从源码构建

需要 Bun、Rust 1.89+ 和 [Tauri 开发环境](https://v2.tauri.app/start/prerequisites/)。

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri build
```

安装包输出到 `src-tauri/target/release/bundle/`。开发时使用 `bun run tauri dev`，详见[开发指南](docs/DEVELOPMENT.md)。
