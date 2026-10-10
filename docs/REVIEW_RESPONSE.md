# 对 Claude 审查的采纳与执行记录

日期：2026-10-10。输入为用户提供的 `REVIEW_HANDOFF.md`；不修改该审查原文。
本次目标是可靠地发现和监控已选择的 coding agent，而不是导入全部历史、
承诺所有 Agent 通用接入，或者为了采纳意见而替换现有生命周期语义。

## 已实施

| 审查项 | 实施与边界 |
|---|---|
| Codex 活跃日志全量重读 | 保存 offset、部分行与折叠状态，只解析新增完整 JSONL；截断/替换重置。首次仍需读取历史日志。新增跨 turn 与全量解析等价测试。 |
| DSH 标题分片重复解析 | 分片按 mtime/size 缓存；改名仅更新元数据，不再使日志解压缓存失效。实际日志改变时仍需解压，不冒充增量 zstd 解码。 |
| 文件事件立即发旧数据 | 事件唤醒 worker，等待新结果；保留 100ms 收集结果与 2s 维护节奏，不把维护闸门误认为采集间隔。 |
| 托盘/面板呈现 | 中文菜单、状态计数 tooltip、计数顺序与任务列表一致；正常行不重复展示采集时间，异常时间仍可见。详情“最近消息”不假装正式最终结果。 |
| 管理窗口导致面板关闭 | 打开管理期间保留面板/选择；管理关闭后恢复通常失焦行为。原生验收发现关闭确认需要额外 destroy 权限，权限仅授予 management 窗口。 |
| 首次启动与空窗闪烁 | 首次未配置自动显示引导；窗口等待 UI 初始快照再显示；读取异常也显示错误。快捷键注册冲突记录警告，不终止启动。 |
| 多余归档 IPC / 旧组件 | 归档列表变化才发独立事件，主列表事件不再重复查询归档；删除四个无引用旧组件，可由 Git 恢复。 |
| 提醒细节 | 保留窄条深色、独立队列、不抢焦点和关闭不已读规则，仅调整测试标记与关闭按钮间距。 |
| fork 身份和文档 | 独立 app ID、移除上游 macOS 签名身份和固定 Team ID；更新 CLAUDE.md、CONTRIBUTING、README 和英文入口。旧 agent-tray 二进制名称保留兼容。 |
| 打包 Hook 脚本 | resources 显式带入 scripts/hooks；解析运行时资源目录，同时保留源码开发回退。绝不重新打包托盘 PNG。 |

app ID 改为 `app.taskpulse.desktop`，可能需要重新允许系统通知，窗口状态目录也
按新身份建立；`~/.taskpulse` 中的关注、未读、配置和通知去重记录不迁移或删除。
macOS 签名证书/Team ID 必须由 fork 自己提供，不意味着本次已验证 macOS 发布。

## Claude Code 接入：修正后采纳

**Agent ≠ 模型 ≠ 来源实例。** VS Code 的 Claude Code 使用 C Switch 代理和 DeepSeek
模型时仍属于 Claude Code。原生消息上报的 model 单独展示，不据此推断代理背后
真实服务商，也不创建 DeepSeek Harness 任务。

- 支持只读发现 `CLAUDE_CONFIG_DIR`（默认 `.claude`）下的原生会话、标题与上报模型。
  仅读取 root 会话 JSONL、session JSON、IDE lock；不读取 `*.key`、认证配置或代理凭据。
- 已知 Agent 的真实数据出现后，面板/管理窗口显示候选；用户添加、保存后才启用。
  新安装的未知 Agent 不会因“存在某个进程”就被宣称已经可靠支持，需新增适配器
  或接入通用 Hook/status 协议。新来源不会批量导入空闲历史。
- 原生私有格式只提供推断证据；损坏/未知数据不生成演示任务，旧未结束日志没有
  活进程时不作为新运行任务导入。正在观测的任务失去证据则保留 stale，不判成功。
- 标题手动重命名优先；项目与标题独立；模型、标题更新不改变稳定 session ID。
  一个 VS Code 项目窗口可以有多个会话，不能用同一个窗口 PID 把它们相互去重。
- 可另行确认安装官方生命周期 Hook：UserPromptSubmit、工具事件、PermissionRequest、
  Stop、StopFailure 和 Notification。首次备份、仅合并自有 Hook，可撤销；已有
  statusLine、Hook、代理设置保留。读取配置失败或并发修改时拒绝覆盖。
  使用官方 exec-form 的 command/args，避免 Windows Bash/PowerShell 引号差异。
- Stop 映射为 WAITING_INPUT / turn_ended；明确 API 失败映射 FAILED，不编造退出码。
  Hook 没有官方 prompt UUID，以相邻原生开始时间做有限关联；原生格式和版本变化
  仍需持续验收，不宣称私有字段是永久协议。
- VS Code 跳转只承诺聚焦匹配的项目窗口，不承诺切换到精确对话 tab。

不能采纳“statusLine 单独作为状态主来源”：官方说明包含 session/model/workspace
等展示元数据，但没有完整生命周期边界。它可用于后续 CLI 展示优化，不能替代
结束/失败/授权事件；CLI 说明也不能直接证明 VS Code 状态栏采用同一套接口。
依据：[statusLine](https://code.claude.com/docs/en/statusline)、
[Hook 事件与执行形式](https://code.claude.com/docs/en/hooks)、
[VS Code 集成](https://code.claude.com/docs/en/vs-code)。

DetectionConfidence 继续表示单条任务状态证据（exact/hook/inferred），不新增重复
枚举，也不将其当作整个接入的健康等级。数据源健康、时效和任务证据分别展示。

## 暂缓的建议与下一步

| 建议 | 暂缓理由 / 验收要求 |
|---|---|
| 关注记录异步落盘 | 未证实是本机主要瓶颈。需先记录 reconcile/序列化/磁盘耗时和阻塞 P95，再设计写合并、退出 flush 与手动已读操作的持久化边界。不能因提速丢未读/固定状态。 |
| WSL 常驻事件 worker | Windows 原生目录监听不能直接等同于 WSL 事件流。后续设计进程复用/事件桥、断线重连与轮询降级，并实测启动开销。 |
| SSH 常驻 push | 涉及远端协议、部署与连接恢复，不能仅缩短轮询冒充解决。保持现有 SSH 超时不判完成；后续按真实事件到 UI/提醒的 P95 验收。 |
| 进程扫描进一步优化 | 两次 10s 窗口都未返回，仍是明显弱项；独立原生适配器不依赖它。后续 profile 每进程 CPU/环境查询并批处理，避免逐 PID 外部命令。 |
| 开机自启 | 保留为明确用户可选的后续功能，本次不改变系统启动配置。 |
| 无限制自动启用 Agent | 与确认来源/隐私/无历史导入规则冲突。使用真实候选 + 显式启用，而非扫描到就监控。 |

## 性能与验证

优化前后用相同本机 `--diagnose --diagnose-seconds=10` 采样：

| Provider | 首次结果：前 → 后 | 末次采集：前 → 后 |
|---|---|---|
| Codex | 2331ms → 2741ms | 201ms → 173ms |
| DSH Desktop | 813ms → 709ms | 7ms → 2ms |
| DSH WSL | 1622ms → 1621ms | 191ms → 212ms |
| Hermes | 101ms → 101ms | 3ms → 2ms |

这些是单次窗口数据，有负载/缓存差异，**不是 P95 或 Agent 结束到提醒的端到端
延迟，也没有证明 Codex 冷启动变快**。可确认的结构收益是追加日志不再全量重读，
标题改名不再触发 DSH 解压。诊断产物在被忽略的 target/ui-smoke 中，不入库私人会话日志。

验证结果：

- fmt、Clippy（零 warning）、Rust 全量 276 通过 / 5 项 opt-in 跳过；UI 模型 9 通过，
  Svelte check 零 error / warning，前后端构建成功，托盘运行期间隔离构建无文件锁。
- 本机 Claude 真实会话只读 smoke 通过：原生标题/项目/DeepSeek 上报模型正确；
  实际 `isApiErrorMessage` 认证错误归类 FAILED，不显示 `<synthetic>` 为模型。
- 真实 SSH smoke 通过，两台配置主机连接正常，发现 149 个原生会话；不将该
  catalog 全部复制到主列表。实际返回的运行范围内任务保留原生名称和会话 ID。
  原生 IPC 另查到 Server3 / HyperMem 的 53 条目录记录，包括“研究 HyperMem 记忆机制”
  与“创建Agentic Retrieval分支”；仅元数据记录显示 unknown，不把目录存在当作运行证据。
- 实际 WebView2 主面板、来源即时启停/恢复、刷新、旧关注、未保存关闭保护通过；
  加强测试验证确认后管理窗口真正消失，不仅检查按钮点击返回。
- Claude 候选在真实管理 UI 添加并保存，其他来源范围不变；启用 Hook 的确认弹窗
  取消和 confirmed=false 拒绝均通过，实际 Claude settings 文件前后完全一致。
  按用户先前明确的添加要求，保留 TaskPulse Claude 本机来源启用；全局 Hook 未安装。
- 提醒原生验收复测通过：320×132 深色窄条、不抢焦点、队列、15 秒收起、长标题、
  消息卡片点击、关闭不已读、主开关清除，无假任务写入。首轮悬停断言曾失败，
  独立复测通过；未据此声称发现并修复新的悬停产品缺陷，仍需留意交互测试波动。

未验证：新 Windows 用户档案的首次启动、macOS/Linux 运行/签名发布、真实 Agent
结束到提醒的 P95、全局 Hook 经 Claude 实际调度及精确对话 tab 跳转。原生 Hook
调度需用户另行确认后验证，本次不会擅自安装或发送消耗模型额度的测试提示词。
