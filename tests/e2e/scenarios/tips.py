"""First-use tips and the feature tour: the first tip once a file shows, a
row click that closes it getting the row tip at once, the level, search and
bookmark tips with their dismissals and the panel held open, turning tips
off and back on, seen tips across launches, the tour from the first tip
(modal, every step lit beside its card, page shortcuts held, its steps
counting as seen tips) and from the shortcut list opening the sample log
(ui.log)."""
import time

import fixtures

STATE = """const $=id=>document.getElementById(id),r=e=>{const b=e.getBoundingClientRect();return [b.left,b.top,b.right,b.bottom].map(Math.round);};
const l=$('tipLayer');return {open:l.open,modal:l.matches(':modal'),tip:l.dataset.tip||null,title:$('tipTitle').textContent,
step:$('tipStep').hidden?null:$('tipStep').textContent,next:$('tipNext').hidden?null:$('tipNext').textContent,back:$('tipBack').hidden?null:$('tipBack').textContent,
side:$('tipCard').dataset.side||null,spot:$('tipSpot').hidden?null:r($('tipSpot')),card:r($('tipCard')),view:[innerWidth,innerHeight],
focus:document.activeElement?.id||null,stored:JSON.parse(localStorage.getItem('logstudio.tips.v1')||'null'),
panelOpen:!$('filterPanel').inert,detail:!$('largeDetail').hidden,dialog:$('shortcutDialog').open};"""
RECT = "const b=(arguments[0]?document.getElementById(arguments[0]):document.querySelector(arguments[1])).getBoundingClientRect();return [b.left,b.top,b.right,b.bottom].map(Math.round);"
POINTER = "document.querySelector(arguments[0]).dispatchEvent(new PointerEvent(arguments[1],{bubbles:true,cancelable:true}));return true;"
# A click as a mouse makes it: pointerdown, mousedown, then click.
CLICK = """const r=document.querySelector(`#logsRows [data-line-number="${arguments[0]}"]`);r.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true}));
for(const t of ['mousedown','click'])r.dispatchEvent(new MouseEvent(t,{bubbles:true,cancelable:true,detail:1,clientX:200,clientY:200}));return true;"""
# Tour steps by title and the control each one lights (rows are checked apart).
STEPS = {'로그 열기': 'openFile', '다시 열기': 'reopenFile', '필터': 'toggleFilters', '저장된 필터': 'savedFilterButton', '열 폭': 'logsHeaders',
         '검색': 'largeSearchTools', '단축키와 둘러보기': 'shortcutHelp'}


def lit(spot, box):
    """The spot surrounds the box by its 4px padding."""
    return spot is not None and all(abs(a - b) <= 2 for a, b in zip(spot, [box[0] - 4, box[1] - 4, box[2] + 4, box[3] + 4]))


def placed(s):
    """The card is inside the window and, unless it covers a large control, apart from the lit spot."""
    card, spot, (w, h) = s['card'], s['spot'], s['view']
    inside = card[0] >= 0 and card[1] >= 0 and card[2] <= w and card[3] <= h
    apart = spot is None or s['side'] == 'inside' or card[2] <= spot[0] or card[0] >= spot[2] or card[3] <= spot[1] or card[1] >= spot[3]
    return inside and apart


def run(ctx):
    fixtures.ui(ctx.fixtures)
    check = ctx.check

    def helpers(app):
        st = lambda: app.js(STATE)
        rect = lambda id=None, css=None: app.js(RECT, id, css)
        tip = lambda name, timeout=15: app.wait(lambda: (lambda s: s['open'] and s['tip'] == name and s)(st()), timeout, what=f'{name} tip')
        closed = lambda what: app.wait(lambda: (lambda v: not v['open'] and v)(st()), what=what)
        return st, rect, tip, closed

    with ctx.launch(tips=True, clear_after=False) as app:
        st, rect, tip, closed = helpers(app)
        time.sleep(1.5)
        check('no tip on the empty state', not st()['open'], st()['tip'])
        app.open_fixture('ui.log')
        app.wait_count(3000)
        s = tip('help')
        check('once a file shows, the first tip points at the help button without taking focus or input',
              not s['modal'] and s['focus'] not in ('tipNext', 'tipBack', 'tipClose') and lit(s['spot'], rect('shortcutHelp'))
              and s['side'] == 'bottom' and placed(s) and s['next'] == '둘러보기' and s['back'] == '닫기', s)
        check('its layer lets presses through to the page',
              app.js("const c=document.getElementById('tipCard').getBoundingClientRect();return document.elementFromPoint(40,c.bottom+40)?.closest('#tipLayer')===null"))
        app.capture('01-first-tip')

        clicked = time.monotonic()
        app.js(CLICK, 5)
        s = tip('rows', timeout=5)
        delay = time.monotonic() - clicked
        body = app.js("return [...document.querySelectorAll('#tipBody .tip-line')].map(e=>e.textContent).join('\\n')")
        check('a row click that closes the first tip gets the row tip after a short pause, not a later click',
              delay < 2 and 'help' in s['stored']['seen'], {'seconds': round(delay, 2), 'seen': s['stored']['seen']})
        check('the row tip lights the selected row with copy, bookmark and row menu, one topic per line',
              lit(s['spot'], rect(css='#logsRows .selected-log-row')) and placed(s) and s['next'] == '확인' and s['focus'] != 'tipNext'
              and len(body.split('\n')) == 3 and '다시 볼 행은' in body, {**s, 'body': body})
        app.capture('06-rows-tip')
        app.key('Escape')
        s = closed('rows tip closed')
        check('Escape closes the tip and leaves the raw detail open', s['detail'] and 'rows' in s['stored']['seen'], s)

        app.click('toggleFilters')
        s = tip('levels')
        check('opening the filters lights the level chips with the card beside the panel',
              lit(s['spot'], rect('levelFilters')) and s['side'] == 'right' and placed(s), s)
        app.js(POINTER, '#filterPanel', 'pointerleave')
        time.sleep(.6)
        s = st()
        check('the auto-hiding panel stays open while the tip points into it', s['open'] and s['panelOpen'], s)
        app.capture('07-levels-tip')
        app.js(POINTER, '#logsRows .log-row', 'pointerdown')
        time.sleep(.3)
        s = st()
        check('a press outside dismisses the tip and then hides the panel as before', not s['open'] and not s['panelOpen'] and 'levels' in s['stored']['seen'], s)

        app.type('searchInput', 'WIFI')
        app.wait(lambda: app.text('largeCount') not in ('0', ''), what='search results')
        s = tip('search')
        check('search results get the tip above them while typing focus stays in the search box',
              lit(s['spot'], rect('largeSearchPane')) and s['side'] == 'top' and placed(s) and s['focus'] == 'searchInput', s)
        app.capture('08-search-tip')
        app.js(POINTER, '#logsRows .log-row', 'pointerdown')
        s = closed('search tip closed')
        check('a press on the page dismisses a tip', 'search' in s['stored']['seen'], s)

        app.click('toggleFilters')
        time.sleep(1.2)
        check('a seen tip does not return', not st()['open'], st()['tip'])
        app.click('toggleFilters')
        app.click_row(3)
        app.key('b', meta=True)
        s = tip('bookmarks')
        check('the first bookmark lights its tab', lit(s['spot'], rect('bookmarkTab')) and placed(s), s)
        app.capture('09-bookmarks-tip', hover='tipNext')
        app.click('tipOff')
        s = closed('tips off')
        check('“팁 그만 보기” turns tips off', s['stored']['off'] is True and 'bookmarks' in s['stored']['seen'], s['stored'])
        app.click('shortcutHelp')
        app.wait(lambda: st()['dialog'], what='shortcut list')
        check('the shortcut list shows the setting off', app.js("return document.getElementById('tipsEnabled').checked") is False)
        app.capture('10-shortcut-list', hover='startTour')
        app.click('shortcutClose')

    with ctx.launch(tips=True, clear_before=False) as app:
        st, rect, tip, closed = helpers(app)
        app.open_fixture('ui.log')
        app.wait_count(3000)
        app.click_row(4)
        time.sleep(2)
        check('tips stay off and seen across launches', not st()['open'] and st()['stored']['off'] is True, st()['stored'])
        app.click('shortcutHelp')
        app.wait(lambda: st()['dialog'], what='shortcut list')
        app.js("const c=document.getElementById('tipsEnabled');c.click();return true;")
        check('the shortcut list turns tips back on', st()['stored']['off'] is False, st()['stored'])
        app.click('shortcutClose')

    # Without the saved work of ui.log, whose restored bookmarks and search
    # would offer their own tips first.
    (ctx.data / 'work-states.json').unlink(missing_ok=True)
    with ctx.launch(tips=True) as app:
        st, rect, tip, closed = helpers(app)
        app.open_fixture('ui.log')
        app.wait_count(3000)
        before = tip('help')['focus']
        app.click('tipNext')
        s = app.wait(lambda: (lambda v: v['modal'] and v['step'] and v)(st()), what='tour')
        check('the first tip starts the modal tour on the open file', s['tip'] == 'tour' and s['title'] == '로그 열기' and s['step'] == '1 / 9'
              and s['focus'] == 'tipNext' and s['stored']['seen'] == ['help'], s)
        app.capture('02-tour-open-file')
        seen = []
        for i in range(9):
            s = st()
            seen.append(s['title'])
            target = STEPS.get(s['title'])
            ok = placed(s) and (lit(s['spot'], rect(target)) if target else s['spot'] is not None)
            check(f"tour step {i + 1} ({s['title']}) lights its control with the card beside it", ok, s)
            if s['title'] == '행 선택과 복사':
                app.capture('03-tour-rows')
            if s['title'] == '북마크':
                app.capture('03b-tour-bookmark')
            if s['title'] == '필터':
                app.key('?')
                app.key('f', meta=True)
                time.sleep(.3)
                held = st()
                check('page shortcuts wait while the tour shows', not held['dialog'] and held['focus'] == 'tipNext' and held['title'] == '필터', held)
                app.capture('04-tour-filters')
            if s['title'] == '검색':
                app.capture('05-tour-search')
            if i < 8:
                app.key('ArrowRight')
                app.wait(lambda: st()['title'] != s['title'], what='next step')
        check('the tour walks every main control once', seen == ['로그 열기', '다시 열기', '필터', '저장된 필터', '행 선택과 복사', '북마크', '열 폭', '검색', '단축키와 둘러보기'], seen)
        app.key('ArrowLeft')
        check('ArrowLeft steps back', app.wait(lambda: st()['title'] == '검색', what='previous step'))
        app.key('ArrowRight')
        app.wait(lambda: st()['next'] == '마치기', what='last step')
        app.click('tipNext')
        s = closed('tour end')
        check('finishing the tour closes it and returns focus', s['focus'] == (before or 'logsViewport'), {'focus': s['focus'], 'before': before})
        app.js(CLICK, 6)
        time.sleep(2)
        s = st()
        check('steps of the tour count as seen tips, so a row click after it shows none',
              not s['open'] and set(s['stored']['seen']) == {'help', 'levels', 'rows', 'bookmarks', 'search'}, s['stored'])

    with ctx.launch(tips=True) as app:
        st, rect, tip, closed = helpers(app)
        app.click('shortcutHelp')
        app.wait(lambda: st()['dialog'], what='shortcut list')
        app.click('startTour')
        s = app.wait(lambda: (lambda v: v['modal'] and v['step'] and v)(st()), what='tour on the sample')
        check('without a file the tour opens the sample log and starts on it', s['title'] == '로그 열기' and not s['dialog']
              and app.text('largeName') == '샘플 로그' and s['step'] == '1 / 9', {**s, 'name': app.text('largeName')})
        app.key('ArrowRight')
        app.key('ArrowRight')
        app.wait(lambda: st()['title'] == '필터', what='filter step')
        app.key('Escape')
        s = closed('tour closed')
        check('Escape ends the tour with focus back in the page', s['focus'] in ('logsViewport', 'shortcutHelp'), s)
        time.sleep(2)
        check('the first tip is not offered after the tour', not st()['open'], st()['tip'])
