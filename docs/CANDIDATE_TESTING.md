# 候选安装包下载与测试

当前候选版本：`0.2.0-alpha.8`。公开稳定版尚未发布；旧 Alpha.7 草稿不是本次修复版。
本页先说明操作方式；具体可交付包以对应运行完成、附件存在及验收记录为准。

## 下载入口

登录维护者 GitHub 账号，打开 [Actions](https://github.com/qingtian-4/TaskPulse/actions)。
只使用指定完整提交对应的成功运行；失败运行的中间文件不能当作验收通过的安装包。

| 设备 | 工作流 / 附件 | 解压后选择 |
|---|---|---|
| Windows 10/11 x64 | Windows Alpha release / `windows-installers` | `TaskPulse_0.2.0-alpha.8_windows_x64_setup.exe`（推荐）或 `.msi`，二选一 |
| Apple Silicon Mac | Experimental Linux and macOS bundles / `experimental-macos-arm64-<SHA>` | `TaskPulse_0.2.0-alpha.8_macos_arm64.dmg` |

GitHub 把 Actions 附件包装成 ZIP，先解压；`Source code.zip` 是源码，不是安装器。
Windows 发布工作流通过安装测试后会另建 Draft pre-release，维护者可在
[Releases](https://github.com/qingtian-4/TaskPulse/releases) 查看。草稿对普通访客不可见；
Actions 附件也需要登录且有保留期限。正式公开预览版前仍需维护者核对实机结果和发布门禁。

## Mac 上的最短测试路径

1. 下载 arm64 附件，解压。在该目录运行 `shasum -a 256 -c macos-arm64-SHA256SUMS.txt`。
2. 打开 DMG，将 TaskPulse 拖到 Applications。先退出旧实例，再从 Finder 打开。
3. 本轮采用 ad-hoc 签名，未 Apple 公证。若提示开发者无法验证，确认来源和校验和后，按
   [Apple 官方步骤](https://support.apple.com/en-us/102445)在“系统设置 → 隐私与安全性 → 仍要打开”处理。
   若提示损坏或仍无法启动，反馈原提示；不要全局关闭系统安全功能。
4. 打开菜单栏 TaskPulse，进入来源管理，在“关于 TaskPulse”核对 `0.2.0-alpha.8`、SHA 和应用路径。
5. 先测试面板开关、来源管理和“发送测试提醒”；再明确添加一个真实本机 Agent 来源。
6. 在原 Agent 开始任务，依次观察运行、真实提问/审批、处理后恢复、明确结束与结果待查看。
   审批只在原 Agent 完成；Hook 需另行预览和授权，Windows WSL Hook 不会自动安装到 Mac。
7. 退出并重启，检查来源范围、固定、未读和通知去重；至少持续使用一轮真实工作任务。

无需安装 Rust、Bun、Xcode 或 Codex 来单纯测试安装包。只有需要修改代码或排查平台问题时，
再按 [Mac 开发指南](MACOS_DEVELOPMENT.md)克隆仓库。

反馈格式：macOS 版本、芯片、TaskPulse 版本/SHA、操作步骤、期望/实际结果和脱敏截图。
请勿附完整 Agent 日志、命令、提示词或凭据。CI 原生窗口/启动检查不能替代上述实机交互。

## Windows 验收边界

发布 CI 在四个独立 runner 上分别完成 NSIS/MSI 两轮安装、实际应用 UI/IPC、重启、重装和卸载。
它不能替代干净 Windows 10/11、缺少 WebView2、网络受限、跨版本/格式迁移和真实 Agent 的实机测试。
安装器未代码签名；更换版本前退出旧实例并备份 `~/.taskpulse/`，不要删除配置来制造“干净通过”。

公开发行要求见 [发布流程](RELEASING.md)与[开源清单](OPEN_SOURCE_CHECKLIST.md)。
