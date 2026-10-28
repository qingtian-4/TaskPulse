# Development

TaskPulse uses Rust, Tauri 2, Svelte 5 and Bun. Follow the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. Windows is the primary platform; macOS/Linux have separate experimental checks.

## Build and run

```sh
bun install --frozen-lockfile
bun run tauri dev
```

For packaged builds, run `bun run tauri build`. Release CI uses Windows x64 MSVC; native Mac builds use the corresponding arm64/x64 target. See [macOS development](MACOS_DEVELOPMENT.md).

A Windows GNU/MinGW environment is also supported. Put its `gcc`, `as`, `windres` and `dlltool` on PATH, set `CC=gcc`, `CXX=g++`, and use `RUSTFLAGS=-C link-self-contained=yes`. The provided `dev.sh` targets that environment; adapt the tool directory to your machine. Do not carry GNU flags into an MSVC build.

## Checks

```sh
bun run check
bun run test:ui-model
bun run test:docs
bun run test:repo
bun run test:release
bun run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --bin agent-tray -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray
```

Keep frontend and Rust failures visible rather than allowing a later command to hide the exit code. Run the focused regression first, then the required suite.

When the Windows tray is running, `powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend` builds into an isolated target. Never replace a running executable or stop processes by name. See [build storage](BUILD_STORAGE.md).

## Native and source verification

Browser mocks can test presentation, but native IPC, CSP, focus and windows require the actual app. Scripts such as `native-model-smoke.ts`, `native-product-smoke.ts` and `native-notice-smoke.ts` use isolated profiles and labelled synthetic data. Their evidence stays in ignored build directories, not in public screenshots by default.

Real-provider tests are opt-in. Run them only with the required source available, and never install hooks, change approval settings or submit Agent prompts as a side effect of testing. WSL tests use an already-running opted-in distro; see [WSL Codex](WSL_CODEX.md).

With configured SSH hosts expected to be reachable:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray live_remote_snapshot -- --ignored --nocapture
```

Review the expected host/project/task in its output. Fake SSH fixtures cover ordinary offline unit tests; they do not certify a real host.

## Data and security

Use a separate `TASKPULSE_HOME` for experiments. Real state normally lives in `~/.taskpulse/`; do not delete it to make a test pass. Preserve source allowlists and unread/pin/notification state.

Run the full frontend advisory report with `bun audit`; `bun run audit:frontend` gates moderate and above. Candidate work also needs `bun run audit:rust` and `bun run audit:secrets`. Review low/maintenance/unsound warnings separately. Audit tools are pinned and workspace-local.

## Contributions

Use a feature branch or worktree, small logical commits, and a PR with the current-head checks. Follow [Git workflow](GIT_WORKFLOW.md), [public file policy](README.md#what-belongs-here) and [release gates](RELEASING.md). Keep local work reports and captures out of Git. Preserve the upstream license and attribution.
