"""Offline probe regression: fixtures only, no WSL/Agent/config access."""
import importlib.util
import json
import sqlite3
import sys
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[1] / 'src-tauri/src/providers/codex_wsl_discovery.py'
spec = importlib.util.spec_from_file_location('codex_discovery', source)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class DiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def sessions(self):
        path = self.root / 'sessions'
        path.mkdir(exist_ok=True)
        return path

    def test_empty_environment_and_generic_directory_are_not_codex(self):
        self.assertIsNone(probe.evidence(self.root / 'absent'))
        self.assertIsNone(probe.evidence(self.root))
        self.sessions()
        (self.root / 'auth.json').write_text('must not be read', encoding='utf-8')
        (self.root / 'config.toml').write_text('must not be read', encoding='utf-8')
        self.assertIsNone(probe.evidence(self.root))

    def test_read_only_schema_requires_codex_fields_and_does_not_mutate_db(self):
        self.sessions()
        path = self.root / 'state_5.sqlite'
        with closing(sqlite3.connect(path)) as db:
            db.execute('create table threads (id text, cwd text, rollout_path text)')
            db.commit()
        before = path.read_bytes()
        self.assertEqual(probe.evidence(self.root), 'thread_schema')
        self.assertEqual(path.read_bytes(), before)
        self.assertFalse((self.root / 'state_5.sqlite-journal').exists())

    def test_unrelated_or_corrupt_sqlite_is_not_codex(self):
        self.sessions()
        with closing(sqlite3.connect(self.root / 'state_4.sqlite')) as db:
            db.execute('create table threads (id text, unrelated text)')
            db.commit()
        (self.root / 'state_5.sqlite').write_bytes(b'corrupt')
        self.assertIsNone(probe.evidence(self.root))

    def test_native_index_fallback_tolerates_corrupt_rows(self):
        self.sessions()
        path = self.root / 'session_index.jsonl'
        path.write_text('broken\n[]\n{}\n' + json.dumps({'id': 'thread', 'thread_name': 'native'}), encoding='utf-8')
        self.assertEqual(probe.evidence(self.root), 'session_index')

    def test_old_rollout_requires_session_metadata_not_arbitrary_commands(self):
        sessions = self.sessions()
        path = sessions / 'rollout-fixture.jsonl'
        path.write_text(json.dumps({'type': 'event_msg', 'payload': {'id': 'tool'}}), encoding='utf-8')
        self.assertIsNone(probe.evidence(self.root))
        path.write_text(json.dumps({'type': 'session_meta', 'payload': {'id': 'thread'}}) + '\nprivate conversation is not parsed', encoding='utf-8')
        self.assertEqual(probe.evidence(self.root), 'rollout_metadata')


if __name__ == '__main__':
    unittest.main()
