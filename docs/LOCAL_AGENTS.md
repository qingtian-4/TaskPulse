# Local Agents

本地 AI Coding Agent（Codex / Hermes / DeepSeek Harness / Claude Code / Gemini）通过 **hook > wrapper > scan** 三层可靠性报告状态，全部基于本地文件与本地进程，无任何网络上传。

## 数据源

| 层级 | 机制 | 目录 |
|---|---|---|
| hook | CLI 官方 hook 事件写入状态文件（Codex Stop → needs-input 等） | `~/.agent-monitor/`（AgentTray 兼容） |
| wrap | `scripts/wrap.sh` / `wrap.ps1` 包装任意命令，实时写状态 | `~/.agent-monitor/` |
| scan | 进程扫描兜底（CPU 平滑、子进程计数、终端 focus 识别） | — |

新推荐目录：`~/.taskpulse/`（v2 格式，可带 `started_at/finished_at/exit_code/confidence`）。TaskPulse 两个目录都读，旧 AgentTray 文件继续有效。

## 状态映射

| AgentTray 旧状态 | TaskPulse 新状态 | 语义 |
|---|---|---|
| needs-input | `WAITING_INPUT` | 完成当前 turn，等用户 → 每个 turn 通知一次 |
| working / starting | `RUNNING` | 正在执行 |
| error | `FAILED` | wrapper 捕获的非零退出码 |
| idle | `IDLE` | 运行中、低 CPU |
| offline | `UNKNOWN` | 无信号 |

**关键区分**：Agent 完成一个 turn（WAITING_INPUT）≠ 命令完成（SUCCEEDED）。只有拿到真实退出码才标 SUCCEEDED/FAILED。

## Codex（Phase 6 已实现）

- 复用 AgentTray 的 Codex hook（`scripts/hooks/`）；`Stop` 事件 → `WAITING_INPUT`。
- **会话元数据 provider**（`providers/codex.rs`）：读 `~/.codex/sessions/**/rollout-*.jsonl`（CLI 与 Desktop 同源——Desktop 的 `state_5.sqlite.threads.rollout_path` 指向同一路径）；`task_complete` → `WAITING_INPUT`、`task_started/user_message/agent_message` → `RUNNING`、`turn_aborted` → `WAITING_INPUT`，置信度 HOOK。
- 优先级：native hook > 会话元数据(node) > 进程扫描；当存在 Codex 会话任务时抑制扫描的 Codex 条目（去重，见 watcher `dedup_codex_over_scan`）。
- 发现窗口为近期 7 天的元数据及已知/固定任务；RUNNING 10 分钟无事件标记待更新，不能推断结束。WAITING_INPUT 的 24h 是首次发现门槛，已关注未读结果由关注记录保留，不因刷新到期而确认已查看。
- adapter 边界隔离在 `providers/codex.rs`，Windows、WSL、SSH 共用 rollout 增量解析；测试覆盖 completed/active/aborted/stale/unicode/recursive、当前工具调用、思考和标题同步。
- `notify = ["...", "turn-ended"]` 是 Codex Desktop 的配置项；接入 TaskPulse 接收者会改动用户配置，当前不自动安装。
- **本机 WSL**：独立 `codex_wsl` 来源，默认用户 Linux `CODEX_HOME` / `~/.codex`。在 Linux 内只读 SQLite + `session_index.jsonl` 获取原生标题、项目和 ID，UNC 只用于读取 rollout 日志，不能跨 Windows/WSL 打开 SQLite WAL。停止的发行版不自动唤醒；当前真实 Ubuntu 会话已验证。配置、降级和证据边界见 [WSL_CODEX.md](WSL_CODEX.md)。

## Hermes（Phase 7 已实现）

- 首选 Hermes 官方 lifecycle hooks（实时状态）。
- **会话元数据 provider**（`providers/hermes.rs`）：读 Hermes 的 SQLite 状态库（`$HERMES_STATE_DB` → `~/.hermes/state.db` → `$LOCALAPPDATA/hermes/state.db`），用 `sessions` 表 + **`session_turn_leases` 表**判断 turn 状态：
  - 有未过期的 turn 租约（`expires_at > now`）→ **RUNNING**（agent 正在处理）
  - 无活跃租约、最近有活动 → **WAITING_INPUT**（完成、等用户）
  - 置信度 = HOOK（来自 Hermes 自身的状态与租约，比进程扫描可靠）
- 优先级：lifecycle hook > 会话元数据 > 进程扫描；存在 Hermes 会话任务时抑制扫描的 Hermes 条目（`dedup_hermes_over_scan`）。
- 生命周期门槛：RUNNING 会话 10 分钟未活动丢弃；WAITING_INPUT 会话 24h 未活动丢弃。
- adapter 隔离在 `providers/hermes.rs`；fixture 测试（active lease→RUNNING / expired→WAITING_INPUT / archived skipped / stale dropped / missing db empty）共 5 个，用临时 SQLite 库。
- 已在本机真实 Hermes 库端到端验证：当前会话（TaskPulse Windows MVP）显示 Running，另两个会话显示 WaitingInput。

## DeepSeek Harness（Phase 8 已实现）

- **会话元数据 provider**（`providers/deepseek_harness.rs`）：dsh（`@deepseek-ai/dsh`）装在 **WSL2**（`$DSH_HOME` 默认 `~/.dsh`），会话是 `~/.dsh/sessions/<workspace>/<session-id>/session.jsonl.zstd`（zstd 压缩的追加式事件流）。
- **状态推断**（读 harness 自己的事件流，置信度 = HOOK）：最新 `turn/end` seq > 最新 `turn/start` seq → **WAITING_INPUT**；有 `turn/start` 无后续 `turn/end` → **RUNNING**；无 turn 事件 → **IDLE**。
- **访问**：该 WSL provider 经 `wsl.exe` 子进程 + WSL 端 `base64` + Rust 侧 `zstd` 解压，并缓存未变化日志。旧阶段的 UNC 不可用观察不是所有 WSL 环境的限制；当前 Codex WSL 已验证可通过 UNC 读取日志，但 SQLite/WAL 必须在 Linux 内打开。
- 项目名取自会话 `cwd` basename；distro 可经 `TASKPULSE_WSL_DISTRO` 覆盖（默认 Ubuntu）。
- adapter 隔离在 `providers/deepseek_harness.rs`；fixture 测试走 zstd 编解码验证推断逻辑。已在本机真实 WSL2 dsh 库端到端验证（Game 工作区会话 → WaitingInput）。

## 可靠性机制（沿用 AgentTray）

- 状态文件原子写；watcher 永不见半个 JSON。
- 陈旧文件 TTL 清理（10 min）；hook 文件孤儿清理（session-id 交叉校验）。
- hook 状态与扫描数据交叉验证（heuristics）：过期 working hook → idle；扫描到高 CPU → working；假等待 → 清除。
