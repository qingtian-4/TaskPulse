# TaskPulse

**专注写代码，其他 Agent 的进展交给 TaskPulse。**

Windows 桌面托盘中的多 Coding Agent 任务监控器：统一查看 Codex、Claude Code、Hermes、DeepSeek Harness 等来源的运行状态，在**明确需要回答、审批或本轮结束**时提醒你。支持部分本机、WSL 和 SSH 工作流；不修改 Agent 的模型与审批权限。

[English](README.en.md) · [使用指南](docs/INSTALL.md) · [兼容性](docs/AGENT_COMPATIBILITY.md) · [架构](docs/ARCHITECTURE.md) · [提交问题](https://github.com/qingtian-4/TaskPulse/issues)

> **Windows 优先 · 下一候选源码预览。** 应用版本号仍为 0.2.0-alpha.7，尚无公开可下载的 Alpha.7 安装包。下方源码安装指向默认分支 `main`；新版面板、消息中心和 Runtime 修复不在历史 Alpha.7 候选包内。按 main SHA 构建的 CI 附件与历史候选分开识别，不代表新版本已发布。[构建与发布规则](docs/RELEASING.md)。

## 一眼了解

![TaskPulse Windows 原生任务面板](assets/screenshots/dashboard-current.png)

*真实 Tauri/WebView2 窗口截图，使用隔离合成任务输入；不是实际 Agent 运行截图，也不代表所有适配器均通过宿主验收。*

| 多任务关注 | 明确的人机请求 | 可恢复的结果 |
|---|---|---|
| 按 Agent、项目与来源查看运行中的任务 | 区分需要授权、等待回答和普通工具调用 | 独立提醒、未读、固定和重启恢复 |
| 思考、等待模型和工具执行仍算运行 | 不把日志静默、工具完成或断连猜成结束 | 采集失败时保留上次状态，标记待更新 |

首页聚焦**进行中、待处理、待查看**；待处理属于进行中的一部分，计数不能相加。铃铛只统计未读消息；读消息不会解除原 Agent 的请求，也不会把任务结果标为已查看。

<details>
<summary>查看更多：消息中心、来源管理与独立提醒（原生截图）</summary>

![消息中心：消息已读与任务待处理相互独立](assets/screenshots/message-center-current.png)

![来源管理](assets/screenshots/sources-current.png)

![独立测试提醒](assets/screenshots/notice-current.png)

截图均为真实 WebView2 渲染；来源与任务来自**隔离合成输入**，提醒为明确标注的测试提醒。旧 Alpha.7 截图保留为历史材料。[素材与复现说明](assets/README.md)。

</details>

### 一个典型使用场景

你在本机让 Codex 改代码，同时让 Claude Code 审查，WSL 内另一个 Codex 运行测试。无需持续切换窗口：TaskPulse 汇总任务；真实审批、提问或结果出现时提醒你返回原 Agent。**TaskPulse 不代替原 Agent 批准操作或执行代码。**

## 快速开始

目前没有公开的 Alpha.7 安装包。Windows x64 用户可以从源码运行；需要 Rust 1.89+、Bun、Microsoft C++ Build Tools 和 [Tauri 2 先决条件](https://v2.tauri.app/start/prerequisites/)（含 WebView2）。

```powershell
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri dev
```

启动后：打开托盘中的**来源管理** → 选择/添加 Agent 来源与项目范围 → **保存并生效** → 在原 Agent 中开始任务 → 使用 `Ctrl+Shift+A` 查看关注面板。测试读取或测试提醒成功，不等于真实 Agent 生命周期已验收。WSL / SSH 来源需要分别启用；不会自动启动已停止的 WSL 或连接全部服务器。

[完整安装与配置](docs/INSTALL.md) · [面板使用说明](docs/PRODUCT_DASHBOARD.md) · [WSL 指南](docs/WSL_CODEX.md) · [排障与开发](docs/DEVELOPMENT.md)

## 支持情况

| 来源 | 已有能力 | 限制 |
|---|---|---|
| **Codex** | 本机原生会话；独立 WSL 来源；SSH 会话采集 | 大历史日志首次追赶可能需要时间；WSL 需 Python 3，且仅采集已运行的发行版 |
| **Claude Code** | 本机记录；可选、经用户确认的 Hook | WSL/SSH 无专用 Provider |
| **Hermes** | 本机 SQLite 与运行租约 | 租约消失不等于任务完成 |
| **DeepSeek Harness** | Desktop/CLI，及独立 WSL 来源 | 本机账号额度不代表远程额度 |
| **Gemini CLI** | 基础 Hook / 进程扫描 | 非完整原生会话支持 |
| **Kimi / TRAE / ZCode** | 部分声明式映射、fixture 测试 | 尚未完成真实宿主 Hook 验收 |
| **WorkBuddy** | 候选入口 | 尚无事件适配器 |
| **自定义 Agent** | 标准事件 v1 / 声明式 Manifest | 仅已有证据支持的能力；不提供通用可执行插件加载器 |
| **普通代码运行** | SSH tmux / `tp-run` | 本机/WSL/SSH 统一运行工作流尚未交付；前台退出不证明后台、容器或 Slurm 任务结束 |

**“检测到来源”不等于“能可靠判定本轮结束”。** Kimi / TRAE / ZCode 当前为 **fixture-tested**，不代表 native-tested。Linux/macOS 仅提供可选实验构建，不具有 Windows 原生验收等级。更细的能力与不支持项见 [Agent 兼容性文档](docs/AGENT_COMPATIBILITY.md)。

## 工作原理

![TaskPulse 系统架构](assets/diagrams/taskpulse-architecture.svg)

1. **Provider 采集**：独立 Worker 获取各来源的日志、Hook、SQLite 或远程只读证据，保留来源身份、采集健康和新鲜度。
2. **统一状态与关注**：通过来源/项目过滤、TaskStore、AttentionTracker 管理 Session/Turn、待处理请求、未读与固定。
3. **可信通知**：TransitionEngine 只对可信的状态迁移产生提醒；标准事件通道使用 Journal、Checkpoint、Outbox 支撑有序恢复与去重。
4. **桌面展示**：Rust / Tauri 2 + Svelte 5 显示任务面板、独立消息中心、托盘状态和提醒；消息已读、结果待查看与请求待处理各自独立，stale 不冒充新的运行或完成事实。

没有 TaskPulse 云账号或遥测服务。设置与关注通常保存在 `~/.taskpulse/`；本地活动和诊断可能包含路径、命令片段，请脱敏后分享。TaskPulse 不自动安装真实 Hook 或放宽审批策略。截图复现与录屏方案见 [演示说明](docs/DEMO_RECORDING_PLAN.md)。

[详细架构](docs/ARCHITECTURE.md) · [第三方接入](docs/AGENT_INTEGRATION_GUIDE.md) · [事件协议](docs/AGENT_INTEGRATION_PROTOCOL.md)

## 当前状态与下一步

- **已有**：代码、自动化测试、来源管理、Windows 原生 UI 验收，以及 Alpha.7 NSIS/MSI **候选安装器的 CI 级测试**。
- **尚待公开发行验收**：在干净的 Windows 用户环境测试安装、升级、卸载；至少两类真实 Agent 的完整新任务/审批/通知链路与长时间稳定性；整理脱敏演示录屏。
- **不会冒充已完成**：不把合成截图当真实宿主验收，不把没有证据的结束或输入请求当事实，不把 Draft Release 当公开下载。

[发布清单](docs/OPEN_SOURCE_CHECKLIST.md) · [安全审查](docs/internal/reports/SECURITY_AUDIT_2026-10.md) · [历史修复与验收](docs/internal)

## 贡献与来源

欢迎提交 Issue、文档修正和适配器贡献。提交问题时请注明操作系统、Agent/宿主版本、来源类型和脱敏复现步骤，**不要上传会话正文、私有路径或凭据**。[CONTRIBUTING](CONTRIBUTING.md) · [SECURITY](SECURITY.md) · [Code of Conduct](CODE_OF_CONDUCT.md)。

本项目基于 [sprklai/agenttray](https://github.com/sprklai/agenttray) 二次开发，保留上游版权、致谢和历史，使用 [MIT License](LICENSE)。
