# TaskPulse 项目审查、监控扩展与命名建议

审查日期：2026-10-12（Asia/Shanghai）。基线：`codex/desktop-discovery-visibility`，`f3aca96`。

本次交付为代码审查与文档更新，不改变采集器、UI、Agent 配置或用户监控范围。原有未跟踪文件 `REVIEW_HANDOFF.md` 保留。以下“已实现”是源码证据，不等同于本次在每一种真实 Agent／服务器环境验证通过。

## 1. 判断与实施顺序

产品应定位为 **AI Agent 与代码运行的统一状态和提醒工具**。现有 Provider、Store、Attention、Transition 分层值得保留，不需要推倒重写。下一阶段应先补可靠的代码运行监控和生命周期证据，再扩展更多品牌。

建议顺序：

1. 修复下述状态／通知可靠性问题，避免任务仍在执行却显示结束、第二次批准不提醒、长任务丢失等情况。
2. 提供统一的普通代码运行入口，覆盖 Windows 本机、WSL 与 SSH，补齐无 tmux 场景。
3. 将主面板改为运行优先，保留可回查的待批准、失败与采集异常。
4. 补 Claude WSL／SSH、Hermes WSL，再按接入证据扩展其他 Agent。
5. 独立额度开关、品牌统一和发布整理。

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
- 目前 “需处理”混合失败、源过期、真正阻塞和疑似授权，不是单纯的“批准请求”。需要分别展示证据与动作。

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
<新名称> run --name "训练实验" -- python train.py
<新名称> attach --pid <PID>
```

同一运行器在真正执行代码的环境中启动：Windows 本地可执行文件、WSL 内 Linux 运行器、SSH 服务器 Linux 运行器。桌面不代用户重新执行已经启动的命令；`attach` 是显式观察。

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

当前实现是：[taskSection](../src/lib/dashboard.ts#L56) 先把 stale、failed、真正阻塞和 approval-check 放进第 0 组，之后才判断 running。结果是底层仍为 RUNNING 的疑似批准任务也不计入“运行”。

建议主导航为：

```text
进行中 8       待查看 3       全部
        提醒入口：待批准 1 · 失败 1 · 来源异常 1

进行中
  训练实验        运行中        2h 15m
  修复登录流程    等待批准      打开 Agent
  分析评测结果    检查授权      状态尚未确认

待查看
  执行测试        成功 · exit 0  标记已查看
  重构方案        本轮结束      标记已查看
```

用“进行中”作为包含 running＋真实 waiting-input blocker 的分组名，比把被阻塞任务统称“正在运行”准确。若坚持标题“运行中”，计数应只统计真实 running，阻塞作为相邻独立计数；不能为了排序改底层生命周期。

具体规则：

- 不确定的批准检查留在 RUNNING，显示“检查授权／状态未确认”，不要写成已确认阻塞。
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

本次完成：

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

交付：重写中文 README，同步英文 README 的支持边界，新增本报告。运行功能和 UI 调整仍为待实施建议。

## 9. 命名候选与迁移

当前品牌已经不是完全未改名：`tauri.conf.json` 的 `productName` 为 TaskPulse，identifier 为 `app.taskpulse.desktop`；但 Cargo／Bun 包与 binary、部分桌面文件沿用 AgentTray 名称。

**TaskPulse 不适合作为最终无重名目标。** 已找到同名任务监控项目 [workermill-examples/taskpulse](https://github.com/workermill-examples/taskpulse)，以及同名 Tauri 任务应用 [alokgorithm/task-pulse](https://github.com/alokgorithm/task-pulse)。

建议不用 Agent 限定范围，因为普通代码任务是核心；不用 Tray 绑定 UI 形式，因为未来可能增加独立窗口。以下为本轮候选：

| 顺序 | 名称 | 建议读法 | 命名理由 | 取舍 |
|---|---|---|---|---|
| 1 | **RunTelli** | run-TEL-ee，近似“朗特利” | Run＋tell 的造词：运行有变化，它来告诉你；同时涵盖 Agent 和代码 | 品牌感较好；需要副标题说明监控定位 |
| 2 | **RunPiko** | run-PEE-koh，近似“朗皮可” | Run＋短促轻巧的 Piko 音节，适合轻量桌面助手 | 最短、较好记，但名字本身不直接表达提醒 |
| 3 | **TaskChirp** | task-chirp | 任务有消息时像鸟鸣一样提醒；适合做简洁图标 | 含义直观；中文用户读 chirp 不如前两个顺口 |

推荐 **RunTelli**，产品副标题可用“AI Agent 与代码运行监控”，英文为 “A desktop monitor for agents and code runs.”；若首要目标是短、易读，则选 RunPiko。

查重记录（2026-10-12）：

- 实际调用 GitHub 公共仓库搜索 API：`/search/repositories?q=<name>+in:name&per_page=10`；`RunTelli`、`RunPiko`、`TaskChirp` 各返回 `total_count: 0`。
- 公开搜索分别检索完整名称及 GitHub 组合，未发现同名开源软件；出现的少量非软件用词不作为开源重名。
- `RunChime` 已查到 `vpukhanov/runchime_web`，淘汰；`TaskMelo` 已有软件产品，淘汰；`RunDori` 有近似仓库及其他用途，淘汰。
- 这里的准确表述是“本次公开检索未发现重名”，无法保证未索引、私有或未来项目不重名。没有核验包注册表、域名或商标可用性；定名前应复查。

选定后分两步迁移：

1. 先统一产品展示名、README、仓库描述、图标文字、窗口标题与通知文案，保留 AgentTray 的 MIT 归属。
2. 再迁移包／binary、打包与桌面文件、CI／脚本、安装路径、Hook 命令、卸载信息。检查已安装 Claude Hook 对旧绝对 exe 路径的引用，不能只改 binary 名。

数据目录与 Tauri identifier 不做盲目全局替换：迁移前备份 `~/.taskpulse`，兼容读取旧目录，保留任务 ID、未查看、固定、通知去重与来源白名单，避免改名后所有任务重复提醒。bundle identifier 是否改变需要结合升级安装策略决定；现有值是 `app.taskpulse.desktop`，不是旧文档所写的上游 identifier。

本次未替用户选定或实际迁移名称，README 继续使用当前可执行产品的展示名。
