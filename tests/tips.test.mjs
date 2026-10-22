import test from 'node:test';
import assert from 'node:assert/strict';
import {mountTips, placeCard, richText, readTipState, TOUR, IDLE_MS, GIVE_UP_MS} from '../dist/tips.js';

const rect = (left, top, right, bottom) => ({left, top, right, bottom, width: right - left, height: bottom - top});
const KEY = 'logstudio.tips.v1';

function listen(target) {
  target.listeners = new Map();
  target.addEventListener = (type, fn, options) => (target.listeners.get(type) ?? target.listeners.set(type, []).get(type)).push({fn, once: options?.once === true});
  target.fire = (type, event = {}) => {
    let prevented = false, stopped = false;
    const full = {target, preventDefault() { prevented = true; }, stopPropagation() { stopped = true; }, ...event};
    for (const entry of [...(target.listeners.get(type) ?? [])]) {
      if (entry.once) target.listeners.set(type, target.listeners.get(type).filter(other => other !== entry));
      entry.fn(full);
    }
    return {prevented, stopped};
  };
}
class Element {
  constructor(document, {id = '', tag = 'div', box = null, parent = null} = {}) {
    Object.assign(this, {ownerDocument: document, id, tagName: tag.toUpperCase(), box, parent, hidden: false, inert: false, isConnected: true, textContent: '', children: [], dataset: {}, attributes: new Map(), offsetWidth: 0, offsetHeight: 0});
    this.style = {values: {}, setProperty(name, value) { this.values[name] = value; }};
    const classes = new Set();
    this.classList = {add: name => classes.add(name), remove: name => classes.delete(name), contains: name => classes.has(name), toggle: (name, on = !classes.has(name)) => { if (on) classes.add(name); else classes.delete(name); return on; }};
    listen(this);
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  removeAttribute(name) { this.attributes.delete(name); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  getBoundingClientRect() { return this.box ?? rect(0, 0, 0, 0); }
  getClientRects() { return this.box && !this.hidden ? [this.box] : []; }
  closest(selector) {
    for (let node = this; node; node = node.parent) if ((selector.includes('[hidden]') && node.hidden) || (selector.includes('[inert]') && node.inert)) return node;
    return null;
  }
  contains(node) { for (; node; node = node.parent) if (node === this) return true; return false; }
  focus() { this.ownerDocument.activeElement = this; }
  click() { this.fire('click'); }
  replaceChildren(...nodes) { this.children = nodes; this.textContent = nodes.map(node => node.textContent).join(''); }
  append(...nodes) { this.replaceChildren(...this.children, ...nodes); }
}
// A dialog opened by attribute is non-modal; showModal() makes it modal.
class Dialog extends Element {
  constructor(document, options) { super(document, {...options, tag: 'dialog'}); this.open = false; this.modal = false; }
  setAttribute(name, value) { if (name === 'open') this.open = true; else super.setAttribute(name, value); }
  removeAttribute(name) { if (name === 'open') { this.open = false; this.modal = false; } else super.removeAttribute(name); }
  showModal() { if (this.open) throw new Error('InvalidStateError'); this.open = true; this.modal = true; }
  close() { if (!this.open) return; this.open = false; this.modal = false; this.fire('close'); }
}

function rig({saved = null, file = true} = {}) {
  let time = 1000, timers = [], frames = [], seq = 0;
  const store = new Map(saved ? [[KEY, JSON.stringify(saved)]] : []);
  const window = {innerWidth: 1200, innerHeight: 800, navigator: {platform: 'MacIntel'},
    localStorage: {getItem: key => store.get(key) ?? null, setItem: (key, value) => store.set(key, value)},
    setTimeout: (fn, ms) => { timers.push({id: ++seq, at: time + ms, fn}); return seq; }, clearTimeout: id => { timers = timers.filter(t => t.id !== id); },
    requestAnimationFrame: fn => { frames.push({id: ++seq, fn}); return seq; }, cancelAnimationFrame: id => { frames = frames.filter(f => f.id !== id); }};
  listen(window);
  const document = {activeElement: null, hidden: false, createElement: tag => new Element(document, {tag}), createTextNode: text => ({textContent: text})};
  document.body = new Element(document, {tag: 'body', box: rect(0, 0, 1200, 800)});
  const el = (id, box, options = {}) => new (options.dialog ? Dialog : Element)(document, {id, box, parent: options.parent ?? document.body, tag: options.tag});
  const layer = el('tipLayer', null, {dialog: true}), card = el('tipCard', null, {parent: layer});
  card.offsetWidth = 320; card.offsetHeight = 140;
  const panel = el('filterPanel', rect(70, 62, 334, 790));
  const viewport = el('logsViewport', rect(80, 120, 1188, 500));
  const rows = [0, 1, 2, 3].map(i => {
    const row = el('', rect(80, 120 + 28 * i, 1188, 148 + 28 * i), {parent: el('logsRows', null)});
    row.classList.add('log-row'); row.children = [el('', rect(80, 120 + 28 * i, 132, 148 + 28 * i), {parent: row})]; return row;
  });
  const elements = {tipLayer: layer, tipCard: card, filterPanel: panel, logsViewport: viewport,
    tipSpot: el('tipSpot', null, {parent: layer}), tipTitle: el('tipTitle', null, {parent: card}), tipBody: el('tipBody', null, {parent: card}), tipStep: el('tipStep', null, {parent: card}),
    tipBack: el('tipBack', null, {parent: card, tag: 'button'}), tipNext: el('tipNext', null, {parent: card, tag: 'button'}), tipOff: el('tipOff', null, {parent: card, tag: 'button'}), tipClose: el('tipClose', null, {parent: card, tag: 'button'}),
    tipsEnabled: el('tipsEnabled', null, {tag: 'input'}), startTour: el('startTour', null, {tag: 'button'}), shortcutDialog: el('shortcutDialog', null, {dialog: true}),
    rowMenu: el('rowMenu', null), dropHint: el('dropHint', null),
    shortcutHelp: el('shortcutHelp', rect(1150, 9, 1184, 43), {tag: 'button'}), openFile: el('openFile', rect(1000, 9, 1140, 43), {tag: 'button'}), reopenFile: el('reopenFile', rect(900, 9, 990, 43), {tag: 'button'}),
    toggleFilters: el('toggleFilters', rect(22, 62, 70, 140), {tag: 'button'}), levelFilters: el('levelFilters', rect(86, 120, 320, 260), {parent: panel}),
    savedFilterButton: el('savedFilterButton', rect(180, 68, 280, 92), {tag: 'button'}),
    logsHeaders: el('logsHeaders', rect(80, 94, 1188, 120)), largeSearchTools: el('largeSearchTools', rect(80, 510, 1188, 556)),
    largeSearchPane: el('largeSearchPane', rect(80, 556, 1188, 790)), bookmarkTab: el('bookmarkTab', rect(90, 515, 150, 545), {tag: 'button'})};
  elements.rowMenu.hidden = elements.dropHint.hidden = true; elements.tipsEnabled.checked = true;
  elements.reopenFile.hidden = true;
  if (!file) for (const id of ['logsViewport', 'logsHeaders', 'largeSearchTools', 'toggleFilters', 'savedFilterButton']) elements[id].hidden = true;
  document.getElementById = id => elements[id] ?? null;
  document.querySelector = selector => ({
    'dialog[open]:not(#tipLayer)': elements.shortcutDialog.open ? elements.shortcutDialog : null,
    '#logsRows .selected-log-row': rows.find(row => row.classList.contains('selected-log-row')) ?? null,
  })[selector] ?? null;
  document.querySelectorAll = selector => selector === '#logsRows .log-row' && !viewport.hidden ? rows : [];
  const calls = {sample: 0, help: 0};
  const tips = mountTips({document, window, now: () => time, needsFile: () => !file, openSample: () => calls.sample++, closeHelp: () => { calls.help++; elements.shortcutDialog.close(); }});
  // Runs due timers in time order, then one animation frame.
  const advance = ms => {
    const until = time + ms;
    for (;;) {
      const due = timers.filter(t => t.at <= until).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      timers = timers.filter(t => t !== due); time = Math.max(time, due.at); due.fn();
    }
    time = until;
  };
  const frame = () => { const run = frames; frames = []; for (const f of run) f.fn(); };
  const stored = () => JSON.parse(store.get(KEY) ?? 'null');
  const setStored = value => store.set(KEY, JSON.stringify(value));
  return {tips, document, window, layer, card, rows, calls, advance, frame, stored, setStored, ...elements};
}

test('a tip waits for a pause, shows without taking focus, and is not offered again once seen', () => {
  const r = rig();
  r.rows[1].classList.add('selected-log-row'); r.logsViewport.focus();
  r.tips.offer('rows');
  assert.equal(r.layer.open, false, 'nothing shows before the pause');
  r.advance(IDLE_MS);
  assert.equal(r.layer.open, true); assert.equal(r.layer.modal, false); assert.equal(r.layer.dataset.tip, 'rows'); assert.equal(r.tips.mode, 'tip');
  assert.equal(r.document.activeElement, r.logsViewport, 'focus stays where the user works');
  assert.equal(r.tipTitle.textContent, '행 복사·북마크·행 메뉴');
  assert.match(r.tipBody.textContent, /다시 볼 행은 ⌘B로 북마크해 두세요/, 'it says what a bookmark is for');
  assert.equal(r.tipBody.children.length, 3, 'one paragraph each for copying, bookmarks and the row menu');
  assert.ok(r.tipBody.children[0].children.some(node => node.tagName === 'KBD' && node.textContent === '⌘C'));
  assert.equal(r.tipNext.textContent, '확인'); assert.equal(r.tipBack.hidden, true); assert.equal(r.tipOff.hidden, false); assert.equal(r.tipStep.hidden, true);
  assert.equal(r.tipBody.getAttribute('aria-live'), 'polite');
  // The lit area is the row inside its list, padded by 4px; the card sits below it.
  assert.deepEqual(r.tipSpot.style.values, {left: '76px', top: '144px', width: '1116px', height: '36px'});
  assert.equal(r.card.dataset.side, 'bottom'); assert.equal(r.card.style.values.top, '192px');
  r.tipNext.click();
  assert.equal(r.layer.open, false); assert.deepEqual(r.stored(), {off: false, seen: ['rows']});
  r.tips.offer('rows'); r.advance(IDLE_MS);
  assert.equal(r.layer.open, false, 'a seen tip is not offered again');
});

test('input postpones a tip, a long busy stretch drops it, and the next tip needs only a pause', () => {
  const r = rig();
  r.tips.offer('search'); r.advance(500); r.window.fire('keydown', {key: 'a'});
  r.advance(IDLE_MS - 1); assert.equal(r.layer.open, false, 'typing postpones it');
  r.advance(1); assert.equal(r.layer.dataset.tip, 'search');
  assert.equal(r.card.dataset.side, 'top', 'the search tip rises above the results');
  r.window.fire('keydown', {key: 'Escape'});
  r.tips.offer('levels'); r.advance(IDLE_MS);
  assert.equal(r.layer.dataset.tip, 'levels', 'a tip right after another closes waits only for a pause');
  r.window.fire('keydown', {key: 'Escape'});
  r.tips.offer('bookmarks');
  for (let t = 0; t < GIVE_UP_MS + 1000; t += 300) { r.window.fire('pointerdown', {target: r.document.body}); r.advance(300); }
  assert.equal(r.layer.open, false, 'a user who never pauses is not interrupted');
  r.tips.offer('bookmarks'); r.advance(IDLE_MS);
  assert.equal(r.layer.dataset.tip, 'bookmarks', 'the trigger offers it again later');
});

test('Escape, a press outside or a text field dismiss a tip; its own card keeps presses to itself', () => {
  const r = rig();
  const show = id => { r.tips.offer(id); r.advance(IDLE_MS); assert.equal(r.layer.dataset.tip, id); assert.equal(r.layer.open, true); };
  show('search');
  assert.deepEqual(r.window.fire('keydown', {key: 'Escape'}), {prevented: true, stopped: true}, 'Escape closes the tip and nothing else');
  assert.equal(r.layer.open, false); assert.deepEqual(r.stored().seen, ['search']);
  assert.deepEqual(r.window.fire('keydown', {key: 'Escape'}), {prevented: false, stopped: false}, 'without a tip Escape is the page\'s');
  show('bookmarks');
  r.window.fire('pointerdown', {target: r.tipNext});
  assert.equal(r.layer.open, true, 'a press on the card is not a press outside');
  assert.equal(r.card.fire('pointerdown').stopped, true, 'the auto-hiding filter panel does not see it');
  assert.equal(r.card.fire('mousedown').prevented, true, 'and focus stays in the page');
  r.window.fire('pointerdown', {target: r.rows[0]});
  assert.equal(r.layer.open, false); assert.deepEqual(r.stored().seen, ['search', 'bookmarks']);
  show('help');
  r.window.fire('focusin', {target: r.document.createElement('button')});
  assert.equal(r.layer.open, true, 'focus moving to a button keeps it');
  const field = r.document.createElement('input'); field.closest = selector => selector.includes('input') ? field : null;
  r.window.fire('focusin', {target: field});
  assert.equal(r.layer.open, false, 'starting to type somewhere else dismisses it');
});

test('a tip closes unseen when its control leaves the screen or another dialog opens', () => {
  const r = rig();
  r.rows[3].classList.add('selected-log-row');
  r.tips.offer('rows'); r.advance(IDLE_MS); assert.equal(r.layer.open, true);
  r.rows[3].classList.remove('selected-log-row'); r.rows[2].classList.add('selected-log-row');
  r.frame();
  assert.equal(r.tipSpot.style.values.top, '172px', 'it follows the selected row');
  r.rows[2].box = rect(80, 490, 1188, 518); r.frame();
  assert.equal(r.layer.open, false, 'a row scrolled out of the list closes it'); assert.equal(r.stored(), null);
  r.tips.offer('search'); r.advance(IDLE_MS); assert.equal(r.layer.open, true);
  r.shortcutDialog.open = true; r.frame();
  assert.equal(r.layer.open, false); assert.equal(r.stored(), null);
  r.tips.offer('search'); r.advance(IDLE_MS);
  assert.equal(r.layer.open, false, 'nothing shows over a dialog');
  r.shortcutDialog.open = false; r.filterPanel.inert = true;
  r.tips.offer('levels'); r.advance(IDLE_MS);
  assert.equal(r.layer.open, false, 'nor for a control in a closed panel');
});

test('a row click that closes the first tip, or comes while it waits, gets the row tip at once', () => {
  const r = rig();
  r.rows[1].classList.add('selected-log-row');
  r.tips.offer('help'); r.advance(IDLE_MS); assert.equal(r.layer.dataset.tip, 'help');
  r.window.fire('pointerdown', {target: r.rows[1]}); r.tips.offer('rows');
  assert.equal(r.layer.open, false); assert.deepEqual(r.stored().seen, ['help']);
  r.advance(IDLE_MS); assert.equal(r.layer.dataset.tip, 'rows', 'the click that closed the first tip shows its own');
  const late = rig();
  late.rows[1].classList.add('selected-log-row');
  late.tips.offer('help'); late.advance(IDLE_MS - 100);
  late.window.fire('pointerdown', {target: late.rows[1]}); late.tips.offer('rows'); late.tips.offer('help');
  late.advance(IDLE_MS); assert.equal(late.layer.dataset.tip, 'rows', 'a tip for what was just done takes the waiting first tip\'s place');
  late.tipNext.click(); late.tips.offer('help'); late.advance(IDLE_MS);
  assert.equal(late.layer.dataset.tip, 'help', 'and the first tip comes back afterwards');
  const busy = rig();
  busy.tips.offer('search'); busy.tips.offer('rows'); busy.advance(IDLE_MS);
  assert.equal(busy.layer.dataset.tip, 'search', 'one action tip does not replace another');
});

test('the level tip holds the auto-hiding filter panel open while it shows', () => {
  const r = rig(), holds = [];
  r.tips.offer('levels', {hold: value => holds.push(value)}); r.advance(IDLE_MS);
  assert.deepEqual(holds, [true]);
  assert.equal(r.card.dataset.side, 'right', 'the card stands beside the panel, not over its fields');
  r.tipClose.click();
  assert.deepEqual(holds, [true, false]); assert.deepEqual(r.stored().seen, ['levels']);
});

test('tips turned off from a tip, the shortcut list or elsewhere stay off', () => {
  const r = rig();
  r.tips.offer('search'); r.advance(IDLE_MS);
  r.tipOff.click();
  assert.equal(r.layer.open, false); assert.deepEqual(r.stored(), {off: true, seen: ['search']}); assert.equal(r.tipsEnabled.checked, false);
  r.tips.offer('bookmarks'); r.advance(IDLE_MS); assert.equal(r.layer.open, false);
  r.tipsEnabled.checked = true; r.tipsEnabled.fire('change');
  assert.deepEqual(r.stored(), {off: false, seen: ['search']});
  r.tips.offer('bookmarks');
  r.setStored({off: true, seen: ['search']});
  r.advance(IDLE_MS);
  assert.equal(r.layer.open, false, 'the setting is read again before a tip shows'); assert.equal(r.tipsEnabled.checked, false);
});

test('the first tip starts the tour: modal, without missing controls, by buttons and arrow keys', () => {
  const r = rig();
  r.logsViewport.focus();
  r.tips.offer('help'); r.advance(IDLE_MS);
  assert.equal(r.tipNext.textContent, '둘러보기'); assert.equal(r.tipBack.textContent, '닫기');
  r.tipNext.click();
  assert.equal(r.tips.mode, 'tour'); assert.equal(r.layer.modal, true); assert.equal(r.layer.dataset.tip, 'tour');
  assert.deepEqual(r.stored().seen, ['help']);
  const steps = TOUR.length - 1;
  assert.equal(r.tipStep.textContent, `1 / ${steps}`, 'the hidden reopen button is left out'); assert.equal(r.tipStep.hidden, false);
  assert.equal(r.tipTitle.textContent, '로그 열기'); assert.equal(r.tipBack.hidden, true); assert.equal(r.tipOff.hidden, true);
  assert.equal(r.document.activeElement, r.tipNext);
  assert.equal(r.layer.fire('keydown', {key: 'ArrowRight'}).stopped, true);
  assert.equal(r.tipTitle.textContent, '필터'); assert.equal(r.card.dataset.side, 'right');
  assert.deepEqual(r.layer.fire('keydown', {key: '?'}), {prevented: false, stopped: true}, 'page shortcuts wait until the tour ends');
  r.layer.fire('keydown', {key: 'ArrowRight'});
  assert.equal(r.tipTitle.textContent, '저장된 필터'); assert.equal(r.card.dataset.side, 'bottom');
  r.layer.fire('keydown', {key: 'ArrowRight'});
  assert.equal(r.tipTitle.textContent, '행 선택과 복사'); assert.equal(r.tipSpot.style.values.top, '116px', 'the first row in view');
  r.layer.fire('keydown', {key: 'ArrowRight'});
  assert.equal(r.tipSpot.style.values.width, '60px', 'the bookmark step lights its row number');
  r.layer.fire('keydown', {key: 'ArrowLeft'}); r.tipBack.click(); r.tipBack.click();
  assert.equal(r.tipTitle.textContent, '필터');
  for (let i = 2; i < steps; i++) r.tipNext.click();
  assert.equal(r.tipNext.textContent, '마치기'); assert.equal(r.tipTitle.textContent, '단축키와 둘러보기');
  r.tipNext.click();
  assert.equal(r.layer.open, false); assert.equal(r.tips.mode, null); assert.equal(r.document.activeElement, r.logsViewport, 'focus returns');
  assert.deepEqual(r.stored().seen, ['help', 'levels', 'rows', 'bookmarks', 'search'], 'steps shown count as seeing their tips');
  r.tips.offer('rows'); r.advance(IDLE_MS); assert.equal(r.layer.open, false, 'so the tour is not repeated by tips');
});

test('from the shortcut list without a file, the tour opens the sample log and starts on its rows', () => {
  const r = rig({file: false});
  r.shortcutDialog.showModal(); r.startTour.focus();
  r.startTour.click();
  assert.equal(r.calls.help, 1); assert.equal(r.shortcutDialog.open, false); assert.equal(r.calls.sample, 1);
  assert.equal(r.layer.modal, true); assert.equal(r.tipTitle.textContent, '샘플 로그를 여는 중…');
  assert.equal(r.layer.classList.contains('no-spot'), true); assert.equal(r.card.dataset.side, 'center'); assert.equal(r.tipNext.hidden, true);
  r.advance(500); assert.equal(r.tipTitle.textContent, '샘플 로그를 여는 중…', 'it waits for rows');
  for (const id of ['logsViewport', 'logsHeaders', 'largeSearchTools', 'toggleFilters']) r[id].hidden = false;
  r.advance(100);
  assert.equal(r.tipTitle.textContent, '로그 열기'); assert.equal(r.layer.classList.contains('no-spot'), false);
  r.window.fire('keydown', {key: 'Escape'});
  assert.equal(r.layer.open, false); assert.equal(r.document.activeElement, r.logsViewport, 'the hidden opener gives way to the logs');
  assert.equal(r.layer.fire('cancel').prevented, true);
});

test('the card takes the first side that fits and otherwise covers the lit area', () => {
  const view = {width: 1200, height: 800}, card = {width: 320, height: 140};
  const corner = placeCard(rect(1146, 5, 1188, 47), card, view);
  assert.deepEqual(corner, {side: 'bottom', left: 872, top: 59, arrow: 295}, 'clamped to the right edge with the pointer under the button');
  assert.equal(placeCard(rect(80, 650, 1188, 790), card, view).side, 'top');
  assert.deepEqual(placeCard(rect(18, 58, 74, 144), card, view, ['right', 'bottom']), {side: 'right', left: 86, top: 58, arrow: 43}, 'beside a small control the card hangs from its top');
  assert.deepEqual(placeCard(rect(80, 100, 300, 700), card, view, ['right']), {side: 'right', left: 312, top: 100, arrow: 70}, 'beside a tall one the pointer stays by the card');
  const whole = placeCard(rect(0, 0, 1200, 800), card, view);
  assert.deepEqual(whole, {side: 'inside', left: 440, top: 648, arrow: null});
  assert.equal(placeCard(rect(500, 100, 600, 700), card, {width: 1200, height: 800}, ['left']).side, 'left');
});

test('tip text renders keys as kbd for the platform, and stored settings are checked', () => {
  const document = {createElement: tag => ({tagName: tag.toUpperCase(), textContent: ''}), createTextNode: text => ({textContent: text})};
  const nodes = richText(document, '{⌘C}로 복사하고 {Shift}+클릭', 'Ctrl+');
  assert.deepEqual(nodes.map(node => [node.tagName ?? '#text', node.textContent]), [['KBD', 'Ctrl+C'], ['#text', '로 복사하고 '], ['KBD', 'Shift'], ['#text', '+클릭']]);
  // ⌘ alone reads like Shift before +클릭: Ctrl off Apple platforms, ⌘ on them.
  const show = mod => richText(document, '{⌘}+클릭', mod).map(node => node.textContent).join('|');
  assert.equal(show('Ctrl+'), 'Ctrl|+클릭');assert.equal(show('⌘'), '⌘|+클릭');
  const storage = value => ({getItem: () => value});
  assert.deepEqual(readTipState(storage('{bad')), {off: false, seen: new Set()});
  assert.deepEqual(readTipState(storage(JSON.stringify({off: 'yes', seen: ['rows', 3, null]}))), {off: false, seen: new Set(['rows'])});
  assert.deepEqual(readTipState({getItem() { throw new Error('denied'); }}), {off: false, seen: new Set()});
});
