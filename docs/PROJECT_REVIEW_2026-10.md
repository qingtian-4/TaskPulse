# TaskPulse 功能改进与实施交接文档

更新日期：2026-10-15（Asia/Shanghai）。当前实施分支：`codex/desktop-discovery-visibility`，版本 `0.2.0-alpha.6`（未发布；alpha.1 打包失败、alpha.2–5 安装验收环境/脚本失败）。初次审查为 2026-10-12，基线 `f3aca96`；禁用推测批准的基线为 `ef52b0f`。

本文面向接手实施的 Agent。第 2–10 节保留历史审查、问题和原始验收清单；未勾选不能直接当作当前未修复证据。第 11–13 节记录后续实际实施，**当前状态以本节摘要和最新执行记录为准**。原有 `REVIEW_HANDOFF.md` 不作修改。

| 任务 | 当前实现 | 尚未完成的验收／范围 |
|---|---|---|
| T00–T04 | 主要可靠性修复已实现；Guardian 过滤另见第 12 节 | 新真实 Claude 批准、Hermes 实时本轮边界与长期端到端延迟仍需验收 |
| T05 | 前台 wrapper 基础已实现，统一工作流未完成 | JobRunProvider、run/attach、独立管理来源 |
| T06 | 运行优先、语义分类、失败结果查看、推断退出码未知已实现 | 本轮原生界面验证与持续使用验收见第 13 节 |
| T07 | 有界队列、轮转、持久请求回查／手动重开已实现 | 不持久化/重播短暂弹窗；依赖 attention 状态恢复，真实新批准链路待验收 |
| T08 | 配置保留与 DSH WSL 防唤醒已完成 | Claude WSL/SSH、Hermes WSL 未新增 |
| T09/T10 | 未实施 | 通用外部事件契约／新 Agent、额度独立开关 |

开源准备：Windows Alpha 版本、发布前检查、草稿预发布和安装验收门禁已加入；不等于安装包已发布或干净系统验收通过。见 [发布流程](RELEASING.md) 和 [更新日志](../CHANGELOG.md)。

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

### T06：运行优先的主面板（原始验收清单）

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

### T07：提醒回查与恢复（原始验收清单）

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

## 11. 第一阶段执行记录

本次继续执行先落实 T00–T04 的已确认可靠性问题，以及 T08 的配置保存和 DeepSeek WSL 防唤醒问题；不是宣称 T05–T10 路线图全部完成。原始审查与勾选清单保留，实际状态以下表为准。

| 项目 | 核对与实施状态 | 实现和限制 |
|---|---|---|
| T00 | 已修复，保留 `ef52b0f` 的禁用降级批准逻辑 | 没有恢复计时批准猜测；普通慢工具／思考仍为运行阶段 |
| T01 | 确认存在，代码修复；真实存储读取通过 | Hermes 缺失／过期 lease 不再生成本轮结束，保留原生命周期为过期；冷启动无证据为 UNKNOWN。原生 ended_at/end_reason 才确认结束；释放 lease 不换已观察 run identity。真实来源本次无新鲜运行／结束，不冒称实时 turn 验收 |
| T02 | 确认存在，代码修复；真实原生读取通过，真实新 Hook 批准尚未验收 | 官方 PermissionRequest 没有 tool_use_id；改成加锁、有序 JSONL 回放＋兼容 checkpoint。唯一工具输入指纹关联 call；并行请求独立去重，匹配结果单独解除。PostToolUseFailure 不等于整轮失败；thinking／coarse idle 不结束任务。新增 Hook 事件仍需用户明确重新确认，不自动安装 |
| T03 | 确认存在，代码修复，真实 SSH 分段验收通过 | 独立 tmux/ps/runs/Codex 成功和错误，watcher 按来源实例更新，不因某段失败阻塞成功来源。pane 消失／重建只未知或过期；仅同 root PID 的新鲜 shell-return 可推断结束，退出码未知 |
| T04 | 确认存在，代码和隔离运行验收通过 | run ID 为身份；A 结果不覆盖 B，匹配 wrapper 后代才抑制重复推断。活跃 run／本地状态文件不因 TTL 消失；心跳不刷新执行活动。核对 PID/start ticks/boot ID，失联不写成功。旧 wrapper 保留参数、并发输出、退出码和终态；Python 3 和 Windows 原生可执行文件限制见下 |
| T05 | 部分基础已具备，完整工作流未实施 | 已有 v2 前台监督脚本和本地兼容状态，不等于新增 JobRunProvider、统一 run/attach CLI 或管理来源；后台、容器、Slurm 仍不能视为受监督作业 |
| T06 | 仅修复失败结果快捷“已查看”，其余未实施 | 保留失败事实，不伪造成功；运行优先分组、语义组号和推断退出码未知的全面 UI 尚待下一阶段 |
| T07 | 保留现有设计并补并行请求匹配；恢复策略未实施 | 原生弹窗烟测通过，真实阻塞无计时且旧 timer 不误关下一条；队列持久化／重启可恢复入口仍待实施 |
| T08 | 确认配置／WSL 问题并修复，其余未实施 | 递归保留 TOML 未知字段，sources 按稳定 ID 合并，删除项不复活；DSH WSL 查询前检查 running distro，不自动唤醒。Claude WSL/SSH、Hermes WSL 尚未新增 |
| T09/T10 | 未实施 | 新 Agent 能力调查、通用外部契约与额度停采集／展示开关留给后续 |

### 实现与兼容

- 生命周期／配置：`providers/hermes.rs`、`providers/deepseek_harness.rs`、`providers/local_file.rs`、`monitoring.rs`。
- Claude：`claude_events.rs`、`claude_integration.rs`、`providers/claude_code.rs`；`domain/task.rs` 的 input_request_ids 兼容原 scalar。新增序号回放、并行请求／旧结果／重启测试。关联不唯一时不猜测 call，不用其他工具结果解除请求。[官方 Hook 输入文档](https://code.claude.com/docs/en/hooks#permissionrequest-input)支持该限制。
- 远端：`remote_tmux/{mod.rs,runfile.rs,state.rs,snapshot.py}`、`providers/mod.rs`、`watcher.rs`。没有执行日志里的 Code Mode，只运行仓库自带只读 collector。远端需要 Python 3；Codex 24 小时发现／64 KiB 尾部窗口仍在，窗口外不能宣称结束。
- 迁移：`attention.rs`／`history.rs` 备份 `*.pre-run-identity.bak` 后规范旧精确 run ID；`transition/mod.rs` 仅迁移相同 run/turn 的通知记忆，保留未读／固定，不给下一 run 继承上一 run 的去重。
- 运行器：`tp-run`／`wrap.sh` 使用 Python 3、继承 stdio、不拼 shell。`wrap.ps1` 保留 CRT argv；重定向宿主的 stdout/stderr 同时进行原始字节转发，不扫描批准文本。批处理需明确调用 cmd.exe。`.gitattributes` 固定 Linux 脚本 LF，避免 Windows checkout 导致启动失败。

### 实际验证（Windows 主机，2026-10-13）

- Rust 全量：`cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray`，322 通过，7 个 opt-in 默认跳过；fmt、clippy `-D warnings` 和构建通过。托盘运行时使用隔离 target，不覆盖实时可执行文件。
- Svelte check：0 errors、0 warnings；`bun run test:ui-model`：16 通过；前端生产构建通过。沙箱下 Vite spawn EPERM 属环境限制，经审批在正常环境重跑通过。
- Windows `bun scripts/run-wrapper-smoke.ts`：空字符串、引号、尾反斜杠、Unicode、重定向 stdin、250 KB stderr、非零退出、静默心跳、启动失败通过；状态全部在 target/wrapper-smoke 内，未写真实监控目录。
- 已运行 Ubuntu 的 `python3 scripts/run-wrapper-smoke.py`：参数、大输出、静默、成功、失败、SIGTERM、SIGKILL、旧 wrap.sh 兼容入口通过。长于 3 小时、PID 复用／重启由 Rust 模拟时间测试覆盖，不宣称连续运行了三小时。
- 真实 SSH：`live_remote_snapshot`，Server1_wanghy／Server3 的四类分段全部健康；154 个原生会话目录；确认 YOLO-RDD 的“梳理 YOLO-RDD 项目流程”和目录里的 HyperMem。HyperMem 本次无近期执行任务，未批量导入空闲历史。用 EXPECT_REMOTE_CATALOG_PROJECT 与 EXPECT_REMOTE_PROJECT 分别验收目录发现／活跃结果。
- 真实只读来源：`live_hermes_snapshot`、`live_claude_snapshot`、`live_wsl_codex_snapshot`、`live_desktop_snapshot`、`live_desktop_schema` 通过。Claude 已安装 VSCode 扩展 2.1.296；DSH 读取 v4 事件和原生标题；WSL Codex 冷采集约 1.2 秒、下一次约 0.4 秒。本次没有发起真实 Agent 任务，也未自动安装 Hook。
- 原生 WebView2 `native-notice-smoke.ts` 两轮通过：深色 320×132、长标题和按钮、悬停、批准超过 15 秒仍保留、普通结果约 15 秒隐藏、多条队列、旧回调、点击详情、总开关和无假任务／已查看变更。结果及 PNG 在 `src-tauri/target/ui-smoke/`。第二轮因屏保跳过编辑器焦点验证；新真实 Agent 完成／批准、OS 系统横幅和编辑器真实焦点仍未验收。不能把测试提醒通过说成真实批准全链路通过。
- 没有更改现有项目白名单、代理、statusLine、批准策略；测试后 config.toml SHA256 与执行前相同。`docs/REVIEW_HANDOFF.md` 是用户已有未跟踪文件，保持原样，不提交。

### 下一步

先由用户显式确认 Claude Hook 接入，验证一次真实新任务／批准／失败恢复链路；再执行 T05 的统一运行监控与 T06 的运行优先 UI，随后处理 T07 的提醒恢复。来源缺少明确事件时保持诚实的不确定性，不用新的静默计时补丁换取看似实时。

### 版本管理

- `35c7a4e`：生命周期证据、Claude 事件回放与并行请求、远端分段健康、run 身份迁移、配置与 WSL 保护。
- `148e281`：Windows／Linux 前台运行监督器、标准流与 argv 修复、隔离实机验收脚本、LF 约束。
- 后续同一分支的 `docs: record review fixes and verified limitations` 提交：失败结果快捷查看、审查记录与产品说明同步。

提交保留在 `codex/desktop-discovery-visibility`，本次不 push／PR；正常托盘已重新启动，临时 CDP 调试实例关闭。远端旧 tp-run 和既有 Claude Hook 不自动更新，需用户明确操作。

## 12. Guardian 内部会话误提醒修复

2026-10-13 用户反馈：YOLO-RDD 主任务仍在执行，却收到未手动启动的“Guardian review · 本轮结束”提醒。只读核对 Server1 原生 SQLite 和 rollout：该 thread 的 source 为 `subagent.other = guardian`，并关联到用户主 thread；通知记录确认了 13:12、13:13、13:15 三次内部 turn 结束的误投递。

根因是来源语义在 SSH 传输／映射中丢失，而不是完成计时或任务标题错误：`codex_metadata.py` 未导出 source；`flush_codex` 未复制解析器已有的 internal；发现目录及仅元数据固定分支也没有来源标记。上次 SSH 验收检查了标题和生命周期，但未断言内部来源，所以漏检。

修复内容：

- 元数据导出 source，统一支持原生对象和 SQLite JSON 文本；共享远端元数据结构同时保留标题、项目和来源，标题暂缺也不能丢来源。
- rollout 原生 session_meta 作为旧数据库／尾部截断路径的来源补充；执行、目录、固定记录均传递 internal。依据来源而非“Guardian”标题，保留同名用户任务和非 Guardian 的 coding 子任务。
- 目录对已证实的内部身份保持稳定，后续缺字段的局部刷新不能重新分类成用户任务；旧关注记录通过正常刷新纠正 internal，不删除未读／固定数据，也不修改 Agent 原生会话。
- Transition 与通知边界拒绝内部会话事件；已排队的旧提醒按最新来源分类移出，既不标记已查看，也不解除用户真实批准。普通轮询不重复发布未变化的提醒，不重置弹窗计时。
- 新增回归：主任务持续运行时内部三轮完成无提醒、同名用户完成正常提醒、完整／截断／旧 schema／无标题路径一致、仅元数据固定不绕过过滤、旧分类修正及排队移除。

验证：Rust 328 通过、7 个默认 opt-in 跳过；fmt／clippy `-D warnings` 和构建通过。前端 check 0 errors/warnings、16 项模型测试及生产构建通过。真实 SSH 快照确认两个 Guardian 的 internal 都为 true 且范围过滤拒绝它们，YOLO-RDD 主会话仍为普通任务；HyperMem 目录与两台主机各采集分段仍正常。新增 `scripts/native-origin-smoke.ts` 的真实 WebView2 只读验收通过：主任务保留，内部会话从关注和可选目录排除，旧记录保留且仍为未读，没有当前内部提醒或新投递，不改配置或已查看状态。结果保存于 `src-tauri/target/ui-smoke/native-origin-result.json`。`native-notice-smoke.ts` 也通过：普通提醒约 15 秒隐藏，真实类型批准无计时，队列／旧回调／总开关没有回归；焦点验证跳过。测试后配置 SHA256 与执行前一致。

限制：本次不主动触发真实 Guardian 或执行服务器任务；“主任务运行、内部任务结束”的时序由确定性回放覆盖，真实来源验证的是已存在会话的分类与隔离。来源字段和原生 header 都缺失时不能凭名字断言内部身份。没有自动关闭整个项目、屏蔽所有子 Agent 或恢复不可靠的结束／批准猜测。

## 13. 2026-10-15：Alpha 交付准备与 T06/T07

### 本轮实现

- T06：ongoing/results/uncertain/pinned 语义分类共用于排序、分组、计数和筛选。
  进行中包含运行与明确阻塞；失败是结果，来源故障不是失败；非 Agent shell-return
  显示退出码未知。旧项目筛选不能挡住全局计数入口。
- T07：铃铛从持久 attention 重建未解决请求，包括已查看但未解决的请求；队列
  最多 128 条，溢出的结果/请求可回查。确认阻塞在有后续消息时轮转，不自动解除。
  手动重开要求来源最近成功采集、非过期/推断、仍在范围内；使用新展示 ID，
  不修改已查看/批准/通知去重，不在重启时批量补发旧完成通知。
- 开源交付：四文件版本统一为 0.2.0-alpha.1；Windows x64 MSVC 工作流明确绑定
  标签/SHA、锁定依赖、规范资产名/平铺校验和，安装验收后只创建草稿预发布。
  Linux/macOS 保留独立实验构建，移除对 Windows 发布的 Apple 证书依赖。
- 真正发现的交付问题：普通 cargo build 使用 devUrl，需要开发服务器。补入
  custom-protocol feature，独立构建/验证/CI 明确内嵌前端；verify 先构建前端再
  编译 Rust，避免验证旧资源。WiX upgradeCode 固定为当前 TaskPulse 默认值。
- README 增加安装入口、三步入门、能力边界和明确标注的示意图；示意图不是实机
  截图。新增安装/发布/适配指南、版本说明、CHANGELOG；保留 MIT 和上游历史。

### 已实际运行的验证

- Windows `scripts/verify.ps1 -Frontend`：Rust 333 通过、7 个默认 opt-in 跳过；
  fmt/clippy -D warnings、内嵌前端可执行文件构建通过。Svelte 0 errors/warnings，
  UI 模型 19 通过，发布模型/工作流结构 4 通过，前端生产构建通过。
- PowerShell installer-smoke 脚本语法检查通过；不是实际安装验收。
- 收尾修正隔离 UI fixture 的通知日志数组类型并捕获 Runtime.exceptionThrown，
  避免只凭页面文字存在漏检运行时错误。Vite 忽略 Rust target/build/dist 产物，
  避免构建或临时浏览器资料触发无关页面刷新；严格烟测重跑通过。
- 隔离 Edge＋模拟 IPC UI smoke：28 条任务滚动、全局入口清除旧筛选、新项目执行、
  请求回查/手动重开不调用已查看、管理窗口通过；不当作真实 Agent 验收。
- 真实内嵌 WebView2 inbox smoke：IPC、分组顺序、回查空状态、440×720 上限与
  无横向溢出、配置不变通过。实际 6 条关注、0 条明确待处理请求，**不是新批准
  到恢复/重开/解除的实机全链路**；多请求和重启由隔离存储回归测试覆盖。
- 原生 notice smoke 两轮：320×132 深色窗口、长标题、悬停、普通约 15 秒收起、
  明确阻塞超过 15 秒保留、手动及自动轮转、旧 ID 回调安全、点击详情、总开关、
  无假任务/已查看变更通过。恢复原配置；桌面锁定时未验证编辑器焦点。
- 真实 SSH 只读快照：两台已配置主机的 tmux/ps/runs/Codex 分段健康，160 个
  原生目录；用户主会话保留，两个 Guardian 都标为 internal。没有发起服务器任务。

### 尚未完成，不应宣称通过

Windows MSVC 安装器的云端构建/实际安装烟测待工作流结果；干净物理机/VM、
缺少 WebView2、跨版本升级、真实新 Claude 批准与编辑器焦点、长期端到端 P50/P95
仍需专门验收。没有签名证书或自动更新器，也没有发布 Portable/稳定版。
T05 完整运行入口、T08 剩余传输、T09/T10 仍未完成；不能用本轮发布整理代替它们。

源码提交：`a851fe7`（T06/T07 与原生测试）、`23d4e54`（Windows Alpha 发布/安装
门禁与内嵌构建）。后续文档提交记录实际范围，不包含用户未跟踪 REVIEW_HANDOFF.md。

## 14. 实际发行构建发现 MSI 版本约束

PR #2 已经 Windows CI 验证并合入 main（`a9e879c`），源代码与本地已验证树相同。
`v0.2.0-alpha.1` 的实际发行流水线 `38614949382`：preflight、前后端门禁、
MSVC ci-release 程序、NSIS EXE 均通过；MSI 在打包时拒绝非数字 prerelease
`alpha.1`，安装烟测和草稿发布被跳过。没有把成功编译或已生成 EXE 冒充完整发行。

修复采用不同数据域，而不是删除 Alpha 标签或伪造稳定版：应用/标签/四文件版本
为 `0.2.0-alpha.2`，WiX 为数字 `0.2.0.1002`；校验器派生并检查映射，manifest
明确记录两种版本，安装烟测读取 MSI ProductVersion 和真正程序 --version。
Windows 升级忽略第 4 字段，未据此宣称跨版本升级通过。失败的 alpha.1 标签保留，
Alpha.2 使用新提交、新 PR、新标签，不覆盖已推送的历史。

本地 Alpha.2 再验收：Rust 333 通过、7 个默认 opt-in 跳过；fmt/clippy/内嵌构建
通过；Svelte 0 errors/warnings、UI 19、发布模型 5、生产构建与安装脚本语法通过。
新的 MSVC MSI/安装/草稿结果必须等待实际流水线；干净机器等人工门禁继续保留。

### 前次停止点：GitHub 审批连接不可用

Alpha.2 修复 `2db67f2` 已推送至 codex/desktop-discovery-visibility；远端 main 为
`a9e879c`（已合并 PR #2）。alpha.1 标签保留；尚未创建 alpha.2 标签或发布。
创建修复 PR 的自动审批返回 stream disconnected，明确说明操作未执行；后续
只读 GitHub 连接健康检查也失败。因此不通过直接写 main/替代认证 API 绕过审批。

本地正常内嵌托盘已构建并启动 Alpha.2；测试调试端口已关闭，用户范围/代理/
Hook 配置不变。待连接恢复，先只读核对有无同分支新 PR，避免重复创建；然后
完成修复 PR 的 CI/合并、验证 main 树、推送新标签、实际安装烟测与草稿交付。
当前没有可宣称完成验收的安装包，也没有公开稳定 Release。

## 15. 审批恢复后的 Alpha.2 合并与发行验收

2026-10-15 恢复后先读取同分支的开放 PR 列表，确认没有重复创建，再推送交接
提交 `ffb2fb0`，创建 [PR #3](https://github.com/qingtian-4/TaskPulse/pull/3)。
[Windows quality 38669975058](https://github.com/qingtian-4/TaskPulse/actions/runs/38669975058)
对该准确 HEAD 完整通过后，以 merge 保留历史合入 main：
`8e36cfe45bb948d099c9380f59d8345ecaf9320c`。合并树与已验证分支无差异。

新标签 `v0.2.0-alpha.2` 已推送且指向上述 main SHA；没有移动 Alpha.1 标签。
[Alpha.2 发行流水线 38670333565](https://github.com/qingtian-4/TaskPulse/actions/runs/38670333565)
已通过 preflight、Windows 完整门禁、EXE/MSI 打包、资产归一和上传。期间连接
间歇中断，恢复后读取实际结果：安装烟测 `116215509942` 失败，草稿被跳过。
日志明确失败于已安装应用的 WebView2 调试页面连接；不是 MSI 版本错误再次发生。
本地复核 release:check、5 项发布模型测试通过；正常托盘进程仍为工作区内的
Alpha.2 内嵌构建且响应正常。未更改用户来源、代理或 Hook 配置。

保留两个历史标签，没有把编译成功等同于安装验收，也没有公开发布稳定版。
第 13 节未验收项仍然有效。

## 16. Alpha.3：安装权限与应用验收权限分离

Alpha.2 安装日志显示 NSIS 安装、安装资源与真实程序 --version 已检查完成；
原生页面探测超时。上游已记录 WebView2 150+ 在提权进程中忽略环境调试参数，
GitHub Windows runner 默认提权，症状吻合。参考
[Wry #1782](https://github.com/tauri-apps/wry/issues/1782) 和
[Tauri 官方 Windows E2E](https://github.com/tauri-apps/tauri/blob/dev/.github/workflows/test-api-e2e.yml)。
这属于有证据的环境诊断，仍须新的真实安装验收确认修复有效。

改为保持安装器正常权限，仅通过临时 CI runner 的 gsudo 将应用/IPC/重启子进程
降到普通用户权限；测试 helper 主动拒绝提权运行，产品不增加调试入口或更改 CSP。
应用 PATH 仍不包含开发工具，真实 onboarding/空任务/空提醒与重启断言均保留。
增加安装器/启动等待上限、stdout/stderr、端口/页面探测和结构化失败结果；首项
失败也会保存 result.json。不在用户电脑安装 gsudo 或修改注册表安全策略。

应用四文件版本为 `0.2.0-alpha.3`，WiX `0.2.0.1003`；通过新的 PR/标签验收，
不移动 Alpha.2，不把环境原因当作可以跳过原生门禁的理由。

本地回归：Rust 333 通过/7 opt-in 跳过，Svelte 0 错误/警告，UI 模型 19、
发布模型 6（含两份 PowerShell 脚本的 Windows 语法检查）通过；fmt/clippy 和
前端生产构建通过。原生安装验收仍须等待新的云端结果，不能由脚本语法代替。

### Alpha.3 合并和发行跟踪

修复提交 `8d93317` 经 [PR #4](https://github.com/qingtian-4/TaskPulse/pull/4) 合入
main `ca37e33daee24612501b4e3501d8904493c67603`。准确 HEAD 的
[Windows quality 38673434536](https://github.com/qingtian-4/TaskPulse/actions/runs/38673434536)
完整通过；合并树与本地已验证树一致。不可变新标签 `v0.2.0-alpha.3` 已推送，
指向该 main SHA；原 Alpha.1/2 标签未改变。

[Alpha.3 发行 38673800337](https://github.com/qingtian-4/TaskPulse/actions/runs/38673800337)
已通过 preflight、前端/Rust 检查、两个安装包构建和上传。独立安装烟测
`116226816968` 在 nsis-version 阶段失败，草稿被跳过；没有把 PR CI 或成功
打包等同于发行验收。

## 17. Alpha.4：统一子进程事实采集，修复验收假失败

实际下载并检查 Alpha.3 证据 artifact `11946035360`：nsis-version.txt 内容为
正确的 `TaskPulse 0.2.0-alpha.3`，不是安装了旧包。本机只调用已有程序 --version
连续 5 次复现：Start-Process -PassThru + WaitForExit 后子进程已退出且输出正确，
但 ExitCode 为 null，旧断言因此误判。没有启动新托盘或 Agent 任务。

根因修复是统一的 `installer-process.ps1`：从 Start 持有真实 Process 句柄，
并发读取 stdout/stderr、限时等待并保存真实退出码；超时只终止自己创建且已核对
可执行路径的子进程。安装器和 --version 使用同一实现；版本字串断言没有放宽，
证据同时记录实际输出/退出码。新增不执行安装器的真实 Windows 进程回归，检查
快速成功输出、非零退出和超时；发布测试 7 项通过，含四份 PS 脚本语法检查。

四版本文件为 `0.2.0-alpha.4`，MSI `0.2.0.1004`。继续使用新 PR/新标签，不移动
历史标签。普通用户 UI/IPC/重启与两个安装器的完整烟测仍须等待下一轮真实结果。

Alpha.4 本机完整 verify -Frontend 通过：Rust 333/7 opt-in、UI 19、发布/进程
测试 7、Svelte 0 错误/警告、fmt/clippy、前端与内嵌原生构建全部通过。对同一
工作区程序 --version 使用新等待实现，实际取得 ExitCode 0 与正确版本输出；
只使用单个验证进程的执行策略参数，没有修改系统执行策略或用户 Agent 设置。

### Alpha.4 合并和真实安装结果

[PR #5](https://github.com/qingtian-4/TaskPulse/pull/5) 经准确 HEAD Windows quality
`38684415101` 通过后合入 main `11b2b708eea2a91597c31fdc3c2a9cf35778648c`，
推送新标签 v0.2.0-alpha.4；未改写历史标签。
[发行 38685012588](https://github.com/qingtian-4/TaskPulse/actions/runs/38685012588)
再次构建两个安装器成功；安装烟测 `116260971947` 版本检查已通过，随后普通
用户首次启动成功，实际 native IPC 证据为 embedded=true、onboarding=false、
tasks=0、prompts=0。清理进程时旧 GetFullPath 字串比较拒绝了路径不同的自有
可执行文件，未执行重启/MSI 全流程，草稿被跳过。证据 artifact `11950415731`
已实际下载检查；没有把首次启动通过写成整个安装验收通过。

## 18. Alpha.5：可执行文件身份与 IPC 证据契约

Windows 最终文件路径使用 GetFinalPathNameByHandleW 的原生文件句柄解析；
自有进程超时清理与原生应用停止使用同一比较器，接受长短名称/最终路径相同
的文件，仍拒绝其他文件，不降级为按进程名停止。原始 expected/observed 路径
保存在 CI 证据。本机真实临时用户目录回归确认 spelling differs=True 而最终
身份相同；不同系统可执行文件的比较仍为 false。见
[Microsoft 最终路径 API](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew)。

Alpha.4 证据还显示 WinPS 异步 Connect/Send 的辅助返回值污染了函数输出。
现在显式抑制辅助值，并要求 native_ipc 是单一、已验证的对象，不能用数组通过
断言。重启前限时等待旧 CDP 端点关闭，避免连接旧实例；成功探测清除旧错误。
上述均是验收工具约束，未改产品监控状态、用户配置或 CSP。

应用版本 `0.2.0-alpha.5` / MSI `0.2.0.1005`，继续经新 PR/标签真实验证，
不更改 Alpha.4 标签，也不把工具修复代替完整安装结果。

审批恢复后，Alpha.5 本机完整 verify -Frontend 已通过：Rust 333 通过/7 个
opt-in 跳过，UI 19、发布/真实进程回归 7、Svelte 0 错误/警告、fmt/clippy、
前端与内嵌原生构建均通过。完整发行验收仍等待新的标签流水线。

## 19. Alpha.6：安装格式的测试环境隔离

[PR #6](https://github.com/qingtian-4/TaskPulse/pull/6) 经准确 HEAD Windows quality
`38689520869` 完整通过后合入 main `7f834eae0bd2d2c4cfde5c46e8db60cd42767b89`，
新标签 v0.2.0-alpha.5 指向该提交。发行
[38690260098](https://github.com/qingtian-4/TaskPulse/actions/runs/38690260098)
两个安装器构建通过。证据 artifact `11952671007` 已实际下载核对：NSIS 的
安装、真实版本、普通用户 UI/IPC、重启、重装、卸载全部通过；签名 NotSigned。

随后 MSI 成功返回 0，但详细日志明确把 INSTALLDIR 从测试指定的 msi 子目录改回
先前 nsis 子目录，并在那里复制了 agent-tray.exe。原因是 Tauri 保留的 InstallDir
注册表偏好；不是安装包缺少程序，也不能称 MSI 的完整生命周期已验收通过。
同一环境顺序安装两个格式是迁移场景，之前将其当成独立首次安装属于测试隔离错误。

修复为 fail-fast=false 的 NSIS/MSI 独立新 runner 矩阵，各验证一个格式，产出不同
命名的证据；两者都通过才允许草稿。参数由 ValidateSet 限定，不删除或改写用户
目录偏好来强行通过检查。跨格式迁移/自定义目录优先级新增明确人工验收项。
应用/标签版本 `0.2.0-alpha.6`，MSI `0.2.0.1006`，仍须真实重新验收。

Alpha.6 本地完整 `scripts/verify.ps1 -Frontend` 已通过：Rust 333 通过、7 个
opt-in 跳过；UI 模型 19 项；发布/真实进程回归 7 项、65 个断言；Svelte 0 错误/
警告，fmt、clippy、前端生产构建和内嵌原生构建通过。该结果不替代新 runner
上的两个安装格式验收；未更改正常运行的托盘、用户来源、代理或 Hook 配置。
