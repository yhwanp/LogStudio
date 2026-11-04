"""The top bar's ⋯ menu: what stays on the main screen, the items and their
keys, more choices in the menu's place with the way back, the press that
closes it, the row number box (Cmd+L), a narrow window (ui.log)."""
import time

import fixtures

BAR = """const shown=e=>e.getClientRects().length>0,names=q=>[...document.querySelectorAll(q)].filter(shown).map(e=>e.getAttribute('aria-label')||e.textContent.trim());
const d=document.documentElement;return {top:names('.top-actions button'),logs:names('.logs-main-pane .pane-heading button'),search:names('#largeSearchPane .pane-heading button'),overflow:d.scrollWidth>d.clientWidth};"""
MENU = """const m=document.getElementById('rowMenu'),b=m.getBoundingClientRect(),o=document.getElementById('moreButton').getBoundingClientRect();
return {hidden:m.hidden,label:m.getAttribute('aria-label'),right:b.right-o.right,gap:b.top-o.bottom,items:[...m.children].map(e=>e.tagName==='HR'?null:e.textContent),
focus:document.activeElement?.textContent??null,opensPopup:document.activeElement?.getAttribute('aria-haspopup'),expanded:document.getElementById('moreButton').getAttribute('aria-expanded')};"""
# A press on the button as a mouse makes it.
PRESS = """const b=document.getElementById('moreButton'),o={bubbles:true,cancelable:true,detail:1};b.dispatchEvent(new PointerEvent('pointerdown',{...o,pointerId:1}));
b.dispatchEvent(new PointerEvent('pointerup',{...o,pointerId:1}));b.dispatchEvent(new MouseEvent('click',o));return true;"""
JUMP = """const f=document.getElementById('logsJumpForm'),b=f.getBoundingClientRect(),o=document.getElementById('moreButton').getBoundingClientRect();
return {hidden:f.hidden,right:b.right-o.right,gap:b.top-o.bottom,help:document.getElementById('jumpHelp').textContent,focus:document.activeElement?.id,tag:document.activeElement?.tagName,
focusLine:document.activeElement?.dataset?.lineNumber??null,invalid:document.getElementById('logsJump').getAttribute('aria-invalid'),error:document.getElementById('jumpError').textContent,
describedby:document.getElementById('logsJump').getAttribute('aria-describedby'),alert:getComputedStyle(document.querySelector('.jump-panel [role=alert]')).display,selected:document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null};"""
# Synthetic keys leave :focus-visible off, so the rule is copied to a class and
# its item drawn with it.
DISABLED_FOCUS = """const s=[...document.styleSheets].find(x=>[...x.cssRules].some(r=>r.selectorText?.includes('row-menu')));
for(const r of [...s.cssRules].filter(x=>x.selectorText?.includes('.row-menu')&&x.selectorText.includes(':focus-visible')))s.insertRule(r.cssText.replaceAll(':focus-visible','.qa-focus'),s.cssRules.length);
const e=document.querySelector('#rowMenu [aria-disabled=true]');e.classList.add('qa-focus');e.focus();
const c=getComputedStyle(e);return {style:c.outlineStyle,width:c.outlineWidth,label:e.textContent};"""
ZOOMED = "const w=document.querySelector('.brand h1').getBoundingClientRect().width;return {width:innerWidth,right:document.getElementById('moreButton').getBoundingClientRect().right,wordmark:w};"
# Quiet once a resize has come since `MARK`: it closes a menu, and arrives a
# moment after the window or the zoom has changed.
WATCH = "window.__resizes=0;if(!window.__resizing){window.__resizing=true;addEventListener('resize',()=>{window.__resizes++;window.__resized=performance.now();});}return true;"
QUIET = "return window.__resizes>0&&performance.now()-window.__resized>100;"
SUBMIT = "const i=document.getElementById('logsJump');i.value=arguments[0];document.getElementById('logsJumpForm').requestSubmit();return true;"


def run(ctx):
    check = ctx.check
    fixtures.ui(ctx.fixtures)
    with ctx.launch(ctx.fixtures / 'ui.log') as app:
        js = app.js
        app.wait_count(3000)
        app.wait(lambda: '최근 파일' in app.menu_items(), what='recent files')
        # An extras build shows 확장 even with no settings file; others never do.
        extras = ['확장 ›'] if app.has_extras() else []
        app.type('searchInput', 'timeout')
        app.wait(lambda: not app.hidden('largeSearchPane'), what='search results')
        app.click_row(7)
        time.sleep(.3)
        s = js(BAR)
        check('the main screen keeps the frequent controls only',
              s['top'] == ['기기 로그', '로그 파일 열기', '메뉴'] and s['logs'] == ['저장된 필터', '처음', '끝', '따라가기', '열 보기', '원문 보기'] and s['search'] == ['처음', '끝'], s)
        app.capture('01-main-screen', hover='moreButton')

        js(PRESS)
        m = app.wait(lambda: (lambda v: not v['hidden'] and v)(js(MENU)), what='menu')
        check('the menu opens under its button, right edges together, keys at the items',
              m['label'] == '메뉴' and abs(m['right']) < 1 and 0 < m['gap'] < 8 and [i and i.split(' · ')[0] for i in m['items']] == ['최근 파일 ›', '다시 열기 ⌘R', '파일 닫기', None, '전체 로그 내보내기…', '검색 결과 내보내기…', None, '행 번호로 이동… ⌘L', '열 폭 ›', None, *extras, '단축키·기능 둘러보기 ?']
              and '· 3,000행' in m['items'][4] and '건' in m['items'][5] and m['expanded'] == 'true', m)
        app.capture('02-menu')
        js(PRESS)
        check('pressing the button again closes the menu', js(MENU)['hidden'])

        js(PRESS)
        app.wait(lambda: not js(MENU)['hidden'], what='menu again')
        app.key('ArrowRight', target='rowMenu')
        m = app.wait(lambda: (lambda v: v['label'] == '최근 파일' and v)(js(MENU)), what='recent files')
        check('Right Arrow shows the recent files in the menu\'s place, the way back first, focus on a file',
              m['items'][:3] == ['‹ 메뉴', None, 'ui.log'] and abs(m['right']) < 1 and m['focus'] == 'ui.log' and m['expanded'] == 'true', m)
        app.capture('03-recent-files')
        app.key('ArrowLeft', target='rowMenu')
        m = app.wait(lambda: (lambda v: v['label'] == '메뉴' and v)(js(MENU)), what='menu back')
        check('Left Arrow returns to the menu, on the item it came from',
              m['items'][0] == '최근 파일 ›' and m['focus'] == '최근 파일 ›' and m['opensPopup'] == 'menu' and m['expanded'] == 'true', m)
        app.key('Escape', target='rowMenu')
        check('Escape then closes it and marks it closed', js(MENU)['hidden'] and js(MENU)['expanded'] == 'false')

        app.menu('열 폭')
        m = app.wait(lambda: (lambda v: v['label'] == '열 폭' and v)(js(MENU)), what='column menu')
        check('the column tools are one step in', m['items'] == ['‹ 메뉴', None, '내용에 맞춤', '폭 초기화'], m)
        app.key('Escape', target='rowMenu')
        m = app.wait(lambda: (lambda v: v['label'] == '메뉴' and v)(js(MENU)), what='back from the columns')
        check('Escape goes back one level, on the item it came from', m['focus'] == '열 폭 ›' and not m['hidden'], m)
        app.key('Escape', target='rowMenu')
        app.click('largeRaw')
        check('raw view leaves the column tools out', '열 폭' not in app.menu_items())
        app.click('largeColumns')

        app.key('l', meta=True, target='logsViewport')
        j = app.wait(lambda: (lambda v: not v['hidden'] and v)(js(JUMP)), what='row box')
        check('⌘L opens the row box where the menu opens, ready to type',
              abs(j['right']) < 1 and 0 < j['gap'] < 8 and j['focus'] == 'logsJump' and '1–3,000행' in j['help'] and j['error'] == '' and j['alert'] != 'none', j)
        app.capture('04-row-box')
        js(SUBMIT, '99999')
        j = app.wait(lambda: (lambda v: v['error'] and v)(js(JUMP)), what='rejected row')
        check('a row past the end keeps the box open and says why inside it',
              not j['hidden'] and j['invalid'] == 'true' and '마지막 행은 3,000행' in j['error'] and j['describedby'] == 'jumpError jumpHelp', j)
        app.capture('05-row-box-invalid')
        js(SUBMIT, '1200')
        j = app.wait(lambda: (lambda v: v['selected'] == '1200' and v)(js(JUMP)), what='row 1200')
        check('a good row closes the box and selects it in the upper logs', j['hidden'] and j['error'] == '', j)
        j = app.wait(lambda: (lambda v: v['focusLine'] == '1200' and v)(js(JUMP)), what='focus on row 1200')
        check('focus lands on the row, so the arrow keys go on from it', j['focusLine'] == '1200', j)
        time.sleep(.3)
        j = js(JUMP)
        check('and stays there once the hidden box lets go', j['focusLine'] == '1200', j)
        app.key('l', meta=True, target='logsViewport')
        app.wait(lambda: not js(JUMP)['hidden'], what='row box from the keyboard')
        js("document.getElementById('logsJumpForm').focus();return true;")
        check('the box itself holds focus, so Escape reaches it after a press on its text', js(JUMP)['focus'] == 'logsJumpForm')
        app.key('Escape', target='logsJumpForm')
        j = js(JUMP)
        check('Escape closes the box, leaves the raw detail open and returns to the row', j['hidden'] and not app.hidden('largeDetail') and j['focusLine'] == '1200', j)
        app.menu('행 번호로 이동…')
        app.wait(lambda: not js(JUMP)['hidden'], what='row box from the menu')
        app.key('Escape', target='logsJump')
        j = js(JUMP)
        check('opened with the mouse, whose press took focus from the row, Escape returns to the ⋯ button', j['hidden'] and j['focus'] == 'moreButton', j)

        js("document.activeElement.blur();return true;")
        js(PRESS)
        app.wait(lambda: not js(MENU)['hidden'], what='menu from no focus')
        app.key('Escape', target='rowMenu')
        check('with nothing focused before it, Escape returns focus to the button', js(JUMP)['focus'] == 'moreButton', js(JUMP))

        app.type('searchInput', '')
        app.wait(lambda: app.hidden('largeSearchPane') or not app.menu_items()['전체 로그 내보내기…']['disabled'], what='search cleared')
        js(PRESS)
        m = app.wait(lambda: (lambda v: not v['hidden'] and v)(js(MENU)), what='menu with a disabled item')
        check('an item that waits says what it waits for', any(i and '검색어를 입력하면' in i for i in m['items']), m)
        f = js(DISABLED_FOCUS)
        check('an item that cannot be chosen still shows where focus is', f['style'] == 'solid' and f['width'] == '2px', f)
        app.capture('07-disabled-item')
        app.key('Escape', target='rowMenu')

        js(WATCH)
        app.rect(width=760, height=640)
        app.wait(lambda: js(QUIET), what='resized')
        s = js(BAR)
        check('a narrow window fits the same controls', not s['overflow'] and s['top'] == ['기기 로그', '로그 파일 열기', '메뉴'], s)
        app.capture('06-narrow')
        for _ in range(5):
            app.key('=', meta=True, code='Equal')
        o = app.wait(lambda: (lambda v: v['width'] < 400 and v)(js(ZOOMED)), what='200% zoom')
        check('at 200% in the smallest window the menu button stays in the window', o['right'] <= o['width'] and o['wordmark'] <= 1, o)
        app.capture('08-zoomed')
        js(WATCH)
        app.key('0', meta=True, code='Digit0')
        app.wait(lambda: js(ZOOMED)['width'] > 700 and js(QUIET), what='zoom reset')
        app.menu('파일 닫기')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        items = app.menu_items()
        check('without a file the menu keeps the recent files, 확장 in an extras build, and the shortcuts', list(items) == ['최근 파일', *(['확장'] if extras else []), '단축키·기능 둘러보기'], items)
