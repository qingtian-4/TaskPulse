# TaskPulse 通用 Coding Agent 接入执行任务书

编写日期：2026-10-20。面向执行者：GPT-6.1 / Codex。

本文是待实施任务书，不是功能完成报告。当前仓库代码和 AGENTS.md 是实施依据；下面新增文件名、接口名、目录和协议字段属于建议，执行者可根据实际代码调整，但不得削弱产品语义或验收标准。

## 1. 目标与完成定义

让使用尚未内置适配的 coding agent 的用户，可以通过官方 Hook 或标准事件接入 TaskPulse，监控来源实际能够证明的运行、等待输入、失败及本轮结束状态。新增兼容品牌不应总是要求修改 Rust 品牌枚举或重新发布 TaskPulse。

必须交付：

1. 独立版本化的通用事件协议、示例、校验器和适配指南。
2. 随应用可用的轻量事件接收入口，以及耐并发、可恢复的本地事件日志。
3. 通用事件 Provider，贯通现有范围过滤、Store、Attention、Transition、通知和未解决请求恢复。
4. 来源管理中的“其他 Agent”流程：配置、能力展示、预览、明确确认、安装/导出、诊断和撤销。
5. 可导入的声明式适配描述文件，使兼容标准载荷的新 Agent 无须新增 Rust 品牌分支。
6. Kimi Code CLI 优先实施；TRAE 国内 IDE、ZCode、WorkBuddy 分别完成能力核验，并实施有可靠依据的适配。
7. 自动测试、Windows 原生验收脚本、兼容矩阵和证据报告。

“通用平台已完成”“适配器代码已完成”“某版本宿主实机已验证”必须分别记录。缺少安装、账号、授权或官方事件时，不得把 fixture 通过写成实机支持；继续完成不依赖该条件的任务，并交付可复现的待验收步骤。

## 2. 本次范围与约束

### 必须保留

- 任务 identity 使用稳定来源实例 + agent + session；turn 是执行轮次，不是任务身份；标题和项目是展示字段。
- 执行 phase 与生命周期独立。思考、执行工具、模型等待、回答都属于运行。
- 只有明确的审批/提问事件才能设置等待输入；匹配 request ID 才能解除对应请求。多个请求不能互相覆盖。
- 工具失败不自动等于本轮失败；工具返回、日志静默、低 CPU、文件缺失、进程消失不证明 Agent turn 结束。
- 改名、心跳和轮询不能刷新 execution activity；未读、手动固定、旧关注归档和通知去重规则保持。
- 首次接入从确认后的新执行开始，不批量导入空闲历史。既有项目 allowlist、exclude、忽略会话设置保持。
- 原生来源与事件桥并存时按明确身份关联；无法可靠关联就不合并，禁止按标题/目录猜同一任务。
- 模型厂商与 Agent 身份分开。例如使用 Kimi 模型的其他客户端不是 Kimi Code。

### 本次不扩张的范围

- 不重写现有 Codex、Claude、Hermes、DeepSeek Provider，不一次性抽象所有 Local/WSL/SSH transport。
- 第一版完整支持 Windows 本机通用事件来源。预留 transport 字段，但 WSL/SSH 未实现时禁用对应选项并说明；不自动唤醒发行版。
- 不做任意代码插件加载器、插件市场、云中转、默认开放 HTTP 服务。
- 不把 MCP 工具调用或提示词要求“结束时上报”当作可靠生命周期来源。若用户自建上报桥，证据必须标为自报且不冒充官方事件。
- 不做 GUI OCR、窗口文字或 CPU 规则推测任务结束。
- 不自动安装 Agent、登录账号、改审批策略、写入真实宿主 Hook 或发送真实 Agent 任务。
- 不推送、不创建 PR、不发版、不改版本号。按 AGENTS.md 在当前合适的 codex/* 分支做小提交。

实施本任务书授权仓库内代码、文档、隔离测试与配置沙箱操作；真实宿主配置写入仍需要明确确认。应先完成可审阅的配置差异和验证工具，再提出具体确认请求。

## 3. 开工审计

先读取 AGENTS.md，执行 git status，保留所有已有修改。本文编写时存在未跟踪的 docs/REVIEW_HANDOFF.md，不要覆盖、删除或纳入自己的提交；执行时以现场状态为准。

重点阅读：

| 位置 | 要确认的事实 |
|---|---|
| `src-tauri/src/providers/mod.rs` | ProviderOutput 当前主要是快照，authoritative 的具体含义 |
| `src-tauri/src/providers/local_file.rs` | v1/v2 .status 兼容、身份、mtime、等待请求、旧 Hook 映射 |
| `src-tauri/src/claude_events.rs` | OS 文件锁、sequence、journal、恢复、请求关联 |
| `src-tauri/src/claude_integration.rs` | 显式安装、配置合并、备份、撤销、stdin 接收 |
| `src-tauri/src/main.rs` | Tauri 启动前已有 --claude-event 入口；命令注册 |
| `src-tauri/src/domain/task.rs` | TaskSource、TaskSnapshot、phase、turn、请求模型 |
| `src-tauri/src/monitoring.rs` | SourceKind、source ID、配置保存、scope、发现目录 |
| `src-tauri/src/watcher.rs`、`providers/worker.rs` | worker、快照合并、health、轮询热更新 |
| `store/`、`attention.rs`、`transition.rs`、`task_notice.rs` | 终态、去重、未读、请求恢复、通知策略 |
| `src/lib/types.ts`、`dashboard.ts`、`src/routes/manage/` | 来源配置、分组、管理 UI 和能力展示 |
| `scripts/verify.ps1`、`scripts/native-claude-smoke.ts`、`scripts/native-notice-smoke.ts` | 隔离构建、原生 IPC、真实提醒验收 |

输出简短基线记录：当前分支/commit、工作树状态、可复用模块、需最小扩展的接口、可用宿主及版本。发现产品不在本机不构成停止整个项目的理由。

## 4. 目标架构

```text
官方 Hook / 用户自建标准生产者
        ↓
接收入口 → 宿主字段映射 → schema 校验 → 最小化事件 → 本机 journal
                                                        ↓
                                               GenericEventProvider
                                                        ↓
                         带身份/顺序/证据的 observation → reducer
                                                        ↓
                    范围过滤 / Store / Attention / Transition / 通知
```

发现层只生成来源候选和能力提示，不直接生成“运行中的任务”。可读取已知安装元数据/文件路径；禁止自动运行从导入 manifest 获得的发现命令。

adapter、agent identity、source instance、transport、capabilities 分离。建议只增加一个通用 TaskSource / SourceKind，并以字符串 agent_id + 展示信息承载未来品牌；不要为四个品牌各复制一套枚举、Store 和通知代码。

所有新增持久化字段需要 serde 默认值/版本兼容。已有 ID、attention、recent、notifications 和配置不得失效。不要使用 DefaultHasher 等不保证跨版本稳定的算法作为持久化身份。

### 4.1 建议模块

```text
src-tauri/src/integrations/
  mod.rs
  schema.rs            对外协议与验证
  journal.rs           加锁写入、恢复、游标/检查点
  reducer.rs           纯状态归约与事件证据
  manifest.rs          声明式描述、版本与导入
  bridge.rs            stdin 接收与宿主载荷转换
  configuration.rs     配置预览/合并/撤销/冲突处理
  adapters/            仅放声明式映射无法表达的已核验逻辑
src-tauri/src/providers/generic_event.rs
integrations/           内置 manifest、schema、协议示例、说明
scripts/                契约测试和 Windows 原生 smoke
```

先旁路新增，保留 --claude-event 与旧 journal 行为；提取真正共用的 I/O 工具可行，但不得顺便改写 Claude 生命周期。正式迁移旧适配器是后续独立任务。

## 5. 协议 v1

区分“生产者输入”和“接收后日志 envelope”。source_instance_id 由已配置来源和命令参数绑定；received_at、journal sequence、映射版本和接收证据由 TaskPulse 写入，不能任由 stdin 自称可信。

### 5.1 建议字段

| 字段 | 约束 |
|---|---|
| schema_version | 第一版固定为 1；不支持的大版本拒绝，不静默按 v1 解释 |
| agent_id | 品牌标识，绑定来源配置；展示名单独设置 |
| session_id | 来源内部稳定身份；不得使用标题、PID 或当前时间替代 |
| turn_id | turn 相关生命周期事件必须可关联；宿主缺失时按明确起始事件派生并持久化，记录关联局限 |
| event_id | 生产者稳定事件 ID 优先；重复投递必须保留同一 ID |
| kind | 枚举事件类型，不允许任意字符串直接变生命周期 |
| occurred_at | UTC epoch 毫秒；允许缺省并标明未知；不能仅靠时钟决定覆盖关系 |
| request_id / call_id | 对应输入请求或工具调用的稳定关联键 |
| payload | 受类型约束，只保留对应事件允许的最小字段 |
| envelope.received_at / sequence / journal_epoch | 接收时间、会话内写入次序、日志世代 |
| envelope.evidence / adapter_version | 由接入渠道与验证逻辑决定 |

event_id 去重、journal sequence 排序和宿主因果顺序是三件事。锁产生的 sequence 仅表示接收顺序；迟到的旧 turn 事件不得覆盖新 turn。宿主没有稳定事件 ID 时，不要靠完整 JSON 哈希合并两个内容相同但实际不同的请求；说明投递去重的边界。

原始 ID 可以含 Unicode/冒号等合法字符，但不得原样拼接文件路径；使用受控编码或稳定摘要作为文件名，保留原 ID 于内容。配置目录与事件目录的写入目标必须限定在选定的受控目录。

### 5.2 事件与状态语义

| 事件 | 行为 |
|---|---|
| session.discovered / session.renamed | 更新目录或展示；不自动成为关注任务，不通知 |
| turn.started | 新执行证据，RUNNING；轮次关联和旧请求收束遵循已验证宿主语义 |
| phase.changed | 思考/工具/回答/模型等待；不能清除未解决输入请求 |
| tool.started / tool.finished | 维护 call ID 集合；只结束匹配工具活动 |
| input.requested | 显式 approval/question，记录 request ID；可按既有策略提醒 |
| input.resolved | 只解除匹配请求；有其他请求时继续等待 |
| turn.stop_requested | 仅表示准备停止，不通知完成 |
| turn.completed | 确认本轮结束，进入结果/未读；不声称用户目标或产出必然正确 |
| turn.failed | 显式轮次失败；按现有失败策略处理 |
| turn.interrupted | 明确中断；保留取消/中断原因，不伪装成功；与现有状态兼容映射需文档化 |
| session.closed | 会话关闭，不自动推出本轮成功；按明确结束原因处理 |
| heartbeat | 来源存活；不延长执行活跃期，不证明待审批仍然有效 |

已终结 turn 收到迟到 phase/tool 事件不能复活；新执行必须有新的稳定轮次证据。同一 turn 的状态反复不重置通知 dedup。

没有原生 request ID 的宿主，可为已观察的显式请求派生本地 ID，但解除必须有唯一可验证关联。关联不清楚时保留请求不确定性，不用“最近一个请求”猜测。

### 5.3 Stop 的硬约束

Kimi/TRAE/ZCode 的 Stop 可能在其他 Hook 阻断后继续执行。不能把名称 Stop 自动映射为 turn.completed，也不能延迟几秒后因“没有新日志”推断完成。

逐产品寻找最终结束事件、最终结构化状态或可靠原生记录。若当前版本无法获得，只支持 stop_requested 并把“本轮最终结束”标为不支持/待验证。一个基础 Hook 适配器可部分可用，不得为宣传完整能力而伪造终态。

同理，UserPromptSubmit 可能被其他 Hook 拒绝，SessionStart 不等于新执行；优先使用实际 TurnStarted 或明确执行证据。

## 6. Journal、持久化与通知通路

建议按 source/session 分 journal，用户目录下使用独立子目录，例如：

```text
~/.taskpulse/integrations/<source-id>/events/<session-key>.jsonl
~/.taskpulse/integrations/<source-id>/state/<session-key>.json
~/.taskpulse/integrations/<source-id>/checkpoints/...
```

最终布局应避免 local_file、通用 watcher 和 notification-log 互相触发。

必须实现：

1. 多进程写入使用 OS 锁，锁等待有上限；单次输入、单行、文件读取和每轮批量大小都有明确上限。
2. 半行尾部恢复；完整坏行有有界诊断；截断、替换、轮转通过文件身份/世代处理，不能只记 byte offset。
3. journal 是事件事实，latest/checkpoint 是加速恢复；不能以覆盖快照替代事件历史。
4. reducer state 与已处理游标原子保存或采用明确可重放协议；先移动游标再保存状态会丢事件，必须规避。
5. 通知继续走 TransitionEngine 和 notifications.json。禁止桥接程序直接弹窗。
6. 同一 poll 内 `start A → complete A → start B` 不能只留下 B 而漏 A 的完成事实。
7. 若现有 ProviderOutput 无法保留第 6 条，最小扩展为有序 observation/batch，并在统一流水线内按顺序处理；最终 UI 仍输出最新快照。不要绕过 scope、Attention 或 dedup。
8. 重启从 journal 恢复未读及 pending 状态；已处理事件不重复通知。离线补收的终态按既有恢复策略进入持久关注，不批量回放过期弹窗。
9. 状态持久化与通知投递之间的崩溃边界必须测试并说明。不要声称操作系统通知具有严格 exactly-once；以可恢复关注状态和持久去重作为保证。
10. 写盘失败、解析失败、积压、锁超时只导致来源降级，不能生成任务成功/失败；有部分结果时不可把其他会话 reconcile 掉。
11. health 区分已配置、未收到事件、已连接、数据陈旧、解析失败；无心跳能力的安静来源不能仅因静默判为故障/完成。
12. 对手动重开批准弹窗，只有新鲜、匹配请求的明确 pending 观察才可授权；重读旧日志或普通 session heartbeat 不构成这种证据。
13. 设置日志保留/轮转规则与磁盘上限；未归约事件、活动任务和未解决请求不能静默丢弃。达到上限要报来源降级并保留恢复方法。

性能验证使用临时数据：至少 20 个并发写入进程和 10,000 条混合事件，检查不丢不重、批次让出与 UI 响应。给出机器环境、P50/P95 和峰值资源占用，不虚构数字。超时和批量参数用测量调整。

## 7. 接收程序与 Windows 分发

第一选择：复用现有 exe 的 Tauri 初始化前入口，新增形如：

```text
agent-tray.exe --integration-event --source <id> --adapter <id>
```

此命令是待实现设计，不是当前可用功能。标准化 stdin 入口与宿主原始载荷转换模式需要明确区分。

- 接收模式不初始化 WebView、不激活窗口、不弹托盘、不启动 watcher。
- 正常 Hook 模式不输出 stdout，不返回审批决定，不注入上下文；失败只写有界脱敏诊断，并按宿主约定非阻断退出。
- 单独的 validate/diagnose 模式必须提供机器可读结果和真实错误退出码；不能因 Hook fail-open 而让测试误报成功。
- 有界读 stdin、超时获取锁，磁盘损坏/配置缺失也不能长期阻塞宿主。
- 真正的 Windows release GUI-subsystem exe 必须用 stdin 重定向验收，不能仅测 debug console binary。
- 若复用 exe 无法可靠接收，再选择独立小型 Rust bridge；同时补 Bun/CI/installer 的构建、分发、路径和升级测试，不能留下只在仓库能运行的脚本依赖。
- 普通用户不应额外安装 Python/Node 才能使用内置 Hook 接收；现有 tp-run 的 Python 要求不在此次重写范围内。
- 配置中的 bridge 路径必须指向持久安装路径，不能使用 codex-check 等临时测试位置。
- 多种宿主可能使用 Git Bash、PowerShell 或直接 exec，按各自文档产生正确引用；用带空格、中文、引号和 shell 元字符的路径测试，不把事件字段插进 shell 命令。

## 8. 声明式 manifest 与能力模型

manifest 最少包含 manifest_version、adapter_id、agent_id、display_name、适用产品形态/系统/版本、官方来源、field mapping 和 capabilities。宿主版本未知时不得自动宣称兼容。

第一版映射只支持受限 JSON 字段路径、常量、事件枚举映射和必要的受限条件；不允许 JS/Python eval、任意命令、网络下载、动态库或任意目标配置写入。

允许用户导入 manifest 后创建标准事件来源：外部生产者发送兼容 JSON，即可生成任务。复杂宿主逻辑通过经审查的转换器扩展；不要为支持任意宿主发明通用编程语言。

官方配置安装与事件映射分开：

- 内置、核验过的配置 recipe 可自动生成差异供确认。
- 任意导入 manifest 默认只能用于映射/标准接入及导出说明，不能凭文件内容获得任意写配置或执行权限。
- 支持版本范围不匹配时降级/拒绝对应能力；manifest 更新保留来源身份和已确认 scope。
- 品牌图标第一版用内置通用图标或文字，不加载任意远程图标。

能力按项记录：session discovery、title、project、turn start、phase、final end、failure、approval request、approval resolution、question、focus。

每项包含：支持程度（explicit/inferred/unsupported）、证据通道、已测试版本、验证状态（documented/fixture-tested/native-tested）、限制。事件存在不自动证明全部关联字段存在。

配置来源不等于身份验证。普通本地生产者的 payload 不能自称“官方/精确”；通过任意自报标准事件接入的来源应保持相应证据标识。

## 9. 用户流程与配置安全

来源管理提供：

1. “添加 Agent”：内置产品和“其他 Agent”。
2. 选择当前支持的环境及项目范围；默认只监控新执行。
3. 显示实际能力，例如“支持开始/工具阶段，最终结束待验证”。
4. 选择内置 Hook 接入、导入描述文件、标准事件接入，或说明已有命令监控入口。
5. 预览：配置文件位置、将新增/删除的 Hook 条目、重载要求、如何撤销。不得展示凭据或整份带秘密的配置。
6. 用户明确确认后才写宿主配置；确认令牌绑定预览版本/文件指纹，防止预览后文件变化仍覆盖。
7. 连接测试与真实任务验证分开。连接测试写专门 test 通道，不作为监控任务、不制造成功历史；测试提示明确标注。
8. 诊断显示最近接收时间、已识别事件、丢弃原因、来源健康、各能力验证程度。
9. 撤销仅移除 TaskPulse 自有条目，保留用户后来添加的 Hook、模型、代理、statusLine 和其他未知字段。

安装/撤销要幂等，写入使用同目录原子替换并保存一次性原始备份。不要把备份整体覆盖当常规撤销，否则会丢用户后续修改。

所有权标记必须使用宿主允许的字段，或 TaskPulse 自己维护的登记信息/命令标识；不要在严格 schema 中添加自造 tag。JSON/TOML/JSONC 以宿主实际格式处理；不支持的格式应提供清晰错误并不修改原文件。

“停用来源”和“移除 Hook”是不同动作：停用不抹去未读/固定状态；重新启用不补播停用期间旧通知。写清事件水位策略，避免恢复时把历史结束当新执行。

跳转只使用已验证的终端/原生定位机制，不运行 manifest 或事件提供的任意命令/URL。

## 10. 四款 Agent 的实施要求

以下是 2026-10-20 查到的官方入口，不是本项目已支持声明。执行时重新核对产品形态、版本、平台与样本载荷；文档没有的字段不要猜。

### Kimi Code CLI：第一个端到端试点

[官方 Hooks](https://www.kimi.com/code/docs/kimi-code-cli/customization/hooks.html) 提供实际 turn 开始、审批请求/结果等事件，并区分准备停止和中断。先核查各事件实际携带的 session/turn/request/call 字段，再实施字段映射；配置仅写官方允许的字段。

重点验收：提交被拒绝、消息排队、新 turn、并发工具、多审批、审批解除、中断、Stop 后继续、宿主重启。最终结束如果不能确认，必须限制能力，不用静默补全。

CLI、Desktop、VS Code 扩展分别记录，不能把 CLI 通过推广到其他形态。

### TRAE 国内 IDE

[官方 Hook 配置](https://docs.trae.cn/ide_hook-configuration-reference) 可作为接入依据。核验具体个人/企业版本的配置和事件差异；海外 IDE、独立 SOLO/Work 不自动纳入。

重点检查 SessionStart、UserPromptSubmit、工具事件、Stop、Notification 的真实语义；Notification 只有结构化明确输入请求才可作为 blocker，不解析普通文案或 idle 通知推断审批。

### ZCode

[官方 Hooks](https://zcode.z.ai/cn/docs/hooks) 提供会话、提交、工具、权限和准备停止入口。核查它自己的载荷和配置，不因事件名类似 Claude 就复用全部 Claude 逻辑。

重点检查权限解除依据、并行调用关联、Stop 被其他 Hook 继续、桌面与 CLI 版本关系。

### WorkBuddy

[官方插件系统](https://www.codebuddy.cn/docs/workbuddy/Plugins) 明确列出 Hook 插件，但这本身不足以证明桌面宿主支持完整 CLI 事件。

先验证桌面版本实际支持的插件格式、配置位置和载荷。CodeBuddy CLI 文档只能作为查证线索，不能作为 WorkBuddy 全能力的证据。

若无法核验：交付候选 manifest/安装说明草案及缺失证据列表，不开放“一键完整接入”或虚构状态。已有可靠能力可部分上线。

### 共同证据要求

每个适配器维护一张矩阵：产品/版本/OS、官方链接、原始事件名称、最小脱敏载荷、映射结果、关联限制、fixture 测试、实机证据日期、未支持项。

原始用户 prompt、工具参数、命令输出和凭据不能进入仓库 fixture 或通知日志。测试构造明确标记的合成输入；实机证据只保留最小字段和脱敏身份。

## 11. 分阶段执行顺序

| 阶段 | 工作包 | 交付物与退出条件 |
|---|---|---|
| P0 | 审计与契约设计 | 基线、能力调查、协议 schema、状态语义、崩溃/水位策略；找出现有快照通路的限制 |
| P1 | 事件接收、journal、reducer | 隔离 CLI 可以写入/重放；并发、半行、乱序、pending、大小限制测试通过 |
| P2 | GenericEventProvider 接入 | 多来源与 scope 正确；短任务、多轮同 poll 不丢；未读/去重/重启正确 |
| P3 | manifest 与来源管理 | 导入新品牌无需 Rust 修改；预览/确认/诊断/撤销跑通；真实 WebView IPC 通过 |
| P4 | 四款宿主适配 | Kimi 优先闭环，其余按核验能力实施；每款给代码/fixture/native 三层结果 |
| P5 | 回归与交付 | 完整检查、原生烟测、文档、兼容矩阵、已知限制和提交记录 |

按依赖顺序持续执行，不在每一阶段机械询问是否继续。确实需要用户提供真实宿主授权/样本时，先完成可独立实施部分，给出具体所需动作。不要未经请求启动多个子 Agent；本任务书不要求多 Agent 并行。

建议提交粒度：协议/reducer；journal/bridge；Provider/流水线；manifest/配置；管理 UI；逐产品适配；验收/文档。提交前排除无关修改和真实用户数据。

## 12. 必须覆盖的验收场景

测试应验证用户可见行为和故障边界，避免只复制函数实现。

| 场景 | 必须结果 |
|---|---|
| 同一 session 改标题 | ID、未读、固定、turn、通知 dedup 不重置 |
| 同一 session 新 turn | 同一任务进入新执行，不生成标题派生身份 |
| 两个来源有相同 session ID | 完全隔离 |
| begin/end 都在两次 poll 之间 | 结果可见且按规则通知，不漏任务 |
| A 完成、B 开始在同一批 | A 事件被处理，最新快照为 B；弹窗动作带对应轮次，不能误作用于 B |
| 重复 start/end、状态反复、重启重读 | 同一生命周期不重复通知 |
| 旧 turn 结束晚于新 turn 到达 | 旧事件不覆盖新状态、不误关新请求 |
| 两个同时 pending 请求，解除一个 | 剩余请求仍可见且不误清 |
| 缺失/歧义 request-call 关联 | 不猜测解除，不伪造精确关联 |
| 工具完成/失败、模型等待、低活动 | 不推断整轮成功、失败或审批 |
| Stop 后其他 Hook 要求继续 | 不产生错误完成通知 |
| 上报失败、journal 部分损坏、日志截断 | 来源降级；不把缺失任务当完成 |
| 锁争用、多进程、半行、轮转 | 事件完整；归约与游标可恢复 |
| 重启/进程在关键持久化点退出 | 未读/pending 可恢复，已处理通知不重播 |
| 心跳、改名、poll | 不延长执行活动窗口 |
| 首次接入、停用再启用、旧日志 | 不批量导入/弹出旧历史 |
| 全局通知关闭及 popup/system/both | 遵循现有开关与单次投递策略 |
| 关闭提醒窗口 | 不标记任务已读；旧 timer 不能关闭下个提醒 |
| 旧 pending 日志 + 新 session heartbeat | 不允许作为“请求仍有效”恢复弹窗 |
| 未知 schema、超大载荷、路径穿越 | 拒绝并有界诊断；无越界写入 |
| manifest 自带任意命令/配置路径 | 不自动执行、不自动写入 |
| Hook 写盘失败/超时 | 不改变宿主批准结果、不阻塞工作 |
| 预览后用户编辑配置 | 检测冲突，要求重新生成预览 |
| 重复安装/卸载；未知字段/其他 Hook | 不重复、不误删；保留用户配置 |
| 测试事件 | 明确测试反馈；不进入真实任务/未读历史 |
| 中文/空格路径与 Windows release exe | stdin 正常、无额外窗口、原生 IPC 可用 |
| 旧 `.status` / --claude-event / 内置 Provider | 原有 fixture 和回归检查仍通过 |

## 13. 构建和实机验证

先跑相关 Rust 单元/契约测试和 UI 模型测试。完成后按 AGENTS.md 跑全量检查。托盘运行时使用：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend
```

该脚本当前使用隔离 target，并先构建 frontend 再嵌入 Rust；执行时确认脚本内容。另跑 Bun 全量 advisory 报告，保留低级别结果，不把阈值通过写成零风险：

```powershell
& "$env:LOCALAPPDATA\hermes\node\bun.cmd" audit
```

新加独立 bin、schema validator 或测试脚本时必须加入检查入口，不能以现有脚本通过替代新目标验证。成为候选发布前执行 audit:rust 和 audit:secrets；本任务不授权发布。

新增例如 `scripts/native-integration-smoke.ts`：

- 使用隔离 TaskPulse home/config/journal 和临时宿主配置；所有状态路径均要确认没有回落到真实用户目录。
- 真正启动原生应用，通过 WebView2 CDP/IPC 完成添加来源、能力展示、配置预览、测试事件、状态变化和撤销。
- 补充 release bridge 子进程 stdin、退出码、锁超时和无窗口检查。
- 测试提醒窗口的点击、焦点、不激活行为，复用现有 native-notice 验收设施。
- 测试结束关闭自己启动的准确路径/PID，不按名称杀用户进程；保留脱敏失败证据。

真实宿主验收另列 opt-in 步骤，先展示配置差异，用户确认后执行；完成后按用户选择保留接入或只撤销本次条目。缺少某款宿主就如实记待验收，不安装软件或伪造成功。

第一版不改远程/provider wrapper 时无需运行无关真实 SSH/WSL 工作负载；如果实际修改相关逻辑，补 AGENTS.md 对应测试。

## 14. 最终交付文件与报告

建议新增/更新：

- `docs/AGENT_INTEGRATION_PROTOCOL.md`：正式实现后的字段、语义、兼容与错误规则。
- `docs/AGENT_INTEGRATION_GUIDE.md`：普通用户接入、诊断、停用、撤销，以及命令级监控限制。
- `docs/AGENT_COMPATIBILITY.md`：具体产品/版本/平台/能力和验证等级。
- `docs/AGENT_INTEGRATION_VALIDATION.md`：命令、退出结果、实机证据、限制和未完成项。
- `integrations/` 下 schema、内置描述文件、合成示例、社区贡献模板。
- 必要时更新 ARCHITECTURE、PRODUCT_DASHBOARD、ATTENTION_LIST 与 scripts README。

最终报告必须回答：

1. 用户现在如何接入一个未内置品牌？给出经过验证的操作路径。
2. 不升级 TaskPulse 能增加哪类品牌？哪些特殊格式仍需代码适配？
3. 四款 Agent 各自已做到什么、在哪个实际版本验证、哪些状态不支持？
4. 如何避免 Stop 误报、并发审批误清、短任务漏报和重启重复通知？
5. 执行了哪些自动/原生检查，哪些通过，哪些因环境未做？
6. 哪些真实用户配置被改动、如何撤销？没有改动时明确说明。
7. 提交列表、已知风险、剩余验收动作；不把尚未验证的适配写成完整支持。

最重要的交付标准：通用接入可以真实运行；状态和通知有证据；未适配/部分适配有清晰入口和能力说明；既有任务生命周期不回退。

## 15. 可直接发给 GPT-6.1 的启动指令

```text
请在 D:\Project\agenttray 中执行 docs/AGENT_INTEGRATION_EXECUTION_PLAN.md。

先读取 AGENTS.md 和任务书，检查当前工作树，保留所有无关修改。
本次是实现任务，不要只返回设计或建议。

按 P0–P5 推进，完成通用事件协议、可分发 bridge、可靠 journal/reducer、
GenericEventProvider、来源管理和声明式适配入口，并逐款实施 Kimi Code CLI、
TRAE 国内 IDE、ZCode、WorkBuddy 有证据支持的能力。

不要把官方文档/fixture 通过当成实机通过；不要把 Stop 直接当最终结束；
不要用静默、CPU、工具返回推断 Agent turn 结束或等待批准。
保留旧 Provider、.status、Claude Hook、未读/固定/通知去重与范围配置行为。

仓库内实现和隔离测试可持续自主执行。真实宿主 Hook 写入需要先给出具体
配置差异并得到明确确认；不自动安装 Agent 或改变审批策略。
若某款宿主不可用，完成独立工作并交付待验证适配与明确的验收缺口。
不要 push、开 PR、发布或改版本；按 AGENTS.md 做小逻辑提交。

完成后交付实现、文档、兼容矩阵、自动测试和 Windows 原生验收证据，
清楚列出已完成、部分支持、未实机验证及用户需要执行的最后步骤。
```
