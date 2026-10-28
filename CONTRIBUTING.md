# Contributing to TaskPulse

For Agent adapters, start with the [capability and fixture checklist](docs/PROVIDER_GUIDE.md).
For Windows Alpha installers, follow the [draft-release and acceptance gates](docs/RELEASING.md).

TaskPulse is an alpha, Windows-first Tauri 2 tray companion for coding-agent tasks.
Read [AGENTS.md](AGENTS.md) for the current development contract. Preserve other
platforms where practical, but do not claim untested paths are supported.

## Local setup

Install Rust 1.89+, Bun, Python 3 for the offline metadata fixture suites,
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) and Windows WebView2:

```powershell
bun install --frozen-lockfile
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
bun run audit:frontend
```

The isolated target avoids linking over the live executable. Elsewhere, run Cargo
fmt/clippy/tests/build with `--manifest-path src-tauri/Cargo.toml` and
`--bin agent-tray`, followed by `bun run check` and `bun run build`.

Provider changes need real-source validation when available. Opt-in tests
`live_remote_snapshot` and `live_claude_snapshot` require SSH/local data and are not
unattended CI tests. UI changes need real Tauri/WebView2 acceptance:
`scripts/native-ui-smoke.ts` and `scripts/native-notice-smoke.ts`. Disable temporary
CDP after testing; do not mark user tasks viewed or send real Agent prompts for tests.

Run `bun audit` to review all frontend advisories. `audit:frontend` fails at
moderate severity and above; passing it does not mean zero advisories. Record
lower-severity findings and Rust advisory checks in the
[publication checklist](docs/OPEN_SOURCE_CHECKLIST.md). Do not force a major
framework upgrade or hide an advisory to make a release pass.

`bun run audit:rust` and `bun run audit:secrets` use checksum-pinned official
Windows/Linux x64 tools under ignored target directories, with no global install.
They require network access; a timeout is not a clean scan. Git scanning requires
full history and never publishes raw reports. Run `bun run test:security` for the
report/workflow regressions. See [release audit requirements](docs/RELEASING.md#发布前).

## Adding an Agent adapter

- Prefer [Standard Events v1 / declarative mappings](docs/AGENT_INTEGRATION_GUIDE.md)
  when a host exposes explicit events. For a native collector, implement a Provider
  and the appropriate source-registry kind.
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
Agents may use the local Standard Events v1 contract or legacy wrapper/status input; detecting a process alone
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
