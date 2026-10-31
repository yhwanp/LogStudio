"""Tag, excluded-tag, PID and TID fields: one value or a pattern, AND across
fields, iOS secondary tags, the comma hint and errors, the time help, long
chips (lists.log, counted against a row-at-a-time oracle)."""
import re
import time

import fixtures

STATE = """const $=id=>document.getElementById(id);return {
  pidError:$('pidFilterError').hidden?null:$('pidFilterError').textContent,tagHint:$('tagFilterHint').hidden?null:$('tagFilterHint').firstChild.textContent,
  pidInvalid:$('pidFilter').getAttribute('aria-invalid'),tidInvalid:$('tidFilter').getAttribute('aria-invalid'),
  hides:[...document.querySelectorAll('.cond-item')].filter(c=>c.querySelector('.cond-kind.hide')).map(c=>c.querySelector('.cond-summary').textContent),
  pending:$('logsPending').hidden?null:$('logsPending').textContent,badge:$('filterBadge').hidden?null:$('filterBadge').textContent};"""


def oracle(lines):
    rows = []
    for line in lines:
        m = re.match(r'06-18 \S+\s+(\d+) (\d+) \w (\S+)\s*:', line)
        if m:
            rows.append((m[1], [m[3]]))
            continue
        m = re.match(r'\S+ \S+ Runner\[(\d+):(\d+)\] (.*)', line)
        rows.append((m[1], re.findall(r'\[([^\]]+)\]', m[3])))

    def expected(pid='', tag='', exclude=''):
        """Rows of any of the '|'-separated PIDs and tags."""
        pids = [p for p in pid.split('|') if p]
        tags = [t.lower() for t in tag.split('|') if t]
        excluded = [t.lower() for t in exclude.split('|') if t]
        hit = lambda items, row_tags: any(i in t.lower() for t in row_tags for i in items)
        return sum(1 for p, ts in rows if (not pids or p in pids) and (not tags or hit(tags, ts)) and not hit(excluded, ts))
    return expected


def run(ctx):
    expected = oracle(fixtures.lists(ctx.fixtures))
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        st = lambda: js(STATE)
        chips = lambda: js("return [...document.querySelectorAll('#activeFilters .filter-chip')].map(c=>[c.querySelector('.chip-edit').firstChild.textContent,c.querySelector('.chip-edit').title])")

        def enter(id, value):
            app.enter(id, value)
            time.sleep(.35)

        def regex(id, on):
            js("const c=document.getElementById(arguments[0]);if(c.checked!==arguments[1])c.click();return true;", id, on)
            time.sleep(.35)
        app.open_fixture('lists.log')
        app.wait_count(60)
        # Pin the panel so screenshots show the fields next to the results.
        pin = "const a=document.getElementById('autoHideFilters');a.checked=arguments[0];a.dispatchEvent(new Event('change',{bubbles:true}));return true;"
        js("const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;")
        js(pin, False)
        time.sleep(.5)
        app.click('addCondition')
        time.sleep(.3)
        fit = js("""const c=document.createElement('canvas').getContext('2d');return ['timeFrom','timeTo'].map(id=>{const e=document.getElementById(id),s=getComputedStyle(e);c.font=s.font;
          return [id,e.placeholder,Math.ceil(c.measureText(e.placeholder).width),e.clientWidth-parseFloat(s.paddingLeft)-parseFloat(s.paddingRight)];});""")
        check('placeholders fit their inputs', all(w <= room for _, _, w, room in fit), fit)
        regex('pidRegex', True)
        enter('pidFilter', '^(2983|3120)$')
        app.wait_count(expected('2983|3120'))
        check('a PID pattern keeps the rows of either PID', True, {'count': app.count()})
        regex('tagRegex', True)
        enter('tagFilter', 'ActivityManager|MyApp')
        app.wait_count(expected('2983|3120', 'ActivityManager|MyApp'))
        c = chips()
        check('the tag pattern ANDs with the PID pattern; one chip per field', [x[0] for x in c] == ['태그 정규식: ActivityManager|MyApp', 'PID 정규식: ^(2983|3120)$'] and st()['badge'] == '1', c)
        app.capture('01-patterns-applied')
        enter('pidFilter', '^(3870|3871)$')
        enter('tagFilter', 'sdk|network')
        app.wait_count(expected('3870|3871', 'sdk|network'))
        check('a pattern finds iOS secondary tags', True, {'count': app.count()})
        # A comma is text; the field says so.
        regex('tagRegex', False)
        enter('tagFilter', 'sdk, network')
        app.wait_count(0)
        s = st()
        check('a comma in a value is text, and the field says how to write several values', s['tagHint'] == '쉼표도 글자입니다. 여러 값은 정규식으로 찾습니다.', s)
        app.capture('01-comma-hint')
        regex('pidRegex', False)
        enter('tagFilter', '')
        enter('pidFilter', '2983, 3120')
        s = st()
        check('a typed list of PIDs is explained beside the field; the last value stays applied',
              s['pidError'] and '정규식으로' in s['pidError'] and s['pidInvalid'] == 'true' and s['tidInvalid'] == 'false' and s['pending'], s)
        app.capture('02-pid-invalid')
        enter('pidFilter', 'x')
        s = st()
        check('a non-digit PID is refused', s['pidError'] and '숫자만' in s['pidError'], s)
        enter('pidFilter', '')
        app.wait_count(60)
        app.exclude_tags('chatty')
        app.wait_count(expected(exclude='chatty'))
        s = st()
        check('a hiding condition applies and lists its value', s['hides'] == ['태그: chatty'], s)
        js("document.querySelector('[aria-controls=timeFilterHelp]').click();return true;")
        time.sleep(.2)
        help_text = js("const h=document.getElementById('timeFilterHelp');return h.hidden?null:h.textContent")
        check('the time help opens in the editor', help_text and '앞에서부터' in help_text, help_text)
        app.capture('04-time-help')
        js("document.querySelector('[aria-controls=timeFilterHelp]').click();return true;")
        app.exclude_tags('')
        time.sleep(.35)
        ids = '|'.join(str(2983 + i) for i in range(32))
        regex('pidRegex', True)
        enter('pidFilter', ids)
        app.wait_count(expected(ids))
        app.close_editor()
        js(pin, True)
        app.click('collapseFilters')
        time.sleep(.5)
        c = chips()
        check('a long chip is clipped and its tooltip keeps all of it',
              c and c[0][0].endswith('…') and c[0][1].startswith('PID 정규식: ' + ids), c)
        app.capture('05-long-chip')
        js("document.querySelector('#activeFilters .chip-remove').click();return true;")
        app.wait_count(60)
        check('removing the chip clears its field', js("return document.getElementById('pidFilter').value") == '' and not chips())
