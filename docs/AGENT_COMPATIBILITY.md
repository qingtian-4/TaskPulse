# Agent 支持与通用接入兼容矩阵

## 内置来源

以下为 Alpha.7 已实现范围，不等于所有版本实机验收。安装发现、配置保存、读取成功、测试提醒与
真实生命周期验证是不同阶段，管理界面不会将其中一个自动提升为后者。

来源管理现在分开展示读取状态、测试提醒和待验收提示，通用来源可展开逐项能力与实测版本。
UI / 原生截图验收见 [第二轮验证报告](UI_UX_VALIDATION_2026-10-21.md)；它不新增任何宿主 native-tested 等级。

| 来源 | 可用环境 | 证据与限制 |
|---|---|---|
| Codex | Windows、按运行发行版启用的 WSL、已配置 SSH | 原生标题/rollout；只在有明确审批事件时提醒。大日志追赶、服务器日志窗口有边界 |
| Claude Code | Windows 本机 | 原生只读记录 best-effort；官方 Hook 显式确认安装，代理模型不改变 Agent 身份；无专用 WSL/SSH 适配 |
| Hermes | Windows 本机 | SQLite 与租约，租约丢失不证明完成；无专用 WSL/SSH 适配 |
| DeepSeek Harness | Windows Desktop/CLI、独立 WSL | 旧版/v4 压缩日志与标题 projection；无专用 SSH 适配 |
| 豆包桌面（实验） | Windows 2.26.10 真实运行任务已读取 | [原生执行检查点](DOUBAO.md)；按 ID 关联真实会话名，无项目时显示独立会话；审批未适配，主任务结束事件未持久化，检查点消失保留为待更新 |
| Gemini CLI | 兼容 Hook / 进程扫描 | 不能作为完整会话/阶段/审批验收 |
| 远程普通代码 | SSH tmux / tp-run | wrapper 的前台退出码可靠性不扩展到 detached/container/Slurm 子任务 |

第一轮只读与原生平台验证见 [可靠性记录](internal/reports/RELIABILITY_OPTIMIZATION_2026-10.md)。这些记录不替下面四款宿主 Hook 背书，也不证明所有内置来源的新任务端到端时延。

## 模型信息与本轮计时

| 来源 | 模型证据 | 验证及边界 |
|---|---|---|
| Codex | 原生 thread/app-server 元数据、rollout turn context | 本机只读验证；WSL/SSH 共用 parser，不等于所有环境实机通过 |
| Claude Code | assistant 消息的 model | 原生 parser 合成回放；当期无合格近期真实会话，代理后实际模型未知 |
| DeepSeek Harness | 原生 session/log state 的 model | 本机只读验证；原生/WSL 分开，projection 不覆盖新日志状态 |
| Hermes | SQLite session/message 模型字段 | 本机只读验证；兼容旧 schema，缺失保持未知 |
| 其他/通用来源 | 仅来源明确提供的可选 model | 不按 Agent 品牌或项目名称猜测 |

模型只作上下文，变化不重建稳定 task ID，不改生命周期、开始时间、未读或通知去重。
空值/缺失显示未知；一个项目可同时使用多模型。模型报告不证明供应商或代理后的实际服务。
本轮经过时间包括思考、工具和模型等待；开始证据缺失不造时间，采集刷新/重命名不续执行活跃期。
Windows 原生界面合成测试不替代当前 PR 安装包或 Mac 真实 Agent 验收。

## 四款通用接入的验证等级

核验日期：2026-10-21（R1-R4 修复后能力边界）。TaskPulse `0.2.0-alpha.7`、Windows 10 Pro 22H2 x64（19045）。**以下四款的宿主 Hook 实机验证版本均为“无”**。安装元数据不是事件验证；合成载荷测试与 TaskPulse 原生 WebView2/GUI exe 验收均不升级为宿主 native-tested。

等级：explicit=原生明确事件/字段；inferred=受限派生或唯一关联；unsupported=不提供该能力。以下 Hook 映射为文档依据的实现，`tested_versions=[]`；未知版本不代表兼容保证。完整逐项元数据也随 `integrations/*.json` 展示于管理界面。

## 产品与实机证据

| 产品形态 | 代码/合成测试 | 实际宿主验证版本 | 只读安装证据 | 未支持/待证实 |
|---|---|---|---|---|
| 新版 Kimi Code CLI | 有映射、配置合并与接收/归约测试 | **无** | 未确认安装 | 最终正常结束、提问、精确跳转；request/call 字段需真实样本；不推广到 Desktop/VS Code 或旧 kimi-cli |
| TRAE 国内 IDE | 有映射、JSON recipe、结构化 Notification 测试 | **无** | TRAE (User) 3.5.62 | 最终正常结束、原生标题、整轮失败、纯思考无工具轮次发现、无可靠关联的输入请求/解除；不推广到海外/企业/SOLO/Work |
| ZCode Desktop | 有映射、process argv recipe、唯一关联测试 | **无** | ZCode 3.11.2 | 原生标题、最终正常结束、整轮失败/提问、纯思考新轮次；不自动启用全局 Hooks |
| WorkBuddy Desktop | 候选描述和限制 UI；**无宿主事件映射/安装** | **无** | 未确认安装 | 桌面配置、payload、session/turn/request/call 契约均缺；不能用 CodeBuddy CLI 替代 |

## 逐项能力（仅文档依据 + 合成验证，不是实机通过）

| 能力 | Kimi CLI | TRAE CN IDE | ZCode Desktop | WorkBuddy Desktop |
|---|---|---|---|---|
| 会话发现 | explicit SessionStart | explicit SessionStart | explicit SessionStart | unsupported |
| 标题 | explicit session_title（有字段时） | unsupported | unsupported | unsupported |
| 项目 | explicit cwd；项目名独立派生 | explicit cwd | explicit cwd | unsupported |
| turn 开始 | explicit TurnStarted/turn_id | inferred：提交后的首个工具证据 | inferred：同左 | unsupported |
| 阶段 | 工具及返回；非工具仍运行 | 工具及返回 | 工具及返回 | unsupported |
| 最终正常结束 | **unsupported；Stop 可继续** | **unsupported；异步 idle_prompt 无可靠 turn ID，仅记未关联观察** | **unsupported；Stop 可继续** | unsupported |
| 整轮失败 | explicit StopFailure，必须有原生 turn_id | unsupported | unsupported；工具失败非整轮失败 | unsupported |
| 批准请求 | explicit PermissionRequest；无 turn_id 时仍需调用关联 | inferred：明确 Notification + 唯一当前调用 | inferred：明确 PermissionRequest + 唯一当前调用/输入签名 | unsupported |
| 批准解除 | inferred：匹配 request_id 或唯一 call/signature；字段不保证 | inferred：唯一 tool_use_id + PostToolUse | inferred：唯一 tool/input 摘要→call + PostToolUse | unsupported |
| 提问 | unsupported | inferred：明确 Notification/ask_user_question，仍须可靠调用关联；缺失仅记录观察 | unsupported | unsupported |
| 原生精确跳转 | unsupported | unsupported | unsupported | unsupported |

TRAE document_review/browser_interaction 等类型未纳入当前映射；未知 Notification 不按文案猜测。所有适配的 Stop 只产生 stop_requested。TRAE/ZCode 的 UserPromptSubmit 只暂存：可能被拒绝，因此不显示为已开始执行；无工具的运行存在明确漏检边界。没有稳定原生事件 ID 的重复投递不能保证请求级去重；不会用内容哈希吞掉两个真实相同请求。

R2 修复不只改表格：mapper 不再把无可靠轮次的终态赋给最新 turn；无关联输入/结果使用 `observation.unassociated`，不结束、不创建/解除阻塞、不续活跃期。历史调用 ID / 输入签名复用证据使用有界单调过滤器保守降级；碰撞只会降低关联能力，不会制造结束或授权。明确旧 turn/request 的解除可释放旧记录，但不更改当前任务。新适配版本为 1.0.1，旧 v1 事件/检查点继续可读。

通用标准协议可显式表达全部生命周期，但证据为 **self_reported**，不等同于上述产品官方支持。CLI 接收/TaskPulse GUI 已测是平台能力，不是替宿主证明其 payload。

2026-10-27 新增来源绑定的本机 MCP stdio 接入口：未列出的宿主若允许自定义本机 MCP，可使用同一个 `taskpulse_report` 工具，无需新增品牌枚举。它属于主动上报，可能漏报，不能自动观察宿主审批或替代可靠 Hook。Windows/macOS 共用接收器；Linux 原生验收仍待完成。内置三款 Hook 写入 recipe 仍仅支持 Windows，在其他平台禁止新增，撤销自有登记仍可执行。

豆包桌面已有上述 Windows 实验原生读取，但**尚无实机 MCP 验收**。官方开放平台的插件 / MCP 能力不能推广为任意版本都开放本机 stdio 配置。其他没有官方事件、可读日志或可用插件接口的宿主仍保持尚未接入，不制造运行/完成/批准状态。MCP 接入步骤见[通用指南](AGENT_INTEGRATION_GUIDE.md#支持本机-mcp-的桌面或-cli-宿主)。

## 官方依据

- [Kimi Code CLI Hooks](https://www.kimi.com/code/docs/kimi-code-cli/customization/hooks.html)：新版 `.kimi-code`、TurnStarted、审批与 Stop/中断。不要混用旧 MoonshotAI/kimi-cli 的 `.kimi` 与 beta 事件。
- [TRAE 国内 IDE Hook 配置参考](https://docs.trae.cn/ide_hook-configuration-reference)：Windows PowerShell、结构化 Notification、工具 ID；直取超时后通过官方搜索索引核验，未获取实机载荷。
- [ZCode Hooks](https://zcode.z.ai/cn/docs/hooks)：独立 process argv/schema、PermissionRequest 关联限制、Stop 可继续、新会话加载。
- [WorkBuddy Plugins](https://www.codebuddy.cn/docs/workbuddy/Plugins)：列有 Hook 插件，但不足以证明桌面端兼容 CLI Hook 载荷。

## 真实宿主验收待办（需明确授权）

每款分别记录安装版/产品形态/OS、具体差异、确认时间、最小脱敏载荷、实际映射和事件序列。确认后只安装该款自有 Hook，保留用户代理/模型/其他 Hook：

1. 提交被其他 Hook 拒绝、排队、实际新 turn、无工具回复、并发调用。
2. 两个同时 pending、只解除其中一个；缺/冲突 ID 不误清；工具失败后继续。
3. Stop 被要求继续，不发完成；可靠最终结束（若没有则保持 unsupported）、中断、宿主重启。
4. 改名、来源停用/重启、通知与未读恢复；核对真实侧栏，不用 fixture 标题。
5. 验收后由用户选择保留或预览撤销，只移除自有条目。没有这些证据不得填写 native-tested 或 tested_versions。

现有 Codex/Claude/Hermes/DeepSeek 原生来源保留原边界，见 README 支持范围；本表不替它们重新背书。
