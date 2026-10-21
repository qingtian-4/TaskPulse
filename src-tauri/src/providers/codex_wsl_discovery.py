"""Identify Codex session storage, not merely an installed/running WSL distro.

Only inspect schema/identity metadata. Never read auth/config, launch Codex or
return conversation content. SQLite is opened inside Linux, read-only.
"""
import json
import os
import sqlite3
from contextlib import closing
from pathlib import Path


def evidence(root):
    if not (root / 'sessions').is_dir():
        return None
    for path in sorted(root.glob('state_*.sqlite'), reverse=True):
        try:
            with closing(sqlite3.connect(path.as_uri() + '?mode=ro', uri=True, timeout=0.2)) as db:
                columns = {row[1] for row in db.execute('pragma table_info(threads)')}
                if 'id' in columns and {'cwd', 'rollout_path'} & columns:
                    return 'thread_schema'
        except (OSError, sqlite3.Error):
            continue
    try:
        with (root / 'session_index.jsonl').open(encoding='utf-8') as stream:
            # Discovery is a bounded presence check, not a history import.
            for line in stream.read(65536).splitlines():
                try:
                    row = json.loads(line)
                    if isinstance(row, dict) and row.get('id') and row.get('thread_name'):
                        return 'session_index'
                except ValueError:
                    continue
    except (OSError, UnicodeError):
        pass
    # Older installations may have rollout logs without a DB/index. Read only
    # the first (session_meta) line, never prompts or subsequent tool events.
    for number, path in enumerate((root / 'sessions').rglob('rollout-*.jsonl')):
        if number >= 32:
            break
        try:
            with path.open(encoding='utf-8') as stream:
                row = json.loads(stream.readline(65536))
            if (isinstance(row, dict) and row.get('type') == 'session_meta'
                    and isinstance(row.get('payload'), dict) and row['payload'].get('id')):
                return 'rollout_metadata'
        except (OSError, ValueError, UnicodeError):
            continue
    return None


if __name__ == '__main__':
    root = Path(os.environ.get('CODEX_HOME') or str(Path.home() / '.codex'))
    if not root.is_absolute() or '..' in root.parts:
        raise ValueError('CODEX_HOME must be an absolute Linux path')
    print(json.dumps({'root': str(root), 'evidence': evidence(root)}))
