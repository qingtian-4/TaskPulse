<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
  <img src="assets/brand/wordmark-light.svg" width="320" alt="TaskPulse">
</picture>

# TaskPulse

**不必盯着 Agent，需要你时再回来。**

让多个 Coding Agent 在后台各自执行任务，TaskPulse 替你关注 Agent，把注意力留给你正在做的事情。

[下载安装](#安装) · [快速上手](#开始使用) · [支持的 Agent](#支持的-agent) · [English](README.en.md)

Windows 10 / 11 · macOS 13.3+ · [MIT](LICENSE)

</div>

TaskPulse 是一款轻量级桌面工具，统一关注 Codex、Claude Code 等 Coding Agent 的任务状态。当来源报告需要批准、等待输入、执行完成或发生异常时，它会及时提醒你。

无需反复切换窗口检查进度。收到提醒后，再回来处理真正需要你介入的事情。

## 安装

从 [GitHub Releases](https://github.com/qingtian-4/TaskPulse/releases) 选择安装包：

| 系统 | 安装包 |
| --- | --- |
| Windows 10 / 11 · x64 | `.exe` 或 `.msi` |
| Windows 11 · ARM64 | `windows_arm64_setup.exe` |
| macOS 13.3+ · Apple Silicon | `macos_arm64.dmg` |
| macOS 13.3+ · Intel | `macos_x64.dmg` |

也可一条命令安装，无需克隆仓库或安装编程工具。

**Windows · PowerShell**

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

**macOS · 终端**

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

脚本安装并校验最新公开版本；Mac 默认安装到 `~/Applications`。[系统安全提示、指定版本与卸载](docs/INSTALL.md)。当前源码版本为 **v0.0.1**，可下载版本以 Releases 为准。1.0.x 用户请先阅读[版本迁移](docs/INSTALL.md#版本迁移)。

## 开始使用

1. **选择来源**：首次启动，在“监控来源 → 本机”确认要关注的 Agent，应用更改。本机数据自动发现，SSH 服务器按需添加。
2. **照常工作**：回到 Agent 开始任务，从托盘查看运行、待处理和未读结果。Windows 可按 `Ctrl+Shift+A` 打开面板。
3. **收到提醒再回来**：在原 Agent 中回答或批准。匹配的来源事件解除请求；看完结果后手动标记已查看。

关闭提醒、打开详情和聚焦窗口都不会标记已读，更不会代你批准。通知、声音、项目范围、字号与更新均可在设置中调整。

## 功能

- **专注当下，无需守候**：多个 Agent 同时运行，不必一直盯着终端。
- **关键时刻主动提醒**：根据来源的明确证据，提醒输入、审批、任务结果和异常。
- **需要处理的任务，一目了然**：集中显示任务、模型、推理强度与耗时；最多四张卡片随内容适应，更多任务在列表内滚动。
- **重要结果不会错过**：未读结果、固定任务和未解决请求跨重启恢复。读取异常保留上次可信状态。
- **自动发现，灵活接入**：管理本机 Agent、多个 SSH 服务器和项目范围；WSL、标准事件与 MCP 按需配置。
- **本地运行，尊重隐私**：无需 TaskPulse 云账号，不上传 Agent 对话，不自动修改 Agent Hook 或审批策略。

## 任务面板

<p align="center"><a href="assets/screenshots/dashboard.png"><img src="assets/screenshots/dashboard.png" width="368" alt="TaskPulse v0.0.1：运行中、等待输入、请求批准与未读结果"></a></p>

Windows 原生界面，使用不含私人数据的示例任务。模型、推理强度和起止时间仅在来源提供时显示。

## 任务提醒

<p align="center">
  <a href="assets/screenshots/notice-input.png"><img src="assets/screenshots/notice-input.png" width="32%" alt="等待输入提醒"></a>
  <a href="assets/screenshots/notice-approval.png"><img src="assets/screenshots/notice-approval.png" width="32%" alt="请求批准提醒"></a>
  <a href="assets/screenshots/notice-completed.png"><img src="assets/screenshots/notice-completed.png" width="32%" alt="本轮结束提醒"></a>
</p>

从左到右：等待输入、请求批准、本轮结束。点击截图查看原图。

## 支持的 Agent

| Agent | 本机能力 |
| --- | --- |
| Codex | 原生任务、模型、明确提问及结束；审批需已有明确事件通道。另支持 WSL、SSH |
| Claude Code | 原生会话观测；可选官方 Hook 提供明确审批与生命周期。原生提问、SSH 尚未支持 |
| DeepSeek Harness | Desktop / CLI 任务、模型、推理强度；按请求 ID 匹配提问、审批与解除 |
| Kimi Code / kimi-cli | 主会话 wire 日志中的轮次、模型、审批与提问，范围取决于格式 |
| Qoder | Desktop 轮次投影、IDE / CLI 转录；完整结束证据与时长。原生审批、提问尚未支持 |
| OpenCode、WorkBuddy、ZCode、Hermes、豆包 | 原生任务观测；模型、阶段与结束能力随来源而异 |
| TRAE | 公开工作区会话发现；当前主会话库的任务状态、模型与请求读取受限 |
| Gemini CLI 与其他来源 | 进程识别仅发现运行进程；完整提醒需宿主发送标准事件或 MCP 事件 |

安装了 Agent 不等于所有提醒都可用。连接、检测、读取健康和当前任务分别显示；没有证据时不会猜测请求或结束。[能力与平台范围](docs/AGENT_COMPATIBILITY.md) · [接入指南](docs/AGENT_INTEGRATION_GUIDE.md)

有可靠身份信息时才聚焦所属宿主窗口，不保证切换到精确会话。无法返回时可查看详情并复制路径或会话 ID。各平台的实机验证范围见兼容说明。

## 工作原理

<p align="center"><a href="assets/architecture/architecture-zh.svg"><img src="assets/architecture/architecture-zh.svg" width="900" alt="TaskPulse 架构：只读来源、可信观测、持久关注与桌面提醒"></a></p>

Rust 负责有界采集、请求匹配与持久状态，Tauri 提供系统窗口与通知，Svelte 呈现同一份状态快照。[架构与模块职责](docs/ARCHITECTURE.md)

## 路线图

- [ ] 扩展原生事件读取与 Agent 版本覆盖。
- [ ] 完善跨平台返回任务、多屏与睡眠恢复。
- [ ] 完成签名、公证与安装升级的实机覆盖。
- [ ] 评估移动端与可穿戴设备提醒。

[完整路线图 / Roadmap](ROADMAP.md) · [使用与故障排查](docs/README.md)

## 本地开发

安装 Bun、Rust 1.90+ 和 [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)，然后运行：

```sh
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run dev:desktop
```

开发应用使用隔离配置目录，保留已安装应用的数据。[贡献指南](CONTRIBUTING.md) 包含检查与维护约定。

## 从源码构建

安装上述依赖后，在仓库根目录运行：

```sh
bun run tauri build
```

默认安装包输出到 `src-tauri/target/release/bundle/`。

构建 Windows ARM64 安装包，需先安装 MSVC C++ ARM64 构建工具和 Windows SDK，然后运行：

```powershell
rustup target add aarch64-pc-windows-msvc
bun run build:windows:arm64
```

默认输出到 `src-tauri/target/aarch64-pc-windows-msvc/release/bundle/nsis/`。ARM64 安装包仅在对应 Release 实际提供时可下载，不以 x64 包替代。[构建前置条件](https://v2.tauri.app/distribute/windows-installer/#building-for-32-bit-or-arm)

## 贡献与许可

[报告问题](https://github.com/qingtian-4/TaskPulse/issues) · [贡献指南](CONTRIBUTING.md) · [安全报告](SECURITY.md) · [行为准则](CODE_OF_CONDUCT.md)

基于 [AgentTray](https://github.com/sprklai/agenttray)，保留版权与贡献历史。采用 [MIT 许可](LICENSE)，依赖许可见 [licenses](licenses/README.md)。
