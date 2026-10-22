// Page zoom goes through the native webview, which scales CSS pixels as a
// whole: row heights, virtual scroll positions and saved column widths stay in
// CSS px, so no view needs to re-measure or re-query anything.
export const ZOOM_LEVELS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
const STORAGE_KEY = 'logstudio.zoom.v1';
const DEFAULT_INDEX = ZOOM_LEVELS.indexOf(1);
// A mouse notch steps once whatever delta the platform reports; a trackpad or
// a fast spin steps at most this often unless it travels a full notch.
const WHEEL_INTERVAL_MS = 150, WHEEL_IDLE_MS = 250, WHEEL_NOTCH = 100;
const INDICATOR_MS = 1200;
// A pinch follows the fingers from the level it started at; a new level
// must be this much closer (in scale ratio) than the current one, so a pinch
// held near the midpoint of two levels does not flicker between them.
const PINCH_HYSTERESIS = 0.03;
const IN_KEYS = new Set(['=', '+']), OUT_KEYS = new Set(['-', '_']);
const GESTURES = ['gesturestart', 'gesturechange', 'gestureend'];
const IN_CODES = new Set(['Equal', 'NumpadAdd']), OUT_CODES = new Set(['Minus', 'NumpadSubtract']), RESET_CODES = new Set(['Digit0', 'Numpad0']);

export function zoomIndexFor(value) {
  if (!Number.isFinite(value)) return DEFAULT_INDEX;
  let best = 0;
  for (let index = 1; index < ZOOM_LEVELS.length; index++) if (Math.abs(ZOOM_LEVELS[index] - value) < Math.abs(ZOOM_LEVELS[best] - value)) best = index;
  return best;
}

export function mountZoom({window = globalThis.window, setZoom, indicator = null} = {}) {
  if (typeof setZoom !== 'function') return null;
  let saved;
  try { saved = JSON.parse(window.localStorage.getItem(STORAGE_KEY)); } catch {}
  let index = zoomIndexFor(Number.isFinite(saved?.level) ? saved.level : 1);
  // One native call at a time; a burst of steps applies only the latest level.
  let applied = 1, running = false, hideTimer = null;
  async function apply() {
    if (running) return;
    running = true;
    try {
      while (applied !== ZOOM_LEVELS[index]) {
        const level = ZOOM_LEVELS[index];
        try { await setZoom(level); } catch { index = zoomIndexFor(applied); break; }
        applied = level;
        try { window.localStorage.setItem(STORAGE_KEY, JSON.stringify({level})); } catch {}
      }
    } finally { running = false; }
  }
  function announce() {
    if (!indicator) return;
    indicator.textContent = `화면 ${Math.round(ZOOM_LEVELS[index] * 100)}%`;
    indicator.hidden = false;
    if (hideTimer !== null) window.clearTimeout(hideTimer);
    hideTimer = window.setTimeout(() => { hideTimer = null; indicator.hidden = true; }, INDICATOR_MS);
  }
  function set(next) {
    next = Math.max(0, Math.min(ZOOM_LEVELS.length - 1, next));
    const changed = next !== index;
    index = next; announce();
    if (changed) void apply();
  }
  const step = direction => set(index + direction);
  const reset = () => set(DEFAULT_INDEX);
  // Capture runs before the log viewports, so a zoom gesture never also scrolls.
  let wheelTotal = 0, lastWheel = -Infinity, lastStep = -Infinity;
  // WebKit on macOS reports a trackpad pinch as gesture events whose scale
  // is relative to the start of the pinch; Chromium (WebView2) reports it as
  // Ctrl+wheel. Wheel zoom pauses while gesture events arrive and briefly
  // after them, so a pinch that a webview reports both ways steps only once.
  let pinch = null, lastGesture = -Infinity;
  const time = event => event.timeStamp ?? window.performance?.now?.() ?? Date.now();
  function onWheel(event) {
    if (!(event.ctrlKey || event.metaKey) || event.altKey || !event.deltaY) return;
    event.preventDefault(); event.stopPropagation();
    const now = time(event);
    if (now - lastGesture < WHEEL_IDLE_MS) return;
    const delta = event.deltaY * (event.deltaMode === 1 ? 40 : event.deltaMode === 2 ? 800 : 1);
    if (now - lastWheel > WHEEL_IDLE_MS || Math.sign(delta) !== Math.sign(wheelTotal)) wheelTotal = 0;
    lastWheel = now; wheelTotal += delta;
    if (now - lastStep < WHEEL_INTERVAL_MS && Math.abs(wheelTotal) < WHEEL_NOTCH) return;
    lastStep = now;
    // Scrolling up (negative delta) zooms in, as in browsers.
    step(wheelTotal < 0 ? 1 : -1); wheelTotal = 0;
  }
  // Preventing the gesture also keeps a webview that allows magnification
  // from scaling the page on its own.
  function onGesture(event) {
    event.preventDefault(); event.stopPropagation(); lastGesture = time(event);
    if (event.type === 'gesturestart') pinch = ZOOM_LEVELS[index];
    else if (event.type === 'gestureend') { pinch = null; return; }
    const scale = Number(event.scale);
    if (pinch === null || !(scale > 0) || !Number.isFinite(scale)) return;
    const target = pinch * scale, next = zoomIndexFor(target);
    const distance = level => Math.abs(Math.log(target / level));
    if (next !== index && distance(ZOOM_LEVELS[next]) + PINCH_HYSTERESIS < distance(ZOOM_LEVELS[index])) set(next);
  }
  function onKey(event) {
    if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
    const key = String(event.key);
    const action = IN_KEYS.has(key) || IN_CODES.has(event.code) ? 1 : OUT_KEYS.has(key) || OUT_CODES.has(event.code) ? -1 : key === '0' || RESET_CODES.has(event.code) ? 0 : null;
    if (action === null) return;
    event.preventDefault(); event.stopPropagation();
    if (action === 0) reset(); else step(action);
  }
  window.addEventListener('wheel', onWheel, {capture: true, passive: false});
  window.addEventListener('keydown', onKey, {capture: true});
  for (const type of GESTURES) window.addEventListener(type, onGesture, {capture: true, passive: false});
  if (ZOOM_LEVELS[index] !== applied) void apply();
  return {
    get level() { return ZOOM_LEVELS[index]; },
    step, reset,
    destroy() {
      window.removeEventListener('wheel', onWheel, {capture: true});
      window.removeEventListener('keydown', onKey, {capture: true});
      for (const type of GESTURES) window.removeEventListener(type, onGesture, {capture: true});
      if (hideTimer !== null) window.clearTimeout(hideTimer);
    },
  };
}
