# 接入其他 Coding Agent

通过标准事件或 MCP，把支持外部事件的 Agent 连接到 TaskPulse。先查看[兼容说明](AGENT_COMPATIBILITY.md)，选择合适的接入方式。事件接入使用本机进程通信；WSL 和 SSH 请使用各自的 Codex 来源。

## 从界面接入

1. 托盘打开设置 → 来源管理 → 新增类型“其他 Agent · 通用事件”。选择内置描述，或展开“导入社区 / 自定义 Agent 描述文件”。
2. 查看每项能力、限制、验证等级和实测版本。填写宿主版本仅是用户记录，**不会升级为实测通过**。选择所有项目的新执行或自己的项目范围，保存。
3. “测试连接”只验证 TaskPulse 接收端，不产生监控任务、不验证真实 Agent。诊断中的 waiting_first_event 表示尚无宿主事件，不是任务完成或失败。
4. Kimi/TRAE CN/ZCode 可生成“预览新增 Hook”：查看具体文件、条目、可读命令与重载要求；取消不会写宿主文件。只有勾选并确认这次差异才写入。文件/来源/描述变化后旧预览失效；5 分钟超时或已使用的 token 要重新生成。
5. 宿主重新加载/开启新会话，做一次你自己可核对的测试执行。逐项对照开始、工具、批准、回复和结束；不能只凭“测试连接成功”宣称宿主可用。

本次仓库实施没有替你改动任何真实宿主 Hook。今后需要安装时仍须审阅差异并明确确认。请把 TaskPulse 安装在长期保留的位置；不把 `target/debug`、`codex-check` 或临时 smoke 路径写进真实配置。

### 内置配置位置及边界

| 产品 | 已实现 recipe | 注意 |
|---|---|---|
| 新版 Kimi Code CLI | `~/.kimi-code/config.toml` 的 `[[hooks]]` | 仅合法 event/command/timeout 字段；保留 TOML 注释；不适用于旧 `.kimi` 产品配置 |
| TRAE 国内 IDE | `~/.trae-cn/hooks.json`，version=1 | PowerShell 固定转发命令；严格 JSON，遇 JSONC/其他版本拒绝并不改文件 |
| ZCode Desktop | `~/.zcode/cli/config.json`，hooks.events | process command + argv；不改变全局 hooks.enabled；需你在宿主确认开关并新建会话 |
| WorkBuddy Desktop | 无 | 只有候选描述，不开放一键安装；需要官方桌面载荷/配置及实测证据 |

安装时备份原文件到同目录 `*.pre-taskpulse-integrations.bak`（一次性）。撤销不把旧备份整体覆盖回去，只移除登记的 TaskPulse 条目，保留后来添加的 Hook、模型、代理、statusLine 与未知字段。“停用来源”不会移除 Hook，也不删除未读/固定；“撤销 Hook”是独立确认动作。

删除来源后仍可在来源页下方“宿主 Hook 登记”查看“来源已移除”的登记，点击“预览撤销已登记 Hook”，核对实际自有条目并勾选确认。安装登记独立于来源配置；旧版本只有 hook-ownership.json 的安装也支持恢复撤销。不会自动改写宿主文件，不会恢复整份旧备份。文件在预览后有变化时必须重新预览。

## 没有内置品牌时

豆包桌面现有独立的[原生读取](DOUBAO.md)，Windows 2.26.10 可观测正在执行的主任务。
请添加“豆包 · 本机”来源；无需先配置 MCP。其完成和审批能力仍有明确限制。

### 支持本机 MCP 的桌面或 CLI 宿主

1. 新建“其他 Agent · 通用事件”，保留“其他 Agent（标准事件）”，把来源名称改为实际宿主名称，保存。每个宿主单独建来源，便于停用和项目范围隔离。
2. 点击“生成本机 MCP 配置”并复制，将其中的 `taskpulse` 条目合并到宿主已有的 `mcpServers`，保留其他服务。配置使用当前已安装应用的绝对路径；macOS 指向 `.app/Contents/MacOS/agent-tray`，不要求安装 Python、Bun 或另一个服务。
3. 在宿主里确认并重载 MCP，验证能发现 `taskpulse_report`。TaskPulse 只导出配置，不自动修改宿主、不安装 Hook、不改变批准策略。
4. 宿主或自动化框架在实际生命周期节点调用该工具。若只能让模型主动调用，应把它视为**可能漏报的协作上报**；MCP 连接本身不能自动观察宿主的对话、系统批准弹窗或最终结束。可靠的原生 Hook/日志优先于提示词要求模型上报。
5. 先调用 `kind=connection.test` 验证接收（不会创建任务），再执行一次可核对的真实任务；仅工具列表可见或连接成功不能记为实机适配通过。

工具参数沿用标准事件的 `session_id`、`turn_id`、`event_id`、`kind`、`request_id`、`call_id` 和最小 `payload`，**不要传 `agent_id`、来源 ID 或凭据**；它们由保存的来源绑定。会话 ID 跨轮次稳定，轮次 ID 每次新执行变化，事件 ID 对同一次事实/重试保持不变。每个批准/提问单独使用 request ID，只解除对应请求。只有明确的 `turn.completed` / `turn.failed` / `turn.interrupted` 能结束该轮；进程退出、MCP 断开和工具返回均不能代替结束事件。

支持的 stdio 协议版本为 `2025-06-18`、`2025-11-25`；不开放 HTTP 端口，也不提供命令执行、批准、文件读取或其他来源任务查询工具。工具只接收来源绑定的最小状态事实，标记为 `self_reported`。来源停用后，已经运行的 MCP 子进程也会拒收新事件。来源重新命名不改变任务身份。

豆包工作开放平台列出了插件 / MCP 接入方式，但这**不证明所有豆包桌面版本支持任意自定义本机 stdio 服务**。若客户端只允许远程 URL、预置连接器或审核后的插件，这份本机配置不能直接使用，需要其正式插件通道及对应宿主验证。原生检查点读取是独立路径。没有开放接入口的桌面/网页助手，不能通过进程名、截图、CPU 或静默时间可靠补出完整任务生命周期。

依据：[MCP stdio](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)、[MCP 生命周期与版本协商](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)、[豆包开放平台](https://open.doubao.com/)。客户端需要支持本机 stdio 服务。

### 支持 Hook / API / 脚本事件的宿主

如果工具能产生[标准 v1 事件](AGENT_INTEGRATION_PROTOCOL.md)，新建来源选择“标准事件”，点击“导出标准接入说明”，使用其中**当前来源绑定的 argv 和 agent_id**。将事件写到 exe stdin 并关闭输入流。生产者应使用其真实 session/turn/event/request/call ID；不要让模型仅靠提示词自报结束来冒充官方生命周期。

仓库 `integrations/examples/` 是明确的合成格式示例，可复制到隔离实验环境；别当作真实采集结果导入日常监控。使用安装路径验证格式的 PowerShell 形式：

```powershell
$OutputEncoding = [Text.UTF8Encoding]::new($false)
Get-Content -Raw -Encoding UTF8 .\event.json | & 'C:\你的持久安装路径\agent-tray.exe' --integration-validate
```

如果原生事件只需改字段/名称，可基于 `integrations/community-template.json` 修改 adapter_id、agent_id、JSON pointer、事件映射和能力限制，通过管理界面验证导入，再新建来源选中它。这样不需要修改 Rust 品牌枚举、重新构建或发布 TaskPulse。导入后没有自动安装/执行权限，证据保持自报。特殊二进制日志、远程传输、复杂关联等仍需代码适配；不写“支持所有 Agent”。

## 状态应该怎么看

- 思考、执行工具、生成回复、等待模型都在“进行中”；没有具体指令时显示安全阶段/工具名，不虚构命令。
- 只有明确批准/提问是“需要你处理”；工具超时、提权参数或安静日志不是批准请求。
- “本轮结束”是 Agent 一轮结束，不是整个项目通过验证。中断单独显示，不伪装成功。
- 没有可靠最终事件的适配器（当前 Kimi/TRAE/ZCode）**不会发送正常完成弹窗**。TRAE 的异步 idle_prompt 缺少 turn ID，不能结束最新任务；Stop 之后可能继续运行。不要用自动刷新/静默超时补一个假结果。
- 关闭提醒保持未查看；查看后手动标记已查看，运行/固定任务继续显示。改名/心跳不刷新最近执行活跃期。

## 诊断与撤销

保存后点“接入诊断”：查看接收数量、最近接收时间、最新采样事件类型、来源 health、历史错误代码。历史 last_error 可以保留，即使后续已恢复；connected 只指有事件/本地收集正常，不代表某版本完整支持。没有心跳的安静来源不会被推断完成。

常见代码：source_disabled（来源停用）、host_event_not_supported（原生事件未适配）、turn_id_required/native_turn_id_missing（缺少轮次）、ambiguous_permission_resolution（无法唯一解除）、permission_call_mismatch（关联冲突）、lock_timeout（2 秒锁等待超时）、journal_quota_export_before_reset（配额）、checkpoint_backup_failed/state_replace_failed（持久化失败）。Hook 失败退出 0 是保护宿主，不代表事件写入成功；应检查诊断和 journal。

### 恢复与磁盘上限

当前没有自动删除/压缩事件。磁盘上限为每会话 32 MiB/每来源 128 MiB，达到后接收失败并诊断。输入请求预算为128：接收端在会话写锁内拒绝新的超限请求，但匹配解除、明确结束不占新槽。旧版本已经接收的超限/无效记录保留在原 journal，并在同一原子检查点内记录有界隔离索引和累计数量后前移 cursor，继续处理解除/结束；单会话检查点故障不阻断健康会话。诊断的 quarantine 字段可查统计；未解决的隔离影响显示为来源不确定，不伪造成功。

seen 去重上限不会清空旧账本；达到后拒绝普通新事实，但幂等的明确解除、工具返回和终态仍可恢复已有执行。每检查点最多保留64条隔离索引；原始事实不删除。磁盘耗尽/持久化失败仍可能无法接收，包括恢复事实，此时保留旧状态并人工排查，不能声称零丢失或自动轮转。

1. 先在原 Agent 核对活动任务和未解决请求，不要标记它们完成来清理。
2. 停用该来源，并关闭**自己的 TaskPulse 实例**；确认没有该来源的 Hook 写入进程。
3. 备份 `~/.taskpulse/integrations/<source-sha256>/` 整个目录及 attention.json、notifications.json、config.toml。只处理这一明确来源，不清空 `.taskpulse` 或其他来源。
4. 损坏检查点能自动备份/重放；损坏 journal 或达到配额则保留原目录导出排查。不要直接删除未消费尾部/未解决请求。需要重新开始时，在 UI **新建独立来源 ID**、预览确认迁移 Hook，只收后续新执行；旧来源与备份继续保留以便回查。

达到存储上限时按界面提示检查，避免继续积累无用事件。
