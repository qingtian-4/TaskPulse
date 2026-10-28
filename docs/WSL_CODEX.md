# 本机 WSL Codex

Windows 的 `~/.codex`、WSL 的 Linux `~/.codex`、SSH 服务器的 Codex 是不同数据来源。仅启用“Codex · 本机”不会读取 VS Code Remote–WSL 中的会话。WSL 会话属于**本机**，但有独立的来源实例和项目范围。

## 启用

1. 在 VS Code / 终端启动使用 Codex 的 WSL 发行版。
2. 打开 TaskPulse 齿轮 → 监控来源。点击“发现 Codex · WSL Ubuntu”的“添加到监控来源”，再“保存并生效”。其他发行版同理，也可手动添加“Codex · WSL”并填写发行版名。
3. 新来源默认关注所有项目的新执行；可以独立限制 Linux 项目名称或完整路径。Windows 和 SSH 的原有范围不改变。
4. 新 turn 自动加入，已有运行 turn 也会加入；启用前已完成的历史不批量导入、不补弹通知。必要时在“已发现会话”按 WSL 来源查找并“加入关注”。主面板若存在筛选，可先“清除筛选”。

需要 Windows WSL 与发行版内的 Python 3。使用该发行版**默认 Linux 用户**的目录：WSL 命令环境中的 `CODEX_HOME`，否则 `~/.codex`。目前不枚举其他 Linux 用户，也不执行登录 shell / 读取用户 shell 配置；只在 shell 启动脚本中设置的自定义 `CODEX_HOME` 不保证被继承。此情况需要后续显式用户/目录配置，不能伪装为已覆盖。

TaskPulse 只检查已运行的发行版，不自动启动停止的 WSL。Python 缺失、目录不存在、SQLite 忙或 WSL 断开会显示来源异常，保留原任务状态并标记待更新，不通知“任务完成”。

自动发现不会把“WSL 正在运行”当作“安装了 Codex”：默认跳过 Docker Desktop / Rancher Desktop 的内部发行版；对其他运行中发行版，在 Linux 内只读验证 Codex thread 表结构、原生会话索引或 rollout 会话元数据后，才显示“发现 Codex”提示。空目录、普通 WSL 环境、验证超时或缺少 Python 不会被推荐。来源可用也不代表有任务正在执行。发现操作不会保存配置、启用来源、导入历史或改动已有项目范围；已手动添加的来源不会被删除。未被自动发现时，仍可手动配置后查看采集状态。

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

## 运行中、采集待更新与恢复

- 正常采集确认的执行显示在“进行中”。思考、等待模型和工具调用都不是任务结束。
- WSL 短暂失联、读取失败或日志追赶未完成时，保留稳定会话/turn 身份、未读、固定和通知去重；旧运行证据显示为“上次确认为运行中，当前采集待更新”。概览另列“上次运行 N”，可直接定位待确认任务，不把它计为新鲜运行，也不通知完成。
- 恢复后必须用新鲜、完整的对应会话日志恢复状态。相同 turn 不重发结束提醒。明确结束进入结果待查看；丢失日志、坏行、静默和工具返回不能证明结束。
- 日志游标使用 Linux 内的 dev/inode、长度、mtime_ns/ctime_ns，并校验文件头和游标内容锚点。UNC 创建/修改时间漂移不再重置 WSL 游标；真实截断、替换、同长度改写仍会重建解析状态。元数据与 UNC 长度不一致只进行一次有界重试，仍不一致则显示退化，不使用中间终态。
- SQLite 忙、可恢复的 Linux I/O 错误最多重试一次。错误带有阶段码，例如 `[wsl:sqlite:sqlite_busy]`、`[wsl:rollout_read:catch_up]`、`[wsl:rollout_decode:gap]`。追赶未完成也是非权威观测，不等于 Python 或 WSL 已崩溃。

高级资源诊断 `get_resource_diagnostics` 中的 `collection_continuity` 是最多 512 条的**进程内**采集流水线记录；`parser_instances` 包含游标、重置原因、正文/校验读取字节、完整性和安全的会话/turn hash。记录 Provider、Worker、来源范围筛选、Store 和 Attention 的数量与状态，不保存任务名称、prompt、思考文本、命令参数或回复。每条最多记录 128 个任务/游标，超出会显式标记截断；它不是生命周期或通知账本，重启不会保留诊断环，也不会重置真实账本。

需要核对是否为前端搜索/分组筛选时，可检查 `.dashboard` 的只读 `data-continuity` JSON：它报告按来源划分的全局与当前可见分类计数，不含搜索文本或任务内容，不保存前端操作历史。

只读复现现有日志（无需发起 Agent 任务，不启动停止的发行版）：

```powershell
$env:CARGO_TARGET_DIR = Join-Path $PWD 'src-tauri\target\codex-check'
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_wsl_continuity_diagnostics -- --ignored --nocapture
# 输出脱敏摘要，完整证据位于该 target 的 ui-smoke/evidence/wsl-continuity-*.json。
# 纯隔离元数据测试，在已有 Python 3 环境执行，不接触真实 Codex 目录：
python3 scripts/wsl-metadata.test.py
```

连续性测试按[开发检查](DEVELOPMENT.md)与对应 CI 证据验证。修复不会热替换运行中的旧托盘；正常退出并启动新构建后才会生效。

## 验证

```powershell
# 普通单元测试不要求 WSL；同一解析器验证启动、重命名、增量完成和身份隔离。
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray wsl
# 离线验证 Python 探测器，只使用临时目录（需要 Python 3，不需要 WSL）。
python scripts/test-wsl-discovery.py
# 真实来源发现验证：保留 Ubuntu Codex，排除 Docker 等内部环境（不启动发行版）。
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_wsl_codex_discovery -- --ignored --nocapture
# 真实只读采集，默认 Ubuntu；可设置 TASKPULSE_WSL_DISTRO。
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_wsl_codex_snapshot -- --ignored --nocapture
```

托盘运行时使用 `CARGO_TARGET_DIR=src-tauri/target/codex-check`，或先执行 `scripts/verify.ps1`。原生管理页验收按 [开发说明](DEVELOPMENT.md) 临时开启 CDP 9337，再运行：

```powershell
bun scripts/native-wsl-codex-smoke.ts
```

脚本验证发现/添加/保存、真实标题和项目、来源健康、隔离身份和不改变其他范围。不发送 Agent 提示词，不更改原生配置，不将历史任务标记已查看。默认恢复 TaskPulse 来源设置；`TASKPULSE_KEEP_WSL_CODEX=1` 仅在全部验收成功时保留新来源启用。可以用 `TASKPULSE_WSL_PROJECT` / `TASKPULSE_WSL_TITLE` 指定验收样本，默认仅检查发现真实会话，不依赖某个用户项目。验收后关闭临时调试进程，正常启动托盘。

### 2026-10-20 自动发现误报回归

在同时运行 Ubuntu 和 docker-desktop 的 Windows 主机上，真实只读发现验证通过：Ubuntu 有 Codex 证据，docker-desktop 无推荐来源，本次耗时 192 ms（单次样本，不是时延保证）。原生 WebView2 IPC 和管理页确认不再出现 Docker 的发现提示，监控设置前后相同。完整验证为 Rust 336 通过 / 8 opt-in 忽略、Python 离线探测 5 项通过，另有 Svelte、Clippy、前端及原生构建通过；这些结果不代替全新 Agent 长期端到端验收。
