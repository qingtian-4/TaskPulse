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

完整发布门禁见 [RELEASING](RELEASING.md)。Linux 暂无命令行安装入口。

## macOS Apple Silicon and Intel

本地审阅脚本后使用；远程 main URL 须等合入及公开版本存在后才能安装。
Review the script first. The main URL requires a merged script and a public release.

```sh
curl --fail --location --proto '=https' --tlsv1.2 https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh -o install.sh
less install.sh
bash install.sh --prerelease --language zh-CN
# Pin a public version, download without installing
bash install.sh --version 0.2.0-alpha.8 --download-only --output-dir ./downloads --language en
# Install into a writable directory, without launching
bash install.sh --prerelease --dir "$HOME/Applications" --no-launch
```

参数：`--version`、`--prerelease`、`--download-only`、`--output-dir`、`--dir`、
`--no-launch`、`--language en|zh-CN`、`--help`。默认公开稳定版；预发布明确选择，
指定版本可含 v。预发布选择最近 100 条公开记录中的最新发布时间；缺包不自动跨架构回退。

系统自带 Bash、curl、plutil、shasum、hdiutil、ditto、codesign 与 lipo 完成读取与验证，
用户不需要 Python/jq/Bun/Rust。检测真实硬件，Rosetta 终端仍选 arm64；未知架构停止。
核验 DMG、架构专用 manifest/校验文件、版本、仓库 URL、源 SHA 与 SHA-256，
只读挂载后检查 bundle ID、版本、实际二进制架构和签名完整性。

Uses macOS system tools, including in Rosetta terminals. Unknown architectures fail.
The DMG, architecture-specific manifest and checksums must match the public release.
Signature integrity does not imply Developer ID or notarization; current packages are ad-hoc and not notarized.

默认到 `~/Applications/TaskPulse.app`，可用 `--dir` 指定已有写权限的位置，不自动 sudo。
目标锁阻止并发；拒绝符号链接、其他产品和残留恢复目录。运行中须正常退出后重试。
同文件系统暂存验证后替换，失败恢复旧 app；异常中断的恢复位置会保留并报告，须先核对。
仅下载到 `TaskPulse-v<version>-macos-<arch>/`，同盘原子保存且不覆盖已有目录。

Installs into `~/Applications/TaskPulse.app` by default. Quit the running app before upgrading.
Same-volume staging, a target lock and rollback preserve the previous app on failure.
Interrupted recovery directories require inspection before another install. Download-only never overwrites an existing version directory.

不删除 TaskPulse 状态、不安装 Hook、不清除 quarantine、不关闭 Gatekeeper。curl 下载补充并核验 quarantine，暂存 app 同样核验。
启动遭拦截保留错误并按 [Apple 官方步骤](https://support.apple.com/en-us/102445)核对。
当前 PR 验证不代表 Finder 下载与真实 Agent 实机通过。

State and hooks are untouched. Quarantine and Gatekeeper remain enabled.
Follow [Apple's guidance](https://support.apple.com/en-us/102445) if launch is blocked; retain the actual error.
PR CI acceptance does not replace physical Mac acceptance or an anonymous public download test.

## PR preview acceptance / PR 预览验收

公开入口拒绝 `build_kind=pr-preview` 和 Draft。测试包按[候选说明](CANDIDATE_TESTING.md#当前-pr-的安装预览)
下载；受控传输仅在 CI 测试进程替换响应，不开放生产镜像、执行或跳过校验开关。
Mac 的模拟旧 metadata 用例只证明事务回滚；真实跨版本升级另记。

Public installers reject PR previews and Drafts. CI substitutes only transport responses for exact-SHA packages;
no production mirror or validation bypass is added. Simulated replacement tests do not prove cross-version upgrades.
