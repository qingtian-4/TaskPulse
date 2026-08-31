# TaskPulse — Architecture Audit

> Status: Phase 0 (audit) complete · 2026-08-27
> Base: AgentTray `sprklai/agenttray` @ `9542685` (main), MIT License
> TaskPulse 是在 AgentTray 之上的 Windows-first 统一任务监控工具：
> 本地 AI Agent（Codex / Hermes / DeepSeek Harness）+ 远端 tmux 任务（Server1/Server3）。

---

## 1. Upstream Inventory (what AgentTray is)

| Module | File(s) | Role |
|---|---|---|
| Shell | `src-tauri/src/main.rs` | Tauri 2 builder, tray menu (Show/Pin/Hide/Quit), tray left-click toggle, global shortcut `Ctrl+Shift+A`, spawns watcher thread, invoke handlers |
| Watcher | `src-tauri/src/watcher.rs` (885 lines) | `AgentStatus` model, status file parse (JSON v1 + legacy pipe), `~/.agent-monitor/` watch, stale TTL (10 min), orphan hook-file cleanup (session-id cross-check), hook>wrap>scan dedup, focus_id budget suppression, sort by status priority, emit `agents-updated`, merge loop (file watcher + 2 s process scan) |
| Tray | `src-tauri/src/tray.rs` | Aggregate state (needs-input>error>working>starting>idle>offline), per-state icon swap, popup window (400 px, transparent, always-on-top, skip-taskbar), pin, hide, window-state position restore |
| Notifier | `src-tauri/src/notifier.rs` | `Notifier` trait; beep + desktop notification; one-shot transition notifications deduped by task id and turn |
| Scanner | `src-tauri/src/scanner/*` | Per-OS process scan (linux/macos/windows), CLI strategies (claude_code/codex/gemini), CPU smoothing, subagent filtering, multiplexer focus via env vars (tmux/kitty/wezterm), tmux pane title read |
| Heuristics | `src-tauri/src/heuristics.rs` | Cross-validate hook status vs scanner data |
| Focus | `src-tauri/src/focusers/*`, `focus.rs` | Jump to terminal/session (VS Code, kitty, tmux, wezterm, …) |
| Hooks | `scripts/hooks/*.sh/.ps1` | Per-CLI hook installers writing status files |
| Wrap | `scripts/wrap.sh`, `wrap.ps1` | Generic wrapper: status file + needs-input regex + exit-code error status |
| Frontend | `src/**` (Svelte 5 / SvelteKit static / Tailwind 4 / TS) | Compact popup: auto-resize, `agents-updated` listener, AgentRow, AggregatePill, pin |
| License | `LICENSE` | MIT — **kept, with attribution** |

### Toolchain / platform facts (this machine, 2026-08-27)

- Windows 10, git-bash, node v22.23.2 + npm 12.0.2, git 2.53, OpenSSH 10.2p1.
- **No MSVC / no winget / no rustup / no bun initially** → toolchain installed as:
  rustup `stable-x86_64-pc-windows-gnu` + w64devkit (MinGW-w64), bun via `npm i -g bun`.
- WebView2 runtime present (151.x) → Tauri popup works.
- SSH config has real aliases: `Server1` (172.25.74.35:1919 user ai), `Server3` (172.25.74.35:514 user ai3), key auth OK, no password needed.
- Remote reality check (Server1/Server3): tmux 3.0a, x86_64 Linux, Python 3.8.10; `tmux list-panes -a -F` works; live panes observed (`%0`, `%3`, bash, cwd `~/student/wanghy/HyperMem`).

---

## 2. What We Reuse (unchanged or near-unchanged)

| Component | Why |
|---|---|
| `main.rs` tray lifecycle | Tray menu, left-click popup toggle, global shortcut, exit-on-last-window prevention — mature, platform-tested |
| `tray.rs` popup machinery | Popup create/show/hide/pin, auto-resize, transparent window, position restore |
| `notifier.rs` Notifier trait + beep/desktop backends | Battle-tested notification plumbing; extended with one-shot lifecycle events and a user test action |
| Status-file format (v1 JSON, `~/.agent-monitor/`) | Backward compat: existing AgentTray hooks/wrappers keep working |
| `scanner/*` process scanning | Local agent process detection stays (source of `scan`-class signals) |
| `heuristics.rs` cross-validation | Kept as an additional signal layer |
| `focusers/*` | Terminal focus jump kept (non-MVP-blocking per spec §28) |
| Hooks (`scripts/hooks/*`) + wrap scripts | Reused as-is for local agents (Codex hook included, §10) |
| Frontend popup shell (Svelte 5 + Tailwind, AgentRow/AggregatePill) | Compact popup UX preserved; types extended, sections added |

## 3. What We Modify

| Component | Change |
|---|---|
| `watcher.rs` | Slims down to orchestrator: file provider + scanner + remote tmux polling all feed a `TaskStore`; keeps `agents-updated` emit contract |
| `AgentStatus` → `TaskSnapshot` | New unified domain model (`domain/task.rs`): status enum (RUNNING/WAITING_INPUT/SUCCEEDED/FAILED/IDLE/DISCONNECTED/UNKNOWN), source enum (Codex/Hermes/DeepSeekHarness/RemoteTmux/LocalFile…), `DetectionConfidence` (EXACT/HOOK/INFERRED), host/project/pid/exit_code/remote fields. Frontend types updated in lockstep |
| Status semantics | `needs-input` → `WAITING_INPUT`; `working`/`starting` → `RUNNING`; `error` → `FAILED`; `idle` → `IDLE`; `offline` → `UNKNOWN` (host-level `DISCONNECTED` for remote) |
| Tray aggregate priority | FAILED > WAITING_INPUT > DISCONNECTED > RUNNING > SUCCEEDED/IDLE (spec §26) |
| Notifications | Transition-driven: RUNNING→WAITING_INPUT, RUNNING→SUCCEEDED, RUNNING→FAILED, RUNNING→IDLE(inferred), CONNECTED→DISCONNECTED (debounced), DISCONNECTED→CONNECTED |
| `tauri.conf.json` / package.json | productName TaskPulse, identifier `dev.taskpulse.app`; MIT LICENSE retained |
| Popup UI | Sections: LOCAL AGENTS / REMOTE TMUX (per host) / RECENT completed (30 min TTL or acknowledged) |

## 4. What We Replace

| Old approach | New approach |
|---|---|
| Single `watcher.rs` god-module | `Provider → Normalize → Store → Transition Engine → Tray/Notification/UI` pipeline (`domain/`, `providers/`, `store/`, `transition/`) |
| Success ⇒ delete status file immediately (wrap) | Completed tasks move to **RECENT history** for 30 min (spec §25) — `TaskStore` keeps finished snapshots with TTL |
| `pane_dead`-style completion signals | **Process-tree detection**: pane metadata + `ps` descendants; shell→workload transitions (spec §16, §17) |
| Per-pane SSH connections | **One SSH snapshot per host** per poll, parallel across hosts (spec §14) |
| SSH timeout ⇒ treat tasks finished | Failure debounce: first failure preserves state (`stale`), ≥2-3 consecutive failures ⇒ host `DISCONNECTED` (spec §22) |

## 5. What We Explicitly Do NOT Copy / Do Not Do

- **No OCR / screenshots / CV / LLM screenshot classification** (spec §29) — banned in MVP.
- **No cloud backend / SaaS / auth server / hosted DB / Kafka / Redis / external WS** (spec §30) — local only: atomic files + fs watcher + SSH polling + Tauri events.
- **No pane content capture** (`capture-pane`) by default — terminal may contain tokens/secrets (spec §23). Any future output tail is opt-in with redaction.
- **No credential storage**: TaskPulse shells out to system `ssh.exe`, uses user `~/.ssh/config`, never saves passwords/keys (spec §13).
- **Do not treat hook/scan inference as exact**: without a command exit code, never emit SUCCEEDED/FAILED (spec §7, §18).
- **Do not copy** AgentMonitor's SQLite-session adapter wholesale — only borrow the idea of `~/.hermes/state.db` as a fallback (spec §11); its code is MIT but unnecessary to vendor.
- **Do not copy** tmux-monitor's pipe-pane daemon / Ollama summarization — out of scope and against the no-content-capture rule.
- **Do not vendor** any reference repo into the tree (spec §2) — reference repos go to `references/` (git submodule/shallow clone) or temp dirs.

## 6. Licensing

- AgentTray: **MIT** — keep `LICENSE`, keep upstream attribution (README + docs/AUDIT).
- AgentMonitor: MIT — referenced only.
- tmux-monitor: MIT — referenced only.
- tmux-agent: MIT — referenced only.
- tmux-notify: MIT — referenced only.
- TaskPulse new code: MIT, compatible with all dependencies (Tauri 2 = MIT/Apache-2.0 dual, Svelte = MIT, notify = CC0/MIT).
- `tp-run` (new, Linux shell): MIT, same header.

## 7. Reference Repos (clone audit)

| Repo | Audit focus | Verdict |
|---|---|---|
| `sprklai/agenttray` | base; hook/wrap/scan reliability, tray shell | **Fork base** |
| `jiweiyeah/AgentMonitor` | hermes.rs SQLite adapter, codex rollout parsing, fs_watch + proc_sampler collector split | Ideas only (hermes fallback, Phase 7) |
| `dbmcco/tmux-monitor` | discovery.py (`list-panes -a -F` fields), state.py status.json shape | Field list reused (spec §15); daemon/pipe-pane not copied |
| `trentdavies/tmux-agent` | process-tree walking (sysinfo) for agent detection, priority process>content>title | Priority ordering reused; no sysinfo dep needed (we parse `ps` output over SSH) |
| `rickstaa/tmux-notify` | notify.sh prompt-suffix trick | Explicitly **not** used: prompt-suffix heuristics are exactly the false-completion trap spec §16/§18 warns about |

## 8. Phase 0 Verification (this machine)

- [x] AgentTray cloned, working tree clean at `9542685`, branch `main`
- [x] SSH Server1/Server3 reachable (key auth), tmux 3.0a present
- [x] WebView2 present; rustup (GNU) + w64devkit + bun installing
- [ ] `cargo check/test` on pristine AgentTray (Windows/GNU)
- [ ] `bun install && bun run check && bun run build`
- [ ] `cargo tauri dev` — tray appears, popup opens, notification works (requires interactive session; recorded)

See `docs/ARCHITECTURE.md` for the target pipeline and `docs/DEVELOPMENT.md` for build/run.
