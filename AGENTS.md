# AGENTS.md

Guidance for coding agents working on this TaskPulse repository.

## Product and platform

TaskPulse is a Windows-first Tauri 2 tray application for monitoring active AI-agent tasks. It is forked from AgentTray, but the current product is an attention dashboard rather than a process-list mirror. Preserve Linux and macOS compatibility where practical; validate the primary path on Windows.

The main user question is: which tasks are running, failed, waiting for input, or completed and still unviewed? Do not import an Agent's full idle history and do not invent demo task names for real provider output.

## Stack

- Backend: Rust, Tauri 2, `notify`, SQLite/JSON provider adapters.
- Frontend: SvelteKit, Svelte 5 runes, Tailwind CSS 4.
- Package manager: Bun, not npm.
- Icons: `@lucide/svelte`; native tray PNGs are embedded with `include_bytes!`.
- Primary host: Windows 10/11; DeepSeek Harness Desktop uses native storage, and the legacy WSL installation is monitored separately.

## Architecture

The runtime pipeline is:

`Provider -> scope filtering/normalization/dedup -> TaskStore -> AttentionTracker -> TransitionEngine -> tray/UI/notifications`

Important locations:

- `src-tauri/src/providers/`: Codex, Hermes, DeepSeek Harness, local status files, and remote tmux providers.
- `src-tauri/src/watcher.rs`: polling orchestration, provider health, merging, and Tauri update events.
- `src-tauri/src/store/`: authoritative-source reconciliation and recent-task retention.
- `src-tauri/src/transition.rs`: lifecycle transitions and one-shot notification events.
- `src-tauri/src/attention.rs`: dynamic attention list (running, unread, failed/waiting, and pinned tasks).
- `src-tauri/src/monitoring.rs`: source instance registry, legacy migration, hot reload and discovered-session catalog.
- `src-tauri/src/history.rs`: recent terminal-task persistence.
- `src-tauri/src/tray.rs`: aggregate tray state and embedded tray icons.
- `src-tauri/src/task_notice.rs` and `src/routes/notice/`: non-activating task reminder window and separate-event queue. Dismissal must never mark a task viewed; stale timer IDs cannot dismiss the next alert.
- `src/routes/+page.svelte` and `src/lib/components/`: popup UI.
- `src/routes/manage/`: independent management window; permissions include both `popup` and `management`.
- `docs/PRODUCT_DASHBOARD.md`: task-first presentation, source configuration, blocking vs normal end, notification rules.
- Lifecycle and provider degradation rules live below and in the corresponding backend tests; detailed maintainer notes stay local.

Stable session/thread IDs are task identity. A title rename updates presentation only; it must not create a new task, reset lifecycle state, or send another notification. Project and task title remain separate model fields.

Execution `phase` is independent of lifecycle: thinking, tool execution, responding and waiting for a model response all remain RUNNING. Clear tool activity by matching call ID on tool results; never infer an Agent turn end from low CPU, silent logs, missing files or a tool return. Only explicit lifecycle events notify; inferred finish is limited to non-Agent tmux shell-return snapshots. Same stable turn status flapping cannot reset notification dedup. Attention v4 conservatively archives v3 false Claude history imports without acknowledging/deleting them; back up to `attention.json.pre-lifecycle.bak` before migration.

The main view defaults to a configurable 7-day execution-activity window (`dashboard.recent_days`, zero disables archival). Inactive attention moves to a separate older-attention view without deleting unread/pin state. Running tasks and explicit TaskPulse manual pins never expire. Legacy pin origin is unknown and does not exempt old records from archival; preserve their flag and allow explicit re-pinning. Polling/renames must not renew execution activity. Preserve existing project allowlists unless the user changes them; new sources default to all projects' new executions.

## Data and configuration

Codex command permission parameters, slow tool results, silent logs and CPU activity are never proof of pending human approval. Do not infer authorization waits or emit timed approval-check reminders. Only an explicit source approval request sets WAITING_INPUT / approval, cleared by its matching request ID. Preserve RUNNING tool/thinking phases; legacy awaiting_tool snapshots remain readable and are not human blockers. Never execute Code Mode snippets or install hooks/change approval policies automatically. Confirmed blocking popups have no automatic timer, may rotate without dismissal, and reconcile only against fresh matching observations. Delivery evidence in `~/.taskpulse/notification-log.json` is bounded to 500 records, excluded from collection/watcher triggers, and never asserts user-read or includes commands/prompts/credentials. See `docs/PRODUCT_DASHBOARD.md` for source limitations.

- TaskPulse state: `~/.taskpulse/attention.json` and `~/.taskpulse/recent.json`.
- Notification dedup: `~/.taskpulse/notifications.json` (must survive restarts).
- Monitoring/remote configuration: `~/.taskpulse/config.toml`.
- Source registry uses `[dashboard]` in that config; once saved it supersedes legacy scopes. UI saves apply immediately; preserve unknown config fields and create the one-time migration backup. Fresh installs require explicit onboarding confirmation.
- Legacy/generic hook input: `~/.agent-monitor/` and `~/.taskpulse/` status files.
- Codex: native thread/app-server metadata plus rollout logs; local and configured remote projects are supported.
- Codex WSL: independent opt-in `codex_wsl` sources per running distro. Read metadata inside Linux using Python 3 / read-only SQLite plus `session_index.jsonl`; never open WSL SQLite/WAL through Windows UNC. Reuse the Codex incremental rollout parser via `\\wsl.localhost\<distro>`. IDs are `<source-instance>:codex:<thread-id>` and Linux project paths stay separate from Windows / SSH. Never auto-wake a stopped distro or query Windows account quota for WSL tasks. See `docs/WSL_CODEX.md`.
- Claude Code: read-only `CLAUDE_CONFIG_DIR` (default `~/.claude`) projects/*.jsonl, sessions/*.json and IDE *.lock evidence; never read *.key. Native formats are best-effort/inferred. Explicit, reversible official hooks write a minimal event envelope to `~/.taskpulse/claude-events/`. Agent identity is independent from the reported/proxy model. Never auto-install hooks or overwrite statusLine/other hooks/proxy settings.
  Hook envelopes use an OS-locked per-session JSONL journal with monotonic sequence; latest JSON is a compatibility checkpoint. PermissionRequest has no native call ID: associate only a unique preceding tool/input signature, preserve all explicit pending request IDs, and resolve only a matching call or explicit end. Native thinking/busy/idle metadata cannot clear an approval or prove turn end. File locking requires Rust 1.89+; the signed updater raises the project minimum to Rust 1.90.
- Hermes: native SQLite session metadata and runtime lease state.
- DeepSeek Harness: native Desktop/CLI `$DSH_HOME` (default `~/.dsh`, override with `TASKPULSE_DSH_HOME`) plus an independent WSL worker. Support both `session.jsonl.zstd` and `session.v4.jsonl.zstd`. Native v4 titles/state use `storages/session_projcache/sessions/<id>.json` (`version`/`record` envelope); legacy versions use `storages/session_projcache.json`. Projection event seq must not override a newer log state. Native IDs are `dsh-native:<session-id>`; existing WSL IDs stay `dsh:<session-id>`.
- Remote: configured SSH hosts, projects, and tmux sessions/panes.

## Commands

Use Bun's executable directly if PowerShell blocks `bun.ps1`:

```powershell
$env:PATH = "C:\tools\mingw64\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CC = 'gcc'
$env:CXX = 'g++'
$env:RUSTFLAGS = '-C link-self-contained=yes'
& "$env:LOCALAPPDATA\hermes\node\bun.cmd" run check
& "$env:LOCALAPPDATA\hermes\node\bun.cmd" run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --bin agent-tray -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray
cargo build --manifest-path src-tauri/Cargo.toml --bin agent-tray --features custom-protocol
```

When the tray app is running, use the isolated verification target so Windows
never has to replace the live executable:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1
```

For the fully configured GNU/MinGW development environment on this machine:

```bash
bash dev.sh
```

Run the opt-in real SSH smoke test only when the configured servers are expected to be reachable:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_remote_snapshot -- --ignored --nocapture
```

Opt-in WSL read-only smoke (Python 3 and a running distro required, default Ubuntu):

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_wsl_codex_snapshot -- --ignored --nocapture
```

Set `TASKPULSE_WSL_DISTRO` for another distro. `scripts/native-wsl-codex-smoke.ts` validates actual source discovery, save, native session titles and isolated scopes with temporary WebView2 CDP, restoring settings by default.

## Windows build and tray process

The tray PNGs are compile-time embedded. Do not add `icons/tray-*.png` back to Tauri `bundle.resources`; copying those redundant runtime resources caused Windows file-lock failures while the tray app was running.

Windows WebView2 creation must use async commands or a separate thread, never synchronous IPC/event handlers. Keep the narrowly scoped `ipc:` / `http://ipc.localhost` CSP connect permissions; actual native IPC/window tests catch failures that browser mocks cannot.

Reminder delivery defaults to the independent TaskPulse popup (`dashboard.notification_mode=popup`, alternatives `system`/`both`). The master `notifications` switch covers all channels. Only lifecycle events trigger task alerts; tests are explicitly labelled and never imported as monitored tasks. Run `scripts/native-notice-smoke.ts` for actual popup visual/click/focus acceptance, not just system notification API success.

While `src-tauri/target/debug/agent-tray.exe` is running, use `scripts/verify.ps1`; it builds into `src-tauri/target/codex-check`, so the linker never replaces the live executable. To rebuild the executable used by the tray itself, stop only the workspace-owned process, run the normal build, and restart it. Never kill unrelated processes by name without verifying their executable path.

`tempfile` is intentionally a normal dependency: production code uses same-directory temporary files for atomic replacement of `attention.json` and `recent.json`. Tests also use it.

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
- release.yml builds Windows x64 MSVC, validates actual installers on a separate
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
- macOS CLI downloads reuse the architecture-specific DMG, manifest and checksums.
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
- Release staging must match version and commit across Windows NSIS/MSI and macOS arm64/x64, verify signatures against the compiled key, then assemble latest.json. Publishing a draft requires maintainer approval and platform acceptance.
- Process reports and maintainer plans stay local/ignored; public docs are user and integration guides. Preserve originals before untracking. UI screenshots require explicit suitable user-provided assets; never publish private desktop captures.
