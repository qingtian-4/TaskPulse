"""Offline public-source binding regressions; never downloads or installs."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('public_install', Path(__file__).with_name('public-install-smoke.py'))
public = importlib.util.module_from_spec(spec)
spec.loader.exec_module(public)


class ManifestTests(unittest.TestCase):
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
