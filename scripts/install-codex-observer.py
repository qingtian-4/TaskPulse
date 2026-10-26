#!/usr/bin/env python3
"""Explicit, reversible installation; never edits Codex hook trust or policies."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

EVENTS = ["PreToolUse", "PermissionRequest", "PostToolUse", "Stop", "Interrupt", "SessionEnd"]


def replace(path, body):
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        original = path.read_bytes()
        digest = hashlib.sha256(original).hexdigest()[:16]
        backup = path.with_name(path.name + ".pre-taskpulse-" + digest + ".bak")
        if not backup.exists():
            shutil.copy2(path, backup)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as handle:
            temporary = Path(handle.name)
            handle.write(body)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)


def install(root, destination, remove=False):
    path = root / "hooks.json"
    value = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
    hooks = value.setdefault("hooks", {})
    # Exact owned command only. Other entries and unknown fields are untouched.
    command = f'python3 "{destination.as_posix()}"'
    if not remove:
        replace(destination, Path(__file__).with_name("codex-observer.py").read_bytes())
    for event in EVENTS:
        groups = hooks.get(event, [])
        kept = []
        for group in groups:
            handlers = [handler for handler in group.get("hooks", []) if handler.get("command") != command]
            if handlers or not group.get("hooks"):
                kept.append({**group, "hooks": handlers})
        if not remove:
            kept.append({"hooks": [{"type": "command", "command": command, "timeout": 3}]})
        if kept:
            hooks[event] = kept
        else:
            hooks.pop(event, None)
    replace(path, (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode())
    print(json.dumps({"installed": not remove, "events": EVENTS, "config": str(path),
                      "script": str(destination), "trust_required": not remove}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-home", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home()/".codex"))))
    parser.add_argument("--destination", type=Path, default=Path.home()/".taskpulse/codex-observer.py")
    parser.add_argument("--remove", action="store_true")
    args = parser.parse_args()
    try:
        install(args.codex_home, args.destination.resolve(), args.remove)
    except (OSError, ValueError, TypeError) as error:
        print(f"Observer installation failed: {type(error).__name__}", file=sys.stderr)
        sys.exit(1)
