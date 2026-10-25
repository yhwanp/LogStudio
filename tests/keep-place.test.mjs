import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as turn} from 'node:timers/promises';
import {createLargeLogController, RELOCATE_HOLD_MS} from '../dist/controller.js';

const deferred = () => {let resolve; const promise = new Promise(r => resolve = r); return {promise, resolve};};
async function settle() {for (let i = 0; i < 20; i++) await turn();}

// Every new query keeps physical rows 10, 20, …, 100: result i is row (i+1)*10.
function engine({count=10} = {}) {
  const id = {datasetId:1}, calls = [];
  let queryId = 0, locate = null;
  const transport = async b => {
    calls.push(b.action);
    if (b.action === 'status') return {...id, name:'synthetic', total:10000, size:100000, status:'ready', complete:true, textComplete:true, query:{id:queryId, status:'complete', count}};
    if (b.action === 'query') {queryId = b.queryId; return {...id, queryId, query:{id:queryId, status:'complete', count}};}
    if (b.action === 'locate') {
      if (locate) await locate.promise;
      const found = b.lineNumber % 10 === 0 && b.lineNumber / 10 <= count;
      return {...id, queryId:b.queryId, lineNumber:b.lineNumber, found, pending:false, index:Math.min(count, Math.floor(b.lineNumber / 10) - (found ? 1 : 0))};
    }
    if (b.action === 'page') return {...id, queryId:b.queryId, start:b.start, count, rows:Array.from({length:Math.max(0, Math.min(b.count, count - b.start))}, (_, n) => ({lineNumber:(b.start + n + 1) * 10, raw:'', kind:'raw'}))};
    if (b.action === 'record') return {...id, queryId:b.queryId, lineNumber:b.lineNumber, startByte:b.startByte, text:`row ${b.lineNumber}`, nextByte:7, totalBytes:7, complete:true};
    if (b.action === 'cancel') return {...id, queryId:b.queryId, status:'ready'};
    throw new Error(`Unexpected ${b.action}`);
  };
  const timers = new Map(); let next = 0;
  const setTimer = (fn, ms) => {const k = ++next; timers.set(k, {fn, ms}); return k;}, clearTimer = k => timers.delete(k);
  const fire = ms => {for (const [k, t] of timers) if (t.ms === ms) {timers.delete(k); t.fn();}};
  return {transport, setTimer, clearTimer, timers, fire, calls, hold:() => (locate = deferred()), release:() => {locate?.resolve(); locate = null;}};
}

test('a new filter keeps the selected row where it was on screen, with its detail', async () => {
  const e = engine(), moved = [];
  const c = createLargeLogController({transport:e.transport, setTimer:e.setTimer, clearTimer:e.clearTimer, onRelocate:r => moved.push(r)});
  await c.start(); await settle();
  c.selectLine(40); await c.loadDetail(40);
  await c.setFilters({include:[{levels:['W']}]}, {place:{line:40, select:true, offset:3}}); await settle();
  const s = c.getState();
  assert.equal(s.selectedLine, 40); assert.equal(s.viewportStart, 3); assert.equal(s.selectedIndex, 3);
  assert.equal(s.scrollAlign, 'offset'); assert.equal(s.scrollOffset, 3);
  assert.equal(s.detail.lineNumber, 40); assert.equal(s.detail.text, 'row 40');
  assert.equal(e.calls.filter(a => a === 'record').length, 1, 'a kept detail is not read again');
  assert.deepEqual(moved, []); assert.equal(c.relocating(), null); assert.equal(s.locationPending, null);
  await c.close({remote:false});
});

test('a selected row the new results leave out gives its place to the next result, or the last', async () => {
  const e = engine(), moved = [];
  const c = createLargeLogController({transport:e.transport, setTimer:e.setTimer, clearTimer:e.clearTimer, onRelocate:r => moved.push(r)});
  await c.start(); await settle();
  c.selectLine(41); await c.loadDetail(41);
  await c.setFilters({include:[{levels:['E']}]}, {place:{line:41, select:true, offset:null}}); await settle();
  let s = c.getState();
  assert.equal(s.selectedLine, 50); assert.equal(s.viewportStart, 4); assert.equal(s.scrollAlign, 'center');
  assert.equal(s.detail.lineNumber, 50, 'the detail follows the selection'); assert.deepEqual(moved, [{lineNumber:41, target:50}]);
  await c.setFilters({}, {place:{line:105, select:true, offset:0}}); await settle();
  s = c.getState();
  assert.equal(s.selectedLine, 100); assert.equal(s.viewportStart, 9); assert.deepEqual(moved.at(-1), {lineNumber:105, target:100});
  await c.close({remote:false});
});

test('without a selection the top row, or the result after it, stays at the top; no place starts the list over', async () => {
  const e = engine();
  const c = createLargeLogController({transport:e.transport, setTimer:e.setTimer, clearTimer:e.clearTimer});
  await c.start(); await settle();
  await c.setFilters({}, {place:{line:57, select:false, offset:0}}); await settle();
  let s = c.getState();
  assert.equal(s.selectedLine, null); assert.equal(s.detail, null); assert.equal(s.viewportStart, 5); assert.equal(s.scrollAlign, 'start');
  c.selectLine(60); await c.loadDetail(60);
  await c.setFilters({include:[{tag:'x'}]}); await settle();
  s = c.getState();
  assert.equal(s.selectedLine, null); assert.equal(s.detail, null); assert.equal(s.viewportStart, 0);
  await c.close({remote:false});
});

test('while a kept place is found the first rows wait briefly, then show; moving the list stops the search', async () => {
  const e = engine();
  const c = createLargeLogController({transport:e.transport, setTimer:e.setTimer, clearTimer:e.clearTimer});
  await c.start(); await settle();
  c.selectLine(40); await c.loadDetail(40);
  e.hold();
  await c.setFilters({include:[{levels:['W']}]}, {place:{line:40, select:true, offset:2}}); await settle();
  assert.deepEqual(c.relocating(), {line:40, offset:2, select:true, shown:false});
  assert.equal(c.getState().detail.lineNumber, 40, 'the detail stays while the row is looked for');
  let read = false; const rows = c.fetchRows(0).then(data => {read = true; return data;});
  await settle(); assert.equal(read, false, 'rows wait for the place');
  e.fire(RELOCATE_HOLD_MS); assert.equal(c.relocating().shown, true);
  assert.equal((await rows).start, 0, 'after the hold the list shows its start');
  const revision = c.getState().scrollRevision;
  c.stopRelocating();
  assert.equal(c.relocating(), null); assert.equal(c.getState().locationPending, null);
  assert.equal(c.getState().selectedLine, 40, 'a kept selection stays by its row');
  e.release(); await settle();
  assert.equal(c.getState().scrollRevision, revision, 'a place found after the user moved does not move the list');
  assert.equal(c.getState().viewportStart, 0);
  await c.close({remote:false});
});

test('rows held for a kept place are read again around it once it is found', async () => {
  const e = engine();
  const c = createLargeLogController({transport:e.transport, setTimer:e.setTimer, clearTimer:e.clearTimer});
  await c.start(); await settle();
  c.selectLine(70);
  e.hold();
  await c.setFilters({}, {place:{line:70, select:true, offset:0}}); await settle();
  const rows = c.fetchRows(0).then(() => 'read', error => error.name); e.release(); await settle();
  assert.equal(await rows, 'AbortError');
  assert.equal(c.getState().viewportStart, 6); assert.equal(c.getState().selectedLine, 70);
  await c.close({remote:false});
});

test('a place kept while typing outlives the reserved query, and a newer filter keeps the place still being found', async () => {
  const e = engine();
  const c = createLargeLogController({transport:e.transport, setTimer:e.setTimer, clearTimer:e.clearTimer, pane:'search'});
  await c.start(); await settle();
  c.selectLine(30); await c.loadDetail(30);
  c.invalidateQuery({place:{line:30, select:true, offset:4}});
  assert.equal(c.getState().selectedLine, 30); assert.equal(c.getState().detail.lineNumber, 30);
  assert.deepEqual(c.relocating(), {line:30, offset:4, select:true, shown:false});
  e.hold();
  await c.setFilters({query:'x', include:[{tag:'a'}]}, {place:c.relocating()}); await settle();
  // A second filter before the first one found the row keeps the same place.
  await c.setFilters({query:'x', include:[{tag:'b'}]}, {place:c.relocating()}); e.release(); await settle();
  const s = c.getState();
  assert.equal(s.selectedLine, 30); assert.equal(s.viewportStart, 2); assert.equal(s.scrollOffset, 4); assert.equal(s.detail.lineNumber, 30);
  // A new search starts over.
  c.invalidateQuery(); assert.equal(c.relocating(), null); assert.equal(c.getState().selectedLine, null); assert.equal(c.getState().detail, null);
  await c.close({remote:false});
});

test('a place the query cannot find leaves the list at its start; the anchor and its detail stay for the next filter', async () => {
  const e = engine({count:0});
  const c = createLargeLogController({transport:e.transport, setTimer:e.setTimer, clearTimer:e.clearTimer});
  await c.start(); await settle();
  c.selectLine(40); await c.loadDetail(40);
  await c.setFilters({include:[{tag:'none'}]}, {place:{line:40, select:true, offset:1}}); await settle();
  const s = c.getState();
  assert.equal(s.selectedLine, null); assert.equal(s.viewportStart, 0); assert.equal(c.relocating(), null);
  assert.equal(s.anchor, 40); assert.equal(s.anchorOffset, 1); assert.equal(s.detail.lineNumber, 40, 'the detail waits for the row');
  await c.close({remote:false});
});

test('a kept place waits for the engine to accept the new query before it locates the row', async () => {
  const e = engine(), reply = deferred(), base = e.transport;
  let locates = 0, held = true;
  const transport = async (b, signal) => {
    if (b.action === 'locate') locates++;
    if (b.action === 'query' && held) {held = false; await reply.promise;}
    return base(b, signal);
  };
  const c = createLargeLogController({transport, setTimer:e.setTimer, clearTimer:e.clearTimer});
  await c.start(); await settle();
  held = true; c.selectLine(40);
  const applied = c.setFilters({include:[{levels:['W']}]}, {place:{line:40, select:true, offset:0}}); await settle();
  // Polls while the query reply is late send no locate for an unknown query.
  for (const [k, t] of [...e.timers]) if (t.ms !== RELOCATE_HOLD_MS) {e.timers.delete(k); t.fn();}
  await settle();
  assert.equal(locates, 0); assert.equal(c.getState().error, '');
  reply.resolve(); await applied; await settle();
  assert.equal(locates, 1); assert.equal(c.getState().selectedLine, 40); assert.equal(c.getState().viewportStart, 3);
  await c.close({remote:false});
});

// Results that depend on the filter: tag 'all' keeps every row, tag 'none'
// keeps none, anything else every tenth row as `engine` does.
function filtered() {
  const id = {datasetId:1};
  let queryId = 0, mode = 'tens';
  const count = () => mode === 'all' ? 1000 : mode === 'none' ? 0 : 100;
  const transport = async b => {
    if (b.action === 'status') return {...id, name:'synthetic', total:1000, size:100000, status:'ready', complete:true, textComplete:true, query:{id:queryId, status:'complete', count:count()}};
    if (b.action === 'query') {
      const tag = b.filters.include[0]?.tag;
      mode = tag === 'all' ? 'all' : tag === 'none' ? 'none' : 'tens'; queryId = b.queryId;
      return {...id, queryId, query:{id:queryId, status:'complete', count:count()}};
    }
    if (b.action === 'locate') {
      if (mode === 'all') return {...id, queryId:b.queryId, lineNumber:b.lineNumber, found:true, pending:false, index:b.lineNumber - 1};
      if (mode === 'none') return {...id, queryId:b.queryId, lineNumber:b.lineNumber, found:false, pending:false, index:0};
      const found = b.lineNumber % 10 === 0;
      return {...id, queryId:b.queryId, lineNumber:b.lineNumber, found, pending:false, index:Math.floor(b.lineNumber / 10) - (found ? 1 : 0)};
    }
    if (b.action === 'page') return {...id, queryId:b.queryId, start:b.start, count:count(), rows:Array.from({length:Math.max(0, Math.min(b.count, count() - b.start))}, (_, n) => ({lineNumber:mode === 'all' ? b.start + n + 1 : (b.start + n + 1) * 10, raw:'', kind:'raw'}))};
    if (b.action === 'record') return {...id, queryId:b.queryId, lineNumber:b.lineNumber, startByte:b.startByte, text:`row ${b.lineNumber}`, nextByte:7, totalBytes:7, complete:true};
    throw new Error(`Unexpected ${b.action}`);
  };
  return {transport};
}

test('the row the user chose stays the anchor while other rows stand in for it, and comes back with a wider filter', async () => {
  const e = engine(), f = filtered(), moved = [];
  const c = createLargeLogController({transport:f.transport, setTimer:e.setTimer, clearTimer:e.clearTimer, onRelocate:r => moved.push(r)});
  await c.start(); await settle();
  await c.setFilters({include:[{tag:'all'}]}); await settle();
  c.selectLine(41); await c.loadDetail(41);
  assert.equal(c.getState().anchor, 41);
  const place = () => ({line:c.getState().anchor ?? c.getState().selectedLine, select:true, offset:2});
  await c.setFilters({include:[{tag:'tens'}]}, {place:place()}); await settle();
  let s = c.getState();
  assert.equal(s.selectedLine, 50); assert.equal(s.anchor, 41, 'a stand-in is not the anchor'); assert.equal(s.detail.lineNumber, 50);
  assert.deepEqual(moved, [{lineNumber:41, target:50}]);
  // The same stand-in again says nothing new.
  await c.setFilters({include:[{tag:'tens', levels:['W']}]}, {place:place()}); await settle();
  assert.equal(c.getState().selectedLine, 50); assert.equal(moved.length, 1);
  // A filter that shows the anchor again returns to it.
  await c.setFilters({include:[{tag:'all'}]}, {place:place()}); await settle();
  s = c.getState();
  assert.equal(s.selectedLine, 41); assert.equal(s.anchor, 41); assert.equal(s.viewportStart, 40); assert.equal(s.detail.lineNumber, 41);
  assert.equal(moved.length, 1, 'returning to the anchor needs no toast');
  await c.close({remote:false});
});

test('no results keep the anchor and its detail; the next filter with the row selects it again', async () => {
  const e = engine(), f = filtered();
  const c = createLargeLogController({transport:f.transport, setTimer:e.setTimer, clearTimer:e.clearTimer});
  await c.start(); await settle();
  c.selectLine(40); await c.loadDetail(40);
  await c.setFilters({include:[{tag:'none'}]}, {place:{line:40, select:true, offset:3}}); await settle();
  let s = c.getState();
  assert.equal(s.selectedLine, null); assert.equal(s.anchor, 40); assert.equal(s.anchorOffset, 3); assert.equal(s.detail.lineNumber, 40);
  await c.setFilters({include:[{tag:'tens'}]}, {place:{line:s.anchor, select:true, offset:s.anchorOffset}}); await settle();
  s = c.getState();
  assert.equal(s.selectedLine, 40); assert.equal(s.viewportStart, 3); assert.equal(s.scrollOffset, 3); assert.equal(s.detail.lineNumber, 40);
  await c.close({remote:false});
});

test('a row followed in place of the other pane\'s anchor keeps this pane\'s anchor; the list start clears it', async () => {
  const e = engine(), f = filtered();
  const c = createLargeLogController({transport:f.transport, setTimer:e.setTimer, clearTimer:e.clearTimer});
  await c.start(); await settle();
  await c.setFilters({include:[{tag:'all'}]}); await settle();
  await c.locateLine(26); assert.equal(c.getState().anchor, 26);
  await c.locateLine(40, {keepAnchor:true});
  assert.equal(c.getState().selectedLine, 40); assert.equal(c.getState().anchor, 26);
  c.seek(0); assert.equal(c.getState().anchor, null);
  await c.close({remote:false});
});
