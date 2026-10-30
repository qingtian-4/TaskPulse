#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

case "${1:---check}" in
  --check|--dry-run)
    if [[ $# -gt 1 ]]; then
      echo "This read-only entry no longer accepts a version override." >&2
      exit 1
    fi
    cd "$ROOT_DIR"
    bun scripts/release.ts check
    echo "No files, commits, tags, remote refs or Releases were changed."
    echo "See CONTRIBUTING.md for explicit PR, tag and draft-release steps."
    ;;
  -h|--help)
    echo "Usage: bash scripts/release.sh [--check|--dry-run]"
    echo "Read-only version validation; never commits, tags, pushes or publishes."
    ;;
  *)
    echo "Automatic version bumps/pushes were removed. Follow CONTRIBUTING.md." >&2
    exit 1
    ;;
esac
