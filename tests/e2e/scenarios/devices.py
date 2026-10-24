"""Device captures with scripted devices (QA builds; no adb, device
service or device runs): a capture file a crashed run left, getting adb,
the device menu with Android devices and iPhones, recording, a device that
leaves and comes back, stopping, the save question before a capture's log
closes or the app quits, opening a left file, a capture that fails, and an
iPhone that leaves and must trust the computer again."""
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
    save:document.getElementById('captureSave').classList.contains('primary')},
  open:document.getElementById('openFile').classList.contains('primary'),
  count:document.getElementById('logsCount').textContent,following:document.getElementById('followFile').getAttribute('aria-pressed'),
  breaks:rows.filter(r=>r.classList.contains('capture-break')).map(r=>({line:Number(r.dataset.lineNumber),label:r.dataset.captureBreak??''})),
  dialog:document.getElementById('captureDialog').open,note:document.getElementById('captureDialogNote').textContent,
  adb:document.getElementById('adbDialog').open,empty:!document.getElementById('emptyState').hidden,
  leftovers:document.getElementById('captureLeftovers').hidden?[]:[...document.querySelectorAll('#leftoverList strong')].map(e=>e.textContent),
  error:document.getElementById('largeError').hidden?null:document.getElementById('largeError').textContent};"""
# Shows the first row after a break near the middle of the logs pane.
SHOW_BREAK = """const v=document.getElementById('logsViewport');const r=document.querySelector('#logsRows .capture-break');
if(!r)return false;v.scrollTop+=r.getBoundingClientRect().top-v.getBoundingClientRect().top-v.clientHeight/2;v.dispatchEvent(new Event('scroll'));return true;"""
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
        s = until(lambda v: v['count'].endswith('건') and not v['count'].endswith(' 0건') and v['bar']['text'].endswith('저장 안 됨'), 'rows arrive')
        check('a capture records the device rows and follows them', s['bar']['text'] == '기록 중 · Pixel 8 · 저장 안 됨' and s['bar']['stop'], s['bar'])
        check('unsaved rows make saving the main action', s['bar']['save'] and not s['open'], {'save': s['bar']['save'], 'open': s['open']})
        app.capture('05-recording', hover='captureStop')

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
        app.exclude_tags(tag)
        s = until(lambda v: v['breaks'] and v['breaks'][0]['line'] != line, 'break under a filter')
        check('a filter that hides the row after a break keeps the break on the next row', s['breaks'][0]['line'] > line and '연결 끊김' in s['breaks'][0]['label'],
              {'tag': tag, 'line': line, 'breaks': s['breaks']})
        app.exclude_tags('')
        until(lambda v: v['breaks'] and v['breaks'][0]['line'] == line, 'filter removed')

        app.click('captureStop')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'stopped' and not v['bar']['stop'], 'stopped')
        check('stopping ends the capture and says its rows are not saved', s['bar']['text'] == '기록 중지됨 · Pixel 8 · 저장 안 됨', s['bar'])

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
        check('a left file opens as a capture to save', s['bar']['text'] == '남은 기기 로그 · 저장 안 됨', s['bar'])
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
        until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['bar']['text'].endswith('저장 안 됨'), 'recording again')
        js(QA, 'conflict')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'failed', 'failed', 30)
        check('a capture that fails keeps its rows and gives the whole reason', s['bar']['text'] == '기록 중단 · Pixel 8 · 저장 안 됨'
              and (s['error'] or '').startswith('버전이 다른 adb가 함께 실행되어'), {'bar': s['bar'], 'error': s['error']})
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
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['bar']['text'].endswith('저장 안 됨')
                  and int(''.join(c for c in v['count'] if c.isdigit()) or 0) >= 6, 'iPhone recording')
        check('an iPhone capture records its rows', s['bar']['text'] == '기록 중 · QA iPhone · 저장 안 됨', s['bar'])
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
        js(QA, 'ios-unplug')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'waiting', 'iPhone waiting')
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
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['breaks'], 'iPhone back')
        check('an iPhone back is recorded again after a break that says its rows meanwhile are missing',
              '연결 끊김 →' in s['breaks'][0]['label'] and s['breaks'][0]['label'].endswith(' · 끊긴 동안 로그 기록 안 됨'), s['breaks'])
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
