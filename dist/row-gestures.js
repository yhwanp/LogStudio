// Mouse gestures shared by the logs, search-result and bookmark lists. A
// press selects at once: Shift extends from the anchor, ⌘ (macOS) or Ctrl
// (elsewhere) toggles one row; macOS Ctrl+click stays the context menu.
// Dragging moves the focus to the row under the pointer, at most once a
// frame, once the pointer has moved a few pixels from the press (a hand
// holding still is no drag); past the list's top or bottom edge the list
// scrolls on while the pointer stays there, in the direction it was dragged.
// So a list that shrinks under a held press (the raw detail opening) does
// not run away beneath a pointer that never moved that way.
// A plain press on the list's only selected row is left to the browser, so
// text can be selected inside that row, until the pointer is well into
// another row: the text is dropped and the drag selects rows from there. Any
// other press drops a text selection.
const EDGE=8,MIN_STEP=2,MAX_STEP=16*28,SLACK=8,DRAG_START=4;

// `list` is the scrolling element; `indexAt(clientY)` names the row at a
// height inside it; `scroll(px)` scrolls it. `press(index,{shift,toggle})`
// selects and says whether a drag may extend from there; `grab(index)` does
// the same for a drag that leaves the selected row it began in, which stays
// selected; `drag(index)` moves the focus; `release()` ends a claimed press.
// `hold()` keeps the list from moving itself (a followed log) for the length
// of a press, text or rows, and returns what lets it go once the click that
// ends the press has been handled. A tap holds nothing: its mousedown comes
// after its pointerup, the release that ends a press, so none would follow.
export function rowGestures({list,document,window,mac,indexAt,scroll,isTextRow,press,grab,drag,release=()=>{},hold=()=>()=>{}}){
  let pointer=null,gesture=null,frame=null,claimed=false;
  // The next frame, or a timer where frames are paused (a hidden window)
  // or stop arriving, so a held drag never stalls.
  function schedule(){
    if(frame)return;
    const run=()=>{unschedule();step();},paused=document?.hidden||!window.requestAnimationFrame;
    frame={raf:paused?null:window.requestAnimationFrame(run),timer:window.setTimeout(run,paused?16:50)};
  }
  function unschedule(){if(!frame)return;if(frame.raf!==null)window.cancelAnimationFrame?.(frame.raf);window.clearTimeout(frame.timer);frame=null;}
  const dropText=()=>{const text=window.getSelection?.();if(text?.rangeCount&&!text.isCollapsed)text.removeAllRanges();};
  function step(final=false){
    if(!gesture)return;
    const box=list.getBoundingClientRect?.();if(!box)return;
    const y=gesture.y,over=y<box.top+EDGE&&y<gesture.start?y-box.top-EDGE:y>box.bottom-EDGE&&y>gesture.start?y-box.bottom+EDGE:0;
    let index=indexAt(Math.max(box.top+1,Math.min(box.bottom-1,y)));
    if(gesture.text){
      // A hand selecting text may stray a little past the row's edge.
      const near=indexAt(Math.max(box.top+1,Math.min(box.bottom-1,y+(y>gesture.start?-SLACK:SLACK))));
      if(!over&&!(Number.isSafeInteger(near)&&near!==gesture.index))return;
      // The browser may still be extending its text selection; it is
      // dropped for the rest of the gesture.
      gesture.text=false;claimed=true;dropText();document?.addEventListener?.('selectionchange',dropText);
      gesture.draggable=grab?.(gesture.index)===true;capture();
    }
    if(!gesture.draggable)return;
    if(over&&final!==true){scroll(Math.sign(over)*Math.min(MAX_STEP,Math.max(MIN_STEP,Math.abs(over)/2)));schedule();index=indexAt(Math.max(box.top+1,Math.min(box.bottom-1,y)));}
    if(Number.isSafeInteger(index)&&index!==gesture.index){gesture.index=index;drag(index);}
  }
  function move(event){
    if(!gesture||(gesture.pointer!==null&&event.pointerId!==undefined&&event.pointerId!==gesture.pointer))return;
    // A release outside the window that never arrived.
    if(event.buttons===0)return end(event);
    if(!Number.isFinite(event.clientY))return;
    gesture.y=event.clientY;
    if(!gesture.moved&&Math.abs(event.clientY-gesture.start)<DRAG_START)return;
    gesture.moved=true;capture();schedule();
  }
  // A drag holds the pointer so it goes on past the window's edge. Only a
  // drag: a press that captured would send its click to the list instead of
  // the row, and the search results' double-click would lose its row.
  function capture(){
    if(!gesture?.draggable||gesture.captured||gesture.pointer===null)return;
    try{list.setPointerCapture?.(gesture.pointer);gesture.captured=true;}catch{}
  }
  // A release after a move applies its position; a click, cancel, blur or
  // new press does not.
  function end(event,apply=event?.type==='pointerup'){
    if(!gesture)return;
    unschedule();
    if(apply&&gesture.moved&&Number.isFinite(event.clientY)){gesture.y=event.clientY;step(true);}
    const done=gesture;gesture=null;
    for(const [name,fn] of listeners)window.removeEventListener(name,fn);
    document?.removeEventListener?.('selectionchange',dropText);
    try{if(done.captured)list.releasePointerCapture?.(done.pointer);}catch{}
    // The click that follows this release belongs to the press.
    window.setTimeout(()=>{if(!gesture)claimed=false;done.letGo();},0);
    if(!done.text)release();
  }
  const listeners=[['pointermove',move],['pointerup',end],['pointercancel',end],['blur',end]];
  list.addEventListener('pointerdown',event=>{pointer={id:event.pointerId??null,type:event.pointerType??'mouse'};});
  list.addEventListener('lostpointercapture',event=>{if(gesture?.captured)end(event);});
  // The row's mousedown: it carries the click count, so the later presses of
  // a double-click belong to the first. Returns whether the press was taken.
  function down(event,index){
    if(gesture)end(event,false);
    claimed=false;
    if((event.button??0)!==0||(mac&&event.ctrlKey)||!Number.isSafeInteger(index))return false;
    const toggle=!!(mac?event.metaKey:event.ctrlKey),shift=!!event.shiftKey,touch=pointer?.type==='touch';
    // The later presses of a multi-click stay the browser's (words, the
    // line), as does a press on the selected row; either becomes a row drag
    // once it leaves that row, so a press soon after a click still drags.
    const text=!toggle&&!shift&&isTextRow(index);
    if(event.detail>1||text){if(text&&!touch)start(event,index,{text:true});return false;}
    event.preventDefault?.();window.getSelection?.()?.removeAllRanges?.();claimed=true;
    start(event,index,{draggable:press(index,{shift,toggle})===true&&!touch,touch});
    return true;
  }
  function start(event,index,{draggable=false,text=false,touch=false}){
    gesture={index,y:event.clientY,start:event.clientY,pointer:pointer?.id??null,draggable,text,captured:false,moved:false,letGo:touch?()=>{}:hold()};
    for(const [name,fn] of listeners)window.addEventListener(name,fn);
  }
  // Whether a click was already handled by its press; it is claimed once.
  function click(){const was=claimed;claimed=false;return was;}
  return {down,click,active:()=>!!gesture};
}
