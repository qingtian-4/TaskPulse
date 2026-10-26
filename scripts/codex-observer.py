#!/usr/bin/env python3
"""Opt-in Codex hook observer. No stdout, decisions, prompts or tool contents.

Install only after explicit consent and review/trust the hook in Codex /hooks.
Python 3 standard library only; writes under CODEX_HOME/taskpulse-events.
"""
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import time

LIMIT = 256 * 1024
EVENTS = {"PreToolUse", "PermissionRequest", "PostToolUse", "Stop", "Interrupt", "SessionEnd"}


@contextlib.contextmanager
def locked(path):
    with path.open("a+b") as handle:
        if os.name == "nt":
            import msvcrt
            if handle.tell() == 0:
                handle.write(b"0")
                handle.flush()
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_LOCK, 1)
        else:
            import fcntl
            fcntl.flock(handle, fcntl.LOCK_EX)
        try:
            yield
        finally:
            if os.name == "nt":
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(handle, fcntl.LOCK_UN)


def signature(event):
    value = event.get("tool_input", {})
    # Codex adds a description to approval input; it is not call identity.
    if isinstance(value, dict) and event.get("tool_name") in {"Bash", "apply_patch"}:
        value = {k: v for k, v in value.items() if k != "description"}
    body = json.dumps([event.get("tool_name"), value], sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(body.encode()).hexdigest()


def record(root, event, now=None):
    session = event.get("session_id", "")
    turn = event.get("turn_id", "")
    kind = event.get("hook_event_name")
    if kind not in EVENTS or not re.fullmatch(r"[A-Za-z0-9_-]{1,160}", session):
        return
    if not isinstance(turn, str) or not turn or len(turn) > 160:
        return  # No guessing which turn a session-wide event belongs to.
    directory = root / "taskpulse-events"
    directory.mkdir(parents=True, exist_ok=True)
    destination = directory / (session + ".json")
    with locked(directory / (session + ".lock")):
        previous = {}
        if destination.exists():
            with destination.open("rb") as handle:
                body = handle.read(LIMIT + 1)
            if len(body) > LIMIT:
                raise ValueError("observer state exceeds budget")
            previous = json.loads(body)
        sequence = previous.get("sequence", 0) + 1
        same_turn = previous.get("turn_id") == turn
        calls = previous.get("calls", {}) if same_turn else {}
        pending = previous.get("pending", []) if same_turn else []
        call = event.get("tool_use_id")
        if not isinstance(call, str) or len(call) > 256:
            call = None
        sig = signature(event)
        if kind == "PreToolUse" and call:
            calls[call] = sig
        elif kind == "PermissionRequest":
            matches = [key for key, value in calls.items() if value == sig]
            associated = matches[0] if len(matches) == 1 else None
            if not associated or not any(p["call_id"] == associated for p in pending):
                pending.append({"id": f"hook:{session}:{sequence}", "call_id": associated, "signature": sig})
        elif kind == "PostToolUse" and call:
            # Match the call, or a unique approval signature when PreToolUse
            # lacked an ID. Parallel identical calls remain unresolved.
            unmatched = [p for p in pending if p["call_id"] is None and p["signature"] == sig]
            sole = unmatched[0]["id"] if len(unmatched) == 1 and not any(v == sig and k != call for k, v in calls.items()) else None
            pending = [p for p in pending if p["call_id"] != call and p["id"] != sole]
            calls.pop(call, None)
        elif kind in {"Stop", "Interrupt", "SessionEnd"}:
            calls, pending = {}, []
        state = {"v": 1, "session_id": session, "turn_id": turn, "sequence": sequence,
                 "at": int(time.time() * 1000) if now is None else now,
                 "event": kind, "calls": calls, "pending": pending}
        body = json.dumps(state, separators=(",", ":")).encode()
        if len(body) > LIMIT:
            raise ValueError("observer state exceeds budget")
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=directory, delete=False) as handle:
                temporary = Path(handle.name)
                handle.write(body)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, destination)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
        return state


def main():
    try:
        # Tool results can be large. Reject oversized input without storing it.
        body = sys.stdin.buffer.read(4 * 1024 * 1024 + 1)
        if len(body) <= 4 * 1024 * 1024:
            root = Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex")))
            record(root, json.loads(body))
    except (OSError, ValueError, TypeError, KeyError):
        pass  # Observation failure never blocks/approves an Agent command.


if __name__ == "__main__":
    main()
