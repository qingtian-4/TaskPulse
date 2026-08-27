# TaskPulse Architecture

```
                    ┌────────────────────────────────────────────────┐
                    │            Provider 层（只观察，不渲染）          │
                    │                                                │
  ~/.agent-monitor  │  LocalFileProvider    Scanner(进程扫描)          │
  ~/.taskpulse      │  (hook/wrap 状态文件)  (Codex/Claude/Gemini)    │
                    │                                                │
  ssh Server1/3     │  RemoteTmuxProvider（并行轮询，每 Host 1 条 SSH） │
                    └──────────────────────┬─────────────────────────┘
                                           │ TaskSnapshot（统一模型）
                                           ▼
                    ┌────────────────────────────────────────────────┐
                    │  TaskStore（按 id 合并；EXACT>HOOK>INFERRED；    │
                    │   RECENT 完成历史 30 分钟 TTL）                  │
                    └──────────────────────┬─────────────────────────┘
                                           ▼
                    ┌────────────────────────────────────────────────┐
                    │  TransitionEngine（只在有意义的状态迁移上发事件；   │
                    │   每个 (task, transition) 只通知一次）            │
                    └──────────────────────┬─────────────────────────┘
                                           ▼
              Tray 图标（聚合优先级）    Windows 通知 + beep     Popup UI
              FAILED > WAITING_INPUT >  （等待输入 10s 提醒）   (Svelte 5)
              DISCONNECTED > RUNNING >
              SUCCEEDED/IDLE > UNKNOWN
```

## 模块布局（src-tauri/src/）

```
domain/            TaskSnapshot / TaskStatus / TaskSource / DetectionConfidence
providers/
  local_file.rs    AgentTray 兼容状态文件（.agent-monitor v1 + .taskpulse v2）
  remote_tmux/
    config.rs      ~/.taskpulse/config.toml（首次运行自动生成）
    ssh.rs         SshExecutor trait + SystemSshExecutor + FakeSshExecutor(测试)
    parser.rs      tmux list-panes -a -F / ps 输出解析
    detector.rs    shell vs workload（进程树判定，深度 ≤6）
    runfile.rs     tp-run run 文件解析 + EXACT 合并 + 存活校验
    state.rs       每 Host 失败计数 / DISCONNECTED 阈值(3) / running_panes
store/             TaskStore：按 id 合并、置信度覆盖、RECENT TTL
transition/        TransitionEngine：状态迁移 → TaskEvent（通知去重）
notifier.rs        beep + Windows 通知 + 等待输入 10s 提醒
tray.rs            聚合优先级 → 图标；popup 窗口生命周期
watcher.rs         编排循环：文件监听 + 2s 进程扫描 + 3s 远端轮询线程
```

## 语义规则（硬约束）

1. `WAITING_INPUT` = agent 完成当前 turn、等待用户。**不是** SUCCEEDED。
2. `SUCCEEDED`/`FAILED` 只能来自真实命令退出码（tp-run）。
3. 推断完成永远是 `IDLE / INFERRED`，绝不带 exit code。
4. SSH 失败 ≠ 任务完成；DISCONNECTED 需要连续失败 ≥3 次。
5. 每个 Host 每轮 1 条 SSH；Host 之间并行。
6. 不存凭据；不 OCR；不截屏；不读 pane 内容（MVP）；无云端依赖。
7. 状态文件一律原子写（tmp + rename）。

## 与 AgentTray 的关系

- 复用：tray/popup 生命周期、通知机制、状态文件格式、hook 脚本、进程扫描、focusers。
- 升级：字符串状态 → 枚举任务模型；单 watcher.rs → Provider/Store/Transition 管道。
- 新增：RemoteTmuxProvider（核心能力）、RECENT 完成历史、tp-run 精确模式。
- 完整清单见 `docs/ARCHITECTURE_AUDIT.md`（reuse / modify / replace / not-copy）。
