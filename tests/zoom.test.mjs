import test from 'node:test';
import assert from 'node:assert/strict';
import {mountZoom, zoomIndexFor, ZOOM_LEVELS} from '../dist/zoom.js';

function rig({saved = null, fail = () => false} = {}) {
  const listeners = new Map(), calls = [], writes = [], timers = new Map();
  let seq = 0, pending = [];
  const window = {
    localStorage: {getItem: () => saved === null ? null : JSON.stringify(saved), setItem: (_key, value) => writes.push(JSON.parse(value))},
    addEventListener: (name, fn, options) => listeners.set(name, {fn, options}),
    removeEventListener: name => listeners.delete(name),
    setTimeout: (fn, ms) => { timers.set(++seq, {fn, ms}); return seq; },
    clearTimeout: id => timers.delete(id),
  };
  // Native calls settle only when the test says so, to expose ordering.
  const setZoom = level => { calls.push(level); return new Promise((resolve, reject) => pending.push(() => fail(level) ? reject(new Error('denied')) : resolve())); };
  const indicator = {hidden: true, textContent: ''};
  const zoom = mountZoom({window, setZoom, indicator});
  const settle = async () => { while (pending.length) { const run = pending; pending = []; for (const fn of run) fn(); await new Promise(resolve => setImmediate(resolve)); } };
  const fire = (name, event) => {
    let prevented = false, stopped = false;
    listeners.get(name).fn({preventDefault() { prevented = true; }, stopPropagation() { stopped = true; }, ...event});
    return {prevented, stopped};
  };
  return {zoom, calls, writes, timers, indicator, settle, fire, listeners};
}

test('keyboard shortcuts step through levels, reset, and persist only applied levels', async () => {
  const r = rig();
  assert.equal(r.calls.length, 0, 'default zoom needs no native call at launch');
  assert.equal(r.listeners.get('keydown').options.capture, true);
  assert.deepEqual(r.fire('keydown', {key: '=', metaKey: true}), {prevented: true, stopped: true});
  await r.settle();
  assert.deepEqual(r.calls, [1.1]); assert.deepEqual(r.writes, [{level: 1.1}]);
  assert.equal(r.indicator.hidden, false); assert.equal(r.indicator.textContent, '화면 110%');
  r.fire('keydown', {key: '+', ctrlKey: true, shiftKey: true}); await r.settle();
  r.fire('keydown', {key: '-', ctrlKey: true}); await r.settle();
  r.fire('keydown', {key: 'x', code: 'NumpadSubtract', ctrlKey: true}); await r.settle();
  assert.deepEqual(r.calls, [1.1, 1.25, 1.1, 1]);
  r.fire('keydown', {key: '=', metaKey: true}); r.fire('keydown', {key: '0', metaKey: true}); await r.settle();
  assert.equal(r.zoom.level, 1);
  assert.deepEqual(r.fire('keydown', {key: '=', metaKey: true, altKey: true}), {prevented: false, stopped: false});
  assert.deepEqual(r.fire('keydown', {key: '='}), {prevented: false, stopped: false}, 'plain typing is untouched');
  assert.deepEqual(r.fire('keydown', {key: 'o', metaKey: true}), {prevented: false, stopped: false}, 'other shortcuts pass through');
});

test('a burst of steps applies only the newest level after the native call in flight', async () => {
  const r = rig();
  for (let i = 0; i < 4; i++) r.fire('keydown', {key: '=', metaKey: true});
  assert.deepEqual(r.calls, [1.1]);
  await r.settle();
  assert.deepEqual(r.calls, [1.1, 1.75]); assert.equal(r.zoom.level, 1.75);
  assert.deepEqual(r.writes.at(-1), {level: 1.75});
});

test('levels clamp at both ends and the indicator still reports the limit', async () => {
  const r = rig();
  for (let i = 0; i < 20; i++) r.zoom.step(1);
  await r.settle(); assert.equal(r.zoom.level, 2); assert.equal(r.indicator.textContent, '화면 200%');
  for (let i = 0; i < 20; i++) r.zoom.step(-1);
  await r.settle(); assert.equal(r.zoom.level, 0.5); assert.equal(r.calls.at(-1), 0.5);
});

test('saved zoom is restored once at launch and unknown values snap to a level', async () => {
  const r = rig({saved: {level: 1.3}});
  assert.equal(r.zoom.level, 1.25); assert.deepEqual(r.calls, [1.25]);
  assert.equal(r.indicator.hidden, true, 'restoring does not flash the indicator');
  assert.equal(rig({saved: {level: 'big'}}).zoom.level, 1);
  assert.equal(zoomIndexFor(NaN), ZOOM_LEVELS.indexOf(1));
});

test('a rejected native call keeps the previous level and is not saved', async () => {
  const r = rig({fail: level => level === 1.1});
  r.zoom.step(1); await r.settle();
  assert.equal(r.zoom.level, 1); assert.deepEqual(r.writes, []);
});

test('modifier wheel zooms per notch without scrolling; plain wheel is left to the viewports', async () => {
  const r = rig();
  assert.deepEqual(r.listeners.get('wheel').options, {capture: true, passive: false});
  assert.deepEqual(r.fire('wheel', {deltaY: 120, timeStamp: 0}), {prevented: false, stopped: false});
  assert.deepEqual(r.fire('wheel', {deltaY: -4, ctrlKey: true, timeStamp: 1000}), {prevented: true, stopped: true}, 'one small notch still zooms');
  await r.settle(); assert.equal(r.zoom.level, 1.1);
  r.fire('wheel', {deltaY: 100, metaKey: true, timeStamp: 2000}); await r.settle();
  assert.equal(r.zoom.level, 1, 'scrolling down zooms out');
  // A trackpad's stream of small deltas steps at most every 150 ms.
  for (let t = 3000; t < 3300; t += 10) r.fire('wheel', {deltaY: -2, ctrlKey: true, timeStamp: t});
  await r.settle(); assert.equal(r.zoom.level, 1.25);
  // A fast mouse spin keeps one step per full notch.
  for (let t = 4000; t < 4060; t += 20) r.fire('wheel', {deltaY: 1, deltaMode: 1, ctrlKey: true, timeStamp: t});
  r.fire('wheel', {deltaY: 3, deltaMode: 1, ctrlKey: true, timeStamp: 4060});
  await r.settle(); assert.equal(r.zoom.level, 1, 'line-mode deltas: first notch, then a full 100px notch');
  assert.deepEqual(r.fire('wheel', {deltaX: 50, deltaY: 0, ctrlKey: true, timeStamp: 5000}), {prevented: false, stopped: false});
});

test('the indicator hides after a short delay and destroy removes listeners', () => {
  const r = rig();
  r.zoom.step(1); const [[, timer]] = r.timers; assert.equal(timer.ms, 1200);
  timer.fn(); assert.equal(r.indicator.hidden, true);
  r.zoom.destroy(); assert.equal(r.listeners.size, 0);
  assert.equal(mountZoom({window: {}}), null, 'without a native zoom there is nothing to mount');
});

// WebKit's GestureEvent: `scale` is relative to the distance at gesturestart.
const pinch = (r, type, scale, timeStamp) => r.fire(type, {type, scale, timeStamp});

test('a trackpad pinch follows the fingers through the zoom levels and persists like the keys', async () => {
  const r = rig();
  for (const type of ['gesturestart', 'gesturechange', 'gestureend']) assert.deepEqual(r.listeners.get(type).options, {capture: true, passive: false});
  assert.deepEqual(pinch(r, 'gesturestart', 1, 0), {prevented: true, stopped: true}, 'the webview never magnifies the page itself');
  assert.equal(r.calls.length, 0, 'starting a pinch changes nothing');
  assert.deepEqual(pinch(r, 'gesturechange', 1.12, 16), {prevented: true, stopped: true});
  await r.settle(); assert.equal(r.zoom.level, 1.1); assert.equal(r.indicator.textContent, '화면 110%');
  pinch(r, 'gesturechange', 1.5, 32); pinch(r, 'gesturechange', 2.4, 48);
  await r.settle(); assert.equal(r.zoom.level, 2, 'pinching past 200% stays at the limit');
  pinch(r, 'gesturechange', 1.26, 64);
  assert.deepEqual(pinch(r, 'gestureend', 1.26, 80), {prevented: true, stopped: true});
  await r.settle(); assert.equal(r.zoom.level, 1.25); assert.deepEqual(r.writes.at(-1), {level: 1.25});
  assert.deepEqual(r.calls, [1.1, 1.5, 2, 1.25], 'the same one-at-a-time native calls as the keys');
  // The next pinch starts from the level the last one left.
  pinch(r, 'gesturestart', 1, 1000); pinch(r, 'gesturechange', 0.5, 1016); pinch(r, 'gestureend', 0.5, 1032);
  await r.settle(); assert.equal(r.zoom.level, 0.67, '125% × 0.5 lands on the nearest level');
  pinch(r, 'gesturestart', 1, 2000); pinch(r, 'gesturechange', 0.1, 2016); pinch(r, 'gestureend', 0.1, 2032);
  await r.settle(); assert.equal(r.zoom.level, 0.5);
});

test('a pinch held between two levels does not flicker, and stray gesture events change nothing', async () => {
  const r = rig();
  pinch(r, 'gesturestart', 1, 0);
  pinch(r, 'gesturechange', 1.06, 10); assert.equal(r.zoom.level, 1, 'not yet clearly closer to 110%');
  pinch(r, 'gesturechange', 1.08, 20); assert.equal(r.zoom.level, 1.1);
  pinch(r, 'gesturechange', 1.04, 30); assert.equal(r.zoom.level, 1.1, 'back over the midpoint is not enough');
  pinch(r, 'gesturechange', 1.02, 40); assert.equal(r.zoom.level, 1);
  for (const scale of [0, -1, NaN, Infinity, undefined]) pinch(r, 'gesturechange', scale, 50);
  assert.equal(r.zoom.level, 1, 'unusable scales are ignored');
  pinch(r, 'gestureend', 1.02, 60);
  assert.deepEqual(pinch(r, 'gesturechange', 2, 1000), {prevented: true, stopped: true});
  await r.settle(); assert.equal(r.zoom.level, 1, 'a change without a started pinch is ignored');
});

test('Ctrl+wheel that a webview reports during or right after a pinch does not zoom twice', async () => {
  const r = rig();
  pinch(r, 'gesturestart', 1, 0); pinch(r, 'gesturechange', 1.12, 16);
  assert.deepEqual(r.fire('wheel', {deltaY: -10, ctrlKey: true, timeStamp: 20}), {prevented: true, stopped: true}, 'still kept from scrolling the log');
  pinch(r, 'gestureend', 1.12, 30);
  r.fire('wheel', {deltaY: -10, ctrlKey: true, timeStamp: 200});
  await r.settle(); assert.equal(r.zoom.level, 1.1, 'only the pinch stepped');
  r.fire('wheel', {deltaY: -10, ctrlKey: true, timeStamp: 600});
  await r.settle(); assert.equal(r.zoom.level, 1.25, 'Ctrl+wheel zooms again once the pinch is over');
});
