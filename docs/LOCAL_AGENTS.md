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
| needs-input | `WAITING_INPUT` | 完成当前 turn，等用户 → 通知 + 10s 提醒 beep |
| working / starting | `RUNNING` | 正在执行 |
| error | `FAILED` | wrapper 捕获的非零退出码 |
| idle | `IDLE` | 运行中、低 CPU |
| offline | `UNKNOWN` | 无信号 |

**关键区分**：Agent 完成一个 turn（WAITING_INPUT）≠ 命令完成（SUCCEEDED）。只有拿到真实退出码才标 SUCCEEDED/FAILED。

## Codex（Phase 6 已实现）

- 复用 AgentTray 的 Codex hook（`scripts/hooks/`）；`Stop` 事件 → `WAITING_INPUT`。
- **会话元数据 provider**（`providers/codex.rs`）：读 `~/.codex/sessions/**/rollout-*.jsonl`（CLI 与 Desktop 同源——Desktop 的 `state_5.sqlite.threads.rollout_path` 指向同一路径）；`task_complete` → `WAITING_INPUT`、`task_started/user_message/agent_message` → `RUNNING`、`turn_aborted` → `WAITING_INPUT`，置信度 HOOK。
- 优先级：native hook > 会话元数据(node) > 进程扫描；当存在 Codex 会话任务时抑制扫描的 Codex 条目（去重，见 watcher `dedup_codex_over_scan`）。
- 生命周期门槛：RUNNING 会话 10 分钟内未写入 → 丢弃；WAITING_INPUT 会话 24h 未更新 → 丢弃。
- adapter 边界隔离在 `providers/codex.rs`，Codex 格式变化只影响该文件；fixture 测试在文件内（completed/active/aborted/stale/unicode/recursive 6 个）。
- `notify = ["...", "turn-ended"]` 是 Codex Desktop 的配置项；接入 TaskPulse 接收者可获得 EXACT turn 结束事件，但会改动用户配置——MVP 不自动改，留作后续。当前机器无活跃 Codex 会话时 provider 正确不产任务。

## Hermes

- 首选 Hermes 官方 lifecycle hooks（实时状态）。
- 兜底：`~/.hermes/state.db`（SQLite）会话解析 + 进程状态推断（Phase 7 计划，参考 AgentMonitor 的 hermes.rs adapter，fixture 测试）。
- 转换：turn 开始 → RUNNING；turn 正常结束 → WAITING_INPUT。

## DeepSeek Harness

- 独立 adapter/plugin（`integrations/deepseek-harness/taskpulse-plugin/`），监听当前版 Plugin/Event API 的 agent/status、session/event。
- 转换：running → RUNNING；running → idle → WAITING_INPUT。
- Harness 是 Developer Preview，API 可能变化——adapter 边界隔离，类型绝不泄漏进 TaskStore。
- 无 OCR / 无截图。

## 可靠性机制（沿用 AgentTray）

- 状态文件原子写；watcher 永不见半个 JSON。
- 陈旧文件 TTL 清理（10 min）；hook 文件孤儿清理（session-id 交叉校验）。
- hook 状态与扫描数据交叉验证（heuristics）：过期 working hook → idle；扫描到高 CPU → working；假等待 → 清除。
