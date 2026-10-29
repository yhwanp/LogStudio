import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {mountLogLayout, widestLine} from '../dist/layout.js';

class Element {
  constructor(document){this.ownerDocument=document;this.children=[];this.listeners=new Map();this.attributes={};this.dataset={};this.hidden=false;this.inert=false;this.hover=false;const classes=new Set();this.classList={toggle:(name,value)=>value?classes.add(name):classes.delete(name),add:name=>classes.add(name),remove:name=>classes.delete(name),contains:name=>classes.has(name)};this.style={setProperty(name,value){this[name]=value;}};}
  append(...nodes){this.children.push(...nodes);}
  replaceChildren(...nodes){this.children=nodes;}
  querySelectorAll(selector){return this.children.flatMap(child=>[...(selector==='.'+child.className?[child]:[]),...child.querySelectorAll(selector)]);}
  setAttribute(key,value){this.attributes[key]=value;}
  getAttribute(key){return this.attributes[key]??null;}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  matches(){return this.hover;}
  getClientRects(){return this.hidden?[]:[{}];}
  addEventListener(name,fn){this.listeners.set(name,fn);}
  fire(name,event={}){this.listeners.get(name)?.(event);}
  focus(){this.ownerDocument.activeElement=this;}
}
function rig(saved=null,setup=()=>{}){
  // Monospace text measures seven pixels per character.
  const canvas={getContext:()=>({font:'',measureText:text=>({width:text.length*7})})};
  const elements=new Map(),document={activeElement:null,listeners:new Map(),getElementById:id=>{if(!elements.has(id))elements.set(id,new Element(document));return elements.get(id);},createElement:tag=>tag==='canvas'?canvas:new Element(document),addEventListener(name,fn){this.listeners.set(name,fn);}};
  const timers=new Map(),writes=[];let seq=0;
  const window={localStorage:{getItem:()=>JSON.stringify(saved),setItem:(_key,value)=>writes.push(JSON.parse(value))},setTimeout:(fn,ms)=>{timers.set(++seq,{fn,ms});return seq;},clearTimeout:id=>timers.delete(id),listeners:new Map(),
    addEventListener(name,fn){(this.listeners.get(name)??this.listeners.set(name,[]).get(name)).push(fn);},removeEventListener(name,fn){const list=this.listeners.get(name)??[];if(list.includes(fn))list.splice(list.indexOf(fn),1);},
    fire(name,event={}){for(const fn of [...this.listeners.get(name)??[]])fn(event);},getComputedStyle:()=>({fontWeight:'400',fontSize:'12px',fontFamily:'mono',paddingLeft:'6px',paddingRight:'6px',lineHeight:'20px'})};
  const get=document.getElementById,panel=get('filterPanel'),input=get('pidFilter');panel.append(input);
  const main=get('logsRows'),selected=new Element(document);main.append(selected);selected.focus();
  setup({get,panel,document});
  const layout=mountLogLayout({document,window});
  return {get,document,window,timers,writes,layout,selected,input,panel};
}

test('filter open/close allocates layout state without removing controls or changing selection/focus',()=>{
  const r=rig(),toggle=r.get('toggleFilters'),workspace=r.get('workspace');
  assert.equal(r.panel.inert,true);assert.equal(r.panel.attributes['aria-hidden'],'true');
  assert.equal(r.panel.hidden,false,'clipped grid slot owns animation, not display:none');
  assert.equal(toggle.attributes['aria-expanded'],'false');
  for(let i=0;i<5;i++){
    toggle.fire('click');assert.equal(workspace.classList.contains('filters-collapsed'),false);
    assert.equal(r.panel.inert,false);assert.equal(toggle.attributes['aria-expanded'],'true');
    assert.equal(r.panel.children[0],r.input);assert.equal(r.document.activeElement,r.selected);
    assert.equal(r.get('logsRows').children[0],r.selected);
    toggle.fire('click');assert.equal(workspace.classList.contains('filters-collapsed'),true);assert.equal(r.panel.inert,true);
  }
  assert.equal(r.timers.size,0,'rapid toggles have no stale animation callbacks');
  assert.equal(r.document.activeElement,r.selected);
});

test('automatic hiding remains in-layout and protects active filter controls',()=>{
  const r=rig();r.get('toggleFilters').fire('click');r.input.focus();
  assert.equal(r.get('workspace').classList.contains('filters-auto-hide'),true);
  r.panel.fire('pointerleave');const [id,timer]=[...r.timers][0];r.timers.delete(id);timer.fn();
  assert.equal(r.panel.inert,false,'typing inside filters does not hide the panel');
  r.selected.focus();r.document.listeners.get('focusin')({target:r.selected});
  assert.equal(r.panel.inert,true);assert.equal(r.document.activeElement,r.selected,'outside interaction keeps its focus');
  r.get('toggleFilters').fire('click');r.input.focus();r.get('collapseFilters').fire('click');
  assert.equal(r.document.activeElement,r.get('toggleFilters'),'explicit collapse returns focus out of inert controls');
});

test('a held auto-hide panel stays open for a tip and is left open when released',()=>{
  const r=rig();r.get('toggleFilters').fire('click');
  r.layout.hold(true);
  r.panel.fire('pointerleave');assert.equal(r.timers.size,0,'leaving the panel schedules no hide');
  r.document.listeners.get('pointerdown')({target:r.selected});r.document.listeners.get('focusin')({target:r.selected});
  assert.equal(r.panel.inert,false,'presses and focus outside leave it open');
  r.layout.hold(false);assert.equal(r.panel.inert,false);assert.equal(r.timers.size,0);
  r.document.listeners.get('pointerdown')({target:r.selected});r.window.fire('pointerup');
  const [id,timer]=[...r.timers][0];r.timers.delete(id);timer.fn();
  assert.equal(r.panel.inert,true,'the next press outside hides it as before');
});

test('a press outside hides the floating panel only once it and a double-click it begins are over',()=>{
  // Hiding moves the log area, which must not slide under the pointer.
  const r=rig();r.get('toggleFilters').fire('click');
  const press=()=>{r.document.listeners.get('pointerdown')({target:r.selected});r.document.listeners.get('focusin')({target:r.selected});};
  press();
  assert.equal(r.panel.inert,false,'not while the press is down, nor for the focus it moves');assert.equal(r.timers.size,0);
  r.window.fire('pointerup');
  assert.equal(r.panel.inert,false);assert.deepEqual([...r.timers.values()].map(t=>t.ms),[500]);
  // The second press of a double-click starts the wait again.
  press();assert.equal(r.timers.size,0);
  r.window.fire('pointerup');
  const [id,timer]=[...r.timers][0];r.timers.delete(id);timer.fn();
  assert.equal(r.panel.inert,true,'it hides once the presses settle');
  // A window losing focus during a press ends it too.
  r.get('toggleFilters').fire('click');press();r.window.fire('blur');
  const [next,later]=[...r.timers][0];r.timers.delete(next);later.fn();
  assert.equal(r.panel.inert,true);
  // Nor does a blur during the wait (a file dialog the click opened) keep it.
  r.get('toggleFilters').fire('click');press();r.window.fire('pointerup');r.window.fire('blur');
  assert.equal(r.timers.size,1,'the close still comes');
  const [blurred,close]=[...r.timers][0];r.timers.delete(blurred);close.fn();
  assert.equal(r.panel.inert,true);
  // A context menu, whose release may never come, ends the press.
  r.get('toggleFilters').fire('click');press();r.window.fire('contextmenu');
  assert.equal(r.timers.size,1);
  const [menu,shut]=[...r.timers][0];r.timers.delete(menu);shut.fn();assert.equal(r.panel.inert,true);
  // A release that lands in the panel keeps it.
  r.get('toggleFilters').fire('click');press();r.window.fire('pointerup',{type:'pointerup',target:r.input});
  assert.equal(r.timers.size,0);assert.equal(r.panel.inert,false);
  // Other buttons hide it at once, as before.
  r.document.listeners.get('pointerdown')({target:r.selected,button:2});
  assert.equal(r.panel.inert,true);
});

test('pinned panel preference preserves the same layout/focus behavior and Escape accessibility',()=>{
  const r=rig({autoHide:false,open:true});
  assert.equal(r.panel.inert,false);assert.equal(r.get('autoHideFilters').checked,false);
  r.input.focus();let prevented=false;r.document.listeners.get('keydown')({key:'Escape',preventDefault(){prevented=true;}});
  assert.equal(prevented,true);assert.equal(r.panel.inert,true);assert.equal(r.document.activeElement,r.get('toggleFilters'));
  r.get('autoHideFilters').checked=true;r.get('autoHideFilters').fire('change');
  assert.equal(r.panel.inert,true,'preference changes do not unexpectedly open or overlay the panel');
});


test('reading options use an initially closed keyboard-native disclosure and layout rerenders retain the setting',async()=>{
  const html=await readFile(new URL('../dist/index.html',import.meta.url),'utf8');
  assert.match(html,/<details class="parse-options" id="readingOptions"><summary>읽기 옵션<\/summary>/);
  assert.match(html,/<option value="auto" selected>자동 인식 \(기본\)<\/option>/);
  const r=rig(),options=r.get('readingOptions'),select=r.get('recognizeLineNumbers');options.append(select);r.panel.append(options);select.value='off';
  for(const open of [true,false,true]){options.open=open;r.get('toggleFilters').fire('click');r.get('autoHideFilters').fire('change');assert.equal(options.open,open);assert.equal(select.value,'off');assert.equal(options.children[0],select);}
});

test('a combined log widens the row column for its file badge without saving the extra width',()=>{
  const r=rig(),workspace=r.get('workspace'),header=r.get('logsHeaders');
  const label=()=>header.children[0].children[0].textContent;
  assert.equal(label(),'행');assert.equal(workspace.style['--line-column'],'52px');
  r.layout.setMultiFile(true);
  assert.equal(label(),'파일 · 행');assert.equal(workspace.style['--line-column'],'76px');
  assert.match(workspace.style['--columns'],/^76px 180px /);assert.equal(workspace.style['--table-min-width'],'732px');
  r.layout.resetColumns();
  assert.equal(r.writes.at(-1).widths[0],52);assert.equal(workspace.style['--line-column'],'76px');
  r.layout.setMultiFile(false);assert.equal(label(),'행');assert.equal(workspace.style['--line-column'],'52px');
});

test('one Escape closes one thing: claimed keys, focus inside, floating panel, then a pinned panel only on request',()=>{
  const escape=(r,defaultPrevented=false)=>{let prevented=false;r.document.listeners.get('keydown')({key:'Escape',defaultPrevented,preventDefault(){prevented=true;}});return prevented;};
  const pinned=rig({autoHide:false,open:true});
  assert.equal(escape(pinned,true),false);assert.equal(pinned.panel.inert,false,'a menu or drag that claimed Escape keeps the panel');
  assert.equal(escape(pinned),false,'focus outside a pinned panel leaves Escape to the raw detail');assert.equal(pinned.panel.inert,false);
  assert.equal(pinned.layout.closePanel(),true);assert.equal(pinned.panel.inert,true);assert.equal(pinned.document.activeElement,pinned.selected);
  assert.equal(pinned.layout.closePanel(),false,'nothing left to close');
  const floating=rig();floating.get('toggleFilters').fire('click');
  assert.equal(escape(floating),true);assert.equal(floating.panel.inert,true,'an auto-hide panel takes Escape from anywhere');
  assert.equal(floating.document.activeElement,floating.selected);
});

test('the filter rail is unavailable without a file and keeps the pinned preference for the next file',()=>{
  const r=rig({autoHide:false,open:true}),toggle=r.get('toggleFilters'),workspace=r.get('workspace');
  r.input.focus();
  assert.equal(r.layout.setAvailable(false),true,'focus inside the disabled panel is reported for the caller to move');
  assert.equal(toggle.disabled,true);assert.equal(r.panel.inert,true);assert.equal(toggle.attributes['aria-expanded'],'false');
  assert.equal(workspace.classList.contains('filters-collapsed'),true);assert.match(toggle.title,/파일을 열면/);
  toggle.fire('click');assert.equal(r.panel.inert,true,'a stray click cannot open it');
  assert.equal(r.layout.closePanel(),false);assert.equal(r.layout.setAvailable(false),false,'repeated renders are no-ops');
  assert.equal(r.writes.at(-1)?.open??true,true,'the saved open preference is untouched');
  r.selected.focus();assert.equal(r.layout.setAvailable(true),false);
  assert.equal(toggle.disabled,false);assert.equal(r.panel.inert,false,'a pinned panel reopens with the next file');
  assert.equal(r.document.activeElement,r.selected);
});

test('field help toggles keep the described note and its disclosure state in sync',()=>{
  let help;
  const r=rig(null,({get,panel,document})=>{help=new Element(document);help.className='help-toggle';help.setAttribute('aria-controls','timeFilterHelp');help.setAttribute('aria-expanded','false');panel.append(help);get('timeFilterHelp').hidden=true;});
  const note=r.get('timeFilterHelp');
  help.fire('click');assert.equal(help.attributes['aria-expanded'],'true');assert.equal(note.hidden,false);
  help.fire('click');assert.equal(help.attributes['aria-expanded'],'false');assert.equal(note.hidden,true);
});

const IOS='2026-09-30 21:55:54.351980',ANDROID='06-18 19:24:05.123';
// Upper rows as the logs table renders them: an iOS time cell also names the process.
function showRows(r,stamps){r.get('logsTable').querySelectorAll=()=>stamps.map(stamp=>({children:[{className:'line-number',textContent:'1'},stamp===null?{className:'raw-content',textContent:'x'.repeat(80)}:{className:'timestamp',textContent:stamp}]}));}
const stampWidth=r=>r.get('workspace').style['--columns'].split(' ')[1];

test('a new file widens the date/time column once to its first rendered timestamps',()=>{
  const r=rig();
  showRows(r,[]);r.layout.fitTimestamp(1);
  assert.equal(stampWidth(r),'180px','no rendered rows: wait for a later paint');
  showRows(r,[null,`${ANDROID}`,`${IOS} · SpringBoard`]);r.layout.fitTimestamp(1);
  assert.equal(stampWidth(r),'212px','fits the microseconds and the ellipsis before the process name, not the name');
  assert.equal(r.writes.at(-1).custom[1],false,'a fitted width is not a chosen one');
  showRows(r,[ANDROID]);r.layout.fitTimestamp(1);
  assert.equal(stampWidth(r),'212px','later paints of the same file keep the width');
  r.layout.fitTimestamp(2);
  assert.equal(stampWidth(r),'180px','the next file starts again from the default, never narrower');
  r.layout.setRawMode(true);showRows(r,[`${IOS} · SpringBoard`]);r.layout.fitTimestamp(3);
  assert.equal(stampWidth(r),'180px','raw rows have no time cell');
  r.layout.setRawMode(false);r.layout.fitTimestamp(3);
  assert.equal(stampWidth(r),'212px');
  // Without a process name the time alone decides.
  showRows(r,[IOS,'2026-06-18 19:24:03.000']);r.layout.fitTimestamp(4);
  assert.equal(stampWidth(r),'198px');
});

test('the date/time fit never replaces a width the user chose and returns with the defaults',()=>{
  const r=rig(),handle=r.get('logsHeaders').children[1].children[1];
  handle.fire('keydown',{key:'ArrowRight',preventDefault(){}});
  assert.equal(stampWidth(r),'188px');assert.equal(r.writes.at(-1).custom[1],true);
  showRows(r,[IOS]);r.layout.fitTimestamp(1);
  assert.equal(stampWidth(r),'188px');
  r.layout.resetColumns();
  assert.equal(stampWidth(r),'198px','defaults include the fit of the shown file');assert.equal(r.writes.at(-1).custom[1],false);
  handle.fire('keydown',{key:'ArrowLeft',preventDefault(){}});handle.fire('keydown',{key:'Home',preventDefault(){}});
  assert.equal(stampWidth(r),'198px','Home restores the default of this file');
  // Saves before the flags count a width other than the default as chosen.
  for(const [saved,width] of [[{widths:[52,220,56,56,76,128]},'220px'],[{widths:[52,180,56,56,76,128]},'198px'],[{widths:[52,200,56,56,76,128],custom:[false,false,false,false,false,false]},'198px']]){
    const old=rig(saved);showRows(old,[IOS]);old.layout.fitTimestamp(1);assert.equal(stampWidth(old),width,JSON.stringify(saved));
  }
});

test('column tools sit in one menu button that raw view hides',()=>{
  const r=rig();
  assert.equal(r.get('columnMenu').hidden,false);
  r.layout.setRawMode(true);assert.equal(r.get('columnMenu').hidden,true);
  r.layout.setRawMode(false);assert.equal(r.get('columnMenu').hidden,false);
  r.get('logsHeaders').children[2].children[1].fire('keydown',{key:'ArrowRight',preventDefault(){}});
  r.layout.fitColumns();
  assert.deepEqual(r.writes.at(-1).widths,[52,100,40,40,56,64],'nothing rendered: each column fits its minimum, the row column its automatic width');
  assert.deepEqual(r.writes.at(-1).custom,[false,true,true,true,true,true],'a fit is the user\'s choice; the row column keeps following the row count');
});

test('the widest row number is the row count, or the most rows of one file in a combined log',()=>{
  assert.equal(widestLine({total:1234567,files:[{firstLine:1}]}),1234567);
  assert.equal(widestLine({total:0}),0);assert.equal(widestLine(),0);
  const files=[{firstLine:1},{firstLine:120001},{firstLine:120001},{firstLine:150001}];
  assert.equal(widestLine({total:10150000,files}),10000000,'the file being indexed counts its rows so far; an empty file adds none');
  assert.equal(widestLine({total:200000,files:[{firstLine:1},{firstLine:150001},{firstLine:null}]}),150000,'files not reached yet are skipped');
});

test('the row column widens to the widest row number as the row count grows, shared by all tables',()=>{
  const r=rig(),workspace=r.get('workspace');let reads=0;
  const lineWidth=()=>workspace.style['--line-column'];
  r.layout.fitLines(12345678);assert.equal(lineWidth(),'52px','nothing rendered yet: a later paint measures');
  r.get('logsTable').querySelectorAll=selector=>{reads++;return selector==='.log-row'?[{children:[{className:'line-number',textContent:'1'}]}]:[];};
  r.layout.fitLines();assert.equal(lineWidth(),'70px','eight 7px digits, bold, plus padding');
  for(const [total,width] of [[99999,'52px'],[100000,'56px'],[999999,'56px'],[1000000,'63px'],[9999999999,'84px']]){r.layout.fitLines(total);assert.equal(lineWidth(),width,String(total));}
  assert.match(workspace.style['--columns'],/^84px 180px /);
  const before=reads;r.layout.fitLines(5000000000);r.layout.fitLines();assert.equal(reads,before,'the same digit count is not measured again');
  r.layout.fitLines(812);assert.equal(lineWidth(),'52px','a smaller next log returns to the default, never narrower');
  r.layout.setMultiFile(true);r.layout.fitLines(widestLine({total:12000000,files:[{firstLine:1},{firstLine:5000001}]}));
  assert.equal(lineWidth(),'87px','the file badge track adds to the fitted number width');
  assert.equal(r.writes.length?r.writes.at(-1).widths[0]:52,52,'an automatic width is not saved as a choice');
});

test('a row column width the user chose stays until defaults, Home or a fit hand it back',()=>{
  const r=rig(),workspace=r.get('workspace'),handle=r.get('logsHeaders').children[0].children[1];
  const lineWidth=()=>workspace.style['--line-column'];
  r.get('logsTable').querySelectorAll=selector=>selector==='.log-row'?[{children:[{className:'line-number',textContent:'1'}]}]:[];
  handle.fire('keydown',{key:'ArrowLeft',preventDefault(){}});assert.equal(lineWidth(),'44px');assert.equal(r.writes.at(-1).custom[0],true);
  r.layout.fitLines(10000000);assert.equal(lineWidth(),'44px','a chosen width is kept');
  handle.fire('keydown',{key:'Home',preventDefault(){}});assert.equal(lineWidth(),'70px');assert.equal(r.writes.at(-1).custom[0],false);
  handle.fire('keydown',{key:'ArrowRight',preventDefault(){}});assert.equal(lineWidth(),'78px');
  r.layout.resetColumns();assert.equal(lineWidth(),'70px','defaults include the row count of the shown log');
  handle.fire('keydown',{key:'ArrowRight',preventDefault(){}});handle.fire('dblclick');
  assert.equal(lineWidth(),'70px','a fit of the row column means the automatic width, not the visible rows');
  handle.fire('keydown',{key:'ArrowRight',preventDefault(){}});
  // Reopening keeps a chosen width; an automatic one starts from the default.
  assert.equal(workspace.style['--line-column'],'78px');
  const chosen=rig(r.writes.at(-1));assert.equal(chosen.get('workspace').style['--line-column'],'78px');
  const auto=rig({widths:[70,180,56,56,76,128],custom:[false,false,false,false,false,false]});assert.equal(auto.get('workspace').style['--line-column'],'52px');
});

test('the lower region splitter starts from the shown height, not only the saved share',()=>{
  const r=rig({lowerRatio:.42}),splitter=r.get('paneSplitter'),workspace=r.get('workspace');
  // The region shows its share of the log area, but never less than `least`
  // (its 150-pixel minimum, or room for results with a detail) nor more than
  // `most` (what the upper logs' minimum leaves).
  let total=300,least=150,most=300;
  r.get('logContent').getBoundingClientRect=()=>({height:total});
  r.get('largeLowerPane').getBoundingClientRect=()=>({height:Math.min(most,Math.max(least,Number(workspace.style['--lower-ratio'])*total))});
  // The 150-pixel minimum is taller than a short log area's 42% share.
  splitter.fire('pointerdown',{button:0,clientY:500,pointerId:1,preventDefault(){}});
  splitter.fire('pointermove',{clientY:470,pointerId:1});
  assert.equal(workspace.style['--lower-ratio'],'0.6','dragging up grows it from what is shown');
  splitter.fire('pointerup',{pointerId:1});assert.equal(r.writes.at(-1).lowerRatio,0.6);
  most=210;
  splitter.fire('pointerdown',{button:0,clientY:500,pointerId:2,preventDefault(){}});
  splitter.fire('pointermove',{clientY:300,pointerId:2});
  assert.equal(workspace.style['--lower-ratio'],'0.7','a share the upper logs leave no room for is not kept');
  assert.equal(splitter.attributes['aria-valuenow'],'70');
  splitter.fire('pointerup',{pointerId:2});assert.equal(r.writes.at(-1).lowerRatio,0.7);
  total=1000;least=400;most=890;
  splitter.fire('keydown',{key:'Home',preventDefault(){}});assert.equal(workspace.style['--lower-ratio'],'0.42');
  splitter.fire('keydown',{key:'ArrowDown',preventDefault(){}});assert.equal(workspace.style['--lower-ratio'],'0.39');
  splitter.fire('keydown',{key:'ArrowDown',preventDefault(){}});
  assert.equal(workspace.style['--lower-ratio'],'0.37','results with a detail keep 40%: keys start from it, not from the saved 39%');
  splitter.fire('pointerdown',{button:0,clientY:500,pointerId:3,preventDefault(){}});
  splitter.fire('pointermove',{clientY:300,pointerId:3});assert.equal(workspace.style['--lower-ratio'],'0.6');
  splitter.fire('pointercancel',{pointerId:3});
  assert.equal(workspace.style['--lower-ratio'],'0.37','a cancelled drag restores the saved share');
  splitter.fire('dblclick');assert.equal(workspace.style['--lower-ratio'],'0.42');
});

test('the raw detail shows the whole lines its own splitter sets, one by default, saved apart from the lower region',()=>{
  const r=rig({lowerRatio:.5}),splitter=r.get('detailSplitter'),workspace=r.get('workspace');
  assert.equal(workspace.style['--detail-lines'],'1');assert.equal(splitter.attributes['aria-valuetext'],'원문 상세 1줄');
  // The rig's detail lines are 20 pixels; other regions' minimums leave room
  // for at most `room` pixels of text.
  let room=Infinity;
  r.get('largeDetailText').getBoundingClientRect=()=>({height:Math.min(room,Number(workspace.style['--detail-lines'])*20)});
  const drag=(id,to,end='pointerup')=>{splitter.fire('pointerdown',{button:0,clientY:500,pointerId:id,preventDefault(){}});splitter.fire('pointermove',{clientY:to,pointerId:id});const lines=workspace.style['--detail-lines'];splitter.fire(end,{pointerId:id});return lines;};
  const key=(name,shiftKey=false)=>{splitter.fire('keydown',{key:name,shiftKey,preventDefault(){}});return workspace.style['--detail-lines'];};
  splitter.fire('pointerdown',{button:0,clientY:500,pointerId:9,preventDefault(){}});
  assert.deepEqual([splitter.classList.contains('dragging'),r.get('paneSplitter').classList.contains('dragging')],[true,false],'only the dragged splitter is marked');
  splitter.fire('pointercancel',{pointerId:9});assert.equal(splitter.classList.contains('dragging'),false);
  assert.equal(drag(1,445),'4','dragging up adds whole lines');
  assert.deepEqual([r.writes.at(-1).detailLines,r.writes.at(-1).lowerRatio],[4,0.5],'the lower region keeps its share');
  assert.equal(workspace.style['--lower-ratio'],'0.5');assert.equal(splitter.attributes['aria-valuenow'],'4');
  // The results' two rows leave room for three of the four lines.
  room=60;
  assert.equal(key('ArrowDown'),'2','keys step from the shown lines');
  assert.equal(key('ArrowUp',true),'3','lines there is no room for are not kept');
  room=Infinity;
  assert.equal(key('ArrowUp',true),'8','Shift steps five lines');
  room=100;
  assert.equal(drag(2,-5000),'5','a drag keeps no more lines than show');
  assert.equal(r.writes.at(-1).detailLines,5);assert.equal(splitter.attributes['aria-valuetext'],'원문 상세 5줄');
  room=Infinity;
  assert.equal(drag(3,400,'pointercancel'),'10');assert.equal(workspace.style['--detail-lines'],'5','a cancelled drag restores the saved lines');
  assert.equal(drag(4,-5000),'100','a drag stops at the most lines');
  // A double-click or Enter fits the current text, as far as there is room.
  r.get('largeDetailText').scrollHeight=7*20;
  splitter.fire('dblclick');assert.equal(workspace.style['--detail-lines'],'7','a double-click shows the whole text');assert.equal(r.writes.at(-1).detailLines,7);
  assert.equal(key('Home'),'1','Home goes back to one line');
  room=100;
  assert.equal(key('Enter'),'5','Enter fits the text as far as there is room');assert.equal(r.writes.at(-1).detailLines,5);
  room=Infinity;r.layout.fitDetail();assert.equal(workspace.style['--detail-lines'],'7','the more-lines button fits the same way');
  assert.equal(key('ArrowUp'),'8');assert.equal(key('Home'),'1');
  for(const [value,shown] of [[500,'100'],[2.6,'3'],[0,'1'],['many','1']])assert.equal(rig({detailLines:value}).get('workspace').style['--detail-lines'],shown,`saved ${value}`);
});

test('a DLT log shows its own columns and keeps their widths apart from text logs',()=>{
  const r=rig({widths:[52,220,56,56,76,128],dlt:{widths:[52,240,104,56,56,56,76]}}),workspace=r.get('workspace');
  const labels=()=>r.get('logsHeaders').children.map(cell=>cell.children[0].textContent);
  assert.deepEqual(labels(),['행','날짜 · 시간','PID','TID','레벨','태그','메시지']);
  r.layout.setProfile('dlt');
  assert.deepEqual(labels(),['행','시간','타임스탬프','ECU','APID','CTID','레벨','페이로드']);
  assert.match(workspace.style['--columns'],/^52px 240px 104px 56px 56px 56px 76px minmax/);
  r.get('logsHeaders').children[3].children[1].fire('keydown',{key:'ArrowRight',preventDefault(){}});
  assert.equal(r.writes.at(-1).dlt.widths[3],64);
  assert.deepEqual(r.writes.at(-1).widths,[52,220,56,56,76,128],'text widths are untouched');
  r.layout.setProfile('text');
  assert.deepEqual(labels(),['행','날짜 · 시간','PID','TID','레벨','태그','메시지']);
  assert.match(workspace.style['--columns'],/^52px 220px 56px 56px 76px 128px minmax/);
});
