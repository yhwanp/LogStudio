import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as turn} from 'node:timers/promises';
import {emptyCondition, readCondition, isEmptyCondition, coreFilters, conditionsWithoutRows, clip} from '../dist/filter-model.js';
const MESSAGE_PATTERN_ERROR = 'Invalid or unsupported message pattern: ';
import {describeCondition, describeFilter, describeLogError} from '../dist/describe.js';
import {toStored, fromStored, sameFields, SAVED_VERSION} from '../dist/saved-filters.js';
import {createLargeLogController} from '../dist/controller.js';

async function settle() {for (let i = 0; i < 20; i++) await turn();}
const cond = (extra={}) => ({...emptyCondition(), ...extra});

test('a message is a criterion used as typed, and its mode means nothing without one', () => {
  assert.equal(isEmptyCondition(cond({message:' '})), false, 'a space is a text');
  assert.equal(isEmptyCondition(cond({messageRegex:true})), true);
  const sent = coreFilters({include:[cond({message:' frame skipped ', messageRegex:false}), cond({messageRegex:true})], exclude:[cond({message:'\\d+ ms', messageRegex:true})]});
  assert.deepEqual(sent.include.map(c => [c.message, c.messageRegex]), [[' frame skipped ', false]], 'a condition without criteria is not sent');
  assert.deepEqual(sent.exclude.map(c => [c.message, c.messageRegex]), [['\\d+ ms', true]]);
  assert.deepEqual(readCondition({message:'x'.repeat(2000), messageRegex:'yes'}), cond({message:'x'.repeat(1000)}), 'bounded, and only true turns a pattern on');
  // A condition marked as having no rows stays marked only while it reads as sent.
  const conds = {include:[cond({message:'a'})], exclude:[]};
  const marked = conditionsWithoutRows(conds, {include:coreFilters(conds).include, exclude:[]}, {include:['none'], exclude:[]});
  assert.deepEqual([...marked.include], [0]);
  conds.include[0].messageRegex = true;
  assert.deepEqual([...conditionsWithoutRows(conds, {include:coreFilters({include:[cond({message:'a'})]}).include, exclude:[]}, {include:['none'], exclude:[]}).include], []);
});

test('chips quote a message and name a pattern; DLT calls it the payload', () => {
  assert.deepEqual(describeCondition(cond({message:'frame skipped'})), [{key:'message', label:'메시지: “frame skipped”', full:'메시지: “frame skipped”'}]);
  assert.equal(describeCondition(cond({message:' a '}))[0].label, '메시지: “ a ”', 'spaces stay visible');
  assert.equal(describeCondition(cond({message:'\\d+ ms', messageRegex:true}))[0].label, '메시지 정규식: “\\d+ ms”');
  assert.equal(describeCondition(cond({message:'x'.repeat(50)}))[0].label, `메시지: “${'x'.repeat(40)}…”`);
  assert.equal(describeCondition(cond({message:'경로'}), 'dlt')[0].label, '페이로드: “경로”');
  assert.equal(describeFilter({include:[cond({tag:'App', message:'boom'})], exclude:[cond({message:'tick'})]}), '태그: App · 메시지: “boom” · 제외 메시지: “tick”');
  assert.equal(describeLogError(`${MESSAGE_PATTERN_ERROR}unclosed group`), '메시지 정규식을 확인해 주세요: unclosed group');
});

test('saved filters keep a message and its mode, and compare them', () => {
  const fields = {include:[cond({message:'frame skipped'})], exclude:[cond({message:'\\d+ ms', messageRegex:true, levels:['V', 'D']})], includeRaw:true};
  const stored = toStored(fields);
  assert.deepEqual(stored, {v:SAVED_VERSION, include:[{message:'frame skipped'}], exclude:[{levels:['V', 'D'], message:'\\d+ ms', messageRegex:true}], includeRaw:true});
  assert.deepEqual(fromStored(stored), fields);
  assert.ok(!sameFields(fields, {...fields, include:[cond({message:'frame skipped', messageRegex:true})]}), 'the mode changes the filter');
  assert.ok(sameFields({include:[cond({tag:'a', messageRegex:true})], exclude:[]}, {include:[cond({tag:'a'})], exclude:[]}), 'a mode without a message does not');
  assert.ok(!sameFields({include:[cond({message:'a '})], exclude:[]}, {include:[cond({message:'a'})], exclude:[]}), 'a message is compared as typed');
  // A filter saved before messages existed reads the same.
  assert.deepEqual(fromStored({v:SAVED_VERSION, include:[{tag:'a'}], exclude:[], includeRaw:true}), {include:[cond({tag:'a'})], exclude:[], includeRaw:true});
});

test('a message pattern the engine rejects keeps the previous query, its place and its filters', async () => {
  const id = {datasetId:1}, timers = new Map(); let queryId = 0, next = 0;
  const reject = Object.assign(new Error('Local request failed'), {userMessage:`${MESSAGE_PATTERN_ERROR}unclosed group`});
  const transport = async b => {
    if (b.action === 'status') return {...id, name:'synthetic', total:100, size:1000, status:'ready', complete:true, textComplete:true, query:{id:queryId, status:'complete', count:10}};
    if (b.action === 'query') {
      if (b.filters.include.some(c => c.message === '(')) throw reject;
      queryId = b.queryId; return {...id, queryId, query:{id:queryId, status:'complete', count:10}};
    }
    if (b.action === 'locate') return {...id, queryId:b.queryId, lineNumber:b.lineNumber, found:true, pending:false, index:Math.floor(b.lineNumber / 10) - 1};
    throw new Error(b.action);
  };
  const c = createLargeLogController({transport, setTimer:(fn, ms) => {const k = ++next; timers.set(k, {fn, ms}); return k;}, clearTimer:k => timers.delete(k)});
  await c.start(); await settle();
  await c.setFilters({include:[{message:'frames'}]}); await settle();
  c.selectLine(50);
  const kept = c.getState().queryId;
  await c.setFilters({include:[{message:'(', messageRegex:true}]}, {place:{line:50, select:true, offset:2}}); await settle();
  const s = c.getState();
  assert.equal(s.queryId, kept); assert.equal(s.queryStatus, 'complete'); assert.equal(s.count, 10); assert.equal(s.error, '');
  assert.equal(s.rejected, `${MESSAGE_PATTERN_ERROR}unclosed group`);
  assert.equal(s.selectedLine, 50); assert.equal(s.viewportStart, 4); assert.equal(s.scrollOffset, 2, 'the list returns to its place');
  assert.deepEqual(c.filters().include.map(f => f.message), ['frames']);
  // A filter the engine accepts clears it.
  await c.setFilters({include:[{message:'(frames)', messageRegex:true}]}); await settle();
  assert.equal(c.getState().rejected, ''); assert.notEqual(c.getState().queryId, kept);
  await c.close({remote:false});
});

test('a filter over the engine\'s bounds keeps the previous query, its place and its filters too', async () => {
  const id = {datasetId:1}, timers = new Map(); let queryId = 0, next = 0;
  let reason = '';
  const transport = async b => {
    if (b.action === 'status') return {...id, name:'synthetic', total:100, size:1000, status:'ready', complete:true, textComplete:true, query:{id:queryId, status:'complete', count:10}};
    if (b.action === 'query') {
      if (b.filters.include.some(c => c.message === 'long')) throw Object.assign(new Error('Local request failed'), {userMessage:reason});
      queryId = b.queryId; return {...id, queryId, query:{id:queryId, status:'complete', count:10}};
    }
    if (b.action === 'locate') return {...id, queryId:b.queryId, lineNumber:b.lineNumber, found:true, pending:false, index:Math.floor(b.lineNumber / 10) - 1};
    throw new Error(b.action);
  };
  const c = createLargeLogController({transport, setTimer:(fn, ms) => {const k = ++next; timers.set(k, {fn, ms}); return k;}, clearTimer:k => timers.delete(k)});
  await c.start(); await settle();
  await c.setFilters({include:[{message:'frames'}]}); await settle();
  c.selectLine(50);
  const kept = c.getState().queryId;
  for (reason of ['Filter budget exceeded: message', 'Filter budget exceeded: conditions', 'Request budget exceeded']) {
    await c.setFilters({include:[{message:'long'}]}, {place:{line:50, select:true, offset:2}}); await settle();
    const s = c.getState();
    assert.equal(s.queryId, kept); assert.equal(s.queryStatus, 'complete'); assert.equal(s.count, 10); assert.equal(s.error, '');
    assert.equal(s.rejected, reason);
    assert.equal(s.selectedLine, 50); assert.equal(s.viewportStart, 4); assert.equal(s.scrollOffset, 2, 'the list returns to its place');
    assert.deepEqual(c.filters().include.map(f => f.message), ['frames']);
  }
  // Any other error still fails the query.
  reason = 'Request failed';
  await c.setFilters({include:[{message:'long'}]}); await settle();
  assert.equal(c.getState().rejected, ''); assert.equal(c.getState().queryStatus, 'error'); assert.equal(c.getState().error, reason);
  await c.setFilters({include:[{message:'short'}]}); await settle();
  assert.equal(c.getState().rejected, ''); assert.equal(c.getState().error, '');
  await c.close({remote:false});
});

test('the engine\'s bounds are explained in Korean with the field', () => {
  const told = (text, format) => describeLogError(`Filter budget exceeded: ${text}`, format);
  assert.equal(told('message'), '조건의 메시지 입력이 너무 깁니다. 줄여 주세요.');
  assert.equal(told('message', 'dlt'), '조건의 페이로드 입력이 너무 깁니다. 줄여 주세요.');
  assert.equal(told('tag'), '조건의 태그 입력이 너무 깁니다. 줄여 주세요.');
  assert.equal(told('tag', 'dlt'), '조건의 모든 ID 입력이 너무 깁니다. 줄여 주세요.');
  assert.equal(told('PID'), '조건의 PID 입력이 너무 깁니다. 줄여 주세요.');
  assert.equal(told('time'), '조건의 시간 입력이 너무 깁니다. 줄여 주세요.');
  assert.equal(told('query'), '검색어가 너무 깁니다. 줄여 주세요.');
  assert.equal(told('conditions'), '조건은 32개까지 쓸 수 있습니다.');
  // Every refusal the page knows has Korean words, a bound without a name too.
  assert.equal(describeLogError('Filter budget exceeded'), '필터 조건이 너무 많거나 깁니다. 조건을 줄여 주세요.');
  assert.equal(describeLogError('Request budget exceeded'), '작업이 너무 커서 완료하지 못했습니다.');
});

test('a text cut to its bound keeps whole characters', () => {
  // Half an emoji would be no text the engine reads.
  assert.equal(clip('a'.repeat(999) + '🙂', 1000), 'a'.repeat(999));
  assert.equal(clip('a'.repeat(998) + '🙂b', 1000), 'a'.repeat(998) + '🙂');
  assert.equal(clip('짧음', 1000), '짧음');
  assert.equal(readCondition({message:'a'.repeat(999) + '🙂'}).message, 'a'.repeat(999));
  assert.equal(readCondition({tag:'a'.repeat(3999) + '🙂'}).tag, 'a'.repeat(3999));
});

test('a refused search goes back to the results typing had taken away, and their place', async () => {
  const id = {datasetId:1}, timers = new Map(), calls = []; let engine = 0, next = 0, refuse = '';
  const transport = async b => {
    calls.push(b);
    if (b.action === 'status') return {...id, name:'synthetic', total:100, size:1000, status:'ready', complete:true, textComplete:true, query:{id:engine, status:'complete', count:10}};
    if (b.action === 'query') {
      if (refuse) throw Object.assign(new Error('Local request failed'), {userMessage:refuse});
      engine = b.queryId; return {...id, queryId:engine, query:{id:engine, status:'complete', count:10}};
    }
    if (b.action === 'locate') return {...id, queryId:b.queryId, lineNumber:b.lineNumber, found:true, pending:false, index:b.lineNumber / 10 - 1};
    if (b.action === 'cancel') return {...id, queryId:b.queryId};
    throw new Error(b.action);
  };
  const c = createLargeLogController({transport, pane:'search', setTimer:(fn, ms) => {const k = ++next; timers.set(k, {fn, ms}); return k;}, clearTimer:k => timers.delete(k)});
  const run = ms => {for (const [k, task] of [...timers]) if (task.ms === ms) {timers.delete(k); task.fn();}};
  const kept = {query:'frames'}, long = {query:'long'}, SIZE = 'Filter budget exceeded: query';
  await c.start(); await settle();
  await c.setFilters(kept); await settle();
  const first = c.getState().queryId;
  const back = {line:50, select:true, offset:2};
  // Typing takes the results away once; more of it changes nothing.
  c.invalidateQuery({kept:{place:back}}); c.invalidateQuery({kept:{place:null}});
  assert.equal(c.getState().count, 0);
  refuse = SIZE; await c.setFilters(long); await settle();
  let s = c.getState();
  assert.deepEqual([s.queryId, s.queryStatus, s.count, s.queryComplete, s.queryAccepted, s.error, s.rejected], [first, 'complete', 10, true, true, '', SIZE]);
  assert.deepEqual([s.selectedLine, s.viewportStart, s.scrollOffset], [50, 4, 2], 'the list returns to its place');
  assert.equal(c.filters().query, 'frames');
  // Refused again, also after the engine was busy for a moment.
  c.invalidateQuery({kept:{place:null}});
  refuse = 'Local engine is busy; retry'; await c.setFilters(long); await settle();
  refuse = SIZE; run(250); await settle();
  assert.deepEqual([c.getState().queryId, c.getState().count], [first, 10]);
  // An accepted search replaces it.
  c.invalidateQuery({kept:{place:null}}); refuse = ''; await c.setFilters({query:'other'}); await settle();
  const second = c.getState().queryId;
  assert.notEqual(second, first); assert.equal(c.getState().rejected, '');
  // Any other error keeps nothing.
  c.invalidateQuery({kept:{place:null}}); refuse = 'Request failed'; await c.setFilters(long); await settle();
  assert.deepEqual([c.getState().queryStatus, c.getState().count], ['error', 0]);
  refuse = SIZE; await c.setFilters(long); await settle();
  assert.deepEqual([c.getState().queryStatus, c.getState().count], ['error', 0], 'nor later');
  // A cleared search has no results to go back to.
  refuse = ''; await c.setFilters(kept); await settle();
  c.invalidateQuery({kept:{place:null}}); c.invalidateQuery();
  refuse = SIZE; await c.setFilters(long); await settle();
  assert.deepEqual([c.getState().queryStatus, c.getState().count], ['error', 0]);
  await c.close({remote:false});
});

test('a search replaced while its answer is awaited is none to go back to', async () => {
  // The engine may have taken it: the results before are gone there.
  const id = {datasetId:1}; let engine = 0, hold = null, refuse = false, next = 0;
  const transport = async b => {
    if (b.action === 'status') return {...id, name:'synthetic', total:100, size:1000, status:'ready', complete:true, textComplete:true, query:{id:engine, status:'complete', count:10}};
    if (b.action === 'query') {
      if (refuse) throw Object.assign(new Error('Local request failed'), {userMessage:'Filter budget exceeded: query'});
      engine = b.queryId; if (hold) await hold;
      return {...id, queryId:b.queryId, query:{id:b.queryId, status:'complete', count:10}};
    }
    return {...id, queryId:b.queryId};
  };
  const c = createLargeLogController({transport, pane:'search', setTimer:() => ++next, clearTimer:() => {}});
  await c.start(); await settle();
  await c.setFilters({query:'a'}); await settle();
  c.invalidateQuery({kept:{place:null}});
  hold = new Promise(() => {}); void c.setFilters({query:'b'}); await settle();
  c.invalidateQuery({kept:{place:null}});
  refuse = true; await c.setFilters({query:'long'}); await settle();
  assert.deepEqual([c.getState().queryStatus, c.getState().count], ['error', 0]);
  await c.close({remote:false});
});
