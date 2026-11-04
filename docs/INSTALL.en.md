# Install and troubleshoot TaskPulse

[简体中文](INSTALL.md)

Release notes put installation downloads at the top. The current version keeps
all attachments. Later versions consolidate metadata into a release manifest,
`SHA256SUMS.txt` and a dependency/license ZIP. Updater signatures stay embedded
in `latest.json`, with sidecars and per-platform license material retained in
the ZIP. Existing MSI installations keep their MSI update package. Windows
ARM64 is selectable only when that release actually provides its installer.
The bootstrap selects the native system architecture.

Choose a package from [GitHub Releases](https://github.com/qingtian-4/TaskPulse/releases). On Windows 10/11 x64, use the EXE setup wizard or the MSI for managed deployment. Choose one format and uninstall it before switching. Setup can download WebView2 if the runtime is missing. On macOS 13.3 or later, pick the Apple Silicon or Intel DMG and drag TaskPulse into Applications. The [command-line installer](CLI_INSTALL.md) verifies SHA-256 and defaults to `~/Applications` on Mac.

Compare the download with its published SHA-256. If Windows reports an unknown publisher or macOS cannot verify the developer, verify the download source and follow the system prompt or contact your administrator. TaskPulse does not remove quarantine or disable system security checks. If the Mac installer reports that installation succeeded but launch failed, open the installed application from Finder. Keep any `.taskpulse-recovery/TaskPulse.app` backup until the new app starts and its state is intact.

## First launch and updates

Confirm the detected local agents and choose **Start monitoring**. Start a task in an enabled Agent and open the tray panel. On Windows, `Ctrl+Shift+A` also opens the panel. Use **Settings → Sources** to configure project scope or advanced connections.

**Settings → Application updates** checks for a new version. Download and installation require your click; TaskPulse verifies the updater signature before running the installer. A network or signature failure leaves the installed version in place. You can disable automatic checks, or install a newer package manually if your old version has no update page. Updates retain the profile under `~/.taskpulse/`.

## Displays, sleep and system prompts

If the panel is obscured after moving between displays, reopen it from the
tray or reopen Settings. Include the OS version, display arrangement and
scales when reporting a layout issue. After wake, check that the Agent, WSL
distro or SSH connection is available and refresh the panel. A stale request
can be checked in its Agent; waking the computer does not resolve it.

A download checksum verifies bytes. The updater verifies a separate signature
against its embedded key. The current Windows packages do not have a publisher
signature; the current macOS packages are ad-hoc signed and not notarized.
Keep system trust checks enabled and contact your administrator if policy
blocks an install. For an update failure, record the old and target versions,
package format and error. Verify that the new process starts before removing
a recovery copy, and retain your TaskPulse profile.

## A task is missing or its state needs an update

Clear dashboard filters and check that the source is detected, enabled, healthy and in scope. Windows, WSL and SSH are distinct connections. WSL requires a running distro with Python 3. Some agents cannot supply approvals, questions or exact session navigation; see the [capability notes](AGENT_COMPATIBILITY.md#english).

An unreadable source does not prove a task finished. Compatibility files are limited to 256 KiB each, with 4 MiB read and 4,096 directory entries inspected across both compatibility directories per collection round. Fix the producer if a record is oversized, incomplete or corrupt. Write status snapshots by atomic replacement and put detailed output in a separate log.

Claude's optional event directory reserves up to 128 MiB for 512 sessions, with an 8 MiB limit on an individual journal. An admitted session can compact to a checkpoint without dropping pending requests. A rejected write does not block Claude and may leave `claude-events/.admission-degraded` so the possible evidence gap remains visible after a restart. To free capacity, pause the observer and back up the event directory. Move the matching `.jsonl`, `.json` and `.lock` files together only for sessions that have ended, have no pending requests and will no longer write. Once capacity and collection are restored, move the marker aside; this does not recover missed events.

Quit from the tray before uninstalling. Remove any optional Agent hooks through their original configuration flow first. Keep `attention.json`, `recent.json` and notification state to preserve unread results, pins and deduplication. Avoid attaching the entire profile, task titles or project paths to a public issue.

[Build from source](../README.en.md#build-from-source) · [Installation options](CLI_INSTALL.md)
