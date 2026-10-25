// First-use tips and the feature tour share one overlay: the page around a
// control dims, the control stays lit and a card beside it explains it.
//
// A tip appears once, the first time its feature is in reach and the user
// pauses, so it belongs to what the user just did. It takes neither focus
// nor input: presses pass through to the page,
// and a press elsewhere, focus moving to a text field, Escape or its buttons
// dismiss it. The tour, started from the shortcut list or the first tip, is
// modal and walks the main controls; without an open file it opens the
// sample log first, and the steps it shows count as seen tips. Seen tips and
// the on/off setting stay in localStorage.
import {isEditableTarget} from './clipboard.js';

const STORAGE_KEY = 'logstudio.tips.v1';
// A tip waits for IDLE_MS without key, pointer or wheel input and gives up
// after GIVE_UP_MS (its trigger offers it again later).
export const IDLE_MS = 700, GIVE_UP_MS = 6000, SAMPLE_WAIT_MS = 10000;
const PAD = 4, GAP = 12, MARGIN = 8, SIDES = ['bottom', 'top', 'right', 'left'];

const byId = id => document => document.getElementById(id);
const logsViewport = byId('logsViewport');
// The selected row of the upper logs if it is wholly in view, else the first
// row that is; rows above and below the list's scroll area do not count.
function rowInView(document) {
  const box = logsViewport(document)?.getBoundingClientRect?.();
  if (!box) return null;
  const inside = row => { const r = row.getBoundingClientRect(); return r.height > 0 && r.top >= box.top - 1 && r.bottom <= box.bottom + 1; };
  const rows = [...document.querySelectorAll('#logsRows .log-row')];
  return rows.find(row => row.classList.contains('selected-log-row') && inside(row)) ?? rows.find(inside) ?? null;
}

// Text marks keys as {key}; a line break starts a new paragraph. Each tip
// names the place that triggers it.
export const TIPS = {
  // Offered while a file is shown; a tip for something the user just did
  // takes its place while it waits for a pause.
  help: {ambient: true, target: byId('shortcutHelp'), title: '처음 사용하시나요?', body: '주요 기능을 화면에서 차례로 짚어 드립니다. 단축키 목록과 이 둘러보기는 이 버튼이나 {?} 키로 언제든 열 수 있습니다.', next: '둘러보기', back: '닫기', tour: true},
  // Offered when the filter panel opens; the panel stays open meanwhile.
  levels: {target: byId('levelFilters'), sides: ['right', 'bottom', 'top', 'left'], title: '레벨 필터', body: '레벨을 눌러 켜고 끄며, {Alt}+클릭하면 그 레벨만 남깁니다. 태그·PID·TID 칸에는 쉼표로 여러 값을 적을 수 있고, + 메시지를 눌러 메시지 내용으로도 거를 수 있습니다.'},
  // Offered when a row of the upper logs is selected.
  rows: {target: document => document.querySelector('#logsRows .selected-log-row'), clip: logsViewport, title: '행 복사·북마크·행 메뉴', body: '행을 드래그하거나 {Shift}+클릭해 이어진 행을, {⌘}+클릭으로 떨어진 행을 선택해 {⌘C}로 원문을 복사합니다.\n다시 볼 행은 {⌘B}로 북마크해 두세요. 필터나 검색을 바꿔도 남고, 아래 북마크 탭에 모여 {F2}로 오갈 수 있습니다.\n행을 우클릭하면 그 행의 태그·메시지·PID·시각으로 바로 거를 수 있습니다.'},
  // Offered when search results are shown.
  search: {target: byId('largeSearchPane'), sides: ['top', 'bottom', 'right', 'left'], title: '검색 결과를 위 로그에서 보기', body: '결과 행을 선택하면 위 로그도 그 행을 보여 주고, 더블클릭이나 {Enter}로 위 로그로 이동합니다. 검색창에서는 {Enter}·{Shift}+{Enter}로 결과를 차례로 넘깁니다.'},
  // Offered when the first bookmark makes the tab appear.
  bookmarks: {target: byId('bookmarkTab'), title: '북마크', body: '북마크한 행은 이 탭에 모입니다. {F2}·{Shift}+{F2}로 위 로그에서 북마크 사이를 오갑니다.'},
};

// Steps whose control is not on screen when the tour starts are left out. A
// step shown counts as seeing the tip it `covers`.
export const TOUR = [
  {target: byId('openFile'), title: '로그 열기', body: '{⌘O}로 파일을 엽니다. 여러 파일을 함께 선택하거나 창에 끌어다 놓으면 시간순으로 합쳐 하나의 로그로 엽니다. 로그 텍스트는 {⌘V}로 붙여넣습니다.'},
  {target: byId('reopenFile'), title: '다시 열기', body: '파일이 바뀌면 {⌘R}로 다시 읽습니다. 필터·검색·북마크와 보던 위치는 그대로 남습니다.'},
  {covers: 'levels', target: byId('toggleFilters'), sides: ['right', 'bottom', 'top', 'left'], title: '필터', body: '레벨·태그·메시지·PID/TID·시간으로 위 로그를 거릅니다. 보기 조건을 추가하면 그중 하나라도 맞는 줄을 보여 주고, 제외 조건에 맞는 줄은 숨깁니다. 레벨은 {Alt}+클릭하면 그 레벨만 남고, 태그·PID·TID 칸에는 쉼표로 여러 값을 적을 수 있습니다.'},
  {target: byId('savedFilterButton'), sides: ['bottom', 'right', 'top', 'left'], title: '저장된 필터', body: '자주 쓰는 필터를 이름 붙여 저장해 두고, 다른 파일을 열었을 때 여기서 선택해 바로 적용합니다. 적용한 뒤 조건을 바꾸면 \'수정됨\'이 붙고, 다시 저장하거나 저장한 조건으로 되돌릴 수 있습니다.'},
  {covers: 'rows', target: rowInView, clip: logsViewport, title: '행 선택과 복사', body: '행을 클릭하면 아래에 그 로그의 원문이 열립니다. 드래그·{Shift}+클릭으로 이어진 행을, {⌘}+클릭으로 떨어진 행을 선택해 {⌘C}로 복사하고, 우클릭하면 그 행의 태그·메시지·PID·시각으로 바로 거릅니다.'},
  {covers: 'bookmarks', target: document => rowInView(document)?.children?.[0] ?? null, clip: logsViewport, title: '북마크', body: '다시 볼 행은 {⌘B}로 북마크해 두세요. 북마크하면 행 번호에 표시가 붙고 아래 북마크 탭에 모이며, 필터나 검색을 바꿔도 남습니다. {F2}·{Shift}+{F2}로 북마크 사이를 오갑니다.'},
  {target: byId('logsHeaders'), title: '열 폭', body: '머리글 경계를 드래그해 폭을 바꾸고, 경계를 더블클릭하면 내용에 맞춥니다. 머리글을 우클릭하면 모든 열을 내용에 맞추거나 기본 폭으로 되돌립니다.'},
  {covers: 'search', target: byId('largeSearchTools'), sides: ['top', 'bottom', 'right', 'left'], title: '검색', body: '{⌘F}나 {/}로 검색창으로 갑니다. 결과는 아래쪽에 열리고 {Enter}·{Shift}+{Enter}로 결과를 위 로그에서 차례로 보여 줍니다. 결과가 열리면 경계선을 끌어 위아래 높이를 바꿀 수 있습니다.'},
  {target: byId('shortcutHelp'), title: '단축키와 둘러보기', body: '전체 단축키 목록은 이 버튼이나 {?} 키로 언제든 엽니다. 이 둘러보기도 그 창에서 다시 시작할 수 있습니다.'},
];

export function readTipState(storage) {
  let saved = null;
  try { saved = JSON.parse(storage?.getItem(STORAGE_KEY) ?? 'null'); } catch {}
  const seen = Array.isArray(saved?.seen) ? saved.seen.filter(id => typeof id === 'string').slice(0, 64) : [];
  return {off: saved?.off === true, seen: new Set(seen)};
}
function writeTipState(storage, state) {
  try { storage?.setItem(STORAGE_KEY, JSON.stringify({off: state.off, seen: [...state.seen]})); } catch {}
}

// Key names become <kbd>; ⌘ reads Ctrl+ off Apple platforms, and ⌘ alone
// (before +클릭) reads Ctrl. Text stays text.
export function richText(document, text, mod = '⌘') {
  return String(text).split(/\{([^{}]+)\}/).flatMap((part, index) => {
    if (!part) return [];
    if (index % 2 === 0) return [document.createTextNode(part)];
    const key = document.createElement('kbd'); key.textContent = part === '⌘' ? mod.replace(/\+$/, '') : part.replace('⌘', mod); return [key];
  });
}

// The card goes on the first side in `sides` where it fits in the viewport
// beside the lit area, else over the area's lower part. Above or below it is
// centered on the area; beside it, it hangs from the area's top. `arrow` is
// the pointer's offset along the card edge that faces the area.
export function placeCard(spot, card, view, sides = SIDES) {
  const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
  const cx = (spot.left + spot.right) / 2, cy = (spot.top + spot.bottom) / 2;
  const x = clamp(cx - card.width / 2, MARGIN, view.width - card.width - MARGIN);
  const y = clamp(spot.top, MARGIN, view.height - card.height - MARGIN);
  const fits = {
    bottom: spot.bottom + GAP + card.height <= view.height - MARGIN, top: spot.top - GAP - card.height >= MARGIN,
    right: spot.right + GAP + card.width <= view.width - MARGIN, left: spot.left - GAP - card.width >= MARGIN,
  };
  const side = sides.find(name => fits[name]);
  // A pointer beside a tall area points into its upper part, by the card.
  const along = (center, start, length) => clamp(center - start, 16, length - 16);
  const beside = Math.min(cy, spot.top + Math.min(card.height, spot.bottom - spot.top) / 2);
  if (side === 'bottom') return {side, left: x, top: spot.bottom + GAP, arrow: along(cx, x, card.width)};
  if (side === 'top') return {side, left: x, top: spot.top - GAP - card.height, arrow: along(cx, x, card.width)};
  if (side === 'right') return {side, left: spot.right + GAP, top: y, arrow: along(beside, y, card.height)};
  if (side === 'left') return {side, left: spot.left - GAP - card.width, top: y, arrow: along(beside, y, card.height)};
  return {side: 'inside', left: x, top: clamp(spot.bottom - GAP - card.height, MARGIN, view.height - card.height - MARGIN), arrow: null};
}

// The part of an element on screen and inside its clipping list, or null
// when it is hidden, inert or mostly out of sight.
function litRect(element, clip, view) {
  if (!element?.isConnected || element.closest?.('[hidden],[inert]') || !element.getClientRects?.().length) return null;
  const r = element.getBoundingClientRect(), c = clip?.getBoundingClientRect?.();
  const left = Math.max(r.left, 0, c?.left ?? 0), top = Math.max(r.top, 0, c?.top ?? 0);
  const right = Math.min(r.right, view.width, c?.right ?? Infinity), bottom = Math.min(r.bottom, view.height, c?.bottom ?? Infinity);
  return right - left >= 8 && bottom - top >= Math.min(r.height / 2, 16) ? {left, top, right, bottom} : null;
}

export function mountTips({document = globalThis.document, window = globalThis.window, mod, needsFile = () => false, openSample = () => {}, closeHelp = () => {}, now = () => window.performance?.now?.() ?? Date.now()} = {}) {
  const $ = id => document.getElementById(id);
  const layer = $('tipLayer'), card = $('tipCard'), spot = $('tipSpot'), title = $('tipTitle'), body = $('tipBody'), counter = $('tipStep');
  const back = $('tipBack'), next = $('tipNext'), off = $('tipOff'), close = $('tipClose'), enabled = $('tipsEnabled');
  if (!layer || !card || !spot || !title || !body || !back || !next) return {offer() {}, startTour() {}, get mode() { return null; }};
  mod ??= /Mac|iPhone|iPad/.test(window.navigator?.platform ?? '') ? '⌘' : 'Ctrl+';
  let storage = null;
  try { storage = window.localStorage; } catch {}
  let state = readTipState(storage);
  // mode: null, 'tip' or 'tour'. `current` is the shown tip or step and its
  // control; `pending` a tip waiting for a pause.
  let mode = null, current = null, pending = null, tour = null, opener = null, frame = null, placed = '';
  let lastInput = -Infinity;
  const view = () => ({width: window.innerWidth || 0, height: window.innerHeight || 0});
  const busy = () => !!document.querySelector('dialog[open]:not(#tipLayer)') || $('rowMenu')?.hidden === false || $('dropHint')?.hidden === false || !!document.querySelector('.resizing-columns,.resizing-panes');
  function syncSetting() { if (enabled) enabled.checked = !state.off; }
  // Settings are read again before each change: another launch or a test
  // harness may have changed them.
  function save(change) { state = readTipState(storage); change(state); writeTipState(storage, state); syncSetting(); }
  syncSetting();

  function offer(id, {hold} = {}) {
    const def = TIPS[id];
    if (!def || mode || state.off || state.seen.has(id)) return;
    if (pending) {
      if (pending.id === id || !pending.def.ambient || def.ambient) return;
      window.clearTimeout(pending.timer);
    }
    pending = {id, def, hold, since: now(), timer: null};
    pending.timer = window.setTimeout(attempt, IDLE_MS);
  }
  function attempt() {
    const wait = pending;
    if (!wait) return;
    const time = now(), idle = time - lastInput;
    if (idle < IDLE_MS) {
      if (time - wait.since < GIVE_UP_MS) wait.timer = window.setTimeout(attempt, IDLE_MS - idle); else pending = null;
      return;
    }
    pending = null; state = readTipState(storage); syncSetting();
    if (mode || state.off || state.seen.has(wait.id) || busy()) return;
    const element = wait.def.target(document);
    if (!litRect(element, wait.def.clip?.(document), view())) return;
    mode = 'tip'; current = {id: wait.id, def: wait.def, hold: wait.hold, element};
    layer.classList.remove('tour-mode'); layer.dataset.tip = wait.id;
    // Opening by attribute shows the layer without moving focus.
    layer.setAttribute('open', '');
    body.setAttribute('aria-live', 'polite');
    fill(wait.def, {step: '', back: wait.def.back ?? null, next: wait.def.next ?? '확인', off: true});
    wait.hold?.(true);
    update(); loop();
  }
  function hideTip(seen) {
    if (mode !== 'tip') return;
    const shown = current; mode = null; current = null; stopLoop(); closeLayer();
    shown.hold?.(false);
    if (seen) save(s => s.seen.add(shown.id));
  }

  function startTour() {
    if (mode === 'tour') return;
    if (pending) { window.clearTimeout(pending.timer); pending = null; }
    hideTip(true);
    // The tour is what the first tip offers; it is not offered afterwards.
    save(s => s.seen.add('help'));
    opener = document.activeElement; mode = 'tour';
    const run = tour = {steps: [], index: -1, timer: null};
    layer.classList.add('tour-mode'); layer.dataset.tip = 'tour';
    closeLayer(); layer.showModal(); body.setAttribute('aria-live', 'off');
    if (needsFile()) {
      current = null;
      fill({title: '샘플 로그를 여는 중…', body: '둘러보기는 샘플 로그로 진행합니다.'}, {step: '', back: null, next: null, off: false});
      update(); close?.focus({preventScroll: true});
      openSample();
      const started = now();
      const poll = () => {
        if (tour !== run) return;
        if (rowInView(document) || now() - started >= SAMPLE_WAIT_MS) begin(); else run.timer = window.setTimeout(poll, 100);
      };
      run.timer = window.setTimeout(poll, 100);
    } else begin();
    loop();
  }
  function begin() {
    tour.steps = TOUR.filter(s => litRect(s.target(document), s.clip?.(document), view()));
    if (tour.steps.length) go(0); else endTour();
  }
  function go(index) {
    const s = tour.steps[index], last = index === tour.steps.length - 1;
    tour.index = index; current = {def: s, element: s.target(document)};
    if (s.covers) save(saved => saved.seen.add(s.covers));
    fill(s, {step: `${index + 1} / ${tour.steps.length}`, back: index ? '이전' : null, next: last ? '마치기' : '다음', off: false});
    update(); next.focus({preventScroll: true});
  }
  function endTour() {
    if (mode !== 'tour') return;
    window.clearTimeout(tour.timer); tour = null; mode = null; current = null; stopLoop(); closeLayer();
    // A sample opened for the tour replaces the empty state that had focus.
    const target = opener && opener !== document.body && litRect(opener, null, view()) ? opener : logsViewport(document);
    opener = null; target?.focus?.({preventScroll: true});
  }
  function closeLayer() {
    if (layer.open && typeof layer.close === 'function') layer.close();
    layer.removeAttribute('open');
  }

  function fill(def, {step, back: backLabel, next: nextLabel, off: offShown}) {
    title.textContent = def.title;
    body.replaceChildren(...def.body.split('\n').map(text => { const line = document.createElement('span'); line.className = 'tip-line'; line.append(...richText(document, text, mod)); return line; }));
    if (counter) { counter.textContent = step; counter.hidden = !step; }
    back.hidden = !backLabel; back.textContent = backLabel ?? '';
    next.hidden = !nextLabel; next.textContent = nextLabel ?? '';
    if (off) off.hidden = !offShown;
    placed = '';
  }
  // Follows the control while the overlay is up: it can move, resize or
  // scroll away. A tip whose control leaves the screen closes unseen.
  function loop() { if (frame === null) frame = window.requestAnimationFrame(tick); }
  function stopLoop() { if (frame !== null) window.cancelAnimationFrame(frame); frame = null; }
  function tick() { frame = null; if (!mode) return; update(); if (mode) loop(); }
  function update() {
    if (mode === 'tip' && busy()) return hideTip(false);
    let rect = null;
    if (current) {
      // A tip follows its control's current element, such as a newly selected row.
      if (mode === 'tip' || !current.element?.isConnected) current.element = current.def.target(document);
      rect = litRect(current.element, current.def.clip?.(document), view());
      if (!rect && mode === 'tip') return hideTip(false);
    }
    place(rect);
  }
  function place(rect) {
    const size = {width: card.offsetWidth || 0, height: card.offsetHeight || 0}, v = view();
    const key = JSON.stringify([rect, size, v]);
    if (key === placed) return;
    placed = key;
    const lit = rect && {left: rect.left - PAD, top: rect.top - PAD, right: rect.right + PAD, bottom: rect.bottom + PAD};
    spot.hidden = !lit; layer.classList.toggle('no-spot', !lit);
    if (lit) for (const [name, value] of [['left', lit.left], ['top', lit.top], ['width', lit.right - lit.left], ['height', lit.bottom - lit.top]]) spot.style.setProperty(name, `${Math.round(value)}px`);
    const at = lit ? placeCard(lit, size, v, current.def.sides) : {side: 'center', left: (v.width - size.width) / 2, top: (v.height - size.height) / 2, arrow: null};
    card.dataset.side = at.side;
    card.style.setProperty('left', `${Math.round(at.left)}px`); card.style.setProperty('top', `${Math.round(at.top)}px`);
    if (at.arrow !== null) card.style.setProperty('--arrow', `${Math.round(at.arrow)}px`);
  }

  next.addEventListener('click', () => {
    if (mode === 'tip') { const tourNext = current.def.tour; hideTip(true); if (tourNext) startTour(); }
    else if (mode === 'tour' && tour.steps.length) { if (tour.index + 1 < tour.steps.length) go(tour.index + 1); else endTour(); }
  });
  back.addEventListener('click', () => { if (mode === 'tip') hideTip(true); else if (mode === 'tour' && tour.index > 0) go(tour.index - 1); });
  close?.addEventListener('click', () => { if (mode === 'tour') endTour(); else hideTip(true); });
  off?.addEventListener('click', () => { save(s => { s.off = true; }); hideTip(true); });
  enabled?.addEventListener('change', () => save(s => { s.off = !enabled.checked; }));
  // The shortcut list returns focus to its opener first, which the tour then keeps.
  $('startTour')?.addEventListener('click', () => {
    const dialog = $('shortcutDialog');
    if (dialog?.open) { dialog.addEventListener('close', startTour, {once: true}); closeHelp(); } else startTour();
  });

  // Input anywhere marks the user busy. Escape closes the tip or the tour
  // before anything else; a press or a text field outside a tip dismisses it.
  window.addEventListener('keydown', event => {
    lastInput = now();
    if (!mode || event.key !== 'Escape' || event.isComposing) return;
    event.preventDefault(); event.stopPropagation();
    if (mode === 'tour') endTour(); else hideTip(true);
  }, {capture: true});
  window.addEventListener('pointerdown', event => {
    lastInput = now();
    if (mode === 'tip' && !card.contains(event.target)) hideTip(true);
  }, {capture: true});
  window.addEventListener('wheel', () => { lastInput = now(); }, {capture: true, passive: true});
  window.addEventListener('focusin', event => { if (mode === 'tip' && !card.contains(event.target) && isEditableTarget(event.target)) hideTip(true); }, {capture: true});
  // A tip sits above the page: pressing its buttons keeps focus where it was
  // and does not count as a press outside the auto-hiding filter panel.
  card.addEventListener('mousedown', event => { if (mode === 'tip') event.preventDefault(); });
  for (const type of ['pointerdown', 'focusin']) card.addEventListener(type, event => { if (mode === 'tip') event.stopPropagation(); });
  // Page shortcuts wait until the tour ends; arrows step through it.
  layer.addEventListener('keydown', event => {
    if (mode !== 'tour') return;
    event.stopPropagation();
    if (event.altKey || event.ctrlKey || event.metaKey || !tour?.steps.length) return;
    if (event.key === 'ArrowRight') { event.preventDefault(); next.click(); }
    else if (event.key === 'ArrowLeft' && tour.index > 0) { event.preventDefault(); go(tour.index - 1); }
  });
  layer.addEventListener('cancel', event => { event.preventDefault(); endTour(); });
  window.addEventListener('pagehide', () => { stopLoop(); if (pending) window.clearTimeout(pending.timer); pending = null; }, {once: true});

  return {offer, startTour, get mode() { return mode; }};
}
