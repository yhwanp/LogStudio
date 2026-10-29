"""Device captures with scripted devices (QA builds; no adb, device
service or device runs): a capture file a crashed run left, getting adb,
the device menu with Android devices and iPhones, recording, a device that
leaves and comes back, stopping and recording on, the save question before a capture's log
closes or the app quits, opening a left file, a capture that fails, an
iPhone whose developer build starts anew with its private values shown, and
an iPhone that leaves and must trust the computer again."""
import re
import time

MENU = "return [...document.querySelectorAll('#rowMenu>*')].map(e=>e.tagName==='HR'?'---':e.textContent)"
PICK = ("const b=[...document.querySelectorAll('#rowMenu>button')].find(b=>b.textContent.startsWith(arguments[0]));"
        "if(!b)return false;b.click();return true;")
QA = "window.__TAURI__.core.invoke('qa_devices',{action:arguments[0]});return true;"
# Chooses the app menu's Quit as ⌘Q does; the menu's item IDs land in
# window.__quit.
QUIT = "window.__quit=null;window.__TAURI__.core.invoke('qa_quit').then(ids=>{window.__quit=ids;});return true;"
STATE = """const bar=document.getElementById('captureBar'),rows=[...document.querySelectorAll('#logsRows .log-row')];return {
  bar:bar.hidden?null:{tone:bar.dataset.tone,text:document.getElementById('captureText').textContent,stop:!document.getElementById('captureStop').hidden,
    stopLabel:document.getElementById('captureStopName').textContent,focus:document.activeElement?.id??null,
    stopBox:(b=>[Math.round(b.left*2)/2,Math.round(b.right*2)/2])(document.getElementById('captureStop').getBoundingClientRect()),
    save:document.getElementById('captureSave').classList.contains('primary')},
  open:document.getElementById('openFile').classList.contains('primary'),
  count:document.getElementById('logsCount').textContent,following:document.getElementById('followFile').getAttribute('aria-pressed'),
  breaks:rows.filter(r=>r.classList.contains('capture-break')).map(r=>({line:Number(r.dataset.lineNumber),label:r.dataset.captureBreak??''})),
  dialog:document.getElementById('captureDialog').open,note:document.getElementById('captureDialogNote').textContent,
  adb:document.getElementById('adbDialog').open,empty:!document.getElementById('emptyState').hidden,
  leftovers:document.getElementById('captureLeftovers').hidden?[]:[...document.querySelectorAll('#leftoverList strong')].map(e=>e.textContent),
  error:document.getElementById('largeError').hidden?null:document.getElementById('largeError').textContent};"""
# Shows the first row after a break near the middle of the logs pane.
SHOW_BREAK = """const v=document.getElementById('logsViewport');const r=[...document.querySelectorAll('#logsRows .capture-break')].find(r=>r.dataset.captureBreak.includes('연결 끊김'));
if(!r)return false;v.scrollTop+=r.getBoundingClientRect().top-v.getBoundingClientRect().top-v.clientHeight/2;v.dispatchEvent(new Event('scroll'));return true;"""
# Brings the logs pane to its end, as scrolling down there does.
AT_END = "const v=document.getElementById('logsViewport');v.scrollTop=v.scrollHeight;v.dispatchEvent(new Event('scroll'));return true;"
# The top of the logs row with line arguments[0].
ROW_TOP = "const r=document.querySelector(`#logsRows [data-line-number=\"${arguments[0]}\"]`);return r?Math.round(r.getBoundingClientRect().top):null"
# Whether the row menu is open with focus, and beside the row with line
# arguments[0]: the row's middle lies within the menu's height.
OPEN = ("const m=document.getElementById('rowMenu'),r=document.querySelector(`#logsRows [data-line-number=\"${arguments[0]}\"]`)?.getBoundingClientRect(),b=m.getBoundingClientRect();"
        "return {open:!m.hidden,label:m.getAttribute('aria-label'),focus:m.contains(document.activeElement),beside:!!r&&b.top<=r.top+r.height/2+1&&r.top+r.height/2<=b.bottom+1}")
# The selected logs row's line.
SELECTED = "return document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null"
# The line of the second-to-last logs row drawn.
NEAR_END = "const r=[...document.querySelectorAll('#logsRows .log-row')];return r.length>3&&Number(r.at(-2).dataset.lineNumber)"
# The last row's line while the logs pane shows its end, else false.
END = ("const v=document.getElementById('logsViewport'),r=[...document.querySelectorAll('#logsRows .log-row')].at(-1);"
       "return !!r&&r.getBoundingClientRect().bottom<=v.getBoundingClientRect().bottom+1&&Number(r.dataset.lineNumber)")
# Whether the capture state's words end in an ellipsis instead of spilling
# onto the file name beside them.
FITS = """const t=document.getElementById('captureText'),info=document.querySelector('.file-info'),s=document.getElementById('captureState');
const a=info.getBoundingClientRect(),b=s.getBoundingClientRect();
return {clipped:t.scrollWidth>t.clientWidth,ellipsis:getComputedStyle(t).textOverflow,apart:a.right<=b.left+1&&getComputedStyle(info).overflow==='hidden',text:t.textContent};"""


def run(ctx):
    # A capture file a crashed run left in the app's cache.
    captures = ctx.home / 'Library/Caches/dev.logstudio.desktop/captures'
    captures.mkdir(parents=True)
    left = captures / 'logcat-Pixel_8-20261024-101500.log'
    text = ''.join(f'10-24 10:15:{i:02d}.000  4321  4321 I NotesUi: opened note {i}\n' for i in range(30))
    left.write_text(text, encoding='utf-8')
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        st = lambda: js(STATE)
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(st()), timeout, what=what)
        menu = lambda pred, what: app.wait(lambda: (lambda v: v and pred(v) and v)(js(MENU)), what=what)
        s = until(lambda v: v['empty'] and v['leftovers'], 'leftover listed')
        check('a capture file a crashed run left is offered with the empty state', s['leftovers'] == [left.name], s['leftovers'])
        app.capture('01-empty-leftover', hover='emptyDevice')

        # Without adb the device menu still lists iPhones, and one item leads
        # to getting adb. Scripted iPhones keep a connected one out of the test.
        js(QA, 'ios')
        js(QA, 'missing')
        app.click('emptyDevice')
        m = menu(lambda v: 'Android 기기 로그를 기록하려면 adb를 설치해 주세요…' in v, 'no adb menu')
        check('without adb the device menu lists iPhones and leads to getting adb',
              'QA iPhone · iOS 27.0' in m and 'adb 위치 지정…' not in m and 'Android' not in m, m)
        described = js("const m=document.getElementById('rowMenu'),ids=(m.getAttribute('aria-describedby')||'').split(' ');return ids.map(id=>document.getElementById(id)?.textContent)")
        check('the menu\'s notes describe it for screen readers', described == ['기록할 기기를 선택해 주세요 · 지금부터 들어오는 로그를 기록합니다'], described)
        app.capture('02-menu-no-adb')
        js(PICK, 'Android 기기 로그를 기록하려면')
        until(lambda v: v['adb'], 'adb help')
        command = js("return document.getElementById('adbCommand').textContent")
        check('the help names the install command for this computer', command == 'brew install --cask android-platform-tools', command)
        marker = js("return getComputedStyle(document.querySelector('.adb-steps li')).display")
        check('the help numbers its steps', marker == 'list-item', marker)
        app.capture('03-adb-help')
        app.click('adbClose')
        until(lambda v: not v['adb'], 'help closed')

        # Scripted devices: one ready, one waiting for authorization.
        js(QA, 'fake')
        app.click('emptyDevice')
        m = menu(lambda v: any(x.startswith('Pixel 8') for x in v), 'device menu')
        check('the menu lists each kind of device under its name and says what the others need',
              m[1:4] == ['Android', 'Pixel 8 · emulator-5554', 'Tablet X1 · QA0SERIAL01 기기 화면에서 USB 디버깅을 허용해 주세요']
              and 'iPhone·iPad' in m and 'QA iPhone · iOS 27.0' in m and any(x.startswith('QA iPad · iPadOS 27.0 기기 화면의') for x in m), m)
        trust = js("const b=[...document.querySelectorAll('#rowMenu>button')].find(b=>b.textContent.startsWith('QA iPad'));return b?b.querySelector('.menu-sub').textContent:null")
        check('an iPad that does not trust the computer says how it comes to, in view', '이 컴퓨터를 신뢰하겠습니까' in (trust or ''), trust)
        waiting_item = js("const b=[...document.querySelectorAll('#rowMenu>button')].find(b=>b.textContent.startsWith('Tablet X1'));return b?b.getAttribute('aria-disabled'):null")
        check('a device that cannot be picked is still a menu item the keyboard reaches', waiting_item == 'true', waiting_item)
        check('the menu does not offer to pick an adb when one was found on its own', 'adb 위치 지정…' not in m, m)
        app.capture('04-menu-devices')
        # Trusted while the menu is open, the iPad becomes choosable on its
        # own, and focus stays on it.
        js("[...document.querySelectorAll('#rowMenu>button')].find(b=>b.textContent.startsWith('QA iPad')).focus();return true;")
        js(QA, 'ios-ipad-trusted')
        state = app.wait(lambda: js("const b=[...document.querySelectorAll('#rowMenu>button')].find(b=>b.textContent.startsWith('QA iPad'));"
                                    "return b&&!b.hasAttribute('aria-disabled')&&{text:b.textContent,focused:document.activeElement===b}"), 10, what='iPad choosable')
        check('a device trusted while the menu is open becomes choosable, keeping focus', state['text'] == 'QA iPad · iPadOS 27.0' and state['focused'], state)
        js(PICK, 'Pixel 8')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['following'] == 'true', 'recording')
        s = until(lambda v: v['count'].endswith('건') and not v['count'].endswith(' 0건') and '저장 안 됨' in v['bar']['text'], 'rows arrive')
        check('a capture records the device rows and follows them', s['bar']['text'] == '기록 중 · 저장 안 됨 · Pixel 8' and s['bar']['stop'], s['bar'])
        check('unsaved rows make saving the main action', s['bar']['save'] and not s['open'], {'save': s['bar']['save'], 'open': s['open']})
        app.capture('05-recording', hover='captureStop')

        # A row's menu stays open while rows arrive, and the followed log
        # holds still so the row stays beside it; closing the menu follows
        # the end again. The scroll event a list sends when it moves itself
        # is sent while the menu is open, and must not close it.
        js(AT_END)
        app.wait(lambda: js("const v=document.getElementById('logsViewport');return v.scrollHeight>v.clientHeight+3*28"), what='rows past the pane')
        js(AT_END)
        line = app.wait(lambda: js(NEAR_END), what='rows shown')
        app.click_row(line, event='contextmenu')
        menu(lambda v: '행 원문 복사' in v, 'row menu while recording')
        # The press opens the raw detail, which shifts the row up; the menu
        # moves with it.
        app.wait(lambda: not app.hidden('largeDetail'), what='raw detail')

        def steady(script, *args):
            first = js(script, *args)
            time.sleep(.2)
            return first if first is not None and first == js(script, *args) else None
        top = app.wait(lambda: steady(ROW_TOP, line), what='row settled')
        count = st()['count']
        until(lambda v: v['count'] != count, 'rows arrive under the menu')
        time.sleep(.4)
        js("document.getElementById('logsViewport').dispatchEvent(new Event('scroll'));return true;")
        held = js(OPEN, line)
        check('a row menu stays open while a capture adds rows, its row held still beside it',
              held == {'open': True, 'label': '행 작업', 'focus': True, 'beside': True} and js(ROW_TOP, line) == top, {'menu': held, 'top': [top, js(ROW_TOP, line)]})
        app.capture('05b-row-menu-recording')
        # Right-clicking another row opens its menu there: the list does not
        # move under the press that closed the first menu.
        other = line - 3
        other_top = js(ROW_TOP, other)
        app.click_row(other, event='contextmenu')
        time.sleep(.3)
        moved = {**js(OPEN, other), 'top': js(ROW_TOP, other), 'selected': js(SELECTED)}
        check('right-clicking another row under an open row menu opens its menu on that row, the list still held',
              moved == {'open': True, 'label': '행 작업', 'focus': True, 'beside': True, 'top': other_top, 'selected': str(other)}, {'was': other_top, 'now': moved})
        app.key('Escape')
        last = app.wait(lambda: js(END), what='end after the menu')
        count = st()['count']
        until(lambda v: v['count'] != count, 'more rows')
        time.sleep(.4)
        check('closing the row menu follows the end again', js(END) and js(END) > last, {'last': last, 'now': js(END)})
        # Scrolling the list closes the menu and leaves the list where it
        # was scrolled to.
        line = app.wait(lambda: js(NEAR_END), what='rows shown')
        app.click_row(line, event='contextmenu')
        menu(lambda v: '행 원문 복사' in v, 'row menu again')
        js("const v=document.getElementById('logsViewport');v.scrollTop-=56;v.dispatchEvent(new Event('scroll'));return true;")
        gone = app.wait(lambda: js("return document.getElementById('rowMenu').hidden"), what='menu closed by scrolling')
        count = st()['count']
        until(lambda v: v['count'] != count, 'rows after scrolling')
        time.sleep(.4)
        check('scrolling the list closes its row menu and the list stays where it was scrolled', gone and not js(END), {'closed': gone, 'end': js(END)})
        # A click that opens the raw detail shortens the list; the log still
        # follows its end.
        app.key('Escape', target='logsViewport')
        app.wait(lambda: app.hidden('largeDetail'), what='raw detail closed')
        js(AT_END)
        app.click_row(app.wait(lambda: js(NEAR_END), what='rows shown'))
        app.wait(lambda: not app.hidden('largeDetail'), what='raw detail')
        count = st()['count']
        until(lambda v: v['count'] != count, 'rows after the detail opened')
        time.sleep(.4)
        check('a click that opens the raw detail leaves the log following its end', bool(js(END)), {'end': js(END)})
        # So does a shorter window: a hidden window still resizes.
        frame = app.rect()
        tall = js("return document.getElementById('logsViewport').clientHeight")
        app.rect(width=frame['width'], height=frame['height'] - 120)
        # A window already at its smallest would test nothing.
        short = app.wait(lambda: (lambda h: h < tall and h)(js("return document.getElementById('logsViewport').clientHeight")), what='shorter list')
        count = st()['count']
        until(lambda v: v['count'] != count, 'rows after the window got shorter')
        time.sleep(.4)
        shorter = js(END)
        app.rect(width=frame['width'], height=frame['height'])
        check('a shorter window leaves the log following its end', bool(shorter), {'end': shorter, 'height': [tall, short]})
        # A press that drifts a few pixels while rows arrive selects nothing
        # more: the list holds still until the press ends, then follows.
        line = app.wait(lambda: js(NEAR_END), what='rows shown')
        y = app.row_y(line)
        app.press_row(line)
        app.pointer('pointermove', y + 6)
        top, count = js(ROW_TOP, line), st()['count']
        until(lambda v: v['count'] != count, 'rows under the press')
        time.sleep(.6)
        pressed = {'top': js(ROW_TOP, line), 'selection': app.text('logsSelection'), 'selected': js(SELECTED)}
        app.pointer('pointerup', y + 6)
        check('a press held while rows arrive keeps its one row and the list still', pressed == {'top': top, 'selection': '', 'selected': str(line)}, {'was': top, 'now': pressed})
        count = st()['count']
        until(lambda v: v['count'] != count, 'rows after the press')
        time.sleep(.4)
        check('after the press the log follows its end again', bool(js(END)), {'end': js(END)})
        # F2 under an open row menu steps to a bookmark and closes the menu,
        # whose items act on the list's selection.
        mark = app.wait(lambda: js(NEAR_END), what='rows shown')
        app.click_row(mark)
        app.key('b', meta=True, target='logsViewport')
        app.wait(lambda: app.text('bookmarkTab') == '북마크 1', what='bookmark')
        app.wait(lambda: (js(NEAR_END) or 0) > mark + 2, what='rows past the bookmark')
        other = app.wait(lambda: js(NEAR_END), what='rows shown')
        app.click_row(other, event='contextmenu')
        menu(lambda v: '북마크 추가' in v, 'row menu before F2')
        app.key('F2')
        stepped = app.wait(lambda: js("return document.getElementById('rowMenu').hidden&&document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber"), what='menu closed by F2')
        check('F2 under an open row menu steps to the bookmark and closes the menu', stepped == str(mark), {'mark': mark, 'other': other, 'selected': stepped})
        # Focus leaves with the menu for the list, so the arrow keys go on.
        focus = js("return document.getElementById('logsViewport').contains(document.activeElement)?'list':document.activeElement?.id||document.activeElement?.tagName")
        check('focus goes back to the list when F2 closes the row menu', focus == 'list', focus)
        # Later steps start from a followed log with no detail, selection or
        # bookmark; the undo notice for the bookmark goes first.
        app.key('b', meta=True, target='logsViewport')
        app.wait(lambda: app.hidden('lowerTabs') or app.text('bookmarkTab') == '북마크 0', what='bookmark removed')
        app.wait(lambda: app.hidden('toast'), 10, what='notice gone')
        app.key('Escape', target='logsViewport')
        app.wait(lambda: app.hidden('largeDetail'), what='raw detail closed')
        app.key('Escape', target='logsViewport')
        js(AT_END)

        # The device menu opens with "looking…" and fills in; Escape still
        # returns focus to the button that opened it.
        js("const b=document.getElementById('deviceButton');b.focus();b.click();return true;")
        menu(lambda v: any(x.startswith('Pixel 8') for x in v), 'device menu while recording')
        app.key('Escape')
        focused = app.wait(lambda: js("return document.getElementById('rowMenu').hidden&&document.activeElement.id"), what='menu closed')
        check('closing the device menu returns focus to its button', focused == 'deviceButton', focused)

        # The device leaves and comes back: no row is lost or repeated, and a
        # labeled dashed line marks where.
        js(QA, 'unplug')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'waiting', 'waiting')
        check('a device that leaves is waited for', s['bar']['text'] == '연결 끊김 · 다시 연결을 기다리는 중 · Pixel 8', s['bar'])
        app.capture('06-waiting')
        # Listed again but not yet allowed: the bar leads with what to do,
        # and a narrow window ends its words in an ellipsis.
        js(QA, 'unauthorized')
        js(QA, 'plug')
        s = until(lambda v: v['bar'] and v['bar']['text'].startswith('다시 연결됨 · USB 디버깅을 허용해 주세요'), 'waiting for approval')
        check('a device plugged in again but not allowed says what it needs first', s['bar']['text'] == '다시 연결됨 · USB 디버깅을 허용해 주세요 · Pixel 8', s['bar'])
        frame = app.rect()
        app.rect(width=900, height=frame['height'])
        time.sleep(.5)
        fit = js(FITS)
        check('at 900 px the state ends in an ellipsis beside the file name, not over it',
              fit['ellipsis'] == 'ellipsis' and fit['apart'], fit)
        app.capture('06b-waiting-approval-900')
        app.rect(width=frame['width'], height=frame['height'])
        js(QA, 'authorized')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['breaks'], 'back with a break')
        label = s['breaks'][0]['label']
        check('the first row after the device came back is labeled', '연결 끊김 →' in label and '다시 연결' in label, s['breaks'])
        app.click('followFile')
        until(lambda v: v['following'] == 'false', 'paused')
        js(SHOW_BREAK)
        time.sleep(.4)
        app.capture('07-reconnected-break')

        # A filter that hides the break's first row moves the label to the
        # next row shown.
        line = s['breaks'][0]['line']
        tag = js("const r=document.querySelector(`#logsRows [data-line-number=\"${arguments[0]}\"] .tag`);return r?r.textContent:null", line)
        # The break's row is selected, so the filter keeps that row's place,
        # not one an earlier step left selected: where the list lands then
        # depends on this step alone.
        app.click_row(line)
        app.exclude_tags(tag)
        s = until(lambda v: v['breaks'] and v['breaks'][0]['line'] != line, 'break under a filter')
        check('a filter that hides the row after a break keeps the break on the next row', s['breaks'][0]['line'] > line and '연결 끊김' in s['breaks'][0]['label'],
              {'tag': tag, 'line': line, 'breaks': s['breaks']})
        app.exclude_tags('')
        until(lambda v: v['breaks'] and v['breaks'][0]['line'] == line, 'filter removed')

        # The trash button clears the rows so far at once, unsaved as they
        # are: the capture goes on into a new file, shown as a new log that
        # keeps the filters and follows again, and the old file goes.
        app.exclude_tags('Choreographer')
        until(lambda v: js("return !!document.querySelector('#hideList .cond-card')"), 'filter applied')
        button = js("""const b=document.getElementById('captureClear');return {hidden:b.hidden,label:b.getAttribute('aria-label'),title:b.title,
          hint:document.getElementById(b.getAttribute('aria-describedby')).textContent,icon:!!b.querySelector('svg'),text:b.textContent.trim(),
          last:document.getElementById('captureSave').nextElementSibling===b,border:getComputedStyle(b).borderTopColor}""")
        warn = '지금까지 기록한 로그를 비우고 계속 기록합니다. 저장하지 않은 로그는 되살릴 수 없습니다'
        check('a live capture offers a quiet trash button after saving that names itself and warns that unsaved rows cannot come back',
              button == {'hidden': False, 'label': '로그 비우기', 'icon': True, 'text': '', 'title': warn, 'hint': warn, 'last': True,
                         'border': 'rgba(0, 0, 0, 0)'}, button)
        app.capture('07b-clear-hover', hover='captureClear')
        recorded = lambda: [p for p in captures.glob('logcat-*.log') if p != left]
        before = recorded()
        last = before[0].read_text(encoding='utf-8').splitlines()[-1][:18] if len(before) == 1 else ''
        app.click('captureClear')
        # The filter may hide the first row; the next row shown takes its mark.
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['following'] == 'true' and not v['dialog']
                  and len(v['breaks']) == 1 and '로그 비움' in v['breaks'][0]['label'], 'cleared')
        toast = app.wait(lambda: js("const t=document.getElementById('toast');return !t.hidden&&t.textContent"), what='clear notice')
        check('clearing asks nothing and says how much went',
              re.fullmatch(r'기록한 로그\([0-9.,]+ (B|KiB|MiB)\)를 비웠습니다\. 지금부터 들어오는 로그를 기록합니다\.', toast or '') is not None, toast)
        # The list's first row has no row above, so its label sits inside it.
        inside = js("const r=document.querySelector('#logsRows .log-row.capture-break');return !!r&&r.classList.contains('capture-break-start')&&getComputedStyle(r,'::after').right")
        check('the first row of the cleared log is marked inside it with when it was cleared',
              re.fullmatch(r'\d\d:\d\d:\d\d 로그 비움', s['breaks'][0]['label']) is not None and inside == '10px', {'breaks': s['breaks'], 'inside': inside})
        after = app.wait(lambda: (lambda f: len(f) == 1 and f[0] not in before and f[0].stat().st_size > 0 and f)(recorded()), what='new capture file')
        first = after[0].read_text(encoding='utf-8').splitlines()[0][:18]
        check('the rows so far go with their file, and the new file holds only rows from then on',
              len(before) == 1 and not before[0].exists() and first > last, {'before': [p.name for p in before], 'after': [p.name for p in after], 'last': last, 'first': first})
        kept = js("return [...document.querySelectorAll('#hideList .cond-card')].map(c=>c.textContent)")
        check('clearing keeps the filters', len(kept) == 1 and 'Choreographer' in kept[0], kept)
        shown = app.wait(lambda: js("return [...document.querySelectorAll('#logsRows .log-row .tag')].map(t=>t.textContent)") or None, what='rows after clearing')
        check('the cleared log applies the kept filters', 'Choreographer' not in shown, shown)
        app.capture('07c-cleared')
        app.exclude_tags('')
        until(lambda v: not js("return !!document.querySelector('#hideList .cond-card')"), 'filter removed again')

        js("document.getElementById('captureStop').focus();return true;")
        box = js(STATE)['bar']['stopBox']
        app.click('captureStop')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'stopped' and v['bar']['stopLabel'] == '이어서 기록', 'stopped')
        check('stopping ends the capture and says its rows are not saved', s['bar']['text'] == '기록 중지됨 · 저장 안 됨 · Pixel 8', s['bar'])
        check('the stop button turns into recording on, keeping focus', s['bar']['stop'] and s['bar']['focus'] == 'captureStop', s['bar'])
        check('the button stays where it was pressed', s['bar']['stopBox'] == box, {'before': box, 'after': s['bar']['stopBox']})
        app.capture('07d-stopped', hover='captureStop')
        # The same button records on into the same log from now on, and the
        # first row after marks where it stopped.
        app.click('captureStop')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['bar']['stopLabel'] == '기록 중지' and v['following'] == 'true'
                  and any('이어서 기록' in b['label'] for b in v['breaks']), 'recorded on')
        mark = next(b['label'] for b in s['breaks'] if '이어서 기록' in b['label'])
        check('pressing it again records on into the log and marks where it stopped',
              re.fullmatch(r'\d\d:\d\d:\d\d 기록 중지 → \d\d:\d\d:\d\d 이어서 기록', mark) is not None
              and s['bar']['text'] == '기록 중 · 저장 안 됨 · Pixel 8' and s['bar']['focus'] == 'captureStop', {'mark': mark, 'bar': s['bar']})
        check('recording on leaves the button where it was', s['bar']['stopBox'] == box, {'before': box, 'after': s['bar']['stopBox']})
        app.capture('07e-recorded-on')
        app.click('captureStop')
        until(lambda v: v['bar'] and v['bar']['tone'] == 'stopped' and v['bar']['stopLabel'] == '이어서 기록', 'stopped again')
        # Recording on without the device waits for it, and does not say it
        # was cut off then.
        js(QA, 'unplug')
        app.click('captureStop')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'waiting', 'waiting to record on')
        check('recording on without the device waits for it', s['bar']['text'] == '기기 연결을 기다리는 중 · Pixel 8' and s['bar']['stopLabel'] == '기록 중지', s['bar'])
        app.capture('07f-resume-waiting')
        js(QA, 'plug')
        until(lambda v: v['bar'] and v['bar']['tone'] == 'live', 'recording once back')
        app.click('captureStop')
        until(lambda v: v['bar'] and v['bar']['tone'] == 'stopped' and v['bar']['stopLabel'] == '이어서 기록', 'stopped once more')

        # Quitting from the app menu asks about the unsaved rows as closing
        # does; 취소 keeps the app and the log.
        js(QUIT)
        s = until(lambda v: v['dialog'], 'save question on quit')
        ids = app.wait(lambda: js("return window.__quit"), what='quit menu')
        check('the app menu Quit asks to save a capture first', 'logstudio-quit' in ids and s['note'].startswith('Pixel 8에서 기록한 로그('), {'ids': ids, 'note': s['note']})
        app.capture('08-save-question', hover='captureSaveAs')
        app.click('captureCancel')
        s = until(lambda v: not v['dialog'], 'kept')
        check('취소 keeps the app and the capture log open', not s['empty'] and s['bar'] is not None, s['bar'])
        app.click('largeClose')
        until(lambda v: v['dialog'], 'asked on close')
        app.click('captureDiscard')
        s = until(lambda v: v['empty'], 'closed')
        time.sleep(.5)
        s = st()
        check('저장 안 함 closes the log and deletes its file', s['leftovers'] == [left.name] and len(list(captures.glob('*.log'))) == 1,
              {'leftovers': s['leftovers'], 'files': [p.name for p in captures.glob('*.log')]})

        # A left file opens as a stopped capture to save or delete.
        js("document.querySelector('#leftoverList .recent-open').click();return true;")
        s = until(lambda v: v['bar'] and not v['empty'] and v['count'] == '로그 · 전체 30건', 'left file shown')
        check('a left file opens as a capture to save, not to record on', s['bar']['text'] == '남은 기기 로그 · 저장 안 됨' and not s['bar']['stop'], s['bar'])
        app.capture('09-left-file-open')
        app.click('largeClose')
        until(lambda v: v['dialog'], 'asked about the left file')
        app.click('captureDiscard')
        s = until(lambda v: v['empty'], 'left file closed')
        time.sleep(.5)
        check('a left file not kept is deleted', st()['leftovers'] == [] and not left.exists(), [p.name for p in captures.glob('*')])

        # Another adb version takes the server: the capture stops, the bar
        # says so and the error bar says why, whole.
        app.click('emptyDevice')
        menu(lambda v: any(x.startswith('Pixel 8') for x in v), 'device menu again')
        js(PICK, 'Pixel 8')
        until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and '저장 안 됨' in v['bar']['text'], 'recording again')
        js(QA, 'conflict')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'failed', 'failed', 30)
        check('a capture that fails keeps its rows and gives the whole reason', s['bar']['text'] == '기록 중단 · 저장 안 됨 · Pixel 8'
              and (s['error'] or '').startswith('버전이 다른 adb가 함께 실행되어'), {'bar': s['bar'], 'error': s['error']})
        check('a capture that failed does not offer to record on', not s['bar']['stop'], s['bar'])
        app.capture('10-failed')
        app.click('largeClose')
        until(lambda v: v['dialog'], 'asked about the failed capture')
        app.click('captureDiscard')
        until(lambda v: v['empty'], 'failed capture closed')
        js(QA, 'real')

        # An iPhone: its rows are iOS rows, recorded only while it is there,
        # and it may need to trust the computer again when it comes back.
        app.click('emptyDevice')
        menu(lambda v: any(x.startswith('QA iPhone') for x in v), 'device menu with the iPhone')
        js(PICK, 'QA iPhone')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and '저장 안 됨' in v['bar']['text']
                  and int(''.join(c for c in v['count'] if c.isdigit()) or 0) >= 6, 'iPhone recording')
        check('an iPhone capture records its rows', s['bar']['text'] == '기록 중 · 저장 안 됨 · QA iPhone', s['bar'])
        files = sorted(p.name for p in captures.glob('ios-QA_iPhone-*.log'))
        check('an iPhone capture file is named for the device', len(files) == 1, [p.name for p in captures.glob('*')])
        cells = js("""return [...document.querySelectorAll('#logsRows .log-row')].slice(0,12).map(r=>({level:r.querySelector('.level')?.textContent,
          title:r.querySelector('.level')?.title,tag:r.querySelector('.tag')?.textContent,pid:r.querySelector('.pid')?.textContent}))""")
        app_error = next((c for c in cells if c['tag'] == 'Sync' and c['level'] == 'Error'), None)
        fault = next((c for c in cells if c['title'] == 'Fault'), None)
        check('iPhone rows take the level the app wrote, or the entry\'s own level, by Apple\'s names', app_error is not None and fault is not None
              and fault['level'] == 'Fault' and all(c['pid'] == '812' for c in cells), cells)
        chip = js("return document.querySelector('#levelFilters [data-level=F] .letter').textContent+'/'+document.querySelector('#levelFilters [data-level=U] .letter').textContent")
        check('the level filter names Apple\'s levels for an iPhone log', chip == 'Fault/Default', chip)
        shown = js("""const r=[...document.querySelectorAll('#logsRows .log-row')].find(r=>r.querySelector('.timestamp.apple-stamp'));
          const t=r.querySelector('.timestamp'),g=r.querySelector('.tag');return {stamp:t.scrollWidth<=t.clientWidth+1,text:t.textContent,tag:getComputedStyle(g).direction}""")
        check('an iPhone row shows its process whole and cuts its tag from the start', shown['stamp'] and shown['tag'] == 'rtl', shown)
        app.capture('11-ios-recording')

        # A developer build on the iPhone being recorded starts anew from the
        # bar; its rows then keep the values other processes give as <private>,
        # and a mark says where they begin.
        shown = js("const b=document.getElementById('captureApp');return !b.hidden&&[b.textContent,b.getAttribute('aria-haspopup'),document.getElementById(b.getAttribute('aria-describedby')).textContent]")
        check('an iPhone being recorded offers to start an app, as a menu, with its purpose for screen readers',
              shown == ['앱 실행…', 'menu', '개발용 서명 앱을 다시 실행해 그 앱의 로그를 <private> 없이 기록합니다'], shown)
        js(QA, 'ios-no-xcode')
        app.click('captureApp')
        m = menu(lambda v: any(x.startswith('앱을 실행하려면') for x in v), 'app menu without Xcode')
        check('without Xcode the app menu says what it needs and offers where to get it',
              m == ['앱을 실행하려면 Xcode가 필요합니다. App Store에서 Xcode를 설치해 주세요.', 'Xcode 설치 주소 복사', '---', '새로 고침'], m)
        app.capture('11c-ios-no-xcode')
        js(QA, 'ios-xcode')
        js(PICK, '새로 고침')
        m = menu(lambda v: any(x.startswith('SampleApp') for x in v), 'app menu')
        check('the app menu lists the developer builds, says a running one starts anew, and names a process that differs',
              m[:4] == ['실행할 앱을 선택해 주세요 · 실행 중이면 종료하고 다시 시작합니다', '기기 잠금을 풀어 두어야 실행됩니다', 'SampleApp com.example.sample',
                        'Sample Widgets · SampleWidgets com.example.sample.widgets'], m)
        whole = js("return [...document.querySelectorAll('#rowMenu .menu-label')].map(l=>l.scrollWidth<=l.clientWidth+1)")
        check('every app name in the menu shows whole', whole == [True, True], whole)
        # A menu taller than the window scrolls; its items keep their height.
        squeezed = js("""const m=document.getElementById('rowMenu'),items=[...m.querySelectorAll('button')],full=items.map(b=>b.getBoundingClientRect().height);
          m.style.maxHeight='90px';const kept=items.map((b,i)=>Math.abs(b.getBoundingClientRect().height-full[i])<0.5);const scrolls=m.scrollHeight>m.clientHeight;
          m.style.maxHeight='';return {kept:kept.every(Boolean),scrolls}""")
        check('a menu taller than the window scrolls instead of squeezing its items', squeezed == {'kept': True, 'scrolls': True}, squeezed)
        label = js("const m=document.getElementById('rowMenu');return [m.getAttribute('aria-label'),document.activeElement.textContent]")
        check('the app menu is named and its first app has focus', label == ['앱 실행', 'SampleApp com.example.sample'], label)
        app.capture('11d-ios-app-menu')
        js(PICK, 'SampleApp')
        toast = app.wait(lambda: (lambda t: '앱을 실행했습니다' in t and t)(app.text('toast')), 10, what='app started')
        check('starting the app says this run\'s rows are kept unredacted', toast == 'SampleApp 앱을 실행했습니다. 이번 실행의 로그는 <private> 없이 기록합니다.', toast)
        # Following may move the mark's row out of view; the results of a
        # search carry it on their first row after it.
        app.type('searchInput', 'request')
        found = app.wait(lambda: js("""const r=[...document.querySelectorAll('#largeRows .log-row')];
          return r.some(e=>e.textContent.includes('to api.example.com'))&&r.some(e=>e.textContent.includes('to <private>'))&&
          {rows:r.length,mark:(r.find(e=>e.dataset.captureBreak)||{dataset:{}}).dataset.captureBreak||''}"""), 20, what='rows with values')
        check('rows of the app started again show their values, earlier ones stay as recorded, and a mark names when the app started',
              found['rows'] >= 2 and re.fullmatch(r'\d\d:\d\d:\d\d SampleApp 실행 · 새 프로세스부터 <private> 없이 기록', found['mark']) is not None, found)
        # The toast goes, so the rows under it show.
        time.sleep(2.8)
        app.capture('11e-ios-app-started')
        app.type('searchInput', '')
        js(AT_END)
        # Opened from the focused button and picked: the button keeps focus
        # while a slow start runs, marked unavailable.
        js(QA, 'ios-slow-launch')
        js("const b=document.getElementById('captureApp');b.focus();b.click();return true;")
        # The followed log scrolls as rows arrive; the menu from the bar stays.
        menu(lambda v: any(x.startswith('SampleApp') for x in v), 'app menu from the focused button')
        time.sleep(.6)
        still = js("const m=document.getElementById('rowMenu');return !m.hidden&&m.getAttribute('aria-label')")
        check('a menu from the bar stays open while the followed log scrolls', still == '앱 실행', still)
        js(PICK, 'SampleApp')
        busy = app.wait(lambda: js("""const b=document.getElementById('captureApp');return b.getAttribute('aria-disabled')==='true'&&
          {text:b.textContent,focused:document.activeElement===b,disabled:b.disabled,status:document.getElementById('captureAppStatus').textContent}"""), what='starting')
        check('while the app starts the button keeps focus and says so', busy == {'text': '앱 실행 중…', 'focused': True, 'disabled': False,
              'status': 'SampleApp 앱을 실행하는 중입니다'}, busy)
        toast = app.wait(lambda: (lambda t: '잠금' in t and t)(app.text('toast')), what='slow start hint')
        check('a start that takes long asks whether the device is unlocked', toast == 'SampleApp 앱을 실행하는 중입니다. 기기 잠금이 풀려 있는지 확인해 주세요.', toast)
        app.capture('11g-ios-app-starting', hover='captureApp')
        # A notice that comes on its own waits for a toast offering an undo:
        # the start ends while a filter from the row menu can still be undone.
        hides = app.text('hideList')
        app.click_row(app.wait(lambda: js(NEAR_END), what='rows shown'), event='contextmenu')
        menu(lambda v: any(x.endswith(' 제외') for x in v), 'row menu while the app starts')
        js("[...document.querySelectorAll('#rowMenu>button')].find(b=>b.textContent.endsWith(' 제외')).click();return true;")
        undo = app.wait(lambda: (lambda t: '제외 조건' in t and t)(app.text('toast')), what='undo notice')
        # The start must still be under way, or nothing waits.
        check('the app is still starting when the undo is offered',
              js("return document.getElementById('captureApp').getAttribute('aria-disabled')") == 'true')
        app.wait(lambda: js("return document.getElementById('captureApp').getAttribute('aria-disabled')==='false'"), 15, what='slow start done')
        kept = app.text('toast')
        js("document.querySelector('#toast .toast-action').click();return true;")
        after = app.wait(lambda: (lambda t: '앱을 실행했습니다' in t and t)(app.text('toast')), what='start notice after the undo')
        check('a start that ends while an undo is offered says so once the undo notice goes',
              kept == undo and after == 'SampleApp 앱을 실행했습니다. 이번 실행의 로그는 <private> 없이 기록합니다.', {'undo': undo, 'kept': kept, 'after': after})
        app.wait(lambda: app.text('hideList') == hides, what='filter undone')
        frame = app.rect()
        app.rect(width=900, height=frame['height'])
        time.sleep(.5)
        fit = js(FITS)
        unsaved = js("""const t=document.getElementById('captureText'),n=t.firstChild,s=t.textContent.indexOf('저장 안 됨');if(!n||s<0)return null;
          const r=document.createRange();r.setStart(n,s);r.setEnd(n,s+'저장 안 됨'.length);return r.getBoundingClientRect().right<=t.getBoundingClientRect().right+0.5""")
        check('at 900 px the iPhone bar ends in an ellipsis beside the file name and still shows the unsaved warning',
              fit['ellipsis'] == 'ellipsis' and fit['apart'] and unsaved is True, {'fit': fit, 'unsaved': unsaved})
        app.capture('11f-ios-recording-900')
        # In the narrowest window the wordmark gives way, so the state still
        # leads the bar and the trash button keeps its shape.
        app.rect(width=760, height=frame['height'])
        time.sleep(.5)
        narrow = js("""const w=id=>Math.round(document.getElementById(id).getBoundingClientRect().width),bar=document.querySelector('.topbar');
          return {state:w('captureState'),clear:w('captureClear'),wordmark:Math.round(document.querySelector('.brand h1').getBoundingClientRect().width),
          overflow:bar.scrollWidth>bar.clientWidth}""")
        check('at 760 px the iPhone bar keeps its state readable and the trash button square',
              narrow['state'] >= 60 and narrow['clear'] == 30 and narrow['wordmark'] <= 1 and not narrow['overflow'], narrow)
        app.capture('11h-ios-recording-760')
        app.rect(width=frame['width'], height=frame['height'])
        js(QA, 'ios-unplug')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'waiting', 'iPhone waiting')
        hidden = js("return document.getElementById('captureApp').hidden")
        check('an iPhone away offers no app to start', hidden is True, hidden)
        hint = js("return document.getElementById('captureHint').textContent")
        check('an iPhone that leaves is waited for, and the bar and screen readers say what it logs meanwhile is lost',
              s['bar']['text'] == '연결 끊김 · 끊긴 동안 로그 기록 안 됨 · QA iPhone' and hint.endswith('끊긴 동안의 로그는 기록되지 않습니다.'), {'bar': s['bar'], 'hint': hint})
        app.capture('11b-ios-away')
        js(QA, 'ios-untrusted')
        js(QA, 'ios-plug')
        s = until(lambda v: v['bar'] and '신뢰' in v['bar']['text'], 'iPhone waiting for trust')
        check('an iPhone back but not trusting the computer says so first', s['bar']['text'] == '다시 연결됨 · 이 컴퓨터를 신뢰해 주세요 · QA iPhone', s['bar'])
        app.capture('12-ios-trust')
        js(QA, 'ios-trusted')
        # The app's launch marks come before it; the reconnect is its own.
        away = lambda v: [b for b in v['breaks'] if '연결 끊김 →' in b['label']]
        s = until(lambda v: js(AT_END) and v['bar'] and v['bar']['tone'] == 'live' and away(v), 'iPhone back')
        check('an iPhone back is recorded again after a break that says its rows meanwhile are missing',
              away(s)[0]['label'].endswith(' · 끊긴 동안 로그 기록 안 됨'), s['breaks'])
        app.click('followFile')
        until(lambda v: v['following'] == 'false', 'iPhone paused')
        js(SHOW_BREAK)
        time.sleep(.4)
        app.capture('12b-ios-break')
        app.click('largeClose')
        until(lambda v: v['dialog'], 'asked about the iPhone capture')
        app.click('captureDiscard')
        until(lambda v: v['empty'], 'iPhone capture closed')
        time.sleep(.5)
        check('an iPhone capture not kept is deleted', not list(captures.glob('ios-*.log')), [p.name for p in captures.glob('*')])

        # Without Apple's device service (Windows without Apple Devices or
        # iTunes), one item leads to a dialog on getting it.
        js(QA, 'ios-missing')
        app.click('emptyDevice')
        m = menu(lambda v: any(x.startswith('iPhone·iPad 로그를 기록하려면') for x in v), 'menu without the device service')
        check('without Apple\'s device service one item leads to getting it', 'iPhone·iPad 로그를 기록하려면 Apple Devices 앱을 설치해 주세요…' in m, m)
        js(PICK, 'iPhone·iPad 로그를 기록하려면')
        url = app.wait(lambda: js("const d=document.getElementById('appleDialog');return d.open&&document.getElementById('appleUrl').textContent"), what='Apple help')
        check('the Apple help names the Store page', url == 'https://apps.microsoft.com/detail/9np83lwlpz9k', url)
        app.capture('13-apple-help')
        app.click('appleClose')
        js(QA, 'ios-real')
