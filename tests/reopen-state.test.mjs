import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as nextTurn} from 'node:timers/promises';
import {mountLargeLogView} from '../dist/large-view.js';
import {desktopTransport} from '../dist/desktop.js';
import {RELOADED} from '../dist/work-state.js';

// The mounted view against a scripted engine; same DOM stand-in as
// view-lifecycle.test.mjs, plus native work-state commands and `lines`.
async function settle(){for(let i=0;i<12;i++)await nextTurn();}
class Element {
  constructor(tag='div'){this.tagName=tag.toUpperCase();this.children=[];this.listeners={};this.dataset={};this.attributes={};this.value='';this.checked=false;this.hidden=false;this.disabled=false;this.title='';this._text='';this.style={setProperty(key,value){this[key]=value;},removeProperty(key){delete this[key];}};this.clientHeight=280;this.scrollTop=0;this.classList={toggle(){}};}
  get textContent(){return this._text+this.children.map(child=>child.textContent).join('');}
  set textContent(value){this._text=String(value);this.children=[];}
  append(...nodes){for(const node of nodes){if(node.fragment){this.append(...node.children);continue;}if(node.parentElement)node.parentElement.children=node.parentElement.children.filter(child=>child!==node);node.parentElement=this;this.children.push(node);}}
  replaceChildren(...nodes){this.children=[];this._text='';this.append(...nodes);}
  insertBefore(node,before){if(node.parentElement)node.parentElement.removeChild(node);const i=before?this.children.indexOf(before):this.children.length;this.children.splice(i,0,node);node.parentElement=this;}
  removeChild(node){this.children=this.children.filter(child=>child!==node);node.parentElement=null;}
  removeAttribute(key){delete this.attributes[key];}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  focus(){if(this.ownerDocument)this.ownerDocument.activeElement=this;}
  getBoundingClientRect(){return {left:0,top:0,right:0,bottom:0,width:0,height:0};}
  querySelectorAll(selector){return selector==='[role^=menuitem]'?this.children.filter(child=>child.attributes.role?.startsWith('menuitem')):[];}
  setAttribute(key,value){this.attributes[key]=String(value);}
  addEventListener(key,handler){(this.listeners[key]??=[]).push(handler);}
  dispatch(key,event={}){return Promise.all((this.listeners[key]??[]).map(fn=>fn(event)));}
  querySelector(selector){const line=selector.match(/^\[data-line-number="(\d+)"\]$/)?.[1];return this.children.find(child=>line?child.dataset.lineNumber===line:child.className===selector.slice(1));}
}
const raw=n=>({lineNumber:n,kind:'raw',raw:`raw ${n}`});
async function rig({rows=Array.from({length:30},(_,i)=>raw(i+1)),work=()=>({reopenable:true,match:null,state:null,reloadOf:null}),reopen=async()=>({}),savedFilters=[]}={}){
  let ownerDocument;const elements=new Map(),get=id=>{if(!elements.has(id))elements.set(id,Object.assign(new Element(),{ownerDocument}));return elements.get(id);};
  delete get('workspace').style;
  new Element().append(get('searchInput'));new Element().append(get('caseSensitive'));get('includeRaw').checked=true;
  const handlers=new Map(),document={getElementById:get,createElement:tag=>new Element(tag),createDocumentFragment:()=>Object.assign(new Element(),{fragment:true}),querySelector:()=>new Element(),addEventListener:(key,fn)=>handlers.set(key,fn)};
  ownerDocument=document;document.createElement=tag=>Object.assign(new Element(tag),{ownerDocument:document});
  for(const element of elements.values())element.ownerDocument=document;
  const timers=new Map();let timerId=0;
  const window={setTimeout:(fn,ms)=>{const id=++timerId;timers.set(id,{fn,ms});return id;},clearTimeout:id=>timers.delete(id),addEventListener(){},navigator:{clipboard:{writeText:async()=>{}}}};
  const env={dataset:1,complete:true,size:100,rows,total:rows.length},events=new Map(),calls=[];
  const native={core:{invoke:async(action,args={})=>{
    calls.push({action,args:structuredClone(args)});
    if(action==='work_state')return work(args.datasetId);
    if(action==='reopen_source')return reopen(args);
    if(action==='saved_filters')return savedFilters;
    if(action!=='log_request')return {};
    const b=args.request,id={datasetId:env.dataset};
    if(b.action==='close')return {closed:true};
    if(b.action==='status')return {...id,name:'grow.log',size:env.size,total:env.total,textComplete:env.complete,summaryComplete:true,complete:env.complete,status:env.complete?'ready':'indexing',summary:{counts:{I:3}},query:null};
    if(b.datasetId!==env.dataset)throw 'Stale source';
    if(b.action==='query'||b.action==='cancel')return {...id,queryId:b.queryId,query:{id:b.queryId,status:'complete',count:env.rows.length,complete:true}};
    if(b.action==='page')return {...id,queryId:b.queryId,start:b.start,count:env.rows.length,rows:env.rows.slice(b.start,b.start+b.count)};
    if(b.action==='locate'&&b.lineNumber>env.total)throw 'Physical row is not indexed yet; retry';
    if(b.action==='locate'){const i=env.rows.findIndex(row=>row.lineNumber>=b.lineNumber),index=i<0?env.rows.length:i;return {...id,queryId:b.queryId,lineNumber:b.lineNumber,found:env.rows[index]?.lineNumber===b.lineNumber,pending:false,index};}
    // Physical rows whatever the filter; the reply stops at the end.
    if(b.action==='lines'){const known=b.lineNumbers.filter(n=>n<=env.total);return {...id,queryId:b.queryId,rows:known.map(n=>env.all?.get(n)??raw(n)),next:known.length};}
    if(b.action==='range'||b.action==='record')return {...id,queryId:b.queryId,lineNumber:b.lineNumber,text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]};
    throw Error(b.action);
  }},event:{listen:async(name,fn)=>events.set(name,fn)}};
  const controller=mountLargeLogView(native,{document,window,transport:desktopTransport(native)});
  await settle();
  const key=event=>{let prevented=false;handlers.get('keydown')({target:new Element(),...event,preventDefault(){prevented=true;}});return prevented;};
  const runTimers=async ms=>{for(const [id,timer] of [...timers])if(timer.ms===ms){timers.delete(id);timer.fn();}await settle();};
  const row=line=>get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  const enter=async(id,value)=>{get(id).value=value;await get(id).dispatch('input');await get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  return {get,document,controller,calls,timers,env,key,runTimers,row,enter,emit:async payload=>{await events.get('source-changed')({payload});await settle();},invoked:action=>calls.filter(c=>c.action===action).map(c=>c.args),lines:()=>calls.filter(c=>c.args.request?.action==='lines').map(c=>c.args.request.lineNumbers)};
}
// A reload's source: the engine moves to a new dataset whose rows the test sets.
async function reopenAs(r,id,{rows=r.env.rows,total=rows.length,size=r.env.size,work={}}={}){
  Object.assign(r.env,{dataset:id,rows,total,size});
  await r.emit({status:'opened',datasetId:id,work:{reopenable:true,match:null,state:null,reloadOf:id-1,...work}});
  await r.emit({status:'validated',datasetId:id});
}
async function bookmark(r,line){await r.row(line).dispatch('click',{detail:1});await settle();r.key({key:'b',metaKey:true});await settle();}

test('the top bar reopens the same file keeping filters, search, view, lower tab, bookmarks and places by physical row',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal(r.get('reopenFile').hidden,false);assert.equal(r.get('reopenFile').disabled,false);
  assert.match(r.get('reopenFile').title,/다시 읽습니다.*\(Ctrl\+R\)$/);
  await r.enter('tagFilter','Net');await r.enter('searchInput','raw');
  await r.get('largeRaw').dispatch('click');
  await bookmark(r,3);await r.get('bookmarkTab').dispatch('click');
  r.controller.logs.seek(10);await settle();
  await r.row(13).dispatch('click',{detail:1});await settle();
  assert.equal(r.controller.logs.getState().viewportStart,10);assert.equal(r.controller.logs.getState().selectedLine,13);
  await r.get('reopenFile').dispatch('click');await settle();
  assert.deepEqual(r.invoked('reopen_source'),[{datasetId:1}],'only the dataset ID crosses to native code');
  assert.equal(r.get('reopenFile').disabled,true,'one reload at a time');
  // Rows 2–4 left the results: row 11, first on screen before, is now the 8th result.
  await reopenAs(r,2,{rows:r.env.rows.filter(row=>row.lineNumber<2||row.lineNumber>4),total:30,work:{match:'changed',state:{v:1,filters:{tag:'Old'}}}});
  assert.equal(r.get('tagFilter').value,'Net','the live work wins over saved work');assert.equal(r.get('searchInput').value,'raw');
  assert.equal(r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.datasetId===2&&c.args.request.pane==='logs').at(-1).args.request.filters.include[0].tag,'Net');
  assert.equal(r.get('largeRaw').attributes['aria-pressed'],'true');
  assert.equal(r.get('bookmarkTab').attributes['aria-selected'],'true');assert.equal(r.get('bookmarkTab').textContent,'북마크 1');
  const upper=r.controller.logs.getState();
  assert.equal(upper.datasetId,2);assert.equal(upper.viewportStart,7);assert.equal(upper.selectedLine,13);
  assert.deepEqual(r.lines(),[[3]],'bookmarked rows are read again from the reopened file');
  assert.equal(r.get('toast').textContent,RELOADED);assert.equal(r.get('reopenFile').disabled,false);
});

test('a reload drops bookmarks past the new end and reports rows whose text changed',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  for(const line of [3,8,25])await bookmark(r,line);
  const rows=Array.from({length:20},(_,i)=>raw(i+1));rows[7]={lineNumber:8,kind:'raw',raw:'rewritten'};
  r.env.all=new Map(rows.map(row=>[row.lineNumber,row]));
  await r.get('reopenFile').dispatch('click');await settle();
  await reopenAs(r,2,{rows,size:60});
  assert.equal(r.get('bookmarkTab').textContent,'북마크 2');
  assert.match(r.get('toast').textContent,/북마크 1개 해제·1개 내용 바뀜/);
  await r.get('bookmarkTab').dispatch('click');
  assert.deepEqual(r.get('bookmarkList').children.map(n=>[n.dataset.lineNumber,n.title]),[['3','raw 3'],['8','rewritten']]);
});

test('a shorter file without bookmarks is reported once its rows are read',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  await r.get('reopenFile').dispatch('click');await settle();
  await reopenAs(r,2,{rows:r.env.rows.slice(0,10),size:40});
  assert.match(r.get('toast').textContent,/이전보다 짧아졌습니다/);
});

test('a place past the end of a shorter file shows the list end; a row not read yet waits',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  r.controller.logs.seek(20);await settle();await r.row(28).dispatch('click',{detail:1});await settle();
  await r.get('reopenFile').dispatch('click');await settle();
  await reopenAs(r,2,{rows:r.env.rows.slice(0,12)});
  const upper=r.controller.logs.getState();
  assert.equal(upper.error,'');assert.equal(r.get('errorBar').hidden,true);
  assert.equal(upper.viewportStart,2,'the last screen of the 12 rows');assert.equal(upper.selectedLine,null,'a selection past the end is dropped');
  // Still indexing: the restore asks nothing until the row has been read.
  Object.assign(r.env,{dataset:3,complete:false,total:5,rows:Array.from({length:5},(_,i)=>raw(i+1))});
  await r.emit({status:'opened',datasetId:3,work:{reopenable:true}});
  void r.controller.logs.restorePosition(9,10);await settle();
  const locates=()=>r.calls.filter(c=>c.args.request?.action==='locate'&&c.args.request.datasetId===3).length;
  assert.equal(locates(),0);assert.equal(r.controller.logs.getState().error,'');
  Object.assign(r.env,{rows:Array.from({length:30},(_,i)=>raw(i+1)),total:30,complete:true});
  await new Promise(done=>setTimeout(done,250));await settle();
  assert.equal(locates(),1,'the next status poll resolves it');
  assert.equal(r.controller.logs.getState().viewportStart,8);assert.equal(r.controller.logs.getState().selectedLine,10);
});

test('pasted text cannot be reopened and Cmd/Ctrl+R never reloads the page',async t=>{
  const r=await rig({work:()=>({reopenable:false})});t.after(()=>r.controller.close());
  assert.equal(r.get('reopenFile').hidden,false);assert.equal(r.get('reopenFile').disabled,true);
  assert.match(r.get('reopenFile').title,/붙여넣은 텍스트와 샘플 로그는 다시 열 수 없습니다/);
  assert.equal(r.key({key:'r',metaKey:true}),true,'the WebView reload is prevented');
  assert.equal(r.key({key:'R',ctrlKey:true}),true);
  assert.deepEqual(r.invoked('reopen_source'),[]);
  await r.controller.close();assert.equal(r.get('reopenFile').hidden,true,'no file, no action');
  assert.equal(r.key({key:'r',metaKey:true}),true);
});

test('Cmd/Ctrl+R reopens a file that can be reopened',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal(r.key({key:'r',ctrlKey:true}),true);await settle();
  assert.deepEqual(r.invoked('reopen_source'),[{datasetId:1}]);
  assert.equal(r.key({key:'r',ctrlKey:true}),true);await settle();
  assert.equal(r.invoked('reopen_source').length,1,'a pending reload is not repeated');
});

test('a changed file offers 다시 열기 in the error banner',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal(r.get('errorBar').hidden,true);
  await r.emit({status:'error',error:'Source changed; reopen the file'});
  assert.equal(r.get('errorBar').hidden,false);assert.match(r.get('largeError').textContent,/변경.*다시 열어/);
  assert.equal(r.get('errorReopen').hidden,false);
  await r.get('errorReopen').dispatch('click');await settle();
  assert.deepEqual(r.invoked('reopen_source'),[{datasetId:1}]);
  assert.equal(r.get('errorReopen').hidden,true,'the action waits for the reload');
  await reopenAs(r,2);
  assert.equal(r.get('errorBar').hidden,true,'the reopened file clears the error');
  await r.emit({status:'error',error:'NUL is not supported; select a UTF-8 text log'});
  assert.equal(r.get('errorBar').hidden,false);assert.equal(r.get('errorReopen').hidden,true,'other errors have no reopen action');
  await r.emit({status:'error',error:'Source truncated'});assert.equal(r.get('errorReopen').hidden,false);
});

test('a failed reload keeps the open view and reports why',async t=>{
  const r=await rig({reopen:async()=>{throw 'Selected file was not found; select it again';}});t.after(()=>r.controller.close());
  await r.enter('tagFilter','Net');
  await r.get('reopenFile').dispatch('click');await settle();
  assert.match(r.get('largeError').textContent,/찾을 수 없습니다/);assert.equal(r.get('errorReopen').hidden,true);
  assert.equal(r.get('reopenFile').disabled,false,'the action is available again');assert.equal(r.get('tagFilter').value,'Net');
});

test('an invalid reopened file rolls back to the previous view with its work',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  await r.enter('tagFilter','Net');await bookmark(r,4);
  await r.get('reopenFile').dispatch('click');await settle();
  r.env.dataset=2;await r.emit({status:'opened',datasetId:2,work:{reopenable:true,reloadOf:1}});
  r.env.dataset=1;await r.emit({status:'rollback',datasetId:2,error:'NUL is not supported; select a UTF-8 text log'});
  assert.equal(r.controller.logs.getState().datasetId,1);assert.equal(r.get('tagFilter').value,'Net');
  assert.equal(r.get('bookmarkTab').textContent,'북마크 1');assert.equal(r.get('reopenFile').disabled,false);
  assert.ok(r.invoked('work_state').some(a=>a.datasetId===1),'the restored dataset is asked again');
});

const saved={v:1,filters:{levels:['V','D','I','W','E','F','A','U'],tag:'Net',excludeTags:'',timeFrom:'',timeTo:'',pid:'',tid:'',query:'raw',caseSensitive:false,includeRaw:true,recognizeLineNumbers:null,searchMode:'literal'},bookmarks:[3,8],upper:{top:11,selected:12},lower:{top:null,selected:null}};

test('the same files opened again restore their saved filters, search, bookmarks and place',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  Object.assign(r.env,{dataset:2,rows:r.env.rows.filter(row=>row.lineNumber%5)});
  await r.emit({status:'opened',datasetId:2,work:{reopenable:true,match:'exact',state:saved}});await r.emit({status:'validated',datasetId:2});
  assert.equal(r.get('tagFilter').value,'Net');assert.equal(r.get('searchInput').value,'raw');
  assert.deepEqual(r.lines(),[[3,8]]);assert.equal(r.get('bookmarkTab').textContent,'북마크 2');
  const upper=r.controller.logs.getState();
  assert.equal(upper.viewportStart,r.env.rows.findIndex(row=>row.lineNumber===11));assert.equal(upper.selectedLine,12);
  assert.equal(r.get('toast').textContent,'지난번 작업을 복원했습니다 · 필터 · 검색 · 북마크 2개 · 위치');
});

test('changed files restore only filters and search, and say so',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  r.env.dataset=2;await r.emit({status:'opened',datasetId:2,work:{reopenable:true,match:'changed',state:saved}});
  assert.equal(r.get('tagFilter').value,'Net');assert.equal(r.get('searchInput').value,'raw');
  assert.equal(r.get('lowerTabs').hidden,true,'no bookmarks');assert.deepEqual(r.lines(),[]);
  assert.equal(r.controller.logs.getState().viewportStart,0);
  assert.match(r.get('toast').textContent,/필터와 검색만 복원했습니다/);
  r.env.dataset=4;await r.emit({status:'opened',datasetId:4,work:{reopenable:false}});
  assert.equal(r.get('toast').hidden,true,'a new source drops the previous file\'s notice');
  // Saved filters the page would reject are not applied, not even in part.
  r.env.dataset=5;await r.emit({status:'opened',datasetId:5,work:{reopenable:true,match:'exact',state:{v:1,filters:{pid:'x1',tag:'Bad'}}}});
  assert.equal(r.get('tagFilter').value,'');assert.equal(r.get('pidFilter').value,'');
});

test('a command-line file restores its saved work once the page sees it',async t=>{
  const r=await rig({work:id=>id===1?{reopenable:true,match:'exact',state:{...saved,bookmarks:[],upper:{top:null,selected:null}}}:{}});t.after(()=>r.controller.close());
  assert.equal(r.get('tagFilter').value,'Net');
  assert.equal(r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters.include[0].tag,'Net');
  assert.equal(r.get('toast').textContent,'지난번 작업을 복원했습니다 · 필터 · 검색');
});

test('work is saved by dataset ID after changes, before another file opens and on close',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  await r.enter('tagFilter','Net');await r.runTimers(500);
  const first=r.invoked('save_work_state');
  assert.equal(first.length,1);assert.deepEqual(Object.keys(first[0]).sort(),['datasetId','state']);
  assert.equal(first[0].datasetId,1);assert.equal(first[0].state.filters.include[0].tag,'Net');assert.deepEqual(first[0].state.bookmarks,[]);
  assert.equal(JSON.stringify(first).includes('raw 1'),false,'no log text is saved');
  await bookmark(r,6);
  r.env.dataset=2;await r.emit({status:'opened',datasetId:2,work:{reopenable:true}});
  const flushed=r.invoked('save_work_state').at(-1);
  assert.equal(flushed.datasetId,1);assert.deepEqual(flushed.state.bookmarks,[6]);assert.deepEqual(flushed.state.upper,{top:null,selected:6});
  await r.enter('tagFilter','Wifi');await r.controller.close();
  const closed=r.invoked('save_work_state').at(-1);
  assert.equal(closed.datasetId,2);assert.equal(closed.state.filters.include[0].tag,'Wifi');
  // Default work with nothing to restore forgets the entry.
  const r2=await rig();t.after(()=>r2.controller.close());
  await r2.enter('tagFilter','x');await r2.runTimers(500);await r2.enter('tagFilter','');await r2.runTimers(500);
  assert.equal(r2.invoked('save_work_state').at(-1).state,null);
});

test('pasted text saves no work',async t=>{
  const r=await rig({work:()=>({reopenable:false})});t.after(()=>r.controller.close());
  await r.enter('tagFilter','Net');await r.runTimers(500);await r.controller.close();
  assert.deepEqual(r.invoked('save_work_state'),[]);
});

test('a saved filter applies to another file, stays through a reload and returns with saved work',async t=>{
  const nav={id:9,name:'NAV',format:'text',filter:{v:1,include:[{tag:'Net',levels:['W','E','F','A']}],exclude:[{tag:'chatty'}],includeRaw:true}};
  const r=await rig({savedFilters:[nav]});t.after(()=>r.controller.close());
  const label=()=>r.get('savedFilterButton').children.map(child=>child.hidden?'':child.textContent).join('|');
  assert.equal(label(),'저장된 필터|');
  await r.get('searchInput').dispatch('input');r.get('searchInput').value='raw';await r.get('searchInput').dispatch('input');await settle();
  await r.get('savedFilterButton').dispatch('click');await settle();
  const item=r.get('rowMenu').children.find(child=>child.textContent==='NAV');
  assert.equal(item.attributes.role,'menuitemradio');assert.equal(item.attributes['aria-checked'],'false');
  await item.dispatch('click');await settle();
  assert.equal(r.get('tagFilter').value,'Net');assert.equal(r.get('hideList').children.length,1);
  assert.equal(r.get('searchInput').value,'raw','the search stays');
  const query=r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  assert.deepEqual([query.include.map(c=>[c.tag,c.levels]),query.exclude.map(c=>c.tag)],[[['Net',['W','E','F','A']]],['chatty']]);
  assert.equal(label(),'NAV|');
  assert.equal(r.get('toast').children[0].textContent,"'NAV' 필터를 적용했습니다.");assert.equal(r.get('toast').children[1].textContent,'실행 취소');
  await r.runTimers(500);
  assert.equal(r.invoked('save_work_state').at(-1).state.savedFilter,9,'saved with the file\'s work');
  await r.enter('tagFilter','Net, Wifi');
  assert.equal(label(),'NAV|수정됨');assert.equal(r.get('saveFilter').hidden,false);
  // A reload keeps the filter it came from.
  await r.get('reopenFile').dispatch('click');await settle();await reopenAs(r,2);
  assert.equal(label(),'NAV|수정됨');assert.equal(r.get('tagFilter').value,'Net, Wifi');
  // Another file starts without a filter; its saved work brings the filter back.
  r.env.dataset=3;await r.emit({status:'opened',datasetId:3,work:{reopenable:true}});
  assert.equal(label(),'저장된 필터|');assert.equal(r.get('tagFilter').value,'');
  r.env.dataset=4;await r.emit({status:'opened',datasetId:4,work:{reopenable:true,match:'exact',state:{v:1,filters:{tag:'Net',excludeTags:'chatty',levels:['W','E','F','A']},savedFilter:9}}});
  assert.equal(label(),'NAV|');
  // 초기화 leaves the saved filter.
  await r.get('resetFilters').dispatch('click');await settle();
  assert.equal(label(),'저장된 필터|');
});
