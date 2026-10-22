"""Build, safely package, and launch a separate local macOS app."""
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import uuid

import package_macos

MARKER = '.logstudio-build-run'
OWNER = 'LogStudio macOS build-run v1\n'


def check_owned(output):
    if output.is_symlink():
        raise ValueError('Local output is a symlink; refusing replacement: ' + str(output))
    if output.exists():
        marker = output / MARKER
        if (not output.is_dir() or marker.is_symlink() or not marker.is_file()
                or marker.read_text() != OWNER):
            raise ValueError('Local output was not created by this script; preserve it and choose another workflow: ' + str(output))


def in_use(app):
    """Whether a running process has this bundle's executable open. Matches
    the file itself, so a bundle renamed after launch still counts; unknown
    (lsof failing) counts as in use."""
    executable = app / 'Contents/MacOS/logstudio-desktop'
    if not executable.exists():
        return False
    try:
        result = subprocess.run(['lsof', '-t', str(executable)], text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    except OSError:
        return True
    return result.returncode != 1 or bool(result.stdout.strip())


def prune_backups(parent, keep):
    """Removes local app backups other than `keep` that this script created
    and no running process uses, so backups do not pile up."""
    for backup in sorted(parent.glob('LogStudio-local-backup-*.app')):
        if backup == keep:
            continue
        try:
            check_owned(backup)
        except ValueError:
            continue
        if in_use(backup):
            print('Older local app still running; kept: ' + str(backup), flush=True)
            continue
        try:
            shutil.rmtree(backup)
            print('Removed older local app: ' + str(backup), flush=True)
        except OSError as error:
            print('Could not remove older local app ' + str(backup) + ': ' + str(error), file=sys.stderr)


def build_binary(root, env):
    try:
        version = subprocess.run(['rustc', '-vV'], cwd=root, env=env,
                                 check=True, text=True, stdout=subprocess.PIPE).stdout
    except (OSError, subprocess.CalledProcessError):
        raise ValueError('The Rust toolchain pinned in rust-toolchain.toml is not installed; '
                         'install rustup and run `rustup toolchain install` in this folder')
    hosts = [line[6:] for line in version.splitlines() if line.startswith('host: ')]
    if len(hosts) != 1 or not hosts[0].endswith('-apple-darwin'):
        raise ValueError('A native macOS Rust toolchain is required')
    command = ['cargo', 'build', '--locked', '--release', '-p', 'logstudio-desktop',
               '--target', hosts[0], '--message-format=json-render-diagnostics']
    print('Building normal Release app (no qa/WebDriver)...', flush=True)
    artifacts = set()
    manifest = (root / 'src-tauri/Cargo.toml').resolve()
    with subprocess.Popen(command, cwd=root, env=env, text=True,
                          stdout=subprocess.PIPE) as process:
        for line in process.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print(line, end='')
                continue
            if message.get('reason') == 'compiler-message':
                rendered = message.get('message', {}).get('rendered')
                if rendered:
                    print(rendered, end='', file=sys.stderr)
            if message.get('reason') != 'compiler-artifact':
                continue
            target = message.get('target', {})
            if (target.get('name') != 'logstudio-desktop' or 'bin' not in target.get('kind', [])
                    or Path(message.get('manifest_path', '')).resolve() != manifest):
                continue
            if 'qa' in message.get('features', []):
                raise ValueError('Refusing to package a qa/WebDriver executable')
            if message.get('executable'):
                binary = Path(message['executable'])
                if not binary.is_absolute():
                    binary = root / binary
                artifacts.add(binary.resolve())
        if process.wait() != 0:
            raise ValueError('Cargo build failed; existing apps were preserved and nothing was launched')
    if len(artifacts) != 1:
        raise ValueError('Cargo did not report exactly one desktop executable; nothing was launched')
    binary = artifacts.pop()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('Reported Cargo executable is missing or not executable: ' + str(binary))
    return binary


def build_and_run(root):
    if platform.system() != 'Darwin':
        raise ValueError('This build script requires macOS')
    root = root.resolve()
    parent = root / 'artifacts/desktop'
    output = parent / 'LogStudio-local.app'
    parent.mkdir(parents=True, exist_ok=True)
    lock = parent / '.logstudio-local-build.lock'
    try:
        lock.mkdir()
    except FileExistsError:
        raise ValueError('Another local build is active (or an interrupted run left the lock): ' + str(lock))
    try:
        check_owned(output)
        env = os.environ.copy()
        env['RUSTUP_AUTO_INSTALL'] = '0'
        env.setdefault('MACOSX_DEPLOYMENT_TARGET', '12.0')
        binary = build_binary(root, env)
        with tempfile.TemporaryDirectory(prefix='.logstudio-local-stage-', dir=parent) as directory:
            staged = Path(directory) / output.name
            package_macos.package(binary, staged)
            (staged / MARKER).write_text(OWNER)
            check_owned(output)
            backup = None
            if output.exists():
                backup = parent / ('LogStudio-local-backup-' + uuid.uuid4().hex + '.app')
                output.rename(backup)
            try:
                staged.rename(output)
            except OSError:
                if backup is not None:
                    backup.rename(output)
                raise
        print('App: ' + str(output), flush=True)
        if backup is not None:
            print('Previous local app preserved: ' + str(backup), flush=True)
        # Only the previous app is kept as a backup; older ones go unless running.
        prune_backups(parent, backup)
        # -n opens this newly built bundle even when another LogStudio is running.
        # No running process is quit or killed.
        subprocess.run(['open', '-n', str(output)], check=True)
        print('Launch requested successfully.', flush=True)
        return output
    finally:
        lock.rmdir()


if __name__ == '__main__':
    try:
        if len(sys.argv) != 1:
            raise ValueError('Usage: sh build-run-macos.command (no arguments)')
        build_and_run(Path(__file__).resolve().parents[1])
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print('LogStudio build-run: ' + str(error), file=sys.stderr)
        sys.exit(1)
