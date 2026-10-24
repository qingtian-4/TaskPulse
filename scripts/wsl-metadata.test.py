"""Isolated Linux helper regressions: python3 scripts/wsl-metadata.test.py.
Never touches real Codex state, wakes a distro, or starts an Agent.
"""
import contextlib
import importlib.util
import io
import json
import os
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('wsl_metadata', Path(__file__).parents[1] / 'src-tauri/src/providers/codex_wsl_metadata.py')
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


class MetadataTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='taskpulse-metadata-')
        self.root = Path(self.temp.name)
        (self.root / 'sessions').mkdir()
        self.env = patch.dict(os.environ, {'CODEX_HOME': str(self.root)})
        self.env.start()

    def tearDown(self):
        self.env.stop()
        self.temp.cleanup()

    def output(self):
        with contextlib.redirect_stdout(io.StringIO()) as stdout:
            helper.main()
        return json.loads(stdout.getvalue())

    def database(self):
        db = sqlite3.connect(self.root / 'state_5.sqlite')
        db.execute('create table threads(id text,title text,cwd text,rollout_path text,archived integer)')
        db.execute("insert into threads values('one','original','/project',null,0)")
        db.commit()
        return db

    def test_native_rename_and_linux_stat_do_not_change_identity(self):
        self.database().close()
        index = self.root / 'session_index.jsonl'
        index.write_text('{"id":"one","thread_name":"Renamed"}\n', encoding='utf8')
        log = self.root / 'sessions/rollout-one.jsonl'
        log.write_text('{}\n', encoding='utf8')
        first = self.output()
        self.assertEqual(first['threads'][0]['sidebar_title'], 'Renamed')
        index.write_text('{"id":"one","thread_name":"Again"}\n', encoding='utf8')
        second = self.output()
        self.assertEqual(second['threads'][0]['id'], 'one')
        self.assertEqual(second['threads'][0]['sidebar_title'], 'Again')
        self.assertEqual(first['rollouts'][0]['stamp'], second['rollouts'][0]['stamp'])

    def test_locked_sqlite_is_bounded_and_staged_without_a_traceback(self):
        db = self.database()
        db.execute('begin exclusive')
        try:
            result = self.output()
            self.assertEqual(result, {'error': {'stage': 'sqlite', 'code': 'sqlite_busy'}})
            self.assertNotIn(str(self.root), json.dumps(result))
        finally:
            db.rollback()
            db.close()
        self.assertNotIn('error', self.output())

    def test_corrupt_db_and_unsupported_schema_are_not_retried_as_success(self):
        path = self.root / 'state_5.sqlite'
        path.write_bytes(b'not a database')
        self.assertEqual(self.output()['error'], {'stage': 'sqlite', 'code': 'sqlite_corrupt'})
        path.unlink()
        sqlite3.connect(path).close()
        self.assertEqual(self.output()['error'], {'stage': 'sqlite', 'code': 'unsupported_schema'})

    def test_index_invalid_utf8_is_identified_without_exposing_content(self):
        (self.root / 'session_index.jsonl').write_bytes(b'private\xff')
        self.assertEqual(self.output()['error'], {'stage': 'session_index', 'code': 'invalid_utf8'})

    def test_transient_retry_is_exactly_once(self):
        calls = []
        def action():
            calls.append(1)
            if len(calls) == 1:
                raise InterruptedError(4, 'private details')
            return 42
        with patch.object(helper.time, 'sleep'):
            self.assertEqual(helper.retry('rollout_stat', action), 42)
        self.assertEqual(len(calls), 2)

    def test_missing_rollout_directory_is_not_an_empty_success(self):
        (self.root / 'sessions').rmdir()
        self.assertEqual(self.output()['error'], {'stage': 'root', 'code': 'sessions_missing'})


if __name__ == '__main__':
    unittest.main()
