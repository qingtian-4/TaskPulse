"""Read-only Linux metadata; stdout is a protocol, never a traceback/prompt."""
import errno
import glob
import json
import os
import sqlite3
import time
from pathlib import Path

class Fault(Exception):
    def __init__(self, stage, code):
        self.stage, self.code = stage, code


def retry(stage, action):
    # One retry only for transient errors, never schema/corruption/permissions.
    for attempt in range(2):
        try:
            return action()
        except sqlite3.OperationalError as exc:
            # Python 3.10 has neither sqlite_errorcode nor exported result
            # constants. Match only SQLite's exact known busy messages there.
            native_code = getattr(exc, 'sqlite_errorcode', None)
            transient = ((native_code & 255) in (5, 6) if native_code is not None else
                str(exc).lower() in ('database is locked', 'database table is locked', 'database is busy'))
            code = 'sqlite_busy' if transient else 'sqlite_query'
        except sqlite3.DatabaseError:
            raise Fault(stage, 'sqlite_corrupt')
        except UnicodeError:
            raise Fault(stage, 'invalid_utf8')
        except OSError as exc:
            transient = exc.errno in (errno.EAGAIN, errno.EINTR, errno.ESTALE)
            code = 'os_' + str(exc.errno or 0)
        if not transient or attempt:
            raise Fault(stage, code)
        time.sleep(0.06)


def collect():
    root = Path(os.environ.get('CODEX_HOME') or str(Path.home() / '.codex'))
    if not root.is_absolute() or '..' in root.parts:
        raise Fault('root', 'invalid_path')
    if not (root / 'sessions').is_dir():
        raise Fault('root', 'sessions_missing')

    def read_index():
        names = {}
        index = root / 'session_index.jsonl'
        if index.exists():
            with index.open(encoding='utf-8') as stream:
                for line in stream:
                    try:
                        row = json.loads(line)
                    except ValueError:
                        continue  # concurrent incomplete final record
                    if isinstance(row, dict) and row.get('id') and row.get('thread_name'):
                        names[row['id']] = row['thread_name']
        return names

    names = retry('session_index', read_index)

    def read_threads():
        databases = sorted(glob.glob(str(root / 'state_*.sqlite')), key=os.path.getmtime, reverse=True)
        if not databases:
            return [{'id': sid, 'sidebar_title': title, 'updated_at': 0} for sid, title in names.items()]
        db = sqlite3.connect(Path(databases[0]).as_uri() + '?mode=ro', uri=True, timeout=0.2)
        try:
            db.row_factory = sqlite3.Row
            columns = {r[1] for r in db.execute('pragma table_info(threads)')}
            fields = [f for f in ['id', 'name', 'title', 'first_user_message', 'cwd', 'rollout_path', 'updated_at', 'is_pinned', 'source'] if f in columns]
            if 'id' not in fields:
                raise Fault('sqlite', 'unsupported_schema')
            query = 'select ' + ','.join(fields) + ' from threads'
            if 'archived' in columns:
                query += ' where archived=0'
            threads = []
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
            return threads
        finally:
            db.close()

    threads = retry('sqlite', read_threads)

    def read_rollouts():
        files = []
        def walk_error(exc):
            raise exc
        for directory, _, entries in os.walk(root / 'sessions', onerror=walk_error, followlinks=False):
            for name in entries:
                if not name.endswith('.jsonl'):
                    continue
                path = Path(directory) / name
                stat = path.stat()
                files.append({'path': str(path), 'stamp': {'dev': stat.st_dev, 'ino': stat.st_ino,
                    'len': stat.st_size, 'modified_ns': stat.st_mtime_ns, 'changed_ns': stat.st_ctime_ns}})
        return files

    rollouts = retry('rollout_stat', read_rollouts)
    return {'root': str(root), 'threads': threads, 'rollouts': rollouts}


def main():
    try:
        result = collect()
    except Fault as exc:
        result = {'error': {'stage': exc.stage, 'code': exc.code}}
    except (sqlite3.DatabaseError, UnicodeError, ValueError, TypeError):
        result = {'error': {'stage': 'metadata', 'code': 'invalid_data'}}
    except OSError as exc:
        result = {'error': {'stage': 'metadata', 'code': 'os_' + str(exc.errno or 0)}}
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
