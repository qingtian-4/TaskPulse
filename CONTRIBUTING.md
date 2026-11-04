# Contributing to TaskPulse

Report bugs, improve documentation or add an Agent integration through [issues](https://github.com/qingtian-4/TaskPulse/issues) and pull requests to `main`. Use Bun, Rust 1.90+, Python 3 and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Windows uses the MSVC toolchain in CI; macOS needs Xcode Command Line Tools.

## Daily development — no installation

```sh
bun install --frozen-lockfile
bun run dev:desktop
```

This starts the actual Tauri tray app: Svelte/CSS edits use Vite hot reload; Rust edits trigger a native rebuild and restart through the [official Tauri development lifecycle](https://v2.tauri.app/develop/). No installer, version bump or GitHub Release is needed. `bun run dev` runs only the frontend in a browser; it does not exercise native IPC, tray behavior or providers.

The launcher chooses a free loopback port and a worktree-specific application identifier. Its Cargo target defaults to `src-tauri/target/desktop-dev`; a running app is never replaced by `scripts/verify.ps1`. Stop the launcher with Ctrl+C. Configuration/build-tool changes may require starting it again. `bun run dev:desktop --dry-run` shows the launch command without starting the app.

| Runtime | Settings, attention, history, notification dedup and GUI lock |
| --- | --- |
| Installed release | `~/.taskpulse/` |
| Debug / portable development build | `~/.taskpulse-dev/<build-directory-id>/` |
| Automated native fixture | Explicit temporary `TASKPULSE_HOME` |

Debug builds select their development directory automatically, including direct `cargo run` or an extracted CI executable. Rebuilding in the same directory keeps that development profile; separate worktrees/build directories get separate profiles. The native window is named **TaskPulse Dev**. `agent-tray.exe --profile-info` (Windows) or `agent-tray --profile-info` prints the channel, exact commit and state directory without opening the GUI. Do not copy or link production state into a development profile or point `TASKPULSE_HOME` at the production directory; debug builds reject that override.

On macOS, development webviews use nonpersistent WebKit stores (including macOS 13.3); native configuration/task data still persists in the development profile. `dev:desktop` also provides a separate application identity. Windows/Linux development builds have separate browser storage and do not register login startup, install updates or claim the installed app's global shortcut. Fresh profiles require source confirmation. Native Agent data stays read-only; WSL and SSH remain opt-in. Configure real host hooks only from an installed build in a test account; development builds block real hook changes and use isolated event fixtures. Existing automated fixtures retain their explicit isolation, including disabled real-source discovery.

On Windows, use `bun.cmd` if PowerShell blocks `bun.ps1`. If using a GNU Rust toolchain, configure its C/C++ compiler and linker as described below; do not install packages just to inspect a UI change.

## Windows compiler and verification commands

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

Machine-specific development wrappers such as `dev.sh` are local-only. Use the shared commands above and the [contributing guide](CONTRIBUTING.md) for reproducible setup.


## Check a change

For a fast local iteration:

```sh
bun run check
bun run test:ui-model
bun run test:workflow
```

Before submitting, run the frontend build, Rust fmt/Clippy and the complete Rust test target listed above. Run `bun run build` before embedding assets with Cargo; it rejects a mismatch between the entry pages and their SvelteKit startup scripts. On Windows the shared entry point is:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend
```

It reuses the repository's audit, documentation, release-contract, unit and native bridge checks, and builds into the separate `codex-check` target. Run the relevant `scripts/native-*-smoke.ts --exe <unused-built-exe>` for UI/window changes; on Windows, `native-development-smoke.ts` verifies automatic development isolation without a `TASKPULSE_HOME` override. Provider changes also need real-source validation when available. The opt-in live SSH/WSL tests must use an already available, explicitly selected source.

Windows tray acceptance requires a clean binary for an explicit full commit SHA. Run `bun tests/native/native-tray-smoke.ts --exe <unused-built-exe> --expected-commit <full-SHA> --callbacks-only`, then repeat with `--physical-only` during a quiet desktop interval. Reports retain the compiled commit, dirty flag, executable SHA-256 and separate cleanup evidence.

## Share a temporary test build

Every successful **Windows quality** run uploads an **installation-free Windows development build**. Open its run summary for the download link and ZIP SHA-256, or download the artifact named `taskpulse-dev-windows-x64-<full-SHA>` with the GitHub CLI. Extract the whole ZIP into a new folder and run `agent-tray.exe`. The manifest identifies the exact source SHA, channel and per-file SHA-256 values; keep `WebView2Loader.dll` beside the executable when present. This uses the isolated development profile above, so the installed application can stay open.

Temporary development, installer-preview and upgrade-evidence artifacts expire after **7 days**. [GitHub artifact retention](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/remove-workflow-artifacts) applies to new uploads; existing older artifacts keep their previous expiry. Download through the run summary, check the SHA and save the final evidence locally before expiry. No application profiles, captured private sessions or signing keys belong in artifacts.

The **macOS compile** workflow checks shared Rust changes and their build inputs on PRs and on main. For package-specific changes, opt into the heavier PR workflows:

| PR label | What it exercises |
| --- | --- |
| `test:installers` | Windows EXE/MSI fresh installs plus both macOS architectures and CLI packaging |
| `test:upgrade` | Installed cross-version upgrades on Windows and both macOS architectures |

Adding the label starts its workflow; subsequent PR commits rerun it with the new exact head. Removing the label stops future package runs. These workflows use read-only permissions and never publish a Release. Installer previews exercise release identity: use a clean VM/test account, rather than installing them over daily production data. Use the portable development artifact for ordinary same-machine testing.

## Add an Agent

Start with the [integration guide](docs/AGENT_INTEGRATION_GUIDE.md) and [event protocol](docs/AGENT_INTEGRATION_PROTOCOL.md). Prefer official lifecycle events; native collectors remain read-only. Keep stable session IDs, titles, projects and model IDs separate. Never infer completion from a tool result, silence or CPU usage. Keep hooks explicit and reversible; cover renames, new turns, missing data, exclusions and duplicate notifications.

## Submit and release

Keep PRs focused and include validation. Preserve unrelated work and upstream attribution. Never commit credentials, private task data, build products or investigation notes. Contributions use the [MIT License](LICENSE), including the original AgentTray attribution.

A formal Release is for an accepted **stable** version, not an iteration/testing channel:

1. Pass quality/security checks at the final PR head, run the relevant installer/upgrade workflows, and complete physical Windows/macOS acceptance. CI's controlled updater feed does not replace public-feed, OS trust/UAC or actual-Agent checks. Merge only after maintainer approval.
2. Choose the next stable SemVer and synchronize `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json` and the derived MSI version. Add concise user-facing notes in `docs/releases/<version>.md`.
3. Run `bun run release:check`, `bun run test:release`, `bun audit`, `bun run audit:rust` and `bun run audit:secrets`. Review lower advisories too; incomplete or failing mandatory checks block release.
4. The maintainer creates and pushes the exact approved `vMAJOR.MINOR.PATCH` tag on `main`. The existing Release workflow builds Windows and both macOS architectures, validates installers/signatures, assembles the signed feed and creates a **draft only**. Prerelease tags are rejected by this formal workflow; use temporary Actions artifacts for testing.
5. Recheck the draft's exact commit, manifests, checksums, platform evidence and update feed, then explicitly publish it. No ordinary PR, development build or verification command creates tags or publishes a Release.

### Same-commit release evidence

Write `docs/releases/<version>.md` in Chinese and English with Added, Changed,
Fixed, Security/compatibility, Installation/upgrade and Known limitations sections
as applicable. Describe user-visible changes since the previous public version.
The draft pipeline puts actual installation links above these sections. Missing
architectures do not receive download links.

Versions through 1.1.0 retain all legacy attachments. Later versions use four
primary installers (Windows x64/ARM64 NSIS and macOS Apple Silicon/Intel DMG),
two Mac updater archives, `latest.json`, a unified `release-manifest.json`,
`SHA256SUMS.txt` and one dependency/license ZIP. Retain the additional Windows
x64 MSI because installed MSI clients select their own updater target. This
gives 11 uploads, plus GitHub's two generated source archives. The 10-upload
target requires a separately verified MSI migration.

The ZIP keeps architecture-specific inventories, SBOMs, license notices,
original manifests/checksums and signature sidecars. Updater signatures also
remain embedded in `latest.json`. Unified checksums cover every other public
attachment. Each platform verifies signatures against the compiled key before
aggregation. Both Windows x64 formats and ARM64 NSIS native acceptance must
pass before a compact draft can be assembled. Published assets stay immutable.
Windows ARM64 must not be advertised as verified before its build, installation
and physical-machine evidence exists.

Choose one clean, full candidate SHA and record the version beside it. For each
applicable check, record its status, run or report link, and tested SHA. Retain
artifact manifests and SHA-256 checksums before Actions artifacts expire. A
passing PR head can gate its merge; the resulting main commit has a different
SHA and needs its own evidence for release. If a row was skipped or tested a
different commit, leave it open or mark it out of scope with a reason.

| Evidence | What to record |
| --- | --- |
| Source and checks | Full candidate SHA on main, version checks, Windows quality and Security runs, and lightweight macOS compile when shared Rust or its build inputs changed. Record frontend/Rust/history audit results separately. |
| Native behavior | Full build commit, `modified=false`, executable SHA-256, OS/build, architecture, runtime, exercised behavior and cleanup. Record fixtures, real Agents and physical desktop tests separately. |
| Resources | Same workload and host for baseline/candidate. Record interval, duration, process coverage, peak and steady working set, private bytes and CPU. On Windows include the verified WebView2 descendants. On macOS attribute WebKit content, GPU and networking services to the candidate; report missing attribution as incomplete. |
| Display and sleep | On physical hardware record monitor arrangement and scales, tray/menu behavior and settings layout before/after scale change, external monitor disconnect and actual OS sleep. Check that fresh evidence resumes, the same unresolved request remains and duplicate alerts do not fire. |
| OS trust | Record Windows Authenticode/publisher status, SmartScreen and ordinary interactive MSI UAC separately from CI. On Mac record the signing identity, hardened runtime, notary and stapling results if configured; test first launch from Finder with quarantine retained on both architectures. |
| Fresh installation | Windows NSIS and MSI separately; macOS arm64 and x64 separately. Record package hashes, matching manifests, install/launch/cleanup and the trust path actually exercised. |
| Installed upgrade | Record old/new versions, package hashes, actual restarted executable SHA-256, version/commit from IPC, rollback on failure, state retention and signature checks. Distinguish CI controlled-feed verification from public HTTPS feed delivery. Keep Windows NSIS and MSI lanes separate. |
| Real request evaluation | Keep per Agent, version, platform and signal cohorts from the [request evaluation guide](docs/REQUEST_EVALUATION.md). Fixture replay, renderer load and short pilots cannot certify real accuracy, render latency, recovery, host return or sustained resources. |
| Draft assets | Verify the approved tag points at the candidate SHA, all packages and manifests match it, dependency notices and checksums are present, and the signed update feed matches the packages. Check the private vulnerability reporting setting and link. |

Use the opt-in PR labels for installer and upgrade checks when the change needs
them. The macOS compile job only type-checks Rust (including test code). It does
not build installers, launch the application, or replace either macOS release
artifact and its native acceptance. Keep the filled checklist with the local
release evidence or PR; the checklist itself does not publish anything.


For a clean Windows development executable, run the isolated settings and
process-tree sample with no screenshot capture:

```powershell
bun tests/native/native-settings-smoke.ts --exe <exe> --commit <full-sha> --sample-resources --no-screenshots
```

The report counts the root process and validated live descendants, including
WebView2. The sum of working sets can count shared pages more than once. CPU
values are lifetime totals for the sampled processes, not utilization rates.
Coverage is limited to descendants reachable in the snapshot. An exited or
orphaned child and any service outside that process tree are outside the sample. Keep app-only
counters separate from tree totals. Do not count absent or partial child
coverage as zero overhead. On Mac a compile check cannot attribute WebKit
services or validate a real display or sleep cycle.

The current Windows packages have no publisher signature, and the current
macOS build intentionally uses an ad-hoc signature and asserts that in its
package check. A trusted-publisher lane needs a provisioned Windows signing
identity or macOS Developer ID Application identity and notary credentials,
plus matching packaging verification. If adding that lane, follow [Microsoft
signature verification](https://learn.microsoft.com/en-us/windows/win32/seccrypto/using-signtool-to-verify-a-file-signature)
or [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
Record the hashes of final distributed bytes after any signing and stapling.
The Tauri updater signature is separate; retain the embedded-key verification
and do not turn a signature failure into an install attempt. Record unavailable
certificates, hardware and host cohorts as open items in the local evidence.
