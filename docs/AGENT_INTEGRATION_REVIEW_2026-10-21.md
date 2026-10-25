# 通用 Agent 接入实施审查与下一步

审查日期：2026-10-21。审查 HEAD：`bf55e8f`；主要实施范围：`ac18c78..bf55e8f`。对照 `AGENT_INTEGRATION_EXECUTION_PLAN.md`，结合代码、自动测试、原生 TaskPulse 验收和官方契约审查。

结论：通用接入基础设施和管理入口已经落地，架构方向符合预期；当前仍属于部分实现/待验收状态，不能判定 P0–P5 全部完成，也不适合宣传四款 Agent 已支持。发现两个优先修复的生命周期/通知问题、两个恢复/撤销问题。四款真实宿主均未验收，文档对此披露准确。

本次未修改生产实现、真实宿主配置或审批策略，未提交、推送或发布。为复现增加过临时测试入口，已恢复原文件并核对 SHA-256 和 git diff；复现源码留在 ignored 的 `src-tauri/target/integration-review-probes.rs`。本报告是本次新增的审查交付。

## 1. Findings

### R1 · P1：静默重放会吞掉同批其他来源的正常通知

位置：`src-tauri/src/watcher.rs:1346`、`:1347`。

`engine.apply_unflushed(&tracked, &hosts, ...)` 对完整任务/主机视图做转换，并修改去重账本。随后 `if observation.notify` 却按当前一条通用 observation 的标志，决定是否保留整个返回事件列表。

因此通用来源正在静默恢复（notify=false）时，如果合并视图中另一个来源刚有真实结束/失败/等待输入，其他来源事件也被丢弃；账本已经消费，后面的 `apply_snapshots` 不会再发出该事件。实际可发生于远程结果在同一 watcher 循环先进入 Store、随后消费恢复中的通用 outbox。

复现：用真实 TransitionEngine 建立 Codex RUNNING，改为明确 TurnEnded；将其与通用 observation 一起输入，执行 watcher 当前的 notify=false 分支，再调用 apply_snapshots。第一次返回了 Codex WaitingInput/TurnEnded 事件但被丢弃，后续返回空；期望 Codex 仍收到一次提醒的断言失败。

这是对实际 engine + watcher 控制流的定向复现，不是声称已经用真实 Codex 会话端到端触发。

修复要求：

- 静默策略只能作用于当前需静默的 observation 对应事件，不能作用于其他任务/来源/主机。
- 进一步检查同批多个通用来源：新鲜来源不能使另一来源的旧快照提前产生提醒，旧来源也不能压制新鲜事件。
- 保留完整视图对 engine 的要求；不要简单只传单个任务而造成其他任务 prev 状态丢失。
- 增加混合原生/通用/主机的永久回归用例，覆盖静默恢复、离线补收、通知去重与持久化重试。

### R2 · P1：TRAE 延迟到达的旧轮次完成通知会结束新轮次

位置：`integrations/trae-cn-ide.json` 的 idle_prompt 映射；`src-tauri/src/integrations/manifest.rs:374`。

TRAE 的完成通知映射为 turn.completed；载荷没有 turn_id 时统一回退为 `ctx.turn`，也就是最后观察到的当前轮次。

[TRAE 官方文档](https://docs.trae.cn/ide_hook-configuration-reference)说明 Notification 异步执行，idle_prompt 表示任务完成，并且不携带工具调用 ID。因此“确实发生了结束”不等于“这条消息属于当前轮次”；不能只凭接收顺序附会到最新 turn。

复现输入顺序：

```text
A: UserPromptSubmit → PreToolUse(call-a) → PostToolUse(call-a) → Stop
B: UserPromptSubmit → PreToolUse(call-b)
A 的异步 Notification(idle_prompt) 此时才到达
```

真实 manifest mapper + reducer 的结果：B 的 turn 被设置 finished_at，状态成为 WaitingInput/TurnEnded，而 B 的工具仍在执行。期望 B 保持 RUNNING 的断言失败。

修复要求：

- 必须获得可验证的轮次关联：例如宿主可靠 query ID、可读取的结构化最终状态/记录；不能靠延迟几秒、到达顺序、标题或最后一个 call 猜。
- 在缺少可验证关联时，将该事件视为未关联结束事实/不确定观察，不修改新轮次，不解除新轮次的 pending，不发送该轮完成提醒。
- 无法解决前将 final_end 能力降级，删除“已有关联执行就足够”的能力承诺。
- 对缺少原生 turn ID 的其他适配器一起检查迟到权限/结果事件，不仅修 fixture 的特定 idle 字符串。

### R3 · P2：一个已接收的超限请求会让后续解除/结束记录永久无法消费

位置：`src-tauri/src/integrations/reducer.rs:44`、`:49`；`src-tauri/src/providers/generic_event.rs:183`、`:199`。

标准接收端能将第 129 个 pending 请求写入 journal。reducer 达到 128 上限时报错，Provider 使用 `?` 直接返回，没有推进或隔离这条记录。以后每次读取同一段都会再次失败，排在其后的 input.resolved/turn.completed 永远执行不到。

复现：真实 journal 写入 start + 129 requests + 一条 resolve + completed，通过 GenericEventProvider 读取并 ACK 成功批次。第一轮输出128条 observation；随后连续4轮都是 `session_state_limit_export_before_reset`、0条 observation，结束一直无法消费。

现有 `pending_capacity_does_not_prevent_resolution_or_explicit_end` 只测“正好128个请求后直接解除”，没有测试“超限记录已进入日志”。所以原测试通过没有覆盖这个问题。

错误还会从 session 处理直接上抛至整个 Provider；需要检查故障 session 是否阻塞其他健康 session 的交付。seen 总量上限也无条件拒绝后续事件，应一起设计可恢复边界。

修复要求：

- 在接收/归约边界设计一致的容量策略。优先阻止不可处理的新请求进入正常队列，同时为解除、明确终止等恢复事件保留处理通道。
- 对已经持久化的超限/语义错误记录提供有界隔离与明确诊断，不可静默丢弃证据或要求用户删除整个历史才能继续。
- 一个 session 的问题不应停止其余 session 的消费。
- 增加129请求、后续解除/结束、旁路健康 session，以及重启后的恢复测试。

### R4 · P2：删除来源后，正常 UI 无法再撤销残留宿主 Hook

位置：`src/routes/manage/+page.svelte:60`、`:183`；`src-tauri/src/integrations/configuration.rs:223`；`src-tauri/src/integrations/bridge.rs:31`。

来源移除直接删除 registry 记录；已经安装到宿主的 Hook 和 ownership 文件仍保留。撤销 prepare 又必须先通过 configured_source 查到来源，删除后返回 source_not_configured。UI 新建通用来源生成新 UUID，也不会自动恢复旧来源的撤销入口。

后果：用户按“移除此监控来源”操作后，宿主继续调用失效桥接命令，用户不能再通过普通撤销流程清理；需要手工编辑宿主配置或恢复原 registry。此项为静态路径审查，未改真实宿主配置来触发。

修复要求：

- 对存在自有 Hook 的来源，删除前提供“先撤销 Hook 再删除”预览/确认流程；或保留独立于活动 registry 的接入登记和可用撤销入口。
- 停用、删除、撤销必须在 UI 上区分，并告知残留 Hook 状态。
- 删除不应自动绕过确认去修改宿主配置。
- 验收安装→删除→撤销/恢复，确认只删除自有条目，保留后来添加的外部配置。

## 2. 对原执行任务书的进度判断

| 工作包 | 当前判断 | 证据与缺口 |
|---|---|---|
| P0 协议与设计 | 基本完成 | 独立 schema、能力/证据分层、范围边界明确 |
| P1 bridge/journal/reducer | 已实现，需要修复恢复边界 | OS 锁、原子状态、有界 stdin、实际 exe 输入测试通过；R3未解决 |
| P2 统一流水线 | 已贯通，尚不能验收完成 | ordered outbox、未读、去重存在；R1涉及既有来源回归，R2会误结束 |
| P3 manifest/管理入口 | 主要流程完成 | 导入品牌无需 Rust 分支、确认/冲突/撤销沙箱通过；R4遗漏完整删除生命周期 |
| P4 四款适配 | 部分实现，未真实验收 | Kimi/TRAE/ZCode有受限映射；WorkBuddy只有候选。四款native-tested均无 |
| P5 验证交付 | 平台检查较完整，产品验收未闭环 | 自动和TaskPulse原生测试通过；缺真实宿主、外部焦点、长期运行，且新增复现失败 |

保留现有分层、沿用 app exe 前置桥接、独立 source identity、标准事件自报标签、Stop不直接映射结束、测试不制造监控任务，这些符合预期。不需要推翻实现重做。

## 3. 当前用户究竟可以做什么

- 自己有可靠事件生产者：可以通过标准协议接入，平台闭环已经具备；仍须修复上述错误后再作为普遍可用功能推广。
- Kimi Code CLI：有开始、工具、请求、失败等受限映射；正常最终结束明确不支持，真实字段未验收。因此不能完成“正常结束后提醒我”的核心体验。
- TRAE国内IDE：有较多文档依据及配置入口；当前结束关联存在R2，不能视为可靠完成监控。
- ZCode：有工具/请求的受限映射；正常最终结束、无工具新轮次等仍缺失。
- WorkBuddy：只有候选和限制说明；用户目前仍须自建标准生产者，不能称为原生适配已完成。

未知终态不能为了补齐体验用静默/CPU计时推断。产品应该说明“最后确认执行中，结束能力受限”，避免用户把长期保留的 RUNNING 当作当前执行的强保证。

## 4. 本次独立执行的验证

### 重新运行并通过

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend -TargetDir src-tauri/target
```

运行前确认该目标没有 live agent-tray.exe，未替换正在运行的应用。

- Rust：361 passed，0 failed，10 ignored。
- Svelte：0 errors，0 warnings；前端 build成功。
- UI模型20、release8、security6、integration contract3项通过。
- cargo fmt、clippy -D warnings、开发构建、实际 Windows bridge stdin/validator/fail-open/锁超时检查通过。
- bridge新证据：`src-tauri/target/integration-bridge-ErRi67/result.json`。

另外重新执行优化 GUI exe 的原生验收：

```powershell
bun scripts/native-integration-smoke.ts --exe src-tauri/target/ci-release/agent-tray.exe
```

8项检查通过，包含真实 WebView2/IPC、来源创建、配置差异确认、Unicode shell转发、独立请求、社区导入、短任务及重启恢复。证据：`src-tauri/target/integration-native-i0b6DR/result.json`。

本次使用现有优化exe，SHA-256为 `EF03EC779C3BFD8CF2B6A46F4D6EF6A0649E9932F3BFC32DECF34184B977F66C`，与6.1报告一致；没有重新构建MSVC安装器。此轮未带 --notices / --stress-worker，不重复宣称验证了通知焦点或完整压测。

### 新增定向复现

临时测试真实 mapper/reducer、journal/Provider、TransitionEngine及watcher筛选控制流，结果 **0 passed / 3 failed**，分别对应R1/R2/R3。它们表达正确行为的断言失败，确认原测试集存在覆盖缺口。

测试入口已还原，`git diff` 确认无生产/测试源改动；备份与恢复后的tests.rs SHA-256相同：`E1543F55D78D0AB08DBC03CE289133952406F4F14F80D4C8AB47AEB265237680`。

### 核查了既有证据，但未重新执行

- 6.1原生记录 `integration-native-O9FAHe/result.json` 确有20进程/10000事件及WebView响应指标，不是只有口头描述。
- 既有notice报告 `focusPreserved=null`，明确未验证外部编辑器焦点；不能将整体passed读成焦点通过。
- 四款真实宿主的realHostsVerified均为空，文档和产物一致。
- 本次Bun完整audit仍有1条low（cookie）；moderate门禁通过不表示零告警。
- 未重跑Rust advisory、全历史secret扫描、远端/WSL live或安装器CI；不替它们追加新的通过结论。

## 5. 下一轮执行顺序

### 第一批：修复审查缺陷，形成独立回归

1. 先修R1，避免新增通用来源影响已有Codex/其他来源提醒。
2. 修R2；暂时无法得到可靠轮次关联时先降级TRAE final_end，不等宿主试验后才阻止误报。
3. 修R3的日志可恢复消费与session故障隔离。
4. 修R4删除/撤销闭环。
5. 将临时复现变成正式行为测试，并增加异步乱序和混合来源的原生/集成用例；原361项及全量检查继续通过。

不要以关闭整个通用通知、重置notifications.json、清空journal/attention或增大计时阈值绕过问题。

### 第二批：完成至少一款真实宿主的端到端接入

根据已有安装证据优先考虑TRAE国内IDE，其次ZCode；若用户提供Kimi CLI环境也可优先Kimi。先展示具体Hook差异，经明确授权后进行。

记录宿主精确版本、OS、最小载荷、真实session/turn/request/call关联，以及：无工具回复、有工具执行、两个审批、只解除一个、Stop后继续、延迟旧事件、新轮次、明确失败/中断、TaskPulse重启。

宿主没有正常最终结束事件时，调查官方只读最终状态/日志接口；不能补上推断完成。目标是至少一款产品真正完成“开始→阻塞/解除→结束→提醒→已查看”的用户闭环，然后再扩其他品牌。

WorkBuddy需独立调查桌面插件/Hook契约，不能使用CodeBuddy CLI映射冒充完成；缺证据就继续保持候选。

### 第三批：面向普通用户的交付

- 在解锁桌面验证提醒不抢外部编辑器焦点、实际点击、来源失联后的提示。
- 验证持久安装路径、升级后Hook路径、移除来源、卸载/重新安装，不依赖target里的开发exe。
- 做长时间运行和日志配额恢复验收；现有“达到上限后人工新建来源”不能等同于自动轮转完成。
- 候选发布前按AGENTS.md跑完整审计与CI门禁。更新兼容矩阵，只把实机通过的具体能力/版本升级为native-tested。

## 6. 可直接交给 GPT-6.1 的后续指令

```text
请读取 AGENTS.md、docs/AGENT_INTEGRATION_EXECUTION_PLAN.md 和
docs/AGENT_INTEGRATION_REVIEW_2026-10-21.md，在当前实现上继续修复。

先处理审查R1–R4：混合来源通知被静默重放吞掉、TRAE异步结束归错turn、
超限journal记录阻塞后续解除/结束、移除来源后残留Hook无法撤销。
先补可复现的正式回归，再修实现，不要只修改兼容说明来掩盖代码缺陷。
临时复现源码在 src-tauri/target/integration-review-probes.rs；若已被清理，
按报告中的完整事件序列重建。R1用例还需补真实流水线的混合来源测试。

保持既有Provider和持久状态，禁止清空用户状态、重置通知账本、用计时猜结束。
TRAE若无法可靠关联终态，必须先降级相应能力而不是继续关联到最新turn。

修复后跑全量验证和原生smoke，报告每条finding的验证结果。
随后准备一款真实宿主的配置差异和验收步骤；写真实Hook前取得明确确认。
没有宿主实测证据时仍保持fixture-tested，不宣称四款适配完成。
不push、不创建PR、不发版；保留所有无关工作树变更。
```
