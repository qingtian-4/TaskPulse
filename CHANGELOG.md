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

TaskPulse's first official release. Built on AgentTray with its MIT attribution and contribution history preserved.
