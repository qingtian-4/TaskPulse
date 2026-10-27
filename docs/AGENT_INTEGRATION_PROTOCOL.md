# 通用 Agent 事件协议 v1

实现入口：`integrations/event-v1.schema.json`、`src-tauri/src/integrations/`。本协议与旧 `.status`、`--claude-event` 并行，不迁移或重写旧来源。标准事件和 MCP stdio 都绑定已保存的本机来源；Windows/macOS 共用实现，Linux 原生验收仍待完成，不含通用 WSL/SSH/HTTP 传输。

## 生产者输入与身份

每次启动已安装的 `agent-tray.exe --integration-event --source <来源ID> --adapter <描述ID>`，通过 UTF-8 stdin 传递**一个 JSON 对象**，随后关闭 stdin。不传命令参数中的完整 JSON，不启动 WebView、托盘或 watcher。内置接收端不需要额外安装 Python/Node。

macOS 使用安装后的 `TaskPulse.app/Contents/MacOS/agent-tray`，其他参数相同。MCP 入口为 `--integration-mcp --source <来源ID>`，通过换行分隔的 JSON-RPC 持续收取 `taskpulse_report` 调用；工具参数不含 `schema_version` / `agent_id`，接收器从绑定来源补充，不允许模型覆盖来源。其余 session/turn/event/request/call 关联规则与本协议一致。MCP 断开不产生结束事件，报告证据为 `self_reported`。配置和宿主边界见[接入指南](AGENT_INTEGRATION_GUIDE.md#支持本机-mcp-的桌面或-cli-宿主)。

```json
{"schema_version":1,"agent_id":"custom-agent","session_id":"stable-session","turn_id":"stable-turn","event_id":"stable-event","kind":"turn.started","payload":{"title":"真实会话标题","project":"项目名"}}
```

此例仅说明格式，不是某个真实 Agent 的采集数据。

| 字段 | 规则 |
|---|---|
| schema_version | 固定整数 1；未知大版本拒绝 |
| agent_id | 必须等于已确认来源和 manifest 的品牌 ID，与模型供应商无关 |
| session_id | 稳定会话 ID，不得使用标题、PID 或时间替代 |
| turn_id | 执行/工具/输入请求事件必填；同一轮不变，新执行换 ID |
| event_id | 稳定事件 ID；重试必须复用；同会话同 ID 只归约一次 |
| kind | 下表枚举，不接受任意状态字符串 |
| occurred_at | 可选 UTC epoch 毫秒，最大 253402300799000；展示时间不超过接收时间，不用时间戳代替因果关联 |
| request_id / call_id | 请求/调用身份；请求及解除必填 request_id，工具事件必填 call_id |
| payload | 可选对象，仅允许 title、project、project_path、model、phase、activity、request_kind |

原始 ID 允许 Unicode、冒号和斜杠，非空、无控制字符、最多 512 UTF-8 字节。实际路径与任务身份组件使用 SHA-256；原始 ID 不拼接文件名。展示字段每项最多 2048 UTF-8 字节，无控制字符。拒绝未知顶层/payload 字段；不接收 prompt、tool_input、output、凭据或任意执行代码。`activity` 只能是最小安全标签；生产者仍有责任脱敏标题/活动中的隐私内容。

任务身份为 `integration:<SHA256(source)>:<SHA256(agent)>:<SHA256(session)>`。标题/项目独立；重命名不新建身份、不重置已查看/固定/通知。新品牌不增加 Rust 枚举。

接收端绑定 source_instance_id，生成 received_at、单会话 sequence、journal_epoch、adapter_version、evidence。生产者不能注入这些 envelope 字段。sequence 表示接收顺序，不代表宿主因果顺序。

## 生命周期

| 事件 | 行为 |
|---|---|
| session.discovered / session.renamed | 发现/展示更新，无执行、无完成提醒 |
| turn.submitted | 仅记录提交意图，不证明已接受或开始执行 |
| turn.started | 明确新执行，RUNNING；重复/旧轮次 start 不复活任务 |
| phase.changed | thinking / tool / responding / awaiting_model / awaiting_tool；始终运行，不解除请求 |
| tool.started / tool.finished | 按 call ID 维护并发工具；工具失败/返回不结束整轮 |
| input.requested | request_kind 必须是 approval / question；保存独立 pending 请求 |
| input.resolved | 仅同 turn/request；提供 call ID 时不得与已关联调用冲突 |
| turn.stop_requested | 准备停止，不等于最终结束，不通知完成 |
| turn.completed | 明确本轮结束 → waiting_input / turn_ended，结果待查看，不宣称产出正确或整个项目完成 |
| turn.failed | 来源明确报告整轮失败；不捏造退出码 |
| turn.interrupted | waiting_input / turn_ended + outcome=interrupted，显示“已中断/非成功” |
| session.closed | 会话关闭，不单独推导整轮成功 |
| heartbeat | 更新接收证据/可能携带标题；不延长 execution activity、不续期审批有效性 |
| connection.test | 写独立测试通道，不入 journal、任务、未读或成功历史 |
| observation.unassociated | 有宿主事实但无可靠 turn/call 关联；不结束、不创建/解除 pending、不续活跃期，可无 turn_id |

迟到的旧 turn、已终结 turn 的阶段/工具事件不覆盖新轮次；旧 start 的标题也不会倒灌。新 start 将当前展示切到新轮次，但不会用其工具事件解除旧轮次请求。多个 pending 独立保存；无 request/call 唯一关联不猜测“最近一个”。标准 `tool.finished` 不自动解除审批；内置 native 映射只有唯一、明确的关联结果才产生 `input.resolved`。

CPU、静默、进程消失、慢工具、模型等待、普通 idle 文案均不能推断 Agent 结束或需要授权。当前 Kimi/TRAE/ZCode 不提供可靠最终正常结束能力。TRAE 异步 idle_prompt 不带可靠轮次 ID，记录为未关联观察，不能把当前 ctx.turn 当成它的来源；不能设置几秒静默补齐它。

## Journal、恢复与水位

默认位置（`TASKPULSE_HOME` 仅用于显式隔离运行）：

```text
~/.taskpulse/integrations/
  manifests/<adapter-sha256>.json
  <source-sha256>/
    writer.lock
    writers/<session-sha256>.lock
    events/<session-sha256>.jsonl
    state/<session-sha256>.json
    state/<session-sha256>.lock
    hook-ownership.json
    hook-installation.json
    connection-test.json
    last-error.json
```

- 会话级 OS 锁保证序号/尾部恢复/追加持久化；短时来源级 OS 锁只覆盖磁盘配额检查与可见追加，各自等待最多 2 秒。不同会话可并行 fsync，避免把慢磁盘同步全局串行化。stdin 等待最多 2 秒，输入最多 64 KiB；单行最多 73728 字节。
- 半行尾部不移动读游标，下次追加前截去未提交尾部。完整坏行有界跳过/诊断；损坏尾行拒绝继续写入，须导出检查，不能猜测 sequence。
- 游标含 epoch、sequence、offset 及最后已消费行的锚点 SHA；截断/替换会重新读取，保留 event ID 去重，不只看 byte offset。
- 单次读最多 256 KiB/128 行，每来源一次最多 128 个 observation，最多检查 256 个会话文件并轮转让出。消费持久化/确认后唤醒该来源继续排空，空批次不自旋；不为每个积压批次额外等待完整轮询间隔。积压标注来源降级，不变成任务失败。
- 检查点原子保存 reducer + cursor + durable outbox。损坏检查点先备份到 `.corrupt.bak` 再重放 journal；备份失败拒绝覆盖。outbox 在 Attention/Transition 持久化成功后，按会话批量确认。
- 已接收的超限/无效 envelope 在同一检查点中保存有界 quarantine 索引和累计数量后，cursor 才前移；原 journal 保留，后续恢复事件继续消费。单会话读写失败不使其他会话回滚/停止。原始坏行仍按既有有界读取逻辑处理，不把它当正常任务事件。
- 有序消费 `start A → end A → start B`，A 的结束事实不会被 latest B 覆盖。弹窗动作保留 turn 身份，不能误作用于 B。
- 每会话日志 32 MiB、每来源 128 MiB，seen 事件 100000、轮次 4096、pending 128。pending 接收预算在会话写锁内更新；超限拒绝新请求，但解除/结束不占新槽。seen 饱和不清旧账本，已有明确恢复事件依靠请求/终态集合幂等处理；磁盘/持久化故障仍可能拒绝包括恢复在内的写入。当前不自动轮转/压缩，长期运行需监控磁盘；见[恢复操作](AGENT_INTEGRATION_GUIDE.md#恢复与磁盘上限)。
- 首次保存来源写 accept_since_ms，只消费确认后的事件。停用时接收入口 fail-open 丢弃并记录 source_disabled，不补录停用期间事件；恢复/热重建旧 outbox 时恢复关注但静默，不回放旧弹窗。

通知沿用 TransitionEngine/notifications.json；通用来源另存稳定 task/turn/status/request 去重账本。关闭提醒不等于已查看。恢复 pending 不代表它仍有效：手动重开弹窗需要 30 秒内匹配请求的明确观察、来源健康和非自报证据，普通心跳不能续期。

每个 observation 的 notify 标志只决定其自身事实是否投递。Transition 在消费该事实时仅选择这一 GenericEvent 候选；其他通用来源的 latest 不能越过 outbox。原生任务和 host 同批仍正常处理并投递，未选中的通用 baseline 保留。不得计算全量事件后按某一个 replay 的 notify 丢弃整批。

Hook 安装登记独立保存于 hook-installation.json（不含宿主模型/代理设置）；source registry 删除后仍可预览、确认撤销。旧 hook-ownership.json 可通过严格解析 TaskPulse 固定包装命令恢复 source/adapter/exe，**不会执行解析内容**。宿主路径始终来自内置 recipe，不由导入 manifest 或事件指定。

持久化去重与操作系统投递不是事务：提交后、实际弹窗前崩溃可能没有弹窗，仍可从持久关注/铃铛找回；不承诺 OS exactly-once。持久化失败保留 outbox、不确认消费；来源故障不当任务失败。

## 声明式适配与信任边界

`--integration-host-event` 是单独的原生载荷转换入口。manifest v1 仅包含品牌/版本信息、受限 JSON pointer、事件枚举、简单等值条件及逐项能力。不能携带 shell、配置路径、下载/安装或任意跳转命令。模板见 `integrations/community-template.json`；Rust Manifest::validate 是导入验证器。

导入描述可接入兼容载荷的新品牌，无需升级 TaskPulse。需要复杂解析、专用存储/transport 或不同因果协议时仍需代码适配。导入文件永远不能获得宿主安装权限，也不能自称官方/实机验证：落盘和读取均降为 self-reported，清空 tested_versions。标准输入同样是 self_reported。内置原生映射的 documented_hook 只表示渠道，**不是身份认证，也不是该宿主版本实测**。

宿主缺稳定事件 ID 时使用本地投递序号派生 ID，不用完整 JSON 哈希把两次相同请求合并；但无法识别宿主重复投递，可能留下多个未解决请求。缺字段、未知事件或版本不匹配必须报告限制，不能伪造关联。

## 错误与兼容

- 正常 Hook 模式 stdout 为空、退出 0，错误仅写有界代码/时间，不返回审批决定、不注入上下文。
- `--integration-validate` 输出机器 JSON，非法输入退出 1；只校验协议，不声称已绑定来源。
- `--integration-diagnose --source <id>` 输出机器 JSON，配置错误退出 1；诊断含接收时间、最新采样事件类型、历史错误，不含载荷。GUI 诊断另包含当前 provider health。
- 协议/manifest 未知大版本拒绝；持久 TaskSnapshot 新字段 optional/default，旧来源、旧 Hook、attention/recent/notifications 保持可读。
