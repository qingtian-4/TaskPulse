# TaskPulse artwork

`brand/` contains the TaskPulse wordmarks and logo. Application and tray icons are in `src-tauri/icons/`.

`screenshots/` contains Chinese and English dashboard/notification screenshots
and short interaction demos captured from the native Windows v1.1.0 interface,
at the maintainer's request on 2026-11-04. Only isolated public sample tasks are
shown. Input requests, approvals and turn-end states come through the existing
Standard Events pipeline with an explicitly named demo source.

The three notification images occupy one responsive image row in each README. Images link
to their full-resolution originals. The demos are expandable to keep the page
compact.

The PNGs capture only the app's WebView, at twice the logical resolution. GIFs
encode a scripted sequence of native UI interactions at 5 FPS. Their timing is
for illustration and is not request-latency or real-Agent acceptance evidence.
Opening details and dismissing notifications preserve unread and request state;
the sample producer sends matching events to resolve its own requests.

See [capture provenance](screenshots/manifest.json) for the clean build commit,
executable hash, frontend tree, image dimensions and asset hashes. These figures
do not contain a private desktop or real Agent sessions. New captures must use
explicitly authorized data and accurately identify their version and platform.

Original AgentTray attribution is preserved in [LICENSE](../LICENSE).
