"""Tip reset checks on synthetic WebKit localStorage stores."""
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'desktop'))
import reset_tips_macos as tips


def store(path, items):
    """A localStorage database as WebKit writes it: UTF-16 values in WAL mode."""
    path.parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(path)
    db.execute('PRAGMA journal_mode=WAL')
    db.execute('CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB NOT NULL ON CONFLICT FAIL)')
    db.executemany('INSERT INTO ItemTable VALUES (?, ?)', [(key, value.encode('utf-16-le')) for key, value in items.items()])
    db.commit()
    return db


def items(path):
    with sqlite3.connect(path) as db:
        return {key: value.decode('utf-16-le') for key, value in db.execute('SELECT key, value FROM ItemTable')}


def lsof(code, out=''):
    return lambda args, **kw: subprocess.CompletedProcess(args, code, stdout=out)


class ResetTipsTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='LogStudio home ')
        self.addCleanup(directory.cleanup)
        self.home = Path(directory.name)
        webkit = self.home / 'Library/WebKit'
        self.app = webkit / 'dev.logstudio.desktop/WebsiteData/Default/salt/salt/LocalStorage/localstorage.sqlite3'
        self.binary = webkit / 'logstudio-desktop/WebsiteData/LocalStorage/tauri_localhost_0.localstorage'
        for path, saved in [(self.app, {'logstudio.zoom.v1': '{"level":1.1}', tips.KEY: '{"off":true,"seen":["help"]}'}),
                            (self.binary, {'logstudio.layout.v1': '{}'})]:
            store(path, saved).close()
        # Another app's store is never touched.
        store(webkit / 'other.app/WebsiteData/Default/s/s/LocalStorage/localstorage.sqlite3', {tips.KEY: '{}'}).close()

    def test_only_the_tips_entry_is_removed_from_each_logstudio_store(self):
        with patch.object(tips.subprocess, 'run', lsof(1)):
            found, cleared = tips.reset(self.home)
        self.assertEqual(found, [self.app, self.binary])
        self.assertEqual(cleared, [self.app])
        self.assertEqual(items(self.app), {'logstudio.zoom.v1': '{"level":1.1}'})
        self.assertEqual(items(self.binary), {'logstudio.layout.v1': '{}'})
        with patch.object(tips.subprocess, 'run', lsof(1)):
            self.assertEqual(tips.reset(self.home)[1], [], 'a second run has nothing to clear')

    def test_a_running_app_or_unknown_state_changes_nothing(self):
        for run in [lsof(0, '4242\n'), lambda *a, **kw: (_ for _ in ()).throw(OSError('no lsof'))]:
            with patch.object(tips.subprocess, 'run', run), self.assertRaisesRegex(ValueError, 'Quit LogStudio first'):
                tips.reset(self.home)
            self.assertIn(tips.KEY, items(self.app))

    def test_no_store_means_nothing_to_clear(self):
        with patch.object(tips.subprocess, 'run', lsof(1)):
            self.assertEqual(tips.reset(self.home / 'empty'), ([], []))

    @unittest.skipUnless(sys.platform == 'darwin', 'lsof as macOS ships it')
    def test_lsof_sees_a_store_held_open(self):
        held = sqlite3.connect(self.app)
        held.execute('SELECT count(*) FROM ItemTable').fetchone()
        try:
            with self.assertRaisesRegex(ValueError, str(self.app)):
                tips.reset(self.home)
        finally:
            held.close()
        self.assertEqual(tips.reset(self.home)[1], [self.app])


if __name__ == '__main__':
    unittest.main()
