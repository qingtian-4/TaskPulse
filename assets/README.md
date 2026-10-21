# 界面素材与来源

公开素材来自当前 TaskPulse 原生 WebView2，不拼接或重绘。原生截图代表真实应用渲染；使用隔离合成事件时必须注明，不能作为 Agent 宿主实机支持证据。

## 当前 Alpha.7 原生素材

| 文件 | 实际画面 | 输入 / 证据边界 |
|---|---|---|
| [dashboard-alpha7.png](screenshots/dashboard-alpha7.png) | 440 × 720 关注面板 | 标准事件来源，公开合成任务；明确提问、思考、工具活动与待查看结果 |
| [sources-alpha7.png](screenshots/sources-alpha7.png) | 1040 × 900 来源管理 | 独立 TASKPULSE_HOME，合成来源；未安装真实 Hook，实机版本仍为无 |
| [notice-alpha7.png](screenshots/notice-alpha7.png) | 320 × 132 独立提醒 | 真正的 test_notification → 原生窗口 → rendered 投递记录；不创建监控任务 |

这些 PNG 直接由原生 WebView2 CDP 输出，不拼接、重绘或后期替换文字。输入标题、项目与活动标签都是公开示例，不含私人会话、命令、账号或服务器；图片不是四款 Agent 宿主验收证据。运行环境和完整复现见 [UI 验证报告](../docs/UI_UX_VALIDATION_2026-10-21.md)。

## 历史素材

- [agenttray-screenshot.png](legacy-agenttray/agenttray-screenshot.png)、[agenttray-demo.gif](legacy-agenttray/agenttray-demo.gif)：上游 AgentTray 历史素材，不是 TaskPulse，归入独立目录保留溯源，不用于产品展示。
- [taskpulse-overview.svg](diagrams/taskpulse-overview.svg)：早期示意图，不是真实采集或原生截图，不用于当前产品展示。

缺少新画面时不以历史或示意素材代替。

## 安全录制

1. 使用未运行的构建目标执行 `scripts/verify.ps1 -Frontend`。
2. 将 EXE 与 WebView2Loader.dll 复制到专属临时目录，设置独立 `TASKPULSE_HOME`，来源列表为空且已确认首次设置。
3. 通过标准事件入口投递公开合成标题、项目和命令，名称含“示例”；使用真实采集流水线，不以浏览器 mock 改写任务。
4. 仅为该隔离进程临时开启 WebView2 CDP，拍摄 Dashboard、来源管理和测试通知，检查私人路径、账号、服务器、额度与对话泄露。
5. 保存输入说明和结果；退出隔离进程，关闭调试端口。不安装真实 Hook，不运行真实 Agent 任务，不提高 fixture-tested 支持等级。
