// Saved filters: the filter's conditions saved under a name, to apply again to
// any log of the same format. Native code keeps them in the app data folder
// and names them by ID; the page validates what it applies. The search and the
// reading options are not part of a filter.
//
// A filter is stored as its showing and hiding conditions and the unparsed
// rows option. A condition stores the fields it sets, `messageRegex: true`
// for a message pattern, and `enabled: false` when it is switched off.
import {LEVELS, tagList, idList, validateFilters, readCondition, emptyCondition, isEmptyCondition, MAX_CONDITIONS} from './filter-model.js';
import {describeFilter, describeLogError} from './describe.js';
import {textSpan} from './dom.js';

export const SAVED_VERSION = 1;
export const MAX_NAME = 60;
const KEYS = ['levels', 'tag', 'pid', 'tid', 'timeFrom', 'timeTo', 'message', 'messageRegex', 'enabled'];
const isObject = value => !!value && typeof value === 'object' && !Array.isArray(value);

export const defaultFields = () => ({include:[emptyCondition()], exclude:[], includeRaw:true});
// The filter's own fields of the page's filters: copies of its conditions.
export function fieldsOf(filters={}) {
  const list = value => (Array.isArray(value) ? value : []).map(readCondition).filter(Boolean);
  const include = list(filters.include);
  return {include:include.length ? include : [emptyCondition()], exclude:list(filters.exclude), includeRaw:filters.includeRaw !== false};
}
// Compared as the core reads them, with each condition's switch: conditions
// without criteria left out, list items trimmed and deduplicated, levels in
// the core order, time bounds trimmed, a message as typed and its mode only
// with one.
const normal = f => JSON.stringify([...['include', 'exclude'].map(list => (f[list] ?? []).filter(c => !isEmptyCondition(c)).map(c =>
  [c.enabled !== false, LEVELS.filter(level => (c.levels ?? LEVELS).includes(level)), tagList(c.tag), idList(c.pid), idList(c.tid), String(c.timeFrom ?? '').trim(), String(c.timeTo ?? '').trim(), String(c.message ?? ''), !!c.message && c.messageRegex === true])), f.includeRaw !== false]);
export const sameFields = (a, b) => normal(a) === normal(b);
export const isDefaultFields = fields => sameFields(fields, defaultFields());

export function toStored(filters) {
  const f = fieldsOf(filters);
  const store = c => {
    const out = {};
    if (c.levels.length !== LEVELS.length) out.levels = c.levels;
    for (const key of ['tag', 'pid', 'tid', 'timeFrom', 'timeTo']) if (String(c[key]).trim()) out[key] = c[key];
    if (c.message) {out.message = c.message; if (c.messageRegex) out.messageRegex = true;}
    if (!c.enabled) out.enabled = false;
    return out;
  };
  const kept = list => list.filter(c => !isEmptyCondition(c)).map(store);
  return {v:SAVED_VERSION, include:kept(f.include), exclude:kept(f.exclude), includeRaw:f.includeRaw};
}
// The fields of a stored filter, or null when it is invalid or has fields
// this version does not know (a filter saved by a later version).
export function fromStored(value) {
  if (!isObject(value) || value.v !== SAVED_VERSION) return null;
  const include = value.include ?? [], exclude = value.exclude ?? [];
  if (!Array.isArray(include) || !Array.isArray(exclude) || include.length + exclude.length > MAX_CONDITIONS) return null;
  if (![...include, ...exclude].every(c => isObject(c) && Object.keys(c).every(key => KEYS.includes(key)))) return null;
  const fields = fieldsOf({include, exclude, includeRaw:value.includeRaw});
  try {validateFilters(fields); return fields;} catch {return null;}
}
// Saved filters as native code lists them, by name.
export function readList(value) {
  const list = [];
  for (const item of Array.isArray(value) ? value : []) {
    if (!Number.isSafeInteger(item?.id) || item.id <= 0 || typeof item.name !== 'string' || !item.name || !['text', 'dlt'].includes(item.format) || list.some(entry => entry.id === item.id)) continue;
    list.push({id:item.id, name:item.name.slice(0, MAX_NAME), format:item.format, fields:fromStored(item.filter), stored:item.filter});
  }
  return list.sort((a, b) => a.name.localeCompare(b.name, 'ko', {numeric:true}));
}

const cleanName = name => String(name ?? '').replace(/[\u0000-\u001f\u007f-\u009f]/g, '').trim();
const sameName = (a, b) => a.toLowerCase() === b.toLowerCase();
// Why a name cannot be saved, checked again natively: empty, too long, or
// the name of another saved filter (case ignored).
export function nameProblem(name, list, id=null) {
  const clean = cleanName(name);
  if (!clean) return '이름을 입력해 주세요.';
  if ([...clean].length > MAX_NAME) return `이름은 ${MAX_NAME}자까지 쓸 수 있습니다.`;
  if (list.some(entry => entry.id !== id && sameName(entry.name, clean))) return '같은 이름의 저장된 필터가 있습니다. 다른 이름을 입력해 주세요.';
  return '';
}
const clip = (value, room) => [...value].length > room ? `${[...value].slice(0, room - 1).join('')}…` : value;
// A new filter is named after its conditions, numbered if that name is taken.
export function suggestName(fields, format, list=[]) {
  const base = describeFilter(fields, format) || '새 필터';
  let name = clip(base, MAX_NAME);
  for (let n = 2; list.some(entry => sameName(entry.name, name)); n++) {const tail = ` (${n})`; name = clip(base, MAX_NAME - tail.length) + tail;}
  return name;
}

// What the current filter is to its saved filters, and the menu for it: the
// filters of this log's format to apply, notes on the others, then what can
// be done with the current filter.
export function savedState({list, applied, fields, format}) {
  const current = list.find(entry => entry.id === applied) ?? null;
  const modified = !!current && (!current.fields || !sameFields(fields, current.fields));
  const empty = isDefaultFields(fields), usable = list.filter(entry => entry.format === format && entry.fields);
  const notes = [], other = list.filter(entry => entry.format !== format).length, unreadable = list.filter(entry => entry.format === format && !entry.fields).length;
  if (other) notes.push(`${format === 'dlt' ? '텍스트 로그' : 'DLT 파일'} 필터 ${other}개는 이 로그에 쓸 수 없습니다`);
  if (unreadable) notes.push(`새 버전에서 저장한 필터 ${unreadable}개는 이 버전에서 쓸 수 없습니다`);
  if (!usable.length && empty) notes.push(list.length ? '조건을 정한 뒤 저장할 수 있습니다' : '저장된 필터가 없습니다. 조건을 정한 뒤 저장할 수 있습니다');
  const actions = [];
  if (current && modified && !empty) actions.push({id:'save', label:`'${current.name}'에 저장`});
  if (current && modified && current.fields) actions.push({id:'revert', label:'저장한 조건으로 되돌리기'});
  if (!empty) actions.push({id:'saveAs', label:current ? '다른 이름으로 저장…' : '현재 필터 저장…'});
  if (current) actions.push({id:'rename', label:'이름 바꾸기…'}, {id:'delete', label:`'${current.name}' 삭제`});
  return {current, modified, empty, usable, notes, actions};
}

// The two menu buttons (log heading and filter panel), the panel's save
// button and the name dialog. `read` gives the current filter's fields,
// `write` applies fields, and `changed` reports a change to save with the
// file's work.
export function mountSavedFilters({native, menu, notify, read, write, changed, format, isGone=() => false, filterOff=() => false, document=globalThis.document}) {
  const $ = id => document.getElementById(id);
  // Each menu button names the applied filter and marks it once changed.
  const buttons = ['savedFilterButton', 'panelSavedFilter'].map(id => {
    const button = $(id), label = textSpan(document, '저장된 필터', 'saved-filter-name'), mark = textSpan(document, '수정됨', 'saved-filter-state');
    mark.hidden = true; button.replaceChildren(label, mark);
    return {button, label, mark};
  }), saveButton = $('saveFilter');
  const dialog = $('filterNameDialog'), form = $('filterNameForm'), input = $('filterNameInput'), error = $('filterNameError'), title = $('filterNameTitle'), hint = $('filterNameHint'), submit = $('filterNameSubmit');
  // `anchor` is the button the dialog was opened from, where focus returns
  // when nothing else in the page had it.
  let list = [], applied = null, mode = null, opener = null, anchor = buttons[0].button, busy = false;
  const invoke = (command, args) => native.core.invoke(command, args);
  const state = () => savedState({list, applied, fields:read(), format:format()});
  const current = () => list.find(entry => entry.id === applied) ?? null;
  const failed = reason => notify(describeLogError(reason));
  function setList(value) {list = readList(value); render();}
  async function refresh() {try {setList(await invoke('saved_filters'));} catch {}}
  function render() {
    const s = state(), name = s.current?.name, off = !!name && filterOff(), notes = [s.modified && '수정됨', off && '꺼 둠'].filter(Boolean);
    for (const {button, label, mark} of buttons) {
      label.textContent = name ?? '저장된 필터'; mark.textContent = notes.join(' · '); mark.hidden = !notes.length;
      button.dataset.state = !name ? 'none' : off ? 'off' : s.modified ? 'modified' : 'applied';
      button.setAttribute('aria-label', name ? `저장된 필터: ${[name, ...notes].join(', ')}` : '저장된 필터');
      button.title = name ? `저장된 필터 '${name}' ${off ? '· 필터를 꺼 두어 적용하지 않습니다' : s.modified ? '· 적용한 뒤 조건을 바꿨습니다' : '적용 중'}` : '필터를 이름 붙여 저장하고, 저장한 필터를 선택해 적용합니다';
    }
    saveButton.hidden = s.empty || (!!s.current && !s.modified);
    saveButton.title = s.current ? `'${name}'에 저장` : '현재 필터를 이름 붙여 저장';
  }
  // Applying replaces the filter's fields and keeps the search; the toast
  // offers the previous filter back.
  function apply(entry, message) {
    if (!entry.fields || entry.format !== format()) return;
    const before = {fields:read(), applied};
    applied = entry.id; write(entry.fields); changed();
    notify(message, {label:'실행 취소', run:() => {applied = before.applied; write(before.fields); changed();}});
  }
  async function save() {
    const entry = current(); if (!entry || busy) return;
    busy = true;
    try {setList((await invoke('save_filter', {id:entry.id, name:entry.name, format:format(), filter:toStored(read())}))?.filters); notify(`'${entry.name}'에 저장했습니다.`);}
    catch (reason) {failed(reason);} finally {busy = false;}
  }
  async function remove() {
    const entry = current(); if (!entry || busy) return;
    busy = true;
    try {
      setList(await invoke('delete_filter', {id:entry.id}));
      applied = null; render(); changed();
      notify(`'${entry.name}' 필터를 삭제했습니다.`, {label:'실행 취소', run:async () => {
        try {
          setList((await invoke('save_filter', {id:entry.id, name:entry.name, format:entry.format, filter:entry.stored}))?.filters);
          if (applied === null) {applied = entry.id; render(); changed();}
        } catch (reason) {failed(reason);}
      }});
    } catch (reason) {failed(reason);} finally {busy = false;}
  }
  function showError(message) {error.textContent = message; error.hidden = !message; input.setAttribute('aria-invalid', String(!!message));}
  function openDialog(next, from=anchor) {
    const fields = read();
    mode = next; opener = document.activeElement; anchor = from;
    title.textContent = mode === 'rename' ? '이름 바꾸기' : '필터 저장';
    submit.textContent = mode === 'rename' ? '바꾸기' : '저장';
    hint.hidden = mode === 'rename';
    hint.textContent = `저장할 조건: ${describeFilter(fields, format())}. 검색어는 저장하지 않습니다.`;
    input.value = mode === 'rename' ? current()?.name ?? '' : suggestName(fields, format(), list);
    showError('');
    if (typeof dialog.showModal === 'function') dialog.showModal(); else dialog.open = true;
    input.focus(); input.select?.();
  }
  function closeDialog() {
    if (!dialog.open) return;
    if (typeof dialog.close === 'function') dialog.close(); else dialog.open = false;
    mode = null;
    const usable = element => element && element !== document.body && element.isConnected !== false && !element.hidden && !dialog.contains?.(element);
    const target = usable(opener) ? opener : anchor;
    opener = null;
    // A button that is not shown (the panel closed, or the heading's while
    // the panel is open) cannot take focus; the other one then does.
    for (const next of [target, ...buttons.map(b => b.button)]) {next?.focus?.({preventScroll:true}); if (document.activeElement === next) break;}
  }
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (busy || !mode) return;
    const renaming = mode === 'rename', entry = current();
    if (renaming && !entry) {closeDialog(); return;}
    const problem = nameProblem(input.value, list, renaming ? entry.id : null);
    if (problem) {showError(problem); input.focus(); return;}
    busy = true; submit.disabled = true;
    try {
      if (renaming) {
        setList(await invoke('rename_filter', {id:entry.id, name:input.value}));
        closeDialog(); notify(`이름을 바꿨습니다 · ${current()?.name ?? cleanName(input.value)}`);
      } else {
        const reply = await invoke('save_filter', {name:input.value, format:format(), filter:toStored(read())});
        setList(reply?.filters);
        if (Number.isSafeInteger(reply?.id)) {applied = reply.id; render(); changed();}
        closeDialog(); notify(`'${current()?.name ?? cleanName(input.value)}' 필터를 저장했습니다.`);
      }
    } catch (reason) {showError(describeLogError(reason)); input.focus();}
    finally {busy = false; submit.disabled = false;}
  });
  input.addEventListener('input', () => showError(''));
  $('filterNameCancel').addEventListener('click', closeDialog);
  // Escape stops at the dialog, so it does not also close the raw detail.
  dialog.addEventListener('keydown', event => {if (event.key === 'Escape') {event.preventDefault(); event.stopPropagation(); closeDialog();}});
  dialog.addEventListener('cancel', event => {event.preventDefault(); closeDialog();});
  dialog.addEventListener('click', event => {if (event.target === dialog) closeDialog();});
  const run = {save, revert:() => {const entry = current(); if (entry) apply(entry, `'${entry.name}'에 저장한 조건으로 되돌렸습니다.`);}, saveAs:() => openDialog('save'), rename:() => openDialog('rename'), delete:remove};
  async function openMenu(button) {
    if (isGone()) return;
    anchor = button;
    // Another running app may have changed the filters.
    await refresh();
    const s = state(), items = s.usable.map(entry => [entry.name, () => apply(entry, `'${entry.name}' 필터를 적용했습니다.`), describeFilter(entry.fields, entry.format), {checked:entry.id === applied}]);
    items.push(...s.notes.map(note => [note, null]));
    if (items.length && s.actions.length) items.push(null);
    items.push(...s.actions.map(action => [action.label, run[action.id]]));
    const box = button.getBoundingClientRect?.();
    menu.show(items, box ? box.left : 0, box ? box.bottom + 4 : 0, '저장된 필터');
  }
  for (const {button} of buttons) button.addEventListener('click', () => void openMenu(button));
  // The panel's save button hides once saved; focus returns to the panel's menu button.
  saveButton.addEventListener('click', () => current() ? void save() : openDialog('save', buttons[1].button));
  void refresh();
  return {
    render, refresh,
    applied:() => applied,
    // The saved filter the current filter came from, as saved with a file's work.
    setApplied(id) {applied = Number.isSafeInteger(id) && id > 0 ? id : null; render();},
  };
}
