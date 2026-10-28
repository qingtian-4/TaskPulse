# macOS development and testing

TaskPulse shares its Rust task model and Svelte UI across platforms. macOS process discovery, focus, menu bar, window behavior and permissions remain platform-specific. Successful Windows checks do not verify them.

## Install a preview

Use the [PR preview guide](CANDIDATE_TESTING.md#当前-pr-的安装预览) to download the artifact for the exact source SHA. Apple Silicon uses arm64; Intel uses x64. Verify the architecture-specific checksum and manifest, open the DMG, copy TaskPulse to Applications, and check the running version/SHA in About.

The [command-line installer](CLI_INSTALL.md#macos-apple-silicon-and-intel) uses the same DMG for public releases. PR artifacts and Draft releases are not public installer inputs.

Current experimental builds are ad-hoc signed and not notarized. A valid local signature does not establish Developer ID. Keep Gatekeeper and quarantine enabled; use [Apple's per-app guidance](https://support.apple.com/en-us/102445) when necessary.

## Local development

Install Xcode Command Line Tools, Rust and Bun following the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#macos). The root `dev.sh` is for Windows/MinGW, not Mac.

```sh
bun install --frozen-lockfile
TASKPULSE_HOME="$PWD/src-tauri/target/mac-dev-profile" bun run tauri dev
```

Run the frontend and Rust checks from the [development guide](DEVELOPMENT.md). Then build a native package:

```sh
# Apple Silicon
bun run tauri build --target aarch64-apple-darwin --features custom-protocol --bundles app,dmg -- --locked
# Intel
bun run tauri build --target x86_64-apple-darwin --features custom-protocol --bundles app,dmg -- --locked
```

Run the matching command on that architecture. A dev-server session is not a substitute for launching a packaged app from Finder.

## CI and installation checks

`experimental-platforms.yml` provides opt-in architecture-specific builds; `installation-preview.yml` builds same-repository PR previews. Apple Silicon uses `macos-15`, Intel uses `macos-15-intel`. Both record the exact source SHA.

`scripts/macos-package.py` stages the DMG, checksum and manifest, then tests installation only on disposable CI runners. It checks bundle identity, actual executable architecture/version, signature integrity, resources, first launch, second activation and restart. The CLI smoke exercises the actual installer with isolated directories and controlled transport.

These checks do not prove Finder-download Gatekeeper behavior, UI clicks, menu-bar interaction, notification focus or real Agent events. Keep each result explicit in [manual testing](CANDIDATE_TESTING.md#实机测试).

## Manual checks

- Menu bar and Dock behavior, popup placement, dark/light mode, Retina, multiple displays and Spaces/full screen.
- Finder launch with a normal GUI PATH, including discovery of required source tools. Shell startup scripts are not inherited automatically.
- Real Agent task start, thinking/tool phases, explicit questions/approvals and matching resolution, completion/failure and rename.
- Reminder visibility without unwanted focus changes, plus unread/pin/dedup recovery after restart.
- Upgrade and uninstall behavior with existing state retained. Never delete user preferences to pass acceptance.

The configured minimum OS target is not proof that every older macOS version has been tested. Developer ID signing and notarization require separately configured Apple credentials in protected CI secrets. See [Tauri signing](https://v2.tauri.app/distribute/sign/macos/) and [release process](RELEASING.md).
