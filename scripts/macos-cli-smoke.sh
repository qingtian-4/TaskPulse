#!/bin/bash
# Production install.sh with test-only curl transport and an isolated profile.
set -euo pipefail
[ "$(uname -s)" = Darwin ] && [ "${GITHUB_ACTIONS:-}" = true ] || {
  echo 'Run only on a disposable macOS CI runner.' >&2; exit 1;
}
python3 "$(dirname "$0")/macos-cli-smoke.py" "$@"
