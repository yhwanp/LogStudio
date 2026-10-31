"""The filter switched off as a whole: every row shows while the conditions
stay as chips, the selected row and its detail stay, the search finds in the
rows shown, a changed condition or the button or the shortcut switches it on,
and a reload keeps it off (ui.log)."""
import time

import fixtures

STATE = """const $=id=>document.getElementById(id),sel=()=>document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null,
 at=line=>{const r=document.querySelector(`#logsRows [data-line-number="${line}"]`);return r?Math.round(r.getBoundingClientRect().top-r.closest('.large-viewport').getBoundingClientRect().top):null;};
return {shown:!$('filtersSwitch').hidden,on:$('filtersOn').checked,note:$('filtersOffNote').textContent,noteClass:$('filtersOffNote').className,
 row:$('filterRow').hidden,chips:[...$('activeFilters').querySelectorAll('.chip-joint,.filter-chip')].map(e=>e.textContent),badge:$('filterBadge').textContent,
 selected:sel(),top:at(sel()),detail:$('largeDetail').hidden?null:$('largeDetailTitle').textContent,toast:$('toast').hidden?null:$('toast').textContent,
 searchCount:$('largeCount').textContent,scope:$('largeTotal').textContent,hint:!$('filtersOffHint').hidden,saved:$('panelSavedFilter').dataset.state,placeholder:$('searchInput').placeholder,chipColor:getComputedStyle($('activeFilters').querySelector('.filter-chip')??document.body).color,
 muted:getComputedStyle($('filterRow')).getPropertyValue('--muted'),label:$('toggleFilters').getAttribute('aria-label')};"""
OPEN_PANEL = "const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;"
PIN = "const a=document.getElementById('autoHideFilters');a.checked=arguments[0];a.dispatchEvent(new Event('change',{bubbles:true}));return true;"
FILTERS = 428  # Rows of WifiMonitor without 'timeout' in the message.


def run(ctx):
    fixtures.ui(ctx.fixtures)
    check = ctx.check
    with ctx.launch() as app:
        js = app.js
        st = lambda: js(STATE)
        app.open_fixture('ui.log')
        app.wait_count(3000)
        s = st()
        check('without conditions there is nothing to switch off', not s['shown'] and s['row'] is True, s)
        app.key('F', meta=True, shift=True, code='KeyF', target='logsViewport')
        time.sleep(.3)
        check('without conditions the shortcut does nothing', st()['on'], st())

        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.3)
        app.enter('tagFilter', 'WifiMonitor')
        app.wait_count(500)
        app.click('addHide')
        time.sleep(.2)
        app.click('openMessage')
        app.enter('messageFilter', 'timeout')
        app.wait_count(FILTERS)
        js(PIN, True)
        js("document.getElementById('collapseFilters').click();return true;")
        app.jump(1001)
        time.sleep(.4)
        app.click_row(1001)
        app.wait(lambda: st()['detail'] == '1,001행 원문', what='detail of row 1001')
        before = st()
        check('with conditions the row has the switch, on', before['shown'] and before['on'] and before['note'] == '', before)
        app.capture('01-filter-on')

        app.click('filtersOn')
        app.wait_count(3000)
        app.wait(lambda: st()['selected'] == '1001', what='row 1001 selected')
        time.sleep(.3)
        s = st()
        check('switched off, every row shows and the selected row, its height and detail stay',
              s['top'] == before['top'] and s['detail'] == '1,001행 원문', {'before': before, 'after': s})
        check('the conditions stay as chips, out of use', s['chips'] == before['chips'] and s['badge'] == before['badge'] and s['chipColor'] != before['chipColor'], s)
        check('the row says the filter is off, and the check box shows it',
              not s['on'] and s['note'] == '필터 꺼짐 · 모든 줄 표시' and s['noteClass'] == 'filters-off-note' and '꺼 둠' in s['label'], s)
        check('the filter on and off leaves no toast', s['toast'] is None, s)
        app.capture('02-filter-off')

        # A row the filter hid, chosen while it is off, gives way when it is on.
        app.click_row(1000)
        app.wait(lambda: st()['detail'] == '1,000행 원문', what='detail of row 1000')
        app.key('F', meta=True, shift=True, code='KeyF', target='logsViewport')
        app.wait_count(FILTERS)
        app.wait(lambda: st()['selected'] == '1001', what='row 1001 stands in')
        s = st()
        check('switched on, a row the filter hides gives its place to the next result and says so',
              s['on'] and s['toast'] == '1,000행은 새 필터 결과에 없어 다음 결과인 1,001행을 선택했습니다.', s)
        check('a screen reader is told the filter is on', s['note'] == '필터 켜짐' and s['noteClass'] == 'visually-hidden', s)

        # The search finds in the rows shown.
        app.type('searchInput', 'Network connected')
        app.wait(lambda: st()['searchCount'] == '72건', what='72 search results')
        check('the search count says it counts the filter results', st()['scope'].startswith(' · 필터 결과'), st())
        app.click('filtersOn')
        app.wait(lambda: st()['searchCount'] == '429건', what='429 search results')
        s = st()
        check('switched off, the search finds in every row and says so', s['placeholder'] == '모든 줄의 원문에서 검색' and s['scope'].startswith(' · 모든 줄'), s)
        app.capture('03-search-with-filter-off')
        app.type('searchInput', 'Cache lookup')
        app.wait(lambda: st()['searchCount'] == '428건', what='428 search results')
        check('a new search leaves the filter off', not st()['on'], st())
        app.type('searchInput', '')
        time.sleep(.3)

        # A reload keeps it off; a changed condition switches it on.
        app.click('reopenFile')
        app.wait(lambda: '다시 열었습니다' in (st()['toast'] or ''), what='reload notice')
        app.wait_count(3000)
        s = st()
        check('a reload keeps the filter off and its conditions', not s['on'] and s['chips'] == before['chips'], s)
        js(OPEN_PANEL)
        time.sleep(.3)
        js("document.querySelector('#showList .cond-summary')?.click();return true;")
        time.sleep(.2)
        check('the panel says the filter is off and how it comes back', st()['hint'], st())
        app.capture('05-panel-with-filter-off')
        app.enter('tagFilter', 'WifiMonitor, RenderThread')
        app.wait(lambda: st()['on'], what='filter on again')
        app.wait(lambda: app.count() != '로그 · 전체 3,000건', what='conditions applied')
        s = st()
        check('a changed condition switches the filter on and says so with an undo',
              s['chips'][0] == '태그: WifiMonitor, RenderThread×' and (s['toast'] or '').startswith('조건이 바뀌어 필터를 켰습니다.'), s)
        app.capture('06-filter-on-again')
        js("document.querySelector('#toast .toast-action').click();return true;")
        app.wait(lambda: not st()['on'], what='filter off after the undo')
        app.wait_count(3000)
        s = st()
        check('its undo puts the condition back and the filter off', s['chips'][0] == '태그: WifiMonitor×', s)
        app.enter('tagFilter', 'WifiMonitor, RenderThread')
        app.wait(lambda: st()['on'], what='filter on again')

        # A long filter wraps under the button and the note, at 760 px too.
        app.click('openIds')
        app.enter('pidFilter', '2983, 12540')
        app.wait(lambda: len(st()['chips']) == 4, what='a third chip')
        js(PIN, True)
        js("document.getElementById('collapseFilters').click();return true;")
        app.click('filtersOn')
        app.wait_count(3000)
        frame = app.rect()
        app.rect(width=760, height=frame['height'])
        time.sleep(.5)
        fit = js("""const r=document.getElementById('filterRow'),b=document.getElementById('filtersSwitch').getBoundingClientRect(),n=document.getElementById('filtersOffNote').getBoundingClientRect();
          return {overflow:r.scrollWidth>r.clientWidth,button:Math.round(b.height),inRow:b.right<=r.getBoundingClientRect().right&&n.right<=r.getBoundingClientRect().right};""")
        check('in a 760 px window the button, the note and the chips fit their row', not fit['overflow'] and fit['inRow'], fit)
        app.capture('04-filter-off-760')
        app.rect(width=frame['width'], height=frame['height'])
        time.sleep(.3)

        # A condition that cannot apply: off still works, on says why.
        js(OPEN_PANEL)
        time.sleep(.3)
        app.enter('pidFilter', 'abc')
        time.sleep(.3)
        check('a wrong field does not switch the filter on', not st()['on'], st())
        app.click('filtersOn')
        time.sleep(.3)
        s = st()
        check('the check box stays off and the toast says why', not s['on'] and s['toast'] == '조건에 오류가 있어 필터를 켜지 못했습니다.', s)
        check('the field says what is wrong', js("return !document.getElementById('idFilterError').hidden"), None)
        app.enter('pidFilter', '2983, 12540')
        time.sleep(.3)

        # Taking away the last condition leaves no filter to switch on.
        js("document.getElementById('resetFilters').click();return true;")
        app.wait_count(3000)
        app.enter('tagFilter', 'WifiMonitor')
        app.wait_count(500)
        app.click('filtersOn')
        app.wait_count(3000)
        js("document.querySelector('#activeFilters .filter-chip').click();return true;")
        app.wait(lambda: not st()['shown'], what='the switch goes with the last chip')
        s = st()
        check('the last chip says it is gone, not that the filter is on', (s['toast'] or '').startswith('마지막 조건을 해제했습니다.'), s)
        app.capture('07-last-condition')

        help_open = "document.getElementById('shortcutHelp').click();return document.getElementById('shortcutDialog').textContent;"
        check('the shortcut list names the switch', '필터 끄기·켜기' in js(help_open), None)
