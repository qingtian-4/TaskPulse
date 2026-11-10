"""Isolated remote reader fixtures; run with python3 -B tests/ssh-claude.test.py."""
import datetime
import json
import os
from pathlib import Path
import runpy
import tempfile
import time
import unittest

reader = runpy.run_path(str(Path(__file__).resolve().parents[1] / "src-tauri/src/providers/ssh_claude.py"))
inspect = reader["inspect"]
SID = "11223344-1111-2222-3333-445566778899"


class RemoteReader(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "projects" / "fixture-project"
        self.project.mkdir(parents=True)
        self.log = self.project / (SID + ".jsonl")
        self.at = datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z")

    def row(self, kind, **extra):
        return {"type": kind, "sessionId": SID, "timestamp": self.at, **extra}

    def write(self, *rows):
        self.log.write_text("".join(json.dumps(row) + "\n" for row in rows))

    def start(self):
        return self.row("user", cwd="/srv/project", uuid="turn", message={"content": "private-prompt"})

    def test_only_current_turn_metadata_and_no_transcript_payload(self):
        self.write(
            self.row("user", cwd="/srv/project", uuid="old", message={"content": "previous-secret"}),
            self.row("assistant", message={"content": [], "stop_reason": "end_turn"}),
            self.start(),
            self.row("assistant", message={"model": "claude-sonnet", "content": [
                {"type": "text", "text": "private-response"},
                {"type": "tool_use", "id": "tool", "name": "Bash", "input": {"command": "private-command"}},
                {"type": "thinking", "thinking": "private-thinking"},
            ]}),
            self.row("user", toolUseResult={"content": "private-result"}, message={"content": [
                {"type": "tool_result", "tool_use_id": "tool", "content": "private-output"}
            ]}),
            self.row("assistant", message={"content": [], "stop_reason": "end_turn"}, isSidechain=True),
        )
        result = inspect(self.root, False, set())
        self.assertEqual(result["state"], "readable")
        rows = result["sessions"][0]["records"]
        self.assertEqual(len(rows), 3)
        self.assertEqual(rows[0]["uuid"], "turn")
        self.assertEqual(rows[0]["cwd"], "/srv/project")
        self.assertEqual(rows[1]["message"]["model"], "claude-sonnet")
        self.assertEqual(rows[2]["message"]["content"][0]["tool_use_id"], "tool")
        self.assertNotIn("end_turn", json.dumps(rows))
        self.assertNotIn("private-", json.dumps(result))
        self.assertNotIn("previous-secret", json.dumps(result))

    def test_incomplete_or_corrupt_write_and_tracked_old_file(self):
        self.write(self.start())
        with self.log.open("ab") as stream:
            stream.write(b'{"type":')
        degraded = inspect(self.root, False, {SID})
        self.assertEqual(degraded["state"], "partial")
        self.assertEqual(degraded["sessions"], [])
        self.write(self.start(), self.row("system", subtype="turn_duration"))
        old = time.time() - 1000
        os.utime(self.log, (old, old))
        self.assertEqual(inspect(self.root, False, set())["state"], "empty")
        self.assertEqual(inspect(self.root, False, {SID})["state"], "readable")
        self.assertEqual(inspect(self.root, True, set())["state"], "readable")

    @unittest.skipUnless(Path("/proc/self/stat").is_file() and Path("/etc/machine-id").is_file(), "Linux procfs")
    def test_pid_reuse_or_wrong_namespace_cannot_prove_a_live_turn(self):
        self.write(self.start())
        sessions = self.root / "sessions"
        sessions.mkdir()
        lease_path = sessions / f"{os.getpid()}.json"
        fields = Path("/proc/self/stat").read_text().rsplit(") ", 1)[-1].split()
        machine = Path("/etc/machine-id").read_text().strip()
        ns = os.readlink("/proc/self/ns/pid")
        lease = {"pid": os.getpid(), "sessionId": SID, "procStart": fields[19],
                 "startedAt": int(time.time() * 1000), "pidDomain": f"linux:{machine}:{ns}"}
        lease_path.write_text(json.dumps(lease))
        self.assertTrue(inspect(self.root, False, set())["sessions"][0]["live"])
        lease_path.write_text(json.dumps({**lease, "procStart": str(int(fields[19]) - 1)}))
        self.assertFalse(inspect(self.root, False, set())["sessions"][0]["live"])
        lease_path.write_text(json.dumps({**lease, "pidDomain": "linux:other:pid:[1]"}))
        self.assertFalse(inspect(self.root, False, set())["sessions"][0]["live"])


if __name__ == "__main__":
    unittest.main()
