import test from 'node:test';
import assert from 'node:assert/strict';
import {createPopupMenu} from '../dist/menu.js';

class Element {
  constructor(document, tag='div') {this.document = document; this.tagName = tag.toUpperCase(); this.children = []; this.attributes = {}; this.listeners = {}; this.style = {}; this.hidden = true; this.textContent = ''; this.className = '';}
  setAttribute(key, value) {this.attributes[key] = String(value);}
  getAttribute(key) {return this.attributes[key] ?? null;}
  replaceChildren(...nodes) {this.children = nodes;}
  addEventListener(key, fn) {(this.listeners[key] ??= []).push(fn);}
  fire(key, event={}) {for (const fn of this.listeners[key] ?? []) fn({preventDefault() {}, stopPropagation() {}, ...event});}
  focus() {this.document.activeElement = this;}
  contains(node) {return node === this || this.children.includes(node);}
  click() {this.fire('click');}
  // Only the selectors the menu uses.
  querySelector(selector) {return this.querySelectorAll(selector)[0] ?? null;}
  querySelectorAll(selector) {
    if (selector === '[aria-checked=true]') return this.children.filter(c => c.attributes['aria-checked'] === 'true');
    if (selector === '[role^=menuitem]') return this.children.filter(c => c.attributes.role?.startsWith('menuitem'));
    return [];
  }
}

function rig() {
  const document = {activeElement:null, listeners:{}, createElement(tag) {return new Element(document, tag);}, addEventListener(key, fn) {this.listeners[key] = fn;}};
  const window = {innerWidth:800, innerHeight:600, addEventListener() {}};
  const menu = new Element(document), opener = new Element(document, 'button');
  opener.focus();
  return {document, menu, opener, popup:createPopupMenu({menu, document, window})};
}

test('notes are plain text, a checked choice is a radio item focused first', () => {
  const r = rig(), picked = [];
  r.popup.show([['A', () => picked.push('A'), 'tip', {checked:false}], ['B', () => picked.push('B'), '', {checked:true}], ['note only', null], null, ['Save', () => picked.push('save')]], 10, 20, 'Saved');
  const [a, b, note, line, save] = r.menu.children;
  assert.equal(r.menu.hidden, false); assert.equal(r.menu.attributes['aria-label'], 'Saved');
  assert.deepEqual([a.attributes.role, a.attributes['aria-checked'], a.title], ['menuitemradio', 'false', 'tip']);
  assert.deepEqual([b.attributes.role, b.attributes['aria-checked']], ['menuitemradio', 'true']);
  assert.deepEqual([note.tagName, note.className, note.textContent, note.attributes.role], ['P', 'menu-note', 'note only', undefined]);
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
