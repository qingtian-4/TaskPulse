# Verification, packaging and delivery

Read only when the change touches this area. User scope takes precedence.

## Verification and version control

- Run focused tests while iterating, then the full Rust test target.
- Run `cargo clippy ... -- -D warnings`, `bun run check`, and `bun run build` before handoff.
- Run `bun audit` for the full frontend advisory report; `bun run audit:frontend`
  gates moderate and above in verify/quality/release. Record lower findings and
  Rust advisory coverage separately; never claim a threshold pass means zero risk.
- Run `bun run audit:rust` and `bun run audit:secrets` before candidate release.
  These use checksum-pinned workspace-local tools, no ignored advisories/baselines,
  full Git history and redacted reports. Rust failures/incomplete registry checks
  must block; maintenance/unsound warnings remain visible and need review.
- For provider changes, validate against the real local data source when available; do not rely only on fixtures.
- For remote-provider changes, run the ignored live snapshot test and confirm the expected server/project/task in its output.
- For wrapper changes, run `bun scripts/run-wrapper-smoke.ts` on Windows and `python3 scripts/run-wrapper-smoke.py` inside an already-running opted-in WSL/Linux environment. These use isolated target directories, never real watched state paths. `tp-run` requires Python 3; direct foreground child exit does not prove a detached/container/Slurm job ended.
- Keep unrelated user changes intact. Start new feature/fix work on a short-lived `codex/*` branch from the latest `origin/main`. Review the code, run relevant tests and required checks, verify the app runs, then create small logical commits. Never commit a known failing change as a completed task.
- The maintainer's standing workflow authorizes pushing completed repository work, opening a PR to `main`, merging after review and successful Windows quality/Security checks for the current PR head, then deleting the completed local/remote branch and returning to an updated local `main`. A later task-specific instruction can narrow this scope. This does not authorize releases, tags, installing Agent hooks, bypassing protections or discarding unrelated work.
- Prefer merge commits to preserve dependency history. Before deleting a branch, verify its current tip is included in remote `main` and no uncommitted work is lost; inspect changed tips again instead of forcing deletion. Retain unfinished work. If local `main` diverges, inspect unique commits and patch equivalents rather than resetting it blindly.
- Preserve the latest main-branch README, brand assets and screenshots when integrating older branches. Review both sides of conflicts and add only the feature's current documentation; never replace the README wholesale with an older branch copy.
- Remove disposable intermediate logs and failed test artifacts only after validating their resolved paths and retaining useful final evidence, original-change backups and any running executable.

## Desktop releases and updates

- Versions in package.json, Cargo.toml, Cargo.lock and tauri.conf.json must match
  the explicit SemVer tag. Run `bun run release:check` and `bun run test:release`.
- release.yml builds Windows x64 MSVC (plus native ARM64 NSIS after 1.1.0), validates actual installers on separate
  runner, builds both macOS architectures, and creates a **draft only**. Stable
  and pre-release versions require exact explicit tags; never override versions
  or publish automatically. Local maintainer release notes are not public docs.
- Tagged desktop releases require Windows and both macOS artifact/acceptance jobs.
  Linux remains a separate opt-in artifact build.
- NSIS/MSI acceptance runs on separate fresh Windows matrix runners with distinct
  evidence artifacts, and both must pass. Retained install-directory preferences
  make sequential formats a migration test; never delete user preferences to pass
  a fresh-install test. Cross-format migration remains a separate manual gate.
- Installer smoke is CI-only. Installers keep normal install permissions; the
  installed app/IPC/restart helper runs at medium integrity through gsudo. WebView2
  150+ ignores environment debug switches in elevated processes. Never weaken the
  product CSP or skip native checks to fix this; retain phase/error/startup evidence.
- `installer-process.test.ps1` is a non-installing Windows process regression.
  Keep process handles/real exit codes and bounded waits; null ExitCode from a
  fast Start-Process child is not evidence of installer or version failure.
- CI-owned process paths must be compared by final file-handle paths, including
  Windows 8.3 aliases; never weaken ownership to a basename match. Native IPC
  evidence is one result object, not an array of .NET async return values.
- Bounded ephemeral popup queues do not replace durable attention. The bell inbox
  restores unresolved prompts without replaying lifecycle notifications. Manual
  reopening requires fresh matching source health and explicit pending state;
  unknown/stale/inferred/internal/disabled/ignored requests must not be reopened.
- Semantic dashboard sections are ongoing/results/uncertain/pinned; thinking and
  model waits stay ongoing. Failures are results, source faults are uncertainty.

## Installation preview and macOS bootstrap

- Same-repository PR installation previews pin the full PR head SHA, use only
  `contents: read`, and upload SHA-named test artifacts. Keep `build_kind=pr-preview`
  and `tag=null`; public bootstraps reject previews. Do not relax release.yml's
  existing tag/exact-main checks or publish a release through the preview path.
- macOS CLI downloads reuse the architecture-specific DMG. Read legacy manifests/checksums and the unified v2 release metadata; preserve full attachments through 1.1.0 and keep the MSI updater lane in later compact releases.
  Preserve/add and verify quarantine; never remove security attributes or disable
  Gatekeeper. Signature integrity is separate from Developer ID/notarization.
- Directory replacement/rollback must verify device/inode ownership. BSD mv can
  nest into a concurrently created destination; never move or clean a colliding
  foreign directory. Retain and report ambiguous recovery state. Native CI and
  physical-machine/Finder/real-Agent acceptance are separate evidence gates.

## Automatic discovery and application updates

- Fresh onboarding confirms detected read-only local sources once. Automatic discovery preserves disabled/removed sources and explicit project scopes; WSL, SSH, hooks and event integrations stay opt-in. Existing installations keep their previous discovery preference.
- ZCode reads its SQLite database read-only, uses the user message as turn identity and never treats tool completion as turn completion. Completed history belongs in the catalog, not the attention list. Agent name/model remain separate fields.
- Application updates use the official Tauri updater, a compiled public key and an HTTPS GitHub feed. Check automatically, download/install only after a user click; never weaken signature, downgrade or OS security checks. Private keys stay outside Git and PR workflows.
- Cross-version acceptance uses disposable hosted runners, two real installed versions and an audited source-copy overlay with ephemeral signing and a loopback feed. Never add test transport/driver overrides to production. Export evidence only, never test binaries or keys. Verify the actual packaged executable, retained state and restarted process; Windows installer dispatch success alone is not upgrade success. Controlled-feed acceptance does not certify public feed delivery, historical migrations or OS trust prompts.
- GUI lock acquisition allows a bounded shutdown grace for updater restarts: macOS LaunchServices can start the replacement before the old process exits. Never release the old writer lock early while its watcher can still persist state; a second live instance must still activate the existing GUI and exit.
- NSIS includes English and Simplified Chinese with automatic language selection. Keep displayLanguageSelector disabled: a silent CLI install may not save a language, and a later passive app update otherwise blocks on a language dialog.
- Windows installed-upgrade CI uses a disposable standard account, asserting the old/new process SID and TokenElevation=0. A gsudo restricted administrator can still report an elevated token and is not equivalent. Keep test-account passwords off argv/env/artifacts, remove ephemeral signing keys before granting access, and stop account-owned processes before restoring desktop ACLs/removing the account. MSI CI covers administrator-advertised managed deployment: authorize only hash/identity-checked packages in an administrator-protected directory, assert advertisement leaves the old executable installed and the new product only advertised, and record the disposable user's inherited installation-directory preference. Capture unchanged installer/UAC policies. Ordinary MSI UAC acceptance remains a separate interactive-machine gate; never relax installer policy to pass CI or label managed deployment as interactive UAC coverage.
- Release staging must match version and commit across Windows NSIS/MSI and macOS arm64/x64, verify signatures against the compiled key, then assemble latest.json. Publishing a draft requires maintainer approval and platform acceptance.
- Process reports and maintainer plans stay local/ignored; public docs are user and integration guides. Preserve originals before untracking. UI screenshots require explicit suitable user-provided assets; never publish private desktop captures.
