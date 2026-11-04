import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as nextTurn} from 'node:timers/promises';
import {readFile} from 'node:fs/promises';
import {mountLargeLogView} from '../dist/large-view.js';
import {desktopTransport} from '../dist/desktop.js';

const deferred=()=>{let resolve;const promise=new Promise(done=>resolve=done);return {promise,resolve};};
async function settle(){for(let i=0;i<10;i++)await nextTurn();}
// The chip row's joining words and chips in reading order, through the groups
// that keep a joining word with its chip.
const chipNodes=row=>row.children.flatMap(node=>node.className==='chip-group'?node.children:[node]);
// A chip's text edits its condition and its × clears the field. A list item
// is the condition's switch, its kind and its summary.
const chipEdit=chip=>chip.children[0],chipClear=chip=>chip.children[1];
const listItems=r=>r.get('conditionList').children,itemKinds=r=>listItems(r).map(item=>item.children[1].textContent),itemSummaries=r=>listItems(r).map(item=>item.children[2].textContent);
// The top-bar menu's item called `label`, opened for the look of it or the click.
const moreItem=async(r,label)=>{await r.get('moreButton').dispatch('click');return r.get('rowMenu').children.find(n=>n.textContent===label||n.textContent.startsWith(`${label} `));};
const closeMenu=r=>r.get('rowMenu').dispatch('keydown',{key:'Escape',preventDefault(){},stopPropagation(){}});
const choose=async(r,label)=>{const item=await moreItem(r,label);assert.ok(item,label);await item.dispatch('click');await settle();};
// What the item offers now, with the menu closed again.
const moreState=async(r,label)=>{const item=await moreItem(r,label);await closeMenu(r);return item&&{disabled:item.attributes['aria-disabled']==='true',title:item.title,text:item.textContent};};
class Element {
  constructor(tag='div'){this.tagName=tag.toUpperCase();this.children=[];this.listeners={};this.dataset={};this.attributes={};this.value='';this.checked=false;this.hidden=false;this._text='';this.style={setProperty(key,value){this[key]=value;},removeProperty(key){delete this[key];}};this.clientHeight=280;this.scrollTop=0;this.classList={toggle(){},add:(...names)=>{this.className=[...new Set([...String(this.className??'').split(' ').filter(Boolean),...names])].join(' ');}};}
  get textContent(){return this._text+this.children.map(child=>child.textContent).join('');}
  set textContent(value){this._text=String(value);this.children=[];}
  append(...nodes){for(const part of nodes){const node=typeof part==='string'?Object.assign(new Element('span'),{_text:part}):part;if(node.fragment){this.append(...node.children);continue;}if(node.parentElement)node.parentElement.children=node.parentElement.children.filter(child=>child!==node);node.parentElement=this;this.children.push(node);}}
  replaceChildren(...nodes){this.children=[];this._text='';this.append(...nodes);}
  insertBefore(node,before){if(node.parentElement)node.parentElement.removeChild(node);const i=before?this.children.indexOf(before):this.children.length;this.children.splice(i,0,node);node.parentElement=this;}
  removeChild(node){this.children=this.children.filter(child=>child!==node);node.parentElement=null;}
  removeAttribute(key){delete this.attributes[key];}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  focus(){if(this.ownerDocument)this.ownerDocument.activeElement=this;}
  click(){return this.dispatch('click');}
  getBoundingClientRect(){return {left:0,top:0,right:0,bottom:0,width:0,height:0};}
  querySelectorAll(selector){return selector==='[role^=menuitem]'?this.children.filter(child=>child.attributes.role?.startsWith('menuitem')):[];}
  setAttribute(key,value){this.attributes[key]=String(value);}
  addEventListener(key,handler){(this.listeners[key]??=[]).push(handler);}
  dispatch(key,event={}){return Promise.all((this.listeners[key]??[]).map(fn=>fn(event)));}
  querySelector(selector){const line=selector.match(/^\[data-line-number="(\d+)"\]$/)?.[1];return this.children.find(child=>line?child.dataset.lineNumber===line:child.className===selector.slice(1));}
}
async function rig({rows=[],detail=null,query=null,page=null,startupError=null,files=null,total=6,format=null,exported=null,extrasList=null,capture=null}={}){
  let ownerDocument;const elements=new Map(),get=id=>{if(!elements.has(id))elements.set(id,Object.assign(new Element(),{ownerDocument}));return elements.get(id);};
  // Column layout has its own mounted tests; this rig exercises view lifecycle.
  delete get('workspace').style;
  new Element().append(get('searchInput'));new Element().append(get('caseSensitive'));get('includeRaw').checked=true;
  const handlers=new Map(),document={getElementById:get,createElement:tag=>new Element(tag),createDocumentFragment:()=>Object.assign(new Element(),{fragment:true}),querySelector:()=>new Element(),addEventListener:(key,fn)=>handlers.set(key,fn)};
  ownerDocument=document;
  document.createElement=tag=>Object.assign(new Element(tag),{ownerDocument:document});
  for(const element of elements.values())element.ownerDocument=document;
  const timers=new Map();let timerId=0;
  const clipboard=[],windowListeners=new Map();
  const window={setTimeout:(fn,ms)=>{const id=++timerId;timers.set(id,{fn,ms});return id;},clearTimeout:id=>timers.delete(id),addEventListener(name,fn){(windowListeners.get(name)??windowListeners.set(name,[]).get(name)).push(fn);},removeEventListener(name,fn){windowListeners.set(name,(windowListeners.get(name)??[]).filter(f=>f!==fn));},navigator:{clipboard:{writeText:async text=>{clipboard.push(text);}}}};
  const events=new Map(),calls=[],picker=deferred();let dataset=1,complete=true,cancel=async()=>({});
  const native={core:{invoke:async(action,args={})=>{
    calls.push({action,args});
    if(action==='pick_file')return picker.promise;
    if(action==='cancel_open')return cancel(args);
    if(action==='take_startup_error')return startupError;
    if(action==='export_results')return exported??{rows:3000};
    // Without `extrasList` this is a build without extensions, which refuses every request.
    if(action==='extras'){if(!extrasList)throw 'This build has no extensions';return args.request.action==='list'?extrasList:{};}
    if(action!=='log_request')return {};
    const b=args.request,id={datasetId:dataset};
    if(b.action==='close')return {closed:true};
    if(b.action==='status')return {...id,name:files?`${files[0].name} 외 ${files.length-1}개`:'synthetic',...(files?{files}:{}),...(format?{format}:{}),...(capture?{capture}:{}),size:100,total,textComplete:complete,summaryComplete:true,complete,status:complete?'ready':'indexing',summary:{counts:{I:3,W:1}},query:null};
    if(b.action==='query'&&query)return {...id,queryId:b.queryId,query:{id:b.queryId,...await query(b)}};
    if(b.action==='query'||b.action==='cancel')return {...id,queryId:b.queryId,query:{id:b.queryId,status:'complete',count:rows.length||6,complete:true}};
    if(b.action==='page')return {...id,queryId:b.queryId,start:b.start,count:rows.length||6,rows:page?await page(b):rows.slice(b.start,b.start+b.count)};
    // Rows are in physical order; a missing row's index counts the rows before it.
    if(b.action==='locate'){const i=rows.findIndex(row=>row.lineNumber>=b.lineNumber),index=i<0?rows.length:i;return {...id,queryId:b.queryId,lineNumber:b.lineNumber,found:rows[index]?.lineNumber===b.lineNumber,pending:false,index};}
    if((b.action==='range'||b.action==='record')&&detail)return {...id,queryId:b.queryId,lineNumber:b.lineNumber,...(typeof detail==='function'?await detail(b):detail)};
    throw Error(b.action);
  }},event:{listen:async(name,fn)=>events.set(name,fn)}};
  const controller=mountLargeLogView(native,{document,window,transport:desktopTransport(native)});
  await settle();
  return {get,document,controller,calls,picker,handlers,timers,clipboard,fireWindow:(name,event)=>{for(const fn of windowListeners.get(name)??[])fn(event);},source(id,done=false){dataset=id;complete=done;},onCancel(fn){cancel=fn;},emit:payload=>events.get('source-changed')({payload}),emitPrefix:payload=>events.get('native-text-prefix')({payload}),emitProgress:payload=>events.get('query-progress')({payload}),listener:name=>events.get(name)};
}

test('picker completion keeps cancellation available until candidate validation ends',async t=>{
  const r=await rig();t.after(()=>r.controller.close());await r.get('openFile').dispatch('click');r.source(2);
  await r.emit({status:'opened',datasetId:2});r.picker.resolve({datasetId:2});await settle();
  assert.equal(r.get('cancelInput').hidden,false);
  r.source(2,true);await r.emit({status:'validated',datasetId:2});
  assert.equal(r.get('cancelInput').hidden,true);await r.controller.close();
});

test('native file errors explain recovery without discarding the open dataset',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  await r.emit({status:'error',error:'NUL is not supported; select a UTF-8 text log'});
  assert.match(r.get('largeError').textContent,/로그.*선택/);
  assert.equal(r.get('largeError').hidden,false);assert.equal(r.controller.logs.getState().datasetId,1);
  await r.emit({status:'error',error:'Source changed; reopen the file'});
  assert.match(r.get('largeError').textContent,/변경.*다시 열어/);
  assert.equal(r.controller.logs.getState().datasetId,1);
  await r.controller.close();assert.equal(r.get('largeError').hidden,true);
});

test('stale source events cannot clear a newer rollback snapshot or replace its view',async t=>{
  const r=await rig();t.after(()=>r.controller.close());r.get('tagFilter').value='original-tag';r.get('pidFilter').value='007';
  r.source(2);await r.emit({status:'opened',datasetId:2});
  await r.emit({status:'validated',datasetId:1});
  r.source(3);await r.emit({status:'opened',datasetId:3});
  await r.emit({status:'rollback',datasetId:2,error:'stale failure'});
  await r.emit({status:'opened',datasetId:2});
  assert.equal(r.controller.logs.getState().datasetId,3);
  assert.equal(r.get('largeError').textContent,'');
  assert.equal(r.get('cancelInput').hidden,false);
  r.source(1,true);await r.emit({status:'rollback',datasetId:3,error:'invalid UTF-8'});
  assert.equal(r.get('tagFilter').value,'original-tag');assert.equal(r.get('pidFilter').value,'007');
  assert.equal(r.get('cancelInput').hidden,true);await r.controller.close();
});

test('opening cancel targets its dataset and a delayed reply cannot hide a newer candidate',async t=>{
  const r=await rig(),cancel=deferred();t.after(()=>r.controller.close());r.onCancel(()=>cancel.promise);
  r.source(2);await r.emit({status:'opened',datasetId:2});
  const pending=r.get('cancelInput').dispatch('click');await settle();
  assert.deepEqual(r.calls.find(c=>c.action==='cancel_open').args,{datasetId:2});
  r.source(3);await r.emit({status:'opened',datasetId:3});
  await r.emit({status:'rollback',datasetId:2,error:'Opening cancelled'});cancel.resolve({});await pending;
  assert.equal(r.get('cancelInput').hidden,false);assert.equal(r.controller.logs.getState().datasetId,3);
  r.source(3,true);await r.emit({status:'validated',datasetId:3});await r.controller.close();
});

// With a log open, a paste asks before it replaces the log.
const confirmPaste=async r=>{assert.equal(r.get('pasteDialog').open,true,'the paste asks first');await r.get('pasteForm').dispatch('submit',{preventDefault(){}});await settle();};
test('text paste sends leading BOM and CR/LF bytes without decoding them again',async t=>{
  const r=await rig(),text='\uFEFF한글 🙂\r\nraw\rnext\n';t.after(()=>r.controller.close());
  r.handlers.get('paste')({target:new Element(),clipboardData:{types:['text/plain'],files:[],items:[],getData:()=>text},preventDefault(){}});
  await settle();await confirmPaste(r);assert.equal(r.calls.find(c=>c.action==='open_text').args.text,text);await r.controller.close();
});

test('a paste over an open log asks first; cancel keeps it, a newer paste replaces the offer, and an empty view opens at once',async t=>{
  const r=await rig(),dialog=r.get('pasteDialog'),row=new Element();t.after(()=>r.controller.close());
  const paste=text=>{let prevented=false;r.handlers.get('paste')({target:new Element(),clipboardData:{getData:type=>type==='text/plain'?text:''},preventDefault(){prevented=true;}});return prevented;};
  const opened=()=>r.calls.filter(c=>c.action==='open_text').map(c=>c.args.text);
  row.ownerDocument=r.document;row.focus();
  assert.equal(paste('06-18 19:24:24.983  1 2 I Tag: one\n'),true);await settle();
  assert.equal(dialog.open,true);assert.deepEqual(opened(),[]);
  assert.equal(r.get('pasteWhat').textContent,'로그 텍스트 1줄 · 35 B');
  assert.equal(r.get('pasteNote').textContent,'열려 있는 로그는 닫히며 다시 열 수 없습니다.','work for this dataset is not reopenable');
  assert.equal(r.document.activeElement,r.get('pasteOpen'),'열기 has focus, so Enter opens');
  // The page's shortcuts wait while the dialog asks.
  const picks=()=>r.calls.filter(c=>c.action==='pick_file').length,key=(name,more={})=>r.handlers.get('keydown')({key:name,metaKey:true,ctrlKey:true,target:r.get('pasteOpen'),preventDefault(){},...more});
  key('o');key('r');key('b');key('F2',{metaKey:false,ctrlKey:false});await settle();
  assert.equal(picks(),0,'⌘O waits for the dialog');assert.equal(r.calls.some(c=>c.action==='reopen'||c.args?.request?.action==='reopen'),false);
  await r.get('pasteCancel').dispatch('click');await settle();
  assert.equal(dialog.open,false);assert.deepEqual(opened(),[]);assert.equal(r.document.activeElement,row,'focus returns');
  assert.equal(r.controller.logs.getState().datasetId,1);
  key('o');await settle();assert.equal(picks(),1,'⌘O works again once the dialog closes');
  // Escape and the backdrop also keep the log; a press inside the form does not close.
  paste('a\nb\n');await settle();await dialog.dispatch('click',{target:r.get('pasteForm')});assert.equal(dialog.open,true);
  let stopped=false;await dialog.dispatch('keydown',{key:'Escape',preventDefault(){},stopPropagation(){stopped=true;}});
  assert.equal(dialog.open,false);assert.equal(stopped,true);
  paste('a\nb\n');await settle();await dialog.dispatch('click',{target:dialog});assert.equal(dialog.open,false);assert.deepEqual(opened(),[]);
  // A second paste while asking replaces what the dialog offers; one opens.
  paste('first\nlog\n');await settle();paste('second\nlog\r\nthird');await settle();
  assert.equal(r.get('pasteWhat').textContent,'로그 텍스트 3줄 · 17 B');
  await confirmPaste(r);assert.deepEqual(opened(),['second\nlog\r\nthird']);
  // Text that is not a log keeps the dialog shut.
  assert.equal(paste('ordinary words'),false);await settle();assert.equal(dialog.open,false);
  await r.controller.close();
  paste('after\nclose\n');await settle();
  assert.equal(dialog.open,false);assert.deepEqual(opened().at(-1),'after\nclose\n','with nothing open a paste opens at once');
});

test('a copied file over an open log holds its grant while asking; declining drops it without pasting',async t=>{
  const r=await rig(),dialog=r.get('pasteDialog'),file=r.listener('native-file-paste');t.after(()=>r.controller.close());
  const finishes=()=>r.calls.filter(c=>['hold_file_paste','finish_file_paste'].includes(c.action)).map(c=>[c.action,c.args.grant,c.args.action]);
  const pending=file({payload:{grant:'native-paste-1',name:'synthetic device.log'}});await settle();
  assert.equal(dialog.open,true);assert.equal(r.get('pasteWhat').textContent,'파일 synthetic device.log');
  await r.get('pasteCancel').dispatch('click');await pending;await settle();
  assert.deepEqual(finishes(),[['hold_file_paste','native-paste-1',undefined],['finish_file_paste','native-paste-1','discard']]);
  const open=file({payload:{grant:'native-paste-2'}});await settle();
  assert.equal(r.get('pasteWhat').textContent,'복사한 파일');
  await r.get('pasteForm').dispatch('submit',{preventDefault(){}});await open;
  assert.deepEqual(finishes().slice(2),[['hold_file_paste','native-paste-2',undefined],['finish_file_paste','native-paste-2','open']]);
  // A text field keeps the standard Paste, without asking.
  r.document.activeElement={isContentEditable:true};
  await file({payload:{grant:'native-paste-3'}});
  assert.deepEqual(finishes().at(-1),['finish_file_paste','native-paste-3','paste']);assert.equal(dialog.open,false);
  r.document.activeElement=null;
  await r.controller.close();
  await file({payload:{grant:'native-paste-4'}});
  assert.deepEqual(finishes().at(-1),['finish_file_paste','native-paste-4','open'],'with nothing open the file opens at once');
  assert.equal(finishes().some(([action,grant])=>action==='hold_file_paste'&&grant==='native-paste-4'),false);
});

test('paste limit still counts UTF-8 bytes rather than JavaScript characters',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const text='한'.repeat(Math.floor(25*1024**2/3)+1)+'\n';
  r.handlers.get('paste')({target:new Element(),clipboardData:{types:['text/plain'],files:[],items:[],getData:()=>text},preventDefault(){}});
  await settle();assert.equal(r.calls.some(c=>c.action==='open_text'),false);
  assert.match(r.get('largeError').textContent,/25 MiB/);
});

test('native BOM metadata restores only markers stripped by the platform and is consumed once',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  for(const [count,present] of [[1,0],[1,1],[2,1],[2,2]]){
    await r.emitPrefix({grant:'prefix-'+count+'-'+present,count});
    r.handlers.get('paste')({target:new Element(),clipboardData:{getData:()=> '\uFEFF'.repeat(present)+'raw\r\n한글 🙂\n'},preventDefault(){}});
    await settle();await confirmPaste(r);assert.equal(r.calls.filter(c=>c.action==='open_text').at(-1).args.text,'\uFEFF'.repeat(count)+'raw\r\n한글 🙂\n');
  }
  r.handlers.get('paste')({target:new Element(),clipboardData:{getData:()=> 'ordinary\nlog\n'},preventDefault(){}});
  await settle();await confirmPaste(r);assert.equal(r.calls.filter(c=>c.action==='open_text').at(-1).args.text,'ordinary\nlog\n');
});

test('native BOM metadata keeps editable-target Paste standard',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const target={isContentEditable:true};r.document.activeElement=target;
  await r.emitPrefix({grant:'editable-prefix',count:1});
  let prevented=false;r.handlers.get('paste')({target,clipboardData:{getData:()=> 'raw\ntext\n'},preventDefault(){prevented=true;}});
  await settle();assert.equal(prevented,false);assert.equal(r.calls.some(c=>c.action==='open_text'),false);
  assert.ok(r.calls.some(c=>c.action==='finish_prefix_paste'&&c.args.grant==='editable-prefix'));
});

test('search-only reset and level selections retain completed level counts',async t=>{
  const r=await rig(),info=()=>r.get('levelFilters').children.find(b=>b.dataset.level==='I');t.after(()=>r.controller.close());
  assert.equal(info().querySelector('.count').textContent,'3');
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');
  await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const upperQueries=()=>r.calls.filter(c=>c.action==='log_request'&&c.args.request.action==='query'&&c.args.request.pane==='logs').length;
  const before=upperQueries();await r.get('resetFilters').dispatch('click');await settle();
  assert.equal(upperQueries(),before,'unchanged upper filter should not rescan');
  assert.equal(info().querySelector('.count').textContent,'3');
  assert.match(info().title,/^Info · 전체 파일의 레벨별 행 수 · Alt\+클릭/);
  const chip=level=>r.get('levelFilters').children.find(b=>b.dataset.level===level);
  assert.equal(chip('V').dataset.empty,'true','a complete summary dims levels without rows');assert.equal(info().dataset.empty,'false');
  await r.get('allLevels').dispatch('click');assert.equal(info().querySelector('.count').textContent,'3');
  await info().dispatch('click');assert.equal(info().querySelector('.count').textContent,'3');await r.controller.close();
});

test('mounted log heading keeps its full total and removes only the duplicate footer count',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal(r.get('logsCount').textContent,'로그 · 전체 6건');
  assert.equal(r.get('largeStatus').textContent,'');
  await r.controller.logs.setFilters({include:[{pid:'007'}]});await settle();
  assert.equal(r.get('logsCount').textContent,'로그 · 결과 6건');
  assert.equal(r.get('largeStatus').textContent,'파일 6행','equal filtered counts still convey a different scope');
  const state=r.controller.logs.getState();
  await r.emitProgress({pane:'logs',state:{datasetId:state.datasetId,status:'ready',size:100,total:6,textComplete:true,summaryComplete:true,complete:true,query:{id:state.queryId,status:'complete',count:6,complete:true}}});
  await new Promise(resolve=>setTimeout(resolve,5));
  assert.equal(r.get('largeStatus').textContent,'파일 6행');
});

test('rapid typing discards old results/detail once while one stable searching label covers debounce and replacement',async t=>{
  const late=deferred();let delayQuery=false;
  const rows=[{lineNumber:1,kind:'raw',raw:'COMMON old'}],detail={text:'COMMON old',startByte:0,nextByte:10,totalBytes:10,complete:true,highlights:[]};
  const r=await rig({rows,detail,query:()=>delayQuery?late.promise:{status:'complete',count:1,complete:true}});t.after(()=>r.controller.close());
  r.get('searchInput').value='old';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const row=r.get('largeRows').children[0];await row.dispatch('click');await settle();
  const old=r.controller.getState(),upperRow=r.get('logsRows').children[0];
  const root=r.get('largeDetailText').children[0];r.document.selection={anchorNode:root};
  const label=r.get('largeTotal'),descriptor=Object.getOwnPropertyDescriptor(Element.prototype,'textContent');let writes=0;
  Object.defineProperty(label,'textContent',{get(){return descriptor.get.call(this);},set(value){writes++;descriptor.set.call(this,value);}});
  const queries=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='search').length,before=queries();
  for(const value of ['C','CO','COMMON']){
    r.get('searchInput').value=value;await r.get('searchInput').dispatch('input');
    assert.equal(label.textContent,' · 검색 중');assert.equal(r.get('largeCount').textContent,'—');
    assert.equal(r.get('largeRows').children.length,0);assert.equal(r.get('largeDetailText').children.length,0);
    assert.equal(r.controller.getState().count,0);assert.equal(r.controller.getState().selectedLine,null);assert.equal(r.get('logsRows').children[0],upperRow);
    assert.equal(r.get('largeSearchPane').attributes['aria-busy'],'true');assert.equal(r.get('largeRange').textContent,'');
    assert.equal(r.get('largeDetailText').contains(r.document.selection.anchorNode),false);
  }
  assert.equal(writes,1,'repeated input does not rewrite identical status text');assert.equal(queries(),before);
  assert.equal([...r.timers.values()].filter(t=>t.ms===150).length,1);
  const pendingId=r.controller.getState().queryId;
  await r.emitProgress({pane:'search',state:{datasetId:old.datasetId,status:'ready',textComplete:true,summaryComplete:true,complete:true,total:6,query:{id:old.queryId,status:'complete',count:1,complete:true}}});await new Promise(resolve=>setTimeout(resolve,5));
  assert.equal(label.textContent,' · 검색 중');assert.equal(writes,1);assert.equal(r.get('largeRows').children.length,0);assert.equal(r.controller.getState().queryId,pendingId);
  delayQuery=true;const [id,timer]=[...r.timers].find(([,timer])=>timer.ms===150);r.timers.delete(id);timer.fn();await settle();
  assert.equal(queries(),before+1);assert.equal(label.textContent,' · 검색 중');assert.equal(writes,1);
  assert.equal(r.get('largeDetailText').children.length,0,'new query invalidates old detail');
  late.resolve({status:'complete',count:1,complete:true});await settle();
  assert.equal(label.textContent,' · 검색 완료');assert.equal(r.get('largeCount').textContent,'1건');assert.equal(writes,2);assert.equal(r.get('largeRows').children.length,1);
  await r.emitProgress({pane:'search',state:{datasetId:old.datasetId,query:{id:old.queryId,status:'running',count:99}}});await new Promise(resolve=>setTimeout(resolve,5));
  assert.equal(label.textContent,' · 검색 완료');assert.equal(r.get('largeCount').textContent,'1건');
});

test('compact mounted search distinguishes no-hit completion, empty input, cancellation and error',async t=>{
  const r=await rig({query:b=>b.filters.query==='none'?{status:'complete',count:0,complete:true}:b.filters.query==='fail'?{status:'error',count:0,complete:false,error:'Search failed; retry'}:{status:'complete',count:6,complete:true}});t.after(()=>r.controller.close());
  const enter=async value=>{r.get('searchInput').value=value;await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('none');assert.equal(r.get('largeTotal').textContent,' · 검색 완료');assert.equal(r.get('largeCount').textContent,'0건');
  assert.equal(r.get('largeSearchPane').attributes['aria-busy'],'false');
  await enter('');assert.equal(r.get('largeSearchPane').hidden,true);
  assert.equal(r.get('largeCancel').disabled,true,'an emptied search has nothing to cancel');assert.equal(r.get('largeSearchPane').attributes['aria-busy'],'false');
  await enter('again');await r.get('largeCancel').dispatch('click');await settle();
  assert.match(r.get('largeTotal').textContent,/취소됨.*부분 결과/);assert.doesNotMatch(r.get('largeTotal').textContent,/검색 완료/);
  await enter('fail');assert.match(r.get('largeTotal').textContent,/미완료.*부분 결과/);assert.doesNotMatch(r.get('largeTotal').textContent,/검색 완료/);
});


test('line-number selector sends auto/on/off to both panes, defaults for new files and restores on rollback',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const queries=()=>r.calls.filter(c=>c.action==='log_request'&&c.args.request.action==='query').map(c=>c.args.request);
  assert.equal(queries().at(-1).filters.recognizeLineNumbers,null);
  r.get('searchInput').value='body';
  for(const [value,mode] of [['off',false],['on',true],['auto',null]]) {
    r.get('recognizeLineNumbers').value=value;
    await r.get('recognizeLineNumbers').dispatch('change');await settle();
    for(const pane of ['logs','search']) assert.equal(queries().findLast(q=>q.pane===pane).filters.recognizeLineNumbers,mode);
  }
  r.get('recognizeLineNumbers').value='off';
  await r.get('recognizeLineNumbers').dispatch('change');await settle();
  r.source(2);await r.emit({status:'opened',datasetId:2});await settle();
  assert.equal(r.get('recognizeLineNumbers').value,'auto');
  r.source(1,true);await r.emit({status:'rollback',datasetId:2,error:'invalid UTF-8'});await settle();
  assert.equal(r.get('recognizeLineNumbers').value,'off');
  assert.equal(queries().findLast(q=>q.pane==='logs').filters.recognizeLineNumbers,false);
  await r.controller.close();
});

test('damaged byte marks survive columns, raw view, detail and normal replacement glyphs',async t=>{
  const row={lineNumber:1,kind:'parsed',raw:'Tag: � �',date:'10-08',time:'10:04:05.491',pid:'6202',tid:'6394',level:'I',tag:'Tag',message:'� �',encodingDamaged:true,encodingDamage:{mask:'80',bytes:1,samples:[{position:7,hex:'FF',sourceStartByte:92}]},fieldDamage:{message:{mask:'04',samples:[{position:2,hex:'FF',sourceStartByte:92}]}},highlights:[[7,8]]};
  const detail={text:row.raw,startByte:0,nextByte:10,totalBytes:10,complete:true,highlights:[[7,8]],encodingDamage:row.encodingDamage};
  const r=await rig({rows:[row],detail});t.after(()=>r.controller.close());
  const collect=(e,cls)=>[...(e.className===cls?[e]:[]),...e.children.flatMap(c=>collect(c,cls))];
  assert.equal(collect(r.get('logsRows'),'encoding-badge').length,1);assert.equal(collect(r.get('logsRows'),'encoding-damage').length,1);
  assert.equal(collect(r.get('logsRows'),'encoding-damage')[0].textContent,'�');
  await r.get('largeRaw').dispatch('click');assert.equal(collect(r.get('logsRows'),'encoding-damage').length,1);assert.match(r.get('logsRows').textContent,/Tag: � �/);
  await r.get('logsRows').children[0].dispatch('click');await settle();assert.match(r.get('largeDetailTitle').textContent,/손상/);assert.equal(r.get('largeDetailText').textContent,row.raw);assert.equal(collect(r.get('largeDetailText'),'encoding-damage').length,1);assert.match(collect(r.get('largeDetailText'),'encoding-damage')[0].title,/FF/);
});

test('detail title names repaired mojibake and clears the note for plain rows, its tooltip holding the whole title',async t=>{
  const row={lineNumber:1,kind:'raw',raw:'text : 안녕'};
  let detail={text:row.raw,startByte:0,nextByte:22,totalBytes:22,complete:true,highlights:[],encodingRepaired:true};
  const r=await rig({rows:[row],detail:()=>detail});t.after(()=>r.controller.close());
  await r.get('logsRows').children[0].dispatch('click');await settle();
  assert.match(r.get('largeDetailTitle').textContent,/깨진 문자 복원/);
  assert.equal(r.get('largeDetailTitle').title.split('\n')[0],r.get('largeDetailTitle').textContent);assert.match(r.get('largeDetailTitle').title,/\nMac OS Roman/);
  assert.equal(r.get('largeDetailText').textContent,row.raw);
  detail={...detail,text:'plain',encodingRepaired:undefined};
  await r.controller.logs.loadDetail(1);await settle();
  assert.doesNotMatch(r.get('largeDetailTitle').textContent,/복원/);assert.equal(r.get('largeDetailTitle').title,r.get('largeDetailTitle').textContent);
});

test('unchanged detail nodes retain selection while new content, damage, query and lifecycle invalidate',async t=>{
  const row={lineNumber:1,kind:'raw',raw:'a� �'};
  let detail={text:row.raw,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[[1,2]],encodingDamage:{mask:'02',bytes:1,samples:[{position:1,hex:'FF',sourceStartByte:1}]}};
  const r=await rig({rows:[row],detail:()=>detail});t.after(()=>r.controller.close());
  await r.controller.logs.loadDetail(1); // A row click establishes the owner.
  await r.get('logsRows').children[0].dispatch('click');await settle();
  const container=r.get('largeDetailText'),root=container.children[0],selected=root.children[0];
  r.document.selection={anchorNode:selected};
  r.controller.logs.seek(0);await settle();
  assert.equal(container.children[0],root);
  assert.ok(container.contains(r.document.selection.anchorNode),'existing text selection still belongs to retained DOM');
  const state=r.controller.logs.getState();
  await r.emitProgress({pane:'logs',state:{datasetId:state.datasetId,status:'ready',size:100,total:6,textComplete:true,summaryComplete:true,complete:true,query:{id:state.queryId,status:'complete',count:6,complete:true}}});
  await new Promise(resolve=>setTimeout(resolve,5));
  assert.equal(container.children[0],root,'unchanged native progress must not rebuild detail');
  detail={...detail,text:'changed �',highlights:[[0,7]],encodingDamage:{mask:'00',bytes:0,samples:[]}};
  await r.controller.logs.loadDetail(1);await settle();
  assert.notEqual(container.children[0],root);assert.equal(container.textContent,detail.text);
  assert.doesNotMatch(r.get('largeDetailTitle').textContent,/손상/);
  const updated=container.children[0];
  await r.controller.logs.setFilters({query:'changed'});await settle();
  assert.equal(container.children.length,0);assert.equal(r.get('largeDetail').hidden,true);
  await r.controller.logs.loadDetail(1);await settle();assert.notEqual(container.children[0],updated);
  r.controller.logs.closeDetail();assert.equal(container.children.length,0);
  await r.controller.close();r.source(2,true);await r.emit({status:'opened',datasetId:2});await settle();
  assert.equal(container.children.length,0);assert.equal(r.get('largeDetail').hidden,true);
});

test('full 64KiB damaged detail and genuine replacement glyph remain visible without new limits',async t=>{
  const text='a�'.repeat(16383)+'�x';
  const mask='aa'.repeat(4095)+'2a';
  const detail={text,startByte:0,nextByte:32768,totalBytes:32768,complete:true,highlights:[],encodingDamage:{mask,bytes:16383,samples:[]}};
  const r=await rig({rows:[{lineNumber:1,kind:'raw',raw:'a�'}],detail});t.after(()=>r.controller.close());
  await r.get('logsRows').children[0].dispatch('click');await settle();
  assert.equal(new TextEncoder().encode(text).length,65536);
  assert.equal(r.get('largeDetailText').textContent,text);
  const root=r.get('largeDetailText').children[0];
  assert.equal(root.children.filter(c=>c.className==='encoding-damage').length,16383);
  assert.equal(root.children.at(-1).textContent,'�x','genuine U+FFFD remains unmarked');
  r.controller.logs.seek(0);assert.equal(r.get('largeDetailText').children[0],root);
  await r.controller.close();assert.equal(r.get('largeDetailText').children.length,0);
});

test('equal detail content from a different pane or physical row receives its own DOM identity',async t=>{
  const rows=[1,2].map(lineNumber=>({lineNumber,kind:'raw',raw:'same'}));
  const detail={text:'same',startByte:0,nextByte:4,totalBytes:4,complete:true,highlights:[]};
  const r=await rig({rows,detail});t.after(()=>r.controller.close());
  await r.get('logsRows').children[0].dispatch('click');await settle();
  const first=r.get('largeDetailText').children[0];
  await r.get('largeRows').children[0].dispatch('click');await settle();
  const otherPane=r.get('largeDetailText').children[0];assert.notEqual(otherPane,first);
  await r.get('largeRows').children[1].dispatch('click');await settle();
  assert.notEqual(r.get('largeDetailText').children[0],otherPane);
  assert.match(r.get('largeDetailTitle').textContent,/2행/);
});

test('an obsolete detail response after Close or source replacement cannot recreate retained content',async t=>{
  const late=deferred();let pending=false;
  const r=await rig({rows:[{lineNumber:1,kind:'raw',raw:'old'}],detail:()=>pending?late.promise:{text:'old',startByte:0,nextByte:3,totalBytes:3,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  await r.get('logsRows').children[0].dispatch('click');await settle();
  pending=true;const loading=r.controller.logs.loadDetail(1);await settle();
  await r.controller.close();r.source(2,true);await r.emit({status:'opened',datasetId:2});await settle();
  late.resolve({text:'stale',startByte:0,nextByte:5,totalBytes:5,complete:true,highlights:[]});await loading;await settle();
  assert.equal(r.get('largeDetailText').children.length,0);assert.equal(r.get('largeDetail').hidden,true);
});

test('native field offsets highlight repeated fields and messages beyond the raw preview',async t=>{
  const message='COMMON '+ '🙂'.repeat(4090);
  const raw='10-08 10:04:05.491 007 007 I 007: '+message;
  const row={lineNumber:1,kind:'parsed',raw:raw.slice(0,4096),date:'10-08',time:'10:04:05.491',pid:'007',tid:'007',level:'I',tag:'007',message,fieldOffsets:{pid:19,tid:23,tag:29,message:34},highlights:[[19,22],[23,26],[29,32],[34,40]]};
  // Build offsets from the exact synthetic layout, like the native page.
  row.fieldOffsets={pid:raw.indexOf('007'),tid:raw.indexOf('007',raw.indexOf('007')+1),tag:raw.indexOf('007:'),message:raw.indexOf(message)};
  row.highlights=Object.entries(row.fieldOffsets).map(([key,at])=>[at,at+(key==='message'?6:3)]);
  const r=await rig({rows:[row]});t.after(()=>r.controller.close());
  const collect=(e,cls)=>[...(e.className===cls?[e]:[]),...e.children.flatMap(c=>collect(c,cls))];
  for(const field of ['pid','tid','tag','message']) {
    const cell=collect(r.get('logsRows'),field)[0];
    const marks=e=>[...(e.tagName==='MARK'?[e.textContent]:[]),...e.children.flatMap(marks)];
    assert.deepEqual(marks(cell),[field==='message'?'COMMON':'007'],field);
  }
});

test('native optional-tag iOS rows render process and IDs without inventing a tag',async t=>{
  const {rows}=JSON.parse(await readFile(new URL('./ios-optional-tag-page.json',import.meta.url),'utf8'));
  const r=await rig({rows,detail:b=>{const row=rows.find(row=>row.lineNumber===b.lineNumber);return {text:row.raw,startByte:0,nextByte:row.rawBytes,totalBytes:row.rawBytes,complete:true,highlights:row.highlights,encodingDamage:row.encodingDamage};}});t.after(()=>r.controller.close());
  const cells=line=>r.get('logsRows').children.find(row=>row.dataset.lineNumber===String(line));
  const cell=(line,cls)=>cells(line).children.find(child=>String(child.className).split(' ').includes(cls));
  const marks=e=>[...(e.tagName==='MARK'?[e.textContent]:[]),...e.children.flatMap(marks)];
  // An iOS row's process and the end of its tag stay in view.
  assert.equal(cell(3,'timestamp').className,'timestamp apple-stamp');
  assert.equal(cell(3,'tag').className,'tag from-end');
  for(const line of [3,4,5,7]){
    assert.doesNotMatch(cells(line).className,/raw-row/);
    assert.equal(cell(line,'pid').textContent,'3870');
    assert.match(cell(line,'timestamp').textContent,/2026-09-15 10:47:35\.(134|184) · Runner/);
    assert.ok(marks(cell(line,'timestamp')).includes('Runner'));
    assert.equal(cell(line,'tag').textContent,line===3?'Memory':'');
    assert.match(cells(line).textContent,/Unknown/);
    if(line!==3){assert.equal(cell(line,'tid').textContent,'481490');assert.ok(marks(cell(line,'message')).includes('Shader'));}
  }
  assert.equal(cell(4,'message').textContent,'Shader cache pool loaded: 0x1004e7a10');
  await cells(7).dispatch('click');await settle();
  assert.equal(r.get('largeDetailText').textContent,rows[3].raw);
  await r.get('largeRaw').dispatch('click');
  assert.equal(cells(4).children.find(child=>child.className==='raw-content').textContent,rows[1].raw);
});

test('native Android time, uid, Android Studio and brief rows render their fields',async t=>{
  const {rows}=JSON.parse(await readFile(new URL('./android-formats-page.json',import.meta.url),'utf8'));
  const r=await rig({rows});t.after(()=>r.controller.close());
  const cells=line=>r.get('logsRows').children.find(row=>row.dataset.lineNumber===String(line));
  const cell=(line,cls)=>cells(line).children.find(child=>child.className===cls);
  const marks=e=>[...(e.tagName==='MARK'?[e.textContent]:[]),...e.children.flatMap(marks)];
  const ids=line=>['pid','tid','tag'].map(cls=>cell(line,cls).textContent);
  // Android Studio's package follows the timestamp, as an iOS process name does.
  for(const [line,level,fields] of [[6,'W',['2983','4241','AlertsService']],[16,'V',['4321','4321','알림🙂']]]){
    assert.match(cells(line).className,new RegExp(`level-row-${level}`));
    assert.match(cell(line,'timestamp').textContent,/^2026-06-18 19:24:0[38]\.000 · com\.example\.app$/);
    assert.match(cell(line,'timestamp').title,/ · com\.example\.app$/);
    assert.deepEqual(marks(cell(line,'timestamp')),['com.example'],'the match lands on the package');
    assert.deepEqual(ids(line),fields);
  }
  assert.equal(cell(2,'timestamp').textContent,'06-18 19:24:01.123');
  assert.deepEqual(ids(2),['2983','','AlertsService']);
  assert.deepEqual(marks(cell(2,'message')),['Sending']);
  assert.deepEqual(ids(9),['2983','4241','AlertsService']);
  // A brief row has no timestamp.
  assert.equal(cell(10,'timestamp').textContent.trim(),'');
  assert.deepEqual(ids(10),['1000','','ActivityManager']);
  assert.match(cells(10).textContent,/Info/);
  for(const [line,level] of [[5,'E'],[7,'W'],[11,'I']])assert.match(cells(line).className,new RegExp(`continuation-row level-row-${level}`));
});

test('viewport resizing and keyboard travel retain visible row nodes and focused pane',async t=>{
  const rows=Array.from({length:1000},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`row ${i+1}`}));
  const r=await rig({rows});t.after(()=>r.controller.close());
  const first=r.get('logsRows').children[0];
  r.get('logsViewport').clientHeight=140;r.controller.logs.closeDetail();await settle();
  assert.equal(r.get('logsRows').children[0],first,'detail resize must keep the clicked row node');
  const viewport=r.get('logsViewport');viewport.ownerDocument=r.document;viewport.scrollTop=200*28;
  await viewport.dispatch('scroll');await settle();
  const focused=r.get('logsRows').children.find(n=>n.dataset.lineNumber==='201');focused.focus();
  await viewport.dispatch('keydown',{key:'ArrowDown',target:focused,preventDefault(){}});await settle();
  assert.equal(r.get('logsRows').children.find(n=>n.dataset.lineNumber==='201'),focused);
  assert.equal(r.document.activeElement,focused);
  // When a jump removes the focused row, subsequent keys still belong to this pane.
  r.controller.logs.seek(800);await settle();
  assert.equal(r.document.activeElement,viewport);
});

test('mounted continuous panes fetch their first page once',async t=>{
  const rows=Array.from({length:300},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`COMMON ${i+1}`}));
  const r=await rig({rows});t.after(()=>r.controller.close());
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');
  await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  for(const pane of ['logs','search']) {
    const queryId=(pane==='logs'?r.controller.logs:r.controller).getState().queryId;
    const firstPages=r.calls.filter(c=>c.action==='log_request'&&c.args.request.action==='page'&&c.args.request.pane===pane&&c.args.request.queryId===queryId&&c.args.request.start===0);
    // The upper rows are read once more to mark the accepted search's results.
    for(const marks of new Set(firstPages.map(c=>c.args.request.searchQueryId)))assert.equal(firstPages.filter(c=>c.args.request.searchQueryId===marks).length,1,`${pane} ${marks}`);
    assert.equal(firstPages.length,pane==='logs'?2:1,pane);
  }
});

test('search double-click keeps the first-clicked row when the detail shifts layout',async t=>{
  const rows=Array.from({length:300},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`COMMON ${i+1}`}));
  const detail={text:'COMMON',startByte:0,nextByte:6,totalBytes:6,complete:true,highlights:[]};
  const r=await rig({rows,detail});t.after(()=>r.controller.close());
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');
  await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const panel=r.get('largeLogPanel'),row=line=>r.get('largeRows').children.find(n=>n.dataset.lineNumber===String(line));
  const ranges=()=>r.calls.filter(c=>c.args.request?.action==='record').map(c=>c.args.request.lineNumber);
  // A single click already follows in the upper logs, so each double-click
  // shows as a further locate of the row it jumps to.
  const locates=()=>r.calls.filter(c=>c.args.request?.action==='locate'&&c.args.request.pane==='logs').map(c=>c.args.request.lineNumber);
  // Each click is a press and a release; capture-phase panel listeners run
  // before the row target's, as in the DOM.
  const click=async(detailCount,target)=>{for(const type of ['mousedown','click']){await panel.dispatch(type,{detail:detailCount});await target?.dispatch(type,{detail:detailCount,button:0});}};
  await click(1,row(5));await settle();
  await click(2,row(7));await panel.dispatch('dblclick');
  assert.equal(r.controller.logs.getState().selectedLine,5,'second click on a shifted row');
  assert.deepEqual(locates(),[5,5]);
  assert.deepEqual(ranges(),[5],'second click must not replace the detail');
  await settle();
  await click(1,row(9));await click(2,null);await panel.dispatch('dblclick');
  assert.equal(r.controller.logs.getState().selectedLine,9,'second click on the detail panel');
  assert.deepEqual(locates(),[5,5,9,9]);
  await settle();
  // The detail may open under the held first press, so its release lands
  // off the row and only the press names it.
  await panel.dispatch('mousedown',{detail:1});await row(13).dispatch('mousedown',{detail:1,button:0});await panel.dispatch('click',{detail:1});
  await click(2,row(15));await panel.dispatch('dblclick');
  assert.equal(r.controller.logs.getState().selectedLine,13,'a release off the pressed row');
  assert.deepEqual(locates(),[5,5,9,9,13,13]);
  await settle();
  // Double-clicking detail text to select a word must not reuse an older row.
  await click(1,row(11));await settle();await click(1,null);await click(2,null);await panel.dispatch('dblclick');
  assert.equal(r.controller.logs.getState().selectedLine,11);assert.deepEqual(locates(),[5,5,9,9,13,13,11]);
  const upper=r.get('logsRows').children.find(n=>n.dataset.lineNumber==='11');await upper.dispatch('click',{detail:1});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,11);assert.equal(upper.attributes['aria-current'],'true');
  assert.equal(r.get('logsRows').children.find(n=>n.dataset.lineNumber==='9').attributes['aria-current'],undefined);
});


for(const [prefix,ownerName] of [['logs','logs'],['large','search']])test(`${prefix} row clicks/arrows select by displayed index, preserve focus and update detail beyond row200`,async t=>{
  const rows=Array.from({length:430},(_,i)=>({lineNumber:i*3+7,kind:'raw',raw:`COMMON ${i*3+7}`}));
  const r=await rig({rows,detail:b=>({text:`line ${b.lineNumber}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]})});t.after(()=>r.controller.close());
  if(prefix==='large'){r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();}
  const owner=ownerName==='logs'?r.controller.logs:r.controller,viewport=r.get(prefix+'Viewport'),container=r.get(prefix+'Rows');
  const click=async index=>{const node=container.children.find(n=>n.attributes['aria-posinset']===String(index+1));assert.ok(node,index);await node.dispatch('click',{detail:1});await settle();};
  await click(4);assert.equal(owner.getState().selectedLine,19);assert.equal(container.children.find(n=>n.dataset.lineNumber==='19').attributes['aria-current'],'true');
  const key=async name=>{await viewport.dispatch('keydown',{key:name,target:r.document.activeElement,preventDefault(){}});await settle();};
  await key('ArrowDown');assert.equal(owner.getState().selectedLine,22);assert.equal(r.get('largeDetailText').textContent,'line 22');
  assert.equal(r.document.activeElement.dataset.lineNumber,'22');assert.equal(container.children.find(n=>n.dataset.lineNumber==='19').attributes['aria-current'],undefined);
  viewport.scrollTop=199*28;await viewport.dispatch('scroll');await settle();await click(199);
  await key('ArrowDown');await key('ArrowDown');assert.equal(owner.getState().selectedLine,610);await key('ArrowUp');assert.equal(owner.getState().selectedLine,607);
  viewport.scrollTop=0;await viewport.dispatch('scroll');await settle();await click(0);await key('ArrowUp');assert.equal(owner.getState().selectedLine,7);
  viewport.scrollTop=429*28;await viewport.dispatch('scroll');await settle();await click(429);await key('ArrowDown');assert.equal(owner.getState().selectedLine,1294);
  const target={isContentEditable:true};await viewport.dispatch('keydown',{key:'ArrowDown',target,preventDefault(){throw Error('editable arrow intercepted');}});assert.equal(owner.getState().selectedLine,1294);
  await owner.setFilters({query:'new'});await settle();assert.equal(owner.getState().selectedLine,null);assert.equal(r.get('largeDetail').hidden,true);
  await r.controller.close();assert.equal(owner.getState().selectedLine,null);
});


for(const prefix of ['logs','large'])test(`${prefix} delayed selection pages keep editable focus and latest detail ranges win`,async t=>{
  const rows=Array.from({length:430},(_,i)=>({lineNumber:i*3+7,kind:'raw',raw:`COMMON ${i}`})),pending=deferred(),ranges=[];
  const r=await rig({rows,page:b=>b.start>=200?pending.promise:rows.slice(b.start,b.start+b.count),detail:b=>{const d=deferred();ranges.push({line:b.lineNumber,...d});return d.promise;}});t.after(()=>r.controller.close());
  if(prefix==='large'){r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();}
  const owner=prefix==='logs'?r.controller.logs:r.controller,viewport=r.get(prefix+'Viewport');
  await r.get(prefix+'Rows').children[0].dispatch('click',{detail:1});await settle();
  await viewport.dispatch('keydown',{key:'ArrowDown',preventDefault(){}});await settle();
  for(const d of [...ranges].reverse())d.resolve({text:`line ${d.line}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]});await settle();
  assert.equal(r.get('largeDetailText').textContent,'line 10','late prior detail cannot overwrite active selection');
  viewport.scrollTop=199*28;await viewport.dispatch('scroll');await settle();
  await r.get(prefix+'Rows').children.find(n=>n.attributes['aria-posinset']==='200').dispatch('click',{detail:1});await settle();
  await viewport.dispatch('keydown',{key:'ArrowDown',preventDefault(){}});await settle();
  const input=r.get('searchInput');input.focus();pending.resolve(rows.slice(200,400));await settle();
  assert.equal(owner.getState().selectedLine,607);assert.equal(r.document.activeElement,input,'page arrival must not steal typing focus');
  for(const d of ranges)d.resolve({text:`line ${d.line}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]});await settle();assert.equal(r.get('largeDetailText').textContent,'line 607');
});


test('new search intent rejects delayed old pages/detail/events before debounce and empty input resets only the lower pane',async t=>{
  const names=new Map(),latePage=deferred(),lateDetail=deferred();let delayed=false;
  const oldRows=[{lineNumber:3,kind:'raw',raw:'old result'}],newRows=[{lineNumber:9,kind:'raw',raw:'new result'}];
  const r=await rig({rows:oldRows,query:b=>{names.set(b.queryId,b.filters.query);return {status:'complete',count:1,complete:true};},page:b=>delayed&&b.pane==='search'&&names.get(b.queryId)==='old'?latePage.promise:names.get(b.queryId)==='new'?newRows:oldRows,detail:b=>delayed?lateDetail.promise:{text:'old detail',startByte:0,nextByte:10,totalBytes:10,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const enter=async value=>{r.get('searchInput').value=value;await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('old');await r.get('largeRows').children[0].dispatch('click',{detail:1});await settle();
  const old=r.controller.getState(),upper=r.get('logsRows').children[0];delayed=true;
  const page=r.controller.fetchRows(0).catch(()=>null),detail=r.controller.loadDetail(3);await settle();
  const container=r.get('largeRows'),remove=container.removeChild;let removals=0;container.removeChild=function(node){removals++;return remove.call(this,node);};
  r.get('searchInput').value='n';await r.get('searchInput').dispatch('input');
  const pendingId=r.controller.getState().queryId;assert.notEqual(pendingId,old.queryId);
  assert.equal(container.children.length,0);assert.equal(r.get('largeDetail').hidden,true);assert.equal(r.controller.getState().count,0);assert.equal(r.controller.getState().selectedLine,null);
  for(const value of ['ne','new']){r.get('searchInput').value=value;await r.get('searchInput').dispatch('input');assert.equal(r.controller.getState().queryId,pendingId);}
  assert.equal(removals,1,'already-empty pending results are not cleared again for each character');
  latePage.resolve(oldRows);lateDetail.resolve({text:'obsolete detail',startByte:0,nextByte:15,totalBytes:15,complete:true,highlights:[]});await Promise.all([page,detail]);
  await r.emitProgress({pane:'search',state:{datasetId:old.datasetId,query:{id:old.queryId,status:'complete',count:999,complete:true}}});await settle();
  assert.equal(container.children.length,0);assert.equal(r.controller.getState().count,0);assert.equal(r.get('largeDetail').hidden,true);assert.equal(r.get('largeTotal').textContent,' · 검색 중');
  delayed=false;await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.equal(container.children[0].dataset.lineNumber,'9');assert.match(container.textContent,/new result/);assert.doesNotMatch(container.textContent,/old result/);assert.equal(r.get('largeTotal').textContent,' · 검색 완료');
  await container.children[0].dispatch('click',{detail:1});await settle();
  r.get('searchInput').value='';await r.get('searchInput').dispatch('input');
  assert.equal(r.get('largeSearchPane').hidden,true);assert.equal(container.children.length,0);assert.equal(r.controller.getState().count,0);assert.equal(r.controller.getState().selectedLine,null);assert.equal(r.get('largeDetail').hidden,true);assert.equal(r.get('logsRows').children[0],upper);
});


test('discarding lower search results preserves the upper filter query, selection and owned detail',async t=>{
  const r=await rig({rows:[{lineNumber:7,kind:'raw',raw:'COMMON'}],detail:{text:'upper detail',startByte:0,nextByte:12,totalBytes:12,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  r.get('pidFilter').value='007';r.get('searchInput').value='old';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const row=r.get('logsRows').children[0];await row.dispatch('click',{detail:1});await settle();
  const upper=r.controller.logs.getState(),root=r.get('largeDetailText').children[0],queries=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').length,before=queries();
  r.get('searchInput').value='new';await r.get('searchInput').dispatch('input');
  assert.equal(r.get('largeRows').children.length,0);assert.equal(r.controller.logs.getState().queryId,upper.queryId);assert.equal(r.controller.logs.getState().selectedLine,7);assert.equal(r.get('logsRows').children[0],row);assert.equal(r.get('largeDetailText').children[0],root);assert.equal(r.get('largeDetail').hidden,false);assert.equal(r.get('pidFilter').value,'007');
  await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();assert.equal(queries(),before);assert.equal(r.get('largeDetailText').children[0],root);
  r.get('searchInput').value='';await r.get('searchInput').dispatch('input');assert.equal(r.get('largeSearchPane').hidden,true);assert.equal(r.get('largeDetailText').children[0],root);
});

test('held arrow keys read detail only where the selection settles',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`row ${i+1}`}));
  const r=await rig({rows,detail:b=>({text:`line ${b.lineNumber}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]})});t.after(()=>r.controller.close());
  const viewport=r.get('logsViewport'),ranges=()=>r.calls.filter(c=>c.args.request?.action==='record').map(c=>c.args.request.lineNumber);
  await r.get('logsRows').children[0].dispatch('click',{detail:1});await settle();assert.deepEqual(ranges(),[1]);
  const key=async repeat=>{await viewport.dispatch('keydown',{key:'ArrowDown',repeat,target:r.document.activeElement,preventDefault(){}});await settle();};
  await key(false);assert.deepEqual(ranges(),[1,2],'a single press reads immediately');
  for(let i=0;i<5;i++)await key(true);
  assert.equal(r.controller.logs.getState().selectedLine,7);assert.deepEqual(ranges(),[1,2]);
  assert.equal(r.get('largeDetailText').textContent,'원문 구간을 읽는 중…');
  await new Promise(resolve=>setTimeout(resolve,200));await settle();
  assert.deepEqual(ranges(),[1,2,7]);assert.equal(r.get('largeDetailText').textContent,'line 7');
});

test('applied field filters stay visible as removable chips and a rail badge',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal(r.get('filterBadge').hidden,true);assert.equal(chipNodes(r.get('activeFilters')).length,0);
  r.get('tagFilter').value='  Wifi ';await r.get('tagFilter').dispatch('input');await r.get('tagFilter').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.equal(r.get('filterBadge').textContent,'1');assert.equal(r.get('filterBadge').hidden,false);
  assert.equal(chipNodes(r.get('activeFilters'))[0].textContent,'태그: Wifi×');
  assert.match(r.get('toggleFilters').attributes['aria-label'],/1개 조건/);
  await chipClear(chipNodes(r.get('activeFilters'))[0]).dispatch('click');await settle();
  assert.equal(r.get('tagFilter').value,'');assert.equal(r.get('filterBadge').hidden,true);
  assert.equal(r.controller.logs.getState().fieldFiltering,false,'removing the chip reapplies the upper query');
});

test('copy supplies raw text for a shift-selected row range and leaves text selections alone',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw line ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  await row(3).dispatch('click',{detail:1});await settle();await row(5).dispatch('click',{detail:1,shiftKey:true});await settle();
  assert.equal(r.get('logsSelection').textContent.startsWith('3행 선택'),true);
  assert.match(row(4).className,/in-selection/);assert.equal(row(4).tabIndex,-1,'rows are not separate tab stops');
  let data=null,prevented=false;
  r.handlers.get('copy')({target:new Element(),clipboardData:{setData:(type,value)=>{data=[type,value];}},preventDefault(){prevented=true;}});
  assert.equal(prevented,true);assert.deepEqual(data,['text/plain','raw line 3\nraw line 4\nraw line 5']);
  assert.match(r.get('toast').textContent,/3행/);
  await row(9).dispatch('click',{detail:1});await settle();
  assert.equal(r.get('logsSelection').textContent,'','a plain click collapses the range');
  r.get('searchInput').focus();prevented=false;
  r.handlers.get('copy')({target:new Element(),clipboardData:{setData(){throw Error('outside the lists, copy stays native');}},preventDefault(){prevented=true;}});
  assert.equal(prevented,false);
});

// Pointer gestures over a list whose box starts at the top of the window.
function gestures(r,list,rowsOf){
  list.getBoundingClientRect=()=>({top:0,bottom:280});
  const node=line=>rowsOf().find(n=>n.dataset.lineNumber===String(line));
  const y=line=>rowsOf().indexOf(node(line))*28+14-(list.scrollTop??0);
  const press=async(line,keys={})=>{let prevented=false;await node(line).dispatch('mousedown',{button:0,detail:1,clientY:y(line),preventDefault(){prevented=true;},...keys});await settle();return prevented;};
  const frame=async()=>{for(const [id,timer] of [...r.timers])if(timer.ms===16){r.timers.delete(id);timer.fn();}await settle();};
  const move=async clientY=>{r.fireWindow('pointermove',{clientY,buttons:1});await frame();};
  const up=async clientY=>{r.fireWindow('pointerup',{type:'pointerup',clientY});await settle();};
  const marked=()=>rowsOf().filter(n=>/in-selection/.test(n.className)).map(n=>Number(n.dataset.lineNumber));
  return {node,y,press,frame,move,up,marked};
}
const copyNow=r=>{let data=null;r.handlers.get('copy')({target:new Element(),clipboardData:{setData:(type,value)=>{data=value;}},preventDefault(){}});return data;};

test('a drag selects a row range, Ctrl+click adds and removes single rows, and actions take the selection in list order',async t=>{
  const rows=Array.from({length:40},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw line ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const viewport=r.get('logsViewport'),g=gestures(r,viewport,()=>r.get('logsRows').children),state=()=>r.controller.logs.getState();
  assert.equal(await g.press(3),true,'a press on another row selects rows, not text');assert.equal(state().selectedLine,3);
  await g.move(g.y(6));
  assert.equal(state().selectedLine,6,'the focus follows the pointer');assert.deepEqual(g.marked(),[3,4,5,6]);
  assert.equal(r.get('logsSelection').textContent,'4행 선택 · Ctrl+C 복사');
  await g.up(g.y(6));
  await g.node(6).dispatch('click',{detail:1});await settle();assert.deepEqual(g.marked(),[3,4,5,6],'the click after a press changes nothing more');
  await g.press(9,{ctrlKey:true});await g.up(g.y(9));
  assert.deepEqual(g.marked(),[3,4,5,6,9]);assert.equal(state().selectedLine,9,'an added row becomes the focus');
  await g.press(4,{ctrlKey:true});await g.up(g.y(4));
  assert.deepEqual(g.marked(),[3,5,6,9],'Ctrl+click takes a row out');assert.equal(state().selectedLine,9,'and keeps the focus');
  assert.equal(r.get('logsSelection').textContent,'4행 선택 · Ctrl+C 복사');
  assert.equal(copyNow(r),'raw line 3\nraw line 5\nraw line 6\nraw line 9','copy keeps list order and raw rows');
  await g.node(5).dispatch('contextmenu',{preventDefault(){},clientX:1,clientY:1});await settle();
  assert.deepEqual(g.marked(),[3,5,6,9],'right-click inside the selection keeps it');
  assert.ok(r.get('rowMenu').textContent.includes('선택한 4행 복사'),r.get('rowMenu').textContent);
  r.handlers.get('keydown')({key:'b',ctrlKey:true,target:new Element(),preventDefault(){}});await settle();
  assert.equal(r.get('bookmarkTab').textContent,'북마크 4');
  // Shift+click replaces the selection with the span from the anchor.
  await g.press(12,{shiftKey:true});await g.up(g.y(12));assert.deepEqual(g.marked(),[9,10,11,12]);
  const key=async(name,shiftKey=false)=>{await viewport.dispatch('keydown',{key:name,shiftKey,target:r.document.activeElement,preventDefault(){}});await settle();};
  await key('ArrowDown',true);assert.deepEqual(g.marked(),[9,10,11,12,13],'Shift+↓ extends');
  await key('ArrowDown');assert.deepEqual(g.marked(),[],'↓ collapses to one row');assert.equal(state().selectedLine,14);
  // Only the one selected row leaves a press to the browser's text selection.
  assert.equal(await g.press(14),false,'the selected row keeps its text selectable');
  await g.node(14).dispatch('click',{detail:1});await settle();assert.equal(state().selectedLine,14,'its click still selects it');
  await g.press(16,{shiftKey:true});await g.up(g.y(16));
  assert.equal(await g.press(15),true,'a row inside a multi-row selection starts a row gesture');await g.up(g.y(15));
  assert.deepEqual(g.marked(),[],'and the press selects that row alone');assert.equal(state().selectedLine,15);
  assert.equal(await g.press(15,{button:2}),false,'other buttons are left to the context menu');
  // A new query clears the selection.
  await g.press(18);await g.move(g.y(16));await g.up(g.y(16));assert.deepEqual(g.marked(),[16,17,18],'a drag upward selects too');
  await r.controller.logs.setFilters({include:[{pid:'1'}]});await settle();
  assert.equal(r.get('logsSelection').textContent,'');assert.equal(copyNow(r),null,'nothing selected, nothing copied as rows');
});

test('a drag past the list edge keeps scrolling and selects rows not read yet',async t=>{
  const rows=Array.from({length:1000},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`row ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const viewport=r.get('logsViewport'),g=gestures(r,viewport,()=>r.get('logsRows').children),state=()=>r.controller.logs.getState();
  await g.press(2);r.fireWindow('pointermove',{clientY:600,buttons:1});
  for(let i=0;i<60;i++)await g.frame();
  const focus=state().selectedLine;
  assert.ok(focus>200,`the selection reaches rows beyond the first page (${focus})`);
  assert.equal(r.get('logsSelection').textContent,`${focus-1}행 선택 · Ctrl+C 복사`);
  r.fireWindow('pointermove',{clientY:150,buttons:1});const reached=state().selectedLine;for(let i=0;i<3;i++)await g.frame();
  assert.ok(state().selectedLine<reached,'back inside the list the focus follows the pointer');
  const end=state().selectedLine;await g.up(150);for(let i=0;i<3;i++)await g.frame();
  assert.equal(state().selectedLine,end,'scrolling stops with the release');
  const copied=copyNow(r)??await (async()=>{for(let i=0;i<20&&!r.clipboard.length;i++)await settle();return r.clipboard.at(-1);})();
  assert.equal(copied,Array.from({length:end-1},(_,i)=>`row ${i+2}`).join('\n'));
});

test('Ctrl+A selects every search result found so far, counted with + while the search runs',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i*2+1,kind:'raw',raw:`hit ${i}`}));
  const r=await rig({rows,query:()=>({status:'running',count:30,complete:false})});t.after(()=>r.controller.close());
  r.get('searchInput').value='hit';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const key=(target,extra={})=>{let prevented=false;r.handlers.get('keydown')({key:'a',ctrlKey:true,target,preventDefault(){prevented=true;},...extra});return prevented;};
  const lower=r.get('largeViewport'),marked=()=>r.get('largeRows').children.filter(n=>/in-selection/.test(n.className)).length;
  lower.scrollTop=5*28;await lower.dispatch('scroll');await settle();
  lower.focus();assert.equal(key(lower),true);await settle();
  assert.equal(r.get('largeSelection').textContent,'30+행 선택 · Ctrl+C 복사','a running search marks its count partial');assert.equal(marked(),r.get('largeRows').children.length);
  assert.equal(r.controller.getState().selectedLine,11,'with no focus the top result in view becomes it');assert.equal(lower.scrollTop,5*28,'the list does not scroll');
  assert.equal(copyNow(r),rows.map(row=>row.raw).join('\n'));
  const state=r.controller.getState();
  await r.emitProgress({pane:'search',state:{datasetId:state.datasetId,status:'ready',size:100,total:60,textComplete:true,summaryComplete:true,complete:true,query:{id:state.queryId,status:'complete',count:30,complete:true}}});
  await new Promise(resolve=>setTimeout(resolve,5));await settle();
  assert.equal(r.get('largeSelection').textContent,'30행 선택 · Ctrl+C 복사');
  // The upper logs keep their selection, and text fields keep native Select All.
  const upper=r.get('logsViewport');await r.get('logsRows').children[2].dispatch('click',{detail:1});await settle();upper.focus();
  assert.equal(key(upper),true,'no page-wide text selection');assert.equal(r.get('logsSelection').textContent,'');
  const field=r.get('searchInput');field.focus();assert.equal(key(field,{}),false,'a text field selects its text');
  // A new search clears the selection.
  r.get('searchInput').value='other';await r.get('searchInput').dispatch('input');await settle();assert.equal(r.get('largeSelection').textContent,'');
});

test('bookmarking rows read page by page stops when the search changes between pages',async t=>{
  const rows=Array.from({length:600},(_,i)=>({lineNumber:i*2+1,kind:'raw',raw:`hit ${i}`})),later=deferred();
  const r=await rig({rows,page:async b=>{if(b.start>=200)await later.promise;return rows.slice(b.start,b.start+b.count);}});t.after(()=>r.controller.close());
  const search=async text=>{r.get('searchInput').value=text;await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await search('hit');
  const lower=r.get('largeViewport');lower.focus();
  r.handlers.get('keydown')({key:'a',ctrlKey:true,target:lower,preventDefault(){}});await settle();
  r.handlers.get('keydown')({key:'b',ctrlKey:true,target:lower,preventDefault(){}});await settle();
  // The search changes while a later page is still being read.
  await search('other');later.resolve();for(let i=0;i<5;i++)await settle();
  assert.equal(r.get('lowerTabs').hidden,true,'no rows of either search are bookmarked');
  assert.match(r.get('toast').textContent,/선택한 행을 읽지 못했습니다/);
});

test('the bookmark list selects like the other lists; Ctrl+C, Ctrl+A and Delete take its selection',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const upper=gestures(r,r.get('logsViewport'),()=>r.get('logsRows').children);
  for(const line of [4,8,12,2,16]){await upper.press(line,{ctrlKey:true});await upper.up(upper.y(line));}
  r.handlers.get('keydown')({key:'b',ctrlKey:true,target:new Element(),preventDefault(){}});await settle();
  assert.equal(r.get('bookmarkTab').textContent,'북마크 5','a discontiguous selection is bookmarked at once');
  await r.get('bookmarkTab').dispatch('click');await settle();
  const list=r.get('bookmarkList'),g=gestures(r,list,()=>list.children),listed=()=>list.children.map(n=>Number(n.dataset.lineNumber));
  assert.deepEqual(listed(),[2,4,8,12,16]);
  await g.press(4);await g.up(g.y(4));
  assert.equal(r.controller.logs.getState().selectedLine,4,'a press reveals its row above once released');
  await g.press(12,{shiftKey:true});await g.up(g.y(12));assert.deepEqual(g.marked(),[4,8,12]);
  assert.equal(r.get('bookmarkSelection').textContent,'3행 선택 · Ctrl+C 복사');
  assert.equal(r.controller.logs.getState().selectedLine,12,'the focus row is revealed');
  await g.press(2,{ctrlKey:true});await g.up(g.y(2));assert.deepEqual(g.marked(),[2,4,8,12]);
  assert.equal(r.document.activeElement,g.node(2),'the focus row takes focus');
  assert.equal(copyNow(r),'raw 2\nraw 4\nraw 8\nraw 12');
  await g.press(4);await g.move(g.y(8));await g.up(g.y(8));assert.deepEqual(g.marked(),[4,8],'a drag selects a range');
  const keyOn=async(node,key,extra={})=>{await node.dispatch('keydown',{key,target:node,preventDefault(){},...extra});await settle();};
  await keyOn(g.node(8),'ArrowDown',{shiftKey:true});assert.deepEqual(g.marked(),[4,8,12],'Shift+↓ extends');
  let scrolled=true;await g.node(12).dispatch('keydown',{key:'Home',target:g.node(12),preventDefault(){scrolled=false;}});await settle();
  assert.equal(scrolled,true,'Home without Shift is left to the list to scroll');assert.deepEqual(g.marked(),[4,8,12],'and keeps the selection');
  await keyOn(g.node(12),'Home',{shiftKey:true});assert.deepEqual(g.marked(),[2,4],'Shift+Home extends from the anchor to the first bookmark');
  await g.press(4);await g.move(g.y(8));await g.up(g.y(8));
  await keyOn(g.node(8),'ArrowDown',{shiftKey:true});assert.deepEqual(g.marked(),[4,8,12]);
  let prevented=false;r.handlers.get('keydown')({key:'a',ctrlKey:true,target:r.document.activeElement,preventDefault(){prevented=true;}});await settle();
  assert.equal(prevented,true);assert.deepEqual(g.marked(),[2,4,8,12,16]);assert.equal(r.get('bookmarkSelection').textContent,'5행 선택 · Ctrl+C 복사');
  await g.press(16,{ctrlKey:true});await g.up(g.y(16));assert.deepEqual(g.marked(),[2,4,8,12]);
  await keyOn(r.document.activeElement,'Delete');
  assert.deepEqual(listed(),[16],'Delete unmarks every selected bookmark');assert.match(r.get('toast').textContent,/북마크 4개를 해제했습니다/);
  assert.equal(r.document.activeElement,list.children[0],'focus stays in the list');
  assert.equal(r.get('bookmarkTab').textContent,'북마크 1');
});

test('invalid IDs are explained beside the fields and the empty state follows the source',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal(r.get('emptyState').hidden,true);assert.equal(r.get('logContent').hidden,false);
  r.get('pidFilter').value='12a';await r.get('pidFilter').dispatch('input');await r.get('pidFilter').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.equal(r.get('pidFilterError').hidden,false);assert.equal(r.get('pidFilter').attributes['aria-invalid'],'true');assert.equal(r.get('tidFilter').attributes['aria-invalid'],'false');
  assert.equal(r.get('largeError').hidden,true,'an open editor shows the error inline only');
  r.get('pidFilter').value='12';await r.get('pidFilter').dispatch('input');await r.get('pidFilter').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.equal(r.get('pidFilterError').hidden,true);
  await choose(r,'파일 닫기');
  assert.equal(r.get('emptyState').hidden,false);assert.equal(r.get('logContent').hidden,true);
});

test('a field takes one value or a pattern, validated beside it and sent as typed',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const check=async(id,on)=>{r.get(id).checked=on;await r.get(id).dispatch('change');await r.get(id.replace('Regex','Filter')).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const upper=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  const shown=()=>upper().include[0]??{};
  await enter('pidFilter','2983');await enter('tidFilter','41');await enter('tagFilter','ActivityManager');
  assert.deepEqual([shown().pid,shown().tid,shown().tag],['2983','41','ActivityManager']);
  assert.deepEqual(chipNodes(r.get('activeFilters')).map(chip=>chip.textContent),['태그: ActivityManager×','PID: 2983×','TID: 41×']);
  assert.equal(r.get('filterBadge').textContent,'1','the badge counts conditions, not fields');
  // A typed list is no value: the field says what to do instead.
  await enter('pidFilter','2983, 3120');
  assert.equal(r.get('pidFilterError').hidden,false);assert.match(r.get('pidFilterError').textContent,/여러 값은 정규식으로/);
  assert.equal(r.get('pidFilter').attributes['aria-invalid'],'true');assert.equal(r.get('tidFilter').attributes['aria-invalid'],'false');
  assert.equal(shown().pid,'2983','invalid input keeps the last applied value');assert.equal(r.get('logsPending').hidden,false);
  await check('pidRegex',true);await enter('pidFilter','2983|3120');
  assert.equal(r.get('pidFilterError').hidden,true);assert.deepEqual([shown().pid,shown().pidRegex],['2983|3120',true]);
  assert.equal(chipNodes(r.get('activeFilters'))[1].textContent,'PID 정규식: 2983|3120×');
  // A comma in a tag is text, and the field says so until a pattern reads it.
  await enter('tagFilter','ActivityManager, MyApp');
  assert.equal(r.get('tagFilterHint').hidden,false);assert.equal(r.get('tagFilterHint').children[0].textContent,'쉼표도 글자입니다. 여러 값은 정규식으로 찾습니다.');
  assert.equal(shown().tag,'ActivityManager, MyApp');
  await check('tagRegex',true);assert.equal(r.get('tagFilterHint').hidden,true);assert.equal(shown().tagRegex,true);
  // A clipped chip keeps all of its value in its tooltip.
  const long=Array.from({length:32},(_,i)=>1000+i).join('|');
  await enter('pidFilter',long);
  const pid=chipNodes(r.get('activeFilters'))[1];
  assert.match(pid.textContent,/…×$/);assert.match(chipEdit(pid).title,/1031 · 클릭하면 편집합니다$/);
  // A hiding condition is edited in the same fields.
  await enter('pidFilter','');await check('pidRegex',false);await enter('tidFilter','');await check('tagRegex',false);await enter('tagFilter','NAV');
  await r.get('addCondition').dispatch('click');await settle();
  assert.equal(r.get('tagFilter').value,'','a new condition starts empty');
  await r.get('kindHide').dispatch('click');await settle();
  await enter('tagFilter','chatty');
  assert.deepEqual(upper().exclude.map(c=>c.tag),['chatty']);
  assert.deepEqual(chipNodes(r.get('activeFilters')).map(chip=>chip.textContent),['태그: NAV×','제외','태그: chatty×'],'the chips read as the filter');
  await chipClear(chipNodes(r.get('activeFilters')).find(chip=>chip.textContent==='태그: NAV×')).dispatch('click');await settle();
  assert.deepEqual(upper().include,[],'removing the chip clears its field');assert.equal(r.get('tagFilter').value,'chatty','the hiding condition stays in the fields');
  // A condition left without criteria goes.
  await chipClear(chipNodes(r.get('activeFilters')).find(chip=>chip.textContent==='태그: chatty×')).dispatch('click');await settle();
  assert.deepEqual(upper().exclude,[]);assert.equal(chipNodes(r.get('activeFilters')).length,0);
  await enter('tagFilter','  ');await enter('pidFilter','  ');
  assert.equal(chipNodes(r.get('activeFilters')).length,0);assert.equal(r.controller.logs.getState().fieldFiltering,false,'spaces alone filter nothing');
});

test('conditions are a list sharing one editor; switched off and removed conditions are not sent',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const upper=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1)?.args.request.filters;
  const close=async()=>{await r.get('closeEditor').dispatch('click');await settle();};
  await close();
  assert.equal(listItems(r).length,0,'a blank condition is no item');assert.equal(r.get('conditionsEmpty').hidden,false);
  assert.equal(r.get('duplicateCondition').disabled,true);assert.equal(r.get('removeCondition').disabled,true);
  await r.get('addCondition').dispatch('click');await settle();
  assert.equal(r.get('filterEditor').hidden,false);assert.equal(r.get('filterEditorTitle').textContent,'새 조건');
  assert.deepEqual(itemSummaries(r),['새 조건'],'the condition being made shows');assert.equal(r.document.activeElement,r.get('tagFilter'));
  await enter('tagFilter','NAV');await close();
  assert.equal(r.get('filterEditor').hidden,true);assert.deepEqual(itemSummaries(r),['태그: NAV']);
  await r.get('addCondition').dispatch('click');await settle();
  assert.equal(r.get('tagFilter').value,'','a new condition starts empty');
  await r.get('errorLevels').dispatch('click');await settle();
  assert.deepEqual(upper().include.map(c=>[c.tag,c.levels.join('')]),[['NAV','VDIWEFAU'],['','WEFA']]);
  assert.deepEqual(itemSummaries(r),['태그: NAV','레벨: Warn 이상']);
  assert.deepEqual(chipNodes(r.get('activeFilters')).map(chip=>chip.textContent),['태그: NAV×','또는','레벨: Warn 이상×']);
  // Switched off: kept in the list, not sent, no chip.
  const toggle=listItems(r)[0].children[0];toggle.checked=false;await toggle.dispatch('change');await settle();
  assert.equal(listItems(r)[0].dataset.off,'true');
  assert.deepEqual(upper().include.map(c=>c.levels.join('')),['WEFA']);
  assert.deepEqual(chipNodes(r.get('activeFilters')).map(chip=>chip.textContent),['레벨: Warn 이상×']);
  // Selecting an item edits it.
  await listItems(r)[0].children[2].dispatch('click');await settle();
  assert.equal(r.get('filterEditorTitle').textContent,'조건 편집');assert.equal(r.get('tagFilter').value,'NAV');
  assert.equal(r.get('levelFilters').children.find(b=>b.dataset.level==='V').attributes['aria-pressed'],'true');
  // A chip's text edits its condition too; its kind can change in place.
  await chipEdit(chipNodes(r.get('activeFilters'))[0]).dispatch('click');await settle();
  assert.equal(r.get('filterEditor').hidden,false);assert.equal(r.get('tagFilter').value,'');
  assert.equal(r.get('kindHide').attributes['aria-checked'],'false');
  await r.get('kindHide').dispatch('click');await settle();
  assert.equal(r.get('kindHide').attributes['aria-checked'],'true');assert.deepEqual(itemKinds(r),['보기','제외']);
  assert.deepEqual(upper().exclude.map(c=>c.levels.join('')),['WEFA']);assert.deepEqual(upper().include,[]);
  await r.get('kindShow').dispatch('click');await settle();
  assert.deepEqual(itemKinds(r),['보기','보기']);assert.deepEqual(upper().exclude,[]);
  // Duplicating edits the copy.
  await r.get('duplicateCondition').dispatch('click');await settle();
  assert.deepEqual(itemSummaries(r),['태그: NAV','레벨: Warn 이상','레벨: Warn 이상']);
  await r.get('errorLevels').dispatch('click');await settle();
  await r.get('removeCondition').dispatch('click');await settle();
  assert.deepEqual(itemSummaries(r),['태그: NAV','레벨: Warn 이상']);assert.equal(r.get('filterEditor').hidden,true,'removing the edited condition closes the editor');
  // Removing the other item leaves only the switched-off one: nothing is sent.
  await listItems(r)[1].children[2].dispatch('click');await settle();
  await r.get('removeCondition').dispatch('click');await settle();
  assert.equal(listItems(r).length,1);assert.deepEqual(upper().include,[]);assert.equal(r.controller.logs.getState().fieldFiltering,false);
  const again=listItems(r)[0].children[0];assert.equal(again.checked,false,'a rebuilt item keeps its switch');again.checked=true;await again.dispatch('change');await settle();
  assert.deepEqual(upper().include.map(c=>c.tag),['NAV']);
  // Conditions are bounded.
  for(let i=1;i<32;i++){await r.get('addCondition').dispatch('click');await enter('pidFilter',String(i));await close();}
  assert.equal(listItems(r).length,32);assert.equal(r.get('addCondition').disabled,true);assert.equal(r.get('duplicateCondition').disabled,true);
  await r.get('addCondition').dispatch('click');await settle();
  assert.equal(listItems(r).length,32);assert.match(r.get('toast').textContent,/32개까지/);
  // 초기화 leaves no condition and clears the search; its toast puts every
  // condition and the search back.
  r.get('searchInput').value='needle';
  await r.get('resetFilters').dispatch('click');await settle();
  assert.equal(listItems(r).length,0);assert.equal(r.get('tagFilter').value,'');assert.equal(r.get('searchInput').value,'');
  const toast=r.get('toast');assert.equal(toast.children[0].textContent,'필터를 초기화했습니다.');assert.equal(toast.children[1].textContent,'실행 취소');
  await toast.children[1].dispatch('click');await settle();
  assert.equal(listItems(r).length,32);assert.equal(r.get('tagFilter').value,'NAV');assert.equal(r.get('searchInput').value,'needle');
  assert.deepEqual(upper().include[0].tag,'NAV');
  // Resetting a filter that is already empty offers nothing to undo.
  await r.get('resetFilters').dispatch('click');await settle();
  toast.replaceChildren();await r.get('resetFilters').dispatch('click');await settle();
  assert.equal(toast.children.length,0);
});

test('the editor shows the fields of the log format',async t=>{
  const shown=r=>['ecu','apid','ctid','tag','pid','tid','message'].filter(key=>!r.get(`${key}Row`).hidden);
  const text=await rig();t.after(()=>text.controller.close());
  assert.deepEqual(shown(text),['tag','pid','tid','message']);
  const dlt=await rig({format:'dlt'});t.after(()=>dlt.controller.close());
  assert.deepEqual(shown(dlt),['ecu','apid','ctid','message'],'a DLT log has no PID or TID, and no tag of its own');
  assert.equal(dlt.get('messageLabel').textContent,'페이로드');
  assert.equal(dlt.get('tagLabel').textContent,'모든 ID');
});

test('row menu adds removable conditions without changing existing ones',async t=>{
  const rows=[{lineNumber:1,kind:'parsed',raw:'06-18 19:24:00.000  100 200 I Choreographer: x',date:'06-18',time:'19:24:00.000',pid:'100',tid:'200',level:'I',tag:'Choreographer',message:'x'},{lineNumber:2,kind:'raw',raw:'plain'},
    {lineNumber:3,kind:'parsed',raw:'06-18 19:24:00.100  100 200 W Binder: y',date:'06-18',time:'19:24:00.100',pid:'100',tid:'200',level:'W',tag:'Binder',message:'y'}];
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  await r.get('closeEditor').dispatch('click');await settle();
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  const menu=()=>r.get('rowMenu').children.filter(n=>n.tagName==='BUTTON').map(n=>n.textContent);
  await row(1).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  assert.equal(r.get('rowMenu').hidden,false);
  assert.deepEqual(menu(),['이 행으로 조건 추가…',"태그 'Choreographer' 보기 조건 추가","태그 'Choreographer' 제외","메시지 'x' 보기 조건 추가","메시지 'x' 제외",'PID 100 보기 조건 추가','TID 200 보기 조건 추가','06-18 19:24:00.000부터 보기 조건 추가','06-18 19:24:00.000까지 보기 조건 추가','Info 레벨 보기 조건 추가','행 원문 복사','북마크 추가']);
  const title=label=>r.get('rowMenu').children.find(n=>n.textContent===label).title;
  assert.equal(title('PID 100 보기 조건 추가'),'이 조건에 맞는 줄만 보여 줍니다','no showing condition is in use yet');
  assert.equal(r.controller.logs.getState().selectedLine,1,'right-click selects the row');
  await r.get('rowMenu').children.find(n=>n.textContent==="태그 'Choreographer' 제외").dispatch('click');await settle();
  assert.equal(r.get('rowMenu').hidden,true);assert.deepEqual(itemKinds(r),['제외']);
  assert.equal(r.get('tagFilter').value,'','the showing condition stays in the fields');
  assert.deepEqual(chipNodes(r.get('activeFilters')).map(chip=>chip.textContent),['제외','태그: Choreographer×']);
  const query=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request;
  assert.deepEqual(query().filters.exclude.map(c=>c.tag),['Choreographer']);assert.equal(r.controller.logs.getState().fieldFiltering,true);
  // Each row action says what it changed in a toast that undoes it.
  const toast=r.get('toast'),undo=async()=>{await toast.children[1].dispatch('click');await settle();};
  assert.equal(toast.children[0].textContent,'제외 조건을 추가했습니다 · 태그: Choreographer');assert.equal(toast.children[1].textContent,'실행 취소');
  // Another excluded tag is a condition of its own; one already excluded
  // changes nothing.
  await row(3).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  await r.get('rowMenu').children.find(n=>n.textContent==="태그 'Binder' 제외").dispatch('click');await settle();
  assert.deepEqual(query().filters.exclude.map(c=>c.tag),['Choreographer','Binder']);
  assert.equal(toast.children[0].textContent,'제외 조건을 추가했습니다 · 태그: Binder');
  await undo();assert.deepEqual(query().filters.exclude.map(c=>c.tag),['Choreographer']);
  toast.replaceChildren();
  await row(1).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  await r.get('rowMenu').children.find(n=>n.textContent==="태그 'Choreographer' 제외").dispatch('click');await settle();
  assert.equal(toast.children.length,0);assert.deepEqual(itemKinds(r),['제외']);
  await row(1).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  await r.get('rowMenu').children.find(n=>n.textContent==="TID 200 보기 조건 추가").dispatch('click');await settle();
  assert.deepEqual(query().filters.include.map(c=>c.tid),['200'],'the blank showing condition takes the value');
  assert.deepEqual(itemKinds(r),['보기','제외']);assert.equal(toast.children[0].textContent,'보기 조건을 추가했습니다 · TID: 200');
  await row(1).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  await r.get('rowMenu').children.find(n=>n.textContent==="태그 'Choreographer' 보기 조건 추가").dispatch('click');await settle();
  assert.deepEqual(query().filters.include.map(c=>[c.tag,c.tid]),[['','200'],['Choreographer','']],'a new showing condition joins with 또는');
  assert.deepEqual(itemKinds(r),['보기','보기','제외']);assert.equal(r.get('tagFilter').value,'Choreographer','the new condition is selected');
  assert.equal(toast.children[0].textContent,'보기 조건을 추가했습니다 · 태그: Choreographer · 기존 보기 조건과 이 조건 중 하나라도 맞으면 보입니다');
  await row(1).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  assert.equal(title('PID 100 보기 조건 추가'),'기존 보기 조건에 더해, 이 조건에 맞는 줄도 함께 보여 줍니다');
  await r.get('rowMenu').children.find(n=>n.textContent==='PID 100 보기 조건 추가').dispatch('click');await settle();
  assert.deepEqual(query().filters.include.map(c=>[c.tag,c.pid,c.tid]),[['','','200'],['Choreographer','',''],['','100','']]);
  await undo();
  assert.deepEqual(query().filters.include.map(c=>[c.tag,c.pid,c.tid]),[['','','200'],['Choreographer','','']],'undo takes the added condition away');
  await row(2).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  assert.deepEqual(menu(),['행 원문 복사','북마크 추가'],'raw rows offer no field actions');
  await r.get('rowMenu').dispatch('keydown',{key:'Escape',preventDefault(){},stopPropagation(){}});
  assert.equal(r.get('rowMenu').hidden,true);
});

test('a row makes a condition of its values in the editor, as DLT Viewer does',async t=>{
  const dlt=row=>({ecu:'HU',apid:'NAV',ctid:'ROUT',...row});
  const rows=[{lineNumber:1,kind:'parsed',raw:'r',date:'2026-03-14',time:'09:26:01.120000',pid:'',tid:'',level:'I',tag:'NAV',message:'route updated',dlt:dlt()}];
  const r=await rig({rows,format:'dlt'});t.after(()=>r.controller.close());
  await r.get('closeEditor').dispatch('click');await settle();
  const query=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request;
  const row=r.get('logsRows').children[0],menu=()=>r.get('rowMenu').children.filter(n=>n.tagName==='BUTTON').map(n=>n.textContent);
  await row.dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  assert.deepEqual(menu().slice(0,5),['이 행으로 조건 추가…',"ECU 'HU' 제외","APID 'NAV' 제외","CTID 'ROUT' 제외","페이로드 'route updated' 보기 조건 추가"]);
  await r.get('rowMenu').children.find(n=>n.textContent==='이 행으로 조건 추가…').dispatch('click');await settle();
  assert.equal(r.get('filterEditor').hidden,false);assert.equal(r.get('filterEditorTitle').textContent,'조건 편집');
  assert.deepEqual(['ecuFilter','apidFilter','ctidFilter'].map(id=>r.get(id).value),['HU','NAV','ROUT']);
  const c=query().filters.include[0];assert.deepEqual([c.ecu,c.apid,c.ctid,c.tag],['HU','NAV','ROUT',''],'the IDs go into one condition, tested apart');
  assert.equal(r.document.activeElement,r.get('ecuFilter'));
  const toast=r.get('toast');assert.equal(toast.children[1].textContent,'실행 취소');
  await toast.children[1].dispatch('click');await settle();
  assert.deepEqual(query().filters.include,[]);assert.equal(r.get('filterEditor').hidden,true,'undo closes the editor of the condition it takes away');
  // A hidden ID hides that ID only.
  await row.dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  await r.get('rowMenu').children.find(n=>n.textContent==="APID 'NAV' 제외").dispatch('click');await settle();
  assert.deepEqual(query().filters.exclude.map(c=>[c.apid,c.tag]),[['NAV','']]);
  assert.equal(chipNodes(r.get('activeFilters')).at(-1).textContent,'APID: NAV×');
});

test('a hiding condition hides the levels pressed, and starts with none',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const upper=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  const level=name=>r.get('levelFilters').children.find(b=>b.dataset.level===name),pressed=()=>r.get('levelFilters').children.filter(b=>b.attributes['aria-pressed']==='true').map(b=>b.dataset.level).join('');
  const press=async(name,event={})=>{await level(name).dispatch('click',event);await settle();};
  await r.get('addCondition').dispatch('click');await settle();
  assert.equal(pressed(),'VDIWEFAU','a showing condition starts with every level');
  await r.get('kindHide').dispatch('click');await settle();
  assert.equal(pressed(),'','a hiding condition starts with none');
  assert.deepEqual([r.get('allLevels').textContent,r.get('errorLevels').textContent],['선택 해제','Debug 이하']);
  assert.match(r.get('levelHint').textContent,/누른 레벨을 숨깁니다/);
  await press('V');
  assert.equal(pressed(),'V');assert.deepEqual(upper().exclude.map(c=>c.levels),[['V']],'one press hides that level, not the others');
  await press('D');assert.deepEqual(upper().exclude[0].levels,['V','D']);
  await press('V');await press('D');
  assert.equal(pressed(),'');assert.deepEqual(upper().exclude,[],'none pressed is no level test, so the condition is empty');
  await r.get('errorLevels').dispatch('click');await settle();assert.equal(pressed(),'VD');
  await r.get('allLevels').dispatch('click');await settle();assert.equal(pressed(),'');
  // A list of levels reads as itself, never as "제외 … 제외".
  for(const name of ['V','D','I','W','E','F'])await press(name);
  assert.deepEqual(chipNodes(r.get('activeFilters')).map(chip=>chip.textContent),['제외','레벨: V·D·I·W·E·F×']);
  await r.get('kindShow').dispatch('click');await settle();
  assert.equal(r.get('allLevels').textContent,'전체 선택');assert.equal(r.get('errorLevels').textContent,'Warn 이상');
});

test('the kind moves with arrow keys, a chip opens its own field, and focus comes back to the item',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('tagFilter','NAV');await enter('pidFilter','100');
  assert.deepEqual([r.get('kindShow').tabIndex,r.get('kindHide').tabIndex],[0,-1],'only the chosen kind is a tab stop');
  r.get('kindShow').focus();await r.get('kindShow').dispatch('keydown',{key:'ArrowRight',preventDefault(){}});await settle();
  assert.equal(r.get('kindHide').attributes['aria-checked'],'true');assert.equal(r.document.activeElement,r.get('kindHide'));assert.deepEqual(itemKinds(r),['제외']);
  assert.deepEqual([r.get('kindShow').tabIndex,r.get('kindHide').tabIndex],[-1,0]);
  await r.get('kindHide').dispatch('keydown',{key:'ArrowLeft',preventDefault(){}});await settle();
  assert.deepEqual(itemKinds(r),['보기']);
  await r.get('closeEditor').dispatch('click');await settle();
  await chipEdit(chipNodes(r.get('activeFilters')).find(chip=>chip.textContent.startsWith('PID'))).dispatch('click');await settle();
  assert.equal(r.document.activeElement,r.get('pidFilter'),'a chip opens on its own field');
  await r.get('closeEditor').dispatch('click');await settle();
  // The list was rebuilt while the editor was open: focus goes to the item.
  const item=listItems(r)[0].children[2];item.focus();item.isConnected=false;
  await item.dispatch('click');await settle();
  await r.get('closeEditor').dispatch('click');await settle();
  assert.equal(r.document.activeElement,listItems(r)[0].children[2]);
});

test('a typed list becomes the pattern it meant in one click',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const upper=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  const convert=id=>r.get(id).children[1];
  await enter('tagFilter','Net, a.b');
  assert.equal(convert('tagFilterHint').hidden,false);
  await convert('tagFilterHint').dispatch('click');await r.get('tagFilter').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.deepEqual([upper().include[0].tag,upper().include[0].tagRegex],['Net|a\\.b',true]);assert.equal(r.get('tagFilterHint').hidden,true);
  await enter('pidFilter','2983, 3120');
  assert.equal(r.get('pidFilterError').hidden,false);assert.equal(r.get('pidFilterHint').hidden,false);assert.equal(convert('pidFilterHint').hidden,false);
  assert.equal(chipNodes(r.get('activeFilters')).some(chip=>chip.textContent.includes('2983')),false,'a value not applied shows no chip');
  await convert('pidFilterHint').dispatch('click');await r.get('pidFilter').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.deepEqual([upper().include[0].pid,upper().include[0].pidRegex],['^(2983|3120)$',true]);assert.equal(r.get('pidFilterError').hidden,true);
  await enter('tagFilter','plain');assert.equal(r.get('tagFilterHint').hidden,true);
});

test('removing goes by the condition, not its place, and the editor goes with it',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const add=async tag=>{await r.get('addCondition').dispatch('click');await settle();await enter('tagFilter',tag);};
  await enter('tagFilter','AAA');await add('BBB');await add('CCC');
  // A blank condition is dropped when the editor closes, so places move.
  await listItems(r)[0].children[2].dispatch('click');await settle();await enter('tagFilter','');
  await listItems(r)[1].children[2].dispatch('click');await settle();
  assert.equal(r.get('tagFilter').value,'BBB');
  await r.get('removeCondition').dispatch('click');await settle();
  assert.deepEqual(itemSummaries(r),['태그: CCC'],'the selected condition goes');assert.match(r.get('toast').textContent,/태그: BBB/);
  assert.equal(r.document.activeElement,listItems(r)[0].children[2],'focus moves to what remains');
  // A chip × that takes the edited condition takes the editor too.
  await listItems(r)[0].children[2].dispatch('click');await settle();
  await add('DDD');
  await chipClear(chipNodes(r.get('activeFilters')).find(chip=>chip.textContent==='태그: DDD×')).dispatch('click');await settle();
  assert.equal(r.get('filterEditor').hidden,true);assert.deepEqual(itemSummaries(r),['태그: CCC']);
});

test('chips of a rejected filter find their conditions, or do nothing',async t=>{
  const query=async b=>{if(b.filters.include.some(c=>c.tag==='['))throw 'Invalid or unsupported tag pattern: bad class';return {status:'complete',count:6,complete:true};};
  const r=await rig({query});t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('tagFilter','AAA');
  await r.get('addCondition').dispatch('click');await settle();await enter('tagFilter','BBB');
  await r.get('addCondition').dispatch('click');await settle();await enter('tagFilter','CCC');
  r.get('tagRegex').checked=true;await r.get('tagRegex').dispatch('change');await enter('tagFilter','[');
  for(const [id,task] of [...r.timers])if(task.ms===600){r.timers.delete(id);task.fn();}
  assert.equal(r.get('tagFilterError').hidden,false);assert.match(r.get('tagFilterError').textContent,/태그 정규식을 확인해 주세요/);
  const chips=()=>chipNodes(r.get('activeFilters')).filter(node=>node.className.startsWith('filter-chip'));
  assert.deepEqual(chips().map(chip=>chip.textContent),['태그: AAA×','태그: BBB×','태그: CCC×'],'the chips show what still applies');
  await listItems(r)[0].children[2].dispatch('click');await settle();await r.get('removeCondition').dispatch('click');await settle();
  assert.deepEqual(itemSummaries(r),['태그: BBB','태그 정규식: [ · 확인 필요']);
  const gone=chips().find(chip=>chip.textContent==='태그: AAA×');
  await chipClear(gone).dispatch('click');await settle();
  assert.deepEqual(itemSummaries(r),['태그: BBB','태그 정규식: [ · 확인 필요'],'a chip of a condition that has gone removes no other');
  await chipEdit(gone).dispatch('click');await settle();
  assert.equal(r.get('filterEditor').hidden,true,'and it opens nothing');
});

test('the Assert level a DLT log hides does not count as pressed in a hiding condition',async t=>{
  const r=await rig({format:'dlt'});t.after(()=>r.controller.close());
  const level=name=>r.get('levelFilters').children.find(b=>b.dataset.level===name),upper=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  await r.get('errorLevels').dispatch('click');await settle();
  await r.get('kindHide').dispatch('click');await settle();
  for(const name of ['W','E','F'])await level(name).dispatch('click');
  await settle();
  assert.deepEqual(upper().exclude,[],'no level left pressed is no level test');
});

test('Alt+R switches the field being typed in, and the head moves the editor by keys',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const box2=r.get('tagRegex');box2.click=()=>{box2.checked=!box2.checked;};
  await r.get('tagFilter').dispatch('keydown',{altKey:true,code:'KeyR',preventDefault(){}});
  assert.equal(r.get('tagRegex').checked,true);assert.equal(r.get('pidRegex').checked,false);
  const head=r.get('filterEditorHead'),box=r.get('filterEditor');
  box.style.left='100px';box.style.top='100px';
  await head.dispatch('keydown',{key:'ArrowRight',target:head,preventDefault(){}});
  await head.dispatch('keydown',{key:'ArrowDown',shiftKey:true,target:head,preventDefault(){}});
  assert.notEqual(box.style.left,'100px');assert.notEqual(box.style.top,'100px');
});

test('the row menu of a row with several tags, or none',async t=>{
  const rows=[{lineNumber:1,kind:'parsed',ios:true,raw:'r',date:'2026-09-15',time:'10:47:35.134',pid:'3870',tid:'481516',level:'U',tag:'Memory',tags:['Memory','SDK','Gallery','Extra'],message:'x'},
    {lineNumber:2,kind:'parsed',raw:'r',date:'2026-09-15',time:'10:47:35.200',pid:'3871',tid:'1',level:'U',tag:'',tags:[],message:'y'}];
  const r=await rig({rows});t.after(()=>r.controller.close());
  await r.get('closeEditor').dispatch('click');await settle();
  const menu=()=>r.get('rowMenu').children.filter(n=>n.tagName==='BUTTON').map(n=>n.textContent),open=async line=>{await r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line)).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();};
  await open(2);assert.equal(menu().includes('이 행으로 조건 추가…'),false,'a row without a tag has nothing to fill');
  await r.get('rowMenu').dispatch('keydown',{key:'Escape',preventDefault(){},stopPropagation(){}});
  await open(1);
  assert.equal(menu()[0],'이 행으로 조건 추가…');assert.equal(menu().filter(text=>text.endsWith(' 제외')&&text.startsWith('태그')).length,3,'three tags at most');
  await r.get('rowMenu').children.find(n=>n.textContent==='이 행으로 조건 추가…').dispatch('click');await settle();
  assert.equal(r.get('tagFilter').value,'Memory','the first tag fills the condition');
});

test('the same showing condition is not added twice from a row',async t=>{
  const rows=[{lineNumber:1,kind:'parsed',raw:'06-18 19:24:00.000  100 200 I App: x',date:'06-18',time:'19:24:00.000',pid:'100',tid:'200',level:'I',tag:'App',message:'x'}];
  const r=await rig({rows});t.after(()=>r.controller.close());
  await r.get('closeEditor').dispatch('click');await settle();
  const row=()=>r.get('logsRows').children[0],open=async()=>{await row().dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();};
  await open();await r.get('rowMenu').children.find(n=>n.textContent==='이 행으로 조건 추가…').dispatch('click');await settle();
  await r.get('closeEditor').dispatch('click');await settle();
  await open();await r.get('rowMenu').children.find(n=>n.textContent==='이 행으로 조건 추가…').dispatch('click');await settle();
  assert.equal(listItems(r).length,1);assert.equal(r.get('filterEditor').hidden,false,'the editor opens on the one there is');
  await r.get('closeEditor').dispatch('click');await settle();
  await open();await r.get('rowMenu').children.find(n=>n.textContent==="태그 'App' 보기 조건 추가").dispatch('click');await settle();
  assert.equal(listItems(r).length,1);assert.equal(r.get('toast').children[0].textContent,'같은 보기 조건이 이미 있습니다.');
});

test('the editor stays open until it is closed, and a blank condition does not outlive it',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('tagFilter','NAV');
  await r.get('addCondition').dispatch('click');await settle();
  await r.get('kindHide').dispatch('click');await settle();
  assert.deepEqual(itemKinds(r),['보기','제외']);
  await r.get('filterEditor').dispatch('keydown',{key:'Escape',preventDefault(){},stopPropagation(){}});await settle();
  assert.equal(r.get('filterEditor').hidden,true);assert.deepEqual(itemKinds(r),['보기'],'the blank hiding condition is gone');
  assert.equal(r.get('tagFilter').value,'NAV','the editor shows the remaining condition');
  await chipEdit(chipNodes(r.get('activeFilters'))[0]).dispatch('click');await settle();
  assert.equal(r.get('filterEditor').hidden,false,'a chip opens it again');
});

test('a condition that goes as a whole, by its item or its last chip, comes back from the toast',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const upper=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1)?.args.request.filters;
  const toast=r.get('toast'),chips=()=>chipNodes(r.get('activeFilters')),undo=async()=>{await toast.children[1].dispatch('click');await settle();};
  await enter('tagFilter','NAV');await enter('pidFilter','100');
  await r.get('addCondition').dispatch('click');await settle();await r.get('errorLevels').dispatch('click');await settle();
  await r.get('addCondition').dispatch('click');await settle();await r.get('kindHide').dispatch('click');await settle();await enter('tagFilter','chatty');
  assert.deepEqual(upper().include.map(c=>[c.tag,c.pid,c.levels.join('')]),[['NAV','100','VDIWEFAU'],['','','WEFA']]);
  // A chip that clears one of two fields leaves its condition, quietly.
  toast.replaceChildren();
  await chipClear(chips().find(chip=>chip.textContent==='PID: 100×')).dispatch('click');await settle();
  assert.equal(toast.children.length,0);assert.equal(listItems(r).length,3);
  // A chip that clears the last field takes the condition away, with an undo.
  await chipClear(chips().find(chip=>chip.textContent==='레벨: Warn 이상×')).dispatch('click');await settle();
  assert.equal(listItems(r).length,2);assert.equal(toast.children[0].textContent,'보기 조건을 삭제했습니다 · 레벨: Warn 이상');
  await undo();
  assert.equal(listItems(r).length,3);assert.deepEqual(upper().include.map(c=>[c.tag,c.levels.join('')]),[['NAV','VDIWEFAU'],['','WEFA']],'it is back where it was');
  // 삭제 does the same to the selected condition.
  await r.get('closeEditor').dispatch('click');await settle();
  await listItems(r)[2].children[2].dispatch('click');await settle();
  await r.get('removeCondition').dispatch('click');await settle();
  assert.equal(listItems(r).length,2);assert.equal(toast.children[0].textContent,'제외 조건을 삭제했습니다 · 태그: chatty');
  await undo();
  assert.equal(listItems(r).length,3);assert.deepEqual(upper().exclude.map(c=>c.tag),['chatty']);
});

test('untranslated engine errors keep their detail behind Korean context',async()=>{
  const {describeLogError}=await import('../dist/describe.js');
  assert.equal(describeLogError('Something odd'),'작업을 완료하지 못했습니다. (Something odd)');
  assert.equal(describeLogError('이미 한국어'),'이미 한국어');assert.equal(describeLogError(''),'');
});

test('export starts only for completed results and reports the written rows',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal((await moreState(r,'전체 로그 내보내기…')).disabled,false);
  await choose(r,'전체 로그 내보내기…');
  const call=r.calls.find(c=>c.action==='export_results');
  assert.deepEqual({pane:call.args.pane,suggestedName:call.args.suggestedName},{pane:'logs',suggestedName:'synthetic-all.log'});
  assert.equal(call.args.queryId,r.controller.logs.getState().queryId);
  assert.equal((await moreState(r,'검색 결과 내보내기…')).disabled,true,'no search results to export');
});

test('recent files are listed by opaque id and reopened through the native record',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  assert.equal(r.get('recentFiles').hidden,true);assert.equal(await moreState(r,'최근 파일'),undefined);
  r.listener('recent-changed')({payload:[{id:7,name:'a.log',folder:'~/logs'},{id:'x',name:'bad'},{id:8,name:'b.log',folder:'/tmp'}]});
  assert.equal(r.get('recentFiles').hidden,false);
  await choose(r,'최근 파일');
  assert.deepEqual(r.get('rowMenu').children.filter(n=>n.tagName==='BUTTON').map(n=>n.textContent),['‹ 메뉴','a.log','b.log','목록 지우기'],'the menu shows them in its place, with a way back');
  await closeMenu(r);await settle();await closeMenu(r);
  const items=r.get('recentList').children;assert.equal(items.length,2,'malformed entries are ignored');
  assert.equal(items[0].children[0].textContent,'a.log~/logs');
  await items[1].children[0].dispatch('click');await settle();
  assert.deepEqual(r.calls.filter(c=>c.action==='open_recent').map(c=>c.args),[{id:8}],'only the id crosses to native code');
  await items[0].children[1].dispatch('click');await settle();
  assert.deepEqual(r.calls.filter(c=>c.action==='forget_recent').map(c=>c.args),[{id:7}]);
});

test('time bounds are validated beside the fields, applied upward and shown as one chip',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('timeFrom','6-18');
  assert.equal(r.get('timeFilterError').hidden,false);assert.equal(r.get('timeFrom').attributes['aria-invalid'],'true');assert.equal(r.get('pidFilterError').hidden,true);
  assert.equal(r.get('largeSearchPane').hidden,true,'a field edit without a search opens no empty search pane');
  assert.equal(r.get('largeCancel').disabled,true,'there is no search to cancel');
  assert.equal(r.get('logsPending').hidden,false);assert.match(r.get('logsPending').textContent,/미적용/);
  await enter('timeFrom','06-18 19:24');await enter('timeTo','19:30:05.5');
  assert.equal(r.get('timeFilterError').hidden,true);assert.equal(r.get('logsPending').hidden,true);
  r.get('searchInput').value='x';await r.get('searchInput').dispatch('input');await settle();
  await enter('timeTo','19:3');
  assert.equal(r.get('largeSearchPane').hidden,false,'an existing search stays visible while its scope is pending');
  assert.equal(r.get('largeCancel').disabled,false);
  assert.equal(r.get('logsPending').hidden,false);
  await enter('timeTo','19:30:05.5');assert.equal(r.get('logsPending').hidden,true);
  const query=r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  assert.deepEqual([query.include[0].timeFrom,query.include[0].timeTo],['06-18 19:24','19:30:05.5']);
  const chip=chipNodes(r.get('activeFilters')).find(c=>c.textContent.startsWith('시간'));
  assert.equal(chip.textContent,'시간: 06-18 19:24 ~ 19:30:05.5×');
  await chipClear(chip).dispatch('click');await settle();
  assert.deepEqual([r.get('timeFrom').value,r.get('timeTo').value],['','']);
});

test('an inverted dated time range is applied and explained beside the fields',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('timeFrom','12-31 23:00');await enter('timeTo','01-01 01:00');
  assert.equal(r.get('timeFilterError').hidden,false);assert.match(r.get('timeFilterError').textContent,/해를 넘기지/);
  assert.equal(r.get('logsPending').hidden,true,'the conditions are applied, not pending');
  const query=r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  assert.deepEqual([query.include[0].timeFrom,query.include[0].timeTo],['12-31 23:00','01-01 01:00']);
  await enter('timeTo','12-31 23:30');
  assert.equal(r.get('timeFilterError').hidden,true);
});

test('a startup argument error is shown even when it precedes the listener',async t=>{
  const r=await rig({startupError:'Select one log file'});t.after(()=>r.controller.close());
  assert.ok(r.calls.some(c=>c.action==='take_startup_error'));
  assert.match(r.get('largeError').textContent,/하나만/);assert.equal(r.get('largeError').hidden,false);
});

test('empty time ranges follow the core comparison', async () => {
  const {isEmptyTimeRange}=await import('../dist/filter-model.js');
  for (const [from,to] of [['12-31 23:00','01-01 01:00'],['06-18 19:25','06-18 19:24:59.5'],['2026-01-01','2025-12-31'],['06-18 19:24:59.9991','06-18 19:24:59.9990']]) assert.equal(isEmptyTimeRange(from,to), true, `${from} ~ ${to}`);
  // Same precision ends inclusively; clock-only or single-date bounds run forward; mixed years may match dated rows.
  for (const [from,to] of [['06-18','06-18'],['06-18 19:25','06-18 19:25'],['06-18 19:24:59.9991','06-18 19:24:59.999'],['23:00','01:00'],['06-18 23:00','01:00'],['2026-01-01','12-31'],['','06-18'],['bad','06-18']]) assert.equal(isEmptyTimeRange(from,to), false, `${from} ~ ${to}`);
});

test('time bound grammar matches the core', async () => {
  const {isTimeBound}=await import('../dist/filter-model.js');
  for (const ok of ['', ' ', '06-19', '2026-06-19', '12', '12:30', '12:30:15', '12:30:15.123456789', '06-19 00', ' 2026-01-01 23:59:59.9 ']) assert.equal(isTimeBound(ok), true, ok);
  for (const bad of ['6-19', '25:00', '06-19 12:3', '12:30:15.', '06-19 12 x', '13-01', '06-32', '12:60', '1:00', '12:30.5', '12:30:15.1234567890', '12:30 06-19']) assert.equal(isTimeBound(bad), false, bad);
});

test('the bookmark menu closes when the user scrolls the list, not when the list scrolls itself',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line)),list=r.get('bookmarkList'),menu=r.get('rowMenu');
  for(const line of [3,8]){await row(line).dispatch('click',{detail:1});await settle();r.handlers.get('keydown')({key:'b',metaKey:true,preventDefault(){},target:new Element()});await settle();}
  await r.get('bookmarkTab').dispatch('click');await settle();
  await list.children[1].dispatch('contextmenu',{preventDefault(){},clientX:1,clientY:1});await settle();
  assert.equal(menu.hidden,false);assert.equal(menu.attributes['aria-label'],'북마크 작업');
  await list.dispatch('scroll');
  assert.equal(menu.hidden,false,'a scroll event that moved nothing is the list keeping itself');
  list.scrollTop+=28;await list.dispatch('scroll');
  assert.equal(menu.hidden,true);
  await r.controller.close();
});

test('bookmarks mark rows, list them in a lower tab, step with F2 and reset with the file',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  const key=event=>r.handlers.get('keydown')({preventDefault(){},target:new Element(),...event});
  const listed=()=>r.get('bookmarkList').children.map(item=>item.dataset.lineNumber);
  assert.equal(r.get('lowerTabs').hidden,true,'no bookmark tab without bookmarks');
  await row(3).dispatch('click',{detail:1});await settle();key({key:'b',metaKey:true});await settle();
  await row(8).dispatch('click',{detail:1});await settle();await row(9).dispatch('click',{detail:1,shiftKey:true});await settle();key({key:'b',ctrlKey:true});await settle();
  assert.equal(r.get('bookmarkTab').textContent,'북마크 3');
  assert.equal(r.get('lowerTabs').hidden,false);assert.equal(r.get('bookmarkPane').hidden,true,'adding marks does not replace the search tab');
  assert.match(row(8).className,/bookmarked/);assert.match(row(3).className,/bookmarked/);assert.doesNotMatch(row(4).className,/bookmarked/);
  await r.get('bookmarkTab').dispatch('click');
  assert.equal(r.get('bookmarkPane').hidden,false);assert.equal(r.get('largeSearchPane').hidden,true);
  assert.equal(r.get('bookmarkTab').attributes['aria-selected'],'true');assert.equal(r.get('searchTab').attributes['aria-selected'],'false');
  assert.deepEqual(listed(),['3','8','9']);
  assert.equal(r.document.activeElement,r.get('bookmarkList').children[0],'opening the tab focuses the list');
  key({key:'F2'});await settle();assert.equal(r.controller.logs.getState().selectedLine,3,'F2 continues after the cursor, wrapping');
  key({key:'F2'});await settle();assert.equal(r.controller.logs.getState().selectedLine,8);
  assert.match(r.get('bookmarkList').children[1].className,/selected-log-row/,'the list marks the cursor row');
  key({key:'F2',shiftKey:true});await settle();assert.equal(r.controller.logs.getState().selectedLine,3);
  const second=r.get('bookmarkList').children[1];
  await second.dispatch('keydown',{key:'ArrowUp',target:second,preventDefault(){}});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,3,'arrow keys reveal the focused bookmark');
  await second.children.at(-1).dispatch('click');
  assert.equal(r.get('bookmarkTab').textContent,'북마크 2');assert.deepEqual(listed(),['3','9']);
  const last=r.get('bookmarkList').children[1];
  await last.dispatch('keydown',{key:'Delete',target:last,preventDefault(){}});
  assert.deepEqual(listed(),['3']);assert.equal(r.document.activeElement,r.get('bookmarkList').children[0],'Delete keeps focus in the list');
  r.get('searchInput').value='raw';await r.get('searchInput').dispatch('input');
  assert.equal(r.get('bookmarkPane').hidden,true,'typing a search returns to the search tab');assert.equal(r.get('largeSearchPane').hidden,false);
  await r.get('bookmarkTab').dispatch('click');r.get('bookmarkPane').append(r.get('bookmarkList'));r.get('bookmarkList').children[0].focus();
  await r.get('clearBookmarks').dispatch('click');
  assert.equal(r.get('lowerTabs').hidden,true);assert.ok(r.get('logsRows').children.includes(r.document.activeElement),'clearing from the list returns focus to the selected log row');
  await r.emit({status:'opened',datasetId:2});await settle();
  assert.equal(r.get('lowerTabs').hidden,true,'a new file starts without bookmarks');
});

test('a bookmark shows its row above when selected and moves focus there on Enter or a double-click',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  const key=event=>r.handlers.get('keydown')({preventDefault(){},target:new Element(),...event});
  const pane=r.get('largeSearchPane'),marks=()=>r.get('bookmarkList').children;
  const upper=()=>r.get('logsViewport').contains(r.document.activeElement)?r.document.activeElement.dataset?.lineNumber:null;
  assert.equal(pane.attributes.role,undefined,'the search results are no tab panel without tabs');
  for(const line of [3,8,12]){await row(line).dispatch('click',{detail:1});await settle();key({key:'b',metaKey:true});await settle();}
  assert.equal(pane.attributes.role,'tabpanel');assert.equal(pane.attributes['aria-labelledby'],'searchTab');
  await r.get('bookmarkTab').dispatch('click');
  assert.equal(marks()[0].attributes['aria-describedby'],'bookmarkRowHint');
  await marks()[0].dispatch('keydown',{key:'ArrowDown',target:marks()[0],preventDefault(){}});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,8);assert.equal(upper(),null,'selecting shows the row above and keeps focus');
  await marks()[1].dispatch('keydown',{key:'Enter',target:marks()[1],preventDefault(){}});await settle();
  assert.equal(upper(),'8','Enter moves focus to the row above');
  const panel=r.get('largeLogPanel'),list=r.get('bookmarkList'),third=marks()[2],on={closest:selector=>selector==='.bookmark-row'?third:null};
  for(const detail of [1,2]){await panel.dispatch('mousedown',{detail});await list.dispatch('mousedown',{detail,button:0,target:on});}
  await panel.dispatch('dblclick');await settle();
  assert.equal(r.controller.logs.getState().selectedLine,12);assert.equal(upper(),'12','a double-click does too');
});

test('a combined log numbers rows within their files and marks each later file start',async t=>{
  const files=[{name:'main.log.1',size:60,firstLine:1},{name:'main.log',size:40,firstLine:3}];
  const row=(lineNumber,file,fileLine)=>({lineNumber,file,fileLine,kind:'raw',raw:`raw ${lineNumber}`});
  const rows=[row(1,0,1),row(2,0,2),row(3,1,1)];
  const r=await rig({rows,files,detail:b=>({...rows[b.lineNumber-1],text:`raw ${b.lineNumber}`,startByte:0,nextByte:5,totalBytes:5,complete:true})});t.after(()=>r.controller.close());
  const cells=r.get('logsRows').children.map(row=>row.children[0]);
  assert.deepEqual(cells.map(cell=>cell.children.map(part=>part.textContent)),[['1','1'],['1','2'],['2','1']]);
  assert.match(cells[2].title,/main\.log · 1행\n합친 로그 3행/);
  assert.deepEqual(r.get('logsRows').children.map(row=>row.className.includes('file-start')),[false,false,true]);
  assert.equal(r.get('largeName').textContent,'main.log.1 외 1개');
  assert.match(r.get('largeName').title,/1\. main\.log\.1[^\n]*\n2\. main\.log/);
  await r.get('logsRows').children[2].dispatch('click');await settle();
  assert.equal(r.get('largeDetailTitle').textContent,'main.log · 1행 원문');
  await r.controller.close();
});

test('a native reorder of files opened together is announced when that log opens',async t=>{
  const files=[{name:'main.log.1',size:60,firstLine:1},{name:'main.log',size:40,firstLine:3}];
  const r=await rig({files});t.after(()=>r.controller.close());
  assert.match(r.get('largeName').title,/^2개 파일을 이 순서로 합쳤습니다\n1\. main\.log\.1/);
  const ordered=r.listener('files-ordered'),toast=()=>r.get('toast').textContent;
  // A note for an open that never happened (it failed or was superseded) stays silent.
  ordered({payload:{datasetId:3,count:2,by:'time'}});
  r.source(2);await r.emit({status:'opened',datasetId:2});await settle();
  assert.equal(toast(),'');
  r.source(3);await r.emit({status:'opened',datasetId:3});await settle();
  assert.equal(toast(),'2개 파일을 시간순으로 합쳤습니다.');assert.equal(r.get('toast').hidden,false);
  ordered({payload:{datasetId:4,count:1200,by:'name'}});r.source(4);await r.emit({status:'opened',datasetId:4});await settle();
  assert.equal(toast(),'1,200개 파일을 이름순으로 합쳤습니다.');
  r.get('toast').textContent='';
  ordered({payload:{datasetId:5,count:2,by:'size'}});r.source(5);await r.emit({status:'opened',datasetId:5});await settle();
  assert.equal(toast(),'','an unknown basis is ignored');
  // A repeated opened event for the same log does not announce again.
  await r.emit({status:'opened',datasetId:4});await settle();
  assert.equal(toast(),'');
});

test('a single file keeps plain physical row numbers',async t=>{
  const r=await rig({rows:[{lineNumber:7,kind:'raw',raw:'raw'}]});t.after(()=>r.controller.close());
  const cell=r.get('logsRows').children[0].children[0];
  assert.equal(cell.textContent,'7');assert.equal(cell.children.length,0);
  await r.controller.close();
});

// A JSON body printed over several rows: its header row, two continuation
// rows (one blank) and the next log.
const ENTRY=[
  {lineNumber:1,kind:'parsed',raw:'2026-09-30 21:55:54.351 [ERROR] [AlertsService] [',date:'2026-09-30',time:'21:55:54.351',pid:'',tid:'',level:'E',tag:'AlertsService',message:'['},
  {lineNumber:2,kind:'raw',raw:'    "body": "COMMON"',highlights:[[13,19]],owner:{lineNumber:1,date:'2026-09-30',time:'21:55:54.351',level:'E',tag:'AlertsService',pid:'',tid:''}},
  {lineNumber:3,kind:'blank',raw:'',owner:{lineNumber:1,date:'2026-09-30',time:'21:55:54.351',level:'E',tag:'AlertsService',pid:'',tid:''}},
  {lineNumber:4,kind:'parsed',raw:'2026-09-30 21:55:55.000 [INFO] [Net] next',date:'2026-09-30',time:'21:55:55.000',pid:'',tid:'',level:'I',tag:'Net',message:'next'},
  {lineNumber:5,kind:'raw',raw:'orphan without an entry'},
];
const RECORD_TEXT=ENTRY.slice(0,3).map(row=>row.raw).join('\n');
const recordDetail=b=>b.lineNumber<=3?{first:1,last:3,recordComplete:true,text:RECORD_TEXT,startByte:0,nextByte:RECORD_TEXT.length,totalBytes:RECORD_TEXT.length,complete:true,highlights:[]}:{first:b.lineNumber,last:b.lineNumber,recordComplete:true,text:ENTRY[b.lineNumber-1].raw,startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]};

test('column rows of a multi-line log read as one entry while raw view keeps physical rows',async t=>{
  const r=await rig({rows:ENTRY,detail:recordDetail});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  assert.match(row(1).className,/level-row-E record-continued/);
  assert.match(row(2).className,/^log-row continuation-row level-row-E record-continued/);
  assert.match(row(3).className,/continuation-row level-row-E/);assert.doesNotMatch(row(3).className,/record-continued|raw-row/,'the entry ends before the next log');
  assert.doesNotMatch(row(4).className,/record-continued/);assert.match(row(5).className,/raw-row/,'an orphan stays a raw row');
  // Physical line, five empty field cells naming the entry, then the message column.
  const cells=row(2).children;assert.equal(cells.length,7);assert.equal(cells[0].textContent,'2');
  for(const cell of cells.slice(1,6)){assert.equal(cell.textContent,'');assert.match(cell.title,/^1행 로그에 이어지는 줄\n2026-09-30 21:55:54.351 · Error · AlertsService$/);}
  assert.equal(cells[6].className,'message continuation-text');assert.equal(cells[6].textContent,'    "body": "COMMON"','raw text keeps its indentation');
  assert.equal(row(3).children[6].textContent,'','a blank continuation row stays blank');
  await r.get('largeRaw').dispatch('click');await settle();
  assert.match(row(2).className,/raw-row/);assert.doesNotMatch(row(2).className,/continuation-row|record-continued/);
  assert.equal(row(2).children.length,2);assert.equal(row(2).textContent,'2    "body": "COMMON"');
});

test('a search hit on a continuation row shows its entry fields dimmed',async t=>{
  const r=await rig({rows:ENTRY.slice(1,2),detail:recordDetail});t.after(()=>r.controller.close());
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const hit=r.get('largeRows').children[0],cells=hit.children;
  assert.match(hit.className,/continuation-row level-row-E/);
  assert.deepEqual(cells.slice(1,6).map(c=>c.textContent),['2026-09-30 21:55:54.351','','','Error','AlertsService']);
  assert.ok(cells.slice(1,6).every(c=>/owner-field/.test(c.className)));assert.match(cells[4].className,/level level-E owner-field/);
  assert.equal(cells[6].children[1].tagName,'MARK');assert.equal(cells[6].children[1].textContent,'COMMON');
});

test('selecting any row of an entry shows and copies the whole entry',async t=>{
  const r=await rig({rows:ENTRY,detail:recordDetail});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  for(const line of [1,2,3]){
    await row(line).dispatch('click',{detail:1});await settle();
    const request=r.calls.filter(c=>c.args.request?.action==='record').at(-1).args.request;
    assert.equal(request.lineNumber,line);assert.equal(request.count,65536);
    assert.equal(r.get('largeDetailTitle').textContent,'1–3행 원문');
    assert.equal(r.get('largeDetailText').textContent,RECORD_TEXT);
  }
  await r.get('largeDetailCopy').dispatch('click');await settle();
  assert.deepEqual(r.clipboard,[RECORD_TEXT]);assert.equal(r.get('toast').textContent,'1–3행 원문을 복사했습니다.');
  // A plain copy of the selected row still copies that physical row.
  await row(2).dispatch('click',{detail:1});await settle();
  let data=null;r.handlers.get('copy')({target:new Element(),clipboardData:{setData:(type,value)=>{data=value;}},preventDefault(){}});
  assert.equal(data,'    "body": "COMMON"');
  await row(5).dispatch('click',{detail:1});await settle();
  assert.equal(r.get('largeDetailTitle').textContent,'5행 원문');
});

test('an entry longer than one detail window is copied window by window',async t=>{
  const parts=['2026-09-30 21:55:54.351 [ERROR] [Big] [','\n  a','\n  b'];let offset=0;const windows=[];
  for(const text of parts){windows.push({first:1,last:3,recordComplete:false,text,startByte:offset,nextByte:offset+text.length,totalBytes:parts.join('').length,complete:offset+text.length===parts.join('').length,highlights:[]});offset+=text.length;}
  const r=await rig({rows:ENTRY,detail:b=>windows.find(w=>w.startByte===b.startByte)});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  await row(2).dispatch('click',{detail:1});await settle();
  assert.equal(r.get('largeDetailTitle').textContent,'1–3행 원문 · 앞부분 1 MiB');
  assert.equal(r.get('largeDetailNavigation').hidden,false);
  await r.get('largeDetailCopy').dispatch('click');await settle();
  assert.deepEqual(r.clipboard,[parts.join('')]);assert.equal(r.get('toast').textContent,'1–3행 원문을 복사했습니다. (앞부분 1 MiB)');
  const reads=r.calls.filter(c=>c.args.request?.action==='record').map(c=>c.args.request.startByte);
  assert.deepEqual(reads.slice(-3),[0,parts[0].length,parts[0].length+parts[1].length]);
});

test('the row menu of an entry row also copies the whole entry',async t=>{
  const r=await rig({rows:ENTRY,detail:recordDetail});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  const menu=()=>r.get('rowMenu').children.filter(n=>n.tagName==='BUTTON');
  for(const [line,shown] of [[1,true],[2,true],[3,true],[4,false],[5,false]]){
    await row(line).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
    assert.equal(menu().some(n=>n.textContent==='로그 전체 원문 복사'),shown,`row ${line}`);
  }
  await row(2).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  await menu().find(n=>n.textContent==='로그 전체 원문 복사').dispatch('click');await settle();
  assert.deepEqual(r.clipboard,[RECORD_TEXT]);assert.equal(r.get('toast').textContent,'1–3행 원문을 복사했습니다.');
});

test('bookmarked rows of an entry read like search hits and join when adjacent',async t=>{
  const r=await rig({rows:ENTRY,detail:recordDetail});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  const key=event=>r.handlers.get('keydown')({preventDefault(){},target:new Element(),...event});
  await row(1).dispatch('click',{detail:1});await settle();await row(2).dispatch('click',{detail:1,shiftKey:true});await settle();key({key:'b',metaKey:true});await settle();
  await row(5).dispatch('click',{detail:1});await settle();key({key:'b',metaKey:true});await settle();
  await r.get('bookmarkTab').dispatch('click');await settle();
  const item=line=>r.get('bookmarkList').children.find(n=>n.dataset.lineNumber===String(line));
  assert.match(item(1).className,/level-row-E record-continued/);
  assert.match(item(2).className,/bookmark-row continuation-row level-row-E/);assert.doesNotMatch(item(2).className,/record-continued/);
  assert.match(item(5).className,/raw-row/,'an orphan stays a raw row');
  const cells=item(2).children;
  assert.deepEqual(cells.slice(1,6).map(c=>c.textContent),['2026-09-30 21:55:54.351','','','Error','AlertsService']);
  assert.ok(cells.slice(1,6).every(c=>/owner-field/.test(c.className)&&c.title.startsWith('1행 로그에 이어지는 줄')));
  assert.equal(cells[6].className,'message continuation-text');assert.equal(cells[6].textContent,'    "body": "COMMON"');
  // Removing the entry row's mark leaves the continuation row on its own.
  await item(1).children.at(-1).dispatch('click',{stopPropagation(){}});await settle();
  assert.match(item(2).className,/continuation-row/);
  await r.get('largeRaw').dispatch('click');await settle();
  assert.match(item(2).className,/raw-row/);assert.equal(item(2).children[1].className,'raw-content');
});

test('one Escape closes the raw detail unless the filter panel or a menu already claimed it',async t=>{
  const rows=Array.from({length:3},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw line ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const escape=defaultPrevented=>{let prevented=false;r.handlers.get('keydown')({key:'Escape',defaultPrevented,target:new Element(),preventDefault(){prevented=true;}});return prevented;};
  await r.get('logsRows').children[0].dispatch('click',{detail:1});await settle();
  assert.equal(r.get('largeDetail').hidden,false);
  assert.equal(escape(false),true);assert.equal(r.get('filterEditor').hidden,true,'the open editor goes first');assert.equal(r.get('largeDetail').hidden,false);
  assert.equal(escape(true),false);await settle();assert.equal(r.get('largeDetail').hidden,false,'the panel closed instead');
  assert.equal(escape(false),true);await settle();assert.equal(r.get('largeDetail').hidden,true);
  assert.equal(escape(false),false,'nothing else to close');
});

test('the raw detail has its own splitter, the region splitter shows with results, and a hiding splitter hands focus to the upper logs',async t=>{
  const rows=Array.from({length:3},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw line ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());
  const shown=()=>[r.get('paneSplitter').hidden,r.get('detailSplitter').hidden].map(hidden=>!hidden);
  await r.get('logsRows').children[0].dispatch('click',{detail:1});await settle();
  assert.deepEqual(shown(),[false,true],'a detail alone keeps its own height under its splitter');
  r.get('searchInput').value='raw';await r.get('searchInput').dispatch('input');await settle();
  assert.equal(r.get('largeSearchPane').hidden,false);assert.deepEqual(shown(),[true,true]);
  r.get('detailSplitter').focus();
  await r.get('closeEditor').dispatch('click');
  r.handlers.get('keydown')({key:'Escape',target:r.get('detailSplitter'),preventDefault(){}});await settle();
  assert.equal(r.get('largeDetail').hidden,true);assert.deepEqual(shown(),[true,false]);
  assert.equal(r.document.activeElement,r.get('logsViewport'));
});

test('level counts on partial summaries are never dimmed as empty',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const state=r.controller.logs.getState(),chip=level=>r.get('levelFilters').children.find(b=>b.dataset.level===level);
  await r.emitProgress({pane:'logs',state:{datasetId:state.datasetId,status:'ready',size:100,total:6,textComplete:true,summaryComplete:false,complete:true,summary:{counts:{I:3}},query:{id:state.queryId,status:'complete',count:6,complete:true}}});
  await new Promise(resolve=>setTimeout(resolve,5));await settle();
  assert.equal(r.controller.logs.getState().summaryComplete,false);assert.equal(chip('V').dataset.empty,'false');assert.match(chip('V').querySelector('.count').textContent,/\+$/);
});

test('search cancel shows inside the search box only while a started search runs for a moment',async t=>{
  const r=await rig({query:b=>b.filters.query==='slow'?{status:'running',count:0,complete:false}:{status:'complete',count:6,complete:true}});t.after(()=>r.controller.close());
  const cancel=r.get('largeCancel'),delayed=()=>[...r.timers].filter(([,timer])=>timer.ms===400).map(([id,timer])=>({fn:()=>{r.timers.delete(id);timer.fn();}}));
  const enter=async value=>{r.get('searchInput').value=value;await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  assert.equal(cancel.hidden,true,'no search: no cancel and no reserved space');
  r.get('searchInput').value='slow';await r.get('searchInput').dispatch('input');
  assert.equal(cancel.disabled,false,'typing may still pause the pending search');assert.equal(cancel.hidden,true,'typing does not flash it');assert.equal(delayed().length,0);
  await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.equal(cancel.hidden,true);assert.equal(delayed().length,1);
  delayed()[0].fn();assert.equal(cancel.hidden,false,'a running search shows it after the delay');
  cancel.focus();await cancel.dispatch('click');await settle();
  assert.equal(cancel.hidden,true);assert.equal(r.document.activeElement,r.get('searchInput'),'focus leaves the vanished button for the search input');
  assert.match(r.get('largeTotal').textContent,/취소됨/);
  await enter('fast');
  assert.equal(cancel.hidden,true);assert.equal(delayed().length,0,'a finished search clears the pending show');
});

test('the top-bar menu lists what the open file offers, in groups',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const labels=()=>r.get('rowMenu').children.map(n=>n.tagName==='HR'?null:n.textContent);
  await r.get('moreButton').dispatch('click');
  assert.deepEqual(labels().map(label=>label?.replace(/^(다시 열기 Ctrl\+R).*/,'$1')??null),['다시 열기 Ctrl+R','파일 닫기',null,'전체 로그 내보내기… · 6행','검색 결과 내보내기… 검색어를 입력하면 내보낼 수 있습니다',null,'행 번호로 이동… Ctrl+L','열 폭 ›',null,'단축키·기능 둘러보기 ?']);
  assert.equal(r.get('rowMenu').attributes['aria-label'],'메뉴');
  await closeMenu(r);await r.get('largeRaw').dispatch('click');
  await r.get('moreButton').dispatch('click');
  assert.ok(!labels().includes('열 폭 ›'),'raw view has no columns');
  await closeMenu(r);await r.controller.close();
  await r.get('moreButton').dispatch('click');
  assert.deepEqual(labels(),['단축키·기능 둘러보기 ?'],'without a file only the app\'s own item stays');
});

test('Cmd/Ctrl+L opens the row box; a row outside the file keeps it open, a good one closes it',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`line ${i+1}`}));
  const r=await rig({rows,total:30});t.after(()=>r.controller.close());
  const lKey=()=>r.handlers.get('keydown')({key:'l',ctrlKey:true,target:r.get('logsViewport'),preventDefault(){}});
  const form=r.get('logsJumpForm'),input=r.get('logsJump'),submit=async value=>{input.value=value;await form.dispatch('submit',{preventDefault(){}});await settle();};
  lKey();
  assert.equal(form.hidden,false);assert.equal(r.document.activeElement,input);
  await submit('99');
  assert.equal(form.hidden,false);assert.equal(input.attributes['aria-invalid'],'true');assert.match(r.get('jumpError').textContent,/마지막 행은 30행/);
  assert.doesNotMatch(r.get('toast').textContent,/마지막 행/,'the reason stays in the box, not in a toast that leaves');
  await input.dispatch('input');assert.equal(r.get('jumpError').textContent,'');assert.equal(input.attributes['aria-invalid'],undefined);
  await submit('5');
  assert.equal(form.hidden,true);assert.equal(r.controller.logs.getState().selectedLine,5);
  const landed=r.document.activeElement;
  assert.ok(landed===r.get('logsViewport')||landed.dataset?.lineNumber==='5','keys go on in the list, not on a hidden box');
  lKey();await r.controller.close();
  assert.equal(form.hidden,true,'no file, no box');
  form.hidden=true;lKey();assert.equal(form.hidden,true);
  assert.match(r.get('toast').textContent,/이동할 행이 없습니다/,'a closed file says so too');
});

test('Cmd/Ctrl+L waits behind a modal dialog',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const form=r.get('logsJumpForm');form.hidden=true;
  const lKey=()=>r.handlers.get('keydown')({key:'l',ctrlKey:true,target:r.get('logsViewport'),preventDefault(){}});
  r.get('appleDialog').open=true;lKey();assert.equal(form.hidden,true);
  r.get('appleDialog').open=false;lKey();assert.equal(form.hidden,false);
});

test('a new query leaves the row box open until it finds no rows, then returns focus',async t=>{
  let count=6;
  const r=await rig({query:()=>({status:'complete',count,complete:true})});t.after(()=>r.controller.close());const f=filtersOffRig(r);
  const form=r.get('logsJumpForm'),input=r.get('logsJump');form.append(input);
  r.get('logsViewport').focus();
  r.handlers.get('keydown')({key:'l',ctrlKey:true,target:r.get('logsViewport'),preventDefault(){}});
  await f.enter('tagFilter','Wifi');
  assert.equal(form.hidden,false);assert.equal(r.document.activeElement,input);
  count=0;await f.enter('tagFilter','none');
  assert.equal(form.hidden,true);assert.equal(r.document.activeElement,r.get('logsViewport'),'focus does not fall out with the box');
});

test('a menu item that cannot run says what it waits for',async t=>{
  const text=async(r,label)=>(await moreState(r,label))?.text;
  const running=await rig({query:()=>({status:'running',count:3,complete:false})});t.after(()=>running.controller.close());
  assert.match(await text(running,'전체 로그 내보내기…'),/처리가 끝나면 내보낼 수 있습니다/,'a log with no filter is whole even at a partial count');
  await filtersOffRig(running).enter('tagFilter','Wifi');
  assert.match(await text(running,'필터 결과 내보내기…'),/처리가 끝나면/,'a filter counts as narrowing until it has finished');
  const none=await rig({query:()=>({status:'complete',count:0,complete:true})});t.after(()=>none.controller.close());
  assert.match(await text(none,'전체 로그 내보내기…'),/내보낼 행이 없습니다/);
  const search=await rig({query:b=>({status:'complete',count:b.filters.query==='none'?0:6,complete:true})});t.after(()=>search.controller.close());
  await filtersOffRig(search).enter('searchInput','none');
  assert.match(await text(search,'검색 결과 내보내기…'),/검색 결과가 없습니다/);
  const late=deferred(),busy=await rig({exported:late.promise});t.after(()=>busy.controller.close());
  await choose(busy,'전체 로그 내보내기…');
  assert.match(await text(busy,'전체 로그 내보내기…'),/내보내는 중입니다/);
  late.resolve({rows:6});await settle();
  assert.equal((await moreState(busy,'전체 로그 내보내기…')).disabled,false);
  const reading=await rig();t.after(()=>reading.controller.close());
  await reading.get('openFile').dispatch('click');reading.source(2);await reading.emit({status:'opened',datasetId:2,work:{reopenable:true}});
  assert.match(await text(reading,'다시 열기'),/읽기가 끝나면 다시 열 수 있습니다/);
});

test('the menu names its keys for assistive technology',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const keys=async label=>{const item=await moreItem(r,label);await closeMenu(r);return item.attributes['aria-keyshortcuts'];};
  assert.equal(await keys('다시 열기'),'Control+R');assert.equal(await keys('행 번호로 이동…'),'Control+L');assert.equal(await keys('단축키·기능 둘러보기'),'Shift+/');
});

test('a DLT export offers its formats in the menu and comes back to its item',async t=>{
  const r=await rig({format:'dlt'});t.after(()=>r.controller.close());
  const item=await moreItem(r,'전체 로그 내보내기');
  assert.equal(item.attributes['aria-haspopup'],'menu');assert.match(item.textContent,/· 6행/);assert.match(item.textContent,/›/);assert.doesNotMatch(item.textContent,/…/);
  await item.dispatch('click');await settle();
  assert.equal(r.get('rowMenu').attributes['aria-label'],'내보내기');
  await closeMenu(r);await settle();
  assert.equal(r.get('rowMenu').attributes['aria-label'],'메뉴');assert.equal(r.document.activeElement.dataset.key,'log-export');
});

test('extensions open in the menu\'s place from the list read when the window came back',async t=>{
  const r=await rig({extrasList:[{id:'track',title:'궤적'}]});t.after(()=>r.controller.close());
  await settle();await choose(r,'확장');
  assert.equal(r.get('rowMenu').attributes['aria-label'],'확장 기능');
  assert.ok(r.get('rowMenu').children.some(n=>n.textContent==='궤적'));
  await closeMenu(r);await settle();
  assert.equal(r.document.activeElement.dataset.key,'extras');
});

test('an extras build without settings files shows 확장 with a note and the settings folder',async t=>{
  const r=await rig({extrasList:[]});t.after(()=>r.controller.close());
  await settle();await choose(r,'확장');
  const menu=r.get('rowMenu'),note=menu.children.find(n=>n.tagName==='P');
  assert.deepEqual(menu.children.map(n=>n.tagName==='HR'?null:n.textContent),['‹ 메뉴',null,'설정 파일(extras/<id>.json)이 없습니다','설정 폴더 열기']);
  // A note, not a choice: menu.test.mjs checks that notes describe the menu.
  assert.equal(note.textContent,'설정 파일(extras/<id>.json)이 없습니다');assert.equal(note.attributes.role,undefined);
});

test('a device capture has no file to read again',async t=>{
  const r=await rig({capture:{state:'recording'}});t.after(()=>r.controller.close());
  assert.equal(await moreState(r,'다시 열기'),undefined);assert.ok(await moreState(r,'파일 닫기'));
});

test('Cmd/Ctrl+L with no rows says so instead of doing nothing',async t=>{
  const r=await rig({query:()=>({status:'complete',count:0,complete:true})});t.after(()=>r.controller.close());
  r.get('logsJumpForm').hidden=true;
  r.handlers.get('keydown')({key:'l',ctrlKey:true,target:r.get('logsViewport'),preventDefault(){}});
  assert.equal(r.get('logsJumpForm').hidden,true);assert.match(r.get('toast').textContent,/이동할 행이 없습니다/);
  assert.match((await moreState(r,'행 번호로 이동…')).text,/이동할 행이 없습니다/);
});

test('column width tools open from the top-bar menu and a header right-click, not in raw view',async t=>{
  const r=await rig();t.after(()=>r.controller.close());
  const items=()=>r.get('rowMenu').children.filter(n=>n.tagName==='BUTTON').map(n=>n.textContent);
  await choose(r,'열 폭');
  assert.equal(r.get('rowMenu').hidden,false);assert.deepEqual(items(),['‹ 메뉴','내용에 맞춤','폭 초기화']);
  await r.get('rowMenu').children.at(-1).dispatch('click');assert.equal(r.get('rowMenu').hidden,true);
  let prevented=false;
  await r.get('largeHeaders').dispatch('contextmenu',{clientX:5,clientY:6,preventDefault(){prevented=true;}});
  assert.equal(prevented,true);assert.deepEqual(items(),['내용에 맞춤','폭 초기화']);
  await r.get('rowMenu').children[0].dispatch('click');
  await r.get('largeRaw').dispatch('click');prevented=false;
  await r.get('logsHeaders').dispatch('contextmenu',{clientX:5,clientY:6,preventDefault(){prevented=true;}});
  assert.equal(prevented,false);assert.equal(r.get('rowMenu').hidden,true);
  assert.equal(await moreState(r,'열 폭'),undefined,'raw view has no columns to fit');
});

const pages=(r,pane)=>r.calls.filter(c=>c.args.request?.action==='page'&&c.args.request.pane===pane).map(c=>c.args.request);
const classes=element=>{const set=new Set();element.classList={toggle(name,on){if(on)set.add(name);else set.delete(name);}};return set;};

test('upper logs mark the accepted search results and drop them on new search intent without a new upper query',async t=>{
  const rows=Array.from({length:40},(_,i)=>({lineNumber:i+1,kind:'raw',raw:i%3?`plain ${i+1}`:`COMMON ${i+1}`}));
  const marked=b=>rows.slice(b.start,b.start+b.count).map(row=>b.searchQueryId&&row.raw.startsWith('COMMON')?{...row,searchHit:true,highlights:[[0,6]]}:row);
  const r=await rig({rows,total:40,page:b=>b.pane==='logs'?marked(b):rows.slice(b.start,b.start+b.count)});t.after(()=>r.controller.close());
  const upperQueries=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').length,before=upperQueries();
  const hits=()=>r.get('logsRows').children.filter(n=>n.className.includes('search-hit')).map(n=>n.dataset.lineNumber);
  assert.deepEqual(hits(),[]);assert.ok(pages(r,'logs').every(p=>p.searchQueryId===undefined));
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');
  assert.ok(pages(r,'logs').every(p=>p.searchQueryId===undefined),'a pending search is not yet known to the engine');
  await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const id=r.controller.getState().queryId,reread=pages(r,'logs').filter(p=>p.searchQueryId===id);
  assert.equal(reread.length,1,'one bounded re-read of the visible upper rows');assert.equal(reread[0].start,0);assert.ok(reread[0].count<=200);
  assert.deepEqual(hits().slice(0,4),['1','4','7','10']);
  const hit=r.get('logsRows').children[0];assert.equal(hit.children[1].children[0].tagName,'MARK');assert.match(hit.children[0].title,/검색 결과/);
  // Typing replaces the search: marks disappear at once, before any reply.
  r.get('searchInput').value='COMMONX';await r.get('searchInput').dispatch('input');
  assert.deepEqual(hits(),[]);assert.match(r.get('logsRows').children[0].textContent,/COMMON 1/);
  await settle();assert.equal(pages(r,'logs').filter(p=>p.searchQueryId===id).length,1,'no read for the replaced search');
  await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const next=r.controller.getState().queryId;assert.notEqual(next,id);assert.equal(pages(r,'logs').filter(p=>p.searchQueryId===next).length,1);
  // Clearing the search unmarks without reading again; the upper query never restarted.
  const count=pages(r,'logs').length;r.get('searchInput').value='';await r.get('searchInput').dispatch('input');await settle();
  assert.deepEqual(hits(),[]);assert.equal(pages(r,'logs').length,count);assert.equal(upperQueries(),before);
});

test('selecting a search result always follows it in the upper logs, and held arrows follow where they settle',async t=>{
  const rows=Array.from({length:60},(_,i)=>({lineNumber:i*3+7,kind:'raw',raw:`COMMON ${i}`}));
  const r=await rig({rows,total:200,detail:b=>({text:`line ${b.lineNumber}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]})});t.after(()=>r.controller.close());
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const locates=()=>r.calls.filter(c=>c.args.request?.action==='locate'&&c.args.request.pane==='logs').map(c=>c.args.request.lineNumber);
  await r.get('largeRows').children[2].dispatch('click',{detail:1});await settle();
  assert.deepEqual(locates(),[13]);assert.equal(r.controller.logs.getState().selectedLine,13);
  assert.equal(r.get('logsViewport').contains(r.document.activeElement),false,'following keeps focus in the results');
  assert.equal(r.get('logsRange').attributes['aria-live'],'off','the rows a followed result shows are not announced');
  const viewport=r.get('largeViewport'),settling=()=>[...r.timers.entries()].filter(([,timer])=>timer.ms===120);
  for(let i=0;i<3;i++){await viewport.dispatch('keydown',{key:'ArrowDown',repeat:true,target:r.document.activeElement,preventDefault(){}});await settle();}
  assert.deepEqual(locates(),[13],'a held arrow waits for the selection to settle');
  for(const [key,timer] of settling()){r.timers.delete(key);timer.fn();}await settle();
  assert.deepEqual(locates(),[13,22]);assert.equal(r.controller.logs.getState().selectedLine,22);
  await r.get('largeRows').children[0].dispatch('click',{detail:1});await settle();
  assert.deepEqual(locates(),[13,22,7]);assert.equal(r.controller.logs.getState().selectedLine,7);
});

test('Enter or a double-click on a result already followed above moves focus to its row there',async t=>{
  const rows=Array.from({length:60},(_,i)=>({lineNumber:i*3+7,kind:'raw',raw:`COMMON ${i}`}));
  const r=await rig({rows,total:200,detail:b=>({text:`line ${b.lineNumber}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]})});t.after(()=>r.controller.close());
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  const upper=()=>r.get('logsViewport').contains(r.document.activeElement)?r.document.activeElement.dataset?.lineNumber:null;
  const result=r.get('largeRows').children[2];await result.dispatch('click',{detail:1});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,13);assert.equal(upper(),null,'a click follows without moving focus');
  await result.dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.equal(upper(),'13','Enter moves focus to the row already shown above');
  const panel=r.get('largeLogPanel'),other=r.get('largeRows').children[4];
  const click=async(detail,target)=>{for(const type of ['mousedown','click']){await panel.dispatch(type,{detail});await target.dispatch(type,{detail,button:0});}};
  await click(1,other);await settle();
  assert.equal(upper(),null);
  await click(2,other);await panel.dispatch('dblclick');await settle();
  assert.equal(upper(),'19','a double-click does too');
});

test('the jump box moves to a physical row number, or the next result after a hidden row',async t=>{
  const rows=Array.from({length:60},(_,i)=>({lineNumber:i*3+7,kind:'raw',raw:`COMMON ${i}`}));
  const r=await rig({rows,total:200});t.after(()=>r.controller.close());
  const input=r.get('logsJump'),jump=async value=>{input.value=value;await r.get('logsJumpForm').dispatch('submit',{preventDefault(){}});await settle();};
  assert.match(r.get('jumpHelp').textContent,/1–200행으로 이동합니다\. 현재 필터 결과에 없으면 그 뒤의 첫 결과로 갑니다/);
  await jump('19');assert.equal(r.controller.logs.getState().selectedLine,19);assert.equal(input.value,'');
  const locate=r.calls.filter(c=>c.args.request?.action==='locate').at(-1).args.request;assert.equal(locate.lineNumber,19);assert.equal(locate.pane,'logs');
  await jump(' 20 ');assert.equal(r.controller.logs.getState().selectedLine,22);assert.match(r.get('toast').textContent,/20행은 현재 필터 결과에 없어 다음 결과인 22행으로/);
  assert.equal(r.calls.filter(c=>c.args.request?.action==='page'&&c.args.request.count===1).at(-1).args.request.start,5);
  await jump('190');assert.equal(r.controller.logs.getState().selectedLine,null);assert.match(r.get('toast').textContent,/190행은 현재 필터 결과에 없고 그 뒤에도 결과가 없습니다/);assert.equal(input.value,'190');
  await jump('201');assert.match(r.get('jumpError').textContent,/마지막 행은 200행/);assert.equal(input.attributes['aria-invalid'],'true');
  await jump('2:5');assert.match(r.get('jumpError').textContent,/숫자로/);
  assert.equal(r.get('largeError').hidden,true,'jump outcomes are not engine errors');
});

test('a combined log jump box accepts file:row',async t=>{
  const files=[{name:'a.log',size:50,firstLine:1},{name:'b.log',size:50,firstLine:101}];
  const rows=Array.from({length:200},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`row ${i+1}`,file:i<100?0:1,fileLine:i<100?i+1:i-99}));
  const r=await rig({rows,files,total:200});t.after(()=>r.controller.close());
  const input=r.get('logsJump'),jump=async value=>{input.value=value;await r.get('logsJumpForm').dispatch('submit',{preventDefault(){}});await settle();};
  assert.match(r.get('jumpHelp').textContent,/2:150\) 또는 합친 로그의 1–\d[\d,]*행으로 이동합니다/);
  await jump('2:5');assert.equal(r.controller.logs.getState().selectedLine,105);
  await jump('1:101');assert.match(r.get('jumpError').textContent,/1번 파일에는 101행이 없습니다/);
  await jump('1:0');assert.match(r.get('jumpError').textContent,/행 번호는 1부터입니다/);
  await jump('3:1');assert.match(r.get('jumpError').textContent,/파일 번호는 1–2/);
  await jump('150');assert.equal(r.controller.logs.getState().selectedLine,150);
});

test('the pane whose row the detail does not show keeps a secondary selection',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`COMMON ${i+1}`}));
  const r=await rig({rows,total:30,detail:b=>({text:`line ${b.lineNumber}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]})});t.after(()=>r.controller.close());
  const upper=classes(r.get('logsRows')),lower=classes(r.get('largeRows'));
  r.get('searchInput').value='COMMON';await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  await r.get('largeRows').children[3].dispatch('click',{detail:1});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,4,'followed');assert.equal(upper.has('secondary-selection'),true,'even the same row is secondary in the pane without focus');
  assert.equal(r.get('largeDetailTitle').textContent,'4행 원문');
  await r.get('logsRows').children[8].dispatch('click',{detail:1});await settle();
  assert.equal(lower.has('secondary-selection'),true);assert.equal(upper.has('secondary-selection'),false);assert.equal(r.get('largeDetailTitle').textContent,'로그 · 9행 원문');
  await r.get('largeRows').children[5].dispatch('click',{detail:1});await settle();
  assert.equal(lower.has('secondary-selection'),false);assert.equal(r.get('largeDetailTitle').textContent,'6행 원문');
});

test('a condition no row of the log matches says so in the list and on its chip while it reads as sent',async t=>{
  const none=new Set(['Nothing','Ghost']),states=list=>list.map(c=>none.has(c.tag)?'none':'some');
  const r=await rig({query:b=>({status:'complete',count:6,complete:true,...(b.pane==='logs'?{conditions:{include:states(b.filters.include),exclude:states(b.filters.exclude)}}:{})})});
  t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const items=()=>listItems(r),rows=()=>items().map(item=>item.dataset.rows),summary=item=>item.children[2];
  const chips=()=>chipNodes(r.get('activeFilters')).filter(node=>node.className.startsWith('filter-chip'));
  await enter('tagFilter','Nothing');
  assert.deepEqual(rows(),['none']);assert.equal(summary(items()[0]).textContent,'태그: Nothing · 줄 없음');
  assert.match(summary(items()[0]).title,/이 로그에 이 조건으로 보이는 줄이 없습니다$/);
  await enter('tagFilter','NAV');
  assert.deepEqual(rows(),['']);assert.equal(summary(items()[0]).textContent,'태그: NAV');
  await r.get('addCondition').dispatch('click');await settle();await enter('tagFilter','Nothing');
  await r.get('addCondition').dispatch('click');await settle();await r.get('kindHide').dispatch('click');await settle();await enter('tagFilter','Ghost');
  assert.deepEqual(rows(),['','none','none']);
  assert.match(summary(items()[1]).title,/보이는 줄이 없습니다$/);assert.match(summary(items()[2]).title,/숨기는 줄이 없습니다$/);
  assert.equal(summary(items()[1]).attributes['aria-label'],'보기 조건: 태그: Nothing (이 로그에 맞는 줄 없음)');
  assert.deepEqual(chips().map(chip=>[chip.textContent,chip.className.includes('no-rows')]),[['태그: NAV×',false],['태그: Nothing줄 없음×',true],['태그: Ghost줄 없음×',true]]);
  // Switched off, a condition is not sent and says nothing; the others keep theirs.
  const toggle=items()[1].children[0];toggle.checked=false;await toggle.dispatch('change');await settle();
  assert.deepEqual(rows(),['','','none']);
  assert.deepEqual(chips().map(chip=>chip.className.includes('no-rows')),[false,true]);
});

test('a condition known after its query is marked when the engine reports it, never from a replaced query',async t=>{
  const r=await rig({query:b=>({status:'complete',count:6,complete:true,...(b.pane==='logs'?{conditions:{include:b.filters.include.map(()=>'pending'),exclude:[]}}:{})})});
  t.after(()=>r.controller.close());
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const card=()=>listItems(r)[0];
  const report=async(query,state)=>{await r.emitProgress({pane:'logs',state:{datasetId:1,status:'ready',size:100,total:6,textComplete:true,summaryComplete:true,complete:true,query:{id:query,status:'complete',count:6,complete:true,conditions:{include:[state],exclude:[]}}}});await new Promise(resolve=>setTimeout(resolve,5));};
  await enter('tagFilter','Late');
  const first=r.controller.logs.getState().queryId;
  assert.equal(card().dataset.rows,'','nothing is said while it is checked');
  await report(first,'none');
  assert.equal(card().dataset.rows,'none');
  await enter('tagFilter','Later');
  assert.equal(card().dataset.rows,'','a new query starts unknown');
  await report(first,'none');
  assert.equal(card().dataset.rows,'','the replaced query cannot mark the new one');
  await report(r.controller.logs.getState().queryId,'some');
  assert.equal(card().dataset.rows,'');
});

test('a message condition comes from a row, the field or its 정규식 switch, and a rejected pattern keeps the last conditions',async t=>{
  const rows=[{lineNumber:1,kind:'parsed',raw:'06-18 19:24:00.000  100 200 I Choreographer: Skipped 12 frames!',date:'06-18',time:'19:24:00.000',pid:'100',tid:'200',level:'I',tag:'Choreographer',message:'Skipped 12 frames!'}];
  const query=async b=>{if(b.filters.include.some(c=>c.message==='('))throw 'Invalid or unsupported message pattern: unclosed group';return {status:'complete',count:1,complete:true};};
  const r=await rig({rows,query});t.after(()=>r.controller.close());
  const upper=()=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane==='logs').at(-1).args.request.filters;
  const chips=()=>chipNodes(r.get('activeFilters')).map(chip=>chip.textContent);
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  const hide="메시지 'Skipped 12 frames!' 제외";
  await row(1).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  assert.ok(r.get('rowMenu').children.some(n=>n.textContent==="메시지 'Skipped 12 frames!' 보기 조건 추가"));
  await r.get('rowMenu').children.find(n=>n.textContent===hide).dispatch('click');await settle();
  assert.deepEqual(upper().exclude.map(c=>[c.message,c.messageRegex]),[['Skipped 12 frames!',false]]);
  assert.deepEqual(chips(),['제외','메시지: “Skipped 12 frames!”×']);
  assert.equal(r.get('toast').children[0].textContent,'제외 조건을 추가했습니다 · 메시지: “Skipped 12 frames!”');
  // The same message hidden again adds nothing.
  await row(1).dispatch('contextmenu',{clientX:10,clientY:20,preventDefault(){}});await settle();
  await r.get('rowMenu').children.find(n=>n.textContent===hide).dispatch('click');await settle();
  assert.deepEqual(itemKinds(r),['보기','제외']);
  // The field takes a string; its 정규식 switch makes it a pattern.
  const field=r.get('messageFilter'),enter=async value=>{field.value=value;await field.dispatch('input');await field.dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  await enter('frames');
  assert.deepEqual(upper().include.map(c=>[c.message,c.messageRegex]),[['frames',false]]);
  r.get('messageRegex').checked=true;await r.get('messageRegex').dispatch('change');await field.dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.deepEqual(upper().include.map(c=>[c.message,c.messageRegex]),[['frames',true]]);
  assert.deepEqual(chips(),['메시지 정규식: “frames”×','제외','메시지: “Skipped 12 frames!”×']);
  // A pattern only the engine can reject: the list and chips keep the last
  // conditions, and the field says why.
  const kept=r.controller.logs.getState().queryId;
  await enter('(');
  assert.equal(r.get('messageFilterError').hidden,true,'not while typing');
  for(const [id,task] of [...r.timers])if(task.ms===600){r.timers.delete(id);task.fn();}
  assert.equal(r.controller.logs.getState().queryId,kept);assert.equal(r.controller.logs.getState().count,1);
  assert.equal(r.get('messageFilterError').textContent,'메시지 정규식을 확인해 주세요: unclosed group');assert.equal(r.get('messageFilterError').hidden,false);
  assert.equal(field.attributes['aria-invalid'],'true');assert.equal(r.get('largeError').hidden,true,'not also in the banner');
  assert.equal(r.get('logsPending').textContent,'입력한 조건 미적용 · 이전 조건 결과');
  assert.deepEqual(chips(),['메시지 정규식: “frames”×','제외','메시지: “Skipped 12 frames!”×']);
  // The mark stays on the condition whose pattern the engine may have
  // rejected, also while another condition is edited.
  const summary=index=>itemSummaries(r)[index];
  assert.match(summary(0),/확인 필요$/);
  await listItems(r)[1].children[2].dispatch('click');await settle();
  assert.match(summary(0),/확인 필요$/);assert.doesNotMatch(summary(1),/확인 필요/);
  assert.equal(r.get('messageFilterError').hidden,true,'the edited condition has no rejected pattern');
  await listItems(r)[0].children[2].dispatch('click');await settle();
  await enter('(frames)');
  assert.equal(r.get('messageFilterError').hidden,true);assert.equal(r.get('logsPending').hidden,true);
  assert.deepEqual(chips(),['메시지 정규식: “(frames)”×','제외','메시지: “Skipped 12 frames!”×']);
  // Its chip clears the message and its mode.
  await chipClear(chipNodes(r.get('activeFilters'))[0]).dispatch('click');await settle();
  assert.equal(field.value,'');assert.equal(r.get('messageRegex').checked,false);
});

// The lower region: bookmarks and search results with the upper logs.
const lineDetail=b=>({text:`line ${b.lineNumber}`,startByte:0,nextByte:8,totalBytes:8,complete:true,highlights:[]});
async function markRows(r,lines){
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line));
  for(const line of lines){await row(line).dispatch('click',{detail:1});await settle();r.handlers.get('keydown')({key:'b',ctrlKey:true,target:new Element(),preventDefault(){}});await settle();}
}
async function searchFor(r,text){r.get('searchInput').value=text;await r.get('searchInput').dispatch('input');await r.get('searchInput').dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();}

test('F2 in the bookmark list moves focus with the cursor, so Enter moves to the bookmark it showed',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw ${i+1}`}));
  const r=await rig({rows,detail:lineDetail});t.after(()=>r.controller.close());
  const key=event=>r.handlers.get('keydown')({preventDefault(){},target:r.document.activeElement,...event});
  await markRows(r,[3,8,12]);
  await r.get('bookmarkTab').dispatch('click');await settle();
  const marks=()=>r.get('bookmarkList').children,focused=()=>r.document.activeElement?.dataset?.lineNumber;
  await marks()[0].dispatch('keydown',{key:'ArrowDown',target:marks()[0],preventDefault(){}});await settle();
  assert.equal(focused(),'8');
  key({key:'F2'});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,12);assert.equal(focused(),'12','focus follows the cursor in the list');
  const active=r.document.activeElement;await active.dispatch('keydown',{key:'Enter',target:active,preventDefault(){}});await settle();
  assert.equal(r.get('logsViewport').contains(r.document.activeElement),true);assert.equal(focused(),'12','Enter moves to the bookmark F2 showed');
  key({key:'F2'});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,3,'F2 outside the list wraps to the first bookmark');
  assert.equal(r.get('logsViewport').contains(r.document.activeElement),true,'and leaves focus in the upper logs');
  const reads=()=>r.calls.filter(c=>c.args.request?.action==='record').length,before=reads();
  key({key:'F2',repeat:true});await settle();key({key:'F2',repeat:true});await settle();
  assert.equal(reads(),before,'a held F2 reads the detail only where it settles');
  for(const [id,timer] of [...r.timers])if(timer.ms===120){r.timers.delete(id);timer.fn();}
  await new Promise(resolve=>setTimeout(resolve,200));await settle();
  assert.equal(reads(),before+1);assert.equal(r.get('largeDetailText').textContent,'line 12');assert.equal(r.controller.logs.getState().selectedLine,12);
});

test('a bookmark shown above becomes the raw detail and the primary selection, as a search result does',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`COMMON ${i+1}`}));
  const r=await rig({rows,total:30,detail:lineDetail});t.after(()=>r.controller.close());
  const upper=classes(r.get('logsRows')),lower=classes(r.get('largeRows'));
  const key=event=>r.handlers.get('keydown')({preventDefault(){},target:r.document.activeElement,...event});
  await markRows(r,[3,8,12]);
  await searchFor(r,'COMMON');
  await r.get('largeRows').children[5].dispatch('click',{detail:1});await settle();
  assert.equal(r.get('largeDetailText').textContent,'line 6');
  r.get('searchInput').focus();
  key({key:'F2'});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,8);
  assert.equal(r.get('largeDetailText').textContent,'line 8','the detail shows the bookmark, not the search result');
  assert.equal(r.get('largeDetailTitle').textContent,'로그 · 8행 원문');
  assert.equal(upper.has('secondary-selection'),false,'the bookmark row above is the primary selection');assert.equal(lower.has('secondary-selection'),true);
  await r.get('bookmarkTab').dispatch('click');await settle();
  const marks=()=>r.get('bookmarkList').children;
  await marks()[2].dispatch('click',{detail:1});await settle();
  assert.equal(r.controller.logs.getState().selectedLine,12);assert.equal(r.get('largeDetailText').textContent,'line 12','a click shows it too');
  // A held arrow key reads the detail only where the selection settles.
  const reads=()=>r.calls.filter(c=>c.args.request?.action==='record').length,before=reads();
  await marks()[2].dispatch('keydown',{key:'ArrowUp',repeat:true,target:marks()[2],preventDefault(){}});await settle();
  assert.equal(reads(),before);
  for(const [id,timer] of [...r.timers])if(timer.ms===120){r.timers.delete(id);timer.fn();}
  await new Promise(resolve=>setTimeout(resolve,200));await settle();
  assert.equal(r.get('largeDetailText').textContent,'line 8');assert.equal(r.controller.logs.getState().selectedLine,8);
});

test('the list with keyboard focus draws its selection as primary and the other panes as secondary',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`COMMON ${i+1}`}));
  const r=await rig({rows,total:30,detail:lineDetail});t.after(()=>r.controller.close());
  const upper=classes(r.get('logsRows')),lower=classes(r.get('largeRows')),marked=classes(r.get('bookmarkList'));
  await searchFor(r,'COMMON');
  const result=r.get('largeRows').children[3];await result.dispatch('click',{detail:1});await settle();
  assert.equal(r.get('largeViewport').contains(r.document.activeElement),true);
  assert.equal(r.controller.logs.getState().selectedLine,4,'the same row is shown above');
  assert.equal(upper.has('secondary-selection'),true,'the row above is secondary while the results have focus');assert.equal(lower.has('secondary-selection'),false);
  await result.dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();
  assert.equal(r.get('logsViewport').contains(r.document.activeElement),true);
  await r.get('largeViewport').dispatch('focusout',{relatedTarget:r.document.activeElement});await r.get('logsViewport').dispatch('focusin',{});
  assert.equal(upper.has('secondary-selection'),false,'focus moved above, so the row above is primary');assert.equal(lower.has('secondary-selection'),true);
  await r.get('logsViewport').dispatch('focusout',{relatedTarget:r.get('searchInput')});r.get('searchInput').focus();
  assert.equal(upper.has('secondary-selection'),true,'outside the lists the pane whose row the detail shows is primary');assert.equal(lower.has('secondary-selection'),false);
  r.handlers.get('keydown')({key:'b',ctrlKey:true,target:new Element(),preventDefault(){}});await settle();
  await r.get('bookmarkTab').dispatch('click');await settle();
  await r.get('bookmarkList').children[0].dispatch('click',{detail:1});await settle();
  assert.equal(r.get('bookmarkList').contains(r.document.activeElement),true);
  assert.equal(marked.has('secondary-selection'),false,'the focused bookmark list is primary');assert.equal(upper.has('secondary-selection'),true);
  const first=r.get('bookmarkList').children[0];r.get('rowMenu').hidden=true;
  await first.dispatch('contextmenu',{preventDefault(){},clientX:0,clientY:0});
  assert.equal(r.get('rowMenu').contains(r.document.activeElement),true,'the menu takes focus');
  await r.get('bookmarkList').dispatch('focusout',{relatedTarget:r.document.activeElement});
  assert.equal(marked.has('secondary-selection'),false,'the list a menu was opened from stays primary');assert.equal(upper.has('secondary-selection'),true);
  first.focus();await r.get('bookmarkList').dispatch('focusin',{});
  r.document.hasFocus=()=>false;await r.get('bookmarkList').dispatch('focusout',{relatedTarget:null});
  assert.equal(marked.has('secondary-selection'),false,'the window going inactive changes nothing');
  r.document.hasFocus=()=>true;await r.get('bookmarkList').dispatch('focusout',{relatedTarget:null});
  assert.equal(marked.has('secondary-selection'),true,'focus leaving for no element falls back to the detail');
});

test('Enter in the search box brings the results tab forward and says which result it shows and when it wraps',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`COMMON ${i+1}`}));
  const r=await rig({rows,total:30,detail:lineDetail});t.after(()=>r.controller.close());
  await markRows(r,[3]);
  await searchFor(r,'COMMON');
  const input=r.get('searchInput'),position=r.get('searchPosition'),enter=async(shiftKey=false)=>{await input.dispatch('keydown',{key:'Enter',shiftKey,preventDefault(){}});await settle();};
  await r.get('bookmarkTab').dispatch('click');await settle();
  assert.equal(r.get('largeSearchPane').hidden,true);
  input.focus();await enter();
  assert.equal(r.get('largeSearchPane').hidden,false,'the results tab comes forward');assert.equal(r.get('bookmarkPane').hidden,true);
  assert.equal(r.get('searchTab').attributes['aria-selected'],'true');assert.equal(r.document.activeElement,input,'typing focus stays');
  assert.equal(r.controller.getState().selectedLine,1);
  assert.equal(position.textContent,'1번째 / 30건');assert.equal(position.attributes['aria-live'],'polite','a step is announced');
  await enter(true);
  assert.equal(r.controller.getState().selectedLine,30);assert.equal(position.textContent,'다시 끝부터 · 30번째 / 30건');
  await r.get('bookmarkTab').dispatch('click');await settle();await r.get('searchTab').dispatch('click');await settle();
  assert.equal(position.textContent,'30번째 / 30건','a wrap note is said once');
  await enter();
  assert.equal(r.controller.getState().selectedLine,1);assert.equal(position.textContent,'다시 처음부터 · 1번째 / 30건');
  await enter();assert.equal(position.textContent,'2번째 / 30건');
  await r.get('largeRows').children[5].dispatch('click',{detail:1});await settle();
  assert.equal(position.textContent,'6번째 / 30건','a click shows its place too');assert.equal(position.attributes['aria-live'],'off','without announcing it');
  await r.get('bookmarkTab').dispatch('click');await settle();
  assert.equal(position.textContent,'','nothing is said of results the bookmarks tab hides');
  await r.get('searchTab').dispatch('click');await settle();assert.equal(position.textContent,'6번째 / 30건');
  input.value='COMMONX';await input.dispatch('input');
  assert.equal(position.textContent,'','a new search clears it');
});

test('removed bookmarks come back from the toast, and 모두 해제 stands apart from 모두 복사',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw ${i+1}`}));
  const r=await rig({rows,detail:lineDetail});t.after(()=>r.controller.close());
  const listed=()=>r.get('bookmarkList').children.map(n=>Number(n.dataset.lineNumber)),count=()=>r.get('bookmarkTab').textContent;
  const undoButton=()=>r.get('toast').children.find(n=>n.className==='toast-action');
  const undo=async()=>{const button=undoButton();assert.equal(button?.textContent,'실행 취소');await button.dispatch('click');await settle();};
  await markRows(r,[3,8,12,20]);
  assert.equal(undoButton(),undefined,'adding offers no undo');
  await markRows(r,[8]);
  assert.match(r.get('toast').textContent,/북마크 1개를 해제했습니다/);assert.equal(count(),'북마크 3');
  await undo();assert.equal(count(),'북마크 4','Ctrl+B on a marked row is undone');
  await r.get('bookmarkTab').dispatch('click');await settle();
  const g=gestures(r,r.get('bookmarkList'),()=>r.get('bookmarkList').children);
  await g.press(8);await g.up(g.y(8));await g.press(12,{shiftKey:true});await g.up(g.y(12));
  await r.document.activeElement.dispatch('keydown',{key:'Delete',target:r.document.activeElement,preventDefault(){}});await settle();
  assert.deepEqual(listed(),[3,20]);
  assert.equal(r.document.activeElement?.dataset?.lineNumber,'20');
  await undo();assert.deepEqual(listed(),[3,8,12,20],'Delete is undone');
  assert.match(r.get('bookmarkList').children[2].className,/selected-log-row/,'with the cursor where it was');
  assert.equal(r.document.activeElement?.dataset?.lineNumber,'12','and focus with it');
  await g.node(20).children.at(-1).dispatch('click');await settle();
  assert.deepEqual(listed(),[3,8,12]);assert.match(r.get('toast').textContent,/북마크 1개를 해제했습니다/,'the × button says so too');
  await undo();assert.deepEqual(listed(),[3,8,12,20]);
  await r.get('clearBookmarks').dispatch('click');await settle();
  assert.equal(r.get('lowerTabs').hidden,true);assert.match(r.get('toast').textContent,/북마크를 모두 해제했습니다/);
  await undo();
  assert.equal(count(),'북마크 4');assert.equal(r.get('bookmarkPane').hidden,false,'the bookmarks tab comes back');assert.deepEqual(listed(),[3,8,12,20]);
  await r.get('clearBookmarks').dispatch('click');await settle();
  const late=undoButton();r.source(2,true);await r.emit({status:'opened',datasetId:2});await settle();
  await late.dispatch('click');await settle();
  assert.equal(r.get('lowerTabs').hidden,true,'an undo never brings marks into another log');
  const html=await readFile(new URL('../dist/index.html',import.meta.url),'utf8');
  assert.match(html,/>모두 복사<\/button><span class="tool-divider" aria-hidden="true"><\/span><button id="clearBookmarks" class="text-button danger-text"/,'a divider and a warning colour set 모두 해제 apart');
});

test('an undo leaves a row marked again meanwhile alone, so a full list takes it back',async t=>{
  const rows=Array.from({length:1000},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`raw ${i+1}`}));
  const r=await rig({rows,total:1000,detail:lineDetail});t.after(()=>r.controller.close());
  const row=line=>r.get('logsRows').children.find(n=>n.dataset.lineNumber===String(line)),key=event=>r.handlers.get('keydown')({preventDefault(){},target:new Element(),...event});
  await row(1).dispatch('click',{detail:1});await settle();
  await r.get('logsViewport').dispatch('keydown',{key:'End',shiftKey:true,target:r.document.activeElement,preventDefault(){}});await settle();
  key({key:'b',ctrlKey:true});await settle();
  assert.equal(r.get('bookmarkTab').textContent,'북마크 1,000');
  await r.get('logsViewport').dispatch('keydown',{key:'Home',target:r.document.activeElement,preventDefault(){}});await settle();
  await row(5).dispatch('click',{detail:1});await settle();key({key:'b',ctrlKey:true});await settle();
  const undo=r.get('toast').children.find(n=>n.className==='toast-action');assert.equal(r.get('bookmarkTab').textContent,'북마크 999');
  key({key:'b',ctrlKey:true});await settle();assert.equal(r.get('bookmarkTab').textContent,'북마크 1,000','marked again');
  await undo.dispatch('click');await settle();
  assert.equal(r.get('bookmarkTab').textContent,'북마크 1,000');assert.doesNotMatch(r.get('toast').textContent,/최대/,'the row already back is not counted twice');
});

// The filter switched off as a whole.
function filtersOffRig(r){
  const sent=pane=>r.calls.filter(c=>c.args.request?.action==='query'&&c.args.request.pane===pane).at(-1)?.args.request.filters;
  const enter=async(id,value)=>{r.get(id).value=value;await r.get(id).dispatch('input');await r.get(id).dispatch('keydown',{key:'Enter',preventDefault(){}});await settle();};
  const press=async()=>{const box=r.get('filtersOn');box.checked=!box.checked;await box.dispatch('change');await settle();};
  const undo=async()=>{await r.get('toast').children[1].dispatch('click');await settle();};
  return {sent,enter,press,undo,off:()=>r.get('filtersOn').checked===false,chips:()=>chipNodes(r.get('activeFilters')).map(chip=>chip.textContent)};
}

test('switching the filter off shows every row with its conditions kept, and on brings them back',async t=>{
  const r=await rig();t.after(()=>r.controller.close());const f=filtersOffRig(r);
  assert.equal(r.get('filtersSwitch').hidden,true,'there is nothing to switch off yet');
  await f.enter('tagFilter','Wifi');await r.get('includeRaw').dispatch('change');await settle();
  await f.enter('searchInput','timeout');
  assert.equal(r.get('filtersSwitch').hidden,false);assert.equal(r.get('filtersOn').checked,true);
  assert.equal(r.get('filterRow').hidden,false);assert.match(r.get('largeTotal').textContent,/^ · 필터 결과/);
  assert.equal(r.get('filtersOn').attributes['aria-keyshortcuts'],'Control+Shift+F');
  await f.press();
  assert.deepEqual([f.sent('logs').include,f.sent('logs').exclude,f.sent('logs').includeRaw],[[],[],true],'the list shows every row');
  assert.deepEqual([f.sent('search').include,f.sent('search').query],[[],'timeout'],'the search finds in the rows shown');
  assert.equal(r.get('tagFilter').value,'Wifi');assert.deepEqual(f.chips(),['태그: Wifi×'],'the conditions stay to read');
  assert.equal(r.get('activeFilters').dataset.off,'true');assert.equal(r.get('filterBadge').dataset.off,'true');assert.equal(r.get('filterRow').dataset.off,'true');
  assert.equal(r.get('filtersOffNote').textContent,'필터 꺼짐 · 모든 줄 표시');assert.equal(r.get('filtersOffNote').className,'filters-off-note');
  assert.equal(r.get('filtersOffHint').hidden,false,'the panel says so too');
  assert.equal(r.get('activeFilters').attributes['aria-label'],'꺼 둔 필터');assert.match(r.get('toggleFilters').attributes['aria-label'],/1개 조건 꺼 둠/);
  assert.equal(r.get('searchInput').placeholder,'모든 줄의 원문에서 검색');assert.equal((await moreState(r,'전체 로그 내보내기…')).title,'모든 줄의 원문을 파일로 저장');
  assert.match(r.get('largeTotal').textContent,/^ · 모든 줄/,'the search count says what it counts');assert.match(r.get('jumpHelp').textContent,/행으로 이동합니다\.$/,'every row is there to jump to');
  assert.doesNotMatch(chipClear(chipNodes(r.get('activeFilters'))[0]).attributes['aria-label'],/필터/,'the last chip leaves no filter to switch on');
  await f.press();
  assert.deepEqual([f.sent('logs').include.map(c=>c.tag),f.sent('search').include.map(c=>c.tag)],[['Wifi'],['Wifi']]);
  assert.equal(r.get('filterBadge').dataset.off,'false');assert.equal(r.get('filtersOffHint').hidden,true);
  assert.equal(r.get('filtersOffNote').textContent,'필터 켜짐','a screen reader is told it is on again');assert.equal(r.get('filtersOffNote').className,'visually-hidden');
  assert.equal(r.get('searchInput').placeholder,'필터 결과의 원문에서 검색');assert.match(r.get('largeTotal').textContent,/^ · 필터 결과/);
});

test('a filter switched off keeps the lists in place, and the shortcut needs a filter to switch',async t=>{
  const rows=Array.from({length:30},(_,i)=>({lineNumber:i+1,kind:'raw',raw:`row ${i+1}`}));
  const r=await rig({rows,detail:{text:'x',startByte:0,nextByte:1,totalBytes:1,complete:true,highlights:[]}});t.after(()=>r.controller.close());const f=filtersOffRig(r);
  const key=async more=>{let prevented=false;await r.handlers.get('keydown')({key:'F',code:'KeyF',metaKey:true,shiftKey:true,target:new Element(),preventDefault(){prevented=true;},...more});await settle();return prevented;};
  assert.equal(await key(),false,'without conditions the key is left alone');
  await f.enter('tagFilter','Wifi');
  await r.get('logsRows').children.find(n=>n.dataset.lineNumber==='5').dispatch('click',{detail:1});await settle();
  assert.equal(await key(),true);assert.equal(f.off(),true);
  assert.equal(r.controller.logs.getState().selectedLine,5,'the selected row stays');
  assert.equal(await key({repeat:true}),true);assert.equal(f.off(),true,'a held key switches once');
  assert.equal(await key({metaKey:false,ctrlKey:true}),true);assert.equal(f.off(),false);
  assert.equal(r.controller.logs.getState().selectedLine,5);
});

test('changing a condition switches the filter back on and says so, and its undo puts all of it back',async t=>{
  const r=await rig();t.after(()=>r.controller.close());const f=filtersOffRig(r);
  await f.enter('tagFilter','Wifi');await f.press();
  await f.enter('searchInput','timeout');
  assert.equal(f.off(),true,'a new search keeps it off');assert.deepEqual(f.sent('search').include,[]);
  await f.enter('tagFilter','Net');
  assert.equal(f.off(),false);assert.deepEqual([f.sent('logs').include[0].tag,f.sent('search').include[0].tag],['Net','Net']);
  assert.equal(r.get('toast').children[0].textContent,'조건이 바뀌어 필터를 켰습니다.');
  await f.undo();
  assert.equal(f.off(),true);assert.equal(r.get('tagFilter').value,'Wifi');assert.deepEqual(f.sent('logs').include,[]);
  // A chip's × is a change too; removing the last one leaves nothing off.
  await chipClear(chipNodes(r.get('activeFilters'))[0]).dispatch('click');await settle();
  assert.equal(f.off(),false);assert.equal(r.get('filtersSwitch').hidden,true);assert.deepEqual(f.chips(),[]);
  assert.equal(r.get('toast').children[0].textContent,'마지막 조건을 해제했습니다.');
  await f.undo();
  assert.equal(f.off(),true,'the undo puts the filter back off');assert.deepEqual(f.chips(),['태그: Wifi×']);
  // A row action's own undo takes the condition away and the filter off.
  await f.press();
  await f.enter('pidFilter','100');await f.press();
  await r.get('resetFilters').dispatch('click');await settle();
  assert.equal(f.off(),false,'초기화 leaves no filter to be off');
  await f.undo();
  assert.equal(f.off(),true,'and its undo puts it back off');assert.deepEqual(f.sent('logs').include,[]);assert.equal(r.get('tagFilter').value,'Wifi');
});

test('with the filter off a chip and a card say what changing them does',async t=>{
  const r=await rig();t.after(()=>r.controller.close());const f=filtersOffRig(r);
  await f.enter('tagFilter','Wifi');await f.enter('pidFilter','100');await f.press();
  const chip=chipNodes(r.get('activeFilters'))[0],toggle=()=>listItems(r)[0].children[0];
  assert.match(chipClear(chip).title,/나머지 조건으로 필터가 켜집니다$/);assert.match(chipClear(chip).attributes['aria-label'],/해제 \(나머지 조건으로 필터 켜짐\)$/);
  assert.equal(toggle().title,'켜진 조건 · 끄면 이 조건만 끄고 필터를 켭니다');
  toggle().checked=false;await toggle().dispatch('change');await settle();
  assert.equal(f.off(),false);assert.equal(toggle().title,'꺼진 조건 · 켜면 적용합니다');
  await f.undo();assert.equal(f.off(),true);
});

test('a condition that cannot apply neither traps the filter on nor switches it on',async t=>{
  const query=async b=>{if(b.filters.include.some(c=>c.message==='('))throw 'Invalid or unsupported message pattern: unclosed group';return {status:'complete',count:6,complete:true};};
  const r=await rig({query});t.after(()=>r.controller.close());const f=filtersOffRig(r);
  await f.enter('tagFilter','Wifi');
  // With a wrong PID the filter still switches off, and the field stays wrong.
  await f.enter('pidFilter','abc');assert.equal(f.sent('logs').include[0].pid,'');
  await f.press();
  assert.equal(f.off(),true);assert.deepEqual(f.sent('logs').include,[]);assert.equal(r.get('pidFilterError').hidden,false);
  // It cannot switch on until the field is right, and says so.
  await f.press();
  assert.equal(f.off(),true,'the box goes back');assert.equal(r.get('toast').textContent,'조건에 오류가 있어 필터를 켜지 못했습니다.');assert.deepEqual(f.sent('logs').include,[]);
  await f.enter('pidFilter','abcd');
  assert.equal(f.off(),true,'a wrong edit leaves it off');assert.equal(r.get('activeFilters').dataset.off,'true');
  await f.enter('pidFilter','');
  assert.equal(f.off(),true,'back to the conditions it left');assert.equal(r.get('pidFilterError').hidden,true);
  // Off, then a pattern the engine rejects: the rows are all still shown, so
  // the filter stays off, and the conditions kept are the ones that were in use.
  await f.enter('messageFilter','(');
  assert.equal(f.off(),true);assert.ok(r.controller.logs.getState().rejected);assert.deepEqual(f.sent('logs').include.length,1);
  assert.deepEqual(f.chips(),['태그: Wifi×'],'the chips read the kept conditions');
  // The pattern gone is a change that applies, so the filter comes on.
  await f.enter('messageFilter','');
  assert.equal(r.controller.logs.getState().rejected,'');assert.equal(f.off(),false);assert.equal(f.sent('logs').include[0].tag,'Wifi');
});

test('the logs exported with the filter off are named as every row',async t=>{
  let count=2;
  const r=await rig({query:()=>({status:'complete',count,complete:true})});t.after(()=>r.controller.close());const f=filtersOffRig(r);
  const named=()=>r.calls.filter(c=>c.action==='export_results').at(-1)?.args.suggestedName;
  await f.enter('tagFilter','Wifi');
  await choose(r,'필터 결과 내보내기…');
  assert.match(named(),/-filtered\./);assert.equal(r.get('toast').children[0].textContent,'3,000행을 내보냈습니다.');
  count=6;await f.press();
  await choose(r,'전체 로그 내보내기…');
  assert.match(named(),/-all\./);assert.equal(r.get('toast').children[0].textContent,'전체 3,000행을 내보냈습니다.');
});

test('a filter switched off lasts through a rollback, and another file starts with its filter on',async t=>{
  const r=await rig();t.after(()=>r.controller.close());const f=filtersOffRig(r);
  await f.enter('tagFilter','Wifi');await f.press();
  r.source(2);await r.emit({status:'opened',datasetId:2});await settle();
  assert.equal(f.off(),false,'the next log has its own filter');
  r.source(1,true);await r.emit({status:'rollback',datasetId:2,error:'invalid UTF-8'});await settle();
  assert.equal(f.off(),true);assert.deepEqual(f.sent('logs').include,[]);assert.equal(r.get('tagFilter').value,'Wifi');
});
