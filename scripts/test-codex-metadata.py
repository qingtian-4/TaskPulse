"""Schema regression tests; runs with Python 3 on Windows, Linux or WSL."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'src-tauri/src/providers/remote_tmux/codex_metadata.py'


class MetadataTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / '.codex'
        self.root.mkdir()

    def probe(self):
        env = dict(os.environ, HOME=self.temp.name, USERPROFILE=self.temp.name)
        output = subprocess.check_output([sys.executable, str(SCRIPT)], env=env, text=True, encoding='utf-8')
        return [json.loads(line.split('\t', 1)[1]) for line in output.splitlines()]

    def legacy_db(self):
        with sqlite3.connect(self.root / 'state_3.sqlite') as db:
            db.execute('create table threads (id text, title text, cwd text)')
            db.execute("insert into threads values ('thread-1','Native task','/home/user/YOLO-RDD')")

    def test_older_schema_without_name_pin_or_archived(self):
        self.legacy_db()
        self.assertEqual(self.probe(), [{'id': 'thread-1', 't': 'Native task', 'cwd': '/home/user/YOLO-RDD', 'p': False, 'u': 0}])

    def test_index_rename_and_incomplete_index_line(self):
        self.legacy_db()
        (self.root / 'session_index.jsonl').write_text('{broken\n' + json.dumps({'id': 'thread-1', 'thread_name': 'Renamed task'}) + '\n', encoding='utf-8')
        self.assertEqual(self.probe()[0]['t'], 'Renamed task')

    def test_manual_name_wins_over_index(self):
        self.legacy_db()
        with sqlite3.connect(self.root / 'state_3.sqlite') as db:
            db.execute('alter table threads add name text')
            db.execute("update threads set name='Manual name'")
        (self.root / 'session_index.jsonl').write_text(json.dumps({'id': 'thread-1', 'thread_name': 'Older index name'}), encoding='utf-8')
        self.assertEqual(self.probe()[0]['t'], 'Manual name')

    def test_no_database_is_created(self):
        self.assertEqual(self.probe(), [])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_index_only_installation(self):
        (self.root / 'session_index.jsonl').write_text(json.dumps({'id': 'thread-1', 'thread_name': 'Native title'}), encoding='utf-8')
        self.assertEqual(self.probe()[0]['t'], 'Native title')


if __name__ == '__main__':
    unittest.main()
