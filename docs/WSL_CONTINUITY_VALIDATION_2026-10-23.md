# Codex WSL 采集连续性专项验证 · 2026-10-23

基线：`codex/desktop-discovery-visibility` / `a4793aa8827bd6dc1704d851f5e5b050136dd956`。范围仅 WSL 采集、共享游标必要兼容修复、脱敏诊断和 Dashboard 不确定性解释。没有改变生命周期事件解释、审批策略、额度、其他 Provider 协议、README 或发布流程。

## 1. 根因与证据强度

| 发现 | 证据与修复 |
|---|---|
| UNC 时间不稳定触发全量重读 | 旧游标以 creation 变化、同长度 mtime 变化重置；WSL 的 creation 可跟随 mtime，而不是 Linux birthtime。正式时间漂移回归在修复前失败，修复后重复读取正文为零。改用 Linux stat 身份/时间，仍校验头部和游标锚点。缓存淘汰水位也使用相同 Linux 时间域，原有容量、时限和保护规则不变。 |
| Running → stale → 前端 uncertain | 确认是“进行中 1 → 0 → 1”的直接分类路径，不等于真实结束。前端正式测试保留这个真实的新鲜度变化，另外明确显示“上次运行 1 · 当前采集待更新”，不强制 Running=1。 |
| 非权威输出覆盖备用快照 | 修复前 partial empty 回归得到 0 条任务；拆分最后可信基线与部分观测，缺失任务只作为 stale 保留，不能被部分空结果删除。 |
| I/O 错误仍可能留下 trusted 游标 | 文件 open/read 失败现在显式使本轮不可信；不能因缓存游标此前追赶完成而宣称当前读取成功。 |
| 原生固定元数据兜底为 Idle | 已知执行中会话未完整解析时，不用固定会话的 Idle 元数据覆盖 Running。正式解析器故障注入覆盖坏行、缺失与恢复。 |
| 新旧 rollout 混合误用旧终态 | 新文件未读完时，旧文件虽完整，也不是该会话的完整新鲜证据。18 MiB 续写 + 同 ID 旧结束文件的正式回归在修复前失败；现在按 session 完整性阻止中间终态，其他完整会话独立更新。 |

这里确认的是可复现代码缺陷和直接分类路径。此前用户那一次“采集失败”没有阶段日志，不能反推为已证实的 Python/SQLite/WSL 启动失败，也不能声称每次闪烁都是同一原因。

## 2. 实机与阶段诊断

实机：Windows 原生 GNU/MinGW 构建，已运行的 WSL Ubuntu，Python **3.10.12**。样本首条原生元数据的 `cli_version` 为 **0.162.0-alpha.17.2**（日志格式证据，不冒充当前新任务宿主验收）。只读现有 Codex 日志，未发起付费任务，未停止/唤醒发行版，未写 Codex SQLite、日志、账号、Hook 或配置。

本轮再次只读对照了一份 51,801,348 B 的日志：UNC CreationTimeUtc 与 LastWriteTimeUtc 都为 `2026-10-23 02:46:01 UTC`，而 Linux birth epoch 为 `1792547274`、mtime epoch 为 `1792723561`。说明 UNC creation 确实不能当成该 Linux 文件的创建身份；未为实机验证修改这个文件的任何时间或内容。

第一轮实机读取 17 个日志，共 116,553,984 B：前 7 轮明确为 `rollout_read:catch_up`，第 8 轮变为权威结果，后续三个热轮询正文增量为 0 B。没有观察到 Python 启动失败或 SQLite 真实忙锁；忙锁、损坏、索引解码错误由隔离测试覆盖，不能当成实机故障证据。

原先刷新界面的“采集失败”涵盖所有 degraded，包括有界追赶未完成；现在保留真实错误阶段，刷新结果随来源恢复更新，不永久保留误导性的历史失败文案。

新诊断包括 source_id、session/turn hash、poll 开始/结束/耗时、权威性、错误阶段码、最后成功时间、每文件长度/offset/重置原因/累计读取、每轮正文读取、Worker 投递延迟、Scope 前后、Store 前后和 Attention display 数量及语义分类。游标详情最多 128 条，事件环最多 512 条，显式报告截断；进程内诊断不写真实状态文件。元数据失败时会标记本轮未观测 rollout，所附旧游标不能冒充本轮证据。

实际前端分类还可只读检查 `.dashboard` 元素的 `data-continuity` JSON：包含全局/当前筛选后的 ongoing、uncertain、results、pinned 和待确认运行数量，按 source_id 分组；不包含任务 ID、标题、路径、搜索内容、命令或回复。它随 UI 数据变化更新但不保存历史，原生 smoke 用它交叉验证真实 DOM，而不是用后端预测冒充前端结果。

## 3. 读取与时间测量

固定隔离文件为 5,032,500 B，10 次仅改变模拟 UNC mtime，Linux stat 保持相同。旧策略作为同一测试内的对照，无真实日志写入。

| 指标 | 旧时间策略 | Linux stat 策略 |
|---|---:|---:|
| 初次读取后的 10 次正文读取 | 50,325,000 B | 0 B |
| 总 reset（含初始化一次） | 11 | 1 |
| 非必要 reset | 10 | 0 |
| 热轮询内容锚点校验 | 随重新扫描进行 | 1,280 B / 10 次 |

最终固定文件复测：冷游标读取 70 ms；10 次热采样的旧策略累计 694,875 μs，新策略累计 1,337 μs。均为本机一次对照，不是跨机器性能保证。计数是应用层读取字节，不是物理磁盘或网络流量；0 B 正文不意味着零 I/O，仍有 stat、打开文件和锚点校验。冷采集指冷解析游标，不宣称清空了 OS 文件缓存。真实 Codex 日志/配置正文写入为 0；诊断环无磁盘写入，回归 fixture 和导出证据只写隔离目录。

首次实机结果：首轮 2504 ms、后续追赶 795–1367 ms/轮；从首次权威结果后的三次热读取分别为 356 / 281 / 288 ms。17 个初始化 reset 后不再增长，0 条完成事件。日志当时没有新鲜 Running 会话，故不能把该只读结果宣称为活跃任务端到端验收。

最终实机复测相同正文总量：首轮 2085 ms，6 轮非权威追赶后第 7 轮完整；其后三次热采样 284 / 237 / 268 ms，正文均为 0 B、每轮锚点校验 2176 B，reset 仍为 17。整个测试窗口 12.72 s（含后续热采样和每轮额外 300 ms 的测试间隔），不当作严格的用户任务检测延迟。所有观测均无完成提醒。

## 4. 故障、恢复、结束行为

| 场景 | Provider / Store / Attention | UI / 提醒 |
|---|---|---|
| 正常运行 | 新鲜同一 session/turn | 进行中；思考/工具/等待模型均不通知结束 |
| 一次、连续失败或部分空结果 | 保留可信业务状态，缺失观测为 stale；未读/固定/账本不变 | 状态待更新 + 上次运行计数，保留错误入口，不计为新鲜运行，不通知结束 |
| 部分会话成功 | 完整会话独立更新；不完整会话不被旧文件终态覆盖 | 新鲜与待确认分开显示 |
| 恢复 | 新鲜证据覆盖 stale，身份不变 | 回到进行中；同 turn 不重放历史结束提醒 |
| 明确真实结束 | 仅完整、对应会话的终态生效 | 进入结果待查看，提醒一次；后续失败/旧缓存不复活 Running |
| 真实缺失/坏行 | 明确降级；不能用计时猜结束 | 可能持续待确认，直到可靠恢复或用户处理来源，不永久伪装为新鲜 Running |
| 重启 | 从隔离 Attention/通知账本恢复稳定 ID、未读和固定；首次观测仍需新鲜度核对 | 不清空关注、不重发已交付终态 |

前端回归覆盖搜索、仅未读、来源/项目筛选、多 Agent 混合；待确认运行提示使用全局列表，不被过滤到零行的视图吞掉，点击定位会清除冲突筛选。原生 WebView2 用隔离合成任务验证真实 IPC、DOM、计数与按钮；不是新的 Agent 宿主认证。

## 5. 验证与证据

- 修复前可运行失败：时间漂移、部分空结果、新旧 rollout 混合三项回归；修复后通过。
- Rust：423 passed / 15 opt-in ignored；包含 F01–F04、旧子线程隔离与异步提问关联、WSL 故障/恢复/结束/重启流水线。
- Python：6 项隔离元数据测试通过（含 Python 3.10 busy 错误兼容、只读锁与重试边界）。
- 前端 Check 0 errors / 0 warnings；UI Model 33、Docs 8、Release contract 8、Security contract 6、Integration contract 3 通过；前端 Build 通过。
- `scripts/verify.ps1 -Frontend` 通过：fmt、Clippy `-D warnings`、完整 Rust、custom-protocol 构建、bridge 原生 CLI smoke。Smoke cleanup 14 passed / 1 显式跳过。
- 最终原生生命周期 smoke 通过：720 px 面板、stale Running 提示与定位、新鲜运行计数归零但任务仍可见、实际前端诊断计数、恢复不通知、明确问题/结束文案区分、额度测试仍正确。中途一次 smoke 在截图期间被正常 worker 刷新合成状态，导致测试点击已不存在的按钮；已改为最多 5 次的前后同态截图与同步读取/点击采样，不冻结 IPC、不停 worker、不放宽产品断言。失败证据保留在 `lifecycle-native-2026-10-23T06-44-53-567Z-xiOWm0`。
- 原生 product smoke 通过：27 个隔离任务、搜索/全局筛选/未读/固定操作、来源读测/事件测、提醒和 1/1.5/2 DPR 布局。没有新建真实宿主配置或以合成事件宣称宿主完整验收。
- SSH `live_remote_snapshot_against_real_hosts` 通过，且设置 `TASKPULSE_EXPECT_REMOTE_CATALOG_PROJECT=HyperMem` 检查预期目录，未发起远端 Agent 任务；不把测试名称本身当作所有远端来源始终健康的保证。
- 完整 `bun audit`：1 项低危 `@sveltejs/kit > cookie@0.6.0`（GHSA-pxg6-pf52-xh8x）；中等及以上门禁通过。本轮不改依赖，不宣称零风险。非发布任务，未重新进行在线 Rust advisory 全量审计。

首份只读脱敏证据：`src-tauri/target/codex-check/ui-smoke/evidence/wsl-continuity-1792735886817.json`。证据与本机构建产物均在忽略目录，不进 Git。

最终只读证据：`src-tauri/target/codex-check/ui-smoke/evidence/wsl-continuity-1792737701220.json`。原生截图/断言/清理证据位于 `src-tauri/target/ui-smoke/evidence/lifecycle-native-2026-10-23T06-50-38-610Z-WpOT2e` 和 `product-native-2026-10-23T06-47-15-089Z-EeuzOo`；前者含 `uncertain-running.png` 及实际前端全局/可见计数 JSON。两者的 `cleanup.json` 均为 `complete: true`，无遗留测试进程或工作目录。原生截图只含明确标记的隔离合成任务，不含用户真实会话。

## 6. 边界与剩余风险

- 真实大日志首次追赶仍有可见启动延迟；这次修复消除反复重置，不承诺冷启动瞬时完成。
- 活跃时 Linux stat 与 UNC 长度仍有竞态；只重试一次，持续不一致必须诚实展示退化。对无明确结束证据的丢失日志，无法自动证明任务已结束。
- 未为验收费发起新的 Coding Agent 任务，也未强制中断用户的活跃 WSL；长时间运行中的真实断连/恢复目前是隔离故障注入证据，不是实机破坏性测试。
- 不声称解决所有 WSL 网络、Python 缺失或未知 Codex 格式兼容问题。后续可用脱敏阶段诊断区分。
- 没有 Push，因此新代码没有远端 CI 结果；基线 CI 通过不等于本次 CI 通过。
- 保留运行中的原托盘 PID 87256，未覆盖其可执行文件；新构建位于 `src-tauri/target/codex-check/debug/agent-tray.exe`，仅隔离 smoke 使用。修复尚未热加载到旧进程。

## 7. 本地提交

- `8f3abbd`：WSL 可信游标、部分观测保留、元数据阶段错误、采集连续性诊断和正式回归。
- `25392e0`：待确认运行任务、动态刷新结果、前端实际计数诊断、原生 UI 回归。
- 文档提交：`docs: document WSL continuity evidence and degradation semantics`（本报告所在提交）；准确 SHA 见交付消息及 `git log`。

不 Push、不合并、不动 Alpha.7 Tag 或 Draft 安装包。用户原有 `Cargo.toml` 修改及三份未跟踪审查/任务文档的 SHA256 均与开工前一致。
