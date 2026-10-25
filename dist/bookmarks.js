// Bookmarks follow physical rows, so they survive filter and search changes.
// A reload and saved work keep their line numbers only; the rows are read
// from the file again. Snapshots keep the list readable without reading rows
// that the current filter hides.
import {LEVEL_NAMES, formatNumber} from './describe.js';
import {textSpan, writeClipboard} from './dom.js';
import {continuationOwner, continuationClass, joinsNext, ownerCells, ownerSnapshot} from './entries.js';
import {dltCells, dltLevelTitle, levelNames} from './columns.js';
import {selectOne, selectAll, extendTo, toggleRow, selectedRanges, selectedCount, inRanges, selectedPositions} from './row-selection.js';
import {rowGestures} from './row-gestures.js';

export const MAX_BOOKMARKS=1000;
// The fields a bookmark keeps of its page row.
export const markOf=row=>({lineNumber:row.lineNumber,file:row.file,fileLine:row.fileLine,raw:row.raw??'',truncated:!!row.truncated,kind:row.kind,date:row.date,time:row.time,process:row.process,pid:row.pid,tid:row.tid,level:row.level,tag:row.tag,message:row.message,owner:ownerSnapshot(row),...(row.dlt?{dlt:{...row.dlt}}:{})});

// `logs` is the upper pane controller; `canNavigate` says whether its current
// result may be navigated; `jump(line)` moves focus to a row there;
// `onChange` repaints rows after a mark changes;
// `openMenu(mark,size,x,y)` opens a row's menu for a selection of `size`.
// The list is a lower-region tab: `onCount` lets the view show or hide that
// tab before the list renders, and the list renders only while it is shown.
export function createBookmarks({ui, document, window, notify, mod, logs, lineCell, jump=()=>{}, openMenu=()=>{}, isRawMode, canNavigate, onChange, onCount}) {
  const bookmarks=new Map();
  // The list selects rows as the other lists do (row-selection.js), by
  // position in line order. `cursor` is the line of its focus row, the row
  // revealed in the upper logs; a changed set of marks keeps only the cursor.
  let cursor=null,selection=null,version=0,order=null,dragFrom=null;
  const sorted=()=>order??=[...bookmarks.keys()].sort((a,b)=>a-b);
  const changed=()=>{order=null;version++;};
  const rowNodes=()=>[...(ui.bookmarkList.children??[])];
  const info=document.getElementById?.('bookmarkSelection'),mac=mod==='⌘',toggleKey=event=>!!(mac?event.metaKey:event.ctrlKey);
  function current(){
    const lines=sorted();
    if(selection?.version===version&&lines[selection.focus]===cursor)return selection;
    const at=cursor===null?-1:lines.indexOf(cursor);
    return at<0?null:selectOne(at);
  }
  // What a key in the list acts on: the selection, or else the focused row.
  function chosen(){
    const at=rowNodes().findIndex(row=>row.contains(document.activeElement));
    return current()??(at<0?null:selectOne(at));
  }
  // Adds the rows, or removes them when every row is already marked.
  function toggle(rows){
    const remove=rows.every(row=>bookmarks.has(row.lineNumber));
    if(!remove&&bookmarks.size+rows.filter(row=>!bookmarks.has(row.lineNumber)).length>MAX_BOOKMARKS)return notify(`북마크는 최대 ${formatNumber(MAX_BOOKMARKS)}개까지입니다.`);
    for(const row of rows){
      if(remove)bookmarks.delete(row.lineNumber);
      else bookmarks.set(row.lineNumber,markOf(row));
    }
    changed();
    notify(remove?`북마크 ${formatNumber(rows.length)}개를 해제했습니다.`:`북마크 ${formatNumber(rows.length)}개를 추가했습니다.`);
    render();onChange();
  }
  // A marked continuation row reads like a search hit: its entry's fields
  // dimmed. Marked rows of one entry next to each other share no separator.
  const entryOwner=mark=>isRawMode()?null:continuationOwner(mark);
  const rowClass=(mark,line,chosen)=>{
    const owner=entryOwner(mark),kind=owner?continuationClass(owner):mark.kind!=='parsed'?'raw-row':`level-row-${mark.level}`;
    return `log-row bookmark-row ${kind}${!isRawMode()&&joinsNext(mark,bookmarks.get(line+1))?' record-continued':''}${line===cursor?' selected-log-row':''}${chosen?' in-selection':''}`;
  };
  function rowNode(line,focusable,chosen){
    const mark=bookmarks.get(line),row=document.createElement('div'),remove=document.createElement('button');
    row.className=rowClass(mark,line,chosen);row.dataset.lineNumber=String(line);row.setAttribute('role','listitem');row.setAttribute('aria-describedby','bookmarkRowHint');row.tabIndex=focusable?0:-1;
    if(line===cursor)row.setAttribute('aria-current','true');
    row.append(Number.isSafeInteger(mark.fileLine)?lineCell(mark):textSpan(document,formatNumber(line),'line-number'));
    const owner=entryOwner(mark);
    if(owner)row.append(...ownerCells(document,mark,owner,logs.getState().files,true),textSpan(document,mark.raw,'message continuation-text'));
    else if(isRawMode()||mark.kind!=='parsed')row.append(textSpan(document,mark.raw||'(빈 줄)','raw-content'));
    else if(mark.dlt){const level=textSpan(document,levelNames('dlt')[mark.level]??mark.level,`level level-${mark.level}`),title=dltLevelTitle(mark);if(title)level.title=title;row.append(...dltCells(mark,(text,className)=>textSpan(document,text,className),level));}
    else row.append(textSpan(document,`${mark.date??''} ${mark.time??''}${mark.process?` · ${mark.process}`:''}`,'timestamp'),textSpan(document,mark.pid??'','pid'),textSpan(document,mark.tid??'','tid'),textSpan(document,LEVEL_NAMES[mark.level]??mark.level,`level level-${mark.level}`),textSpan(document,mark.tag??'','tag'),textSpan(document,mark.message??'','message'));
    row.title=mark.raw.slice(0,1000);
    remove.type='button';remove.className='bookmark-remove';remove.textContent='×';remove.title='북마크 해제';remove.tabIndex=-1;remove.setAttribute('aria-label',`${line}행 북마크 해제`);
    remove.addEventListener('click',event=>{event.stopPropagation?.();unmark(line);});
    row.append(remove);
    // A press selects as in the other lists; a click no press took selects
    // too, and both reveal the focus row once the button is released.
    row.addEventListener('mousedown',event=>{if(!event.target?.closest?.('button'))gestures.down(event,rowNodes().indexOf(row));});
    row.addEventListener('click',event=>{
      if(event.detail>1||(mac&&event.ctrlKey)||gestures.click())return;
      press(rowNodes().indexOf(row),{shift:!!event.shiftKey,toggle:toggleKey(event)});if(cursor!==null)reveal(cursor);
    });
    // Right-click keeps a selection that contains the row, or selects the row
    // alone; the menu acts on that selection. As in the search results, the
    // upper logs stay where they are, which also keeps the menu open.
    const menuAt=(x,y)=>{
      const at=rowNodes().indexOf(row);
      if(!inRanges(selectedRanges(current(),sorted().length),at))press(at);else row.focus?.({preventScroll:true});
      openMenu(mark,selectedCount(current(),sorted().length)||1,x,y);
    };
    row.addEventListener('contextmenu',event=>{event.preventDefault();menuAt(event.clientX,event.clientY);});
    // Keys act on the focused row, or on the selection it belongs to.
    row.addEventListener('keydown',event=>{
      if(event.target!==row)return;
      if(event.key==='ContextMenu'||(event.key==='F10'&&event.shiftKey)){event.preventDefault();const box=row.getBoundingClientRect?.();menuAt(box?box.left+48:0,box?box.bottom:0);return;}
      const rows=rowNodes(),at=rows.indexOf(row),last=rows.length-1;
      // Shift with a page or end key extends the selection that far; without
      // Shift the browser scrolls the list, as in the other lists.
      const page=Math.max(1,Math.floor((ui.bookmarkList.clientHeight||0)/(row.getBoundingClientRect?.().height||28))-1);
      const target={ArrowDown:at+1,ArrowUp:at-1,...(event.shiftKey?{PageDown:at+page,PageUp:at-page,Home:0,End:last}:{})}[event.key],remove=event.key==='Delete'||event.key==='Backspace';
      if(target===undefined&&!remove&&event.key!=='Enter'&&event.key!==' ')return;
      event.preventDefault();
      if(!inRanges(selectedRanges(current(),rows.length),at))select(selectOne(at));
      if(target!==undefined){const index=Math.max(0,Math.min(last,target));select(event.shiftKey?extendTo(current(),index,rows.length):selectOne(index));reveal(cursor);}
      else if(remove)removeSelected();
      else moveTo(line);
    });
    return row;
  }
  function render(){
    onCount(bookmarks.size);
    ui.bookmarkTab.textContent=`북마크 ${formatNumber(bookmarks.size)}`;
    ui.bookmarkCount.textContent=`북마크 · ${formatNumber(bookmarks.size)}개`;
    if(ui.bookmarkPane.hidden){ui.bookmarkList.replaceChildren();showCount(0);return;}
    // A rebuilt list keeps focus on the same row, or the row taking its place.
    const before=rowNodes(),at=before.findIndex(row=>row.contains(document.activeElement)),focused=at<0?null:Number(before[at].dataset.lineNumber);
    const lines=sorted(),focusLine=bookmarks.has(cursor)?cursor:lines[0],s=current(),chosen=selectedRanges(s,lines.length),size=selectedCount(s,lines.length);
    ui.bookmarkList.replaceChildren(...lines.map((line,i)=>rowNode(line,line===focusLine,size>1&&inRanges(chosen,i))));
    showCount(size);
    if(at<0)return;
    const rows=rowNodes();(rows.find(row=>Number(row.dataset.lineNumber)===focused)??rows[Math.min(at,rows.length-1)])?.focus?.();
  }
  const showCount=size=>{const text=size>1?`${formatNumber(size)}행 선택 · ${mod}C 복사`:'';if(info&&info.textContent!==text)info.textContent=text;};
  // Marks the cursor and the selection in place so a focused row keeps its
  // focus; a drag repaints only rows whose marks change.
  function paint(){
    const rows=rowNodes(),s=current(),chosen=selectedRanges(s,rows.length),size=selectedCount(s,rows.length);
    rows.forEach((row,i)=>{
      const line=Number(row.dataset.lineNumber),isCursor=line===cursor,className=rowClass(bookmarks.get(line),line,size>1&&inRanges(chosen,i));
      if(row.className!==className)row.className=className;row.tabIndex=isCursor?0:-1;
      if(isCursor){row.setAttribute('aria-current','true');row.scrollIntoView?.({block:'nearest'});}else row.removeAttribute('aria-current');
    });
    showCount(size);
  }
  // Shows a new selection; its focus row becomes the cursor and takes focus.
  function select(next){
    selection={...next,version};cursor=sorted()[next.focus]??null;paint();
    rowNodes().find(row=>Number(row.dataset.lineNumber)===cursor)?.focus?.({preventScroll:true});
  }
  // Shift extends from the anchor, ⌘/Ctrl toggles one row, a plain press
  // selects the row alone; a drag extends from a row the press selected.
  function press(index,{shift=false,toggle=false}={}){
    const count=sorted().length,prior=current();
    if(!(index>=0&&index<count))return false;
    const next=toggle&&!shift?toggleRow(prior,index,count):shift?extendTo(prior,index,count,{keep:toggle}):selectOne(index);
    if(next!==prior)select(next);
    dragFrom=next.focus===index?next:null;
    return !!dragFrom;
  }
  const gestures=rowGestures({list:ui.bookmarkList,document,window,mac,press,
    indexAt:y=>{
      const list=ui.bookmarkList,rows=rowNodes(),box=list.getBoundingClientRect?.(),height=rows[0]?.getBoundingClientRect?.().height||28;
      return rows.length&&box?Math.max(0,Math.min(rows.length-1,Math.floor((y-box.top-(list.clientTop||0)+list.scrollTop)/height))):null;
    },
    scroll:delta=>{ui.bookmarkList.scrollTop+=delta;},
    isTextRow:index=>{const s=current();return s?.focus===index&&selectedCount(s,sorted().length)===1;},
    grab:index=>{dragFrom=selectOne(index);return true;},
    drag:index=>{if(dragFrom)select(extendTo(dragFrom,index,sorted().length,{keep:true}));},
    release:()=>{if(cursor!==null)reveal(cursor);}});
  function unmark(line){bookmarks.delete(line);changed();if(cursor===line)cursor=null;render();onChange();}
  // Delete, Backspace and ⌘/Ctrl+B in the list unmark the selected rows; the
  // row now in the place of the first of them takes the cursor.
  function removeSelected(){
    const lines=sorted(),s=chosen();
    if(!s)return;
    const gone=[...selectedPositions(s,lines.length)].map(i=>lines[i]),first=selectedRanges(s,lines.length)[0][0];
    for(const line of gone)bookmarks.delete(line);
    changed();const rest=sorted();cursor=rest[Math.min(first,rest.length-1)]??null;selection=null;
    notify(`북마크 ${formatNumber(gone.length)}개를 해제했습니다.`);
    render();if(rest.length)focusList();onChange();
  }
  function selectEvery(){const count=sorted().length;if(count)select(selectAll(chosen()?.focus??0));}
  // A selection of several bookmarks becomes its focus row alone.
  function collapse(){const s=current();if(selectedCount(s,sorted().length)<2)return false;select(selectOne(s.focus));return true;}
  // Shows the list; focus goes to the cursor row, or the first one.
  function focusList(){
    const rows=rowNodes();(rows.find(row=>Number(row.dataset.lineNumber)===cursor)??rows[0])?.focus?.();
  }
  // Reveals a bookmark in the upper logs; a row hidden by the filter is
  // reported by the locate error, and the cursor still advances past it.
  function reveal(line){
    cursor=line;paint();
    if(canNavigate())void logs.locateLine(line);
  }
  // Enter moves focus to a bookmark's row above, as on a search result.
  function moveTo(line){
    cursor=line;paint();
    if(canNavigate())jump(line);
  }
  function step(direction){
    const lines=sorted();
    if(!lines.length)return notify(`북마크가 없습니다. 행을 선택하고 ${mod}B로 추가하세요.`);
    const current=cursor??logs.getState().selectedLine??(direction>0?0:Infinity);
    const next=direction>0?lines.find(line=>line>current)??lines[0]:[...lines].reverse().find(line=>line<current)??lines.at(-1);
    selection=null;reveal(next);
  }
  // The marks' raw rows in line order; a row too long for its mark is read again.
  async function text(marks=sorted().map(line=>bookmarks.get(line))){
    const lines=[];
    for(const mark of marks)lines.push(mark.truncated?await logs.readLineText(mark.lineNumber):mark.raw);
    return lines.join('\n');
  }
  // ⌘/Ctrl+C in the list copies the selected rows.
  function copyPlan(){
    const lines=sorted(),s=chosen();if(!s)return null;
    const marks=[...selectedPositions(s,lines.length)].map(i=>bookmarks.get(lines[i]));
    return {count:marks.length,text:marks.some(mark=>mark.truncated)?null:marks.map(mark=>mark.raw).join('\n'),load:()=>text(marks)};
  }
  // Replaces every mark without repainting rows; the caller repaints.
  function restore(entries=[]){bookmarks.clear();cursor=null;selection=null;for(const [line,mark] of entries)bookmarks.set(line,mark);changed();render();}
  // A list that shrinks (the raw detail opening under it) keeps its cursor
  // row in view when that row was in view.
  let listHeight=0;
  function keepCursor(){
    const list=ui.bookmarkList,box=list.getBoundingClientRect?.();if(!box)return;
    const shrunk=listHeight-box.height;listHeight=box.height;
    const row=cursor===null?null:rowNodes().find(node=>Number(node.dataset.lineNumber)===cursor),at=row?.getBoundingClientRect?.();
    if(at&&shrunk>0&&at.top>=box.top&&at.bottom>box.bottom&&at.bottom-box.bottom<=shrunk+1)list.scrollTop+=at.bottom-box.bottom;
  }
  if(window.ResizeObserver)new window.ResizeObserver(keepCursor).observe(ui.bookmarkList);
  ui.copyBookmarks.addEventListener('click',()=>{
    const count=bookmarks.size;
    writeClipboard(window,text()).then(()=>notify(`북마크 ${formatNumber(count)}행을 복사했습니다.`),()=>notify('복사하지 못했습니다. 다시 시도해 주세요.'));
  });
  ui.clearBookmarks.addEventListener('click',()=>{restore();onChange();notify('북마크를 모두 해제했습니다.');});
  return {has:line=>bookmarks.has(line), size:()=>bookmarks.size, toggle, step, restore, render, focusList, copyPlan, removeSelected, selectAll:selectEvery, collapse, snapshot:()=>new Map(bookmarks)};
}
