#!/usr/bin/env bash
# TaskPulse dev runner — sets the MinGW-W64 toolchain env, then launches
# `tauri dev`. Use from git-bash:  bash dev.sh   (or  ./dev.sh)
# This machine has no MSVC; the GNU target + winlibs mingw64 + rust-lld
# self-contained toolchain is required for cargo to build.
set -euo pipefail
cd "$(dirname "$0")"

export PATH="/c/tools/mingw64/bin:$HOME/.cargo/bin:$PATH"
export CC="gcc"
export CXX="g++"
export RUSTFLAGS="-C link-self-contained=yes"

echo "== TaskPulse dev =="
echo "   bash/rust:  $(bash --version | head -1)"
echo "   gcc:        $(${CC} --version | head -1 | sed 's/^/   /')"
echo "   rust:       $(cargo --version)"
echo ""
exec bunx tauri dev "$@"
