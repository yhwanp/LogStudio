// App extensions, which only `extras` builds have (extras/src/lib.rs): the
// 확장 menu that opens their windows, the row selected here that their
// pages show, and a row they pick, which is selected here. Other builds
// list none, so the button stays hidden and nothing is sent.
const SETTLE_MS = 120;

export function mountExtras({button, native, menu, notify, locate, current, isGone, window = globalThis.window, setTimer = setTimeout, clearTimer = clearTimeout}) {
  const invoke = request => native.core.invoke('extras', {request});
  let listed = [], sent = null, timer = null;
  // The row each pane selected last, so that one pane's repaint does not
  // send its row again after the other pane's.
  const last = new Map();
  // Listed again when the window comes back, so a settings file written
  // meanwhile shows the button without a restart.
  async function refresh() {
    try {listed = await invoke({action:'list'});} catch {listed = [];}
    if (!Array.isArray(listed)) listed = [];
    button.hidden = !listed.length;
  }
  function send(datasetId, line) {
    const key = `${datasetId}:${line}`;
    if (key === sent) return;
    sent = key;
    void invoke({action:'select', datasetId, line}).catch(() => {});
  }
  async function open(id) {
    try {await invoke({action:'open', id});}
    catch (error) {notify(String(error).slice(0, 300));return;}
    // A new window shows the row selected now.
    const row = current();
    if (row) {sent = null;send(row.datasetId, row.line);}
  }
  async function reveal() {
    try {await invoke({action:'reveal'});}
    catch (error) {notify(String(error).slice(0, 300));}
  }
  // A broken extension stays in the menu as an item that cannot be chosen,
  // with what is wrong under it, readable while the settings are fixed.
  function items() {
    return [
      ...listed.map(extra => extra.error
        ? [`${extra.title} · 설정 오류`, null, extra.error, {disabled:true, note:`extras/${extra.id}.json · ${extra.error}`}]
        : [extra.title, () => void open(extra.id), `${extra.title} 창을 엽니다`]),
      null,
      ['설정 폴더 열기', () => void reveal(), '확장 기능의 설정 파일(extras/<id>.json)이 있는 폴더를 엽니다'],
    ];
  }
  button.addEventListener('click', async () => {
    await refresh();
    if (isGone() || !listed.length) return;
    const box = button.getBoundingClientRect?.();
    menu.show(items(), box ? box.left : 0, box ? box.bottom + 4 : 0, '확장 기능');
  });
  window?.addEventListener?.('focus', () => {if (!isGone()) void refresh();});
  void native.event.listen('extras-locate', ({payload}) => {
    if (!isGone() && Number.isSafeInteger(payload?.datasetId) && Number.isSafeInteger(payload?.line)) locate(payload.datasetId, payload.line);
  }).catch(() => {});
  void refresh();
  return {
    // Pane `pane` shows dataset `datasetId` with row `line` selected (null
    // when none); the row is sent once the selection settles.
    selected(pane, datasetId, line) {
      if (!listed.length || !Number.isSafeInteger(datasetId) || !Number.isSafeInteger(line)) return;
      const key = `${datasetId}:${line}`;
      if (last.get(pane) === key) return;
      last.set(pane, key);
      if (timer !== null) clearTimer(timer);
      timer = setTimer(() => {timer = null;send(datasetId, line);}, SETTLE_MS);
    },
  };
}
