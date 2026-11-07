# Changelog

## 0.0.1

[Bilingual release notes](docs/releases/0.0.1.md)

### Added

- Native Kimi Code and Qoder readers, with explicit format and request capability boundaries. TRAE exposes public session discovery and its current storage limits.
- Local Agent settings and multi-server SSH setup with separate connection/data checks and project scopes.
- Reported reasoning effort, frozen end-to-end duration, configurable task text and adaptive four-card panel sizing.

### Improved

- Preserve pending requests, unread results and pins across restarts and incomplete reads.
- Distinguish support, detection, enablement, read health, sessions and active tasks.
- Bound compatibility-file and Claude journal reads; retain trusted state on capacity or format errors.
- Organize runtime, tools and verification entrypoints with shared maintenance guidance, updated bilingual usage and architecture diagrams.

### Fixed

- Windows tray click/focus races, settings focus ownership, source-detection deduplication and verified project-path aliases.
- Codex large-record validation and matching input-request resolution, including DeepSeek Harness.
- Aliased Kimi data roots no longer create false duplicate sessions.

The source version is 0.0.1. Available downloads and migration from the earlier numbering scheme are described in [installation](docs/INSTALL.en.md#version-migration). Copyright, history, configuration and source identities are preserved.

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
