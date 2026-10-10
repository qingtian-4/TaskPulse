# TaskPulse

Know which coding agents are running, need your input, or have results you have
not viewed—without repeatedly switching conversations. A compact tray dashboard
shows real session titles and observable activity, and a dark, non-activating
reminder appears when your attention is needed.

TaskPulse is an **alpha Windows-first desktop companion**, forked from
[AgentTray](https://github.com/sprklai/agenttray) (MIT). It is not a universal IDE
extension and does not infer fabricated percentage progress.

- Local Codex, Hermes, DeepSeek Harness Desktop/WSL and Claude Code adapters.
- Configured SSH Codex and tmux projects/panes, with stale/disconnected handling.
- Explicit source selection; actual detected candidates, not silent installation.
- Dynamic attention: actionable → running → unread results → pinned idle.
- Stable session IDs, native/manual titles, independent project and reported model.
- Per-turn notifications, restart-safe dedup, and separate archived attention.

## Run from source

Install Rust, Bun, Tauri prerequisites and WebView2 on Windows:

```powershell
bun install
bun run tauri dev
```

First launch opens source confirmation. Later use the tray or Ctrl+Shift+A.
Source discovery is read-only. Claude hook installation requires an additional
explicit confirmation and preserves existing settings; unknown native formats
are best-effort, not an official integration guarantee. Remote monitoring uses
configured SSH, never an embedded maintainer host list.

An Agent turn ending means waiting for the next input, not command success.
Only real exit codes establish command success/failure. Losing access to a log
or server never means the task completed. Dismissing a reminder does not mark
the result viewed. Opening VS Code focuses its project window, not necessarily
the exact conversation tab.

See [Chinese README](README.md), [contributing](CONTRIBUTING.md),
[product rules](docs/PRODUCT_DASHBOARD.md), [architecture](docs/ARCHITECTURE.md),
and [critical review response](docs/REVIEW_RESPONSE.md).

MIT; retain the original AgentTray copyright and license.
