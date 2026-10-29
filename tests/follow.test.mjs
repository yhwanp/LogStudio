import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as nextTurn} from 'node:timers/promises';
import {mountLargeLogView} from '../dist/large-view.js';
import {desktopTransport} from '../dist/desktop.js';
import {createLargeLogController} from '../dist/controller.js';
import {createContinuousViewport, ROW_HEIGHT} from '../dist/continuous-view.js';
import {describeLargeProgress, describeFollowError} from '../dist/describe.js';

async function settle(){for(let i=0;i<10;i++)await nextTurn();}
class Element {
  constructor(tag='div'){this.tagName=tag.toUpperCase();this.children=[];this.listeners={};this.dataset={};this.attributes={};this.value='';this.checked=false;this.hidden=false;this._text='';this.style={setProperty(key,value){this[key]=value;},removeProperty(key){delete this[key];}};this.clientHeight=280;this.scrollTop=0;this.classList={toggle(){}};}
  get textContent(){return this._text+this.children.map(child=>child.textContent).join('');}
  set textContent(value){this._text=String(value);this.children=[];}
  append(...nodes){for(const node of nodes){if(node.fragment){this.append(...node.children);continue;}if(node.parentElement)node.parentElement.children=node.parentElement.children.filter(child=>child!==node);node.parentElement=this;this.children.push(node);}}
  replaceChildren(...nodes){this.children=[];this._text='';this.append(...nodes);}
  insertBefore(node,before){if(node.parentElement)node.parentElement.removeChild(node);const i=before?this.children.indexOf(before):this.children.length;this.children.splice(i,0,node);node.parentElement=this;}
  removeChild(node){this.children=this.children.filter(child=>child!==node);node.parentElement=null;}
  removeAttribute(key){delete this.attributes[key];}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  focus(){if(this.ownerDocument)this.ownerDocument.activeElement=this;}
  setAttribute(key,value){this.attributes[key]=String(value);}
  addEventListener(key,handler){(this.listeners[key]??=[]).push(handler);}
  dispatch(key,event={}){return Promise.all((this.listeners[key]??[]).map(fn=>fn(event)));}
  querySelector(selector){const line=selector.match(/^\[data-line-number="(\d+)"\]$/)?.[1];return this.children.find(child=>line?child.dataset.lineNumber===line:child.className===selector.slice(1));}
}
const row=n=>({lineNumber:n,raw:`raw ${n}`,kind:'raw'});
// The mounted view against a native engine whose file can be followed.
async function rig({followable=true}={}){
  let ownerDocument;const elements=new Map(),get=id=>{if(!elements.has(id))elements.set(id,Object.assign(new Element(),{ownerDocument}));return elements.get(id);};
  delete get('workspace').style;
  new Element().append(get('searchInput'));new Element().append(get('caseSensitive'));get('includeRaw').checked=true;
  const document={getElementById:get,createElement:tag=>Object.assign(new Element(tag),{ownerDocument:document}),createDocumentFragment:()=>Object.assign(new Element(),{fragment:true}),querySelector:()=>new Element(),addEventListener(){}};
  ownerDocument=document;for(const element of elements.values())element.ownerDocument=document;
  const window={setTimeout:()=>0,clearTimeout(){},addEventListener(){},navigator:{}};
  // Engine state: the follow request flips `following`; tests grow the file.
  const engine={following:false,total:6,tailRevision:0,followError:'',revision:0,status:'complete'};
  const requests=[],events=new Map();
  const status=b=>({datasetId:1,revision:++engine.revision,name:'live.log',size:100,total:engine.total,textComplete:true,summaryComplete:!engine.following,complete:!engine.following,status:'ready',followable,following:engine.following,followError:engine.followError,tailRevision:engine.tailRevision,summary:{counts:{I:engine.total}},query:b.queryId||b.action==='follow'||b.action==='status'?{id:querySeen[b.pane]??0,status:engine.following?'running':engine.status,count:engine.total,scannedBytes:100,complete:!engine.following,caughtUp:engine.following}:null});
  const querySeen={};
  const native={core:{invoke:async(action,args={})=>{
    if(action==='take_startup_error')return null;
    if(action!=='log_request')return {};
    const b=args.request;requests.push(b);
    if(b.action==='close')return {closed:true};
    if(b.action==='follow'){engine.following=b.follow;return status(b);}
    if(b.action==='status')return status(b);
    if(b.action==='query'){querySeen[b.pane]=b.queryId;return {datasetId:1,queryId:b.queryId,query:{id:b.queryId,status:engine.following?'running':'complete',count:engine.total,complete:!engine.following,caughtUp:engine.following}};}
    if(b.action==='page')return {datasetId:1,queryId:b.queryId,start:b.start,count:engine.total,rows:Array.from({length:Math.max(0,Math.min(b.count,engine.total-b.start))},(_,i)=>row(b.start+i+1))};
    throw Error(b.action);
  }},event:{listen:async(name,fn)=>events.set(name,fn)}};
  const controller=mountLargeLogView(native,{document,window,transport:desktopTransport(native)});
  await settle();
  return {get,controller,requests,engine,poll:async()=>{await controller.logs.start();await settle();}};
}

test('the follow toggle shows only for a followable file and turns following on and off',async t=>{
  const plain=await rig({followable:false});t.after(()=>plain.controller.close());
  assert.equal(plain.get('followFile').hidden,true);await plain.controller.close();
  const r=await rig();t.after(()=>r.controller.close());
  const button=r.get('followFile');
  assert.equal(button.hidden,false);assert.equal(button.disabled,false);assert.equal(button.attributes['aria-pressed'],'false');
  assert.equal(r.get('exportLogs').disabled,false);
  await button.dispatch('click');await settle();
  assert.deepEqual(r.requests.filter(b=>b.action==='follow').map(b=>[b.follow,b.datasetId]),[[true,1]]);
  assert.equal(button.attributes['aria-pressed'],'true');
  assert.equal(r.get('largeStatus').textContent,'파일 6행 · 따라가는 중');
  // Results are partial while rows can still arrive: no export, no final count.
  assert.equal(r.get('exportLogs').disabled,true);assert.match(r.get('exportLogs').title,/따라가기를 끄면/);
  assert.match(r.get('logsCount').textContent,/현재까지 6건/);
  // The search pane asked for its status after the toggle.
  assert.ok(r.requests.some(b=>b.action==='status'&&b.pane==='search'));
  await button.dispatch('click');await settle();
  assert.deepEqual(r.requests.filter(b=>b.action==='follow').map(b=>b.follow),[true,false]);
  assert.equal(button.attributes['aria-pressed'],'false');
  assert.equal(r.get('largeStatus').textContent,'');
  await r.controller.close();
});

test('arriving rows grow the followed list and a replaced last row is read again',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  await r.get('followFile').dispatch('click');await settle();
  const pages=()=>r.requests.filter(b=>b.action==='page').length;
  r.engine.total=9;await r.poll();
  assert.equal(r.controller.logs.getState().count,9);
  const before=pages();
  r.engine.tailRevision=1;r.engine.total=8;await r.poll();
  // A replaced row may leave one result fewer; shown rows are read again.
  assert.equal(r.controller.logs.getState().count,8);
  assert.ok(pages()>before);
  await r.controller.close();
});

test('a following error is explained and the rows read so far stay usable',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  await r.get('followFile').dispatch('click');await settle();
  r.engine.following=false;r.engine.followError='NUL is not supported; select a UTF-8 text log';await r.poll();
  assert.equal(r.get('largeError').hidden,false);
  assert.match(r.get('largeError').textContent,/NUL 바이트.*따라가기를 멈췄습니다/);
  assert.equal(r.get('followFile').attributes['aria-pressed'],'false');
  await r.controller.close();
});

test('following status text separates waiting for rows from searching',()=>{
  const base={datasetId:1,status:'ready',textComplete:true,total:12,size:100,indexedBytes:100,count:3,queryStatus:'running',queryComplete:false,summaryComplete:false};
  const waiting=describeLargeProgress({...base,following:true,caughtUp:true},{compactSearch:true});
  assert.equal(waiting.query,'따라가는 중');assert.equal(waiting.status,'파일 12행 · 따라가는 중');
  assert.equal(describeLargeProgress({...base,following:true,caughtUp:false},{compactSearch:true}).query,'검색 중');
  assert.equal(describeLargeProgress({...base,count:0,following:true,caughtUp:true}).range,'아직 결과가 없습니다 · 따라가는 중');
  assert.equal(describeLargeProgress(base).status,'파일 12행 · 레벨 집계 중');
  // Rows appended while not following are announced, not read.
  assert.equal(describeLargeProgress({...base,summaryComplete:true,grown:true}).status,'파일 12행 · 새 줄 있음');
  assert.equal(describeLargeProgress({...base,following:true,caughtUp:true,grown:true}).status,'파일 12행 · 따라가는 중');
  assert.match(describeFollowError('A physical line exceeds 1 MiB'),/^추가된 줄이 1 MiB를 넘어 따라가기를 멈췄습니다/);
  assert.match(describeFollowError('Following stopped at 8 GiB'),/8 GiB/);
  assert.equal(describeFollowError(''),'');
});

function clock(){
  const pending=new Map();let next=0;
  return {pending,setTimer(fn,ms){const id=++next;pending.set(id,{fn,ms});return id;},clearTimer(id){pending.delete(id);}};
}
test('a completed query of a followed file runs again and polls slowly while waiting',async()=>{
  const timers=clock(),snapshots=[];let queryId=0;
  const transport=async body=>{
    if(body.action==='query'){queryId=body.queryId;return {datasetId:1,queryId,query:{id:queryId,status:'complete',count:5,complete:true}};}
    if(body.action==='status')return snapshots.shift();
    throw Error(body.action);
  };
  const snap=(revision,extra,query)=>({datasetId:1,revision,status:'ready',textComplete:true,summaryComplete:true,complete:true,total:5,query:{id:queryId,scannedBytes:10,...query},...extra});
  const c=createLargeLogController({pane:'logs',transport,...timers});
  snapshots.push({datasetId:1,revision:1,status:'ready',textComplete:true,complete:true,total:5,query:null});
  await c.start();await settle();
  assert.equal(c.getState().queryStatus,'complete');assert.equal(c.getState().queryComplete,true);
  // Following resumes the completed identity instead of being ignored.
  snapshots.push(snap(2,{following:true,summaryComplete:false,complete:false},{status:'running',count:5,complete:false,caughtUp:true}));
  await c.start();await settle();
  assert.equal(c.getState().queryStatus,'running');assert.equal(c.getState().queryComplete,false);assert.equal(c.getState().caughtUp,true);
  // The timer the earlier query left fires once; waiting for rows then polls at the normal cadence.
  snapshots.push(snap(3,{following:true,summaryComplete:false,complete:false},{status:'running',count:5,complete:false,caughtUp:true}));
  for(const [id,task] of [...timers.pending])if(task.ms!==800){timers.pending.delete(id);task.fn();}
  await settle();
  assert.deepEqual([...timers.pending.values()].map(t=>t.ms),[800]);
  // Counts never shrink, except after a replaced last row.
  snapshots.push(snap(4,{following:true,tailRevision:0},{status:'running',count:4,complete:false,caughtUp:true}));
  await c.start();await settle();assert.equal(c.getState().count,5);
  snapshots.push(snap(5,{following:true,tailRevision:1},{status:'running',count:4,complete:false,caughtUp:true}));
  await c.start();await settle();assert.equal(c.getState().count,4);
  // Without following, a stale running snapshot cannot reopen a final query.
  snapshots.push(snap(6,{following:false},{status:'complete',count:4,complete:true}));
  await c.start();await settle();
  snapshots.push(snap(7,{following:false},{status:'running',count:4,complete:false}));
  await c.start();await settle();assert.equal(c.getState().queryStatus,'complete');
  c.close({remote:false});
});

test('a followed list keeps its end in view only while the end is in view',async()=>{
  const viewport=new Element(),rowsElement=new Element();viewport.listeners=new Map();
  viewport.addEventListener=(n,f)=>viewport.listeners.set(n,f);viewport.removeEventListener=n=>viewport.listeners.delete(n);
  const window={addEventListener(){},removeEventListener(){}};
  const view=createContinuousViewport({viewport,rowsElement,document:{createElement:()=>new Element()},window,fetchRows:async start=>({start,rows:[]}),paint(){}});
  const state=(count,extra={})=>({datasetId:1,queryId:1,count,viewportStart:0,scrollRevision:0,queryComplete:false,queryStatus:'running',closed:false,selectedLine:3,...extra});
  const visible=Math.floor(280/ROW_HEIGHT);
  view.update(state(5),{stick:true});
  view.update(state(50),{stick:true});
  assert.equal(view.metrics().last,50);assert.equal(view.metrics().first,50-visible);
  // Rows arriving without following, or after scrolling up, do not move it.
  view.update(state(60));assert.equal(view.metrics().last,50);
  viewport.scrollTop=0;viewport.listeners.get('scroll')();
  view.update(state(70),{stick:true});assert.equal(view.metrics().first,0);
  view.scrollToEnd();assert.equal(view.metrics().last,70);
  view.update(state(80),{stick:true});assert.equal(view.metrics().last,80);
  // A seek (new anchor) wins over the end.
  view.update(state(90,{scrollRevision:1,viewportStart:10}),{stick:true});assert.equal(view.metrics().first,10);
  view.destroy();
});

test('a held followed list stays still as rows arrive and goes back to the end when let go',async()=>{
  // A row's menu holds the list, so the row stays beside the menu.
  const viewport=new Element(),rowsElement=new Element();viewport.listeners=new Map();
  viewport.addEventListener=(n,f)=>viewport.listeners.set(n,f);viewport.removeEventListener=n=>viewport.listeners.delete(n);
  const window={addEventListener(){},removeEventListener(){}},moves=[];
  const view=createContinuousViewport({viewport,rowsElement,document:{createElement:()=>new Element()},window,fetchRows:async start=>({start,rows:[]}),paint(){},onUserMove:()=>moves.push(view.metrics().first)});
  const state=(count,extra={})=>({datasetId:1,queryId:1,count,viewportStart:0,scrollRevision:0,queryComplete:false,queryStatus:'running',closed:false,selectedLine:3,...extra});
  const scrolled=()=>viewport.listeners.get('scroll')();
  view.update(state(5),{stick:true});view.update(state(50),{stick:true});
  let release=view.hold();
  view.update(state(60),{stick:true});assert.equal(view.metrics().last,50);
  // The scroll event echoing the list's own move is not the user's.
  scrolled();assert.deepEqual(moves,[]);assert.equal(view.metrics().last,50);
  release();assert.equal(view.metrics().last,60);
  scrolled();release();assert.deepEqual(moves,[]);
  view.update(state(70),{stick:true});assert.equal(view.metrics().last,70,'it follows the end again');
  // A press that closes one menu may open another: the last release lets go.
  const first=view.hold(),second=view.hold();
  view.update(state(80),{stick:true});first();assert.equal(view.metrics().last,70);
  second();assert.equal(view.metrics().last,80);
  // Rows scrolled by the user while held stay where they were moved.
  release=view.hold();view.update(state(90),{stick:true});
  viewport.scrollTop=10*ROW_HEIGHT;scrolled();
  assert.deepEqual(moves,[10],'the user moving it is heard');
  release();assert.equal(view.metrics().first,10);
  view.update(state(100),{stick:true});assert.equal(view.metrics().first,10);
  // So do rows a locate showed (a bookmark stepped to) while held.
  view.scrollToEnd();release=view.hold();
  view.update(state(110,{scrollRevision:1,viewportStart:20}),{stick:true});release();assert.equal(view.metrics().first,20);
  // A list not following when held stays when let go.
  view.update(state(120,{scrollRevision:1}));view.scrollToEnd();
  release=view.hold();view.update(state(130,{scrollRevision:1}));release();assert.equal(view.metrics().last,120);
  // Another query is not held: it follows its own end, and the release
  // leaves it there.
  view.update(state(140,{scrollRevision:1}),{stick:true});view.scrollToEnd();release=view.hold();
  for(const count of [0,5,40])view.update(state(count,{queryId:2}),{stick:true});
  assert.equal(view.metrics().last,40,'the new query follows its end while the menu is open');
  release();assert.equal(view.metrics().last,40);
  assert.deepEqual(moves,[10],'the list moving itself is never the user');
  view.destroy();
});

test('a row menu on a followed log stays open while rows arrive, the list held under it',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  // Only what the menu measures and searches.
  const menu=r.get('rowMenu');menu.hidden=true;menu.getBoundingClientRect=()=>({width:120,height:80});menu.querySelectorAll=()=>[];
  await r.get('followFile').dispatch('click');await settle();
  r.engine.total=40;await r.poll();
  const viewport=r.get('logsViewport'),range=()=>r.get('logsRange').textContent;
  const menuOn=line=>r.get('logsRows').querySelector(`[data-line-number="${line}"]`).dispatch('contextmenu',{preventDefault(){},clientX:10,clientY:10});
  assert.equal(range(),'31–40 / 40건');
  await menuOn(38);await settle();
  assert.equal(menu.hidden,false);
  r.engine.total=50;await r.poll();await viewport.dispatch('scroll');
  assert.equal(menu.hidden,false,'rows arriving, and the scroll event they cause, leave the menu open');
  assert.equal(range(),'31–40 / 50건','the list holds still under the menu');
  await menu.dispatch('keydown',{key:'Escape',preventDefault(){},stopPropagation(){}});
  assert.equal(menu.hidden,true);assert.equal(range(),'41–50 / 50건','closed, it follows the end again');
  // The user scrolling the list closes the menu; the list stays there.
  await menuOn(48);await settle();
  viewport.scrollTop-=2*ROW_HEIGHT;await viewport.dispatch('scroll');
  assert.equal(menu.hidden,true);
  r.engine.total=60;await r.poll();
  assert.equal(range(),'39–48 / 60건');
  await r.controller.close();
});

test('a completed single file is checked slowly for appended rows until some are seen',async()=>{
  const timers=clock(),snapshots=[];let queryId=0;
  const transport=async body=>{
    if(body.action==='query'){queryId=body.queryId;return {datasetId:1,queryId,query:{id:queryId,status:'complete',count:5,complete:true}};}
    if(body.action==='status')return snapshots.shift();
    throw Error(body.action);
  };
  const snap=(revision,grown)=>({datasetId:1,revision,status:'ready',textComplete:true,summaryComplete:true,complete:true,total:5,followable:true,grown,query:{id:queryId,status:'complete',count:5,scannedBytes:10,complete:true}});
  const c=createLargeLogController({pane:'logs',transport,...timers});
  snapshots.push({datasetId:1,revision:1,status:'ready',textComplete:true,complete:true,total:5,followable:true,query:null});
  await c.start();await settle();
  // The timer the query left fires once; then only the slow look remains.
  for(const [id,task] of [...timers.pending]){timers.pending.delete(id);snapshots.push(snap(2,false));task.fn();}
  await settle();
  assert.deepEqual([...timers.pending.values()].map(t=>t.ms),[2000]);
  const [[id,task]]=[...timers.pending];timers.pending.delete(id);snapshots.push(snap(3,true));task.fn();await settle();
  assert.equal(c.getState().grown,true);assert.equal(timers.pending.size,0);
  c.close({remote:false});
});
