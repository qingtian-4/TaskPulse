# 界面素材与来源

公开素材来自当前 TaskPulse 原生 WebView2，不拼接或重绘。原生截图代表真实应用渲染；使用隔离合成事件时必须注明，不能作为 Agent 宿主实机支持证据。

## 下一候选源码预览（2026-10-28 · TP-03）

以下 PNG 由隔离的真实 Tauri / Windows WebView2 直接输出，输入均为公开合成事件；未拼接、重绘或替换文字。截图不是实际 Agent 新任务或宿主兼容性证据。

| 文件 | 逻辑尺寸 | 输入 / 证据边界 |
|---|---|---|
| [dashboard-current.png](screenshots/dashboard-current.png) | 412 × 620 | 合成审批与已结束未查看结果；底部状态区保留来源和额度入口 |
| [message-center-current.png](screenshots/message-center-current.png) | 412 × 620 | 合成审批、明确失败与完成；消息已读不解除待处理请求 |
| [sources-current.png](screenshots/sources-current.png) | 1040 × 900 | 隔离来源管理，未安装真实 Hook |
| [notice-current.png](screenshots/notice-current.png) | 320 × 132 | 独立原生测试提醒，不创建监控任务 |
| [notice-question-current.png](screenshots/notice-question-current.png) | 320 × 132 | 隔离标准事件触发的真实待输入弹窗，合成示例 |
| [notice-approval-current.png](screenshots/notice-approval-current.png) | 320 × 132 | 隔离标准事件触发的真实待批准弹窗，合成示例 |

先执行 `scripts/verify.ps1 -Frontend`，再依次运行（端口不能并用）：

```powershell
bun scripts/native-product-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --capture
bun scripts/native-message-center-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --capture
```

只有全部检查通过才更新公开图片；每次输入与结果留在忽略提交的 `src-tauri/target/ui-smoke/evidence/`。
新增的两张请求截图经实际 Provider/TaskStore/AttentionTracker/TransitionEngine 到原生提醒窗口产生，未替换 DOM 文案；测试还检查关闭不解除请求，只有匹配 request ID 的解决事件恢复运行。
2026-10-27 本轮请求截图与原生回归通过，证据保留于本地
`src-tauri/target/ui-smoke/evidence/product-native-2026-10-27T07-45-36-931Z-yIbGDw/`；
中文架构图和 README 另经本地浏览器渲染检查，预览位于 `src-tauri/target/readme-refresh-20261027/`。
检查覆盖 412×620、380×640、320×480、440×720，WebView2 DPR 1 / 1.25 / 1.5 / 2，以及消息已读、请求解除、结果未查看和重启恢复。
这不等同 Windows 实际改变显示比例、混合 DPI 显示器迁移或真实 Agent 生命周期验收。

本次 current 图片使用已确认的 TP-03（明显的 T × P）品牌标记，替代旧双色 P；原有界面尺寸和布局保持不变。
[品牌资源与 Social Preview](brand/README.md) · [TP-03 验收结果](../docs/TP_LOGO_VALIDATION_2026-10-28.md)。

## 历史 Alpha.7 原生素材

2026-10-21 视觉升级后重新捕获：统一青绿色桌面主题、语义状态卡片与紧凑通知，仍为真实 Tauri / WebView2 渲染和公开合成输入。没有为截图提高任何 Agent 的支持等级。

| 文件 | 实际画面 | 输入 / 证据边界 |
|---|---|---|
| [dashboard-alpha7.png](screenshots/dashboard-alpha7.png) | 440 × 720 关注面板 | 标准事件来源，公开合成任务；明确提问、思考、工具活动与待查看结果 |
| [sources-alpha7.png](screenshots/sources-alpha7.png) | 1040 × 900 来源管理 | 独立 TASKPULSE_HOME，合成来源；未安装真实 Hook，实机版本仍为无 |
| [notice-alpha7.png](screenshots/notice-alpha7.png) | 320 × 132 独立提醒 | 真正的 test_notification → 原生窗口 → rendered 投递记录；不创建监控任务 |

这些 PNG 直接由原生 WebView2 CDP 输出，不拼接、重绘或后期替换文字。输入标题、项目与活动标签都是公开示例，不含私人会话、命令、账号或服务器；图片不是四款 Agent 宿主验收证据。运行环境和完整复现见 [当前视觉验收报告](../docs/VISUAL_POLISH_VALIDATION_2026-10-21.md)。

三个页面均补测 WebView2 CDP DPR 1 / 1.5 / 2（100% / 150% / 200%）：固定逻辑窗口下检查重叠、横向溢出与可见控件命中，倍图留在忽略提交的测试证据目录。该模拟不等同 Windows 真正切换显示比例或跨混合 DPI 显示器实测。

## 历史素材

- [agenttray-screenshot.png](legacy-agenttray/agenttray-screenshot.png)、[agenttray-demo.gif](legacy-agenttray/agenttray-demo.gif)：上游 AgentTray 历史素材，不是 TaskPulse，归入独立目录保留溯源，不用于产品展示。
- [taskpulse-overview.svg](diagrams/taskpulse-overview.svg)：早期示意图，不是真实采集或原生截图，不用于当前产品展示。

缺少新画面时不以历史或示意素材代替。

## 当前架构示意

[taskpulse-architecture.svg](diagrams/taskpulse-architecture.svg) 来自 PR #10，2026-10-23 按源码修正：native/WSL Worker、远端独立线程、watcher 内兼容文件采集分别标明；Journal/Checkpoint/Outbox 只适用于 Generic Event，ack 不等于可见投递。它是说明图，不是应用截图。详细事实与限制见 [架构](../docs/ARCHITECTURE.md)。

中文首页使用 [taskpulse-architecture-zh.svg](diagrams/taskpulse-architecture-zh.svg)：保留上述数据流，翻译标题、模块职责与说明；英文首页保留英文版。手机 App 只出现在路线图中，不纳入现有实现图。

2026-10-23 人工检查的三张 Alpha.7 PNG 继续保留原文件，尺寸分别为 440×720、1040×900、320×132；不覆盖其历史证据。当前 UI 为中文。没有新 MP4/GIF 成品，复现命令、storyboard、字幕及 Release 附件方案见 [演示计划](../docs/DEMO_RECORDING_PLAN.md)。

## 安全录制

1. 使用未运行的构建目标执行 `scripts/verify.ps1 -Frontend`。
2. 将 EXE 与 WebView2Loader.dll 复制到专属临时目录，设置独立 `TASKPULSE_HOME`，来源列表为空且已确认首次设置。
3. 通过标准事件入口投递公开合成标题、项目和命令，名称含“示例”；使用真实采集流水线，不以浏览器 mock 改写任务。
4. 仅为该隔离进程临时开启 WebView2 CDP，拍摄 Dashboard、来源管理和测试通知，检查私人路径、账号、服务器、额度与对话泄露。
5. 保存输入说明和结果；退出隔离进程，关闭调试端口。不安装真实 Hook，不运行真实 Agent 任务，不提高 fixture-tested 支持等级。
