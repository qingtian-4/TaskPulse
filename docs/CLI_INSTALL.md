# 命令行安装选项

基础一行命令直接见 [README](../README.md#安装)。以下是可选参数。脚本默认选择公开的 v0.0.1 安装包。

## Windows x64 / ARM64

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex
```

| 参数 | 用途 |
| --- | --- |
| `-Version 0.0.1` | 指定已公开版本 |
| `-DownloadOnly` | 只下载校验，不安装 |
| `-OutputDirectory .\downloads` | 下载目录 |
| `-Language zh-CN` 或 `en` | 输出语言 |

需要参数时先下载脚本，也可先审阅：

```powershell
irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 -OutFile install.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Version 0.0.1
```

Windows 打开标准 NSIS 安装向导；MSI 用户继续使用 MSI，切换格式前卸载旧格式。
仅下载保存到 `TaskPulse-v<version>/`，不覆盖已有目录。

GitHub API 限流时，稳定版和明确指定的版本会改用同一仓库的公开 Release 页面确定版本，仍核对安装包、manifest 和 SHA-256。临时连接失败会重试一次；失败信息包含具体附件及官方地址。可将该地址在浏览器中打开，区分脚本读取成功与附件下载是否可达。

## macOS Apple Silicon / Intel

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash
```

| 参数 | 用途 |
| --- | --- |
| `--version 0.0.1` | 指定已公开版本 |
| `--download-only --output-dir ./downloads` | 仅下载校验 |
| `--dir "$HOME/Applications"` | 安装位置，默认值如左 |
| `--no-launch` | 安装后不启动 |
| `--language zh-CN` 或 `en` | 输出语言 |
| `--help` | 显示帮助 |

例如指定版本：

```sh
curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash -s -- --version 0.0.1
```

使用 macOS 自带 Bash 和系统工具，不要求 Python、jq、Bun 或 Rust。按真实硬件选择 arm64/x64，Rosetta 下仍选择 arm64。仅下载目录为 `TaskPulse-v<version>-macos-<arch>/`。

## 升级与校验

也可在应用设置中检查并安装更新。使用脚本升级前，正常退出 TaskPulse，备份 `~/.taskpulse/`，再运行安装命令。脚本核对来源、版本、平台、架构、manifest 和 SHA-256；失败时不执行新安装包。Mac 同盘暂存、目标锁和回滚保留旧 app；中断恢复目录须先核对再重试。

当前 Windows 包未做发行者签名，Mac 为 ad-hoc 签名、未公证。SHA-256 验证完整性，不替代发布者身份签名；校验文件与包共享同一仓库信任边界。可先下载审阅脚本，或将脚本 URL 的 main 换成已审阅的提交 SHA。

脚本不安装 Agent Hook，不删除用户状态，不清除 quarantine 或关闭 Gatekeeper。系统拦截请按 [Apple 官方步骤](https://support.apple.com/en-us/102445)核对。卸载、配置和手动安装见[安装指南](INSTALL.md)。
