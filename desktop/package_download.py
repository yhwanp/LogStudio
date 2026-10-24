"""Package a built native executable into the GitHub download ZIP and its checksum."""
import argparse
import hashlib
import json
import pathlib
import platform
import re
import shutil
import struct
import subprocess
import tempfile
import zipfile

import package_macos

ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGETS = {
    'x86_64-pc-windows-msvc': ('Windows', 'windows-x64', 0x8664),
    'aarch64-apple-darwin': ('Darwin', 'macos-arm64', 0x0100000C),
}


def version():
    value = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())['version']
    cargo = re.search(r'^version\s*=\s*"([^"]+)"',
                      (ROOT / 'src-tauri/Cargo.toml').read_text(), re.MULTILINE)
    frontend = json.loads((ROOT / 'package.json').read_text())['version']
    if cargo is None or cargo[1] != value or frontend != value:
        raise ValueError('Desktop and frontend package versions must agree')
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', value):
        raise ValueError('Unsupported package version')
    return value


def verify_binary(binary, target):
    with binary.open('rb') as file:
        header = file.read(64)
        if target == 'x86_64-pc-windows-msvc':
            if len(header) < 64 or header[:2] != b'MZ':
                raise ValueError('Expected a native Windows PE executable')
            file.seek(struct.unpack_from('<I', header, 60)[0])
            pe = file.read(6)
            if len(pe) != 6 or pe[:4] != b'PE\0\0':
                raise ValueError('Invalid Windows PE header')
            machine = struct.unpack_from('<H', pe, 4)[0]
        else:
            if len(header) < 8 or header[:4] != b'\xcf\xfa\xed\xfe':
                raise ValueError('Expected a native 64-bit Mach-O executable')
            machine = struct.unpack_from('<I', header, 4)[0]
        if machine != TARGETS[target][2]:
            raise ValueError('Executable architecture does not match the download target')


def package(binary, target, output):
    system, label, _ = TARGETS[target]
    if platform.system() != system:
        raise ValueError('Package on a native runner for the selected target')
    if not binary.is_file():
        raise ValueError('Build the production executable first')
    verify_binary(binary, target)
    name = 'LogStudio-{}-{}.zip'.format(version(), label)
    archive = output / name
    checksum = output / (name + '.sha256')
    if archive.exists() or checksum.exists():
        raise ValueError('Download already exists; choose a new output directory')
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as directory:
        payload = pathlib.Path(directory) / 'payload'
        payload.mkdir()
        if system == 'Darwin':
            app = payload / 'LogStudio.app'
            package_macos.package(binary, app)
            # Ad-hoc signing preserves a runnable ARM binary; it is not Developer ID signing.
            subprocess.run(['codesign', '--force', '--sign', '-', str(app)], check=True)
            subprocess.run(['codesign', '--verify', '--strict', str(app)], check=True)
            instructions = (
                'Move LogStudio.app to Applications and open it. macOS 12 or later on Apple Silicon is required.\n'
                'This app has no Developer ID signature or Apple notarization, so macOS blocks the first launch.\n'
                'Allow it once with Open Anyway in System Settings > Privacy & Security:\n'
                'https://support.apple.com/102445\n'
            )
        else:
            shutil.copy2(binary, payload / 'LogStudio.exe')
            shutil.copy2(ROOT / 'LICENSE', payload / 'LICENSE.txt')
            shutil.copy2(ROOT / 'desktop/THIRD_PARTY_NOTICES.md', payload)
            instructions = (
                'Extract all files to a folder and open LogStudio.exe. Windows 10/11 x64 is required.\n'
                'Microsoft Edge WebView2 Runtime is required (included on most Windows 11 PCs).\n'
                'If missing, install it from https://developer.microsoft.com/microsoft-edge/webview2/\n'
                'This executable has no publisher certificate; Windows may show a SmartScreen prompt.\n'
            )
        (payload / 'README.txt').write_text(instructions, encoding='utf-8')
        if system == 'Darwin':
            # Extended attributes such as com.apple.provenance would add __MACOSX entries;
            # the ad-hoc signature lives inside the bundle and does not need them.
            subprocess.run(['ditto', '-c', '-k', '--norsrc', '--noextattr', '--noacl',
                            str(payload), str(archive.resolve())], check=True)
        else:
            with zipfile.ZipFile(archive, 'x', zipfile.ZIP_DEFLATED) as file:
                for item in sorted(payload.iterdir()):
                    file.write(item, item.name)
    with zipfile.ZipFile(archive) as file:
        damaged = file.testzip()
        if damaged is not None:
            raise ValueError('Damaged archive member: ' + damaged)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum.write_text('{}  {}\n'.format(digest, name), encoding='utf-8')
    return archive


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=pathlib.Path)
    parser.add_argument('--target', required=True, choices=TARGETS)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    args = parser.parse_args()
    try:
        result = package(args.binary, args.target, args.output)
    except (ValueError, OSError) as error:
        parser.error(str(error))
    print(result.resolve())
