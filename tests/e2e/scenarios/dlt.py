"""DLT files: their columns and level names, the filter fields they use, ID
and time filters, search marks in DLT cells, control, non-verbose and issue
rows, the row menu, the export format menu, no following, combined DLT files
in time order, the mixing error, the file named when one of several fails,
the reason a DLT file without storage headers does not open, a file written
by dlt-daemon, and text columns again for a text log (vehicle.dlt, early.dlt,
the COVESA samples, ui.log)."""
import time

import fixtures

ROW = """const r=document.querySelector(`#logsRows [data-line-number="${arguments[0]}"]`);if(!r)return null;
const c=k=>r.querySelector(':scope > .'+k);const marks=e=>e?[...e.querySelectorAll('mark')].map(m=>m.textContent):[];
const level=r.querySelector(':scope > .level');
return {cls:r.className,stamp:c('timestamp')?.textContent??null,uptime:c('uptime')?.textContent??null,ecu:c('ecu')?.textContent??null,
apid:c('apid')?.textContent??null,ctid:c('ctid')?.textContent??null,level:level?.textContent??null,levelTitle:level?.title??null,
message:c('message')?.textContent??null,raw:c('raw-content')?.textContent??null,marks:{message:marks(c('message')),apid:marks(c('apid'))}};"""
LEVELS = """return Object.fromEntries([...document.querySelectorAll('#levelFilters .level-chip')].map(b=>[b.dataset.level,
[b.querySelector('.letter').textContent,b.querySelector('.count').textContent,b.hidden]]))"""
HEADERS = "return [...document.querySelectorAll('#logsHeaders .column-heading')].map(e=>e.textContent)"
ERROR = "const e=document.getElementById('largeError');return e.hidden?'':e.textContent"
# The uptime cell of a row and the width the widest uptime (u32 / 10000) needs.
UPTIME = """const r=document.querySelector(`#logsRows [data-line-number="${arguments[0]}"]`);const c=r?.querySelector(':scope > .uptime');if(!c)return null;
const probe=c.cloneNode(false);probe.textContent='429496.7295';Object.assign(probe.style,{position:'absolute',visibility:'hidden',width:'auto',maxWidth:'none'});
r.appendChild(probe);const need=probe.getBoundingClientRect().width;probe.remove();
return {text:c.textContent,width:c.getBoundingClientRect().width,need,clipped:c.scrollWidth>c.clientWidth};"""
MENU = "const m=document.getElementById('rowMenu');return m.hidden?null:[...m.querySelectorAll('[role=menuitem]')].map(e=>e.textContent)"


def local(seconds, micros=None):
    text = time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(seconds))
    return text if micros is None else f'{text}.{micros:06d}'


def run(ctx):
    rows = fixtures.dlt(ctx.fixtures)
    fixtures.ui(ctx.fixtures)
    fixtures.covesa(ctx.fixtures)
    messages = [r for r in rows if 'apid' in r]
    line = lambda key: next(r['line'] for r in rows if r.get(key))
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        row = lambda n: js(ROW, n)
        app.open_fixture('vehicle.dlt')
        app.wait_count(len(rows))
        headers = js(HEADERS)
        check('a DLT file shows its own columns', headers == ['행', '시간', '타임스탬프', 'ECU', 'APID', 'CTID', '레벨', '페이로드'], headers)
        r1, e1 = app.wait(lambda: row(1), what='row 1'), messages[0]
        check('a log message fills the DLT columns in local time',
              [r1['stamp'], r1['uptime'], r1['ecu'], r1['apid'], r1['ctid'], r1['level'], r1['message']]
              == [local(e1['seconds'], e1['micros']), e1['uptime'], e1['ecu'], e1['apid'], e1['ctid'], e1['level'], e1['message']], r1)
        korean = row(messages[1]['line'])
        check('UTF-8 payloads keep Korean and emoji', korean['message'] == messages[1]['message'] and korean['ecu'] == 'HU', korean)
        control = row(line('control'))
        check('a control response shows as 기타 with its message type in the tooltip',
              control['level'] == '기타' and control['levelTitle'] == 'control:response' and control['message'] == '[get_log_info ok] 07 01', control)
        plain = row(line('nonverbose'))
        check('a message without an extended header shows its message ID and data',
              plain['level'] == '기타' and plain['apid'] == '' and plain['uptime'] == '' and plain['message'] == '[4096] AB-|41 42 00', plain)
        issue = row(line('issue'))
        check('bytes that are no message are one raw row', 'raw-row' in issue['cls'] and issue['raw'].startswith('!! DLT 메시지가 아닌 데이터 5바이트'), issue)
        levels = app.wait(lambda: (lambda v: '+' not in ''.join(x[1] for x in v.values()) and v)(js(LEVELS)), what='level counts')
        want = {name: str(sum(1 for r in messages if r['level'] == name)) for name in ['Fatal', 'Error', 'Warn', 'Info', 'Debug', 'Verbose']}
        check('level chips count DLT levels, hide Assert and name other messages 기타',
              levels['A'][2] is True and levels['U'][:2] == ['기타', '2'] and all(levels[name[0]][1] == count for name, count in want.items()), levels)
        check('a DLT file has no follow button', app.hidden('followFile'))
        app.capture('01-columns')

        js("const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;")
        panel = js("""const $=id=>document.getElementById(id);return {tag:$('tagLabel').textContent,
ids:!$('idFields').hidden||!$('openIds').hidden,reading:$('readingOptions').hidden,from:$('timeFrom').placeholder}""")
        check('the filter panel offers ECU · APID · CTID and no PID, TID or reading options',
              panel == {'tag': 'ECU · APID · CTID', 'ids': False, 'reading': True, 'from': '03-14 09:26'}, panel)
        app.type('tagFilter', 'NAV')
        app.wait_count(sum(1 for r in messages if r['apid'] == 'NAV'))
        check('the ID filter keeps the messages of an APID', all(row(r['line']) for r in messages if r['apid'] == 'NAV') and row(1)['apid'] == 'NAV')
        app.capture('02-filter-panel')
        app.type('tagFilter', 'HU, BOOT')
        app.wait_count(sum(1 for r in messages if r['ecu'] == 'HU' or r['ctid'] == 'BOOT'))
        check('one ID field matches ECUs and CTIDs', True, app.count())
        app.type('tagFilter', '')
        app.exclude_tags('TCU', enter=False)
        # The non-verbose message is TCU's; rows of other bytes are never excluded.
        app.wait_count(len(rows) - sum(1 for r in messages if r['ecu'] == 'TCU') - 1)
        check('excluding an ECU hides its messages', True, app.count())
        app.exclude_tags('')
        start = fixtures.DLT_BASE + 10
        app.type('timeFrom', local(start))
        app.type('timeTo', local(start + 9))
        app.wait_count(sum(1 for r in messages if start <= r['seconds'] <= start + 9) + 1)
        check('the time filter reads DLT times as shown (local time, both ends included)', True, app.count())
        app.type('timeFrom', '')
        app.type('timeTo', '')
        app.wait_count(len(rows))
        js("const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')==='true')t.click();return true;")

        app.type('searchInput', 'NAV')
        marked = app.wait(lambda: (lambda v: v['marks']['apid'] and v)(row(1)), what='APID mark')
        check('a search match is marked in the APID cell', marked['marks']['apid'] == ['NAV'], marked)
        app.type('searchInput', 'recalculated')
        marked = app.wait(lambda: (lambda v: v['marks']['message'] and v)(row(1)), what='payload mark')
        check('a search match is marked in the payload', marked['marks']['message'] == ['recalculated'], marked)
        app.capture('03-search')
        app.type('searchInput', '')

        app.click_row(1)
        app.click_row(1, event='contextmenu')
        items = app.wait(lambda: js(MENU), what='row menu')
        check('the row menu filters by ECU, APID and CTID, by time and level, never by PID',
              "ECU 'ECU1' 보기 조건 추가" in items and "APID 'NAV' 보기 조건 추가" in items and "CTID 'RTE' 제외" in items
              and f"{local(e1['seconds'], e1['micros'])}부터 보기 조건 추가" in items and 'Warn 레벨 보기 조건 추가' in items and not any(i.startswith(('PID', 'TID')) for i in items), items)
        app.capture('04-row-menu')
        app.key('Escape', target='rowMenu')
        app.wait(lambda: js(MENU) is None, what='menu closed')
        app.click('exportLogs')
        items = app.wait(lambda: js(MENU), what='export menu')
        check('export asks for DLT messages or text rows first', items == ['DLT 원본 (.dlt)', '텍스트 (.txt)'], items)
        app.capture('05-export-menu')
        app.key('Escape', target='rowMenu')

        app.open_fixtures(['vehicle.dlt', 'early.dlt'], True)
        app.wait_count(len(rows) + 1)
        first = app.wait(lambda: row(1), what='combined row 1')
        check('combined DLT files open in the order of their first message', first['apid'] == 'EARL' and first['stamp'] == local(fixtures.DLT_BASE - 3600, 0), first)
        js("window.__err=null;window.__TAURI__.core.invoke('qa_open_fixtures',{names:['vehicle.dlt','ui.log']}).catch(e=>window.__err=String(e));return true;")
        err = app.wait(lambda: js("return window.__err"), what='mixing error')
        check('a DLT file and a text log do not open together', err == 'DLT files and text logs cannot be opened together', err)
        app.open_fixtures(['vehicle.dlt', fixtures.COVESA_V2])
        error = app.wait(lambda: js(ERROR), what='file error')
        # The error can show while the view returns to the open log.
        app.wait_count(len(rows) + 1)
        check('an open of several files that fails names the file and keeps the open log',
              error == f'{fixtures.COVESA_V2}: DLT 버전 2 파일은 지원하지 않습니다.' and js(ERROR) == error, error)
        app.capture('06-file-error')
        app.open_fixture(fixtures.COVESA_HEADERLESS)
        error = app.wait(lambda: (lambda t: t.startswith('저장 헤더') and t)(js(ERROR)), what='headerless error')
        check('a DLT file without storage headers says so', error == '저장 헤더가 없는 DLT 데이터는 열 수 없습니다. 메시지마다 저장 헤더가 있는 .dlt 파일을 열어 주세요.', error)
        app.capture('07-headerless')
        app.open_fixture(fixtures.COVESA_EXTENDED)
        app.wait_count(111)
        app.wait(lambda: not js(ERROR), what='error cleared')
        bool_row = app.wait(lambda: row(24), what='row 24')
        app.jump(60)
        plain = app.wait(lambda: row(60), what='row 60')
        check("dlt-daemon's own file opens with its typed arguments and non-verbose data",
              [bool_row['ecu'], bool_row['apid'], bool_row['ctid'], bool_row['level'], bool_row['message']] == ['ECU', 'LOG', 'TES2', 'Info', 'true']
              and plain['message'] == '[102] --|f3 03' and plain['level'] == '기타', {'24': bool_row, '60': plain})
        uptime = app.wait(lambda: js(UPTIME, 55), what='uptime cell')
        check('the uptime column fits the widest uptime by default',
              uptime['text'] == '106603.7030' and not uptime['clipped'] and uptime['need'] <= uptime['width'], uptime)
        app.capture('08-dlt-daemon')
        app.open_fixture('ui.log')
        app.wait_count(3000)
        headers = js(HEADERS)
        panel = js("const $=id=>document.getElementById(id);return {tag:$('tagLabel').textContent,ids:!$('idFields').hidden||!$('openIds').hidden,a:document.querySelector('#levelFilters [data-level=A]').hidden}")
        check('a text log shows its own columns and filter fields again',
              headers == ['행', '날짜 · 시간', 'PID', 'TID', '레벨', '태그', '메시지'] and panel == {'tag': '태그', 'ids': True, 'a': False}, {'headers': headers, 'panel': panel})
