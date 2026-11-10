"""Read Claude Code transcript metadata; never send message or tool payloads."""
import contextlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import time

MAX_ENTRIES = 8192
MAX_FILES = 64
TAIL_BYTES = 512 * 1024
TOTAL_BYTES = 8 * 1024 * 1024
OUTPUT_BYTES = 2 * 1024 * 1024
SESSION_ID = re.compile(r"[0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}")


def clean(value, limit=256):
    if isinstance(value, str) and len(value.encode("utf-8", "surrogatepass")) <= limit:
        if value and not any(ord(c) < 32 or 127 <= ord(c) < 160 or 0xD800 <= ord(c) <= 0xDFFF for c in value):
            return value
    return None


@contextlib.contextmanager
def read_file(path):
    # A directory entry can be replaced between enumeration and open.
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
    with os.fdopen(fd, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError("not a regular file")
        yield stream


def project(row, session_id):
    if not isinstance(row, dict):
        raise ValueError("invalid transcript row")
    if row.get("isSidechain") is True or row.get("isMeta") is True:
        return None
    if "sessionId" in row and row["sessionId"] != session_id:
        return None
    kind = row.get("type")
    if kind not in ("user", "assistant", "custom-title", "ai-title", "system"):
        return None
    if kind == "system" and row.get("subtype") != "turn_duration":
        return None
    result = {"type": kind}
    for key, limit in (("timestamp", 64), ("cwd", 4096), ("uuid", 128), ("promptId", 128)):
        if value := clean(row.get(key), limit):
            result[key] = value
    if kind in ("custom-title", "ai-title"):
        key = "customTitle" if kind == "custom-title" else "aiTitle"
        if value := clean(row.get(key), 512):
            result[key] = value
        return result
    if kind == "system":
        result["subtype"] = "turn_duration"
        return result
    message = row.get("message")
    if not isinstance(message, dict):
        raise ValueError("invalid transcript message")
    content = message.get("content")
    blocks = []
    has_text = isinstance(content, str) and bool(content.strip())
    if isinstance(content, list):
        for item in content:
            if not isinstance(item, dict):
                continue
            block = item.get("type")
            if block == "text" and isinstance(item.get("text"), str) and item["text"].strip():
                has_text = True
            elif block in ("tool_use", "tool_result", "thinking"):
                blocks.append({key: value for key in ("type", "id", "name", "tool_use_id")
                               if (value := clean(item.get(key), 128))})
    if kind == "user":
        if "toolUseResult" in row or any(item["type"] == "tool_result" for item in blocks):
            result["toolUseResult"] = True
        elif has_text:
            # A marker for the shared parser, never the actual user text.
            result["message"] = {"content": ""}
            return result
        else:
            return None
    elif row.get("isApiErrorMessage") is True:
        result["isApiErrorMessage"] = True
    if has_text and kind == "assistant":
        blocks.append({"type": "text", "text": ""})
    result["message"] = {"content": blocks}
    if kind == "assistant":
        if model := clean(message.get("model"), 128):
            result["message"]["model"] = model
        if message.get("stop_reason") == "end_turn":
            result["message"]["stop_reason"] = "end_turn"
    return result


def live_sessions(root):
    """Linux lease proof: match same-user PID, namespace and process start ticks."""
    live = set()
    try:
        machine = Path("/etc/machine-id").read_text().strip()
        namespace = os.readlink("/proc/self/ns/pid")
        with open("/proc/stat", encoding="ascii") as proc:
            boot_ms = next(int(line.split()[1]) * 1000 for line in proc if line.startswith("btime "))
        domain = f"linux:{machine}:{namespace}"
        with os.scandir(root / "sessions") as entries:
            for index, entry in enumerate(entries):
                if index >= MAX_ENTRIES:
                    break
                if not re.fullmatch(r"[0-9]{1,10}\.json", entry.name) or not entry.is_file(follow_symlinks=False):
                    continue
                try:
                    with read_file(entry.path) as stream:
                        lease = json.loads(stream.read(16385))
                    if not isinstance(lease, dict):
                        continue
                    pid = int(entry.name[:-5])
                    sid = lease.get("sessionId")
                    started = lease.get("startedAt")
                    if (pid != lease.get("pid") or not isinstance(sid, str)
                            or not SESSION_ID.fullmatch(sid) or type(started) is not int
                            or started < boot_ms or lease.get("pidDomain", domain) != domain):
                        continue
                    proc_dir = Path("/proc") / str(pid)
                    if proc_dir.stat().st_uid != os.getuid():
                        continue
                    # Field 22 follows the parenthesized comm (which can contain spaces).
                    fields = (proc_dir / "stat").read_text().rsplit(") ", 1)[-1].split()
                    if fields[0] not in ("Z", "X") and lease.get("procStart") == fields[19]:
                        live.add(sid)
                except (OSError, ValueError, IndexError):
                    continue
    except (OSError, ValueError, StopIteration):
        pass  # Older Claude or non-Linux hosts use fresh transcript evidence only.
    return live


def inspect(root, probe, tracked):
    result = {"schema_version": 1, "state": "missing", "sessions": []}
    live = set() if probe else live_sessions(root)
    now = time.time()
    paths = []
    seen = 0
    partial = False
    try:
        with os.scandir(root / "projects") as projects:
            for directory in projects:
                seen += 1
                if seen > MAX_ENTRIES:
                    break
                if not directory.is_dir(follow_symlinks=False):
                    continue
                with os.scandir(directory.path) as files:
                    for entry in files:
                        seen += 1
                        if seen > MAX_ENTRIES:
                            break
                        sid = entry.name[:-6]
                        if not entry.name.endswith(".jsonl") or not SESSION_ID.fullmatch(sid):
                            continue
                        if not entry.is_file(follow_symlinks=False):
                            continue
                        modified = entry.stat(follow_symlinks=False).st_mtime
                        priority = sid in tracked or sid in live
                        if probe or priority or modified >= now - 300:
                            paths.append((priority, modified, sid, entry.path))
                if seen > MAX_ENTRIES:
                    break
    except FileNotFoundError:
        if seen == 0:
            return result
        partial = True
    except OSError:
        partial = True
    partial |= seen > MAX_ENTRIES or len(paths) > MAX_FILES
    used = encoded = 0
    ids = set()
    for _, _, sid, path in sorted(paths, reverse=True)[:MAX_FILES]:
        if used >= TOTAL_BYTES or sid in ids:
            partial = True
            continue
        ids.add(sid)
        try:
            with read_file(path) as stream:
                before = os.fstat(stream.fileno())
                size = before.st_size
                count = min(TAIL_BYTES, TOTAL_BYTES - used, size)
                offset = size - count
                if offset:
                    stream.seek(offset - 1)
                    boundary = stream.read(1) == b"\n"
                    if not boundary:
                        offset_in_tail = stream.readline(count)
                        count -= len(offset_in_tail)
                        used += len(offset_in_tail) + 1
                body = stream.read(count)
                used += len(body)
                after = os.fstat(stream.fileno())
                if (len(body) != count or after.st_size != size
                        or after.st_mtime_ns != before.st_mtime_ns):
                    raise ValueError("transcript changed during read")
            rows = []
            # A trailing partial line cannot verify the latest turn.
            if body and not body.endswith(b"\n"):
                raise ValueError("incomplete transcript write")
            for line in body.splitlines():
                if line.strip() and (item := project(json.loads(line), sid)):
                    if item["type"] == "user" and "toolUseResult" not in item:
                        # Only the latest turn needs to cross SSH.
                        rows = [r for r in rows if r["type"] in ("custom-title", "ai-title")][-2:]
                    rows.append(item)
            if not rows:
                if size:
                    partial = True
                continue
            if len(rows) > 256:
                partial = True
                continue
            session = {"id": sid, "live": sid in live, "records": rows}
            encoded += len(json.dumps(session))
            if encoded > OUTPUT_BYTES:
                partial = True
                break
            result["sessions"].append(session)
        except (OSError, ValueError, RecursionError):
            partial = True
    result["state"] = ("partial" if partial else "readable" if result["sessions"] else "empty")
    return result


if __name__ == "__main__":
    root = Path(os.environ.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude").expanduser()
    print(json.dumps(inspect(root, sys.argv[1] == "probe", set(sys.argv[2:])), separators=(",", ":")))
