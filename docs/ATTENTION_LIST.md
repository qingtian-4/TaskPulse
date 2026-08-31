# 动态关注列表与数据源

TaskPulse 主界面不是 Agent 历史会话浏览器。它只显示当前需要用户关注的任务：正在运行、刚结束且未查看、失败、等待输入，以及手动固定的任务。任务数量不设上限。

## 数据源审计

| Agent | 稳定会话 ID | 原生标题 | 项目 | turn 事件 | 重命名 | 原生已查看 |
|---|---|---|---|---|---|---|
| Hermes | `sessions.id` | `display_name/name` | `cwd/git_repo_root` | `session_turn_leases` + `ended_at/end_reason` | 下一次 SQLite 读取同步 | `last_read_at` 存在，但部分版本不维护 |
| Codex 本地 | rollout / app-server `thread.id` | app-server `thread/list.name` | `cwd` | rollout `task_started/task_complete/...` | app-server 每 15 秒刷新标题 | 当前没有稳定、可移植的未读字段 |
| Codex 远程 | rollout UUID | 远程 `state_5.sqlite` 的 `name/title` | `cwd` | rollout 事件 | 下一次 SSH 快照同步 | 无稳定字段 |
| DeepSeek Harness | session 目录 ID | session title/name；否则首条用户消息 | `cwd` | `turn/start` 与 `turn/end` | 下一次事件流读取同步 | 无稳定字段 |
| 远程 tmux | `host + pane_id`；`tp-run` 使用 run ID | `tp-run` label 或进程命令 | pane cwd | 进程变化；`tp-run` 提供精确结束码 | 不适用 | 无 |

降级顺序统一为：手动/原生名称 → 原生正式标题 → 首条用户消息摘要 → `未命名任务 · 短 ID`。标题变化只更新同一稳定 ID 的展示数据，不改变状态、未读状态或通知去重键。

Codex 的 app-server 只负责原生标题和重命名同步。运行状态仍来自 rollout，因为 TaskPulse 启动的独立 app-server 实例不会继承 Codex Desktop 当前已加载线程的实时状态。app-server 不可用时自动降级到只读 SQLite 和 rollout，不会阻止监控。

## 生命周期

```text
新 turn → 加入关注列表 → RUNNING
        → WAITING_INPUT / FAILED / SUCCEEDED
        → 通知一次并保持未读
        → 打开或标记已查看
        → 无后续运行且未固定时移出
```

### 状态边界

- `RUNNING`：存在明确的进行中 turn、有效运行租约、未结束的工具调用，或仍存活的受监控进程。仅仅“会话窗口仍打开”不算运行中。
- `WAITING_INPUT`：对话型 Agent 的本轮已经结束，会话仍可继续，下一步需要用户输入、确认或发起新 turn。Hermes 的 turn lease 已释放、Codex 的 `task_complete`、DSH 的 `turn/end` 都属于这一类。
- `SUCCEEDED`：有权威的成功结束信号，例如 wrapper 的退出码 0，或 Agent 原生的明确完成阶段。普通对话回复完成默认是 `WAITING_INPUT`，不是 `SUCCEEDED`。
- `FAILED`：有权威失败信号，例如非零退出码、原生 failed/error/crash 状态。不能只根据文本中出现“失败”二字推断。
- `IDLE`：没有正在执行的 turn，也没有一个需要用户继续响应的开放对话；常见于空闲 shell、正常关闭的会话或只有进程级推断的结束状态。
- `DISCONNECTED`：配置的远程主机连续探测失败并越过防抖阈值。它描述连接健康，不代表任务本身失败。
- `UNKNOWN`：数据源尚未给出足够信号，或状态无法可靠映射。不得把 `UNKNOWN` 自动当作 `RUNNING`。

Provider 的原生事件时间用于展示最后活动时间；Store 的轮询时间不能覆盖它。相同 Provider 的下一次权威快照可以在原生时间戳不变时更新生命周期，例如 Hermes 在写完最后一条 assistant 消息后释放 turn lease。

- “隐藏”只清除当前 turn 的关注状态；同一稳定会话 ID 出现新的 `turn_key` 时自动重新加入。
- 在运行期间标记已查看，不会吞掉结束通知；从 `RUNNING` 进入结束状态时会再次标为未读。
- 固定记录跨重启保留。重命名不会生成新记录。
- Provider 初次启动只建立基线，不批量导入历史空闲会话。
- 范围外任务在进入状态机前被过滤，因此既不显示也不通知。

TaskPulse 自己的查看、隐藏和固定状态保存在 `~/.taskpulse/attention.json`。这是原生已查看状态缺失时的权威降级方案。

## 监控范围

编辑 `~/.taskpulse/config.toml`：

```toml
[monitoring]
local_codex_projects = ["agenttray"]
hermes_projects = ["*"]
deepseek_harness_projects = ["*"]

[[monitoring.remote_codex]]
host = "Server3"
projects = ["HyperMem"]

[[remote_hosts]]
id = "server3"
ssh_host = "Server3"
enabled = true
poll_interval_ms = 3000
tmux_sessions = ["3", "train*"]
tmux_panes = ["%12", "3:0.1"]
```

项目、服务器、session 和 pane 匹配均不区分大小写，支持 `*` 通配符。项目列表为空表示禁用该类别；`["*"]` 表示全部。旧配置未声明 `[monitoring]` 时继续监控全部项目，以保持向后兼容。

当前配置在托盘进程启动时读取；修改后重启 `bun run tauri dev` 或生产托盘程序生效。
