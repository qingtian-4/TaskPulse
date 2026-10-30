#!/usr/bin/env python3
"""Offline system-tool doubles. Only tests prepend this directory to PATH."""
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import time

name = Path(sys.argv[0]).name
args = sys.argv[1:]
root = Path(os.environ['TP_FIXTURE'])
options = json.loads((root / 'options.json').read_text())
with (root / 'calls.jsonl').open('a') as journal:
    journal.write(json.dumps([name, args]) + '\n')

def option(key, default=None):
    return options.get(key, default)

if name == 'uname':
    print(option('os', 'Darwin') if args == ['-s'] else option('machine', 'arm64'))
elif name == 'sw_vers':
    print(option('system_version', '15.0'))
elif name == 'sysctl':
    value = option('translated', '0') if args[-1] == 'sysctl.proc_translated' else option('arm', '1')
    if args[-1] == 'sysctl.proc_translated' and option('translated_error'):
        print(option('translated_error'), file=sys.stderr)
        sys.exit(1)
    if value is None:
        print("sysctl: unknown oid '" + args[-1] + "'", file=sys.stderr)
        sys.exit(1)
    print(value)
elif name == 'plutil':
    file = Path(args[-1])
    try:
        data = plistlib.loads(file.read_bytes()) if file.suffix == '.plist' else json.loads(file.read_text())
        for key in args[args.index('-extract') + 1].split('.'):
            data = data[int(key)] if isinstance(data, list) else data[key]
        dest = args[args.index('-o') + 1]
        mode = args[args.index('-extract') + 2]
        if mode == 'json':
            out = json.dumps(data)
        elif data is True:
            out = 'true'
        elif data is False:
            out = 'false'
        elif data is None:
            out = 'null'
        else:
            out = str(data)
        if dest == '-':
            print(out)
        else:
            Path(dest).write_text(out)
    except (OSError, ValueError, KeyError, IndexError):
        if option('missing_key_diagnostic'):
            print('Could not extract value, error: No value at that key path.')
        sys.exit(1)
elif name == 'curl':
    dest = Path(args[args.index('--output') + 1])
    url = args[-1]
    if option('interrupt') and url.endswith('.dmg'):
        dest.write_bytes(b'partial')
        sys.exit(28)
    if option('delay') and url.endswith('.dmg'):
        time.sleep(option('delay'))
    file = root / 'assets' / url.rsplit('/', 1)[-1] if '/download/' in url else root / ('releases.json' if '?per_page' in url else 'release.json')
    shutil.copyfile(file, dest)
elif name == 'hdiutil':
    if args[0] == 'attach':
        mount = Path(args[args.index('-mountpoint') + 1])
        shutil.copytree(root / 'dmg-app', mount / 'TaskPulse.app', symlinks=True)
        if option('extra_app'):
            (mount / 'Other.app').mkdir()
    else:
        if option('detach_fail'):
            sys.exit(1)
        for entry in Path(args[1]).iterdir():
            if entry.is_dir(): shutil.rmtree(entry)
            else: entry.unlink()
elif name == 'ditto':
    shutil.copytree(args[0], args[1], symlinks=True)
elif name == 'codesign':
    if option('bad_signature'):
        sys.exit(1)
    if option('fail_installed') and args[-1] == str(root / 'home/Applications/TaskPulse.app'):
        sys.exit(1)
elif name == 'od':
    print(option('binary_header', 'cf fa ed fe ' + ('0c 00 00 01' if option('binary_arch', 'arm64') == 'arm64' else '07 00 00 01')))
elif name == 'lipo':
    print(option('binary_arch', 'arm64'))
elif name == 'ps':
    if option('ps_fail'):
        sys.exit(1)
    if option('running'):
        print('123 ' + str(root / 'home/Applications/TaskPulse.app/Contents/MacOS/agent-tray'))
elif name == 'open':
    sys.exit(1 if option('launch_fail') else 0)
elif name == 'xattr':
    # Portable test metadata follows inode identity across mv. Native acceptance
    # uses the real macOS xattr command, never this offline stand-in.
    path = args[-1]
    value = os.stat(path, follow_symlinks=False)
    key = f'{value.st_dev}:{value.st_ino}'
    database = root / 'quarantine.json'
    attributes = json.loads(database.read_text()) if database.exists() else {}
    if args[0] == '-p':
        if key not in attributes: sys.exit(1)
        print(attributes[key])
    else:
        attributes[key] = args[-2]
        database.write_text(json.dumps(attributes))
elif name == 'stat':
    value = os.stat(args[-1], follow_symlinks=False)
    print(f'{value.st_dev}:{value.st_ino}')
elif name == 'uuidgen':
    print('00000000-0000-4000-8000-000000000000')
elif name == 'mv':
    if option('fail_move') and args[-2].endswith('/TaskPulse.app') and '/.taskpulse-stage.' in args[-2]:
        sys.exit(1)
    if (option('publish_race') or option('publish_collision')) and 'TaskPulse-v' in args[-1]:
        Path(args[-1]).mkdir()
        (Path(args[-1]) / 'external.txt').write_text('preserve')
        if option('publish_collision'):
            collision = Path(args[-1]) / Path(args[-2]).name
            collision.mkdir()
            (collision / 'external-important.txt').write_text('preserve collision')
    if option('install_race') and args[-1].endswith('/Applications/TaskPulse.app') and '/.taskpulse-stage.' in args[-2]:
        Path(args[-1]).mkdir()
        (Path(args[-1]) / 'external.txt').write_text('preserve app')
    sys.exit(subprocess.run(['/bin/mv', *args]).returncode)
else:
    raise RuntimeError(name)
