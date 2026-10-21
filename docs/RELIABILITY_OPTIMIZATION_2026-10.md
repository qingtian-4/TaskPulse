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
