# Contributing to TaskPulse

Report bugs, improve the documentation or add an Agent integration through [GitHub issues](https://github.com/qingtian-4/TaskPulse/issues) and pull requests to `main`.

## Run locally

Install Bun, Rust 1.90+, Python 3 and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Read [AGENTS.md](AGENTS.md) for repository conventions. The executable name `agent-tray` remains for compatibility.

## Verify changes

```sh
bun run check
bun run build
bun run test:ui-model
bun run test:release
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --bin agent-tray -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray
```

On Windows, `scripts/verify.ps1` uses an isolated target directory so it does not overwrite a running tray executable. UI changes also need native Tauri/WebView2 checks; provider changes need real-source validation where available.

Run `bun audit`, `bun run audit:rust` and `bun run audit:secrets` before proposing a release. Do not suppress advisories or treat incomplete checks as passes. Release workflows build exact version tags and create drafts; publishing is a maintainer action.

## Add an Agent

Start with the [integration guide](docs/AGENT_INTEGRATION_GUIDE.md) and [event protocol](docs/AGENT_INTEGRATION_PROTOCOL.md). Prefer official lifecycle events; native collectors must remain read-only.

Use stable session IDs and keep task titles, projects and reported models separate. Never infer a turn end from a tool result, silence or CPU usage. Keep Hook installation explicit and reversible. Test renames, new turns, missing data, project exclusions and duplicate notifications.

## Submit a change

Keep pull requests focused, explain the user-visible behavior and include relevant verification. Preserve unrelated work and upstream attribution. Never commit credentials, private task data, build products or local investigation notes.

Contributions use the [MIT License](LICENSE), including the original AgentTray attribution.

## Releases

Use SemVer: bug fixes increment the patch version, compatible features the minor version, and incompatible changes the major version. Keep package.json, Cargo.toml, Cargo.lock and tauri.conf.json synchronized, including the derived Windows MSI version.

Run `bun run release:check` and `bun run test:release`, pass the current PR quality/security checks and merge to `main`. An explicit version tag such as `v1.0.0` builds Windows and both macOS architectures, verifies signed updates and installers, and creates a GitHub release draft. The maintainer reviews the exact package evidence before publishing. Release notes belong in `docs/releases/<version>.md`; local reports and downloaded packages stay ignored.
