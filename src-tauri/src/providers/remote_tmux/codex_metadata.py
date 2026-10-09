"""Read native sidebar metadata without creating or mutating any Codex DB."""
import glob
import json
import os
import sqlite3
from pathlib import Path

root = Path.home() / '.codex'
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

databases = sorted(glob.glob(str(root / 'state_*.sqlite')), key=os.path.getmtime, reverse=True)
if databases:
    db = sqlite3.connect(Path(databases[0]).as_uri() + '?mode=ro', uri=True, timeout=1)
    db.row_factory = sqlite3.Row
    columns = {r[1] for r in db.execute('pragma table_info(threads)')}
    fields = [f for f in ['id', 'name', 'title', 'first_user_message', 'cwd', 'is_pinned', 'updated_at'] if f in columns]
    if 'id' in fields:
        query = 'select ' + ','.join(fields) + ' from threads'
        if 'archived' in columns:
            query += ' where archived=0'
        for record in db.execute(query):
            row = dict(record)
            # A manual DB name wins over generated names; index tracks renames
            # in CLI/IDE versions which do not yet have a name column.
            title = row.get('name') or names.get(row['id']) or row.get('title') or row.get('first_user_message') or ''
            print('META\t' + json.dumps({'id': row['id'], 't': title[:240], 'cwd': row.get('cwd') or '', 'p': bool(row.get('is_pinned')), 'u': row.get('updated_at') or 0}, ensure_ascii=False))
    db.close()
else:
    for session_id, title in names.items():
        print('META\t' + json.dumps({'id': session_id, 't': title, 'cwd': '', 'p': False}, ensure_ascii=False))
