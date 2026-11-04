# Changelog

## Unreleased · 1.1.0

[Bilingual release notes](docs/releases/1.1.0.md)

### Added

- Source capability/platform metadata, grouped settings and advanced connections.
- Request replay/evaluation tools and same-commit release acceptance guidance.
- Per-file/shared status read budgets and Claude journal admission/recovery safeguards.
- Windows resource sampling including verified WebView2 descendants.

### Changed

- Separate source support, data detection, enablement, health, sessions and current tasks.
- Preserve explicit requests, unread results and pins across incomplete reads and restarts.
- Reduce repeated Claude admission I/O and omit development debug symbols.
- Update attention-focused bilingual product copy and prepare consolidated future release assets, retaining current attachments and MSI updates.

### Fixed

- Windows tray click/focus races and late popup readiness stealing settings focus.
- Scanner detection disappearing when all records are deduplicated.
- Standard-account probe cold-start stalls and mismatched frontend bootstrap assets.

新增来源能力与高级连接分组、读取预算、日志准入和恢复保护、包含 WebView2 的资源采样。修复托盘竞争、来源检测漏报、探针冷启动及前端构建资源不匹配。当前公开附件保持完整，后续紧凑布局保留 MSI 更新兼容包。真实来源与平台实机验收的剩余范围见更新说明。

## 1.0.1 · 2026-10-31

Fix macOS command-line installation when optional release metadata fields are absent. [Release details](docs/releases/1.0.1.md)

修复 macOS 命令行安装对缺省发布字段的读取，版本、架构、校验和与系统安全检查继续保留。

## 1.0.0 · 2026-10-31

First public TaskPulse release. [Release details](docs/releases/1.0.0.md)

- Monitor local, WSL and SSH Agent tasks with status, model and elapsed time.
- Keep task reminders, unread results and pinned tasks available across restarts.
- Customize appearance, language, login startup and monitoring sources.
- Install Windows/macOS packages or use one command; receive signed in-app updates.

首次公开发布，支持本机及远程任务监控、提醒与未读保留、中英文设置、Windows/macOS 安装和应用内更新。

TaskPulse is based on AgentTray; its original copyright and contribution history remain preserved.
