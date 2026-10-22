"""Page zoom: Cmd+=/-/0, Ctrl+wheel and pinch gestures, the indicator,
clamping at 50% and 200%, layout while zoomed, and the level saved across
launches (ui.log)."""
import json
import time

import fixtures

STATE = """const v=document.getElementById('logsViewport'),rows=document.querySelectorAll('#logsRows .log-row');return {
  width:innerWidth,overflow:document.documentElement.scrollWidth-innerWidth,
  indicator:document.getElementById('zoomIndicator').hidden?null:document.getElementById('zoomIndicator').textContent,
  saved:localStorage.getItem('logstudio.zoom.v1'),viewportHeight:Math.round(v.getBoundingClientRect().height),
  rows:rows.length,rowHeight:rows[0]?Math.round(rows[0].getBoundingClientRect().height):null,scrollTop:v.scrollTop,
  selected:document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null};"""
WHEEL = "const v=document.getElementById('logsViewport');const e=new WheelEvent('wheel',{deltaY:arguments[0],ctrlKey:true,bubbles:true,cancelable:true});v.dispatchEvent(e);return e.defaultPrevented;"
# A pinch as WebKit reports it, optionally followed at once by the Ctrl+wheel
# a webview might also derive from it.
PINCH = """const fire=(type,scale)=>{let e;try{e=new GestureEvent(type,{scale,bubbles:true,cancelable:true});}catch{e=new Event(type,{bubbles:true,cancelable:true});Object.defineProperty(e,'scale',{value:scale});}
document.body.dispatchEvent(e);return e.defaultPrevented;};
const r=[fire('gesturestart',1),fire('gesturechange',arguments[0]/2+.5),fire('gesturechange',arguments[0]),fire('gestureend',arguments[0])];
if(arguments[1]){const w=new WheelEvent('wheel',{deltaY:-100,ctrlKey:true,bubbles:true,cancelable:true});document.body.dispatchEvent(w);r.push(w.defaultPrevented);}return r;"""


def run(ctx):
    fixtures.ui(ctx.fixtures)
    check = ctx.check
    with ctx.launch(clear_after=False) as app:
        st = lambda: app.js(STATE)
        zoom = lambda k, code: app.key(k, meta=True, code=code)
        app.open_fixture('ui.log')
        app.wait_count(3000)
        time.sleep(.3)
        s0 = st()
        check('starts at 100% with nothing saved', s0['saved'] is None and s0['indicator'] is None, s0)
        found = app.js("return {GestureEvent:typeof GestureEvent,constructible:(()=>{try{return new GestureEvent('gesturechange',{scale:2}).scale===2}catch(e){return String(e)}})()}")
        check("WKWebView has WebKit's GestureEvent (pinches arrive as gesture events)", found['GestureEvent'] == 'function', found)
        app.click_row(3)
        zoom('=', 'Equal')
        s1 = app.wait(lambda: (lambda v: v['width'] < s0['width'] - 10 and v)(st()), what='110%')
        check('Cmd+= zooms to 110% and shows and saves it', abs(s0['width'] / s1['width'] - 1.1) < .02 and s1['indicator'] == '화면 110%' and json.loads(s1['saved'])['level'] == 1.1, {'before': s0['width'], 'after': s1['width']})
        check('the CSS row height is unchanged by zoom', s1['rowHeight'] == s0['rowHeight'], {'before': s0['rowHeight'], 'after': s1['rowHeight']})
        check('the selection survives zoom', s1['selected'] == '3', s1['selected'])
        top = st()['scrollTop']
        prevented = app.js(WHEEL, -100)
        s2 = app.wait(lambda: (lambda v: v['width'] < s1['width'] - 10 and v)(st()), what='125%')
        check('Ctrl+wheel up zooms in to 125% without scrolling the log', prevented and abs(s0['width'] / s2['width'] - 1.25) < .02 and s2['scrollTop'] == top, s2)
        for _ in range(6):
            zoom('=', 'Equal')
        app.wait(lambda: abs(s0['width'] / st()['width'] - 2) < .02, what='200%')
        time.sleep(.4)
        s3 = st()
        check('zoom clamps at 200% with no horizontal overflow and rows filling the viewport',
              s3['indicator'] == '화면 200%' and s3['overflow'] <= 0 and s3['rows'] * s3['rowHeight'] >= s3['viewportHeight'], s3)
        app.capture('01-zoom-200')
        app.js("document.getElementById('logsViewport').focus();return true;")
        app.key('ArrowDown')
        time.sleep(.3)
        check('row navigation still works while zoomed', st()['selected'] == '4', st()['selected'])
        for _ in range(12):
            zoom('-', 'Minus')
        app.wait(lambda: abs(s0['width'] / st()['width'] - .5) < .02, what='50%')
        time.sleep(.4)
        s4 = st()
        check('zoom clamps at 50% and the virtual list renders the extra rows',
              s4['indicator'] == '화면 50%' and s4['rows'] * s4['rowHeight'] >= s4['viewportHeight'] and s4['rows'] > s0['rows'], s4)
        app.capture('02-zoom-50')
        zoom('0', 'Digit0')
        app.wait(lambda: st()['width'] == s0['width'], what='100%')

        prevented = app.js(PINCH, 1.3, True)
        w1 = app.wait(lambda: (lambda w: w < s0['width'] - 10 and w)(st()['width']), what='pinch zoom')
        z = st()
        check('a pinch out ×1.3 zooms to 125%; every gesture event and the wheel right after are prevented',
              all(prevented) and abs(s0['width'] / w1 - 1.25) < .02 and z['indicator'] == '화면 125%', {'prevented': prevented, 'indicator': z['indicator']})
        app.capture('03-pinch-125')
        time.sleep(.6)
        check('the pinch level is saved; the Ctrl+wheel right after added no step',
              json.loads(st()['saved'])['level'] == 1.25 and abs(s0['width'] / st()['width'] - 1.25) < .02, st()['saved'])
        app.js(PINCH, 0.5, False)
        app.wait(lambda: abs(s0['width'] / st()['width'] - 0.67) < .02, what='67%')
        check('a pinch in ×0.5 from 125% lands on 67%', True)
        zoom('0', 'Digit0')
        app.wait(lambda: st()['width'] == s0['width'], what='100%')
        zoom('=', 'Equal')
        app.wait(lambda: abs(s0['width'] / st()['width'] - 1.1) < .02, what='110%')
        time.sleep(1.5)
        check('the indicator hides after a moment', st()['indicator'] is None, st()['indicator'])

    with ctx.launch(clear_before=False) as app:
        st = lambda: app.js(STATE)
        s6 = app.wait(lambda: (lambda v: abs(s0['width'] / v['width'] - 1.1) < .02 and v)(st()), 10, what='restored 110%')
        check('the saved 110% is restored on the next launch', s6['indicator'] is None, s6)
        app.capture('04-restored-empty-state')
        hint = app.js("return [...document.querySelectorAll('.empty-hints li')].at(-1).textContent")
        check('the empty state lists the zoom shortcuts', '화면 확대·축소' in hint, hint)
        app.key('0', meta=True, code='Digit0')
        s7 = app.wait(lambda: (lambda v: v['width'] == s0['width'] and v)(st()), what='100%')
        check('Cmd+0 returns to 100% and saves it', json.loads(s7['saved'])['level'] == 1 and s7['indicator'] == '화면 100%', s7)
