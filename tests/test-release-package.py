"""Offline checks for legacy preservation and compact release provenance."""
import importlib.util
import json
import tempfile
import unittest
import zipfile
from pathlib import Path

spec = importlib.util.spec_from_file_location('release_package', (Path(__file__).resolve().parent / '../scripts/release-package.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
SHA = 'a' * 40


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='tp-release-package-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        self.source.mkdir()
        self.out = self.root / 'out'
        self.license = self.root / 'LICENSE'
        self.license.write_text('MIT fixture', encoding='utf-8')

    def write(self, name, value):
        (self.source / name).write_text(json.dumps(value) if isinstance(value, dict) else value, encoding='utf-8')

    def fixture(self, version):
        self.version = version
        compact = tuple(map(int, version.split('.'))) > (1, 1, 0)
        feed = {'version': version, 'platforms': {}}
        for platform, triple in package.TARGETS.items():
            if platform == 'windows-arm64' and not compact:
                continue
            system, arch = platform.split('-')
            prefix = '' if platform == 'windows-x64' else platform + '-'
            installers = [f'TaskPulse_{version}_{system}_{arch}' + ('_setup.exe' if system == 'windows' else '.dmg')]
            if platform == 'windows-x64':
                installers.append(f'TaskPulse_{version}_windows_x64.msi')
            update = {'version': version, 'commit': SHA, 'platforms': {}}
            updates = installers if system == 'windows' else [f'TaskPulse_{version}_macos_{arch}.app.tar.gz']
            for name in set(installers + updates):
                self.write(name, 'fixture:' + name)
            for name in updates:
                target = ('windows-x86_64-' if arch == 'x64' else 'windows-aarch64-') + ('msi' if name.endswith('.msi') else 'nsis') if system == 'windows' else ('darwin-aarch64' if arch == 'arm64' else 'darwin-x86_64')
                self.write(name + '.sig', 'fixture signature')
                entry = dict(url=f'https://github.com/qingtian-4/TaskPulse/releases/download/v{version}/{name}', signature='fixture signature', sha256=package.digest(self.source / name))
                update['platforms'][target] = entry
                feed['platforms'][target] = {k: entry[k] for k in ('url', 'signature')}
                if system == 'windows' and name.endswith('.exe'):
                    generic = target.removesuffix('-nsis')
                    update['platforms'][generic] = entry
                    feed['platforms'][generic] = feed['platforms'][target]
            self.write('updater-' + ('windows' if platform == 'windows-x64' else platform) + '.json', update)
            manifest = dict(version=version, tag='v' + version, commit=SHA, platform=system, architecture=arch, assets=installers)
            if system == 'macos':
                manifest['sha256'] = package.digest(self.source / installers[0])
            self.write(prefix + ('release-manifest.json' if system == 'windows' else 'manifest.json'), manifest)
            self.write(prefix + 'SHA256SUMS.txt', ''.join(f'{package.digest(self.source / n)}  {n}\n' for n in installers))
            self.write(prefix + 'DEPENDENCIES.json', dict(commit=SHA, target=triple))
            self.write(prefix + 'SBOM.cdx.json', {'metadata': {'component': {'version': version}, 'properties': [dict(name='taskpulse:source-commit', value=SHA), dict(name='taskpulse:target', value=triple)]}})
            self.write(prefix + 'THIRD_PARTY_NOTICES.txt', 'fixture MIT text')
        self.write('latest.json', feed)

    def run_package(self):
        return package.package_release(self.source, self.out, self.version, SHA, self.license)

    def test_current_version_preserves_all_26_assets_byte_for_byte(self):
        self.fixture('1.1.0')
        names = self.run_package()
        self.assertEqual(len(names), 26)
        for name in names:
            self.assertEqual((self.out / name).read_bytes(), (self.source / name).read_bytes())

    def test_future_layout_has_four_primary_installers_and_retains_msi_update(self):
        self.fixture('1.1.1')
        names = self.run_package()
        self.assertEqual(len(names), 11)
        self.assertEqual(len([n for n in names if n.endswith(('.exe', '.dmg'))]), 4)
        self.assertEqual(len([n for n in names if n.endswith('.msi')]), 1)
        rows = (self.out / 'SHA256SUMS.txt').read_text().splitlines()
        self.assertEqual(len(rows), 10)
        for row in rows:
            digest, name = row.split('  ')
            self.assertEqual(digest, package.digest(self.out / name))
        manifest = json.loads((self.out / 'release-manifest.json').read_text())
        self.assertEqual(manifest['commit'], SHA)
        self.assertEqual(set(manifest['platforms']['windows']), {'arm64', 'x64'})
        with zipfile.ZipFile(self.out / 'TaskPulse_1.1.1_dependency-licenses.zip') as archive:
            for platform in package.TARGETS:
                for name in package.NOTICES:
                    self.assertIn(platform + '/' + name, archive.namelist())
            self.assertIn('LICENSE', archive.namelist())
            self.assertEqual(len([n for n in archive.namelist() if n.startswith('signatures/')]), 5)

    def test_missing_arm_payload_cannot_be_presented_as_a_four_platform_release(self):
        self.fixture('1.1.1')
        (self.source / 'TaskPulse_1.1.1_windows_arm64_setup.exe').unlink()
        with self.assertRaises(ValueError):
            self.run_package()

    def test_corrupt_or_duplicate_checksum_blocks_packaging(self):
        for corrupt in ('payload', 'checksum'):
            with self.subTest(corrupt=corrupt):
                self.fixture('1.1.1')
                if corrupt == 'payload':
                    self.write('TaskPulse_1.1.1_windows_x64_setup.exe', 'changed')
                else:
                    file = self.source / 'SHA256SUMS.txt'
                    file.write_text(file.read_text() * 2)
                with self.assertRaises(ValueError):
                    self.run_package()

    def test_mixed_sha_inventory_or_feed_and_stale_output_fail_closed(self):
        for name, mutate in [
                ('macos-arm64-manifest.json', lambda m: m.update(commit='b' * 40)),
                ('windows-arm64-DEPENDENCIES.json', lambda m: m.update(target='wrong-target')),
                ('macos-x64-SBOM.cdx.json', lambda m: m['metadata']['component'].update(version='wrong')),
                ('updater-windows.json', lambda m: m.update(commit='b' * 40)),
                ('latest.json', lambda m: m['platforms'].pop('windows-x86_64-msi'))]:
            with self.subTest(name=name):
                self.fixture('1.1.1')
                data = json.loads((self.source / name).read_text())
                mutate(data)
                self.write(name, data)
                with self.assertRaises(ValueError):
                    self.run_package()
        self.fixture('1.1.1')
        self.run_package()
        with self.assertRaisesRegex(ValueError, 'empty'):
            self.run_package()

    def test_modified_signature_or_feed_url_is_rejected(self):
        self.fixture('1.1.1')
        self.write('TaskPulse_1.1.1_windows_x64_setup.exe.sig', 'changed')
        with self.assertRaisesRegex(ValueError, 'signature'):
            self.run_package()


if __name__ == '__main__':
    unittest.main()
