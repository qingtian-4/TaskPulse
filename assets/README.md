# TaskPulse artwork

`brand/` holds the wordmarks and logo. App and tray icons are in `src-tauri/icons/`.

`architecture/` contains editable Chinese and English SVG diagrams of the current
runtime boundaries. See the [architecture map](../docs/ARCHITECTURE.md).
The current diagrams use the maintainer-supplied designs, with code paths and
capability wording checked against the runtime.

`promo/` contains the maintainer-supplied eight-second promotional animation.
It uses Standard Events example images and is not an interaction recording or
real-host performance evidence. Its [manifest](promo/manifest.json) records the
original file hash, dimensions, frame count and duration. Chinese captions are
preserved in both READMEs.

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
