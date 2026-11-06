"""Capability checks must not confuse missing/failed data with a working Agent."""
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('ssh_probe', Path(__file__).resolve().parent.parent / 'src-tauri/src/ssh_sources_probe.py')
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='taskpulse-readonly-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / '.codex'

    def test_missing_and_readable_empty_are_distinct(self):
        self.assertEqual(probe.inspect(self.root)['state'], 'missing')
        (self.root / 'sessions').mkdir(parents=True)
        result = probe.inspect(self.root)
        self.assertEqual(result['state'], 'empty')
        self.assertFalse(result['data_detected'])

    def test_projects_come_from_metadata_without_returning_prompt_content(self):
        sessions = self.root / 'sessions'
        sessions.mkdir(parents=True)
        file = sessions / 'rollout.jsonl'
        body = json.dumps(dict(type='session_meta', payload=dict(id='public', cwd='/srv/public')))+'\n'+json.dumps(dict(type='message', text='PRIVATE_SENTINEL'))
        file.write_text(body, encoding='utf-8')
        before = file.read_bytes()
        result = probe.inspect(self.root)
        self.assertEqual(result['state'], 'readable')
        self.assertEqual(result['projects'], ['/srv/public'])
        self.assertNotIn('PRIVATE_SENTINEL', json.dumps(result))
        self.assertEqual(file.read_bytes(), before)

    def test_corruption_and_unrecognized_schema_are_not_success(self):
        self.root.mkdir()
        file = self.root / 'state_1.sqlite'
        file.write_bytes(b'bad database')
        self.assertEqual(probe.inspect(self.root)['state'], 'read_error')
        file.unlink()
        db = sqlite3.connect(file)
        db.execute('create table threads(id text)')
        db.commit()
        db.close()
        self.assertEqual(probe.inspect(self.root)['state'], 'unsupported_format')
        file.unlink()  # The read-only probe must release its SQLite handles.

    def test_database_is_read_only_and_bounded(self):
        self.root.mkdir()
        file = self.root / 'state_1.sqlite'
        db = sqlite3.connect(file)
        db.execute('create table threads(id text,cwd text)')
        db.executemany('insert into threads values(?,?)', [(str(n), '/srv/project-'+str(n)) for n in range(160)])
        db.commit()
        db.close()
        before = file.read_bytes()
        result = probe.inspect(self.root)
        self.assertEqual(result['state'], 'readable')
        self.assertLessEqual(len(result['projects']), 128)
        self.assertEqual(file.read_bytes(), before)
        self.assertEqual(set(p.name for p in self.root.iterdir()), {'state_1.sqlite'})

    def test_invalid_metadata_and_directory_budget_do_not_report_readability(self):
        sessions = self.root / 'sessions'
        sessions.mkdir(parents=True)
        file = sessions / 'rollout.jsonl'
        for body in ['[]', '{"type":"session_meta","payload":[]}', 'x' * 65537]:
            file.write_text(body, encoding='utf-8')
            result = probe.inspect(self.root)
            self.assertIn(result['state'], ['unsupported_format', 'read_error'])
            self.assertFalse(result['data_detected'])
        for n in range(512):
            (self.root / str(n)).touch()
        self.assertEqual(probe.inspect(self.root)['state'], 'read_error')


if __name__ == '__main__':
    unittest.main()
