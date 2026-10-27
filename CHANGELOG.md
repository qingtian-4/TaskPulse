# Changelog

TaskPulse-specific changes. Upstream AgentTray history and attribution are retained.

## Unreleased

- Reduce macOS dashboard dimensions by 10% and use a 300×124 reminder. Tighten
  narrow-layout spacing while retaining body text size and Windows defaults.

## 0.2.0-alpha.8 — Candidate; not released

- Exclude internal Codex services and TaskPulse descendants from task discovery;
  preserve native task identity and project scope across process observations.
- Add explicit, reversible Codex approval observation; never guess approval from
  command permission parameters, silence or elapsed tool time.
- Persist manual attention changes atomically, prevent duplicate GUI writers and
  improve notification inbox focus and dashboard refresh ordering.
- Bound subprocess lifetime/output on Windows and Unix; update zstd/tokio fixes,
  pin CI inputs and export dependency inventory, SBOM and available license texts.
- Prepare native Apple Silicon DMGs with ad-hoc signatures, checksums and exact
  source metadata; gate them on tests and isolated installed-app startup/restart.
  macOS desktop interactions, Gatekeeper and real Agent events need manual testing.
- Retain independent fresh-runner NSIS/MSI acceptance and draft-only Windows releases.

## 0.2.0-alpha.7 — Candidate; not released

### Added and refined

- Dynamic task attention, native session titles, opt-in Windows/WSL/SSH sources,
  and separate explicit task reminders; keep source uncertainty distinct from failure.
- Local declarative event integrations and reversible Hook previews/consent/cleanup.
  Kimi/TRAE/ZCode remain partial and fixture-tested; WorkBuddy is candidate-only.
- Bounded incremental parsing, checkpoint/outbox recovery and scoped evidence gaps;
  no automatic replay of unresolved business actions or guessed turn completion.
- Product-first bilingual documentation, compact desktop visual system, search and
  unread filters, source capability/evidence stages, and native synthetic screenshots.

### Fixed

- Select the unmerged Alpha.7 candidate branch in source-start instructions instead
  of cloning the older default main; gate documentation regressions in quality/release.

- Require read-only Codex session evidence before recommending a running WSL
  distribution. Docker/Rancher internal runtimes are not Agent discoveries;
  existing saved sources and project scopes remain unchanged.

### Security and maintenance

- Raise the SvelteKit 2, Svelte 5 and Vite 8 dependency floors to patched versions
  and refresh vulnerable transitive dependencies without crossing major versions.
- Audit locked frontend dependencies at moderate severity and above in local
  verification, Windows quality CI and release builds. The remaining low-severity
  SvelteKit 2 cookie dependency is documented, not silently claimed fixed.
- Consolidate public-release evidence and remaining acceptance/security gates.
- Upgrade notification/XML dependencies and compatible lockfile entries to remove
  the matched quick-xml vulnerabilities, fixed anyhow/event-listener/rand warnings
  and the yanked fastrand version. Remaining maintenance/unsound warnings stay visible.
- Add checksum-pinned, local-only Rust/Git security tools, fail-closed report
  validation, redacted full-history scans and reusable read-only CI release gates.

## 0.2.0-alpha.6 — Installer acceptance passed; draft only

### Fixed

- Isolate NSIS and MSI acceptance in separate fresh Windows runners, with both
  required and distinct evidence. Alpha.5's NSIS lifecycle passed, but its later
  MSI reused NSIS's retained directory preference; sequential formats were not
  independent fresh installs. Preserve preferences and keep format migration a
  separate, unverified acceptance item.

## 0.2.0-alpha.5 — NSIS passed; mixed-format harness failed, not released

### Fixed

- Compare final file-handle paths for owned CI processes, accepting Windows 8.3
  aliases without allowing unrelated executables. Add real temp-profile alias tests.
- Suppress auxiliary .NET async outputs so native acceptance evidence is one IPC
  result, and wait for the old CDP endpoint to close before restart acceptance.

## 0.2.0-alpha.4 — Installed native UI/IPC passed; cleanup harness failed, not released

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
