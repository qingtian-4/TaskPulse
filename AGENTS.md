# TaskPulse repository guidance

TaskPulse is a Windows-first Rust / Tauri 2 / Svelte 5 tray app, maintained with
Bun. Preserve macOS/Linux compatibility. It monitors tasks and attention, not
an Agent's full idle history.

## Start here

- Use the [TaskPulse maintenance skill](.agents/skills/taskpulse-maintenance/SKILL.md)
  for repository cleanup and structural changes.
- Read the [architecture map](docs/ARCHITECTURE.md) for data flow and ownership.
- [CONTRIBUTING.md](CONTRIBUTING.md) owns development and verification commands.
- Load detailed [runtime](.agents/skills/taskpulse-maintenance/references/runtime.md),
  [platform](.agents/skills/taskpulse-maintenance/references/platforms.md) and
  [delivery](.agents/skills/taskpulse-maintenance/references/delivery.md) rules
  when the task touches those areas.

## Invariants

- Stable source/session/turn/request IDs define identity. Renames and polling
  must not create executions, renew activity or reset notification dedup.
- Thinking, tool completion, silence, missing data and CPU do not prove approval
  or turn end. Only explicit requests and matching resolutions change a blocker.
  Limits, corruption and incomplete reads preserve trusted state.
- Unread results, pins and unresolved requests survive restart. Reminder dismissal,
  detail selection and host focus never acknowledge results or approve requests.
  Bounded delivery logs/queues do not replace durable attention.
- Preserve config, unknown fields, disabled/removed sources, project scopes,
  backups, CAS, drafts and first confirmation. SourceSettings owns close protection.
- Keep `locales/`, frontend/native consumers and key-consistency tests together.
  Translate app-owned copy only, preserving native titles, paths and model IDs.
- Keep scoped IPC, focus identity checks, resource budgets, updater signatures,
  quarantine and OS security checks. Hooks, WSL/SSH and integrations stay opt-in.
  Never install hooks or change real Agent configuration/approval policies automatically.

## Verify and work safely

Use an isolated `codex/*` worktree from current `origin/main`, preserving user
edits. Build frontend assets before Rust embeds them. Ordinary checks are:

```sh
bun run check
bun run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --bin agent-tray -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray
```

Use `scripts/verify.ps1 -Frontend` on Windows with an unused verification target.
Never overwrite a running executable or kill processes by name. Native tests use
owned temporary profiles and record cleanup. Live hosts, real request cohorts,
interactive UAC and physical display/sleep tests are separate evidence.

User scope overrides standing workflow. When permitted, completed work may be
pushed, reviewed and merged with a merge commit after current-head Windows
quality, Security and relevant platform checks pass. Verify main and ancestry
before cleanup. A local-only task stays local. Tags, Releases and real Agent
configuration changes need their own authorization. Keep transient reports,
captures, credentials and build products local and ignored.
