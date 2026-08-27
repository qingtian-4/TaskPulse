# Remote Tmux Monitoring

TaskPulse 监控 **Server1 / Server3 上 tmux pane 内正在运行的代码**（VS Code Remote-SSH → tmux → python/pytest/评测脚本），不需要用户改变任何工作习惯。

```
Windows TaskPulse
→ 系统 ssh.exe（使用用户 ~/.ssh/config 别名）
→ Server1 / Server3
→ tmux list-panes -a -F … + ps …   （每次轮询每个 Host 只有 1 条 SSH）
→ pane + 进程树 → TaskSnapshot
```

## 自动推断模式（Automatic / Inferred）

用户直接 `python eval.py`，TaskPulse 自动识别：

| 观察 | 结论 |
|---|---|
| pane 前台命令是 workload（python/pytest/uv/torchrun/node/make/…） | `RUNNING / INFERRED` |
| pane 前台是 shell，但进程树里有 workload 后代（`bash run_eval.sh → python eval.py`） | `RUNNING / INFERRED` |
| workload → shell 空闲（或 pane 消失），且在**成功轮询**中观察到 | `IDLE / INFERRED`，`finished_at = now` |
| SSH 超时/失败 | **不产生任何完成判定**；保留最后已知状态（`stale = true`） |

关键规则：

- **不用 `pane_dead` 判断完成**。实际结构是 `tmux pane → bash → python`；python 退出后 pane 还在（bash 空闲）。`pane_dead == false` 不代表 python 还在跑。
- **默认不读取 pane 内容**（`capture-pane`）。终端里可能有 token / API key / 实验输出。未来若加输出 tail，必须 opt-in + redaction。
- **推断的完成 ≠ exit code 0**。没有真实退出码，所以推断完成永远是 `IDLE`，**绝不标记 `SUCCEEDED`**。通知文案明确写 "exit code unknown"。
- 任务开始时间未知时用 `detected_at`（首次发现时间），不伪造 `started_at`。

## 精确模式（Exact / tp-run）

```bash
tp-run "LoCoMo Eval" -- python eval.py
```

`tp-run` 是极轻量的 bash 包装（安装到 `~/.local/bin/tp-run`，见 `scripts/tp-run`）：

- **完整保留 stdin/stdout/stderr**——用户感觉和直接跑 `python eval.py` 几乎一样。
- 绑定 `$TMUX_PANE`，向 `~/.local/state/taskpulse/runs/<run_id>.json` **原子写入**（tmp + rename）状态。
- 记录：`schema_version / run_id / label / hostname / pane_id / cwd / command / pid / started_at / finished_at / status / exit_code`。
- 结束后 `exit "$rc"` 原样保留退出码；`SIGINT/SIGTERM` trap 也会落盘最终状态。

结果：

| tp-run 状态 | TaskPulse 显示 |
|---|---|
| `exit 0` | `SUCCEEDED / EXACT` |
| `exit != 0` | `FAILED / EXACT`（带真实退出码） |
| 仍在运行 | `RUNNING / EXACT` |

## EXACT 与 INFERRED 合并

同一 pane（identity = `ssh:<host>:<pane_id>`，例如 `ssh:Server3:%3`）同时有推断状态和 tp-run 精确状态时，**只显示一条**，按置信度合并：

```
EXACT > HOOK > INFERRED
```

- tp-run 说 `SUCCEEDED`，进程树推断说 `IDLE` → 显示 `SUCCEEDED / EXACT`（推断的"完成"被升级为精确结果，不重复通知）。
- tp-run 说 `RUNNING`，推断短暂丢失 workload → 显示 `RUNNING / EXACT`（**绝不误报完成**）。
- 防呆：run 文件写着 `running` 但进程已死、pane 也空闲（例如 `kill -9`）→ 忽略该 run 文件，不把任务永远钉在 RUNNING。

## SSH 失败语义

```
第一次失败        → 保留最后已知任务（stale = true），Host 仍显示 connected
连续失败 ≥ 3 次   → Host DISCONNECTED（任务保留、变灰、显示 "last seen …"）
恢复连接          → 自动回到真实状态，通知一次
```

**SSH 超时绝不等于任务完成**——不会把 running 任务判成 finished。

## 轮询与并发

- 每个 Host 每轮只有 **1 条 SSH**（一个命令同时返回 tmux panes + ps + tp-run run files）。
- Host 之间**并行**（每个 Host 独立线程），Server1 卡住不影响 Server3。
- 默认 `poll_interval_ms = 3000`、SSH `ConnectTimeout = 3`，见 `~/.taskpulse/config.toml`。

## 配置

```toml
# ~/.taskpulse/config.toml（首次启动自动生成）
[[remote_hosts]]
id = "server1"
ssh_host = "Server1"   # ~/.ssh/config 中的别名
enabled = true
poll_interval_ms = 3000
tmux_sessions = ["wavelet", "8*"]   # 只监控这些 tmux 会话（精确名或 * 通配）；空=全部

[[remote_hosts]]
id = "server3"
ssh_host = "Server3"
enabled = true
poll_interval_ms = 3000
tmux_sessions = []   # 空 = 监控该服务器全部 tmux 会话
```

**`tmux_sessions`（Phase 6 加）**：只监控你指定的 tmux 会话，条目支持精确会话名或 `*` 通配（如 `train*`）；**留空 = 全部会话**（默认行为）。

⚠️ **重要**：远端监控只看 **tmux pane**。如果真实任务（vllm/llama-server 等 GPU 推理）运行在 **VS Code Remote-SSH 的 pty 终端**里（父链 `llama-server ← bash(vscode shellIntegration) ← node(ptyHost) ← code-server`），它们**不在 tmux 里**，不会被抓到。要监控这类任务需额外的「终端附件工作负载」检测（可后续加）。

TaskPulse **不保存任何凭据**：走系统 `ssh.exe` + `BatchMode=yes`，认证完全依赖用户自己的 `~/.ssh/config` / key / ssh-agent。

## 验收演示（Server3）

```bash
tmux new -s demo
python -c "import time; time.sleep(30)"
```

TaskPulse 自动出现 `Server3 · demo:0.0 · python … · RUNNING`；30 秒后变 `Finished (inferred)`，Windows 通知一次。然后：

```bash
tp-run "success-demo" -- bash -c "sleep 10; exit 0"   # → SUCCEEDED exit 0
tp-run "failure-demo" -- bash -c "sleep 10; exit 2"   # → FAILED exit 2
```

拔掉 Server3 网络 → `DISCONNECTED`（任务不消失、不判完成）；恢复 → 自动回来。
