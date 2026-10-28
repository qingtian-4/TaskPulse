"""Native CLI acceptance. Only transport is replaced; app/DMG tools are real."""
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    if platform.system() != 'Darwin' or os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Disposable macOS CI only')
    assets = Path(sys.argv[1]).resolve()
    arch = sys.argv[2]
    if arch not in ('arm64', 'x64'):
        raise ValueError('Unknown architecture')
    evidence = Path(sys.argv[3]).resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    repo = Path(__file__).resolve().parent.parent
    script = repo / 'install.sh'
    manifest = json.loads((assets / f'macos-{arch}-manifest.json').read_text())
    if manifest.get('tag') is not None or manifest.get('build_kind') != 'pr-preview':
        raise RuntimeError('Expected preview manifest')
    version = manifest['version']
    tag = 'v' + version
    name = f'TaskPulse_{version}_macos_{arch}.dmg'
    result = dict(passed=False, os=platform.mac_ver()[0], architecture=platform.machine(),
                  source_commit=manifest['commit'], script_sha256=digest(script),
                  package_sha256=digest(assets / name), installation_method='production-install.sh',
                  anonymous_public_download_verified=False, gatekeeper_download_verified=False,
                  physical_mac_verified=False, cross_version_upgrade_verified=False, checks=[])
    try:
        with tempfile.TemporaryDirectory(prefix='taskpulse-cli-') as temporary:
            root = Path(temporary)
            response = root / 'response'
            response.mkdir()
            files = [name, f'macos-{arch}-SHA256SUMS.txt', f'macos-{arch}-manifest.json']
            for file in files:
                shutil.copy2(assets / file, response / file)
            release = dict(tag_name=tag, draft=False, prerelease=True, published_at='2026-10-28T00:00:00Z',
                           assets=[dict(name=file, browser_download_url=f'https://github.com/qingtian-4/TaskPulse/releases/download/{tag}/{file}') for file in files])
            (response / 'release.json').write_text(json.dumps(release))
            transport = root / 'bin'
            transport.mkdir()
            # This stand-in exists only on the disposable test PATH. Production
            # has no configurable repository, mirror or skip-validation switch.
            curl = transport / 'curl'
            curl.write_text('#!' + sys.executable + '\n' + '''import os, pathlib, shutil, sys
args=sys.argv[1:]; url=args[-1]; dest=pathlib.Path(args[args.index('--output')+1])
root=pathlib.Path(os.environ['TASKPULSE_TEST_RESPONSE'])
if url==os.environ['TASKPULSE_TEST_API']: file='release.json'
elif url.startswith(os.environ['TASKPULSE_TEST_ASSETS']):
    file=url[len(os.environ['TASKPULSE_TEST_ASSETS']):]
    if '/' in file or file not in os.environ['TASKPULSE_TEST_FILES'].split('|'): raise SystemExit('Unexpected asset')
else: raise SystemExit('Unexpected URL')
shutil.copyfile(root/file, dest)
''')
            curl.chmod(0o755)
            home = root / 'home'
            home.mkdir()
            state = home / '.taskpulse'
            state.mkdir()
            (state / 'marker').write_text('preserve-user-state')
            env = dict(os.environ, HOME=str(home), TMPDIR=str(root), TASKPULSE_HOME=str(state),
                       PATH=str(transport) + ':/usr/bin:/bin:/usr/sbin:/sbin',
                       TASKPULSE_TEST_RESPONSE=str(response), TASKPULSE_TEST_API=f'https://api.github.com/repos/qingtian-4/TaskPulse/releases/tags/{tag}',
                       TASKPULSE_TEST_ASSETS=f'https://github.com/qingtian-4/TaskPulse/releases/download/{tag}/', TASKPULSE_TEST_FILES='|'.join(files))

            def run(label, *args, success=True):
                child = subprocess.run(['/bin/bash', str(script), '--version', version, '--language', 'en', *args],
                                       cwd=root, env=env, capture_output=True, text=True, timeout=120)
                (evidence / f'{label}.txt').write_text(child.stdout + child.stderr)
                if (child.returncode == 0) != success:
                    raise RuntimeError(f'{label}: unexpected exit {child.returncode}: {child.stderr}')
                result['checks'].append(label)
                return child

            # The actual production script must reject the unchanged preview.
            run('preview-rejected', '--download-only', success=False)
            manifest.update(build_kind='release', tag=tag)
            (response / files[-1]).write_text(json.dumps(manifest))
            run('download-only', '--download-only', '--output-dir', './下载 with spaces')
            downloaded = root / '下载 with spaces' / f'TaskPulse-{tag}-macos-{arch}' / name
            if digest(downloaded) != result['package_sha256']:
                raise RuntimeError('Download changed package')
            run('first-install', '--no-launch')
            target = home / 'Applications/TaskPulse.app'
            info = plistlib.loads((target / 'Contents/Info.plist').read_bytes())
            exe = target / 'Contents/MacOS' / info['CFBundleExecutable']
            result['application_sha256'] = digest(exe)
            quarantine = subprocess.run(['xattr', '-p', 'com.apple.quarantine', str(target)], check=True, capture_output=True, text=True, timeout=10).stdout.strip()
            if not quarantine: raise RuntimeError('Missing installed quarantine')
            assessment = subprocess.run(['spctl', '--assess', '--type', 'execute', '--verbose=4', str(target)], capture_output=True, text=True, timeout=60)
            (evidence / 'gatekeeper-assessment.txt').write_text(assessment.stdout + assessment.stderr)
            result['gatekeeper_assessment_exit'] = assessment.returncode
            result['download_quarantine_verified'] = True
            if assessment.returncode == 0 or 'rejected' not in assessment.stderr.lower():
                raise RuntimeError('Expected explicit Gatekeeper rejection for the ad-hoc unnotarized package')
            result['checks'].append('quarantine-and-gatekeeper-rejection')
            run('same-version', '--no-launch')
            # Simulated older metadata exercises replacement/rollback with the
            # real signed incoming DMG; it is not cross-version upgrade evidence.
            info['CFBundleShortVersionString'] = '0.0.0'
            old_info = plistlib.dumps(info)
            (target / 'Contents/Info.plist').write_bytes(old_info)
            opened = transport / 'open'
            opened.write_text('#!/bin/sh\necho "Synthetic launch failure" >&2\nexit 1\n')
            opened.chmod(0o755)
            run('replacement-rollback-simulated', success=False)
            if (target / 'Contents/Info.plist').read_bytes() != old_info or digest(exe) != result['application_sha256']:
                raise RuntimeError('Rollback did not preserve old app')
            run('replacement-simulated', '--no-launch')
            if (state / 'marker').read_text() != 'preserve-user-state':
                raise RuntimeError('User state changed')
            result['checks'].append('state-preserved')
            result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
