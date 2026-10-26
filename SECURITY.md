# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| Current `main` (alpha) | Best effort; no supported stable release yet |

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

TaskPulse is maintained by [qingtian-4](https://github.com/qingtian-4), independently
of AgentTray. Do not send TaskPulse reports to AgentTray's original maintainers.

Use this repository's **Security → Report a vulnerability** private channel if
available. If private reporting has not been enabled, ask the maintainer to
provide a private channel; a public issue may request contact, but must contain
**no exploit details, credentials, personal data or sensitive logs**. No private
email address is published here without the maintainer's explicit choice.

Include the following only in the agreed private report:

1. A description of the vulnerability
2. Affected version or commit
3. Steps to reproduce
4. Potential impact assessment
5. Any suggested fixes (optional)

## Response Timeline

This alpha project has no guaranteed response or fix SLA. The maintainer will
assess reports on a best-effort basis and coordinate disclosure after a fix.
Enable and verify a private reporting channel before presenting a stable release
as security-supported.

## What Qualifies as a Security Issue

- Arbitrary code execution via crafted status files
- Hook script injection or command injection
- Privilege escalation through the tray app
- Credential or token exposure in status files
- Vulnerable dependencies still shipped by TaskPulse, including when an upstream fix is already available
- Path traversal in file watcher or focus handlers
- Unauthorized process control via focus commands

## What Does NOT Qualify

- Bugs that do not have a security impact
- Feature requests
- Issues requiring physical access to the machine
- Reports about versions that TaskPulse does not distribute or use
