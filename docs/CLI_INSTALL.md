# 命令行安装选项 / CLI options

基础一行命令直接见 [README](../README.md#安装) / [English](../README.en.md#install)。以下是可选参数，无需先阅读本页才能安装。

Scripts select public stable releases by default. Alpha releases require explicit selection. Drafts and PR test packages are rejected. Current candidates are not publicly released.

## Windows 10/11 x64

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

| 参数 / Option | 用途 / Purpose |
| --- | --- |
| `-Prerelease` | 包含公开 Alpha / Include public prereleases |
| `-Version 0.2.0-alpha.8` | 指定已公开版本 / Select a published version |
| `-DownloadOnly` | 只下载校验，不安装 / Verify and save without installing |
| `-OutputDirectory .\downloads` | 下载目录 / Download directory |
| `-Language zh-CN` or `en` | 输出语言 / Output language |

需要参数时先下载脚本，也可先审阅：

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 -OutFile install.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Prerelease
```

Windows 打开标准 NSIS 安装向导；MSI 用户继续使用 MSI，切换格式前卸载旧格式。
仅下载保存到 `TaskPulse-v<version>/`，不覆盖已有目录。

Windows opens the standard NSIS setup wizard. Existing MSI users should stay with MSI; uninstall before switching formats. Download-only creates a new version directory without overwriting an existing one.

## macOS Apple Silicon and Intel

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

| 参数 / Option | 用途 / Purpose |
| --- | --- |
| `--prerelease` | 包含公开 Alpha / Include public prereleases |
| `--version 0.2.0-alpha.8` | 指定已公开版本 / Select a published version |
| `--download-only --output-dir ./downloads` | 仅下载校验 / Verify and save without installing |
| `--dir "$HOME/Applications"` | 安装位置，默认值如左 / Installation directory and default |
| `--no-launch` | 安装后不启动 / Do not launch after installation |
| `--language zh-CN` or `en` | 输出语言 / Output language |
| `--help` | 显示帮助 / Show help |

例如选择公开 Alpha：

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash -s -- --prerelease
```

使用 macOS 自带 Bash 和系统工具，不要求 Python、jq、Bun 或 Rust。按真实硬件选择 arm64/x64，Rosetta 下仍选择 arm64。仅下载目录为 `TaskPulse-v<version>-macos-<arch>/`。

Uses the macOS system Bash and tools. No Python, jq, Bun, or Rust is needed. Architecture selection follows the hardware, including under Rosetta.

## 升级与校验 / Updates and verification

也可在应用设置中检查并安装更新。使用脚本升级前，正常退出 TaskPulse，备份 `~/.taskpulse/`，再运行安装命令。脚本核对来源、版本、平台、架构、manifest 和 SHA-256；失败时不执行新安装包。Mac 同盘暂存、目标锁和回滚保留旧 app；中断恢复目录须先核对再重试。

You can also check and install updates in the application settings. For script-based updates, quit TaskPulse and back up `~/.taskpulse/` before updating. Downloads are checked before installation. Mac updates use same-volume staging, a target lock and rollback. Inspect retained recovery directories after an interrupted update.

当前 Windows 包未做发行者签名，Mac 为 ad-hoc 签名、未公证。SHA-256 验证完整性，不替代发布者身份签名；校验文件与包共享同一仓库信任边界。可先下载审阅脚本，或将脚本 URL 的 main 换成已审阅的提交 SHA。

Current Windows packages have no publisher signing; Mac packages are ad-hoc signed and not notarized. SHA-256 checks integrity, not publisher identity. Review scripts before executing or pin their URL to a reviewed commit.

脚本不安装 Agent Hook，不删除用户状态，不清除 quarantine 或关闭 Gatekeeper。系统拦截请按 [Apple 官方步骤](https://support.apple.com/en-us/102445)核对。卸载、配置和手动安装见[安装指南](INSTALL.md)。
