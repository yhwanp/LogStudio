"""Several files as one log: the name and file list, file badges and
in-file rows, CRLF and unterminated file ends, search, detail and filters
across files, duplicate selections, and ordering picks by log time or name
(first/second/third.log, a/b/c.log, logcat_*.log)."""
import time

import fixtures

CELLS = "return [...document.querySelectorAll(arguments[0]+' .log-row')].map(r=>({badge:r.querySelector('.file-badge')?.textContent??null,line:r.querySelector('.line-number').textContent,start:r.classList.contains('file-start'),title:r.querySelector('.line-number').title}))"
HEADER = "return document.querySelector('#logsHeaders .column-heading').textContent"
ORDER = """const t=document.getElementById('toast');return {count:document.getElementById('logsCount').textContent,
  name:document.getElementById('largeName').textContent,title:document.getElementById('largeName').title,
  toast:t.hidden?null:t.textContent,rows:[...document.querySelectorAll('#logsRows .log-row')].map(r=>r.textContent)};"""


def run(ctx):
    fixtures.combined(ctx.fixtures)
    fixtures.order(ctx.fixtures)
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        app.open_fixtures(['second.log', 'first.log', 'third.log'])
        app.wait_count(5)
        s = js("const n=document.getElementById('largeName');return {name:n.textContent,title:n.title}")
        check('the top bar names the combined log and lists its files in order',
              s['name'] == 'second.log 외 2개' and '1. second.log' in s['title'] and '2. first.log' in s['title'] and '3. third.log' in s['title'], s)
        rows = js(CELLS, '#logsRows')
        check('rows show the file badge and in-file row; later files mark their start',
              [(r['badge'], r['line'][len(r['badge'] or ''):], r['start']) for r in rows] == [('1', '1', False), ('1', '2', False), ('2', '1', True), ('2', '2', False), ('3', '1', True)], rows)
        check('CRLF and an unterminated file end each close their rows',
              js("return [...document.querySelectorAll('#logsRows .log-row .message')].map(e=>e.textContent)") == ['second-1 WIFI', 'second-2', 'first-1', 'first-2', 'third-1 wifi'])
        check('the row column is labelled for files', js(HEADER) == '파일 · 행', js(HEADER))
        check('the row tooltip names the file and the combined row', rows[4]['title'] == 'third.log · 1행\n합친 로그 5행', rows[4]['title'])
        app.capture('01-combined')
        app.enter('searchInput', 'wifi')
        app.wait(lambda: app.text('largeCount') == '2건', what='search count')
        found = js(CELLS, '#largeRows')
        check('a search covers every file and keeps the file badges', [r['badge'] for r in found] == ['1', '3'], found)
        js("document.querySelectorAll('#largeRows .log-row')[1].click();return true;")
        app.wait(lambda: app.text('largeDetailTitle') == 'third.log · 1행 원문', what='detail title')
        check('the raw detail names its file', True)
        app.capture('02-search-detail')
        app.enter('searchInput', '')
        js("document.querySelector('#levelFilters [data-level=E]').dispatchEvent(new MouseEvent('click',{bubbles:true,altKey:true}));return true;")
        app.wait_count(2)
        check('a level filter spans the files', [r['badge'] for r in js(CELLS, '#logsRows')] == ['2', '3'])
        app.click('allLevels')
        app.wait_count(5)
        app.open_fixture('third.log')
        app.wait(lambda: app.text('largeName') == 'third.log' and app.count() == '로그 · 전체 1건', what='single file')
        rows = js(CELLS, '#logsRows')
        check('a single file shows plain row numbers', js(HEADER) == '행' and rows == [{'badge': None, 'line': '1', 'start': False, 'title': ''}], rows)
        js("window.__err=null;window.__TAURI__.core.invoke('qa_open_fixtures',{names:['first.log','first.log']}).catch(e=>window.__err=String(e));return true;")
        err = app.wait(lambda: js("return window.__err"), what='duplicate refusal')
        check('a duplicate selection is refused without replacing the open file', err == 'The same file was selected more than once' and app.text('largeName') == 'third.log', err)

        # Ordering: picks and drops (`arrange`) sort by log time, then by name.
        st = lambda: js(ORDER)
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        empty = js("return document.querySelector('#emptyState p').textContent")
        check('the empty state says files picked together combine in time order', '시간순으로 하나의 로그로' in empty, empty)

        def open_set(names, arrange, total):
            app.open_fixtures(names, arrange)
            return app.wait(lambda: (lambda v: v['count'].endswith(f'전체 {total}건') and v['rows'] and v)(st()), what=f'{names}')
        s = open_set(['a.log', 'b.log', 'c.log'], True, 5)
        app.capture('03-time-order-toast')
        order = [next(i for i, r in enumerate(s['rows']) if k in r) for k in ('c1', 'b1', 'a1')]
        check('picked newest first, the log opens the oldest file first', s['name'] == 'c.log 외 2개' and order == sorted(order), {'name': s['name'], 'rows': s['rows']})
        check('a toast announces the time order', s['toast'] == '3개 파일을 시간순으로 합쳤습니다.', s['toast'])
        check('the top-bar tooltip lists the combined order',
              s['title'].startswith('3개 파일을 이 순서로 합쳤습니다\n1. c.log') and '\n2. b.log' in s['title'] and '\n3. a.log' in s['title'], s['title'])
        time.sleep(2.8)
        check('the toast hides after a moment', st()['toast'] is None)
        s = open_set(['logcat_10.log', 'logcat_2.log'], True, 2)
        check('files without log times open in natural name order with a name-order toast',
              s['name'] == 'logcat_2.log 외 1개' and s['toast'] == '2개 파일을 이름순으로 합쳤습니다.' and 'two' in s['rows'][0], s)
        time.sleep(2.8)
        open_set(['c.log', 'b.log', 'a.log'], True, 5)
        time.sleep(.3)
        s = st()
        check('an order that already holds opens without a toast', s['name'] == 'c.log 외 2개' and s['toast'] is None, s)
        open_set(['a.log', 'b.log', 'c.log'], False, 5)
        time.sleep(.3)
        s = st()
        check('the given order (as command-line paths) is kept without a toast', s['name'] == 'a.log 외 2개' and s['toast'] is None and 'a1' in s['rows'][0], {'name': s['name'], 'toast': s['toast']})
        app.click('largeClose')
        recent = app.wait(lambda: js("return [...document.querySelectorAll('#recentList li')].map(li=>li.textContent)"), what='recent list')
        check('the recent list records the arranged combination', any(r.startswith('c.log 외 2개') for r in recent), recent)
        js("[...document.querySelectorAll('#recentList li')].find(li=>li.textContent.startsWith('c.log 외 2개')).querySelector('button').click();return true;")
        app.wait(lambda: st()['count'].endswith('전체 5건'), what='recent combination')
        time.sleep(.3)
        s = st()
        check('reopening the recent entry keeps its stored order without a toast', s['name'] == 'c.log 외 2개' and s['toast'] is None and 'beginning of main' in s['rows'][0], {'name': s['name'], 'toast': s['toast'], 'first': s['rows'][0]})
