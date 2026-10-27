# 豆包桌面原生观测（实验）

“通用事件 / MCP 接入口”不会自动连接任意桌面软件。豆包桌面 2.26.10 的
Windows 工作任务另有原生读取路径：管理来源 → 添加 → **豆包 · 本机（实验）**，
确认启用并保存。不要求豆包主动调用 TaskPulse 工具，不修改豆包配置或审批策略。

默认只读 `%LOCALAPPDATA%/Doubao/User Data/Default/IndexedDB/` 中
`chrome_doubao-chat_0.indexeddb.leveldb` 的
`default-chat-im-protocol-store` 执行检查点及该记录引用的 blob。
自定义位置填 **IndexedDB 目录**。macOS/Linux 保留编译兼容和候选默认目录，
尚未实机验证豆包数据布局，不能宣称已适配。

## 能力和限制

- 观测当前正在执行的主任务；稳定 conversation ID 为任务身份，session ID 为轮次。
  不导入全部聊天历史，不把委派子任务重复计入主任务列表。
- 检查点未提供可依赖的会话标题时，展示“豆包工作任务 · 会话 ID 尾号”。
  不把提示词或完整回答复制到 TaskPulse。
- 明确的整轮 `FINALLY` 状态才可结束已经观察到的同一轮。
  单条消息完成、工具返回、连接断开和文件消失都不能代表整轮完成。
- 豆包可能在两次轮询之间清除结束检查点，**完成提醒不能保证送达**。
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
