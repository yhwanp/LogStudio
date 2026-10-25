import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as turn} from 'node:timers/promises';
import {emptyCondition, readCondition, isEmptyCondition, coreFilters, conditionsWithoutRows, MESSAGE_PATTERN_ERROR} from '../dist/filter-model.js';
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
