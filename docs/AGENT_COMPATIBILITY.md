# 支持的 Agent 与接入方式

[English](#english)

TaskPulse 关注正在执行、等待处理和结束后未查看的任务。原生接入只读本机数据；不会导入整个空闲历史，也不会因日志静默或工具返回而宣告任务完成。

## 本机自动发现

首次点击“开始监控”后，自动发现会定期检查新的本机来源。已关闭或移除的来源不会自动恢复；自定义目录与项目范围保留。

| Agent | 读取来源 | 任务状态与边界 |
| --- | --- | --- |
| Codex | `CODEX_HOME` 或 `~/.codex` 中的元数据与会话日志 | 原生任务、项目、模型和执行阶段；读取宿主留下的明确提问，审批需有明确事件，见[审批观察器](CODEX_APPROVAL_OBSERVER.md) |
| Claude Code | `CLAUDE_CONFIG_DIR` 或 `~/.claude` | 原生会话观测；精确生命周期及审批可用可选官方 Hook，原生提问未支持 |
| Hermes | 原生 SQLite 数据库与运行租约 | 标题、项目、模型及运行状态；缺少结束证据时保留待更新状态 |
| DeepSeek Harness | `DSH_HOME` 或 `~/.dsh`，可用 `TASKPULSE_DSH_HOME` 覆盖 | Desktop / CLI 原生会话，支持旧日志与 v4 日志；明确提问按调用与问题 ID 匹配回答，审批按请求 ID 匹配决定；读取请求中报告的推理强度 |
| OpenCode | `~/.local/share/opencode/opencode.db`，支持 `XDG_DATA_HOME` | 读取运行阶段、模型与明确结束记录；支持 v2 和旧版会话表；不提供独立提问或审批提醒 |
| WorkBuddy | `~/.workbuddy/workbuddy.db` 与 `projects/*.jsonl` 会话目录 | 数据库与增量日志关联本轮开始/结束；不包含云端任务，不提供独立提问或审批提醒 |
| ZCode | `~/.zcode/cli/db/db.sqlite` | 原生标题、项目、模型、工具阶段与整轮结果；不提供独立审批事件 |
| 豆包 | 桌面端原生执行存储 | 当前工作任务与执行状态；不提供审批提醒；目录设置见[本机来源](LOCAL_AGENTS.md) |

原生数据格式可能随 Agent 版本改变。读取失败会显示来源异常或状态待更新，不伪造任务结束。来源没有提供模型时不猜测模型；代理所报告的模型名不保证揭示代理背后的实际模型。

OpenCode 2.2.2 和 WorkBuddy 5.9.3 的 Windows 本机记录已做只读核对；这不代表所有旧版、国际版或 macOS 版本均已实机验收。接入操作见[本机 Agent](LOCAL_AGENTS.md#opencode-与-workbuddy)。

## 平台与返回任务

Windows 10/11 x64 是主要验收平台。macOS 13.3+ 提供 Intel 与 Apple Silicon 包；编译和打包通过不代表每个 Agent、每种宿主都已实测。Linux 桌面当前没有同等交付验收。Windows 的 WSL 是独立来源，不等同于 Linux 桌面支持。

只有能核验所属进程和窗口时，TaskPulse 才会把宿主聚焦当作成功。聚焦窗口不代表已跳转到精确会话。无法可靠返回时，请打开任务详情并复制路径或会话 ID。关闭通知、返回宿主都不会自动标为已读或批准。

## 按需连接

| Agent / 来源 | 如何接入 |
| --- | --- |
| Codex WSL | 添加独立发行版来源，需要运行中的 WSL 和 Python 3；不会唤醒停止的发行版 |
| Codex SSH | 添加你自己的 SSH 主机与项目范围 |
| Gemini CLI | 启用进程扫描发现运行进程；完整任务生命周期需要事件接入 |
| Kimi Code CLI | 选择适配器，连接标准事件或 MCP；宿主需发送对应事件 |
| TRAE 中国版 IDE | 选择适配器，连接标准事件或 MCP；宿主需发送对应事件 |
| ZCode 事件接入 | 除原生读取外，可按需使用标准事件 / MCP |
| Qoder | 原生读取尚未适配；IDE 与 CLI Hook 不等同，可通过“其他 Agent（标准事件）”手动接入 |
| 其他 Agent | 使用标准事件、MCP 或兼容状态文件 |
| 命令行与远程 tmux | `tp-run` 可报告前台子进程退出码；普通 shell 返回只能表示推断结束 |

本机列表下的“事件接入”提供 Kimi Code、TRAE 及其他标准事件入口；接入能力与原生自动发现分开显示。

事件接入需要宿主实际发送开始、结束、失败或请求事件，安装一个 Agent 并不会自动补齐这些能力。TaskPulse 不会自动修改 Agent Hook、代理或审批策略。[接入指南](AGENT_INTEGRATION_GUIDE.md) · [标准事件协议](AGENT_INTEGRATION_PROTOCOL.md)

## 提醒规则

- 思考、工具执行和等待模型响应都保持运行中。
- 仅明确的审批请求或提问产生待处理提醒；耗时和权限参数不是请求证据。
- 工具完成只清除匹配的工具活动，不结束整轮任务。
- 同一任务重命名不改变身份，不重新计时或重复提醒。
- 有明确开始和结束时间的来源在结束后保留本轮总时长；没有结束时间的旧记录不补造时长。进程扫描不提供任务时长或完整生命周期。
- 模型使用来源报告的标识。DSH 的 `deepseek-flash` 是模型别名；产品显示名可能随宿主提供商目录变化，不据此猜测底层版本。
- 关闭提醒不等于查看结果。未读结果、固定任务和未解决请求仍可回查。


## English

TaskPulse tracks active turns, explicit requests and unread results. Local discovery reads supported native data after the initial confirmation. The Sources page lists detected and configured agents; use Advanced to manage WSL, SSH, events or custom directories.

| Agent or connection | Evidence |
| --- | --- |
| Codex | Native metadata and logs can report sessions, models, turn outcomes and explicit questions. Permission requests require an existing explicit event channel; see the [approval observer guide](CODEX_APPROVAL_OBSERVER.md). WSL and SSH are configured separately. |
| Claude Code | Native logs provide session evidence. The optional official hook provides explicit approval and lifecycle events. Native questions are unsupported. |
| Hermes, DeepSeek Harness, OpenCode, WorkBuddy, ZCode, Doubao | Native tasks and models where the Agent supplies evidence. A running lease or completed tool call alone cannot establish a turn outcome. OpenCode and WorkBuddy native records do not expose independent approvals or questions. |
| DeepSeek Harness | Native `ask_user_question` calls are resolved only by matching call/question IDs; approval decisions match their native request IDs. Request configuration supplies reasoning effort. The reported model ID may be an alias. |
| Qoder | Native collection is not adapted. IDE and CLI Hooks differ; manual standard-event integration requires the producer to supply reliable correlation. |
| Gemini CLI, Kimi Code, TRAE and other agents | Process discovery or configured Standard Events, MCP and compatibility files, as available. A host must actually emit the event to support a precise reminder. |
| Shell / tmux | `tp-run` can report the foreground child's exit code. An ordinary tmux shell return is inferred and does not establish an Agent turn outcome. |

Windows 10/11 x64 is the primary validation platform. macOS 13.3+ packages target both Apple Silicon and Intel. Compile and packaging checks do not establish runtime support for every Agent or host. Linux desktop has no equivalent delivery gate; a Windows WSL source is separate.

A verified host window can be focused when TaskPulse can validate its live owner. Host focus does not establish an exact session jump. Use task details or copy the session ID or path when return is unavailable. Dismissing a reminder or focusing a window does not mark a task as read or approve a request. See [installation and troubleshooting](INSTALL.en.md).

Explicit start/end timestamps retain the total turn duration after completion. Missing historical timestamps are not reconstructed from refresh time. Kimi Code and TRAE event adapters are available below the local readers; they require separate setup and do not claim native-host validation.
