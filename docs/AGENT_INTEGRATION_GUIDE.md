# 接入其他 Coding Agent

本文说明如何维护已配置的本机事件发送端。日常使用从「设置 → 监控来源 → 本机」选择 Agent 即可。支持范围见[兼容说明](AGENT_COMPATIBILITY.md)。

## 已有事件连接

本机 Agent 使用原生只读检测。已配置的通用 Hook、MCP 和脚本连接保留原设置，设置页不列出这些连接。

### 旧 Hook 的配置位置

| 产品 | 已实现 recipe | 注意 |
|---|---|---|
| 新版 Kimi Code CLI | `~/.kimi-code/config.toml` 的 `[[hooks]]` | 仅合法 event/command/timeout 字段；保留 TOML 注释；不适用于旧 `.kimi` 产品配置 |
| TRAE 国内 IDE | `~/.trae-cn/hooks.json`，version=1 | PowerShell 固定转发命令；严格 JSON，遇 JSONC/其他版本拒绝并不改文件 |
| ZCode Desktop | `~/.zcode/cli/config.json`，hooks.events | process command + argv；不改变全局 hooks.enabled；需你在宿主确认开关并新建会话 |
| WorkBuddy Desktop | 无需 Hook，可添加本机原生来源 | 只读本地数据库与会话日志；通用 Hook 描述仍不提供宿主事件映射或安装 |

如需停用旧 Hook，请在宿主的设置或配置文件里只移除原有的 TaskPulse 条目，保留其他 Hook。更改前先备份配置。

### 支持本机 MCP 的桌面或 CLI 宿主

1. 保留宿主已有的 `mcpServers.taskpulse` 配置，确认宿主仍能发现 `taskpulse_report`。
2. 宿主或自动化框架在实际生命周期节点调用该工具。若只能让模型主动调用，应把它视为**可能漏报的协作上报**；MCP 连接本身不能自动观察宿主的对话、系统批准弹窗或最终结束。可靠的原生 Hook/日志优先于提示词要求模型上报。
3. 先调用 `kind=connection.test` 验证接收（不会创建任务），再执行一次可核对的真实任务；仅工具列表可见或连接成功不能记为实机适配通过。

工具参数沿用标准事件的 `session_id`、`turn_id`、`event_id`、`kind`、`request_id`、`call_id` 和最小 `payload`，**不要传 `agent_id`、来源 ID 或凭据**；它们由保存的来源绑定。会话 ID 跨轮次稳定，轮次 ID 每次新执行变化，事件 ID 对同一次事实/重试保持不变。每个批准/提问单独使用 request ID，只解除对应请求。只有明确的 `turn.completed` / `turn.failed` / `turn.interrupted` 能结束该轮；进程退出、MCP 断开和工具返回均不能代替结束事件。

支持的 stdio 协议版本为 `2025-06-18`、`2025-11-25`；不开放 HTTP 端口，也不提供命令执行、批准、文件读取或其他来源任务查询工具。工具只接收来源绑定的最小状态事实，标记为 `self_reported`。来源停用后，已经运行的 MCP 子进程也会拒收新事件。来源重新命名不改变任务身份。

客户端须支持自定义本机 stdio 服务。仅支持远程 URL 或预置连接器的宿主，需要使用其正式插件通道；本机 MCP 配置不能直接用于这些宿主。

依据：[MCP stdio](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)、[MCP 生命周期与版本协商](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)。客户端需要支持本机 stdio 服务。

### 支持 Hook / API / 脚本事件的宿主

如果工具能产生[标准 v1 事件](AGENT_INTEGRATION_PROTOCOL.md)，沿用发送端已有的命令和绑定的 `agent_id`。将事件写到 exe stdin 并关闭输入流。生产者应使用其真实 session/turn/event/request/call ID；不要让模型仅靠提示词自报结束来冒充官方生命周期。

仓库 `integrations/examples/` 是明确的合成格式示例，可复制到隔离实验环境；别当作真实采集结果导入日常监控。使用安装路径验证格式的 PowerShell 形式：

```powershell
$OutputEncoding = [Text.UTF8Encoding]::new($false)
Get-Content -Raw -Encoding UTF8 .\event.json | & 'C:\你的持久安装路径\TaskPulse.exe' --integration-validate
```

自定义事件映射的格式示例见 `integrations/community-template.json`。特殊二进制日志、远程传输和复杂关联需要单独适配。

## 状态应该怎么看

- 思考、执行工具、生成回复、等待模型都在“进行中”；没有具体指令时显示安全阶段/工具名，不虚构命令。
- 只有明确批准/提问是“需要你处理”；工具超时、提权参数或安静日志不是批准请求。
- “本轮结束”是 Agent 一轮结束，不是整个项目通过验证。中断单独显示，不伪装成功。
- 没有可靠最终事件的 Hook 适配器（当前 Kimi/TRAE/ZCode）**不会发送正常完成弹窗**。这与本机原生读取是独立通道；Kimi wire、ZCode 原生记录的明确结束能力见[兼容说明](AGENT_COMPATIBILITY.md)。TRAE 的异步 idle_prompt 缺少 turn ID，不能结束最新任务；Stop 之后可能继续运行。不要用自动刷新/静默超时补一个假结果。
- 关闭提醒保持未查看；查看后手动标记已查看，运行/固定任务继续显示。改名/心跳不刷新最近执行活跃期。

Windows 安装版的主程序为 `TaskPulse.exe`。旧版生成的 Hook 和 MCP 仍可通过原安装目录里的兼容入口接收事件；兼容入口不能打开任务面板，不需要修改现有宿主配置。

## 诊断与撤销

已有连接的诊断记录保存在对应来源目录，可查看接收数量、最近接收时间、最新采样事件类型和历史错误代码。历史 last_error 可以保留，即使后续已恢复；connected 只指有事件/本地收集正常，不代表某版本完整支持。没有心跳的安静来源不会被推断完成。

常见代码：source_disabled（来源停用）、host_event_not_supported（原生事件未适配）、turn_id_required/native_turn_id_missing（缺少轮次）、ambiguous_permission_resolution（无法唯一解除）、permission_call_mismatch（关联冲突）、lock_timeout（2 秒锁等待超时）、journal_quota_export_before_reset（配额）、checkpoint_backup_failed/state_replace_failed（持久化失败）。Hook 失败退出 0 是保护宿主，不代表事件写入成功；应检查诊断和 journal。

### 恢复与磁盘上限

当前没有自动删除/压缩事件。磁盘上限为每会话 32 MiB/每来源 128 MiB，达到后接收失败并诊断。输入请求预算为128：接收端在会话写锁内拒绝新的超限请求，但匹配解除、明确结束不占新槽。旧版本已经接收的超限/无效记录保留在原 journal，并在同一原子检查点内记录有界隔离索引和累计数量后前移 cursor，继续处理解除/结束；单会话检查点故障不阻断健康会话。诊断的 quarantine 字段可查统计；未解决的隔离影响显示为来源不确定，不伪造成功。

seen 去重上限不会清空旧账本；达到后拒绝普通新事实，但幂等的明确解除、工具返回和终态仍可恢复已有执行。每检查点最多保留64条隔离索引；原始事实不删除。磁盘耗尽/持久化失败仍可能无法接收，包括恢复事实，此时保留旧状态并人工排查，不能声称零丢失或自动轮转。

1. 先在原 Agent 核对活动任务和未解决请求，不要标记它们完成来清理。
2. 在宿主停用该来源的事件发送端，并关闭**自己的 TaskPulse 实例**；确认没有该来源的 Hook 写入进程。
3. 备份 `~/.taskpulse/integrations/<source-sha256>/` 整个目录及 attention.json、notifications.json、config.toml。只处理这一明确来源，不清空 `.taskpulse` 或其他来源。
4. 损坏检查点能自动备份/重放；损坏 journal 或达到配额则保留原目录导出排查。不要直接删除未消费尾部/未解决请求。保留该来源和备份，在继续采集前排查导出的记录。

达到存储上限时，按上述步骤保留证据再排查。
