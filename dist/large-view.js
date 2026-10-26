import {handleLogPaste, isEditableTarget, FILE_PASTE_FALLBACK} from './clipboard.js';
import {mountPasteConfirm, createPasteFlow, replacesLog, describeClosing} from './paste-confirm.js';
import {makeDemo} from './demo.js';
import {LEVELS, validateFilters, isTimeBound, isEmptyTimeRange, idListError, tagListError, tagList, idList, emptyCondition, isEmptyCondition, normalizeFilters, coreFilters, conditionsWithoutRows, MAX_CONDITIONS, MESSAGE_PATTERN_ERROR} from './filter-model.js';
import {mountLogLayout, widestLine} from './layout.js';
import {createContinuousViewport, ROW_HEIGHT} from './continuous-view.js';
import {selectOne, selectAll, extendTo, toggleRow, selectedRanges, selectedCount, inRanges} from './row-selection.js';
import {rowGestures} from './row-gestures.js';
import {createLargeLogController, DEFAULT_FILTERS, DETAIL_SETTLE_MS, FINAL_QUERY_STATES} from './controller.js';
import {LEVEL_NAMES, formatNumber, formatSize, describeLargeProgress, describeLogError, describeFileError, describeFollowError, describeActiveFilters, rowSource} from './describe.js';
import {textSpan, writeClipboard} from './dom.js';
import {createPopupMenu} from './menu.js';
import {mountRecentFiles} from './recent.js';
import {mountDevices, captureBreaks, breakLabel} from './devices.js';
import {mountExtras} from './extras.js';
import {mountSavedFilters, fieldsOf} from './saved-filters.js';
import {mountConditions, conditionSummary, rowlessText} from './conditions.js';
import {createBookmarks, MAX_BOOKMARKS} from './bookmarks.js';
import {createWorkSaver, readWork, defaultFilters, mergeReloadMarks, mergeSavedMarks, describeReloadMarks, describeRestore, describeMissingMarks, RELOADED, LINES_PER_REQUEST} from './work-state.js';
import {continuationOwner, continuationClass, joinsNext, ownerCells} from './entries.js';
import {DLT_IDS, dltCells, dltLevelTitle, levelNames, profileOf, useAppleLevels} from './columns.js';

const number = value => Number.isSafeInteger(value) && value >= 0;

// iOS rows keep their original level word in the tooltip, DLT messages
// without a log level their message type.
function levelSpan(document, entry) {
  const span = textSpan(document, levelNames(entry.dlt ? 'dlt' : 'text')[entry.level] ?? entry.level, `level level-${entry.level}`);
  const title = entry.levelText || dltLevelTitle(entry);
  if (title) span.title = title;
  return span;
}

// Offsets come from the same native matcher used for counts/pages. Render text
// nodes through textContent; never reinterpret source logs as HTML or JS regex.
export function highlightedSpan(document, text, className='', ranges=[], damage=null) {
  text = String(text ?? '');
  if (typeof damage?.mask === 'string' && /^[0-9a-f]*$/.test(damage.mask) && damage.mask.length <= 32768 && damage.mask.length) {
    const span=textSpan(document,'',className), valid=(Array.isArray(ranges)?ranges:[]).slice(0,32).filter(p=>Array.isArray(p)&&p.length===2&&p.every(Number.isSafeInteger));
    const damaged=i=>((parseInt(damage.mask.slice(Math.floor(i/8)*2,Math.floor(i/8)*2+2)||'00',16)>>(i%8))&1)!==0;
    const marked=i=>valid.some(([a,b])=>i>=a&&i<b);
    let from=0;
    while(from<text.length){
      const bad=damaged(from), hit=marked(from);let end=from+1;
      while(end<text.length&&damaged(end)===bad&&marked(end)===hit)end++;
      const part=textSpan(document,text.slice(from,end),bad?'encoding-damage':'');
      if(bad){const sample=damage.samples?.find(s=>s.position>=from&&s.position<end);part.title=sample?`손상 바이트 ${sample.hex} · 파일 바이트 위치 ${sample.sourceStartByte} (0부터)`:'손상 바이트를 표시용 �로 대체했습니다. 원본 파일은 유지됩니다.';}
      if(hit){const mark=document.createElement('mark');mark.append(part);span.append(mark);}else span.append(part);
      from=end;
    }
    return span;
  }
  if (!Array.isArray(ranges) || !ranges.length) return textSpan(document, text, className);
  const span = document.createElement('span'); span.className = className;
  let from = 0;
  for (const pair of ranges.slice(0, 32)) {
    if (!Array.isArray(pair) || pair.length !== 2 || !pair.every(Number.isSafeInteger)) continue;
    const start = Math.max(from, pair[0]), end = Math.min(text.length, pair[1]);
    if (start < from || start >= end || start > text.length) continue;
    if (start > from) span.append(textSpan(document, text.slice(from, start)));
    const mark = document.createElement('mark'); mark.textContent = text.slice(start, end); span.append(mark); from = end;
  }
  if (from < text.length) span.append(textSpan(document, text.slice(from)));
  return span;
}
function fieldSpan(document, entry, value, className) {
  const text = String(value ?? ''), raw = String(entry.raw ?? '');
  const at = entry.fieldOffsets?.[className] ?? (className === 'message' ? raw.lastIndexOf(text) : raw.indexOf(text));
  // Ambiguous repeated field text has no reliable raw offset; raw view retains
  // the exact complete highlight. Do not mark a different occurrence by guess.
  const unique = number(entry.fieldOffsets?.[className]) || className === 'message' || raw.indexOf(text, at + 1) === -1;
  const ranges = at < 0 || !text || !unique ? [] : (entry.highlights ?? []).map(([a,b]) => [Math.max(0,a-at),Math.min(text.length,b-at)]).filter(([a,b]) => a < b && b > 0 && a < text.length);
  const span = highlightedSpan(document, text, className, ranges, entry.fieldDamage?.[className]);
  span.title = text.length > 1000 ? `${text.slice(0, 1000)}…` : text;
  return span;
}

export function mountLargeLogView(native, {transport, tips=null, document=globalThis.document, window=globalThis.window}) {
  const $ = id => document.getElementById(id);
  $('largeLogPanel').hidden=false;
  const ids = ['largeName','largeStatus','largeError','largeCount','largeTotal','largeRows','largeViewport','largeFirst','largeLast','largeJump','largeJumpForm','largeCancel','largeClose','largeColumns','largeRaw','largeHeaders','largeRange','largeDetail','largeDetailTitle','largeDetailText','largeDetailPrevious','largeDetailNext','largeDetailClose','levelFilters','allLevels','errorLevels','resetFilters','searchInput','tagFilter','pidFilter','tidFilter','caseSensitive','includeRaw','recognizeLineNumbers','logsCount','logsRows','logsViewport','logsHeaders','logsFirst','logsLast','logsJump','logsJumpForm','logsRange','largeSearchPane','filterBadge','activeFilters','filterRow','logsSelection','largeSelection','toast','largeDetailCopy','largeDetailMore','largeDetailHint','toggleFilters','emptyState','logContent','loadProgress','loadProgressBar','paneSplitter','idFilterError','tagFilterError','searchError','dropHint','filterPanel','largeRegex','rowMenu','largeSize','exportLogs','exportSearch','exportCancel','recentFiles','recentList','clearRecent','recentButton','timeFrom','timeTo','timeFilterError','lowerTabs','searchTab','bookmarkTab','bookmarkPane','bookmarkCount','bookmarkList','copyBookmarks','clearBookmarks','searchPosition','logsPending','reopenFile','errorBar','errorReopen','followFile','messageFilter','messageRegex','messageFilterError','messageLabel'];
  for (const id of ['extrasButton','deviceButton','emptyDevice','captureBar','captureState','captureText','captureHint','captureApp','captureAppStatus','captureStop','captureSave','captureLeftovers','leftoverList','openFile']) ids.push(id);
  const ui = Object.fromEntries(ids.map(id => [id,$(id)]));
  // Device captures (devices.js) mount once the menu and toasts exist.
  let devices=null;
  // App extensions (extras.js) mount with the devices; other builds have none.
  let extras=null;
  // The layout reports its first render before the controllers exist.
  let ready=false;
  // First-use tips are offered where their feature comes into reach.
  const layout = mountLogLayout({document, window, onPanelChange:shown=>{if(!ready)return;errors();if(shown)tips?.offer('levels',{hold:value=>layout.hold(value)});}});
  const regexInput=ui.largeRegex;
  let inputError='', pageGone=false, savedInput=null;
  let pendingSourceId=null, lastSourceId=0, sourceEventSequence=0, pickerSequence=0, pickerPending=false;
  let pendingPastePrefix=null;
  // Reload and saved work: what native code allows for the shown dataset, a
  // reload waiting for its source, bookmarks to read again by line number,
  // places still being located, and the filters last applied.
  let work={datasetId:null,reopenable:false},reloadRequest=null,pendingMarks=null,restoreTargets=null,appliedFilters=null;
  let selectedLevels=new Set(LEVELS), rawMode=false, debounce=null, disposed=false, validationError=false, inputPending=false, inputPaused=false, upperFingerprint=null, detailOwner=null, doubleClickLine=null;
  let detailRender = null;
  // Saved filters, mounted once the work saver exists.
  let saved = null;
  // The filter's conditions; the one being edited is in the panel's level,
  // tag, ID and time fields, and the cards show the others.
  let conds={include:[emptyCondition()],exclude:[]},active={list:'include',index:0},conditionView=null;
  // The open log's format: its columns, level names and filter fields.
  let shownFormat = 'text', levelKey = 'text';
  const previousRows=new Map(), viewports=new Map(), focusedSelection=new Map(), navigationFocus=new Map(), selections=new Map(), selectedAt=new Map(), paintedSelection=new Map(), paintedMarks=new Map();
  const MOD=/Mac|iPhone|iPad/.test(window.navigator?.platform??'')?'⌘':'Ctrl+';
  // Only macOS reads a copied file from the native pasteboard.
  if(MOD!=='⌘'){$('searchShortcut').textContent='Ctrl+F';$('pasteHint').textContent='로그 텍스트 붙여넣기';for(const key of document.querySelectorAll?.('.mod-key')??[])key.textContent='Ctrl';}
  let toastTimer=null,exporting=false,exportTotal=0;
  // A toast may offer one action, such as undoing what it reports; it then
  // stays a little longer and takes pointer input.
  function notify(message,action=null,{ms=action?6000:2500}={}){
    if(toastTimer!==null)window.clearTimeout(toastTimer);
    const hide=()=>{if(toastTimer!==null)window.clearTimeout(toastTimer);toastTimer=null;ui.toast.hidden=true;};
    ui.toast.replaceChildren(textSpan(document,message));
    if(action){const button=document.createElement('button');button.type='button';button.className='toast-action';button.textContent=action.label;button.addEventListener('click',()=>{hide();void action.run();});ui.toast.append(button);}
    ui.toast.classList.toggle('has-action',!!action);ui.toast.hidden=false;
    // A toast that would cover an open raw detail's heading and buttons
    // shows in the heading's free space, between its title and its buttons,
    // or else just above the detail.
    ui.toast.style.bottom=ui.toast.style.right='';
    const detail=document.getElementById?.('largeDetail'),panel=ui.toast.parentElement?.getBoundingClientRect?.();
    const box=detail&&!detail.hidden?detail.getBoundingClientRect?.():null,heading=box&&detail.querySelector?.('.detail-heading')?.getBoundingClientRect?.(),toast=ui.toast.getBoundingClientRect?.();
    if(panel&&box&&heading&&toast&&toast.top<heading.bottom+6&&panel.bottom>box.top){
      const actions=detail.querySelector?.('.detail-actions')?.getBoundingClientRect?.(),title=detail.querySelector?.('.detail-heading strong')?.getBoundingClientRect?.();
      if(actions&&title&&actions.left-12-toast.width>=title.right+12){ui.toast.style.bottom=`${Math.round(panel.bottom-heading.bottom+(heading.height-toast.height)/2)}px`;ui.toast.style.right=`${Math.round(panel.right-actions.left+12)}px`;}
      else ui.toast.style.bottom=`${Math.round(panel.bottom-box.top)+10}px`;
    }
    toastTimer=window.setTimeout(hide,ms);
  }
  // A pane's row selection (row-selection.js) is kept only while its focus
  // row stays the pane's selected row in the same query. Any other selection,
  // such as a jump, F2 or a new query, leaves that row alone, known by its
  // position once it is painted.
  const selectionIdentity=state=>`${state.datasetId}:${state.queryId}`;
  let selectionRevision=0;
  function selectionOf(prefix,state){
    const s=selections.get(prefix),identity=selectionIdentity(state),at=selectedAt.get(prefix);
    if(s&&s.identity===identity&&s.focusLine===state.selectedLine)return s;
    return state.selectedLine&&at?.identity===identity&&at.line===state.selectedLine?selectOne(at.index):null;
  }
  function setSelection(prefix,state,next,line){selections.set(prefix,{...next,identity:selectionIdentity(state),focusLine:line});selectionRevision++;dropRowText(prefix,next,line,state.count);}
  // A selection still growing with its search counts its rows with +.
  function updateSelectionInfo(prefix,state){
    const selection=selectionOf(prefix,state),size=selectedCount(selection,state.count);
    const text=size>1?`${formatNumber(size)}${selection.all&&!state.queryComplete?'+':''}행 선택 · ${MOD}C 복사`:'';
    if(ui[prefix+'Selection'].textContent!==text)ui[prefix+'Selection'].textContent=text;
  }
  // Shows a pane's new selection. A new focus row is selected through the
  // viewport, which may still have to read it; with the same focus only the
  // marks change, unless `reselect` asks for the row again (a plain click
  // reopens its detail).
  function applySelection(prefix,next,{repeat=false,reselect=true}={}){
    const owner=prefix==='logs'?logs:search,current=owner.getState(),view=viewports.get(prefix);
    if(!view||current.closed)return;
    if(!reselect&&current.selectedLine&&selectionOf(prefix,current)?.focus===next.focus){setSelection(prefix,current,next,current.selectedLine);return prefix==='logs'?renderLogs(owner.getState()):renderSearch(owner.getState());}
    view.selectIndex(next.focus,{force:true,repeat,extend:next});
  }
  // ⌘/Ctrl+A in the search results takes every row found so far and the
  // rows still to come; the upper logs keep their selection. The focus stays,
  // or with none is the top row in view, so the list does not scroll.
  function selectAllRows(prefix){
    const owner=prefix==='logs'?logs:search,state=owner.getState(),view=viewports.get(prefix);
    if(prefix!=='large'||inputPending||state.closed||!state.count||!view)return;
    const shown=view.metrics(),top=Math.min(state.count-1,Math.ceil(shown.logicalPosition/ROW_HEIGHT));
    applySelection(prefix,selectAll(selectionOf(prefix,state)?.focus??top),{reselect:false});
  }
  // Text selected in a row stays only while that row is its list's one
  // selected row; a text selection started in a row stays inside that row.
  const rowOf=node=>(node?.nodeType===1?node:node?.parentElement)?.closest?.('.log-row')??null;
  function dropRowText(prefix,next,line,count){
    const text=window.getSelection?.(),row=text&&String(text)?rowOf(text.anchorNode):null;
    if(row&&ui[prefix+'Rows'].contains(row)&&(row.dataset.lineNumber!==String(line)||selectedCount(next,count)>1))text.removeAllRanges();
  }
  document.addEventListener('selectionchange',()=>{
    const text=window.getSelection?.(),row=text&&!text.isCollapsed?rowOf(text.anchorNode):null;
    if(!row||row.contains(text.focusNode))return;
    const end=[...row.childNodes].filter(node=>node.nodeName!=='BUTTON').at(-1),after=text.anchorNode.compareDocumentPosition?.(text.focusNode)&4;
    if(after&&end)text.extend(end,end.childNodes.length);else text.extend(row,0);
  });
  const activeCondition=()=>conds[active.list][active.index];
  // Whether the editor's message is a pattern (its .* switch).
  let messageRegex=false;
  function syncEditor(){const c=activeCondition();if(!c)return;c.levels=LEVELS.filter(level=>selectedLevels.has(level));c.tag=ui.tagFilter.value;c.pid=ui.pidFilter.value;c.tid=ui.tidFilter.value;c.timeFrom=ui.timeFrom.value;c.timeTo=ui.timeTo.value;c.message=ui.messageFilter.value;c.messageRegex=messageRegex;}
  function loadEditor(){const c=activeCondition();selectedLevels=new Set(c.levels);ui.tagFilter.value=c.tag;ui.pidFilter.value=c.pid;ui.tidFilter.value=c.tid;ui.timeFrom.value=c.timeFrom;ui.timeTo.value=c.timeTo;ui.messageFilter.value=c.message??'';messageRegex=c.messageRegex===true;ui.messageRegex.setAttribute('aria-pressed',String(messageRegex));updateLevels();showOptionalFields();}
  // The message, PID·TID and the time range show for a condition that uses
  // them; otherwise a + button opens them. Once shown they stay while that
  // condition is edited, so clearing a field never moves the editor under the
  // cursor. A DLT log has no PID or TID.
  const optionalFields=[{group:$('messageFields'),opener:$('openMessage'),inputs:[ui.messageFilter],available:()=>true},{group:$('idFields'),opener:$('openIds'),inputs:[ui.pidFilter,ui.tidFilter],available:()=>shownFormat!=='dlt'},{group:$('timeFields'),opener:$('openTime'),inputs:[ui.timeFrom,ui.timeTo],available:()=>true}];
  let openedFields=null;
  const openedGroups=()=>{const c=activeCondition();if(openedFields?.condition!==c)openedFields={condition:c,groups:new Set()};return openedFields.groups;};
  function showOptionalFields(){
    const groups=openedGroups();
    for(const field of optionalFields){
      const available=field.available(),shown=available&&(groups.has(field)||field.inputs.some(input=>input.value));
      if(shown)groups.add(field);
      field.group.hidden=!shown;field.opener.hidden=!available||shown;
    }
    $('fieldOpeners').hidden=optionalFields.every(field=>field.opener.hidden);
  }
  for(const field of optionalFields){
    field.opener.addEventListener('click',()=>{openedGroups().add(field);showOptionalFields();field.inputs[0].focus();});
    for(const input of field.inputs)input.addEventListener('input',showOptionalFields);
  }
  const readFilters=()=>{syncEditor();const copy=c=>({...c,levels:[...c.levels]});return {include:conds.include.map(copy),exclude:conds.exclude.map(copy),query:ui.searchInput.value,caseSensitive:ui.caseSensitive.checked,includeRaw:ui.includeRaw.checked,recognizeLineNumbers:ui.recognizeLineNumbers.value==='off'?false:ui.recognizeLineNumbers.value==='on'?true:null,searchMode:regexInput.checked?'regex':'literal'};};
  // Only a complete summary proves a level absent; such chips are dimmed but stay usable.
  function updateLevels(summary=logs.getState().summary,complete=logs.getState().summaryComplete) {const LEVEL_NAMES=levelNames(shownFormat);for(const button of ui.levelFilters.children){const level=button.dataset.level;button.setAttribute('aria-pressed',String(selectedLevels.has(level)));button.querySelector('.count').textContent=`${formatNumber(summary?.counts?.[level])}${summary&&!complete?'+':''}`;button.dataset.empty=String(!!summary&&!!complete&&!summary.counts?.[level]);button.title=`${LEVEL_NAMES[level]} · ${complete?'전체 파일의 레벨별 행 수':'지금까지 분석한 행 수'} · Alt+클릭: 이 레벨만`;}}
  function updateInputCancellation(state=logs.getState()) {
    const validating=pendingSourceId!==null&&(state.datasetId!==pendingSourceId||!state.textComplete);
    $('cancelInput').hidden=!pickerPending&&!validating;
  }
  // A combined log numbers each row within its own file behind a file badge;
  // the physical row of the combined log stays in the tooltip.
  function lineCell(entry){
    const source=rowSource(entry,logs.getState().files);
    if(!source)return textSpan(document,entry.lineNumber,'line-number');
    const cell=textSpan(document,'','line-number');
    cell.append(textSpan(document,String(source.index+1),'file-badge'),textSpan(document,String(source.line),'line-text'));
    cell.title=`${source.name} · ${formatNumber(source.line)}행\n합친 로그 ${formatNumber(entry.lineNumber)}행`;
    return cell;
  }
  function table(state, prefix, owner, isSearch) {
    let view=viewports.get(prefix);
    if(!view){
      const realViewport=ui[prefix+'Viewport'];
      let retainedNodes=new Map(),dragFrom=null;
      // A press, or a click no press took: Shift extends from the anchor,
      // ⌘/Ctrl toggles one row, a plain press selects the row alone. Says
      // whether a drag may extend the selection from there.
      const press=(index,{shift=false,toggle=false}={})=>{
        realViewport.focus({preventScroll:true});
        const current=owner.getState(),prior=selectionOf(prefix,current),count=current.count;
        const next=toggle&&!shift?toggleRow(prior,index,count):shift?extendTo(prior,index,count,{keep:toggle}):selectOne(index);
        dragFrom=next.focus===index?next:null;
        if(next!==prior)applySelection(prefix,next,{reselect:!toggle||shift});
        return !!dragFrom;
      };
      const gestures=rowGestures({list:realViewport,document,window,mac:MOD==='⌘',indexAt:y=>view.indexAt(y),scroll:delta=>view.scrollBy(delta),press,
        isTextRow:index=>{const current=owner.getState(),s=selectionOf(prefix,current);return s?.focus===index&&selectedCount(s,current.count)===1;},
        grab:index=>{realViewport.focus({preventScroll:true});dragFrom=selectOne(index);return true;},
        drag:index=>{if(dragFrom)applySelection(prefix,extendTo(dragFrom,index,owner.getState().count,{keep:true}),{repeat:true});}});
      const toggleKey=event=>!!(MOD==='⌘'?event.metaKey:event.ctrlKey);
      view=createContinuousViewport({viewport:realViewport,rowsElement:ui[prefix+'Rows'],document,window,
        fetchRows:start=>owner.fetchRows(start),onError:owner.reportError,onUserMove:()=>stoppedFinding(owner),
        onSelect:(entry,index,{repeat=false,extend=false}={})=>{const current=owner.getState();setSelection(prefix,current,extend?.focus===index?extend:extend===true?extendTo(selectionOf(prefix,current),index,current.count):selectOne(index),entry.lineNumber);detailOwner=owner;if(isSearch)followUpper(entry.lineNumber,repeat);owner.selectLine(entry.lineNumber);void owner.loadDetail(entry.lineNumber,0,{delay:repeat?DETAIL_SETTLE_MS:0});saver.schedule();if(!isSearch)tips?.offer('rows');},
        onVisible:(first,last,total)=>{
          owner.rememberPosition(first);saver.schedule();
          if(total)showRange(prefix,owner,`${formatNumber(first+1)}–${formatNumber(last)} / ${formatNumber(total)}건`,false);
          ui[prefix+'First'].disabled=!total||first===0;ui[prefix+'Last'].disabled=!total||last>=total;
        },
        paint:(entries,current)=>{
          const breaks=captureBreaks(current.capture);
          const nodes=[],nextNodes=new Map(),activeElement=document.activeElement,hadFocus=realViewport.contains(activeElement),selected=selectedRanges(selectionOf(prefix,current),current.count),multiple=selected.length>1||selected[0]?.[1]>selected[0]?.[0];
          for(let k=0;k<entries.length;k++){
            const {row:entry,index}=entries[k];
            const prior=retainedNodes.get(index);
            let row=prior?.entry===entry&&prior?.rawMode===rawMode&&prior?.levels===levelKey?prior.node:null;
            if(!entry){
              row??=textSpan(document,'행을 읽는 중…','virtual-loading-row');row.setAttribute('aria-hidden','true');
            }else{
            const fresh=!row;row??=document.createElement('div');
            const fileStart=entry.fileLine===1&&entry.file>0;
            // A capture's device left and came back before this row, or before
            // it and after the row shown above it when a filter hides rows.
            const above=k>0&&entries[k-1].index===index-1?entries[k-1].row:null;
            const captureBreak=breaks.length?breakLabel(breaks,index===0?0:above?above.lineNumber:entry.lineNumber-1,entry.lineNumber):'';
            // Column rows of one log entry read as one: continuation rows
            // take the entry's level, and no separator splits the entry.
            const entryOwner=rawMode?null:continuationOwner(entry),next=entries[k+1]?.index===index+1?entries[k+1].row:null;
            const continued=!rawMode&&joinsNext(entry,next);
            const kindClass=entryOwner?continuationClass(entryOwner):entry.kind!=='parsed'?'raw-row':`level-row-${entry.level}`;
            row.className=`log-row ${kindClass}${continued?' record-continued':''}${fileStart?' file-start':''}${captureBreak?' capture-break':''}${current.selectedLine===entry.lineNumber?' selected-log-row':''}${entry.encodingDamaged?' encoding-damaged-row':''}${multiple&&inRanges(selected,index)?' in-selection':''}${bookmarks.has(entry.lineNumber)?' bookmarked':''}${!isSearch&&entry.searchHit?' search-hit':''}`; row.tabIndex=-1;row.dataset.lineNumber=String(entry.lineNumber);row.setAttribute('role','listitem');row.setAttribute('aria-posinset',String(index+1));row.setAttribute('aria-setsize',String(current.queryComplete?current.count:-1));
            if(captureBreak)row.dataset.captureBreak=captureBreak;else delete row.dataset.captureBreak;
            if(current.selectedLine===entry.lineNumber){row.setAttribute('aria-current','true');selectedAt.set(prefix,{identity:selectionIdentity(current),line:entry.lineNumber,index});}
            else row.removeAttribute('aria-current');
            if(fresh){
            row.setAttribute('aria-describedby',prefix+'RowHint');
            const number=lineCell(entry);
            if(!isSearch&&entry.searchHit)number.title=number.title?`${number.title}\n검색 결과`:'검색 결과';
            if(entry.encodingDamaged){const badge=textSpan(document,'⚠','encoding-badge');badge.title='UTF-8 손상 구간이 있습니다. 행은 유지하며 손상 부분만 표시용 �로 보여줍니다.';number.append(badge);}
            row.append(number);
            if(entryOwner){
              const message=highlightedSpan(document,entry.raw??'','message continuation-text',entry.highlights,entry.encodingDamage),raw=String(entry.raw??'');
              message.title=raw.length>1000?`${raw.slice(0,1000)}…`:raw;
              row.append(...ownerCells(document,entry,entryOwner,logs.getState().files,isSearch),message);
            }
            else if(rawMode||entry.kind!=='parsed')row.append(highlightedSpan(document,entry.raw||'(빈 줄)','raw-content',entry.highlights,entry.encodingDamage));
            else if(entry.dlt)row.append(...dltCells(entry,(text,className)=>fieldSpan(document,entry,text,className),levelSpan(document,entry)));
            else {
              const stamp=fieldSpan(document,entry,`${entry.date??''} ${entry.time??''}`,'timestamp'),tag=fieldSpan(document,entry,entry.tag,'tag');
              if(entry.process){stamp.append(textSpan(document,' · '),fieldSpan(document,entry,entry.process,'process'));stamp.title+=` · ${entry.process}`;}
              // Only iOS rows carry `year`. Their process names the source as
              // a tag does, and their tag ends with the category: both stay
              // in view.
              if(entry.year!==undefined){stamp.classList.add('apple-stamp');tag.classList.add('from-end');}
              row.append(stamp,fieldSpan(document,entry,entry.pid,'pid'),fieldSpan(document,entry,entry.tid,'tid'),levelSpan(document,entry),tag,fieldSpan(document,entry,entry.message,'message'));
            }
            // A press selects; a click no press took (one on the selected
            // row, whose text the browser may select) selects as it would.
            // Later clicks of a multi-click belong to the first click's row;
            // macOS Ctrl+click is the context menu.
            row.addEventListener('mousedown',event=>{
              if(isSearch&&event.detail===1&&!event.shiftKey&&!toggleKey(event))doubleClickLine=entry.lineNumber;
              gestures.down(event,index);
            });
            row.addEventListener('click',event=>{
              if(event.detail>1||(MOD==='⌘'&&event.ctrlKey))return;
              if(!gestures.click())press(index,{shift:event.shiftKey===true,toggle:toggleKey(event)});
            });
            row.addEventListener('keydown',event=>{
              if(event.key==='ContextMenu'||(event.key==='F10'&&event.shiftKey)){event.preventDefault();const box=row.getBoundingClientRect?.();openRowMenu(entry,index,prefix,owner,isSearch,box?box.left+48:0,box?box.bottom:0);return;}
              if(event.key==='Enter'||event.key===' '){event.preventDefault();press(index);if(isSearch)jumpTo(entry.lineNumber);}
            });
            // Right-click keeps a selection that contains the row.
            row.addEventListener('contextmenu',event=>{
              event.preventDefault();const current=owner.getState();
              if(!inRanges(selectedRanges(selectionOf(prefix,current),current.count),index))press(index);else realViewport.focus({preventScroll:true});
              openRowMenu(entry,index,prefix,owner,isSearch,event.clientX,event.clientY);
            });
            }
            }
            nodes.push(row);nextNodes.set(index,{entry,rawMode,levels:levelKey,node:row});
          }
          // Retain overlapping DOM nodes in place. A detail resize must not
          // destroy a clicked row or the row receiving keyboard input.
          const container=ui[prefix+'Rows'];
          const wanted=new Set(nodes);
          for(const node of [...container.children])if(!wanted.has(node))container.removeChild(node);
          for(let i=0;i<nodes.length;i++)if(container.children[i]!==nodes[i])container.insertBefore(nodes[i],container.children[i]??null);
          retainedNodes=nextNodes;
          if(prefix==='logs')layout.fitTimestamp(current.datasetId);layout.fitLines();
          if(hadFocus&&!realViewport.contains(document.activeElement)){
            // A re-read row (new search marks) keeps focus on its new node.
            const line=activeElement?.dataset?.lineNumber,same=line&&container.querySelector?.(`[data-line-number="${line}"]`);
            (realViewport.contains(activeElement)?activeElement:same||realViewport).focus({preventScroll:true});
          }
          focusSelected(prefix,current,hadFocus);
        }});
      viewports.set(prefix,view);
    }
    // A capture's marks may arrive for rows already painted, as when an app
    // it started finishes starting after its first rows came.
    const marks=captureBreaks(state.capture).map(b=>`${b.kind}:${b.row}`).join(',');
    view.update(state,{force:!previousRows.has(prefix)||previousRows.get(prefix)!==rawMode||paintedSelection.get(prefix)!==selectionRevision||paintedMarks.get(prefix)!==marks,stick:!isSearch&&!!state.following});previousRows.set(prefix,rawMode);paintedSelection.set(prefix,selectionRevision);paintedMarks.set(prefix,marks);updateSelectionInfo(prefix,state);
  }
  // Focuses a pane's newly selected row when the pane had focus, or when a
  // jump asked for it and focus has not moved since the jump.
  function focusSelected(prefix,current,hadFocus=false){
    const selection=`${current.datasetId}:${current.queryId}:${current.scrollRevision}:${current.selectedLine}`;
    if(navigationFocus.has(prefix)&&navigationFocus.get(prefix)!==document.activeElement)navigationFocus.delete(prefix);
    if(current.selectedLine&&focusedSelection.get(prefix)!==selection&&(hadFocus||navigationFocus.has(prefix))){
      const row=ui[prefix+'Rows'].querySelector?.(`[data-line-number="${current.selectedLine}"]`);
      if(row){row.focus?.({preventScroll:true});focusedSelection.set(prefix,selection);navigationFocus.delete(prefix);}
    }
  }
  // A jump moves focus to the row it lands on. The paint after a scroll does
  // that, but a row already shown (as a followed search result is) paints
  // nothing, so the jump also tries once the locate settles.
  function locateAndFocus(prefix,owner,line,options){
    navigationFocus.set(prefix,document.activeElement);
    void owner.locateLine(line,options).then(()=>{if(!disposed)focusSelected(prefix,owner.getState());});
  }
  function jumpTo(line){if(!inputPending&&!validationError&&!disposed)locateAndFocus('logs',logs,line);}
  // Where a pane's list is, to keep across a new query of the same search:
  // its anchor (the row the user chose, see controller.js) where its
  // selection is on screen when in view, or else its top row unless the list
  // is at its start. A place still being found again stays as it was.
  function placeOf(prefix,owner){
    const kept=owner.relocating();if(kept)return kept;
    const state=owner.getState(),view=viewports.get(prefix);
    if(!view||state.closed||!state.queryId)return null;
    const shown=view.metrics(),at=shown.selectionIndex,line=state.anchor??state.selectedLine;
    if(line)return {line,select:true,offset:state.selectedLine&&Number.isSafeInteger(at)&&at>=shown.first&&at<shown.last?at-shown.first:state.anchorOffset};
    const top=shown.first>0?view.cachedRows(shown.first,shown.first)?.[0]?.lineNumber:null;
    return Number.isSafeInteger(top)?{line:top,select:false,offset:0}:null;
  }
  // A pane looking for its kept place says so where it shows its range once
  // that takes a moment, with how far the scan is toward the row when its
  // detail gives the row's place in the file; the brief hold before shows no
  // count, which would be the new results' first rows.
  function findingText(owner){
    const kept=owner.relocating();if(!kept)return '';if(!kept.shown)return ' ';
    const state=owner.getState(),d=state.detail,target=d?.lineNumber===kept.line&&Number.isSafeInteger(d.sourceStartByte)&&d.sourceStartByte>0?d.sourceStartByte:state.size;
    const scanned=target>0&&state.scannedBytes>0?` ${Math.min(99,Math.floor(state.scannedBytes/target*100))}%`:'';
    return `${rowLabel(kept.line)} 찾는 중…${scanned}`;
  }
  // A range text is a live status for what a query is doing, not for the
  // rows a scroll or a followed result happens to show; a pane finding its
  // kept place shows that instead.
  function showRange(prefix,owner,text,announce){
    const finding=findingText(owner),node=ui[prefix+'Range'];
    node.setAttribute('aria-live',announce?'polite':'off');
    node.textContent=finding||text;node.classList?.toggle('finding',finding.trim()!=='');
  }
  // The user moved a list whose anchor was still being found: the list stays,
  // and a toast offers to move there for a moment.
  function stoppedFinding(owner){
    const given=owner.stopRelocating();
    if(!given?.select||!given.shown||disposed)return;
    notify(`${rowLabel(given.line)} 찾기를 멈췄습니다.`,{label:'이동',run:()=>{if(!disposed)locateAndFocus(owner===logs?'logs':'large',owner,given.line);}});
  }
  // A selected row the new results leave out gave its place to the one
  // after it (or before it, at the end); the pane showing its detail says so,
  // unless a toast offers to undo the change that hid it. Upper logs that
  // showed the same row take the row the search chose, as they show every
  // selected search result.
  function relocated(owner,{lineNumber,target}){
    if(owner===search&&logs.getState().anchor===lineNumber)void logs.locateLine(target,{keepAnchor:true});
    if(owner!==(detailOwner??logs)||(!ui.toast.hidden&&ui.toast.classList.contains?.('has-action')))return;
    notify(`${rowLabel(lineNumber)}은 ${owner===logs?'새 필터 결과':'새 검색 결과'}에 없어 ${target>lineNumber?'다음':'마지막'} 결과인 ${rowLabel(target)}을 선택했습니다.`,null,{ms:4000});
  }
  // Selecting a search result reveals its row in the upper logs without
  // moving focus, as a bookmark does; a held arrow key follows only where
  // the selection settles.
  let followTimer=null;
  function followUpper(line,repeat){
    if(followTimer!==null)window.clearTimeout(followTimer);followTimer=null;
    const go=()=>{followTimer=null;if(!inputPending&&!validationError&&!disposed)void logs.locateLine(line);};
    if(repeat)followTimer=window.setTimeout(go,DETAIL_SETTLE_MS);else go();
  }
  // A bookmark shown above becomes the upper logs' selected row and the raw
  // detail's row, as a selected search result does; `focus` also moves
  // focus there.
  function showBookmark(line,{repeat=false,focus=false}={}){
    if(inputPending||validationError||disposed)return;
    detailOwner=logs;
    if(focus){if(followTimer!==null)window.clearTimeout(followTimer);followTimer=null;locateAndFocus('logs',logs,line);}
    else followUpper(line,repeat);
    void logs.loadDetail(line,0,{delay:repeat?DETAIL_SETTLE_MS:0});markSelections();
  }
  // The jump box takes the physical row number of the row column; a combined
  // log also takes file:row. A row outside the pane's results selects the
  // next result after it, and the outcome is reported briefly.
  const jumpRequests=new Map();
  function jumpTitle(where,state){
    const files=state.files?.length>1;
    return `${files?'파일 번호:행(예: 2:150) 또는 합친 로그의 행 번호':'행 번호(행 칸의 번호)'}로 이동합니다. ${where}에 없는 행이면 그 뒤의 첫 결과로 이동합니다.`;
  }
  function rowLabel(line){
    const files=logs.getState().files??[];let k=-1;
    if(files.length>1)for(let i=0;i<files.length;i++)if(Number.isSafeInteger(files[i].firstLine)&&files[i].firstLine<=line)k=i;
    return k<0?`${formatNumber(line)}행`:`${k+1}번 파일 ${formatNumber(line-files[k].firstLine+1)}행`;
  }
  function jumpToRow(prefix,owner){
    const input=ui[prefix+'Jump'],state=logs.getState(),files=state.files??[],multi=files.length>1;
    const reject=message=>{input.setAttribute('aria-invalid','true');notify(message);};
    const m=String(input.value).trim().match(/^(\d{1,16})(?:\s*:\s*(\d{1,16}))?$/);
    if(!m||(m[2]!==undefined&&!multi))return reject(multi?'행 번호 또는 파일 번호:행(예: 2:150)을 입력해 주세요.':'행 번호를 숫자로 입력해 주세요.');
    let line=Number(m[1]);
    if(m[2]!==undefined){
      const k=line-1,file=files[k],next=files[k+1]?.firstLine;
      if(!file)return reject(`파일 번호는 1–${files.length}입니다.`);
      if(!Number.isSafeInteger(file.firstLine))return reject(`${k+1}번 파일은 아직 읽지 않았습니다. 읽기가 끝난 뒤 다시 시도해 주세요.`);
      line=file.firstLine+Number(m[2])-1;
      if(Number(m[2])<1||(Number.isSafeInteger(next)&&line>=next))return reject(`${k+1}번 파일에는 ${Number(m[2])<1?'0':formatNumber(Number(m[2]))}행이 없습니다.`);
    }
    if(!Number.isSafeInteger(line)||line<1)return reject('행 번호는 1부터입니다.');
    if(line>state.total)return reject(state.textComplete?`마지막 행은 ${rowLabel(state.total)}입니다.`:`${rowLabel(line)}은 아직 읽지 않았습니다. 읽기가 끝난 뒤 다시 시도해 주세요.`);
    input.removeAttribute('aria-invalid');jumpRequests.set(owner,{line,typed:true});
    locateAndFocus(prefix,owner,line,{nearest:true});
  }
  function located(owner,{lineNumber,outcome,target}){
    // Each locate replaces the pane's previous one, so any outcome ends a jump.
    const wanted=jumpRequests.get(owner);jumpRequests.delete(owner);
    if(wanted?.line!==lineNumber)return;
    const input=ui[owner===logs?'logsJump':'largeJump'],where=owner===logs?'현재 필터 결과':'검색 결과';
    if(wanted.typed&&(outcome==='found'||outcome==='nearest'))input.value='';
    if(outcome==='nearest')notify(`${rowLabel(lineNumber)}은 ${where}에 없어 다음 결과인 ${rowLabel(target)}으로 이동했습니다.`);
    else if(outcome==='hidden')notify(`${rowLabel(lineNumber)}은 ${where}에 없고 그 뒤에도 ${owner.getState().queryComplete?'결과가 없습니다':'아직 찾은 결과가 없습니다'}.`);
    else if(outcome==='incomplete')notify(`${where} 처리가 끝나지 않아 ${rowLabel(lineNumber)}의 위치를 확인할 수 없습니다.`);
  }
  // The first press of a double-click selects the row and opens the detail,
  // which may shift layout before the button is even released, so the rest
  // of it can land on another row, on the list itself or on the detail.
  // Every press sequence starts by clearing the target; only the first press
  // on a search result or a bookmark sets it, and the double-click jumps to
  // that row.
  const panel=$('largeLogPanel'),lowerPane=$('largeLowerPane'),detailSplitter=$('detailSplitter');
  panel.addEventListener('mousedown',event=>{if(event.detail<=1)doubleClickLine=null;},true);
  ui.bookmarkList.addEventListener('mousedown',event=>{
    const row=event.target?.closest?.('.bookmark-row');
    if(event.detail===1&&row&&!event.target.closest('button')&&!event.shiftKey&&!(MOD==='⌘'?event.metaKey:event.ctrlKey))doubleClickLine=Number(row.dataset.lineNumber);
  });
  // The browser selects a word under a double-click; the jump drops it.
  panel.addEventListener('dblclick',()=>{const line=doubleClickLine;doubleClickLine=null;if(line!==null){window.getSelection?.()?.removeAllRanges?.();jumpTo(line);}});
  // Search results and bookmarks share the lower region as tabs; the
  // bookmark tab exists only while there are bookmarks. The region takes its
  // resizable share only while it shows them; a raw detail alone keeps its
  // own height.
  let lowerTab='search',bookmarkCount=0;
  function updateLowerPane(){
    // Focus inside a closing bookmark tab returns to the upper logs.
    if(!bookmarkCount&&lowerTab==='bookmarks'){lowerTab='search';if(ui.bookmarkPane.contains(document.activeElement))ui.logsViewport.focus({preventScroll:true});}
    const marks=lowerTab==='bookmarks';
    ui.lowerTabs.hidden=!bookmarkCount;
    // The search results are a tab panel only while the tabs show.
    if(bookmarkCount){ui.largeSearchPane.setAttribute('role','tabpanel');ui.largeSearchPane.setAttribute('aria-labelledby','searchTab');}
    else{ui.largeSearchPane.removeAttribute('role');ui.largeSearchPane.removeAttribute('aria-labelledby');}
    ui.searchTab.setAttribute('aria-selected',String(!marks));ui.searchTab.tabIndex=marks?-1:0;
    ui.bookmarkTab.setAttribute('aria-selected',String(marks));ui.bookmarkTab.tabIndex=marks?0:-1;
    ui.largeSearchPane.hidden=marks||!ui.searchInput.value;ui.bookmarkPane.hidden=!marks;
    const lists=!ui.largeSearchPane.hidden||marks;lowerPane.classList.toggle('expanded',lists);
    // The detail has its own splitter above it. A splitter that hides hands
    // its focus to the upper logs.
    for(const [splitter,shown] of [[ui.paneSplitter,lists],[detailSplitter,!ui.largeDetail.hidden]]){if(!shown&&splitter.contains(document.activeElement))ui.logsViewport.focus({preventScroll:true});splitter.hidden=!shown;}
    if(ready)renderPosition();
  }
  // The selected result's place among the results shows in the search box
  // while the results tab is shown. A step from the box is announced once,
  // and says when it went around the end of the results.
  let stepNote=null;
  function renderPosition(state=search.getState()){
    const index=lowerTab==='search'&&!inputPending&&!state.closed&&state.selectedLine?viewports.get('large')?.metrics().selectionIndex??null:null;
    if(stepNote&&(stepNote.queryId!==state.queryId||lowerTab!=='search'||(index!==null&&stepNote.index!==index)))stepNote=null;
    const note=index===null?null:stepNote,text=index===null?'':`${note?.wrap?`${note.wrap} · `:''}${formatNumber(index+1)}번째 / ${formatNumber(state.count)}${state.queryComplete?'':'+'}건`;
    if(ui.searchPosition.textContent!==text){ui.searchPosition.setAttribute('aria-live',note?.announce?'polite':'off');ui.searchPosition.textContent=text;}
    if(note)note.announce=false;
  }
  function selectLowerTab(tab,{focus=false}={}){
    if(tab==='bookmarks'&&!bookmarkCount)return;
    const changed=lowerTab!==tab;lowerTab=tab;updateLowerPane();
    if(changed&&tab==='bookmarks')bookmarks.render();
    if(!focus)return;
    if(tab==='bookmarks')bookmarks.focusList();else if(!ui.searchInput.value)ui.searchInput.focus();else ui.searchTab.focus();
  }
  ui.searchTab.addEventListener('click',()=>selectLowerTab('search',{focus:true}));
  ui.bookmarkTab.addEventListener('click',()=>selectLowerTab('bookmarks',{focus:true}));
  ui.lowerTabs.addEventListener('keydown',event=>{
    if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
    event.preventDefault();const tab=lowerTab==='search'?'bookmarks':'search';selectLowerTab(tab);(tab==='search'?ui.searchTab:ui.bookmarkTab).focus();
  });
  // A detail shows the whole log entry of the selected row; `line` is the
  // entry row's number in its own file.
  const entryRows=d=>Number.isSafeInteger(d.first)&&Number.isSafeInteger(d.last)?d.last-d.first+1:1;
  function recordTitle(d,line){
    const rows=entryRows(d);
    return rows<2?`${formatNumber(line)}행 원문`:`${formatNumber(line)}–${formatNumber(line+rows-1)}행 원문`;
  }
  // While the other pane selects a different row, the title names the pane
  // whose row the detail shows.
  function renderDetailTitle(d=detailOwner?.getState().detail,owner=detailOwner){
    if(!d||!owner)return;
    const source=rowSource(d,logs.getState().files),other=(owner===logs?search:logs).getState().selectedLine;
    // With no results the anchor's detail stays, named as out of them.
    const state=owner.getState(),missing=state.selectedLine!==d.lineNumber&&state.anchor===d.lineNumber;
    const from=`${other&&other!==d.lineNumber?owner===logs?'로그 · ':'검색 결과 · ':''}${source?`${source.name} · `:''}`,range=recordTitle(d,source?source.line:d.first??d.lineNumber);
    const notes=[[entryRows(d)>1&&d.recordComplete===false,'앞부분 1 MiB',''],[missing,'현재 결과에 없음',''],[d.encodingDamage?.bytes,'손상 바이트 표시',' damage'],[d.encodingRepaired,'깨진 문자 복원',' damage']].filter(([on])=>on);
    const text=[from+range,...notes.map(([,note])=>note)].join(' · ');
    // The title keeps to one line, so a narrow detail never grows taller:
    // where the row comes from may be cut short, never its range or notes.
    // The tooltip holds all of it; the separators are for screen readers.
    if(ui.largeDetailTitle.textContent!==text)ui.largeDetailTitle.replaceChildren(...(from?[textSpan(document,from,'detail-from')]:[]),textSpan(document,range,'detail-range'),
      ...notes.flatMap(([,note,tone])=>[textSpan(document,' · ','visually-hidden'),textSpan(document,note,`detail-note${tone}`)]));
    const hint=d.encodingRepaired?'Mac OS Roman으로 잘못 읽혀 깨진 UTF-8 문자(예: ÏïàÎÖï → 안녕)를 복원해 표시했습니다. 원본 파일과 내보내기 결과는 바뀌지 않습니다.':'';
    const tip=hint?`${text}\n${hint}`:text;
    if(ui.largeDetailTitle.title!==tip)ui.largeDetailTitle.title=tip;
    // A screen reader in the text hears the repair note the tooltip shows.
    if(ui.largeDetailHint.textContent!==hint)ui.largeDetailHint.textContent=hint;
  }
  // Every list keeps its selected row, but one draws it as primary: the list
  // with keyboard focus (a row menu counts as the list it was opened from),
  // or else the pane whose row the raw detail shows. The others draw theirs
  // as secondary, even on the same row, so the keys' target is always clear.
  // `focused` is the element taking focus, when a list is losing it.
  function markSelections(focused=document.activeElement){
    const primary=paneOf(ready&&ui.rowMenu.contains(focused)?menu.opener():focused)??(detailOwner===logs?'logs':detailOwner===search?'large':null);
    for(const [prefix,rows] of [['logs',ui.logsRows],['large',ui.largeRows],['bookmarks',ui.bookmarkList]])rows.classList.toggle('secondary-selection',primary!==null&&primary!==prefix);
    renderDetailTitle();
  }
  // Focus leaving with the window keeps the marks until it comes back.
  for(const list of [ui.logsViewport,ui.largeViewport,ui.bookmarkList]){list.addEventListener('focusin',()=>markSelections());list.addEventListener('focusout',event=>{if(event.relatedTarget||document.hasFocus?.()!==false)markSelections(event.relatedTarget);});}
  function clearDetail(){detailRender=null;ui.largeDetailText.replaceChildren();ui.largeDetail.hidden=true;updateLowerPane();}
  // A new text opens at the selected row, at its first search hit if it has
  // one: with a few lines shown, a later row of a multi-line entry, or a hit
  // deep in a long row, would be out of view. The entry starts the first
  // window; a later window holds part of one row. An empty inline marker
  // measures where that place starts, wrapped lines included.
  function revealDetailLine(d){
    const text=ui.largeDetailText;text.scrollTop=0;
    if(d.loading)return;
    let start=0;
    for(let rows=d.startByte===0&&Number.isSafeInteger(d.first)?d.lineNumber-d.first:0;rows>0&&start>=0;rows--)start=d.text.indexOf('\n',start)+1||-1;
    if(start<0)return;
    const end=d.text.indexOf('\n',start),hit=(Array.isArray(d.highlights)?d.highlights:[]).find(p=>Number.isSafeInteger(p?.[0])&&p[0]>=start&&(end<0||p[0]<end));
    let place=hit?hit[0]:start;
    const walker=place>0?document.createTreeWalker?.(text,4):null;
    for(let node=walker?.nextNode();node;node=walker.nextNode()){
      if(place>=node.data.length){place-=node.data.length;continue;}
      const marker=document.createElement('span'),parent=node.parentNode;
      node.splitText(place).before(marker);
      text.scrollTop=marker.getBoundingClientRect().top-text.getBoundingClientRect().top;
      marker.remove();parent.normalize();return;
    }
  }
  // Shades mark the edges of the text with more of it past them.
  function renderEdges(){
    const text=ui.largeDetailText,above=text.scrollTop;
    text.classList.toggle('more-above',above>1);text.classList.toggle('more-below',text.scrollHeight-text.clientHeight-above>1);
  }
  ui.largeDetailText.addEventListener('scroll',renderEdges,{passive:true});
  // How many lines of the text are out of view, on a button that fits the
  // detail to them as a double-click on its splitter does.
  function renderHiddenLines(){
    renderEdges();
    const text=ui.largeDetailText,line=parseFloat(window.getComputedStyle?.(text)?.lineHeight);
    const hidden=!ui.largeDetail.hidden&&line>0?Math.round((text.scrollHeight-text.clientHeight)/line):0;
    ui.largeDetailMore.hidden=!(hidden>0);
    if(hidden>0&&ui.largeDetailMore.textContent!==`+${formatNumber(hidden)}줄`){ui.largeDetailMore.textContent=`+${formatNumber(hidden)}줄`;ui.largeDetailMore.setAttribute('aria-label',`${formatNumber(hidden)}줄 더 보기`);}
  }
  // Focus on the button, which hides once nothing is out of view, moves to
  // the text it revealed.
  ui.largeDetailMore.addEventListener('click',()=>{const focused=document.activeElement===ui.largeDetailMore;layout.fitDetail();renderHiddenLines();if(focused&&ui.largeDetailMore.hidden)ui.largeDetailText.focus({preventScroll:true});});
  if(window.ResizeObserver)new window.ResizeObserver(renderHiddenLines).observe(ui.largeDetailText);
  function detail(state,owner){
    if(detailOwner!==owner)return;
    ui.largeDetail.hidden=!state.detail;updateLowerPane();
    if(!state.detail){if(detailRender)clearDetail();return;}
    const d=state.detail;renderDetailTitle(d,owner);
    $('largeDetailNavigation').hidden=d.loading||(d.startByte===0&&d.complete);
    // Controller snapshots clone the detail wrapper, while immutable native
    // content/range objects retain their identity until a new range arrives.
    const identity=[owner,state.datasetId,state.queryId,d.lineNumber,d.startByte,d.loading,d.text,d.highlights,d.encodingDamage];
    if(!detailRender||identity.some((value,i)=>value!==detailRender[i])){
      ui.largeDetailText.replaceChildren(highlightedSpan(document,d.loading?'원문 구간을 읽는 중…':d.text,'',d.highlights,d.encodingDamage));detailRender=identity;
      revealDetailLine(d);renderHiddenLines();
    }
    ui.largeDetailPrevious.disabled=d.loading||d.startByte===0;ui.largeDetailNext.disabled=d.loading||d.complete;ui.largeDetailCopy.disabled=d.loading;
  }
  // Field problems are shown beside their field; the banner keeps file and
  // engine errors, plus field errors the closed filter panel would hide.
  const REGEX_ERROR='Invalid or unsupported search pattern: ';
  // How long a typed message rests before a pattern the engine rejected is
  // explained; `patternHeld` while it has not.
  const PATTERN_PAUSE_MS=600;let patternHeld=false,patternTimer=null;
  const SOURCE_CHANGES=new Set(['Source changed; reopen the file','Source truncated']);
  function errors(){
    const lower=search.getState(),upper=logs.getState();
    const [pidMessage,tidMessage,tagMessage]=[[idListError,ui.pidFilter],[idListError,ui.tidFilter],[tagListError,ui.tagFilter]].map(([check,input])=>validationError?check(input.value):'');
    const fromBad=validationError&&!isTimeBound(ui.timeFrom.value),toBad=validationError&&!isTimeBound(ui.timeTo.value);
    const idMessage=pidMessage||tidMessage,listMessage=tagMessage;
    // A well-formed but inverted dated range is applied; it just matches nothing.
    const timeMessage=fromBad||toBad?'시간은 06-18 19:24:05.123처럼 로그와 같은 형식으로, 앞에서부터 입력해 주세요.':isEmptyTimeRange(ui.timeFrom.value,ui.timeTo.value)?'끝 시각이 시작 시각보다 앞서 일치하는 행이 없습니다. 연도 없는 날짜 범위는 해를 넘기지 않습니다.':'';
    // A message pattern only the engine can check: the upper logs keep the
    // last conditions it accepted. The editor shows why while it edits a
    // condition with a pattern; otherwise the banner does.
    const patternOf=value=>String(value??'').startsWith(MESSAGE_PATTERN_ERROR)?value:'';
    const pattern=upper.rejected||patternOf(!upper.closed&&upper.error)||patternOf(!lower.closed&&lower.error);
    const editing=activeCondition(),inField=!!pattern&&ui.filterPanel.inert!==true&&(upper.rejected?badPattern(editing):!!editing?.message&&editing.messageRegex===true);
    // A pattern is explained once typing pauses, not at its first "(".
    const patternMessage=inField&&!patternHeld?describeLogError(pattern,shownFormat):'';
    ui.messageFilterError.textContent=patternMessage;ui.messageFilterError.hidden=!patternMessage;ui.messageFilter.setAttribute('aria-invalid',String(!!patternMessage));
    // Invalid field input leaves the upper logs on the last applied conditions.
    const stale=(validationError||!!upper.rejected)&&!disposed;ui.logsPending.hidden=!stale;ui.logsPending.textContent=stale?'입력한 조건 미적용 · 이전 조건 결과':'';syncFilterRow();
    ui.idFilterError.textContent=idMessage;ui.idFilterError.hidden=!idMessage;
    ui.tagFilterError.textContent=listMessage;ui.tagFilterError.hidden=!listMessage;
    ui.timeFilterError.textContent=timeMessage;ui.timeFilterError.hidden=!timeMessage;
    for(const [input,bad] of [[ui.pidFilter,pidMessage],[ui.tidFilter,tidMessage],[ui.tagFilter,tagMessage],[ui.timeFrom,fromBad],[ui.timeTo,toBad]])input.setAttribute('aria-invalid',String(!!bad));
    const lowerError=!lower.closed&&lower.error||'',regexError=lowerError.startsWith(REGEX_ERROR)?describeLogError(lowerError):'';
    ui.searchError.textContent=regexError;ui.searchError.hidden=!regexError;ui.searchInput.setAttribute('aria-invalid',String(!!regexError));
    const fieldMessage=idMessage||listMessage||timeMessage;
    const followMessage=!upper.closed&&upper.followError?describeFollowError(upper.followError):'';
    // The capture bar only says a capture stopped; the bar here says why.
    const captureMessage=!upper.closed&&upper.capture?.state==='failed'?upper.capture.error:'';
    const raw=inputError||(fieldMessage&&ui.filterPanel.inert===true?fieldMessage:'')||(pattern&&!inField?describeLogError(pattern,shownFormat):'')||(regexError||patternOf(lowerError)?'':lowerError)||(!upper.closed&&!patternOf(upper.error)&&upper.error)||followMessage||captureMessage||'';
    const message=describeLogError(raw);ui.largeError.textContent=message;ui.largeError.hidden=!message;ui.errorBar.hidden=!message;
    // A changed file is one action away from being read again with its work.
    ui.errorReopen.hidden=!(SOURCE_CHANGES.has(raw)&&canReopen(upper));
  }
  function renderLogs(state){
    extras?.selected('logs',state.datasetId,state.selectedLine);
    // CLI sources can already be indexing before the event listener attaches.
    if(!lastSourceId&&Number.isSafeInteger(state.datasetId)&&state.datasetId>0){
      lastSourceId=state.datasetId;if(!state.textComplete)pendingSourceId=state.datasetId;
      void adopt(state.datasetId);
    }
    updateInputCancellation(state);
    const hasFile=state.datasetId!==null&&!state.closed&&state.status!=='empty';
    ui.emptyState.hidden=hasFile;ui.logContent.hidden=!hasFile;ui.largeClose.hidden=!hasFile;renderReopen(state,hasFile);
    // Filters need a file; focus leaving the disabled rail/panel lands on the empty state.
    // Reconnecting to the next file keeps the rail as it was instead of flickering.
    if((hasFile||state.status!=='connecting'||!lastSourceId)&&layout.setAvailable(hasFile))$('emptyOpen').focus?.({preventScroll:true});
    const loading=hasFile&&!state.textComplete&&state.size>0&&!['error','cancelled'].includes(state.status);
    ui.loadProgress.hidden=!loading;
    if(loading){const percent=Math.min(100,Math.floor(state.indexedBytes/state.size*100));ui.loadProgressBar.style.width=`${percent}%`;ui.loadProgress.setAttribute('aria-valuenow',String(percent));}
    ui.largeName.textContent=hasFile?state.name:'열린 파일 없음';
    const files=hasFile&&state.files?.length>1?state.files:null;layout.setMultiFile(!!files);if(hasFile){applyLevelNames(state.ios===true||state.capture?.platform==='ios');applyFormat(state.format);}if(hasFile)layout.fitLines(widestLine(state));
    ui.largeName.title=!hasFile?'':files?`${files.length}개 파일을 이 순서로 합쳤습니다\n${files.map((file,i)=>`${i+1}. ${file.name} · ${formatSize(file.size)}`).join('\n')}\n합계 ${formatSize(state.size)}`:`${state.name}\n${formatSize(state.size)}`;ui.largeSize.textContent=hasFile&&state.size>0?formatSize(state.size):'';
    const p=describeLargeProgress(state,{omitDuplicateFileCount:true});ui.logsCount.textContent=`로그 · ${p.count}`;
    showRange('logs',logs,p.range.replaceAll('검색 결과','필터 결과'),true);
    ui.logsLast.textContent=p.exact?'끝':'현재 끝';ui.largeStatus.textContent=p.status;updateLevels(state.summary,state.summaryComplete);
    const empty=state.closed||!state.count;ui.logsFirst.disabled=empty||state.viewportStart===0;ui.logsLast.disabled=empty||state.viewportStart>=state.count-1;
    ui.logsJump.disabled=!state.count||state.closed;ui.logsJump.title=jumpTitle('현재 필터 결과',state);
    ui.exportLogs.disabled=exporting||!hasFile||!state.queryComplete||!state.count;
    renderFollow(state,hasFile);devices?.render(state);
    // The empty state lists capture files an earlier run left.
    if(hasFile!==shownFile){shownFile=hasFile;if(!hasFile)void devices?.loadLeftovers();}
    table(state,'logs',logs,false);detail(state,logs);errors();markSelections();resolveMarks(state);
    // Cards and chips say when the engine finds no row for a condition, and
    // chips show the conditions kept when it rejects a message pattern.
    const marks=rowless(state),seen=JSON.stringify([[...marks.include],[...marks.exclude],marks.following,state.rejected]);
    if(seen!==rowlessShown){rowlessShown=seen;renderFilterSummary();}
    if(hasFile&&state.count)tips?.offer('help');
  }
  // The conditions no row of the open log matches, as the logs pane's query
  // checked them; none while a file is closed.
  let rowlessShown='';
  function rowless(state=logs.getState()){
    const open=state.datasetId!==null&&!state.closed&&state.status!=='empty';
    const marks=open?conditionsWithoutRows(conds,state.checkedConditions,state.conditions):{include:new Set(),exclude:new Set()};
    return {...marks,following:state.following};
  }
  // Following a single opened file: rows appended to it keep arriving, and
  // the upper list keeps its end in view while it is there. A replaced last
  // row (appended bytes completed it) is read again where it is shown.
  let followSource=null,followRevision=0,shownFile=false;
  function renderFollow(state,hasFile){
    const following=hasFile&&!!state.following;
    ui.followFile.hidden=!hasFile||!state.followable;
    ui.followFile.disabled=!state.textComplete||state.closed||state.status==='error'||followBusy;
    ui.followFile.setAttribute('aria-pressed',String(following));
    const live=state.capture&&['recording','waiting'].includes(state.capture.state);
    ui.followFile.title=live?following?'따라가기 끄기 · 화면만 멈추고 기록은 계속합니다':`${state.grown?'새 줄이 있습니다 · ':''}기기에서 들어오는 새 줄을 이어서 보여 줍니다`:following?'따라가기 끄기 · 지금까지 읽은 줄에서 멈춥니다':`${state.grown?'파일 끝에 새 줄이 있습니다 · ':''}파일 끝에 추가되는 줄을 이어서 읽고 위 로그의 끝을 보여 줍니다`;
    const exportHint=following?'따라가기를 끄면 내보낼 수 있습니다':'';
    ui.exportLogs.title=exportHint||'현재 필터 결과의 원문을 파일로 저장';ui.exportSearch.title=exportHint||'검색 결과의 원문을 파일로 저장';
    if(followSource!==state.datasetId){followSource=state.datasetId;followRevision=state.tailRevision??0;return;}
    if(!Number.isSafeInteger(state.tailRevision)||state.tailRevision===followRevision)return;
    followRevision=state.tailRevision;
    for(const view of viewports.values())view.refresh(row=>row);
    const d=detailOwner?.getState().detail;
    if(d&&!d.loading)void detailOwner.loadDetail(d.lineNumber,d.startByte);
  }
  let followBusy=false;
  ui.followFile.addEventListener('click',async()=>{
    const state=logs.getState();if(disposed||followBusy||!state.followable)return;
    const on=!state.following;followBusy=true;renderLogs(logs.getState());
    try{await logs.setFollow(on);}finally{followBusy=false;}
    if(disposed||pageGone)return;
    // The search pane learns the change from its next status.
    void search.start();
    if(on&&logs.getState().following)viewports.get('logs')?.scrollToEnd();
    renderLogs(logs.getState());
  });
  function renderSearch(state){
    extras?.selected('search',state.datasetId,state.selectedLine);
    const p=describeLargeProgress({...state,inputPending,inputPaused,inputInvalid:validationError},{compactSearch:true});
    // Field edits keep the lower pane closed until there is a search to show.
    const visible=!!ui.searchInput.value;updateLowerPane();
    // New intent has invalidated the old result set. Identical pending status
    // text needs no DOM rewrite while the replacement waits for debounce.
    // Scanned bytes show how far a long search has reached through the file.
    const scanning=p.query==='검색 중'&&!inputPending&&state.size>0&&state.scannedBytes>0;
    const queryText=p.query?` · ${p.query}${scanning?` ${Math.min(99,Math.floor(state.scannedBytes/state.size*100))}%`:''}`:'';
    if(ui.largeCount.textContent!==p.count)ui.largeCount.textContent=p.count;
    if(ui.largeTotal.textContent!==queryText)ui.largeTotal.textContent=queryText;
    if(ui.largeRange.textContent!==(findingText(search)||p.range))showRange('large',search,p.range,true);
    ui.largeSearchPane.setAttribute('aria-busy',String(visible&&p.query==='검색 중'));
    if(inputPending)ui.largeRows.setAttribute('data-search-pending','true');else ui.largeRows.removeAttribute('data-search-pending');
    // An emptied search leaves an idle identity: there is nothing to cancel.
    ui.largeCancel.disabled=!state.datasetId||state.closed||!ui.searchInput.value||((!inputPending||inputPaused)&&(state.queryStatus==='idle'||FINAL_QUERY_STATES.has(state.queryStatus)));ui.largeClose.disabled=state.closed||!state.datasetId;
    updateSearchCancel(state);
    ui.largeLast.textContent='끝';ui.largeJump.title=jumpTitle('검색 결과',logs.getState());ui.largeJump.disabled=inputPending||!state.count||state.closed;
    const empty=inputPending||!state.count||state.closed;ui.largeFirst.disabled=empty||state.viewportStart===0;ui.largeLast.disabled=empty||state.viewportStart>=state.count-1;
    ui.exportSearch.disabled=exporting||inputPending||!ui.searchInput.value||state.closed||!state.queryComplete||!state.count;
    table(inputPending?{...state,queryComplete:false}:state,'large',search,true);detail(state,search);errors();markSelections();syncUpperMarks(state);renderPosition(state);
    if(visible&&lowerTab==='search'&&!inputPending&&!state.closed&&state.count)tips?.offer('search');
  }
  // Upper rows mark the results of the search the engine has accepted; new
  // search intent unmarks them at once, and the upper query never restarts.
  const unmark=row=>row&&(row.searchHit||row.highlights?.length)?{...row,highlights:[],searchHit:false}:row;
  function syncUpperMarks(state){
    const id=!disposed&&!inputPending&&!validationError&&!!ui.searchInput.value&&state.queryAccepted&&!state.closed?state.queryId:null;
    if(logs.setSearchMarks(id))viewports.get('logs')?.refresh(unmark,{reread:id!==null});
  }
  // Cancel shows inside the search box only while a started search runs, and
  // only after a moment, so quick searches and typing do not flash it. Focus
  // on a cancel that disappears moves back to the search input.
  const CANCEL_DELAY_MS=400;let cancelTimer=null;
  const searchRunning=state=>!ui.largeCancel.disabled&&!inputPending&&state.queryStatus==='running'&&!state.caughtUp;
  function updateSearchCancel(state){
    if(searchRunning(state)){
      if(ui.largeCancel.hidden&&cancelTimer===null)cancelTimer=window.setTimeout(()=>{cancelTimer=null;if(!disposed&&searchRunning(search.getState()))ui.largeCancel.hidden=false;},CANCEL_DELAY_MS);
      return;
    }
    if(cancelTimer!==null)window.clearTimeout(cancelTimer);cancelTimer=null;
    if(ui.largeCancel.hidden)return;
    const focused=document.activeElement===ui.largeCancel;ui.largeCancel.hidden=true;if(focused)ui.searchInput.focus();
  }
  const logs=createLargeLogController({transport,onChange:renderLogs,onLocate:result=>located(logs,result),onRelocate:result=>relocated(logs,result),pane:'logs'});
  const search=createLargeLogController({transport,onChange:renderSearch,onLocate:result=>located(search,result),onRelocate:result=>relocated(search,result),pane:'search'});
  // The search the lower pane last started; another filter keeps its place.
  let searchKey=null;
  const searchOf=f=>JSON.stringify([f.query,!!f.caseSensitive,f.searchMode==='regex']);
  function apply(){
    if(debounce!==null)window.clearTimeout(debounce);debounce=null;if(disposed)return;
    try{
      const filters=readFilters();validateFilters(filters);validationError=false;inputPending=false;inputPaused=false;appliedFilters=filters;saver.schedule();
      const upper={...filters,query:'',caseSensitive:false,searchMode:'literal'}, fingerprint=JSON.stringify(coreFilters(upper));
      // Both lists keep their place across a new filter; a new search starts
      // at its first result.
      if(fingerprint!==upperFingerprint){upperFingerprint=fingerprint;void logs.setFilters(upper,{place:placeOf('logs',logs)});}
      renderFilterSummary(filters);
      const key=filters.query?searchOf(filters):null;
      if(key)void search.setFilters(filters,{place:key===searchKey?placeOf('large',search):null});else {search.invalidateQuery();renderSearch(search.getState());}
      searchKey=key;
    }catch{validationError=true;inputPending=true;conditionView?.refresh(conds,active,shownFormat);renderSearch(search.getState());}
  }
  const MAX_COPY_ROWS=10000;
  function paneOf(node){return !node?null:ui.largeViewport.contains(node)?'large':ui.logsViewport.contains(node)?'logs':ui.bookmarkList.contains(node)?'bookmarks':null;}
  function activePrefix(){return paneOf(document.activeElement);}
  // The rows at the selected positions of `state`'s query, in list order,
  // read a page at a time. The positions are that query's, so a query
  // replaced between pages rejects rather than mixing in another's rows.
  async function readSelected(owner,ranges,state){
    const rows=[];let page=[],from=0;
    for(const [first,last] of ranges)for(let i=first;i<=last;i++){
      if(i<from||i>=from+page.length){
        const now=owner.getState();if(now.datasetId!==state.datasetId||now.queryId!==state.queryId)throw new Error('Copy source changed');
        page=await owner.readRows(i);from=i;if(!page?.length)throw new Error('Copy source changed');
      }
      rows.push(page[i-from]);
    }
    return rows;
  }
  // Copies keep raw-log fidelity: every selected row is its original text,
  // in list order, joined by LF.
  function copyPlan(prefix=activePrefix()){
    if(!prefix)return null;
    if(prefix==='bookmarks')return bookmarks.copyPlan();
    const owner=prefix==='logs'?logs:search,state=owner.getState(),view=viewports.get(prefix);
    if(!state.selectedLine||state.closed||!view)return null;
    const selection=selectionOf(prefix,state);
    if(!selection)return {count:1,text:null,load:()=>owner.readLineText(state.selectedLine)};
    const ranges=selectedRanges(selection,state.count),count=selectedCount(selection,state.count);
    if(count>MAX_COPY_ROWS)return {count,error:`한 번에 최대 ${formatNumber(MAX_COPY_ROWS)}행까지 복사할 수 있습니다. 결과 전체는 내보내기로 저장할 수 있습니다.`};
    let text=[];
    for(const [first,last] of ranges){const rows=view.cachedRows(first,last);if(!rows||rows.some(row=>row.truncated)){text=null;break;}for(const row of rows)text.push(row.raw??'');}
    return {count,text:text?.join('\n')??null,load:async()=>{
      const lines=[];
      for(const row of await readSelected(owner,ranges,state))lines.push(row.truncated?await owner.readLineText(row.lineNumber):row.raw??'');
      return lines.join('\n');
    }};
  }
  // Row actions add conditions rather than rewrite one the user may not be
  // looking at; each says in a toast what it did and can be undone there.
  // A showing value fills a blank showing condition, or starts a new one.
  const menu=createPopupMenu({menu:ui.rowMenu,document,window,scrollers:[ui.logsViewport,ui.largeViewport,ui.bookmarkList]});
  const quote=text=>text.length>24?`${text.slice(0,24)}…`:text;
  function addShowing(values){
    syncEditor();
    const others=conds.include.some(c=>c.enabled!==false&&!isEmptyCondition(c));
    const blank=conds.include.findIndex(c=>c.enabled!==false&&isEmptyCondition(c));
    let c;
    if(blank<0){if(!addCondition('include',values))return;c=conds.include.at(-1);}
    else{c=conds.include[blank];Object.assign(c,values);active={list:'include',index:blank};loadEditor();buildConditions();}
    apply();
    const summary=conditionSummary(c,shownFormat);
    offerUndo(`보기 조건을 추가했습니다 · ${summary}${others?' · 기존 보기 조건과 이 조건 중 하나라도 맞으면 보입니다':''}`,()=>{
      syncEditor();const i=conds.include.indexOf(c);if(i<0)return;
      if(blank<0){removeCondition('include',i);return;}
      Object.assign(c,emptyCondition());loadEditor();buildConditions();apply();
    });
  }
  // An excluded tag joins the first enabled hiding condition of tags only,
  // or starts one; the toast says which, and undoes it.
  function addExcludedTag(tag){
    syncEditor();
    const target=conds.exclude.find(c=>c.enabled!==false&&LEVELS.every(level=>c.levels.includes(level))&&!idList(c.pid).length&&!idList(c.tid).length&&!c.timeFrom.trim()&&!c.timeTo.trim());
    if(target){
      const before=target.tag,list=tagList(before);
      if(list.some(item=>item.toLowerCase()===tag.toLowerCase()))return;
      list.push(tag);target.tag=list.join(', ');
      loadEditor();buildConditions();apply();
      offerUndo(`제외 조건을 수정했습니다 · ${conditionSummary(target,shownFormat)}`,()=>{
        syncEditor();if(!conds.exclude.includes(target))return;
        target.tag=before;loadEditor();buildConditions();apply();
      });
      return;
    }
    if(!addCondition('exclude',{tag},{select:false}))return;
    const c=conds.exclude.at(-1);
    loadEditor();buildConditions();apply();
    offerUndo(`제외 조건을 추가했습니다 · ${conditionSummary(c,shownFormat)}`,()=>{
      syncEditor();const i=conds.exclude.indexOf(c);if(i>=0)removeCondition('exclude',i);
    });
  }
  // A hidden message is a hiding condition of its own; the toast undoes it.
  function addExcludedMessage(message){
    syncEditor();
    if(conds.exclude.some(c=>c.enabled!==false&&c.message===message&&!c.messageRegex&&isEmptyCondition({...c,message:''})))return;
    if(!addCondition('exclude',{message},{select:false}))return;
    const c=conds.exclude.at(-1);
    loadEditor();buildConditions();apply();
    offerUndo(`제외 조건을 추가했습니다 · ${conditionSummary(c,shownFormat)}`,()=>{
      syncEditor();const i=conds.exclude.indexOf(c);if(i>=0)removeCondition('exclude',i);
    });
  }
  function copyFromPane(prefix){
    const plan=copyPlan(prefix);if(!plan)return;
    if(plan.error)return notify(plan.error);
    writeClipboard(window,plan.text??plan.load()).then(()=>notify(copied(plan.count)),()=>notify(copyFailed));
  }
  function openRowMenu(entry,index,prefix,owner,isSearch,x,y){
    if(disposed)return;
    const items=[...textItems(ui[prefix+'Rows'],entry.lineNumber),...conditionItems(entry,pickedText(ui[prefix+'Rows'],entry.lineNumber))],state=owner.getState(),size=selectedCount(selectionOf(prefix,state),state.count)||1;
    items.push([size>1?`선택한 ${formatNumber(size)}행 복사`:'행 원문 복사',()=>copyFromPane(prefix)]);
    // A row of a multi-line log entry also copies the whole entry.
    const next=Number.isSafeInteger(index)?viewports.get(prefix)?.cachedRows(index+1,index+1)?.[0]:null;
    if(size===1&&(continuationOwner(entry)||(entry.kind==='parsed'&&continuationOwner(next)?.lineNumber===entry.lineNumber)))items.push(['로그 전체 원문 복사',()=>copyRecord(owner,owner.readRecordText(entry.lineNumber))]);
    items.push([size>1?`선택한 ${formatNumber(size)}행 북마크 추가/해제`:bookmarks.has(entry.lineNumber)?'북마크 해제':'북마크 추가',()=>toggleBookmarks(prefix)]);
    if(isSearch)items.push(['위 로그로 이동',()=>jumpTo(entry.lineNumber)]);
    menu.show(items,x,y,'행 작업',{onList:true});
  }
  // The bookmark list's menu: the same conditions, then its selection's copy
  // and unmark.
  function openBookmarkMenu(mark,size,x,y){
    if(disposed)return;
    const items=[...textItems(ui.bookmarkList,mark.lineNumber),...conditionItems(mark,pickedText(ui.bookmarkList,mark.lineNumber))];
    items.push([size>1?`선택한 ${formatNumber(size)}행 복사`:'행 원문 복사',()=>copyFromPane('bookmarks')]);
    items.push([size>1?`선택한 북마크 ${formatNumber(size)}개 해제`:'북마크 해제',()=>bookmarks.removeSelected()]);
    menu.show(items,x,y,'북마크 작업',{onList:true});
  }
  // Text selected inside the row (the one selected row of its list) is
  // copied from the menu too, since the menu replaces the browser's own.
  function textItems(rows,line){
    const value=pickedText(rows,line);if(!value)return [];
    return [['선택한 텍스트 복사',()=>{writeClipboard(window,value).then(()=>notify('선택한 텍스트를 복사했습니다.'),()=>notify(copyFailed));}],null];
  }
  // Text selected inside the row of `line` in `rows`, or ''.
  function pickedText(rows,line){
    const text=window.getSelection?.(),value=text&&!text.isCollapsed?String(text):'',row=value?rowOf(text.anchorNode):null;
    return row&&rows?.contains?.(row)&&row.dataset.lineNumber===String(line)?value:'';
  }
  // Conditions a row's values make, then a separator; none for an
  // unrecognized row. The message conditions take `picked`, text selected in
  // the row's message, or else the whole message.
  function conditionItems(entry,picked=''){
    if(entry.kind!=='parsed')return [];
    // A DLT row's tags are its ECU, APID and CTID, named as such.
    const tags=entry.dlt?DLT_IDS.map(([key,label])=>[label,entry.dlt[key]]):(Array.isArray(entry.tags)&&entry.tags.length?entry.tags:[entry.tag]).map(tag=>['태그',tag]);
    const named=tags.filter(([,tag])=>typeof tag==='string'&&tag.trim()).slice(0,3);
    // Showing conditions combine with 또는, so with another one in use an
    // added condition shows more rows, not fewer.
    syncEditor();
    const note=conds.include.some(c=>c.enabled!==false&&!isEmptyCondition(c))?'지금 보기 조건에 더해, 이 값에 맞는 줄도 보입니다':'이 값에 맞는 줄만 보입니다';
    // One group per field, separated, so the menu reads by field.
    const groups=[named.flatMap(([label,tag])=>[[`${label} '${quote(tag)}' 보기 조건 추가`,()=>addShowing({tag}),note],[`${label} '${quote(tag)}' 제외`,()=>addExcludedTag(tag),'이 값에 맞는 줄을 숨깁니다']])];
    const whole=String(entry.message??''),part=picked.trim(),chosen=part&&whole.includes(part),message=(chosen?part:whole.trim()).slice(0,1000),field=entry.dlt?'페이로드':'메시지';
    // Without text selected in the row the whole message is taken; the
    // tooltip says how to take part of it.
    const partly=chosen?'':' · 일부만 쓰려면 텍스트를 선택해 우클릭';
    if(message)groups.push([[`${field} '${quote(message)}' 보기 조건 추가`,()=>addShowing({message}),note+partly],[`${field} '${quote(message)}' 제외`,()=>addExcludedMessage(message),`이 내용이 들어 있는 줄을 숨깁니다${partly}`]]);
    groups.push([['pid','PID'],['tid','TID']].filter(([key])=>/^\d+$/.test(String(entry[key]??''))).map(([key,label])=>[`${label} ${entry[key]} 보기 조건 추가`,()=>addShowing({[key]:String(entry[key])}),note]));
    const stamp=[entry.date,entry.time].filter(Boolean).join(' ');
    if(isTimeBound(stamp)&&stamp)groups.push([[`${stamp}부터 보기 조건 추가`,()=>addShowing({timeFrom:stamp}),note],[`${stamp}까지 보기 조건 추가`,()=>addShowing({timeTo:stamp}),note]]);
    const names=levelNames(shownFormat);
    if(names[entry.level])groups.push([[`${names[entry.level]} 레벨 보기 조건 추가`,()=>addShowing({levels:[entry.level]}),note]]);
    return groups.filter(group=>group.length).flatMap(group=>[...group,null]);
  }
  // Export writes original bytes natively; the page only starts, follows and
  // cancels it. Only completed results can be exported.
  // A DLT log saves either its messages (.dlt) or the rows as shown (.txt).
  async function exportPane(owner,isSearch,format=null){
    const state=owner.getState();
    if(exporting||disposed||!state.queryComplete||!state.count)return;
    exporting=true;exportTotal=state.count;ui.exportCancel.hidden=false;renderLogs(logs.getState());renderSearch(search.getState());
    const stem=String(state.name||'log').replace(/\.[^.]*$/,'')||'log',extension={dlt:'dlt',text:'txt'}[format]??'log';
    try{
      const result=await native.core.invoke('export_results',{pane:isSearch?'search':'logs',datasetId:state.datasetId,queryId:state.queryId,suggestedName:`${stem}-${isSearch?'search':'filtered'}.${extension}`,...(format?{format}:{})});
      if(Number.isSafeInteger(result?.rows))notify(`${formatNumber(result.rows)}행을 내보냈습니다.${Number.isSafeInteger(result.skipped)&&result.skipped>0?` DLT 메시지가 아닌 ${formatNumber(result.skipped)}행은 뺐습니다.`:''}`);
    }catch(error){
      const text=String(error).slice(0,500);
      if(text==='Export cancelled')notify('내보내기를 취소했습니다.');
      else if(!pageGone){inputError=text;errors();}
    }finally{
      exporting=false;ui.exportCancel.hidden=true;
      if(!pageGone){renderLogs(logs.getState());renderSearch(search.getState());}
    }
  }
  function startExport(owner,isSearch,button){
    if(owner.getState().format!=='dlt')return void exportPane(owner,isSearch);
    const box=button.getBoundingClientRect?.();
    menu.show([['DLT 원본 (.dlt)',()=>void exportPane(owner,isSearch,'dlt'),'결과 메시지를 원본 DLT 그대로 저장 · dlt-viewer에서 열 수 있습니다'],['텍스트 (.txt)',()=>void exportPane(owner,isSearch,'text'),'화면에 보이는 행을 텍스트로 저장']],box?box.left:0,box?box.bottom+4:0,'내보내기');
  }
  ui.exportLogs.addEventListener('click',()=>startExport(logs,false,ui.exportLogs));
  ui.exportSearch.addEventListener('click',()=>startExport(search,true,ui.exportSearch));
  ui.exportCancel.addEventListener('click',()=>{void native.core.invoke('cancel_export').catch(()=>{});});
  void native.event.listen('export-progress',({payload})=>{
    if(!exporting||!Number.isSafeInteger(payload?.rows))return;
    notify(`내보내는 중… ${formatNumber(payload.rows)}행${exportTotal?` (${Math.min(99,Math.floor(payload.rows/exportTotal*100))}%)`:''}`);
  }).catch(()=>{});
  // Column width tools are rare: one menu holds them, also on a header right-click.
  const columnMenu=$('columnMenu');
  const columnItems=()=>[['내용에 맞춤',()=>layout.fitColumns(),'현재 표시된 로그에 맞춰 열 폭 조절'],['폭 초기화',()=>layout.resetColumns(),'열 폭을 기본값으로 되돌리기']];
  columnMenu.addEventListener('click',()=>{const box=columnMenu.getBoundingClientRect?.();menu.show(columnItems(),box?box.left:0,box?box.bottom+4:0,'열 폭');});
  for(const header of [ui.logsHeaders,ui.largeHeaders,$('bookmarkHeaders')])header.addEventListener('contextmenu',event=>{if(rawMode||disposed)return;event.preventDefault();menu.show(columnItems(),event.clientX,event.clientY,'열 폭',{onList:true});});
  devices=mountDevices({ui,document,window,native,menu,logs,isGone:()=>pageGone,report:message=>{inputError=message;errors();},unreport:message=>{if(message&&inputError===message){inputError='';errors();}},notify});
  // A row an extension picks is selected in the upper logs, as a typed row
  // number is, without moving focus.
  extras=mountExtras({button:ui.extrasButton,native,menu,notify,isGone:()=>pageGone,
    locate:(datasetId,line)=>{if(disposed||inputPending||validationError||logs.getState().datasetId!==datasetId)return;jumpRequests.set(logs,{line,typed:false});void logs.locateLine(line,{nearest:true});},
    current:()=>{const state=logs.getState();return Number.isSafeInteger(state.selectedLine)?{datasetId:state.datasetId,line:state.selectedLine}:null;}});
  mountRecentFiles({ui,document,native,menu,isGone:()=>pageGone,onError:message=>{inputError=message;errors();},guard:()=>devices.guard(),keep:id=>devices.keep(id),shown:()=>logs.getState().datasetId});
  function repaintRows(){previousRows.clear();renderLogs(logs.getState());renderSearch(search.getState());}
  const bookmarks=createBookmarks({ui,document,window,notify,mod:MOD,logs,lineCell,onReveal:showBookmark,offerUndo:undoBookmarks,openMenu:openBookmarkMenu,isRawMode:()=>rawMode,onChange:()=>{repaintRows();saver.schedule();},onCount:count=>{bookmarkCount=count;updateLowerPane();if(count)tips?.offer('bookmarks');}});
  const saver=createWorkSaver({invoke:(command,args)=>native.core.invoke(command,args),capture:captureWork,setTimer:(fn,ms)=>window.setTimeout(fn,ms),clearTimer:id=>window.clearTimeout(id)});
  saved=mountSavedFilters({native,menu,notify,document,read:()=>fieldsOf(currentFilters()),write:writeFields,changed:()=>saver.schedule(),format:()=>shownFormat,isGone:()=>pageGone});
  conditionView=mountConditions({document,lists:{include:$('showList'),exclude:$('hideList')},hints:{include:$('showHint'),exclude:$('hideHint')},adders:{include:$('addShow'),exclude:$('addHide')},editor:$('condEditor'),rowless:()=>rowless(),problem:rejectedProblem,
    onSelect:(list,index)=>{selectCondition(list,index);ui.tagFilter.focus();},onToggle:toggleCondition,onRemove:dropCondition,
    onAdd:list=>{if(addCondition(list)){ui.tagFilter.focus();apply();}}});
  buildConditions();showOptionalFields();
  // A removal undone brings back the bookmarks tab it was made from.
  function undoBookmarks(message,undo){
    const shown=lowerTab==='bookmarks';
    offerUndo(message,()=>{undo();if(shown)selectLowerTab('bookmarks');});
  }
  // Marks the pane's selected rows, reading those not in view; rows all
  // marked already are unmarked. The bookmark list unmarks its selection.
  function toggleBookmarks(prefix=activePrefix()??(detailOwner===search?'large':'logs')){
    if(prefix==='bookmarks')return bookmarks.removeSelected();
    const owner=prefix==='logs'?logs:search,state=owner.getState(),view=viewports.get(prefix),selection=selectionOf(prefix,state);
    if(!state.selectedLine||!view)return notify(`북마크할 행을 먼저 선택해 주세요.`);
    if(!selection)return notify('선택한 행을 읽는 중입니다. 잠시 후 다시 시도해 주세요.');
    if(selectedCount(selection,state.count)>MAX_BOOKMARKS)return notify(`북마크는 최대 ${formatNumber(MAX_BOOKMARKS)}개까지입니다.`);
    const ranges=selectedRanges(selection,state.count),rows=ranges.flatMap(([first,last])=>view.cachedRows(first,last)??[null]);
    if(!rows.includes(null))return bookmarks.toggle(rows);
    readSelected(owner,ranges,state).then(read=>{if(!disposed)bookmarks.toggle(read);},()=>{if(!disposed)notify('선택한 행을 읽지 못했습니다. 다시 시도해 주세요.');});
  }
  const copied=count=>`${formatNumber(count)}행을 복사했습니다.`;
  const copyFailed='복사하지 못했습니다. 다시 시도해 주세요.';
  // A text selection is copied as text, except one left in a row that is not
  // the focused list's one selected row (a key or a jump moved on).
  function hasTextSelection(){
    const text=window.getSelection?.();if(!String(text??''))return false;
    const row=rowOf(text.anchorNode);
    return !row||(row.closest('.large-viewport')?.contains(document.activeElement)===true&&row.classList.contains('selected-log-row')&&!row.classList.contains('in-selection'));
  }
  // macOS enables Edit → Copy without a text selection only when beforecopy
  // is cancelled; the copy event then supplies the selected rows.
  document.addEventListener('beforecopy',event=>{if(!disposed&&!hasTextSelection()&&!isEditableTarget(event.target)&&copyPlan())event.preventDefault();});
  document.addEventListener('copy',event=>{
    if(disposed||hasTextSelection()||isEditableTarget(event.target))return;
    const plan=copyPlan();if(!plan)return;
    event.preventDefault();
    if(plan.error)return notify(plan.error);
    if(plan.text!==null&&event.clipboardData){event.clipboardData.setData('text/plain',plan.text);return notify(copied(plan.count));}
    notify(`${formatNumber(plan.count)}행을 복사하는 중…`);
    writeClipboard(window,plan.load()).then(()=>notify(copied(plan.count)),()=>notify(copyFailed));
  });
  // A detail copy, like the detail, is the selected row's whole log entry.
  const recordCopied=r=>r.last>r.first?`${formatNumber(r.first)}–${formatNumber(r.last)}행 원문을 복사했습니다.${r.recordComplete===false?' (앞부분 1 MiB)':''}`:`${formatNumber(r.first)}행 원문을 복사했습니다.`;
  function copyRecord(owner,read){
    const done=Promise.resolve(read);
    writeClipboard(window,done.then(r=>r.text)).then(()=>done.then(r=>notify(recordCopied(r))),()=>notify(copyFailed));
  }
  ui.largeDetailCopy.addEventListener('click',()=>{
    const owner=detailOwner,d=owner?.getState().detail;if(!d||d.loading)return;
    copyRecord(owner,d.startByte===0&&d.complete?{text:d.text,first:d.first??d.lineNumber,last:d.last??d.lineNumber,recordComplete:d.recordComplete}:owner.readRecordText(d.lineNumber));
  });
  // An iOS log names Apple's levels (Fault, Default); badges, chips and
  // filter summaries follow, and shown rows are drawn again.
  function applyLevelNames(apple){
    if(!useAppleLevels(apple))return;
    levelKey=apple?'apple':'text';const names=levelNames(shownFormat);
    for(const button of ui.levelFilters.children){const letter=button.querySelector('.letter');if(letter)letter.textContent=names[button.dataset.level];}
    updateLevels();renderFilterSummary();conditionView?.refresh(conds,active,shownFormat);
    for(const view of viewports.values())view.refresh(row=>row,{reread:false});
  }
  // A DLT file filters by ECU, APID and CTID through the tag fields and has
  // no PID, TID, Assert level or numbered rows; text logs keep their fields.
  function applyFormat(format){
    format=format==='dlt'?'dlt':'text';
    if(format===shownFormat)return;
    shownFormat=format;const dlt=format==='dlt',names=levelNames(format);
    layout.setProfile(profileOf(format));
    $('tagLabel').textContent=dlt?'ECU · APID · CTID':'태그';
    // A DLT row's message is its payload, named as DLT tools name it.
    ui.messageLabel.textContent=dlt?'페이로드':'메시지';
    $('openMessage').textContent=dlt?'+ 페이로드':'+ 메시지';$('openMessage').title=dlt?'이 조건의 페이로드 입력란을 엽니다':'이 조건의 메시지 입력란을 엽니다';
    ui.messageFilter.title=dlt?'페이로드에 입력한 문자열이 들어 있으면 조건에 맞습니다 · 대소문자 무시 · 쉼표도 그대로 검색':'메시지에 입력한 문자열이 들어 있으면 조건에 맞습니다 · 대소문자 무시 · 쉼표도 그대로 검색';
    ui.tagFilter.placeholder=dlt?'쉼표로 구분 · 예: NAV, ECU1':'쉼표로 구분 · 예: MyApp';
    ui.tagFilter.title=dlt?'ECU·APID·CTID 중 하나에 쉼표로 구분한 값 중 하나라도 들어 있으면 이 조건에 맞습니다 · 예: NAV, ECU1':'태그에 쉼표로 구분한 값 중 하나라도 들어 있으면 이 조건에 맞습니다 · 예: ActivityManager, MyApp';
    $('readingOptions').hidden=dlt;showOptionalFields();
    if(dlt){$('idFilterHelp').hidden=true;document.querySelector?.('[aria-controls=idFilterHelp]')?.setAttribute('aria-expanded','false');}
    ui.timeFrom.placeholder=dlt?'03-14 09:26':'06-18 19:24';ui.timeTo.placeholder=dlt?'03-14 09:30':'06-18 19:30';
    // Assert stays selected while hidden: DLT has no such level.
    for(const button of ui.levelFilters.children){button.hidden=dlt&&button.dataset.level==='A';button.querySelector('.letter').textContent=names[button.dataset.level];}
    updateLevels();
    // A DLT row has no PID or TID, so no condition keeps them.
    syncEditor();
    const all=[...conds.include,...conds.exclude],ids=dlt&&all.some(c=>c.pid||c.tid);
    if(ids){for(const c of all){c.pid='';c.tid='';}loadEditor();apply();}
    else renderFilterSummary();
  }
  function renderFilterSummary(filters=logs.getState().rejected?keptFilters():readFilters()){
    const chips=describeActiveFilters(filters,shownFormat);
    ui.filterBadge.textContent=chips.length?String(chips.length):'';ui.filterBadge.hidden=!chips.length;
    ui.toggleFilters.setAttribute('aria-label',chips.length?`필터 · ${chips.length}개 조건 적용 중`:'필터');
    ui.activeFilters.hidden=!chips.length;
    // The chips read as the filter: showing conditions joined by 또는, each
    // hiding condition after 제외. A joining word wraps together with the
    // chip it introduces.
    const nodes=[];let group=null;
    const marks=rowless();
    for(const chip of chips){
      const next=chip.list?`${chip.list}:${chip.index}`:'raw',hide=chip.list==='exclude',none=!!marks[chip.list]?.has(chip.index);
      const joint=chip.list&&next!==group&&(hide||group?.startsWith('include'))?textSpan(document,hide?'제외':'또는',`chip-joint${hide?' exclude':''}`):null;
      group=next;
      const button=document.createElement('button');button.type='button';button.className=`filter-chip${hide?' exclude':''}${none?' no-rows':''}`;
      const name=`${hide?'제외 조건 ':''}${chip.label}${none?' (이 로그에 맞는 줄 없음)':''}`;
      button.title=`${hide?'제외 조건 · ':''}${chip.full??chip.label}${none?` · ${rowlessText(chip.list,marks.following)}`:''} · 클릭하면 조건을 해제합니다`;button.setAttribute('aria-label',`${name} 해제`);
      button.append(textSpan(document,chip.label),...(none?[textSpan(document,'줄 없음','chip-note')]:[]),textSpan(document,'×','chip-remove'));
      button.addEventListener('click',()=>clearChip(chip));
      if(!joint){nodes.push(button);continue;}
      const pair=document.createElement('span');pair.className='chip-group';pair.append(joint,button);nodes.push(pair);
    }
    ui.activeFilters.replaceChildren(...nodes);
    syncFilterRow();conditionView?.refresh(conds,active,shownFormat);saved?.render();
  }
  // The chips and the pending note have a row of their own under the heading,
  // wrapping rather than shrinking, so a long filter stays readable; the row
  // shows only while it has either.
  function syncFilterRow(){ui.filterRow.hidden=ui.activeFilters.hidden&&ui.logsPending.hidden;}
  // A chip clears its field, which folds back behind its + button in the
  // editor; a condition left without criteria goes, unless it is the only
  // showing one.
  function clearChip(chip){
    syncEditor();
    if(chip.key==='includeRaw')ui.includeRaw.checked=true;
    else{
      const c=conds[chip.list]?.[chip.index];if(!c)return;
      const before={...c,levels:[...c.levels]};
      if(chip.key==='levels')c.levels=[...LEVELS];else if(chip.key==='time'){c.timeFrom='';c.timeTo='';}else if(chip.key==='message'){c.message='';c.messageRegex=false;}else c[chip.key]='';
      // Clearing its last field takes the condition away, with an undo.
      if(isEmptyCondition(c)&&(chip.list==='exclude'||conds.include.length>1)){Object.assign(c,before);dropCondition(chip.list,chip.index);return;}
      if(c===activeCondition())openedFields=null;
      loadEditor();
    }
    apply();
  }
  function buildConditions(){conditionView?.build(conds,active,shownFormat);}
  // Whether a condition's message pattern may be the one the engine
  // rejected: the engine names no condition, and every pattern among the
  // conditions it kept compiled.
  function badPattern(c){
    if(!logs.getState().rejected||!c?.message||c.messageRegex!==true)return false;
    const kept=logs.filters();
    return ![...kept.include,...kept.exclude].some(k=>k.messageRegex===true&&k.message===c.message);
  }
  // Such a condition's card says it needs checking.
  function rejectedProblem(list,index){return badPattern(conds[list]?.[index])?describeLogError(logs.getState().rejected,shownFormat):'';}
  function setConditions(filters){
    const f=normalizeFilters(filters);
    conds={include:f.include.length?f.include:[emptyCondition()],exclude:f.exclude};active={list:'include',index:0};
    loadEditor();buildConditions();
  }
  function selectCondition(list,index){
    if(active.list===list&&active.index===index)return;
    syncEditor();active={list,index};loadEditor();buildConditions();errors();
  }
  // Adds a condition, edited at once unless `select` is false.
  function addCondition(list,values={},{select=true}={}){
    if(conds.include.length+conds.exclude.length>=MAX_CONDITIONS){notify(`조건은 ${MAX_CONDITIONS}개까지 쓸 수 있습니다.`);return false;}
    syncEditor();conds[list].push({...emptyCondition(),...values});
    if(select)active={list,index:conds[list].length-1};
    loadEditor();buildConditions();return true;
  }
  function removeCondition(list,index){
    syncEditor();conds[list].splice(index,1);
    if(!conds.include.length)conds.include.push(emptyCondition());
    if(active.list===list&&active.index>=index)active=active.index===index?{list:'include',index:0}:{list,index:active.index-1};
    loadEditor();buildConditions();apply();
  }
  function toggleCondition(list,index,on){syncEditor();conds[list][index].enabled=on;apply();}
  // 초기화 and a condition that comes or goes as a whole (a card's ×, a chip
  // clearing its last field, a row action) can be undone from the toast
  // while the same log is open.
  function offerUndo(message,undo){
    const dataset=logs.getState().datasetId;
    notify(message,{label:'실행 취소',run:()=>{if(!disposed&&logs.getState().datasetId===dataset)undo();}});
  }
  // Removes a condition and offers it back where it was; a lone blank
  // showing condition left behind takes it back in place.
  function dropCondition(list,index){
    syncEditor();
    const c=conds[list][index];if(!c)return;
    const kept={...c,levels:[...c.levels]},summary=conditionSummary(c,shownFormat);
    removeCondition(list,index);
    offerUndo(`${list==='include'?'보기':'제외'} 조건을 삭제했습니다 · ${summary}`,()=>{
      syncEditor();
      const blank=list==='include'&&conds.include.length===1&&isEmptyCondition(conds.include[0]);
      if(!blank&&conds.include.length+conds.exclude.length>=MAX_CONDITIONS){notify(`조건은 ${MAX_CONDITIONS}개까지 쓸 수 있습니다.`);return;}
      const at=blank?0:Math.min(index,conds[list].length);
      if(blank)conds.include[0]=kept;else conds[list].splice(at,0,kept);
      active={list,index:at};loadEditor();buildConditions();apply();
    });
  }
  // Enter / Shift+Enter in the search box walk the found results and reveal
  // each one in the upper logs; typing focus stays in the search box. The
  // results tab comes forward first, so the walk is never through hidden rows.
  function stepSearchResult(step,event){
    selectLowerTab('search');
    const state=search.getState(),view=viewports.get('large');
    if(!view||!state.count||state.closed)return;
    event.preventDefault();
    const current=state.selectedLine?view.metrics().selectionIndex:null;
    const next=current===null?(step>0?0:state.count-1):(current+step+state.count)%state.count;
    const wrap=current===null||state.count<2?'':step>0&&next<=current?'다시 처음부터':step<0&&next>=current?'다시 끝부터':'';
    stepNote={queryId:state.queryId,index:next,wrap,announce:true};
    view.selectIndex(next,{force:true});renderPosition();
  }
  // A saved filter replaces the filter's conditions; the search and reading options stay.
  function writeFields(fields){setConditions(fields);ui.includeRaw.checked=fields.includeRaw;apply();}
  // Reset clears the conditions, the search and its options at once; the
  // toast puts them back for the same log, as saved filters do.
  function reset(){
    const before=readFilters(),applied=saved?.applied()??null;
    saved?.setApplied(null);setConditions({});ui.searchInput.value='';ui.caseSensitive.checked=false;regexInput.checked=false;ui.includeRaw.checked=true;apply();
    if(JSON.stringify(before)===JSON.stringify(readFilters()))return;
    offerUndo('필터를 초기화했습니다.',()=>{
      setConditions(before);ui.searchInput.value=before.query;ui.caseSensitive.checked=before.caseSensitive;regexInput.checked=before.searchMode==='regex';ui.includeRaw.checked=before.includeRaw;
      saved?.setApplied(applied);apply();
    });
  }
  for(const level of LEVELS){const b=document.createElement('button');b.className='level-chip';b.dataset.level=level;b.append(textSpan(document,LEVEL_NAMES[level],'letter'),textSpan(document,'0','count'));b.title=`${LEVEL_NAMES[level]} · Alt+클릭: 이 레벨만`;b.addEventListener('click',event=>{if(event.altKey)selectedLevels=new Set([level]);else selectedLevels.has(level)?selectedLevels.delete(level):selectedLevels.add(level);updateLevels();apply();});ui.levelFilters.append(b);}
  ui.allLevels.addEventListener('click',()=>{selectedLevels=new Set(LEVELS);updateLevels();apply();});ui.errorLevels.addEventListener('click',()=>{selectedLevels=new Set(['W','E','F','A']);updateLevels();apply();});ui.resetFilters.addEventListener('click',reset);
  // Typing waits for a pause before it applies; a field other than the
  // search keeps the search results' place.
  function edited(input){if(disposed)return;if(input===ui.searchInput)lowerTab='search';if(debounce!==null)window.clearTimeout(debounce);inputPending=!!ui.searchInput.value||input!==ui.searchInput;inputPaused=false;validationError=false;search.invalidateQuery({place:input===ui.searchInput?null:placeOf('large',search)});renderSearch(search.getState());debounce=window.setTimeout(apply,150);}
  // The message's .* switch reads it as a pattern; without a message it only
  // waits for one.
  ui.messageRegex.addEventListener('click',()=>{messageRegex=!messageRegex;ui.messageRegex.setAttribute('aria-pressed',String(messageRegex));syncEditor();if(ui.messageFilter.value)edited(ui.messageFilter);else conditionView?.refresh(conds,active,shownFormat);});
  // Alt+R switches the message between a string and a pattern.
  ui.messageFilter.addEventListener('keydown',event=>{if(event.altKey&&event.code==='KeyR'&&!event.metaKey&&!event.ctrlKey&&!event.isComposing){event.preventDefault();ui.messageRegex.click();}});
  ui.messageFilter.addEventListener('input',()=>{
    patternHeld=true;if(patternTimer!==null)window.clearTimeout(patternTimer);
    patternTimer=window.setTimeout(()=>{patternTimer=null;patternHeld=false;if(!disposed)errors();},PATTERN_PAUSE_MS);
  });
  for(const input of [ui.searchInput,ui.tagFilter,ui.messageFilter,ui.timeFrom,ui.timeTo,ui.pidFilter,ui.tidFilter]){
    input.addEventListener('input',()=>edited(input));
    input.addEventListener('keydown',event=>{
      if(event.key!=='Enter'||event.isComposing||disposed)return;
      if(inputPending||['cancelled','error'].includes(search.getState().queryStatus)){event.preventDefault();apply();}
      else if(input===ui.searchInput&&ui.searchInput.value)stepSearchResult(event.shiftKey===true?-1:1,event);
    });
  }
  for(const input of [ui.caseSensitive,ui.includeRaw,ui.recognizeLineNumbers,regexInput])input.addEventListener('change',apply);
  for(const [prefix,owner] of [['logs',logs],['large',search]]){
    ui[prefix+'First'].addEventListener('click',()=>owner.seek(0));
    ui[prefix+'Last'].addEventListener('click',()=>owner.seek(Math.max(0,owner.getState().count-1)));
    ui[prefix+'JumpForm'].addEventListener('submit',event=>{event.preventDefault();jumpToRow(prefix,owner);});
    ui[prefix+'Jump'].addEventListener('input',()=>ui[prefix+'Jump'].removeAttribute('aria-invalid'));
  }
  function setView(raw){rawMode=raw;for(const id of ['largeTable','logsTable','bookmarkTable'])$(id).classList.toggle('raw-mode',raw);ui.largeColumns.setAttribute('aria-pressed',String(!raw));ui.largeRaw.setAttribute('aria-pressed',String(raw));layout.setRawMode(raw);bookmarks.render();previousRows.clear();renderLogs(logs.getState());renderSearch(search.getState());}
  ui.largeColumns.addEventListener('click',()=>setView(false));ui.largeRaw.addEventListener('click',()=>setView(true));
  ui.largeDetailClose.addEventListener('click',()=>detailOwner?.closeDetail());ui.largeDetailNext.addEventListener('click',()=>{const d=detailOwner?.getState().detail;if(d&&!d.loading&&!d.complete)void detailOwner.loadDetail(d.lineNumber,d.nextByte);});ui.largeDetailPrevious.addEventListener('click',()=>{void detailOwner?.previousDetail();});
  ui.largeCancel.addEventListener('click',()=>{if(debounce!==null)window.clearTimeout(debounce);debounce=null;if(inputPending)inputPaused=true;void search.cancel();});
  const closeSearch=search.close;
  function captureInput(){return {filters:readFilters(),savedFilter:saved?.applied()??null,upper:logs.getState(),lower:search.getState(),bookmarks:bookmarks.snapshot()};}
  async function reconnect(snapshot,valid=()=>!pageGone){
    if(!valid())return;
    const filters=snapshot?.filters??defaultFilters();
    disposed=false;ui.searchInput.value=filters.query??'';setConditions(filters);
    ui.caseSensitive.checked=filters.caseSensitive;regexInput.checked=filters.searchMode==='regex';ui.recognizeLineNumbers.value=filters.recognizeLineNumbers===false?'off':filters.recognizeLineNumbers===true?'on':'auto';ui.includeRaw.checked=filters.includeRaw;
    inputPending=inputPaused=validationError=false;detailOwner=null;clearDetail();previousRows.clear();updateLevels();saved?.setApplied(snapshot?.savedFilter??null);renderFilterSummary(filters);
    // Bookmarks belong to one opened file; a rolled-back file gets its own back.
    // Marks restored by line number are read again once the file is read.
    bookmarks.restore(snapshot?.bookmarks);
    pendingMarks=snapshot?.marks?{...snapshot.marks,datasetId:null,busy:false,failures:0}:null;restoreTargets=null;appliedFilters=filters;
    const upper={...filters,query:'',caseSensitive:false,searchMode:'literal'};upperFingerprint=JSON.stringify(coreFilters(upper));searchKey=filters.query?searchOf(filters):null;
    await Promise.all([logs.reopen(upper),search.reopen(filters.query?filters:DEFAULT_FILTERS)]);
    if(!valid()||disposed)return;
    if(pendingMarks)pendingMarks.datasetId=logs.getState().datasetId;
    // A reload and saved work return to physical rows; a rollback returns to
    // the same dataset, so its list positions are exact.
    if(snapshot?.positions)restorePlaces(snapshot.positions,filters.query);
    else{
      if(snapshot?.upper?.viewportStart)logs.seek(snapshot.upper.viewportStart,{restore:true,selectedLine:snapshot.upper.selectedLine});
      if(valid()&&snapshot?.lower?.viewportStart&&filters.query)search.seek(snapshot.lower.viewportStart,{restore:true});
    }
    if(snapshot?.notice)notify(snapshot.notice);
    resolveMarks(logs.getState());
  }
  // The filters to keep: what is typed when valid, else what was applied.
  function currentFilters(){const typed=readFilters();try{validateFilters(typed);return logs.getState().rejected?keptFilters(typed):typed;}catch{return appliedFilters??defaultFilters();}}
  // The typed search and options with the conditions the upper logs kept
  // when the engine rejected a message pattern.
  function keptFilters(typed=readFilters()){const kept=logs.filters();return {...typed,include:kept.include,exclude:kept.exclude,includeRaw:kept.includeRaw};}
  function topLine(prefix,state){const i=state.viewportStart;return i>0?viewports.get(prefix)?.cachedRows(i,i)?.[0]?.lineNumber??null:null;}
  // A place still being located is kept as it was saved.
  function positionOf(prefix,owner){
    const state=owner.getState(),target=restoreTargets?.[prefix];
    if(target?.top&&state.locationPending===target.top)return target;
    // So is a place kept across a new filter.
    const kept=owner.relocating();if(kept)return {top:kept.line,selected:kept.select?kept.line:null};
    return {top:topLine(prefix,state),selected:state.selectedLine??null};
  }
  function captureWork(){
    const filters=currentFilters(),marks=new Set(bookmarks.snapshot().keys());
    for(const at of pendingMarks?.lines??[])marks.add(at);
    return {filters,savedFilter:saved?.applied()??null,bookmarks:[...marks].sort((a,b)=>a-b).slice(0,MAX_BOOKMARKS),upper:positionOf('logs',logs),lower:filters.query?positionOf('large',search):{top:null,selected:null}};
  }
  // Each list returns to its first visible row, or the next result after it,
  // and to its selected row once the scan reaches them.
  function restorePlaces(positions,query){
    restoreTargets=positions;
    for(const [prefix,owner] of [['logs',logs],['large',search]]){
      const {top=null,selected=null}=positions[prefix]??{};
      if(prefix==='large'&&!query)continue;
      if(top)void owner.restorePosition(top,selected);else if(selected)owner.seek(0,{restore:true,selectedLine:selected});
    }
  }
  // Restored bookmarks are read again once the file is fully read: rows past
  // the end are dropped, and after a reload rows whose text changed are
  // counted. Marks changed meanwhile keep the user's change.
  function resolveMarks(state){
    const p=pendingMarks;
    if(!p||p.busy||disposed||state.closed||state.datasetId!==p.datasetId||!state.textComplete||!state.queryAccepted)return;
    p.busy=true;
    void (async()=>{
      const wanted=[...new Set([...(p.old?bookmarks.snapshot().keys():[]),...(p.lines??[])])].filter(at=>at<=state.total).sort((a,b)=>a-b),rows=new Map();
      try{
        for(let i=0;i<wanted.length;){
          const data=await logs.readLines(wanted.slice(i,i+LINES_PER_REQUEST));
          // A replaced query answers nothing; the next render tries again.
          if(!data){p.busy=false;return;}
          for(const row of data.rows)if(Number.isSafeInteger(row?.lineNumber))rows.set(row.lineNumber,row);
          if(!data.next)break;i+=data.next;
        }
      }catch{p.busy=false;if(++p.failures>2&&pendingMarks===p)pendingMarks=null;return;}
      const now=logs.getState();
      if(pendingMarks!==p||disposed||now.datasetId!==p.datasetId)return;
      pendingMarks=null;
      const kept=p.old?mergeReloadMarks(bookmarks.snapshot(),p.old,rows,now.total):{entries:[...bookmarks.snapshot()],changed:0,dropped:0};
      const added=p.lines?mergeSavedMarks(new Map(kept.entries),p.lines,rows):{entries:kept.entries,dropped:0};
      bookmarks.restore(added.entries);repaintRows();saver.schedule();
      const text=p.old?describeReloadMarks({changed:kept.changed,dropped:kept.dropped,shrank:now.size<p.size}):describeMissingMarks(added.dropped);
      if(text)notify(text);
    })();
  }
  const workInfo=value=>({reopenable:value?.reopenable===true,match:['exact','changed'].includes(value?.match)?value.match:null,state:value?.state??null,reloadOf:Number.isSafeInteger(value?.reloadOf)?value.reloadOf:null});
  async function loadWork(id){try{return workInfo(await native.core.invoke('work_state',{datasetId:id}));}catch{return workInfo(null);}}
  function setWork(id,info){work={datasetId:id,reopenable:info.reopenable};renderReopen();errors();}
  // Saved work for files opened again: everything when every file is
  // unchanged, filters and search only when a size or time differs.
  function savedSnapshot(info){
    const stored=readWork(info.state);if(!stored||!info.match)return null;
    const exact=info.match==='exact';
    return {filters:stored.filters,savedFilter:stored.savedFilter,positions:exact?{logs:stored.upper,large:stored.lower}:null,marks:exact&&stored.bookmarks.length?{lines:stored.bookmarks}:null,notice:describeRestore(info.match,stored)};
  }
  function reloadSnapshot(){
    const marks=bookmarks.snapshot();
    return {filters:currentFilters(),savedFilter:saved?.applied()??null,bookmarks:marks,positions:{logs:positionOf('logs',logs),large:positionOf('large',search)},marks:{old:marks,size:logs.getState().size,lines:pendingMarks?.lines},notice:RELOADED};
  }
  const validating=state=>pendingSourceId!==null&&(state.datasetId!==pendingSourceId||!state.textComplete);
  function canReopen(state=logs.getState()){return !disposed&&!pageGone&&state.datasetId!==null&&!state.closed&&work.datasetId===state.datasetId&&work.reopenable&&reloadRequest===null&&!validating(state);}
  function renderReopen(state=logs.getState(),hasFile=state.datasetId!==null&&!state.closed&&state.status!=='empty'){
    const text=work.datasetId===state.datasetId&&!work.reopenable?'붙여넣은 텍스트와 샘플 로그는 다시 열 수 없습니다':`같은 파일을 다시 읽습니다 · 필터·검색·북마크·위치 유지 (${MOD}R)`;
    // A capture's file is temporary and tied to its capture.
    ui.reopenFile.hidden=!hasFile||!!state.capture;ui.reopenFile.disabled=!canReopen(state);if(ui.reopenFile.title!==text)ui.reopenFile.title=text;
  }
  // Reads the open files again (same paths, order and source mode), keeping
  // filters, search, view, lower tab, bookmarks and places by physical row.
  function reopenCurrent(){
    const state=logs.getState();
    if(!canReopen(state))return;
    saver.flush();
    const from=state.datasetId;reloadRequest={from,snapshot:reloadSnapshot()};renderReopen();errors();
    void native.core.invoke('reopen_source',{datasetId:from}).catch(error=>{
      if(reloadRequest?.from!==from)return;reloadRequest=null;renderReopen();
      if(!pageGone){inputError=String(error).slice(0,500);errors();}
    });
  }
  // A file opened from the command line can be reading before the page
  // listens for openings; its saved work is asked for once the page sees it.
  async function adopt(id){
    const sequence=sourceEventSequence,info=await loadWork(id);
    if(sequence!==sourceEventSequence||pageGone||disposed||logs.getState().datasetId!==id)return;
    setWork(id,info);
    const snapshot=savedSnapshot(info);
    if(snapshot){
      const next=++sourceEventSequence;
      await reconnect(snapshot,()=>next===sourceEventSequence&&!pageGone);errors();
      if(next!==sourceEventSequence||pageGone)return;
    }
    if(info.reopenable&&!disposed)saver.track(id,info.state);
  }
  function close(){
    saver.flush();saver.stop();work={datasetId:null,reopenable:false};reloadRequest=null;pendingMarks=null;restoreTargets=null;
    sourceEventSequence++;pickerSequence++;pickerPending=false;pendingSourceId=null;savedInput=null;pendingPastePrefix=null;$('cancelInput').hidden=true;
    menu.close(false);bookmarks.restore();
    if(disposed)return Promise.resolve();disposed=true;inputPending=inputPaused=validationError=false;inputError='';
    if(debounce!==null)window.clearTimeout(debounce);debounce=null;clearDetail();
    return Promise.all([closeSearch({remote:false}),logs.close()]);
  }
  search.close=close;search.logs=logs;
  // A capture's unsaved rows are asked about before its log closes.
  ui.largeClose.addEventListener('click',async()=>{if(await devices.guard()!=='cancel')void close();});
  window.addEventListener('pagehide',()=>{
    saver.flush();pageGone=true;sourceEventSequence++;pickerSequence++;pickerPending=false;pendingSourceId=null;savedInput=null;pendingPastePrefix=null;
    clearDetail();
    if(debounce!==null)window.clearTimeout(debounce);debounce=null;disposed=true;
    void closeSearch({remote:false});void logs.close({remote:false});
    for(const view of viewports.values())view.destroy();viewports.clear();
  },{once:true});
  const MODALS=['pasteDialog','shortcutDialog','filterNameDialog','tipLayer','captureDialog','adbDialog'];
  document.addEventListener('keydown',event=>{
    // A modal dialog has made the page inert, so its shortcuts wait until the
    // dialog closes: ⌘O or ⌘R under the paste question would open something
    // that the dialog's 열기 then replaces.
    if(MODALS.some(id=>$(id)?.open===true))return;
    const command=(event.metaKey||event.ctrlKey)&&!event.altKey&&!event.shiftKey,key=String(event.key).toLowerCase();
    if(command&&key==='o'){event.preventDefault();void openPicker();return;}
    // Cmd/Ctrl+R reads the file again; the page itself never reloads.
    if(command&&key==='r'){event.preventDefault();reopenCurrent();return;}
    // Cmd/Ctrl+S saves a shown device capture; other logs are files already.
    if(command&&key==='s'&&logs.getState().capture&&!logs.getState().closed){event.preventDefault();void devices.save();return;}
    if(command&&key==='f'&&!disposed){event.preventDefault();selectLowerTab('search');ui.searchInput.focus();ui.searchInput.select?.();return;}
    if(command&&key==='b'&&!disposed&&!isEditableTarget(event.target)){event.preventDefault();toggleBookmarks();return;}
    // In the search results and the bookmarks ⌘/Ctrl+A selects every row; in
    // the upper logs it does nothing rather than select the page's text.
    if(command&&key==='a'&&!disposed&&!isEditableTarget(event.target)&&activePrefix()){event.preventDefault();if(activePrefix()==='bookmarks')bookmarks.selectAll();else selectAllRows(activePrefix());return;}
    if(event.key==='F2'&&!disposed&&!event.altKey&&!event.metaKey&&!event.ctrlKey){event.preventDefault();bookmarks.step(event.shiftKey?-1:1,{repeat:event.repeat===true});return;}
    if(disposed)return;if(event.key==='/'&&!['INPUT','TEXTAREA'].includes(document.activeElement?.tagName)){event.preventDefault();selectLowerTab('search');ui.searchInput.focus();}
    // The filter panel and popup menus claim Escape first; one press closes one
    // thing: the raw detail, else a pinned panel (not while editing elsewhere),
    // else the focused list's selection of several rows becomes its one row.
    if(event.key==='Escape'&&!event.defaultPrevented){if(detailOwner?.getState().detail){event.preventDefault();detailOwner.closeDetail();}else if(!isEditableTarget(event.target)&&(layout.closePanel()||collapseSelection()))event.preventDefault();}});
  // A selection of several rows becomes its focus row alone.
  function collapseSelection(){
    const prefix=activePrefix();
    if(prefix==='bookmarks')return bookmarks.collapse();
    if(!prefix||inputPending)return false;
    const owner=prefix==='logs'?logs:search,state=owner.getState(),selection=selectionOf(prefix,state);
    if(state.closed||selectedCount(selection,state.count)<2)return false;
    applySelection(prefix,selectOne(selection.focus),{reselect:false});
    return true;
  }
  async function openInput(text,name,size){
    if(pageGone)return;
    const limit=25*1024**2;
    // Paste inspection already measured UTF-8 bytes; avoid encoding it twice.
    if(text.length>limit||(size??new TextEncoder().encode(text).byteLength)>limit){inputError='붙여넣기는 최대 25 MiB입니다. 큰 파일은 파일 선택으로 열어 주세요.';errors();return;}
    try{await native.core.invoke('open_text',{text,name});}catch(error){inputError=String(error).slice(0,500);errors();}
  }
  async function openPicker(){
    if(pageGone||await devices.guard()==='cancel')return;
    const sequence=++pickerSequence,shown=logs.getState().datasetId;pickerPending=true;updateInputCancellation();
    void native.core.invoke('pick_file').then(result=>{if(result?.cancelled)devices.keep(shown);},error=>{devices.keep(shown);if(sequence===pickerSequence&&!pageGone){inputError=String(error).slice(0,500);errors();}}).finally(()=>{if(sequence===pickerSequence){pickerPending=false;updateInputCancellation();}});
  }
  $('openFile').addEventListener('click',()=>void openPicker());$('emptyOpen').addEventListener('click',()=>void openPicker());
  ui.reopenFile.addEventListener('click',reopenCurrent);ui.errorReopen.addEventListener('click',reopenCurrent);
  document.addEventListener('paste',event=>{
    const prefix=pendingPastePrefix;pendingPastePrefix=null;
    if(prefix&&!isEditableTarget(event.target)){
      const data=event.clipboardData,text=data?.getData('text/plain')??'';
      let present=0;while(text.charCodeAt(present)===0xfeff)present++;
      const restored='\uFEFF'.repeat(Math.max(0,prefix.count-present))+text;
      const original=event;
      event={target:original.target,preventDefault:()=>original.preventDefault(),clipboardData:{files:data?.files,items:data?.items,getData:type=>type==='text/plain'?restored:data?.getData(type)??''}};
    }
    handleLogPaste(event,{openFile:()=>{inputError=FILE_PASTE_FALLBACK;errors();},openText:(text,size,lines)=>{void pastes.text({kind:'text',text,size,lines});},onError:message=>{inputError=message;errors();}});
  });
  // A paste that would replace the shown log, or one being opened, asks
  // first; with nothing open it opens at once (paste-confirm.js).
  const pasteDialog=mountPasteConfirm({document});
  const pastes=createPasteFlow({invoke:(command,args)=>native.core.invoke(command,args),dialog:pasteDialog,guard:()=>devices.guard(),
    replaces:()=>replacesLog(logs.getState(),pendingSourceId!==null),
    closing:()=>{const state=logs.getState();return describeClosing(work.datasetId===state.datasetId?work.reopenable:null);},
    openText:item=>{void openInput(item.text,'붙여넣은 로그 텍스트',item.size);},
    report:error=>{inputError=String(error).slice(0,500);errors();},isGone:()=>pageGone,inField:()=>isEditableTarget(document.activeElement)});
  window.addEventListener('pagehide',()=>pasteDialog.dismiss());
  void native.event.listen('native-text-prefix',async ({payload})=>{
    if(!Number.isSafeInteger(payload.count)||payload.count<1||payload.count>Math.floor(25*1024**2/3)+1)return;
    if(!pageGone&&!isEditableTarget(document.activeElement))pendingPastePrefix=payload;
    try{await native.core.invoke('finish_prefix_paste',{grant:payload.grant});}
    catch(error){if(pendingPastePrefix?.grant===payload.grant)pendingPastePrefix=null;if(!pageGone){inputError=String(error).slice(0,500);errors();}}
  });
  void native.event.listen('native-file-paste',({payload})=>pastes.file(payload));
  $('loadDemo').addEventListener('click',async()=>{if(await devices.guard()!=='cancel')void openInput(makeDemo(),'샘플 로그');});
  $('cancelInput').addEventListener('click',async()=>{
    const sequence=pickerSequence;
    try{
      await native.core.invoke('cancel_open',{datasetId:pendingSourceId});
      if(sequence===pickerSequence){pickerSequence++;pickerPending=false;}
    }catch(error){if(!pageGone){inputError=String(error).slice(0,500);errors();}}
    updateInputCancellation();
  });
  // File drops anywhere on the page must not navigate the browser away.
  for(const type of ['dragenter','dragover'])document.addEventListener(type,event=>{
    if(Array.from(event.dataTransfer?.types??[]).includes('Files')){event.preventDefault();event.dataTransfer.dropEffect='copy';}
  });
  document.addEventListener('drop',event=>{if(event.dataTransfer?.files?.length)event.preventDefault();});
  upperFingerprint=JSON.stringify(coreFilters({...readFilters(),query:'',caseSensitive:false,searchMode:'literal'}));appliedFilters=readFilters();
  ready=true;
  updateLevels();renderFilterSummary();bookmarks.render();renderLogs(logs.getState());renderSearch(search.getState());
  void native.event.listen('query-progress',({payload})=>{
    if(pageGone||disposed)return;
    if(payload.pane==='logs')logs.acceptProgress(payload.state);
    else if(payload.pane==='search')search.acceptProgress(payload.state);
  }).catch(()=>{}); // Polling also covers unavailable/missed startup events.
  // Native drag events only report entry and exit; the hint never reads files.
  void native.event.listen('drag-state',({payload})=>{ui.dropHint.hidden=pageGone||payload?.active!==true;}).catch(()=>{});
  // Files picked or dropped together are reordered natively just before they
  // open; the note is shown only if that log opens.
  let orderNote=null;
  void native.event.listen('files-ordered',({payload})=>{
    if(Number.isSafeInteger(payload?.datasetId)&&Number.isSafeInteger(payload.count)&&['time','name'].includes(payload.by))orderNote={id:payload.datasetId,message:`${formatNumber(payload.count)}개 파일을 ${payload.by==='time'?'시간순':'이름순'}으로 합쳤습니다.`};
  }).catch(()=>{});
  const sourceEvents=native.event.listen('source-changed',async ({payload})=>{
    if(pageGone)return;ui.dropHint.hidden=true;
    if(payload.status==='error'){inputError=payload.error;errors();return;}
    const id=payload.datasetId;
    if(!Number.isSafeInteger(id)||id<=0)return;
    if(payload.status==='opened'){
      if(id<=lastSourceId)return;
      // The replaced file's work is saved, and its notice dropped, before the view switches.
      saver.flush();saver.stop();if(toastTimer!==null)window.clearTimeout(toastTimer);toastTimer=null;ui.toast.hidden=true;
      lastSourceId=id;pendingSourceId=id;savedInput??=captureInput();inputError='';
      if(orderNote?.id===id)notify(orderNote.message);
      const sequence=++sourceEventSequence;updateInputCancellation();
      // Native code attaches the new source's saved work to the event.
      const info=payload.work&&typeof payload.work==='object'?workInfo(payload.work):await loadWork(id);
      if(sequence!==sourceEventSequence||pageGone)return;
      const reload=reloadRequest&&info.reloadOf===reloadRequest.from?reloadRequest.snapshot:null;if(reload)reloadRequest=null;
      setWork(id,info);
      await reconnect(reload??savedSnapshot(info),()=>sequence===sourceEventSequence&&!pageGone);errors();
      if(sequence===sourceEventSequence&&!pageGone&&!disposed&&info.reopenable)saver.track(id,info.state);
    }else if(id===pendingSourceId&&payload.status==='validated'){
      pendingSourceId=null;savedInput=null;updateInputCancellation();renderReopen();errors();
    }else if(id===pendingSourceId&&payload.status==='rollback'){
      const snapshot=savedInput;pendingSourceId=null;savedInput=null;
      // Of several files, native code names the one that failed.
      inputError=typeof payload.file==='string'&&payload.file?describeFileError(payload.error,payload.file):payload.error??'';
      const sequence=++sourceEventSequence;updateInputCancellation();saver.stop();
      await reconnect(snapshot,()=>sequence===sourceEventSequence&&!pageGone);errors();
      // The restored dataset is the one native code kept for rollback; a
      // capture shown again keeps its file.
      const restored=logs.getState().datasetId;devices.keep(restored);
      if(sequence!==sourceEventSequence||pageGone||!Number.isSafeInteger(restored)||restored<=0)return;
      const info=await loadWork(restored);
      if(sequence!==sourceEventSequence||pageGone||disposed)return;
      setWork(restored,info);if(info.reopenable)saver.track(restored,info.state);
    }
  });
  // A startup argument or file error can precede the listener; ask once it listens.
  void Promise.resolve(sourceEvents).then(()=>native.core.invoke('take_startup_error')).then(error=>{
    if(typeof error==='string'&&error&&!pageGone){inputError=error;errors();}
  }).catch(()=>{});
  void logs.start();void search.start();
  return search;
}
