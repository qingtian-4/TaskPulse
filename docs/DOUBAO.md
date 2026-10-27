# 豆包桌面原生观测（实验）

“通用事件 / MCP 接入口”不会自动连接任意桌面软件。豆包桌面 2.26.10 的
Windows 工作任务另有原生读取路径：管理来源 → 添加 → **豆包 · 本机（实验）**，
确认启用并保存。不要求豆包主动调用 TaskPulse 工具，不修改豆包配置或审批策略。

默认只读 `%LOCALAPPDATA%/Doubao/User Data/Default/IndexedDB/` 中
`chrome_doubao-chat_0.indexeddb.leveldb` 的
`default-chat-im-protocol-store` 执行检查点，以及 `conversations` 对象存储的
会话名称缓存和这些记录引用的 blob。数据库和对象存储编号从元数据解析，
不绑定某个账号的编号。
自定义位置填 **IndexedDB 目录**。macOS/Linux 保留编译兼容和候选默认目录，
尚未实机验证豆包数据布局，不能宣称已适配。

## 能力和限制

- 观测当前正在执行的主任务；稳定 conversation ID 为任务身份，session ID 为轮次。
  不导入全部聊天历史，不把委派子任务重复计入主任务列表。
- 按 conversation ID 关联原生会话名称；没有项目关联时显示“独立会话”。
  会话标题不写入项目字段，改名不改变任务身份、执行时间或通知状态。
  名称缓存缺失、损坏或相同 ID 出现冲突时，使用“豆包工作任务 · 会话 ID 尾号”。
  不导入缓存里的闲置会话，不把提示词或完整回答复制到 TaskPulse。
- 明确的整轮 `FINALLY` 状态才可结束已经观察到的同一轮。
  单条消息完成、工具返回、连接断开和文件消失都不能代表整轮完成。
- 在已检查的 2.26.10 实现中，主任务收到 `FINALLY` 时跳过最后一次检查点保存，
  随后删除任务。**仅靠当前检查点不能可靠发送完成提醒，提高轮询频率也不能修复**。
  未观察到明确结束时保留上次状态并标记“状态待更新”；不能伪装为已完成。
- **审批 / 提问弹窗尚未适配**。权限模式、工具名称、等待时长不被当作批准请求。
- 此路径依赖内部存储格式，标为推断/实验能力；版本变化、并发写入、损坏、
  超限或不支持的序列化格式会报告读取异常，保留已有任务，不产生结束通知。

## 读取边界

直接读取现有文件，不打开可写 LevelDB，不获取其 LOCK，不启动或重启豆包。
按 CURRENT/MANIFEST 选择有效表和日志，检查 CRC32C、事务序号、删除记录及
同一次读取前后的文件状态。支持观察到的 Blink v21 / V8 v15 数据、Snappy
压缩和外部 blob；有文件、总量、深度和节点预算。只缓存投影后的任务元数据。
支持 V8 对象/数组的共享引用；按原始对象编号重放，循环、无效引用和过量展开
会被拒绝。可选统计中的 NaN/Infinity 按 JSON 语义投影为 null，任务身份、
创建时间和状态仍独立校验，不把无效值当作零或结束状态。
不读取 Cookies、登录数据库或密钥，不调用豆包网络接口。
Windows 上豆包进程不存在时，来源只标记为不可用，保留已知任务而不宣告完成。

格式依据：[LevelDB 日志](https://github.com/google/leveldb/blob/main/doc/log_format.md)、
[LevelDB 表](https://github.com/google/leveldb/blob/main/doc/table_format.md)、
[Chromium IndexedDB 编码](https://chromium.googlesource.com/chromium/src/+/main/content/browser/indexed_db/leveldb_coding_scheme.md)。
对象引用依据 [V8 序列化实现](https://github.com/v8/v8/blob/main/src/objects/value-serializer.cc)。
豆包字段与 `ReplyFinish` / `FINALLY` 的对应关系来自安装包 2.26.10
的本地序列化实现和真实运行检查点，不是官方稳定集成契约。

## 验证

单元测试覆盖活动主任务、历史不导入、消息结束不等于轮次结束、数据消失、
轮询不刷新执行时间、日志校验/截断、压缩 blob、压实表、删除记录与文件轮换。
真实数据检查只读，日志只输出计数，不打印提示词、回答、账户或任务内容：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_doubao_snapshot -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_doubao_reference_checkpoints -- --ignored --nocapture
bun scripts/native-doubao-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --wait-seconds 120
```

真实任务验收需让豆包执行一个正常工作任务，确认 TaskPulse 显示豆包来源和
当前运行状态；另行检查完成、待更新和审批能力，不能用读取成功替代这些验收。
原生窗口脚本默认必须看到同一运行任务卡片持续显示至少 5 秒，并收到至少
3 次新的来源读取结果；空闲来源不算通过。`--startup-only` 仅用于启动/读取检查，
报告明确记录 `running_ui_verified=false`。历史检查点回归只输出读取计数，不导入任务。

### Windows 真实任务验收（2026-10-27）

豆包桌面 2.26.10 的一个真实工作任务已通过原生窗口验收。使用包含
`a75cd26` 解析修复的本地 0.2.0-alpha.8 构建；同一任务卡片以“运行中”
持续显示超过 5 秒，取得 11 次窗口采样和 6 次新的来源读取，执行进度发生更新。
正在使用的托盘运行相同二进制，其真实状态文件也记录了这条运行任务，
`stale=false`。隔离验收配置已自动清理，未导入历史检查点或合成任务。

本次确认的是运行任务的采集和显示；审批提醒、完成提醒、macOS/Linux
豆包适配仍不在已验收范围内。

### 审批和完成的后续接入条件（2026-10-27）

本地安装包的 `async-message-node.js` 与 `58484.js` 中存在明确的人机确认卡片：
`block_type=10080`、`quick_reply_block.scene=2`，状态枚举为 Default=0、Available=1、Hide=2。
危险操作的允许/拒绝动作分别为 1010/1011，部分请求通过
`schema_payload.server_option.callback_id` 关联。它们是内部实现字段，不是已验证的
落盘协议：界面还存在只在内存中隐藏按钮的路径，不能仅凭旧缓存里的 Available
认定请求仍未处理。因此本版本没有启用这些字段对应的审批弹窗。

下一次自然出现审批时，需只读核对请求前、待审批、处理后的连续检查点：确认请求 ID
与本轮任务绑定，并有相同 ID 的明确解除记录。之后才能接入现有审批队列、去重与
关闭逻辑；如果解除信息只在内存中，就需要宿主提供完整事件接口，不能靠超时关闭。
本轮没有真实审批样本，不需要为了测试而执行不必要的高权限操作。

可靠完成同样需要额外的明确事件来源：主任务 `SSE_REPLY_END` 的 ReplyFinish（3），
与会话、本轮 session ID 绑定，先写入可重放日志，再由 TaskPulse 确认与去重。
消息结束（1）、回答段落结束（2）、子任务结束、`agentTaskDone`（包括中断路径）和
工具退出均不满足该条件。已检查的本机日志没有提供这条可用记录；当前没有可直接
启用的官方持久化事件开关。只有宿主事件/插件接口经验证后，才适合新增观察桥接。
通用 MCP 接口的存在本身不能补出这些事件，也不能作为豆包已经适配的证明。

### 会话名称修复验证（2026-10-27）

- `scripts/verify.ps1 -Frontend` 通过：497 项 Rust 测试通过、24 项外部环境测试
  忽略，格式/Clippy、Svelte 检查、前端构建、UI 模型及集成桥接检查通过。
- 真实会话名称缓存读取通过；当前原生执行检查点为空。原生窗口启动与来源读取
  验收通过，尚未完成这一版的真实运行卡片标题验收。保留的历史 WAL 样本测试
  因当前日志没有执行样本而不满足前提，没有把它计为通过。
- 回归覆盖动态数据库/对象存储编号、改名、名称冲突、损坏/删除的名称缓存、
  不导入闲置会话，以及明确结束事件在改名、状态反复和通知状态恢复后只提醒一次。
- 前端中风险以上审计门槛通过；完整 `bun audit` 仍报告 3 项低风险告警
  （SvelteKit 2 项、cookie 1 项）。本次未更改依赖，未重新进行 Rust 依赖审计。
  这些结果不代表豆包审批/完成适配或新的安装包发布已经通过验收。
