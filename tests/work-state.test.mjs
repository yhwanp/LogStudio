import test from 'node:test';
import assert from 'node:assert/strict';
import {readWork, readSavedFilters, isEmptyWork, writeWork, mergeReloadMarks, mergeSavedMarks, describeReloadMarks, describeRestore, describeMissingMarks, createWorkSaver, defaultFilters, WORK_VERSION} from '../dist/work-state.js';
import {MAX_BOOKMARKS} from '../dist/bookmarks.js';
import {emptyCondition} from '../dist/filter-model.js';

const place = (top=null, selected=null) => ({top, selected});
const work = (extra={}) => ({filters:defaultFilters(), savedFilter:null, bookmarks:[], upper:place(), lower:place(), ...extra});

const condition = (extra={}) => ({...emptyCondition(), ...extra});
test('saved filters keep known fields of the right type and fall back as a whole when invalid', () => {
  const filters = readSavedFilters({include:[{levels:['E', 'X', 'W'], tag:'Net', pid:'42', extra:'ignored'}, 'x'], exclude:[{tag:'chatty', enabled:false}], caseSensitive:'yes', includeRaw:false, recognizeLineNumbers:true, searchMode:'glob', query:'q'.repeat(2000), extra:'ignored'});
  assert.deepEqual(filters.include, [condition({levels:['W', 'E'], tag:'Net', pid:'42'})], 'levels keep the core order and drop unknown letters');
  assert.deepEqual(filters.exclude, [condition({tag:'chatty', enabled:false})]);
  assert.equal(filters.includeRaw, false);
  assert.equal(filters.caseSensitive, false, 'a wrong type keeps the default');
  assert.equal(filters.recognizeLineNumbers, true); assert.equal(filters.searchMode, 'literal');
  assert.equal(filters.query.length, 1000); assert.equal('extra' in filters, false);
  assert.deepEqual(readSavedFilters({include:[{tag:'Net'}, {pid:'abc'}]}), defaultFilters(), 'a rejected PID resets every field');
  assert.deepEqual(readSavedFilters({exclude:[{timeFrom:'25:00'}]}), defaultFilters());
  assert.deepEqual(readSavedFilters({include:Array.from({length:40}, (_, i) => ({tag:`t${i}`}))}).include.length, 32, 'at most 32 conditions');
  assert.deepEqual(readSavedFilters({include:[], exclude:[{tag:'x'}]}).include, [emptyCondition()], 'the panel always edits a showing condition');
  assert.deepEqual(readSavedFilters(null), defaultFilters());
});

test('work saved before conditions reads as one showing and one hiding condition', () => {
  const old = readSavedFilters({levels:['W', 'E', 'F', 'A'], tag:'Net', excludeTags:'chatty', timeFrom:'12:00', timeTo:'', pid:'', tid:'7', query:'boom', includeRaw:true});
  assert.deepEqual(old.include, [condition({levels:['W', 'E', 'F', 'A'], tag:'Net', timeFrom:'12:00', tid:'7'})]);
  assert.deepEqual(old.exclude, [condition({tag:'chatty'})]);
  assert.equal(old.query, 'boom');
  assert.deepEqual(readSavedFilters({tag:'Net', excludeTags:' , '}).exclude, [], 'no excluded tags, no hiding condition');
  assert.deepEqual(readSavedFilters({tag:'Net', pid:'abc'}), defaultFilters(), 'a rejected PID still resets every field');
});

test('saved work is read only in this version, with bounded positive sorted bookmarks', () => {
  assert.equal(readWork(null), null); assert.equal(readWork({v:99}), null); assert.equal(readWork([1]), null);
  const lines = Array.from({length:MAX_BOOKMARKS + 50}, (_, i) => MAX_BOOKMARKS + 50 - i);
  const saved = readWork({v:WORK_VERSION, bookmarks:[...lines, 0, -1, 1.5, '7', 3, 3], upper:{top:9, selected:'x'}, lower:{top:-2}});
  assert.equal(saved.bookmarks.length, MAX_BOOKMARKS);
  assert.deepEqual(saved.bookmarks.slice(0, 3), [1, 2, 3]);
  assert.deepEqual(saved.upper, place(9, null)); assert.deepEqual(saved.lower, place());
  assert.deepEqual(readWork(writeWork(work({bookmarks:[4], upper:place(2, 3)}))), work({bookmarks:[4], upper:place(2, 3)}));
  assert.deepEqual(readWork(writeWork(work({savedFilter:1739000000000}))), work({savedFilter:1739000000000}), 'the saved filter the work came from');
  for (const bad of [0, -3, 1.5, '7', 2 ** 53]) assert.equal(readWork({v:WORK_VERSION, savedFilter:bad}).savedFilter, null);
  assert.equal(readWork({v:WORK_VERSION}).savedFilter, null, 'work saved before saved filters');
});

test('work with nothing to restore counts as empty', () => {
  assert.equal(isEmptyWork(work()), true);
  assert.equal(isEmptyWork(work({bookmarks:[1]})), false);
  assert.equal(isEmptyWork(work({upper:place(null, 4)})), false);
  assert.equal(isEmptyWork(work({lower:place(8)})), false);
  assert.equal(isEmptyWork(work({filters:{...defaultFilters(), include:[condition({tag:'x'})]}})), false);
  assert.equal(isEmptyWork(work({filters:{...defaultFilters(), exclude:[condition({tag:'x', enabled:false})]}})), false, 'a switched-off condition is kept');
  const levels = emptyCondition().levels;
  assert.equal(isEmptyWork(work({filters:{...defaultFilters(), include:[condition({levels:[...levels.slice(1), levels[0]]}), condition()], exclude:[condition({tag:' , '})]}})), true, 'level order and conditions without criteria do not matter');
});

test('a reload refreshes marks, drops rows past the end and counts changed text', () => {
  const mark = (lineNumber, raw) => [lineNumber, {lineNumber, raw, kind:'raw', truncated:false}];
  const old = new Map([mark(2, 'two'), mark(5, 'five'), mark(9, 'nine')]);
  const current = new Map([...old, mark(6, 'added since')]);
  const rows = new Map([[2, {lineNumber:2, raw:'two', kind:'raw'}], [5, {lineNumber:5, raw:'FIVE', kind:'raw'}]]);
  const {entries, changed, dropped} = mergeReloadMarks(current, old, rows, 8);
  assert.deepEqual(entries.map(([line, m]) => [line, m.raw]), [[2, 'two'], [5, 'FIVE'], [6, 'added since']]);
  assert.equal(changed, 1); assert.equal(dropped, 1);
});

test('saved marks join marks made meanwhile, up to the limit, and report missing rows', () => {
  const rows = new Map([[3, {lineNumber:3, raw:'three', kind:'raw'}], [7, {lineNumber:7, raw:'seven', kind:'raw', owner:{lineNumber:6, level:'E'}}]]);
  const current = new Map([[1, {lineNumber:1, raw:'one'}]]);
  const {entries, dropped} = mergeSavedMarks(current, [3, 7, 40], rows);
  assert.deepEqual(entries.map(([line]) => line), [1, 3, 7]);
  assert.equal(entries[1][1].raw, 'three'); assert.equal(dropped, 1);
  const full = new Map(Array.from({length:MAX_BOOKMARKS}, (_, i) => [100 + i, {lineNumber:100 + i, raw:''}]));
  assert.equal(mergeSavedMarks(full, [3], rows).dropped, 1, 'the limit still holds');
});

test('toast copy says what a reload kept and what a restore skipped', () => {
  assert.equal(describeReloadMarks({}), '');
  assert.match(describeReloadMarks({shrank:true}), /짧아졌습니다.*밀렸을 수/);
  assert.match(describeReloadMarks({dropped:2}), /북마크 2개를 해제/);
  assert.match(describeReloadMarks({changed:3}), /3개의 내용이 바뀌었/);
  assert.match(describeReloadMarks({changed:1, dropped:1}), /북마크 1개 해제·1개 내용 바뀜.*밀렸을 수/);
  const saved = work({filters:{...defaultFilters(), include:[condition({tag:'Net'})], query:'x'}, bookmarks:[1, 2], upper:place(5)});
  assert.equal(describeRestore('exact', saved), '지난번 작업을 복원했습니다 · 필터 · 검색 · 북마크 2개 · 위치');
  assert.match(describeRestore('changed', saved), /필터와 검색만 복원/);
  assert.match(describeRestore('changed', work({bookmarks:[1]})), /북마크와 위치는 복원하지 않았/);
  assert.equal(describeMissingMarks(0), ''); assert.match(describeMissingMarks(4), /4개는 파일에 그 행이 없어/);
});

test('the saver sends changed work for its dataset only, debounced, and forgets empty work', async () => {
  const sent = [], timers = new Map(); let id = 0, current = work({bookmarks:[3]});
  const saver = createWorkSaver({invoke:async (command, args) => {sent.push([command, structuredClone(args)]);}, capture:() => current, setTimer:(fn, ms) => {timers.set(++id, {fn, ms}); return id;}, clearTimer:key => timers.delete(key)});
  const run = async () => {for (const [key, {fn}] of [...timers]) {timers.delete(key); fn();} await Promise.resolve(); await Promise.resolve();};
  saver.schedule(); assert.equal(timers.size, 0, 'nothing is tracked yet');
  saver.track(7);
  saver.schedule(); saver.schedule(); assert.equal(timers.size, 1); assert.equal([...timers.values()][0].ms, 500);
  await run();
  assert.deepEqual(sent, [['save_work_state', {datasetId:7, state:writeWork(current)}]]);
  saver.flush(); await run(); assert.equal(sent.length, 1, 'unchanged work is not sent again');
  current = work(); saver.flush(); await run();
  assert.deepEqual(sent.at(-1), ['save_work_state', {datasetId:7, state:null}], 'empty work forgets the entry');
  saver.track(8, null); saver.flush(); await run(); assert.equal(sent.length, 2, 'what native code already holds is not resent');
  current = work({bookmarks:[2]}); saver.flush(); saver.stop(); await run();
  assert.equal(sent.at(-1)[1].datasetId, 8, 'a flush right before stop keeps its dataset');
  current = work({bookmarks:[1]}); saver.flush(); await run(); assert.equal(sent.length, 3, 'a stopped saver sends nothing');
  assert.equal(saver.tracking(), null);
});
