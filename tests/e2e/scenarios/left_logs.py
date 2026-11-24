"""Many device logs that earlier runs left: the newest come first, all of them
and the recent files below can be reached with the mouse and the keyboard, and
deleting one with the keyboard goes on to the next."""
import os

LIST = """const state=document.getElementById('emptyState'),view=state.getBoundingClientRect(),inside=e=>{const b=e.getBoundingClientRect();return b.top>=view.top-1&&b.bottom<=view.bottom+1};
return {names:[...document.querySelectorAll('#leftoverList strong')].map(e=>e.textContent),role:document.getElementById('leftoverList').getAttribute('role'),
  title:inside(state.querySelector('h2')),first:inside(document.querySelector('#leftoverList li')),last:inside(document.querySelector('#leftoverList li:last-child')),hints:inside(state.querySelector('.empty-hints')),
  top:state.scrollTop,scrolls:state.scrollHeight>state.clientHeight+1,wide:state.scrollWidth>state.clientWidth,focus:document.activeElement?.getAttribute('aria-label')??document.activeElement?.id??null};"""
SCROLL = "const s=document.getElementById('emptyState');s.scrollTop=arguments[0]?s.scrollHeight:0;return true;"


def run(ctx):
    captures = ctx.home / 'Library/Caches/dev.logstudio.desktop/captures'
    captures.mkdir(parents=True)
    names = [f'{"ios-QA_iPhone" if day % 7 == 0 else "logcat-Pixel_8"}-202611{day:02d}-100000.log' for day in range(1, 31)]
    for day, name in enumerate(names):
        path = captures / name
        path.write_text('11-01 10:15:00.000  4321  4321 I NotesUi: opened note\n', encoding='utf-8')
        os.utime(path, (1_790_000_000 + day * 86_400,) * 2)
    newest = names[::-1]
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        s = app.wait(lambda: (lambda v: v['names'] and v)(js(LIST)), what='left logs')
        check('every left log is listed, the newest first', s['names'] == newest, s['names'][:2])
        check('a long list starts at its top, and the list says it is one', s['title'] and s['first'] and s['top'] == 0 and s['scrolls'] and s['role'] == 'list', s)
        app.capture('01-many-left-logs')
        js(SCROLL, True)
        s = js(LIST)
        check('the end of the list scrolls into view', s['last'] and s['hints'] and not s['wide'], {k: s[k] for k in ('last', 'hints', 'wide')})
        app.capture('02-scrolled-to-the-end')
        js(SCROLL, False)
        # As Tab does, focus brings a file far down into view.
        js("document.querySelector('#leftoverList li:last-child .leftover-remove').focus();return true;")
        check('focus brings the oldest log into view', js(LIST)['last'])

        # Deleting with the keyboard: press, confirm, and the next is ready.
        press = "document.querySelectorAll('#leftoverList .leftover-remove')[arguments[0]].focus();document.activeElement.click();document.activeElement.click();return true;"
        js(press, 0)
        s = app.wait(lambda: (lambda v: len(v['names']) == 29 and v)(js(LIST)), what='newest deleted')
        check('deleting one with the keyboard goes on to the next', s['names'] == newest[1:] and s['focus'] == f'{newest[1]} 삭제' and not (captures / newest[0]).exists(), s['focus'])
        js(press, 28)
        s = app.wait(lambda: (lambda v: len(v['names']) == 28 and v)(js(LIST)), what='oldest deleted')
        check('and at the end to the one before', s['focus'] == f'{newest[-2]} 삭제', s['focus'])
        # A focus moved on while the next one is deleted stays with its file.
        js("const real=window.__fetch=window.fetch;window.fetch=async(url,...rest)=>{if(String(url).endsWith('/delete_leftover'))await new Promise(go=>{window.__go=go;});return real(url,...rest);};return true;")
        js(press, 0)
        app.wait(lambda: js("return !!window.__go"), what='deleting')
        js("document.querySelectorAll('#leftoverList .recent-open')[3].focus();window.__go();return true;")
        app.wait(lambda: len(js(LIST)['names']) == 27, what='next deleted')
        focus = js("return document.activeElement?.querySelector?.('strong')?.textContent??document.activeElement?.tagName")
        check('a focus moved on meanwhile stays with its log', focus == newest[4], focus)
        js("window.fetch=window.__fetch;return true;")
        app.rect(x=0, y=0, width=760, height=600)
        js(SCROLL, False)
        s = js(LIST)
        check('a small window scrolls the list too', s['title'] and s['scrolls'] and not s['wide'], s['top'])
        app.capture('03-many-left-logs-760')
        app.rect(x=0, y=0, width=1280, height=820)
        # Left to the last one, focus goes on past the list.
        for name in newest[3:-1]:
            (captures / name).unlink(missing_ok=True)
        js(press, 0)
        app.wait(lambda: js("return document.getElementById('captureLeftovers').hidden"), what='list gone')
        focus = js("return document.activeElement?.id")
        check('deleting the last one goes on to 기기 로그 기록', focus == 'emptyDevice', focus)
        s = js("const e=document.getElementById('emptyState'),v=e.getBoundingClientRect(),a=e.querySelector('.empty-icon').getBoundingClientRect(),b=e.querySelector('.empty-hints').getBoundingClientRect();return [a.top-v.top,v.bottom-b.bottom,e.scrollHeight<=e.clientHeight]")
        check('a short empty state stays centred', abs(s[0] - s[1]) < 2 and s[2], s)
        app.capture('04-centred')
