// Keep native requests, retained rows and DOM independently bounded. Logical
// positions stay exact while a capped native scrollbar provides global travel.
export const ROW_HEIGHT = 28;
export const SCROLL_HEIGHT_LIMIT = 8_000_000;
export const DOM_ROW_LIMIT = 128;
export const CACHE_ROW_LIMIT = 800;
export const CACHE_BYTE_LIMIT = 8 * 1024 * 1024;
const OVERSCAN = 16, PREFETCH = 64;
const clamp = (n, lo, hi) => Math.max(lo, Math.min(hi, n));

export class ScrollPosition {
  constructor() {this.count=0;this.height=0;this.position=0;}
  update(count,height) {
    this.count=Number.isSafeInteger(count)&&count>=0?count:0;
    this.height=Math.max(1,Number(height)||1);
    this.position=clamp(this.position,0,this.maximum);
  }
  // The end position is a whole row, so End never clips the first visible
  // row; up to one row of space may follow the last row instead.
  get maximum(){return Math.max(0,Math.ceil((this.count*ROW_HEIGHT-this.height)/ROW_HEIGHT)*ROW_HEIGHT);}
  get extent(){return Math.max(this.height,Math.min(SCROLL_HEIGHT_LIMIT,this.maximum+this.height));}
  get physicalMaximum(){return Math.max(0,this.extent-this.height);}
  get scaled(){return this.maximum>this.physicalMaximum;}
  get physical(){return this.maximum?this.position/this.maximum*this.physicalMaximum:0;}
  native(top){this.position=this.physicalMaximum?clamp(top,0,this.physicalMaximum)/this.physicalMaximum*this.maximum:0;}
  move(delta){this.position=clamp(this.position+delta,0,this.maximum);}
  jump(index){this.position=clamp(index*ROW_HEIGHT,0,this.maximum);}
  get first(){return Math.floor(this.position/ROW_HEIGHT);}
  get last(){return Math.min(this.count,Math.ceil((this.position+this.height)/ROW_HEIGHT));}
  get range(){const start=Math.max(0,this.first-OVERSCAN);return {start,end:Math.min(this.count,start+DOM_ROW_LIMIT,this.last+OVERSCAN)};}
}

const MOVE_HINT='. 스크롤하거나 Home/End/PageUp/PageDown으로 이동';
// `onUserMove` hears of the user scrolling the list (wheel, scrollbar or
// keys), as opposed to the list moving itself.
export function createContinuousViewport({viewport,rowsElement,document,window,fetchRows,paint,onVisible=()=>{},onError=()=>{},onSelect=()=>{},onUserMove=()=>{}}) {
  const model=new ScrollPosition(),cache=new Map();let bytes=0,epoch=0,key=null,state=null;
  let flight=null,blocked=null,viewVersion=0,scrollRevision=null,pendingAnchor=null,disposed=false;
  let lastPaint=null,lastProgrammatic=0;
  let selectionIndex=null,pendingSelection=null,pendingRepeat=false,pendingExtend=false,lastSelectedLine=null;
  const space=document.createElement('div');space.className='continuous-space';space.append(rowsElement);viewport.replaceChildren(space);
  // The page names each pane; only how to move through it is added here.
  const name=(viewport.getAttribute?.('aria-label')||'로그 목록').replace(MOVE_HINT,'');
  viewport.tabIndex=0;viewport.setAttribute('aria-label',name+MOVE_HINT);
  function clear(){cache.clear();bytes=0;lastPaint=null;blocked=null;}
  function remember(index,row) {
    if(index>=model.count)return;
    const old=cache.get(index);
    if(old?.row===row){old.epoch=epoch;return;}
    const size=JSON.stringify(row).length*2;
    if(size>CACHE_BYTE_LIMIT)return;
    if(old)bytes-=old.size;
    cache.set(index,{row,size,epoch});bytes+=size;
    while(cache.size>CACHE_ROW_LIMIT||bytes>CACHE_BYTE_LIMIT){
      let farthest=-1,distance=-1;
      for(const i of cache.keys()){const d=Math.abs(i-model.first);if(d>distance){farthest=i;distance=d;}}
      const removed=cache.get(farthest);cache.delete(farthest);bytes-=removed.size;
    }
  }
  // A list that shrinks (the raw detail opening under it) keeps its selected
  // row in view when that row was in view.
  function dimensions(){
    const before=model.height,shown=selectionIndex!==null&&selectionIndex*ROW_HEIGHT>=model.position&&(selectionIndex+1)*ROW_HEIGHT<=model.position+before;
    model.update(state?.closed||!state?.datasetId?0:state?.count??0,viewport.clientHeight||280);space.style.height=`${model.extent}px`;
    if(shown&&model.height<before&&(selectionIndex+1)*ROW_HEIGHT>model.position+model.height)model.position=Math.min(model.maximum,(selectionIndex+1)*ROW_HEIGHT-model.height);
  }
  function sync(){viewport.scrollTop=model.physical;lastProgrammatic=viewport.scrollTop;}
  function render(force=false) {
    const {start,end}=model.range,entries=[];
    for(let i=start;i<end;i++)entries.push({index:i,row:cache.get(i)?.row??null});
    if(pendingSelection===null&&state?.selectedLine){const selected=entries.find(entry=>entry.row?.lineNumber===state.selectedLine);if(selected)selectionIndex=selected.index;}
    // Using actual native scrollTop also avoids pixel quantization moving rows
    // when the logical range is much larger than the native scrollbar.
    rowsElement.style.transform=`translateY(${viewport.scrollTop+start*ROW_HEIGHT-model.position}px)`;
    const changed=force||!lastPaint||lastPaint.start!==start||lastPaint.selected!==state?.selectedLine||lastPaint.count!==model.count||lastPaint.complete!==state?.queryComplete||entries.length!==lastPaint.entries.length||entries.some((e,i)=>e.row!==lastPaint.entries[i].row);
    if(changed){paint(entries,state);lastPaint={start,entries,selected:state?.selectedLine,count:model.count,complete:state?.queryComplete};}
    // A pending restore/locate anchor has not moved the model yet; reporting
    // its provisional position would overwrite the owner's remembered target.
    if(pendingAnchor===null)onVisible(model.first,model.last,model.count);
    if(pendingSelection!==null&&cache.has(pendingSelection)){
      const index=pendingSelection;pendingSelection=null;onSelect(cache.get(index).row,index,{repeat:pendingRepeat,extend:pendingExtend});
    }
  }
  // A row kept across a refresh paints until its re-read arrives.
  const current=i=>cache.get(i)?.epoch===epoch;
  function missing(start,end){for(let i=start;i<end;i++)if(!current(i))return i;return null;}
  function pump() {
    if(disposed||flight||!state?.datasetId||state.closed||!state.queryId||!model.count)return;
    const signature=`${epoch}:${viewVersion}:${model.count}`;
    if(blocked===signature)return;
    const visible=model.range;
    const start=(pendingSelection!==null&&!current(pendingSelection)?pendingSelection:null)??missing(model.first,model.last)??missing(visible.start,visible.end)??missing(Math.max(0,model.first-PREFETCH),Math.min(model.count,model.last+PREFETCH));
    if(start===null)return;
    const job={epoch,start};flight=job;
    Promise.resolve().then(()=>fetchRows(start)).then(data=>{
      if(disposed||epoch!==job.epoch)return;
      if(!data||data.start!==start||!Array.isArray(data.rows)||data.rows.length>200)throw Error('Invalid bounded viewport page');
      if(!data.rows.length){blocked=signature;return;}
      for(let i=0;i<data.rows.length;i++)remember(start+i,data.rows[i]);
      if(!current(start))blocked=signature;
      render();
    }).catch(error=>{
      // An aborted read (query cancelled, seek, locate) failed nothing: leave the
      // range unblocked so the finally-pump re-reads it against the current state.
      if(!disposed&&epoch===job.epoch&&error?.name!=='AbortError'){blocked=signature;onError(error);}
    }).finally(()=>{if(flight===job)flight=null;if(!disposed)pump();});
  }
  function changed(){viewVersion++;blocked=null;render();pump();}
  function onScroll(){if(disposed)return;const moved=Math.abs(viewport.scrollTop-lastProgrammatic)>.01;if(moved)model.native(viewport.scrollTop);lastProgrammatic=viewport.scrollTop;changed();if(moved)onUserMove();}
  function onWheel(event){
    if(!model.scaled||event.ctrlKey||event.shiftKey||!event.deltaY)return;
    event.preventDefault();model.move(event.deltaY*(event.deltaMode===1?ROW_HEIGHT:event.deltaMode===2?model.height:1));sync();changed();onUserMove();
  }
  function onKey(event){
    if(event.defaultPrevented||event.altKey||event.ctrlKey||event.metaKey||event.target?.isContentEditable||event.target?.closest?.('input,textarea,select,button,[contenteditable]:not([contenteditable="false"])'))return;
    if((event.key==='ArrowDown'||event.key==='ArrowUp')&&selectionIndex!==null){event.preventDefault();selectIndex(selectionIndex+(event.key==='ArrowDown'?1:-1),{repeat:event.repeat===true,extend:event.shiftKey===true});return;}
    // Shift with a page or end key extends the selection that far; without
    // Shift these keys only scroll.
    const page=Math.max(1,Math.floor((model.height-ROW_HEIGHT)/ROW_HEIGHT)),reach=event.shiftKey&&selectionIndex!==null?{PageDown:selectionIndex+page,PageUp:selectionIndex-page,Home:0,End:model.count-1}[event.key]:undefined;
    if(reach!==undefined){event.preventDefault();selectIndex(reach,{repeat:event.repeat===true,extend:true});return;}
    const delta={ArrowDown:ROW_HEIGHT,ArrowUp:-ROW_HEIGHT,PageDown:Math.max(ROW_HEIGHT,model.height-ROW_HEIGHT),PageUp:-Math.max(ROW_HEIGHT,model.height-ROW_HEIGHT)}[event.key];
    if(delta!==undefined){event.preventDefault();model.move(delta);}
    else if(event.key==='Home'||event.key==='End'){event.preventDefault();model.jump(event.key==='Home'?0:Math.max(0,model.count-1));}
    else return;
    sync();changed();onUserMove();
  }
  // Selection travels in displayed ordinal order, including uncached rows. A
  // held key replaces the pending ordinal; page replies publish only the latest.
  // `repeat` tells the owner that more selections are likely to follow.
  function selectIndex(index,{force=false,repeat=false,extend=false}={}){
    if(disposed||!model.count||state?.closed||!Number.isSafeInteger(index))return;
    index=clamp(index,0,model.count-1);
    if(!force&&index===selectionIndex&&pendingSelection===null)return;
    selectionIndex=index;pendingSelection=index;pendingRepeat=repeat;pendingExtend=extend;
    if(index*ROW_HEIGHT<model.position)model.position=index*ROW_HEIGHT;
    else if((index+1)*ROW_HEIGHT>model.position+model.height)model.position=Math.min(model.maximum,(index+1)*ROW_HEIGHT-model.height);
    sync();changed();
  }
  function resize(){if(disposed)return;dimensions();sync();changed();}
  // Row annotations changed for the same query: replies in flight are
  // discarded, `strip` maps the kept rows at once (so nothing stale paints),
  // and with `reread` the rows are read again around the current position.
  function refresh(strip=row=>row,{reread=true}={}){
    if(disposed||!key)return;
    epoch++;
    for(const entry of cache.values()){
      const row=strip(entry.row);
      if(row!==entry.row){bytes-=entry.size;entry.row=row;entry.size=JSON.stringify(row).length*2;bytes+=entry.size;}
      entry.epoch=reread?-1:epoch;
    }
    changed();
  }
  viewport.addEventListener('scroll',onScroll);viewport.addEventListener('wheel',onWheel,{passive:false});viewport.addEventListener('keydown',onKey);
  const observer=window.ResizeObserver?new window.ResizeObserver(resize):null;observer?.observe(viewport);window.addEventListener('resize',resize);
  // `stick` keeps the end in view while rows arrive (a followed file), as
  // long as the end was in view; selection and focus stay where they are.
  function update(next,{force=false,stick=false}={}) {
    state=next;
    const identity=next.closed?null:`${next.datasetId}:${next.generation}:${next.queryId}`;
    const atEnd=stick&&identity===key&&pendingAnchor===null&&pendingSelection===null&&model.position>=model.maximum;
    if(identity!==key){key=identity;epoch++;clear();model.position=0;scrollRevision=null;pendingAnchor=null;selectionIndex=pendingSelection=lastSelectedLine=null;}
    dimensions();
    if(atEnd&&scrollRevision===next.scrollRevision)model.position=model.maximum;
    const anchorMoved=scrollRevision!==next.scrollRevision;
    if(anchorMoved||next.selectedLine!==lastSelectedLine){
      // An outside selection replaces a pending keyboard one. Use the row's
      // known ordinal; otherwise keep it unknown until render() sees the row,
      // so arrows never travel from a stale row or cancel a pending locate.
      lastSelectedLine=next.selectedLine;pendingSelection=null;selectionIndex=null;
      if(next.selectedLine){
        for(const [index,entry] of cache)if(entry.row.lineNumber===next.selectedLine){selectionIndex=index;break;}
        if(selectionIndex===null&&anchorMoved&&Number.isSafeInteger(next.selectedIndex))selectionIndex=next.selectedIndex;
      }
    }
    if(anchorMoved){scrollRevision=next.scrollRevision;pendingAnchor=next.viewportStart??0;viewVersion++;blocked=null;}
    if(pendingAnchor!==null&&(model.count>pendingAnchor||next.queryComplete||next.queryStatus==='cancelled')){
      // `offset` rows stay above the anchor, as many as fit above its row.
      const rows=Math.floor(model.height/ROW_HEIGHT),above=next.scrollAlign==='center'?Math.floor(rows/2):next.scrollAlign==='offset'?Math.min(next.scrollOffset|0,Math.max(0,rows-1)):0;
      model.jump(Math.max(0,pendingAnchor-above));pendingAnchor=null;
    }
    sync();render(force);pump();
  }
  function scrollToEnd(){if(disposed)return;model.position=model.maximum;sync();changed();}
  // A drag past the list's edge scrolls by logical pixels, as the wheel does.
  function scrollBy(delta){if(disposed||!model.count||!Number.isFinite(delta))return;model.move(delta);sync();changed();}
  // The row at a client y inside the viewport, kept to the rows in view.
  function indexAt(clientY){
    const box=viewport.getBoundingClientRect?.();
    if(disposed||!model.count||!box||!Number.isFinite(clientY))return null;
    return clamp(Math.floor((clientY-box.top-(viewport.clientTop||0)+model.position)/ROW_HEIGHT),model.first,Math.max(model.first,model.last-1));
  }
  function destroy(){disposed=true;epoch++;clear();observer?.disconnect();viewport.removeEventListener?.('scroll',onScroll);viewport.removeEventListener?.('wheel',onWheel);viewport.removeEventListener?.('keydown',onKey);window.removeEventListener?.('resize',resize);}
  // Rows for a copy are taken from the cache only when every one is present.
  function cachedRows(start,end){const rows=[];for(let i=start;i<=end;i++){const row=cache.get(i)?.row;if(!row)return null;rows.push(row);}return rows;}
  return {update,destroy,selectIndex,cachedRows,refresh,scrollToEnd,scrollBy,indexAt,metrics:()=>({selectionIndex,pendingSelection,logicalPosition:model.position,first:model.first,last:model.last,count:model.count,scrollHeight:model.extent,cachedRows:cache.size,cachedBytes:bytes,renderedRows:lastPaint?.entries.length??0,pending:!!flight})};
}
