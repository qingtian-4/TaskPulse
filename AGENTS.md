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
- Primary host: Windows 10/11 with WSL used for DeepSeek Harness discovery.

## Architecture

The runtime pipeline is:

`Provider -> normalization/dedup -> TaskStore -> TransitionEngine -> AttentionTracker -> tray/UI/notifications`

Important locations:

- `src-tauri/src/providers/`: Codex, Hermes, DeepSeek Harness, local status files, and remote tmux providers.
- `src-tauri/src/watcher.rs`: polling orchestration, provider health, merging, and Tauri update events.
- `src-tauri/src/store/`: authoritative-source reconciliation and recent-task retention.
- `src-tauri/src/transition.rs`: lifecycle transitions and one-shot notification events.
- `src-tauri/src/attention.rs`: dynamic attention list (running, unread, failed/waiting, and pinned tasks).
- `src-tauri/src/history.rs`: recent terminal-task persistence.
- `src-tauri/src/tray.rs`: aggregate tray state and embedded tray icons.
- `src/routes/+page.svelte` and `src/lib/components/`: popup UI.
- `docs/ATTENTION_LIST.md`: attention-list product rules and provider degradation behavior.
- `docs/ARCHITECTURE.md`: detailed backend architecture.

Stable session/thread IDs are task identity. A title rename updates presentation only; it must not create a new task, reset lifecycle state, or send another notification. Project and task title remain separate model fields.

## Data and configuration

- TaskPulse state: `~/.taskpulse/attention.json` and `~/.taskpulse/recent.json`.
- Monitoring/remote configuration: `~/.taskpulse/config.toml`.
- Legacy/generic hook input: `~/.agent-monitor/` and `~/.taskpulse/` status files.
- Codex: native thread/app-server metadata plus rollout logs; local and configured remote projects are supported.
- Hermes: native SQLite session metadata and runtime lease state.
- DeepSeek Harness: WSL `~/.dsh/sessions/**/session.jsonl.zstd`, overlaid by `~/.dsh/storages/session_projcache.json` for native titles and live state.
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
cargo build --manifest-path src-tauri/Cargo.toml --bin agent-tray
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

## Windows build and tray process

The tray PNGs are compile-time embedded. Do not add `icons/tray-*.png` back to Tauri `bundle.resources`; copying those redundant runtime resources caused Windows file-lock failures while the tray app was running.

While `src-tauri/target/debug/agent-tray.exe` is running, use `scripts/verify.ps1`; it builds into `src-tauri/target/codex-check`, so the linker never replaces the live executable. To rebuild the executable used by the tray itself, stop only the workspace-owned process, run the normal build, and restart it. Never kill unrelated processes by name without verifying their executable path.

`tempfile` is intentionally a normal dependency: production code uses same-directory temporary files for atomic replacement of `attention.json` and `recent.json`. Tests also use it.

## Verification and version control

- Run focused tests while iterating, then the full Rust test target.
- Run `cargo clippy ... -- -D warnings`, `bun run check`, and `bun run build` before handoff.
- For provider changes, validate against the real local data source when available; do not rely only on fixtures.
- For remote-provider changes, run the ignored live snapshot test and confirm the expected server/project/task in its output.
- Keep unrelated user changes intact. Use small logical commits on the current `codex/*` branch; do not push or open a PR unless requested.
