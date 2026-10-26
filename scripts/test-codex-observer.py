import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("observer", Path(__file__).with_name("codex-observer.py"))
observer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(observer)


class ObserverTests(unittest.TestCase):
    def test_mcp_description_remains_part_of_input_identity(self):
        event = {"tool_name":"mcp__example__edit", "tool_input":{"description":"first"}}
        other = {**event, "tool_input":{"description":"second"}}
        self.assertNotEqual(observer.signature(event), observer.signature(other))

    def test_install_is_reversible_and_preserves_other_handlers(self):
        import contextlib
        import io
        import json
        spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-codex-observer.py"))
        installer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installer)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "hooks.json"
            original = {"custom":"keep", "hooks":{"PermissionRequest":[{"hooks":[{"type":"command","command":"other-observer"}]}]}}
            path.write_text(json.dumps(original))
            with contextlib.redirect_stdout(io.StringIO()):
                installer.install(root,root/"observer.py")
                installed=path.read_bytes()
                installer.install(root,root/"observer.py")
                self.assertEqual(path.read_bytes(),installed)
                installer.install(root,root/"observer.py",remove=True)
            self.assertEqual(json.loads(path.read_text()),original)
            self.assertTrue(list(root.glob("hooks.json.pre-taskpulse-*.bak")))

    def test_parallel_approvals_resolve_only_matching_call_without_storing_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def send(kind, call=None, command="secret-command", turn="turn", description=False):
                value = {"command": command}
                if description:
                    value["description"] = "private-reason"
                return observer.record(root, {"session_id": "s", "turn_id": turn,
                    "hook_event_name": kind, "tool_use_id": call, "tool_name": "Bash", "tool_input": value})
            send("PreToolUse", "a")
            first = send("PermissionRequest", description=True)
            self.assertEqual(first["pending"][0]["call_id"], "a")
            send("PreToolUse", "b", "other-secret")
            both = send("PermissionRequest", command="other-secret")
            self.assertEqual(len(both["pending"]), 2)
            self.assertEqual(len(send("PostToolUse", "unrelated", "unrelated")["pending"]), 2)
            remaining = send("PostToolUse", "a")
            self.assertEqual(remaining["pending"][0]["call_id"], "b")
            self.assertNotIn("secret", (root / "taskpulse-events/s.json").read_text())
            self.assertEqual(send("Stop")["pending"], [])
            send("PermissionRequest")
            self.assertEqual(send("PreToolUse", "new", turn="new-turn")["pending"], [])

    def test_ambiguous_identical_calls_do_not_clear_other_requests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            event = {"session_id":"s", "turn_id":"t", "tool_name":"Bash", "tool_input":{"command":"x"}}
            for call in ["a", "b"]:
                observer.record(root, {**event,"hook_event_name":"PreToolUse","tool_use_id":call})
            state = observer.record(root, {**event,"hook_event_name":"PermissionRequest"})
            self.assertIsNone(state["pending"][0]["call_id"])
            state = observer.record(root, {**event,"hook_event_name":"PostToolUse","tool_use_id":"a"})
            self.assertEqual(len(state["pending"]), 1)

    def test_bad_identity_and_corrupt_state_never_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            observer.record(root, {"session_id":"../bad", "turn_id":"t", "hook_event_name":"PermissionRequest"})
            self.assertEqual(list(root.iterdir()), [])
            events = root / "taskpulse-events"
            events.mkdir()
            path = events / "s.json"
            path.write_text("{partial")
            with self.assertRaises(ValueError):
                observer.record(root,{"session_id":"s","turn_id":"t","hook_event_name":"PermissionRequest"})
            self.assertEqual(path.read_text(), "{partial")


if __name__ == "__main__":
    unittest.main()
