# R1–R4 修复与验证报告

日期：2026-10-21；基线 bf55e8f，分支 codex/desktop-discovery-visibility，产品版本仍为 0.2.0-alpha.7。读取 AGENTS.md、执行任务书及本轮审查报告后，先加入表达正确行为的正式回归并观察失败，再修实现；没有用兼容说明替代代码修复。

## Finding 结果

| Finding | 根因及实现修复 | 正式验证结果 |
|---|---|---|
| R1 混合来源通知丢失 | 原来全量 apply 后由单个 observation.notify 丢弃整批。现在 Transition 只消费当前 GenericEvent 候选，保存其他通用 baseline；静默仅抑制自身事实，原生任务/host 仍正常投递；latest 不越过其他 outbox | `review_r1_mixed_pipeline_replay_is_silent_only_for_its_own_fact` 通过。实际 journal/两个 GenericEventProvider → Store → Attention → watcher 生产分发函数 → 持久 Transition；混合原生 Codex 快照、主机断连、旧来源重放和另一来源新事件；检查重启/ack 后不重复 |
| R2 TRAE 异步结束归错轮次 | idle_prompt 改映射 observation.unassociated，明确不赋 ctx.turn、不结束、不续活跃；无可靠 turn 的终态同样降级。请求/结果须明确原生 turn 或唯一当前调用/签名；历史复用降级。明确旧 turn 请求的解除只释放旧资源，不影响当前任务 | A-submit/tool/result/Stop → B-submit/tool → A-late-idle 正式回归通过，B 保持 RUNNING/tool；另有旧调用、复用调用/签名、Kimi 明确旧 turn 解除测试通过。TRAE final_end 在代码和矩阵均为 unsupported |
| R3 超限记录阻塞恢复 | 标准接收端在会话写锁下执行 pending128 预算；拒绝新的超限请求，但放行恢复。已接收的旧超限/无效 envelope 在同一原子 checkpoint 保存有界 quarantine 索引、累计数量和 cursor；原 journal 不删除；单会话故障隔离。seen 饱和允许已有执行幂等解除/返回/终态，不清账本 | 旧 journal 的129请求→解除→结束及健康 peer 均消费；重启不重投，诊断保留1条隔离。接收端拒绝129并接受解除/结束；100000去重边界恢复、旧 turn 清理、不可读的单会话 checkpoint 不阻断 peer，均通过 |
| R4 来源删除后不能撤销 Hook | hook-installation.json 独立于 source registry；旧 hook-ownership.json 可严格解析固定包装命令恢复身份，不执行命令。来源页独立 Hook 登记/预览/确认撤销；只移除登记条目，包含已退役事件名；不会整体回滚备份或删其他 exe 的相同 argv | 原生 WebView2：安装→删除来源（宿主文件不变）→只留下旧 ownership 格式→UI 找到来源已移除登记→勾选确认撤销，保留外来 Hook/注释；重启仍保留未查看/待处理。正式单位测试覆盖旧登记、Unicode/引号及外来/空事件保留 |

R1 的混合流水线用原生 TaskSnapshot 契约，不是实时登录某个 Codex 宿主进行测试。R2 的宿主载荷也为合成数据；这些验证均不升级为宿主 native-tested。异步 Notification 及其 idle 缺调用关联的边界再次对照了 [TRAE 官方说明](https://docs.trae.cn/ide_hook-configuration-reference)。

## 先失败、再修复的记录

- 新增 R1/R2/R3 正式测试在旧实现上为 **0 passed / 3 failed**：native end 被静默吞掉、B 被改为 WaitingInput、超限 session 无法结束；dedup 饱和的追加回归也先失败。
- 原生脚本先增加删除来源后撤销的回归，再运行旧优化 exe；在来源删除后的预览处失败，证据：`src-tauri/target/integration-native-gYa7H4/failure.json`。
- 测试后来扩展至实际持久 outbox、两个通用来源、故障 peer、重复调用和旧格式安装恢复。保留临时 review probes，不将 target 内临时源码作为正式测试交付。
- 中途新诊断断言查错 fixture source ID，以及一处未格式化导致 verify 拒绝，均修正后重跑，没有把失败算通过。

## 全量和原生验收

执行 `scripts/verify.ps1 -Frontend -TargetDir src-tauri/target`；最后两处 R4 保留性细节改动后，再执行 fmt、clippy -D warnings、全量 Rust、开发及 ci-release 构建，随后重新跑最终原生 smoke。未替换 live 用户 exe，未按进程名批量结束程序。

| 检查 | 本轮结果 |
|---|---|
| Rust 全量 | **373 passed / 0 failed / 10 ignored**（原361项 +12项正式回归）；ignored 含宿主/SSH/WSL实测及显式压测入口 |
| fmt / clippy -D warnings | 通过 |
| Svelte / 前端生产构建 | 0 errors / 0 warnings；构建通过 |
| UI 模型 / release / security / integration contract | 20 / 8 / 6 / 3 项通过；版本一致，未发版 |
| GNU 开发及优化 GUI exe | 构建通过；优化 exe 实际 PE GUI subsystem 检查通过 |
| GUI bridge 原生 stdin/校验/fail-open/锁超时 | 通过，`src-tauri/target/integration-bridge-e6gnw2/result.json` |
| 原生 WebView2 完整流程 | **10项检查通过**，`src-tauri/target/integration-native-ymoR8D/result.json`；管理页截图同目录 |
| 原生提醒 | 320×132 深色、长标题/点击/暂停/队列/旧 callback/主开关通过；普通提醒自动收起 **15111ms**，明确批准不自动收起；`src-tauri/target/ui-smoke/native-notice-result.json` |

最终原生命令：

```powershell
bun scripts/integration-bridge-smoke.ts --exe src-tauri/target/ci-release/agent-tray.exe --require-gui
bun scripts/native-integration-smoke.ts --exe src-tauri/target/ci-release/agent-tray.exe --stress-worker src-tauri/target/debug/deps/agent_tray-e3b58ad33ea110c0.exe --notices
```

实测优化 exe SHA-256：`D769F7D844BD0EDD01487F2F9EBC3A08AF61DE4411B80F5FD786982A8112791C`。GNU 验收依赖相邻 WebView2Loader.dll，不宣称单 exe portable 或 MSVC 安装器已验收。

20个实际子进程/10000合成事件，排空墙钟34083.81ms。真实 WebView IPC + animation-frame 103样本 P50/P95 为2.95/8.80ms；app 峰值 Working Set 49893376 bytes，累计CPU25.45s。不含全部 WebView2/生产者进程聚合资源，也不等于真实宿主端到端时延或长期运行稳定性。

一次原生 notice 验收失败：最终计时通知只有 `user_dismissed`，没有 `auto_dismissed` 记录，不能据此断言产品提前超时。smoke 已加强为按同一提醒ID核对实际可见→自动收起证据，而非只从较晚的 CDP 等待起点测量；没有放宽14秒下限。完整重跑通过。失败记录保留于 `integration-native-bneUNA/failure.json`，未覆盖为成功。

## 审计和未验证项

- `bun audit` 全量仍有1条 low：cookie@0.6.0 / GHSA-pxg6-pf52-xh8x；moderate门禁通过不是零风险。
- Rust审计通过，无漏洞、无 registry incomplete；仍有10条 unmaintained、2条 unsound 警告。证据：`src-tauri/target/security-audit/rust-summary.json`，未忽略 advisories。
- gitleaks 校验固定版本、全可达Git历史脱敏扫描通过；最终文档提交后再次扫描，未用忽略规则或基线掩盖问题。
- 未验证：四款真实宿主 Hook、真实最终结束闭环、外部编辑器焦点（focusPreserved=null）、OS banner、MSVC fresh-runner 安装器及长时间真实宿主负载。未改原生SSH/WSL Provider，本轮不冒充重新完成其 ignored live 验收。
- 磁盘满、锁/写入失败仍会 fail-open 丢掉未接收事实；达到32/128MiB后需保留证据人工处置，无自动轮转。隔离索引每session最多64条，原 journal 保留。没有清空 attention/recent/journal 或重置 notifications 账本。

## 四款能力与下一步

| 产品 | 已实现 | 实机验证版本 | 未支持/限制 |
|---|---|---|---|
| Kimi Code CLI | 明确turn开始；工具；关联请求/解除；带原生turn的失败/中断；标题/心跳；TOML预览撤销 | **无** | 正常最终结束、提问、精确跳转；缺可靠turn终态只记录观察；字段仍需真实样本 |
| TRAE 国内 IDE | 工具派生轮次；唯一关联请求/结果；JSON预览撤销 | **无**；本机仅安装元数据3.5.62 | **正常最终结束unsupported**；标题、纯思考开始、整轮失败、无关联提问/批准等 |
| ZCode Desktop | 工具派生轮次；唯一调用/输入关联请求与结果；process argv预览撤销 | **无**；本机安装3.11.2 / exe ProductVersion3.11.2.6792 | 正常最终结束、标题、纯思考开始、整轮失败/提问/精确跳转；不改全局Hook开关 |
| WorkBuddy Desktop | 候选描述/限制展示 | **无** | 无宿主事件映射、安装与端到端验收；不以CodeBuddy CLI替代 |

详见[兼容矩阵](AGENT_COMPATIBILITY.md)、[使用与恢复](AGENT_INTEGRATION_GUIDE.md)、[协议](AGENT_INTEGRATION_PROTOCOL.md)。本轮只准备了 [ZCode 具体差异和验收方案](ZCODE_HOST_ACCEPTANCE_PROPOSAL.md)及其[完整JSON提案](proposals/zcode-hooks-2026-10-21.json)：真实目标配置不存在，持久TaskPulse路径尚未创建，尚未添加真实来源或写真实Hook。**等待明确确认，不自动继续。**

本地实现提交：12bd74f（R1–R3流水线/关联/恢复），0fe5b0a（R4安装登记及UI闭环/原生测试）。另有本次文档提交。用户原有三个未跟踪文档保留；无 push、PR、tag 或发布。
