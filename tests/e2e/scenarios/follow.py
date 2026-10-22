"""Following a growing log: the toggle, rows arriving at the end, kept
selection and scroll position, the completed last row, a growing search,
rows written without an ending, announcing rows while off, closing, and
stopping on truncation or NUL bytes (live.log)."""
import time

import fixtures
from fixtures import follow_row as row

STATE = """const v=document.getElementById('logsViewport'),rows=[...document.querySelectorAll('#logsRows .log-row')],b=document.getElementById('followFile');return {
  count:document.getElementById('logsCount').textContent,status:document.getElementById('largeStatus').textContent,
  button:{hidden:b.hidden,disabled:b.disabled,pressed:b.getAttribute('aria-pressed'),text:b.textContent},
  first:rows[0]?.dataset.lineNumber??null,last:rows.at(-1)?.dataset.lineNumber??null,lines:rows.map(r=>r.dataset.lineNumber),
  bottom:Math.round(v.scrollHeight-v.scrollTop-v.clientHeight),
  detail:document.getElementById('largeDetail').hidden?null:document.getElementById('largeDetailTitle').textContent,
  error:document.getElementById('largeError').hidden?null:document.getElementById('largeError').textContent,
  exportDisabled:document.getElementById('exportLogs').disabled,searchCount:document.getElementById('largeCount').textContent,searchTotal:document.getElementById('largeTotal').textContent};"""
TEXT_OF = "const r=document.querySelector('#logsRows [data-line-number=\"'+arguments[0]+'\"]');return r?r.textContent:null;"
SCROLL = "const v=document.getElementById('logsViewport');v.scrollTop=arguments[0];v.dispatchEvent(new Event('scroll'));return true;"


def run(ctx):
    live = fixtures.live(ctx.fixtures)

    def append(text, mode='a'):
        with open(live, mode, encoding=None if 'b' in mode else 'utf-8') as f:
            f.write(text)
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        st = lambda: js(STATE)
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(st()), timeout, what=what)
        app.open_fixture('live.log')
        s = until(lambda v: v['count'] == '로그 · 전체 300건' and v['status'] == '', 'opened')
        check('a single opened file shows the follow toggle, off',
              not s['button']['hidden'] and not s['button']['disabled'] and s['button']['pressed'] == 'false' and s['button']['text'] == '따라가기', s['button'])
        app.capture('01-follow-off', hover='followFile')
        app.click_row(5)
        until(lambda v: v['detail'] == '5행 원문', 'detail')
        app.click('followFile')
        until(lambda v: v['button']['pressed'] == 'true', 'following')
        s = until(lambda v: '300' in v['lines'] and v['bottom'] <= 28, 'end in view')
        check('turning it on shows following, partial counts and the end of the log', '따라가는 중' in s['status'] and '현재까지 300건' in s['count'] and s['exportDisabled'], s)
        check('turning it on keeps the selected row and its detail', s['detail'] == '5행 원문', s['detail'])
        app.capture('02-follow-on', hover='followFile')
        append(' completed\n' + ''.join(row(n, tag='Net' if n % 10 else 'Needle') + '\n' for n in range(301, 351)))
        s = until(lambda v: v['count'].endswith('350건') and '350' in v['lines'] and v['bottom'] <= 28, '350 rows')
        check('appended rows arrive and the end stays in view', True, {k: s[k] for k in ('count', 'last', 'bottom')})
        check('selection and detail are kept while rows arrive', st()['detail'] == '5행 원문')
        js(SCROLL, 290 * 28)
        t300 = app.wait(lambda: (lambda t: t and 'partial completed' in t and t)(js(TEXT_OF, '300')), 5, what='row 300 redrawn')
        check('the unterminated last row is redrawn once appended bytes complete it', 'partial completed' in t300, t300)
        app.type('searchInput', 'needle')
        s = until(lambda v: v['searchCount'] == '5건' and '따라가는 중' in v['searchTotal'], 'growing search')
        check('a search over the followed file waits for more rows', True, {k: s[k] for k in ('searchCount', 'searchTotal')})
        append(''.join(row(n, tag='Needle') + '\n' for n in range(351, 354)))
        s = until(lambda v: v['searchCount'] == '8건' and v['count'].endswith('353건'), 'search grows')
        check('search results grow with matching appended rows', True, {k: s[k] for k in ('searchCount', 'count')})
        js(SCROLL, 0)
        until(lambda v: v['first'] == '1', 'top')
        append(''.join(row(n) + '\n' for n in range(354, 380)))
        until(lambda v: v['count'].endswith('379건'), '379 rows')
        time.sleep(.3)
        check('after scrolling up, arriving rows leave the position alone', st()['first'] == '1', st()['first'])
        app.click('logsLast')
        until(lambda v: '379' in v['lines'], 'end')
        append(''.join(row(n) + '\n' for n in range(380, 400)))
        s = until(lambda v: '399' in v['lines'] and v['bottom'] <= 28, 'end in view again')
        check('back at the end, following keeps the end in view again', True, {k: s[k] for k in ('last', 'bottom')})
        append(row(400, text='written without an ending yet'))
        time.sleep(1.2)
        s = st()
        check('a row still being written is not shown yet', s['count'].endswith('399건') and '400' not in s['lines'], s['count'])
        app.click('followFile')
        s = until(lambda v: v['button']['pressed'] == 'false' and v['count'] == '로그 · 전체 400건' and '400' in v['lines'] and v['status'] == '', 'stopped')
        check('turning it off ends the rows at the file end and completes them', not s['exportDisabled'] and '따라가는 중' not in s['status'], {k: s[k] for k in ('count', 'status')})
        app.capture('03-follow-stopped')
        append(' and more\n' + row(401) + '\n')
        s = until(lambda v: '새 줄 있음' in v['status'], 'new rows announced', 10)
        check('rows appended while not following are announced, not read', s['count'] == '로그 · 전체 400건' and s['error'] is None, {k: s[k] for k in ('status', 'count')})
        app.capture('04-new-rows-announced')
        app.click('followFile')
        until(lambda v: v['count'].endswith('401건'), '401 rows')
        t400 = js(TEXT_OF, '400')
        check('following again reads them, completing the row closed when it stopped', t400 is not None and 'and more' in t400, t400)
        app.click('largeClose')
        until(lambda v: v['count'] == '로그 · 0건', 'closed')
        append(row(402) + '\n')
        time.sleep(1)
        check('closing the file stops following without errors', st()['error'] is None and st()['button']['hidden'], st()['button'])

        with open(live, 'r+') as f:
            f.truncate(100)
        app.open_fixture('live.log')
        until(lambda v: v['count'] == '로그 · 전체 2건', 'reopened short file')
        app.click('followFile')
        until(lambda v: v['button']['pressed'] == 'true', 'following')
        with open(live, 'r+') as f:
            f.truncate(40)
        s = until(lambda v: v['error'], 'truncation error', 10)
        check('a truncated followed file stops following with the reopen message', '크기가 줄어들었습니다' in s['error'] and s['button']['pressed'] == 'false', {k: s[k] for k in ('error', 'button')})
        app.capture('05-truncated')

        live.write_text(''.join(row(n) + '\n' for n in range(1, 50)), encoding='utf-8')
        app.open_fixture('live.log')
        until(lambda v: v['count'] == '로그 · 전체 49건', 'rewritten file')
        app.click('followFile')
        until(lambda v: v['button']['pressed'] == 'true', 'following')
        append(b'bad\0row\n', 'ab')
        s = until(lambda v: v['error'] and 'NUL' in v['error'], 'NUL error', 10)
        check('NUL in appended bytes stops following with a notice', s['button']['pressed'] == 'false', s['error'])
