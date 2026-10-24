# Codex 子线程结束误报修复（2026-10-22）

## 实际原因与证据

用户反馈主任务仍在思考、工具执行或上下文压缩时，弹出“本轮结束”。只读核对真实 WSL rollout 与通知投递记录发现：

- 两个自动子线程的日志首先写入自己的 `session_meta`，下一条却是从父会话复制的 `session_meta`，随后包含继承的历史，再执行自己的 turn。
- 旧解析器遇到每一条元数据都覆盖 `session_id`，把子线程重新归到了父会话；子线程的结束时间比父会话最近活动时间更新，因此可覆盖父任务。
- 2026-10-22 12:42:20.970 UTC 的子线程 `task_complete` 对应 12:42:21.922 UTC 的父任务错误结束提醒；随后另一子线程结束又触发相同父任务的提醒。父日志在该轮并没有对应的结束事件。

因此这是日志身份／来源隔离缺陷，不是低 CPU、长时间思考或压缩上下文本身代表任务结束；也不是用户错过了一个真实审批请求。

## 修复边界

1. 每份 rollout 首个有效原生元数据锁定身份，跨增量批次保留。后续不同 ID 的继承元数据不能覆盖身份、项目或来源；文件真正替换时仍由增量读取器重建状态。
2. 仅依据明确原生 `thread_spawn.parent_thread_id` 标记自动子线程为内部执行，与已有 guardian 一样排除出独立用户任务与完成提醒。没有父线程证据的未知来源不强行归类。
3. 本地、WSL 与 SSH 复用相同规则；远程完整日志、截断尾部以及元数据目录保持一致，不让复制的父元数据污染目录分类。
4. 普通会话、手动分支不因标题、项目、`forked_from_id` 或名称含有“review”而被隐藏。父任务仍需自己的明确结束事件才通知。
5. 不清空关注状态、不重置通知账本、不安装 Hook、不改变账号或审批配置，也不使用计时猜测结束。

## 回归与验证

正式回归位于 `src-tauri/src/providers/codex.rs`：

- `fork_identity_cannot_be_overwritten_by_inherited_metadata_across_batches`：分批读取继承元数据时身份不变；普通分支不误分类。
- `forked_workers_cannot_end_running_parent_through_store_attention_and_restart`：本地／WSL 两种身份下，两个子线程运行与结束均不改变父任务；经过 Store、Attention、持久通知账本重启后，真正父任务结束仍只通知一次。
- `fork_spawn_origin_survives_remote_full_and_truncated_rollouts`：远端完整与截断日志、对象与 JSON 字符串来源均保留内部分类。
- `user_fork_does_not_inherit_service_classification_from_copied_history`：普通用户分支不会继承历史中 guardian 的分类。

前三项已在修复前运行失败：分别复现父 ID 覆盖、子 turn 覆盖父 turn，以及遗漏子线程来源分类；修复后通过。

只读实际日志回放 `live_codex_fork_identity_replay`：26 份真实 rollout（其中 14 份原生自动子线程）均在整文件和有界增量读取中保持文件自身身份；反馈中的父任务回放为 `RUNNING / Thinking`，没有父任务结束证据。只输出身份、状态和计数，不导出指令、提示词或思考内容。

可重复执行（需明确提供已运行环境的会话根目录，不会启动 Agent）：

```powershell
$env:CARGO_TARGET_DIR = 'src-tauri/target/codex-check' # 不替换运行中的托盘 EXE
$env:TASKPULSE_CODEX_REPLAY_ROOT = '<已有的 Codex sessions 目录>'
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_codex_fork_identity_replay -- --ignored --nocapture
```

此回放是实际日志兼容证据，不等于重新发起付费 Agent 任务完成了全生命周期宿主验收。原生 WebView2 smoke 使用隔离合成任务，不把其通过宣称为宿主支持范围扩大。

其他验证结果：

- `scripts/verify.ps1`：fmt、Clippy（`-D warnings`）、Rust 407 项通过／13 项显式 opt-in 跳过、custom-protocol 构建通过。
- 前端 Check：0 errors / 0 warnings；Build、UI Model 30 项、Docs 8 项、Release contract 8 项、Security contract 6 项通过。
- `live_wsl_codex_snapshot`：Ubuntu 真实元数据与增量读取通过；该次冷读取约 29 秒才追赶完成，第二次读取约 692 ms。不能将此宣称为已解决所有 WSL 延迟。
- `live_remote_snapshot_against_real_hosts`：两台已配置主机连接正常、Codex 与 tmux 来源完整、174 条目录元数据解析成功，预期项目目录存在；没有为验收额外执行远程 Agent 任务。
- `native-attention-regression-smoke.ts`：隔离 WebView2 实测思考不通知、明确提问保持提醒、匹配回答撤回提醒、下一轮撤回旧结果、额度测试不生成任务，面板高度保持 720 px。
- `native-product-smoke.ts`：隔离原生任务、筛选、来源保存与读取、提醒渲染以及 1/1.5/2 DPR 布局通过。证据目录分别位于忽略的 `src-tauri/target/ui-smoke/evidence/lifecycle-native-2026-10-22T12-58-05-359Z-G7iVdp` 和 `product-native-2026-10-22T12-58-13-353Z-J2IWZP`。
- 前端中等及以上漏洞门禁通过；完整 `bun audit` 仍报告 1 项低危：`@sveltejs/kit > cookie@0.6.0`（GHSA-pxg6-pf52-xh8x）。本次未改依赖，不宣称零风险。Secret audit 无发现；本次不是候选发布，未重新执行在线 Rust advisory 全量审计。

## 不在本次范围

- 此修复没有更改额度账号识别／缓存隔离（仍在 `TODO.md` 暂缓）。
- 此修复没有处理先前已诊断的 WSL UNC 创建时间不稳定导致重复追赶的性能问题；采集不完整时仍保持不确定性，不能据此宣布任务结束。
