"""Closing a capture's log or the window while its save or an export runs
(QA builds with scripted devices): the question says so, and its answers wait
for the save, stop it, or keep the log; closing again later does not skip the
question, and a page that stays silent does not keep the window."""
import time

from scenarios.devices import MENU, PICK, QA, QUIT

# Stands in for the native save, so that it lasts: a save ends when the page is
# told to (`window.__end`) or when the page stops it, unless it hangs
# (`window.__hangs`). Off with `window.__real`.
SAVES = """const real=window.__fetch=window.fetch,answer=(body,state)=>new Response(JSON.stringify(body),{headers:{'Content-Type':'application/json','Tauri-Response':state}});
window.__saves=0;window.fetch=(url,...rest)=>{const command=String(url).split('/').at(-1);
if(command==='save_capture'&&!window.__real){window.__saves++;return new Promise(resolve=>{window.__end=()=>resolve(answer({bytes:1024},'ok'));window.__stop=()=>resolve(answer('Save cancelled','error'));});}
if(command==='cancel_export'&&!window.__hangs)window.__stop?.();
return real(url,...rest);};return true;"""
# Nothing the page says about closing arrives, as from one that hangs.
SILENT = """const real=window.fetch;window.fetch=(url,...rest)=>String(url).endsWith('/answer_close')?new Promise(()=>{}):real(url,...rest);return true;"""
STATE = """const text=id=>document.getElementById(id).textContent;return {
  dialog:document.getElementById('captureDialog').open,title:text('captureDialogTitle'),note:text('captureDialogNote'),described:document.getElementById('captureDialogNote').checkVisibility()?text('captureDialogNote'):null,
  buttons:['captureDiscard','captureCancel','captureSaveAs'].map(text),focus:document.activeElement?.id??null,
  bar:document.getElementById('captureBar').hidden?null:text('captureText'),save:text('captureSave'),toast:text('toast'),
  empty:!document.getElementById('emptyState').hidden,saves:window.__saves,
  leftovers:document.getElementById('captureLeftovers').hidden?[]:[...document.querySelectorAll('#leftoverList strong')].map(e=>e.textContent),
  error:document.getElementById('largeError').hidden?null:text('largeError')};"""
# How long native code waits for a silent page.
ANSWER = 5


def run(ctx):
    captures = ctx.home / 'Library/Caches/dev.logstudio.desktop/captures'
    files = lambda: sorted(p.name for p in captures.glob('*.log'))
    with ctx.launch() as app:
        js, check = app.js, ctx.check
        until = lambda pred, what, timeout=20: app.wait(lambda: (lambda v: pred(v) and v)(js(STATE)), timeout, what=what)

        def quit_app():
            try:
                js(QUIT)
            except RuntimeError:  # the app may be gone before it answers
                pass

        def record():
            app.click('emptyDevice')
            app.wait(lambda: any(x.startswith('Pixel 8') for x in js(MENU)), what='device menu')
            js(PICK, 'Pixel 8')
            until(lambda v: v['bar'] and '기록 중' in v['bar'] and '저장 안 됨' in v['bar'], 'recording')
        js(QA, 'fake')
        js(SAVES)
        record()

        # A question that is being read is not skipped by closing again.
        quit_app()
        until(lambda v: v['dialog'], 'question')
        time.sleep(ANSWER + .5)
        quit_app()
        time.sleep(.5)
        s = js(STATE)
        check('closing again after 5 s keeps asking', app.proc.poll() is None and s['dialog'], s['title'])
        app.click('captureCancel')
        until(lambda v: not v['dialog'], 'kept')

        # Closing while it saves: 취소 keeps the log, also once the save ends.
        app.click('captureSave')
        until(lambda v: v['save'] == '저장 취소', 'saving')
        quit_app()
        s = until(lambda v: v['dialog'], 'question while saving')
        check('closing while a capture is saved says that it is',
              s['title'] == '기기 로그를 저장하고 있습니다' and s['buttons'] == ['저장 멈추고 닫기', '취소', '저장 끝나면 닫기']
              and s['note'] == '' and s['described'] is None and s['focus'] == 'captureSaveAs', s)
        app.capture('01-close-while-saving', hover='captureSaveAs')
        app.rect(x=0, y=0, width=760, height=600)
        app.capture('02-close-while-saving-760')
        app.rect(x=0, y=0, width=1280, height=820)
        app.click('captureCancel')
        js('window.__end();return true;')
        s = until(lambda v: v['save'] == '저장', 'saved')
        time.sleep(.5)
        check('취소 keeps the log once the save ends', app.proc.poll() is None and js(STATE)['bar'] is not None and '저장했습니다' in s['toast'], s['toast'])

        # 저장 chosen as the log closes, then closing again and 취소: the first
        # close is not made behind the answer.
        app.menu('파일 닫기')
        s = until(lambda v: v['dialog'], 'plain question')
        check('the plain question is back', s['title'] == '기기 로그를 저장할까요?' and s['buttons'] == ['저장\u00a0안\u00a0함', '취소', '저장'], s['buttons'])
        app.click('captureSaveAs')
        until(lambda v: v['save'] == '저장 취소', 'saving to close')
        quit_app()
        until(lambda v: v['dialog'] and v['title'].endswith('저장하고 있습니다'), 'asked again')
        app.click('captureCancel')
        js('window.__end();return true;')
        until(lambda v: v['save'] == '저장', 'saved again')
        time.sleep(.5)
        s = js(STATE)
        check('a close kept is not made by the one before', s['bar'] is not None and not s['empty'] and s['saves'] == 2, s['saves'])

        # 저장 멈추고 닫기 stops the save and deletes the rows.
        app.click('captureSave')
        until(lambda v: v['save'] == '저장 취소', 'saving once more')
        app.menu('파일 닫기')
        until(lambda v: v['dialog'], 'question while saving')
        app.click('captureDiscard')
        s = until(lambda v: v['empty'], 'closed')
        app.wait(lambda: not files(), what='rows deleted')
        s = js(STATE)
        check('저장 멈추고 닫기 closes the log and deletes its rows', s['empty'] and not s['leftovers'] and not files() and s['error'] is None, {'leftovers': s['leftovers'], 'files': files()})

        # 저장 끝나면 닫기 waits for the save that runs.
        record()
        app.click('captureSave')
        until(lambda v: v['save'] == '저장 취소', 'saving to the end')
        app.menu('파일 닫기')
        until(lambda v: v['dialog'], 'question')
        app.click('captureSaveAs')
        time.sleep(.5)
        s = js(STATE)
        check('저장 끝나면 닫기 waits, without another save', s['bar'] is not None and not s['dialog'] and s['saves'] == 4, s['saves'])
        js('window.__end();return true;')
        until(lambda v: v['empty'], 'closed after the save')

        # A capture saved before closes as it would: its file goes, saved.
        old = files()
        record()
        app.click('captureStop')
        until(lambda v: v['bar'] and '저장 안 됨' in v['bar'] and '기록 중지' in v['bar'], 'stopped')
        js('window.__real=true;return true;')
        app.click('captureSave')
        until(lambda v: v['bar'] and '저장함' in v['bar'], 'saved for real')
        saved = {p.name: p.read_bytes() for p in ctx.saves.iterdir()}
        js('window.__real=false;return true;')
        app.click('captureSave')
        until(lambda v: v['save'] == '저장 취소', 'saving again')
        app.menu('파일 닫기')
        s = until(lambda v: v['dialog'], 'question while saving again')
        check('closing while a saved capture is saved again asks', s['title'] == '기기 로그를 저장하고 있습니다', s['title'])
        app.click('captureDiscard')
        until(lambda v: v['empty'], 'closed')
        app.wait(lambda: files() == old, what='saved rows gone')
        check('and closes with the copy it has', {p.name: p.read_bytes() for p in ctx.saves.iterdir()} == saved and len(saved) == 1, list(saved))

        # A save that hangs does not keep the log open, and its rows stay.
        record()
        before = files()
        js('window.__hangs=true;return true;')
        app.click('captureSave')
        until(lambda v: v['save'] == '저장 취소', 'saving to a disk that hangs')
        app.menu('파일 닫기')
        until(lambda v: v['dialog'], 'question while it hangs')
        app.click('captureDiscard')
        time.sleep(1)
        check('a save gets a moment to stop', js(STATE)['bar'] is not None)
        s = until(lambda v: v['empty'] and v['leftovers'], 'closed over the save')
        check('a save that does not stop closes the log and keeps its rows', sorted(s['leftovers']) == before == files() and s['error'] is None, s['leftovers'])
        js('window.__hangs=false;window.__stop();return true;')

        # Only a page that stays silent is closed over, and its rows stay.
        record()
        before = files()
        js(SILENT)
        quit_app()
        until(lambda v: v['dialog'], 'question of a silent page')
        quit_app()
        time.sleep(.5)
        check('a silent page keeps the window at first', app.proc.poll() is None)
        time.sleep(ANSWER)
        quit_app()
        try:
            app.proc.wait(5)
        except Exception:
            pass
        check('a page that stays silent does not keep the window, and its rows stay', app.proc.poll() is not None and files() == before, files())
