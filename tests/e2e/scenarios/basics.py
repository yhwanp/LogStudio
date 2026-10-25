"""Empty state, opening, filter chips, levels, ID validation, selection and
copy (Shift, drag, Cmd+click, text inside the selected row, a drag held past
the edge, Cmd+A in search results), the long-row detail, search stepping,
the pane splitter, the row menu, a drag on a scaled scrollbar, raw view and
closing (ui.log, scaled.log)."""
import time

import fixtures

VISIBLE = "const v=id=>{const e=document.getElementById(id);return !!e&&!e.hidden&&getComputedStyle(e).display!=='none'&&e.getClientRects().length>0;};"
CHIPS = "return [...document.querySelectorAll('#activeFilters .filter-chip')].map(e=>e.textContent)"
SELECTION = """return {range:[...document.querySelectorAll('#logsRows .in-selection')].map(e=>Number(e.dataset.lineNumber)),
selected:document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null,info:document.getElementById('logsSelection').textContent,text:String(getSelection())};"""
LOWER = """return {info:document.getElementById('largeSelection').textContent,rows:document.querySelectorAll('#largeRows .log-row').length,
marked:document.querySelectorAll('#largeRows .in-selection').length,focus:document.querySelector('#largeRows .selected-log-row')?.dataset.lineNumber??null};"""
# A copy event on the focused element; `copy` says the page supplied rows.
COPY = """const event=new Event('copy',{bubbles:true,cancelable:true}),store={};
Object.defineProperty(event,'clipboardData',{value:{setData:(t,v)=>{store[t]=v;},getData:t=>store[t]}});
(document.activeElement||document.body).dispatchEvent(event);return {copy:event.defaultPrevented,data:store['text/plain']??null};"""
# Row 20 is the one selected row. Its press is left to the browser; a text
# selection made in it (as a drag would) is copied as text, and one stretched
# into row 22 is pulled back into row 20.
TEXT_IN_ROW = """const row=n=>document.querySelector(`#logsRows [data-line-number="${n}"]`),us=e=>{const s=getComputedStyle(e);return s.webkitUserSelect||s.userSelect;};
const r20=row(20),b=r20.getBoundingClientRect(),o={bubbles:true,cancelable:true,detail:1,button:0,buttons:1,clientX:b.left+400,clientY:b.top+b.height/2};
r20.dispatchEvent(new PointerEvent('pointerdown',{...o,pointerId:1,pointerType:'mouse',isPrimary:true}));const down=new MouseEvent('mousedown',o);r20.dispatchEvent(down);
document.dispatchEvent(new PointerEvent('pointerup',{...o,buttons:0,pointerId:1}));
const m=r20.querySelector('.message').firstChild,s=getSelection();s.setBaseAndExtent(m,0,m,Math.min(24,m.length));const text=String(s);
const copy=new Event('copy',{bubbles:true,cancelable:true});Object.defineProperty(copy,'clipboardData',{value:{setData(){}}});r20.dispatchEvent(copy);
s.setBaseAndExtent(m,0,row(22).querySelector('.message').firstChild,10);
return {styles:[us(r20),us(row(21)),us(row(22))],pressSelected:down.defaultPrevented,text,copyTaken:copy.defaultPrevented};"""
# A pointermove (button held) or pointerup at a row's centre, `dy` below it.
POINTER_AT_ROW = """const b=document.querySelector(`#logsRows [data-line-number="${arguments[1]}"]`).getBoundingClientRect();
document.dispatchEvent(new PointerEvent(arguments[0],{bubbles:true,cancelable:true,clientX:200,clientY:b.top+b.height/2+arguments[2],pointerId:1,pointerType:'mouse',isPrimary:true,button:0,buttons:arguments[0]==='pointerup'?0:1}));return true;"""
SPREAD = "const s=getSelection();return {inRow:document.querySelector('#logsRows [data-line-number=\"20\"]').contains(s.focusNode),text:String(s)};"
# The synchronous work of one drag step (selection, render, forced layout):
# press a row near the top, move and release over another row.
STEP_COST = """const v=document.getElementById('logsViewport').getBoundingClientRect(),x=v.left+300,row=n=>document.querySelector(`#logsRows [data-line-number="${n}"]`);
const a=Number(document.elementFromPoint(x,v.top+20).closest('.log-row').dataset.lineNumber)+1,y=n=>{const b=row(n).getBoundingClientRect();return b.top+b.height/2;},times=[];
const o=(at,buttons)=>({bubbles:true,cancelable:true,detail:1,button:0,buttons,clientX:x,clientY:at,pointerId:1,pointerType:'mouse',isPrimary:true});
for(let k=0;k<40;k++){const r=row(a);r.dispatchEvent(new PointerEvent('pointerdown',o(y(a),1)));r.dispatchEvent(new MouseEvent('mousedown',o(y(a),1)));
const to=y(a+1+k%6);document.dispatchEvent(new PointerEvent('pointermove',o(to,1)));const t0=performance.now();document.dispatchEvent(new PointerEvent('pointerup',o(to,0)));void document.body.offsetHeight;times.push(performance.now()-t0);}
const rows=document.querySelectorAll('#logsRows .log-row').length;times.sort((p,q)=>p-q);
return {median:+times[20].toFixed(2),p95:+times[37].toFixed(2),max:+times[39].toFixed(2),steps:40,renderedRows:rows};"""


def hold_drag(app, seconds):
    """Presses on the first row in view of the upper logs and holds the
    pointer far below the list for `seconds`; returns the pressed row, the
    list's bottom and what was seen (rows in view, focus samples, whether
    WebKit saw the window hidden)."""
    view = app.js("const v=document.getElementById('logsViewport').getBoundingClientRect(),e=document.elementFromPoint(v.left+300,v.top+20);"
                  "return {start:Number(e.closest('.log-row').dataset.lineNumber),bottom:v.bottom,visible:Math.floor(v.height/28),hidden:document.hidden};")
    app.press_row(view['start'])
    app.pointer('pointermove', view['bottom'] + 1000)
    samples, began = [], time.monotonic()
    while time.monotonic() - began < seconds:
        time.sleep(.25)
        samples.append(int(app.js("return Number(document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??0)")))
    app.pointer('pointerup', view['bottom'] + 1000)
    time.sleep(.3)
    rows = samples[-1] - view['start'] + 1
    return view['start'], view['bottom'], {'visible': view['visible'], 'hidden': view['hidden'], 'samples': samples, 'rowsPerSecond': round(rows / seconds)}


def run(ctx):
    lines = fixtures.ui(ctx.fixtures)
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        a = js(VISIBLE + "return {empty:v('emptyState'),content:v('logContent'),close:v('largeClose'),"
               "topbar:document.querySelector('.topbar').getBoundingClientRect().height};")
        check('empty state shown before any file', a['empty'] and not a['content'] and not a['close'], a)
        app.capture('01-empty')

        app.open_fixture('ui.log')
        app.wait_count(3000)
        b = js(VISIBLE + "const r=document.querySelector('#logsRows .log-row');return {empty:v('emptyState'),content:v('logContent'),close:v('largeClose'),"
               "progress:v('loadProgress'),name:document.getElementById('largeName').textContent,size:document.getElementById('largeSize').textContent,"
               "rowTab:r.tabIndex,describedby:r.getAttribute('aria-describedby'),label:r.getAttribute('aria-label'),"
               "exportLogs:document.getElementById('exportLogs').disabled,exportSearch:document.getElementById('exportSearch').disabled};")
        check('file view replaces the empty state', not b['empty'] and b['content'] and b['close'] and not b['progress'] and b['name'] == 'ui.log', b)
        check('rows are not tab stops and keep readable content', b['rowTab'] == -1 and b['describedby'] == 'logsRowHint' and b['label'] is None, b)
        check('top bar shows the file size; export only for completed results', b['size'] == '449.5 KiB' and b['exportLogs'] is False and b['exportSearch'] is True, b)
        tint = js("const r=document.querySelector('#logsRows .level-row-E');return r&&{bg:getComputedStyle(r).backgroundColor,plain:getComputedStyle(document.querySelector('#logsRows .level-row-I')).backgroundColor};")
        check('error rows are tinted', tint and tint['bg'] != tint['plain'], tint)
        app.capture('02-loaded')

        # Filter chips and the badge.
        app.enter('tagFilter', 'Wifi')
        app.wait_count(500)
        c = js(VISIBLE + "return {badge:document.getElementById('filterBadge').textContent,shown:v('filterBadge'),chips:[...document.querySelectorAll('#activeFilters .filter-chip')].map(e=>e.textContent)};")
        check('an applied tag filter shows the badge and a chip', c['badge'] == '1' and c['shown'] and c['chips'] == ['태그: Wifi×'], c)
        app.capture('03-chip')
        js("document.querySelector('#activeFilters .filter-chip').click();return true;")
        app.wait_count(3000)
        check('the chip clears only that condition', js("return document.getElementById('tagFilter').value") == '' and js(CHIPS) == [])

        app.exclude_tags('Wifi, network')
        app.wait_count(2000)
        e = js("return {chips:[...document.querySelectorAll('#activeFilters .chip-joint,#activeFilters .filter-chip')].map(e=>e.textContent),tags:[...new Set([...document.querySelectorAll('#logsRows .tag')].map(e=>e.textContent))]};")
        check('excluded tags remove rows case-insensitively and show a chip after 제외', e['chips'] == ['제외', '태그: Wifi, network×'] and not any(t in ('WifiMonitor', 'NetworkStack') for t in e['tags']), e)
        js("document.querySelector('#activeFilters .filter-chip').click();return true;")
        app.wait_count(3000)

        # Alt+click keeps one level.
        js("document.querySelector('#levelFilters [data-level=W]').dispatchEvent(new MouseEvent('click',{bubbles:true,altKey:true}));return true;")
        app.wait_count(300)
        d = js("return {chips:[...document.querySelectorAll('#activeFilters .filter-chip')].map(e=>e.textContent),levels:[...document.querySelectorAll('#levelFilters [aria-pressed=true]')].map(e=>e.dataset.level)};")
        check('Alt+click keeps only that level', d['levels'] == ['W'] and d['chips'] == ['레벨: Warn×'], d)
        app.click('allLevels')
        app.wait_count(3000)

        # A non-digit PID is explained beside the field.
        app.enter('pidFilter', '12a')
        e = app.wait(lambda: (lambda v: not v['hidden'] and v)(js("return {error:document.getElementById('idFilterError').textContent,hidden:document.getElementById('idFilterError').hidden,invalid:document.getElementById('pidFilter').getAttribute('aria-invalid')};")), what='PID error')
        check('an invalid PID is explained beside the field', '숫자' in e['error'] and e['invalid'] == 'true', e)
        app.enter('pidFilter', '')
        app.wait(lambda: js("return document.getElementById('idFilterError').hidden") and app.count() == '로그 · 전체 3,000건', what='PID cleared')

        # A press on the last row in view opens the raw detail, which shrinks
        # the list under the pointer: the pressed row stays in view, and a
        # hand holding still neither drags nor scrolls the list away.
        last = js("const v=document.getElementById('logsViewport').getBoundingClientRect();"
                  "return Math.max(...[...document.querySelectorAll('#logsRows .log-row')].filter(r=>r.getBoundingClientRect().bottom<=v.bottom).map(r=>Number(r.dataset.lineNumber)));")
        y = app.row_y(last)
        app.press_row(last)
        app.wait(lambda: js("return !document.getElementById('largeDetail').hidden"), what='detail under the press')
        time.sleep(.3)
        app.pointer('pointermove', y + 1)
        time.sleep(.4)
        held = {**js(SELECTION), **js("const v=document.getElementById('logsViewport').getBoundingClientRect(),r=document.querySelector('#logsRows .selected-log-row')?.getBoundingClientRect();"
                                      "return {inView:!!r&&r.top>=v.top-1&&r.bottom<=v.bottom+1,range:document.getElementById('logsRange').textContent};")}
        app.pointer('pointerup', y + 1)
        time.sleep(.15)
        check('a press on the last row keeps it in view as the detail opens, and holding still selects nothing more', held['selected'] == str(last) and held['inView'] and held['info'] == '', {'last': last, **held})
        app.key('Escape', target='logsViewport')
        js("document.getElementById('logsFirst').click();return true;")
        app.wait(lambda: js("return document.getElementById('largeDetail').hidden&&document.getElementById('logsRange').textContent.startsWith('1–')"), what='back at the top')

        # Shift range selection, without a browser text selection.
        app.click_row(3)
        app.wait(lambda: js("return !document.getElementById('largeDetail').hidden&&document.getElementById('largeDetailText').textContent.length>0"), what='detail')
        app.click_row(6, shift=True)
        f = js("return {info:document.getElementById('logsSelection').textContent,range:[...document.querySelectorAll('#logsRows .in-selection')].map(e=>e.dataset.lineNumber),"
               "text:String(getSelection()),active:document.getElementById('logsViewport').contains(document.activeElement)};")
        check('shift-click selects a row range', f['range'] == ['3', '4', '5', '6'] and f['info'].startswith('4행 선택') and f['text'] == '' and f['active'], f)
        # Copy: beforecopy enables Edit → Copy; copy writes the raw rows LF-joined.
        g = js("""const before=new Event('beforecopy',{bubbles:true,cancelable:true});document.activeElement.dispatchEvent(before);
let data=null;const event=new Event('copy',{bubbles:true,cancelable:true});const store={};
Object.defineProperty(event,'clipboardData',{value:{setData:(t,v)=>{store[t]=v;},getData:t=>store[t]}});
document.activeElement.dispatchEvent(event);return {before:before.defaultPrevented,copy:event.defaultPrevented,data:store['text/plain']};""")
        check('beforecopy is cancelled so Edit → Copy can enable', g['before'])
        check('copy supplies the four raw rows', g['copy'] and g['data'] == '\n'.join(lines[2:6]), {'match': g['data'] == '\n'.join(lines[2:6])})
        t = js("const r=e=>e.getBoundingClientRect(),t=document.getElementById('toast'),d=document.getElementById('largeDetail'),h=d.querySelector('.detail-heading');"
               "return {shown:!t.hidden,text:t.textContent,detail:!d.hidden,top:r(t).top,bottom:r(t).bottom,left:r(t).left,right:r(t).right,headingTop:r(h).top,headingBottom:r(h).bottom,"
               "titleRight:r(h.querySelector('strong')).right,actionsLeft:r(d.querySelector('.detail-actions')).left};")
        check('the copy toast never covers the open raw detail\'s title and buttons', t['shown'] and '4행' in t['text'] and t['detail']
              and (t['bottom'] <= t['headingTop'] or t['top'] >= t['headingBottom'] or (t['left'] >= t['titleRight'] and t['right'] <= t['actionsLeft'])), t)
        app.capture('04-range')
        # Shift with a page key extends from the same anchor a page at a time.
        page = js("return Math.floor((document.getElementById('logsViewport').clientHeight-28)/28)")
        app.key('PageDown', shift=True, target='logsViewport')
        p = app.wait(lambda: (lambda v: v['selected'] == str(6 + page) and v)(js(SELECTION)), what='Shift+PageDown')
        app.key('PageUp', shift=True, target='logsViewport')
        q = app.wait(lambda: (lambda v: v['selected'] == '6' and v)(js(SELECTION)), what='Shift+PageUp')
        check('Shift+PageDown extends the range a page and Shift+PageUp takes it back', p['range'] == list(range(3, 7 + page)) and p['info'].startswith(f'{4 + page}행 선택')
              and q['range'] == [3, 4, 5, 6] and q['info'].startswith('4행 선택'), {'page': page, 'down': p, 'up': q})


        # Mouse gestures, as the pointer and mouse events a real mouse sends
        # (synthetic: a real trackpad or mouse is not driven). A drag selects
        # the rows from the press to the pointer, in either direction.
        took = app.drag_rows(8, 12)
        h = js(SELECTION)
        check('a drag selects the rows from the press to the pointer', took and h['range'] == [8, 9, 10, 11, 12] and h['selected'] == '12' and h['info'].startswith('5행 선택') and h['text'] == '', h)
        app.capture('04a-drag-range')
        took = app.drag_rows(14, 11)
        h = js(SELECTION)
        check('a drag upward selects too, and its focus is the row under the pointer', took and h['range'] == [11, 12, 13, 14] and h['selected'] == '11', h)
        # Cmd+click adds and removes single rows; actions take them in list order.
        app.click_row(16, meta=True)
        app.click_row(12, meta=True)
        h = js(SELECTION)
        check('Cmd+click adds and removes rows apart from the range', h['range'] == [11, 13, 14, 16] and h['selected'] == '16' and h['info'].startswith('4행 선택'), h)
        g = js(COPY)
        check('copy writes the discontiguous rows in list order', g['copy'] and g['data'] == '\n'.join(lines[i - 1] for i in (11, 13, 14, 16)), g)
        app.capture('04b-discontiguous')
        app.click_row(13, event='contextmenu')
        m = app.wait(lambda: (lambda v: not v['hidden'] and v)(js("const m=document.getElementById('rowMenu');return {hidden:m.hidden,items:[...m.querySelectorAll('[role=menuitem]')].map(e=>e.textContent)};")), what='row menu')
        check('a right-click inside the selection keeps it and the menu acts on all of it', '선택한 4행 복사' in m['items'] and js(SELECTION)['range'] == [11, 13, 14, 16], m)
        app.key('Escape', target='rowMenu')

        # Escape closes the raw detail first; with nothing left to close it
        # makes the selection of several rows its focus row.
        detail_open = js("return !document.getElementById('largeDetail').hidden")
        app.key('Escape', target='logsViewport')
        time.sleep(.15)
        first = js(SELECTION)
        if detail_open:
            app.key('Escape', target='logsViewport')
            time.sleep(.15)
        c = js(SELECTION)
        check('Escape, with nothing left to close, makes a selection of several rows its focus row',
              (not detail_open or first['info'].startswith('4행 선택')) and c['range'] == [] and c['selected'] == '16' and c['info'] == '', {'detail': detail_open, 'first': first, 'after': c})

        # Text: only the one selected row leaves a press to the browser, so its
        # text can be selected; the selection cannot spread to other rows.
        app.click_row(20)
        t = js(TEXT_IN_ROW)
        time.sleep(.15)
        spread = js(SPREAD)
        check('only the one selected row is text-selectable, and a press on it is left to the browser', t['styles'] == ['text', 'none', 'none'] and not t['pressSelected'], t)
        check('text selected inside the row is copied as text, not as rows', t['text'] == lines[19].split(': ', 1)[1][:24] and not t['copyTaken'], t)
        check('a text selection cannot spread past its row', spread['inRow'] and spread['text'] != '' and lines[21].split(': ', 1)[1][:10] not in spread['text'], spread)
        js("const m=document.querySelector('#logsRows [data-line-number=\"20\"] .message').firstChild;getSelection().setBaseAndExtent(m,0,m,Math.min(24,m.length));return true;")
        app.capture('04c-text-in-row')
        # The row menu replaces the browser's, so it copies that text too: the
        # text as it was when the menu opened (WebKit may drop the shown
        # selection as focus moves into the menu).
        app.click_row(20, event='contextmenu')
        m = app.wait(lambda: (lambda v: not v['hidden'] and v)(js("const m=document.getElementById('rowMenu');return {hidden:m.hidden,items:[...m.querySelectorAll('[role=menuitem]')].map(e=>e.textContent)};")), what='row menu over text')
        app.capture('04e-text-menu')
        app.capture_clipboard()
        js("document.querySelector('#rowMenu [role=menuitem]').click();return true;")
        got = app.captured()
        check('the row menu over text selected in the row offers to copy that text first, and copies it', m['items'][0] == '선택한 텍스트 복사' and got == t['text'], {'items': m['items'][:2], 'copied': got, 'selected': t['text']})
        took = app.press_row(21)
        app.pointer('pointerup', app.row_y(21))
        g = js(COPY)
        check('a press on another row is a row gesture that drops the text, so copy takes that row', took and g['copy'] and g['data'] == lines[20] and js("return String(getSelection())") == '', g)
        a = js("const v=document.getElementById('logsViewport');v.focus();const e=new KeyboardEvent('keydown',{key:'a',metaKey:true,bubbles:true,cancelable:true});v.dispatchEvent(e);"
               "return {prevented:e.defaultPrevented,text:String(getSelection()),info:document.getElementById('logsSelection').textContent,selected:document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber};")
        check('Cmd+A in the upper logs changes nothing and selects no page text', a == {'prevented': True, 'text': '', 'info': '', 'selected': '21'}, a)
        # A drag that starts as text in the one selected row stays text inside
        # that row; past the row it drops the text and selects rows from there.
        # Row 21 is the list's last row, so the drag goes up, within the list.
        # Each pointer event is placed on its row in the same call that sends
        # it, so a layout change in between cannot move it to another row.
        time.sleep(.3)
        js("const m=document.querySelector('#logsRows [data-line-number=\"21\"] .message').firstChild;getSelection().setBaseAndExtent(m,0,m,Math.min(12,m.length));return true;")
        took = app.press_row(21)
        js(POINTER_AT_ROW, 'pointermove', 21, -5)
        time.sleep(.1)
        inside = js(SELECTION)
        js(POINTER_AT_ROW, 'pointermove', 19, 0)
        time.sleep(.1)
        js(POINTER_AT_ROW, 'pointerup', 19, 0)
        time.sleep(.15)
        out = js(SELECTION)
        check('a drag from the one selected row selects text inside it, and past the row drops the text and selects rows from that row',
              not took and inside['range'] == [] and inside['text'] != '' and out['range'] == [19, 20, 21] and out['selected'] == '19' and out['text'] == '', {'inside': inside, 'out': out})
        app.capture('04d-drag-from-selected-row')

        # Holding a drag past the bottom edge keeps scrolling and selecting.
        # A window covered by others is hidden to WebKit, which then pauses
        # frames and slows timers, so the scroll rate is recorded, not checked;
        # a drag step's synchronous work (selection, render, layout) is timed.
        js("document.getElementById('logsViewport').focus();return true;")
        app.jump(100)
        app.wait(lambda: app.row_exists(100), what='row 100')
        start, bottom, rows = hold_drag(app, 4)
        end = js(SELECTION)
        check('a drag held past the bottom edge keeps scrolling and selecting', end['selected'] and int(end['selected']) - start + 1 > rows['visible'] + 16
              and end['info'].startswith(f"{int(end['selected']) - start + 1:,}행 선택"), {'start': start, **rows, 'info': end['info']})
        step = js(STEP_COST)
        check('one drag step updates the selection and lays out in a frame budget', step['median'] < 16, step)

        # A truncated long row copies its whole raw text through the async clipboard.
        app.jump(50)
        app.wait(lambda: app.row_exists(50), what='row 50')
        app.click_row(50)
        app.wait(lambda: js("return document.getElementById('largeDetailTitle').textContent.startsWith('50행')&&!document.getElementById('largeDetailText').textContent.includes('읽는 중')"), what='row 50 detail')
        app.capture_clipboard()
        js("const e=new Event('copy',{bubbles:true,cancelable:true});Object.defineProperty(e,'clipboardData',{value:{setData(){throw Error('sync path used for a truncated row');}}});document.activeElement.dispatchEvent(e);return true;")
        got = app.captured()
        check('a truncated long row copies its full raw text', got == lines[49], {'length': len(got), 'expected': len(lines[49])})
        nav = js(VISIBLE + "return {nav:v('largeDetailNavigation'),next:!document.getElementById('largeDetailNext').disabled,height:document.getElementById('largeDetailText').getBoundingClientRect().height};")
        check('the long-row detail pages through the row at a usable height', nav['nav'] and nav['next'] and nav['height'] > 120, nav)
        app.click('largeDetailNext')
        app.wait(lambda: js("return !document.getElementById('largeDetailPrevious').disabled&&!document.getElementById('largeDetailText').textContent.includes('읽는 중')"), what='next detail window')
        app.capture_clipboard()
        app.click('largeDetailCopy')
        check('detail copy from a later window copies the whole row', app.captured() == lines[49])
        app.capture('05-long-detail')

        # Search: Enter/Shift+Enter step through results, reveal them above and keep focus.
        app.enter('searchInput', 'timeout')
        app.wait(lambda: '검색 완료' in app.text('largeTotal') and js("return document.querySelectorAll('#largeRows .log-row').length>0"), what='search')
        sel = "return {search:document.querySelector('#largeRows .selected-log-row')?.dataset.lineNumber,logs:document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber,active:document.activeElement.id};"
        app.key('Enter', target='searchInput')
        s1 = app.wait(lambda: (lambda v: v['search'] and v['search'] == v['logs'] and v)(js(sel)), what='first result')
        app.key('Enter', target='searchInput')
        s2 = app.wait(lambda: (lambda v: v['search'] and v['search'] != s1['search'] and v['search'] == v['logs'] and v)(js(sel)), what='next result')
        app.key('Enter', shift=True, target='searchInput')
        s3 = app.wait(lambda: (lambda v: v['search'] == s1['search'] and v['logs'] == s1['search'] and v)(js(sel)), what='previous result')
        check('Enter/Shift+Enter walk results, reveal them above and keep typing focus', s1['active'] == s2['active'] == s3['active'] == 'searchInput', [s1, s2, s3])
        position = lambda: js("const p=document.getElementById('searchPosition');return {text:p.textContent,live:p.getAttribute('aria-live'),shown:p.getBoundingClientRect().width>0};")
        p1 = position()
        check('the search box shows the result it steps to, announced', p1['text'] == f"1번째 / {app.text('largeCount')}" and p1['live'] == 'polite' and p1['shown'], p1)
        app.key('Enter', shift=True, target='searchInput')
        p2 = app.wait(lambda: (lambda v: v['text'].startswith('다시 끝부터') and v)(position()), what='wrap to the last result')
        app.key('Enter', target='searchInput')
        p3 = app.wait(lambda: (lambda v: v['text'].startswith('다시 처음부터') and v)(position()), what='wrap to the first result')
        check('a step around the end of the results says so', p2['text'] == f"다시 끝부터 · {app.text('largeCount').removesuffix('건')}번째 / {app.text('largeCount')}" and p3['text'] == f"다시 처음부터 · 1번째 / {app.text('largeCount')}", [p2, p3])
        app.capture('06c-search-position')
        # Clicking a result always follows it above, as a bookmark does, and keeps focus in the results.
        other = js("return [...document.querySelectorAll('#largeRows .log-row')].map(r=>r.dataset.lineNumber).find(n=>n!==arguments[0]);", s1['search'])
        app.click_row(other, pane='largeRows')
        s4 = app.wait(lambda: (lambda v: v['search'] == other and v['logs'] == other and v)(js(sel)), what='clicked result')
        s4['inResults'] = js("return document.getElementById('largeViewport').contains(document.activeElement);")
        s4['toggle'] = js("return !!document.getElementById('followSearch');")
        check('clicking a result follows it above with no toggle and keeps focus in the results', s4['inResults'] and not s4['toggle'], s4)
        s4 = js("const c=id=>document.getElementById(id).classList.contains('secondary-selection');return {upper:c('logsRows'),lower:c('largeRows')};")
        check('while the results have focus the same row above is secondary', s4['upper'] and not s4['lower'], s4)
        # Enter on that result moves focus to its row above, though the row is already shown.
        app.key('Enter')
        s5 = app.wait(lambda: (lambda v: v['line'] == other and v)(js("const a=document.activeElement;return {line:document.getElementById('logsViewport').contains(a)?a.dataset.lineNumber:null};")), what='focus on the row above')
        check('Enter on a followed result moves focus to its row above', s5['line'] == other, s5)
        # The list with the keys draws its selection as primary, the other as secondary, even on the same row.
        panes = "const c=id=>document.getElementById(id).classList.contains('secondary-selection');return {upper:c('logsRows'),lower:c('largeRows')};"
        a = app.wait(lambda: (lambda v: not v['upper'] and v['lower'] and v)(js(panes)), what='upper pane primary')
        check('focus moved above makes the row above primary and the result secondary', a, a)
        app.capture('06b-upper-focused')
        # Back on the first result, so the upper logs are at their start again for the steps below.
        app.click_row(s1['search'], pane='largeRows')
        app.wait(lambda: (lambda v: v['search'] == s1['search'] and v['logs'] == s1['search'] and v)(js(sel)), what='first result again')
        lower = js(VISIBLE + "return {splitter:v('paneSplitter'),expanded:document.getElementById('largeLowerPane').classList.contains('expanded'),"
                   "lower:document.getElementById('largeLowerPane').getBoundingClientRect().height,content:document.getElementById('logContent').getBoundingClientRect().height};")
        check('the lower region expands with a splitter', lower['splitter'] and lower['expanded'] and 0.3 < lower['lower'] / lower['content'] < 0.55, lower)
        app.capture('06-search')

        # Cmd+A in the search results selects every result; copy takes them all.
        hits = [line for line in lines if 'timeout' in line.lower()]
        a = js("const v=document.getElementById('largeViewport');v.focus();const e=new KeyboardEvent('keydown',{key:'a',metaKey:true,bubbles:true,cancelable:true});v.dispatchEvent(e);return e.defaultPrevented;")
        s = app.wait(lambda: (lambda v: v['info'] and v)(js(LOWER)), what='all results selected')
        check('Cmd+A selects every search result', a and s['info'] == f'{len(hits):,}행 선택 · ⌘C 복사' and s['marked'] == s['rows'] > 0, s)
        app.capture('06a-select-all-search')
        app.capture_clipboard()
        g = js(COPY)
        got = g['data'] if g['data'] is not None else app.captured()
        check('copy takes every selected result in order', g['copy'] and got == '\n'.join(hits), {'rows': len(got.split('\n')), 'expected': len(hits)})

        # Splitter: arrow keys and pointer drag resize and persist.
        # Keys step 0.03 from the shown share of the content height.
        shown = lower['lower'] / lower['content']
        js("const s=document.getElementById('paneSplitter');s.focus();for(let i=0;i<3;i++)s.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowUp',bubbles:true,cancelable:true}));return true;")
        k = app.wait(lambda: (lambda v: v['height'] > lower['lower'] and v)(js(
            "return {ratio:Number(getComputedStyle(document.getElementById('workspace')).getPropertyValue('--lower-ratio')),now:document.getElementById('paneSplitter').getAttribute('aria-valuenow'),"
            "height:document.getElementById('largeLowerPane').getBoundingClientRect().height,stored:JSON.parse(localStorage.getItem('logstudio.layout.v1')||'{}').lowerRatio};")), what='splitter keys')
        check('splitter arrow keys grow the lower region and persist', abs(k['ratio'] - (shown + 0.09)) < 0.02 and k['now'] == str(round(k['ratio'] * 100)) and k['stored'] == k['ratio'], {**k, 'shownBefore': round(shown, 3)})
        p = js("""const s=document.getElementById('paneSplitter'),b=s.getBoundingClientRect(),y=b.top+3,x=b.left+200;let error=null;
try{s.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true,pointerId:1,button:0,clientX:x,clientY:y,isPrimary:true}));
s.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,pointerId:1,clientX:x,clientY:y+80}));
s.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,pointerId:1,clientX:x,clientY:y+80}));}catch(e){error=String(e);}
return {error,ratio:getComputedStyle(document.getElementById('workspace')).getPropertyValue('--lower-ratio').trim(),resizing:document.getElementById('workspace').classList.contains('resizing-panes')};""")
        check('dragging the splitter down shrinks the lower region', p['error'] is None and float(p['ratio']) < k['ratio'] and not p['resizing'], p)
        js("document.getElementById('paneSplitter').dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));return true;")
        app.enter('searchInput', '')

        # Row menu: inside the window, keyboard navigation, Escape, an action, outside click.
        menu = "const m=document.getElementById('rowMenu'),b=m.getBoundingClientRect();return {hidden:m.hidden,items:[...m.querySelectorAll('[role=menuitem]')].map(e=>e.textContent),active:document.activeElement.textContent,inside:b.left>=0&&b.top>=0&&b.right<=innerWidth&&b.bottom<=innerHeight};"
        app.jump(1)
        app.click_row(4, event='contextmenu')
        m = app.wait(lambda: (lambda v: not v['hidden'] and v)(js(menu)), what='row menu')
        check('right-click opens the row menu inside the window with focus on the first item',
              m['items'][:2] == ["태그 'ExampleApp' 보기 조건 추가", "태그 'ExampleApp' 제외"] and m['active'] == m['items'][0] and m['inside'], m)
        app.capture('07-row-menu')
        app.key('ArrowDown', target='rowMenu')
        check('arrow keys move through the menu items', js("return document.activeElement.textContent") == m['items'][1])
        app.key('Escape', target='rowMenu')
        check('Escape closes the menu and returns focus to the list', js("return document.getElementById('rowMenu').hidden&&document.getElementById('logsViewport').contains(document.activeElement)"))
        app.click_row(4, event='contextmenu')
        app.wait(lambda: not js("return document.getElementById('rowMenu').hidden"), what='row menu')
        js("[...document.querySelectorAll('#rowMenu [role=menuitem]')].find(e=>e.textContent.startsWith('PID')).click();return true;")
        app.wait_count(1000)
        q = js("return {pid:document.getElementById('pidFilter').value,chips:[...document.querySelectorAll('#activeFilters .filter-chip')].map(e=>e.textContent),hidden:document.getElementById('rowMenu').hidden};")
        check('the menu PID action applies a removable PID filter', q['pid'] == '2983' and q['chips'] == ['PID: 2983×'] and q['hidden'], q)
        js("document.querySelector('#logsRows .log-row').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:200,clientY:200}));return true;")
        app.wait(lambda: not js("return document.getElementById('rowMenu').hidden"), what='row menu')
        js("document.getElementById('logsViewport').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));return true;")
        check('clicking elsewhere closes the menu', js("return document.getElementById('rowMenu').hidden"))
        js("document.querySelector('#activeFilters .filter-chip').click();return true;")
        app.wait_count(3000)

        # Shortcut handler (synthetic keydown; native key delivery is not covered).
        js("document.getElementById('logsViewport').focus();return true;")
        app.key('f', meta=True)
        check('Cmd+F focuses the search box', js("return document.activeElement.id") == 'searchInput')

        # A drag held past the edge of a list whose native scrollbar is scaled
        # (400,000 rows) keeps one-row precision and selects every row passed.
        fixtures.scaled(ctx.fixtures)
        app.open_fixture('scaled.log')
        app.wait_count(400_000, 60)
        app.jump(200_000)
        app.wait(lambda: app.row_exists(200_000), what='row 200,000')
        geo = js("const v=document.getElementById('logsViewport'),b=v.getBoundingClientRect(),e=document.elementFromPoint(b.left+300,b.top+20);"
                 "return {top:Number(e.closest('.log-row').dataset.lineNumber),bottom:b.bottom,height:v.scrollHeight};")
        start, bottom, rows = hold_drag(app, 4)
        end = js(SELECTION)
        count = int(end['selected']) - start + 1
        check('a drag past the edge of a scaled list selects every row it passes', geo['height'] < 400_000 * 28 and count > rows['visible'] + 16 and end['info'].startswith(f'{count:,}행 선택'),
              {**geo, **rows, 'selected': end['selected'], 'rows': count, 'info': end['info']})
        app.capture_clipboard()
        g = js(COPY)
        got = g['data'] if g['data'] is not None else app.captured()
        check('copy takes exactly those rows of the scaled list', got == '\n'.join(fixtures.scaled_row(n - 1) for n in range(start, int(end['selected']) + 1)), {'rows': len(got.split('\n')), 'expected': count})
        app.enter('searchInput', 'timeout')
        app.wait(lambda: '검색 완료' in app.text('largeTotal'), 60, what='scaled search')
        js("const v=document.getElementById('largeViewport');v.focus();v.dispatchEvent(new KeyboardEvent('keydown',{key:'a',metaKey:true,bubbles:true,cancelable:true}));return true;")
        s = app.wait(lambda: (lambda v: v['info'] and v)(js(LOWER)), what='all results selected')
        g = js(COPY)
        toast = app.text('toast')
        check('Cmd+A takes all 100,000 results; copy keeps its 10,000-row limit and points to export', s['info'] == '100,000행 선택 · ⌘C 복사' and g['copy'] and g['data'] is None and '최대 10,000행' in toast and '내보내기' in toast, {**s, 'toast': toast})
        app.enter('searchInput', '')

        # The drop hint follows native drag events, which the page cannot emit
        # (its capability allows listening only); it is not covered here.

        # Raw view, then close.
        app.click('largeRaw')
        check('raw view renders rows', app.wait(lambda: js("return document.getElementById('logsTable').classList.contains('raw-mode')&&document.querySelectorAll('#logsRows .raw-content').length>0"), 5, what='raw view'))
        app.click('largeColumns')
        app.click('largeClose')
        app.wait(lambda: js(VISIBLE + "return v('emptyState')&&!v('logContent')"), what='empty state')
        check('closing returns to the empty state without an error', js("return document.getElementById('largeError').hidden"))
