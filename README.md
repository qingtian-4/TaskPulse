<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/wordmark-light.svg">
  <img alt="TaskPulse: Your agents, one glance." src="assets/brand/wordmark-light.svg" width="410">
</picture>

# TaskPulse

**专注写代码，其他 Agent 的进展交给 TaskPulse。**

![Windows first](https://img.shields.io/badge/Windows-first-76DFCD?style=flat-square)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?style=flat-square)
[![MIT](https://img.shields.io/badge/License-MIT-blue?style=flat-square)](LICENSE)

[English](README.en.md) · [快速开始](#快速开始) · [支持的 Agent](#支持的-agent) · [路线图](#路线图) · [反馈问题](https://github.com/qingtian-4/TaskPulse/issues)

</div>

TaskPulse 是一个多 Coding Agent 任务监控器。把本机、WSL 和 SSH 环境中的任务汇总到桌面托盘，在 Agent **需要回答、等待批准或本轮结束**时提醒你，减少反复切换窗口查看进度。

> 当前为 **0.2.0-alpha.8**，尚未公开发布。[候选安装包与测试说明](docs/CANDIDATE_TESTING.md)。Windows 优先，macOS/Linux 为实验支持；手机 App 在规划中。

## 看一眼产品

<p align="center"><img src="assets/screenshots/dashboard-current.png" alt="TaskPulse 任务面板：进行中、待处理与待查看" width="412"></p>

| 等待你回答 | 等待你批准 |
|---|---|
|![待输入弹窗](assets/screenshots/notice-question-current.png) |![待批准弹窗](assets/screenshots/notice-approval-current.png) |

*真实 Tauri/WebView2 截图，使用隔离合成任务测试。弹窗提醒你回到原 Agent 处理；关闭提醒不等于批准或解决请求。[素材与复现](assets/README.md)。*

<details>
<summary>消息中心与来源管理</summary>

![消息中心](assets/screenshots/message-center-current.png)

![来源管理](assets/screenshots/sources-current.png)

</details>

## 快速开始

安装版无需 Bun、Rust 或克隆源码。**候选尚未公开发布**；测试者按[候选下载说明](docs/CANDIDATE_TESTING.md)登录 Actions 获取精确源码 SHA 的附件。PR 预览仅为测试附件，不是公开版本。

| 平台 | 安装包 | 命令行入口 | 当前阶段 |
|---|---|---|---|
| Windows 10/11 x64 | NSIS EXE / MSI | [`install.ps1`](docs/CLI_INSTALL.md#windows-1011-x64) | 主要平台；当前 PR 的包仍需独立验收 |
| macOS Apple Silicon | arm64 DMG | [`install.sh`](docs/CLI_INSTALL.md#macos-apple-silicon-and-intel) | 实验、ad-hoc 签名、未公证；当前 PR 与实机检查待完成 |
| macOS Intel | x64 DMG | [`install.sh`](docs/CLI_INSTALL.md#macos-apple-silicon-and-intel) | 实验；独立原生构建和实机检查待完成 |

命令行复用公开 Release 的安装包，须等脚本合入 main 且版本经审查公开后使用。默认稳定版，Alpha 须明确选择；Draft 和 PR 预览不可安装。Linux 仍按[源码开发说明](docs/DEVELOPMENT.md)实验验证。

启动后打开 **来源管理 → 选择 Agent 与项目 → 保存并生效**，然后在原 Agent 中开始任务。使用 `Ctrl+Shift+A` 打开任务面板。

[安装与升级](docs/INSTALL.md) · [命令行参数](docs/CLI_INSTALL.md) · [WSL](docs/WSL_CODEX.md) · [Mac](docs/MACOS_DEVELOPMENT.md)

## 核心特性

- **统一关注面板**：按 Agent、项目和来源查看进行中、待处理、失败与未查看结果。
- **聚合托盘状态**：待处理优先于异常、未查看结果和运行中，多任务同时执行也能发现阻塞。
- **明确事件提醒**：思考和工具执行仍算运行；日志静默、CPU 空闲或采集失败不会被当成完成。
- **可恢复的关注记录**：未查看结果与固定任务跨重启保留；消息已读、结果已查看、请求已解决分别管理。

## 支持的 Agent

| 来源 | 当前接入方式 |
|---|---|
| Codex | 本机原生会话、独立 WSL 来源、已配置的 SSH 会话 |
| Claude Code | 本机只读记录、用户显式启用的官方 Hook |
| Hermes / DeepSeek Harness | 本机原生数据；DeepSeek Harness 另有独立 WSL 来源 |
| 豆包桌面（实验） | Windows 原生运行检查点；审批未适配，结束提醒不保证 |
| Gemini CLI / SSH tmux / `tp-run` | 兼容 Hook、进程扫描或运行包装器 |
| Kimi / TRAE / ZCode | 部分事件映射，fixture-tested，真实宿主验收待完成 |
| 其他 Agent | 标准事件、声明式 Manifest；支持本机 MCP 的宿主可主动上报 |

各来源的环境、事件覆盖与验证程度不同，详见[兼容性矩阵](docs/AGENT_COMPATIBILITY.md)。想接入更多 Agent，请看[接入指南](docs/AGENT_INTEGRATION_GUIDE.md)。

## 模型信息与本轮计时

Codex 从原生元数据和 rollout 上下文读取模型；Claude Code 从 assistant 记录读取；DeepSeek Harness 从会话/日志状态读取；Hermes 从 SQLite 会话/消息读取。缺失、空值或尚不支持的来源保持未知。报告或代理模型不证明背后的实际服务，模型变化不会改变任务身份、未读或通知去重。

本轮经过时间包括思考、工具执行与等待模型响应，不是纯模型计算时长。开始时间缺失不补造，来源更新时间另行展示；只有明确请求才建立待批准状态。[来源与验证边界](docs/AGENT_COMPATIBILITY.md#模型信息与本轮计时)。

![合成模型示例](assets/screenshots/models-synthetic.png)

*隔离合成 transcript 的原生 WebView2 截图，展示已报告模型和未提供模型的任务，不代表真实宿主认证。*

## 工作原理

![TaskPulse 中文架构图](assets/diagrams/taskpulse-architecture-zh.svg)

**采集来源 → 统一任务状态 → 筛选需要关注的任务 → 托盘、面板与事件提醒。**

基于 Rust、Tauri 2 和 Svelte 5。当前状态保存在本机，无 TaskPulse 云账号要求；来源失联保留不确定状态，真实审批仍由原 Agent 处理。[详细架构](docs/ARCHITECTURE.md)。

## 隐私边界

TaskPulse 读取配置范围内的本地记录及明确启用的 WSL/SSH 来源。原生记录可能含提示词、标题和路径；TaskPulse 不向自有云上传对话，也不要求云账号。GitHub 安装下载、系统通知和已配置 SSH 仍各有网络或系统边界。设置、关注和通知去重保存在 `~/.taskpulse/`，诊断分享前须核对。不会自动安装 Agent Hook 或代为批准请求。[数据边界](docs/INSTALL.md#数据与隐私)。

## 源码开发

开发者需要 Bun、Rust 1.89+ 和 [Tauri 环境](https://v2.tauri.app/start/prerequisites/)：

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri dev
```

见[开发与排障](docs/DEVELOPMENT.md)。普通安装无需这些工具。说明与安装输出维护简中/英文，全量界面国际化不在本次范围内。

## 路线图

- [ ] **手机 App**：在手机查看已配对电脑的任务，接收待回答、待批准、失败和完成提醒。
- [ ] **更多 Coding Agent**：扩展官方事件、原生数据和 MCP 接入口，逐个补齐真实宿主验证。
- [ ] **跨设备体验**：设备配对、断线状态、通知去重与阅读状态同步。
- [ ] **桌面稳定性**：完善安装升级、多屏 DPI 与长期运行体验。

以上为规划，手机端和跨设备通知尚未实现。[需求范围与阶段划分](docs/ROADMAP.md)。

## 贡献与许可

欢迎提交问题、文档和适配器改进。代码通过 **功能分支 → Pull Request → 检查与审查 → 合入 main** 协作。[贡献指南](CONTRIBUTING.md) · [合入 main 的说明](docs/GIT_WORKFLOW.md) · [安全反馈](SECURITY.md)。

基于 [sprklai/agenttray](https://github.com/sprklai/agenttray) 开发，保留上游版权与历史，采用 [MIT License](LICENSE)。
