"""Consolidate already verified release builds. Never tag or publish a release."""
import argparse
import hashlib
import json
import re
import shutil
import zipfile
from pathlib import Path


TARGETS = {'windows-x64': 'x86_64-pc-windows-msvc',
           'windows-arm64': 'aarch64-pc-windows-msvc',
           'macos-arm64': 'aarch64-apple-darwin', 'macos-x64': 'x86_64-apple-darwin'}
NOTICES = ('DEPENDENCIES.json', 'SBOM.cdx.json', 'THIRD_PARTY_NOTICES.txt')


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def package_release(source, output, version, commit, license_file):
    if not re.fullmatch(r'\d+\.\d+\.\d+', version) or not re.fullmatch(r'[a-f0-9]{40}', commit):
        raise ValueError('Exact stable version and full SHA required')
    compact = tuple(map(int, version.split('.'))) > (1, 1, 0)
    platforms = ['windows-x64', 'macos-arm64', 'macos-x64'] + (['windows-arm64'] if compact else [])
    metadata, payloads, material = {}, {}, {}

    def file(name):
        if not re.fullmatch(r'[A-Za-z0-9_.-]+', name):
            raise ValueError('Invalid asset path')
        path = source / name
        if path.is_symlink() or not path.is_file():
            raise ValueError('Missing or non-regular asset: ' + name)
        return path

    def load(name):
        return json.loads(file(name).read_text(encoding='utf-8-sig'))

    def checksums(name, expected):
        rows = file(name).read_text(encoding='utf-8-sig').splitlines()
        found = {}
        for row in rows:
            match = re.fullmatch(r'([a-f0-9]{64})  ([A-Za-z0-9_.-]+)', row)
            if not match or match[2] in found:
                raise ValueError('Invalid/duplicate checksum')
            found[match[2]] = match[1]
        if set(found) != set(expected) or any(digest(file(n)) != h for n, h in found.items()):
            raise ValueError('Checksum coverage or payload mismatch')

    for platform in platforms:
        windows = platform.startswith('windows')
        prefix = '' if platform == 'windows-x64' else platform + '-'
        manifest_name = prefix + ('release-manifest.json' if windows else 'manifest.json')
        manifest = load(manifest_name)
        arch = platform.split('-')[1]
        if (manifest.get('version') != version or manifest.get('commit') != commit
                or manifest.get('tag', 'v' + version) != 'v' + version
                or manifest.get('build_kind') not in (None, 'release')
                or manifest.get('platform') != ('windows' if windows else 'macos')
                or manifest.get('architecture') != arch):
            raise ValueError('Platform identity mismatch: ' + platform)
        names = ([f'TaskPulse_{version}_windows_{arch}_setup.exe']
                 + ([f'TaskPulse_{version}_windows_x64.msi'] if arch == 'x64' else [])) if windows else [f'TaskPulse_{version}_macos_{arch}.dmg']
        if manifest.get('assets') != names:
            raise ValueError('Unexpected installer coverage')
        checksums(prefix + 'SHA256SUMS.txt', names)
        if not windows and manifest.get('sha256') != digest(file(names[0])):
            raise ValueError('DMG digest mismatch')
        metadata[platform] = manifest
        for name in names:
            payloads[name] = {'platform': platform, 'kind': 'installer', 'sha256': digest(file(name))}
        for suffix in NOTICES:
            name = prefix + suffix
            material[platform + '/' + suffix] = file(name)
        inventory = load(prefix + 'DEPENDENCIES.json')
        sbom = load(prefix + 'SBOM.cdx.json')
        props = {p['name']: p['value'] for p in sbom.get('metadata', {}).get('properties', [])}
        if (inventory.get('commit') != commit or inventory.get('target') != TARGETS[platform]
                or props.get('taskpulse:source-commit') != commit or props.get('taskpulse:target') != TARGETS[platform]
                or sbom.get('metadata', {}).get('component', {}).get('version') != version):
            raise ValueError('Dependency provenance mismatch: ' + platform)
        material['metadata/' + manifest_name] = file(manifest_name)
        material['metadata/' + prefix + 'SHA256SUMS.txt'] = file(prefix + 'SHA256SUMS.txt')

    feed = load('latest.json')
    expected_targets = {'windows-x86_64', 'windows-x86_64-nsis', 'windows-x86_64-msi', 'darwin-aarch64', 'darwin-x86_64'}
    if compact:
        expected_targets |= {'windows-aarch64', 'windows-aarch64-nsis'}
    if feed.get('version') != version or set(feed.get('platforms', {})) != expected_targets:
        raise ValueError('Incomplete update feed')
    # The native verifier in each mandatory platform build checks the embedded
    # public key. Here bind its signed bytes and sidecars to the final feed.
    for lane in ['windows', 'macos-arm64', 'macos-x64'] + (['windows-arm64'] if compact else []):
        update = load('updater-' + lane + '.json')
        targets = {'windows-x86_64', 'windows-x86_64-nsis', 'windows-x86_64-msi'} if lane == 'windows' else ({'windows-aarch64', 'windows-aarch64-nsis'} if lane == 'windows-arm64' else {'darwin-aarch64' if lane == 'macos-arm64' else 'darwin-x86_64'})
        if update.get('version') != version or update.get('commit') != commit or set(update.get('platforms', {})) != targets:
            raise ValueError('Update provenance mismatch')
        for target, entry in update['platforms'].items():
            entry_name = entry['url'].rsplit('/', 1)[-1]
            expected_url = f'https://github.com/qingtian-4/TaskPulse/releases/download/v{version}/{entry_name}'
            signature = file(entry_name + '.sig').read_text(encoding='utf-8').strip()
            if (entry['url'] != expected_url or entry['sha256'] != digest(file(entry_name))
                    or entry['signature'] != signature or not signature
                    or feed['platforms'].get(target) != {'url': expected_url, 'signature': signature}):
                raise ValueError('Update bytes/signature mismatch')
            if entry_name not in payloads:
                payloads[entry_name] = {'platform': lane, 'kind': 'update', 'sha256': entry['sha256']}
            material['signatures/' + entry_name + '.sig'] = file(entry_name + '.sig')
    if output.exists() and any(output.iterdir()):
        raise ValueError('Output directory must be empty')
    output.mkdir(parents=True, exist_ok=True)
    if not compact:
        # Preserve every legacy public asset, including standalone .sig, MSI,
        # per-platform manifests/checksums and dependency files.
        names = set(payloads) | {p.name for p in material.values()} | {'latest.json'}
        for name in sorted(names):
            shutil.copyfile(file(name), output / name)
        return sorted(names)
    for name in sorted(payloads):
        shutil.copyfile(file(name), output / name)
    shutil.copyfile(file('latest.json'), output / 'latest.json')
    archive = f'TaskPulse_{version}_dependency-licenses.zip'
    with zipfile.ZipFile(output / archive, 'x', compression=zipfile.ZIP_DEFLATED) as bundle:
        for name, path in sorted(material.items()):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(info, path.read_bytes())
        bundle.writestr('LICENSE', license_file.read_bytes())
        bundle.writestr('provenance.json', json.dumps({'version': version, 'commit': commit, 'platforms': platforms}, sort_keys=True) + '\n')
    names = sorted(payloads) + ['latest.json', archive]
    assets = {name: digest(output / name) for name in names}
    unified = {'format': 'taskpulse-release-v2', 'version': version, 'tag': 'v' + version,
               'commit': commit, 'build_kind': 'release', 'platforms': {}, 'assets': assets,
               'compatibility': {'windows_x64_msi': 'Retained for existing installed updater clients'}}
    for name, record in metadata.items():
        system, arch = name.split('-')
        unified['platforms'].setdefault(system, {})[arch] = record
    (output / 'release-manifest.json').write_text(json.dumps(unified, indent=2) + '\n', encoding='utf-8')
    names.append('release-manifest.json')
    (output / 'SHA256SUMS.txt').write_text(''.join(f'{digest(output / name)}  {name}\n' for name in sorted(names)), encoding='utf-8')
    names.append('SHA256SUMS.txt')
    if len(names) != 11:
        raise ValueError('Unexpected compact asset count')
    return sorted(names)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--version', required=True)
    parser.add_argument('--commit', required=True)
    args = parser.parse_args()
    print(json.dumps(package_release(args.source, args.out, args.version, args.commit, Path(__file__).resolve().parent.parent / 'LICENSE')))
