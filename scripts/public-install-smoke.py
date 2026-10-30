"""Anonymous, real-network installation acceptance on disposable hosted runners."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import subprocess
import tempfile
import urllib.request

REPO = 'qingtian-4/TaskPulse'
ROOT = Path(__file__).resolve().parent.parent


def download(url, destination):
    request = urllib.request.Request(url, headers={'User-Agent': 'TaskPulse-public-install-acceptance', 'Accept': 'application/vnd.github+json'})
    with urllib.request.urlopen(request, timeout=120) as response, destination.open('wb') as output:
        while True:
            block = response.read(1024 * 1024)
            if not block:
                break
            output.write(block)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, log, env=None, timeout=180, input_data=None):
    child = subprocess.run(command, cwd=ROOT, env=env, input=input_data, capture_output=True, timeout=timeout)
    log.write_bytes(child.stdout + child.stderr)
    if child.returncode:
        raise RuntimeError(f'{command[0]} exited {child.returncode}; see {log.name}')
    return child.stdout


def validate_manifest(manifest, tag, commit, platform_name, architecture, package_name):
    if (manifest.get('version') != tag[1:] or manifest.get('commit') != commit
            or manifest.get('platform') != platform_name or manifest.get('architecture') != architecture
            or manifest.get('tag', tag) != tag or manifest.get('build_kind') not in (None, 'release')
            or package_name not in manifest.get('assets', [])):
        raise RuntimeError('Public package manifest does not match the checked release source/platform')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--gsudo')
    options = parser.parse_args()
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted' or platform.system() not in ('Windows', 'Darwin'):
        raise RuntimeError('Installation acceptance is limited to disposable Windows/macOS CI')
    if not re.fullmatch(r'v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', options.tag):
        raise ValueError('An explicit stable tag is required')
    out = options.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    version = options.tag[1:]
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    result = dict(passed=False, tag=options.tag, source_commit=commit, os=platform.system(),
                  architecture=platform.machine(), anonymous_public_download_verified=False,
                  os_trust_prompts_verified=False, checks=[])
    try:
        with tempfile.TemporaryDirectory(prefix='taskpulse-public-install-') as temporary:
            temporary = Path(temporary)
            api = temporary / 'release.json'
            download(f'https://api.github.com/repos/{REPO}/releases/latest', api)
            release = json.loads(api.read_bytes())
            if release.get('tag_name') != options.tag or release.get('draft') or release.get('prerelease') or not release.get('published_at'):
                raise RuntimeError('The default public stable release does not match the requested tag')

            def asset(name, destination):
                matches = [a for a in release['assets'] if a['name'] == name]
                expected = f'https://github.com/{REPO}/releases/download/{options.tag}/{name}'
                if len(matches) != 1 or matches[0]['browser_download_url'] != expected:
                    raise RuntimeError('Missing, duplicate or unexpected asset: ' + name)
                download(expected, destination)
                if matches[0].get('digest') and matches[0]['digest'] != 'sha256:' + digest(destination):
                    raise RuntimeError('GitHub asset digest mismatch: ' + name)

            windows = platform.system() == 'Windows'
            name = 'install.ps1' if windows else 'install.sh'
            bootstrap = temporary / name
            # This is exactly the floating entry point documented in README.
            download(f'https://raw.githubusercontent.com/{REPO}/main/{name}', bootstrap)
            if digest(bootstrap) != digest(ROOT / name):
                raise RuntimeError('README bootstrap changed since the checked release tag; verify the new script separately')
            result['script_sha256'] = digest(bootstrap)
            if windows:
                if not options.gsudo:
                    raise ValueError('--gsudo is required for medium-integrity Windows runtime acceptance')
                destination = temporary / 'downloads'
                run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(bootstrap),
                     '-DownloadOnly', '-OutputDirectory', str(destination), '-Language', 'en'], out / 'bootstrap.txt', timeout=600)
                assets = destination / ('TaskPulse-' + options.tag)
                manifest = json.loads((assets / 'release-manifest.json').read_text(encoding='utf-8-sig'))
                validate_manifest(manifest, options.tag, commit, 'windows', 'x64', f'TaskPulse_{version}_windows_x64_setup.exe')
                msi = f'TaskPulse_{version}_windows_x64.msi'
                asset(msi, assets / msi)
                # The existing native acceptance validates BOTH checksums and MSI
                # metadata before installing the exact downloaded NSIS package.
                env = dict(os.environ, TASKPULSE_PROBE_COMMIT=commit)
                run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                     str(ROOT / 'scripts/installer-smoke.ps1'), '-Assets', str(assets), '-Version', version,
                     '-Gsudo', options.gsudo, '-InstallerKind', 'nsis'], out / 'native-installer.txt', env, timeout=840)
                result['package_sha256'] = digest(assets / f'TaskPulse_{version}_windows_x64_setup.exe')
                result['checks'].append('public bootstrap download, actual NSIS install, native IPC, restart and uninstall')
            else:
                architecture = 'arm64' if platform.machine() == 'arm64' else 'x64'
                package_name = f'TaskPulse_{version}_macos_{architecture}.dmg'
                manifest_file = temporary / f'macos-{architecture}-manifest.json'
                asset(manifest_file.name, manifest_file)
                manifest = json.loads(manifest_file.read_bytes())
                validate_manifest(manifest, options.tag, commit, 'macos', architecture, package_name)
                image = temporary / package_name
                asset(package_name, image)
                if digest(image) != manifest.get('sha256'):
                    raise RuntimeError('Public DMG does not match its release manifest')
                result['package_sha256'] = digest(image)
                home = temporary / 'home'
                home.mkdir()
                state = home / '.taskpulse'
                state.mkdir()
                (state / 'marker').write_text('preserve-public-install-state')
                env = dict(os.environ, HOME=str(home), TASKPULSE_HOME=str(state), PATH='/usr/bin:/bin:/usr/sbin:/sbin')
                destination = temporary / 'Applications'
                run(['/bin/bash', '-s', '--', '--dir', str(destination), '--no-launch', '--language', 'en'],
                    out / 'bootstrap.txt', env, timeout=600, input_data=bootstrap.read_bytes())
                app = destination / 'TaskPulse.app'
                info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
                if info.get('CFBundleIdentifier') != 'app.taskpulse.desktop' or info.get('CFBundleShortVersionString') != version:
                    raise RuntimeError('Installed bundle identity/version mismatch')
                exe = app / 'Contents/MacOS' / info['CFBundleExecutable']
                if not exe.resolve().is_relative_to(app.resolve()):
                    raise RuntimeError('Installed executable escapes the application')
                run(['codesign', '--verify', '--deep', '--strict', str(app)], out / 'signature.txt')
                quarantine = run(['xattr', '-p', 'com.apple.quarantine', str(app)], out / 'quarantine.txt').strip()
                if not quarantine or (state / 'marker').read_text() != 'preserve-public-install-state':
                    raise RuntimeError('Install lost quarantine or user state')
                expected_arch = 'arm64' if platform.machine() == 'arm64' else 'x86_64'
                if run(['lipo', '-archs', str(exe)], out / 'architecture.txt').decode().strip() != expected_arch:
                    raise RuntimeError('Wrong installed architecture')
                assessment = subprocess.run(['spctl', '--assess', '--type', 'execute', '--verbose=4', str(app)], capture_output=True, timeout=60)
                (out / 'gatekeeper.txt').write_bytes(assessment.stdout + assessment.stderr)
                result['gatekeeper_assessment_exit'] = assessment.returncode
                mount = temporary / 'public-volume'
                mount.mkdir()
                run(['hdiutil', 'attach', str(image), '-readonly', '-nobrowse', '-mountpoint', str(mount)], out / 'mount.txt')
                try:
                    reference = mount / 'TaskPulse.app/Contents/MacOS' / info['CFBundleExecutable']
                    if digest(reference) != digest(exe):
                        raise RuntimeError('Installed application differs from the exact public DMG')
                finally:
                    run(['hdiutil', 'detach', str(mount)], out / 'detach.txt')
                result['application_sha256'] = digest(exe)
                result['checks'].append('public bootstrap download and install, correct bundle/architecture, strict signature, quarantine and state retained')
                # Native startup/restart of these DMG bytes is independently
                # verified by release.yml. This job never bypasses Gatekeeper.
                result['finder_launch_verified'] = False
                result['checks'].append('first Finder launch may require the documented macOS user confirmation')
            result['anonymous_public_download_verified'] = True
            result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
