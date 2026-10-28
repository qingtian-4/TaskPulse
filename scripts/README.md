# TaskPulse scripts

## Install and connect

- Repository-root `install.ps1` and `install.sh`: install released Windows or macOS packages. See [installation](../docs/INSTALL.md).
- `tp-run`: report the lifecycle of a foreground command. See [remote tasks](../docs/REMOTE_TMUX.md).
- `hooks/`: optional Agent event bridges. Configure these through the application or the [integration guide](../docs/AGENT_INTEGRATION_GUIDE.md).

## Repository checks

`package.json` lists the supported check, build, test and audit commands. `verify.ps1` uses a separate Windows Cargo target directory to avoid overwriting the running tray executable. See [contributing](../CONTRIBUTING.md).

Native smoke scripts use isolated profiles and preserve evidence under ignored output directories. They do not replace your own TaskPulse state. Release scripts validate exact versions and stage artifacts; the GitHub workflow creates a draft and never publishes automatically.
