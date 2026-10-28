# TaskPulse v<version> · Preview / 预览

> Copy this template to `<version>.md` after selecting an explicitly authorized version.
> 此模板不是发布授权。所有未执行项目写“未验证”，不要从旧版本复制通过结论。

## Changes / 变化

Describe only behavior present in the candidate, with one concrete user outcome per item.
仅列候选已实现的变化，每项说明用户可观察的结果。

## Packages and requirements / 安装包与要求

| Platform / 平台 | Artifact / 资产 | OS tested / 已测系统 | Status / 状态 |
|---|---|---|---|
| Windows x64 | NSIS EXE / MSI | Record exact Windows build / 记录系统版本 | Not verified / 未验证 |
| macOS Apple Silicon | arm64 DMG | Record macOS version / 记录系统版本 | Experimental; not verified / 实验、未验证 |
| macOS Intel | x64 DMG | Record macOS version / 记录系统版本 | Experimental; not verified / 实验、未验证 |

Windows needs WebView2. A configured deployment target is not compatibility evidence.
Windows 依赖 WebView2；构建最低目标不等于各系统验收通过。

## Installation and upgrades / 安装与升级

Package installation: link exact assets, manifest and checksum files; do not link a Draft as a public download.
CLI installation: link the reviewed script revision and public tag only after publication. PR artifacts cannot be consumed by the public bootstrap.

包安装：列精确安装包、manifest 和校验文件。CLI 安装：记录脚本 SHA 和公开 tag；PR 附件不能通过公开入口安装。
升级前退出 TaskPulse 并备份 `~/.taskpulse/`。记录起始版本与目标版本，核对来源范围、未读、固定及通知去重。
Windows 切换 EXE/MSI 格式另行验收；重装不等于跨版本升级。

## Trust and limitations / 信任与限制

Record Windows Authenticode, macOS signing identity and notarization separately.
Current experimental Mac packages are ad-hoc signed and not notarized; checksums do not identify a publisher.
Quarantine and Gatekeeper remain enabled. Follow [Apple guidance](https://support.apple.com/en-us/102445) for system alerts.

分别记录 Windows 签名、Mac 签名与公证。实验包当前 ad-hoc、未公证，不移除安全属性。
列来源/模型未知值、事件支持程度及尚未验收的平台，不将 CI 代替真实 Agent/桌面体验。

## Provenance and acceptance / 提交与验收

Source commit: record full SHA from each manifest. / 源提交：填写 manifest 中的完整 SHA。
Script SHA-256: record each bootstrap digest. / 脚本校验：填写各安装脚本的校验值。
Packages / 包校验：Windows `SHA256SUMS.txt`；Mac `macos-<arch>-SHA256SUMS.txt`。
Manifests / 元数据：`release-manifest.json`、`macos-<arch>-manifest.json`。

| Check / 检查 | Windows | Mac arm64 | Mac x64 | Evidence / 证据 |
|---|---|---|---|---|
| Package install + actual app / 包安装与实际 app | Not verified / 未验证 | Not verified / 未验证 | Not verified / 未验证 | CI artifact/run |
| CLI download/install / CLI 下载与安装 | Not verified / 未验证 | Not verified / 未验证 | Not verified / 未验证 | Script + package digest |
| Cross-version upgrade + state / 跨版本升级与状态 | Not verified / 未验证 | Not verified / 未验证 | Not verified / 未验证 | Old/new version + checklist |
| Real Agent lifecycle + model / 真实 Agent 与模型 | Not verified / 未验证 | Not verified / 未验证 | Not verified / 未验证 | OS + Agent version, redacted steps |
| Browser/Finder security + desktop / 下载安全与桌面体验 | Not verified / 未验证 | Not verified / 未验证 | Not verified / 未验证 | Physical machine / 实机 |
| Anonymous public download / 匿名公开下载 | Not verified / 未验证 | Not verified / 未验证 | Not verified / 未验证 | After separate publication / 单独授权公开后 |

Maintainer stability confirmation / 维护者稳定确认：pending / 待确认。
Version/tag/Draft creation requires separate authorization. Keep existing immutable tags.
发布仍需单独授权，同步四处版本后再创建不可变 tag 和 Draft；不移动旧 tag。
