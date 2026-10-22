"""Saved filters: saving the filter under a name from the panel, the marks in
the log heading and the panel, a changed filter saved over, reverted and
undone, applying a saved filter to another file, keeping it across a restart
with the file's work, renaming, a name in use, deleting with undo, Escape in
the name dialog, and a DLT file that cannot use text-log filters (ui.log,
ui-copy.log, vehicle.dlt)."""
import json
import re
import shutil
import time

import fixtures

STATE = """const $=id=>document.getElementById(id),b=$('savedFilterButton'),p=$('panelSavedFilter'),m=$('rowMenu'),d=$('filterNameDialog'),t=$('toast');
const label=e=>[...e.children].filter(c=>!c.hidden).map(c=>c.textContent).join('|');return {
 heading:label(b),headingState:b.dataset.state,aria:b.getAttribute('aria-label'),panel:label(p),panelState:p.dataset.state,
 save:$('saveFilter').hidden?null:$('saveFilter').textContent,tag:$('tagFilter').value,hides:[...document.querySelectorAll('#hideList .cond-summary')].map(e=>e.textContent),query:$('searchInput').value,
 count:$('logsCount').textContent,
 menu:m.hidden?null:[...m.children].map(c=>c.tagName==='HR'?'---':c.tagName==='P'?`(${c.textContent})`:(c.getAttribute('aria-checked')==='true'?'* ':'')+c.textContent),
 dialog:d.open,title:$('filterNameTitle').textContent,name:$('filterNameInput').value,error:$('filterNameError').hidden?null:$('filterNameError').textContent,
 hint:$('filterNameHint').hidden?null:$('filterNameHint').textContent,focus:document.activeElement?.id||null,
 toast:t.hidden?null:t.textContent,action:t.querySelector('.toast-action')?.textContent??null,detail:!$('largeDetail').hidden};"""
PICK = """const i=[...document.querySelectorAll('#rowMenu [role^=menuitem]')].find(e=>e.textContent===arguments[0]);if(!i)return false;i.click();return true;"""
SUBMIT = "const i=document.getElementById('filterNameInput');i.value=arguments[0];i.dispatchEvent(new Event('input',{bubbles:true}));document.getElementById('filterNameForm').requestSubmit();return true;"
PIN = "const a=document.getElementById('autoHideFilters');a.checked=arguments[0];a.dispatchEvent(new Event('change',{bubbles:true}));return true;"
OPEN_PANEL = "const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;"


def oracle(lines):
    rows = [re.match(r'\S+ \S+\s+\d+ \d+ (\w) (\S+)', line).groups() for line in lines]

    def expected(tags=(), levels='VDIWEFAU', exclude=()):
        return sum(1 for level, tag in rows if level in levels and (not tags or any(t.lower() in tag.lower() for t in tags))
                   and not any(t.lower() in tag.lower() for t in exclude))
    return expected


def run(ctx):
    expected = oracle(fixtures.ui(ctx.fixtures))
    shutil.copy(ctx.fixtures / 'ui.log', ctx.fixtures / 'ui-copy.log')
    fixtures.dlt(ctx.fixtures)
    store = ctx.data / 'saved-filters.json'
    check = ctx.check
    first = '경고 이상 ExampleApp'
    with ctx.launch() as app:
        js = app.js
        st = lambda: js(STATE)
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(st()), timeout, what=what)

        def menu(button='panelSavedFilter'):
            app.click(button)
            return until(lambda s: s['menu'] is not None, 'saved filter menu')['menu']

        def pick(label):
            check(f'the menu offers {label}', app.wait(lambda: js(PICK, label), what=label))
            time.sleep(.3)

        app.open_fixture('ui.log')
        app.wait_count(3000)
        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.4)
        s = st()
        check('with no saved filter the buttons say so and nothing can be saved',
              s['heading'] == '저장된 필터' and s['headingState'] == 'none' and s['panel'] == '저장된 필터' and s['save'] is None, s)
        check('an empty list explains how to save', menu() == ['(저장된 필터가 없습니다. 조건을 정한 뒤 저장할 수 있습니다)'], st()['menu'])
        app.key('Escape')
        time.sleep(.2)
        check('Escape closes a menu of notes only, not the pinned panel',
              st()['menu'] is None and js("return document.getElementById('toggleFilters').getAttribute('aria-expanded')") == 'true', st()['menu'])

        app.enter('tagFilter', 'ExampleApp')
        app.click('errorLevels')
        want = expected(['ExampleApp'], 'WEFA')
        app.wait_count(want)
        check('a filter with conditions can be saved from the panel', st()['save'] == '저장', st())
        # A click does not focus a button in WebKit, so focus starts on the page.
        js("document.activeElement.blur();return true;")
        app.click('saveFilter')
        s = until(lambda s: s['dialog'], 'name dialog')
        check('the name dialog suggests a name from the conditions and says what is saved',
              s['title'] == '필터 저장' and s['name'] == '레벨: Warn 이상 · 태그: ExampleApp' and s['focus'] == 'filterNameInput'
              and s['hint'] == '저장할 조건: 레벨: Warn 이상 · 태그: ExampleApp. 검색어는 저장하지 않습니다.', s)
        app.capture('01-save-dialog')
        js(SUBMIT, '   ')
        s = until(lambda s: s['error'], 'empty name error')
        check('an empty name is refused in the dialog', s['dialog'] and s['error'] == '이름을 입력해 주세요.', s)
        js(SUBMIT, first)
        s = until(lambda s: not s['dialog'], 'saved')
        check('saving names the filter in the heading and the panel',
              s['heading'] == first and s['headingState'] == 'applied' and s['panel'] == first and s['save'] is None
              and s['toast'] == f"'{first}' 필터를 저장했습니다." and s['focus'] == 'panelSavedFilter', s)
        disk = json.loads(store.read_text())
        check('saved-filters.json holds the name, format and conditions',
              len(disk) == 1 and disk[0]['name'] == first and disk[0]['format'] == 'text'
              and disk[0]['filter'] == {'v': 1, 'include': [{'levels': ['W', 'E', 'F', 'A'], 'tag': 'ExampleApp'}], 'exclude': [], 'includeRaw': True}, disk)

        app.enter('tagFilter', 'ExampleApp, WifiMonitor')
        app.wait_count(expected(['ExampleApp', 'WifiMonitor'], 'WEFA'))
        s = until(lambda s: s['headingState'] == 'modified', 'modified mark')
        check('changing the conditions marks the saved filter as changed',
              s['heading'] == f'{first}|수정됨' and s['panel'] == f'{first}|수정됨' and s['aria'] == f'저장된 필터: {first}, 수정됨' and s['save'] == '저장', s)
        app.capture('02-modified')
        line = "const h=document.querySelector('.logs-main-pane .pane-heading'),t=e=>Math.round(e.getBoundingClientRect().top);return [t(document.getElementById('logsCount')),t(h.querySelector('.table-tools')),getComputedStyle(document.getElementById('savedFilterButton')).display];"
        top, tools, shown = js(line)
        check('with the panel open its button stands in for the heading one, and the heading stays one line', shown == 'none' and abs(top - tools) <= 4, [top, tools, shown])
        js(PIN, True)
        js("document.getElementById('collapseFilters').click();return true;")
        time.sleep(.6)
        top, tools, shown = js(line)
        check('with the panel closed the heading shows the saved filter on one line', shown != 'none' and abs(top - tools) <= 4, [top, tools, shown])
        app.capture('02b-modified-heading')
        menu('savedFilterButton')
        place = js("const b=document.getElementById('savedFilterButton').getBoundingClientRect(),m=document.getElementById('rowMenu').getBoundingClientRect();return [Math.round(m.left-b.left),Math.round(m.top-b.bottom)];")
        check('the heading button opens the menu under itself', place[0] == 0 and 0 <= place[1] <= 8, place)
        app.key('Escape')
        time.sleep(.2)
        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.6)
        items = menu('panelSavedFilter')
        check('the menu marks the applied filter and offers what fits a changed one',
              items == [f'* {first}', '---', f"'{first}'에 저장", '저장한 조건으로 되돌리기', '다른 이름으로 저장…', '이름 바꾸기…', f"'{first}' 삭제"], items)
        check('the applied filter has focus in the menu', st()['focus'] is None or js("return document.activeElement.getAttribute('aria-checked')") == 'true')
        app.capture('03-menu')
        pick('저장한 조건으로 되돌리기')
        app.wait_count(want)
        s = st()
        check('reverting puts back the saved conditions and offers to undo',
              s['tag'] == 'ExampleApp' and s['headingState'] == 'applied' and s['action'] == '실행 취소'
              and s['toast'] == f"'{first}'에 저장한 조건으로 되돌렸습니다.실행 취소", s)
        app.capture('04-undo-toast', hover=None)
        js("document.querySelector('#toast .toast-action').click();return true;")
        app.wait_count(expected(['ExampleApp', 'WifiMonitor'], 'WEFA'))
        check('undo returns the changed conditions', until(lambda s: s['headingState'] == 'modified', 'undone')['tag'] == 'ExampleApp, WifiMonitor')
        app.click('saveFilter')
        s = until(lambda s: s['headingState'] == 'applied', 'saved over')
        check('the panel saves a changed filter over the saved one', s['toast'] == f"'{first}'에 저장했습니다." and s['save'] is None, s)
        check('the file holds the new conditions', json.loads(store.read_text())[0]['filter']['include'][0]['tag'] == 'ExampleApp, WifiMonitor')

        # Another file opens without a filter; the saved one applies in one pick.
        app.enter('searchInput', 'timeout')
        app.open_fixture('ui-copy.log')
        app.wait_count(3000)
        s = st()
        check('another file starts with no filter', s['heading'] == '저장된 필터' and s['tag'] == '' and s['headingState'] == 'none', s)
        app.type('searchInput', 'timeout')
        check('the menu lists the saved filter to apply; an empty filter offers no save', menu() == [first], st()['menu'])
        pick(first)
        app.wait_count(expected(['ExampleApp', 'WifiMonitor'], 'WEFA'))
        s = st()
        check('picking it applies its conditions and keeps the search',
              s['tag'] == 'ExampleApp, WifiMonitor' and s['query'] == 'timeout' and s['heading'] == first
              and s['toast'] == f"'{first}' 필터를 적용했습니다.실행 취소", s)
        time.sleep(1)

    # A restart keeps the list, and the file's work keeps the filter it used.
    with ctx.launch() as app:
        js = app.js
        st = lambda: js(STATE)
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(st()), timeout, what=what)
        app.open_fixture('ui-copy.log')
        s = until(lambda s: s['heading'] == first, 'restored saved filter')
        app.wait_count(expected(['ExampleApp', 'WifiMonitor'], 'WEFA'))
        check('after a restart the file returns with its saved filter', s['tag'] == 'ExampleApp, WifiMonitor' and s['headingState'] == 'applied', s)
        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.4)

        def menu(button='panelSavedFilter'):
            app.click(button)
            return until(lambda s: s['menu'] is not None, 'saved filter menu')['menu']

        def pick(label):
            check(f'the menu offers {label}', app.wait(lambda: js(PICK, label), what=label))
            time.sleep(.3)
        menu('panelSavedFilter')
        pick('이름 바꾸기…')
        s = until(lambda s: s['dialog'], 'rename dialog')
        check('renaming starts from the current name without the conditions note', s['title'] == '이름 바꾸기' and s['name'] == first and s['hint'] is None, s)
        js(SUBMIT, '내 앱 경고')
        s = until(lambda s: not s['dialog'], 'renamed')
        check('renaming changes both buttons', s['heading'] == '내 앱 경고' and s['panel'] == '내 앱 경고' and s['toast'] == '이름을 바꿨습니다 · 내 앱 경고', s)

        app.enter('tagFilter', 'RenderThread')
        app.wait_count(expected(['RenderThread'], 'WEFA'))
        menu()
        pick('다른 이름으로 저장…')
        until(lambda s: s['dialog'], 'save as dialog')
        js(SUBMIT, '내 앱 경고 ')
        s = until(lambda s: s['error'], 'name in use')
        check('a name in use, ignoring case and spaces, is refused in the dialog', s['dialog'] and s['error'] == '같은 이름의 저장된 필터가 있습니다. 다른 이름을 입력해 주세요.', s)
        app.capture('05-name-in-use')
        js(SUBMIT, 'Render')
        until(lambda s: not s['dialog'], 'saved as')
        s = st()
        check('save as makes a second filter and applies it', s['heading'] == 'Render' and len(json.loads(store.read_text())) == 2, s)
        items = menu()
        check('saved filters are listed by name in Korean order', items[:2] == ['내 앱 경고', '* Render'], items)
        pick("'Render' 삭제")
        s = until(lambda s: s['headingState'] == 'none', 'deleted')
        check('deleting keeps the conditions and offers to undo',
              s['tag'] == 'RenderThread' and s['action'] == '실행 취소' and [e['name'] for e in json.loads(store.read_text())] == ['내 앱 경고'], s)
        js("document.querySelector('#toast .toast-action').click();return true;")
        s = until(lambda s: s['heading'] == 'Render', 'delete undone')
        check('undo saves the deleted filter again', sorted(e['name'] for e in json.loads(store.read_text())) == ['Render', '내 앱 경고'], s)

        # Escape closes the dialog only, not the raw detail under it.
        app.click_row(js("return document.querySelector('#logsRows .log-row').dataset.lineNumber"))
        until(lambda s: s['detail'], 'detail')
        menu()
        pick('이름 바꾸기…')
        until(lambda s: s['dialog'], 'dialog')
        app.key('Escape', target='filterNameInput')
        time.sleep(.3)
        s = st()
        check('Escape closes only the name dialog', not s['dialog'] and s['detail'] and s['heading'] == 'Render', s)

        app.open_fixture('vehicle.dlt')
        until(lambda s: s['count'].startswith('로그 · 전체'), 'dlt rows')
        items = menu()
        check('a DLT file lists no text-log filters and says why', items == ['(텍스트 로그 필터 2개는 이 로그에 쓸 수 없습니다)', '(조건을 정한 뒤 저장할 수 있습니다)'], items)
        app.capture('06-dlt-menu')
        app.key('Escape')
