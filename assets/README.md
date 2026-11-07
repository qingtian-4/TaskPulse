# TaskPulse artwork

`brand/` holds the wordmarks and logo. App and tray icons are in `src-tauri/icons/`.

`architecture/` contains editable Chinese and English SVG diagrams of the current
runtime boundaries. See the [architecture map](../docs/ARCHITECTURE.md).

`screenshots/` contains native Windows panel and reminder images in Chinese and
English. Only authorized example tasks are shown; no private desktop, account or
Agent conversation appears. Example events use the actual collection, state and
notification pipeline. They illustrate the interface, not real-host accuracy or
latency measurements. They do not impersonate a native Agent reader.

Each README displays three reminders in one row, linking to full-resolution
PNGs. The [media manifest](screenshots/manifest.json) records the clean build
commit, executable hash, frontend tree, dimensions and image hashes.

New images must match the current interface and use authorized, nonprivate data.
Raw profiles, intermediate captures and recordings stay outside the public tree.
Original AgentTray attribution is preserved in [LICENSE](../LICENSE).
