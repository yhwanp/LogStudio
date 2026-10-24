// One pane's query lifecycle against the native engine: status polling,
// pushed progress, paging, location and detail reads. No DOM access.
import {validateFilters, defaultFilters, normalizeFilters, coreFilters} from './filter-model.js';

export const PAGE_SIZE = 200;
export const DETAIL_BYTES = 65536;
const POLL_MS = 800;
// How often a completed single file is checked for appended rows.
const WATCH_MS = 2000;
export const DETAIL_SETTLE_MS = 120;
const MAX_QUERY_RETRIES = 3;
export const BUSY = 'Local engine is busy; retry';
export const FINAL_QUERY_STATES = new Set(['complete', 'cancelled', 'error']);
// The page's filter with nothing set; copy it with `defaultFilters()`.
export const DEFAULT_FILTERS = Object.freeze(defaultFilters());
const number = value => Number.isSafeInteger(value) && value >= 0;
const CAPTURE_STATES = new Set(['recording', 'waiting', 'stopped', 'failed']);
// A device capture as the engine reports it; null for any other log. A
// break is where the device left and came back: `row` is the first after it,
// and `gap` says rows logged meanwhile were not recorded.
export function captureState(value) {
  if (!value || typeof value !== 'object' || !CAPTURE_STATES.has(value.state)) return null;
  const text = (v, n = 300) => typeof v === 'string' ? v.slice(0, n) : '';
  const time = v => Number.isSafeInteger(v) ? v : null;
  const breaks = Array.isArray(value.breaks) ? value.breaks.slice(0, 1000).filter(b => number(b?.row) && b.row > 0).map(b => ({row:b.row, lost:time(b.lost), resumed:time(b.resumed), reboot:b.reboot === true, gap:b.gap === true})) : [];
  // Apps the device started with their private values shown.
  const launches = Array.isArray(value.launches) ? value.launches.slice(0, 1000).filter(l => number(l?.row) && l.row > 0).map(l => ({row:l.row, at:time(l.at), app:text(l.app, 120), process:text(l.process, 120)})) : [];
  return {state:value.state, device:text(value.device), serial:text(value.serial), bytes:number(value.bytes) ? value.bytes : 0, error:text(value.error, 500), breaks, launches, lost:time(value.lost), reason:text(value.reason), unsaved:value.unsaved === true, recovered:value.recovered === true, platform:value.platform === 'ios' ? 'ios' : 'android'};
}
const CONDITION_STATES = new Set(['some', 'none', 'pending', null]);
// The engine's state of each condition a logs pane query sent, or null.
function conditionStates(value) {
  const list = v => Array.isArray(v) && v.length <= 64 && v.every(s => CONDITION_STATES.has(s)) ? [...v] : null;
  const include = list(value?.include), exclude = list(value?.exclude);
  return include && exclude ? {include, exclude} : null;
}
const identity = value => (typeof value === 'string' && value.length > 0 && value.length <= 128) || number(value);

// The physical rows of the entry a detail or copy read covers; a reply
// without a valid span covers the requested row only.
export function recordSpan(data, lineNumber) {
  const valid = Number.isSafeInteger(data?.first) && Number.isSafeInteger(data?.last) && data.first >= 1 && data.first <= lineNumber && lineNumber <= data.last;
  return valid ? {first:data.first, last:data.last, recordComplete:data.recordComplete !== false} : {first:lineNumber, last:lineNumber, recordComplete:true};
}

export function createLargeLogController({transport, onChange=()=>{}, onLocate=()=>{}, setTimer=globalThis.setTimeout, clearTimer=globalThis.clearTimeout, pollMs=POLL_MS, pane=null}) {
  let disposed = false, timer = null, retryTimer = null, statusBusy = false, serial = 0, detailSerial = 0, navigationSerial = 0;
  let progressTimer = null, pendingProgress = null, statusRevision = 0, queryInvalidated = false;
  let detailHistory = [], locationBusy = false, locationNearest = false, locationRestore = null, searchMarks = null;
  let querySequence = 0, desiredFilters = defaultFilters();
  const requests = new Map();
  const initialState = {datasetId:null, name:'', files:[], size:0, status:'connecting', indexedBytes:0, total:0, summary:null, complete:false, textComplete:false, summaryComplete:false, analysis:null, queryComplete:false, fieldFiltering:false, queryId:0, queryAccepted:false, queryStatus:'idle', caughtUp:false, count:0, scannedBytes:0, viewportStart:0, scrollRevision:0, scrollAlign:'start', selectedIndex:null, locationPending:null, detail:null, selectedLine:null, error:'', closed:false, followable:false, following:false, grown:false, followError:'', tailRevision:0, format:'text', ios:false, conditions:null, checkedConditions:null, capture:null};
  const state = {...initialState};
  const snapshot = () => ({...state, detail:state.detail && {...state.detail}});
  const emit = () => { if (!disposed) onChange(snapshot()); };
  const matches = (response, expected, withQuery=false) => !disposed && response?.datasetId === expected.datasetId && (!withQuery || response.queryId === expected.queryId) && state.datasetId === expected.datasetId && (!withQuery || state.queryId === expected.queryId);
  const capture = () => ({datasetId:state.datasetId, queryId:state.queryId});
  const safeMessage = value => typeof value === 'string' ? value.slice(0, 500) : '';
  const cancelRequests = kinds => {for (const [controller, kind] of requests) if (!kinds || kinds.includes(kind)) controller.abort();};
  const fail = error => {
    if (disposed || error?.name === 'AbortError') return;
    state.error = safeMessage(error?.userMessage) || '로그를 읽을 수 없습니다. 파일을 다시 열어 주세요.';
    emit();
  };
  async function request(action, values={}, kind=action) {
    if (disposed) return null;
    const controller = new AbortController();
    requests.set(controller, kind);
    try {
      for(let attempt=0;;attempt++) {
        try {
          const data = await transport({action, ...(pane ? {pane} : {}), ...values}, controller.signal);
          return disposed ? null : data;
        } catch (error) {
          if (error?.userMessage !== BUSY || !['status','page','range','record','locate','lines'].includes(action) || attempt >= 2) throw error;
          await new Promise((resolve,reject)=>{
            const signal=controller.signal;
            if(signal.aborted)return reject(new DOMException('Aborted','AbortError'));
            const retry=setTimer(()=>{signal.removeEventListener('abort',stop);resolve();},50*(attempt+1));
            function stop(){clearTimer(retry);signal.removeEventListener('abort',stop);reject(new DOMException('Aborted','AbortError'));}
            signal.addEventListener('abort',stop,{once:true});
          });
        }
      }
    } finally { requests.delete(controller); }
  }
  function schedule() {
    if (disposed || queryInvalidated || timer !== null) return;
    if (['empty', 'cancelled', 'error', 'closed'].includes(state.status)) return;
    // A single opened file keeps a slow look for appended rows to announce.
    const watch = pane === 'logs' && state.followable && !state.following && !state.grown;
    if (!state.locationPending && state.complete && FINAL_QUERY_STATES.has(state.queryStatus)) {
      // Conditions that need rows read are known after the query. Pushed
      // progress tells; a slow poll covers a missed notification.
      const checking = ['include', 'exclude'].some(list => state.conditions?.[list]?.includes('pending'));
      if (checking || watch) timer = setTimer(() => { timer = null; void poll(); }, checking ? pollMs : WATCH_MS);
      return;
    }
    // Poll quickly until the first result or completion; optional metadata
    // and a followed file waiting for rows return to the slower cadence.
    const delay = state.locationPending || ((!state.count || state.queryStatus === 'running') && !state.caughtUp) ? Math.min(pollMs, 100) : pollMs;
    timer = setTimer(() => { timer = null; void poll(); }, Math.max(50, Math.min(2000, delay)));
  }
  function queryProgress(query, replaced=false) {
    if (!query || query.id !== state.queryId) return false;
    // A notification can overtake the initial query/status response. Only a
    // followed file's completed query runs again, over the rows that follow.
    if (FINAL_QUERY_STATES.has(state.queryStatus) && query.status === 'running' && !resumed(query)) return false;
    // The engine knows this identity; other panes may now refer to it.
    state.queryAccepted = true;
    const changed = number(query.count) && query.count !== state.count;
    // A replaced last row of a followed file may leave one result fewer.
    if (number(query.count)) state.count = replaced ? query.count : Math.max(state.count, query.count);
    state.caughtUp = query.caughtUp === true;
    if (number(query.scannedBytes)) state.scannedBytes = Math.max(state.scannedBytes, query.scannedBytes);
    if (typeof query.status === 'string') state.queryStatus = query.status;
    if (query.conditions !== undefined) state.conditions = conditionStates(query.conditions);
    state.queryComplete = state.textComplete && state.queryStatus === 'complete' && query.complete !== false;
    if (query.status === 'error' || (query.error && query.status !== 'cancelled')) state.error = safeMessage(query.error) || '검색을 완료할 수 없습니다. 읽기 옵션과 필터를 확인해 주세요.';
    return changed;
  }
  const resumed = query => state.following && state.queryStatus === 'complete' && query.status === 'running';
  function sourceProgress(data) {
    if (typeof data.name === 'string') state.name = data.name;
    // DLT files show their own columns; anything else reads as text.
    if (typeof data.format === 'string') state.format = data.format === 'dlt' ? 'dlt' : 'text';
    // Every file is an iOS log, whose levels go by Apple's names.
    if (typeof data.ios === 'boolean') state.ios = data.ios;
    state.followable = data.followable === true; state.following = data.following === true; state.grown = data.grown === true; state.followError = safeMessage(data.followError);
    // Every status of a capture's log carries the capture.
    if (data.capture !== undefined) state.capture = captureState(data.capture);
    const replaced = number(data.tailRevision) && data.tailRevision !== state.tailRevision;
    if (number(data.tailRevision)) state.tailRevision = data.tailRevision;
    // A combined log lists its files in order with each first physical row.
    if (Array.isArray(data.files)) state.files = data.files.slice(0, 64).filter(f => typeof f?.name === 'string' && number(f.size)).map(f => ({name:f.name.slice(0, 300), size:f.size, firstLine:number(f.firstLine) ? f.firstLine : null}));
    for (const key of ['size', 'indexedBytes', 'total']) if (number(data[key])) state[key] = data[key];
    state.status = typeof data.status === 'string' ? data.status : state.status;
    state.summary = data.summary ?? state.summary;
    state.complete = data.complete === true;
    state.textComplete = data.textComplete === undefined ? state.complete : data.textComplete === true;
    state.summaryComplete = data.summaryComplete === undefined ? state.complete : data.summaryComplete === true;
    state.analysis = data.analysis ?? state.analysis;
    if (data.error) state.error = safeMessage(data.error);
    else if (state.status === 'error') state.error = '파일을 읽을 수 없습니다. 파일을 확인하고 다시 열어 주세요.';
    return queryProgress(data.query, replaced);
  }
  function clearProgress() {
    if (progressTimer !== null) clearTimer(progressTimer);
    progressTimer = null; pendingProgress = null;
  }
  // Polled replies and pushed notifications race. Native snapshots carry a
  // per-dataset revision; only a newer snapshot may replace source/query state.
  const stale = data => number(data?.revision) && data.revision <= statusRevision;
  function applyStatus(data) {
    if (number(data.revision)) statusRevision = data.revision;
    sourceProgress(data); emit();
  }
  function followStatus() {
    if (state.locationPending && !locationBusy) void resolveLocation();
  }
  function acceptProgress(data) {
    if (queryInvalidated || !matches(data, capture()) || data?.query?.id !== state.queryId || !state.queryId || ['cancelled','error'].includes(state.queryStatus) || stale(data)) return;
    if (data.query.status === 'running' && !(data.following === true && state.queryStatus === 'complete') && (FINAL_QUERY_STATES.has(state.queryStatus) || (pendingProgress && FINAL_QUERY_STATES.has(pendingProgress.query.status)))) return;
    if (pendingProgress && number(pendingProgress.revision) && number(data.revision) && data.revision < pendingProgress.revision) return;
    pendingProgress = data;
    if (progressTimer !== null) return;
    // At most one queued render per pane; the native producer sends only first
    // availability and terminal state. Regular polling remains the fallback.
    progressTimer = setTimer(() => {
      progressTimer = null; const data = pendingProgress; pendingProgress = null;
      if (queryInvalidated || !matches(data, capture()) || data?.query?.id !== state.queryId || stale(data)) return;
      applyStatus(data); followStatus();
      if (timer !== null) clearTimer(timer); timer = null; schedule();
    }, 0);
  }
  async function poll({waitForQuery=false}={}) {
    if (disposed || queryInvalidated || statusBusy) return;
    statusBusy = true;
    const expected = capture(), observedSerial = serial;
    try {
      const data = await request('status');
      if (!data || disposed || observedSerial !== serial) return;
      if(data.empty){state.status='empty';state.count=state.total=0;emit();return;}
      if (expected.datasetId !== null && !matches(data, expected)) return;
      if (!identity(data.datasetId)) throw new Error('Invalid identity');
      if (stale(data)) return;
      if(!state.queryId&&number(data.query?.id))querySequence=Math.max(querySequence,data.query.id);
      if (state.datasetId === null) state.datasetId = data.datasetId;
      applyStatus(data);
      if (!state.queryId && !['cancelled', 'error', 'closed'].includes(state.status)) {const initial=setFilters(desiredFilters);if(waitForQuery)await initial;}
      else followStatus();
    } catch (error) {if (observedSerial === serial) fail(error);} finally {statusBusy = false; schedule();}
  }
  // Immediately discard the prior query on new intent, before debounce submits
  // its replacement. A reserved identity rejects late replies/events and clears
  // viewport caches once; repeated pending keystrokes do no extra clearing.
  function invalidateQuery(){
    if(disposed||queryInvalidated||!state.datasetId)return;
    const old=capture(),running=state.queryStatus==='running';
    queryInvalidated=true;clearProgress();serial++;detailSerial++;navigationSerial++;
    cancelRequests();if(timer!==null)clearTimer(timer);if(retryTimer!==null)clearTimer(retryTimer);timer=retryTimer=null;
    detailHistory=[];
    state.queryId=++querySequence;state.queryAccepted=false;state.queryStatus='idle';state.queryComplete=false;state.count=0;state.scannedBytes=0;
    state.viewportStart=0;state.scrollRevision++;state.scrollAlign='start';state.selectedIndex=null;
    state.locationPending=null;state.selectedLine=null;state.detail=null;state.error='';state.conditions=null;state.checkedConditions=null;emit();
    // Native cancellation is scoped to the discarded identity. A delayed cancel
    // cannot stop the replacement; source changes and already-replaced queries
    // may reject it harmlessly. It never publishes discarded state.
    if(running&&old.queryId)void request('cancel',old).catch(()=>{});
  }
  async function setFilters(filters, {retryAttempt=0}={}) {
    if (disposed) return;
    clearProgress();
    if (retryTimer !== null) clearTimer(retryTimer); retryTimer = null;
    validateFilters(filters); queryInvalidated=false;
    desiredFilters = normalizeFilters(filters);
    if (state.datasetId === null) return;
    cancelRequests(['query', 'range', 'viewport', 'locate']);
    detailSerial++; navigationSerial++;
    detailHistory = [];
    const expected = {...capture(), queryId:++querySequence};
    state.queryId = expected.queryId; state.queryAccepted = false; state.queryStatus = 'running'; state.count = 0;
    state.queryComplete = false;
    const sent = coreFilters(desiredFilters);
    state.fieldFiltering = !!(sent.include.length || sent.exclude.length || !sent.includeRaw);
    // The engine's condition states refer to these, by position.
    state.checkedConditions = {include:sent.include, exclude:sent.exclude}; state.conditions = null;
    state.viewportStart = 0; state.scrollRevision++; state.scrollAlign = 'start'; state.selectedIndex = null; state.locationPending = null; state.detail = null; state.selectedLine = null; state.scannedBytes = 0; state.error = '';
    const sequence = ++serial;
    if (timer !== null) clearTimer(timer); timer = null;
    emit(); schedule();
    try {
      const data = await request('query', {...expected, filters:sent});
      if (!data || sequence !== serial || !matches(data, expected, true)) return;
      queryProgress(data.query ?? {id:data.queryId, status:data.status, count:data.count, scannedBytes:data.scannedBytes, error:data.error});
      emit();
    } catch (error) {
      if (sequence !== serial) return;
      if (error?.userMessage === BUSY && retryAttempt < MAX_QUERY_RETRIES) {
        retryTimer = setTimer(() => {retryTimer = null; if (!disposed && sequence === serial) void setFilters(desiredFilters, {retryAttempt:retryAttempt + 1});}, Math.min(2000, 250 * 2 ** retryAttempt));
      } else {state.queryStatus = 'error'; fail(error);}
    }
  }
  // The continuous viewport owns row reads and its bounded cache.
  async function fetchRows(start) {
    if(disposed||!state.datasetId||!state.queryId||!number(start))return null;
    const expected=capture(),marks=searchMarks;
    const data=await request('page',{...expected,start,count:PAGE_SIZE,...(marks?{searchQueryId:marks}:{})},'viewport');
    return data&&matches(data,expected,true)?data:null;
  }
  // Upper pages may also mark the search pane's current results. The owner
  // re-reads its rows when this reports a change; a replaced search identity
  // simply returns unmarked rows.
  function setSearchMarks(queryId){const next=number(queryId)&&queryId>0?queryId:null;if(next===searchMarks)return false;searchMarks=next;return true;}
  // Copy reads use their own request kind so scrolling and seeking cannot
  // abort them; identity checks still reject replies from a replaced query.
  async function readRows(start){
    if(disposed||!state.datasetId||!state.queryId||!number(start))return null;
    const expected=capture();
    const data=await request('page',{...expected,start,count:PAGE_SIZE},'copy');
    if(!data||!matches(data,expected,true)||data.start!==start||!Array.isArray(data.rows))throw new Error('Copy source changed');
    return data.rows;
  }
  // Rows at ascending physical line numbers whatever this pane filters; a
  // replaced query or source answers null. `next` counts the lines answered.
  async function readLines(lineNumbers){
    if(disposed||!state.datasetId||!state.queryId||!Array.isArray(lineNumbers)||!lineNumbers.length)return null;
    const expected=capture();
    const data=await request('lines',{...expected,lineNumbers},'lines');
    return data&&matches(data,expected,true)&&Array.isArray(data.rows)&&number(data.next)?data:null;
  }
  async function readLineText(lineNumber){
    if(disposed||!state.datasetId||!state.queryId||!Number.isSafeInteger(lineNumber)||lineNumber<1)throw new Error('No line');
    const expected=capture();let text='',startByte=0;
    // A physical row is at most 1 MiB, so this loop is bounded.
    for(let i=0;i<20;i++){
      const data=await request('range',{...expected,lineNumber,startByte,count:DETAIL_BYTES},'copy');
      if(!data||!matches(data,expected,true)||data.lineNumber!==lineNumber||data.startByte!==startByte||typeof data.text!=='string'||!number(data.nextByte))throw new Error('Copy source changed');
      text+=data.text;
      if(data.complete===true||data.nextByte<=startByte)return text;
      startByte=data.nextByte;
    }
    return text;
  }
  // The whole log entry of a row: its entry row and continuation rows,
  // joined by LF. The core caps an entry at 1 MiB, so this loop is bounded.
  async function readRecordText(lineNumber){
    if(disposed||!state.datasetId||!state.queryId||!Number.isSafeInteger(lineNumber)||lineNumber<1)throw new Error('No line');
    const expected=capture();let text='',startByte=0,span=null;
    for(let i=0;i<40;i++){
      const data=await request('record',{...expected,lineNumber,startByte,count:DETAIL_BYTES},'copy');
      if(!data||!matches(data,expected,true)||data.lineNumber!==lineNumber||data.startByte!==startByte||typeof data.text!=='string'||!number(data.nextByte))throw new Error('Copy source changed');
      span??=recordSpan(data,lineNumber);text+=data.text;
      if(data.complete===true||data.nextByte<=startByte)break;
      startByte=data.nextByte;
    }
    return {text,...span};
  }
  function selectLine(line){
    if(disposed||!state.datasetId||!state.queryId||!Number.isSafeInteger(line)||line<1)return;
    navigationSerial++;cancelRequests(['locate']);state.locationPending=null;state.selectedLine=line;emit();
  }
  function rememberPosition(index){if(number(index))state.viewportStart=index;}
  function seek(index,{selectedLine=null,center=false,restore=false}={}){
    if(disposed||!number(index))return;
    navigationSerial++;cancelRequests(['viewport','locate']);
    state.locationPending=null;state.selectedLine=selectedLine;
    state.viewportStart=restore?index:Math.min(index,Math.max(0,state.count-1));state.scrollAlign=center?'center':'start';
    // A restored anchor is the first visible row, not the selected row's ordinal.
    state.selectedIndex=selectedLine&&!restore?state.viewportStart:null;state.scrollRevision++;emit();
  }
  async function resolveLocation(navigation=navigationSerial){
    if(disposed||locationBusy||!state.locationPending)return;
    locationBusy=true;const expected=capture(),line=state.locationPending,restore=locationRestore;
    // A restored row past the rows read so far waits for indexing; past the
    // end of a fully read (shorter) file, the list shows its end once the
    // query has counted it.
    const kept=selected=>selected&&(!state.textComplete||selected<=state.total)?selected:null;
    try{
      if(restore&&line>state.total){
        if(state.textComplete&&FINAL_QUERY_STATES.has(state.queryStatus)){state.locationPending=null;seek(Math.max(0,state.count-1),{selectedLine:kept(restore.selectedLine)});}
        return;
      }
      const data=await request('locate',{...expected,lineNumber:line});
      if(!data||navigation!==navigationSerial||!matches(data,expected,true)||state.locationPending!==line)return;
      if(data.pending)return;
      state.locationPending=null;
      // A restored place puts the row, or the next result after it, first.
      if(restore){if(number(data.index))seek(data.index,{restore:true,selectedLine:kept(restore.selectedLine)});else emit();return;}
      if(data.found&&number(data.index)){state.error='';seek(data.index,{selectedLine:line,center:true});onLocate({lineNumber:line,outcome:'found'});return;}
      // A row number typed by the user reports the outcome to the owner and
      // may select the next result instead: the engine's index of a missing
      // row is the number of results before it.
      if(locationNearest){
        state.selectedLine=null;
        if(data.incomplete!==true&&number(data.index)&&data.index<state.count){
          const page=await request('page',{...expected,start:data.index,count:1},'locate');
          if(navigation!==navigationSerial||!matches(page,expected,true)||state.locationPending!==null)return;
          const next=page.rows?.[0]?.lineNumber;
          if(page.start===data.index&&Number.isSafeInteger(next)&&next>line){seek(data.index,{selectedLine:next,center:true});onLocate({lineNumber:line,outcome:'nearest',target:next});return;}
        }
        emit();onLocate({lineNumber:line,outcome:data.incomplete===true?'incomplete':'hidden'});return;
      }
      if(data.incomplete===true){state.selectedLine=null;state.error='필터 처리가 완료되지 않아 선택한 행의 위치를 확인할 수 없습니다. 필터를 다시 적용해 주세요.';emit();}
      else{state.selectedLine=null;state.error='선택한 행을 현재 필터 결과에서 찾을 수 없습니다. 필터 완료 후 다시 시도해 주세요.';emit();}
    }catch(error){if(navigation===navigationSerial){state.locationPending=null;fail(error);}}
    // A location requested while this one was busy resolves now, not at the next poll.
    finally{locationBusy=false;if(state.locationPending&&navigation!==navigationSerial&&!disposed)void resolveLocation();schedule();}
  }
  async function locateLine(line,{nearest=false}={}){
    if(disposed||!state.datasetId||!state.queryId||!Number.isSafeInteger(line)||line<1)return;
    navigationSerial++;cancelRequests(['locate']);locationNearest=nearest;locationRestore=null;state.locationPending=line;state.selectedLine=line;emit();
    await resolveLocation();schedule();
  }
  // Returns to a saved place by physical rows: `line` (or the first result
  // after it) at the top and `selectedLine` selected, once the scan reaches
  // it. Any navigation before then replaces the restore.
  async function restorePosition(line,selectedLine=null){
    if(disposed||!state.datasetId||!state.queryId||!Number.isSafeInteger(line)||line<1)return;
    navigationSerial++;cancelRequests(['locate']);locationNearest=false;locationRestore={selectedLine:Number.isSafeInteger(selectedLine)&&selectedLine>0?selectedLine:null};
    state.locationPending=line;state.selectedLine=locationRestore.selectedLine;emit();
    await resolveLocation();schedule();
  }
  async function loadDetail(lineNumber, startByte=0, {delay=0}={}) {
    if (disposed || !Number.isSafeInteger(lineNumber) || lineNumber < 1 || !number(startByte)) return;
    const expected = capture(), sequence = ++detailSerial;
    cancelRequests(['range']);
    const previous = state.detail;
    if (!previous || previous.lineNumber !== lineNumber || startByte === 0) detailHistory = [];
    else if (startByte === previous.nextByte && !previous.loading) {detailHistory.push(previous.startByte); if (detailHistory.length > 32) detailHistory.shift();}
    state.detail = {lineNumber, startByte, text:'', loading:true, complete:false, nextByte:startByte, totalBytes:0, previousByte:detailHistory.at(-1) ?? 0}; emit();
    try {
      // A held arrow key selects rows faster than a native block read; read
      // only the row where the selection settles.
      if (delay > 0) {await new Promise(resolve => setTimer(resolve, delay)); if (disposed || sequence !== detailSerial) return;}
      const data = await request('record', {...expected, lineNumber, startByte, count:DETAIL_BYTES}, 'range');
      if (!data || sequence !== detailSerial || !matches(data, expected, true) || data.lineNumber !== lineNumber || data.startByte !== startByte) return;
      if (typeof data.text !== 'string' || !number(data.nextByte) || !number(data.totalBytes) || data.nextByte < startByte || data.nextByte > data.totalBytes || new TextEncoder().encode(data.text).length > DETAIL_BYTES) throw new Error('Invalid detail');
      state.detail = {lineNumber, ...recordSpan(data, lineNumber), ...(number(data.file) && number(data.fileLine) ? {file:data.file, fileLine:data.fileLine} : {}), startByte, text:data.text, encodingDamage:data.encodingDamage, encodingRepaired:data.encodingRepaired === true, sourceStartByte:data.sourceStartByte, highlights:Array.isArray(data.highlights) ? data.highlights : [], nextByte:data.nextByte, totalBytes:data.totalBytes, complete:data.complete === true, loading:false, previousByte:detailHistory.at(-1) ?? 0}; emit();
    } catch (error) {if (sequence === detailSerial) {if (state.detail) state.detail.loading = false; fail(error);}}
  }
  // Starts or stops following the open file; the reply is a status snapshot.
  async function setFollow(on) {
    if (disposed || !state.datasetId || state.closed) return;
    const expected = capture();
    try {
      const data = await request('follow', {datasetId:expected.datasetId, follow:on === true});
      if (!data || !matches(data, expected) || stale(data)) return;
      applyStatus(data);
      if (timer !== null) clearTimer(timer); timer = null; schedule();
    } catch (error) {fail(error);}
  }
  function previousDetail() {const detail = state.detail; if (!detail || detail.loading) return Promise.resolve(); return loadDetail(detail.lineNumber, detailHistory.pop() ?? 0);}
  function closeDetail() {detailSerial++; cancelRequests(['range']); detailHistory = []; state.detail = null; emit();}
  async function cancel({clearError=false}={}) {
    if (disposed || state.datasetId === null) return;
    if(queryInvalidated){state.queryStatus='cancelled';if(clearError)state.error='';emit();return;}
    const expected = capture();
    clearProgress();
    if (retryTimer !== null) clearTimer(retryTimer); retryTimer = null;
    const sequence = ++serial; detailSerial++; navigationSerial++; cancelRequests();
    state.detail = null; state.queryStatus = 'cancelled'; state.queryComplete = false; if(clearError)state.error=''; emit();
    try {
      const data = await request('cancel', expected);
      if (!data || !matches(data, expected, true)) return;
      state.status = data.status ?? state.status;
      state.queryStatus = 'cancelled';
      if (state.status === 'cancelled') state.error = '파일 읽기가 취소되었습니다. 로그 파일 열기에서 다시 선택해 주세요.';
      emit();
    } catch (error) {if (sequence === serial && state.queryId === expected.queryId) fail(error);}
    schedule();
  }
  function close({remote=true}={}) {
    if (disposed) return Promise.resolve();
    disposed = true; serial++; detailSerial++; navigationSerial++;
    clearProgress();
    if (timer !== null) clearTimer(timer); timer = null;
    if (retryTimer !== null) clearTimer(retryTimer); retryTimer = null; cancelRequests();
    state.detail = null; state.selectedLine = null; state.locationPending = null; state.closed = true;
    onChange(snapshot());
    if (!remote) return Promise.resolve();
    // The one cleanup request is best-effort.
    return Promise.resolve().then(() => transport({action:'close'})).catch(()=>{});
  }
  function reopen(filters=DEFAULT_FILTERS) {
    queryInvalidated=false;clearProgress();
    cancelRequests();serial++;detailSerial++;navigationSerial++;
    if(timer!==null)clearTimer(timer);if(retryTimer!==null)clearTimer(retryTimer);
    timer=retryTimer=null;statusBusy=false;disposed=false;
    querySequence=0;statusRevision=0;detailHistory=[];searchMarks=null;desiredFilters=normalizeFilters(filters);
    Object.assign(state,initialState,{detail:null});emit();return poll({waitForQuery:true});
  }
  return {reopen,start:poll, setFollow, acceptProgress, invalidateQuery, setFilters, fetchRows, readRows, readLines, readLineText, readRecordText, seek, rememberPosition, selectLine, locateLine, restorePosition, setSearchMarks, reportError:fail, loadDetail, previousDetail, closeDetail, cancel, close, getState:snapshot};
}
