import test from 'node:test';
import assert from 'node:assert/strict';
import {mountMoreMenu} from '../dist/more-menu.js';

class Element {
  constructor(document){this.document=document;this.hidden=false;this.attributes={};this.listeners={};this.style={};this.value='';this.selected=0;}
  addEventListener(key,fn){(this.listeners[key]??=[]).push(fn);}
  fire(key,event={}){for(const fn of this.listeners[key]??[])fn({preventDefault(){},stopPropagation(){},...event});}
  setAttribute(key,value){this.attributes[key]=String(value);}
  removeAttribute(key){delete this.attributes[key];}
  contains(node){return node===this||node?.parent===this;}
  focus(){this.document.activeElement=this;}
  select(){this.selected++;}
  closest(){return null;}
  getBoundingClientRect(){return {left:760,right:800,bottom:40};}
}
function rig(){
  const document={activeElement:null},listeners={},shown=[];let version=0;
  const window={innerWidth:900,addEventListener(key,fn){(listeners[key]??=[]).push(fn);}};
  const element=()=>new Element(document);
  const ui={moreButton:element(),rowMenu:element(),logsJumpForm:element(),logsJump:element(),jumpError:element()};
  ui.logsJump.parent=ui.logsJumpForm;ui.rowMenu.hidden=true;ui.logsJumpForm.hidden=true;
  const menu={show(items,x,y,label,options){ui.rowMenu.hidden=false;shown.push({items,x,y,label,options});return ++version;},version:()=>version};
  const more=mountMoreMenu({ui,menu,items:()=>[['다시 열기',()=>{}]],document,window});
  const press=target=>{for(const fn of listeners.pointerdown)fn({target});};
  return {ui,menu,more,shown,press,document,window,resize:()=>listeners.resize.forEach(fn=>fn()),closeMenu(){ui.rowMenu.hidden=true;version++;}};
}

test('the menu opens under its button, aligned to its right edge',()=>{
  const r=rig();
  r.ui.moreButton.fire('click');
  const {options,...opened}=r.shown[0];
  assert.deepEqual(opened,{items:[['다시 열기',opened.items[0][1]]],x:800,y:44,label:'메뉴'});
  assert.equal(options.alignEnd,true);assert.equal(r.ui.moreButton.attributes['aria-expanded'],'true');
  options.onClose();assert.equal(r.ui.moreButton.attributes['aria-expanded'],'false');
});

test('the press that closes the open menu does not open it again',()=>{
  const r=rig(),button=r.ui.moreButton;
  button.fire('click');assert.equal(r.shown.length,1);
  r.press(button);r.closeMenu();button.fire('click');
  assert.equal(r.shown.length,1,'closed by that press');
  r.press(button);button.fire('click');
  assert.equal(r.shown.length,2,'the next press opens it');
  // Another menu on screen is closed by the press; this one still opens.
  r.closeMenu();r.ui.rowMenu.hidden=false;r.press(button);r.ui.rowMenu.hidden=true;button.fire('click');
  assert.equal(r.shown.length,3);
});

test('a press released off the button does not swallow the next key',()=>{
  const r=rig(),button=r.ui.moreButton;
  button.fire('click');r.press(button);r.closeMenu();
  button.fire('click',{detail:0});
  assert.equal(r.shown.length,2,'Enter on the button opens it');
});

test('more choices take the menu\'s place, with the way back to it first',()=>{
  const r=rig();
  r.more.sub('열 폭',[['폭 초기화',()=>{}]]);
  const {items,label}=r.shown[0];
  assert.equal(label,'열 폭');
  assert.deepEqual(items.map(item=>item?.[0]??null),['‹ 메뉴',null,'폭 초기화']);
  assert.equal(items[0][3].back,true);
  items[0][1]();
  assert.equal(r.shown[1].label,'메뉴');
});

test('the row box opens where the menu does and returns focus unless a row was chosen',()=>{
  const r=rig(),{logsJumpForm:form,logsJump:input}=r.ui,row=new Element(r.document);
  row.focus();r.more.openJump();
  assert.equal(form.hidden,false);assert.equal(r.document.activeElement,input);assert.equal(input.selected,1);
  assert.deepEqual(form.style,{right:'100px',top:'44px'});
  form.fire('keydown',{key:'a'});assert.equal(form.hidden,false);
  let stopped=false;form.fire('keydown',{key:'Escape',stopPropagation(){stopped=true;}});
  assert.equal(form.hidden,true);assert.equal(stopped,true,'one press closes one thing');assert.equal(r.document.activeElement,row);
  r.more.openJump();input.setAttribute('aria-invalid','true');r.ui.jumpError.textContent='마지막 행은 30행입니다.';r.more.closeJump();
  assert.equal(form.hidden,true);assert.equal(input.attributes['aria-invalid'],undefined);assert.equal(r.ui.jumpError.textContent,'');assert.equal(r.document.activeElement,input,'the caller moves focus');
});

test('with nothing focused before it, the row box returns focus to its button',()=>{
  const r=rig(),{logsJumpForm:form,logsJump:input}=r.ui;
  r.document.body=new Element(r.document);r.document.body.focus();
  r.more.openJump();form.fire('keydown',{key:'Escape'});
  assert.equal(r.document.activeElement,r.ui.moreButton);
  r.document.body.focus();r.more.openJump();r.resize();
  assert.equal(form.hidden,true);assert.equal(r.document.activeElement,r.ui.moreButton,'a resize does not drop focus');
  assert.notEqual(r.document.activeElement,input);
});

test('a row gone from the list while the box was open gives way to the button',()=>{
  const r=rig(),row=new Element(r.document);
  row.focus();r.more.openJump();row.isConnected=false;
  r.ui.logsJumpForm.fire('keydown',{key:'Escape'});
  assert.equal(r.document.activeElement,r.ui.moreButton);
});

test('the row box closes on a press or focus elsewhere and on resize, not on its own controls',()=>{
  const r=rig(),{logsJumpForm:form,logsJump:input}=r.ui,other=new Element(r.document);
  r.more.openJump();r.press(input);assert.equal(form.hidden,false);
  form.fire('focusout',{relatedTarget:input});assert.equal(form.hidden,false);
  form.fire('focusout',{relatedTarget:null});assert.equal(form.hidden,false,'focus leaving the window keeps it');
  r.press(other);assert.equal(form.hidden,true);
  r.more.openJump();form.fire('focusout',{relatedTarget:other});assert.equal(form.hidden,true);
  r.more.openJump();r.resize();assert.equal(form.hidden,true);
});
