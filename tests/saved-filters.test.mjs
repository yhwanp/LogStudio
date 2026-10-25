import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as nextTurn} from 'node:timers/promises';
import {defaultFields, fieldsOf, sameFields, isDefaultFields, toStored, fromStored, readList, nameProblem, suggestName, savedState, mountSavedFilters, MAX_NAME, SAVED_VERSION} from '../dist/saved-filters.js';
import {LEVELS, emptyCondition, MAX_CONDITIONS} from '../dist/filter-model.js';

const cond = (extra={}) => ({...emptyCondition(), ...extra});
// One showing condition and the excluded tags as one hiding condition.
const fields = ({excludeTags='', includeRaw=true, ...show}={}) => ({include:[cond(show)], exclude:excludeTags ? [cond({tag:excludeTags})] : [], includeRaw});
const entry = (id, name, format='text', extra={}) => ({id, name, format, filter:toStored(fields(extra))});
async function settle() {for (let i = 0; i < 5; i++) await nextTurn();}

test('a filter is stored as the fields its conditions set', () => {
  const f = fields({levels:['W', 'E', 'F', 'A'], tag:'MyApp, Net', excludeTags:'chatty', pid:'2983 3120', timeFrom:'06-18 19:24', includeRaw:false});
  const stored = toStored({...f, query:'boom', caseSensitive:true, searchMode:'regex', recognizeLineNumbers:false});
  assert.deepEqual(stored, {v:SAVED_VERSION, include:[{levels:['W', 'E', 'F', 'A'], tag:'MyApp, Net', pid:'2983 3120', timeFrom:'06-18 19:24'}], exclude:[{tag:'chatty'}], includeRaw:false}, 'the search and reading options are not saved');
  assert.deepEqual(fromStored(stored), f);
  assert.deepEqual(toStored(defaultFields()), {v:SAVED_VERSION, include:[], exclude:[], includeRaw:true}, 'empty conditions are not stored');
  assert.deepEqual(fromStored(toStored(fields({excludeTags:'Binder'}))), fields({excludeTags:'Binder'}));
  assert.deepEqual(fromStored({v:SAVED_VERSION, include:[{tag:'a'}]}), fields({tag:'a'}), 'missing levels are all levels');
  // Several conditions, a switched-off one and hiding levels.
  const many = {include:[cond({tag:'NAV'}), cond({levels:['E', 'F']}), cond({tag:'X', enabled:false})], exclude:[cond({tag:'Binder', levels:['V', 'D']})], includeRaw:true};
  assert.deepEqual(toStored(many), {v:SAVED_VERSION, include:[{tag:'NAV'}, {levels:['E', 'F']}, {tag:'X', enabled:false}], exclude:[{levels:['V', 'D'], tag:'Binder'}], includeRaw:true});
  assert.deepEqual(fromStored(toStored(many)), many);
  // A filter saved before conditions could be switched off reads the same.
  assert.deepEqual(fromStored({v:SAVED_VERSION, include:[{levels:['E'], tag:'a', pid:'', tid:'', timeFrom:'', timeTo:''}], exclude:[{tag:'b'}], includeRaw:true}), fields({levels:['E'], tag:'a', excludeTags:'b'}));
});

test('filters with unknown fields or values the filter rejects are not applied', () => {
  assert.equal(fromStored({v:SAVED_VERSION, include:[{tag:'a', payload:'x'}]}), null, 'an unknown field');
  assert.equal(fromStored({v:SAVED_VERSION, include:Array.from({length:MAX_CONDITIONS + 1}, (_, i) => ({tag:`t${i}`}))}), null, 'too many conditions');
  assert.equal(fromStored({v:2, include:[]}), null);
  for (const bad of [null, [], 'x', {v:SAVED_VERSION, include:'a'}, {v:SAVED_VERSION, include:[[1]]}]) assert.equal(fromStored(bad), null);
  assert.equal(fromStored({v:SAVED_VERSION, include:[{pid:'abc'}]}), null, 'an invalid PID');
  assert.equal(fromStored({v:SAVED_VERSION, include:[{timeFrom:'25:00'}]}), null, 'an invalid time');
  assert.equal(fromStored({v:SAVED_VERSION, include:[{tag:Array(33).fill('t').map((t, i) => t + i).join(',')}]}), null, 'too many tags');
});

test('filters compare as the core reads them, with each condition switch', () => {
  assert.ok(sameFields({include:[cond({tag:'a'}), cond()], exclude:[cond({tag:' , '})]}, fields({tag:'a'})), 'conditions without criteria do not count');
  assert.ok(!sameFields({include:[cond({tag:'a', enabled:false})], exclude:[]}, fields({tag:'a'})), 'switching a condition off changes the filter');
  assert.ok(!sameFields({include:[cond({tag:'a'}), cond({tag:'b'})], exclude:[]}, {include:[cond({tag:'b'}), cond({tag:'a'})], exclude:[]}), 'conditions keep their order');
  assert.ok(sameFields(fields({tag:' MyApp ,, MyApp', levels:['E', 'W']}), fields({tag:'MyApp', levels:['W', 'E']})));
  assert.ok(sameFields(fields({pid:'1,2'}), fields({pid:' 1  2 '})));
  assert.ok(!sameFields(fields({pid:'0100'}), fields({pid:'100'})), 'IDs are strings');
  assert.ok(!sameFields(fields({includeRaw:false}), fields()));
  assert.ok(isDefaultFields(fieldsOf({query:'x', include:[{levels:[...LEVELS].reverse()}]})), 'the search is not part of the filter');
  assert.deepEqual(fieldsOf({include:[]}).include, [emptyCondition()], 'the panel always has a showing condition');
});

test('the list keeps valid entries by name and marks those this version cannot apply', () => {
  const list = readList([
    entry(3, 'b 10'), entry(4, 'b 9', 'dlt'), {id:5, name:'later', format:'text', filter:{v:2}},
    {id:3, name:'same id', format:'text', filter:{}}, {id:0, name:'zero', format:'text'}, {id:6, name:'', format:'text'}, {id:7, name:'csv', format:'csv'}, null,
  ]);
  assert.deepEqual(list.map(e => [e.id, e.name, !!e.fields]), [[4, 'b 9', true], [3, 'b 10', true], [5, 'later', false]]);
  assert.deepEqual(readList({}), []);
});

test('names are required, bounded and unique ignoring case', () => {
  const list = readList([entry(1, 'APP 분석')]);
  assert.equal(nameProblem('  ', list), '이름을 입력해 주세요.');
  assert.equal(nameProblem('가'.repeat(MAX_NAME + 1), list), `이름은 ${MAX_NAME}자까지 쓸 수 있습니다.`);
  assert.match(nameProblem(' app 분석 ', list), /같은 이름/);
  assert.equal(nameProblem('app 분석', list, 1), '', 'a filter keeps its own name');
  assert.equal(nameProblem('가'.repeat(MAX_NAME), list), '');
  assert.equal(suggestName(fields({tag:'MyApp'}), 'text', list), '태그: MyApp');
  assert.equal(suggestName(defaultFields(), 'text', readList([entry(2, '새 필터')])), '새 필터 (2)');
  const long = suggestName(fields({tag:'x'.repeat(200)}), 'text');
  assert.ok([...long].length <= MAX_NAME && long.endsWith('…'), long);
  const taken = readList([entry(2, long)]), next = suggestName(fields({tag:'x'.repeat(200)}), 'text', taken);
  assert.ok([...next].length <= MAX_NAME && next.endsWith('… (2)'), next);
});

test('the menu offers this format’s filters and what fits the current filter', () => {
  const list = readList([entry(1, 'NAV', 'text', {tag:'NAV'}), entry(2, 'ECU', 'dlt', {tag:'ECU1'}), {id:3, name:'later', format:'text', filter:{v:2}}]);
  const ids = s => s.actions.map(a => a.id);
  let s = savedState({list, applied:null, fields:defaultFields(), format:'text'});
  assert.deepEqual(s.usable.map(e => e.id), [1]);
  assert.deepEqual(s.notes, ['DLT 파일 필터 1개는 이 로그에 쓸 수 없습니다', '새 버전에서 저장한 필터 1개는 이 버전에서 쓸 수 없습니다']);
  assert.deepEqual(ids(s), [], 'an empty filter is not saved');
  s = savedState({list, applied:null, fields:fields({tag:'x'}), format:'text'});
  assert.deepEqual(s.actions, [{id:'saveAs', label:'현재 필터 저장…'}]);
  s = savedState({list, applied:1, fields:fields({tag:'NAV'}), format:'text'});
  assert.equal(s.modified, false); assert.deepEqual(ids(s), ['saveAs', 'rename', 'delete']);
  s = savedState({list, applied:1, fields:fields({tag:'NAV, HMI'}), format:'text'});
  assert.equal(s.modified, true); assert.deepEqual(ids(s), ['save', 'revert', 'saveAs', 'rename', 'delete']);
  assert.equal(s.actions[0].label, "'NAV'에 저장");
  s = savedState({list, applied:1, fields:defaultFields(), format:'text'});
  assert.deepEqual(ids(s), ['revert', 'rename', 'delete'], 'cleared conditions can go back but are not saved');
  assert.deepEqual(savedState({list:[], applied:null, fields:defaultFields(), format:'dlt'}).notes, ['저장된 필터가 없습니다. 조건을 정한 뒤 저장할 수 있습니다']);
  assert.deepEqual(savedState({list, applied:null, fields:defaultFields(), format:'dlt'}).notes, ['텍스트 로그 필터 2개는 이 로그에 쓸 수 없습니다']);
  assert.deepEqual(savedState({list:list.filter(e => e.name === 'NAV'), applied:null, fields:defaultFields(), format:'dlt'}).notes, ['텍스트 로그 필터 1개는 이 로그에 쓸 수 없습니다', '조건을 정한 뒤 저장할 수 있습니다'], 'only filters of another format');
  assert.equal(savedState({list, applied:99, fields:fields({tag:'NAV'}), format:'text'}).current, null, 'a filter deleted elsewhere');
});

class Element {
  constructor(tag='div') {this.tagName = tag.toUpperCase(); this.children = []; this.listeners = {}; this.dataset = {}; this.attributes = {}; this.hidden = false; this.value = ''; this._text = ''; this.className = ''; this.disabled = false;}
  get textContent() {return this._text + this.children.map(child => child.textContent).join('');}
  set textContent(value) {this._text = String(value); this.children = [];}
  append(...nodes) {this.children.push(...nodes);}
  replaceChildren(...nodes) {this.children = []; this._text = ''; this.append(...nodes);}
  setAttribute(key, value) {this.attributes[key] = String(value);}
  addEventListener(key, fn) {(this.listeners[key] ??= []).push(fn);}
  dispatch(key, event={}) {return Promise.all((this.listeners[key] ?? []).map(fn => fn({preventDefault() {}, stopPropagation() {}, ...event})));}
  focus() {document.activeElement = this;}
  select() {}
}
const document = {elements:new Map(), activeElement:null, getElementById(id) {if (!this.elements.has(id)) this.elements.set(id, new Element()); return this.elements.get(id);}, createElement:tag => new Element(tag)};

function rig({stored=[], format='text'}={}) {
  document.elements.clear();
  let disk = structuredClone(stored), nextId = 100, current = fields(), view = format;
  const calls = [], toasts = [], menus = [], changes = [];
  const reply = () => structuredClone(disk);
  const native = {core:{invoke:async (command, args={}) => {
    calls.push([command, args]);
    if (command === 'saved_filters') return reply();
    if (command === 'save_filter') {
      if (disk.some(e => e.id !== args.id && e.name.toLowerCase() === args.name.trim().toLowerCase())) throw 'A saved filter has this name';
      const id = args.id ?? nextId++, item = {id, name:args.name.trim(), format:args.format, filter:args.filter}, at = disk.findIndex(e => e.id === id);
      if (at < 0) disk.push(item); else disk[at] = item;
      return {id, filters:reply()};
    }
    if (command === 'rename_filter') {disk.find(e => e.id === args.id).name = args.name.trim(); return reply();}
    if (command === 'delete_filter') {disk = disk.filter(e => e.id !== args.id); return reply();}
    throw command;
  }}};
  const saved = mountSavedFilters({native, document, menu:{show:(items, x, y, label) => menus.push({items, label})}, notify:(message, action=null) => toasts.push({message, action}),
    read:() => current, write:f => {current = f; saved.render();}, changed:() => changes.push(saved.applied()), format:() => view});
  const $ = id => document.getElementById(id);
  const button = $('savedFilterButton');
  return {saved, calls, toasts, menus, changes, $, button, elsewhere:item => disk.push(item),
    set:f => {current = fields(f); saved.render();}, get fields() {return current;}, setFormat:f => {view = f;},
    label:() => [button.children[0].textContent, !button.children[1].hidden, button.dataset.state],
    async menu() {await button.dispatch('click'); await settle(); return menus.at(-1).items;},
    async pick(label) {const item = (await this.menu()).find(i => i?.[0] === label); assert.ok(item, `menu item ${label}`); await item[1](); await settle();},
    async submit(name) {$('filterNameInput').value = name; await $('filterNameForm').dispatch('submit'); await settle();}};
}

test('a filter is saved under a name, applied to another file and undone', async () => {
  const r = rig();
  await settle();
  assert.deepEqual(r.label(), ['저장된 필터', false, 'none']);
  assert.equal(r.$('saveFilter').hidden, true, 'nothing to save');
  r.set({tag:'MyApp', levels:['W', 'E', 'F', 'A']});
  assert.equal(r.$('saveFilter').hidden, false);
  await r.pick('현재 필터 저장…');
  assert.equal(r.$('filterNameDialog').open, true);
  assert.equal(r.$('filterNameInput').value, '레벨: Warn 이상 · 태그: MyApp', 'named after the conditions');
  assert.match(r.$('filterNameHint').textContent, /검색어는 저장하지 않습니다/);
  await r.submit('  ');
  assert.equal(r.$('filterNameError').textContent, '이름을 입력해 주세요.');
  assert.equal(r.$('filterNameDialog').open, true);
  await r.submit('내 앱 경고');
  assert.equal(r.$('filterNameDialog').open, false);
  assert.deepEqual(r.label(), ['내 앱 경고', false, 'applied']);
  assert.equal(r.$('saveFilter').hidden, true, 'saved as it is');
  assert.equal(document.activeElement, r.$('savedFilterButton'), 'focus returns to the menu button it was opened from');
  assert.deepEqual(r.changes, [100], 'the applied filter is saved with the work');
  assert.equal(r.toasts.at(-1).message, "'내 앱 경고' 필터를 저장했습니다.");

  // Another file starts with no filter; picking the saved one applies it.
  r.saved.setApplied(null); r.set({pid:'42'});
  const items = await r.menu();
  assert.deepEqual(items.map(i => i && [i[0], !!i[1], i[3]?.checked]), [['내 앱 경고', true, false], null, ['현재 필터 저장…', true, undefined]]);
  assert.equal(items[0][2], '레벨: Warn 이상 · 태그: MyApp', 'the tooltip reads the filter as one line');
  await items[0][1]();
  assert.deepEqual(r.fields, fields({tag:'MyApp', levels:['W', 'E', 'F', 'A']}));
  assert.deepEqual(r.label(), ['내 앱 경고', false, 'applied']);
  const undo = r.toasts.at(-1);
  assert.equal(undo.message, "'내 앱 경고' 필터를 적용했습니다."); assert.equal(undo.action.label, '실행 취소');
  undo.action.run();
  assert.deepEqual(r.fields, fields({pid:'42'})); assert.deepEqual(r.label(), ['저장된 필터', false, 'none']);
  assert.equal(r.calls.filter(([c]) => c === 'saved_filters').length, 3, 'the list is read at mount and when the menu opens');
});

test('a changed filter is marked and can be saved, reverted, renamed and deleted with undo', async () => {
  const r = rig({stored:[entry(7, 'NAV', 'text', {tag:'NAV'}), entry(8, 'HMI', 'text', {tag:'HMI'})]});
  await settle();
  r.saved.setApplied(7); r.set({tag:'NAV'});
  assert.deepEqual(r.label(), ['NAV', false, 'applied']);
  r.set({tag:'NAV, Route'});
  assert.deepEqual(r.label(), ['NAV', true, 'modified']);
  assert.equal(r.button.attributes['aria-label'], '저장된 필터: NAV, 수정됨');
  assert.equal(r.$('saveFilter').hidden, false);
  assert.equal(r.$('saveFilter').title, "'NAV'에 저장");
  await r.$('saveFilter').dispatch('click'); await settle();
  assert.deepEqual(r.calls.at(-1), ['save_filter', {id:7, name:'NAV', format:'text', filter:toStored(fields({tag:'NAV, Route'}))}]);
  assert.deepEqual(r.label(), ['NAV', false, 'applied']);

  r.set({tag:'NAV, Route, Map'});
  await r.pick('저장한 조건으로 되돌리기');
  assert.deepEqual(r.fields, fields({tag:'NAV, Route'}));
  assert.equal(r.toasts.at(-1).message, "'NAV'에 저장한 조건으로 되돌렸습니다.");

  await r.pick('이름 바꾸기…');
  assert.equal(r.$('filterNameInput').value, 'NAV'); assert.equal(r.$('filterNameHint').hidden, true);
  await r.submit('hmi');
  assert.match(r.$('filterNameError').textContent, /같은 이름/, 'checked before asking native code');
  assert.equal(r.$('filterNameInput').attributes['aria-invalid'], 'true');
  await r.submit('내비게이션');
  assert.deepEqual(r.label(), ['내비게이션', false, 'applied']);

  await r.pick("'내비게이션' 삭제");
  assert.deepEqual(r.label(), ['저장된 필터', false, 'none']);
  assert.deepEqual(r.fields, fields({tag:'NAV, Route'}), 'the conditions stay');
  await r.toasts.at(-1).action.run(); await settle();
  assert.deepEqual(r.calls.at(-1)[1], {id:7, name:'내비게이션', format:'text', filter:toStored(fields({tag:'NAV, Route'}))}, 'undo saves it again under its ID');
  assert.deepEqual(r.label(), ['내비게이션', false, 'applied']);
});

test('native refusals show in the dialog and filters of another format cannot be applied', async () => {
  const r = rig({stored:[entry(5, 'Taken', 'dlt', {tag:'ECU1'})]});
  await settle();
  r.set({tag:'x'});
  let items = await r.menu();
  assert.deepEqual(items.map(i => i && [i[0], !!i[1]]), [['DLT 파일 필터 1개는 이 로그에 쓸 수 없습니다', false], null, ['현재 필터 저장…', true]]);
  await items.at(-1)[1](); await settle();
  // Another running app saves the same name after this list was read.
  r.elsewhere(entry(6, 'Mine', 'text', {tag:'y'}));
  await r.submit('mine');
  assert.equal(r.calls.at(-1)[0], 'save_filter', 'refused natively');
  assert.equal(r.$('filterNameError').textContent, '같은 이름의 저장된 필터가 있습니다. 다른 이름을 입력해 주세요.');
  assert.equal(r.$('filterNameDialog').open, true);
  assert.equal(r.$('filterNameSubmit').disabled, false);
  assert.deepEqual(r.label(), ['저장된 필터', false, 'none']);
  r.setFormat('dlt');
  items = await r.menu();
  assert.equal(items[0][0], 'Taken');
  await r.$('filterNameCancel').dispatch('click');
  assert.equal(r.$('filterNameDialog').open, false);
});
