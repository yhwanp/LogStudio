"""Time bounds: inclusive date+time bounds, malformed and inverted bounds,
the error moving to the banner when the panel closes, ranges over midnight,
the pending note for invalid input, and the row menu's start time."""
import time

import fixtures


def run(ctx):
    fixtures.ui(ctx.fixtures)
    fixtures.days(ctx.fixtures)
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        chips = lambda: js("return [...document.querySelectorAll('#activeFilters .filter-chip')].map(e=>e.textContent)")
        error = "return {hidden:document.getElementById('timeFilterError').hidden,text:document.getElementById('timeFilterError').textContent,invalid:document.getElementById('timeTo').getAttribute('aria-invalid')};"
        app.open_fixture('ui.log')
        app.wait_count(3000)
        # ui.log minute 24 holds rows 0..1199, 20 rows per second.
        app.click('toggleFilters')
        app.enter('timeFrom', '06-18 19:24:10')
        app.enter('timeTo', '06-18 19:24:12.5')
        app.wait_count(52)
        t = js("return document.querySelector('#logsRows .log-row .timestamp')?.textContent")
        check('date+time bounds filter inclusively to the given precision',
              chips() == ['시간: 06-18 19:24:10 ~ 06-18 19:24:12.5×'] and t == '06-18 19:24:10.000' and js(error)['hidden'], {'chips': chips(), 'first': t})
        app.enter('timeTo', '19:24:1')
        e = app.wait(lambda: (lambda v: not v['hidden'] and v)(js(error)), what='time error')
        check('a malformed bound is explained beside the field', e['invalid'] == 'true', e)
        app.capture('01-time-error')
        js("const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')==='false')t.click();document.getElementById('collapseFilters').click();return true;")
        time.sleep(.3)
        b = js("return {collapsed:document.getElementById('toggleFilters').getAttribute('aria-expanded'),hidden:document.getElementById('largeError').hidden,text:document.getElementById('largeError').textContent};")
        check('closing the panel moves the field error to the banner', b['collapsed'] == 'false' and not b['hidden'] and '시간' in b['text'], b)
        app.capture('02-time-error-banner')
        app.click('toggleFilters')
        time.sleep(.3)
        check('reopening the panel shows the error inline only', js("return document.getElementById('largeError').hidden&&!document.getElementById('timeFilterError').hidden"))
        js("document.querySelector('#activeFilters .filter-chip').click();return true;")
        app.wait_count(3000)
        js("document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));return true;")

        app.click_row(5, event='contextmenu')
        app.wait(lambda: not app.hidden('rowMenu'), what='row menu')
        js("[...document.querySelectorAll('#rowMenu [role=menuitem]')].find(e=>e.textContent.endsWith('부터 보기 조건 추가')).click();return true;")
        app.wait_count(2996)
        check('the row menu sets a start time from the row', js("return document.getElementById('timeFrom').value") == '06-18 19:24:00.200')
        js("document.querySelector('#activeFilters .filter-chip').click();return true;")
        app.wait_count(3000)

        # days.log: a row every 30 minutes over two days.
        app.open_fixture('days.log')
        app.wait_count(96)
        js("const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;")
        stamps = "return [...document.querySelectorAll('#logsRows .log-row .timestamp')].map(e=>e.textContent)"
        for frm, to, n, first, last in [
            ('06-18 23:00', '01:00', 5, '06-18 23:00:00.000', '06-19 01:00:00.000'),
            ('06-18 12:00', '13:00', 3, '06-18 12:00:00.000', '06-18 13:00:00.000'),
            ('23:00', '06-19 01:00', 5, '06-18 23:00:00.000', '06-19 01:00:00.000'),
        ]:
            app.enter('timeFrom', frm)
            app.enter('timeTo', to)
            try:
                app.wait_count(n, 10)
            except Exception:
                pass
            rows = js(stamps)
            check(f'{frm} ~ {to} runs forward from the start', app.count() == f'로그 · 전체 {n}건' and rows[:1] == [first] and rows[-1:] == [last], {'count': app.count(), 'rows': rows})
        # Invalid input without a search: no empty lower pane, a note above.
        app.enter('timeTo', '01:0')
        time.sleep(.4)
        s = js("return {pane:document.getElementById('largeSearchPane').hidden,note:document.getElementById('logsPending').hidden,text:document.getElementById('logsPending').textContent,count:document.getElementById('logsCount').textContent,cancel:document.getElementById('largeCancel').disabled}")
        check('invalid input keeps the search pane closed and notes the upper logs', s['pane'] and not s['note'] and '미적용' in s['text'] and s['count'] == '로그 · 전체 5건' and s['cancel'], s)
        app.capture('03-pending-note')
        app.enter('timeTo', '06-19 01:00')
        time.sleep(.4)
        check('valid input clears the note', js("return document.getElementById('logsPending').hidden"))
        # With a search, the lower pane stays and both panes show the pending state.
        app.enter('searchInput', 'WIFI')
        app.wait(lambda: app.text('largeCount') == '5건', what='search count')
        app.enter('timeTo', '01:0')
        time.sleep(.4)
        s = js("return {pane:document.getElementById('largeSearchPane').hidden,note:document.getElementById('logsPending').hidden,total:document.getElementById('largeTotal').textContent,cancel:document.getElementById('largeCancel').disabled}")
        check('an existing search stays visible while its scope is pending', not s['pane'] and not s['note'] and '미적용' in s['total'] and not s['cancel'], s)
        app.capture('04-pending-with-search')
        app.enter('timeTo', '06-19 01:00')
        app.wait(lambda: app.text('largeCount') == '5건', what='search count')
        check('fixing the input restores both panes', js("return document.getElementById('logsPending').hidden&&!document.getElementById('largeSearchPane').hidden"))
        app.enter('searchInput', '')
        # An inverted dated range applies and explains the empty result.
        app.enter('timeFrom', '12-31 23:00')
        app.enter('timeTo', '01-01 01:00')
        app.wait_count(0, 10)
        h = js("return {hidden:document.getElementById('timeFilterError').hidden,text:document.getElementById('timeFilterError').textContent,pending:document.getElementById('logsPending').hidden}")
        check('an inverted dated range applies and explains the empty result', not h['hidden'] and '해를 넘기지' in h['text'] and h['pending'], h)
        app.enter('timeFrom', '')
        app.enter('timeTo', '')
        app.wait_count(96, 10)
        check('clearing the range removes the hint', app.hidden('timeFilterError'))
