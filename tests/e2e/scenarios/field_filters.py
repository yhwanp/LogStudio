"""Tag, excluded-tag, PID and TID lists: OR within a field, AND across
fields, iOS secondary tags, item and length errors, the help text, long
chips (lists.log, counted against a row-at-a-time oracle)."""
import re
import time

import fixtures

STATE = """const $=id=>document.getElementById(id);return {
  idError:$('idFilterError').hidden?null:$('idFilterError').textContent,tagError:$('tagFilterError').hidden?null:$('tagFilterError').textContent,
  pidInvalid:$('pidFilter').getAttribute('aria-invalid'),tidInvalid:$('tidFilter').getAttribute('aria-invalid'),
  tagInvalid:$('tagFilter').getAttribute('aria-invalid'),hides:[...document.querySelectorAll('#hideList .cond-summary')].map(e=>e.textContent),
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
        pids = [p for p in re.split(r'[\s,]+', pid) if p]
        tags = [t.strip().lower() for t in tag.split(',') if t.strip()]
        excluded = [t.strip().lower() for t in exclude.split(',') if t.strip()]
        hit = lambda items, row_tags: any(i in t.lower() for t in row_tags for i in items)
        return sum(1 for p, ts in rows if (not pids or p in pids) and (not tags or hit(tags, ts)) and not hit(excluded, ts))
    return expected


def run(ctx):
    expected = oracle(fixtures.lists(ctx.fixtures))
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        st = lambda: js(STATE)
        chips = lambda: js("return [...document.querySelectorAll('#activeFilters .filter-chip')].map(c=>[c.firstChild.textContent,c.title])")

        def enter(id, value):
            app.enter(id, value)
            time.sleep(.35)
        app.open_fixture('lists.log')
        app.wait_count(60)
        # Pin the panel so screenshots show the fields next to the results.
        pin = "const a=document.getElementById('autoHideFilters');a.checked=arguments[0];a.dispatchEvent(new Event('change',{bubbles:true}));return true;"
        js("const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;")
        js(pin, False)
        time.sleep(.5)
        app.click('openIds')
        fit = js("""const c=document.createElement('canvas').getContext('2d');return ['tagFilter','pidFilter','tidFilter'].map(id=>{const e=document.getElementById(id),s=getComputedStyle(e);c.font=s.font;
          return [id,e.placeholder,Math.ceil(c.measureText(e.placeholder).width),e.clientWidth-parseFloat(s.paddingLeft)-parseFloat(s.paddingRight)];});""")
        check('placeholders fit their inputs', all(w <= room for _, _, w, room in fit), fit)
        enter('pidFilter', '2983, 3120')
        app.wait_count(expected('2983, 3120'))
        check('a PID list keeps the rows of either PID', True, {'count': app.count()})
        enter('tagFilter', 'ActivityManager, MyApp')
        app.wait_count(expected('2983, 3120', 'ActivityManager, MyApp'))
        c = chips()
        check('the tag list ANDs with the PID list; one chip per field', [x[0] for x in c] == ['태그: ActivityManager, MyApp', 'PID: 2983, 3120'] and st()['badge'] == '2', c)
        app.capture('01-lists-applied')
        enter('pidFilter', '3870 3871')
        enter('tagFilter', 'sdk, network')
        app.wait_count(expected('3870 3871', 'sdk, network'))
        check('whitespace-separated PIDs and iOS secondary tags', True, {'count': app.count()})
        enter('tagFilter', '')
        enter('pidFilter', '2983, x')
        s = st()
        check('a non-digit item is explained beside the IDs; the last list stays applied',
              s['idError'] and '숫자만' in s['idError'] and s['pidInvalid'] == 'true' and s['tidInvalid'] == 'false' and s['pending'], s)
        app.capture('02-pid-invalid')
        enter('pidFilter', ', '.join(str(1000 + i) for i in range(33)))
        s = st()
        check('a 33-item ID list is refused with the limit', s['idError'] and '최대 32개' in s['idError'], s)
        enter('pidFilter', '')
        app.wait_count(60)
        app.exclude_tags(', '.join(f'tag{i}' for i in range(33)))
        time.sleep(.35)
        s = st()
        check('a 33-item tag list of a hiding condition is explained below the tag field and on its card',
              s['tagError'] and '최대 32개' in s['tagError'] and s['tagInvalid'] == 'true' and s['idError'] is None and s['hides'][0].endswith('확인 필요'), s)
        app.capture('03-tag-list-limit')
        app.exclude_tags('chatty')
        app.wait_count(expected(exclude='chatty'))
        s = st()
        check('fixing the list clears the error and applies it', s['tagError'] is None and s['tagInvalid'] == 'false' and s['hides'] == ['태그: chatty'], s)
        app.click('openIds')
        js("document.querySelector('[aria-controls=idFilterHelp]').click();return true;")
        time.sleep(.2)
        help_text = js("const h=document.getElementById('idFilterHelp');return h.hidden?null:h.textContent")
        check('the PID/TID help explains lists', help_text and '쉼표나 공백' in help_text and '32개' in help_text, help_text)
        app.capture('04-id-help')
        js("document.querySelector('[aria-controls=idFilterHelp]').click();return true;")
        app.exclude_tags('')
        time.sleep(.35)
        ids = ','.join(str(2983 + i) for i in range(32))
        enter('pidFilter', ids)
        app.wait_count(expected(ids))
        js(pin, True)
        app.click('collapseFilters')
        time.sleep(.5)
        c = chips()
        check('a long list chip is clipped and its tooltip keeps every item',
              c and c[0][0].endswith('…') and c[0][1].startswith('PID: ' + ', '.join(str(2983 + i) for i in range(32))), c)
        app.capture('05-long-list-chip')
        js("document.querySelector('#activeFilters .filter-chip').click();return true;")
        app.wait_count(60)
        check('removing the chip clears the whole list', js("return document.getElementById('pidFilter').value") == '' and not chips())
