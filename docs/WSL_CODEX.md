# 本机 WSL Codex

Windows 的 `~/.codex`、WSL 的 Linux `~/.codex`、SSH 服务器的 Codex 是不同数据来源。仅启用“Codex · 本机”不会读取 VS Code Remote–WSL 中的会话。WSL 会话属于**本机**，但有独立的来源实例和项目范围。

## 启用

1. 在 VS Code / 终端启动使用 Codex 的 WSL 发行版。
2. 打开 TaskPulse 齿轮 → 监控来源。点击“发现 Codex · WSL Ubuntu”的“添加到监控来源”，再“保存并生效”。其他发行版同理，也可手动添加“Codex · WSL”并填写发行版名。
3. 新来源默认关注所有项目的新执行；可以独立限制 Linux 项目名称或完整路径。Windows 和 SSH 的原有范围不改变。
4. 新 turn 自动加入，已有运行 turn 也会加入；启用前已完成的历史不批量导入、不补弹通知。必要时在“已发现会话”按 WSL 来源查找并“加入关注”。主面板若存在筛选，可先“清除筛选”。

需要 Windows WSL 与发行版内的 Python 3。使用该发行版**默认 Linux 用户**的目录：WSL 命令环境中的 `CODEX_HOME`，否则 `~/.codex`。目前不枚举其他 Linux 用户，也不执行登录 shell / 读取用户 shell 配置；只在 shell 启动脚本中设置的自定义 `CODEX_HOME` 不保证被继承。此情况需要后续显式用户/目录配置，不能伪装为已覆盖。

TaskPulse 只检查已运行的发行版，不自动启动停止的 WSL。Python 缺失、目录不存在、SQLite 忙或 WSL 断开会显示来源异常，保留原任务状态并标记待更新，不通知“任务完成”。

## 数据与边界

| 信息 | 来源 / 策略 |
|---|---|
| 稳定身份 | 原生 thread ID；TaskPulse ID 为 `<来源实例>:codex:<thread-id>`，与 Windows/SSH 不混合 |
| 标题 | SQLite 手动 `name` → `session_index.jsonl` 原生 `thread_name` → DB 正式标题/首条消息摘要 → 未命名 + 短 ID |
| 项目 | SQLite `cwd`，保留完整 Linux 路径；项目名独立于任务标题 |
| 生命周期 / 当前指令 / 思考 | 原生 rollout JSONL，复用 Windows/SSH 的增量解析与阶段状态机；明确结束事件才通知 |
| 重命名 | 每轮刷新元数据；只更新名称，稳定身份和 turn 不变 |
| 已查看 | TaskPulse 显式“已查看”；没有可靠原生阅读状态，不冒充已读取 |
| 额度 | 不用 WSL 历史日志冒充实时额度，也不把 Windows 账号额度赋给 WSL 任务 |

**SQLite 必须在 Linux 内 `mode=ro` 打开**。真机验证表明从 Windows 通过 UNC 读取正在使用的 WSL SQLite/WAL 会报锁定。元数据通过有超时、无可见终端的 `wsl.exe --exec python3` 读取，不修改 Agent 配置、Hook、auth 或会话。rollout 通过 WSL UNC 共享增量读取，含空格的项目目录正常保留。

每个 WSL 来源独立 worker，目标轮询间隔约 2 秒，不阻塞其他来源。间隔不是端到端时延保证；WSL 停止与启动竞态、UNC 延迟、源日志落盘时间仍影响实际延迟。

## 验证

```powershell
# 普通单元测试不要求 WSL；同一解析器验证启动、重命名、增量完成和身份隔离。
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray wsl
# 真实只读采集，默认 Ubuntu；可设置 TASKPULSE_WSL_DISTRO。
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_wsl_codex_snapshot -- --ignored --nocapture
```

托盘运行时使用 `CARGO_TARGET_DIR=src-tauri/target/codex-check`，或先执行 `scripts/verify.ps1`。原生管理页验收按 [开发说明](DEVELOPMENT.md) 临时开启 CDP 9337，再运行：

```powershell
bun scripts/native-wsl-codex-smoke.ts
```

脚本验证发现/添加/保存、真实标题和项目、来源健康、隔离身份和不改变其他范围。不发送 Agent 提示词，不更改原生配置，不将历史任务标记已查看。默认恢复 TaskPulse 来源设置；`TASKPULSE_KEEP_WSL_CODEX=1` 仅在全部验收成功时保留新来源启用。可以用 `TASKPULSE_WSL_PROJECT` / `TASKPULSE_WSL_TITLE` 指定验收样本，默认仅检查发现真实会话，不依赖某个用户项目。验收后关闭临时调试进程，正常启动托盘。
