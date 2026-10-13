# TaskPulse 功能改进与实施交接文档

更新日期：2026-10-13（Asia/Shanghai）。当前核对基线：`codex/desktop-discovery-visibility`，`ef52b0f`。初次审查为 2026-10-12，基线 `f3aca96`。

本文面向接手实施的 Agent，可作为单独的交接材料。第 2–7 节给出现状、问题和方案，第 9 节提供任务编号、依赖和验收清单，第 10 节提供可直接复制的执行说明。本次仅整理文档，未实施其中的功能建议；原有 `REVIEW_HANDOFF.md` 不作修改。

以下“已实现”指源码中已有入口，不等于已在全部真实环境验收；静态审查发现应先核对最新代码并复现，再修改。执行时以仓库最新 `AGENTS.md` 和用户要求为准。

**已完成的基线修复（不得回退）：** `ef52b0f` 已移除推测式 Codex 批准提醒。命令权限参数、工具响应慢、日志静默和 CPU 活动均不能证明等待人类批准；禁止定时推测或发送授权检查提醒。只有来源明确的批准事件可设置 `WAITING_INPUT / approval`，并按匹配请求 ID 解除。兼容旧 `awaiting_tool` 数据时仍视为运行阶段，不显示批准标记。

## 1. 判断与实施顺序

产品应定位为 **AI Agent 与代码运行的统一状态和提醒工具**。现有 Provider、Store、Attention、Transition 分层值得保留，不需要推倒重写。下一阶段应先补可靠的代码运行监控和生命周期证据，再扩展更多品牌。

建议顺序：

1. 修复下述状态／通知可靠性问题，避免任务仍在执行却显示结束、第二次批准不提醒、长任务丢失等情况。
2. 提供统一的普通代码运行入口，覆盖 Windows 本机、WSL 与 SSH，补齐无 tmux 场景。
3. 将主面板改为运行优先，保留可回查的待批准、失败与采集异常。
4. 补 Claude WSL／SSH、Hermes WSL，再按接入证据扩展其他 Agent。
5. 独立额度开关和发布文档整理；额度不是本轮核心功能的前置依赖。

## 2. Agent 支持的实际边界

| Agent | Windows 本机 | Windows 观察 WSL | SSH 原生会话 | 主要证据 |
|---|---|---|---|---|
| Codex | 已实现 | 已实现，按发行版启用 | 已实现 | `codex.rs`、`codex_wsl.rs`、`remote_tmux/codex_metadata.py` |
| Claude Code | 已实现；原生记录＋可选 Hook | 没有专用来源／传输 | 没有专用来源／传输 | `claude_code.rs`、`claude_integration.rs`；SourceKind 只有 `ClaudeCode` |
| Hermes | 已实现，本机数据库与 lease | 没有专用 Provider | 没有专用 Provider | `hermes.rs`；配置只提供本机数据库路径 |
| DeepSeek Harness | 已实现，Desktop/CLI | 已实现，独立 worker | 没有专用适配 | `deepseek_harness.rs` 的 native／WSL 两个入口 |
| Gemini CLI | 兼容 Hook 与扫描 | 没有专用原生会话适配 | 没有专用原生会话适配 | `scanner/strategies/gemini.rs`、旧 Hook |
| ZCode／Trae／WorkBuddy | 未实现专用适配 | 未实现 | 未实现 | Provider 模块与来源注册表无对应项 |

入口核对：[SourceKind](../src-tauri/src/monitoring.rs#L15)、[worker 构造](../src-tauri/src/watcher.rs#L668)、[管理窗口来源选项](../src/routes/manage/+page.svelte#L36)。

**不能因为提供了一个目录输入框，就宣称支持 WSL。** Claude 的存活证据涉及本机进程，Hermes 涉及 SQLite/WAL；手填 UNC 路径不等于完成发行版发现、Linux PID、权限、故障和路径语义适配。更不能通过 Windows UNC 打开正在使用的 WSL SQLite 来代替 Linux 内只读查询。

Codex 本机与 WSL 复用增量解析；SSH 仍以每次快照读取近期日志尾部为主：当前有 24 小时文件发现窗口、每个日志 64 KiB 尾部窗口。长时间无写入、单条超大记录或一轮早期事件位于窗口之外时，证据可能不完整。README 应描述为已实现但有边界，不能写成三个环境完全等价。

DeepSeek 原生路径已支持 v4 日志与独立 projection shard；当前已有 shard 缓存，不能再直接照搬旧交接文档中“没有 shard 缓存”的结论。变化的压缩日志仍需关注解压成本。WSL 路径直接调用 `wsl.exe`，不像 Codex WSL 那样先检查运行中发行版，建议统一为显式启用且默认不唤醒停止的发行版。

### CC Switch 的影响

CC Switch 的职责是管理／切换配置，不是新的 Agent 执行器。其上游说明描述的是切换 endpoint、key、model 等配置字段；TaskPulse 应始终将此类会话识别为 Claude Code，模型作为另一个字段显示。[CC Switch 上游说明](https://github.com/farion1231/cc-switch/blob/main/README.md)

现有 Hook 安装实现会合并自身条目、备份配置，并通过测试验证保留代理、`statusLine` 和其他 Hook。当前 `command`＋`args` 的 exec form 也有官方文档支持。[Claude Hook reference](https://code.claude.com/docs/en/hooks#exec-form-and-shell-form)

这次没有读取或修改用户的密钥、代理配置，也没有切换 CC Switch profile。仍需实机验收：切换后数据目录是否改变，Hook 是否保留、是否实际投递，新旧版本字段是否兼容；不能仅根据模型名称宣称识别了真实服务商。

## 3. 优先发现的问题

优先级：P1 表示应在扩大监控范围／对外宣传稳定支持前修复；P2 表示下一阶段产品与维护性改进。以下均为静态代码路径发现，除验证节列出的自动检查外，没有伪装成真实服务器故障复现。

### P1：Hermes 把 lease 过期当成本轮结束

证据：[hermes.rs](../src-tauri/src/providers/hermes.rs#L109) 的 `row_to_snapshot` 在没有 live lease 且没有明确 session end 时，仍设为 `WAITING_INPUT / TurnEnded`；`expired_lease_is_waiting_input` 测试固化了该行为。[TransitionEngine](../src-tauri/src/transition/mod.rs#L205) 随后可产生本轮结束提醒。

触发：Agent 仍等待模型／工具，但 lease 续期延迟或丢失。影响：用户收到错误的结束提醒。

修复：lease 用于证明活跃，过期只表示活跃证据不足；保留最后生命周期并标记待核实，只有明确 turn-end、session-end 或可验证事件才能结束。置信度应区分租约推断和 Hook，不应把所有数据库记录统一视为 Hook。

### P1：Claude 同一轮第二次批准可能不再提醒

证据：[apply_hook](../src-tauri/src/providers/claude_code.rs#L524) 只从 `PermissionRequest.tool_use_id` 生成请求 ID；官方说明该事件 **不包含 `tool_use_id`**。[Claude PermissionRequest 输入](https://code.claude.com/docs/en/hooks#permissionrequest-input)

缺少 ID 时，同轮多个批准会落到 TransitionEngine 的 `(task_id, WaitingInput)` 去重；第一次已提醒后，后面的独立批准不能可靠区分。`Notification.permission_prompt` 路径同样没有创建独立 ID。Codex 已有较完整的 request-level 去重，Claude 尚未一致。

修复：采集侧持久化每个批准请求实例的序号／事件 ID，并关联当前 turn 与待执行调用；后续 permission notification 关联已有请求，避免同一请求弹两次。不要用观察时间每轮重新生成 ID。验收包含一轮两次批准、重复事件、重启、批准后拒绝／工具失败。

此外 Claude 接收器当前每个 session 只保存一个最新事件文件，短时间内连续事件会覆盖；需要事件序列或增量日志才能保证轮询之间的完整性。

### P1：长时间运行的代码缺少可靠保活

证据：[远端快照](../src-tauri/src/providers/remote_tmux/mod.rs#L128) 只读取 `-mmin -180` 的 run 文件；[tp-run](../scripts/tp-run#L88) 在开始、结束或信号处理时写入，运行期间没有心跳。

触发：运行超过三小时。影响：精确记录暂时从采集中消失；在 tmux 中可能退回推断，在 tmux 外缺少其他证据。完成时重新写入会再次可见，但不能保证期间监控连续。

本地也有类似边界：[LocalFileProvider](../src-tauri/src/providers/local_file.rs#L346) 按文件修改时间清理，旧格式 10 分钟、v2 30 分钟，运行状态没有豁免。输出安静的旧 wrapper 不会持续更新状态。

修复：活跃运行使用 heartbeat／进程身份核对，不受“最近结果”窗口限制；只对结束记录应用保留时间。心跳丢失应变为状态待确认，而不是完成。区分 `observed_at`、`heartbeat_at`、`execution_activity_at`，避免轮询／心跳把旧任务永远留在近期活动里。

### P1：旧本地 wrapper 不是可靠的通用代码监控方案

证据：[wrap.ps1](../scripts/wrap.ps1#L78) 同时重定向 stdout/stderr，却先同步读完 stdout 才 drain stderr；大量 stderr 可填满管道并卡住程序。参数用空格拼接，复杂引号／空格参数也可能改变。

[成功分支](../scripts/wrap.ps1#L104) 直接删除状态文件；`wrap.sh` 成功时也删除。现有 Store／Transition 正确地不把“文件消失”当完成，因此成功任务可能没有可查看的明确结果。旧脚本没有统一的精确退出码数据契约。

修复：用专门的运行监督器记录 child exit status，默认继承 stdio；如需日志，应并发排空两个流。保留终态、原样传播退出码、正确处理参数及取消，退出前停止心跳。不能仅给旧 wrapper 增加一个 UI 按钮就宣称功能完成。

### P1：同一 tmux pane 的旧 run 记录可覆盖新运行

证据：[run_file_to_task](../src-tauri/src/providers/remote_tmux/runfile.rs#L120) 在有 pane 时使用 `ssh:<host>:<pane>` 作为 ID；[tasks_from_run_files](../src-tauri/src/providers/remote_tmux/runfile.rs#L215) 输出全部近期文件，没有按 pane 选取最新运行；快照 `find` 未按开始时间排序。Store 同来源同置信度合并时接受后来的快照。

触发：同一 pane 连续执行 A、B，或者 A 使用 `tp-run`、B 直接执行。影响：历史 A 的 Exact 终态可能盖住正在运行的 B，或同 pane 多个 run 最后由文件遍历顺序决定。

修复：代码运行以 `run_id` 作为运行身份，pane 只作为定位／关联字段；Exact 只覆盖同一次运行的推断，而不是 pane 中所有后续任务。至少在当前架构下先选择最新适用 run 并验证 PID 创建时间。添加顺序颠倒、多次运行和 wrapper／非 wrapper 混用测试。

### P1：SSH 成功不代表每个采集段成功

证据：[snapshot_command](../src-tauri/src/providers/remote_tmux/mod.rs#L128) 将 tmux、ps、run files、Codex 拼在一条命令里，结尾 `true`；解析器跳过非结构化行。`process_snapshot` 会把成功快照中消失的 pane 标成推断结束。

触发：SSH 本身连通，但 tmux socket 权限／命令执行失败，返回错误文本而非 pane 列表。影响：错误可能变成空列表并产生错误的结束判断。

修复：快照每一段都返回结构化 `ok/error/observed_at`；按段执行 authoritative reconciliation。确认“无 tmux server”的语义与异常分开。SSH 超时保护已有，但覆盖不了远端子命令失败。

### P2：配置保留、提醒恢复与来源能力描述

- [write_settings](../src-tauri/src/monitoring.rs#L360) 保留顶层未知 TOML 字段，但整个替换 `[dashboard]`；其内部与 source 内未知字段仍可能丢失。应递归保留未知键，并覆盖旧版本编辑新配置的测试。
- [NoticeQueue](../src-tauri/src/task_notice.rs#L119) 是内存队列，通知去重则持久化；重启后未解决提醒的恢复语义需要定义。这也是不能把待处理信息仅放在弹窗里的原因。
- 当前“需处理”混合失败、源过期和真正阻塞，不是单纯的“批准请求”。需要分别展示证据与动作；推测授权已在 `ef52b0f` 中移除，不得重新加入。

## 4. 代码运行监控：现在怎么用，下一步怎么补

### 当前可用的服务器路径

1. 使用自己的 SSH config 别名，先确认能够非交互连接。
2. 管理窗口添加“tmux · 远程服务器”，选择该别名，设置项目范围。
3. 在 tmux 内直接运行 `python train.py`、`pytest` 等已识别 workload，可得到推断状态。程序退出回 shell 时只能说明“观察到结束，退出码未知”。
4. 需要退出码时，复制仓库 `scripts/tp-run` 到服务器安装：

```bash
mkdir -p ~/.local/bin
install -m 755 scripts/tp-run ~/.local/bin/tp-run
~/.local/bin/tp-run "训练实验" -- python train.py
```

这条命令也可以在普通 SSH／VS Code 终端中运行，不要求 `TMUX_PANE` 存在。无 pane 时源码生成 `ssh:<host>:run:<run_id>`。当前必须启用该服务器的 tmux 来源，并把会话／pane 过滤留空，否则无 pane 记录会被排除。项目路径过滤仍应保留。上述长任务、同 pane 多 run 与采集段健康问题仍待修复；这里说明接线方法，不承诺稳定生产验收。

未使用 wrapper、也不在 tmux 中的已有远端程序，目前不会通过这一机制自动完整监控。旧 `REMOTE_TMUX.md` 中“只看 tmux”的描述仅适用于自动发现，未覆盖 run-file 的无 pane 分支。

### 当前本机状态接口

已有接入点是“通用 Hooks”来源读取 `~/.taskpulse/<唯一运行ID>.status`。自动化脚本可以在启动时写以下 v2 JSON，并同目录原子替换；本例是协议说明，不是已经新增的运行命令：

```json
{
  "v": 2,
  "status": "running",
  "cwd": "D:/work/experiment",
  "started_at": 1791750000000,
  "message": "训练实验"
}
```

时间必须取实际 Unix 毫秒时间；结束时写 `status=succeeded|failed`、真实 `exit_code`、`finished_at`，保留 `started_at`。运行时需定期更新文件以避免当前 TTL 清理。每次运行使用新文件名，因为当前身份来自文件名，不能多个并行任务共用一个文件。

这个接口的标题目前主要取 cwd／文件名，也没有完善的运行进程身份字段；它能作为过渡接入，不能代替新 JobRunProvider。不要把 `tp-run` 的 JSON 直接改扩展名当成本机协议：它的目录、字段和秒／毫秒时间单位不同。

### 推荐实现：统一 RunSupervisor＋JobRunProvider

建议实现后的用户入口（**尚不存在**）：

```text
agent-tray run --name "训练实验" -- python train.py
agent-tray attach --pid <PID>
```

以上复用当前可执行文件名称表示拟议 CLI，不代表现有命令可直接运行。同一运行器在真正执行代码的环境中启动：Windows 本地可执行文件、WSL 内 Linux 运行器、SSH 服务器 Linux 运行器。桌面不代用户重新执行已经启动的命令；`attach` 是显式观察。

| 环节 | 实施方式 |
|---|---|
| 启动 | 创建唯一 `run_id`，保存 argv 数组、cwd、host/source、PID＋创建时间、开始时间 |
| 输出 | 默认继承 stdin/stdout/stderr；用户显式开启后才采集日志；脱敏展示命令 |
| 保活 | 约 5–10 秒心跳；活跃运行不按结果 TTL 淘汰；PID 创建时间避免复用误判 |
| 结束 | 父监督器等待子进程，记录真实 exit code；取消／信号与自然失败分开 |
| 失联 | wrapper 被强杀、系统重启、SSH 中断 → Unknown／stale，不能补写成功 |
| 本机读取 | 新 `JobRunProvider` 读取统一 runs 目录并监听变化 |
| WSL 读取 | 只检查运行中的发行版，在 Linux 内采集；复用只读 WSL transport |
| SSH 读取 | 在已有每主机快照中加结构化 runs 段；不要求安装常驻网络服务 |
| 精确与推断合并 | 按同一次运行关联；pane 是定位，不是 run 身份 |
| 已有进程 attach | 观察 PID／创建时间／进程树；事后一般不能恢复任意进程的真实退出码，结束标为未知结果 |

对于 `nohup`、后台进程、容器、Slurm 等，必须明确观察对象：提交器退出不代表实际作业结束。第一版只保证监督器直接等待的前台命令；后台／调度系统另做 job ID 适配，不能将 `sbatch` 返回 0 当训练完成。

进度可另设显式接口，例如业务代码上报 `current/total/unit`；CPU、GPU 利用率和日志频率不能换算成完成百分比。

## 5. UI 分组建议

**赞成把运行放到首位，也赞成减少“需处理”对主列表的割裂；不赞成让批准信息只存在于弹窗。** 弹窗会被错过、关闭，或者应用重启后丢失；失败和采集异常也不都是批准请求。

当前实现是：[taskSection](../src/lib/dashboard.ts#L55) 先把 stale、failed、真正阻塞放进第 0 组，之后才判断 running。原有 approval-check 分类已移除；接手 Agent 应调整分组展示，而非恢复这一推测分支。

建议主导航为：

```text
进行中 8       待查看 3       全部
        提醒入口：待批准 1 · 失败 1 · 来源异常 1

进行中
  训练实验        运行中        2h 15m
  修复登录流程    等待批准      打开 Agent
  分析评测结果    等待模型响应  本轮仍在运行

待查看
  执行测试        成功 · exit 0  标记已查看
  重构方案        本轮结束      标记已查看
```

用“进行中”作为包含 running＋真实 waiting-input blocker 的分组名，比把被阻塞任务统称“正在运行”准确。若坚持标题“运行中”，计数应只统计真实 running，阻塞作为相邻独立计数；不能为了排序改底层生命周期。

具体规则：

- 权限参数、慢工具或静默日志均不触发批准判断；保留 RUNNING 及实际工具／思考阶段，不产生授权标记和定时提醒。
- 已确认批准／提问属于未完成的进行中任务；行内黄标、弹窗提醒、提醒入口可回查。动作是“打开 Agent／查看详情”，TaskPulse 不直接批准。
- 失败的结束任务进入待查看，保留红色结果与错误原因；查看后不再强制占据永久首屏，事件记录仍可保留。
- 来源失联属于数据新鲜度提示，保留最后已知状态，不把其批量当作业务失败。
- 待查看的单位是任务的本轮／一次运行，不是整个项目。项目可包含多任务，不能把一个 Agent 回答结束说成项目完成。
- “标记已查看”独立于打开详情、聚焦 Agent、关闭提醒；固定与归档继续保留。
- 分组编号改成语义枚举，计数和筛选共用一个分类函数，避免改顺序后硬编码的 `0/1/2` 错位。

## 6. 弹窗与额度

### 弹窗

现有视觉方向可以保留。本次读到的逻辑优点包括：非激活窗口、任务上下文独立字段、独立队列、15 秒普通提醒与悬停暂停、阻塞不自动关闭、旧 ID 不关闭下一条、关闭不标已读、投递记录有界。

证据：[task_notice.rs](../src-tauri/src/task_notice.rs)、[提醒界面](../src/routes/notice/+page.svelte)、[notice.test.ts](../scripts/notice.test.ts)。

建议先改行为而非重画：未解决请求可从面板重新打开；普通完成提醒不要因一条长期批准永远排在后面（当前可手动“下一条”）；定义提醒队列上限、重启恢复、突发多任务轮转。保留“不抢焦点”。将“稍后处理”明确成收起，若未来实现 snooze 再显示具体提醒时间。

本次没有重新运行原生弹窗视觉／点击／焦点 smoke，因此不将单元测试通过写成 DPI、多屏、真实窗口焦点验收通过。

### Codex 额度

赞成做成设置中的可选项，优先级低于代码监控。[Dashboard.svelte](../src/lib/components/Dashboard.svelte#L221) 当前非 demo 时固定渲染额度区，[DashboardSettings](../src-tauri/src/monitoring.rs#L57) 没有额度字段；[Codex app-server 查询](../src-tauri/src/providers/codex.rs#L615) 同时请求线程与额度。

后续拆成 `quota.enabled`（是否采集）与 `quota.show_in_dashboard`（是否展示），名字仅为建议。关闭采集必须停止额度请求，而不只是隐藏 footer；线程元数据仍可继续读取。初装默认关闭或按用户明确选择开启。额度明确标注账号／来源，缺失显示未知，不能混用本机、WSL 和 SSH 的账号额度。

## 7. 更多 Agent 的接入与架构建议

### 用能力驱动接入，不为每个品牌复制整套逻辑

用户添加来源时选择“Agent 类型 → 本机／WSL／SSH → 数据位置 → 项目范围”，随后展示实际能力：会话发现、标题、运行、本轮结束、批准、失败、跳转。每项标记“明确事件／推断／不支持”。“发现进程”和“完整接入”必须分开。

新 Agent 按顺序评估：

1. 有官方 Hook／事件：标准化 start、phase、approval、question、turn-end、error。
2. 有稳定可读的原生数据：只读适配，记录 Agent 版本与已验证 schema，保留推断标签。
3. 只有 CLI：包装整个进程只能提供 CLI 运行边界，不承诺内部每轮对话／批准识别。
4. 只有 GUI／封闭格式：最多展示进程存在或明确“不支持”；不要靠窗口文字、静默时间编造生命周期。

这次没有调查 ZCode／Trae／WorkBuddy 各版本的事件 API，不能先承诺“都用同一个 Hook 即可”。建议先提供版本化的通用事件契约，再按产品逐一确认官方能力。

### 保留现有分层，拆开三个维度

```text
Local / WSL / SSH transport
            ↓
Agent parser / JobRun parser
            ↓
带证据的 Observation → 生命周期归约 → Store / Attention / Notifications
```

由 transport 读取，parser 解释；传输方式不决定 Agent 的生命周期语义。

建议具体拆分：

- `adapter`：codex／claude／hermes／deepseek／generic-event／job-run。
- `transport`：local／wsl／ssh，负责超时、路径、健康、读取边界；避免把 `remote_codex` 永久塞在 `remote_tmux` 里。
- `capabilities`：哪些状态有明确证据，是否支持精确 exit、批准请求与定位。
- `task_kind`：agent_session 与 command_run；Agent 的 turn-end 和程序 exit 不能共用“成功”语义。

观察模型建议携带 `source_instance_id`、稳定 session／run ID、turn ID、request／call ID、事件序号、发生时间、观察时间、证据类别及 freshness。单一 `EXACT > HOOK > INFERRED` 无法表达“新一轮推断比上一轮精确结果新”，应先比较身份和事件顺序，再在同一事实之间比较证据强弱。

通用事件协议建议独立版本化。第一阶段用受控目录里的原子文件／JSONL 即可，不必立即做第三方可执行插件或开放 HTTP 服务。事件重放应覆盖：同轮反复状态、改名、重复到达、乱序、两个批准、短任务落在两次 poll 之间、日志截断、失联和重启。

现有独立采集 worker、降级保留、显式 onboarding、通知去重持久化、归档而不删除未读等设计应继续保留。优先补共享 reducer 和 adapter contract tests；性能优化用测量驱动，不根据旧文档假定当前仍是全量 Codex 解析。

## 8. 验证记录与交付范围

以下为 2026-10-12 初次审查、基线 `f3aca96` 的历史验证记录，不能用作后续实施完成的证据。接手 Agent 必须针对自己的最终变更重新验证：

| 检查 | 结果 |
|---|---|
| Rust format | 通过 |
| Clippy `-D warnings` | 通过 |
| Rust 完整测试 | 302 通过，0 失败，7 项 opt-in 忽略 |
| 隔离目录 Rust build | 通过，`src-tauri/target/codex-check` |
| `bun run check` | 0 errors、0 warnings |
| `bun run test:ui-model` | 15 通过，0 失败 |
| `bun run build` | 通过；第一次受沙箱 `spawn EPERM` 限制，允许子进程后标准命令重试成功 |

未执行：真实 SSH 任务、WSL Agent 会话、用户批准操作、CC Switch profile 切换、原生 WebView2 弹窗 smoke。未停止／替换正在运行的托盘程序，未安装 Hook、提交真实 Agent 提示词或修改用户设置。

自动测试通过说明当前测试集通过，不否定本报告指出的覆盖缺口。特别是同 pane 连续 run、lease 过期与真实结束区分、同轮多个 Claude 批准，需要新增针对性回归测试。

初次交付为中文／英文 README 与审查报告。本次将报告整理成执行交接文档，并同步最新批准规则；运行功能和 UI 调整仍为待实施建议。

2026-10-13 文档交接复核：Markdown 链接、代码围栏与差异检查通过；当前基线 Rust 测试 304 通过、7 忽略，前端模型测试 15 通过，format、Clippy、Rust build、前端类型检查与构建通过。前端构建首次受沙箱 `spawn EPERM` 限制，允许子进程后重试成功。未执行新的真实 Agent／SSH／WSL／原生弹窗验收。这些结果仅验证当前基线，不表示第 9 节待实施任务已完成。

## 9. 实施任务与验收清单

下表是建议实施顺序，不表示功能已经完成。不要一次性改完全部 Provider 再验证。每个任务先确认问题仍存在，再完成聚焦测试、真实来源验证和小范围提交。某项已经由其他提交修复时，记录证据，不重复实现。

### 9.1 阶段与依赖

| 阶段 | 任务 | 目标 | 依赖 |
|---|---|---|---|
| 第一阶段：可靠性 | T00 | 核对基线与保留明确批准规则 | 无 |
| 第一阶段：可靠性 | T01 | Hermes 生命周期修正 | T00 |
| 第一阶段：可靠性 | T02 | Claude 请求身份与事件连续性 | T00 |
| 第一阶段：可靠性 | T03 | 远端采集段健康 | T00 |
| 第一阶段：可靠性 | T04 | 代码运行身份、长任务保活和旧 wrapper | T03 |
| 第二阶段：核心功能 | T05 | 统一代码运行入口，覆盖本机／WSL／SSH | T04 |
| 第二阶段：核心功能 | T06 | 运行优先的面板分组 | T00；与 T05 使用相同状态契约 |
| 第二阶段：核心功能 | T07 | 未解决提醒回查与队列恢复 | T02、T06 |
| 第三阶段：扩展 | T08 | 配置保留、传输复用、缺失 Agent 来源 | T01–T05 的契约稳定 |
| 第三阶段：扩展 | T09 | 新 Agent 能力调查与通用事件协议 | T08 |
| 后续可选 | T10 | Codex 额度独立开关 | 配置保留完成；不阻塞 T01–T09 |

### T00：核对基线与批准规则

**范围：** AGENTS.md、codex.rs、domain/task.rs、transition/mod.rs、dashboard.ts、task_notice.rs，以及对应测试。

- [ ] 记录当前分支、提交、工作区已有改动，保留用户文件；核对本报告中的问题是否仍存在。
- [ ] 保持 ef52b0f 的明确事件规则，不从权限参数、工具超时、日志静默或 CPU 推断批准。
- [ ] 真实批准按稳定 request ID 提醒一次，匹配结果解除；旧请求的返回不解除新请求。
- [ ] 旧 awaiting_tool 快照兼容读取并保持运行，无黄色批准标记、无定时授权提醒。
- [ ] 改名、相同 turn 状态抖动和应用重启均不重新发送已投递通知。

**完成条件：** 回归测试覆盖上述情况，文档和 UI 文案没有恢复旧的批准猜测。

### T01：Hermes 生命周期

**范围：** providers/hermes.rs；必要时调整证据字段、Store 和 Transition 的通用契约。

- [ ] lease 到期／缺失但没有明确结束事件时，保留已知生命周期并显示证据不足；冷启动无历史证据时显示未知，不假定刚结束。
- [ ] 明确区分会话结束、单轮结束、失败和真实用户阻塞。
- [ ] 用真实只读来源核对可获取的结束字段；若当前版本无法证明单轮结束，明确标注能力限制。
- [ ] 更新固化错误语义的测试，覆盖长时间模型等待、续期延迟、明确结束和重启。

**完成条件：** lease 过期不会产生本轮结束提醒，明确事件仍能触发一次正确提醒。

### T02：Claude 批准事件和事件保存

**范围：** claude_integration.rs、providers/claude_code.rs、transition/mod.rs、task_notice.rs。

- [ ] 对照实际安装版本及官方输入格式核实 PermissionRequest 字段，不假定存在 tool_use_id。
- [ ] 为没有来源 request ID 的明确请求建立持久、可重放的请求实例 ID；不能按每次 poll 时间生成。
- [ ] 将同一请求的 PermissionRequest 与后续 permission_prompt 关联，避免双重提醒；关联证据不足时不得擅自解除。
- [ ] 避免单个“最新事件文件”覆盖掉轮询间的独立事件；采用有序事件日志或可靠队列，处理并发写入、半行和重复读取。
- [ ] 一轮两个独立批准分别提醒；同一请求重复到达／跨重启只提醒一次；延迟到达的旧结果不清除新阻塞。
- [ ] 验证安装／移除仍保留用户其他 Hook、statusLine 和 CC Switch 代理配置。未获用户授权时只测试临时配置，不修改真实设置。

**完成条件：** 上述序列测试通过；记录实际 Agent 版本和真实事件验证情况。没有真实样本的字段须标为待验证。

### T03：SSH 分段健康与失败隔离

**范围：** remote_tmux/mod.rs、parser.rs、ssh.rs、state.rs、watcher.rs 和 Store 的来源核对。

- [ ] tmux、ps、runs、Codex 分别带成功／错误和观察时间；SSH exit 0 不再代表每个采集段有效。
- [ ] 部分失败只让对应来源过期，其他成功段继续更新；失败段不能用于删除任务或推断结束。
- [ ] 区分明确“没有 tmux server”和权限／格式错误。
- [ ] pane 消失本身不产生任务完成提醒；非 Agent 的推断结束仅限新鲜、有效的 workload → shell-return 观察。
- [ ] 覆盖 SSH 超时、tmux 错误、ps 失败、run JSON 损坏、部分数据和恢复。
- [ ] 服务器可达时运行真实只读快照，确认预期主机、项目和任务；不可达时记录未验收，不宣称通过。

**完成条件：** 采集错误不会产生完成通知，且无需等待所有来源恢复才能展示成功来源。

### T04：运行身份与长任务基础修复

**范围：** scripts/tp-run、remote_tmux/runfile.rs、local_file.rs、store/mod.rs、旧 wrap 脚本。

- [ ] 用 source instance＋run ID 表示一次代码执行；pane 只负责关联和跳转。
- [ ] 精确记录只覆盖同一次运行，不能用 A 的结束结果盖住 B 的运行；打乱文件顺序仍得到一致结果。
- [ ] 明确旧 pane 身份向 run 身份的兼容策略，保留已有未查看记录，不能整库清空规避迁移。
- [ ] 活跃任务不受最近三小时／本地状态文件 TTL 淘汰；心跳与执行活动时间分离。
- [ ] wrapper 被强杀、机器重启、PID 复用和心跳失联只能标为未知／过期，不能写成功。
- [ ] 修复或以兼容入口替代旧 wrapper 的 stdout/stderr 阻塞、参数破坏、成功删状态问题。
- [ ] 覆盖超三小时的模拟时间测试、同 pane 连续运行、并行运行、Exact／Inferred 混合和非 tmux 运行。

**完成条件：** 新运行不会被旧结果覆盖，长任务持续可见，结束后保留真实退出码与未查看结果。

### T05：统一普通代码运行监控

**范围：** 新运行监督器、JobRunProvider、来源管理、domain/types 和第 4 节所述三种传输。

- [ ] 先定义版本化 run schema、目录、时间单位、ID、终态和清理规则；兼容现有 tp-run 与 .status 格式。
- [ ] Windows 运行器以参数数组启动程序，默认继承 stdio，记录真实 exit status，并在心跳停止后原子写终态。
- [ ] CLI 运行模式在初始化托盘／WebView2 前分流；WSL／SSH 的 Linux 运行器不依赖图形桌面，必要时做独立轻量 binary。
- [ ] WSL 只访问显式启用且运行中的发行版；SSH 按主机批量读取，不要求 tmux，保留来源和项目过滤。
- [ ] 管理界面明确提供代码运行来源，不再让非 tmux 运行依赖名为 tmux 的入口。
- [ ] attach 只观察用户指定已有进程，不重启命令；拿不到退出码时明确说明结果未知。
- [ ] 成功、非零退出、取消、启动失败、空格／Unicode 参数、大量 stderr、长时间无输出都有测试。
- [ ] 只把被监督的前台命令结束作为边界；Slurm 提交成功、容器启动成功和后台化不能冒充实际工作完成。

**完成条件：** 本机、WSL、SSH 各有一条可复现操作步骤与验证记录；环境不可用的路径列为未验收。Agent 会话生命周期不因加入代码运行 Provider 而改变。

### T06：运行优先的主面板

**范围：** dashboard.ts、Dashboard.svelte、TaskList.svelte、types.ts；必要时联动 attention.rs、产品文档与测试。

- [ ] 将“进行中”放在首位，包含 running 和来源明确报告的批准／提问阻塞；行内状态保持真实。
- [ ] “待查看”包含未读的明确本轮结束、代码结果和失败；不能把一轮完成写成项目完成。
- [ ] 为已观察到的普通代码推断结束显示“退出码未知”，不要变成成功；不确定是否结束的任务不进入完成结果。
- [ ] 顶部保留紧凑的待批准／失败／来源异常入口；取消现有大块“需处理”分组后仍能回查。
- [ ] 明确允许查看失败结果；查看后移出待查看，保留失败事实。同步后端 attention 规则，不能只隐藏前端行。
- [ ] 新执行重新出现，标题改名不重置已查看；7 天归档和手动固定语义保留。
- [ ] 用语义分类替代数字组号；总数、筛选、全局运行计数和隐藏提示一致。
- [ ] 普通慢工具仍显示真实运行阶段，无推测批准 UI。

**完成条件：** 前端模型测试和真实 Tauri 面板验收通过；筛选清空后能找到所有对应任务，不出现计数与列表不一致。

### T07：提醒回查与恢复

**范围：** task_notice.rs、notice 页面、Transition／通知记录及面板提醒入口。

- [ ] 保留现有视觉风格和非激活窗口，不以重新设计皮肤替代行为修复。
- [ ] 关闭、轮转、打开详情均不标记结果已查看，也不等于批准已完成。
- [ ] 真正阻塞无自动关闭计时；旧 timer／旧 request ID 不能操作下一条提醒。
- [ ] 定义有界队列、轮转和重启恢复策略；可恢复未解决请求的可见性，不绕过通知去重重新投递旧事件。
- [ ] 只依据新鲜且匹配的来源事件解除请求；来源失联不清除未解决请求。
- [ ] 通知总开关继续覆盖全部渠道，投递记录上限 500 且不保存命令／凭据。
- [ ] 用原生 notice smoke 验证多条提醒、点击、焦点、悬停与不同 DPI；测试任务不能导入真实关注列表。

**完成条件：** 错过弹窗后仍可找回未解决事项，应用不会抢焦点或误标已查看。

### T08：配置和已有 Agent 的跨环境补齐

**范围：** monitoring.rs、Provider 注册／worker、WSL／SSH transport、管理窗口与各 Agent 解析器。

- [ ] 保存配置时保留顶层、dashboard 内部及每个 source 的未知字段；识别 source ID，不按数组下标错误合并。
- [ ] 保留项目白名单、一次性备份和即时生效行为；新来源经过用户确认后才启用。
- [ ] 将传输能力与 Agent 解析逐步分离，避免一次性重写所有 Provider。
- [ ] 补 Claude WSL／SSH 的原生只读来源，再按授权提供可撤销事件接入。
- [ ] 补 Hermes WSL，在 Linux 内通过只读 SQLite 查询，不通过 Windows UNC 打开数据库。
- [ ] DeepSeek WSL 统一为不自动唤醒停止发行版；保持 Desktop 与 WSL 的来源隔离。
- [ ] 明确 Codex SSH 发现／截取窗口的降级语义，对窗口外活跃任务不宣称结束。
- [ ] 每个新增来源覆盖同 session ID 跨主机／发行版不冲突、Linux 路径不被 Windows 归一化、离线与恢复、范围变更。

**完成条件：** 支持矩阵按“代码已实现／真实环境已验证／存在限制”分别记录，不能把一个手填目录输入框当作跨环境适配完成。

### T09：其他 Agent 与通用事件契约

**范围：** 能力调查文档、版本化事件解析、来源注册、可回放 fixture 和支持矩阵。

- [ ] 分别调查 ZCode、Trae、WorkBuddy 实际版本的官方事件和存储；记录可核验来源与样本。
- [ ] 每项能力明确为显式事件、推断或不支持；调查无结果时写清限制，不编造适配。
- [ ] 优先采用官方 Hook，其次只读原生记录；CLI wrapper 仅承诺进程边界。
- [ ] 通用事件携带来源实例、session／run、turn、request／call、事件序号、发生与观察时间。
- [ ] 测试乱序、重复、丢事件、截断、重启，以及多个独立批准；不能从工具结果推断 Agent 结束。
- [ ] 第一阶段无需可执行插件市场、OCR、截图识别或常驻公网服务。

**完成条件：** 至少完成可信的接入能力报告与通用契约；只有具备证据且通过测试的产品才能标为已支持。

### T10：额度可选功能，后续实施

- [ ] 配置分别控制采集与展示，默认策略明确；升级时保留已有用户选择。
- [ ] 禁用采集后不发送额度请求，线程元数据查询正常工作。
- [ ] 标明本机账号范围、数据时间及错误状态，不套用到 WSL／SSH。
- [ ] 前端隐藏、后端停采集和配置热更新均有验证。

**完成条件：** 用户能彻底关闭额度功能；它不影响主监控链路。

## 10. 给执行 Agent 的说明与交付要求

可将下面这段连同本文交给执行 Agent：

> 请以仓库最新 AGENTS.md 为约束，按本文第 9 节的顺序实施。先核对当前代码和已有改动，对仍存在的问题建立聚焦回归测试，再修改；已修复的项记录提交／测试证据，不重复实现。先完成第一阶段可靠性，再推进普通代码监控与运行优先 UI，之后补跨环境来源。额度功能留到核心工作完成后。
>
> 保持现有弹窗视觉方向。禁止从权限参数、工具超时、静默日志、低 CPU 推断批准或 Agent 完成。仅来源明确请求设置批准阻塞，按请求 ID 解除。不得自动安装 Hook、改变批准策略、覆盖代理配置，或用测试任务污染真实任务列表。
>
> 每项完成后记录修改文件、验证命令、真实来源验收结果和未覆盖限制。测试环境不可用时继续可独立完成的工作，最终明确列出未验收范围。不得把单元测试或模拟 UI 通过描述成真实 Agent／SSH／WSL／WebView2 验收通过。

执行过程使用当前 codex/* 分支，保留无关用户改动，小逻辑提交；不自行 push 或创建 PR。运行中的托盘应用使用隔离验证目录，不能按进程名批量杀程序。

功能修改后的基本检查：

~~~powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend
bun run test:ui-model
~~~

若 PowerShell 阻止 bun.ps1，按 AGENTS.md 使用 Bun 的实际可执行文件。脚本或功能变更还应按相关任务执行真实来源和原生窗口验收；远端测试仅在配置服务器预计可达时执行。

每项任务完成后填写：

| 项目 | 必须提供的内容 |
|---|---|
| 任务 ID 与状态 | 已完成／部分完成／未实施，不能只写“已优化” |
| 问题与行为 | 可复现触发条件、修改前后表现 |
| 实现 | 修改文件、数据迁移及兼容处理 |
| 自动验证 | 实际执行命令、通过／失败数量或关键结果 |
| 实机验证 | 系统、Agent 版本、来源类型、验证日期；未执行则写原因 |
| 剩余限制 | 未支持场景、待验证假设、下一任务依赖 |
| 提交 | 当前任务对应 commit，不包含无关用户文件 |

全局完成标准：身份稳定、状态不误报、通知不重复、来源故障不伪装结束、长代码任务不因时间窗口丢失、运行优先且结果可回查；支持矩阵与实际能力一致。
