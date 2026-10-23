# Codebase review — 2026-10-23

审查基线 `5e9c083f1e6816c7680030c374cd34294ca1237b`；展示基线为 [Draft PR #10](https://github.com/qingtian-4/TaskPulse/pull/10) 的 `eed1655008aa4f81c0bb7ffe7a5a1f1a0dc37bd9`。审查在当前 `codex/desktop-discovery-visibility` 上继续，未推送/合并/发布。原有 Cargo.toml 与三份未跟踪用户文档不属于本次修改。

这是静态调用链审查加实际回归，不是形式化证明。P0 = 立即阻断的已证实严重破坏/泄露；P1 = 可靠性、隐私或发布前需决定的风险；P2 = 维护性、证据或文档问题。标为“静态证据”的条目未在用户真实状态上触发故障。**未发现已证实 P0，不能据此推断没有缺陷。** 执行结果与外部验收缺口见 [验证记录](OPEN_SOURCE_READINESS_2026-10-23.md)，文件操作见 [分类清单](REPOSITORY_HYGIENE.md)。

## 入口与覆盖地图

| 路径 | 核对的真实调用/责任 | 现有证据与边界 |
|---|---|---|
| main → tray / watcher | pre-Tauri CLI dispatch；setup 启动 OS watcher；tray/shortcut；window ExitRequested | tray 聚合单测，native IPC/窗口 smoke；不能以浏览器测试替代 |
| watcher → Monitoring / Worker | revision/scope/configure；每来源最新结果 + ordered 结果；100ms loop；独立 remote thread | worker slow-peer/refresh、health、watcher 自身文件过滤测试；热更新缺口见 R03 |
| Codex / questions | metadata/index/rollout 增量；thread/fork origin；turn/request/call；异步问答 | fork/restart、question ACK 非回答、旧 turn end、permission 参数不推断审批、large log/catch-up 测试 |
| Claude / claude_events / claude_integration | native 只读记录 + 自愿 Hook；OS lock journal + latest checkpoint | 并行 permission、唯一 signature 关联、restart replay、proxy/statusLine 保留；隐私/预算见 R04/R05 |
| Hermes | SQLite read-only + runtime lease | lease expiry、missing row、schema degradation；没有专用 WSL/SSH transport |
| DeepSeek Harness | native 与 WSL 独立；legacy/v4 zstd；projection seq | v4/rename/corrupt/projection、bounded streaming、completion/restart tests |
| WSL Codex | running distro 探测 → Linux SQLite/stat → UNC rollout，共用 parser | partial empty、incomplete resumed file、drift/rollover、worker/store/attention continuity；禁止 UNC SQLite/WAL |
| SSH / tmux / tp-run | 参数校验、BatchMode、host 并行、Codex/tmux 分段健康、run identity | fake executor、stale/failure debounce、run identity/detached 边界；live 不自动等于通过 |
| Generic v1 / Manifest / Bridge | bounded stdin、restricted mapping → locked journal → reducer/checkpoint/outbox → ordered dispatch/ack | schema/identity/quota/recovery、concurrent writers、mixed sources、outbox failure、ownership-only Hook revoke |
| Store → Attention → Transition | source-instance reconciliation；TTL vs durable unread/pin；stable turn/request dedup | stale restart、migration、viewed、manual pin、flapping、pending inbox、late timer/rotate/overflow |
| Dashboard / Manage / Notice | Tauri listeners + IPC；独立管理窗口；render ack 与 read 分离 | Bun models + real WebView2 smoke；generation 缺口见 R08 |
| CI / Release / Security | quality/security 独立；NSIS/MSI fresh matrix；medium-integrity app smoke；draft only | release contract、PowerShell process ownership/timeouts、checksum-pinned audits；缺口见 R10/R11 |

## 风险表

| ID / 级别 | 文件、函数与实际证据 | 影响范围 | 建议 / 本轮处理 |
|---|---|---|---|
| R01 / P1 | [transition/mod.rs](../../src-tauri/src/transition/mod.rs) `apply_snapshots` 在 `flush()` 成功后返回事件；[watcher.rs](../../src-tauri/src/watcher.rs) `watch` 的 ordered 分支也先 flush ledger，再 `notifier::handle_events`，再 ack | ledger 已记账到实际 API/窗口接收之间崩溃会漏掉瞬时提醒；重启 replay 会去重。持久 Attention/inbox 仍在 | 不宣称 exactly-once visible delivery。后续设计 notification delivery intent/receipt 状态机与 crash-injection 测试；避免简单交换顺序造成重复。**本轮仅记录** |
| R02 / P1 | [attention.rs](../../src-tauri/src/attention.rs) `open` 对 read/JSON parse/version 不支持使用 `unwrap_or_default`；[transition/mod.rs](../../src-tauri/src/transition/mod.rs) `open` 同样默认空；v4 migration copy 失败只 warn 后继续，而 run-identity migration 已有 backup gate | 损坏/短暂读失败可让随后 flush 覆写原 Attention/dedup；备份失败不阻断 v4 migration，可能失去迁移前证据 | 分离 NotFound、损坏、未知版本和 I/O 错误，保全原字节并显示 degraded；备份成功才迁移。现有 corrupt history 单测只针对 recent，不覆盖这些失败路径。**待单独修复** |
| R03 / P1 | [watcher.rs](../../src-tauri/src/watcher.rs) 原 configure 只按 id/kind/location 保留捕获 SourceConfig 的 worker，closure 直接写 catalog。复核补充：save_monitoring_settings 已禁止同 ID 更换 Agent/adapter，原报告对 UI 路径的推断过宽；配置文件变更、接受策略及旧 worker 晚到仍需防护 | 替换后的旧 worker 可更新发现目录；采集策略变化需明确失效。未在真实用户配置触发故障 | [Issue #12](https://github.com/qingtian-4/TaskPulse/issues/12)：按采集相关字段失效；独立 receiver 作为 generation，丢弃旧缓存/暂存事实但不 ack outbox，catalog 只在主循环提交。保留 native 项目筛选/重命名时的解析缓存；加入晚到屏障注入、配置匹配与真实 reducer 截止策略回归。合入状态以 PR 为准 |
| R04 / P1 | [claude_events.rs](../../src-tauri/src/claude_events.rs) `record` 取 `/tool_input/command` 或 file_path 前 300 字符写入 `activity`，并持久化 cwd/config_dir；[notifier.rs](../../src-tauri/src/notifier.rs) beep/debug 日志包含 task.name/reason；watcher `diagnose` 输出任务标题/项目/turn/quota | 自愿 Hook journal 和导出的诊断可包含私人路径、命令参数甚至嵌在前缀中的 token。最小 notification-log 不含 body 并不能代表全部应用状态脱敏 | 清楚说明本地 activity 数据；后续提供安全 activity 分类/用户可选详情与诊断默认脱敏。测试 sentinel command/path/token 不进公开诊断，不能只测 resource diagnostics。未更改用户 Hook 或旧证据 |
| R05 / P1 | [claude_events.rs](../../src-tauri/src/claude_events.rs) `record` append + sync 无 session/source quota 或 compaction；与 Generic journal 的 32/128MiB 限制不同 | 长期开启 Hook 的 journal 可永久增长；latest JSON 并不替代 Journal，不能直接删除历史来“修复” | 为 Claude lane 单独设计 cursor 安全 compaction/export/预算；加长时增长测试。保留并行权限关联与恢复机制 |
| R06 / P1 | [remote_tmux/ssh.rs](../../src-tauri/src/providers/remote_tmux/ssh.rs) `SystemSshExecutor::run` stdout/stderr reader 直接 `read_to_string`；poll timeout 仅控制 child 等待，reader join 没有独立输出字节/耗时预算 | 远端超大/异常输出可增加内存；若继承管道保持打开，join 有额外阻塞风险。影响远端采集，不等于所有 worker 都受限 | bounded stdout/stderr collector + 明确 oversize/degraded + child wait/reap；模拟大流/延迟 EOF。不要截断 JSON 后当健康空快照 |
| R07 / P2 | [attention.rs](../../src-tauri/src/attention.rs) records 归档仅改变 display；[monitoring.rs](../../src-tauri/src/monitoring.rs) `observe` catalog 只 insert；[transition/mod.rs](../../src-tauri/src/transition/mod.rs) turn/request dedup 长期保留，generic 100,000 达限后拒绝继续确认 | 7 天主视图不会降低持久记录规模；catalog 分页也不降低内存；序列化与 clone 成本增长，达限需维护 | 度量规模并提供用户驱动 export/retention 方案；保留 unread/pin 与去重 tombstones，禁止按 TTL 直接删账本。当前不改数据语义 |
| R08 / P2 | [Dashboard.svelte](../../src/lib/components/Dashboard.svelte) `snapshots()` Promise.all 无 request epoch，和 agents/provider/settings 监听分别赋值；backend LATEST_* 也是不同 mutex 缓存 | 较慢的旧 IPC 结果可短暂盖过较新事件；多个列表/健康可能跨批；这是展示一致性风险，后端 Attention/dedup 不因此改变 | 统一 dashboard snapshot generation 或前端拒绝旧响应；加入乱序 promise/event 回归，避免定时 sleep 测试掩盖问题 |
| R09 / P2 | [watcher.rs](../../src-tauri/src/watcher.rs) `merge_and_emit` 无调用且 allow(dead_code)，其中 engine 在 reconcile_attention 前；`refresh_local` 同属旧 helper；`AgentStatus`/TerminalInfo re-export 仍被 scanner/heuristics/focus 使用；Monitoring ↔ watcher ↔ task_notice 存在双向调用 | 旧 helper 与生产顺序不同，维护者可能误用；编译模块引用存在环，不能称为完全单向分层。没有据此证实锁死；`update_task_attention` 已特意先 resolve 再锁 tracker | 保留仍有调用的兼容类型/测试/focusers；后续单独审计无调用 helper 并隔离 test/diagnose 路径；逐步抽取 facade/共享 domain，禁止本轮整块删除 |
| R10 / P2 | [quality.yml](../../.github/workflows/quality.yml) PR 触发仅 main；PR #10 base 为 codex/desktop-discovery-visibility，head 为 docs/*，不匹配 push 过滤；接入 PR 后本机 docs tests 实际 4 fail | 展示 PR 对开发分支缺少同等 quality gate，broken anchors/文案契约可漏到主线前 | 扩展 PR target 至 codex/**，增加全仓链接/素材/跟踪检查。本轮修正并回归，不添加发布权限 |
| R11 / P1 | 本轮 `bun audit` 报 cookie 0.6.0 low；checksum-pinned cargo-audit 无 vulnerability/yanked，但 10 项 unmaintained、2 项 unsound（glib、rand）；[公开验收清单](../OPEN_SOURCE_CHECKLIST.md) 仍保留暂缓人工 gate | threshold pass 不是零风险；Linux/macOS 依赖也需审查。CI 安装器证据不替代 clean-user/真实 Agent/长时运行 | 单独依赖升级/target 可达性评审；保留 warnings 明细。公开安装器仍需维护者接受剩余风险与人工 gate，**本轮不发布** |
| R12 / P2，已修正 | PR #10 README 去掉精确版本字串、fixture-tested 与 synthetic standard events；INSTALL 旧 anchor 失效，clone 契约变化；旧 ARCHITECTURE 把 WAITING_INPUT 等同 turn end、声称只持久化 RECENT；PROVIDER_GUIDE 仍将已实现 v1 event contract 称为未来提案 | 新贡献者会按错误状态语义实现 Provider；展示门禁实际失败 | 复用 PR，补版本、证据与 main clone；重写架构；修正 Provider 贡献路径、源注释及 SVG；不改生命周期实现 |
| R13 / P2 | [integrations/journal.rs](../../src-tauri/src/integrations/journal.rs) `append_using` 在 source quota 锁内每次 read_dir 并累加所有 journal 文件；Generic provider sample 每 poll 列出并排序全部 paths，再限制处理 256 个 | 每次提交/轮询的目录工作随 session 数增长；字节预算不约束目录枚举成本 | 增加规模测量和可恢复 quota/index 设计，保留锁、跨 writer 准入一致性；不要以未验证缓存替代磁盘 quota |
| R14 / P2，已修正 | [test-codex-metadata.py](../../scripts/test-codex-metadata.py) 初跑旧 schema 断言缺 `source: None`；fixture 的 sqlite3 context manager 只提交/回滚，不关闭连接，Windows 清理报 WinError 32；该脚本与 wsl-metadata.test.py 原未进入 Windows quality/release 门禁 | 远端 origin 元数据契约回归可漏检，测试在 Windows 不可靠 | 显式 closing fixture connection、保留新 source 字段并增加 native origin 断言；两份 Python suite 与 discovery 一起进入 CI/verify，生产采集代码不变 |

## 跨 Provider 语义结论

身份、phase/lifecycle 分离、matching request、fresh/authoritative 与 internal filtering 已有大量实质回归。Codex/Claude/DSH/Hermes 都保留“没有结束证据不宣布完成”的保护。Snapshot 来源与 durable ordered 来源的可靠性不能写成同一级。旧 v1 文件、Scanner 和 heuristics 属于低可信兼容层，未证明可安全删除；源码旧状态注释确有误导，已更正文档与注释。

本轮没有新增 Provider，没有更改 task identity、scope、Attention、Transition 或 journal 行为。R01/R02/R03/R04/R05/R06 的修复都需要有边界的专项设计和故障注入，不能借开源整理实施大规模状态机重写。

## 测试盲区与后续顺序

已有 Rust 438 个测试定义（默认执行与 ignored 的结果见验证记录），包含协议/Bridge、Codex/WSL/SSH fixtures、重启去重/Attention、权限关联、并发 journal、预算与隔离。不能把“有测试文件”当覆盖率统计。本轮没有生成行覆盖率，也没有证明所有故障分支。

后续状态以 Issues 为准，不再追加重复交付报告。R02 跟踪于 [#11](https://github.com/qingtian-4/TaskPulse/issues/11)：修复分支已加入失败时保全/写保护、备份校验、降级提示及损坏/未来版本/备份失败/重启故障注入；是否合入以对应 PR 为准。

优先顺序：R02 的损坏/backup failure 保全 → R03 的 in-flight config generation → R04 的敏感参数保留边界 → R05/R06 的磁盘/输出预算 → R01 的投递意图事务设计。补测时保持以下区分：fixture / isolated native TaskPulse / read-only real source / actual new Agent lifecycle / fresh installer / sustained runtime。只有最后两类的实测才能闭合相应公开发布 gate。
