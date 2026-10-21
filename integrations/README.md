# Declarative integrations

- `event-v1.schema.json`: standardized producer input; Rust `Event::validate` is the runtime authority.
- `standard-events.json`: self-reported, local v1 producer; no host installer.
- `kimi-code-cli.json`, `trae-cn-ide.json`, `zcode-desktop.json`: partial documented Hook mappings, synthetic tests only; **no real host version verified**.
- `workbuddy-candidate.json`: unsupported candidate; no native mappings/installer.
- `community-template.json`: copy, rename IDs, restrict mappings to actual evidence, import through management. It cannot install commands or claim official/native-tested status.
- `examples/`: synthetic protocol examples, never real monitored tasks.

See [protocol](../docs/AGENT_INTEGRATION_PROTOCOL.md), [user guide](../docs/AGENT_INTEGRATION_GUIDE.md), [compatibility](../docs/AGENT_COMPATIBILITY.md), and [validation](../docs/AGENT_INTEGRATION_VALIDATION.md).

Preserve stable session/turn/event/request/call identity. A prompt submission or blockable Stop is not a completed execution. Additional binary formats, transports and causal protocols need reviewed code, not arbitrary manifest execution.
