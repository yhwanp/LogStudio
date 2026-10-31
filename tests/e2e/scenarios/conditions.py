"""Filters with several conditions: the list of conditions, showing conditions
joined by 또는, a hiding condition with levels, items and their switches, the
chips reading as the filter, the editor floating over the log, the row menu
adding a condition, a search inside the conditions, a saved filter of several
conditions after a restart, conditions no row of the log matches, and DLT
conditions (ui.log, ui-copy.log, vehicle.dlt; counts from a row-at-a-time
oracle)."""
import json
import re
import shutil
import time

import fixtures

STATE = """const $=id=>document.getElementById(id),items=kind=>[...document.querySelectorAll('.cond-item')].filter(c=>c.querySelector(`.cond-kind.${kind}`)).map(c=>({summary:c.querySelector('.cond-summary').textContent,active:c.dataset.active,off:c.dataset.off}));
return {shows:items('show'),hides:items('hide'),chips:[...$('activeFilters').querySelectorAll('.chip-joint,.filter-chip')].map(e=>e.textContent),badge:$('filterBadge').hidden?null:$('filterBadge').textContent,
 editor:!$('filterEditor').hidden,title:$('filterEditorTitle').textContent,hideChecked:$('kindHide').getAttribute('aria-checked'),tag:$('tagFilter').value,count:$('logsCount').textContent,search:$('largeCount').textContent};"""
LEVELS = """const want=new Set(arguments[0]);for(const b of document.querySelectorAll('#levelFilters .level-chip')){
const on=b.getAttribute('aria-pressed')==='true';if(on!==want.has(b.dataset.level))b.click();}return true;"""
ITEM = "return [...document.querySelectorAll('.cond-item')].filter(c=>c.querySelector(`.cond-kind.${arguments[0]}`))[arguments[1]].querySelector(arguments[2]).click(),true;"
PIN = "const a=document.getElementById('autoHideFilters');a.checked=arguments[0];a.dispatchEvent(new Event('change',{bubbles:true}));return true;"
ROWLESS = """const items=kind=>[...document.querySelectorAll('.cond-item')].filter(c=>c.querySelector(`.cond-kind.${kind}`)).map(c=>({rows:c.dataset.rows,text:c.querySelector('.cond-summary').textContent,title:c.querySelector('.cond-summary').title}));
return {shows:items('show'),hides:items('hide'),chips:[...document.querySelectorAll('#activeFilters .filter-chip')].filter(e=>e.classList.contains('no-rows')).map(e=>e.textContent)};"""
OPEN_PANEL = "const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;"
FIELDS = """const $=id=>document.getElementById(id),shown=key=>!$(`${key}Row`).hidden;return {fields:['ecu','apid','ctid','tag','pid','tid','message'].filter(shown),focus:document.activeElement?.id??null,pid:$('pidFilter').value};"""
PLACE = """const $=id=>document.getElementById(id),box=e=>{const r=e.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom};};return {editor:box($('filterEditor')),panel:box($('filterPanel')),
 position:getComputedStyle($('filterEditor')).position,panelOpen:!$('filterPanel').inert,hidden:$('filterEditor').hidden};"""
DRAG = """const h=document.getElementById('filterEditorHead'),r=h.getBoundingClientRect(),e=(type,x,y)=>h.dispatchEvent(new PointerEvent(type,{bubbles:true,cancelable:true,pointerId:1,button:0,clientX:x,clientY:y}));
const x=r.left+40,y=r.top+10;e('pointerdown',x,y);e('pointermove',x+arguments[0],y+arguments[1]);e('pointerup',x+arguments[0],y+arguments[1]);return true;"""


def ui_rows(lines):
    return [re.match(r'\S+ \S+\s+(\d+) \d+ (\w) (\S+)', line).groups() for line in lines]


def count(rows, shows=(), hides=()):
    """A condition is (levels, tag substring or None, pid or None)."""
    def match(c, row):
        levels, tag, pid = c
        return row[1] in levels and (tag is None or tag.lower() in row[2].lower()) and (pid is None or row[0] == pid)
    return sum(1 for row in rows if (not shows or any(match(c, row) for c in shows)) and not any(match(c, row) for c in hides))


ALL = 'VDIWEFAU'


def run(ctx):
    rows = ui_rows(fixtures.ui(ctx.fixtures))
    shutil.copy(ctx.fixtures / 'ui.log', ctx.fixtures / 'ui-copy.log')
    messages = [r for r in fixtures.dlt(ctx.fixtures) if 'apid' in r]
    check = ctx.check
    with ctx.launch() as app:
        js = app.js
        st = lambda: js(STATE)

        def item(kind, i, part='.cond-summary'):
            js(ITEM, kind, i, part)
            time.sleep(.3)

        def remove(kind, i):
            item(kind, i)
            app.click('removeCondition')
            time.sleep(.3)

        def levels(want):
            js(LEVELS, list(want))
            time.sleep(.35)

        app.open_fixture('ui.log')
        app.wait_count(3000)
        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.5)
        s = st()
        check('with no condition the list is empty and the editor closed',
              s['shows'] == [] and s['hides'] == [] and not s['editor'] and not app.hidden('conditionsEmpty'), s)
        app.capture('01-no-conditions')
        app.click('addCondition')
        time.sleep(.3)
        s, f, p = st(), js(FIELDS), js(PLACE)
        check('+ 추가 opens the editor on a new condition, beside the panel, with the fields of a text log',
              s['editor'] and s['title'] == '새 조건' and [c['summary'] for c in s['shows']] == ['새 조건'] and f['fields'] == ['tag', 'pid', 'tid', 'message']
              and f['focus'] == 'tagFilter' and p['position'] == 'fixed' and p['editor']['left'] >= p['panel']['right'], {'state': s, 'fields': f, 'place': p})
        clipped = js("""const c=document.createElement('canvas').getContext('2d');return [...document.querySelectorAll('#levelFilters .level-chip .letter')].filter(e=>{
          const s=getComputedStyle(e);c.font=`600 ${s.fontSize} ${s.fontFamily}`;return c.measureText(e.textContent).width+8>e.parentElement.clientWidth;}).map(e=>e.textContent);""")
        check('every level name fits its chip with room to spare, also in the bold of a level with rows', clipped == [], clipped)
        app.capture('01-one-condition')
        app.enter('tagFilter', 'ExampleApp')
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)]))
        app.add_condition()
        time.sleep(.3)
        levels('E')
        want = count(rows, [(ALL, 'ExampleApp', None), ('E', None, None)])
        app.wait_count(want)
        s = st()
        check('a second showing condition adds its rows (또는)',
              [c['summary'] for c in s['shows']] == ['태그: ExampleApp', '레벨: Error'] and s['shows'][1]['active'] == 'true'
              and s['chips'] == ['태그: ExampleApp×', '또는', '레벨: Error×'], s)

        app.add_condition(hide=True)
        time.sleep(.3)
        app.enter('tagFilter', 'ExampleApp')
        levels('VD')
        pressed = js("return [...document.querySelectorAll('#levelFilters [aria-pressed=true]')].map(b=>b.dataset.level).join('')")
        check('a hiding condition shows the levels it hides as pressed', pressed == 'VD', pressed)
        want = count(rows, [(ALL, 'ExampleApp', None), ('E', None, None)], [('VD', 'ExampleApp', None)])
        app.wait_count(want)
        s = st()
        check('a hiding condition with levels hides only those levels of its tag',
              [c['summary'] for c in s['hides']] == ['레벨: Verbose·Debug · 태그: ExampleApp'] and s['hideChecked'] == 'true'
              and s['chips'] == ['태그: ExampleApp×', '또는', '레벨: Error×', '제외', '레벨: Verbose·Debug×', '태그: ExampleApp×'] and s['badge'] == '3', s)
        app.capture('02-conditions')

        # The editor stays open over the log, moves by its heading and opens
        # again where it was left.
        js("document.querySelector('#logsRows .log-row').click();return true;")
        time.sleep(.3)
        check('clicking the log leaves the editor open', js(PLACE)['hidden'] is False, js(PLACE))
        before = js(PLACE)['editor']
        js(DRAG, 120, 80)
        time.sleep(.2)
        moved = js(PLACE)['editor']
        check('the heading moves the editor', moved['left'] == before['left'] + 120 and moved['top'] == before['top'] + 80, {'before': before, 'moved': moved})
        app.capture('02-editor-moved')
        app.click('closeEditor')
        time.sleep(.2)
        item('hide', 0)
        again = js(PLACE)
        check('it opens again where it was left', not again['hidden'] and again['editor']['left'] == moved['left'] and again['editor']['top'] == moved['top'], again)
        js(DRAG, -120, -80)
        time.sleep(.2)
        # One Escape closes one thing: the editor, then the panel.
        js("document.getElementById('autoHideFilters').click();return true;")
        time.sleep(.2)
        app.key('Escape', target='tagFilter')
        time.sleep(.2)
        p = js(PLACE)
        focus = js("return document.activeElement.className")
        check('Escape closes the editor first, the auto-hiding panel stays, and the condition takes focus', p['hidden'] and p['panelOpen'] and focus == 'cond-summary', {'place': p, 'focus': focus})
        js(ITEM, 'hide', 0, '.cond-summary')
        js("document.getElementById('autoHideFilters').click();return true;")
        time.sleep(.3)
        # The kind changes in place: the condition moves to the other kind.
        app.click('kindShow')
        time.sleep(.4)
        s = st()
        check('보기 turns the hiding condition into a showing one', [c['summary'] for c in s['shows']][-1] == '레벨: Verbose·Debug · 태그: ExampleApp' and s['hides'] == [], s)
        app.click('kindHide')
        time.sleep(.4)
        s = st()
        check('and 제외 turns it back', len(s['hides']) == 1 and s['hideChecked'] == 'true', s)
        app.wait_count(want)
        app.click('duplicateCondition')
        time.sleep(.3)
        s = st()
        check('복제 edits a copy', len(s['hides']) == 2 and s['hides'][1]['active'] == 'true' and s['title'] == '조건 편집', s)
        app.click('removeCondition')
        time.sleep(.3)
        check('삭제 takes the selected condition and closes its editor', len(st()['hides']) == 1 and not st()['editor'] and app.text('toast').startswith('제외 조건을 삭제했습니다'), st())

        # Conditions no row of this log matches say so in the list and on their
        # chips: a tag the log lacks, decided from the tables, and a time
        # outside the log, known once its rows are read. Network matches the
        # NetworkStack rows of the log, though none of them is shown here, so
        # it is not marked: the mark is about the log, not the result.
        app.add_condition(hide=True)
        time.sleep(.3)
        app.enter('tagFilter', 'Bluetooth')
        app.add_condition(hide=True)
        time.sleep(.3)
        app.enter('timeFrom', '20:00')
        app.enter('timeTo', '21:00')
        app.add_condition(hide=True)
        time.sleep(.3)
        app.enter('tagFilter', 'Network')
        app.close_editor()
        app.wait_count(want)
        app.wait(lambda: [c['rows'] for c in js(ROWLESS)['hides']] == ['', 'none', 'none', ''], what='rowless marks')
        m = js(ROWLESS)
        check('a tag and a time no row of the log matches are marked in the list; a condition with rows is not',
              [c['rows'] for c in m['shows']] == ['', ''] and m['hides'][1]['text'] == '태그: Bluetooth · 줄 없음' and not m['hides'][3]['text'].endswith('줄 없음')
              and m['hides'][1]['title'] == '태그: Bluetooth · 이 로그에 이 조건이 숨기는 줄이 없습니다', m)
        check('their chips are dashed and say 줄 없음',
              m['chips'] == ['태그: Bluetooth줄 없음×', '시간: 20:00 ~ 21:00줄 없음×'], m)
        app.capture('02-conditions-no-rows')
        for i in (3, 2, 1):
            remove('hide', i)
        app.wait_count(want)
        m = js(ROWLESS)
        check('removing them leaves the earlier conditions unmarked', [c['rows'] for c in m['hides']] == [''] and m['chips'] == [], m)
        js(PIN, True)
        app.click('collapseFilters')
        time.sleep(.6)
        app.capture('03-conditions-chips')
        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.5)

        # A switched-off condition stays in the panel but filters nothing.
        item('show', 1, '.cond-toggle')
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)], [('VD', 'ExampleApp', None)]))
        s = st()
        check('switching a condition off removes its rows and chip, and dims its item',
              s['shows'][1]['off'] == 'true' and '레벨: Error×' not in s['chips'], s)
        app.capture('04-condition-off')
        item('show', 1, '.cond-toggle')
        app.wait_count(want)

        # A search looks inside the conditions.
        app.type('searchInput', 'timeout')
        expect = sum(1 for i, row in enumerate(rows)
                     if ((('exampleapp' in row[2].lower()) or row[1] == 'E') and not (row[1] in 'VD' and 'exampleapp' in row[2].lower()))
                     and (i % 7) == 4)
        app.wait(lambda: st()['search'] == f'{expect:,}건' or st()['search'] == str(expect), what=f'{expect} results')
        check('a search finds rows within the conditions', True, st()['search'])
        app.type('searchInput', '')

        # The row menu adds a showing condition from a row.
        js("const r=document.querySelector('#logsRows .log-row');r.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:300,clientY:300}));return true;")
        time.sleep(.3)
        labels = js("return [...document.querySelectorAll('#rowMenu [role=menuitem]')].map(e=>e.textContent)")
        check('the row menu makes a condition of the row, then offers its tag', labels[:2] == ['이 행으로 조건 추가…', "태그 'ExampleApp' 보기 조건 추가"], labels)
        js("document.getElementById('rowMenu').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;")

        # Save this filter, then find it after a restart on another file.
        app.click('panelSavedFilter')
        app.wait(lambda: js("return !document.getElementById('rowMenu').hidden"), what='menu')
        js("[...document.querySelectorAll('#rowMenu [role^=menuitem]')].find(e=>e.textContent==='현재 필터 저장…').click();return true;")
        app.wait(lambda: js("return document.getElementById('filterNameDialog').open"), what='dialog')
        js("const i=document.getElementById('filterNameInput');i.value='ExampleApp 또는 Error';document.getElementById('filterNameForm').requestSubmit();return true;")
        app.wait(lambda: not js("return document.getElementById('filterNameDialog').open"), what='saved')
        stored = json.loads((ctx.data / 'saved-filters.json').read_text())[0]['filter']
        check('a saved filter keeps every condition',
              stored == {'v': 2, 'include': [{'tag': 'ExampleApp'}, {'levels': ['E']}], 'exclude': [{'levels': ['V', 'D'], 'tag': 'ExampleApp'}], 'includeRaw': True}, stored)
        time.sleep(1)

    with ctx.launch() as app:
        js = app.js
        st = lambda: js(STATE)
        # The file's saved work brings its conditions back after a restart.
        app.open_fixture('ui.log')
        app.wait_count(want)
        s = st()
        check('after a restart the same file returns with every condition of its work',
              [c['summary'] for c in s['shows']] == ['태그: ExampleApp', '레벨: Error'] and [c['summary'] for c in s['hides']] == ['레벨: Verbose·Debug · 태그: ExampleApp'], s)
        app.open_fixture('ui-copy.log')
        app.wait_count(3000)
        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.5)
        app.click('panelSavedFilter')
        app.wait(lambda: js("return !document.getElementById('rowMenu').hidden"), what='menu')
        js("[...document.querySelectorAll('#rowMenu [role^=menuitem]')].find(e=>e.textContent==='ExampleApp 또는 Error').click();return true;")
        app.wait_count(want)
        s = st()
        check('after a restart the saved filter brings back every condition on another file',
              [c['summary'] for c in s['shows']] == ['태그: ExampleApp', '레벨: Error'] and [c['summary'] for c in s['hides']] == ['레벨: Verbose·Debug · 태그: ExampleApp'], s)
        # Removing conditions from the list.
        for kind, i in (('hide', 0), ('show', 1)):
            js(ITEM, kind, i, '.cond-summary')
            time.sleep(.3)
            app.click('removeCondition')
            time.sleep(.3)
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)]))
        s = st()
        check('removing them leaves one showing condition', len(s['shows']) == 1 and s['hides'] == [] and s['tag'] == 'ExampleApp', s)
        app.enter('tagFilter', 'Bluetooth')
        app.wait(lambda: js(ROWLESS)['shows'][0]['rows'] == 'none', what='lone rowless note')
        m = js(ROWLESS)
        check('a lone showing condition without rows says so', m['shows'][0]['text'] == '태그: Bluetooth · 줄 없음', m)
        app.capture('06-lone-condition-no-rows')
        app.enter('tagFilter', 'ExampleApp')
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)]))
        # The count shows before the rows are drawn.
        app.wait(lambda: js("return !!document.querySelector('#logsRows .log-row')"), what='rows of the new count')
        # A row makes a condition of its values in the editor.
        app.close_editor()
        js("const r=document.querySelector('#logsRows .log-row');r.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:300,clientY:300}));return true;")
        time.sleep(.3)
        js("[...document.querySelectorAll('#rowMenu [role=menuitem]')].find(e=>e.textContent==='이 행으로 조건 추가…').click();return true;")
        time.sleep(.5)
        s, f = st(), js(FIELDS)
        check('이 행으로 조건 추가 opens the editor on the condition there is, not a copy',
              len(s['shows']) == 1 and s['editor'] and s['title'] == '조건 편집' and s['tag'] == 'ExampleApp' and f['focus'] == 'tagFilter', {'state': s, 'fields': f})
        app.capture('06-row-menu-condition')
        app.click('closeEditor')
        time.sleep(.2)

        app.open_fixture('vehicle.dlt')
        app.wait(lambda: st()['count'].startswith('로그 · 전체'), what='dlt rows')
        app.add_condition()
        time.sleep(.3)
        f = js(FIELDS)
        check('a DLT condition has the IDs apart and no PID, TID or tag', f['fields'] == ['ecu', 'apid', 'ctid', 'message'], f)
        app.enter('apidFilter', 'NAV')
        app.enter('ctidFilter', 'GPS')
        shown = [m for m in messages if m['apid'] == 'NAV' and m['ctid'] == 'GPS']
        app.wait_count(len(shown))
        check('an APID and a CTID combine with AND', st()['chips'] == ['APID: NAV×', 'CTID: GPS×'], st())
        app.enter('ctidFilter', '')
        app.add_condition()
        time.sleep(.3)
        js(LEVELS, ['E', 'F'])
        time.sleep(.3)
        app.add_condition(hide=True)
        time.sleep(.3)
        app.enter('ecuFilter', 'TCU')
        shown = [m for m in messages if (m['apid'] == 'NAV' or m['level'] in ('Error', 'Fatal')) and m['ecu'] != 'TCU']
        app.wait_count(len(shown))
        check('DLT conditions: an APID or Error and Fatal, without an ECU', True, {'count': app.count(), 'expected': len(shown)})
        app.capture('05-dlt-conditions')
