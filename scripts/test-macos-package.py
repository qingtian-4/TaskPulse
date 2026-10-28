"""Offline staging regressions; does not install or launch an application."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("macos_package", Path(__file__).with_name("macos-package.py"))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class PackagingTests(unittest.TestCase):
    def test_compact_empty_window_matches_available_screen_not_old_scale(self):
        normal={"work_area":{"width":1440,"height":900},"windows":[{"width":368,"height":400}]}
        small={"work_area":{"width":1024,"height":700},"windows":[{"width":328,"height":400}]}
        short={"work_area":{"width":800,"height":440},"windows":[{"width":320,"height":361}]}
        for value in [normal,small,short]:
            self.assertTrue(package.compact_empty_dashboard(value))
        for window in [{"width":371,"height":360},{"width":412,"height":620},{"width":368,"height":560}]:
            self.assertFalse(package.compact_empty_dashboard({**normal,"windows":[window]}))
        self.assertFalse(package.compact_empty_dashboard({**normal,"windows":[]}))
        with self.assertRaisesRegex(ValueError,"work area"):
            package.compact_empty_dashboard({"windows":[{"width":368,"height":400}]})

    def test_staging_and_stale_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / "bundle"
            bundle.mkdir()
            (bundle / "original.dmg").write_bytes(b"test payload" + b"koly" + bytes(508))
            output = root / "output"
            dmg = package.stage(bundle, output, "0.2.0-alpha.8", "a" * 40, "arm64")
            manifest = json.loads((output / "macos-arm64-manifest.json").read_text())
            self.assertEqual(manifest["commit"], "a" * 40)
            self.assertEqual(manifest["assets"], [dmg.name])
            self.assertFalse(manifest["notarized"])
            self.assertEqual((output / "macos-arm64-SHA256SUMS.txt").read_text(),
                             f"{package.hashlib.sha256(dmg.read_bytes()).hexdigest()}  {dmg.name}\n")
            with self.assertRaisesRegex(ValueError, "stale"):
                package.stage(bundle, output, "0.2.0-alpha.8", "a" * 40, "arm64")

    def test_preview_marker_is_explicit_and_legacy_default_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / "bundle"
            bundle.mkdir()
            (bundle / "image.dmg").write_bytes(b"payload" + b"koly" + bytes(508))
            package.stage(bundle, root / "preview", "0.2.0-alpha.8", "a" * 40, "x64", "pr-preview")
            preview = json.loads((root / "preview/macos-x64-manifest.json").read_text())
            self.assertEqual(preview["build_kind"], "pr-preview")
            self.assertIsNone(preview["tag"])
            package.stage(bundle, root / "legacy", "0.2.0-alpha.8", "a" * 40, "x64")
            legacy = json.loads((root / "legacy/macos-x64-manifest.json").read_text())
            self.assertNotIn("build_kind", legacy)
            with self.assertRaisesRegex(ValueError, "kind"):
                package.stage(bundle, root / "invalid", "0.2.0-alpha.8", "a" * 40, "x64", "release")

    def test_bad_identity_and_malformed_or_duplicate_images(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for version, commit, arch in [("0.2.0-invalid", "a" * 40, "arm64"),
                                           ("0.2.0-alpha.8", "main", "arm64"),
                                           ("0.2.0-alpha.8", "a" * 40, "../x")]:
                with self.assertRaises(ValueError):
                    package.stage(root, root / "out", version, commit, arch)
            for body in (b"short", bytes(600)):
                (root / "bad.dmg").write_bytes(body)
                with self.assertRaisesRegex(ValueError, "trailer"):
                    package.stage(root, root / "out", "0.2.0-alpha.8", "a" * 40, "arm64")
            (root / "second.dmg").write_bytes(bytes(600))
            with self.assertRaisesRegex(ValueError, "exactly one"):
                package.stage(root, root / "out", "0.2.0-alpha.8", "a" * 40, "arm64")


if __name__ == "__main__":
    unittest.main()
