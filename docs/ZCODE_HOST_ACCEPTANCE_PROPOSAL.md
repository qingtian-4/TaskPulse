# ZCode 宿主验收方案：待用户明确确认，尚未写真实 Hook

只读预检日期：2026-10-21。Windows 注册表 ZCode 3.11.2；`D:\Installed_Apps\ZCode\ZCode.exe` 的 ProductVersion 为 3.11.2.6792。该安装证据不是 Hook 实测，本次仍为 fixture-tested、tested_versions=[]。

## 待确认的具体差异

目标文件：`%USERPROFILE%\.zcode\cli\config.json`（本机解析为 `C:\Users\Administrator\.zcode\cli\config.json`），预检时不存在。拟新增的完整内容见 [JSON 差异提案](proposals/zcode-hooks-2026-10-21.json)，**这个文件只是仓库里的提案，不能直接覆盖已有用户配置**。如果确认前宿主创建/修改了文件，重新生成 UI 指纹预览并合并，只追加 TaskPulse 条目。

只新增 hooks.events 下 SessionStart / UserPromptSubmit / PreToolUse / PermissionRequest / PostToolUse / PostToolUseFailure / Stop 共7个事件组。每组同一固定 process + argv：

```text
command: D:\Installed_Apps\TaskPulse\agent-tray.exe
args: --integration-host-event --source generic-event:zcode-acceptance-20261021 --adapter zcode-desktop
timeoutMs: 10000
matcher: 空（所有工具）
```

持久 TaskPulse 路径当前尚不存在；不能把 target/debug、ci-release 或 smoke 临时 exe 写进真实 Hook。确认后先准备经验证的构建及相邻运行依赖到该持久位置，核对 SHA-256、版本和接收端，再通过该位置运行 TaskPulse 的预览/确认流程。**本轮未复制或启动此持久安装，也没有新增真实来源。**

TaskPulse 侧拟新增独立来源 `generic-event:zcode-acceptance-20261021`：kind=generic_event、agent_id=zcode、adapter_id=zcode-desktop、transport=local、host_version=3.11.2、projects=["*"]，用户确认保存时才设 accept_since。保留所有既有来源和项目 allowlist，不导入历史。

不写代理、模型、statusLine、其他 Hooks 或批准策略，也不替用户批准。提案**不写 hooks.enabled**。ZCode 官方要求用户级开关开启并新建 session 才生效，用户需在 ZCode 设置页自行检查/开启；如果不愿启用，就停止验收，不把“配置已写”当成事件成功。[官方 Hook 说明](https://zcode.z.ai/cn/docs/hooks)

## 确认后的验收步骤

1. 记录确认范围、精确宿主/TaskPulse版本、文件前后指纹和备份路径。重新读取配置并展示真实差异；通过勾选及单次 token 确认后才写入。文件变化或 token 过期则重做预览。
2. 宿主设置检查 Hook 开关，新建专用测试 session。先发一个纯文本无工具请求：预期只记录 submitted，不宣称已捕获纯思考轮次。不得把这个漏检限制标为通过。
3. 在隔离验收项目要求读取一个普通文本文件：核对原生 session ID、独立项目路径、工具调用 ID、RUNNING/tool→thinking；标题缺失用未命名+短ID，不能用测试标题冒充原生标题。
4. 由用户在正常权限模式下触发可控批准。对照原生弹窗和 TaskPulse；无法触发两个并发请求则该项记未验证，不自动修改批准策略。两个明确请求应独立显示，只批准一个只解除其对应请求；重复输入或缺失关联不得自动清理。
5. 核对 Stop 后可能继续：不弹正常完成。当前 ZCode 缺可靠最终 turn 事件，正常结束、整轮失败和中断闭环**未支持/待调查**，不要用计时猜测。这轮不能证明“开始→结束→完成提醒”整体支持。
6. 对照工具失败后重试、新 turn、调用/签名复用、延迟旧回调、TaskPulse 重启和来源停用。合成重放只算 fixture；只有真实宿主发生的序列才记实机证据。
7. 只记录最小脱敏字段及映射结果，不保存原始 prompt、tool_input/output、代理凭据。逐项填写真实版本/未验证项；不将四款整体升级为 native-tested。
8. 验收结束由用户选择保留或撤销。撤销再次预览确认，只移除登记的 TaskPulse 条目；即使删除来源，来源页“宿主 Hook 登记”仍可操作。保留用户后来修改，不整份覆盖备份。

## 本轮确认问题

是否同意按上述范围准备持久 TaskPulse 路径、添加这一独立 ZCode 来源，并在重新展示最终文件差异后写入这7组自有 Hook？Hooks 全局开关由用户自行操作；不同意则不改任何真实宿主文件。

当前答案：**尚未取得明确确认，未执行。**
