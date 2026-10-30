"""Offline behavioral tests for the CI-only installed-upgrade harness."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request

spec = importlib.util.spec_from_file_location("upgrade", Path(__file__).with_name("upgrade-smoke.py"))
upgrade = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upgrade)


class UpgradeTests(unittest.TestCase):
    def test_nsis_standard_account_secret_uses_stdin_not_arguments(self):
        with patch.object(upgrade, "require_ci"), patch.object(upgrade, "owned_plan", return_value={"kind": "nsis"}), patch.object(upgrade.subprocess, "run") as child:
            upgrade.install_and_test(Path("fixture-plan.json"))
            arguments = child.call_args.args[0]
            password = child.call_args.kwargs["input"]
            self.assertGreater(len(password), 32)
            self.assertNotIn(password, " ".join(map(str, arguments)))
            self.assertNotIn("env", child.call_args.kwargs)
            self.assertIn("upgrade-standard-user.ps1", " ".join(map(str, arguments)))
        with patch.object(upgrade, "require_ci"), patch.object(upgrade, "owned_plan", return_value={"kind": "msi"}):
            with self.assertRaises(RuntimeError):
                upgrade.install_and_test(Path("fixture-plan.json"), standard_user=True)

    def test_manual_diagnostic_boot_cannot_count_as_automatic_restart(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            upgrade.save(root / "boot-1.0.0-1.json", {"pid": 1, "diagnostic_launch": True})
            self.assertIsNone(upgrade.automatic_boot(root, "1.0.0"))
            upgrade.save(root / "boot-1.0.0-2.json", {"pid": 2, "diagnostic_launch": False})
            self.assertEqual(upgrade.automatic_boot(root, "1.0.0")["pid"], 2)
            upgrade.save(root / "boot-1.0.0-3.json", {"pid": 3, "diagnostic_launch": False})
            with self.assertRaises(RuntimeError):
                upgrade.automatic_boot(root, "1.0.0")

    def test_nsis_passive_update_cannot_open_language_selector_after_silent_install(self):
        config = json.loads((upgrade.REPO / "src-tauri/tauri.conf.json").read_text())
        self.assertEqual(config["bundle"]["windows"]["nsis"]["languages"], ["English", "SimpChinese"])
        self.assertFalse(config["bundle"]["windows"]["nsis"]["displayLanguageSelector"])

    def test_msi_property_quotes_value_for_unicode_space_install_directory(self):
        command = upgrade.msi_command(["/i", r"C:\fixture\old.msi", "/qn"], {"INSTALLDIR": r"C:\测试 install"})
        self.assertIn(' INSTALLDIR="C:\\测试 install"', command)
        self.assertNotIn('"INSTALLDIR=', command)
        with self.assertRaises(ValueError):
            upgrade.msi_command([], {"INSTALLDIR": 'bad"value'})

    def test_never_installs_on_developer_or_self_hosted_machine(self):
        for variables in ({}, {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "self-hosted"},
                          {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "TASKPULSE_HOME": "test"}):
            with patch.dict(os.environ, variables, clear=True), patch.object(upgrade.platform, "system", return_value="Windows"):
                with self.assertRaises(RuntimeError):
                    upgrade.require_ci()

    def test_two_versions_and_unknown_scheme_fail_closed(self):
        self.assertEqual(upgrade.next_version("0.2.0-alpha.8"), "0.2.0-alpha.9")
        with self.assertRaises(ValueError):
            upgrade.next_version("1.0.0")

    def test_overlay_keeps_crypto_and_installed_guards(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "src-tauri").mkdir()
            for name in ("package.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/tauri.conf.json"):
                (root / name).write_bytes((upgrade.REPO / name).read_bytes())
            old = json.loads((root / "package.json").read_text())["version"]
            new = upgrade.next_version(old)
            upgrade.overlay(root, "fixture-public-key", "http://127.0.0.1:19001/token/latest.json", new, old)
            config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
            self.assertTrue(config["plugins"]["updater"]["requireSignedVersion"])
            self.assertTrue(config["bundle"]["createUpdaterArtifacts"])
            self.assertEqual(config["version"], new)
            self.assertEqual(config["bundle"]["windows"]["wix"]["version"], "0.2.0.1009")
            self.assertIn('version     = "' + new + '"', (root / "src-tauri/Cargo.toml").read_text())
            self.assertIn('name = "agent-tray"\nversion = "' + new + '"', (root / "src-tauri/Cargo.lock").read_text())

    def test_patch_contract_fails_on_missing_or_duplicate_anchor(self):
        with tempfile.TemporaryDirectory() as temp:
            file = Path(temp) / "source"
            for text in ("none", "anchor anchor"):
                file.write_text(text)
                with self.assertRaises(RuntimeError):
                    upgrade.replace_once(file, "anchor", "changed")
                self.assertEqual(file.read_text(), text)

    def test_state_assertions_detect_unread_and_ledger_loss(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(upgrade.Path, "home", return_value=Path(temp)):
            state, config = upgrade.seed_profile()
            upgrade.assert_state(state, config)
            with self.assertRaises(RuntimeError):
                upgrade.seed_profile()
            file = state / "attention.json"
            original = file.read_text()
            data = json.loads(original)
            data["records"]["upgrade-fixture:completed"]["unread"] = False
            upgrade.save(file, data)
            with self.assertRaises(AssertionError):
                upgrade.assert_state(state, config)
            file.write_text(original)
            ledger = state / "notifications.json"
            data = json.loads(ledger.read_text())
            data["notified"] = []
            upgrade.save(ledger, data)
            with self.assertRaises(AssertionError):
                upgrade.assert_state(state, config)

    def test_feed_serves_only_allowlisted_payload_and_never_arbitrary_paths(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            payload = root / "TaskPulse_0.2.0-alpha.9.exe"
            payload.write_bytes(b"payload" * 100)
            Path(str(payload) + ".sig").write_text("good")
            (root / "invalid-package.bin").write_bytes(b"invalid")
            (root / "private.key").write_text("must-never-serve")
            info = dict(route="random-route", new="0.2.0-alpha.9", root=str(root), new_payload=str(payload),
                        negative_signatures={"wrong-key": "wrong"})
            server = upgrade.http.server.ThreadingHTTPServer(("127.0.0.1", 0), upgrade.Feed)
            info["port"] = server.server_port
            shared = dict(info=info, scenario="good", lock=threading.Lock(), requests=[])
            server.shared = shared
            threading.Thread(target=server.serve_forever, daemon=True).start()
            try:
                base = f"http://127.0.0.1:{server.server_port}/"
                for path in ("private.key", "../private.key", "random-route/../../private.key"):
                    with self.assertRaises(urllib.error.HTTPError):
                        urllib.request.urlopen(base + path)
                manifest = json.load(urllib.request.urlopen(base + "random-route/latest.json"))
                url = manifest["platforms"]["windows-x86_64-nsis"]["url"]
                self.assertEqual(urllib.request.urlopen(url).read(), payload.read_bytes())
                shared["scenario"] = "tampered"
                self.assertNotEqual(urllib.request.urlopen(url).read(), payload.read_bytes())
                shared["scenario"] = "interrupted"
                with self.assertRaises(Exception):
                    urllib.request.urlopen(url).read()
            finally:
                server.shutdown()
                server.server_close()

    def test_production_has_no_test_driver_or_transport_override(self):
        main = (upgrade.REPO / "src-tauri/src/main.rs").read_text()
        self.assertNotIn("upgrade_probe", main)
        config = json.loads((upgrade.REPO / "src-tauri/tauri.conf.json").read_text())
        self.assertNotIn("dangerousInsecureTransportProtocol", config["plugins"]["updater"])
        self.assertTrue(all(url.startswith("https://github.com/") for url in config["plugins"]["updater"]["endpoints"]))
        self.assertIn('url.host_str() == Some("github.com")', (upgrade.REPO / "src-tauri/src/updates.rs").read_text())


if __name__ == "__main__":
    unittest.main()
