"""Reopening with kept work and saved per-file work: the top-bar action, the
changed-file error with 다시 열기, what a reload keeps, Cmd+R on a rewritten
file, work-states.json, restoring after closing and after a restart,
restoring filters only for a changed file, pasted text, the confirmation
before a paste replaces an open log (synthetic paste events; the native
Cmd+V menu and Finder file paste cannot be driven), and clearing the recent
list (grow.log)."""
import json
import time

import fixtures
from fixtures import grow_row as row

VIEW = """const v=document.getElementById('logsViewport'),top=v.getBoundingClientRect().top,rows=[...document.querySelectorAll('#logsRows .log-row')];
const first=rows.find(r=>r.getBoundingClientRect().top>=top-1);const b=document.getElementById('reopenFile');return {
 count:document.getElementById('logsCount').textContent,range:document.getElementById('logsRange').textContent,status:document.getElementById('largeStatus').textContent,
 tag:document.getElementById('tagFilter').value,query:document.getElementById('searchInput').value,
 raw:document.getElementById('largeRaw').getAttribute('aria-pressed'),tab:document.getElementById('bookmarkTab').getAttribute('aria-selected'),
 marks:document.getElementById('bookmarkTab').textContent,top:first?.dataset.lineNumber??null,selected:document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null,
 button:{hidden:b.hidden,disabled:b.disabled,title:b.title},error:document.getElementById('errorBar').hidden?null:document.getElementById('largeError').textContent,
 errorReopen:!document.getElementById('errorReopen').hidden,toast:document.getElementById('toast').hidden?null:document.getElementById('toast').textContent,marker:window.__marker??null};"""
SCROLL = "const v=document.getElementById('logsViewport');v.scrollTop=arguments[0];v.dispatchEvent(new Event('scroll'));return true;"
# A paste event as WebKit delivers it outside text fields, with text only.
PASTE = """const text=arguments[0],e=new Event('paste',{bubbles:true,cancelable:true});
Object.defineProperty(e,'clipboardData',{value:{files:[],items:[],types:['text/plain'],getData:t=>t==='text/plain'?text:''}});
document.body.dispatchEvent(e);return e.defaultPrevented;"""
# The paste dialog, and how often it was shown (counted from the first use).
DIALOG = """const d=document.getElementById('pasteDialog');if(!d.__counted){d.__counted=true;window.__asked=0;const show=d.showModal.bind(d);d.showModal=()=>{window.__asked++;show();};}
return {open:d.open,what:document.getElementById('pasteWhat').textContent,note:document.getElementById('pasteNote').textContent,
 focus:document.activeElement?.id??null,asked:window.__asked};"""


def run(ctx):
    log = fixtures.grow(ctx.fixtures)
    store = ctx.data / 'work-states.json'
    check = ctx.check
    with ctx.launch() as app:
        js = app.js
        st = lambda: js(VIEW)
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(st()), timeout, what=what)

        def scroll(px):
            js(SCROLL, px)
            time.sleep(.3)
        js("window.__marker='page-1';return true;")
        app.open_fixture('grow.log')
        app.wait_count(3000)
        time.sleep(.3)
        s = st()
        check('the top bar shows an enabled 다시 열기 for a file', not s['button']['hidden'] and not s['button']['disabled'] and '⌘R' in s['button']['title'], s['button'])
        app.capture('01-topbar-reopen', hover='reopenFile')

        # Work to keep: tag filter, search, raw view, two bookmarks, the bookmark tab, scroll and selection.
        app.enter('tagFilter', 'Wifi')
        app.wait_count(400)
        app.enter('searchInput', 'timeout=3')
        app.click('largeRaw')
        app.click_row(23)
        app.key('b', meta=True)
        app.click_row(53)
        app.key('b', meta=True)
        app.click('bookmarkTab')
        scroll(28 * 120)
        app.click_row(js("const v=document.getElementById('logsViewport'),top=v.getBoundingClientRect().top;return [...document.querySelectorAll('#logsRows .log-row')].filter(r=>r.getBoundingClientRect().top>=top-1)[3].dataset.lineNumber"))
        before = st()

        # Rewriting read bytes changes the file (rows only appended would be
        # announced as new rows); the next read reports it with the action. Row
        # 0 becomes a raw row outside the Wifi filter, so 480 rows match after.
        text = log.read_text(encoding='utf-8')
        log.write_text('X' + text[1:] + ''.join(row(i) + '\n' for i in range(3000, 3600)), encoding='utf-8')
        scroll(28 * 300)
        e = until(lambda v: v['error'], 'changed-file error')
        check('a changed file shows the error with 다시 열기', '변경' in e['error'] and e['errorReopen'], {'error': e['error']})
        scroll(28 * 120)
        app.capture('02-error-bar', hover='errorReopen')
        started = time.monotonic()
        app.click('errorReopen')
        a = until(lambda v: v['count'] == '로그 · 전체 480건' and v['status'].startswith('파일 3,600행') and v['top'] == before['top'] and v['selected'] == before['selected'], 'reloaded')
        keys = ('tag', 'query', 'raw', 'tab', 'marks')
        check('a reload keeps filters, search, view, lower tab, bookmarks, top row and selection',
              [a[k] for k in keys] == [before[k] for k in keys] and a['error'] is None and a['marker'] == 'page-1',
              {'before': {k: before[k] for k in keys}, 'after': {k: a[k] for k in keys}, 'seconds': round(time.monotonic() - started, 3)})
        check('the reload says what it kept', a['toast'] == '파일을 다시 열었습니다. 필터·검색·북마크·위치를 유지합니다.', a['toast'])
        app.capture('03-reloaded-toast')
        time.sleep(2.8)

        # A rewritten, shorter file: Cmd+R; one mark is past the end, two rows changed.
        app.click('logsLast')
        time.sleep(.5)
        app.click_row(js("return [...document.querySelectorAll('#logsRows .log-row')].at(-1).dataset.lineNumber"))
        app.key('b', meta=True)
        time.sleep(.2)
        fixtures.grow(ctx.fixtures, 2400, 'v2-')
        app.key('r', meta=True)
        b = until(lambda v: v['status'].startswith('파일 2,400행') and '북마크 1개 해제·2개 내용 바뀜' in (v['toast'] or '') and v['count'] == '로그 · 전체 320건', 'Cmd+R reload')
        check('Cmd+R reloads; marks past the end drop, changed rows are reported, a lost place shows the list end',
              b['marks'] == '북마크 2' and b['marker'] == 'page-1' and b['error'] is None and b['range'].endswith('–320 / 320건') and not b['range'].startswith('1–'), b)
        app.capture('04-reload-bookmarks-toast')

        # Closing saves the work; the store holds paths and state, no log text.
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        saved = app.wait(lambda: store.exists() and json.loads(store.read_text()), what='work-states.json')
        text = store.read_text()
        first = saved[0]
        check('work-states.json holds the canonical path, stamps and state but no log text',
              first['files'][0]['path'] == str(log.resolve()) and first['files'][0]['size'] == log.stat().st_size and first['state']['filters']['include'][0]['tag'] == 'Wifi'
              and first['state']['bookmarks'] == [23, 53] and 'synthetic message' not in text and '화면 준비' not in text, first)
        js("document.querySelector('#recentList .recent-open').click();return true;")
        r = until(lambda v: v['marks'] == '북마크 2' and (v['toast'] or '').startswith('지난번'), 'restored')
        check('the same file reopened from the recent list restores its work',
              r['tag'] == 'Wifi' and r['query'] == 'timeout=3' and r['toast'].startswith('지난번 작업을 복원했습니다') and '북마크 2개' in r['toast'], r)
        app.capture('05-restored-toast')

    with ctx.launch() as app:
        js = app.js
        st = lambda: js(VIEW)
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(st()), timeout, what=what)
        app.open_fixture('grow.log')
        n = until(lambda v: v['marks'] == '북마크 2' and v['toast'], 'restored after restart')
        check('saved work survives an app restart', n['tag'] == 'Wifi' and n['query'] == 'timeout=3', n)
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        with log.open('a', encoding='utf-8') as f:
            f.write(row(2400) + '\n')
        app.open_fixture('grow.log')
        c = until(lambda v: v['status'].startswith('파일 2,401행') and '필터와 검색만' in (v['toast'] or ''), 'changed restore')
        check('a changed file restores filters and search only, and says so',
              c['tag'] == 'Wifi' and c['query'] == 'timeout=3' and c['tab'] == 'false' and '필터와 검색만 복원' in c['toast'] and app.hidden('lowerTabs'), c)
        app.capture('06-changed-toast')

        # A paste over the open file asks first: 취소, Escape and the
        # backdrop keep the file; 열기 replaces it.
        dialog = lambda: js(DIALOG)
        asking = lambda what: app.wait(lambda: (lambda d: d['open'] and d)(dialog()), what=what)
        dialog()
        js("document.getElementById('largeRaw').focus();return true;")
        check('a paste outside text fields is taken by the page', js(PASTE, 'plain\nsecond\n'), None)
        d = asking('paste dialog over a file')
        check('a paste over an open file asks first, says what opens and that its work is kept, with 열기 focused',
              d['what'] == '로그 텍스트 2줄 · 13 B' and d['note'] == '열려 있는 로그는 닫힙니다. 필터·검색·북마크는 저장되어 같은 파일을 다시 열면 복원됩니다.'
              and d['focus'] == 'pasteOpen' and d['asked'] == 1, d)
        app.capture('08-paste-confirm-file')
        app.click('pasteCancel')
        time.sleep(.3)
        k, d = st(), dialog()
        check('취소 keeps the open file and returns focus', not d['open'] and d['focus'] == 'largeRaw' and k['count'] == c['count']
              and k['status'].startswith('파일 2,401행') and not k['button']['disabled'], {'dialog': d, 'count': k['count']})
        js(PASTE, 'plain\nsecond\n')
        asking('paste dialog again')
        app.key('Escape', target='pasteDialog')
        time.sleep(.2)
        js(PASTE, 'plain\nsecond\n')
        asking('paste dialog for the backdrop')
        js("const d=document.getElementById('pasteDialog');d.dispatchEvent(new MouseEvent('click',{bubbles:true}));return true;")
        time.sleep(.3)
        k, d = st(), dialog()
        check('Escape and the backdrop keep the open file too', not d['open'] and d['asked'] == 3 and k['count'] == c['count'] and k['status'].startswith('파일 2,401행'),
              {'dialog': d, 'count': k['count']})
        js(PASTE, 'plain\nsecond\n')
        asking('paste dialog to open')
        app.click('pasteOpen')
        p = until(lambda v: v['count'] == '로그 · 전체 2건' and v['button']['disabled'], 'pasted text')
        check('열기 replaces the file with the pasted text', not dialog()['open'] and p['tag'] == '' and p['marks'] != '북마크 2', p)
        check('pasted text disables 다시 열기 with a reason', not p['button']['hidden'] and '다시 열 수 없습니다' in p['button']['title'], p['button'])
        app.capture('07-reopen-disabled', hover='reopenFile')

        # Over pasted text the dialog says it cannot be opened again; a
        # second paste while asking replaces the offer, and one opens.
        js(PASTE, 'x\ny\n')
        d = asking('paste dialog over pasted text')
        check('over pasted text the dialog says the log cannot be opened again', d['what'] == '로그 텍스트 2줄 · 4 B' and d['note'] == '열려 있는 로그는 닫히며 다시 열 수 없습니다.', d)
        app.capture('09-paste-confirm-text')
        js(PASTE, 'a\nb\nc\n')
        d = app.wait(lambda: (lambda d: d['what'] == '로그 텍스트 3줄 · 6 B' and d)(dialog()), what='replaced offer')
        check('a second paste while asking replaces the offer in the same dialog', d['open'] and d['asked'] == 5 and d['focus'] == 'pasteOpen', d)
        app.click('pasteOpen')
        until(lambda v: v['count'] == '로그 · 전체 3건', 'second pasted text')
        check('only the newer paste opens', not dialog()['open'], st()['count'])

        # With nothing open a paste opens at once.
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        js(PASTE, '06-18 19:24:24.983  2983 4241 D SyntheticExample: pasted\n')
        e = until(lambda v: v['count'] == '로그 · 전체 1건', 'pasted into the empty view')
        check('the empty view opens a paste without asking', dialog()['asked'] == 5 and not dialog()['open'], e['count'])

        # The paste commands keep the grant rules: an unknown grant cannot
        # be held or opened, discarding it is harmless, and actions are fixed.
        js("""window.__grants=null;const i=(c,a)=>window.__TAURI__.core.invoke(c,a).then(v=>({ok:v}),e=>({error:String(e)}));
Promise.all([i('hold_file_paste',{grant:'native-paste-999'}),i('finish_file_paste',{grant:'native-paste-999',action:'open'}),
 i('finish_file_paste',{grant:'native-paste-999',action:'discard'}),i('finish_file_paste',{grant:'native-paste-999',action:'other'})]).then(r=>{window.__grants=r;});return true;""")
        r = app.wait(lambda: js("return window.__grants"), what='paste command replies')
        unavailable = ('Paste grant is unavailable', 'Paste requires the main window')
        check('an unknown grant cannot be held or opened; discard is harmless; other actions are refused',
              r[0].get('error') in unavailable and r[1].get('error') in unavailable and r[2] == {'ok': {'ignored': True}} and r[3] == {'error': 'Invalid paste action'}, r)
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        app.click('clearRecent')
        check('목록 지우기 also clears saved work', app.wait(lambda: json.loads(store.read_text()) == [], what='cleared store'), store.read_text())
