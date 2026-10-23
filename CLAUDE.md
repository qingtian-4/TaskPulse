# Claude Code guidance for TaskPulse

Read [AGENTS.md](AGENTS.md) first: it is the canonical development contract.

TaskPulse is a **Windows-first attention dashboard**, forked from AgentTray.
The executable remains `agent-tray` for script compatibility; desktop identity is
`app.taskpulse.desktop`. This is not upstream's Linux-oriented process-list app.

Pipeline: Provider → scope/normalization/dedup → TaskStore → AttentionTracker →
TransitionEngine → tray/UI/reminders. State lives in `~/.taskpulse/`.
Legacy `~/.agent-monitor/` status files are compatible inputs, not the main architecture.

- Use Bun, Svelte 5, Tauri 2 and Rust.
- Keep stable session IDs and separate task titles, projects and reported models.
- Do not import idle history or invent titles, completion percentages or exit codes.
- An Agent turn ending means `WAITING_INPUT / turn_ended`, not command success.
- Discovery is read-only. A new Agent type needs user confirmation to enable.
- Never read Claude `*.key` files or print proxy credentials. Hook installation
  needs separate confirmation and must preserve existing hooks/settings.
- While the Windows tray runs, use `powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend`.
  The isolated target avoids replacing the live executable. Tray PNGs are embedded.
- Verify providers against available real data, not just fixtures.
- Preserve unrelated edits; use small commits on the current `codex/*` branch.

See [architecture](docs/ARCHITECTURE.md), [product rules](docs/PRODUCT_DASHBOARD.md),
[attention rules](docs/ATTENTION_LIST.md), and [review response](docs/internal/reports/REVIEW_RESPONSE.md).
