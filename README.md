# TaskPulse

> 多个 coding agent 的任务状态和当前活动，一眼可见；结束或需要输入时主动提醒。

同时开着几个 coding agent 时，反复切换对话查看有没有完成，会打断手头工作。TaskPulse 把本地和远程任务放在一个托盘面板：谁正在执行什么指令、谁等待输入、谁失败、哪些结果还没看。任务需要关注时弹出系统通知，用户不必一直盯着窗口。

TaskPulse 是基于 [AgentTray](https://github.com/sprklai/agenttray)（MIT）的 Windows-first 桌面伴随工具，目前为 **alpha**。当前形态是独立托盘应用，尚不是可安装到所有 IDE/Agent 的通用插件。

## 与 AgentTray 的区别

上游已经具备多会话托盘、Claude Code / Codex CLI / Gemini CLI hooks、进程扫描、终端跳转和桌面通知。TaskPulse 复用这些基础，在其上增加：

| 场景 | TaskPulse 扩展 |
|---|---|
| 多个桌面 Agent 的进程名难以对应真实任务 | Codex 原生 thread 标题、Hermes 会话与 DeepSeek Harness 元数据；项目名与任务名分开 |
| 闲置历史掩盖需要关注的任务 | 动态关注列表：运行、未查看、失败/等输入和固定任务；隐藏后新 turn 自动回来 |
| 工作同时在本机和服务器运行 | 服务器/项目分组，远程 Codex 与 tmux；可限定监控范围 |
| 不想逐个点进去才知道在做什么 | 任务行显示当前指令/活动、状态和可获取的运行时长 |
| 慢进程扫描、WSL 或 API 拖住其他 Agent | 独立采集线程，Codex/Hermes 1 秒轮询和文件变化触发，DeepSeek 2 秒采集周期 |
| 完成通知重复或错误成功 | 按任务/turn 去重；区分等输入、真实退出结果、推断完成；通知发送错误可见 |

“进度”指 Agent 的当前状态和可观测活动。未提供步骤总数或完成比例的 Agent 不显示虚构百分比。支持范围、实测数据和剩余限制见 [版本审查](docs/RELEASE_READINESS.md)。

在系统托盘即可知道：谁在跑、谁在等用户输入、远端哪个任务还在跑、哪个刚结束（以及精确退出码）、哪个服务器失联——任务结束或需要人工输入时主动弹 Windows 通知。

[架构说明](docs/ARCHITECTURE.md)

## 核心能力

| 能力 | 说明 |
|---|---|
| 本地 Agent 状态 | hook > wrapper > scan 三层可靠性，沿用 AgentTray 机制（`~/.agent-monitor/` 兼容 + `~/.taskpulse/`） |
| 远端 tmux 监控（自动） | 每 Host 每轮 1 条 SSH，并行轮询；进程树判定 workload；推断完成（无退出码时绝不标 SUCCEEDED） |
| 远端 tmux 监控（精确） | `tp-run "label" -- python eval.py`：完整保留 stdio + 真实退出码 → SUCCEEDED/FAILED / EXACT |
| 服务器健康 | SSH 失败保留状态（stale）→ 连续 3 次失败 → DISCONNECTED（带 last seen）；恢复自动回来 |
| 数据源健康 | 本地 Provider 读取失败时保留上次状态并显示降级提示，不会把读取失败误判为任务结束 |
| 通知 | 只在有意义迁移通知一次：等输入 / 成功 / 失败 / 推断完成 / 断连 / 恢复；可在监控窗口或托盘菜单测试 Windows 通知 |
| 完成历史 | 完成任务保留在 RECENT 30 分钟并跨应用重启恢复（`~/.taskpulse/recent.json`） |
| 托盘聚合 | FAILED > WAITING_INPUT > DISCONNECTED > RUNNING > SUCCEEDED/IDLE |

## 快速开始（Windows）

```powershell
# 前置：Rust（GNU/MSVC 均可）、bun、WebView2
git clone https://github.com/qingtian-4/TaskPulse.git taskpulse
cd taskpulse
bun install
bun run tauri dev
```

首次启动生成 `~/.taskpulse/config.toml`，默认不连接远程主机。按 [监控范围配置](docs/ATTENTION_LIST.md) 添加自己的 SSH 别名与项目；已有配置不改写。当前 DeepSeek 适配使用 WSL 的 Ubuntu，可用 `TASKPULSE_WSL_DISTRO` 指定发行版。

面板右上角铃铛或托盘菜单可发送测试通知。需要 Windows 允许通知；“请勿打扰”可能隐藏横幅。窗口关闭后采集和通知继续运行。

开发者可运行已构建的 `agent-tray.exe --diagnose`，用同一套采集线程采样 10 秒，输出任务标题、状态、采集次数和耗时（不包含提示词与指令正文）。`--test-notification` 会发送一次真实系统通知并返回 API 结果。

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

## Dynamic attention list

Task/session visibility, native-title sources, fallback behavior, and project/server/tmux scope configuration are documented in [docs/ATTENTION_LIST.md](docs/ATTENTION_LIST.md).

## License

MIT。TaskPulse 派生自 [AgentTray](https://github.com/sprklai/agenttray)（MIT, © sprklai）——保留原作者版权与授权。参考项目：AgentMonitor、tmux-monitor、tmux-agent、tmux-notify（均为 MIT，仅参考未 vendoring）。
