"""Read-only WSL Codex metadata. Never open auth/config or execute Agent work."""
import glob
import json
import os
import sqlite3
from pathlib import Path

root = Path(os.environ.get('CODEX_HOME') or str(Path.home() / '.codex'))
if not root.is_absolute():
    raise ValueError('CODEX_HOME must be an absolute Linux path')
if not (root / 'sessions').is_dir():
    raise FileNotFoundError('WSL Codex sessions directory not found')
names = {}
index = root / 'session_index.jsonl'
if index.exists():
    with index.open(encoding='utf-8') as stream:
        for line in stream:
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get('id') and row.get('thread_name'):
                names[row['id']] = row['thread_name']

threads = []
databases = sorted(glob.glob(str(root / 'state_*.sqlite')), key=os.path.getmtime, reverse=True)
if databases:
    db = sqlite3.connect(Path(databases[0]).as_uri() + '?mode=ro', uri=True, timeout=1)
    db.row_factory = sqlite3.Row
    columns = {r[1] for r in db.execute('pragma table_info(threads)')}
    fields = [f for f in ['id', 'name', 'title', 'first_user_message', 'cwd', 'rollout_path', 'updated_at', 'is_pinned', 'source'] if f in columns]
    if 'id' not in fields:
        raise ValueError('Unsupported Codex thread schema')
    query = 'select ' + ','.join(fields) + ' from threads'
    if 'archived' in columns:
        query += ' where archived=0'
    for record in db.execute(query):
        row = dict(record)
        row['sidebar_title'] = names.get(row['id'])
        row['is_pinned'] = bool(row.get('is_pinned'))
        row['updated_at'] = row.get('updated_at') or 0
        try:
            row['source'] = json.loads(row.get('source') or 'null')
        except (ValueError, TypeError):
            row['source'] = None
        threads.append(row)
    db.close()
else:
    threads = [{'id': sid, 'sidebar_title': title, 'updated_at': 0} for sid, title in names.items()]
print(json.dumps({'root': str(root), 'threads': threads}, ensure_ascii=False))
