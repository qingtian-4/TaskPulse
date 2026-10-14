"""Read-only snapshot with independent segment outcomes; never run Agent code."""
import base64
import glob
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def collect(name, function):
    try:
        data = function()
        return {"ok": True, "observed_at": int(time.time() * 1000), "data": data}
    except Exception as error:
        # Keep diagnostics bounded. No process arguments or credentials in errors.
        return {"ok": False, "observed_at": int(time.time() * 1000), "error": type(error).__name__, "data": ""}


def panes():
    fmt = "\t".join("#{" + field + "}" for field in [
        "session_name", "window_index", "pane_index", "pane_id", "pane_pid",
        "pane_tty", "pane_current_path", "pane_current_command", "pane_title", "window_name"])
    result = subprocess.run(["tmux", "list-panes", "-a", "-F", fmt], capture_output=True, text=True, timeout=2)
    if result.returncode:
        error = result.stderr.lower()
        # A missing server is empty discovery, not proof a previous job exited.
        if "no server running" in error or ("no such file or directory" in error and "connect" in error):
            return ""
        raise RuntimeError("tmux query failed")
    for line in result.stdout.splitlines():
        if len(line.split("\t")) != 10:
            raise ValueError("invalid pane record")
    return result.stdout


process_starts = {}


def processes():
    rows = []
    for directory in Path("/proc").iterdir():
        if not directory.name.isdigit():
            continue
        try:
            stat = (directory / "stat").read_text()
            end = stat.rfind(")")
            fields = stat[end + 2:].split()
            pid = int(directory.name)
            process_starts[pid] = fields[19]
            comm = stat[stat.find("(") + 1:end].replace(" ", "_").replace("\n", "_")
            argv = (directory / "cmdline").read_bytes().replace(b"\0", b" ").decode("utf-8", errors="replace")
            argv = argv.replace("\n", "\\n").replace("\r", "\\r")
            rows.append(f"{pid} {int(fields[1])} {comm} {argv or comm}")
        except (FileNotFoundError, ProcessLookupError):
            continue  # A process exited between the two reads.
        except PermissionError:
            # Do not silently claim a complete process tree if access is denied.
            raise
    return "\n".join(rows)


def runs():
    directory = Path.home() / ".local/state/taskpulse/runs"
    if not directory.exists():
        return ""
    boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    rows = []
    for path in directory.glob("*.json"):
        # Recent terminal results only; active/stale runs have NO age cutoff.
        row = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(row, dict) or not row.get("run_id") or not row.get("status"):
            raise ValueError("invalid run record")
        if row["status"] in ("succeeded", "failed", "interrupted") and time.time() - path.stat().st_mtime > 10800:
            continue
        row["observed_pid_start_ticks"] = process_starts.get(row.get("pid"))
        row["observed_boot_id"] = boot_id
        row["observed_at_ms"] = int(time.time() * 1000)
        rows.append(json.dumps(row, ensure_ascii=False))
    return "\n".join(rows)


def codex():
    directory = Path.home() / ".codex/sessions"
    rows = []
    files = sorted(directory.glob("**/rollout-*.jsonl"), key=lambda path: path.stat().st_mtime, reverse=True)
    for path in files:
        if time.time() - path.stat().st_mtime > 86400:
            continue
        rows.append(f"===S:{path.parent.parent.name}===F:{path.name}===")
        with path.open("rb") as stream:
            first = stream.readline().decode("utf-8", errors="replace")
            stream.seek(0, os.SEEK_END)
            length = stream.tell()
            offset = max(0, length - 65536)
            stream.seek(offset)
            tail = stream.read()
            if offset:
                # The first partial record is not an event; discard it explicitly.
                tail = tail.split(b"\n", 1)[-1] if b"\n" in tail else b""
                rows.append(first)
            rows.append(tail.decode("utf-8", errors="replace"))
    metadata = subprocess.run([sys.executable, "-c", base64.b64decode(sys.argv[1]).decode("utf-8")],
                              capture_output=True, text=True, timeout=2.5)
    if metadata.returncode:
        raise RuntimeError("Codex metadata unavailable")
    rows.append(metadata.stdout)
    return "\n".join(rows)


segments = {}
segments["tmux"] = collect("tmux", panes)
segments["ps"] = collect("ps", processes)
segments["runs"] = collect("runs", runs)
segments["codex"] = collect("codex", codex)
print(json.dumps({"schema_version": 2, "segments": segments}, ensure_ascii=False))
