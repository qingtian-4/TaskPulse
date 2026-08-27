# TaskPulse

> A Windows system-tray monitor for local AI coding agents and remote tmux tasks.

TaskPulse 是 [AgentTray](https://github.com/sprklai/agenttray)（MIT）的 Windows-first 分支扩展：本地 AI Agent（Codex / Hermes / DeepSeek Harness / Claude Code / Gemini）+ 远端 tmux 任务（VS Code Remote-SSH → Server1/Server3 → tmux pane 里的 python / pytest / 评测脚本），统一监控。

在系统托盘即可知道：谁在跑、谁在等用户输入、远端哪个任务还在跑、哪个刚结束（以及精确退出码）、哪个服务器失联——任务结束或需要人工输入时主动弹 Windows 通知。

![architecture](docs/ARCHITECTURE.md)

## 核心能力

| 能力 | 说明 |
|---|---|
| 本地 Agent 状态 | hook > wrapper > scan 三层可靠性，沿用 AgentTray 机制（`~/.agent-monitor/` 兼容 + `~/.taskpulse/`） |
| 远端 tmux 监控（自动） | 每 Host 每轮 1 条 SSH，并行轮询；进程树判定 workload；推断完成（无退出码时绝不标 SUCCEEDED） |
| 远端 tmux 监控（精确） | `tp-run "label" -- python eval.py`：完整保留 stdio + 真实退出码 → SUCCEEDED/FAILED / EXACT |
| 服务器健康 | SSH 失败保留状态（stale）→ 连续 3 次失败 → DISCONNECTED（带 last seen）；恢复自动回来 |
| 通知 | 只在有意义迁移通知一次：等输入 / 成功 / 失败 / 推断完成 / 断连 / 恢复；等输入 10s 提醒 |
| 完成历史 | 完成任务保留在 RECENT 30 分钟（`✓ LoCoMo Eval · 7m ago · exit 0`） |
| 托盘聚合 | FAILED > WAITING_INPUT > DISCONNECTED > RUNNING > SUCCEEDED/IDLE |

## 快速开始（Windows）

```powershell
# 前置：Rust（GNU/MSVC 均可）、bun、WebView2
git clone https://github.com/sprklai/agenttray.git taskpulse
cd taskpulse
bun install
bun run tauri dev
```

首次启动自动生成 `~/.taskpulse/config.toml`（Server1/Server3 示例，改 `ssh_host` 为你的 `~/.ssh/config` 别名即可）。

远端安装精确模式包装：

```bash
# 在 Server1/Server3 上
install -m 755 scripts/tp-run ~/.local/bin/tp-run
```

## 文档

- [架构审计（复用/修改/替换/不复制 + License）](docs/ARCHITECTURE_AUDIT.md)
- [架构总览（Provider → Store → Transition → Tray）](docs/ARCHITECTURE.md)
- [远端 tmux：自动推断 vs tp-run 精确模式](docs/REMOTE_TMUX.md)
- [本地 Agent：hook/wrap/scan 与状态映射](docs/LOCAL_AGENTS.md)
- [开发：工具链、测试、已知限制](docs/DEVELOPMENT.md)

## 语义红线

- Agent turn 完成 = `WAITING_INPUT`，不是 `SUCCEEDED`。
- 没有真实退出码就没有 SUCCEEDED/FAILED。
- SSH 超时 ≠ 任务完成。
- 不存凭据（走系统 ssh + ~/.ssh/config）；不 OCR；不读 pane 内容；无云端。

## 技术栈

Tauri 2 · Rust · Svelte 5 / SvelteKit · TypeScript · Tailwind 4

## License

MIT。TaskPulse 派生自 [AgentTray](https://github.com/sprklai/agenttray)（MIT, © sprklai）——保留原作者版权与授权。参考项目：AgentMonitor、tmux-monitor、tmux-agent、tmux-notify（均为 MIT，仅参考未 vendoring）。
