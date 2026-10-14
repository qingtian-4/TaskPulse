# Changelog

TaskPulse-specific changes. Upstream AgentTray history and attribution are retained.

## 0.2.0-alpha.1 — Unreleased

### Added

- Windows Alpha draft release pipeline with tag/SHA/version checks, locked builds,
  deterministic installer names, SHA256 manifests and a separate installer smoke job.
- Unresolved-request inbox reconstructed from attention state after restart.
- Explicit reopening of fresh confirmed prompts, without approval, acknowledgement,
  lifecycle redelivery or resetting notification deduplication.

### Changed

- Ongoing work first, failed/ended results separate from source uncertainty;
  semantic classification shared by filters, counters and task groups.
- Popup queue bounded to 128 entries. Blockers can rotate without dismissal;
  overflowed items remain in the task/result inbox.
- Linux/macOS builds are separate opt-in experimental artifacts.

### Fixed

- False Hermes turn ends on lease loss, Claude event overwrites/parallel request
  deduplication, remote collection failures masquerading as empty successful snapshots.
- Long-run retention, same-pane run identity collisions and foreground wrapper
  arguments/stdio/exit-code handling.
- Native desktop/WSL source discovery and session titles; Guardian/internal remote
  sessions no longer emit user completion notices.

### Known limitations

No dedicated Claude WSL/SSH or Hermes WSL adapter, universal run/attach CLI,
executable plugin marketplace, or independent quota toggle. Source format and
approval-event coverage is version-dependent. See [release notes](docs/releases/0.2.0-alpha.1.md).
