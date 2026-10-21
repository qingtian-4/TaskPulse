# 可靠性与资源优化（2026-10-21）

基线：46127fd，Alpha.7 候选，codex/desktop-discovery-visibility。按 F01→F02→F03→F04→DSH→诊断执行。保留用户已有 profile.dev.debug=0 修改和三个未跟踪文档，不将其纳入本轮提交。不安装真实 Hook、不改 Agent 配置、不删除缓存/用户状态、不 push/PR/tag/发布。

## F01：接收通道与事实缺口分离

- 根因：last-error 与整个来源 max received 比较，B 的成功/心跳可使 A 的丢失事实变为健康；重启也缺少独立缺口证据。
- 修改：integrations/health.rs、bridge.rs、providers/generic_event.rs；接收失败只保存哈希 session/turn/request/call、事件类型和时间，不保存载荷。每会话/未知关联缺口独立持久化；checkpoint 保存已恢复的缺口身份。旧 source-only 操作错误保守迁移，普通协议验证错误不污染健康。
- 恢复：匹配请求/工具解除或明确当前轮终态可恢复对应缺口；可信新 turn 可独立建立新基线。心跳、其他会话成功和普通 phase 不恢复。未知关联缺口对旧任务保守保留，不阻止其后新轮次；健康 peer 仍保持新鲜。
- 测试：f01_peer_success_and_heartbeat_cannot_repair_a_lost_resolution 先失败再通过；f01_scoped_gap_requires_matching_recovery_and_survives_restart 覆盖跨会话、解除、缺失结束、心跳、新 turn、重启、普通验证错误。现有 receiver_write_failures 回归同步为心跳不恢复规则。
- 兼容/回滚：新增 optional checkpoint 字段及独立 gaps 文件；无状态清空。回滚旧程序会失去事实缺口隔离能力，不应将此作为生产恢复方案；保留文件升级回来继续读取。

后续任务数据和最终全量验证将在各任务完成后补充到本文件，不以旧报告代替新验收。

## F02：一致的检查点预算与旧文件恢复

- 根因：读端 16 MiB 上限与写端无界状态不对称；call 标签和最长 512 字节去重身份重复存储，outbox 也没有字节预算。
- 修改：integrations/reducer.rs、journal.rs、providers/generic_event.rs。去重集合采用稳定 SHA-256（不清空旧集合），并发新 call 上限 128，标签限 256 字符，保留匹配结束/解除通道；reducer 状态 10 MiB、outbox 4 MiB、完整原子检查点 16 MiB。outbox 在事件边界暂停，ack 后继续，不提前提交游标。
- 正式回归：f02_6800_maximum_field_events_do_not_create_an_unreadable_checkpoint；f02_legacy_oversized_checkpoint_keeps_evidence_and_pending_and_can_end；f02_bounded_writer_preserves_previous_bytes_on_overflow；f02_large_pending_outbox_pauses_at_fact_boundary_and_drains_after_restart，四项通过。6800 条极限事件先复现旧版失败：20,951,308 字节；修复后 51,732 字节（128 call 接纳、6671 条超额新 call 明确拒绝）。含 6800 旧 call 和 pending 的超限文件：20,951,534→2,673,216 字节，调用身份、pending、unread、pin 保留，随后显式结束成功。
- 兼容：原始超限文件先保存 .oversized.bak，旧身份编码迁移先保存 .pre-budget.bak；不修改 journal。旧读取恢复窗口单独限 128 MiB，并非放宽正常写入上限；更大文件保留原状并返回 legacy_checkpoint_export_required，需要离线导出迁移。无法缩入预算的旧状态同样保留证据、不清空；未知身份编码拒绝解释。不要降级到不理解哈希编码的旧程序。

## F03：空闲轮询零检查点写入

- 根因：每次空 outbox 都无条件 serialize/fsync/replace，即便 journal 游标、owner、recovery 都没有变化；重复 ack 也无条件写盘。
- 修改：generic_event.rs、journal.rs，明确跟踪 cursor/state/outbox/owner/recovery dirty；旧编码/损坏恢复标记也参与持久化。重复 ack 无变更不写；新进程只持久化一次 owner 转移，旧 outbox 的原 owner 不改写，不重新发历史通知。
- 回归 f03_one_hundred_idle_polls_write_nothing_and_ack_is_idempotent 先失败后通过：100 稳定空轮询+重复 ack，从 **101 次 / 141,703 字节** 降至 **0 次 / 0 字节**。新 owner 写入一次、重启无 outbox 重放。integrations:: 38 通过、2 项实机 opt-in 忽略，包含既有 R1–R4、游标、隔离、quarantine、ack 恢复用例。
- 诊断 quarantine_summary 增加本进程累计 checkpoint 写入数和字节数（非历史全局统计、不包含事件载荷）。维持同目录临时文件、fsync、原子替换；没有引入新的持久化系统。

## F04a：有界 JSONL 与非权威追赶

- 修改 jsonl_tail.rs、codex.rs、claude_code.rs。每次最多读 256 KiB，16 KiB 分块，单行上限 4 MiB；只保留未完成行，完整消费即释放 Vec。UTF-8 在行完整后验证，兼容完整末行无换行。截断/替换/anchor 不匹配重置解析，不把不同文件拼成一轮。
- Codex/Claude 在追赶、半行或坏行期间保留上次观测为 stale、source 非权威；不把日志中间的旧结束/请求当成当前最终事实。超长/坏行继续有界消费后续记录，但保留 gap，不能以静默丢行宣称可信。当前 gap 需要完整新文件基线/替换来清除；没有用计时猜结束。
- 三个正式 f04_ 回归通过；Codex 39 通过/1 实机忽略，Claude 10 通过/1 实机忽略。8×8 MiB 用例先失败：Vec 驻留 67,108,864 字节、单批 8,388,608；修复后 **0 字节 / 262,144 字节**。这两项是容器容量/批量，不是进程内存。
- 回归覆盖跨块 Unicode、完整无换行末行、超长/坏行继续追赶、替换恢复，以及日志中间旧终态后还有新 turn 不误报。单行超限的不可解析事实明确降级，不宣称支持无界输入。

## F04b：缓存退出候选集与分层保留

- 根因：Codex cache.contains_key / seen、Claude transcripts.contains_key 使旧已完成文件不断选择自己，超过发现窗口也无法释放。修改 codex.rs、claude_code.rs，新增共享 parser_cache.rs：不活跃解析缓存最多 128 项、估算 16 MiB、7 天；运行中、明确未解除请求及未追赶完成状态不强制驱逐（安全优先，不宣称活跃状态硬内存上限）。
- 淘汰的冷文件通过单个时间水位避免每轮重新全量读取；正常 native append 或新文件 replacement 更新 modified/created 后重新发现。进程重启允许重建一次，日志身份/turn 不改变；未读、固定、通知账本不迁移、不删除。已完成的轻量任务观测与解析缓冲分开保留，Codex 仍同步 metadata 名称。
- f04_cold_cache_expires_without_replaying_or_losing_reactivation 先失败后通过；共享 budgets_preserve_active_and_do_not_reparse_evicted_unchanged_files（140 文件，128 冷缓存+2 活跃）、f04_claude_cold_parser_eviction_keeps_business_result_without_replay 通过。provider 全组 167 通过/8 opt-in 忽略（新增 Claude 回归另通过）。
- 边界：16 MiB 是解析状态估算预算，不等于进程 RSS；活跃/未知必要状态可超过软预算并应诊断。冷文件由宿主保留 modified 和 created 时间原地改写的非标准操作，不能保证立即重发现；重启会重建。没有引入数据库、自动日志归档或 compaction，也没有删除宿主日志。
