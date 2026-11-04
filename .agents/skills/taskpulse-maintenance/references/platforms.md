# Native platforms and isolated development

Read only when the change touches this area. User scope takes precedence.

## Windows build and tray process

The tray PNGs are compile-time embedded. Do not add `icons/tray-*.png` back to Tauri `bundle.resources`; copying those redundant runtime resources caused Windows file-lock failures while the tray app was running.

Windows WebView2 creation must use async commands or a separate thread, never synchronous IPC/event handlers. Keep the narrowly scoped `ipc:` / `http://ipc.localhost` CSP connect permissions; actual native IPC/window tests catch failures that browser mocks cannot.

Popup visibility belongs to the backend `popup_interaction` coordinator. Frontend focus handlers refresh content only. A Windows tray press decides once; release, native double-click and late WebView readiness cannot invert a newer intent. Update controller state and release its lock before native setters. Native menu observation is restricted to the app-owned tray window; development acceptance must not take the installed app's global shortcut.

Reminder delivery defaults to the independent TaskPulse popup (`dashboard.notification_mode=popup`, alternatives `system`/`both`). The master `notifications` switch covers all channels. Only lifecycle events trigger task alerts; tests are explicitly labelled and never imported as monitored tasks. Run `scripts/native-notice-smoke.ts` for actual popup visual/click/focus acceptance, not just system notification API success.

`tempfile` is intentionally a normal dependency: production code uses same-directory temporary files for atomic replacement of `attention.json` and `recent.json`. Tests also use it.

## Settings presentation

- Every custom IPC command needs a checked-in application permission and explicit local window capability. Settings, updates, integration writes, and startup changes belong to management. Reminder actions belong to task-notice, which fetches current state from native IPC before trusting a notification event. Keep confirmation, backups, CAS, and isolated-profile guards inside their existing handlers. Legacy status-file readers do not require the old hook installer IPC.
- Windows window focus uses an observed process lifetime and validates the live window owner through Win32. Keep PID-only legacy status readable, with task details as the fallback. Never turn focus metadata into shell source.

- The management window uses one close-request owner in SourceSettings. Tauri's onCloseRequested destroys the window independently per listener unless prevented; additional listeners must not bypass source drafts or pending preference saves.
- Save preferences through their compare-and-swap endpoint. Preserve unsaved source arrays when preference-only settings broadcasts refresh the editor baseline; real source changes require conflict resolution.
- Translate only application-owned copy. Never translate or rewrite native task titles, project paths, source names chosen by the user or reported model IDs. Both locale catalogs must retain matching keys.
- Native settings acceptance: run `bun scripts/native-settings-smoke.ts --exe <unused-built-exe>` on Windows. It uses an isolated profile and never registers login startup or installs Agent hooks.
- Windows login commands quote the executable and create new registrations in HKCU. Repair only exact owned legacy commands, preserving their scope and StartupApproved state; never silently enable an OS-disabled entry. Registry tests use a private uncommitted transaction and native launch tests use suspended, handle-owned processes.
- Login-startup settings verify the OS result after explicit writes and retain failures or external reversions across settings sections. Focus reads cannot overwrite pending writes. Never retry enabling automatically or bypass startup protection; native UI fixtures simulate only the IPC responses inside an isolated profile.
- Management entry points share the six-section allowlist for cold URLs and warm navigation. Refit on display/work-area/scale changes; ordinary same-monitor dragging must not snap back. Update cached display state and release locks before native setters emit follow-up events.
- The source catalog declares multi-instance support. SSH/WSL/event connections retain an add entry after the first instance; source IDs remain stable and the backend still rejects duplicate kind/location pairs.
- A partial Codex collection may verify independent sessions without declaring the whole source healthy. Runtime task proofs bind the source, exact lifecycle/turn/request state and latest poll time; replace them each attempt and expire them after 30 seconds. Dashboard, tray and pending prompts must agree; cached missing or rewound sessions never gain fresh proof.
- WSL child processes must not inherit a Windows-path CODEX_HOME through WSLENV. Remove only that child override and its WSLENV entry, preserving explicit Linux overrides and all other environment entries; never load a login shell or modify user environment settings for discovery.

## Development and test artifacts

- Use `bun run dev:desktop` for daily native work; Svelte hot reload and Tauri Rust rebuilds do not require installing a package. Browser-only `bun run dev` cannot validate native IPC.
- Debug state defaults to `~/.taskpulse-dev/<build-directory-id>`; installed release state remains `~/.taskpulse`. Windows/Linux browser storage follows the profile; isolated macOS webviews use nonpersistent WebKit stores, including on macOS 13.3. Explicit smoke profiles remain isolated. Debug must not register startup, update the installed app, take its global shortcut or export an MCP connection into production state. Preserve the separate discovery guard for explicit test profiles so normal development can confirm real read-only sources.
- Windows quality produces exact-SHA portable development artifacts with 7-day retention and dependency notices. Full PR installers and upgrade acceptance are opt-in via `test:installers` / `test:upgrade`; formal stable-tag releases still require all platform gates and create drafts only. Do not use GitHub Releases for per-change testing. See CONTRIBUTING.md for the daily commands and release procedure.
- Windows integration checkpoint replacement and backup must give tempfile canonical parent paths for both temporary and destination files; raw Win32 persistence otherwise fails in deep worktree profiles even when event append succeeds.
- Native startup failures retain bounded CDP and actual owned-process token/browser evidence before cleanup. Requested integrity is not proof of TokenElevation; never export full command lines, configuration bytes or task content into this evidence.
- Windows quality runs default-profile native verification as a disposable standard account, asserting the actual app has the same user, medium integrity and TokenElevation=0. Passwords stay off argv/env/artifacts; stop that account's processes before restoring desktop/file ACLs and deleting the account. No installer/UAC policy or production driver override is used.
