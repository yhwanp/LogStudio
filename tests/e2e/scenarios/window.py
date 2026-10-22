"""Window geometry: the default centered frame, saving a settled move or
resize, restoring it, maximized state, saving at exit, and recovering from
off-screen, oversized and corrupt saved states. Moves the real window."""
import json
import time


def run(ctx):
    check = ctx.check
    state = ctx.data / 'window-state.json'

    def saved():
        try:
            return json.loads(state.read_text())
        except Exception:
            return None

    def centered(r, screen):
        return r['width'] == 1280 and abs((r['x'] - screen['x']) * 2 + r['width'] - screen['width']) <= 2

    with ctx.launch() as app:
        screen = app.js("return {x:screen.availLeft,y:screen.availTop,width:screen.availWidth,height:screen.availHeight}")
        r0 = app.rect()
        check('without a saved state the 1280x820 content opens centered on the main screen', centered(r0, screen) and r0['height'] >= 820, {'rect': r0, 'screen': screen})
        app.rect(x=screen['x'] + 120, y=screen['y'] + 60, width=1000, height=700)
        time.sleep(1.2)
        s1 = saved()
        check('a move or resize is saved once it settles (outer top-left, content size, title bar)',
              s1 and s1['x'] == screen['x'] + 120 and s1['y'] == screen['y'] + 60 and s1['width'] + s1['borderWidth'] == 1000
              and s1['height'] + s1['borderHeight'] == 700 and not s1['maximized'], s1)
    with ctx.launch() as app:
        r1 = app.rect()
        check('the next launch restores the saved size and position', (r1['x'], r1['y'], r1['width'], r1['height']) == (screen['x'] + 120, screen['y'] + 60, 1000, 700), r1)
        app.maximize()
        time.sleep(1.2)
        s2 = saved()
        check('maximizing keeps the frame it returns to and records maximized', s2 and s2['maximized'] and s2['x'] == s1['x'] and s2['width'] == s1['width'], s2)
    with ctx.launch() as app:
        time.sleep(.8)
        r2 = app.rect()
        check('the next launch opens maximized', abs(r2['width'] - screen['width']) <= 2 and abs(r2['y'] + r2['height'] - screen['y'] - screen['height']) <= 2, {'rect': r2, 'screen': screen})
        # Quitting right after a change: the window is destroyed without a close request.
        app.rect(x=screen['x'] + 40, y=screen['y'] + 30, width=900, height=650)
        app.close_window()
        s3 = saved()
        check('destroying the window right after a change still saves it at exit', s3 and s3['x'] == screen['x'] + 40 and s3['width'] == 900 and not s3['maximized'], s3)
    for name, content, expect in [
        ('an off-screen frame (disconnected monitor) opens centered at the default', {'x': 50000, 'y': 40000, 'width': 1000, 'height': 672, 'borderWidth': 0, 'borderHeight': 28}, 'default'),
        ('a partly off-screen frame is pulled fully onto the screen', {'x': screen['x'] + screen['width'] - 300, 'y': screen['y'] + screen['height'] - 200, 'width': 1000, 'height': 672, 'borderWidth': 0, 'borderHeight': 28}, 'inside'),
        ('an oversized frame shrinks to the work area', {'x': screen['x'], 'y': screen['y'], 'width': 9000, 'height': 9000, 'borderWidth': 0, 'borderHeight': 28}, 'fill'),
        ('a corrupt state file opens centered at the default', 'not json', 'default'),
    ]:
        state.write_text(content if isinstance(content, str) else json.dumps(content))
        with ctx.launch() as app:
            r = app.rect()
            if expect == 'default':
                ok = centered(r, screen)
            elif expect == 'inside':
                # The written border (28) may differ from this window's real frame; only containment matters.
                ok = r['x'] + r['width'] <= screen['x'] + screen['width'] and r['y'] + r['height'] <= screen['y'] + screen['height'] and r['width'] == 1000 and r['height'] >= 672
            else:
                ok = r['width'] == screen['width'] and screen['y'] <= r['y'] and r['y'] + r['height'] <= screen['y'] + screen['height']
            check(name, ok, {'rect': r, 'screen': screen})
