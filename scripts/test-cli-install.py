"""Run the actual macOS bootstrap offline with controlled system/transport doubles."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

REPO = Path(__file__).resolve().parent.parent
SCRIPT = REPO / 'install.sh'
VERSION = '0.2.0-alpha.8'
BASH = os.environ.get('TASKPULSE_TEST_BASH', '/bin/bash' if sys.platform == 'darwin' else 'bash')


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='tp-cli-空 格-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.home = self.root / 'home'
        self.home.mkdir()
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        tool_source = self.root / 'tool.py'
        shutil.copy2(REPO / 'scripts/fixtures/cli-install/tool.py', tool_source)
        tool_source.chmod(0o755)
        for tool in ['uname', 'sysctl', 'plutil', 'curl', 'hdiutil', 'ditto', 'codesign', 'lipo', 'ps', 'open', 'mv', 'xattr', 'uuidgen', 'stat', 'sw_vers', 'od']:
            (self.bin / tool).symlink_to(tool_source)
        self.options = {}
        self.release = {'tag_name': 'v' + VERSION, 'draft': False, 'prerelease': True,
                        'published_at': '2026-10-28T09:00:00Z', 'assets': []}
        self.assets = self.root / 'assets'
        self.assets.mkdir()
        self.prepare_assets()
        self.make_app(self.root / 'dmg-app', VERSION)
        self.environment = dict(os.environ, HOME=str(self.home), TMPDIR=str(self.root),
                                PATH=str(self.bin) + os.pathsep + os.environ['PATH'], TP_FIXTURE=str(self.root))

    def make_app(self, app, version, bundle_id='app.taskpulse.desktop'):
        (app / 'Contents/MacOS').mkdir(parents=True)
        (app / 'Contents/MacOS/agent-tray').write_text('synthetic executable')
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
            'CFBundleIdentifier': bundle_id, 'CFBundleShortVersionString': version,
            'CFBundleExecutable': 'agent-tray', 'LSMinimumSystemVersion': '13.3'}))

    def prepare_assets(self, version=VERSION, arch='arm64', kind=None):
        name = f'TaskPulse_{version}_macos_{arch}.dmg'
        body = b'synthetic dmg'
        (self.assets / name).write_bytes(body)
        digest = hashlib.sha256(body).hexdigest()
        (self.assets / f'macos-{arch}-SHA256SUMS.txt').write_text(f'{digest}  {name}\n')
        manifest = {'version': version, 'commit': 'a' * 40, 'platform': 'macos', 'architecture': arch,
                    'assets': [name], 'sha256': digest, 'signing': 'ad-hoc', 'notarized': False}
        if kind: manifest.update(build_kind=kind, tag=None)
        (self.assets / f'macos-{arch}-manifest.json').write_text(json.dumps(manifest))
        self.release['assets'] = [{'name': n, 'browser_download_url':
            f'https://github.com/qingtian-4/TaskPulse/releases/download/v{version}/{n}'}
            for n in [name, f'macos-{arch}-SHA256SUMS.txt', f'macos-{arch}-manifest.json']]

    def sync(self):
        (self.root / 'options.json').write_text(json.dumps(self.options))
        (self.root / 'release.json').write_text(json.dumps(self.release))
        (self.root / 'releases.json').write_text(json.dumps(getattr(self, 'releases', [self.release])))

    def run_script(self, *args, success=True):
        self.sync()
        result = subprocess.run([BASH, str(SCRIPT), *args], cwd=self.root, env=self.environment,
                                capture_output=True, text=True, encoding="utf-8", errors="backslashreplace", timeout=60)
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            if '--help' not in args: self.assertIn('SHA-256', result.stdout)
        else: self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def test_unicode_message_variables_have_explicit_boundaries(self):
        import re
        self.assertIsNone(re.search(r'\$[A-Za-z_][A-Za-z0-9_]*(?=[^\x00-\x7f])', SCRIPT.read_text()))

    def test_bootstrap_parses_on_selected_shell(self):
        result = subprocess.run([BASH, '-n', str(SCRIPT)], capture_output=True)
        self.assertEqual(result.returncode, 0, repr(result.stderr))

    def test_tool_doubles_are_executable_and_selected(self):
        for name in ['uname', 'curl', 'plutil', 'mv', 'xattr', 'stat']:
            self.assertEqual(shutil.which(name, path=self.environment['PATH']), str(self.bin / name))
            self.assertTrue(os.access(self.bin / name, os.X_OK))

    def test_platform_architecture_and_language(self):
        for options in [{'os': 'Linux'}, {'machine': 'unknown'}, {'machine': 'x86_64', 'arm': None, 'translated': None, 'translated_error': 'sysctl: Operation not permitted'}]:
            self.options = options
            self.run_script('--prerelease', success=False)
        self.options = {'machine': 'x86_64', 'translated': '1'}
        self.assertIn('已保存', self.run_script('--prerelease', '--download-only', '--language', 'zh-CN'))
        self.assertTrue((self.root / f'TaskPulse-v{VERSION}-macos-arm64').is_dir())

    def test_intel(self):
        self.options = {'machine': 'x86_64', 'translated': None, 'arm': '0', 'binary_arch': 'x86_64'}
        self.prepare_assets(arch='x64')
        self.run_script('--version', VERSION, '--no-launch')

    def test_intel_missing_arm_and_rosetta_keys(self):
        self.options = {'machine': 'x86_64', 'translated': None, 'arm': None, 'binary_arch': 'x86_64'}
        self.prepare_assets(arch='x64')
        self.run_script('--version', VERSION, '--no-launch')

    def test_stable_default_and_exact_prerelease(self):
        self.run_script('--download-only', success=False)
        self.run_script('--version', 'v' + VERSION, '--download-only')
        calls = (self.root / 'calls.jsonl').read_text()
        self.assertIn('/latest', calls)
        self.assertIn('/tags/v' + VERSION, calls)

    def test_newest_public_selection_skips_draft(self):
        self.releases = [dict(self.release, draft=True, published_at='2026-10-29T09:00:00Z'), self.release]
        self.run_script('--prerelease', '--download-only')

    def test_draft_unpublished_and_wrong_version(self):
        for key, value in [('draft', True), ('published_at', None), ('tag_name', 'v9.0.0')]:
            old = self.release[key]; self.release[key] = value
            self.run_script('--version', VERSION, '--download-only', success=False)
            self.release[key] = old

    def test_assets_missing_duplicate_and_external(self):
        original = list(self.release['assets'])
        for assets in [original[1:], original + [original[0]], [dict(original[0], browser_download_url='https://evil.invalid/payload'), *original[1:]]]:
            self.release['assets'] = assets
            self.run_script('--prerelease', '--download-only', success=False)
        self.assertFalse(any(self.root.glob('TaskPulse-v*')))

    def test_manifest_and_checksum(self):
        manifest = self.assets / 'macos-arm64-manifest.json'
        original = json.loads(manifest.read_text())
        for key, value in [('version', '0.0.0'), ('architecture', 'x64'), ('commit', 'main'), ('sha256', 'b' * 64), ('build_kind', 'pr-preview'), ('tag', None), ('platform', 'windows')]:
            manifest.write_text(json.dumps(dict(original, **{key: value})))
            self.run_script('--prerelease', '--download-only', success=False)
        manifest.write_text(json.dumps(original))
        sums = self.assets / 'macos-arm64-SHA256SUMS.txt'
        sums.write_text(sums.read_text() * 2)
        self.run_script('--prerelease', '--download-only', success=False)

    def test_interruption_and_corruption(self):
        self.options['interrupt'] = True
        self.run_script('--prerelease', success=False)
        self.options.clear()
        (self.assets / f'TaskPulse_{VERSION}_macos_arm64.dmg').write_bytes(b'corrupt')
        self.run_script('--prerelease', success=False)
        self.assertFalse((self.home / 'Applications/TaskPulse.app').exists())
        self.assertFalse(list(self.root.glob('taskpulse-download.*')))

    def test_paths_and_repeat_download(self):
        args = ['--prerelease', '--download-only', '--output-dir', './下载 with spaces']
        self.run_script(*args)
        self.run_script(*args, success=False)
        self.assertTrue((self.root / '下载 with spaces' / f'TaskPulse-v{VERSION}-macos-arm64').is_dir())

    def test_install_idempotence_and_state(self):
        state = self.home / '.taskpulse'; state.mkdir(); (state / 'marker').write_text('preserve')
        self.run_script('--prerelease', '--no-launch')
        self.assertIn('Already installed', self.run_script('--prerelease', '--no-launch'))
        self.assertEqual((state / 'marker').read_text(), 'preserve')
        calls = (self.root / 'calls.jsonl').read_text()
        self.assertNotIn('["open"', calls)
        self.assertIn('"-readonly"', calls)

    def test_upgrade_rollback_move_and_launch_failure(self):
        target = self.home / 'Applications/TaskPulse.app'
        self.make_app(target, '0.1.0')
        for options in [{'fail_move': True}, {'fail_installed': True}]:
            self.options = options
            self.run_script('--prerelease', success=False)
            self.assertEqual(plistlib.loads((target / 'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'], '0.1.0')
            self.assertFalse((target.parent / '.taskpulse-recovery').exists())
        self.options = {}
        self.run_script('--prerelease', '--no-launch')
        self.assertEqual(plistlib.loads((target / 'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'], VERSION)

    def test_running_other_product_symlink_recovery_lock(self):
        parent = self.home / 'Applications'; parent.mkdir()
        target = parent / 'TaskPulse.app'
        self.make_app(target, '0.1.0')
        self.options['running'] = True
        self.run_script('--prerelease', success=False)
        self.options.clear()
        shutil.rmtree(target); self.make_app(target, '0.1.0', 'other.product')
        self.run_script('--prerelease', success=False)
        shutil.rmtree(target); target.symlink_to(self.root / 'dmg-app', target_is_directory=True)
        self.run_script('--prerelease', success=False)
        target.unlink()
        for name in ['.taskpulse-recovery', '.taskpulse-install.lock']:
            path = parent / name; path.mkdir()
            self.run_script('--prerelease', success=False)
            self.assertTrue(path.is_dir()); path.rmdir()

    def test_binary_signature_extra_app_and_process_errors(self):
        for options in [{'binary_arch': 'x86_64'}, {'bad_signature': True}, {'extra_app': True}, {'binary_header': 'ca fe ba be 00 00 00 02'}]:
            self.options = options
            self.run_script('--prerelease', '--no-launch', success=False)
        self.options = {'ps_fail': True}
        self.make_app(self.home / 'Applications/TaskPulse.app', '0.1.0')
        self.run_script('--prerelease', success=False)

    def test_concurrent_download(self):
        self.options['delay'] = 1
        self.sync()
        command = [BASH, str(SCRIPT), '--prerelease', '--download-only']
        a = subprocess.Popen(command, cwd=self.root, env=self.environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        def stop():
            if a.poll() is None: a.kill()
            a.communicate()
        self.addCleanup(stop)
        deadline = time.monotonic() + 10
        while not list(self.root.glob('.*.lock')):
            self.assertLess(time.monotonic(), deadline); time.sleep(.02)
        self.run_script('--prerelease', '--download-only', success=False)
        out, err = a.communicate(timeout=20)
        self.assertEqual(a.returncode, 0, out + err)

    def test_publication_race_preserves_external_directory(self):
        self.options['publish_race'] = True
        self.run_script('--prerelease', '--download-only', success=False)
        dest = self.root / f'TaskPulse-v{VERSION}-macos-arm64'
        self.assertEqual([p.name for p in dest.iterdir()], ['external.txt'])
        self.assertFalse(list(self.root.glob('.taskpulse-download.*')))

    def test_publication_collision_preserves_foreign_child(self):
        self.options['publish_collision'] = True
        self.run_script('--prerelease', '--download-only', success=False)
        dest = self.root / f'TaskPulse-v{VERSION}-macos-arm64'
        children = list(dest.glob('.taskpulse-download.*/external-important.txt'))
        self.assertEqual(len(children), 1)
        self.assertEqual(children[0].read_text(), 'preserve collision')

    def test_install_race_retains_foreign_app_and_previous_backup(self):
        target = self.home / 'Applications/TaskPulse.app'
        self.make_app(target, '0.1.0')
        self.options['install_race'] = True
        output = self.run_script('--prerelease', '--no-launch', success=False)
        self.assertIn('Recovery required', output)
        self.assertEqual((target / 'external.txt').read_text(), 'preserve app')
        backup = target.parent / '.taskpulse-recovery/TaskPulse.app/Contents/Info.plist'
        self.assertEqual(plistlib.loads(backup.read_bytes())['CFBundleShortVersionString'], '0.1.0')

    def test_first_install_launch_failure_retains_verified_app(self):
        self.options['launch_fail'] = True
        output = self.run_script('--prerelease', success=False)
        target = self.home / 'Applications/TaskPulse.app'
        self.assertTrue(target.exists(), 'A system launch failure must not delete a validated installation')
        self.assertEqual(plistlib.loads((target / 'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'], VERSION)
        self.assertIn('installed', output.lower())
        self.assertIn(str(target), output)
        self.assertNotIn('No security attributes were removed', output)

    def test_upgrade_launch_failure_retains_new_app_and_old_backup(self):
        target = self.home / 'Applications/TaskPulse.app'
        self.make_app(target, '0.1.0')
        self.options['launch_fail'] = True
        output = self.run_script('--prerelease', success=False)
        self.assertEqual(plistlib.loads((target / 'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'], VERSION)
        old = target.parent / '.taskpulse-recovery/TaskPulse.app/Contents/Info.plist'
        self.assertEqual(plistlib.loads(old.read_bytes())['CFBundleShortVersionString'], '0.1.0')
        self.assertIn(str(old.parent.parent.parent), output)
        self.assertFalse((target.parent / '.taskpulse-install.lock').exists())


    def test_unsupported_os_never_replaces_existing_app(self):
        target = self.home / 'Applications/TaskPulse.app'
        self.make_app(target, '0.1.0')
        self.options['system_version'] = '13.2.1'
        output = self.run_script('--prerelease', success=False)
        self.assertIn('requires macOS 13.3', output)
        self.assertEqual(plistlib.loads((target / 'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'], '0.1.0')
        self.assertFalse((target.parent / '.taskpulse-recovery').exists())
        for version in ['unknown', '13.3.0.1', '13.03', '9999.0']:
            self.options['system_version'] = version
            self.run_script('--prerelease', success=False)
        self.options['system_version'] = '13.3'
        self.run_script('--prerelease', '--no-launch')

    def test_argument_validation(self):
        for args in [['--bad'], ['--version'], ['--version', '../x'], ['--language', 'fr']]:
            self.run_script(*args, success=False)
        self.assertIn('TaskPulse', self.run_script('--help'))


if __name__ == '__main__':
    unittest.main()
