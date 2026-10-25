"""App extensions, in a build that has them (--features qa,extras): no 확장
button without a settings file, the menu that extras/track.json adds, the
track window (its title from the start, points and the rows without one,
synthetic tiles from a local tile server, the selected row's place and
coordinates, hovering and clicking the track's end, [ and ] stepping
between points, a row picked there selected in the main window, the place
it opened at next time), closing it without closing the log, a broken
settings file listed with what is wrong, and the app ending with the main
window while the track window is open (track.log)."""
import http.server
import json
import subprocess
import threading
import time

import fixtures
from harness import Timeout

EXTRAS = True

MAP = """const s=document.getElementById('status'),e=document.getElementById('empty'),c=document.getElementById('selection'),a=document.getElementById('alert');return {
status:s.textContent,failed:s.classList.contains('failed'),selection:c.hidden?null:c.textContent,alert:a.hidden?null:a.textContent,
note:e.hidden?null:e.textContent,fit:document.getElementById('fit').disabled,title:document.getElementById('title').textContent,page:document.title,
tiles:document.querySelectorAll('.leaflet-tile-loaded').length,attribution:document.querySelector('.leaflet-control-attribution')?.textContent??null,
zoom:[...document.querySelectorAll('.leaflet-control-zoom a')].map(b=>b.getAttribute('aria-label')),marks:document.querySelectorAll('.leaflet-overlay-pane svg path').length,
here:!!document.querySelector('.leaflet-marker-pane .here')};"""
MENU = """const m=document.getElementById('rowMenu');return !m.hidden&&[...m.querySelectorAll('[role=menuitem]')].map(b=>({
label:(b.querySelector('.menu-label')??b).textContent,note:b.querySelector('.menu-sub')?.textContent??null,disabled:b.getAttribute('aria-disabled')==='true'}));"""
CHOOSE = "[...document.querySelectorAll('#rowMenu [role=menuitem]')].find(b=>b.textContent===arguments[0]).click();return true;"
SELECTED = "return document.querySelector('#logsRows .selected-log-row')?.dataset.lineNumber??null;"
# Picks a row as the track window does: the host shows it in the main window.
PICK = """window.__picked=null;fetch('host/state').then(r=>r.json()).then(s=>fetch(`host/locate?dataset=${s.datasetId}&line=${arguments[0]}`))
.then(r=>{window.__picked=r.status;});return true;"""
# The mouse over the track's end mark (the second round mark), then a click
# there when asked; returns the hover tip.
END = """const p=document.querySelectorAll('.leaflet-overlay-pane svg path')[1].getBoundingClientRect(),m=document.getElementById('map');
const o={bubbles:true,cancelable:true,clientX:p.left+p.width/2,clientY:p.top+p.height/2,button:0};
m.dispatchEvent(new MouseEvent('mousemove',o));if(arguments[0]){m.dispatchEvent(new MouseEvent('mousedown',o));m.dispatchEvent(new MouseEvent('mouseup',o));m.dispatchEvent(new MouseEvent('click',o));}
const t=document.getElementById('tip');return {tip:t.hidden?null:t.textContent,pointer:m.classList.contains('near-track')};"""
KEY = "document.dispatchEvent(new KeyboardEvent('keydown',{key:arguments[0],bubbles:true,cancelable:true}));return true;"


def serve_tiles():
    """A tile server on this computer that draws synthetic tiles."""
    class Tiles(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                z, x, y = self.path.strip('/').removesuffix('.png').split('/')
                body = fixtures.tile_png(int(x), int(y))
            except ValueError:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type', 'image/png')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass
    # A map asks for many tiles at once; the default backlog of 5 resets some.
    http.server.ThreadingHTTPServer.request_queue_size = 64
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Tiles)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def run(ctx):
    points = {line: (lat, lon) for line, lat, lon in fixtures.track(ctx.fixtures)}
    where = lambda line: '{:.5f}, {:.5f}'.format(*points[line])
    check = ctx.check
    server = serve_tiles()
    settings = ctx.data / 'extras' / 'track.json'
    good = json.dumps({
        'title': '테스트 궤적', 'marker': 'onPositionChanged', 'latitude': 'fix.lat', 'longitude': 'fix.lon', 'scale': 1e-7,
        'tiles': f'http://127.0.0.1:{server.server_address[1]}/{{z}}/{{x}}/{{y}}.png', 'attribution': '합성 타일 · 테스트 전용',
    }, ensure_ascii=False)
    try:
        with ctx.launch() as app:
            app.open_fixture('track.log')
            app.wait_count(1200)
            check('without a settings file the 확장 button stays hidden', app.hidden('extrasButton'))
        settings.parent.mkdir(parents=True, exist_ok=True)
        settings.write_text(good, encoding='utf-8')
        with ctx.launch() as app:
            js = app.js
            handles = lambda: app.call(f'/session/{app.sid}/window/handles')
            switch = lambda handle: app.call(f'/session/{app.sid}/window', {'handle': handle})
            state = lambda: js(MAP)
            until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(state()), timeout, what=what)

            def open_track():
                app.click('extrasButton')
                app.wait(lambda: js(MENU), what='extensions menu')
                js(CHOOSE, '테스트 궤적')
                return app.wait(lambda: (lambda h: 'extra-track' in h and h)(handles()), what='track window')
            try:
                app.wait(lambda: not app.hidden('extrasButton'), 10, what='확장 button')
            except Timeout:
                check('the build has extensions (--features qa,extras)', False)
                return
            app.open_fixture('track.log')
            app.wait_count(1200)
            app.click('extrasButton')
            items = app.wait(lambda: js(MENU), what='extensions menu')
            check('the 확장 menu lists the extension by its title, then the settings folder',
                  [i['label'] for i in items] == ['테스트 궤적', '설정 폴더 열기'] and not any(i['disabled'] for i in items), items)
            app.capture('01-extras-menu')
            js(CHOOSE, '테스트 궤적')
            opened = app.wait(lambda: (lambda h: 'extra-track' in h and h)(handles()), what='track window')
            check('choosing it opens the track window', sorted(opened) == ['extra-track', 'main'], opened)

            switch('extra-track')
            title = app.wait(lambda: js("return document.readyState==='complete'&&document.title"), what='track page loaded')
            check('the page has its title from the start', title == '테스트 궤적', title)
            s = until(lambda v: v['status'].startswith('지점 395개') and v['tiles'] > 0, 'points read', 30)
            check('the window draws every point and counts the marked rows without one',
                  s['status'] == '지점 395개 · 좌표를 읽지 못한 행 2개' and not s['failed'] and s['note'] is None and s['alert'] is None and not s['fit'], s)
            check("it shows its settings' title and tile credit, with tiles from the local server, and Korean zoom buttons",
                  s['title'] == '테스트 궤적' and '합성 타일 · 테스트 전용' in (s['attribution'] or '') and s['zoom'] == ['확대', '축소'], s)
            app.capture('02-track-window')

            # A row selected in the main window marks where it was logged.
            switch('main')
            app.jump(500)
            app.wait(lambda: js(SELECTED) == '500', what='row 500 selected')
            switch('extra-track')
            s = until(lambda v: v['selection'] and '500행' in v['selection'], 'selected row on the map')
            check('a selected row without a point shows the last point before it, with its coordinates',
                  s['selection'] == f'500행 → 499행 지점 · {where(499)}' and s['here'], s)
            check("the map draws the start, the end and the selected row's place", s['marks'] == 2 and s['here'], s)
            # A covered window draws its canvas only when the view moves.
            js("document.getElementById('fit').click();return true;")
            app.capture('03-selected-row')

            # Hovering the track's end names it; clicking selects its row.
            hovered = js(END, False)
            check('hovering the track names the nearest pass first, with a pointer',
                  hovered == {'tip': '끝 · 1,198행 · 이곳을 2번 지남 (2/2)', 'pointer': True}, hovered)
            app.capture('04-hover-end')
            js(END, True)
            switch('main')
            line = app.wait(lambda: (lambda v: v == '1198' and v)(js(SELECTED)), what='row 1198 selected')
            check("clicking the track's end selects its row in the main window", line == '1198', line)
            # Clicking there again moves to the earlier pass over that place.
            switch('extra-track')
            js(END, True)
            switch('main')
            line = app.wait(lambda: (lambda v: v == '814' and v)(js(SELECTED)), what='row 814 selected')
            check('clicking the same place again selects the other pass there', line == '814', line)

            # A row the map picks is selected in the main window's logs.
            switch('extra-track')
            js(PICK, 700)
            picked = app.wait(lambda: js("return window.__picked"), what='pick answered')
            switch('main')
            line = app.wait(lambda: (lambda v: v == '700' and v)(js(SELECTED)), what='row 700 selected')
            check('a row the track window picks is selected in the main window', picked == 200 and line == '700', [picked, line])
            app.capture('05-picked-row')
            switch('extra-track')
            s = until(lambda v: v['selection'] and v['selection'].startswith('700행'), 'picked row on the map')
            check("the map then marks the picked row's own point", s['selection'] == f'700행 위치 · {where(700)}', s)

            # ] and [ step to the next and previous point's row.
            js(KEY, ']')
            switch('main')
            after = app.wait(lambda: (lambda v: v == '703' and v)(js(SELECTED)), what='row 703 selected')
            switch('extra-track')
            js(KEY, '[')
            js(KEY, '[')
            switch('main')
            before = app.wait(lambda: (lambda v: v == '697' and v)(js(SELECTED)), what='row 697 selected')
            check('] and [ select the next and previous points\' rows', [after, before] == ['703', '697'], [after, before])

            # A narrow window keeps the selected row's chip whole.
            switch('extra-track')
            app.rect(width=520, height=600)
            time.sleep(.5)
            js("document.getElementById('fit').click();return true;")
            # A covered window does not refit its map to the new size; only
            # the bar's layout is checked.
            narrow = js("""const c=document.getElementById('selection').getBoundingClientRect(),t=document.querySelector('.tools').getBoundingClientRect();
return {chip:document.getElementById('selection').textContent,apart:t.top>=c.bottom||t.left>=c.right,keys:getComputedStyle(document.querySelector('.keys')).display}""")
            check('a narrow window keeps the selected row whole, the tools apart from it, and drops the key hint',
                  narrow['chip'].startswith('697행') and narrow['apart'] and narrow['keys'] == 'none', narrow)
            app.capture('07-narrow')

            # The window opens where it was last.
            switch('extra-track')
            app.rect(x=140, y=120, width=820, height=560)
            time.sleep(.5)
            placed = app.rect()
            app.call(f'/session/{app.sid}/window', method='DELETE', timeout=10)
            left = app.wait(lambda: (lambda h: 'extra-track' not in h and h)(handles()), what='track window closed')
            switch('main')
            time.sleep(.3)
            check('closing the track window leaves the main window and its log as they were',
                  left == ['main'] and app.count() == '로그 · 전체 1,200건' and js(SELECTED) == '697', [left, app.count()])
            saved = json.loads((ctx.data / 'extras-windows.json').read_text())
            open_track()
            switch('extra-track')
            again = app.rect()
            same = all(abs(again[k] - placed[k]) <= 2 for k in ('x', 'y', 'width', 'height'))
            check('the track window opens again where it was, at its size', same and 'track' in saved, {'placed': placed, 'again': again, 'saved': saved})
            app.call(f'/session/{app.sid}/window', method='DELETE', timeout=10)
            app.wait(lambda: 'extra-track' not in handles(), what='track window closed again')
            switch('main')

            # A broken settings file stays listed with what is wrong.
            settings.write_text('{"marker": "onPositionChanged",', encoding='utf-8')
            app.click('extrasButton')
            items = app.wait(lambda: js(MENU), what='menu with a broken extension')
            broken = items[0] if items else {}
            check('a broken settings file shows as an item that cannot be chosen, with what is wrong under it',
                  broken.get('label') == '좌표 궤적 · 설정 오류' and broken.get('disabled')
                  and broken.get('note') == 'extras/track.json · 설정 파일이 올바른 JSON이 아닙니다 (1행).', items)
            app.capture('06-broken-settings')
            app.key('Escape', target='rowMenu')

            # The app ends with the main window, the track window open or not.
            settings.write_text(good, encoding='utf-8')
            open_track()
            try:
                app.close_window()
                ended = True
            except subprocess.TimeoutExpired:
                ended = False
            check('closing the main window ends the app while the track window is open', ended)
    finally:
        server.shutdown()
