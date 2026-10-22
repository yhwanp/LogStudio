"""Maintainer-only: render src-tauri/icons from the two SVG sources on a Mac.

icon.svg is full-bleed square art; macOS masks it to its own icon shape, so
icon.icns comes from it. icon-rounded.svg carries the rounded shape for
Windows (icon.ico) and the window icon (icon.png). Needs Google Chrome for the
SVG rendering, and sips and iconutil from macOS.
"""
import argparse
import pathlib
import platform
import struct
import subprocess
import tempfile

ICONS = pathlib.Path(__file__).resolve().parent.parent / 'src-tauri/icons'
CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]


def render(chrome, svg, png):
    subprocess.run([chrome, '--headless=new', '--disable-gpu', '--hide-scrollbars',
                    '--force-device-scale-factor=1', '--window-size=1024,1024',
                    '--default-background-color=00000000', '--screenshot=' + str(png),
                    svg.as_uri()], check=True, capture_output=True)


def resize(source, size, target):
    subprocess.run(['sips', '-z', str(size), str(size), str(source), '--out', str(target)],
                   check=True, capture_output=True)


def main(chrome):
    if platform.system() != 'Darwin':
        raise SystemExit('Icon rendering uses macOS sips and iconutil')
    with tempfile.TemporaryDirectory() as directory:
        work = pathlib.Path(directory)
        square, rounded = work / 'square.png', work / 'rounded.png'
        render(chrome, ICONS / 'icon.svg', square)
        render(chrome, ICONS / 'icon-rounded.svg', rounded)
        iconset = work / 'icon.iconset'
        iconset.mkdir()
        for size in [16, 32, 128, 256, 512]:
            resize(square, size, iconset / f'icon_{size}x{size}.png')
            resize(square, size * 2, iconset / f'icon_{size}x{size}@2x.png')
        subprocess.run(['iconutil', '-c', 'icns', str(iconset), '-o', str(ICONS / 'icon.icns')], check=True)
        images = []
        for size in ICO_SIZES:
            resize(rounded, size, work / f'{size}.png')
            images.append((work / f'{size}.png').read_bytes())
        # ICO entries hold PNG data; a width or height of 0 means 256.
        offset = 6 + 16 * len(images)
        header = struct.pack('<HHH', 0, 1, len(images))
        for size, data in zip(ICO_SIZES, images):
            header += struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(data), offset)
            offset += len(data)
        (ICONS / 'icon.ico').write_bytes(header + b''.join(images))
        resize(rounded, 512, ICONS / 'icon.png')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--chrome', default=CHROME)
    main(parser.parse_args().chrome)
