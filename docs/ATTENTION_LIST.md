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

TaskPulse 自己的查看、隐藏、固定和长期忽略状态保存在 `~/.taskpulse/attention.json`。通知去重保存在 `~/.taskpulse/notifications.json`。这是原生已查看状态缺失时的权威降级方案。聚焦 Agent 窗口不自动标记已查看。

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

以上为旧版配置格式。新版本通过齿轮打开“监控来源”管理窗口；首次保存将旧范围迁移到同文件的 `[dashboard]`，备份为 `config.toml.pre-dashboard.bak`，原有其他字段保留。保存立即生效，停用来源不触发完成通知。之后以 `[dashboard]` 为准；手动改文件仍需重启，不要同时编辑旧范围和新范围。完整规则见 [PRODUCT_DASHBOARD.md](PRODUCT_DASHBOARD.md)。

SSH 别名必须与实际运行 Agent 的账号一致。同一机器的两个别名可以使用不同用户，例如 `Server1` 与 `Server1_wanghy`；只连接到机器并不能读取另一个用户的 `~/.codex`。`remote_hosts.ssh_host` 和 `monitoring.remote_codex.host` 应指向相同别名，同时启用主机并将项目加入范围。远程 Codex 不要求运行在 tmux 中，IDE Remote SSH 的原生 rollout 也可以采集。

## Codex 剩余额度

额度是账号级数据，不是会话进度。面板底部独立显示“本地 Codex 账号剩余额度”，来自本机登录账号的 `account/rateLimits/read`，约每 10 秒发起一次只读查询。刷新按钮会唤醒额度采集线程，不再只刷新会话列表。界面显示成功读取时间；查询失败或超过 60 秒未更新时标记“待更新”，不把旧日志百分比伪装成实时额度。读取中显示“刷新中”。

额度查询不依赖任务是否运行或是否被隐藏，标题列表查询失败也不会丢弃成功的额度响应。优先读取 `codex` 额度桶，校验 5 小时/7 天窗口；不把其他模型的额度套用到这两个标签。

远程 SSH 用户可能登录不同 Codex 账号，因此不再将本地账号额度或某个旧远程会话的 token_count 复制到所有任务。当前版本未单独展示各远程账号额度；底部明确标注本地来源。账号切换、网络故障和 Codex 服务端缓存仍会影响读数，轮询不等于服务端实时推送。

只读诊断（不会标记任务已查看）：

```powershell
.\src-tauri\target\debug\agent-tray.exe --diagnose --diagnose-seconds=25 --refresh-quota
powershell -ExecutionPolicy Bypass -File scripts/probe-codex.ps1
powershell -ExecutionPolicy Bypass -File scripts/probe-codex.ps1 -RemoteHost YourSshAlias
```

诊断结果包含 `quota_observations` 的成功读取时间，可检查自动轮询与第 5 秒手动唤醒是否真正获得新响应。原生元数据兼容测试：`python3 scripts/test-codex-metadata.py`（也可通过 WSL 执行）。

## DeepSeek Harness 桌面版

[官方桌面文档](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md)说明 Desktop 与 CLI 共享 `$DSH_HOME` 下的会话与工作区数据。TaskPulse 直接只读采集这些数据，不连接桌面版的私有认证接口，不读取 `.credentials.yaml`，不修改 DeepSeek 的配置或安装文件。

- 默认本机 `~/.dsh`；`TASKPULSE_DSH_HOME` 优先于 `DSH_HOME`。更改路径后重启托盘。
- 同时识别旧 `session.jsonl.zstd` 和新 `session.v4.jsonl.zstd`；同一会话迁移后只选 v4，避免重复记录。
- 读取逐会话 `storages/session_projcache/sessions/<id>.json` 的原生侧栏标题、cwd、turnBoundary/sessionStats；保留旧聚合缓存兼容。重命名不需要日志同时变化。
- 标题变化不改 session ID/turn key。新 turn 开始进入关注列表；`turn/end` 后等待输入，明确错误为失败，goal 成功信号为完成。事件序号较旧的缓存不能覆盖较新的日志；步骤之间仍有 open turn 时保持运行，不误报结束。
- 桌面采集是独立 1 秒轮询，并监听本机 sessions/storages 文件变化；WSL 继续使用独立 2 秒 worker，耗时/失败不拖住桌面版。两者按不同存储实例使用稳定 ID，历史空闲会话仍不回灌。
- 当前工具命令写入 activity，结束后清除运行指令；缓存半写入/压缩日志暂不可读时显示降级，保留已有快照，不假装任务已结束。

已完成本机 v4/投影 v7 数据验证；其他桌面分支或未来 schema 不保证自动兼容。首次安装后尚无会话会显示空列表。当前轮询和本机日志写入存在延迟，1 秒是轮询周期，不是端到端 SLA。
