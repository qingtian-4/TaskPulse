# 安装与快速入门

## 下载安装包

打开 [GitHub Releases](https://github.com/qingtian-4/TaskPulse/releases)，按系统选择：

| 系统 | 文件 |
| --- | --- |
| Windows 10 / 11，x64 | `TaskPulse_<版本>_windows_x64_setup.exe` 或同版本 `.msi` |
| macOS 13.3+，Apple Silicon | `TaskPulse_<版本>_macos_arm64.dmg` |
| macOS 13.3+，Intel | `TaskPulse_<版本>_macos_x64.dmg` |

Windows 推荐 EXE 安装向导；MSI 安装需要相应管理员权限。无需 Rust、Bun 或 Node。缺少 WebView2 时，安装器会联网安装运行时。EXE 和 MSI 选择一种，切换格式前先卸载旧格式。

Windows 首次打开可能显示“未知发布者”或 SmartScreen 提示。确认安装包来自本仓库并完成校验后，按系统提供的“更多信息 / 仍要运行”操作；组织策略禁止运行时，请联系设备管理员。

Mac 打开 DMG，将 TaskPulse 拖入 Applications。命令行安装默认使用 `~/Applications`。尚未经过发行者签名或 Apple 公证的包可能触发系统安全提示；请核对下载来源，并按照 [Apple 官方说明](https://support.apple.com/en-us/102445)操作。TaskPulse 不会自动关闭 Gatekeeper 或清除隔离属性。

首次启动若提示无法验证开发者，请先确认下载自本仓库，再到**系统设置 → 隐私与安全性**，按系统提示选择**仍要打开**。若系统或组织策略没有提供此选项，请联系设备管理员。命令行脚本不会关闭系统安全检查。

若终端显示“已安装，但无法启动”，应用仍保留在输出的安装路径，请核对原始错误和系统提示后打开。升级时若保留了 `.taskpulse-recovery/TaskPulse.app`，这是旧版副本；确认新版本可用后先移走或妥善保留该副本，再移除空的恢复目录，之后重试安装。需要回退时，先正常退出 TaskPulse，在 Finder 中将新应用移到其他位置，再将旧版移回安装目录；配置仍保存在用户主目录。

发布附件提供 SHA-256 校验文件。可按校验文件核对下载内容。

## 命令行安装

Windows PowerShell：

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

macOS 终端：

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

脚本获取 Releases 中的最新正式版本，自动选择架构并校验 SHA-256。没有可用版本时会明确报错，不会改装其他软件。指定版本、仅下载和语言选项见[命令行选项](CLI_INSTALL.md)。

## 开始监控

1. 启动 TaskPulse，确认自动发现的本机 Agent，点击**开始监控**。
2. 回到 Agent 开始一个任务。任务会自动出现在托盘面板，不需要逐个添加对话。
3. 点击托盘图标查看状态；Windows 也可以按 `Ctrl+Shift+A`。

默认关注所有项目的新执行。可以在来源管理中限制项目或关闭来源；自动发现不会恢复你关闭、移除的来源，也不会覆盖已有项目范围。WSL、SSH、MCP 和需要修改 Agent 设置的 Hook 单独配置。

任务进行中、结果待查看和状态待更新分别展示。思考、工具执行和等待模型响应都属于进行中；本轮结束不等于整个项目完成。详见[任务面板](PRODUCT_DASHBOARD.md)。

## 应用更新

默认自动检查新版本，只显示提示。打开**设置 → 应用更新**，点击**下载并安装 · 将重启**后，TaskPulse 才会下载和验证更新签名，然后安装并重启。

- 自动检查可以在同一页关闭，也可随时手动检查。
- 更新保留 `~/.taskpulse/` 中的配置与任务状态。
- 网络或签名校验失败时显示错误，未通过验证的安装包不会运行。
- 检查更新只访问 GitHub 的版本清单，不发送任务内容。
- 旧版没有更新入口时，退出 TaskPulse 后重新运行安装命令或新版安装包。

## 找不到任务时

先清除面板筛选，确认来源已启用且项目在监控范围内。Windows、WSL 和 SSH 是独立来源；WSL Codex 需要运行中的发行版和 Python 3。SSH 使用你配置的别名和系统凭据。

“状态待更新”表示来源暂时不可读，不表示任务已结束。模型、审批或完成信息取决于 Agent 提供的记录，见[兼容说明](AGENT_COMPATIBILITY.md)。

## 数据与隐私

配置、关注记录、最近结果和通知去重保存在本机 `~/.taskpulse/`。TaskPulse 不需要云账号，不向自有云上传对话。SSH 仅连接你添加的服务器，不会自动安装 Hook，也不会替你批准请求。

任务标题和路径可能包含私人信息，请勿将整个状态目录上传到 Issue。通知投递日志最多保留 500 条，不包含命令或提示词，也不代表你已经阅读。

## 卸载

先从托盘退出，然后使用系统卸载功能；Mac 可移除应用。用户状态默认保留，便于重新安装。若安装过 Agent Hook，先在来源管理中撤销，再卸载程序。
