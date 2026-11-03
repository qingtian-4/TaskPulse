# 请求闭环评测

TaskPulse 观察宿主的显式请求，不回答问题或批准操作。返回动作的结果分别记录为精确会话、宿主窗口和详情。当前 Codex / Claude 接入不承诺精确会话跳转；有已验证的活动窗口时可聚焦宿主，否则打开详情，可复制项目和会话 ID。详情、聚焦、关闭提醒都不会清除未读。

Claude 的可选 Hook 观察显式审批。原生提问暂不计为支持：MCP Elicitation 在对话框前触发，可能由已有 Hook 自动回答；对话框通知缺少可可靠关联结果的 ID。普通 `AskUserQuestion` 工具预调用也不等于用户已看到问题。参见 [Claude Hook 参考](https://code.claude.com/docs/en/hooks)。不会自动安装 Hook 或修改权限模式。

## 固定回放

```sh
bun scripts/native-request-replay.ts --exe <clean-development-exe> --expected-commit <full-sha>
```

Windows 上使用隔离应用、实际 Provider、观察脚本和 WebView2。固定真值在 `scripts/fixtures/request-replay.json`，不从 Provider 输出推导。检查普通工具事件、真实形状的明确请求、错误结果、匹配解除、重启恢复、不重复投递、详情回退和未读不变。输入全部为标注的合成样本。脚本不会打开真实 Agent、注册 Hook 或写真实配置。

每条请求从写入合成输入前计时，因此回放延迟包含了输入写入开销，是事件可观测到渲染的上界。结束点是实际提醒 WebView 的帧确认和原生可见性校验，不使用后端“已入队”作为渲染时间。支持的固定场景必须零误报、零漏报，全部预期恢复及返回行为通过。Claude 提问只验证不会由预调用产生假提醒，不计入漏报率。样本数、编译提交、可执行文件哈希和清理结果随运行保留。

## 真实来源

1. 按 Agent、宿主版本、平台、请求种类分别建组。用宿主 UI 或独立宿主事件记录标记每个观测窗口的预期请求和可观测时间。记录没有请求的对照窗口，不能从 TaskPulse 解析器或通知历史反推真值。未经支持的通道标为 `unsupported`。
2. 启用弹窗提醒时，用 TaskPulse 原生投递记录的 `queued` 与同一 `notice_id` 的 `rendered` 建立观测记录。窗口必须完整覆盖；遇到记录截断、程序退出或无法确认渲染时间时标为缺测。重新打开不是新的自动提醒。每次重启以相同请求 ID 验证恢复，每次聚焦用真实前台窗口确认，单纯调用返回成功不足以证明跳转。
3. 保留本地去敏后的独立真值和观测文件。不要记录提问内容、答案、命令、凭据、目录或未脱敏的宿主日志。记录资源采样的工作集、CPU 时间差和采样间隔；原生压力测试可提供峰值负载下的附加证据。
4. 运行 `bun scripts/request-evaluation.ts truth.json observations.json`。类型定义和最小样本见 [评测脚本](../scripts/request-evaluation.ts) 和 [评分测试](../scripts/request-evaluation.test.ts)。

报告分别列出未入队、未实际渲染、重复/误报和缺测。漏报分母是有支持的真实请求数，误报分母是无请求对照窗口数，未绑定和重复提醒计入误报数。两者目标各 ≤1%，每组至少 100 个请求和 100 个对照窗口才作门禁判断。分别报告事件路径 P95 ≤2 秒和轮询路径 P95 ≤两次轮询间隔 +2 秒。不支持、无样本或样本不足的组不报告通过，不算作零漏报。返回成功按目标分别计数，详情回退不计作窗口聚焦或会话跳转。

对已保存的真实投递记录，可运行 `bun scripts/request-delivery-evidence.ts truth.json notification-log-copy.json capture.json`。捕获文件由评测者提供独立的观测起止时间、完整性确认、该组的任务范围及每条真值对应的任务/请求绑定。正例必须绑定独立核实的请求 ID；完成提醒和其他请求类型不会充当正例。工具只读副本，关联同一提醒的入队与渲染，拒绝截断或重叠且无法区分的观测窗口，并将没有对应真值的提醒计入误报。输出不包含任务 ID、问题或答案。真实前台验证和资源样本仍须由实机采集提供。

完整组超过日志容量时，分成若干不重叠的短捕获，为每段导出观测，并使用全组真值运行 `bun scripts/request-evaluation.ts truth-all.json observations-a.json observations-b.json ...`。每段沿用全组唯一的去敏案例 ID，重复 ID 会报错；不要扩大产品日志上限。
