# 通用 Agent 接入：实施与验证报告

实施起始 2026-10-20；交付记录日期 2026-10-21。对应任务书 AGENT_INTEGRATION_EXECUTION_PLAN.md。本文件报告实际执行，不把设计、文档或 fixture 当成真实宿主支持。

**历史报告说明：下文为 bf55e8f 阶段记录，不能当作 R1–R4 已通过的证据。** 2026-10-21 审查确认了混合通知、异步 turn 关联、超限恢复及孤立 Hook 撤销缺陷。后续修复、正式回归和新原生证据统一见 [R1–R4 修复验证报告](AGENT_INTEGRATION_FIXES_2026-10-21.md)。TRAE idle 正常结束能力已在代码中降级为 unsupported，以下旧表格的相关“idle 结束”描述不再代表当前能力。

## P0–P5 交付

| 阶段 | 已交付 | 边界 |
|---|---|---|
| P0 | 基线、官方契约调查、v1 schema、身份/生命周期/水位 | 基线 ac18c78，版本保持 0.2.0-alpha.7 |
| P1 | exe 前置接收、校验、OS 锁、journal、reducer、原子检查点/outbox、恢复 | 本机、有界保留，不无限制自动轮转 |
| P2 | GenericEventProvider、有序 observation、scope/Store/Attention/Transition、重启去重 | 未重写现有原生 Provider |
| P3 | manifest 导入、逐项能力、预览/确认、诊断/测试/导出/撤销 | 导入不能执行/安装 Hook/自称官方；通用 WSL/SSH 禁用 |
| P4 | Kimi/TRAE CN/ZCode 部分文档依据映射；WorkBuddy 候选和限制 | 四款宿主 Hook 实机验证版本均为无 |
| P5 | 回归、Windows GUI exe/stdin、原生 WebView2/通知、并发、指南/矩阵 | 真实宿主授权验收、外部焦点、MSVC 安装器/长期运行另列门禁 |

用户原有未跟踪文件 docs/REVIEW_HANDOFF.md、docs/AGENT_INTEGRATION_EXECUTION_PLAN.md 保留，不纳入本次提交。真实宿主 Hook、模型/代理、审批策略未修改；旧用户托盘未停掉/替换。所有宿主配置写入验收均位于独立 TASKPULSE_HOME/host-sandbox。

## 四款产品：三层证据分别报告

| 产品 | 代码/合成验证 | 真实宿主验证版本 | 未支持/待验证 |
|---|---|---|---|
| 新版 Kimi Code CLI | TurnStarted、工具、显式审批、失败/中断、心跳/标题、TOML recipe | 无；未确认安装 | 正常最终结束、精确跳转、提问；request/call 字段需真实样本；不推广到 Desktop/VS Code/旧 kimi-cli |
| TRAE 国内 IDE | 工具派生轮次、结构化批准/提问/idle 结束、JSON recipe | 无；安装元数据为 3.5.62，非事件验收 | 纯思考新轮次、标题、整轮失败、无 ID 的解除；不推广到海外/企业/SOLO |
| ZCode Desktop | 工具派生轮次、显式请求、唯一调用结果解除、process argv recipe | 无；安装元数据为 3.11.2，非事件验收 | 正常最终结束、标题、纯思考新轮次、提问/整轮失败；不改 hooks.enabled |
| WorkBuddy Desktop | 候选描述与限制 UI，无载荷映射/安装 | 无；未确认安装 | 桌面 payload、配置及关联契约缺失，不能替代为 CodeBuddy CLI |

官方依据、11 项能力和逐产品待验收步骤见[兼容矩阵](AGENT_COMPATIBILITY.md)。Kimi/ZCode Stop 可继续，**不会假造正常完成弹窗**。tested_versions 全为空，无 native-tested 声明。

## 自动检查与兼容

执行入口（Bun 为本机 bun.cmd，GNU 环境按 AGENTS.md）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend -TargetDir src-tauri/target
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray integrations::journal -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray integration_concurrent_stress -- --ignored --nocapture
cargo build --manifest-path src-tauri/Cargo.toml --bin agent-tray --profile ci-release --features custom-protocol --locked
bun scripts/integration-bridge-smoke.ts --exe src-tauri/target/ci-release/agent-tray.exe --require-gui
bun scripts/native-integration-smoke.ts --exe src-tauri/target/ci-release/agent-tray.exe --stress-worker <本次Cargo-test的绝对exe路径> --notices
bun audit
bun run audit:rust
bun run audit:secrets
```

codex-check 内旧用户托盘仍在运行，因此选择不含该 live exe 的正常 target。verify 新参数只允许工作区 target 内的目标，并拒绝替换对应 live exe；未按名字杀进程。

- Svelte check 0 errors/0 warnings；前端 build 成功。
- UI 模型 20 项、release 8 项、security 6 项、新 integration contract 3 项/168 assertions，通过。
- Rust 全量 **361 passed / 0 failed / 10 ignored**，旧 .status、--claude-event、原生来源、attention/未读/固定、远端/WSL 离线回归仍通过。未修改 SSH/WSL provider/wrapper，不运行无关真实远端工作负载。
- fmt、clippy -D warnings、开发与优化 GUI-subsystem 构建通过；没有构建/发布安装包或改版本号。
- 专项：半行/坏行、有界锁、同尺寸/更大替换的锚点检测、损坏检查点备份重放、双审批/歧义/冲突、Stop 后继续、旧轮次不覆盖标题、同批 A-end/B-start、已查看/重启重放、pending 上限仍可解除、心跳不续期、latest 快照不能绕过 outbox 提交边界。

阶段失败没有计为通过：首次 GNU smoke 缺相邻 WebView2Loader.dll；脚本过早点击尚未启用的按钮；隔离模式调用未注册窗口状态插件；压测向 Transition 错传单行而非完整任务视图；编辑期间 fmt 检查捕获新测试未格式化；一轮长压测后的后台 management CDP 会话超时。已分别修复依赖处理、脚本或代码并复跑；长压测后重新激活/连接管理窗口，最终全流程通过。未用真实用户配置“修好测试”。

## 原生 TaskPulse 验收，不是真实宿主验收

真实优化 GUI exe 复制到含中文、空格、&、apostrophe 的临时路径；真正重定向 stdin、PowerShell 固定转发、WebView2 CDP/Tauri IPC，无浏览器 mock。

已验证：

1. Hook stdout 空/fail-open；validator/diagnose 真正报错退出；超大、伪造身份、私密字段拒绝且不写 journal；锁超时有界；接收入口不进入 Tauri。
2. 原生界面添加来源、能力/禁用传输、保存即时生效；连接测试不创建任务；社区品牌可导入、选中、保存，不加 Rust 分支。
3. sandbox 指纹冲突拒绝、明确确认、TOML 注释/其他 Hook/后来修改保留、自有条目撤销。
4. 状态进入统一流水线；Stop 不结束；两请求只解除一个；短轮次、已查看/重启去重。
5. 真正深色 320×132 提醒；长标题不溢出；鼠标点击、悬停暂停、普通结果约 15 秒收起、批准不自动收起、队列轮换、旧 callback 安全、总开关、投递证据、无假任务/已读污染。

最终原生记录：src-tauri/target/integration-native-O9FAHe/result.json（10 项检查全部通过、管理窗口截图同目录）；GUI stdin：src-tauri/target/integration-bridge-nWeujQ/result.json（guiSubsystem=true、lockTimeoutVerified=true）；弹窗/截图：src-tauri/target/ui-smoke/native-notice-result.json 及同目录。均为 ignored 本机证据，临时路径不写真实 Hook。脚本每次产生独立目录。

验收优化 exe SHA-256：EF03EC779C3BFD8CF2B6A46F4D6EF6A0649E9932F3BFC32DECF34184B977F66C（src-tauri/target/ci-release/agent-tray.exe）。GNU 构建使用相邻的 build-owned WebView2Loader.dll，不宣称单 exe portable 或 MSVC 安装器已验收。

没有验证：四款宿主真实 Hook、真实任务最终结束、外部编辑器焦点、OS 系统 banner、MSVC fresh-runner 安装器。notice 使用 locked-desktop 降级，focusPreserved=null，不计为焦点通过。

## 并发与性能

Windows 10 Pro 22H2 x64（19045）、Intel i7-10700K @3.80GHz、16 logical CPUs，GNU 构建。本机磁盘/缓存/后台负载影响数值，不代表所有电脑/宿主时延。

20 个实际子进程 × 500 条 = **10000 混合事件/1000 个轮次**。包含 start、工具、审批/解除、阶段、stop intent、end、rename、heartbeat；断言序号唯一、全部消费、全部结束归约一次。结果：src-tauri/target/integration-stress-result.json。

| 指标 | 会话顺序锁 + 短时配额锁实测 |
|---|---:|
| 写入总墙钟 | 10553 ms |
| 写入 P50/P95 | 7.30/81.06 ms |
| Provider poll P50/P95 | 213.23/417.12 ms |
| 最大单个 writer 峰值 Working Set | 10694656 bytes（约10.20 MiB） |
| 锁超时重试 | **0** |

旧全局锁覆盖 fsync 时有 8 次超时重试、写入 P95 212.22 ms/总写入22700 ms。已拆锁，持久化 ack 唤醒下一有界批次，避免积压每128条额外等待完整轮询周期。Hook 仍是 fail-open/最佳努力；锁/磁盘/配额失败时事件可能未接收，应看诊断，不承诺任意故障下零丢失。

真实 WebView 另测 get_agents IPC + requestAnimationFrame，不把 poll P95 当 UI/宿主延迟。最终20进程/10000条有104样本，P50 **3.60 ms**、P95 **7.33 ms**；主 app 峰值 **49582080 bytes（约47.29 MiB）**、累计 CPU **28.53 s**。写入并完整排空的墙钟 **36930.61 ms**，此前每批等待轮询的实测约81594 ms。**不含 WebView2/生产者/控制器聚合峰值，不是整机 CPU 百分比**；采样覆盖写入阶段，不等同于真实宿主端到端时延或长期运行验收。

最终原生负载的20份 worker 结果另行汇总：10000条、lockRetries=0。

## 安全与发行门禁

- Bun 全量：1 low，@sveltejs/kit > cookie@0.6.0 / GHSA-pxg6-pf52-xh8x；moderate 阈值通过不是零告警，未盲目跨版本升级框架。
- cargo-audit 0.22.2 完整重试通过：数据库 e6f94f3760d5e6c54c1b05f204002263bc025524，1348 advisories；vulnerabilities 空，无 registry incomplete。曾因 registry 不完整退出1，未降标准，重试后才通过。
- Rust 仍有10条 unmaintained、2条 unsound 警告（glib/rand等既有跨平台依赖），清单：src-tauri/target/security-audit/rust-summary.json；不声称风险消除。
- gitleaks 8.30.1 全 Git 可达历史脱敏扫描通过，无忽略规则。提交后发现合成测试 turn_key 比较被误报为 generic-api-key，已改写表达式并只整理本次未推送的两个提交；原有用户/上游历史未改。最终文档提交后285个可达提交再次通过，不使用 allowlist 或忽略规则。
- sha2/toml_edit 直接复用锁图现有版本；最终只加所需依赖/feature，未保留 proc-macro-crate 意外降级。
- 未运行远端 CI/安装器、未 tag/发布。现有 draft-only release/双安装器门禁保留，只添加契约与 GUI bridge 检查。

## 剩余验收与使用

来源管理 → 其他 Agent → 选择/导入描述 → 查看能力 → 保存 → 测试/诊断 → **预览差异并明确确认** → 原宿主新会话实测。操作/标准生产者/磁盘恢复见[指南](AGENT_INTEGRATION_GUIDE.md)，身份与状态见[协议](AGENT_INTEGRATION_PROTOCOL.md)。

四款 native-tested、真实关联字段、长期负载、安全自动轮转、通用跨平台传输、原生精确 focus 均未完成，不宣传全面支持。本次没有改真实宿主 Hook，无真实配置需要撤销。只关闭自己的临时 PID/路径，旧用户托盘保持原样；查看新 UI 时先正常退出旧托盘，再启动最新构建/开发命令。

本地实现提交（本报告随最后文档提交保存）：

- fcb0040：v1 协议、journal/reducer/bridge、部分宿主描述与配置 recipe。
- 7170d52：有序 Provider、统一 Store/Attention/Transition、恢复与隔离。
- 944904b：逐项能力、其他 Agent 管理、导入/确认/诊断/撤销 UI。
- e68e23a：协议契约、Windows bridge/原生 WebView2/并发与通知验收、CI 检查入口。
- bd799da：提交确认后排空积压，长负载后重新连接管理窗口。
- ae9e2ca：接收端失败纳入来源 uncertainty，成功新事实恢复 health，不推断任务结束。

未 push、未创建/修改 PR、未发布；版本号没有改变。
