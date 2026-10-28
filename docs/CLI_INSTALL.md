# 命令行安装 / Command-line installation

当前仍是未公开候选阶段。`install.ps1` 合入 main 且对应 Release 经人工验收并公开后，远程命令才可使用；脚本不会安装 Draft。开发分支可用 `powershell -ExecutionPolicy Bypass -File .\install.ps1 -Prerelease -DownloadOnly` 测试无公开版本时的提示。

The project is still at the unpublished candidate stage. The remote commands become available after this script reaches main and a reviewed Release is public. Drafts are never selected.

## Windows 10/11 x64

公开 Alpha / Public alpha (explicit opt-in):

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1))) -Prerelease
```

公开稳定版 / Public stable release (when one exists):

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

先下载并审阅脚本 / Download and inspect first:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 -OutFile install.ps1
Get-Content .\install.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Prerelease -Language zh-CN
```

`-Language en` selects English; `zh-CN` selects Simplified Chinese. By default the script follows the Windows UI language (Chinese maps to Simplified Chinese). These are the only two languages added here.

## 指定版本与仅下载 / Pin a version or download only

以下版本仅为候选示例，须先在 Releases 公开 / This example requires the candidate tag to be published first:

```powershell
.\install.ps1 -Version 0.2.0-alpha.8 -DownloadOnly -OutputDirectory .\downloads -Language en
```

`-Version` accepts a published version with or without `v`, including an explicitly selected prerelease. Without a version, the default uses GitHub's latest stable release; `-Prerelease` selects the most recently published release from the latest 100 entries, including prereleases. Use an exact version for older releases. No public stable version means a clear failure, not an automatic alpha fallback.

仅下载会新建 `downloads/TaskPulse-v<版本>/`，保留安装包、SHA256SUMS 和 manifest；已有目录不覆盖。下载失败或校验失败不会启动安装器。网络请求有超时，不需要 GitHub 登录、Bun、Node 或 Rust。

Download-only saves the installer, checksums and manifest in a new version directory without overwriting an earlier download. Network or verification failures stop before execution. No GitHub login or development tools are required.

## 安装、升级与信任边界 / Installation, updates and trust

脚本核对仓库、精确资产名、tag、版本、平台、架构、源 SHA 格式和安装包 SHA-256，再打开**标准 NSIS 安装向导**。升级时备份 `~/.taskpulse/` 并从托盘退出，重新运行命令；由现有安装器负责安装/升级/卸载。已经使用 MSI 的用户继续使用 MSI，切换格式先卸载旧格式。脚本不自动改开机启动、删除设置、安装 Hook、停止其他进程或静默提权。

The script verifies the repository asset URL, exact filename, tag, version, platform, architecture, source SHA format and installer SHA-256, then opens the **normal NSIS setup wizard**. Back up `~/.taskpulse/` and exit the tray before updating. Existing MSI users should continue with MSI; uninstall before switching formats. Existing installer behavior handles settings and uninstall. The bootstrap does not change startup preferences or install hooks.

当前安装包未代码签名，Windows 可能提示未知发布者。SHA-256 可以检查下载完整性，不能替代发布者签名，也不能抵御仓库自身被入侵。manifest 与校验和来自同一 Release；源 SHA 仅作版本归属检查，不是签名证明。下载远程脚本后会执行其内容，可选择先审阅并改用已审查的提交 SHA URL。

Current packages are unsigned. Checksums verify download integrity, not publisher identity or a compromised repository. The manifest and checksums share the release's trust boundary. Review the bootstrap before execution and pin its URL to a reviewed commit when desired.

仅 Windows x64 接入此入口。macOS/Linux 继续按实验平台说明获取资产，不将实验构建自动提升为稳定安装渠道。完整发布门禁见 [RELEASING](RELEASING.md)。
