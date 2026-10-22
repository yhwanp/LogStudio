"""Package a release build of LogStudio into an unsigned macOS app bundle.
Build first with cargo build --locked --release -p logstudio-desktop (without qa).
"""
import argparse
import json
import pathlib
import platform
import plistlib
import shutil


def package(binary, output):
    if platform.system() != 'Darwin':
        raise ValueError('macOS app packaging requires a native Mac build')
    if not binary.is_file():
        raise ValueError('Build the production executable first')
    notices = pathlib.Path(__file__).with_name('THIRD_PARTY_NOTICES.md')
    license_file = notices.parent.parent / 'LICENSE'
    icon = notices.parent.parent / 'src-tauri/icons/icon.icns'
    if not notices.is_file() or not license_file.is_file():
        raise ValueError('The license and production third-party notices are required')
    if not icon.is_file():
        raise ValueError('src-tauri/icons/icon.icns is missing; run desktop/make_icons.py')
    if output.exists():
        raise ValueError('Output exists; choose a new path to preserve its contents')
    config = json.loads((notices.parent.parent / 'src-tauri/tauri.conf.json').read_text())
    app = output / 'Contents'
    (app / 'MacOS').mkdir(parents=True)
    (app / 'Resources').mkdir()
    executable = app / 'MacOS/logstudio-desktop'
    shutil.copy2(binary, executable)
    executable.chmod(0o755)
    with (app / 'Info.plist').open('wb') as file:
        plistlib.dump({
            'CFBundleName': config['productName'],
            'CFBundleDisplayName': config['productName'],
            'CFBundleExecutable': 'logstudio-desktop',
            'CFBundleIconFile': 'icon.icns',
            'CFBundleIdentifier': config['identifier'],
            'CFBundlePackageType': 'APPL',
            'CFBundleShortVersionString': config['version'],
            'CFBundleVersion': config['version'],
            'LSMinimumSystemVersion': '12.0',
            'NSHighResolutionCapable': True,
            'NSHumanReadableCopyright': config['bundle']['copyright'],
        }, file)
    shutil.copy2(icon, app / 'Resources/icon.icns')
    shutil.copy2(license_file, app / 'Resources/LICENSE.txt')
    shutil.copy2(notices, app / 'Resources/THIRD_PARTY_NOTICES.md')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=pathlib.Path)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    args = parser.parse_args()
    try:
        package(args.binary, args.output)
    except ValueError as error:
        parser.error(str(error))
    print(args.output.resolve())
