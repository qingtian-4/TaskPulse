"""Offline public-source binding regressions; never downloads or installs."""
import importlib.util
from pathlib import Path
import unittest
import tempfile
import subprocess
import json
import os
import re
import sys
import textwrap

spec = importlib.util.spec_from_file_location('public_install', Path(__file__).with_name('public-install-smoke.py'))
public = importlib.util.module_from_spec(spec)
spec.loader.exec_module(public)


class ManifestTests(unittest.TestCase):
    def test_actual_workflow_platform_plan_keeps_legacy_and_adds_arm64_only_later(self):
        workflow = (Path(__file__).resolve().parent.parent / '.github/workflows/public-install-smoke.yml').read_text(encoding='utf-8')
        program = textwrap.dedent(re.search(r"python3 - <<'PY'\n(.*?)\n          PY", workflow, re.S).group(1))
        for tag, count in [('v1.0.1', 3), ('v1.1.0', 3), ('v1.1.1', 4), ('v2.0.0', 4), ('bad', None), ('v1.1.1-rc.1', None)]:
            with self.subTest(tag=tag), tempfile.TemporaryDirectory(prefix='tp-public-plan-') as temp:
                output = Path(temp) / 'output'
                result = subprocess.run([sys.executable, '-c', program], env=dict(os.environ, RELEASE_TAG=tag, GITHUB_OUTPUT=str(output)), capture_output=True, timeout=10)
                if count is None:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(output.exists())
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    rows = json.loads(output.read_text().removeprefix('matrix='))['include']
                    self.assertEqual(len(rows), count)
                    self.assertEqual(any(row['os'] == 'windows-11-arm' for row in rows), count == 4)

    def test_unified_root_and_platform_identity_are_both_required(self):
        record = dict(version='1.1.1', tag='v1.1.1', commit='a' * 40, platform='windows', architecture='arm64', assets=['setup.exe'])
        metadata = dict(format='taskpulse-release-v2', version='1.1.1', tag='v1.1.1', commit='a' * 40, build_kind='release', platforms={'windows': {'arm64': record}})
        public.validate_manifest(metadata, 'v1.1.1', 'a' * 40, 'windows', 'arm64', 'setup.exe')
        for change in [dict(commit='b' * 40), dict(format='unknown'), dict(build_kind='pr-preview'), dict(platforms={})]:
            with self.assertRaisesRegex(RuntimeError, 'does not match'):
                public.validate_manifest(dict(metadata, **change), 'v1.1.1', 'a' * 40, 'windows', 'arm64', 'setup.exe')

    def test_released_bootstrap_uses_git_blob_bytes_with_a_crlf_checkout(self):
        with tempfile.TemporaryDirectory(prefix='tp-public-blob-') as temp:
            root = Path(temp)
            for args in [('init', '-q'), ('config', 'user.name', 'TaskPulse fixture'), ('config', 'user.email', 'fixture@example.invalid'), ('config', 'core.autocrlf', 'false')]:
                subprocess.run(['git', *args], cwd=root, check=True, capture_output=True)
            original = b'Write-Output "TaskPulse"\n'
            (root / 'install.ps1').write_bytes(original)
            subprocess.run(['git', 'add', 'install.ps1'], cwd=root, check=True)
            subprocess.run(['git', 'commit', '-qm', 'fixture'], cwd=root, check=True)
            commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
            (root / 'install.ps1').write_bytes(original.replace(b'\n', b'\r\n'))
            self.assertNotEqual((root / 'install.ps1').read_bytes(), original)
            self.assertEqual(public.canonical_bootstrap(root, commit, 'install.ps1'), original)
            with self.assertRaises(ValueError):
                public.canonical_bootstrap(root, commit, '../outside')

    def test_platform_and_tag_source_are_required_for_both_systems(self):
        commit = 'a' * 40
        for platform, architecture, package in [('windows', 'x64', 'setup.exe'), ('macos', 'arm64', 'app.dmg'), ('macos', 'x64', 'app.dmg')]:
            metadata = dict(version='1.0.0', commit=commit, platform=platform, architecture=architecture, assets=[package])
            public.validate_manifest(metadata, 'v1.0.0', commit, platform, architecture, package)
            for change in [dict(commit='b' * 40), dict(version='1.0.1'), dict(platform='other'), dict(architecture='other'),
                           dict(tag='v1.0.1'), dict(build_kind='pr-preview'), dict(assets=[])]:
                with self.subTest(platform=platform, architecture=architecture, change=change):
                    with self.assertRaisesRegex(RuntimeError, 'does not match'):
                        public.validate_manifest({**metadata, **change}, 'v1.0.0', commit, platform, architecture, package)


if __name__ == '__main__':
    unittest.main()
