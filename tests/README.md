# TaskPulse verification

| Location | Purpose / execution boundary |
| --- | --- |
| Root `*.test.ts`, `test-*.py`, `*.test.ps1` | Automated model, protocol, repository, process and offline transport regressions. Run existing `bun run test:*` commands or the Python commands in CI. |
| `support/` | Bounded process launch and owned smoke-workspace lifecycle helpers. |
| `fixtures/` | Public/sanitized inputs and independently authored replay expectations. These are verification data, not disposable generated output. |
| `native/` | Native/browser/host acceptance harnesses. Inspect each script's guard and side effects before running. |
| `install/` | Installer, bootstrap and controlled-upgrade acceptance plus their helpers. Installation/upgrade execution is disposable-CI-only and opt-in. |

Use exact root test paths from package.json to avoid collecting nested worktrees.
[CONTRIBUTING](../CONTRIBUTING.md) describes daily commands and evidence boundaries.

`bun tests/native/native-settings-smoke.ts --exe <unused-built-exe>` uses an owned
profile and real WebView2/IPC. Dashboard, request replay and development smokes also
have isolated `--exe` modes. Keep full SHA/clean-build checks and cleanup evidence.

Some retained native/host scripts attach to a configured CDP port or real source,
and `ui-smoke.ts` is a browser-mock harness. They are not interchangeable with
isolated native acceptance and are not a batch to execute automatically. Live
Claude/Codex/WSL/SSH tests need their documented source/consent prerequisites.
Do not install real hooks, send host decisions or change approval policies to
make a maintenance check pass.

A replay, pressure test or short pilot does not certify real-source accuracy,
actual rendered P95, host-return success, physical DPI/sleep or ordinary MSI UAC.
Retain unavailable checks as open items. Generated evidence belongs in ignored
target directories, never fixtures or public source guides.
