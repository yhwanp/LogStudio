"""Bookmarks in the lower-region tab: adding with Cmd+B, the tab and its list,
selecting in it (click, Shift, Cmd, drag, Cmd+A) with copy, Delete and the
row menu,
revealing, keyboard and F2 stepping, switching with search, raw view,
removing, copying, clearing, and restoring them with the file's saved work
(ui.log)."""
import time

import fixtures

STATE = """const $=id=>document.getElementById(id),r=e=>e.getBoundingClientRect();return {
  tabs:!$('lowerTabs').hidden,label:$('bookmarkTab').textContent,selected:$('bookmarkTab').getAttribute('aria-selected'),
  pane:!$('bookmarkPane').hidden,search:!$('largeSearchPane').hidden,expanded:$('largeLowerPane').classList.contains('expanded'),
  items:[...document.querySelectorAll('#bookmarkList .bookmark-row')].map(e=>e.dataset.lineNumber),
  current:document.querySelector('#bookmarkList [aria-current=true]')?.dataset.lineNumber??null,
  focus:document.activeElement?.closest?.('#bookmarkList')?document.activeElement.dataset.lineNumber:document.activeElement?.id||document.activeElement?.className,
  upper:document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null};"""
MENU = "const m=document.getElementById('rowMenu');return {hidden:m.hidden,label:m.getAttribute('aria-label'),items:[...m.querySelectorAll('[role=menuitem]')].map(e=>e.textContent)};"
MARKED = """return {marked:[...document.querySelectorAll('#bookmarkList .in-selection')].map(e=>e.dataset.lineNumber),info:document.getElementById('bookmarkSelection').textContent};"""
COPY = """const event=new Event('copy',{bubbles:true,cancelable:true}),store={};
Object.defineProperty(event,'clipboardData',{value:{setData:(t,v)=>{store[t]=v;},getData:t=>store[t]}});
(document.activeElement||document.body).dispatchEvent(event);return {copy:event.defaultPrevented,data:store['text/plain']??null};"""


def run(ctx):
    lines = fixtures.ui(ctx.fixtures)
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        st = lambda: js(STATE)
        app.open_fixture('ui.log')
        app.wait_count(3000)
        check('no bookmark tab before any bookmark', not st()['tabs'])
        app.click_row(2)
        app.key('b', meta=True)
        app.click_row(7)
        app.click_row(9, shift=True)
        app.key('b', meta=True)
        time.sleep(.2)
        s = st()
        check('Cmd+B shows the tab with its count without replacing the search tab', s['tabs'] and s['label'] == '북마크 4' and s['selected'] == 'false' and not s['pane'], s)
        app.capture('01-tab-appears', hover='bookmarkTab')
        app.click('bookmarkTab')
        s = app.wait(lambda: (lambda v: v['pane'] and v)(st()), what='bookmark pane')
        check('the tab opens the list in the resizable lower region and focuses it', s['expanded'] and s['items'] == ['2', '7', '8', '9'] and s['focus'] == '2', s)
        cols = js("const g=id=>getComputedStyle(document.getElementById(id)).gridTemplateColumns;return {logs:g('logsHeaders'),marks:g('bookmarkHeaders'),row:getComputedStyle(document.querySelector('#bookmarkList .bookmark-row')).gridTemplateColumns,cells:[...document.querySelector('#bookmarkList [data-line-number=\"7\"]').children].map(e=>e.textContent)};")
        check('bookmark rows use the log columns and show every field', cols['logs'] == cols['marks'] == cols['row'] and cols['cells'][2:6] == ['2983', '4241', 'Error', 'ActivityManager'], cols)
        app.capture('02-bookmark-tab')
        js("document.querySelectorAll('#bookmarkList .bookmark-row')[2].click();return true;")
        s = app.wait(lambda: (lambda v: v['upper'] == '8' and v)(st()), what='reveal 8')
        check('clicking a bookmark reveals it above while the list stays open and focused', s['pane'] and s['current'] == '8' and s['focus'] == '8', s)
        app.key('ArrowDown')
        s = app.wait(lambda: (lambda v: v['upper'] == '9' and v)(st()), what='reveal 9')
        check('arrow keys walk the list and reveal each bookmark', s['focus'] == '9' and s['current'] == '9', s)
        app.key('Enter')
        e = app.wait(lambda: (lambda v: v['line'] == '9' and v)(js("const a=document.activeElement;return {line:document.getElementById('logsViewport').contains(a)?a.dataset.lineNumber:null};")), what='focus on bookmark 9 above')
        check('Enter on a bookmark moves focus to its row above', e['line'] == '9', e)
        js("document.getElementById('logsViewport').focus();return true;")
        app.key('F2')
        time.sleep(.3)
        s = st()
        check('F2 wraps to the first bookmark and moves the list cursor', s['upper'] == '2' and s['current'] == '2', s)
        app.key('F2', shift=True)
        time.sleep(.3)
        check('Shift+F2 steps back to the last bookmark', st()['upper'] == '9', st()['upper'])
        app.type('searchInput', 'WIFI')
        s = app.wait(lambda: (lambda v: v['search'] and v)(st()), what='search tab')
        check('typing a search switches to the search tab', not s['pane'] and s['selected'] == 'false', s)
        app.wait(lambda: app.text('largeCount') != '0', what='search results')
        app.click('bookmarkTab')
        s = app.wait(lambda: (lambda v: v['pane'] and v)(st()), what='bookmark pane')
        check('the bookmark tab returns while the search keeps its query', not s['search'] and js("return document.getElementById('searchInput').value") == 'WIFI', s)
        app.click('largeRaw')
        time.sleep(.3)
        raw = js("return {head:document.getElementById('bookmarkHeaders').children.length,raw:document.querySelector('#bookmarkList [data-line-number=\"7\"] .raw-content')?.textContent}")
        check('raw view applies to the bookmark list', raw['head'] == 2 and raw['raw'] == lines[6], raw)
        app.click('largeColumns')
        time.sleep(.3)
        js("document.querySelector('#bookmarkList [data-line-number=\"7\"]').focus();return true;")
        app.key('Delete')
        time.sleep(.2)
        s = st()
        check('Delete removes the focused bookmark and keeps focus in the list',
              s['items'] == ['2', '8', '9'] and s['focus'] == '8' and s['label'] == '북마크 3' and not js("return !!document.querySelector('#logsRows [data-line-number=\"7\"].bookmarked')"), s)
        js("document.querySelector('#bookmarkList [data-line-number=\"9\"] .bookmark-remove').click();return true;")
        time.sleep(.2)
        check('the row remove button unmarks it', st()['items'] == ['2', '8'])
        app.capture_clipboard()
        app.click('copyBookmarks')
        check('copy all writes the raw rows in order', app.captured() == '\n'.join(lines[i - 1] for i in (2, 8)))
        # The list selects as the other lists do (synthetic pointer and mouse
        # events; a real mouse is not driven). Marks for a discontiguous
        # upper selection are added at once.
        app.jump(12)
        app.wait(lambda: app.row_exists(15), what='rows 11-15')
        app.drag_rows(11, 13)
        app.click_row(15, meta=True)
        app.key('b', meta=True)
        time.sleep(.2)
        s = st()
        check('Cmd+B marks a drag range and a Cmd+clicked row together', s['items'] == ['2', '8', '11', '12', '13', '15'] and s['label'] == '북마크 6', s)
        app.click_row(8, pane='bookmarkList')
        app.wait(lambda: st()['upper'] == '8', what='reveal 8')
        app.click_row(12, pane='bookmarkList', shift=True)
        app.click_row(15, pane='bookmarkList', meta=True)
        s = app.wait(lambda: (lambda v: v['upper'] == '15' and v)({**st(), **js(MARKED)}), what='reveal 15')
        check('Shift+click and Cmd+click select bookmarks apart; the focus row is revealed above', s['marked'] == ['8', '11', '12', '15'] and s['current'] == '15' and s['focus'] == '15' and s['info'] == '4행 선택 · ⌘C 복사', s)
        # A shrinking list keeps its cursor row in view.
        js("document.getElementById('bookmarkList').scrollTop=0;return true;")
        time.sleep(.1)
        js("document.getElementById('bookmarkList').style.maxHeight='70px';return true;")
        k = app.wait(lambda: (lambda v: v['seen'] and v)(js("const l=document.getElementById('bookmarkList').getBoundingClientRect(),r=document.querySelector('#bookmarkList [aria-current=true]').getBoundingClientRect();"
                                                            "return {seen:r.top>=l.top-1&&r.bottom<=l.bottom+1,list:l.height,current:document.querySelector('#bookmarkList [aria-current=true]').dataset.lineNumber};")), timeout=5, what='cursor kept in view')
        check('a bookmark list that shrinks keeps its cursor row in view', k['current'] == '15' and k['list'] < 6 * 28, k)
        js("document.getElementById('bookmarkList').style.maxHeight='';return true;")
        time.sleep(.2)
        app.capture('03-bookmark-multi')
        app.capture_clipboard()
        g = js(COPY)
        check('Cmd+C copies the selected bookmarks in row order', g['copy'] and (g['data'] if g['data'] is not None else app.captured()) == '\n'.join(lines[i - 1] for i in (8, 11, 12, 15)), g)
        app.drag_rows(11, 13, pane='bookmarkList')
        check('a drag selects a range of bookmarks', js(MARKED)['marked'] == ['11', '12', '13'], js(MARKED))
        # The row menu: a right-click inside the selection keeps it and acts on
        # all of it; one outside selects that bookmark alone. The upper logs
        # stay, as for a right-click in the search results.
        app.click_row(12, pane='bookmarkList', event='contextmenu')
        m = app.wait(lambda: (lambda v: not v['hidden'] and v)(js(MENU)), what='bookmark menu')
        check('a right-click inside the bookmark selection keeps it, and its menu copies or unmarks all of it',
              m['label'] == '북마크 작업' and m['items'][-2:] == ['선택한 3행 복사', '선택한 북마크 3개 해제'] and any('보기 조건 추가' in item for item in m['items'])
              and js(MARKED)['marked'] == ['11', '12', '13'], m)
        app.capture('03b-bookmark-menu')
        app.key('Escape', target='rowMenu')
        app.click_row(2, pane='bookmarkList', event='contextmenu')
        upper = st()['upper']
        m = app.wait(lambda: (lambda v: not v['hidden'] and v)(js(MENU)), what='bookmark menu')
        time.sleep(.3)
        s = {**st(), **js(MARKED), 'open': not js(MENU)['hidden']}
        check('a right-click outside it selects that bookmark alone, leaves the upper logs and the menu stays open',
              m['items'][-2:] == ['행 원문 복사', '북마크 해제'] and s['marked'] == [] and s['current'] == '2' and s['upper'] == upper and s['open'], {**m, **s})
        app.key('Escape', target='rowMenu')
        app.drag_rows(11, 13, pane='bookmarkList')
        a = js("const e=new KeyboardEvent('keydown',{key:'a',metaKey:true,bubbles:true,cancelable:true});document.activeElement.dispatchEvent(e);return e.defaultPrevented;")
        m = js(MARKED)
        check('Cmd+A selects every bookmark', a and m['marked'] == ['2', '8', '11', '12', '13', '15'] and m['info'] == '6행 선택 · ⌘C 복사', m)
        # Shift+PageUp/PageDown extend from the anchor as in the other lists.
        app.click_row(2, pane='bookmarkList')
        app.key('PageDown', shift=True)
        time.sleep(.15)
        pd = js(MARKED)
        check('Shift+PageDown extends the bookmark selection from the clicked row', len(pd['marked']) > 1 and pd['marked'][0] == '2', pd)
        js("const e=new KeyboardEvent('keydown',{key:'a',metaKey:true,bubbles:true,cancelable:true});document.activeElement.dispatchEvent(e);return true;")
        detail_open = js("return !document.getElementById('largeDetail').hidden")
        app.key('Escape')
        time.sleep(.15)
        first = js(MARKED)
        if detail_open:
            app.key('Escape')
            time.sleep(.15)
        e = js(MARKED)
        check('Escape, after the raw detail, makes a selection of several bookmarks its focus row',
              (not detail_open or len(first['marked']) == 6) and e['marked'] == [] and e['info'] == '', {'detail': detail_open, 'first': first, 'after': e})
        js("const e=new KeyboardEvent('keydown',{key:'a',metaKey:true,bubbles:true,cancelable:true});document.activeElement.dispatchEvent(e);return true;")
        app.click_row(2, pane='bookmarkList', meta=True)
        app.key('Delete')
        time.sleep(.2)
        s = st()
        check('Delete unmarks every selected bookmark and keeps focus in the list', s['items'] == ['2'] and s['focus'] == '2' and s['label'] == '북마크 1' and '북마크 5개를 해제했습니다' in app.text('toast'), s)
        app.click('clearBookmarks')
        time.sleep(.2)
        s = st()
        check('clearing every bookmark hides the tab, shows the search again and returns focus to the logs',
              not s['tabs'] and not s['pane'] and s['search'] and js("return document.getElementById('logsViewport').contains(document.activeElement)"), s)
        app.click_row(5)
        app.key('b', meta=True)
        time.sleep(.2)
        app.click('largeClose')
        app.wait(lambda: not app.hidden('emptyState'), what='empty state')
        app.open_fixture('ui.log')
        app.wait_count(3000)
        s = app.wait(lambda: (lambda v: v['tabs'] and v)(st()), what='restored bookmark tab')
        toast = app.text('toast')
        check('reopening the unchanged file restores its bookmark with the saved work', s['label'] == '북마크 1' and '북마크 1개' in toast, {'label': s['label'], 'toast': toast})
