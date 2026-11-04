# TaskPulse repository tools

- `develop.ts`, `verify.ps1`, `verify-frontend-assets.ts`: native development and build verification. Package scripts and [CONTRIBUTING](../CONTRIBUTING.md) are the command entrypoints.
- `release.ts`, `release.sh`, `updater-assets.ts`, `release-package.py`, `release-notes.ts`, `macos-package.py`: version validation, package staging and draft-release preparation.
- `dependency-notices.ts`, `security.ts`, `repository-audit.ts`: dependency/license provenance, pinned audits and tracked-file/navigation checks.
- `request-evaluation.ts`, `request-delivery-evidence.ts`: score/join independently labelled request evidence. Real-source acceptance remains distinct from replay.
- `disk-report.ps1`, `cache-common.ps1`, `clean-build-cache.ps1`: bounded disk accounting and explicit owned-cache cleanup, with dry-run defaults.
- `probe-codex.ps1`: retained manual diagnostic. Review its local-metadata output before use.

## Stable runtime and installation paths

Repository-root `install.ps1` and `install.sh` are public installer entrypoints.
`tp-run`, `wrap.sh`, `wrap.ps1`, `codex-observer.py`, `install-codex-observer.py`
and `hooks/` retain their paths. The Hook directory is a production Tauri bundle
resource, not test scaffolding. None is automatically installed by repository tests.
See [installation](../docs/INSTALL.md) and [integration](../docs/AGENT_INTEGRATION_GUIDE.md).

Verification code and sanitized fixtures live under [tests](../tests/README.md).
Do not put task reports, captured profiles or build products in either directory.
