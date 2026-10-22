"""Forget which first-use tips LogStudio has shown on this Mac, so each tip
shows once again; a tips setting turned off is turned back on.

    python3 desktop/reset_tips_macos.py

Only the page's `logstudio.tips.v1` localStorage entry is removed; zoom,
layout and the other settings stay. WebKit keeps a store per app: the
bundle ID for the app (installed or from build-run-macos.command) and the
executable name for a binary run directly (target/*/logstudio-desktop, also
the QA build). Both are cleared. Quit LogStudio first: WebKit holds the
store open while it runs, and nothing is changed then. macOS only.
"""
from contextlib import closing
from pathlib import Path
import platform
import sqlite3
import subprocess
import sys

KEY = 'logstudio.tips.v1'
STORES = ['dev.logstudio.desktop', 'logstudio-desktop']


def databases(home, stores=STORES):
    """The localStorage databases of the stores: one per page origin, in the
    current layout or the older one file per origin."""
    for name in stores:
        data = home / 'Library/WebKit' / name / 'WebsiteData'
        yield from sorted(data.glob('Default/*/*/LocalStorage/localstorage.sqlite3'))
        yield from sorted(data.glob('LocalStorage/*.localstorage'))


def in_use(path):
    """Whether a running process has the database open; unknown (lsof
    failing) counts as in use."""
    try:
        result = subprocess.run(['lsof', '-t', str(path)], text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    except OSError:
        return True
    return result.returncode != 1 or bool(result.stdout.strip())


def reset(home, stores=STORES):
    """Removes the tips entry from every store; returns the databases found
    and those that had one. Changes nothing while any of them is open."""
    found = list(databases(home, stores))
    busy = [path for path in found if in_use(path)]
    if busy:
        raise ValueError('Quit LogStudio first; its storage is open: ' + ', '.join(map(str, busy)))
    cleared = []
    for path in found:
        with closing(sqlite3.connect(path)) as db, db:
            if db.execute('DELETE FROM ItemTable WHERE key = ?', (KEY,)).rowcount:
                cleared.append(path)
    return found, cleared


if __name__ == '__main__':
    try:
        if len(sys.argv) != 1:
            raise ValueError('Usage: python3 desktop/reset_tips_macos.py (no arguments)')
        if platform.system() != 'Darwin':
            raise ValueError('This script clears macOS WebKit storage only')
        found, cleared = reset(Path.home())
        for path in found:
            print(('Cleared tips: ' if path in cleared else 'No tips recorded: ') + str(path))
        print('Each tip shows once again on the next launch.' if cleared else 'Nothing to clear.')
    except (OSError, ValueError, sqlite3.Error) as error:
        print('LogStudio reset-tips: ' + str(error), file=sys.stderr)
        sys.exit(1)
