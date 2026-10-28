# agent-speed 对照与本次范围

研究日期：2026-10-28。参考固定到 `zoefix/agent-speed` 的 `a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c`（v1.4.1），避免 main 漂移。本文只记录已读取的实现，不把对方仓库文档当成本仓库指令。没有复制对方代码或视觉素材。

## 架构与数据来源

Agent Speed 的 macOS 前端为 SwiftUI/AppKit，Windows 为 WPF，核心解析与速度计算分别在 Swift/C# 实现，以共享规格、fixtures 和 Python 参考实现核对。整体链路是本地文件/数据库发现、增量逐行解析、统一 response/phase/model/project、去重、速度引擎/日汇总、仪表盘。TaskPulse 使用 Rust/Tauri/Svelte 和 Provider → Store → Attention → Transition → UI，二者可借鉴数据边界而不必换技术栈。

[测量规格](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/spec/METRICS.md)给出各 Agent 的原生存储路径和解析规则。Codex 读 rollout；Claude 读 projects JSONL；DSH 读压缩 session 事件；OpenCode 等读取各自 SQLite/JSON。仅来源明确给出的模型字段才可展示，不从 Agent 品牌猜模型，不读取凭据或修改来源配置。

- [CodexLine](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/windows/src/AgentSpeed.Core/Parsing/CodexLine.cs)：`turn_context.payload.model`，会话 metadata 提供上下文。
- [ClaudeParser](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/windows/src/AgentSpeed.Core/Parsing/ClaudeParser.cs)：assistant message 的 model，跳过 synthetic；重复 message 合并。
- [DshLine](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/windows/src/AgentSpeed.Core/Parsing/DshLine.cs)：request header config model 和 assistant message source model。响应可给出与请求别名不同的实际报告名。
- Hermes 不在此参考项目的来源列表，本次依照本机只读数据库核对 `sessions.model`；老 schema 缺少该列仍能采集。

速度规格把输出 token 除以一次模型请求的端到端耗时（含首 token 等待），进行中的速度可以估计，最终使用测量值纠正。TaskPulse 的任务计时覆盖完整轮次，包括思考、工具、模型等待，因此不能把 TPS 计算或对方 idle 判断移植成 TaskPulse 完成事件。TaskPulse 仍只依据明确生命周期事件通知。

## 展示信息取舍

| 参考项目信息 | TaskPulse 决定 |
| --- | --- |
| 来源、项目、模型、执行阶段、子 Agent | 本次补模型；保留现有来源/项目/任务身份与阶段 |
| 当前/峰值/平均 TPS、token、请求量 | 明确不纳入本次及后续补齐方案 |
| API 等值费用、网络流量 | 本次不新增，无需价格网络请求、辅助服务或遥测 |
| 日汇总、各模型历史、CSV | 保留 TaskPulse 关注/未读状态语义，不导入全量旧会话 |
| 悬浮球和动态仪表盘 | 沿用紧凑任务列表，模型一行可见且长名称换行 |

模型是“来源最近上报的名称”，可能是路由/代理别名，不证明后端真实模型身份。Codex 本地/WSL 共用增量解析；SSH 有模型上下文时可显示，否则保持未知。Claude Code 即使使用 DeepSeek 代理也仍标记为 Claude Code。模型切换不改变 stable session ID、turn key、开始时间或通知事件。详情显示未知来源，搜索和复制包含模型。

## 安装与发布借鉴

[安装脚本](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/install.ps1)采用 GitHub Release 下载与 SHA-256 校验，手动解包和维护卸载/自启项。TaskPulse 采用同样的下载校验思路，复用现有 NSIS 安装器，不复制对方手动安装、按进程名停止或默认开启自启的逻辑。

[发布工作流](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/.github/workflows/release.yml)在 tag 上验证版本、分别构建两平台、汇总资产和 SHA256，再组合版本说明与模板；若已有 draft 则留待人工发布。本项目已有更严格的 Windows 质量/安全/独立安装器门禁和 draft-only 策略，应保留。可学习固定资产命名、明确语言与升级说明；不引入直接公开、版本覆盖或实验平台稳定承诺。

本次新增 `install.ps1`，仅简体中文/英文；默认 stable，显式 `-Prerelease` 或 `-Version` 可选择公开 Alpha，`-DownloadOnly` 用于审阅校验。当前候选仍为 Draft，真实公共下载路径须在未来明确批准公开后验收。未创建标签、发布 Release、合入 main 或清理本功能 worktree。

## 后续补齐

README 重整、macOS 命令行安装、当前 PR 双平台安装包及验收的具体范围见
[后续执行方案](MODEL_INSTALL_FOLLOWUP_PLAN.md)。方案区分已经实现、仓库既有能力与待做事项，不新增测速功能。
