"""A new filter keeps the lists' places: the selected row at its height on
screen with its detail, in the logs and in the search results, and the next
result in its place when the row is filtered out. Message conditions: the
field, its .* switch, a pattern the engine rejects (the last conditions stay)
and the row menu with text selected in a row (ui.log)."""
import time

import fixtures

STATE = """const $=id=>document.getElementById(id),top=(pane,line)=>{const r=document.querySelector(`#${pane} [data-line-number="${line}"]`);
 return r?Math.round(r.getBoundingClientRect().top-r.closest('.large-viewport').getBoundingClientRect().top):null;};
const sel=pane=>document.querySelector(`#${pane} .selected-log-row`)?.dataset.lineNumber??null;
return {logs:sel('logsRows'),search:sel('largeRows'),logsTop:top('logsRows',sel('logsRows')),searchTop:top('largeRows',sel('largeRows')),
 detail:$('largeDetail').hidden?null:$('largeDetailTitle').textContent,toast:$('toast').hidden?null:$('toast').textContent,
 count:$('logsCount').textContent,searchCount:$('largeCount').textContent,chips:[...$('activeFilters').querySelectorAll('.chip-joint,.filter-chip')].map(e=>e.textContent),
 messageError:$('messageFilterError').hidden?null:$('messageFilterError').textContent,invalid:$('messageFilter').getAttribute('aria-invalid'),
 pending:$('logsPending').hidden?null:$('logsPending').textContent,banner:$('largeError').hidden?null:$('largeError').textContent,regex:$('messageRegex').getAttribute('aria-pressed')};"""
ONLY = """const b=document.querySelector(`#levelFilters .level-chip[data-level="${arguments[0]}"]`);b.dispatchEvent(new MouseEvent('click',{bubbles:true,altKey:true}));return true;"""
OPEN_PANEL = "const t=document.getElementById('toggleFilters');if(t.getAttribute('aria-expanded')!=='true')t.click();return true;"
PIN = "const a=document.getElementById('autoHideFilters');a.checked=arguments[0];a.dispatchEvent(new Event('change',{bubbles:true}));return true;"
# Selects `arguments[1]` inside the message of row `arguments[0]` of the logs.
PICK = """const r=document.querySelector(`#logsRows [data-line-number="${arguments[0]}"] .message`),t=r.firstChild,at=t.textContent.indexOf(arguments[1]);
const range=document.createRange();range.setStart(t,at);range.setEnd(t,at+arguments[1].length);const s=getSelection();s.removeAllRanges();s.addRange(range);return String(s);"""
MENU = """const r=document.querySelector(`#logsRows [data-line-number="${arguments[0]}"]`),b=r.getBoundingClientRect();
r.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:b.left+300,clientY:b.top+10,button:2}));
return [...document.querySelectorAll('#rowMenu button')].map(e=>e.firstChild?.textContent??e.textContent);"""
CHOOSE = "const b=[...document.querySelectorAll('#rowMenu button')].find(e=>(e.firstChild?.textContent??e.textContent)===arguments[0]);b.click();return true;"


def run(ctx):
    fixtures.ui(ctx.fixtures)
    check = ctx.check
    with ctx.launch() as app:
        js = app.js
        st = lambda: js(STATE)
        app.open_fixture('ui.log')
        app.wait_count(3000)
        js(OPEN_PANEL)
        js(PIN, False)
        time.sleep(.4)
        # The search results keep their selected row too.
        app.type('searchInput', 'Network connected')
        app.wait(lambda: st()['searchCount'] == '429건', what='429 search results')
        app.js("const i=document.getElementById('largeJump');i.value='1004';document.getElementById('largeJumpForm').requestSubmit();return true;")
        time.sleep(.5)
        app.click_row(1004, pane='largeRows')
        app.wait(lambda: st()['detail'] == '1,004행 원문', what='detail of row 1004')
        before = st()
        js(ONLY, 'W')
        app.wait_count(300)
        app.wait(lambda: st()['searchCount'] == '43건', what='43 search results')
        time.sleep(.5)
        s = st()
        check('a new filter keeps the selected search result at its height, with its detail',
              s['search'] == '1004' and s['searchTop'] == before['searchTop'] and s['detail'] == '1,004행 원문' and s['logs'] == '1004', {'before': before, 'after': s})
        # The upper logs, which show the selected search result, follow the row the search puts
        # in place of a hidden one: row 24 (Warn) gives way to row 31 (Info).
        app.click('allLevels')
        app.wait_count(3000)
        app.js("const i=document.getElementById('largeJump');i.value='24';document.getElementById('largeJumpForm').requestSubmit();return true;")
        time.sleep(.5)
        app.click_row(24, pane='largeRows')
        app.wait(lambda: st()['logs'] == '24', what='upper logs follow row 24')
        js(ONLY, 'I')
        app.wait(lambda: st()['search'] == '31', what='search row 31 stands in')
        app.wait(lambda: st()['logs'] == '31', what='upper logs follow row 31')
        check('the upper logs follow the row the search puts in place of the hidden one', st()['logs'] == '31', st())
        app.click('allLevels')
        app.wait_count(3000)
        app.type('searchInput', '')
        time.sleep(.4)
        # The logs: the selected row stays where it was on screen.
        js(ONLY, 'W')
        app.wait_count(300)
        app.jump(1004)
        time.sleep(.5)
        app.click_row(1004)
        app.wait(lambda: st()['detail'] == '1,004행 원문', what='detail of row 1004')
        before = st()
        js("document.querySelector('#levelFilters .level-chip[data-level=\"E\"]').click();return true;")
        app.wait_count(600)
        time.sleep(.5)
        s = st()
        check('the selected row and its detail stay where they were', s['logs'] == '1004' and s['logsTop'] == before['logsTop'] and s['detail'] == '1,004행 원문', {'before': before, 'after': s})
        js(ONLY, 'E')
        app.wait(lambda: st()['logs'] == '1007', what='row 1007 selected')
        s = st()
        check('a row filtered out gives its place to the next result, its detail too, and says so',
              s['logsTop'] == before['logsTop'] and s['detail'] == '1,007행 원문' and s['toast'] == '1,004행은 새 필터 결과에 없어 다음 결과인 1,007행을 선택했습니다.', s)
        app.capture('01-next-row-kept-in-place')
        app.click('allLevels')
        app.wait_count(3000)
        app.wait(lambda: st()['logs'] == '1004', what='row 1004 selected again')
        s = st()
        check('back to every level, the row the user chose returns in its place, with its detail',
              s['logsTop'] == before['logsTop'] and s['detail'] == '1,004행 원문', s)

        # Message conditions: a hiding condition of its own.
        app.click('addHide')
        time.sleep(.2)
        check('the message field waits behind its + button, as PID·TID and time do',
              js("return [document.getElementById('messageFields').hidden,document.getElementById('openMessage').hidden]") == [True, False])
        app.click('openMessage')
        time.sleep(.2)
        fit = js("""const c=document.createElement('canvas').getContext('2d'),e=document.getElementById('messageFilter'),s=getComputedStyle(e);c.font=s.font;
          return [e.placeholder,Math.ceil(c.measureText(e.placeholder).width),e.clientWidth-parseFloat(s.paddingLeft)-parseFloat(s.paddingRight)];""")
        check('the message placeholder fits beside the .* switch', fit[1] <= fit[2], fit)
        app.enter('messageFilter', 'frame RENDERED')
        app.wait_count(2571)
        s = st()
        check('a message hides the rows whose message contains it, ignoring case', s['chips'] == ['제외', '메시지: “frame RENDERED”×'], s)
        app.click('messageRegex')
        app.enter('messageFilter', 'Frame (rendered')
        app.wait(lambda: st()['messageError'] is not None, what='pattern error')
        time.sleep(.3)
        s = st()
        check('a pattern the engine rejects is explained by the field; the last conditions stay applied',
              s['messageError'].startswith('메시지 정규식을 확인해 주세요') and s['invalid'] == 'true' and s['regex'] == 'true' and s['banner'] is None
              and s['pending'] == '입력한 조건 미적용 · 이전 조건 결과' and s['count'] == '로그 · 전체 2,571건' and s['chips'] == ['제외', '메시지: “frame RENDERED”×'], s)
        app.capture('02-message-pattern-error')
        app.enter('messageFilter', r'^Frame rendered in \d+ ms$')
        app.wait(lambda: st()['messageError'] is None, what='pattern accepted')
        app.wait_count(2571)
        check('a pattern reads the message only: ^ is where the message starts', st()['chips'] == ['제외', '메시지 정규식: “^Frame rendered in \\d+ ms$”×'], st())
        js("document.querySelector('#hideList .cond-remove').click();return true;")
        app.wait_count(3000)

        # The row menu takes text selected in the row's message.
        line = 4  # i = 3: 'Slow response: 420 ms'
        app.jump(1)
        time.sleep(.4)
        app.click_row(line)
        picked = js(PICK, line, 'Slow response')
        items = js(MENU, line)
        check('the row menu offers the selected text of the message', picked == 'Slow response' and "메시지 'Slow response' 제외" in items
              and "메시지 'Slow response' 보기 조건 추가" in items, items)
        app.capture('03-row-menu-message')
        js(CHOOSE, "메시지 'Slow response' 제외")
        app.wait_count(2571)
        app.wait(lambda: st()['logs'] == '5', what='row 5 takes the hidden row\'s place')
        s = st()
        check('hiding it adds a hiding condition whose undo toast stays; the next row takes its place',
              s['chips'] == ['제외', '메시지: “Slow response”×'] and s['toast'] == '제외 조건을 추가했습니다 · 메시지: “Slow response”실행 취소' and s['detail'] == '5행 원문', s)
