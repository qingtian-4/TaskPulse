# Changelog

TaskPulse-specific changes. Upstream AgentTray history and attribution are retained.

## 0.2.0-alpha.4 — Unreleased

### Fixed

- Retain the native process handle for installer/version exit codes and bounded
  waits. Alpha.3 installed the correct version, but its test misread a fast child's
  null ExitCode as failure. Add real fast-zero/nonzero/output/timeout regressions;
  keep the actual semantic-version assertion and record exit/output evidence.

## 0.2.0-alpha.3 — Installers built, version-check harness failed; not released

### Fixed

- Separate privileged installation from ordinary-user native app acceptance.
  New WebView2 versions ignore environment debug flags in elevated CI hosts;
  a CI-only medium-integrity helper now runs mandatory UI/IPC/restart checks.
- Bound installer/startup waits and retain phase, error, stdout/stderr and probe
  evidence even if the first launch fails. No product security policy is weakened.

## 0.2.0-alpha.2 — Installers built, native acceptance failed; not released

### Fixed

- Actual Windows packaging found that MSI rejects non-numeric `alpha.N`. Keep
  the app/tag version unchanged and derive/validate numeric WiX metadata separately
  (`0.2.0-alpha.2` → `0.2.0.1002`). Runtime `--version` remains the semantic version.
- Retain the failed alpha.1 tag without rewriting it. Installers and draft
  publication still require actual CI smoke and remaining manual acceptance.

## 0.2.0-alpha.1 — Build failed, not released

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
