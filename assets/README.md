# Public assets

This directory contains the TaskPulse brand source and architecture diagrams used by public documentation.

- [Brand source and export rules](brand/README.md): TP logo, wordmarks, tray source SVGs and the asset manifest.
- [Architecture, English](diagrams/taskpulse-architecture.svg) and [中文](diagrams/taskpulse-architecture-zh.svg): documentation diagrams, not application screenshots.

Keep production icons in `src-tauri/icons/`; they are required by the application and installers.

## Screenshots

Publish only selected, reviewed captures. Use the original PNG at native resolution, preferably a 2× capture displayed at half its pixel width. Do not enlarge a low-resolution capture, compress it through a messenger, or replace UI text in an image editor.

A useful capture shows a few readable tasks, their models or elapsed time, and one clear attention state. Remove private paths, conversations and accounts before capture by using a dedicated test profile. Label synthetic data as synthetic; it is not proof of real-agent support.

Raw captures, stress-test images, old versions and upstream demonstration media stay local and ignored. A newly approved public image requires an explicit `.gitignore` inclusion and a reference from the public documentation. Preserve upstream copyright in [LICENSE](../LICENSE); old screenshots are not required for attribution.
