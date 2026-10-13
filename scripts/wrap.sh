#!/usr/bin/env bash
# Compatibility entry: observes process boundaries, never scans Agent prompts.
set -eu
if [ "$#" -lt 2 ]; then
  echo 'Usage: wrap.sh <name> <executable> [args...]' >&2
  exit 2
fi
job_script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export TASKPULSE_STATUS_DIR="${TASKPULSE_STATUS_DIR:-$HOME/.agent-monitor}"
exec python3 "$job_script_dir/tp-run" "$@"
