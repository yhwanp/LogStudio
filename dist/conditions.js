// The filter panel's conditions as one list: showing conditions, then hiding
// ones, each with its switch, kind and summary. A condition without criteria
// shows only while it is being edited. The list is rebuilt when conditions
// are added, removed or selected; typing refreshes the summaries in place. A
// condition no row of the log matches (`rowless`, from the engine) says so in
// its item. `problem` names a problem only the engine finds, such as a
// pattern it rejected.
import {describeCondition} from './describe.js';
import {isEmptyCondition, conditionProblem, MAX_CONDITIONS} from './filter-model.js';

const LISTS = ['include', 'exclude'];
export function conditionSummary(c, format, hiding=false, part='label') {
  const parts = describeCondition(c, format, hiding).map(chip => chip[part] ?? chip.label);
  return parts.length ? parts.join(' · ') : '새 조건';
}

// What an item's title says of a condition without rows, by list.
export function rowlessText(list, following=false) {
  return `${following ? '지금까지 ' : ''}이 로그에 ${list === 'include' ? '이 조건으로 보이는 줄이 없습니다' : '이 조건이 숨기는 줄이 없습니다'}`;
}

export function mountConditions({document, list:box, empty, tools, editing, onSelect, onToggle, onAdd, onDuplicate, onRemove, rowless=() => ({include:new Set(), exclude:new Set(), following:false}), problem:engineProblem=() => '', filterOff=() => false}) {
  let items = [];
  // WebKit leaves focus where it was on a click, and the editor gives focus
  // back to what opened it: the panel must keep it, or auto-hide closes it.
  const pressed = (button, action) => button.addEventListener('click', event => {button.focus({preventScroll:true}); action(event);});
  pressed(tools.add, onAdd);
  pressed(tools.duplicate, onDuplicate);
  tools.remove.addEventListener('click', onRemove);
  function item(list, index) {
    const row = document.createElement('li'), toggle = document.createElement('input'), kind = document.createElement('span'), summary = document.createElement('button');
    const text = document.createElement('span'), state = document.createElement('span');
    text.className = 'cond-text'; state.className = 'cond-state'; summary.append(text, state);
    row.className = 'cond-item';
    toggle.type = 'checkbox'; toggle.className = 'cond-toggle';
    toggle.addEventListener('change', () => onToggle(list, index, toggle.checked));
    kind.className = `cond-kind ${list === 'include' ? 'show' : 'hide'}`; kind.textContent = list === 'include' ? '보기' : '제외';
    summary.type = 'button'; summary.className = 'cond-summary';
    pressed(summary, () => onSelect(list, index));
    row.append(toggle, kind, summary);
    return {row, toggle, summary, text, state, list, index};
  }
  function build(conds, active, format) {
    const open = editing();
    items = LISTS.flatMap(list => conds[list].flatMap((c, index) => !isEmptyCondition(c) || (open && active.list === list && active.index === index) ? [item(list, index)] : []));
    box.replaceChildren(...items.map(view => view.row));
    refresh(conds, active, format);
  }
  // Summaries, switches and states, without replacing the items.
  function refresh(conds, active, format) {
    const marks = rowless(), off = filterOff(), open = editing();
    for (const view of items) {
      const {list, index} = view, c = conds[list][index];
      if (!c) continue;
      const current = active.list === list && active.index === index;
      const hiding = list === 'exclude', text = conditionSummary(c, format, hiding), name = `${list === 'include' ? '보기' : '제외'} 조건`;
      const problem = conditionProblem(c) || engineProblem(list, index), none = !!marks[list]?.has(index);
      Object.assign(view.row.dataset, {active:String(current && (open || !isEmptyCondition(c))), off:String(c.enabled === false), invalid:String(!!problem), rows:none ? 'none' : ''});
      view.toggle.checked = c.enabled !== false;
      view.toggle.setAttribute('aria-label', `${name} 켜기: ${text}`);
      view.toggle.title = c.enabled === false ? `꺼진 조건 · 켜면 ${off ? '필터도 함께 켜집니다' : '적용합니다'}` : `켜진 조건 · 끄면 ${off ? '이 조건만 끄고 필터를 켭니다' : '적용하지 않습니다'}`;
      // The state stays in view when the summary is cut; the title has all of it.
      view.text.textContent = text; view.state.textContent = problem ? ' · 확인 필요' : none ? ' · 줄 없음' : '';
      view.summary.title = `${conditionSummary(c, format, hiding, 'full')} · ${problem || (none ? rowlessText(list, marks.following) : '눌러서 편집')}`;
      view.summary.setAttribute('aria-current', String(view.row.dataset.active === 'true'));
      view.summary.setAttribute('aria-label', `${name}: ${text}${c.enabled === false ? ' (꺼짐)' : ''}${problem ? ' (확인 필요)' : none ? ' (이 로그에 맞는 줄 없음)' : ''}`);
    }
    const count = conds.include.length + conds.exclude.length, full = count >= MAX_CONDITIONS, chosen = !isEmptyCondition(conds[active.list]?.[active.index] ?? {});
    empty.hidden = items.length > 0;
    tools.add.disabled = full; tools.add.title = full ? `조건은 ${MAX_CONDITIONS}개까지 쓸 수 있습니다` : '새 조건을 만듭니다';
    tools.duplicate.disabled = full || !chosen; tools.remove.disabled = !chosen;
    tools.duplicate.title = full ? `조건은 ${MAX_CONDITIONS}개까지 쓸 수 있습니다` : '선택한 조건을 복제합니다'; tools.remove.title = '선택한 조건을 삭제합니다';
  }
  // The item of a condition, for focus after a rebuild.
  const summaryOf = (list, index) => items.find(view => view.list === list && view.index === index)?.summary ?? null;
  return {build, refresh, summaryOf};
}
