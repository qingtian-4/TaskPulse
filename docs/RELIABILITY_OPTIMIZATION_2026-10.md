# 可靠性与资源优化（2026-10-21）

基线：46127fd，Alpha.7 候选，codex/desktop-discovery-visibility。按 F01→F02→F03→F04→DSH→诊断执行。保留用户已有 profile.dev.debug=0 修改和三个未跟踪文档，不将其纳入本轮提交。不安装真实 Hook、不改 Agent 配置、不删除缓存/用户状态、不 push/PR/tag/发布。

## F01：接收通道与事实缺口分离

- 根因：last-error 与整个来源 max received 比较，B 的成功/心跳可使 A 的丢失事实变为健康；重启也缺少独立缺口证据。
- 修改：integrations/health.rs、bridge.rs、providers/generic_event.rs；接收失败只保存哈希 session/turn/request/call、事件类型和时间，不保存载荷。每会话/未知关联缺口独立持久化；checkpoint 保存已恢复的缺口身份。旧 source-only 操作错误保守迁移，普通协议验证错误不污染健康。
- 恢复：匹配请求/工具解除或明确当前轮终态可恢复对应缺口；可信新 turn 可独立建立新基线。心跳、其他会话成功和普通 phase 不恢复。未知关联缺口对旧任务保守保留，不阻止其后新轮次；健康 peer 仍保持新鲜。
- 测试：f01_peer_success_and_heartbeat_cannot_repair_a_lost_resolution 先失败再通过；f01_scoped_gap_requires_matching_recovery_and_survives_restart 覆盖跨会话、解除、缺失结束、心跳、新 turn、重启、普通验证错误。现有 receiver_write_failures 回归同步为心跳不恢复规则。
- 兼容/回滚：新增 optional checkpoint 字段及独立 gaps 文件；无状态清空。回滚旧程序会失去事实缺口隔离能力，不应将此作为生产恢复方案；保留文件升级回来继续读取。

后续任务数据和最终全量验证将在各任务完成后补充到本文件，不以旧报告代替新验收。
