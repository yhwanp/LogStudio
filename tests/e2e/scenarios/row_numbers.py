"""Row-number column width: the default for small files, 8 digits for a
10,000,050-row file in the logs and in search results, file badges in a
combined log, and a chosen width kept until Home (rows-10m.log, ~400 MB)."""
import time

import fixtures

LARGE = True

# Width the row column needs for `digits` digits, measured as layout.js does.
NEED = """const cell=document.querySelector('#logsRows .log-row .line-number');const s=getComputedStyle(cell);const c=document.createElement('canvas').getContext('2d');
c.font=`700 ${s.fontSize} ${s.fontFamily}`;const d=Math.max(...'0123456789'.split('').map(x=>c.measureText(x).width));
return Math.ceil(d*arguments[0]+parseFloat(s.paddingLeft)+parseFloat(s.paddingRight));"""
# Row number cells that are cut off (text wider than the cell).
CUT = """const rows=[...document.querySelectorAll(arguments[0]+' .log-row')];return {line:getComputedStyle(document.getElementById('workspace')).getPropertyValue('--line-column').trim(),
rows:rows.length,last:rows.at(-1)?.dataset.lineNumber??null,cut:rows.filter(r=>{const c=r.children[0];return c.scrollWidth>c.clientWidth+.5;}).map(r=>r.dataset.lineNumber).slice(0,5),
text:rows.at(-1)?.children[0].textContent??null,header:document.querySelector('#logsHeaders .column-heading')?.textContent};"""
RESIZER = "const h=document.querySelector('#logsHeaders .column-resizer');h.focus();h.dispatchEvent(new KeyboardEvent('keydown',{key:arguments[0],shiftKey:arguments[1],bubbles:true}));return true;"


def run(ctx):
    fixtures.rows_10m(ctx.fixtures)
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        cut = lambda pane='#logsRows': js(CUT, pane)
        app.open_fixture('small.log')
        app.wait_count(3000)
        time.sleep(.3)
        c0 = cut()
        check('a 3,000-row file keeps the default 52px row column', c0['line'] == '52px' and not c0['cut'], c0)
        need = js(NEED, 8)
        app.open_fixture('rows-10m.log')
        app.wait_count(10_000_050, 120)
        time.sleep(.3)
        c1 = cut()
        check('a 10,000,050-row file widens the row column to fit 8 digits', c1['line'] == f'{max(52, need + 2)}px', {'state': c1, 'need': need})
        app.click('logsLast')
        app.wait(lambda: cut()['last'] == '10000050', 30, what='last row')
        time.sleep(.4)
        c2 = cut()
        check('the last rows (8-digit numbers) are not cut off', not c2['cut'] and c2['text'] == '10000050', c2)
        app.capture('01-row-numbers-8-digits')
        app.type('searchInput', 'PolishTail')
        app.wait(lambda: app.text('largeCount') == '1건', 60, what='search hit')
        time.sleep(.4)
        c3 = cut('#largeRows')
        check('search results share the widened column; the 8-digit hit is not cut off', c3['rows'] == 1 and not c3['cut'] and c3['text'] == '10000026', c3)
        app.capture('02-row-numbers-search')
        app.type('searchInput', '')
        app.open_fixtures(['small.log', 'rows-10m.log'])
        app.wait_count(10_003_050, 120)
        app.click('logsLast')
        app.wait(lambda: cut()['last'] == '10003050', 30, what='combined last row')
        time.sleep(.4)
        c4 = cut()
        check('combined log: the badge and the 8-digit in-file number fit (fitted width + 24px badge)',
              c4['line'] == f'{max(52, need + 2) + 24}px' and not c4['cut'] and c4['text'] == '210000050' and c4['header'] == '파일 · 행', c4)
        app.capture('03-row-numbers-multi-file')
        js(RESIZER, 'ArrowLeft', True)
        c5 = cut()
        js(RESIZER, 'Home', False)
        c6 = cut()
        check('a chosen width is kept (cut off) until Home restores the automatic width', c5['cut'] and not c6['cut'] and c6['line'] == c4['line'], {'chosen': c5, 'home': c6})
