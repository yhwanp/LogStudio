"""Drives a QA build of the desktop app over its WebDriver plugin.

Each launch gets a free port, the scenario's temporary HOME (app data such as
recent files, work states and window geometry) and fixture folder, so the
real app data is never touched. A bare QA binary keeps its WebView storage in
~/Library/WebKit/<executable name>, apart from the installed app's. A launch
clears that storage when it stops, and at start only if an earlier run left
something (that reloads the page, which would drop a startup error), unless a
scenario keeps it across launches on purpose. First-use tips are turned off
unless a launch asks for them, so they never cover another scenario's checks.
"""
import base64
import json
import os
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path


class Timeout(Exception):
    pass


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def port_open(port):
    try:
        with socket.create_connection(('127.0.0.1', port), timeout=.2):
            return True
    except OSError:
        return False


# Turns off transitions, forces the dark (true) or light (false) scheme by
# switching the stylesheet's prefers-color-scheme:dark media rules to `all` or
# `not all`, or restores them (null): the app follows the system scheme, which
# a run does not change. Copies `:hover` rules to a `.qa-hover` class. CSSOM
# only: the app's CSP rejects injected style elements.
THEME = """if(!window.__qa){const sheet=[...document.styleSheets].find(s=>{try{return s.cssRules.length>0}catch{return false}});
const media=[],hover=[];const walk=(rules,host)=>{for(const r of [...rules]){if(r instanceof CSSMediaRule){if(/prefers-color-scheme:\\s*dark/.test(r.conditionText))media.push(r);walk(r.cssRules,r);}
else if(r instanceof CSSStyleRule&&r.selectorText.includes(':hover'))hover.push([host,r]);}};walk(sheet.cssRules,sheet);
for(const [host,r] of hover)host.insertRule(r.cssText.replaceAll(':hover','.qa-hover'),host.cssRules.length);
sheet.insertRule('*,*::before,*::after{transition:none!important;animation:none!important}',sheet.cssRules.length);
window.__qa={media,original:media.map(r=>r.media.mediaText)};}
const dark=arguments[0];window.__qa.media.forEach((r,i)=>{r.media.mediaText=dark===null?window.__qa.original[i]:dark?'all':'not all';});
document.documentElement.style.colorScheme=dark===null?'':dark?'dark':'light';return window.__qa.media.length;"""

# Closes a tip that may already show, then turns tips off for this storage.
NO_TIPS = """const l=document.getElementById('tipLayer');if(l&&l.open)document.getElementById('tipClose').click();
const k='logstudio.tips.v1',s=JSON.parse(localStorage.getItem(k)||'{}');localStorage.setItem(k,JSON.stringify({seen:s.seen||[],off:true}));return true;"""

ROW = "document.querySelector(`#${arguments[0]} [data-line-number=\"${arguments[1]}\"]`)"

# A click as the mouse sends it on element `r`: pointer and mouse press and
# release at its centre, then `kind` (click, or a right-button contextmenu).
CLICK = """const b=r.getBoundingClientRect(),right=kind==='contextmenu',button=right?2:0;
const o={bubbles:true,cancelable:true,detail:1,clientX:b.left+Math.min(200,b.width/2),clientY:b.top+b.height/2,button,...keys};
const p=buttons=>({...o,buttons,pointerId:1,pointerType:'mouse',isPrimary:true});
r.dispatchEvent(new PointerEvent('pointerdown',p(right?2:1)));r.dispatchEvent(new MouseEvent('mousedown',{...o,buttons:right?2:1}));
r.dispatchEvent(new PointerEvent('pointerup',p(0)));r.dispatchEvent(new MouseEvent('mouseup',o));r.dispatchEvent(new MouseEvent(kind,o));"""


class App:
    def __init__(self, ctx, args=(), clear_before=True, clear_after=True, tips=False):
        self.ctx = ctx
        self.clear_after = clear_after
        self.port = free_port()
        env = os.environ.copy()
        env.update(HOME=str(ctx.home), TAURI_WEBDRIVER_PORT=str(self.port), LOGSTUDIO_QA_FIXTURES=str(ctx.fixtures))
        if getattr(ctx, 'background', True):
            env['LOGSTUDIO_QA_BACKGROUND'] = '1'
        self.proc = subprocess.Popen([str(ctx.binary), *map(str, args)], env=env,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.sid = None
        try:
            self._connect(clear_before)
            if not tips:
                self.js(NO_TIPS)
        except BaseException:
            self.stop()
            raise

    # WebDriver -----------------------------------------------------------
    def call(self, path, body=None, method=None, timeout=60):
        req = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}',
                                     data=None if body is None else json.dumps(body).encode(),
                                     headers={'Content-Type': 'application/json'}, method=method)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                v = json.load(r)
        except urllib.error.HTTPError as e:
            raise RuntimeError(e.read().decode()[:300]) from None
        return v.get('value', v)

    def _connect(self, clear_before):
        until = time.monotonic() + 20
        while not port_open(self.port):
            if self.proc.poll() is not None:
                raise RuntimeError(f'app exited with {self.proc.returncode} before WebDriver started')
            if time.monotonic() > until:
                raise Timeout('WebDriver port did not open')
            time.sleep(.05)
        self.sid = self.call('/session', {'capabilities': {'alwaysMatch': {}}})['sessionId']
        # The webview can accept the session before its page answers scripts.
        time.sleep(1.5)
        self.wait(lambda: self.js("return document.readyState==='complete'&&!!document.getElementById('largeStatus')"),
                  what='page ready')
        if clear_before and self.js("return localStorage.length") > 0:
            self.js("localStorage.clear();location.reload();return true;")
            time.sleep(1)
            self.wait(lambda: self.js("return document.readyState==='complete'&&!!document.getElementById('largeStatus')"),
                      what='page ready after clearing storage')

    def js(self, script, *args):
        return self.call(f'/session/{self.sid}/execute/sync', {'script': script, 'args': list(args)})

    def wait(self, fn, timeout=20, what=None):
        """Polls fn until it returns something truthy; errors count as not yet."""
        until = time.monotonic() + timeout
        last = None
        while True:
            try:
                v = fn()
            except Exception as e:  # the page may be reloading
                v, last = None, e
            if v:
                return v
            if time.monotonic() > until:
                raise Timeout(f'timed out waiting for {what or "condition"}' + (f' ({last})' if last else ''))
            time.sleep(.03)

    def stop(self):
        if self.sid and self.clear_after:
            try:
                self.js("localStorage.clear();return true;")
            except Exception:
                pass
        if self.sid:
            try:
                self.call(f'/session/{self.sid}', method='DELETE', timeout=5)
            except Exception:
                pass
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        until = time.monotonic() + 10
        while port_open(self.port) and time.monotonic() < until:
            time.sleep(.05)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.stop()

    # Window --------------------------------------------------------------
    def rect(self, **frame):
        if frame:
            return self.call(f'/session/{self.sid}/window/rect', frame)
        return self.call(f'/session/{self.sid}/window/rect')

    def maximize(self):
        return self.call(f'/session/{self.sid}/window/maximize', {})

    def close_window(self):
        try:
            self.call(f'/session/{self.sid}/window', method='DELETE', timeout=10)
        except Exception:
            pass
        self.proc.wait(10)
        self.sid = None

    # Page helpers --------------------------------------------------------
    def text(self, id):
        return self.js("return document.getElementById(arguments[0]).textContent", id)

    def hidden(self, id):
        return self.js("return document.getElementById(arguments[0]).hidden", id)

    def click(self, id):
        self.js("document.getElementById(arguments[0]).click();return true;", id)

    def enter(self, id, value):
        """Types a value into a field and presses Enter."""
        self.js("const i=document.getElementById(arguments[0]);i.focus();i.value=arguments[1];"
                "i.dispatchEvent(new Event('input',{bubbles:true}));"
                "i.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));return true;", id, value)

    def type(self, id, value):
        """Types a value into a field without pressing Enter (search runs as you type)."""
        self.js("const i=document.getElementById(arguments[0]);i.focus();i.value=arguments[1];"
                "i.dispatchEvent(new Event('input',{bubbles:true}));return true;", id, value)

    def key(self, key, meta=False, shift=False, alt=False, code=None, target=None):
        """Dispatches a keydown on the focused element (or the element with id `target`)."""
        self.js("const t=arguments[5]?document.getElementById(arguments[5]):(document.activeElement||document.body);"
                "t.dispatchEvent(new KeyboardEvent('keydown',{key:arguments[0],code:arguments[4]||'',metaKey:arguments[1],shiftKey:arguments[2],altKey:arguments[3],bubbles:true,cancelable:true}));return true;",
                key, meta, shift, alt, code, target)

    def exclude_tags(self, value, enter=True):
        """Types tags into the first hiding condition (adding one if there is
        none) and presses Enter; an empty value removes that condition, and
        the first showing condition is edited again."""
        self.js("""const list=document.getElementById('hideList');let c=list.querySelector('.cond-card');
if(!arguments[0]){if(c)c.querySelector('.cond-remove').click();return true;}
if(!c){document.getElementById('addHide').click();c=list.querySelector('.cond-card');}else c.querySelector('.cond-summary').click();
const i=document.getElementById('tagFilter');i.focus();i.value=arguments[0];i.dispatchEvent(new Event('input',{bubbles:true}));
if(arguments[1])i.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));return true;""", value, enter)

    def count(self):
        return self.text('logsCount')

    def wait_count(self, n, timeout=30):
        want = f'로그 · 전체 {n:,}건'
        return self.wait(lambda: self.count() == want, timeout, what=want)

    def open_fixture(self, name):
        self.js("window.__TAURI__.core.invoke('qa_open_fixture',{name:arguments[0]});return true;", name)

    def open_fixtures(self, names, arrange=None):
        self.js("window.__TAURI__.core.invoke('qa_open_fixtures',{names:arguments[0],arrange:arguments[1]});return true;",
                names, arrange)

    def row_exists(self, n, pane='logsRows'):
        return self.js(f"return !!{ROW}", pane, str(n))

    def click_row(self, n, pane='logsRows', shift=False, event='click', meta=False, ctrl=False):
        """Presses and releases on a row as a real click does (pointer and
        mouse events at the row's centre), then sends `event`: a click, or a
        right-button contextmenu."""
        self.wait(lambda: self.row_exists(n, pane), what=f'row {n} in {pane}')
        self.js("const r=" + ROW + ",kind=arguments[3],keys={shiftKey:arguments[2],metaKey:arguments[4],ctrlKey:arguments[5]};" + CLICK + "return true;",
                pane, str(n), shift, event, meta, ctrl)
        time.sleep(.15)

    def press_row(self, n, pane='logsRows', **keys):
        """A primary-button press on a row's centre that stays down; returns
        whether the page took it (cancelled the mousedown)."""
        self.wait(lambda: self.row_exists(n, pane), what=f'row {n} in {pane}')
        return self.js("const r=" + ROW + ",b=r.getBoundingClientRect(),o={bubbles:true,cancelable:true,detail:1,clientX:b.left+Math.min(200,b.width/2),clientY:b.top+b.height/2,button:0,buttons:1,...arguments[2]};"
                       "r.dispatchEvent(new PointerEvent('pointerdown',{...o,pointerId:1,pointerType:'mouse',isPrimary:true}));const m=new MouseEvent('mousedown',o);r.dispatchEvent(m);return m.defaultPrevented;",
                       pane, str(n), keys)

    def pointer(self, kind, y, x=200):
        """A pointermove (button held) or pointerup at a client position."""
        self.js("document.dispatchEvent(new PointerEvent(arguments[0],{bubbles:true,cancelable:true,clientX:arguments[2],clientY:arguments[1],pointerId:1,pointerType:'mouse',isPrimary:true,button:0,buttons:arguments[0]==='pointerup'?0:1}));return true;",
                kind, y, x)

    def row_y(self, n, pane='logsRows'):
        return self.js("const b=" + ROW + ".getBoundingClientRect();return b.top+b.height/2;", pane, str(n))

    def drag_rows(self, start, end, pane='logsRows', steps=3, **keys):
        """Presses on row `start`, moves to row `end` (both in view) in steps a
        few frames apart and releases there."""
        y0, y1 = self.row_y(start, pane), self.row_y(end, pane)
        taken = self.press_row(start, pane, **keys)
        for i in range(1, steps + 1):
            self.pointer('pointermove', y0 + (y1 - y0) * i / steps)
            time.sleep(.06)
        self.pointer('pointerup', y1)
        time.sleep(.15)
        return taken

    def jump(self, n):
        self.js("const i=document.getElementById('logsJump');i.value=String(arguments[0]);"
                "document.getElementById('logsJumpForm').requestSubmit();return true;", n)

    def capture_clipboard(self):
        """Replaces the page's clipboard methods so copies land in window.__captured."""
        self.js("window.__captured=null;navigator.clipboard.write=async items=>{window.__captured=await (await items[0].getType('text/plain')).text();};"
                "navigator.clipboard.writeText=async text=>{window.__captured=text;};return true;")

    def captured(self):
        return self.wait(lambda: self.js("return window.__captured"), what='clipboard write')

    # Screenshots ---------------------------------------------------------
    def shot(self, name):
        path = self.ctx.shots / f'{name}.png'
        path.write_bytes(base64.b64decode(self.call(f'/session/{self.sid}/screenshot')))
        return path

    def capture(self, name, hover=None, schemes=('light', 'dark')):
        """Screenshots the current state in each scheme, optionally with the
        element `hover` drawn in its hover state."""
        for scheme in schemes:
            self.js(THEME, scheme == 'dark')
            self.js("document.querySelectorAll('.qa-hover').forEach(e=>e.classList.remove('qa-hover'));"
                    "if(arguments[0])document.getElementById(arguments[0]).classList.add('qa-hover');return true;", hover)
            time.sleep(.25)
            self.shot(f'{name}-{scheme}')
        self.js(THEME, None)
        self.js("document.querySelectorAll('.qa-hover').forEach(e=>e.classList.remove('qa-hover'));return true;")
