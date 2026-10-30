# Remote Tmux Monitoring

TaskPulse 监控 **已配置 SSH 服务器上 tmux pane 内正在运行的代码**（VS Code Remote-SSH → tmux → python/pytest/评测脚本），不需要用户改变任何工作习惯。

```
Windows TaskPulse
→ 系统 ssh.exe（使用用户 ~/.ssh/config 别名）
→ research-box / build-box
→ tmux list-panes -a -F … + ps …   （每次轮询每个 Host 只有 1 条 SSH）
→ pane + 进程树 → TaskSnapshot
```

## 自动推断模式（Automatic / Inferred）

用户直接 `python eval.py`，TaskPulse 自动识别：

| 观察 | 结论 |
|---|---|
| pane 前台命令是 workload（python/pytest/uv/torchrun/node/make/…） | `RUNNING / INFERRED` |
| pane 前台是 shell，但进程树里有 workload 后代（`bash run_eval.sh → python eval.py`） | `RUNNING / INFERRED` |
| workload → 同一 pane root PID 的 shell 空闲，且 tmux/进程树分段都成功 | `IDLE / INFERRED`，`finished_at = now`，退出码未知 |
| pane 消失或 root PID 改变 | UNKNOWN／过期，不发送完成通知 |
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

`tp-run` 是 Python 3 前台命令监督器（安装到 `~/.local/bin/tp-run`，见 `scripts/tp-run`）：

- **完整保留 stdin/stdout/stderr**——用户感觉和直接跑 `python eval.py` 几乎一样。
- 绑定 `$TMUX_PANE`，向 `~/.local/state/taskpulse/runs/<run_id>.json` **原子写入**（tmp + rename）状态。
- v2 记录 run ID、原始参数数组的已知敏感参数脱敏副本、PID/start ticks、boot ID、秒时间和显式 `_ms` 时间、状态与退出码。执行时使用原始 argv，不重新拼接 shell 命令。
- 默认每 5 秒原子写心跳，但不更新执行活动时间；终态由同一监督循环写入，不会被晚到心跳覆盖。
- 保留退出码；SIGINT/SIGTERM 转发给直接子进程并记录 interrupted。SIGKILL／机器重启无法写终态，采集端标记过期，不写成功。
- 仅保证直接前台子进程的边界；后台化、容器启动或 Slurm 提交退出不代表实际作业完成。不会安装到服务器或修改 Agent 配置，需用户自行更新旧脚本。

结果：

| tp-run 状态 | TaskPulse 显示 |
|---|---|
| `exit 0` | `SUCCEEDED / EXACT` |
| `exit != 0` | `FAILED / EXACT`（带真实退出码） |
| 仍在运行 | `RUNNING / EXACT` |

## EXACT 与 INFERRED 合并

精确运行 identity = `ssh:<host>:run:<run_id>`，pane 仅用于关联和跳转。运行 A 的结果不能覆盖同一 pane 的运行 B；同时运行按不同 run ID 分开显示。只有确认推断 workload 是新鲜精确 wrapper 的后代时才抑制这条重复推断。

```
EXACT > HOOK > INFERRED
```

- 精确 running 要求 wrapper PID、启动 ticks、boot ID 匹配且心跳不超过 30 秒。无法核对则保留为过期，既不宣布完成也不删除。
- 旧 v1 running 记录没有身份／心跳证据时同样保留为不确定；终态保留真实退出码。
- 活跃／未知运行文件没有发现时间窗口；终态文件的发现窗口仍为 3 小时，已采到的未读结果由关注列表独立持久化。
- 旧 pane 级精确记录迁移时备份 attention/recent 为 `*.pre-run-identity.bak`，保留未读／固定；通知迁移仅匹配同一个 run/turn，不转移到下一次运行。

## SSH 失败语义

```
第一次失败        → 保留最后已知任务（stale = true），Host 仍显示 connected
连续失败 ≥ 3 次   → Host DISCONNECTED（任务保留、变灰、显示 "last seen …"）
恢复连接          → 自动回到真实状态，通知一次
```

**SSH 超时绝不等于任务完成**——不会把 running 任务判成 finished。

一次 SSH 返回版本化 JSON，tmux、进程树、run files、Codex 各有独立成功／错误和观察时间。SSH 连通不代表所有分段健康；失败分段保留过期任务，成功来源继续更新。远端需要 Python 3；Codex rollout 仍有 24 小时发现和 64 KiB 尾部窗口，窗口外的任务不能宣称结束。

## 轮询与并发

- 每个 Host 每轮只有 **1 条 SSH**（一个命令同时返回 tmux panes + ps + tp-run run files）。
- Host 之间**并行**执行，但本轮批量交付仍受最慢 Host 的 SSH 超时限制；尚非逐主机流式交付。失败主机不会阻止本轮成功来源更新。
- 默认 `poll_interval_ms = 3000`、SSH `ConnectTimeout = 3`，见 `~/.taskpulse/config.toml`。

## 配置

```toml
# ~/.taskpulse/config.toml（首次启动自动生成）
[[remote_hosts]]
id = "server1"
ssh_host = "research-box"   # ~/.ssh/config 中的别名
enabled = true
poll_interval_ms = 3000
tmux_sessions = ["wavelet", "8*"]   # 只监控这些 tmux 会话（精确名或 * 通配）；空=全部

[[remote_hosts]]
id = "server3"
ssh_host = "build-box"
enabled = true
poll_interval_ms = 3000
tmux_sessions = []   # 空 = 监控该服务器全部 tmux 会话
```

**`tmux_sessions`（Phase 6 加）**：只监控你指定的 tmux 会话，条目支持精确会话名或 `*` 通配（如 `train*`）；**留空 = 全部会话**（默认行为）。

⚠️ **重要**：远端监控只看 **tmux pane**。如果真实任务（vllm/llama-server 等 GPU 推理）运行在 **VS Code Remote-SSH 的 pty 终端**里（父链 `llama-server ← bash(vscode shellIntegration) ← node(ptyHost) ← code-server`），它们**不在 tmux 里**，不会被抓到。要监控这类任务需额外的「终端附件工作负载」检测（可后续加）。

TaskPulse **不保存任何凭据**：走系统 `ssh.exe` + `BatchMode=yes`，认证完全依赖用户自己的 `~/.ssh/config` / key / ssh-agent。

## 验收演示（build-box）

```bash
tmux new -s demo
python -c "import time; time.sleep(30)"
```

TaskPulse 自动出现 `build-box · demo:0.0 · python … · RUNNING`；30 秒后变 `Finished (inferred)`，Windows 通知一次。然后：

```bash
tp-run "success-demo" -- bash -c "sleep 10; exit 0"   # → SUCCEEDED exit 0
tp-run "failure-demo" -- bash -c "sleep 10; exit 2"   # → FAILED exit 2
```

拔掉 build-box 网络 → `DISCONNECTED`（任务不消失、不判完成）；恢复 → 自动回来。
