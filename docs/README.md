# TaskPulse documentation

## Install and use

- [Installation and upgrades](INSTALL.md)
- [Command-line options](CLI_INSTALL.md)
- [Preview packages and testing](CANDIDATE_TESTING.md)
- [Agent compatibility](AGENT_COMPATIBILITY.md)
- [WSL Codex](WSL_CODEX.md), [local agents](LOCAL_AGENTS.md), [remote tmux](REMOTE_TMUX.md)
- [Task dashboard](PRODUCT_DASHBOARD.md) and [attention behavior](ATTENTION_LIST.md)

## Develop and integrate

- [Development](DEVELOPMENT.md), [macOS](MACOS_DEVELOPMENT.md), [build storage](BUILD_STORAGE.md)
- [Architecture](ARCHITECTURE.md) and [design system](DESIGN_SYSTEM.md)
- [Agent integration](AGENT_INTEGRATION_GUIDE.md), [event protocol](AGENT_INTEGRATION_PROTOCOL.md), [provider development](PROVIDER_GUIDE.md)
- [Codex approval observer](CODEX_APPROVAL_OBSERVER.md) and [experimental Doubao adapter](DOUBAO.md)
- [Git workflow](GIT_WORKFLOW.md), [release process](RELEASING.md), [publication checklist](OPEN_SOURCE_CHECKLIST.md)
- [Roadmap](ROADMAP.md)

## What belongs here

Public documentation describes how to use, extend, build or release TaskPulse. Version-specific release notes belong in `releases/`.

Local plans, dated validation reports, agent handoffs, audits, machine inventories and troubleshooting transcripts are not public documentation. Keep them in ignored local folders. `.gitignore` lists the public documents explicitly; adding a new public guide requires including it there and linking it from this index.

Keep generated binaries, caches and test evidence under ignored build/output directories. Removing a file from current Git tracking does not erase it from earlier commits.
