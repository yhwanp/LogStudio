"""Download integrity and target checks; no Python is bundled in the product."""
import hashlib
import pathlib
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'desktop'))
import package_download


class DownloadTests(unittest.TestCase):
    def test_windows_archive_contains_exact_binary_license_notices_and_checksum(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            header = bytearray(64)
            header[:2] = b'MZ'
            struct.pack_into('<I', header, 60, 64)
            data = bytes(header) + b'PE\0\0' + struct.pack('<H', 0x8664) + b'synthetic'
            binary = root / 'app.exe'
            binary.write_bytes(data)
            with patch.object(package_download.platform, 'system', return_value='Windows'):
                archive = package_download.package(binary, 'x86_64-pc-windows-msvc', root / 'out')
                with self.assertRaisesRegex(ValueError, 'already exists'):
                    package_download.package(binary, 'x86_64-pc-windows-msvc', root / 'out')
            with zipfile.ZipFile(archive) as file:
                self.assertEqual(set(file.namelist()),
                                 {'LogStudio.exe', 'README.txt', 'LICENSE.txt', 'THIRD_PARTY_NOTICES.md'})
                self.assertEqual(file.read('LogStudio.exe'), data)
                self.assertEqual(file.read('LICENSE.txt'), (package_download.ROOT / 'LICENSE').read_bytes())
                self.assertEqual(file.read('THIRD_PARTY_NOTICES.md'),
                                 (package_download.ROOT / 'desktop/THIRD_PARTY_NOTICES.md').read_bytes())
            expected = '{}  {}\n'.format(hashlib.sha256(archive.read_bytes()).hexdigest(), archive.name)
            self.assertEqual(archive.with_suffix('.zip.sha256').read_text(), expected)

    def test_wrong_architecture_and_non_executables_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = pathlib.Path(directory) / 'app'
            binary.write_bytes(b'not an executable')
            with self.assertRaisesRegex(ValueError, 'PE executable'):
                package_download.verify_binary(binary, 'x86_64-pc-windows-msvc')
            binary.write_bytes(b'\xcf\xfa\xed\xfe' + struct.pack('<I', 0x0100000C))
            package_download.verify_binary(binary, 'aarch64-apple-darwin')
            binary.write_bytes(b'\xcf\xfa\xed\xfe' + struct.pack('<I', 0x01000007))
            with self.assertRaisesRegex(ValueError, 'architecture'):
                package_download.verify_binary(binary, 'aarch64-apple-darwin')


if __name__ == '__main__':
    unittest.main()
