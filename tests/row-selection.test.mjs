import test from 'node:test';
import assert from 'node:assert/strict';
import {selectOne, selectAll, extendTo, toggleRow, selectedRanges, selectedCount, inRanges, selectedPositions} from '../dist/row-selection.js';
import {rowGestures} from '../dist/row-gestures.js';

const ranges=(s,count=100)=>selectedRanges(s,count);

test('one row, a Shift span from the anchor, and nothing before any selection',()=>{
  assert.deepEqual(ranges(null),[]);assert.equal(selectedCount(null,10),0);
  const one=selectOne(4);
  assert.deepEqual(ranges(one),[[4,4]]);assert.equal(selectedCount(one,100),1);
  const down=extendTo(one,9,100);
  assert.deepEqual(ranges(down),[[4,9]]);assert.equal(down.anchor,4);assert.equal(down.focus,9);
  // Shift again replaces the span from the same anchor, in either direction.
  assert.deepEqual(ranges(extendTo(down,1,100)),[[1,4]]);
  assert.deepEqual(ranges(extendTo(null,3,100)),[[3,3]],'no selection: Shift selects the row');
  assert.deepEqual(ranges(selectOne(4),0),[],'an empty list selects nothing');
});

test('⌘/Ctrl toggles rows into a discontiguous selection kept in list order',()=>{
  let s=extendTo(selectOne(2),4,100);
  s=toggleRow(s,8,100);
  assert.deepEqual(ranges(s),[[2,4],[8,8]]);assert.equal(s.focus,8,'an added row becomes the focus');
  s=toggleRow(s,6,100);assert.deepEqual(ranges(s),[[2,4],[6,6],[8,8]]);
  s=toggleRow(s,5,100);assert.deepEqual(ranges(s),[[2,6],[8,8]],'touching ranges join');
  s=toggleRow(s,3,100);assert.deepEqual(ranges(s),[[2,2],[4,6],[8,8]]);assert.equal(s.focus,5,'removing another row keeps the focus');
  assert.equal(selectedCount(s,100),5);
  assert.deepEqual([...selectedPositions(s,100)],[2,4,5,6,8]);
  for(const [index,want] of [[1,false],[2,true],[3,false],[6,true],[7,false],[8,true],[9,false]])assert.equal(inRanges(ranges(s),index),want,String(index));
  // ⌘/Ctrl+Shift adds the span from the anchor and keeps the other rows.
  const kept=extendTo(s,12,100,{keep:true});
  assert.deepEqual(ranges(kept),[[2,2],[4,12]]);assert.equal(kept.anchor,5);
  assert.deepEqual(ranges(extendTo(toggleRow(s,20,100),24,100,{keep:true})),[[2,2],[4,6],[8,8],[20,24]]);
  // Plain Shift replaces the whole selection with the anchor's span.
  assert.deepEqual(ranges(extendTo(toggleRow(s,20,100),24,100)),[[20,24]]);
});

test('removing the focus row hands the focus to the nearest selected row; the last row stays',()=>{
  let s=toggleRow(toggleRow(selectOne(2),10,100),5,100);
  assert.equal(s.focus,5);
  s=toggleRow(s,5,100);assert.deepEqual(ranges(s),[[2,2],[10,10]]);assert.equal(s.focus,2,'2 is nearer to 5 than 10');assert.equal(s.anchor,2);
  s=toggleRow(s,2,100);assert.deepEqual(ranges(s),[[10,10]]);assert.equal(s.focus,10);
  assert.equal(toggleRow(s,10,100),s,'the only selected row is not removed');
  // A tie goes to the earlier row; the removed row is never re-added by its span.
  const tie=toggleRow(extendTo(selectOne(3),7,100),5,100);
  assert.deepEqual(ranges(tie),[[3,4],[6,7]]);assert.equal(tie.focus,7,'the focus row stays selected');
  const middle=toggleRow(toggleRow(toggleRow(selectOne(3),5,100),7,100),5,100);
  assert.deepEqual(ranges(middle),[[3,3],[7,7]]);assert.equal(middle.focus,7);
  const gone=toggleRow(middle,7,100);assert.deepEqual(ranges(gone),[[3,3]]);assert.equal(gone.focus,3);
});

test('select all follows a growing list and is materialized by later gestures',()=>{
  const all=selectAll(3);
  assert.deepEqual(ranges(all,5),[[0,4]]);assert.equal(selectedCount(all,5),5);
  assert.equal(selectedCount(all,8),8,'rows found later are selected too');
  assert.equal(all.focus,3);
  const less=toggleRow(all,2,8);
  assert.deepEqual(ranges(less,12),[[0,1],[3,7]],'toggling keeps the rows there were');assert.equal(less.all,false);
  assert.deepEqual(ranges(extendTo(all,6,8,{keep:true}),12),[[0,7]]);
  assert.deepEqual(ranges(extendTo(all,6,8),12),[[3,6]]);
  assert.deepEqual(ranges(selectAll(0),0),[]);
});

test('ranges are clipped to the rows the list has',()=>{
  assert.deepEqual(ranges(extendTo(selectOne(2),50,100),10),[[2,9]]);
  assert.deepEqual(ranges({base:[[20,30]],anchor:1,focus:1,all:false},10),[[1,1]]);
  assert.deepEqual([...selectedPositions(extendTo(selectOne(7),3,100),100)],[3,4,5,6,7],'positions in list order whatever the drag direction');
});

// A list element and window that record listeners; frames run on demand.
function gestureRig({text=()=>false,press=()=>true,mac=false,box={top:100,bottom:380}}={}){
  const listeners=new Map(),windowListeners=new Map(),documentListeners=new Map(),timers=new Map(),calls=[];let id=0,captured=null,removed=0;
  const on=(map)=>(name,fn)=>{(map.get(name)??map.set(name,[]).get(name)).push(fn);};
  const off=(map)=>(name,fn)=>{map.set(name,(map.get(name)??[]).filter(f=>f!==fn));};
  const list={addEventListener:on(listeners),getBoundingClientRect:()=>({...box}),setPointerCapture(pointer){captured=pointer;},releasePointerCapture(pointer){if(captured===pointer)captured=null;}};
  const window={addEventListener:on(windowListeners),removeEventListener:off(windowListeners),setTimeout:(fn,ms)=>{timers.set(++id,{fn,ms});return id;},clearTimeout:key=>timers.delete(key),getSelection:()=>({rangeCount:1,isCollapsed:false,removeAllRanges(){removed++;}})};
  const document={addEventListener:on(documentListeners),removeEventListener:off(documentListeners)};
  const gestures=rowGestures({list,window,document,mac,indexAt:y=>Math.floor((y-100)/28)+10,scroll:delta=>calls.push(['scroll',delta]),isTextRow:text,press:(index,keys)=>{calls.push(['press',index,keys]);return press(index,keys);},grab:index=>{calls.push(['grab',index]);return true;},drag:index=>calls.push(['drag',index]),release:()=>calls.push(['release'])});
  const fire=(map,name,event={})=>{for(const fn of map.get(name)??[])fn(event);};
  const frames=()=>{const due=[...timers].filter(([,t])=>t.ms===16);for(const [key,t] of due){timers.delete(key);t.fn();}return due.length;};
  const down=(event={})=>{let prevented=false;fire(listeners,'pointerdown',{pointerId:1,pointerType:'mouse',...event});const taken=gestures.down({button:0,detail:1,clientY:110,...event,preventDefault(){prevented=true;}},'index' in event?event.index:10);return {taken,prevented};};
  return {gestures,calls,down,frames,timers,move:event=>fire(windowListeners,'pointermove',{pointerId:1,buttons:1,...event}),up:event=>fire(windowListeners,'pointerup',{type:'pointerup',pointerId:1,...event}),windowListeners,captured:()=>captured,removed:()=>removed,fireList:(name,event)=>fire(listeners,name,event),fireDocument:(name,event)=>fire(documentListeners,name,event),documentListeners};
}

test('a press selects at once and a drag moves the focus once a frame, capturing the pointer',()=>{
  const r=gestureRig();
  const {taken,prevented}=r.down({clientY:110});
  assert.equal(taken,true);assert.equal(prevented,true,'no browser text selection');assert.equal(r.removed(),1,'an old text selection is dropped');
  assert.deepEqual(r.calls,[['press',10,{shift:false,toggle:false}]]);
  assert.equal(r.captured(),null,'a press alone does not capture, so its click reaches the row');
  r.move({clientY:112});assert.equal(r.captured(),null,'nor does a hand holding still');
  r.move({clientY:140});assert.equal(r.captured(),1,'a drag captures the pointer');
  r.move({clientY:170});r.move({clientY:200});
  assert.equal(r.frames(),1,'moves within a frame are applied once');
  assert.deepEqual(r.calls.slice(1),[['drag',13]]);
  r.move({clientY:205});r.frames();assert.equal(r.calls.length,2,'the same row is not applied again');
  r.up({clientY:230});
  assert.deepEqual(r.calls.slice(2),[['drag',14],['release']],'the release applies its own position');
  assert.equal(r.captured(),null);assert.equal(r.gestures.active(),false);
  assert.equal(r.gestures.click(),true,'the click after the press is claimed');assert.equal(r.gestures.click(),false,'only once');
  r.move({clientY:300});r.frames();assert.equal(r.calls.length,4,'moves after the release do nothing');
  const click=gestureRig();click.down({clientY:110});click.up({clientY:400});
  assert.deepEqual(click.calls.map(([kind])=>kind),['press','release'],'a release without a move never drags');
});

test('past the list edge the list keeps scrolling while the pointer stays there',()=>{
  const r=gestureRig();r.down({clientY:110});
  r.move({clientY:500});
  for(let i=0;i<3;i++)r.frames();
  const scrolls=r.calls.filter(([kind])=>kind==='scroll');
  assert.equal(scrolls.length,3,'one step a frame without further moves');
  assert.ok(scrolls.every(([,delta])=>delta>0&&delta<=16*28),JSON.stringify(scrolls));
  assert.deepEqual(r.calls.find(([kind])=>kind==='drag'),['drag',19],'the focus follows the row at the edge');
  r.move({clientY:20});r.frames();
  assert.ok(r.calls.at(-2)[0]==='scroll'&&r.calls.at(-2)[1]<0,'above the list it scrolls up');assert.deepEqual(r.calls.at(-1),['drag',10]);
  r.move({clientY:200});const before=r.calls.length;r.frames();r.frames();
  assert.equal(r.calls.slice(before).filter(([kind])=>kind==='scroll').length,0,'inside the list scrolling stops');
  r.up({clientY:200});assert.equal([...r.timers.values()].filter(t=>t.ms===16).length,0,'no frame left after the release');
});

test('the selected row keeps its text; other buttons, keys and multi-clicks are not taken',()=>{
  const text=gestureRig({text:index=>index===10});
  let result=text.down({clientY:110});
  assert.deepEqual(result,{taken:false,prevented:false},'the one selected row is left to the browser');assert.deepEqual(text.calls,[]);
  result=text.down({clientY:110,shiftKey:true});assert.equal(result.taken,true,'Shift on it still extends');
  const other=gestureRig();
  assert.equal(other.down({button:2}).taken,false,'secondary button');
  assert.equal(other.down({detail:2}).taken,false,'the second press of a double-click');
  assert.equal(other.down({index:null}).taken,false);
  assert.deepEqual(other.calls,[]);
  const mac=gestureRig({mac:true});
  assert.equal(mac.down({ctrlKey:true}).taken,false,'macOS Ctrl+click is the context menu');
  mac.down({metaKey:true});assert.deepEqual(mac.calls.at(-1),['press',10,{shift:false,toggle:true}]);
  const pc=gestureRig();
  pc.down({ctrlKey:true,shiftKey:true});assert.deepEqual(pc.calls.at(-1),['press',10,{shift:true,toggle:true}]);
  pc.down({metaKey:true});assert.deepEqual(pc.calls.at(-1),['press',10,{shift:false,toggle:false}],'⌘ is not the toggle key off macOS');
});

test('a drag leaving the selected row drops its text and selects rows from that row',()=>{
  const r=gestureRig({text:index=>index===10});
  assert.deepEqual(r.down({clientY:110}),{taken:false,prevented:false},'the press is still left to the browser');
  r.move({clientY:125});r.frames();
  assert.deepEqual(r.calls,[],'inside the row the browser selects text');assert.equal(r.removed(),0);
  r.move({clientY:133});r.frames();
  assert.deepEqual(r.calls,[],'a few pixels into the next row is still the text');
  r.move({clientY:170});r.frames();
  assert.deepEqual(r.calls,[['grab',10],['drag',12]],'past the row the drag selects rows from it');
  assert.equal(r.removed(),1,'the text is dropped');
  r.fireDocument('selectionchange');assert.equal(r.removed(),2,'and stays dropped while the drag goes on');
  r.up({clientY:200});
  assert.deepEqual(r.calls.slice(2),[['drag',13],['release']]);
  assert.equal(r.gestures.click(),true,'the click after the drag is claimed');
  assert.equal(r.documentListeners.get('selectionchange').length,0,'the listener goes with the drag');
  const edge=gestureRig({text:index=>index===10});edge.down({clientY:105});edge.move({clientY:60});edge.frames();
  assert.deepEqual(edge.calls.slice(0,2),[['grab',10],['scroll',edge.calls[1]?.[1]]],'above the list it scrolls as any drag');assert.ok(edge.calls[1][1]<0);
  const stay=gestureRig({text:index=>index===10});stay.down({clientY:105});stay.move({clientY:125});stay.up({clientY:125});
  assert.deepEqual(stay.calls,[],'a text selection inside the row is no row gesture');
  assert.equal(stay.gestures.click(),false,'its click selects as usual');assert.equal(stay.gestures.active(),false);
  const quick=gestureRig({text:index=>index===10});
  assert.equal(quick.down({clientY:110,detail:2}).taken,false,'the second press of a click is the browser\'s');
  quick.move({clientY:170});quick.frames();
  assert.deepEqual(quick.calls,[['grab',10],['drag',12]],'but a drag from it out of the row selects rows');
  const word=gestureRig();word.down({clientY:110,detail:2});word.move({clientY:170});word.frames();
  assert.deepEqual(word.calls,[],'a multi-click on a row that is not the one selected row does nothing more');
    const touch=gestureRig({text:index=>index===10});touch.down({clientY:110,pointerType:'touch'});
  assert.equal(touch.gestures.active(),false,'a touch on the row is left alone');
});

test('a hand holding still is no drag, and a list shrinking under the press does not run away',()=>{
  const still=gestureRig();still.down({clientY:110});still.move({clientY:112});still.move({clientY:108});still.frames();still.up({clientY:112});
  assert.deepEqual(still.calls.map(([kind])=>kind),['press','release'],'a few pixels of jitter neither drag nor scroll');
  // The raw detail opens under a press near the list's bottom: the pointer
  // is now below the list without having moved.
  const box={top:100,bottom:380},r=gestureRig({box});r.down({clientY:370,index:19});box.bottom=340;
  r.move({clientY:371});r.frames();
  assert.deepEqual(r.calls.map(([kind])=>kind),['press'],'no scroll and no drag');
  r.move({clientY:362});r.frames();
  assert.equal(r.calls.some(([kind])=>kind==='scroll'),false,'dragged up, it never scrolls down');
  assert.deepEqual(r.calls.at(-1),['drag',18],'the focus is the last row in view');
  r.move({clientY:420});r.frames();
  assert.ok(r.calls.some(([kind,delta])=>kind==='scroll'&&delta>0),'dragged down past the edge it scrolls down');
});

test('a press that may not drag, a touch, a lost release and a new press end cleanly',()=>{
  const fixed=gestureRig({press:()=>false});fixed.down({clientY:110});fixed.move({clientY:300});fixed.frames();
  assert.deepEqual(fixed.calls.map(([kind])=>kind),['press'],'a toggled-off row does not drag');
  fixed.up({clientY:300});assert.deepEqual(fixed.calls.at(-1),['release']);
  const touch=gestureRig();touch.down({pointerType:'touch'});touch.move({clientY:300});touch.frames();
  assert.equal(touch.calls.some(([kind])=>kind==='drag'),false,'touch presses select without dragging');
  const lost=gestureRig();lost.down({clientY:110});lost.move({clientY:300,buttons:0});
  assert.equal(lost.gestures.active(),false,'a move without buttons ends a press released elsewhere');
  assert.equal(lost.calls.some(([kind])=>kind==='drag'),false,'without applying its position');
  const again=gestureRig();again.down({clientY:110});again.move({clientY:300});
  again.down({clientY:150,index:12});
  assert.deepEqual(again.calls.map(([kind,index])=>[kind,index]),[['press',10],['release',undefined],['press',12]],'a stale press ends without its pending move');
  assert.equal(again.frames(),0);
  for(const name of ['pointermove','pointerup'])assert.equal(again.windowListeners.get(name).length,1,'listeners do not pile up');
});
