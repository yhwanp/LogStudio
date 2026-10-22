import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as nextTurn} from 'node:timers/promises';
import {readFile} from 'node:fs/promises';
import {mountLargeLogView, highlightedSpan} from '../dist/large-view.js';
import {createLargeLogController, recordSpan, PAGE_SIZE, DETAIL_BYTES} from '../dist/controller.js';
import {describeLargeProgress} from '../dist/describe.js';

const ID = {datasetId:'dataset-one'};
const row = lineNumber => ({lineNumber, raw:`raw ${lineNumber}`, kind:'raw'});
const engineError = message => Object.assign(new Error('Local request failed'), {userMessage:message});
const deferred = () => {let resolve, reject; const promise = new Promise((yes, no) => {resolve = yes; reject = no;}); return {promise, resolve, reject};};
async function settle() {for (let i = 0; i < 5; i++) await nextTurn();}
function clock() {
  const pending = new Map(); let next = 0;
  return {pending, setTimer(fn, ms) {const id = ++next; pending.set(id, {fn, ms}); return id;}, clearTimer(id) {pending.delete(id);}, fire() {const [id, task] = pending.entries().next().value; pending.delete(id); task.fn();}};
}
function server(total=1000) {
  let queryId = 0;
  const requests = [], timers = clock();
  const state = {status:'ready', complete:true, total, count:total, queryStatus:'complete'};
  const transport = async body => {
    requests.push({body});
    if (body.action === 'status') return ({...ID, name:'large.log', size:1000000, indexedBytes:1000000, total:state.total, summary:{counts:{}}, status:state.status, complete:state.complete, query:{id:queryId, status:state.queryStatus, count:state.count, scannedBytes:1000000}});
    if (body.action === 'query') {queryId = body.queryId; return ({...ID, queryId, status:state.queryStatus, count:state.count, scannedBytes:1000000});}
    if (body.action === 'page') return ({...ID, queryId:body.queryId, start:body.start, count:state.count, complete:state.complete, rows:Array.from({length:Math.min(body.count, Math.max(0, state.count-body.start))}, (_, offset) => row(body.start + offset + 1))});
    if (body.action === 'range' || body.action === 'record') return ({...ID, queryId:body.queryId, lineNumber:body.lineNumber, startByte:body.startByte, text:'detail <script>', nextByte:body.startByte + 15, totalBytes:100000, complete:false});
    if (body.action === 'cancel') return ({...ID, queryId:body.queryId, status:state.status});
    if (body.action === 'close') return ({closed:true});
    throw new Error('Unexpected action');
  };
  return {requests, timers, transport, state};
}

function progressState(controller, count, status='running', revision=undefined) {
  const state=controller.getState();
  return {...ID,revision,name:'large.log',size:1000000,indexedBytes:1000000,total:1000,status:'ready',textComplete:true,summaryComplete:true,complete:true,query:{id:state.queryId,status,count,scannedBytes:1000000,complete:status==='complete'}};
}
function flushProgress(timers) {
  const entry=[...timers.pending].find(([,task])=>task.ms===0);assert.ok(entry,'notification must schedule one bounded flush');
  timers.pending.delete(entry[0]);entry[1].fn();
}

test('first/terminal notifications coalesce without extra status traffic and preserve terminal order',async()=>{
  const api=server();api.state.queryStatus='running';api.state.count=0;
  const changes=[];const c=createLargeLogController({pane:'search',viewportManaged:true,transport:api.transport,...api.timers,onChange:s=>changes.push(s)});
  await c.start();await settle();
  const initialRequests=api.requests.length;
  for(let i=1;i<=100;i++)c.acceptProgress(progressState(c,i));
  assert.equal([...api.timers.pending.values()].filter(t=>t.ms===0).length,1);
  flushProgress(api.timers);assert.equal(c.getState().count,100);assert.equal(c.getState().queryComplete,false);
  c.acceptProgress(progressState(c,1000,'complete'));
  c.acceptProgress(progressState(c,1,'running')); // Delayed first-available notification.
  flushProgress(api.timers);assert.equal(c.getState().count,1000);assert.equal(c.getState().queryComplete,true);
  assert.equal(api.timers.pending.size,0,'finished source/query should not keep timers');
  assert.equal(api.requests.length,initialRequests,'native snapshots need no new status request');
  assert.ok(changes.some(s=>s.count===100&&!s.queryComplete));
  await c.close({remote:false});
});

test('completion notification can overtake query reply and pending status without losing exactness',async()=>{
  const api=server(),query=deferred();let queryBody,statusReply=null;
  const c=createLargeLogController({pane:'search',viewportManaged:true,...api.timers,transport:b=>{
    if(b.action==='query'){queryBody=b;return query.promise;}
    if(b.action==='status'&&statusReply)return statusReply.promise;
    return api.transport(b);
  }});
  await c.start();await settle();assert.ok(queryBody);
  statusReply=deferred();const polling=c.start();await settle();
  c.acceptProgress(progressState(c,1000,'complete',2));flushProgress(api.timers);
  query.resolve({...ID,queryId:queryBody.queryId,query:{id:queryBody.queryId,status:'running',count:0,scannedBytes:0,complete:false}});await settle();
  statusReply.resolve({...ID,revision:1,status:'indexing',complete:false,textComplete:false,query:{id:queryBody.queryId,status:'running',count:0,complete:false}});await polling;
  assert.equal(c.getState().queryStatus,'complete');assert.equal(c.getState().queryComplete,true);assert.equal(c.getState().textComplete,true);assert.equal(c.getState().count,1000);
  await c.close({remote:false});
});

test('an older pushed snapshot cannot roll back source progress applied by a newer poll',async()=>{
  const api=server(),changes=[];api.state.queryStatus='running';api.state.count=0;
  let revision=0;
  const c=createLargeLogController({pane:'search',viewportManaged:true,...api.timers,onChange:s=>changes.push(s),transport:async b=>{
    const reply=await api.transport(b);
    return b.action==='status'?{...reply,revision:++revision}:reply;
  }});
  await c.start();await settle();
  api.state.queryStatus='complete';api.state.count=1000;await c.start();await settle();
  assert.equal(c.getState().textComplete,true);assert.equal(c.getState().queryComplete,true);
  const before=changes.length;
  c.acceptProgress({...progressState(c,1,'running',1),status:'indexing',indexedBytes:10,textComplete:false,summaryComplete:false,complete:false});
  assert.equal([...api.timers.pending.values()].some(t=>t.ms===0),false,'an already superseded snapshot is not queued');
  c.acceptProgress({...progressState(c,1000,'complete',revision),status:'indexing',textComplete:false});
  assert.equal([...api.timers.pending.values()].some(t=>t.ms===0),false,'an equal revision was already applied');
  assert.equal(changes.length,before);assert.equal(c.getState().textComplete,true);assert.equal(c.getState().indexedBytes,1000000);
  // A queued snapshot is rechecked when its flush runs after a newer poll.
  c.acceptProgress(progressState(c,1000,'complete',revision+1));api.state.count=1000;await c.start();await settle();
  flushProgress(api.timers);assert.equal(c.getState().queryComplete,true);
  await c.close({remote:false});
});

test('cancel, query replacement, Close/reopen and stale identities reject queued or late notifications',async()=>{
  const api=server();api.state.queryStatus='running';api.state.count=0;
  const c=createLargeLogController({pane:'search',viewportManaged:true,transport:api.transport,...api.timers});
  await c.start();await settle();const old=progressState(c,1000,'complete');
  c.acceptProgress(old);await c.cancel();
  assert.equal([...api.timers.pending.values()].some(t=>t.ms===0),false);
  c.acceptProgress(old);assert.equal(c.getState().queryStatus,'cancelled');
  await c.setFilters({query:'new'});c.acceptProgress(old);c.acceptProgress({...progressState(c,1000,'complete'),datasetId:'dataset-two'});
  assert.equal(c.getState().count,0);
  c.acceptProgress(progressState(c,1000,'complete'));await c.close({remote:false});assert.equal(api.timers.pending.size,0);
  c.acceptProgress(old);assert.equal(api.timers.pending.size,0);
  await c.reopen({query:'reopen'});await settle();
  assert.equal(c.getState().count,0);assert.equal(c.getState().queryStatus,'running');
  c.acceptProgress(progressState(c,0,'complete'));flushProgress(api.timers);
  assert.equal(c.getState().queryComplete,true);assert.equal(c.getState().count,0,'exact zero completion is delivered');
  await c.close({remote:false});
});

test('missing notifications retain the existing 100ms fallback rather than permanent faster polling',async()=>{
  const api=server();api.state.queryStatus='running';api.state.count=0;
  const c=createLargeLogController({pane:'search',viewportManaged:true,transport:api.transport,...api.timers});
  await c.start();await settle();assert.deepEqual([...api.timers.pending.values()].map(t=>t.ms),[100]);
  api.state.queryStatus='complete';api.state.count=1000;api.timers.fire();await settle();
  assert.equal(c.getState().queryComplete,true);assert.equal(api.timers.pending.size,0);await c.close({remote:false});
});
async function readyRig(total=1000) {
  const api = server(total), changes = [];
  const controller = createLargeLogController({ transport:api.transport, onChange:state => changes.push(state), ...api.timers});
  await controller.start(); await settle();
  return {...api, controller, changes};
}

test('huge logical counts use bounded viewport reads and seek positions', async () => {
  const rig = await readyRig(9_000_000_000);
  assert.equal(rig.controller.getState().count, 9_000_000_000);
  const page = await rig.controller.fetchRows(8_999_999_800);
  assert.equal(page.start, 8_999_999_800); assert.equal(page.rows.length, 200);
  rig.controller.seek(8_000_000_000); assert.equal(rig.controller.getState().viewportStart, 8_000_000_000);
  rig.controller.seek(9_500_000_000); assert.equal(rig.controller.getState().viewportStart, 8_999_999_999);
  assert.ok(rig.requests.filter(r => r.body.action === 'page').every(r => r.body.count === PAGE_SIZE));
  assert.equal(rig.requests.find(r => r.body.action === 'query').body.filters.recognizeLineNumbers, null);
  assert.equal('locale' in rig.requests.find(r => r.body.action === 'query').body.filters, false);
  await rig.controller.close();
});

test('latest query ignores older responses even when transport ignores abort', async () => {
  const pending = [];
  let intercept = null;
  const api = server();
  const controller = createLargeLogController({transport:(body, signal)=>intercept ? intercept(body, signal) : api.transport(body), ...api.timers});
  await controller.start(); await settle();
  intercept = (body, signal) => {const d = deferred(); pending.push({body, signal, ...d}); return d.promise;};
  const oldQuery = controller.setFilters({query:'old'}), newQuery = controller.setFilters({query:'new', recognizeLineNumbers:true});
  const [old, newer] = pending;
  assert.equal(old.signal.aborted, true);
  assert.equal(newer.body.filters.recognizeLineNumbers, true);
  newer.resolve({...ID, queryId:newer.body.queryId, status:'complete', count:1000}); await newQuery;
  old.resolve({...ID, queryId:old.body.queryId, status:'complete', count:1}); await oldQuery;
  assert.equal(controller.getState().count, 1000); assert.equal(controller.getState().queryId, newer.body.queryId);
  intercept = null; await controller.close();
});

test('stale dataset and query identity cannot supply viewport rows or detail', async () => {
  const api = server(), pending = []; let intercept = false;
  const controller = createLargeLogController({transport:body=>{if (!intercept) return api.transport(body); const d=deferred(); pending.push({body, ...d}); return d.promise;}, ...api.timers});
  await controller.start(); await settle();
  intercept = true;
  const rows = controller.fetchRows(200); pending[0].resolve({datasetId:'dataset-two', queryId:1, start:200, count:1000, rows:[row(999)]});
  assert.equal(await rows, null);
  const detail = controller.loadDetail(1); pending[1].resolve({...ID, queryId:99, lineNumber:1, startByte:0, text:'stale', nextByte:5, totalBytes:5, complete:true}); await detail;
  assert.equal(controller.getState().detail.text, '');
  intercept = false; await controller.close();
});

test('detail ranges are bounded and newer selection supersedes an old chunk', async () => {
  const rig = await readyRig(); await rig.controller.loadDetail(1);
  const request = rig.requests.find(r => r.body.action === 'record');
  assert.equal(request.body.count, DETAIL_BYTES); assert.equal(request.body.queryId, 1);
  assert.equal(rig.controller.getState().detail.text, 'detail <script>');
  await rig.controller.loadDetail(1, rig.controller.getState().detail.nextByte);
  assert.equal(rig.controller.getState().detail.startByte, 15);
  await rig.controller.previousDetail(); assert.equal(rig.controller.getState().detail.startByte, 0);
  rig.controller.closeDetail(); assert.equal(rig.controller.getState().detail, null);
  await rig.controller.close();
});

test('engine errors are plain and bounded',async()=>{
  const timers=clock();
  const controller=createLargeLogController({...timers,transport:async body=>{
    if(body.action==='close')return {};
    throw engineError(`Unsupported Unicode casing <script> ${'x'.repeat(1000)}`);
  }});
  await controller.start();assert.ok(controller.getState().error.startsWith('Unsupported Unicode casing <script>'));
  assert.ok(controller.getState().error.length<=500);
  await controller.close();assert.equal(timers.pending.size,0);
});

test('a busy engine query retries with a newer query id and close cancels the retry',async()=>{
  const api=server(), queryIds=[];let stopOnce=true;
  const controller=createLargeLogController({...api.timers,transport:body=>{
    if(body.action==='query'){queryIds.push(body.queryId);if(stopOnce){stopOnce=false;return Promise.reject(engineError('Local engine is busy; retry'));}}
    return api.transport(body);
  }});
  await controller.start();await settle();
  const retry=[...api.timers.pending].find(([,task])=>task.ms===250);assert.ok(retry);
  api.timers.pending.delete(retry[0]);retry[1].fn();await settle();
  assert.deepEqual(queryIds,[1,2]);assert.equal(controller.getState().count,1000);assert.equal(controller.getState().queryStatus,'complete');
  stopOnce=true;await controller.setFilters({query:'another'});assert.ok([...api.timers.pending.values()].some(task=>task.ms===250));
  await controller.close();assert.equal(api.timers.pending.size,0);
});

test('busy retries are latest-only and terminate visibly after bounded exhaustion',async()=>{
  const api=server(), queryIds=[];let busy=false;
  const controller=createLargeLogController({...api.timers,transport:body=>{
    if(body.action==='query'){queryIds.push(body.queryId);if(busy)return Promise.reject(engineError('Local engine is busy; retry'));}
    return api.transport(body);
  }});
  await controller.start();await settle();busy=true;await controller.setFilters({query:'busy'});
  for(const ms of [250,500,1000]){
    const retry=[...api.timers.pending].find(([,task])=>task.ms===ms);assert.ok(retry,`missing ${ms}ms retry`);
    api.timers.pending.delete(retry[0]);retry[1].fn();await settle();
  }
  assert.deepEqual(queryIds,[1,2,3,4,5]);assert.equal(controller.getState().queryStatus,'error');assert.equal(controller.getState().error,'Local engine is busy; retry');
  while(api.timers.pending.size){api.timers.fire();await settle();}
  assert.equal(controller.getState().queryStatus,'error');
  await controller.setFilters({query:'old pending'});busy=false;await controller.setFilters({query:'new query'});await settle();
  assert.equal(controller.getState().queryStatus,'complete');assert.ok(![...api.timers.pending.values()].some(task=>task.ms===250));
  await controller.close();
});

test('a complete query stops polling when indexing is complete; query cancellation preserves partial counts',async()=>{
  const api=server();
  const controller=createLargeLogController({transport:api.transport,...api.timers});
  await controller.start();await settle();if(api.timers.pending.size){api.timers.fire();await settle();}
  assert.equal(api.timers.pending.size,0);await controller.cancel();
  const cancel=api.requests.find(r=>r.body.action==='cancel');assert.equal(cancel.body.queryId,1);
  assert.equal(controller.getState().queryStatus,'cancelled');assert.equal(controller.getState().count,1000);
  await controller.close();
});

test('partial counts refresh on bounded polling, then polling stops on completion', async () => {
  const api = server(); Object.assign(api.state, {status:'indexing', complete:false, count:2, total:2, queryStatus:'running'});
  const controller = createLargeLogController({ transport:api.transport, pollMs:10, ...api.timers});
  await controller.start(); await settle();
  assert.equal(controller.getState().count, 2); assert.equal(controller.getState().complete, false);
  assert.equal(api.timers.pending.size, 1); assert.equal([...api.timers.pending.values()][0].ms, 50);
  Object.assign(api.state, {status:'ready', complete:true, count:5, total:5, queryStatus:'complete'});
  api.timers.fire(); await settle();
  assert.equal(controller.getState().count, 5); assert.equal(api.timers.pending.size, 0);
  await controller.close();
});

test('close aborts requests and timers and never restarts work', async () => {
  const d = deferred(), calls = [], timers = clock();
  const controller = createLargeLogController({transport:(body, signal)=>{calls.push({body, signal}); return body.action === 'close' ? Promise.resolve({closed:true}) : d.promise;}, ...timers});
  const pending = controller.start(); await controller.close();
  assert.equal(calls[0].signal.aborted, true); assert.equal(timers.pending.size, 0);
  d.resolve({...ID, status:'indexing', complete:false, query:{id:0}}); await pending;
  await controller.setFilters({query:'ignored'}); await controller.start();
  assert.equal(calls.length, 2); assert.equal(controller.getState().closed, true);
});

test('empty desktop and closed files never report file processing or partial results', async () => {
  const timers=clock();
  const controller=createLargeLogController({...timers,transport:async()=>({empty:true})});
  const states=[controller.getState()];
  await controller.start();states.push(controller.getState());
  for (const state of states) {
    const progress=describeLargeProgress(state);
    assert.equal(progress.count,'0건');assert.equal(progress.range,'');assert.equal(progress.query,'');
    assert.match(progress.status,/파일을 열거나/);assert.doesNotMatch(progress.status,/읽는 중|집계|%/);
  }
  await controller.close();
  const progress=describeLargeProgress(controller.getState());
  assert.equal(progress.count,'0건');assert.equal(progress.range,'');assert.equal(progress.query,'파일 닫힘');
  assert.doesNotMatch(progress.status,/읽는 중|집계|%/);
});

test('progress preserves partial counts and exact searches independently of level statistics', () => {
  const state={datasetId:1,status:'indexing',queryStatus:'running',queryComplete:false,textComplete:false,summaryComplete:false,total:20,count:12,size:100,indexedBytes:30};
  let progress=describeLargeProgress(state);
  assert.equal(progress.exact,false);assert.match(progress.count,/현재까지 12건/);
  assert.match(progress.query,/부분 결과/);assert.match(progress.status,/30%/);
  assert.doesNotMatch(progress.status,/ B|KiB|분석 완료/);
  assert.match(describeLargeProgress({...state,indexedBytes:100}).status,/99%/);
  progress=describeLargeProgress({...state,textComplete:true,queryComplete:true,queryStatus:'complete'});
  assert.equal(progress.exact,true);assert.match(progress.count,/전체 12건/);
  assert.match(progress.status,/파일 20행.*집계 중/);
  progress=describeLargeProgress({...state,textComplete:true,queryStatus:'cancelled',analysis:{status:'complete'}});
  assert.equal(progress.exact,false);assert.match(progress.query,/취소됨.*부분 결과/);
  assert.match(progress.status,/집계 취소됨/);
  progress=describeLargeProgress({...state,inputPending:true,queryComplete:true,textComplete:true,queryStatus:'complete'});
  assert.equal(progress.exact,false);assert.match(progress.count,/이전 검색/);assert.equal(progress.range,'새 조건 적용 대기');
});

test('upper full log total suppresses only a semantically duplicate exact file count',()=>{
  const state={datasetId:1,status:'ready',queryStatus:'complete',queryComplete:true,textComplete:true,summaryComplete:true,fieldFiltering:false,total:26166491,count:26166491,size:100,indexedBytes:100};
  const upper=s=>describeLargeProgress(s,{omitDuplicateFileCount:true});
  assert.equal(upper(state).count,'전체 26,166,491건');assert.equal(upper(state).status,'');
  assert.equal(describeLargeProgress(state).status,'파일 26,166,491행','other progress consumers retain file context');
  assert.equal(upper({...state,count:12,fieldFiltering:true}).status,'파일 26,166,491행');
  assert.equal(upper({...state,fieldFiltering:true}).status,'파일 26,166,491행','coincidentally equal filtered counts keep their distinct scope');
  assert.equal(upper({...state,count:12}).status,'파일 26,166,491행','different counts remain useful even without a field-filter flag');
  assert.equal(upper({...state,fieldFiltering:undefined}).status,'파일 26,166,491행','unknown scope is not presumed unfiltered');
  assert.equal(upper({...state,queryStatus:'running',queryComplete:false}).status,'파일 26,166,491행');
  assert.match(upper({...state,status:'indexing',textComplete:false,queryComplete:false,indexedBytes:30}).status,/파일 읽는 중 30%/);
  assert.equal(upper({...state,summaryComplete:false}).status,'레벨 집계 중','metadata loading remains visible without a leading separator');
  assert.equal(upper({...state,inputPending:true}).status,'파일 26,166,491행');
  assert.equal(upper({...state,queryStatus:'cancelled',queryComplete:false}).status,'파일 26,166,491행');
});

test('compact search wording stays searching until exact completion and never disguises stopped results',()=>{
  const state={datasetId:1,status:'ready',queryStatus:'complete',queryComplete:true,textComplete:true,summaryComplete:true,count:12,total:20};
  const compact=s=>describeLargeProgress(s,{compactSearch:true});
  assert.equal(compact(state).query,'검색 완료');assert.equal(compact(state).count,'12건');
  for(const s of [{...state,inputPending:true},{...state,queryStatus:'running',queryComplete:false},{...state,textComplete:false,queryComplete:false}]){
    const p=compact(s);assert.equal(p.query,'검색 중');assert.equal(p.exact,false);assert.doesNotMatch(p.count+p.query+p.range,/이전 검색|현재까지/);
  }
  assert.equal(compact({...state,inputPending:true}).count,'—','old count must not appear as the new count');
  assert.equal(compact({...state,count:0}).query,'검색 완료');
  assert.equal(compact({...state,count:0}).range,'검색 결과가 없습니다');
  assert.match(compact({...state,queryStatus:'cancelled',queryComplete:false}).query,/취소됨.*부분 결과/);
  assert.match(compact({...state,queryStatus:'error',queryComplete:false,error:'failed'}).query,/미완료.*부분 결과/);
  assert.match(compact({...state,inputPending:true,inputPaused:true}).query,/취소됨/);
  assert.equal(compact({...state,inputPending:true,inputInvalid:true}).query,'입력한 조건 미적용');
  assert.equal(compact({...state,closed:true}).query,'파일 닫힘');
});

test('active filter summary names only conditions that hide rows', async () => {
  const {describeActiveFilters, describeCondition} = await import('../dist/describe.js');
  const one = (c, includeRaw=true) => ({include:[c], exclude:[], includeRaw});
  assert.deepEqual(describeActiveFilters(one({levels:['V','D','I','W','E','F','A','U'], tag:' ', pid:'', tid:''})), []);
  assert.deepEqual(describeActiveFilters(one({levels:['W','E','F','A'], tag:'Wifi', pid:'12', tid:''}, false)).map(chip => chip.label), ['레벨: Warn 이상', '태그: Wifi', 'PID: 12', '미인식 줄 제외']);
  assert.equal(describeCondition({levels:['E']})[0].label, '레벨: Error');
  assert.equal(describeCondition({levels:['V','D','I']})[0].label, '레벨: V·D·I');
  assert.equal(describeCondition({levels:['V','D','I','W']})[0].label, '레벨: V·D·I·W', 'as many hidden as shown keeps the shown levels');
  assert.equal(describeCondition({levels:['D','I','W','E','F','A','U']})[0].label, '레벨: Verbose 제외');
  assert.equal(describeCondition({levels:['U','A','F','E','W','I']})[0].label, '레벨: Verbose·Debug 제외', 'selection order does not matter');
  assert.equal(describeCondition({levels:['W','E','F','A','U']})[0].label, '레벨: Verbose·Debug·Info 제외');
  assert.equal(describeCondition({levels:['V','E']})[0].label, '레벨: Verbose·Error');
  assert.equal(describeCondition({levels:[]})[0].label, '레벨: 없음');
});

test('each chip names its condition; switched off and empty conditions show none', async () => {
  const {describeActiveFilters, describeFilter} = await import('../dist/describe.js');
  const filters = {include:[{tag:'NAV'}, {levels:['E','F']}, {tag:'X', enabled:false}, {tag:' , '}], exclude:[{tag:'chatty'}, {levels:['V'], tag:'Binder'}], includeRaw:true};
  assert.deepEqual(describeActiveFilters(filters).map(chip => [chip.list, chip.index, chip.key, chip.label]), [
    ['include', 0, 'tag', '태그: NAV'], ['include', 1, 'levels', '레벨: Error·Fatal'],
    ['exclude', 0, 'tag', '태그: chatty'], ['exclude', 1, 'levels', '레벨: Verbose'], ['exclude', 1, 'tag', '태그: Binder']]);
  assert.equal(describeFilter(filters), '태그: NAV 또는 레벨: Error·Fatal · 제외 태그: chatty · 제외 (레벨: Verbose · 태그: Binder)');
  assert.equal(describeFilter({include:[{tag:'A', levels:['E']}, {tag:'B'}]}), '(레벨: Error · 태그: A) 또는 태그: B', 'a group of several fields is bracketed');
  assert.equal(describeFilter({include:[{tag:'A', levels:['E']}], includeRaw:false}), '레벨: Error · 태그: A · 미인식 줄 제외');
  assert.equal(describeFilter({include:[{}], exclude:[]}), '');
  assert.equal(describeActiveFilters({include:[{tag:'ECU1'}]}, 'dlt')[0].label, 'ID: ECU1');
});

test('tag and ID lists read like the core parses them and show as one chip per field', async () => {
  const {describeActiveFilters} = await import('../dist/describe.js');
  const {tagList, idList, idListError, tagListError, validateFilters, MAX_LIST_ITEMS, MAX_CONDITIONS} = await import('../dist/filter-model.js');
  assert.equal(MAX_LIST_ITEMS, 32);
  assert.deepEqual(tagList(' ActivityManager , MyApp,,MyApp, my app '), ['ActivityManager', 'MyApp', 'my app'], 'spaces stay inside a tag');
  assert.deepEqual(idList(' 2983, 3120\t2983,,　 41 '), ['2983', '3120', '41'], 'commas and whitespace separate IDs');
  assert.deepEqual([tagList(' , '), idList(' , ')], [[], []]);
  for (const ok of ['', ' ', '2983', '2983, 3120', '1 2 3', Array.from({length:32}, (_, i) => i).join(',')]) assert.equal(idListError(ok), '', ok);
  for (const bad of ['12a', '１２', '12, abc', '-1', '1;2']) assert.match(idListError(bad), /숫자만/, bad);
  assert.match(idListError(Array.from({length:33}, (_, i) => i).join(' ')), /최대 32개/);
  assert.equal(idListError(`${'7,'.repeat(40)}7`), '', 'duplicates do not count');
  assert.equal(tagListError(Array.from({length:32}, (_, i) => `t${i}`).join(',')), '');
  assert.match(tagListError(Array.from({length:33}, (_, i) => `t${i}`).join(',')), /최대 32개/);
  const tags = Array.from({length:33}, (_, i) => `t${i}`).join(',');
  assert.doesNotThrow(() => validateFilters({include:[{pid:'2983, 3120', tid:'1 2', tag:'a, b'}], exclude:[{tag:'c'}]}));
  for (const input of [{include:[{pid:'1, x'}]}, {exclude:[{tid:'x'}]}, {include:[{tag:tags}]}, {exclude:[{tag:tags}]}, {exclude:[{timeFrom:'25:00'}]}, {include:Array.from({length:MAX_CONDITIONS + 1}, () => ({}))}])
    assert.throws(() => validateFilters(input), JSON.stringify(input).slice(0, 40));
  const chips = describeActiveFilters({include:[{tag:' ActivityManager ,MyApp,MyApp', pid:'2983 3120', tid:'4241'}], exclude:[{tag:'chatty,,chatty'}]});
  assert.deepEqual(chips.map(chip => [chip.list, chip.key, chip.label]), [['include', 'tag', '태그: ActivityManager, MyApp'], ['include', 'pid', 'PID: 2983, 3120'], ['include', 'tid', 'TID: 4241'], ['exclude', 'tag', '태그: chatty']]);
  assert.equal(describeActiveFilters({include:[{tag:'  Wifi '}]})[0].label, '태그: Wifi', 'one value reads as before');
  const long = describeActiveFilters({include:[{pid:Array.from({length:32}, (_, i) => 1000 + i).join(',')}]})[0];
  assert.equal(long.label, `PID: ${Array.from({length:32}, (_, i) => 1000 + i).join(', ').slice(0, 40)}…`);
  assert.equal(long.full, `PID: ${Array.from({length:32}, (_, i) => 1000 + i).join(', ')}`, 'the tooltip keeps every item');
});

test('a detail or copy covers the entry span the engine reports, or only the requested row', () => {
  assert.deepEqual(recordSpan({first:3, last:9, recordComplete:false}, 5), {first:3, last:9, recordComplete:false});
  assert.deepEqual(recordSpan({first:3, last:9}, 9), {first:3, last:9, recordComplete:true});
  for (const reply of [{}, {first:6, last:9}, {first:3, last:4}, {first:0, last:5}, {first:'3', last:9}, null])
    assert.deepEqual(recordSpan(reply, 5), {first:5, last:5, recordComplete:true}, JSON.stringify(reply));
});

test('a logs query keeps the engine state of each condition it sent, and polls while one is pending',async()=>{
  const api=server();let states={include:['some','pending'],exclude:['none']};
  const transport=async body=>{
    const reply=await api.transport(body);
    if(body.action==='status')reply.query={...reply.query,complete:true,conditions:states};
    return reply;
  };
  const c=createLargeLogController({pane:'logs',transport,...api.timers});
  await c.start();await settle();
  await c.setFilters({include:[{tag:'NAV'},{tag:'Nothing'}],exclude:[{tag:'Ghost'},{tag:''},{tag:'Off',enabled:false}]});await settle();
  // The states refer to the conditions sent: enabled ones with criteria.
  assert.deepEqual(c.getState().checkedConditions.include.map(x=>x.tag),['NAV','Nothing']);
  assert.deepEqual(c.getState().checkedConditions.exclude.map(x=>x.tag),['Ghost']);
  assert.equal(c.getState().conditions,null,'unknown until the engine says');
  while(!c.getState().conditions)api.timers.fire(),await settle();
  assert.deepEqual(c.getState().conditions,{include:['some','pending'],exclude:['none']});
  // A complete query whose conditions are still checked keeps a slow poll.
  assert.equal(c.getState().queryStatus,'complete');
  assert.deepEqual([...api.timers.pending.values()].map(t=>t.ms),[800]);
  states={include:['some','none'],exclude:['none']};api.timers.fire();await settle();
  assert.deepEqual(c.getState().conditions.include,['some','none']);
  assert.equal(api.timers.pending.size,0,'no poll once every condition is known');
  // Malformed states are dropped; a replaced query starts unknown.
  c.acceptProgress({...progressState(c,1000,'complete'),query:{...progressState(c,1000,'complete').query,conditions:{include:['maybe'],exclude:[]}}});
  flushProgress(api.timers);assert.equal(c.getState().conditions,null);
  await c.setFilters({include:[{tag:'Other'}]});
  assert.equal(c.getState().conditions,null);assert.deepEqual(c.getState().checkedConditions.include.map(x=>x.tag),['Other']);
  await c.close({remote:false});
});
