"""Bounded, read-only capability check. Return project paths, never conversations."""
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import stat


MAX_ENTRIES = 512
MAX_PROJECTS = 128
MAX_PROJECT_BYTES = 480 * 1024


def inspect(root):
    result = {"state": "missing", "data_detected": False, "projects": []}
    if not root.is_dir():
        return result
    projects = set()
    detected = readable = partial = incompatible = unreadable = False
    encoded_size = 0

    def add_project(path):
        nonlocal encoded_size, partial
        if not isinstance(path, str) or not path or path in projects:
            return
        raw = path.encode("utf-8", "surrogatepass")
        if len(raw) > 4096 or any(ord(c) < 32 or 127 <= ord(c) < 160 or 0xD800 <= ord(c) <= 0xDFFF for c in path):
            partial = True
            return
        cost = len(json.dumps(path, ensure_ascii=False).encode("utf-8")) + 2
        if len(projects) >= MAX_PROJECTS or encoded_size + cost > MAX_PROJECT_BYTES:
            partial = True
            return
        projects.add(path)
        encoded_size += cost

    try:
        databases = []
        with os.scandir(root) as entries:
            for index, entry in enumerate(entries):
                if index >= MAX_ENTRIES:
                    partial = True
                    break
                if entry.name.startswith("state_") and entry.name.endswith(".sqlite") and entry.is_file(follow_symlinks=False):
                    databases.append((entry.stat(follow_symlinks=False).st_mtime_ns, Path(entry.path)))
        if databases:
            _, path = max(databases)
            with closing(sqlite3.connect(path.absolute().as_uri() + "?mode=ro", uri=True, timeout=1)) as db:
                remaining = 2048
                def progress():
                    nonlocal remaining
                    remaining -= 1
                    return remaining <= 0
                db.set_progress_handler(progress, 1024)
                columns = {r[1] for r in db.execute("pragma table_info(threads)")}
                if not {"id", "cwd"}.issubset(columns):
                    incompatible = True
                else:
                    readable = True
                    detected = db.execute("select 1 from threads limit 1").fetchone() is not None
                    query = """select distinct cwd from threads
                        where typeof(cwd)='text' and length(cast(cwd as blob)) between 1 and 4096
                        order by cwd limit 129"""
                    for row in db.execute(query):
                        add_project(row[0])
                    if db.execute("""select 1 from threads where typeof(cwd)='text'
                            and length(cast(cwd as blob)) > 4096 limit 1""").fetchone():
                        partial = True
    except (OSError, ValueError, sqlite3.Error):
        unreadable = True

    sessions = root / "sessions"
    if sessions.is_dir():
        count = 0
        pending = [(sessions, 0)]
        while pending and count < MAX_ENTRIES:
            directory, depth = pending.pop()
            try:
                with os.scandir(directory) as entries:
                    readable = True
                    for entry in entries:
                        if count >= MAX_ENTRIES:
                            partial = True
                            break
                        count += 1
                        if entry.is_dir(follow_symlinks=False):
                            if depth < 4:
                                pending.append((Path(entry.path), depth + 1))
                            else:
                                partial = True
                        elif entry.name.endswith(".jsonl") and entry.is_file(follow_symlinks=False):
                            try:
                                fd = os.open(entry.path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
                                with os.fdopen(fd, "rb") as stream:
                                    if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                                        raise ValueError("not a regular file")
                                    line = stream.readline(65537)
                                if not line.endswith(b"\n") or len(line) > 65536:
                                    partial = True
                                    continue
                                row = json.loads(line)
                                if not isinstance(row, dict) or row.get("type") != "session_meta" or not isinstance(row.get("payload"), dict):
                                    incompatible = True
                                    continue
                                detected = True
                                add_project(row["payload"].get("cwd"))
                            except (OSError, ValueError, RecursionError):
                                unreadable = True
            except OSError:
                unreadable = True
        partial |= bool(pending)
    state = (
        "partial" if partial or detected and (unreadable or incompatible)
        else "read_error" if unreadable
        else "unsupported_format" if incompatible
        else "readable" if detected
        else "empty" if readable
        else "missing"
    )
    result.update(state=state, data_detected=detected, projects=sorted(projects))
    return result


if __name__ == "__main__":
    print(json.dumps({"schema_version": 1, "codex": inspect(Path.home() / ".codex")}, ensure_ascii=False))
