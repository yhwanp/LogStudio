"""Local build workflow checks with fake Cargo and native launch calls."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'desktop'))
import build_run_macos as workflow

REAL_RUN = subprocess.run
REAL_POPEN = subprocess.Popen


class FakeCargo:
    def __init__(self, messages, code=0):
        self.stdout = io.StringIO(''.join(json.dumps(m) + '\n' for m in messages))
        self.code = code

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def wait(self):
        return self.code


class BuildRunTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='LogStudio path with spaces ')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        (self.root / 'src-tauri').mkdir()
        (self.root / 'src-tauri/Cargo.toml').write_text('synthetic')
        self.binary = self.root / 'custom target/aarch64-apple-darwin/release/logstudio-desktop'
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b'new binary')
        self.binary.chmod(0o755)
        self.output = self.root / 'artifacts/desktop/LogStudio-local.app'
        self.events = []
        self.messages = [{
            'reason': 'compiler-artifact', 'manifest_path': str(self.root / 'src-tauri/Cargo.toml'),
            'target': {'name': 'logstudio-desktop', 'kind': ['bin']},
            'features': ['default', 'custom-protocol'], 'executable': str(self.binary),
        }]
        self.build_code = 0
        self.launch_failure = False
        self.running = []
        self.addCleanup(patch.stopall)
        patch.object(workflow.platform, 'system', return_value='Darwin').start()
        patch.object(workflow.subprocess, 'run', side_effect=self.run_command).start()
        patch.object(workflow.subprocess, 'Popen', side_effect=self.cargo).start()
        package = workflow.package_macos.package
        def package_with_event(binary, output):
            self.events.append(('package', binary))
            return package(binary, output)
        patch.object(workflow.package_macos, 'package', side_effect=package_with_event).start()

    def run_command(self, command, **kwargs):
        if command[0] == 'lsof':
            running = any(command[2].startswith(str(p) + '/') for p in self.running)
            return subprocess.CompletedProcess(command, 0 if running else 1, '123\n' if running else '')
        if command == ['rustc', '-vV']:
            self.assertEqual(kwargs['env']['RUSTUP_AUTO_INSTALL'], '0')
            return subprocess.CompletedProcess(command, 0, 'host: aarch64-apple-darwin\n')
        self.events.append(('launch', command))
        self.assertEqual(command, ['open', '-n', str(self.output)])
        self.assertTrue((self.output / 'Contents/MacOS/logstudio-desktop').is_file())
        if self.launch_failure:
            raise subprocess.CalledProcessError(1, command)
        return subprocess.CompletedProcess(command, 0)

    def cargo(self, command, **kwargs):
        self.events.append(('build', command))
        self.assertEqual(kwargs['cwd'], self.root)
        self.assertEqual(command, ['cargo', 'build', '--locked', '--release', '-p',
                                  'logstudio-desktop', '--target', 'aarch64-apple-darwin',
                                  '--message-format=json-render-diagnostics'])
        self.assertNotIn('qa', command)
        self.assertEqual(kwargs['env'].get('CARGO_TARGET_DIR'), 'custom target directory')
        return FakeCargo(self.messages, self.build_code)

    def build(self):
        with patch.dict(os.environ, {'CARGO_TARGET_DIR': 'custom target directory'}):
            return workflow.build_and_run(self.root)

    def old_output(self):
        self.output.mkdir(parents=True)
        (self.output / workflow.MARKER).write_text(workflow.OWNER)
        (self.output / 'old-data').write_bytes(b'previous app')

    def test_shell_entry_from_other_directory_with_spaces(self):
        entry = Path(__file__).resolve().parents[1] / 'build-run-macos.command'
        copied = self.root / entry.name
        copied.write_text(entry.read_text())
        desktop = self.root / 'desktop'
        desktop.mkdir()
        (desktop / 'build_run_macos.py').write_text(
            'from pathlib import Path; print(Path(__file__).resolve().parents[1])')
        with patch.object(workflow.subprocess, 'Popen', REAL_POPEN):
            result = REAL_RUN(['sh', str(copied)], cwd='/', check=True,
                              text=True, stdout=subprocess.PIPE)
        self.assertEqual(result.stdout.strip(), str(self.root))

    def test_sequence_override_path_spaces_and_production_preserved(self):
        production = self.output.with_name('LogStudio.app')
        production.mkdir(parents=True)
        (production / 'untouched').write_text('production')
        self.assertEqual(self.build(), self.output)
        self.assertEqual([e[0] for e in self.events], ['build', 'package', 'launch'])
        self.assertEqual(self.events[1][1], self.binary.resolve())
        self.assertEqual((production / 'untouched').read_text(), 'production')
        self.assertFalse((self.output.parent / '.logstudio-local-build.lock').exists())

    def backups(self):
        return list(self.output.parent.glob('LogStudio-local-backup-*.app'))

    def test_repeat_keeps_only_the_previous_output(self):
        self.old_output()
        self.build()
        first = self.backups()
        self.assertEqual(len(first), 1)
        self.assertTrue((first[0] / 'old-data').exists())
        self.binary.write_bytes(b'second binary')
        self.build()
        self.binary.write_bytes(b'third binary')
        self.build()
        backups = self.backups()
        self.assertEqual(len(backups), 1)
        self.assertEqual((backups[0] / 'Contents/MacOS/logstudio-desktop').read_bytes(), b'second binary')
        self.assertEqual((self.output / 'Contents/MacOS/logstudio-desktop').read_bytes(), b'third binary')

    def test_running_or_foreign_backups_are_kept(self):
        self.build()
        self.build()
        running = self.backups()[0]  # a built app, launched before it was renamed
        self.running = [running]
        foreign = self.output.parent / 'LogStudio-local-backup-foreign.app'
        foreign.mkdir()
        (foreign / 'mine').write_text('not created by the script')
        self.build()
        self.build()
        backups = self.backups()
        self.assertIn(running, backups)
        self.assertIn(foreign, backups)
        self.assertEqual(len(backups), 3)

    def test_failed_build_never_packages_or_launches_stale_artifact(self):
        self.old_output()
        self.build_code = 1  # Even a reported executable must not be used after failure.
        with self.assertRaisesRegex(ValueError, 'Cargo build failed'):
            self.build()
        self.assertEqual([e[0] for e in self.events], ['build'])
        self.assertEqual((self.output / 'old-data').read_bytes(), b'previous app')

    def test_failed_package_preserves_old_app_and_never_launches(self):
        self.old_output()
        with patch.object(workflow.package_macos, 'package', side_effect=ValueError('package failed')):
            with self.assertRaisesRegex(ValueError, 'package failed'):
                self.build()
        self.assertEqual([e[0] for e in self.events], ['build'])
        self.assertTrue((self.output / 'old-data').exists())
        self.assertEqual(list(self.output.parent.glob('.logstudio-local-stage-*')), [])

    def test_missing_ambiguous_or_qa_artifact_is_not_launched(self):
        valid = self.messages[0]
        for messages in [[], [dict(valid, executable=str(self.binary) + '-other')],
                         [valid, dict(valid, executable=str(self.binary) + '-other')],
                         [dict(valid, features=['qa'])],
                         [dict(valid, manifest_path=str(self.root / 'wrong/Cargo.toml'))]]:
            with self.subTest(messages=messages):
                self.events.clear()
                self.messages = messages
                with self.assertRaises(ValueError):
                    self.build()
                self.assertEqual([e[0] for e in self.events], ['build'])

    def test_foreign_output_and_symlink_are_preserved(self):
        self.output.mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, 'not created by this script'):
            self.build()
        self.output.rmdir()
        other = self.root / 'unrelated'
        other.mkdir()
        self.output.symlink_to(other)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.build()
        self.assertTrue(other.is_dir())
        self.assertEqual(self.events, [])

    def test_publish_failure_restores_previous_output(self):
        self.old_output()
        rename = Path.rename
        def failing_publish(path, destination):
            if path.parent.name.startswith('.logstudio-local-stage-'):
                raise OSError('publish failed')
            return rename(path, destination)
        with patch.object(Path, 'rename', failing_publish):
            with self.assertRaisesRegex(OSError, 'publish failed'):
                self.build()
        self.assertTrue((self.output / 'old-data').exists())
        self.assertEqual([e[0] for e in self.events], ['build', 'package'])

    def test_existing_lock_blocks_concurrent_run(self):
        lock = self.output.parent / '.logstudio-local-build.lock'
        lock.mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, 'Another local build'):
            self.build()
        self.assertTrue(lock.exists())
        self.assertEqual(self.events, [])

    def test_launch_failure_is_reported_and_preserves_backup(self):
        self.old_output()
        self.launch_failure = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.build()
        self.assertTrue(self.output.is_dir())
        self.assertEqual(len(list(self.output.parent.glob('LogStudio-local-backup-*.app'))), 1)


if __name__ == '__main__':
    unittest.main()
