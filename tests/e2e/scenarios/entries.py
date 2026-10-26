"""Multi-line log entries: joined rows, continuation cells and guides, the
whole-entry detail and copy, a long entry over two detail windows, search
hits and bookmarks inside an entry, and raw view (record.log)."""
import time

import fixtures

CELLS = """const r=document.querySelector(`#logsRows [data-line-number="${arguments[0]}"]`),s=getComputedStyle(r),m=r.lastElementChild;
  return {cls:r.className,border:s.borderBottomColor,cells:[...r.children].slice(1,6).map(c=>c.textContent),title:r.children[1].title,
  message:m.className,text:m.textContent,white:getComputedStyle(m).whiteSpace,guide:getComputedStyle(m).boxShadow,left:Math.round(m.getBoundingClientRect().left),
  bg:s.backgroundColor,height:Math.round(r.getBoundingClientRect().height)};"""
DETAIL = """const $=id=>document.getElementById(id);return {title:$('largeDetailTitle').textContent,text:$('largeDetailText').textContent,
  loading:$('largeDetailText').textContent==='원문 구간을 읽는 중…',navigation:!$('largeDetailNavigation').hidden,next:!$('largeDetailNext').disabled};"""
# The first line the detail shows, from its top left corner to the line end.
FIRST_SHOWN = """const t=document.getElementById('largeDetailText'),b=t.getBoundingClientRect(),c=document.caretRangeFromPoint(b.left+2,b.top+2);
  if(!c||!t.contains(c.startContainer))return null;const r=document.createRange();r.setStart(c.startContainer,c.startOffset);r.setEnd(t,t.childNodes.length);return r.toString().split('\\n')[0];"""
MORE = """const m=document.getElementById('largeDetailMore'),t=document.getElementById('largeDetailText'),s=JSON.parse(localStorage.getItem('logstudio.layout.v1')||'{}');
  return {shown:!m.hidden,label:m.textContent,name:m.getAttribute('aria-label'),fits:t.scrollHeight<=t.clientHeight+1,lines:s.detailLines,shades:['more-above','more-below'].filter(c=>t.classList.contains(c)),shadow:getComputedStyle(t).boxShadow};"""
MARK_SEEN = "const t=document.getElementById('largeDetailText').getBoundingClientRect(),m=document.querySelector('#largeDetailText mark')?.getBoundingClientRect();return !!m&&m.top>=t.top-1&&m.bottom<=t.bottom+1;"
CLEAR = 'rgba(0, 0, 0, 0)'


def run(ctx):
    meta = fixtures.records(ctx.fixtures)
    lines = meta['lines']
    entry = lambda a, b: '\n'.join(lines[a - 1:b])
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        cells = lambda n: js(CELLS, n)
        detail = lambda title: app.wait(lambda: (lambda v: v['title'] == title and not v['loading'] and v)(js(DETAIL)), 10, what=title)

        def first_shown(want):
            """The detail's first shown line, once it is `want` or after 2 s."""
            for _ in range(20):
                got = js(FIRST_SHOWN)
                if got == want:
                    break
                time.sleep(.1)
            return got
        app.open_fixture('record.log')
        app.wait_count(meta['rows'])
        a, b = meta['ios']
        head, cont, last = cells(a), cells(a + 2), cells(b)
        head_left = js("return Math.round(document.querySelector(`#logsRows [data-line-number=\"${arguments[0]}\"] .message`).getBoundingClientRect().left)", a)
        check('an entry row and its continuation rows join: no separator inside the entry, one after it',
              'level-row-D record-continued' in head['cls'] and 'continuation-row level-row-D record-continued' in cont['cls'] and 'record-continued' not in last['cls']
              and head['border'] == CLEAR and cont['border'] == CLEAR and last['border'] != CLEAR, [head['cls'], cont['cls'], last['cls'], last['border']])
        check('continuation field cells are empty and name their entry; physical row height is unchanged',
              cont['cells'] == [''] * 5 and cont['title'].startswith(f'{a}행 로그에 이어지는 줄\n2026-09-30 21:55:54.351980 · Debug · AlertsService') and cont['height'] == head['height'], cont)
        check('continuation text sits in the message column with its indentation and a level guide',
              cont['message'] == 'message continuation-text' and cont['text'] == lines[a + 1] and cont['white'] == 'pre' and cont['left'] >= head_left and 'inset' in cont['guide'],
              {'text': cont['text'], 'white': cont['white'], 'guide': cont['guide']})
        sa, sb = meta['android']
        trace = cells(sa + 1)
        check('a stack trace takes its Error entry tint and keeps the tab indent', 'continuation-row level-row-E' in trace['cls'] and trace['text'] == lines[sa] and trace['bg'] == cells(sa)['bg'], trace)
        check('an orphan row and the row after a malformed stamped row stay raw', all('raw-row' in cells(n)['cls'] for n in (1, meta['malformed'], meta['orphanTail'])))
        app.capture('01-columns')

        for n, label in ((a, 'entry row'), (a + 3, 'continuation row')):
            app.click_row(n)
            d = detail(f'{a}–{b}행 원문')
            check(f'selecting the {label} shows the whole entry without CR', d['text'] == entry(a, b) and '\r' not in d['text'] and not d['navigation'], d['title'])
            # A one-line detail opens at the selected row's line.
            shown = first_shown(lines[n - 1])
            check(f'the one-line detail opens at the {label}', shown == lines[n - 1], {'shown': shown, 'row': lines[n - 1]})
        app.capture('02-detail')
        # A one-line detail says how many lines are out of view; its button
        # fits the detail to the whole entry, and Home on the splitter goes
        # back to one line.
        rows = b - a + 1
        more = js(MORE)
        check('a one-line detail of a multi-line entry counts its hidden lines and shades the edge', more['shown'] and more['label'] == f'+{rows - 1}줄'
              and more['name'] == f'{rows - 1}줄 더 보기' and more['shades'] == ['more-above', 'more-below'] and 'inset' in more['shadow'], more)
        js("document.getElementById('largeDetailMore').click();return true;")
        fitted = app.wait(lambda: (lambda v: v['fits'] and v)(js(MORE)), what='detail fitted')
        check('the hidden-lines button fits the detail to the whole entry, unshaded, and saves it', not fitted['shown'] and fitted['lines'] == rows and fitted['shades'] == [] and fitted['shadow'] == 'none', fitted)
        app.capture('02a-detail-fitted')
        js("const s=document.getElementById('detailSplitter');s.focus();s.dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true,cancelable:true}));return true;")
        back = app.wait(lambda: (lambda v: v['shown'] and v)(js(MORE)), what='one line again')
        check('Home on the detail splitter goes back to one line', back['lines'] == 1, back)
        app.capture_clipboard()
        app.click('largeDetailCopy')
        check('detail copy writes the whole entry', app.captured() == entry(a, b) and app.wait(lambda: app.text('toast') == f'{a}–{b}행 원문을 복사했습니다.', what='copy toast'))
        app.capture_clipboard()
        app.click_row(sa + 2, event='contextmenu')
        items = app.wait(lambda: js("const m=document.getElementById('rowMenu');return !m.hidden&&[...m.querySelectorAll('button')].map(b=>b.textContent)"), what='row menu')
        js("[...document.querySelectorAll('#rowMenu button')].find(b=>b.textContent==='로그 전체 원문 복사').click();return true;")
        check('the row menu of a stack trace row copies the whole Android entry', '로그 전체 원문 복사' in items and app.captured() == entry(sa, sb), items)
        for n in (meta['single'], meta['malformed'], meta['orphanTail']):
            app.click_row(n)
            detail(f'{n}행 원문')
        check('a single-row log, a malformed row and its tail each show one row', True)

        ba, bb = meta['big']
        app.jump(ba + 1200)
        started = time.monotonic()
        app.click_row(ba + 1200)
        d = detail(f'{ba:,}–{bb:,}행 원문')
        big = entry(ba, bb)
        check('a long entry opens at its first window with paging',
              d['navigation'] and d['next'] and big.startswith(d['text']) and len(d['text'].encode()) <= 65536,
              {'bytes': len(big.encode()), 'window': len(d['text'].encode()), 'seconds': round(time.monotonic() - started, 3)})
        shown = first_shown(lines[ba + 1199])
        check('the long entry opens at the selected row inside its first window', shown == lines[ba + 1199], shown)
        app.capture_clipboard()
        app.click('largeDetailCopy')
        check('copying the long entry joins every window', app.captured() == big, {'bytes': len(big.encode())})
        app.capture('03-long-entry')

        app.type('searchInput', 'Worker2')
        app.wait(lambda: js("return document.querySelectorAll('#largeRows .log-row').length") == 1, what='one search hit')
        hit = js("""const r=document.querySelector('#largeRows .log-row');return {line:r.dataset.lineNumber,cls:r.className,cells:[...r.children].slice(1,6).map(c=>c.textContent),
          opacity:getComputedStyle(r.children[1]).opacity,mark:r.querySelector('mark')?.textContent};""")
        check('a search hit in a stack trace shows its entry time, level and tag dimmed',
              hit['line'] == str(sa + 3) and 'continuation-row level-row-E' in hit['cls'] and hit['cells'] == ['09-21 12:07:30.386', '1234', '5678', 'Error', 'AndroidRuntime']
              and float(hit['opacity']) < 1 and hit['mark'] == 'Worker2', hit)
        app.click_row(sa + 3, pane='largeRows')
        d = detail(f'{sa}–{sb}행 원문')
        check('the hit detail shows the whole entry with the hit highlighted', d['text'] == entry(sa, sb) and js("return document.querySelector('#largeDetailText mark')?.textContent") == 'Worker2')
        shown = first_shown(lines[sa + 2])
        seen = js("const t=document.getElementById('largeDetailText').getBoundingClientRect(),m=document.querySelector('#largeDetailText mark').getBoundingClientRect();return m.top>=t.top-1&&m.bottom<=t.bottom+1;")
        check('the one-line hit detail opens at the hit row with its mark in view', shown == lines[sa + 2] and seen, {'shown': shown, 'seen': seen})
        app.capture('04-search')

        # A word deep inside a long row: the one-line detail opens at the hit.
        app.type('searchInput', 'NEEDLE')
        app.wait(lambda: js("return document.querySelectorAll('#largeRows .log-row').length") == 1, what='needle hit')
        app.click_row(meta['needle'], pane='largeRows')
        detail(f"{meta['needle']:,}행 원문")
        seen = app.wait(lambda: js(MARK_SEEN), 3, what='hit in view')
        check('a hit deep inside a long row opens the one-line detail at the hit', seen)
        app.capture('04a-deep-hit')
        app.type('searchInput', '')
        app.jump(1)
        time.sleep(.3)
        app.click_row(a)
        app.click_row(a + 2, shift=True)
        app.key('b', meta=True, target='logsViewport')
        app.wait(lambda: app.text('bookmarkTab') == '북마크 3', what='3 bookmarks')
        app.click('bookmarkTab')
        marks = app.wait(lambda: (lambda v: len(v) == 3 and v)(js("""return [...document.querySelectorAll('#bookmarkList .log-row')].map(r=>({line:r.dataset.lineNumber,cls:r.className,
          cells:[...r.children].slice(1,6).map(c=>c.textContent),text:r.children[6]?.textContent,white:getComputedStyle(r.children[6]).whiteSpace,border:getComputedStyle(r).borderBottomColor}));""")), what='bookmark rows')
        check('bookmarked rows of one entry show its fields dimmed, keep indentation and share no separator',
              'level-row-D record-continued' in marks[0]['cls'] and 'continuation-row level-row-D record-continued' in marks[1]['cls'] and 'record-continued' not in marks[2]['cls']
              and marks[1]['cells'] == ['2026-09-30 21:55:54.351980', '', '', 'Debug', 'AlertsService'] and marks[2]['text'] == lines[a + 1] and marks[2]['white'] == 'pre'
              and marks[0]['border'] == CLEAR and marks[2]['border'] != CLEAR, marks)
        app.capture('05-bookmarks')
        app.click('largeRaw')
        time.sleep(.3)
        app.jump(1)
        time.sleep(.3)
        raw = app.wait(lambda: cells(a + 2), what='raw row')
        check('raw view keeps physical rows', 'raw-row' in raw['cls'] and 'continuation' not in raw['cls'] and raw['border'] != CLEAR, raw['cls'])
        app.click('largeColumns')
