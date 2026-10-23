# Scripts

`bun run test:repo` checks all Git-tracked Markdown links/anchors, SVG references,
ignored tracked outputs and disposable media. It skips fenced code examples and
does not fetch external URLs. New documentation must be staged to enter this inventory.
`verify.ps1 -Frontend` also requires Python 3 for the three isolated Codex/WSL
metadata fixture suites; it prefers `python3.11.exe` when available, otherwise `python`.

## Generic Agent integration acceptance

`bun run test:integration-contract` checks the public schema/manifests; Rust
`cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray integrations`
checks reduction, correlation, journal recovery and configuration merging.

After building the embedded frontend/executable:

```powershell
bun scripts/integration-bridge-smoke.ts --exe src-tauri/target/ci-release/agent-tray.exe --require-gui
bun scripts/native-integration-smoke.ts --exe src-tauri/target/ci-release/agent-tray.exe --notices
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray integration_concurrent_stress -- --ignored --nocapture
```

The native script owns an isolated TASKPULSE_HOME, host-sandbox and WebView2
profile; CDP 9337 must be unused. It refuses to attach to another app. Optional
`--stress-worker <absolute Rust test executable>` adds 20-process/10000-event
synthetic load and real IPC/animation-frame latency measurement. Test binaries
need their build-owned WebView2Loader.dll on PATH in GNU environments.
`--notices` reuses native-notice acceptance in that owned sandbox; it explicitly
does not verify focus preservation against an external editor.

These exercise real TaskPulse binaries, **not actual Agent hosts**. No real host
configuration is modified. For actual host tests, first show the exact diff and
obtain confirmation. See [integration validation](../docs/AGENT_INTEGRATION_VALIDATION.md).

When a tray is running, `verify.ps1 -TargetDir src-tauri/target/<isolated-target>`
can select an owned verification target; the script refuses a target containing
a running executable. Never overwrite the live binary or kill processes by name.

Windows Alpha releases: `bun run release:check`, `bun run test:release` and
[docs/RELEASING.md](../docs/RELEASING.md). `release.sh` is read-only; no implicit
commit/tag/push. `installer-smoke.ps1` installs real packages only on disposable
GitHub Actions Windows runners and must not run against a developer's profile.
`native-inbox-smoke.ts` is read-only WebView2 inbox/section acceptance at temporary
CDP 9337; restart the app without CDP after use. It does not reopen real requests.

These scripts are how hook setup, custom CLI support, and local builds work today.

## Common Tasks

### Install Hooks for Supported CLIs

Unix:

```bash
./scripts/hooks/install-hooks.sh all
./scripts/hooks/install-hooks.sh claude
./scripts/hooks/install-hooks.sh codex
./scripts/hooks/install-hooks.sh gemini
./scripts/hooks/install-hooks.sh all --uninstall
```

Windows:

```powershell
.\scripts\hooks\install-hooks.ps1 -Target all
.\scripts\hooks\install-hooks.ps1 -Target claude
.\scripts\hooks\install-hooks.ps1 -Target codex
.\scripts\hooks\install-hooks.ps1 -Target gemini
.\scripts\hooks\install-hooks.ps1 -Target all -Uninstall
```

Notes:

- Unix hook installation requires `jq`
- Installers merge AgentTray entries into CLI settings instead of overwriting them
- AgentTray entries are tagged as `"agent-tray"` so uninstall removes only its own hooks

Settings files modified:

| CLI | File |
| --- | --- |
| Claude Code | `~/.claude/settings.json` |
| Codex CLI | `~/.codex/hooks.json` |
| Gemini CLI | `~/.gemini/settings.json` |

### Wrap Unsupported CLIs

Unix:

```bash
./scripts/wrap.sh my-agent -- claude chat
```

Windows:

```powershell
.\scripts\wrap.ps1 my-agent -- claude chat
```

This creates `~/.agent-monitor/my-agent.status` with atomic JSON updates that AgentTray can watch.

Example payload:

```json
{
  "v": 1,
  "status": "working",
  "message": "Processing input...",
  "terminal": {
    "kind": "x11_generic",
    "focus_id": "12345678",
    "outer_id": "",
    "label": "Terminal"
  }
}
```

### Build Locally

```bash
./scripts/build.sh --dev
./scripts/build.sh --release
./scripts/build.sh --release --bundle deb,appimage
```

PowerShell:

```powershell
.\scripts\build.ps1 -Dev
.\scripts\build.ps1 -Release
```

## Script Reference

### `scripts/hooks/agent-tray-hook.sh` and `scripts/hooks/agent-tray-hook.ps1`

Universal hook bridges for supported CLIs. They read hook event JSON, map it to AgentTray states, and write status files into `~/.agent-monitor/`.

### `scripts/registry.sh`

Terminal detector registry. It sources scripts from `scripts/detectors/` and returns the first matching terminal descriptor as JSON.

### `scripts/detectors/`

Add a terminal detector as `NN_name.sh`. Each detector should print JSON with `kind`, `focus_id`, and `label` when it matches, or print nothing otherwise.

Adding a new terminal type usually means:

1. Add a detector in `scripts/detectors/`
2. Add a Rust focuser in `src-tauri/src/focusers/`
3. Register the new focuser in `src-tauri/src/focusers/mod.rs`

## Maintainer Docs

- Release process: [../docs/maintainers/releasing.md](../docs/RELEASING.md)
