# 界面素材与来源

公开素材来自当前 TaskPulse 原生 WebView2，不拼接或重绘。原生截图代表真实应用渲染；使用隔离合成事件时必须注明，不能作为 Agent 宿主实机支持证据。

## 历史素材

- `agenttray-screenshot.png`、`agenttray-demo.gif`：上游 AgentTray 历史素材，不是 TaskPulse，保留用于溯源，不用于产品展示。
- `taskpulse-overview.svg`：早期示意图，不是真实采集或原生截图，不用于当前产品展示。

当前版本原生截图将在隔离验收后添加。缺少素材时不以示意图代替。

## 安全录制

1. 使用未运行的构建目标执行 `scripts/verify.ps1 -Frontend`。
2. 将 EXE 与 WebView2Loader.dll 复制到专属临时目录，设置独立 `TASKPULSE_HOME`，来源列表为空且已确认首次设置。
3. 通过标准事件入口投递公开合成标题、项目和命令，名称含“示例”；使用真实采集流水线，不以浏览器 mock 改写任务。
4. 仅为该隔离进程临时开启 WebView2 CDP，拍摄 Dashboard、来源管理和测试通知，检查私人路径、账号、服务器、额度与对话泄露。
5. 保存输入说明和结果；退出隔离进程，关闭调试端口。不安装真实 Hook，不运行真实 Agent 任务，不提高 fixture-tested 支持等级。
