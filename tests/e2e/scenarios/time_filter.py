"""Time bounds: inclusive date+time bounds, malformed and inverted bounds,
the error moving to the banner when the panel closes, ranges over midnight,
the pending note for invalid input, and the row menu's times, which narrow
the rows shown: a start and an end, or a selection, make one range. Logs that
cross a year end (year-end.log, year-end-ios.log): ranges typed, from a
selection, from one end and from search results."""
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
        app.close_editor()
        time.sleep(.3)
        b = js("return {hidden:document.getElementById('largeError').hidden,text:document.getElementById('largeError').textContent};")
        check('closing the editor moves the field error to the banner', not b['hidden'] and '시간' in b['text'], b)
        app.capture('02-time-error-banner')
        js("document.querySelector('.cond-item .cond-summary').click();return true;")
        time.sleep(.3)
        check('reopening the editor shows the error inline only', js("return document.getElementById('largeError').hidden&&!document.getElementById('timeFilterError').hidden"))
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(3000)
        js("document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));return true;")

        # Row menu times narrow what shows: a start, then an end, make one
        # range, where two showing conditions would join with 또는.
        menu = lambda n: (app.click_row(n, event='contextmenu'), app.wait(lambda: not app.hidden('rowMenu'), what='row menu'))
        find = "const i=[...document.querySelectorAll('#rowMenu [role=menuitem]')].find(e=>(e.querySelector('.menu-label')??e).textContent===arguments[0]);"
        pick = lambda label: js(find + "if(i)i.click();return !!i;", label)
        note = lambda label: js(find + "return i?.querySelector('.menu-sub')?.textContent??null;", label)
        items = "return document.querySelectorAll('#conditionList .cond-item').length"
        menu(5)
        check('the row menu offers the row time as a start', pick('06-18 19:24:00.200부터만 보기'))
        app.wait_count(2996)
        check('the row menu sets a start time from the row', js("return document.getElementById('timeFrom').value") == '06-18 19:24:00.200')
        menu(10)
        app.capture('05-row-menu-end')
        check('the end item names the range it makes', note('06-18 19:24:00.450까지만 보기') == '시간: 06-18 19:24:00.200 ~ 19:24:00.450', note('06-18 19:24:00.450까지만 보기'))
        check('the row menu offers the row time as an end', pick('06-18 19:24:00.450까지만 보기'))
        app.wait_count(6)
        check('a start and then an end make one range', chips() == ['시간: 06-18 19:24:00.200 ~ 06-18 19:24:00.450×'] and js(items) == 1, {'chips': chips(), 'items': js(items)})
        app.capture('06-time-range')
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(3000)
        # A selection narrows to the time from its first row to its last.
        app.click_row(3)
        app.click_row(8, shift=True)
        menu(8)
        app.capture('07-selection-range-menu')
        check('the range item counts the rows and names the range', note('선택한 6행의 시간 범위만 보기') == '시간: 06-18 19:24:00.100 ~ 19:24:00.350', note('선택한 6행의 시간 범위만 보기'))
        check('a selection offers its time range', pick('선택한 6행의 시간 범위만 보기'))
        app.wait_count(6)
        check('the selection range applies', chips() == ['시간: 06-18 19:24:00.100 ~ 06-18 19:24:00.350×'] and js(items) == 1, chips())
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
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
            check(f'{frm} ~ {to} runs forward from the start', app.count() == f'로그 · 결과 {n}건' and rows[:1] == [first] and rows[-1:] == [last], {'count': app.count(), 'rows': rows})
        # Invalid input without a search: no empty lower pane, a note above.
        app.enter('timeTo', '01:0')
        time.sleep(.4)
        s = js("return {pane:document.getElementById('largeSearchPane').hidden,note:document.getElementById('logsPending').hidden,text:document.getElementById('logsPending').textContent,count:document.getElementById('logsCount').textContent,cancel:document.getElementById('largeCancel').disabled}")
        check('invalid input keeps the search pane closed and notes the upper logs', s['pane'] and not s['note'] and '미적용' in s['text'] and s['count'] == '로그 · 결과 5건' and s['cancel'], s)
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
        app.enter('timeFrom', '06-19 12:00')
        app.enter('timeTo', '06-18 12:00')
        app.wait_count(0, 10)
        h = js("return {hidden:document.getElementById('timeFilterError').hidden,text:document.getElementById('timeFilterError').textContent,pending:document.getElementById('logsPending').hidden}")
        check('an inverted dated range applies and explains the empty result', not h['hidden'] and '반년 넘게' in h['text'] and h['pending'], h)
        app.capture('08-inverted-range')
        app.enter('timeFrom', '')
        app.enter('timeTo', '')
        app.wait_count(96, 10)
        check('clearing the range removes the hint', app.hidden('timeFilterError'))

        # A log without years that crosses a year end.
        fixtures.year_end(ctx.fixtures)
        app.open_fixture('year-end.log')
        app.wait_count(145)
        notes = "const t=id=>{const e=document.getElementById(id);return e.hidden?null:e.textContent};return {note:t('timeFilterNote'),error:t('timeFilterError')};"
        ends = lambda: (lambda rows: [rows[0], rows[-1]] if rows else rows)(js(stamps))
        app.enter('timeFrom', '12-31 23:00')
        app.enter('timeTo', '01-01 01:00')
        app.wait_count(5, 10)
        app.wait(lambda: ends() == ['12-31 23:00:00.000', '01-01 01:00:00.000'], what='rows around the year end')
        check('a typed range past the year end shows the rows between its ends and says so',
              js(notes) == {'note': '해를 넘기는 범위로 봅니다.', 'error': None}, js(notes))
        app.capture('09-year-end-range')
        app.close_editor()
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(145)
        # Rows 72 and 73 are 12-31 23:30 and 01-01 00:00.
        app.jump(72)
        app.wait(lambda: app.row_exists(70) and app.row_exists(75), what='rows of the year end')
        app.click_row(70)
        app.click_row(75, shift=True)
        menu(75)
        app.capture('10-year-end-selection-menu')
        check('a selection across the year end names its range in order', note('선택한 6행의 시간 범위만 보기') == '시간: 12-31 22:30:00.000 ~ 01-01 01:00:00.000', note('선택한 6행의 시간 범위만 보기'))
        pick('선택한 6행의 시간 범위만 보기')
        app.wait_count(6, 10)
        app.wait(lambda: ends() == ['12-31 22:30:00.000', '01-01 01:00:00.000'], what='the selected rows')
        check('it shows the selected rows', True, ends())
        app.capture('11-year-end-selection')
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(145)
        # One end of the log's year end takes the log's own other end.
        app.jump(72)
        app.wait(lambda: app.row_exists(72), what='row 72')
        menu(72)
        app.capture('12-year-end-one-side-menu')
        check('one end names the range it makes', note('12-31 23:30:00.000부터만 보기') == '시간: 12-31 23:30:00.000 ~ 01-02' and note('12-31 23:30:00.000까지만 보기') == '시간: 12-30 ~ 12-31 23:30:00.000',
              [note('12-31 23:30:00.000부터만 보기'), note('12-31 23:30:00.000까지만 보기')])
        pick('12-31 23:30:00.000부터만 보기')
        app.wait_count(74, 10)
        app.wait(lambda: app.row_exists(72), what='row 72')
        app.capture('13-year-end-one-side')
        check('a start keeps the rows of the new year', chips() == ['시간: 12-31 23:30:00.000 ~ 01-02×'], chips())
        app.wait(lambda: app.row_exists(75), what='row 75')
        menu(75)
        pick('01-01 01:00:00.000까지만 보기')
        app.wait_count(4, 10)
        check('an end after the year end narrows that range', chips() == ['시간: 12-31 23:30:00.000 ~ 01-01 01:00:00.000×'], chips())
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(145)
        app.jump(75)
        app.wait(lambda: app.row_exists(75), what='row 75')
        menu(75)
        pick('01-01 01:00:00.000까지만 보기')
        app.wait_count(75, 10)
        check('an end in the new year keeps the rows of the old', chips() == ['시간: 12-30 ~ 01-01 01:00:00.000×'], chips())
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(145)
        # A followed log has no last date yet.
        app.click('followFile')
        app.wait(lambda: js("return document.getElementById('followFile').getAttribute('aria-pressed')") == 'true', what='following')
        app.jump(72)
        app.wait(lambda: app.row_exists(72), what='row 72')
        menu(72)
        app.capture('14-year-end-followed-menu')
        check('a followed log past its year end cannot narrow to a start alone', note('12-31 23:30:00.000부터만 보기') == '해를 넘긴 로그를 따라가는 중에는 끝 시각도 필요합니다'
              and js(find + "return i.getAttribute('aria-disabled');", '12-31 23:30:00.000부터만 보기') == 'true', note('12-31 23:30:00.000부터만 보기'))
        js("document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));return true;")
        app.click('followFile')
        # Search results and bookmarks.
        app.enter('searchInput', 'YearTag')
        app.wait(lambda: app.text('largeCount') == '145건', what='search count')
        js("const v=document.getElementById('largeViewport');v.scrollTop=71/145*v.scrollHeight;v.dispatchEvent(new Event('scroll'));return true;")
        app.wait(lambda: app.row_exists(72, 'largeRows') and app.row_exists(74, 'largeRows'), what='search results of the year end')
        app.click_row(72, pane='largeRows')
        app.click_row(74, pane='largeRows', shift=True)
        app.click_row(74, pane='largeRows', event='contextmenu')
        app.wait(lambda: not app.hidden('rowMenu'), what='row menu')
        check('a selection of search results names its range in order', note('선택한 3행의 시간 범위만 보기') == '시간: 12-31 23:30:00.000 ~ 01-01 00:30:00.000', note('선택한 3행의 시간 범위만 보기'))
        pick('선택한 3행의 시간 범위만 보기')
        app.wait_count(3, 10)
        check('and shows its rows', True)
        app.enter('searchInput', '')
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(145)

        # Rows with years keep theirs.
        # It has as many rows as the log before.
        app.open_fixture('year-end-ios.log')
        app.wait(lambda: app.text('largeName') == 'year-end-ios.log', what='the new log')
        app.wait_count(145)
        app.jump(72)
        app.wait(lambda: app.row_exists(70) and app.row_exists(75), what='rows of the year end')
        app.click_row(71)
        app.click_row(74, shift=True)
        menu(74)
        check('a selection of rows with years names them', note('선택한 4행의 시간 범위만 보기') == '시간: 2026-12-31 23:00:00.134000 ~ 2027-01-01 00:30:00.134000', note('선택한 4행의 시간 범위만 보기'))
        pick('선택한 4행의 시간 범위만 보기')
        app.wait_count(4, 10)
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(145)
        app.jump(72)
        app.wait(lambda: app.row_exists(72), what='row 72')
        menu(72)
        check('one end with a year stays alone', pick('2026-12-31 23:30:00.134000부터만 보기'))
        app.wait_count(74, 10)
        check('and keeps the rows of the new year', chips() == ['시간: 2026-12-31 23:30:00.134000 ~ 끝×'], chips())

        # Ends 183 days apart take whole times, as their dates cross no year end.
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(145)
        app.open_fixture('half-year.log')
        app.wait_count(4)
        app.wait(lambda: app.row_exists(3), what='row 3')
        menu(3)
        app.capture('15-half-year-menu')
        check('a log whose ends are 183 days apart names whole times', note('01-01 08:30:00.000까지만 보기') == '시간: 07-02 10:00:00.000 ~ 01-01 08:30:00.000', note('01-01 08:30:00.000까지만 보기'))
        pick('01-01 08:30:00.000까지만 보기')
        app.wait_count(3, 10)
        check('and keeps its rows', chips() == ['시간: 07-02 10:00:00.000 ~ 01-01 08:30:00.000×'], chips())
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(4)
        app.wait(lambda: app.row_exists(2), what='row 2')
        menu(2)
        pick('07-02 11:00:00.000부터만 보기')
        app.wait_count(3, 10)
        check('so does its start', chips() == ['시간: 07-02 11:00:00.000 ~ 01-01 09:00:00.000×'], chips())
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(4)

        # A range of two years around a whole one takes every row without a year.
        app.open_fixture('whole-year.log')
        app.wait_count(5)
        js("const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;")
        app.enter('timeFrom', '2024-12-31')
        app.enter('timeTo', '2026-01-01')
        app.wait(lambda: app.count() == '로그 · 결과 5건', what='every row of the years')
        check('two years around a whole one show every row', js("return document.querySelectorAll('#logsRows .log-row').length;") == 5 and app.hidden('timeFilterError'), app.count())
