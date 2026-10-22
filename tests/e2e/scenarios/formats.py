"""Recognized Android formats (Android Studio, -v time, uid, brief, year,
nsec) and iOS rows in columns, level counts, continuation and malformed rows,
the unrecognized-rows switch, tag filter and search marks, and the brief row
menu (formats.log)."""
import time

import fixtures

ROW = """const r=document.querySelector(`#logsRows [data-line-number="${arguments[0]}"]`);if(!r)return null;
const c=k=>r.querySelector(':scope > .'+k);const marks=e=>e?[...e.querySelectorAll('mark')].map(m=>m.textContent):[];
return {cls:r.className,stamp:c('timestamp')?.textContent??null,title:c('timestamp')?.title??null,pid:c('pid')?.textContent??null,
tid:c('tid')?.textContent??null,level:r.querySelector(':scope > .level')?.textContent??null,tag:c('tag')?.textContent??null,
message:c('message')?.textContent??null,stampMarks:marks(c('timestamp'))};"""
LEVELS = "return Object.fromEntries([...document.querySelectorAll('#levelFilters .level-chip')].map(b=>[b.dataset.level,b.querySelector('.count').textContent]))"


def run(ctx):
    fixtures.formats(ctx.fixtures)
    fixtures.startup(ctx.fixtures)
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        row = lambda n: js(ROW, n)
        app.open_fixture('formats.log')
        app.wait_count(16)
        lv = app.wait(lambda: (lambda v: '+' not in ''.join(v.values()) and v['I'] == '4' and v)(js(LEVELS)), what='level counts')
        check('level counts cover every format', lv == {'V': '1', 'D': '1', 'I': '4', 'W': '1', 'E': '2', 'F': '0', 'A': '0', 'U': '0'}, lv)
        time.sleep(.3)
        r6, r16, r2, r9, r10 = (row(n) for n in (6, 16, 2, 9, 10))
        check('Android Studio row: package after the timestamp, fields in columns',
              r6['stamp'] == '2026-06-18 19:24:03.000 · com.example.app' and r6['title'].endswith(' · com.example.app')
              and [r6['pid'], r6['tid'], r6['level'], r6['tag'], r6['message']] == ['2983', '4241', 'Warn', 'AlertsService', 'Studio 한글 🙂'] and 'raw-row' not in r6['cls'], r6)
        check('Android Studio row with a Unicode tag', r16['stamp'] == '2026-06-18 19:24:08.000 · com.example.app' and r16['tag'] == '알림🙂' and r16['level'] == 'Verbose', r16)
        check('-v time row', r2['stamp'] == '06-18 19:24:01.123' and [r2['pid'], r2['tid'], r2['level'], r2['tag'], r2['message']] == ['2983', '', 'Debug', 'AlertsService', 'Sending update'], r2)
        check('threadtime -v uid row', [r9['pid'], r9['tid'], r9['tag'], r9['message']] == ['2983', '4241', 'AlertsService', 'uid row'], r9)
        check('brief row has an empty timestamp', r10['stamp'].strip() == '' and [r10['pid'], r10['tid'], r10['level'], r10['tag']] == ['1000', '', 'Info', 'ActivityManager'], r10)
        fit = js("""const c=document.querySelector('#logsRows [data-line-number="6"] > .timestamp'),s=getComputedStyle(c),m=document.createElement('canvas').getContext('2d');
m.font=`${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;const pad=parseFloat(s.paddingLeft)+parseFloat(s.paddingRight);
return {cell:c.clientWidth,timeAndEllipsis:Math.ceil(m.measureText('2026-06-18 19:24:03.000 …').width+pad)};""")
        check('the date/time column fits the whole time and the ellipsis before the package', fit['cell'] >= fit['timeAndEllipsis'], fit)
        cont = {n: row(n)['cls'] for n in (3, 5, 7, 11, 12, 13)}
        check('continuation rows join their entry; malformed rows stay raw',
              all('continuation-row' in cont[n] for n in (3, 5, 7, 11)) and 'level-row-W' in cont[7] and 'raw-row' in cont[12], cont)
        app.capture('01-columns')
        app.click('includeRaw')
        app.wait_count(13)
        check('turning off 미인식 줄 포함 hides only the 3 orphan rows', row(12) is None and row(11) is not None, app.count())
        app.type('tagFilter', 'alerts')
        app.wait_count(5)
        lines = js("return [...document.querySelectorAll('#logsRows .log-row')].map(r=>Number(r.dataset.lineNumber))")
        check('a tag filter keeps the entries with their continuation rows', lines == [2, 3, 6, 7, 9], lines)
        app.type('tagFilter', '')
        app.type('searchInput', 'com.example')
        app.wait_count(13)
        hit = app.wait(lambda: (lambda v: v['stampMarks'] and v)(row(6)), what='package mark')
        check('a search match inside the package is marked in the timestamp cell', hit['stampMarks'] == ['com.example'], hit)
        app.capture('02-search-package')
        app.click_row(10)
        app.click_row(10, event='contextmenu')
        menu = app.wait(lambda: js("const m=document.getElementById('rowMenu');return m&&!m.hidden&&m.textContent"), what='row menu')
        check('the brief row menu offers tag, PID and level but no time or TID', 'ActivityManager' in menu and 'PID 1000' in menu and '부터 보기' not in menu and 'TID' not in menu, menu)

        # threadtime, -v year, -v nsec and iOS stamps; the stack line stays raw.
        app.key('Escape', target='rowMenu')
        app.open_fixture('startup.log')
        app.wait_count(5)
        stamps = js("return [...document.querySelectorAll('#logsRows .log-row .timestamp')].map(e=>e.textContent)")
        cls = js("return document.querySelector('#logsRows [data-line-number=\"5\"]').className")
        check('-v year, -v nsec and iOS rows get columns; the stack line continues the iOS entry',
              'continuation-row' in cls and '2026-09-21 12:07:31.386' in stamps and '09-21 12:07:32.386123456' in stamps and any(s.startswith('2026-09-30 21:55:53.776261') for s in stamps), {'stamps': stamps, 'row5': cls})
