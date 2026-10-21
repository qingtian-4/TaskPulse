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

## F05：DSH 有界解压与 WSL 二进制流

- 修改 deepseek_harness.rs。原实现同时分配 compressed Vec、decode_all Vec、解压文本；WSL 另有 Base64 字节/String/解码副本。现在原生 File/WSL stdout 直接进入 zstd Decoder 和有界逐行 reducer，不使用不可靠压缩字节偏移。压缩输入 64 MiB、解压 128 MiB、单行 4 MiB、zstd window 最大 64 MiB；发现超限/损坏/半帧时不发布部分终态，保留旧状态为 stale。
- WSL 小型元数据 stdout 限 8 MiB、stderr 限 8 KiB，日志按管道流式处理；保留 running-distro 前置检查，Linux timeout 管理命令组、Windows 15 秒截止及 child drop kill/wait。不安装组件，不唤醒已停发行版。若 Linux 缺少 timeout，明确探测失败，不绕过安全限制。
- 正式回归 f05_oversized_decoded_record_cannot_publish_a_terminal_after_missing_facts 先失败后通过；f05_streaming_multiframe_matches_full_decode_and_rejects_partial_or_corrupt 覆盖多帧、无换行末行、半帧、损坏及与整块参考解压状态一致。DSH 模块 23 通过/3 opt-in 忽略，随后新增 WSL opt-in 单独通过。
- **Windows 隔离进程测量**（同一测试、同一 66,977,850 字节 JSONL / 6,226 字节压缩输入，5ms 采样，进程句柄保留，退出码均 0）：整块解压 550ms、采样 private 峰值 71,163,904 字节、peak working set 77,668,352；流式 489ms、private 3,117,056、working set 9,433,088。为合成高度可压缩数据的单次测量，不等于实际 GUI 或长期平均。
- 实机只读通过：本机 Desktop 11 个现有会话、projection 11 项，采样 643ms；已经运行的 Ubuntu 三份旧版日志经实际 WSL 二进制管道/解压通过（compressed/decoded：1,306,858/2,653,769；1,884,745/4,806,395；16,158/49,432 字节）。未触发真实任务或写宿主配置，不把历史只读采样冒称实时 Hook 验收。
- CPU 边界：相同 mtime 的缓存继续复用；变更文件仍从头流式解压。减少了峰值内存和副本，**没有消除变更日志重复解压的 CPU 成本**。

## F06：磁盘预览与最小资源诊断

- 新增 scripts/disk-report.ps1、clean-build-cache.ps1、cache-common.ps1、cache-tools.test.ps1。只读报告将 Rust debug/optimized/隔离 target、node_modules、前端构建、Git 历史、安装包与其他内容分别计量，不重复计算；用户状态单独列出。默认清理命令仅 dry-run。显式删除只接受白名单可重建子目录，拒绝链接/junction、越界、不可读树、编译器或目标内正在运行的进程；不终止进程，不删除 .git、安装包、用户配置或状态。
- Windows 隔离测试通过：默认预览不改文件、字节计量、非白名单拒绝、junction 拒绝、**仅临时 fixture** 的显式删除及历史保留。真实工作区没有执行 Delete。verify.ps1 -Frontend 纳入该测试，继续复用原有 TargetDir 构建机制。
- 当前机器实际只读统计（4218ms）：总计 **3,268,405,882 字节**，Rust debug 3,108,178,332；node_modules 136,274,842；.git 13,701,946；前端构建 982,071；用户 .taskpulse 234,180。不是历史提到的 60GB，不能将两次不同环境/时间的差额归因于本轮优化。预览默认可重建缓存约 1.39GB，未删除。
- 新增 providers/resources.rs：IPC get_resource_diagnostics 与 --diagnose 的 resources 字段报告当前进程的 parser 项数/保留缓冲/读取字节、DSH 解压次数/成功读取字节、checkpoint 写入次数/字节、有界 journal/state 目录统计、90% journal 配额压力、Windows private/working-set/peak-working-set。Windows 数据不包括 WebView2 子进程；Linux/macOS 明确标为不可用。路径使用 source/root 哈希，不含命令/提示词/凭证。资源统计不持久化、不触发 watcher。
- diagnostics_do_not_expose_paths_or_payloads_and_disk_usage_is_read_only 与 Clippy 通过；新增未变 mtime 不重复解压断言。诊断目录统计最多 10,000 项，配额扫描最多 64 source × 1024 journal，达到边界明确 complete=false，不伪装精确总数。

### 使用

```powershell
# 只读定位磁盘，-Json 可用于保存本机诊断（含本地文件路径，勿未经检查公开）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/disk-report.ps1
# 默认预览，不删除；本轮未在真实目录使用 -Delete
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/clean-build-cache.ps1
# 构建验证（选择未运行的原有 target，不替换用户正在运行的托盘）
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend -TargetDir src-tauri/target
```

--diagnose 的原有输出包含真实任务标题/项目，仅本地排查，不应原样公开。get_resource_diagnostics 可从已有 WebView2 原生 IPC 调用，只读且异步扫描；本轮不扩展主界面。

## 实机发现后的 F04 追加修正

最初单块预算直接等于一轮 provider 预算，真实 Ubuntu 20 份 Codex 日志（总 90,724,462 字节、最大 52,048,572）导致首次 smoke 仍在追赶，未通过。没有放宽可信终态判定：新增共享 JsonlTail::drain，Codex / Claude transcript / Claude Hook 都逐块折叠，每次最多 16 MiB 或 500ms 工作预算，内存仍是单块/单行预算，剩余积压继续非权威。时间只限制后台工作，不判断任务结束或审批。

f04_partial_catchup_cannot_publish_a_historical_terminal 扩为 18 MiB，保证跨 provider 轮次仍不发布中途旧终态；provider 171 通过/10 opt-in 忽略。真实 WSL 初次 poll 2562ms，完全追赶 13,865ms，后续轮询 651ms，20 个 native catalog 身份/路径隔离通过。实机 smoke 等待有界追赶（30 秒失败截止、保留正常轮询节奏），仍要求 authoritative；坏行不会被重试时间洗成健康。当前大日志冷启动仍有十余秒开销，不宣称瞬时启动。

资源诊断复核又补一项正式红绿回归 quota_scan_never_claims_completeness_after_an_entry_budget_cutoff：含不相关条目的目录不能在扫描被截断后仍声明 complete=true。已将入口扫描预算与有效 source 数分开，读取错误也显式不完整；不通过忽略未知项来夸大配额统计覆盖。
