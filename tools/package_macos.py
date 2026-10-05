#!/usr/bin/env python3
"""Build a self-contained native .app and ZIP. No microphone, input, or network activation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=['aarch64-apple-darwin', 'x86_64-apple-darwin'],
                        default='aarch64-apple-darwin' if os.uname().machine == 'arm64' else 'x86_64-apple-darwin')
    parser.add_argument('--identity', default='-', help='codesign identity; default is local ad-hoc signing')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    os.chdir(root)
    env = dict(os.environ, MACOSX_DEPLOYMENT_TARGET='13.0')
    build = subprocess.Popen(['cargo', 'build', '--release', '--locked', '--target', args.target,
                              '--message-format=json-render-diagnostics'], env=env, stdout=subprocess.PIPE, text=True)
    binary = None
    adapter = None
    for line in build.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            print(line, end='')
            continue
        if message.get('reason') == 'compiler-artifact' and message.get('executable'):
            binary = Path(message['executable'])
        if message.get('reason') == 'build-script-executed' and message.get('package_id', '').split('#')[-1].startswith('dictation-hotkey-native'):
            candidate = Path(message['out_dir']) / 'libDictationMac.dylib'
            if candidate.exists():
                adapter = candidate
    if build.wait() or not binary or not adapter:
        sys.exit('Build failed or macOS artifacts missing')
    arch = 'arm64' if args.target.startswith('aarch64') else 'x64'
    directory = root / 'dist' / f'macos-{arch}'
    app = directory / 'Dictation Hotkey.app'
    if app.exists():
        sys.exit(f'{app} already exists; move it aside before packaging again')
    contents = app / 'Contents'
    (contents / 'MacOS').mkdir(parents=True)
    (contents / 'Frameworks').mkdir()
    executable = contents / 'MacOS' / 'DictationHotkey'
    library = contents / 'Frameworks' / 'libDictationMac.dylib'
    shutil.copy2(binary, executable)
    shutil.copy2(adapter, library)
    shutil.copy2(root / 'resources/macos/Info.plist', contents / 'Info.plist')
    # Distribution must resolve the adapter inside the bundle, not the developer's Cargo directory.
    subprocess.run(['install_name_tool', '-delete_rpath', str(adapter.parent), str(executable)], check=True)
    subprocess.run(['codesign', '--force', '--sign', args.identity, str(library)], check=True)
    subprocess.run(['codesign', '--force', '--sign', args.identity, '--options', 'runtime',
                    '--entitlements', str(root / 'resources/macos/Entitlements.plist'), str(app)], check=True)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    archive = root / 'dist' / f'DictationHotkey-macos-{arch}.zip'
    subprocess.run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(app), str(archive)], check=True)
    hasher = hashlib.sha256()
    with archive.open('rb') as stream:
        for block in iter(lambda: stream.read(65536), b''):
            hasher.update(block)
    digest = hasher.hexdigest()
    (root / 'dist' / f'SHA256SUMS-macos-{arch}.txt').write_text(f'{digest}  {archive.name}\n')
    print(app)
    print(archive)


if __name__ == '__main__':
    main()
