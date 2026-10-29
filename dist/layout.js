// Layout preferences never require log queries or whole-file measurements.
import {PROFILES} from './columns.js';
// Text column widths stay at the top level of the saved layout, as before
// DLT columns existed; other profiles keep theirs under their name.
const STORAGE_KEY = 'logstudio.layout.v1';
const MESSAGE_MIN = 160;
// The two splitters' values, neither following the length of the row the
// raw detail shows: the log area's share for search results or bookmarks,
// and the lines the raw detail shows, one by default. Keys step by the
// first step, or the second with Shift.
const LOWER = {fallback: 0.42, min: 0.15, max: 0.8, steps: [0.03, 0.1], round: value => Math.round(value * 1000) / 1000};
const DETAIL = {fallback: 1, min: 1, max: 100, steps: [1, 5], round: Math.round};
const clampTo = (value, {min, max, round}) => round(Math.max(min, Math.min(max, value)));
const savedTo = (value, limits) => Number.isFinite(value) ? clampTo(value, limits) : limits.fallback;
const clampWidth = (value, index, p) => Math.round(Math.max(p.min[index], Math.min(p.max[index], value)));
// The widest row number a log can show: its row count so far, or in a
// combined log, whose rows are numbered per file, the most rows of one file.
export function widestLine({total = 0, files = []} = {}) {
  if (!(files?.length > 1)) return total;
  let widest = 0;
  for (let k = 0; k < files.length && Number.isSafeInteger(files[k]?.firstLine); k++) {
    const next = files[k + 1]?.firstLine;
    widest = Math.max(widest, (Number.isSafeInteger(next) ? next : total + 1) - files[k].firstLine);
  }
  return widest;
}

export function mountLogLayout({document = globalThis.document, window = globalThis.window, onPanelChange = () => {}} = {}) {
  const $ = id => document.getElementById(id);
  const workspace = $('workspace'), panel = $('filterPanel'), toggle = $('toggleFilters');
  // Controller-only environments need no browser layout.
  if (!workspace?.style || !panel || !toggle) return {setRawMode() {}, setMultiFile() {}, setProfile() {}, fitColumns() {}, resetColumns() {}, fitTimestamp() {}, fitLines() {}, setAvailable() { return false; }, closePanel() { return false; }, hold() {}, fitDetail() {}};
  const autoHideInput = $('autoHideFilters');
  const headers = [$('logsHeaders'), $('largeHeaders'), $('bookmarkHeaders')];
  const tables = [$('logsTable'), $('largeTable'), $('bookmarkTable')];
  let saved;
  try { saved = JSON.parse(window.localStorage.getItem(STORAGE_KEY)); } catch {}
  function load(name) {
    const p = PROFILES[name], own = name === 'text' ? saved : saved?.[name];
    const widths = p.defaults.map((width, index) => Number.isFinite(own?.widths?.[index]) ? clampWidth(own.widths[index], index, p) : width);
    // Widths the user chose; the others may follow the content. Older saves
    // have no flags, so a width other than the default counts as chosen.
    const custom = p.defaults.map((width, index) => Array.isArray(own?.custom) ? own.custom[index] === true : widths[index] !== width);
    // An automatic row column width belonged to the last file shown.
    if (!custom[0]) widths[0] = p.defaults[0];
    return {widths, custom};
  }
  const columns = {text: load('text'), dlt: load('dlt')};
  let profile = 'text', {widths, custom} = columns.text;
  const P = () => PROFILES[profile];
  let autoHide = typeof saved?.autoHide === 'boolean' ? saved.autoHide : true;
  let lowerRatio = savedTo(saved?.lowerRatio, LOWER), detailLines = savedTo(saved?.detailLines, DETAIL);
  let open = !autoHide && saved?.open !== false, available = true, rawMode = false, multiFile = false, hideTimer = null, drag = null, timestampFor = null;
  // Whether a press outside the floating panel is down, and whether its
  // close is waiting (closeAfterPress).
  let pressing = false, settling = false;
  // While a tip points into the panel, auto-hide leaves it open.
  let held = false;
  // A combined log adds a file badge to the row column without changing the
  // saved width.
  const FILE_BADGE = 24;
  const lineWidth = () => widths[0] + (multiFile ? FILE_BADGE : 0);
  function persist() {
    columns[profile] = {widths, custom};
    try { window.localStorage.setItem(STORAGE_KEY, JSON.stringify({autoHide, open: !autoHide && open, widths: columns.text.widths, custom: columns.text.custom, dlt: columns.dlt, lowerRatio, detailLines})); } catch {}
  }
  function cancelHide() {
    if (hideTimer !== null) window.clearTimeout(hideTimer);
    hideTimer = null;
    settling = false;
  }
  // Without an open file the filters have nothing to act on: the rail is
  // disabled and the panel shown closed, keeping the saved open preference.
  const shown = () => open && available;
  function renderPanel() {
    // Keep the panel in its clipped grid slot while its track collapses.
    // Inert/ARIA remove closed controls immediately from keyboard navigation.
    panel.hidden = false;
    panel.inert = !shown();
    panel.setAttribute('aria-hidden', String(!shown()));
    workspace.classList.toggle('filters-collapsed', !shown());
    workspace.classList.toggle('filters-auto-hide', autoHide);
    autoHideInput.checked = autoHide;
    toggle.disabled = !available;
    toggle.setAttribute('aria-expanded', String(shown()));
    toggle.title = !available ? '로그 파일을 열면 필터를 사용할 수 있습니다' : open ? '필터 접기' : '필터 펼치기';
    // Field errors move between the panel and the banner as it opens/closes.
    onPanelChange(shown());
  }
  function setOpen(value, restoreFocus = false) {
    cancelHide(); open = value; renderPanel(); persist();
    if (restoreFocus) toggle.focus({preventScroll: true});
  }
  function scheduleHide() {
    cancelHide();
    if (!autoHide || !shown() || held || pressing) return;
    hideTimer = window.setTimeout(() => {
      hideTimer = null;
      // Typing, keyboard navigation and an open select must not be interrupted.
      if (!panel.contains(document.activeElement) && !panel.matches(':hover') && !toggle.matches(':hover')) setOpen(false);
    }, 350);
  }
  toggle.addEventListener('click', () => { if (available) setOpen(!shown()); });
  $('collapseFilters').addEventListener('click', () => setOpen(false, true));
  panel.addEventListener('pointerenter', cancelHide);
  panel.addEventListener('pointerleave', scheduleHide);
  panel.addEventListener('focusin', cancelHide);
  panel.addEventListener('focusout', scheduleHide);
  autoHideInput.addEventListener('change', () => {
    cancelHide(); autoHide = autoHideInput.checked; renderPanel(); persist();
  });
  // A primary press outside closes the floating panel once the press, and
  // a double-click it may begin, are over: closing moves the log area, which
  // must not slide under the pointer. The wait is the usual double-click
  // interval on macOS and Windows. Focus the press moves out waits too. A
  // release that lands in the panel keeps it, and a context menu, whose
  // release may never come, ends the press.
  const PRESS_SETTLE_MS = 500;
  function closeAfterPress() {
    cancelHide();
    if (pressing) return;
    pressing = true;
    const ends = ['pointerup', 'pointercancel', 'contextmenu'];
    const done = event => {
      for (const name of ends) window.removeEventListener(name, done, true);
      window.removeEventListener('blur', done);
      pressing = false;
      cancelHide();
      if (event?.type === 'pointerup' && panel.contains(event.target)) return;
      settling = true;
      hideTimer = window.setTimeout(() => {
        hideTimer = null; settling = false;
        if (autoHide && shown() && !held && !panel.contains(document.activeElement)) setOpen(false);
      }, PRESS_SETTLE_MS);
    };
    for (const name of ends) window.addEventListener(name, done, true);
    window.addEventListener('blur', done);
  }
  document.addEventListener('pointerdown', event => {
    if (!autoHide || !shown() || held || panel.contains(event.target) || toggle.contains(event.target)) return;
    // Other buttons close it at once, as before: no double-click follows.
    if ((event.button ?? 0) !== 0) setOpen(false);
    else closeAfterPress();
  });
  document.addEventListener('focusin', event => {
    if (autoHide && shown() && !held && !pressing && !settling && !panel.contains(event.target) && !toggle.contains(event.target)) setOpen(false);
  });
  // One Escape closes one thing. Popup menus and drags claim it first; the
  // panel takes it while focus is inside it or while it floats (auto-hide).
  // A pinned panel leaves it to the raw detail and closes via closePanel().
  document.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented || !shown()) return;
    const inside = panel.contains(document.activeElement);
    if (inside || autoHide) { event.preventDefault(); setOpen(false, inside); }
  });
  // Short field help stays collapsed so every input fits; aria-describedby
  // keeps pointing at the hidden note.
  for (const help of panel.querySelectorAll?.('.help-toggle') ?? []) help.addEventListener('click', () => {
    const note = $(help.getAttribute('aria-controls')), expanded = help.getAttribute('aria-expanded') !== 'true';
    help.setAttribute('aria-expanded', String(expanded)); if (note) note.hidden = !expanded;
  });

  function applyWidths() {
    workspace.style.setProperty('--columns', `${[lineWidth(), ...widths.slice(1)].map(width => `${width}px`).join(' ')} minmax(${MESSAGE_MIN}px,1fr)`);
    workspace.style.setProperty('--line-column', `${lineWidth()}px`);
    workspace.style.setProperty('--table-min-width', `${widths.reduce((sum, width) => sum + width, MESSAGE_MIN + lineWidth() - widths[0])}px`);
    for (const header of headers) for (const handle of header.querySelectorAll('.column-resizer')) {
      handle.setAttribute('aria-valuenow', String(widths[Number(handle.dataset.column)]));
      handle.setAttribute('aria-valuetext', `${widths[Number(handle.dataset.column)]} 픽셀`);
    }
  }
  function resize(index, width, chosen = true) { widths[index] = clampWidth(width, index, P()); if (chosen) custom[index] = true; applyWidths(); }
  let measure;
  // Widest text of a column among the rendered rows, or null without any.
  function measureColumn(index, rows, textOf = cell => cell.textContent) {
    measure ??= document.createElement('canvas').getContext('2d');
    if (!measure) return null;
    let width = null;
    for (const row of rows) {
      const cell = row.children[index];
      if (!cell) continue;
      const style = window.getComputedStyle(cell);
      measure.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
      const padding = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
      // Level badges leave six pixels on each side inside their grid track.
      width = Math.max(width ?? 0, Math.ceil(measure.measureText(textOf(cell)).width + padding + (index === P().level ? 12 : 4)));
    }
    return width;
  }
  // The row column follows the widest row number of the shown log unless the
  // user chose its width, never narrower than the default. It is measured
  // again only when the digit count changes; rows not yet shown would
  // otherwise be cut off once scrolled to.
  let lineDigits = 1, lineFit = 0;
  function fitLines(widest) {
    if (widest !== undefined) lineDigits = String(Math.max(1, Math.floor(widest) || 0)).length;
    if (custom[0] || lineFit === lineDigits) return;
    // Any rendered row gives the number font; without one a later paint fits.
    const cell = tables.map(table => table.querySelectorAll('.log-row')[0]?.children[0]).find(Boolean);
    measure ??= document.createElement('canvas').getContext('2d');
    if (!cell || !measure) return;
    const style = window.getComputedStyle(cell);
    // Bookmarked rows draw the number in bold.
    measure.font = `700 ${style.fontSize} ${style.fontFamily}`;
    const digit = Math.max(...'0123456789'.split('').map(d => measure.measureText(d).width));
    const width = Math.ceil(digit * lineDigits + parseFloat(style.paddingLeft) + parseFloat(style.paddingRight) + 2);
    lineFit = lineDigits;
    const next = clampWidth(Math.max(P().defaults[0], Math.min(P().fit[0], width)), 0, P());
    if (next !== widths[0]) resize(0, next, false);
  }
  function autoLines() { custom[0] = false; lineFit = 0; resize(0, P().defaults[0], false); fitLines(); }
  function fitColumn(index) {
    // Fitting the row column to the visible rows would cut off later ones.
    if (index === 0) { autoLines(); return; }
    // Panes are bounded (200 rows, 1,000 bookmarks); hidden tables do not affect fit.
    const rows = tables.filter(table => table.getClientRects().length).flatMap(table => [...table.querySelectorAll('.table-header, .log-row:not(.raw-row)')]);
    // The file badge's track is added separately from the row number.
    const width = measureColumn(index, rows, cell => index === 0 ? cell.querySelector?.('.line-text')?.textContent ?? cell.textContent : cell.textContent);
    if (width !== null || measure) resize(index, Math.min(P().fit[index], Math.max(P().min[index], width ?? 0)));
  }
  // A newly opened file fits the date/time column once to the timestamps of
  // its first rendered upper rows (iOS microseconds, Android nanoseconds),
  // never narrower than the default and never over a width the user chose.
  // A process name (iOS) or package (Android Studio) after the time stays
  // truncated with its tooltip; the width leaves room for that cell's
  // ellipsis, which would otherwise hide the last digits of the time.
  function fitTimestamp(datasetId) {
    if (datasetId === null || datasetId === undefined || datasetId === timestampFor || rawMode) return;
    if (custom[1]) { timestampFor = datasetId; return; }
    const rows = [...tables[0].querySelectorAll('.log-row:not(.raw-row):not(.continuation-row)')].filter(row => /\btimestamp\b/.test(row.children[1]?.className ?? ''));
    // An iOS row keeps room for a process of 16 characters, as names of
    // rows to come may be longer than the first; others show a hint.
    const width = measureColumn(1, rows, cell => {
      const [time, process] = cell.textContent.split(' · ');
      if (process === undefined) return time;
      return cell.classList?.contains('apple-stamp') ? `${time} · ${'0'.repeat(16)}` : `${time} …`;
    });
    // Nothing parsed is on screen yet; a later paint tries again.
    if (width === null) return;
    timestampFor = datasetId;
    const next = clampWidth(Math.max(P().defaults[1], Math.min(P().fit[1], width)), 1, P());
    if (next !== widths[1]) { resize(1, next, false); persist(); }
  }
  function fitColumns() {
    for (let index = 0; index < widths.length; index++) fitColumn(index);
    persist();
  }
  function refitTimestamp() { const id = timestampFor; timestampFor = null; fitTimestamp(id); }
  // Defaults include the date/time fit of the current file.
  function resetColumns() {
    widths = [...P().defaults]; custom = custom.map(() => false); lineFit = 0; applyWidths(); fitLines(); refitTimestamp(); persist();
  }
  function finishDrag(event, cancelled = false) {
    if (!drag || (event && event.pointerId !== drag.pointerId)) return;
    const current = drag; drag = null;
    workspace.classList.remove('resizing-columns');
    if (cancelled) { resize(current.index, current.width, false); custom[current.index] = current.custom; }
    if (current.handle.hasPointerCapture(current.pointerId)) current.handle.releasePointerCapture(current.pointerId);
    persist();
  }
  function renderHeaders() {
    finishDrag(null, true);
    for (const header of headers) {
      const labels = multiFile ? ['파일 · 행', ...P().labels.slice(1)] : P().labels;
      header.replaceChildren(...(rawMode ? [labels[0], '원문'] : labels).map((label, index, labels) => {
        const cell = document.createElement('span');
        cell.className = 'column-heading';
        const text = document.createElement('span'); text.textContent = label; cell.append(text);
        if (index === labels.length - 1) return cell;
        const handle = document.createElement('span');
        handle.className = 'column-resizer'; handle.dataset.column = String(index); handle.tabIndex = 0;
        handle.setAttribute('role', 'separator'); handle.setAttribute('aria-orientation', 'vertical');
        handle.setAttribute('aria-label', `열 폭: ${label}`);
        handle.setAttribute('aria-valuemin', String(P().min[index]));
        handle.setAttribute('aria-valuemax', String(P().max[index]));
        handle.title = '드래그로 폭 조절 · 더블클릭으로 내용에 맞춤 · ←/→ 키로 조절';
        handle.addEventListener('pointerdown', event => {
          if (event.button !== 0 || drag) return;
          event.preventDefault(); cancelHide();
          drag = {index, width: widths[index], custom: custom[index], x: event.clientX, pointerId: event.pointerId, handle};
          handle.setPointerCapture(event.pointerId); handle.focus({preventScroll: true});
          workspace.classList.add('resizing-columns');
        });
        handle.addEventListener('pointermove', event => {
          if (drag?.handle === handle && drag.pointerId === event.pointerId) resize(index, drag.width + event.clientX - drag.x);
        });
        handle.addEventListener('pointerup', event => finishDrag(event));
        handle.addEventListener('pointercancel', event => finishDrag(event, true));
        handle.addEventListener('lostpointercapture', event => finishDrag(event));
        handle.addEventListener('dblclick', () => { fitColumn(index); persist(); });
        handle.addEventListener('keydown', event => {
          if (event.key === 'Escape' && drag) { event.preventDefault(); finishDrag(null, true); return; }
          if (!['ArrowLeft', 'ArrowRight', 'Home', 'Enter'].includes(event.key)) return;
          event.preventDefault();
          if (event.key === 'Enter') fitColumn(index);
          else if (event.key === 'Home') { if (index === 0) autoLines(); else { resize(index, P().defaults[index], false); custom[index] = false; } if (index === 1) refitTimestamp(); }
          else resize(index, widths[index] + (event.key === 'ArrowLeft' ? -1 : 1) * (event.shiftKey ? 20 : 8));
          persist();
        });
        cell.append(handle); return cell;
      }));
    }
    applyWidths();
  }
  // A splitter resizes the region below it; dragging up grows it. Drags and
  // keys start from the shown size, which other regions' minimum heights can
  // make differ from the saved one, and a size those minimums leave no room
  // for is not kept: it would show later, all at once, when they let go.
  // `unit` is the pixels of one step of the value. A double-click or Enter
  // takes the value that shows everything, `fill`, where there is one (as a
  // column edge fits its column), or else the default.
  function mountSplitter(splitter, {limits, get, set, shown, unit, fill = null}) {
    let moving = null;
    function fit(value) {
      set(value);
      const room = shown();
      if (room < get() - limits.steps[0] / 2) set(room);
    }
    function finish(event, cancelled = false) {
      if (!moving || (event && event.pointerId !== moving.pointerId)) return;
      const current = moving; moving = null;
      workspace.classList.remove('resizing-panes'); splitter.classList.remove('dragging');
      if (cancelled) set(current.value);
      if (splitter.hasPointerCapture?.(current.pointerId)) splitter.releasePointerCapture(current.pointerId);
      persist();
    }
    splitter.addEventListener('pointerdown', event => {
      if (event.button !== 0 || moving) return;
      event.preventDefault();
      moving = {value: get(), base: shown(), unit: Math.max(1, unit() || 1), y: event.clientY, pointerId: event.pointerId};
      splitter.setPointerCapture?.(event.pointerId); splitter.focus({preventScroll: true});
      workspace.classList.add('resizing-panes'); splitter.classList.add('dragging');
    });
    splitter.addEventListener('pointermove', event => {
      if (moving?.pointerId === event.pointerId) fit(moving.base - (event.clientY - moving.y) / moving.unit);
    });
    splitter.addEventListener('pointerup', event => finish(event));
    splitter.addEventListener('pointercancel', event => finish(event, true));
    splitter.addEventListener('lostpointercapture', event => finish(event));
    function fitAll() { if (fill) fit(fill()); else set(limits.fallback); persist(); }
    splitter.addEventListener('dblclick', fitAll);
    splitter.addEventListener('keydown', event => {
      if (event.key === 'Escape' && moving) { event.preventDefault(); finish(null, true); return; }
      if (event.key === 'Enter' && fill) { event.preventDefault(); fitAll(); return; }
      if (!['ArrowUp', 'ArrowDown', 'Home'].includes(event.key)) return;
      const step = limits.steps[event.shiftKey ? 1 : 0], from = shown();
      event.preventDefault(); fit({ArrowUp: from + step, ArrowDown: from - step, Home: limits.fallback}[event.key]); persist();
    });
    return {cancel: () => finish(null, true), fitAll};
  }
  const paneSplitter = $('paneSplitter'), content = $('logContent'), lowerPane = $('largeLowerPane');
  const detailSplitter = $('detailSplitter'), detailText = $('largeDetailText');
  const height = element => element?.getBoundingClientRect?.().height;
  function setLower(value) {
    lowerRatio = clampTo(value, LOWER);
    workspace.style.setProperty('--lower-ratio', String(lowerRatio));
    const percent = Math.round(lowerRatio * 100);
    paneSplitter.setAttribute('aria-valuenow', String(percent));
    paneSplitter.setAttribute('aria-valuetext', `하단 영역 ${percent}%`);
  }
  // A detail shows whole lines unless other regions' minimums leave less room.
  const lineHeight = () => parseFloat(window.getComputedStyle?.(detailText)?.lineHeight);
  function setDetail(value) {
    detailLines = clampTo(value, DETAIL);
    workspace.style.setProperty('--detail-lines', String(detailLines));
    detailSplitter.setAttribute('aria-valuenow', String(detailLines));
    detailSplitter.setAttribute('aria-valuetext', `원문 상세 ${detailLines}줄`);
  }
  paneSplitter.setAttribute('aria-valuemin', String(LOWER.min * 100));
  paneSplitter.setAttribute('aria-valuemax', String(LOWER.max * 100));
  detailSplitter.setAttribute('aria-valuemin', String(DETAIL.min));
  detailSplitter.setAttribute('aria-valuemax', String(DETAIL.max));
  const splits = [
    mountSplitter(paneSplitter, {limits: LOWER, get: () => lowerRatio, set: setLower, unit: () => height(content),
      shown: () => { const total = height(content), shown = height(lowerPane); return total > 0 && shown > 0 ? shown / total : lowerRatio; }}),
    // Filling the detail shows all of its current text, once: later rows
    // keep that many lines. The text's inline box spans its lines but
    // leaves out the leading of the last, so it measures a fraction short;
    // its scroll height never measures less than the box around it.
    mountSplitter(detailSplitter, {limits: DETAIL, get: () => detailLines, set: setDetail, unit: lineHeight,
      shown: () => { const line = lineHeight(), shown = height(detailText); return line > 0 && shown > 0 ? shown / line : detailLines; },
      fill: () => { const all = Math.ceil((height(detailText.firstElementChild) ?? detailText.scrollHeight) / lineHeight() - 0.05); return all > 0 ? all : detailLines; }}),
  ];
  setLower(lowerRatio); setDetail(detailLines);
  // A blur ends a press outside (closeAfterPress), whose close still comes.
  function cancelDrags() { finishDrag(null, true); for (const split of splits) split.cancel(); if (!pressing && !settling) cancelHide(); }
  window.addEventListener('blur', cancelDrags);
  window.addEventListener('pagehide', cancelDrags, {once: true});
  renderPanel(); renderHeaders();
  return {setAvailable(value) {
    // Returns whether focus was in the controls that just became unavailable.
    value = !!value;
    if (available === value) return false;
    const active = document.activeElement, displaced = !value && (active === toggle || panel.contains(active));
    cancelHide(); available = value; renderPanel();
    if (displaced) active?.blur?.();
    return displaced;
  }, closePanel() {
    if (!shown()) return false;
    setOpen(false, panel.contains(document.activeElement)); return true;
  }, hold(value) {
    // Released, the panel stays open until the next press or focus elsewhere.
    held = !!value; if (held) cancelHide();
  }, setRawMode(value) {
    rawMode = value;
    $('columnMenu').hidden = value;
    renderHeaders();
  }, setMultiFile(value) {
    if (multiFile === !!value) return;
    multiFile = !!value; renderHeaders();
  }, setProfile(name) {
    // A DLT file shows its own columns, with widths kept apart from text logs.
    name = PROFILES[name] ? name : 'text';
    if (profile === name) return;
    columns[profile] = {widths, custom}; profile = name; ({widths, custom} = columns[name]);
    lineFit = 0; renderHeaders(); fitLines();
  }, fitDetail: splits[1].fitAll, fitColumns, resetColumns, fitTimestamp, fitLines};
}
