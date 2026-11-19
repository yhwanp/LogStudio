"""An iOS simulator's capture with a scripted simulator (QA builds; no
Xcode tool runs): the device menu with a simulator alone, with none running,
with its list failing, and after iPhones beside Android devices; recording,
the bar naming it a simulator without the app menu, and a simulator shut
down and started again."""
from scenarios.devices import MENU, PICK, QA, STATE

# Shows the first row after the simulator came back near the middle of the
# logs pane.
SHOW_BACK = """const v=document.getElementById('logsViewport');const r=[...document.querySelectorAll('#logsRows .capture-break')].find(r=>r.dataset.captureBreak.includes('시뮬레이터 꺼짐'));
if(!r)return false;v.scrollTop+=r.getBoundingClientRect().top-v.getBoundingClientRect().top-v.clientHeight/2;v.dispatchEvent(new Event('scroll'));return true;"""
HEADINGS = ('Android', 'iPhone·iPad')


def run(ctx):
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        st = lambda: js(STATE)
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(st()), timeout, what=what)
        menu = lambda pred, what: app.wait(lambda: (lambda v: v and pred(v) and v)(js(MENU)), what=what)

        def close():
            app.key('Escape')
            app.wait(lambda: js("return document.getElementById('rowMenu').hidden"), what='menu closed')

        # A simulator alone needs no heading; it says it is one.
        js(QA, 'sim')
        app.click('emptyDevice')
        m = menu(lambda v: any(x.startswith('iPhone 17') for x in v), 'device menu with a simulator alone')
        check('a simulator alone has no heading and says it is one',
              'iPhone 17 · iOS 27.0 · 시뮬레이터' in m and not any(x in HEADINGS for x in m), m)
        app.capture('01-menu-simulator-alone')
        close()
        # None running: the menu does not mention them.
        js(QA, 'sim-shutdown')
        app.click('emptyDevice')
        m = menu(lambda v: '연결된 기기가 없습니다' in v, 'device menu with no simulator running')
        check('with none running, the menu does not mention simulators', not any('시뮬레이터' in x for x in m), m)
        close()
        # A list that fails says so and how to retry.
        js(QA, 'sim-error')
        app.click('emptyDevice')
        m = menu(lambda v: '연결된 기기가 없습니다' in v, 'device menu with the simulator list failing')
        failed = 'iOS 시뮬레이터 목록을 가져오지 못했습니다. Xcode를 연 뒤 새로 고침해 주세요.'
        check('a failed simulator list says so', failed in m, m)
        app.capture('02-menu-simulator-error')
        close()
        js(QA, 'sim')
        js(QA, 'fake')
        js(QA, 'ios')
        app.click('emptyDevice')
        m = menu(lambda v: any(x.startswith('iPhone 17') for x in v), 'device menu with a simulator')
        groups = [x for x in m if x in HEADINGS]
        check('simulators follow iPhones under their heading', groups == list(HEADINGS) and m.index('iPhone 17 · iOS 27.0 · 시뮬레이터') > m.index('iPhone·iPad'), m)
        title = js("const b=[...document.querySelectorAll('#rowMenu>button')].find(b=>b.textContent.startsWith('iPhone 17'));return b?.title??null")
        check('its tooltip names it a simulator', title == 'iPhone 17 시뮬레이터의 로그를 지금부터 기록합니다', title)
        app.capture('03-menu-simulator')
        js(PICK, 'iPhone 17')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and '저장 안 됨' in v['bar']['text'], 'recording a simulator')
        check('the bar names the simulator', s['bar']['text'] == '기록 중 · 저장 안 됨 · iPhone 17 시뮬레이터', s['bar'])
        hidden = js("return document.getElementById('captureApp').hidden")
        check('apps are not started on a simulator', hidden is True, hidden)
        app.capture('04-recording-simulator')
        # The bar says why at once, with no generic "connection lost" first.
        js("window.__seen=[];new MutationObserver(()=>window.__seen.push(document.getElementById('captureText').textContent))"
           ".observe(document.getElementById('captureText'),{childList:true,characterData:true,subtree:true});return true;")
        js(QA, 'sim-shutdown')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'waiting', 'simulator shut down')
        check('a shut down simulator is waited for, and the bar says so',
              s['bar']['text'] == '시뮬레이터 꺼짐 · iPhone 17', s['bar'])
        seen = js("return window.__seen")
        check('the bar never says the connection was lost', not any('연결 끊김' in t for t in seen), seen)
        hint = js("return document.getElementById('captureState').title")
        check('its tooltip says it records on once started again', hint.endswith('에 시뮬레이터가 꺼졌습니다. 다시 켜면 이어서 기록합니다.'), hint)
        # WebKit screenshots show no tooltip; its words are checked above.
        app.capture('05-simulator-shut-down')
        js(QA, 'sim-boot')
        s = until(lambda v: v['bar'] and v['bar']['tone'] == 'live' and v['breaks'], 'recording again')
        label = s['breaks'][0]['label']
        check('started again, it records on and marks the time it was off',
              '시뮬레이터 꺼짐 →' in label and label.endswith('다시 켜짐'), s['breaks'])
        app.click('followFile')
        js(SHOW_BACK)
        app.capture('06-simulator-back')
