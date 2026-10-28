# 下载测试包

TaskPulse 当前为未公开的 Alpha。开发者源码、PR 测试附件和公开 Release 是不同的交付渠道。

## 当前 PR 的安装预览

1. 打开 [Installation preview 工作流](https://github.com/qingtian-4/TaskPulse/actions/workflows/installation-preview.yml)，选择目标 PR 的成功运行。
2. 核对该运行的完整 head SHA，登录 GitHub 后下载对应平台的 artifact。
3. Windows x64 选择 EXE 或 MSI；Mac 根据芯片选择 arm64 或 x64 DMG。两种 Windows 安装格式不要同时安装。
4. 解压后核对 manifest 的源码 SHA、版本、平台和架构，并验证附带 SHA-256。PR 预览标记为 `build_kind=pr-preview`。
5. 正常退出旧实例后安装。Windows 使用安装向导；Mac 打开 DMG 并复制 app。首次运行核对“关于”中的版本、SHA 和程序路径。

附件有保留期限，并需要登录。原生 runner 的验收结果在同一次运行的 evidence artifacts 中；检查实际 result JSON，不能只看绿色工作流图标。公共一行安装入口拒绝 PR 预览和 Draft。

## 实机测试

使用独立测试配置，或先备份 `~/.taskpulse/`。不要为通过测试删除真实状态或更改 Agent 的审批策略。

- 真实任务：开始、思考、工具调用、明确提问/批准及匹配解除、完成与失败。
- 展示：项目、标题、模型、计时、改名，以及来源断连后的不确定状态。
- 关注：未读、固定、手动已查看、重启恢复，以及不重复提醒。
- 安装：首次安装、正常退出、重启、同格式升级、卸载后状态保留。
- Windows：WebView2 缺失/网络受限、不同 DPI、多屏和提醒焦点。
- Mac：Finder 启动、Gatekeeper、菜单栏、Retina/多屏和通知焦点。

跨版本升级与 EXE/MSI 迁移需要单列验证。Mac ad-hoc 签名不等于 Developer ID 或公证；按 [Apple 指引](https://support.apple.com/en-us/102445)处理单应用提示，不关闭系统安全检查。

每项记录系统/架构、源码 SHA、包 SHA-256、步骤、预期、实际结果及通过/失败/未验证。详细过程和原始截图保存在本地，向 Issue 提交时只附脱敏的最小复现。

## 发布状态

测试者确认稳定后才合并待测试 PR；公开发布还需维护者单独确认。Draft、暂缓和未验证都不等于已公开或已通过。完整门禁见 [发布流程](RELEASING.md)。
