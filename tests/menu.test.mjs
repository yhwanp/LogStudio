import test from 'node:test';
import assert from 'node:assert/strict';
import {createPopupMenu} from '../dist/menu.js';
import {focusOwner} from '../dist/dom.js';
import {mountMoreMenu} from '../dist/more-menu.js';

class Element {
  constructor(document, tag='div') {this.document = document; this.tagName = tag.toUpperCase(); this.children = []; this.attributes = {}; this.listeners = {}; this.style = {}; this.dataset = {}; this.hidden = true; this._text = ''; this.className = '';}
  get textContent() {return this._text + this.children.map(c => typeof c === 'string' ? c : c?.textContent ?? '').join('');}
  set textContent(value) {this._text = String(value); this.children = [];}
  append(...nodes) {this.children.push(...nodes);}
  setAttribute(key, value) {this.attributes[key] = String(value);}
  removeAttribute(key) {delete this.attributes[key];}
  getAttribute(key) {return this.attributes[key] ?? null;}
  replaceChildren(...nodes) {this.children = nodes;}
  addEventListener(key, fn) {(this.listeners[key] ??= []).push(fn);}
  fire(key, event={}) {for (const fn of this.listeners[key] ?? []) fn({preventDefault() {}, stopPropagation() {}, ...event});}
  focus() {this.document.activeElement = this;}
  contains(node) {return node === this || this.children.includes(node);}
  getBoundingClientRect() {return {width:120, height:80};}
  click() {this.fire('click');}
  // Only the selectors the menu uses.
  querySelector(selector) {return this.querySelectorAll(selector)[0] ?? null;}
  querySelectorAll(selector) {
    if (selector === '[aria-checked=true]') return this.children.filter(c => c.attributes['aria-checked'] === 'true');
    if (selector === '[role^=menuitem]') return this.children.filter(c => c.attributes.role?.startsWith('menuitem'));
    if (selector === '[role^=menuitem]:not([aria-disabled=true]):not([data-back])') return this.children.filter(c => c.attributes.role?.startsWith('menuitem') && c.attributes['aria-disabled'] !== 'true' && !c.dataset.back);
    if (selector === '.menu-note') return this.children.filter(c => c?.className?.split(' ').includes('menu-note'));
    return [];
  }
}

function rig() {
  const document = {activeElement:null, listeners:{}, createElement(tag) {return new Element(document, tag);}, addEventListener(key, fn) {this.listeners[key] = fn;}};
  const listeners = {}, window = {innerWidth:800, innerHeight:600, addEventListener(key, fn) {(listeners[key] ??= []).push(fn);}};
  const menu = new Element(document), opener = new Element(document, 'button');
  opener.focus();
  return {document, menu, opener, popup:createPopupMenu({menu, document, window}), fireWindow:key => listeners[key]?.forEach(fn => fn())};
}

test('notes are plain text, a checked choice is a radio item focused first', () => {
  const r = rig(), picked = [];
  r.popup.show([['A', () => picked.push('A'), 'tip', {checked:false}], ['B', () => picked.push('B'), '', {checked:true}], ['note only', null], null, ['Save', () => picked.push('save')]], 10, 20, 'Saved');
  const [a, b, note, line, save] = r.menu.children;
  assert.equal(r.menu.hidden, false); assert.equal(r.menu.attributes['aria-label'], 'Saved');
  assert.deepEqual([a.attributes.role, a.attributes['aria-checked'], a.title], ['menuitemradio', 'false', 'tip']);
  assert.deepEqual([b.attributes.role, b.attributes['aria-checked']], ['menuitemradio', 'true']);
  assert.deepEqual([note.tagName, note.className, note.textContent, note.attributes.role], ['P', 'menu-note', 'note only', undefined]);
  assert.equal(r.menu.attributes['aria-describedby'], note.id, 'notes describe the menu, as items cannot');
  assert.equal(line.attributes.role, 'separator');
  assert.deepEqual([save.attributes.role, 'aria-checked' in save.attributes], ['menuitem', false]);
  assert.equal(r.document.activeElement, b, 'the current choice has focus');
  r.menu.fire('keydown', {key:'ArrowDown'});
  assert.equal(r.document.activeElement, save, 'arrow keys skip notes');
  r.menu.fire('keydown', {key:'ArrowDown'});
  assert.equal(r.document.activeElement, a);
  a.click();
  assert.deepEqual(picked, ['A']); assert.equal(r.menu.hidden, true); assert.equal(r.document.activeElement, r.opener);
});

test('a menu of notes only takes focus itself so Escape closes it', () => {
  const r = rig();
  r.popup.show([['Nothing saved yet', null]], 0, 0, 'Saved');
  assert.equal(r.document.activeElement, r.menu); assert.equal(r.menu.tabIndex, -1);
  r.menu.fire('keydown', {key:'Escape'});
  assert.equal(r.menu.hidden, true); assert.equal(r.document.activeElement, r.opener);
});

test('a menu shown again while open returns focus to its first opener', () => {
  // The device menu shows "looking…" and then the devices it found.
  const r = rig();
  r.popup.show([['Looking…', null]], 0, 0, 'Devices');
  assert.equal(r.document.activeElement, r.menu);
  r.popup.show([['Pixel 8', () => {}]], 0, 0, 'Devices');
  r.menu.fire('keydown', {key:'Escape'});
  assert.equal(r.document.activeElement, r.opener);
});

test('an item that cannot be chosen is reached but does nothing', () => {
  const r = rig(), picked = [];
  r.popup.show([['Tablet · allow USB debugging', null, undefined, {disabled:true}], ['Pixel 8', () => picked.push('pixel')]], 0, 0, 'Devices');
  const [tablet, pixel] = r.menu.children;
  assert.deepEqual([tablet.tagName, tablet.attributes.role, tablet.attributes['aria-disabled']], ['BUTTON', 'menuitem', 'true']);
  assert.equal(r.document.activeElement, pixel, 'focus starts on an item that can be chosen');
  r.menu.fire('keydown', {key:'ArrowDown'});
  assert.equal(r.document.activeElement, tablet, 'arrow keys reach it');
  tablet.click();
  assert.deepEqual(picked, []); assert.equal(r.menu.hidden, false);
});

test('a device item keeps what follows its name, and a second line says what it needs', () => {
  const r = rig();
  r.popup.show([['Android', null, undefined, {group:true}], ['Pixel 8', () => {}, 'tip', {detail:'emulator-5554', key:'emulator-5554'}],
    ['QA iPad', null, undefined, {disabled:true, detail:'iPadOS 27.0', note:'Trust this computer', key:'U2'}]], 0, 0, 'Devices');
  const [group, pixel, ipad] = r.menu.children;
  assert.equal(group.className, 'menu-note menu-group');
  assert.equal(pixel.className, 'menu-rich');
  assert.equal(pixel.textContent, 'Pixel 8 · emulator-5554', 'the parts read apart');
  assert.equal(ipad.textContent, 'QA iPad · iPadOS 27.0 Trust this computer');
  assert.deepEqual([pixel.dataset.key, ipad.attributes['aria-disabled']], ['emulator-5554', 'true']);
});

test('a menu shown again while open keeps focus on the item of the same key', () => {
  // The device menu lists devices again while one waits on the person.
  const r = rig();
  const items = trusted => [['Pixel 8', () => {}, '', {key:'P'}], trusted ? ['QA iPad', () => {}, '', {key:'U2'}] : ['QA iPad', null, undefined, {disabled:true, note:'Trust', key:'U2'}]];
  r.popup.show(items(false), 0, 0, 'Devices');
  r.menu.fire('keydown', {key:'ArrowDown'});
  assert.equal(r.document.activeElement.dataset.key, 'U2');
  r.popup.show(items(true), 0, 0, 'Devices');
  const ipad = r.menu.children[1];
  assert.equal(r.document.activeElement, ipad, 'focus stays on the iPad, now choosable');
  r.menu.fire('keydown', {key:'Escape'});
  assert.equal(r.document.activeElement, r.opener);
});

test('a list the user scrolls closes a menu on its rows, not one from a button', () => {
  const r = rig();
  r.popup.show([['Pixel 8', () => {}]], 10, 20, '기기 로그');
  r.popup.moved();
  assert.equal(r.menu.hidden, false);
  r.popup.show([['복사', () => {}]], 10, 20, '행 작업', {onList:true});
  r.popup.moved();
  assert.equal(r.menu.hidden, true);
  assert.equal(r.document.activeElement, r.opener, 'focus goes back to the list it was opened on');
  // The next menu from a button stays again.
  r.popup.show([['Pixel 8', () => {}]], 10, 20, '기기 로그');
  r.popup.moved();
  assert.equal(r.menu.hidden, false);
});

test('focus moving out of the menu closes it, as a shortcut to the search box does', () => {
  const r = rig(), search = new Element(r.document, 'input');
  r.popup.show([['복사', () => {}]], 0, 0, '행 작업', {onList:true});
  r.document.listeners.focusin({target:r.menu.children[0]});
  assert.equal(r.menu.hidden, false, 'focus inside keeps it');
  search.focus();
  r.document.listeners.focusin({target:search});
  assert.equal(r.menu.hidden, true);
  assert.equal(r.document.activeElement, search, 'focus stays where it went');
});

test('onClose runs once when the menu closes or is replaced, after a chosen item acts', () => {
  const r = rig(), heard = [];
  const open = name => r.popup.show([['복사', () => heard.push('copied')]], 0, 0, '행 작업', {onList:true, onClose:press => heard.push(press ? `${name}:${press.type}` : name)});
  open('chosen');
  r.menu.children[0].click();
  open('escape');
  r.menu.fire('keydown', {key:'Escape'});
  // An outside press is passed on, so the owner can wait for it to end.
  open('outside');
  r.document.listeners.pointerdown({type:'pointerdown', target:r.opener});
  open('scrolled');
  r.popup.moved();
  // A menu shown in its place is a new menu; a later close tells only it.
  open('replaced');
  open('closed');
  r.popup.close();
  r.popup.close();
  assert.deepEqual(heard, ['copied', 'chosen', 'escape', 'outside:pointerdown', 'scrolled', 'replaced', 'closed']);
});

test('an open menu moves with what it was opened on, inside the window', () => {
  const r = rig();
  r.popup.show([['복사', () => {}]], 10, 100, '행 작업', {onList:true});
  r.popup.moveBy(-60);
  assert.equal(r.menu.style.top, '40px');
  r.popup.moveBy(-60);
  assert.equal(r.menu.style.top, '4px', 'it stays inside the window');
  r.popup.close();
  r.popup.moveBy(30);
  assert.equal(r.menu.style.top, '4px', 'a closed menu stays put');
});

test('every show and close changes the version, so a late answer can tell its menu was closed or replaced', () => {
  const r = rig();
  const shown = r.popup.show([['Looking…', null]], 0, 0, 'Devices');
  assert.equal(r.popup.version(), shown);
  assert.equal(r.menu.popupOpener, r.opener, 'the open menu names its opener');
  const again = r.popup.show([['Pixel 8', () => {}]], 0, 0, 'Devices');
  assert.notEqual(again, shown, 'a menu shown in its place is another');
  r.popup.close();
  assert.notEqual(r.popup.version(), again);
  assert.equal(r.menu.popupOpener, null);
});

test('a dialog opened from inside a popup menu returns focus to the menu opener', () => {
  const opener = {}, popup = {popupOpener:opener}, item = {closest:selector => selector === '[role=menu]' ? popup : null}, field = {closest:() => null};
  assert.equal(focusOwner({activeElement:item}), opener);
  assert.equal(focusOwner({activeElement:field}), field);
  assert.equal(focusOwner({activeElement:null}), null);
});

test('an item can show a key, open more choices and lead back; Right and Left Arrows choose those', () => {
  const r = rig(), picked = [];
  r.popup.show([['Reopen', () => picked.push('reopen'), '', {shortcut:'⌘R'}], ['Recent', () => picked.push('recent'), '', {opens:true}]], 10, 20, 'Menu');
  const [reopen, recent] = r.menu.children;
  assert.equal(reopen.textContent, 'Reopen ⌘R');
  assert.equal(recent.attributes['aria-haspopup'], 'menu'); assert.equal(recent.textContent, 'Recent ›');
  r.menu.fire('keydown', {key:'ArrowRight'});
  assert.deepEqual(picked, [], 'an item without more choices ignores Right Arrow');
  r.menu.fire('keydown', {key:'ArrowDown'});
  r.menu.fire('keydown', {key:'ArrowRight'});
  assert.deepEqual(picked, ['recent']); assert.equal(r.menu.hidden, true);
  r.popup.show([['Back', () => picked.push('back'), '', {back:true}], ['a.log', () => {}]], 10, 20, 'Recent');
  r.menu.fire('keydown', {key:'ArrowLeft'});
  assert.deepEqual(picked, ['recent', 'back']);
});

test('alignEnd puts the menu\'s right edge at x', () => {
  const r = rig();
  r.popup.show([['A', () => {}]], 500, 20, 'Menu', {alignEnd:true});
  assert.equal(r.menu.style.left, '380px');
  r.popup.close();
  r.popup.show([['A', () => {}]], 500, 20, 'Menu');
  assert.equal(r.menu.style.left, '500px', 'the next menu aligns as it asks');
});

// The ⋯ menu on the real popup menu: a mock of it cannot show what happens
// when a chosen item's close comes after the menu that item opened.
function moreRig() {
  const r = rig(), element = () => new Element(r.document), button = r.opener;
  const ui = {moreButton:button, rowMenu:r.menu, logsJumpForm:element(), logsJump:element(), jumpError:element()};
  const window = {innerWidth:800, addEventListener() {}};
  const items = () => [['최근 파일', () => more.sub('최근 파일', [['a.log', () => {}], ['b.log', () => {}]], 'recent'), null, {opens:true, key:'recent'}], ['파일 닫기', () => {}], ['열 폭', () => more.sub('열 폭', [['내용에 맞춤', () => {}]], 'columns'), null, {opens:true, key:'columns'}]];
  const more = mountMoreMenu({ui, menu:r.popup, items, document:r.document, window});
  const named = text => r.menu.children.find(item => item?.textContent?.startsWith(text));
  const key = name => r.menu.fire('keydown', {key:name});
  return {...r, button, more, named, key, expanded:() => button.attributes['aria-expanded']};
}

test('more choices keep the ⋯ menu open and marked so, and focus on a choice', () => {
  const r = moreRig();
  r.button.fire('click');
  assert.equal(r.expanded(), 'true');
  r.named('열 폭').focus();r.key('ArrowRight');
  assert.equal(r.menu.attributes['aria-label'], '열 폭');assert.equal(r.expanded(), 'true', 'its close does not mark what it opened closed');
  assert.equal(r.document.activeElement, r.named('내용에 맞춤'), 'not the way back: Right then Enter must not return');
  r.key('ArrowLeft');
  assert.equal(r.menu.attributes['aria-label'], '메뉴');assert.equal(r.expanded(), 'true');
  assert.equal(r.document.activeElement, r.named('열 폭'), 'back on the item it came from');
});

test('Escape in more choices goes back one level, and then closes', () => {
  const r = moreRig();
  r.button.fire('click');r.key('ArrowRight');
  assert.equal(r.menu.attributes['aria-label'], '최근 파일');
  r.key('Escape');
  assert.equal(r.menu.hidden, false);assert.equal(r.menu.attributes['aria-label'], '메뉴');
  r.key('Escape');
  assert.equal(r.menu.hidden, true);assert.equal(r.expanded(), 'false');assert.equal(r.document.activeElement, r.button);
});

test('another menu taking over clears the ⋯ menu\'s open mark', () => {
  const r = moreRig();
  r.button.fire('click');
  r.popup.show([['행 복사', () => {}]], 0, 0, '행 작업');
  assert.equal(r.expanded(), 'false');
});

test('with nothing focused, as after a click on macOS, a menu returns to its stand-in', () => {
  const r = moreRig();
  r.document.body = new Element(r.document);r.document.body.focus();
  r.button.fire('click');
  assert.equal(r.popup.opener(), r.button);
  r.key('Escape');assert.equal(r.document.activeElement, r.button);
  const row = new Element(r.document);row.focus();
  r.button.fire('click');
  assert.equal(r.popup.opener(), row, 'a real owner of focus wins');
});

test('what an item shows at its end is not part of its name', () => {
  const r = rig();
  r.popup.show([['다시 열기', () => {}, null, {shortcut:'⌘R', keys:'Meta+R'}], ['최근 파일', () => {}, null, {opens:true}], ['‹ 메뉴', () => {}, '메뉴로 돌아가기', {back:true}], ['행 번호', null, null, {disabled:true, dim:true, note:'이동할 행이 없습니다'}]], 0, 0, '메뉴');
  const [again, recent, back, row] = r.menu.children, end = item => item.children[0].children.at(-1);
  assert.equal(end(again).attributes['aria-hidden'], 'true');assert.equal(again.attributes['aria-keyshortcuts'], 'Meta+R');
  assert.equal(end(recent).attributes['aria-hidden'], 'true');assert.equal(recent.attributes['aria-haspopup'], 'menu');
  assert.equal(back.attributes['aria-label'], '메뉴로 돌아가기');assert.equal(row.dataset.dim, 'true');
});

test('a window resize or leaving the window returns focus to the opener', () => {
  for (const key of ['resize', 'blur']) {
    const r = rig();
    r.popup.show([['A', () => {}]], 0, 0, 'Menu');
    r.fireWindow(key);
    assert.equal(r.menu.hidden, true);assert.equal(r.document.activeElement, r.opener, key);
  }
});

test('a press in the menu keeps focus there, where the keys work', () => {
  const r = rig();
  r.popup.show([['A', () => {}], ['B', null, null, {disabled:true}]], 0, 0, 'Menu');
  let kept = false;
  r.menu.fire('mousedown', {preventDefault() {kept = true;}});
  assert.equal(kept, true);
});

test('an opener that is gone gives way to the stand-in', () => {
  const r = rig(), standIn = new Element(r.document, 'button');
  r.popup.show([['A', () => {}]], 0, 0, 'Menu', {opener:standIn});
  r.opener.isConnected = false;
  r.menu.fire('keydown', {key:'Escape'});
  assert.equal(r.document.activeElement, standIn);
});
