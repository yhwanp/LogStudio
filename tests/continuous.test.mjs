import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as turn} from 'node:timers/promises';
import {ScrollPosition,createContinuousViewport,ROW_HEIGHT,SCROLL_HEIGHT_LIMIT,DOM_ROW_LIMIT,CACHE_ROW_LIMIT,CACHE_BYTE_LIMIT} from '../dist/continuous-view.js';
import {createLargeLogController} from '../dist/controller.js';
const deferred=()=>{let resolve;const promise=new Promise(r=>resolve=r);return {promise,resolve};};
async function settle(){for(let i=0;i<20;i++)await turn();}
class Element {
  constructor(){this.style={};this.scrollTop=0;this.clientHeight=280;this.listeners=new Map();}
  append(){} replaceChildren(){} setAttribute(){}
  addEventListener(n,f){this.listeners.set(n,f);} removeEventListener(n){this.listeners.delete(n);}
  scroll(index){this.scrollTop=index*ROW_HEIGHT;this.listeners.get('scroll')?.();}
}
const row=i=>({lineNumber:i+1,raw:`physical ${i+1}`,kind:'raw'});
function state(count=10000,extra={}){return {datasetId:1,queryId:1,count,viewportStart:0,scrollRevision:0,queryComplete:true,queryStatus:'complete',closed:false,...extra};}
function rig(fetchRows=async start=>({start,rows:Array.from({length:200},(_,n)=>row(start+n))}),onVisible=()=>{},onSelect=()=>{}){
  const viewport=new Element(),rowsElement=new Element(),painted=[];
  const document={createElement:()=>new Element()},window={listeners:new Map(),addEventListener(name,fn){this.listeners.set(name,fn);},removeEventListener(name){this.listeners.delete(name);}};
  const view=createContinuousViewport({viewport,rowsElement,document,window,fetchRows,paint:entries=>painted.push(entries),onVisible,onSelect,onError:e=>{throw e;}});
  return {view,viewport,window,painted,last:()=>painted.at(-1)};
}

test('repeated pane resizing retains its logical scroll anchor and selected cached row',async()=>{
  const r=rig();r.view.update(state(10000,{selectedLine:1901}));await settle();r.viewport.scroll(1900);await settle();
  const selected=r.last().find(entry=>entry.index===1900).row;
  for(const height of [220,340,260,280]){
    r.viewport.clientHeight=height;r.window.listeners.get('resize')();await settle();
    assert.equal(r.view.metrics().first,1900);assert.equal(r.last().find(entry=>entry.index===1900).row,selected);
    assert.ok(r.view.metrics().renderedRows<=DOM_ROW_LIMIT);
  }
  r.view.destroy();
});

test('huge scrollbar has bounded height and wheel retains one-row precision across count growth',()=>{
  const p=new ScrollPosition();p.update(9_000_000_000,280);p.jump(8_000_000_000);
  assert.equal(p.first,8_000_000_000);assert.equal(p.extent,SCROLL_HEIGHT_LIMIT);
  p.move(ROW_HEIGHT);assert.equal(p.first,8_000_000_001);
  const anchor=p.position;p.update(9_000_000_001,560);assert.equal(p.position,anchor);
  p.native(p.physicalMaximum);assert.equal(p.last,p.count);assert.ok(p.range.end-p.range.start<=DOM_ROW_LIMIT);
});

test('scroll crosses 200-row request boundaries in both directions without a page action',async()=>{
  const requests=[],r=rig(async start=>{requests.push(start);return {start,rows:Array.from({length:17},(_,n)=>row(start+n))};});
  r.view.update(state());await settle();
  for(const index of [190,201,399,420,199]){r.viewport.scroll(index);await settle();assert.equal(r.last().find(e=>e.index===index)?.row.lineNumber,index+1);}
  assert.ok(requests.includes(190));assert.ok(r.last().length<=DOM_ROW_LIMIT);
  assert.ok(r.view.metrics().cachedRows<=CACHE_ROW_LIMIT);assert.ok(r.view.metrics().cachedBytes<=CACHE_BYTE_LIMIT);r.view.destroy();
});

test('rapid scroll changes keep one flight and publish rows only at current ordinal positions',async()=>{
  const pending=[],r=rig(start=>{const d=deferred();pending.push({start,...d});return d.promise;});
  r.view.update(state());await settle();assert.equal(pending.length,1);
  r.viewport.scroll(1500);r.viewport.scroll(8500);await settle();assert.equal(pending.length,1);
  pending[0].resolve({start:pending[0].start,rows:Array.from({length:200},(_,i)=>row(i))});await settle();
  assert.equal(pending.length,2);assert.equal(pending[1].start,8500);assert.ok(r.last().every(e=>!e.row));
  const second=pending[1];second.resolve({start:second.start,rows:Array.from({length:200},(_,i)=>row(second.start+i))});await settle();
  assert.equal(r.last().find(e=>e.index===8500).row.lineNumber,8501);r.view.destroy();
});

test('source/query replacement rejects stale rows, Close clears caches and reopen starts clean',async()=>{
  const old=deferred();let calls=0;
  const r=rig(start=>++calls===1?old.promise:Promise.resolve({start,rows:Array.from({length:200},(_,i)=>({...row(start+i),raw:'new query'}))}));
  r.view.update(state());await settle();r.view.update(state(500,{datasetId:2,queryId:2}));
  old.resolve({start:0,rows:[{...row(0),raw:'stale old query'}]});await settle();assert.ok(r.last().every(e=>!e.row||e.row.raw==='new query'));
  r.view.update(state(500,{closed:true}));assert.equal(r.view.metrics().cachedRows,0);assert.equal(r.view.metrics().renderedRows,0);
  r.view.update(state(300,{datasetId:3,queryId:1}));await settle();assert.equal(r.last()[0].row.raw,'new query');r.view.destroy();
});

test('partial count growth and scaled wheel movement keep the existing visible anchor',async()=>{
  const r=rig();r.view.update(state());await settle();r.viewport.scroll(1900);await settle();
  r.view.update(state(20_000_000,{queryComplete:false,queryStatus:'running'}));assert.equal(r.view.metrics().first,1900);
  let prevented=false;r.viewport.listeners.get('wheel')({deltaY:ROW_HEIGHT,deltaMode:0,preventDefault(){prevented=true;}});
  assert.equal(prevented,true);assert.equal(r.view.metrics().first,1901);assert.ok(r.view.metrics().scrollHeight<=SCROLL_HEIGHT_LIMIT);r.view.destroy();
});

test('large previews stay within cache budget and prefetch stops instead of spinning on eviction',async()=>{
  let requests=0;const r=rig(async start=>{requests++;return {start,rows:Array.from({length:8},(_,i)=>({...row(start+i),raw:'x'.repeat(60000)}))};});
  r.view.update(state());await settle();
  for(const index of [200,400,600,800]){r.viewport.scroll(index);await settle();assert.equal(r.last().find(e=>e.index===index).row.lineNumber,index+1);assert.ok(r.view.metrics().cachedBytes<=CACHE_BYTE_LIMIT);}
  assert.ok(requests<120,requests);r.view.destroy();
});

test('explicit millionth-result jump waits for a growing count and centers selected physical row',async()=>{
  const r=rig();r.view.update(state(10,{viewportStart:1_000_000,scrollRevision:1,queryComplete:false}));await settle();assert.equal(r.view.metrics().first,0);
  r.view.update(state(2_000_000,{viewportStart:1_000_000,scrollRevision:1,scrollAlign:'center'}));await settle();
  assert.equal(r.view.metrics().first,999_995);assert.equal(r.last().find(e=>e.index===1_000_000).row.lineNumber,1_000_001);r.view.destroy();
});

test('physical locator pending completion is query guarded and seek retains physical selection',async()=>{
  const id={datasetId:1};let pending=deferred(),queryId=0,locates=0;
  const transport=async b=>{
    let data;
    if(b.action==='status')data={...id,name:'synthetic',total:10000,size:100000,status:'ready',complete:true,textComplete:true,query:{id:queryId,status:'complete',count:5000}};
    if(b.action==='query'){queryId=b.queryId;data={...id,queryId,query:{id:queryId,status:'complete',count:5000}};}
    if(b.action==='page')data={...id,queryId:b.queryId,start:b.start,count:5000,rows:[row(b.start)]};
    if(b.action==='locate'){locates++;data=await pending.promise;}
    return data;
  };
  const controller=createLargeLogController({transport,setTimer:()=>1,clearTimer(){}});await controller.start();await settle();
  const locating=controller.locateLine(4001);await settle();assert.equal(locates,1);
  pending.resolve({...id,queryId:controller.getState().queryId,lineNumber:4001,found:true,pending:false,index:2000});await locating;
  assert.equal(controller.getState().viewportStart,2000);assert.equal(controller.getState().selectedLine,4001);
  pending=deferred();const oldQuery=controller.getState().queryId,oldLocation=controller.locateLine(6001);await settle();
  await controller.setFilters({query:'replacement'});
  pending.resolve({...id,queryId:oldQuery,lineNumber:6001,found:true,pending:false,index:3000});await oldLocation;
  assert.equal(controller.getState().selectedLine,null);assert.equal(controller.getState().viewportStart,0);
  await controller.close({remote:false});
});

test('a pending restore anchor does not overwrite the owner position with a provisional zero',async()=>{
  const visible=[],r=rig(undefined,(first,last,count)=>visible.push({first,count}));
  r.view.update(state(0,{viewportStart:500,scrollRevision:1,queryComplete:false,queryStatus:'running'}));await settle();
  assert.deepEqual(visible,[]);
  r.view.update(state(10000,{viewportStart:500,scrollRevision:1,queryComplete:false,queryStatus:'running'}));await settle();
  assert.equal(visible.at(-1).first,500);r.view.destroy();
});

test('locate retries a busy engine response instead of losing the jump',async()=>{
  const id={datasetId:1},timers=[];let queryId=0,locates=0;
  const transport=async b=>{
    let data;
    if(b.action==='status')data={...id,name:'synthetic',total:10000,size:100000,status:'ready',complete:true,textComplete:true,query:{id:queryId,status:'complete',count:5000}};
    if(b.action==='query'){queryId=b.queryId;data={...id,queryId,query:{id:queryId,status:'complete',count:5000}};}
    if(b.action==='page')data={...id,queryId:b.queryId,start:b.start,count:5000,rows:[row(b.start)]};
    if(b.action==='locate'){locates++;if(locates===1)throw Object.assign(new Error('Local request failed'),{userMessage:'Local engine is busy; retry'});data={...id,queryId:b.queryId,lineNumber:b.lineNumber,found:true,pending:false,index:7};}
    return data;
  };
  const controller=createLargeLogController({transport,setTimer:fn=>timers.push(fn),clearTimer(){}});
  const flush=async()=>{for(let i=0;i<10;i++){await settle();while(timers.length)timers.shift()();}};
  const started=controller.start();await flush();await started;
  const locating=controller.locateLine(4001);await flush();await locating;
  assert.equal(locates,2);assert.equal(controller.getState().error,'');
  assert.equal(controller.getState().viewportStart,7);assert.equal(controller.getState().selectedLine,4001);
  await controller.close({remote:false});
});

test('a locate for a stopped query reports unknown, not absence from the filter',async()=>{
  const id={datasetId:1};let queryId=0;
  const transport=async b=>{
    let data;
    if(b.action==='status')data={...id,name:'synthetic',total:10000,size:100000,status:'ready',complete:true,textComplete:true,query:{id:queryId,status:'cancelled',count:10}};
    if(b.action==='query'){queryId=b.queryId;data={...id,queryId,query:{id:queryId,status:'cancelled',count:10}};}
    if(b.action==='locate')data={...id,queryId:b.queryId,lineNumber:b.lineNumber,found:false,pending:false,incomplete:true};
    return data;
  };
  const controller=createLargeLogController({transport,setTimer:()=>1,clearTimer(){}});await controller.start();await settle();
  await controller.locateLine(9000);
  assert.match(controller.getState().error,/완료되지 않아/);assert.equal(controller.getState().selectedLine,null);
  await controller.close({remote:false});
});

test('an aborted viewport read is retried instead of leaving loading placeholders until the next scroll',async()=>{
  let calls=0;
  const r=rig(async start=>{calls++;if(calls===1)throw new DOMException('Aborted','AbortError');return {start,rows:Array.from({length:200},(_,n)=>row(start+n))};});
  r.view.update(state(10000,{queryComplete:false,queryStatus:'cancelled'}));await settle();
  assert.ok(calls>=2);assert.equal(r.last().find(e=>e.index===0)?.row?.lineNumber,1);r.view.destroy();
});


test('selected arrows follow filtered ordinal order with minimal scrolling and bounds',async()=>{
  const selected=[],r=rig(async start=>({start,rows:Array.from({length:17},(_,i)=>({...row(start+i),lineNumber:(start+i)*3+7}))}),undefined,(row,index)=>selected.push({line:row.lineNumber,index}));
  r.view.update(state(430));await settle();r.view.selectIndex(8);await settle();
  const key=(name,target)=>r.viewport.listeners.get('keydown')({key:name,target,preventDefault(){}});
  assert.equal(r.view.metrics().logicalPosition,0);key('ArrowDown');await settle();assert.equal(selected.at(-1).line,34);assert.equal(r.view.metrics().logicalPosition,0);
  key('ArrowDown');await settle();assert.equal(r.view.metrics().logicalPosition,ROW_HEIGHT);
  r.view.selectIndex(199);await settle();key('ArrowDown');key('ArrowDown');await settle();
  assert.deepEqual(selected.at(-1),{line:610,index:201});key('ArrowUp');await settle();assert.equal(selected.at(-1).index,200);
  r.view.selectIndex(0);await settle();const count=selected.length;key('ArrowUp');await settle();assert.equal(selected.length,count);
  r.view.selectIndex(429);await settle();key('ArrowDown');await settle();assert.equal(selected.at(-1).index,429);
  for(const target of [{isContentEditable:true},{closest:()=>({})}]){key('ArrowUp',target);assert.equal(r.view.metrics().selectionIndex,429);}
  r.view.destroy();
});

test('held arrows coalesce uncached selection and reject replaced query/closed replies',async()=>{
  const pending=[],selected=[];
  const r=rig(start=>{const d=deferred();pending.push({start,...d});return d.promise;},undefined,(row,index)=>selected.push({line:row.lineNumber,index}));
  r.view.update(state(500));await settle();pending.shift().resolve({start:0,rows:[row(0)]});await settle();r.view.selectIndex(0);
  const key=()=>r.viewport.listeners.get('keydown')({key:'ArrowDown',preventDefault(){}});
  for(let i=0;i<220;i++)key();await settle();assert.equal(r.view.metrics().selectionIndex,220);
  pending[0].resolve({start:pending[0].start,rows:Array.from({length:200},(_,i)=>row(pending[0].start+i))});await settle();
  assert.equal(selected.length,1,'loaded earlier pages cannot publish a superseded selection');
  const latest=pending[1];assert.equal(latest.start,220);latest.resolve({start:220,rows:[row(220)]});await settle();assert.deepEqual(selected.at(-1),{line:221,index:220});
  key();await settle();r.view.update(state(500,{queryId:2}));const old=pending.find(x=>x!==latest&&x!==pending[0]);
  if(old){old.resolve({start:old.start,rows:[row(old.start)]});await settle();}
  assert.equal(r.view.metrics().selectionIndex,null);
  r.view.update(state(500,{closed:true,queryId:2}));for(const d of pending)d.resolve({start:d.start,rows:[row(d.start)]});await settle();
  assert.equal(selected.at(-1).index,220);assert.equal(r.view.metrics().cachedRows,0);r.view.destroy();
});


test('resolved physical navigation resets the keyboard ordinal even when the selected line stays the same',async()=>{
  const selected=[],r=rig(async start=>({start,rows:Array.from({length:200},(_,i)=>({...row(start+i),lineNumber:(start+i)*3+7}))}),undefined,(row,index)=>selected.push({line:row.lineNumber,index}));
  r.view.update(state(500,{selectedLine:610,locationPending:610}));await settle();
  r.view.update(state(500,{selectedLine:610,viewportStart:201,selectedIndex:201,scrollRevision:1,scrollAlign:'center'}));await settle();
  r.viewport.listeners.get('keydown')({key:'ArrowDown',preventDefault(){}});await settle();
  assert.deepEqual(selected.at(-1),{line:613,index:202});r.view.destroy();
});

test('a restored anchor keeps the cached selected row ordinal instead of the first visible row',async()=>{
  const selected=[],r=rig(undefined,undefined,(row,index)=>selected.push({line:row.lineNumber,index}));
  r.view.update(state(500));await settle();
  // Restore puts row 40 at the top while the selected row 5 stays cached above it.
  r.view.update(state(500,{selectedLine:5,viewportStart:40,selectedIndex:null,scrollRevision:1}));await settle();
  assert.equal(r.view.metrics().selectionIndex,4);
  r.viewport.listeners.get('keydown')({key:'ArrowDown',preventDefault(){}});await settle();
  assert.deepEqual(selected.at(-1),{line:6,index:5});r.view.destroy();
});

test('an uncached outside selection leaves arrows unselected until its ordinal is known',async()=>{
  const selected=[],r=rig(async start=>({start,rows:Array.from({length:Math.min(200,500-start)},(_,n)=>row(start+n))}),undefined,(row,index)=>selected.push({line:row.lineNumber,index}));
  r.view.update(state(5000,{selectedLine:3}));await settle();assert.equal(r.view.metrics().selectionIndex,2);
  // A pending locate selects a physical row whose filtered ordinal is unknown.
  r.view.update(state(5000,{selectedLine:4000,locationPending:4000}));await settle();
  assert.equal(r.view.metrics().selectionIndex,null);
  r.viewport.listeners.get('keydown')({key:'ArrowDown',preventDefault(){}});await settle();
  assert.equal(selected.length,0,'the arrow scrolls instead of selecting and cancelling the locate');
  assert.equal(r.view.metrics().logicalPosition,ROW_HEIGHT);r.view.destroy();
});

test('held arrow selections report key repeat to the owner',async()=>{
  const selected=[],r=rig(undefined,undefined,(row,index,options)=>selected.push({index,...options}));
  r.view.update(state(500));await settle();r.view.selectIndex(0);await settle();
  const key=repeat=>r.viewport.listeners.get('keydown')({key:'ArrowDown',repeat,preventDefault(){}});
  key(false);await settle();key(true);await settle();
  assert.deepEqual(selected.slice(-2),[{index:1,repeat:false,extend:false},{index:2,repeat:true,extend:false}]);r.view.destroy();
});

test('shift arrows extend the selection and cached rows are read only when complete',async()=>{
  const selected=[],r=rig(undefined,undefined,(row,index,options)=>selected.push({index,extend:options.extend}));
  r.view.update(state(500));await settle();r.view.selectIndex(3);await settle();
  r.viewport.listeners.get('keydown')({key:'ArrowDown',shiftKey:true,preventDefault(){}});await settle();
  assert.deepEqual(selected.at(-1),{index:4,extend:true});
  assert.deepEqual(r.view.cachedRows(3,4).map(row=>row.lineNumber),[4,5]);
  assert.equal(r.view.cachedRows(0,499),null,'uncached rows are never guessed');r.view.destroy();
});

test('a shrinking list keeps its selected row in view only when it was in view',async()=>{
  const r=rig();r.view.update(state(500));await settle();r.view.selectIndex(8);await settle();
  assert.equal(r.view.metrics().logicalPosition,0);
  r.viewport.clientHeight=140;r.window.listeners.get('resize')();await settle();
  assert.equal(r.view.metrics().logicalPosition,9*ROW_HEIGHT-140,'the selected row is the last row in view');
  r.viewport.clientHeight=280;r.window.listeners.get('resize')();await settle();
  assert.equal(r.view.metrics().logicalPosition,9*ROW_HEIGHT-140,'growing keeps the position');
  r.view.selectIndex(400);await settle();r.viewport.scrollTop=0;r.viewport.listeners.get('scroll')();await settle();
  const away=r.view.metrics().logicalPosition;
  r.viewport.clientHeight=140;r.window.listeners.get('resize')();await settle();
  assert.equal(r.view.metrics().logicalPosition,away,'a selection out of view does not pull the list');r.view.destroy();
});

test('shift with page and end keys extends the selection; without shift they only scroll',async()=>{
  const selected=[],r=rig(undefined,undefined,(row,index,options)=>selected.push({index,extend:options.extend}));
  r.view.update(state(500));await settle();r.view.selectIndex(3);await settle();
  const key=(name,shiftKey=true)=>r.viewport.listeners.get('keydown')({key:name,shiftKey,preventDefault(){}});
  const page=Math.floor((r.viewport.clientHeight-ROW_HEIGHT)/ROW_HEIGHT);assert.equal(page,9);
  key('PageDown');await settle();assert.deepEqual(selected.at(-1),{index:3+page,extend:true});
  key('PageUp');key('PageUp');await settle();assert.deepEqual(selected.at(-1),{index:0,extend:true},'clamped at the first row');
  key('End');await settle();assert.deepEqual(selected.at(-1),{index:499,extend:true});
  assert.ok(r.view.metrics().logicalPosition>0,'the new focus row is shown');
  key('Home');await settle();assert.deepEqual(selected.at(-1),{index:0,extend:true});
  const count=selected.length;key('End',false);key('PageUp',false);await settle();
  assert.equal(selected.length,count,'plain keys scroll without selecting');r.view.destroy();
});

test('a drag finds the row at a client y in view and scrolls by logical rows, also on a scaled scrollbar',async()=>{
  const r=rig();
  assert.equal(r.view.indexAt(60),null,'no rows yet');
  r.viewport.getBoundingClientRect=()=>({top:50,bottom:330});
  r.view.update(state(1_000_000_000));await settle();
  assert.equal(r.view.indexAt(50),0);assert.equal(r.view.indexAt(50+ROW_HEIGHT*3+5),3);
  assert.equal(r.view.indexAt(5000),r.view.metrics().last-1,'kept to the rows in view');assert.equal(r.view.indexAt(-5000),0);
  r.view.scrollBy(ROW_HEIGHT*1000);await settle();
  assert.equal(r.view.metrics().first,1000);assert.equal(r.view.indexAt(51),1000);
  assert.equal(r.last().find(e=>e.index===1000)?.row.lineNumber,1001,'rows scrolled into view are read');
  r.view.scrollBy(ROW_HEIGHT*700_000_000);await settle();
  assert.equal(r.view.metrics().first,700_001_000,'one-row precision far beyond the native scrollbar');
  assert.ok(r.viewport.scrollTop<=SCROLL_HEIGHT_LIMIT);assert.equal(r.view.indexAt(50+ROW_HEIGHT),700_001_001);
  r.view.scrollBy(-Infinity);r.view.scrollBy(NaN);assert.equal(r.view.metrics().first,700_001_000,'nonsense deltas are ignored');
  r.view.scrollBy(-ROW_HEIGHT*2_000_000_000);await settle();assert.equal(r.view.metrics().first,0);
  r.view.destroy();assert.equal(r.view.indexAt(60),null);
});

test('the end position shows whole rows without changing scaled travel',()=>{
  for(const height of [280,290,300,307]){
    const p=new ScrollPosition();p.update(1000,height);p.jump(999);
    assert.equal(p.position%ROW_HEIGHT,0,`${height}`);assert.equal(p.last,1000);assert.ok(p.position+height>=1000*ROW_HEIGHT);
    assert.equal(p.physical,p.position,'an unscaled list maps native and logical positions one to one');assert.equal(p.scaled,false);
  }
  const few=new ScrollPosition();few.update(5,290);assert.equal(few.maximum,0);assert.equal(few.extent,290);
  const huge=new ScrollPosition();huge.update(9_000_000_000,290);huge.native(huge.physicalMaximum);
  assert.equal(huge.position%ROW_HEIGHT,0);assert.equal(huge.last,huge.count);assert.equal(huge.extent,SCROLL_HEIGHT_LIMIT);
});

test('refresh drops replies in flight, strips kept rows at once and re-reads only around the position',async()=>{
  const pending=[],r=rig(start=>{const d=deferred();pending.push({start,...d});return d.promise;});
  const marked=start=>Array.from({length:200},(_,n)=>({...row(start+n),searchHit:true}));
  r.view.update(state(5000));await settle();pending[0].resolve({start:0,rows:marked(0)});await settle();
  r.viewport.scroll(150);await settle();assert.equal(pending.length,2);
  const strip=row=>row.searchHit?{...row,searchHit:false}:row;
  r.view.refresh(strip);
  assert.ok(r.last().every(e=>e.row&&!e.row.searchHit),'kept rows paint without stale marks');assert.equal(r.view.metrics().logicalPosition,150*ROW_HEIGHT);
  pending[1].resolve({start:pending[1].start,rows:marked(pending[1].start)});await settle();
  assert.ok(r.last().every(e=>!e.row?.searchHit),'a reply requested before the refresh is discarded');
  assert.equal(pending.length,3);assert.ok(pending[2].start<=150,'the visible rows are read again first');
  pending[2].resolve({start:pending[2].start,rows:Array.from({length:200},(_,n)=>row(pending[2].start+n))});await settle();
  const reads=pending.length;r.view.refresh(strip,{reread:false});await settle();
  assert.equal(pending.length,reads,'unmarking needs no read');r.view.destroy();
});

test('a typed row reports its outcome and a hidden row selects the next counted result',async()=>{
  const id={datasetId:1},outcomes=[];let queryId=0,stale=null;
  const transport=async b=>{
    if(b.action==='status')return {...id,name:'synthetic',total:10000,size:100000,status:'ready',complete:true,textComplete:true,query:{id:queryId,status:'complete',count:10}};
    if(b.action==='query'){queryId=b.queryId;return {...id,queryId,query:{id:queryId,status:'complete',count:10}};}
    if(b.action==='locate')return b.lineNumber===50?{...id,queryId:b.queryId,lineNumber:50,found:false,pending:false,incomplete:true}:{...id,queryId:b.queryId,lineNumber:b.lineNumber,found:b.lineNumber%10===0,pending:false,index:Math.floor(b.lineNumber/10)-(b.lineNumber%10===0?1:0)};
    if(b.action==='page'){if(stale)await stale.promise;return {...id,queryId:b.queryId,start:b.start,count:10,rows:[{lineNumber:(b.start+1)*10,raw:'',kind:'raw'}]};}
  };
  const controller=createLargeLogController({transport,setTimer:()=>1,clearTimer(){},onLocate:o=>outcomes.push(o)});await controller.start();await settle();
  await controller.locateLine(30,{nearest:true});assert.deepEqual(outcomes.at(-1),{lineNumber:30,outcome:'found'});
  await controller.locateLine(31,{nearest:true});assert.deepEqual(outcomes.at(-1),{lineNumber:31,outcome:'nearest',target:40});
  assert.equal(controller.getState().selectedLine,40);assert.equal(controller.getState().viewportStart,3);assert.equal(controller.getState().error,'');
  await controller.locateLine(101,{nearest:true});assert.deepEqual(outcomes.at(-1),{lineNumber:101,outcome:'hidden'});assert.equal(controller.getState().selectedLine,null);
  await controller.locateLine(50,{nearest:true});assert.deepEqual(outcomes.at(-1),{lineNumber:50,outcome:'incomplete'});assert.equal(controller.getState().error,'');
  // A newer jump supersedes a next-result read still in flight.
  stale=deferred();const first=controller.locateLine(61,{nearest:true});await settle();
  const pendingRead=stale;stale=null;await controller.locateLine(20,{nearest:true});pendingRead.resolve();await first;await settle();
  assert.equal(controller.getState().selectedLine,20);assert.deepEqual(outcomes.at(-1),{lineNumber:20,outcome:'found'});assert.equal(outcomes.filter(o=>o.lineNumber===61).length,0);
  // Without `nearest`, a missing row keeps the existing error report.
  await controller.locateLine(31);assert.match(controller.getState().error,/찾을 수 없습니다/);
  await controller.close({remote:false});
});
