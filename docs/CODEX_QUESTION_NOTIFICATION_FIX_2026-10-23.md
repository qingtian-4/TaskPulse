# Codex 异步提问漏报与通知区分（2026-10-23）

## 原因核对

截图里存在两个独立会话：电商 Agent 显示提问卡片，而绿色提醒标题是 TaskPulse 项目的 `agenttray` 会话。通知记录也确认绿色提醒属于后者的正常结束，不能把它归为电商任务的结束事件。

电商 Agent 本身仍有真实漏报：旧解析器只识别同步 `request_user_input`，遗漏原生异步 `request_user_input_async`。后者立即返回 `{"accepted":true}`，随后 Agent 可以继续执行工具；用户回复则通过独立的原生用户消息到达，不能套用“工具返回就表示已回答”的规则。

只读真实日志证据（均为 2026-10-22 UTC）：

| 事件 | 时间 | 正确含义 |
| --- | --- | --- |
| 异步提问调用 | 13:00:34.349 | 发起问题，call ID 稳定 |
| 同 call ID 返回 `accepted=true` | 13:00:34.449 | 问题已发送，尚未回答 |
| 另一 TaskPulse 会话结束提醒 | 13:03:50.355 | 另一个任务的结果，不是此提问的状态 |
| 匹配 `questionItemId` 的原生用户回复 | 13:04:05.839 | 该问题被回答，可解除其待处理状态 |

## 实现

- 新增 `providers/codex/questions.rs` 请求关联模块，仅保留请求 ID、是否确认发送和未回答问题编号，不持久化问题／回答内容。
- 同步请求与异步请求分开处理；异步发送 ACK、其他命令结果与未知输出不能解除待回答状态，明确拒绝发送也不会制造待处理任务。
- 只解析原生用户角色、完整回复 envelope 和匹配的 call ID／问题编号，不搜索自然语言问号，不运行 Code Mode，不推断授权等待。
- 并行请求分别保留；重复调用、ACK 与重复回复不重开已处理请求。新 turn／明确中断按原有生命周期清理旧 turn 的关联。
- 普通 `task_complete` 不能把已确认但未回答的异步问题改成完成提示；用户回答后，依据此前是否有明确结束事件决定恢复运行还是展示普通结束结果。
- 本地与 WSL 的增量解析、SSH 快照共用关联规则，向既有通知链路提供完整 `input_request_ids`；保留原有状态文件、通知账本与重启去重。
- 明确问题及手动重开问题统一显示“需处理：等待输入”；不改变授权、额度耗尽、普通结束的独立语义，真正问题提醒仍无自动关闭计时器。

## 验证范围

先运行原有实现上的回归，确认异步请求漏识别、完成覆盖问题，以及新问题未跟踪等状态断言失败；修复后验证命令继续运行、多个问题、并行请求、错误／重复回复、拒绝发送、同步兼容及重启去重。

实际日志 `live_codex_async_question_replay` 回放成功：发送 ACK 之后的 96 条连续观察均保持 `WAITING_INPUT / Question`，直到匹配用户回复才恢复 `RUNNING`，turn ID 与开始时间保持不变。此测试只读历史原生日志，不触发新 Agent 请求、不修改宿主配置，也不将其宣称为完整宿主端到端验收。

`scripts/native-attention-regression-smoke.ts` 进一步断言真实 WebView2 中问题显示“需处理：等待输入”、任务标题匹配、没有完成字样且无计时器；普通结束则保留独立结果文案和计时器。输入使用隔离合成事件，与实际日志回放证据分开记录。

本轮验收：

- fmt、Clippy `-D warnings`、Rust 414 项通过／14 项显式 opt-in 跳过，custom-protocol 构建通过。
- 前端 Check 为 0 errors / 0 warnings；Build、UI Model 31 项、Docs 8 项、Release contract 8 项、Security contract 6 项通过。
- 两台已配置远程主机连接与原生元数据解析正常，174 条目录记录解析通过、预期项目目录存在；没有额外执行远程 Agent 任务。
- 隔离原生生命周期和产品 UI smoke 通过；最终证据分别保存在忽略目录 `src-tauri/target/ui-smoke/evidence/lifecycle-native-2026-10-22T17-05-10-995Z-jjup31` 与 `product-native-2026-10-22T17-05-18-808Z-xODGFT`，不导出用户项目或真实提问内容（UTC 时间戳对应北京时间 10 月 23 日）。
- 前端中等及以上漏洞门禁通过；完整 `bun audit` 仍有已有的 `@sveltejs/kit > cookie@0.6.0` 低危 GHSA-pxg6-pf52-xh8x，未在此功能修复中升级依赖。Secret audit 无发现；本次不是候选发布，未重新执行在线 Rust advisory 全量审计。

复现实际日志回放（需已运行的 WSL 环境或本地文件）：

```powershell
$env:CARGO_TARGET_DIR = 'src-tauri/target/codex-check'
$env:TASKPULSE_CODEX_QUESTION_REPLAY_FILE = '<实际 rollout 文件>'
$env:TASKPULSE_CODEX_QUESTION_CALL_ID = '<原生异步提问的 call ID>'
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_codex_async_question_replay -- --ignored --nocapture
```

## 已知边界

- 仅支持实际观测到的原生请求／ACK／用户回复格式。不把自由文本问题或 Code Mode 代码字符串当成真实请求；未知格式需要后续独立验证。
- 本次没有解决既有 WSL UNC 创建时间漂移导致的日志追赶延迟，不承诺事件发生到通知零延迟；不完整／过期数据不能充当新的结束证据。
- 未修改额度账号隔离 TODO、Agent 登录、真实 Hook、发布版本、远端分支或 Release。
