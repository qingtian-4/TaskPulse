# 通用接入兼容矩阵

核验日期：2026-10-21（R1–R4 修复后能力边界）。TaskPulse `0.2.0-alpha.7`、Windows 10 Pro 22H2 x64（19045）。**以下四款的宿主 Hook 实机验证版本均为“无”**。安装元数据不是事件验证；合成载荷测试与 TaskPulse 原生 WebView2/GUI exe 验收均不升级为宿主 native-tested。

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
