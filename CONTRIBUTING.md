# Contributing to TaskPulse

For Agent adapters, start with the [capability and fixture checklist](docs/PROVIDER_GUIDE.md).
For Windows Alpha installers, follow the [draft-release and acceptance gates](docs/RELEASING.md).

TaskPulse is an alpha, Windows-first Tauri 2 tray companion for coding-agent tasks.
Read [AGENTS.md](AGENTS.md) for the current development contract. Preserve other
platforms where practical, but do not claim untested paths are supported.

## Local setup

Install Rust, Bun, [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
and Windows WebView2:

```powershell
bun install
bun run tauri dev
```

First launch asks which detected sources to monitor. Discovery does not modify
Agent settings. Never copy the maintainer's servers or projects into defaults.
The executable name `agent-tray` remains for compatibility.

## Verification

While the Windows tray is running:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend
bun run test:ui-model
```

The isolated target avoids linking over the live executable. Elsewhere, run Cargo
fmt/clippy/tests/build with `--manifest-path src-tauri/Cargo.toml` and
`--bin agent-tray`, followed by `bun run check` and `bun run build`.

Provider changes need real-source validation when available. Opt-in tests
`live_remote_snapshot` and `live_claude_snapshot` require SSH/local data and are not
unattended CI tests. UI changes need real Tauri/WebView2 acceptance:
`scripts/native-ui-smoke.ts` and `scripts/native-notice-smoke.ts`. Disable temporary
CDP after testing; do not mark user tasks viewed or send real Agent prompts for tests.

## Adding an Agent adapter

- Implement a Provider and a source-registry kind.
- Discover an actual installed/data-backed candidate; require confirmation to enable.
- Use stable session IDs; keep project, native/manual title and reported model separate.
- Add file watches where supported and independent polling fallback.
- Read/network failure retains stale state; disappearance never implies success.
- Prefer official events; undocumented native formats are best-effort evidence.
- Hook installation is opt-in, reversible and preserves unknown config fields.
- Test rename, append/truncation, new turns, unread/pinned state, scope exclusions,
  malformed data and notification deduplication.

Pipeline: Provider → TaskStore → AttentionTracker → TransitionEngine → UI/alerts.
The dashboard is an attention list, not a historical-session browser. Unknown
Agents may use the generic wrapper/status protocol; detecting a process alone
cannot supply reliable task titles or lifecycle boundaries.

## Changes and license

The canonical repository is `qingtian-4/TaskPulse`; target `main` for TaskPulse
pull requests, not AgentTray's upstream. Develop on a short-lived topic branch,
push that branch, describe and review the PR, pass the Windows quality workflow,
then merge. A push is not a merge. The initial TaskPulse integration uses a
merge commit to preserve upstream and fork-development history. Never rewrite
someone else's author metadata to change the contributors display. See
[Git / PR workflow](docs/GIT_WORKFLOW.md) for the maintainer's Chinese guide.

Keep changes scoped and include tests and documentation. Describe migrations and
compatibility limits. Never publish credentials or machine-specific fixtures.
Do not force-push during review. Contributions use the [MIT License](LICENSE);
retain AgentTray attribution.
