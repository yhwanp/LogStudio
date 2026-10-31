"""Runs the real-WebView scenarios against a QA build (macOS).

    python3 tests/e2e/run.py <QA binary> [--out DIR] [--large] [--extras] [--foreground] [scenario ...]

Each scenario gets its own temporary HOME and fixture folder and launches
the app as often as it needs. Results (results.json) and screenshots (each
captured state in the light and the dark scheme) go to artifacts/e2e/, which
each run replaces and git ignores; --out keeps a run elsewhere (a new or
empty folder). The app runs in the background: it takes no focus and its
window stays below other windows; --foreground shows it as a person would.
Exits 1 if any check fails.
"""
import argparse
import importlib
import json
import shutil
import sys
import tempfile
import time
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from harness import App  # noqa: E402

# Run order: quick UI checks first, slow and window-moving ones last.
SCENARIOS = ['basics', 'time_filter', 'bookmarks', 'field_filters', 'saved_filters', 'conditions', 'filter_place', 'filters_off', 'formats', 'dlt', 'entries', 'tips', 'launch',
             'multi_file', 'follow', 'devices', 'reopen', 'zoom', 'window', 'row_numbers', 'extras']


class Context:
    def __init__(self, name, binary, out, results, background=True):
        self.name = name
        self.binary = binary
        self.background = background
        self.results = results
        self.home = Path(tempfile.mkdtemp(prefix=f'logstudio-e2e-{name}-home-'))
        self.fixtures = Path(tempfile.mkdtemp(prefix=f'logstudio-e2e-{name}-fixtures-'))
        self.shots = out / name
        self.shots.mkdir(parents=True, exist_ok=True)
        self.data = self.home / 'Library/Application Support/dev.logstudio.desktop'

    def launch(self, *args, clear_before=True, clear_after=True, tips=False):
        return App(self, args, clear_before, clear_after, tips)

    def check(self, name, ok, detail=None):
        self.results.append({'scenario': self.name, 'check': name, 'ok': bool(ok), 'detail': detail})
        text = '' if detail is None else ' :: ' + json.dumps(detail, ensure_ascii=False, default=str)[:300]
        print(f"{'PASS' if ok else 'FAIL'} {self.name}: {name}{text}", flush=True)
        return bool(ok)

    def cleanup(self):
        shutil.rmtree(self.home, ignore_errors=True)
        shutil.rmtree(self.fixtures, ignore_errors=True)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('binary', type=Path, help='QA build, e.g. target/qa/release/logstudio-desktop')
    p.add_argument('scenarios', nargs='*', help=f'subset of: {", ".join(SCENARIOS)}')
    p.add_argument('--out', type=Path, help='keep results and screenshots in this new or empty folder (default: replace artifacts/e2e)')
    p.add_argument('--large', action='store_true', help='include scenarios that write ~400 MB fixtures')
    p.add_argument('--extras', action='store_true', help='include scenarios of app extensions (a build with --features qa,extras)')
    p.add_argument('--foreground', action='store_true', help='let the app activate and its window come to the front (default: background)')
    a = p.parse_args()
    binary = a.binary.resolve()
    if not binary.is_file():
        p.error(f'{binary} not found; build it with --features qa (see tests/e2e/README.md)')
    names = a.scenarios or SCENARIOS
    unknown = [n for n in names if n not in SCENARIOS]
    if unknown:
        p.error(f'unknown scenarios: {unknown}')
    if a.out:
        out = a.out.resolve()
        if out.exists() and any(out.iterdir()):
            p.error(f'{out} is not empty; pick a new folder')
    else:
        out = (HERE.parent.parent / 'artifacts' / 'e2e').resolve()
        shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True, exist_ok=True)
    results = []
    for name in names:
        module = importlib.import_module(f'scenarios.{name}')
        if getattr(module, 'LARGE', False) and not a.large and name not in a.scenarios:
            print(f'skip {name}: needs --large', flush=True)
            continue
        if getattr(module, 'EXTRAS', False) and not a.extras and name not in a.scenarios:
            print(f'skip {name}: needs --extras', flush=True)
            continue
        ctx = Context(name, binary, out, results, background=not a.foreground)
        started = time.monotonic()
        try:
            module.run(ctx)
        except Exception as e:
            ctx.check('scenario finished without an exception', False, f'{type(e).__name__}: {e}')
            traceback.print_exc()
        finally:
            ctx.cleanup()
        print(f'-- {name}: {time.monotonic() - started:.1f}s', flush=True)
    (out / 'results.json').write_text(json.dumps(results, ensure_ascii=False, indent=1, default=str) + '\n', encoding='utf-8')
    failed = [f"{r['scenario']}: {r['check']}" for r in results if not r['ok']]
    print(f'{len(results) - len(failed)}/{len(results)} passed; results and screenshots in {out}')
    for f in failed:
        print(f'  FAIL {f}')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
