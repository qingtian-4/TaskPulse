# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| Latest [published stable release](https://github.com/qingtian-4/TaskPulse/releases/latest) | Best effort |
| Development builds, prereleases and older versions | Reproduce on the latest stable release if possible |

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

TaskPulse is maintained by [qingtian-4](https://github.com/qingtian-4), independently
of AgentTray. Do not send TaskPulse reports to AgentTray's original maintainers.

Use [Report a vulnerability](https://github.com/qingtian-4/TaskPulse/security/advisories/new)
to send a private report to the TaskPulse maintainers. If you cannot access the
form, a public issue may request a private contact channel. Keep exploit details,
credentials, personal data and sensitive logs out of public issues.

Include the following in the private report:

1. A description of the vulnerability
2. Affected version or commit
3. Steps to reproduce
4. Potential impact assessment
5. Any suggested fixes (optional)

## Response Timeline

There is no guaranteed response or fix SLA. The maintainer will
assess reports on a best-effort basis and coordinate disclosure after a fix.

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
