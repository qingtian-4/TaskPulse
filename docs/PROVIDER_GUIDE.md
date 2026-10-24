# Contributing an Agent adapter

TaskPulse uses built-in Rust providers and local Standard Events v1 / declarative
Manifest integrations. There is no executable plugin marketplace or general dynamic loader.
Finding an installed Agent does not prove that its titles, turns or approvals are observable.
The independent desktop app observes supported sources; optional IDE bridges are future work.

## Propose capabilities before implementation

Open an issue with the Agent/version, documented event or native format, transport
(Windows, running opted-in WSL distro, or configured SSH host), a sanitized sample,
and which fields are explicit, inferred, unavailable or unverified:

- stable source/session/thread/run identity;
- native title and rename, project name and full project path;
- explicit turn start/end and failure;
- request/call IDs for independent approvals, resolution and tool activities;
- phase events (thinking/tool/responding/model wait), timestamp and sequence;
- read-only boundaries, collection errors and incremental cursor/restart behavior.

## Current internal interface

`src-tauri/src/providers/mod.rs` defines `Provider::poll(now) -> ProviderOutput`.
Adapters normalize into `TaskSnapshot`; they do not modify attention, send alerts,
approve commands, execute collected snippets or invent completion from silence.
This internal Rust model is **not** a promised stable third-party event ABI.

Add source registration, discovery hints, configuration/scope handling, independent
health and transport, then fixtures and actual source verification. Preserve unknown
config fields and existing allowlists. The same session ID on two hosts/distros must
not collide. A rename changes presentation, not identity or notification state.

Require regression sequences for duplicates, out-of-order/partial records, missing
results, two concurrent requests, restart, source outage/recovery, scope removal,
new projects, internal sessions and long silent thinking. A failed collection is
not an empty successful snapshot; a tool result is not the end of an Agent turn.

Official hooks require explicit reversible confirmation. Never auto-install them,
overwrite proxy/statusLine/other hooks, read credential files or run an untrusted
third-party adapter with the user's privileges automatically.

## Implemented local event contract

For a host that can emit explicit lifecycle facts, first consider the
[Standard Events v1 protocol](AGENT_INTEGRATION_PROTOCOL.md) and
[integration guide](AGENT_INTEGRATION_GUIDE.md). A declarative manifest can map
bounded host events into stable session/turn/request/call identities without
adding a Rust source-kind enum for every brand. The CLI Bridge validates facts,
writes a locked journal, and GenericEventProvider consumes ordered durable
outboxes through the same Store/Attention/Transition boundary.

Local transport is implemented. Generic WSL/SSH transports and arbitrary
third-party executable loading are not implemented. Built-in mappings and fixture
tests do not certify a host version; see the [capability matrix](AGENT_COMPATIBILITY.md).
Configuration previews and explicit confirmation apply to supported host recipes;
imported manifests do not gain installation or execution privileges.
