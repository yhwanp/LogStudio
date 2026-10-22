"""Filters with several conditions: the panel of one condition, showing
conditions joined by 또는, a hiding condition with levels, cards and their
switches, the chips reading as the filter, the row menu adding a showing
condition, a search inside the conditions, a saved filter of several
conditions after a restart, PID·TID and time fields behind + buttons until
used, conditions no row of the log matches, and DLT conditions (ui.log,
ui-copy.log, vehicle.dlt; counts from a row-at-a-time oracle)."""
import json
import re
import shutil
import time

import fixtures

STATE = """const $=id=>document.getElementById(id),cards=id=>[...$(id).children].map(c=>({summary:c.querySelector('.cond-summary').textContent,
 head:getComputedStyle(c.querySelector('.cond-head')).display!=='none',active:c.dataset.active,off:c.dataset.off,editing:c.contains($('condEditor'))}));
return {shows:cards('showList'),hides:cards('hideList'),chips:[...$('activeFilters').querySelectorAll('.chip-joint,.filter-chip')].map(e=>e.textContent),badge:$('filterBadge').hidden?null:$('filterBadge').textContent,
 showHint:$('showHint').textContent,hideHint:$('hideHint').textContent,tag:$('tagFilter').value,count:$('logsCount').textContent,search:$('largeCount').textContent};"""
LEVELS = """const want=new Set(arguments[0]);for(const b of document.querySelectorAll('#levelFilters .level-chip')){
const on=b.getAttribute('aria-pressed')==='true';if(on!==want.has(b.dataset.level))b.click();}return true;"""
CARD = "return document.querySelectorAll(`#${arguments[0]} .cond-card`)[arguments[1]].querySelector(arguments[2]).click(),true;"
PIN = "const a=document.getElementById('autoHideFilters');a.checked=arguments[0];a.dispatchEvent(new Event('change',{bubbles:true}));return true;"
ROWLESS = """const $=id=>document.getElementById(id),cards=id=>[...$(id).children].map(c=>({rows:c.dataset.rows,note:!c.querySelector('.cond-alert').hidden,
 text:c.querySelector('.cond-alert').textContent,title:c.querySelector('.cond-alert').title}));return {shows:cards('showList'),hides:cards('hideList'),
 chips:[...$('activeFilters').querySelectorAll('.filter-chip')].filter(e=>e.classList.contains('no-rows')).map(e=>e.textContent)};"""
OPEN_PANEL = "const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;"
FIELDS = """const $=id=>document.getElementById(id),panel=$('filterPanel');return {ids:!$('idFields').hidden,time:!$('timeFields').hidden,
 openIds:!$('openIds').hidden,openTime:!$('openTime').hidden,focus:document.activeElement?.id??null,pid:$('pidFilter').value,
 rawShown:$('includeRaw').getBoundingClientRect().bottom<=panel.getBoundingClientRect().bottom,scrolled:panel.scrollTop};"""


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

        def card(list_id, i, part):
            js(CARD, list_id, i, part)
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
        check('one showing condition reads as one set of fields: no card header, no hiding condition',
              len(s['shows']) == 1 and not s['shows'][0]['head'] and s['shows'][0]['editing'] and s['hides'] == [] and s['showHint'] == '' and s['hideHint'] == '', s)
        f = js(FIELDS)
        check('an empty condition shows levels and tag; PID·TID and time wait behind + buttons',
              not f['ids'] and not f['time'] and f['openIds'] and f['openTime'], f)
        app.capture('01-one-condition', hover='openIds')

        app.enter('tagFilter', 'ExampleApp')
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)]))
        app.click('addShow')
        time.sleep(.3)
        levels('E')
        want = count(rows, [(ALL, 'ExampleApp', None), ('E', None, None)])
        app.wait_count(want)
        s = st()
        check('a second showing condition adds its rows (또는) and both cards show headers',
              [c['summary'] for c in s['shows']] == ['태그: ExampleApp', '레벨: Error'] and all(c['head'] for c in s['shows'])
              and s['shows'][1]['editing'] and s['showHint'] == '하나라도 맞으면 보입니다' and s['chips'] == ['태그: ExampleApp×', '또는', '레벨: Error×'], s)

        app.click('addHide')
        time.sleep(.3)
        app.enter('tagFilter', 'ExampleApp')
        levels('VD')
        want = count(rows, [(ALL, 'ExampleApp', None), ('E', None, None)], [('VD', 'ExampleApp', None)])
        app.wait_count(want)
        s = st()
        check('a hiding condition with levels hides only those levels of its tag',
              [c['summary'] for c in s['hides']] == ['레벨: Verbose·Debug · 태그: ExampleApp'] and s['hideHint'] == '맞는 줄을 숨깁니다'
              and s['chips'] == ['태그: ExampleApp×', '또는', '레벨: Error×', '제외', '레벨: Verbose·Debug×', '태그: ExampleApp×'] and s['badge'] == '4', s)
        f = js(FIELDS)
        check('with three conditions 미인식 줄 포함 is in view without scrolling the panel', f['rawShown'] and f['scrolled'] == 0, f)
        app.capture('02-conditions')
        app.click('openIds')
        time.sleep(.2)
        f = js(FIELDS)
        check('+ PID·TID opens the PID and TID fields of the edited condition and focuses PID',
              f['ids'] and not f['openIds'] and f['openTime'] and not f['time'] and f['focus'] == 'pidFilter', f)
        app.capture('02-pid-tid-opened')
        card('showList', 0, '.cond-summary')
        card('hideList', 0, '.cond-summary')
        f = js(FIELDS)
        check('PID and TID left empty fold back when the condition is edited again', not f['ids'] and f['openIds'], f)

        # Conditions no row of this log matches say so on their cards and
        # chips: a tag the log lacks, decided from the tables, and a time
        # outside the log, known once its rows are read. Network matches the
        # NetworkStack rows of the log, though none of them is shown here, so
        # it is not marked: the mark is about the log, not the result.
        app.click('addHide')
        time.sleep(.3)
        app.enter('tagFilter', 'Bluetooth')
        app.click('addHide')
        time.sleep(.3)
        app.click('openTime')
        time.sleep(.2)
        app.enter('timeFrom', '20:00')
        app.enter('timeTo', '21:00')
        app.click('addHide')
        time.sleep(.3)
        app.enter('tagFilter', 'Network')
        app.wait_count(want)
        app.wait(lambda: [c['rows'] for c in js(ROWLESS)['hides']] == ['', 'none', 'none', ''], what='rowless marks')
        m = js(ROWLESS)
        check('a tag and a time no row of the log matches are marked on their cards; a condition with rows is not',
              [c['note'] for c in m['hides']] == [False, True, True, False] and [c['rows'] for c in m['shows']] == ['', '']
              and m['hides'][1]['text'] == '이 로그에 맞는 줄 없음'
              and m['hides'][1]['title'] == '이 로그에 이 조건과 맞는 줄이 없습니다 · 이 조건이 숨기는 줄이 없습니다', m)
        check('their chips are dashed and say 줄 없음',
              m['chips'] == ['태그: Bluetooth줄 없음×', '시간: 20:00 ~ 21:00줄 없음×'], m)
        app.capture('02-conditions-no-rows')
        for i in (3, 2, 1):
            js(CARD, 'hideList', i, '.cond-remove')
            time.sleep(.3)
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
        card('showList', 1, '.cond-toggle')
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)], [('VD', 'ExampleApp', None)]))
        s = st()
        check('switching a condition off removes its rows and chip, and dims its card',
              s['shows'][1]['off'] == 'true' and '레벨: Error×' not in s['chips'], s)
        app.capture('04-condition-off')
        card('showList', 1, '.cond-toggle')
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
        item = js("return [...document.querySelectorAll('#rowMenu [role=menuitem]')].map(e=>e.textContent).find(t=>t.endsWith('보기 조건 추가'))")
        check('the row menu offers a showing condition for the row tag', item == "태그 'ExampleApp' 보기 조건 추가", item)
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
              stored == {'v': 1, 'include': [{'tag': 'ExampleApp'}, {'levels': ['E']}], 'exclude': [{'levels': ['V', 'D'], 'tag': 'ExampleApp'}], 'includeRaw': True}, stored)
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
        # Removing conditions with their cards.
        js(CARD, 'hideList', 0, '.cond-remove')
        time.sleep(.3)
        js(CARD, 'showList', 1, '.cond-remove')
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)]))
        s = st()
        check('removed cards leave one showing condition in the fields', len(s['shows']) == 1 and not s['shows'][0]['head'] and s['hides'] == [] and s['tag'] == 'ExampleApp', s)
        # A lone showing condition has no card header: its note is a sentence above the fields.
        app.enter('tagFilter', 'Bluetooth')
        app.wait(lambda: js(ROWLESS)['shows'][0]['rows'] == 'none', what='lone rowless note')
        m = js(ROWLESS)
        check('a lone showing condition without rows says so above its fields',
              m['shows'][0]['note'] and m['shows'][0]['text'] == '이 로그에 이 조건과 맞는 줄이 없습니다 · 이 조건으로 보이는 줄이 없습니다.', m)
        app.capture('06-lone-condition-no-rows')
        app.enter('tagFilter', 'ExampleApp')
        app.wait_count(count(rows, [(ALL, 'ExampleApp', None)]))
        # The count shows before the rows are drawn.
        app.wait(lambda: js("return !!document.querySelector('#logsRows .log-row')"), what='rows of the new count')
        # A row value lands in a new condition with its field in view.
        js("const r=document.querySelector('#logsRows .log-row');r.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:300,clientY:300}));return true;")
        time.sleep(.3)
        js("[...document.querySelectorAll('#rowMenu [role=menuitem]')].find(e=>/^PID \\d+ 보기 조건 추가$/.test(e.textContent)).click();return true;")
        time.sleep(.5)
        s, f = st(), js(FIELDS)
        check('a PID from the row menu shows the PID field of the new condition',
              len(s['shows']) == 2 and s['shows'][1]['editing'] and s['shows'][1]['summary'] == f"PID: {f['pid']}" and f['ids'] and not f['time'] and f['openTime'], {'state': s, 'fields': f})
        app.capture('06-row-menu-pid')

        app.open_fixture('vehicle.dlt')
        app.wait(lambda: st()['count'].startswith('로그 · 전체'), what='dlt rows')
        f = js(FIELDS)
        check('a DLT condition offers + 시간 but no PID·TID', not f['ids'] and not f['openIds'] and f['openTime'], f)
        app.enter('tagFilter', 'NAV')
        app.click('addShow')
        time.sleep(.3)
        js(LEVELS, ['E', 'F'])
        time.sleep(.3)
        app.click('addHide')
        time.sleep(.3)
        app.enter('tagFilter', 'TCU')
        shown = [m for m in messages if (m['apid'] == 'NAV' or m['level'] in ('Error', 'Fatal')) and not (m['ecu'] == 'TCU' or m['apid'] == 'TCU' or m['ctid'] == 'TCU')]
        app.wait_count(len(shown))
        check('DLT conditions: an APID or Error and Fatal, without an ECU', True, {'count': app.count(), 'expected': len(shown)})
        app.capture('05-dlt-conditions')
