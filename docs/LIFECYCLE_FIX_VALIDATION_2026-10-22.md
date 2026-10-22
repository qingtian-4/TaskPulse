# 运行状态、误提醒与面板高度修复

日期：2026-10-22（Asia/Shanghai）。本次为本地源码修复，不修改既有 Alpha.7 标签、安装包或 Draft；不推送、不发版，不提升扩展 Agent 的宿主验证等级。

## 根因与改动

- 投递账本中，同一个 WSL Codex 会话出现了 87 条“本轮结束”检测记录，不是仅用户漏看提醒。
- 真实 rollout 含约 13MB 的 `compacted` 历史压缩记录，超过 JSONL 读取器 4MB 单行预算。此前超限行永久留下 gap，后续新 turn 无法成为可信观察，UI 可能仍显示旧结束状态。现在 Codex 可流式丢弃超限的 `compacted` / `world_state` 非生命周期载荷，识别唯一根层 type、支持字段顺序变化，不保留巨大历史内容；通用读取器默认行为、未知超限记录、坏行和 F04 追赶门禁仍保守。
- Store 曾允许较旧的同源等置信度状态覆盖新观察；Codex 结束/错误/审批事件未完整校验 turn。现在旧 turn 不结束当前执行，旧时间戳不倒退原生会话状态。新的用户消息可重新开启普通已结束会话，但不能解除明确待批准/待回答请求。
- 原通知账本按任务只记最近一次事件，turn 来回切换可能重复发送旧结果。现在原生 Agent 按 session + turn 去重，沿用并迁移现有账本，不清空历史收据；重放和重启不再重复已投递结果，旧错误 inferred-finish 不吞掉真实结束。
- 普通结束弹窗原先不重新核对当前任务。现在新执行/新请求撤回同会话旧结果弹窗，不改变未读/固定状态。未知等待不发送原生 Agent 的“需处理”通知，思考、工具返回、模型等待不代表结束。
- 明确授权/提问显示“需处理”。明确非自动重试的额度错误显示“需处理：额度已用完”、不自动超时；不会被同 turn 后续 `task_complete` 改成绿色成功。普通限流、服务繁忙、自动重试、慢工具或提权参数不当作额度耗尽/待授权证据。
- Native 初始高度原为 520px，前端只有 max-height，按自身内容反向缩小窗口。现初始窗口与面板目标均为 720px，仍按可用屏幕空间/viewport 限制，列表独立滚动。

## 验证结果

1. 先加入正式回归，初次 7 项新用例失败，复现大 compaction、旧 turn 结束、新用户消息、quota 误分类、Store 回退、重复提醒和队列残留；修复后通过，并补字段顺序/嵌套伪 type/重复 type、legacy 账本迁移、错误重试与跨 turn 用例。
2. `scripts/verify.ps1 -Frontend` 成功：Svelte Check 0 错误/0 警告、前端 Build、fmt、clippy `-D warnings`、Rust 401 通过/12 opt-in 忽略、UI Model 30、Docs 8、Release 8、Security Model 6、Integration Contract 3，以及隔离 EXE bridge 通过。前端仍有 1 low，阈值通过不等于零风险。
3. 实际安装的 `codex-cli 0.163.0-alpha.10.2` 可只读导出协议 schema，核对 `ErrorNotification` 的 thread/turn、`willRetry`、`usageLimitExceeded` 字段。没有运行付费模型任务或使真实账号耗尽。
4. 修复后二进制对真实数据执行 30秒 `--diagnose`：本机当前任务及 WSL 当前任务均为 running、非 stale，两来源 connected，观察窗口 transition_count=0。部署新托盘后，再读真实关注状态仍为两个 running，分别观察到 tool / thinking 阶段。
5. opt-in Ubuntu 只读 smoke 通过：当前任务 running，冷追赶约 11.3秒、第二次采集约 579ms。冷启动并非瞬时；未宣称 P50/P95 或长期零误报。
6. SSH 只读 smoke 核对 HyperMem 原生目录通过；首轮 Server1 metadata 暂时异常，复查两台服务器 Codex/tmux 均 authoritative，目录共 174 条。没有将短暂源故障当作任务完成。
7. 新增 `bun scripts/native-attention-regression-smoke.ts --exe <isolated-exe>`，真实 WebView2 + IPC、隔离合成来源通过：窗口/面板 720px，列表 433px；两个运行任务可见；思考不提醒；提问明确需处理且不超时；匹配回答撤回；新 turn 撤回旧结果；quota 测试提醒不创建真实任务。首次功能通过但清理 EBUSY；新脚本增加有界重试后完整退出码为 0。原有只读 `native-lifecycle-smoke.ts` 保留原用途与内容。

原生证据位于 Git 忽略目录：

`src-tauri/target/ui-smoke/evidence/lifecycle-2026-10-22T01-34-19-035Z/`

含 `result.json`、`dashboard.png`、`quota.png`。截图为实际原生渲染的合成测试，不是真实额度耗尽或宿主认证。

8. 既有 `native-product-smoke.ts` 首轮在管理页查询 `.integration-advanced` 的 `open` 时元素为 null，退出 1。定位为测试就绪条件不完整：`.setup-flow` 随 settings 先渲染，而当前来源在异步发现后才选中并渲染编辑器。保留该文件原有未提交修改，只增加等待具体编辑器、Hook 登记与能力摘要就绪，不删改折叠状态或能力边界断言。重新运行退出 0：跨 Dashboard / 来源管理 / 通知三页、DPR 1/1.5/2、480px 短窗口、27 任务滚动、搜索/未读/已查看、来源保存/测试读取/事件接收、真实通知渲染均通过。仍是隔离合成来源，不等于真实宿主认证。

完整原生 smoke 成功证据（`result.json` 记录三页九项 DPR 检查；另保留 `sources-100.png`，Dashboard / quota 截图见上面的 lifecycle 目录）：

`src-tauri/target/ui-smoke/evidence/product-native-2026-10-22T01-42-47-050Z/`

首轮失败证据 `product-native-2026-10-22T01-26-30-228Z/` 保留。成功轮结束时 Windows 仍持有临时 WebView2 profile，脚本记录清理 EBUSY 警告；证据已复制到上述目录，临时目录 `target/product-native-sTzxRN` 仍保留，不是用户配置目录，未为清理它而停止真实 Agent。

## 发布与能力边界

- 仅重启本工作区 TaskPulse，未停止 Codex、WSL 或服务器任务。旧 EXE 和 attention/recent/notifications 替换前以 `pre-lifecycle-fix-19de7e13` 后缀备份；未清空状态，未修改真实 Hook、代理、模型或审批设置。
- 支持明确结构化 `usageLimitExceeded` / usage-limit 错误；不按普通百分比或 HTTP 429 猜测任务被阻塞。本机账号不代替 WSL/远程账号；没有明确额度事件的宿主仍可能无法投递该类提醒。
- 真实账号额度耗尽、长期误报/漏报统计仍有缺口；完整跨页面 smoke 已补通过，独立干净 Windows 安装验收继续暂缓。
- 真实会话、命令与完整日志不提交 Git。此修复不包含在既有源 SHA `b5dc64a` 的 Alpha.7 安装包内。
