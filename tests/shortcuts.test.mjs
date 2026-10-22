import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {mountShortcutHelp} from '../dist/shortcuts.js';

class Element {
  constructor(document,tag='div'){this.ownerDocument=document;this.tagName=tag.toUpperCase();this.listeners=new Map();this.attributes={};this.isConnected=true;}
  setAttribute(key,value){this.attributes[key]=String(value);}
  addEventListener(name,fn){(this.listeners.get(name)??this.listeners.set(name,[]).get(name)).push(fn);}
  fire(name,event={}){let prevented=false,stopped=false;const full={target:this,preventDefault(){prevented=true;},stopPropagation(){stopped=true;},...event};for(const fn of this.listeners.get(name)??[])fn(full);return {prevented,stopped};}
  focus(){this.ownerDocument.activeElement=this;}
  closest(selector){return selector.includes('input')&&this.tagName==='INPUT'?this:null;}
}
// A native modal dialog: close() fires `close`, as the browser does.
class Dialog extends Element {
  constructor(document){super(document,'dialog');this.open=false;}
  showModal(){this.open=true;}
  close(){if(!this.open)return;this.open=false;this.fire('close');}
}
function rig(){
  const document={activeElement:null,listeners:new Map(),addEventListener(name,fn){this.listeners.set(name,fn);}};
  const elements={shortcutHelp:new Element(document,'button'),shortcutDialog:new Dialog(document),shortcutClose:new Element(document,'button')};
  document.getElementById=id=>elements[id]??null;
  const help=mountShortcutHelp({document});
  const key=(event,target=document.activeElement??new Element(document))=>{let prevented=false;document.listeners.get('keydown')({target,preventDefault(){prevented=true;},...event});return prevented;};
  return {document,help,key,...elements};
}

test('the help button and `?` open the dialog and every close returns focus to the opener',()=>{
  const r=rig(),row=new Element(r.document);
  assert.equal(r.shortcutHelp.attributes['aria-expanded'],'false');
  r.shortcutHelp.focus();r.shortcutHelp.fire('click');
  assert.equal(r.shortcutDialog.open,true);assert.equal(r.shortcutHelp.attributes['aria-expanded'],'true');
  assert.equal(r.document.activeElement,r.shortcutClose,'focus moves into the dialog');
  r.shortcutClose.fire('click');
  assert.equal(r.shortcutDialog.open,false);assert.equal(r.document.activeElement,r.shortcutHelp);assert.equal(r.shortcutHelp.attributes['aria-expanded'],'false');
  row.focus();assert.equal(r.key({key:'?'}),true);assert.equal(r.shortcutDialog.open,true);
  // Escape stops at the dialog so the raw detail and the filter panel stay open.
  const escape=r.shortcutDialog.fire('keydown',{key:'Escape'});
  assert.deepEqual(escape,{prevented:true,stopped:true});assert.equal(r.shortcutDialog.open,false);assert.equal(r.document.activeElement,row);
  row.focus();r.key({key:'?'});
  r.shortcutDialog.fire('click',{target:r.shortcutClose});assert.equal(r.shortcutDialog.open,true,'a press inside the content keeps it');
  r.shortcutDialog.fire('click');assert.equal(r.shortcutDialog.open,false,'a press on the backdrop closes it');assert.equal(r.document.activeElement,row);
  row.focus();r.key({key:'?'});row.isConnected=false;r.shortcutDialog.fire('cancel');
  assert.equal(r.shortcutDialog.open,false);assert.equal(r.document.activeElement,r.shortcutHelp,'a removed opener falls back to the help button');
});

test('`?` types normally in text fields and leaves modified keys alone',()=>{
  const r=rig(),input=new Element(r.document,'input');
  input.focus();assert.equal(r.key({key:'?'}),false);assert.equal(r.shortcutDialog.open,false);
  r.document.activeElement=null;
  for(const event of [{key:'?',metaKey:true},{key:'?',ctrlKey:true},{key:'?',altKey:true},{key:'?',isComposing:true},{key:'?',defaultPrevented:true},{key:'/'}])
    assert.equal(r.key(event),false,JSON.stringify(event));
  assert.equal(r.shortcutDialog.open,false);
});

test('the dialog lists the shortcuts the views handle, with the platform modifier key',async()=>{
  const html=await readFile(new URL('../dist/index.html',import.meta.url),'utf8');
  const dialog=html.slice(html.indexOf('<dialog id="shortcutDialog"'),html.indexOf('</dialog>'));
  const rows=[...dialog.matchAll(/<div><dt>(.*?)<\/dt><dd>(.*?)<\/dd><\/div>/g)].map(([,keys,text])=>[keys.replace(/<kbd class="mod-key">⌘<\/kbd>/g,'Mod+').replace(/<[^>]+>/g,'').replace(/\s+/g,' ').trim(),text.replace(/<[^>]+>/g,'')]);
  const keys=rows.map(([keys])=>keys);
  for(const expected of ['Mod+O','Mod+V','Mod+F /','Enter ShiftEnter','↑↓','Shift↑↓ Shift+클릭 · 드래그','Mod++클릭','Mod+A','PageUpPageDown','HomeEnd','Mod+C','ShiftF10 우클릭','Esc','Mod+B','F2 ShiftF2','Delete','Mod++ Mod+−','Mod+0','?'])
    assert.ok(keys.includes(expected),`${expected} in ${JSON.stringify(keys)}`);
  assert.match(rows.find(([keys])=>keys==='Mod++ Mod+−')[1],/⌘\+마우스 휠/);
  // Mod keys are rewritten by the view at mount, like the other `.mod-key` hints.
  assert.doesNotMatch(dialog,/Ctrl/);
  for(const source of ['large-view.js','zoom.js','bookmarks.js','continuous-view.js']){
    const code=await readFile(new URL(`../dist/${source}`,import.meta.url),'utf8');
    for(const handled of {'large-view.js':["key==='o'","key==='f'","key==='b'","key==='a'","'F2'","event.key==='/'","'F10'","'Escape'"],'zoom.js':["'Digit0'","onWheel"],'bookmarks.js':["'Delete'"],'continuous-view.js':["PageDown","'Home'","extend:event.shiftKey"]}[source])
      assert.ok(code.includes(handled),`${source} handles ${handled}`);
  }
});
