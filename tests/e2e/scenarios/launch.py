"""Command-line paths: a plain path opens (and is recorded, followed and
reopened), several paths combine, a missing path and removed options show
the launch error; recent files reopen and drop missing entries."""
import json
import time

import fixtures

STATE = """const e=document.getElementById('largeError'),f=document.getElementById('followFile'),r=document.getElementById('reopenFile');return {
  count:document.getElementById('logsCount').textContent,name:document.getElementById('largeName').textContent,status:document.getElementById('largeStatus').textContent,
  error:e.hidden?null:e.textContent,rows:document.querySelectorAll('#logsRows .log-row').length,
  follow:{hidden:f.hidden,disabled:f.disabled},reopen:{hidden:r.hidden,disabled:r.disabled}};"""


def run(ctx):
    fixtures.startup(ctx.fixtures)
    fixtures.combined(ctx.fixtures)
    log = ctx.fixtures / 'startup.log'
    check = ctx.check
    recent = ctx.data / 'recent-files.json'

    with ctx.launch(log) as app:
        st = lambda: app.js(STATE)
        s = app.wait(lambda: (lambda v: v['count'] == '로그 · 전체 5건' and v['status'] == '' and v)(st()), what='opened path')
        check('a plain path argument opens the file without an error', s['rows'] > 0 and s['error'] is None, s)
        check('a single opened file can be followed and reopened', not s['follow']['hidden'] and not s['follow']['disabled'] and not s['reopen']['hidden'] and not s['reopen']['disabled'], s)
        app.wait(lambda: not app.hidden('recentButton'), 5, what='recent button')
        stored = json.loads(recent.read_text())
        check('the validated path is recorded natively, under the temporary HOME', [e['path'] for e in stored] == [str(log)], stored)
        app.capture('01-path-opened')
        with open(log, 'a', encoding='utf-8') as f:
            f.write(''.join(f"06-18 19:01:00.{n:03}  11 21 W App: appended {n}\n" for n in range(10)))
        app.click('reopenFile')
        s = app.wait(lambda: (lambda v: v['count'] == '로그 · 전체 15건' and not v['reopen']['disabled'] and v)(st()), what='reopened')
        check('다시 열기 reads the same file again', s['error'] is None, s)
        app.click('recentButton')
        items = app.js("return [...document.querySelectorAll('#rowMenu [role=menuitem]')].map(b=>b.textContent)")
        check('the recent-files menu lists the file and the clear action', items[:1] == ['startup.log'] and '목록 지우기' in items, items)
        app.key('Escape', target='rowMenu')
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState') and not app.hidden('recentFiles'), what='empty state with recent files')
        listed = app.js("return [...document.querySelectorAll('#recentList .recent-open')].map(e=>e.textContent)")
        check('the empty state lists the recent file with its folder', len(listed) == 1 and listed[0].startswith('startup.log') and ctx.fixtures.name in listed[0], listed)
        app.capture('02-recent-files')
        app.js("document.querySelector('#recentList .recent-open').click();return true;")
        app.wait(lambda: app.count() == '로그 · 전체 15건' and app.text('largeName') == 'startup.log', what='recent reopened')
        check('clicking a recent file reopens it', True)
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        moved = log.with_suffix('.moved')
        log.rename(moved)
        app.js("document.querySelector('#recentList .recent-open').click();return true;")
        gone = app.wait(lambda: app.js("return document.getElementById('recentFiles').hidden&&!document.getElementById('largeError').hidden&&document.getElementById('largeError').textContent"), what='missing file error')
        check('a missing recent file is reported and dropped from the list', '찾을 수 없습니다' in gone and json.loads(recent.read_text()) == [], gone)
        moved.rename(log)

    first, second = ctx.fixtures / 'first.log', ctx.fixtures / 'second.log'
    with ctx.launch(first, second) as app:
        s = app.wait(lambda: (lambda v: v['count'] == '로그 · 전체 4건' and v)(app.js(STATE)), what='combined paths')
        check('several path arguments open as one combined log in the given order', s['name'] == 'first.log 외 1개' and s['error'] is None, s)

    with ctx.launch(ctx.fixtures / 'missing.log') as app:
        error = app.wait(lambda: app.js(STATE)['error'], 10, what='launch error')
        check('a missing startup file is reported even before the page listens', '찾을 수 없습니다' in error, error)

    for args in (['--source', 'ram', log], ['--ram-source', log], ['--ram-budget-mib', '64', log]):
        with ctx.launch(*args) as app:
            s = app.wait(lambda: (lambda v: v['error'] and v)(app.js(STATE)), 10, what='launch error')
            name = ' '.join(map(str, args[:-1]))
            check(f'removed option {name}: the launch error lists the supported options and nothing opens',
                  s['error'] == '실행 인자를 확인해 주세요. 로그 파일 경로나 --large-log 파일 경로만 받습니다.' and s['rows'] == 0 and s['reopen']['hidden'], s)
            if args[0] == '--source':
                app.capture('03-removed-option')
        time.sleep(.2)
