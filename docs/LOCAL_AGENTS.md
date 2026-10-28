# 本机 Agent

TaskPulse 自动发现受支持的本机 Agent。首次确认后，来源中的新执行会自动进入面板；不会把整个历史会话列表导入关注列表。

| 来源 | 默认数据位置 | 自定义位置 |
| --- | --- | --- |
| Codex | `~/.codex` | 环境变量 `CODEX_HOME` |
| Claude Code | `~/.claude` | `CLAUDE_CONFIG_DIR` 或来源目录 |
| Hermes | 本机 Hermes 状态数据库 | 来源中填写数据库路径 |
| DeepSeek Harness | `~/.dsh` | `DSH_HOME` / `TASKPULSE_DSH_HOME` 或来源目录 |
| ZCode | `~/.zcode/cli/db/db.sqlite` | 来源中填写数据库路径 |
| 豆包 | 桌面端 IndexedDB | 见[豆包说明](DOUBAO.md) |

`~` 表示运行 TaskPulse 的用户主目录。设置环境变量后，需要重新启动 TaskPulse，使其读取新环境。

## 范围与状态

默认关注所有项目的新执行。来源管理可以限制项目、排除项目或停用来源。自动发现不会重新打开你主动关闭的来源。

任务使用稳定会话 ID；重命名只更新标题。模型显示来源报告的标识，代理后的实际模型未必可见。任务的思考、工具执行和等待模型响应均属于运行中。

数据暂时不可读时，TaskPulse 保留上次状态并显示待更新，不根据日志静默判断结束。原生记录的支持范围见[兼容说明](AGENT_COMPATIBILITY.md)。

## 接入精确事件

Claude Code 可在来源高级设置中显式安装官方 Hook；原有 Hook、statusLine、代理和模型设置保留，也可以撤销。Codex 的审批事件见 [Codex 审批提醒](CODEX_APPROVAL_OBSERVER.md)。

其他 Agent 使用[标准事件或 MCP](AGENT_INTEGRATION_GUIDE.md)。通用事件与进程扫描都不会自动安装 Hook 或更改审批策略。
