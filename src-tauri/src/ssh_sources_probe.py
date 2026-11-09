"""Bounded, read-only capability check. Return project paths, never conversations."""
import json
import os
from pathlib import Path
import sqlite3


def inspect(root):
    result = {"state": "missing", "data_detected": False, "projects": []}
    if not root.is_dir():
        return result
    projects = set()
    detected = False
    try:
        # Do not enumerate an unbounded remote directory just to choose one DB.
        databases = []
        with os.scandir(root) as entries:
            for index, entry in enumerate(entries):
                if index >= 512:
                    return {**result, "state": "read_error"}
                if entry.is_file(follow_symlinks=False) and entry.name.startswith("state_") and entry.name.endswith(".sqlite"):
                    databases.append(Path(entry.path))
        databases = sorted(databases, key=lambda p: p.stat().st_mtime, reverse=True)[:1]
        if databases:
            db = sqlite3.connect(databases[0].as_uri() + "?mode=ro", uri=True, timeout=1)
            try:
                columns = {r[1] for r in db.execute("pragma table_info(threads)")}
                if not {"id", "cwd"}.issubset(columns):
                    return {**result, "state": "unsupported_format"}
                for row in db.execute("select distinct cwd from threads where cwd is not null limit 128"):
                    if isinstance(row[0], str) and row[0] and len(row[0]) <= 4096:
                        projects.add(row[0])
                detected = bool(db.execute("select 1 from threads limit 1").fetchone())
            finally:
                db.close()
        sessions = root / "sessions"
        readable = bool(databases)
        if sessions.is_dir():
            # Opening/listing the directory must really succeed, not merely pass
            # an existence check. Bound both traversal and individual first lines.
            count = 0
            pending = [(sessions, 0)]
            while pending and count < 512:
                directory, depth = pending.pop()
                with os.scandir(directory) as entries:
                    for entry in entries:
                        count += 1
                        if count > 512:
                            break
                        if entry.is_dir(follow_symlinks=False) and depth < 4:
                            pending.append((Path(entry.path), depth + 1))
                        elif entry.is_file(follow_symlinks=False) and entry.name.endswith(".jsonl"):
                            with open(entry.path, "rb") as stream:
                                line = stream.readline(65537)
                            if len(line) > 65536:
                                return {**result, "state": "read_error"}
                            row = json.loads(line)
                            if not isinstance(row, dict) or row.get("type") != "session_meta":
                                return {**result, "state": "unsupported_format"}
                            detected = True
                            payload = row.get("payload")
                            if not isinstance(payload, dict):
                                return {**result, "state": "unsupported_format"}
                            path = payload.get("cwd")
                            if isinstance(path, str) and path and len(path) <= 4096 and len(projects) < 128:
                                projects.add(path)
            readable = True
        result.update(state=("readable" if detected else "empty") if readable else "missing",
                      data_detected=detected, projects=sorted(projects)[:128])
    except (OSError, ValueError, sqlite3.Error):
        result.update(state="read_error", data_detected=False, projects=[])
    return result


if __name__ == "__main__":
    print(json.dumps({"schema_version": 1, "codex": inspect(Path.home() / ".codex")}, ensure_ascii=False))
