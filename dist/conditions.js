// The filter panel's conditions: a card per showing and per hiding condition,
// and one editor (the panel's level, tag, message, ID and time fields) inside
// the card being edited. A lone showing condition being edited has no card
// header, so a filter of one condition reads as one set of fields. Cards are
// rebuilt only when conditions are added, removed or selected; typing
// refreshes their summaries in place, so the editor keeps focus. A condition
// no row of the log matches (`rowless`, from the engine) says so in a line
// under its summary, which keeps its width, or under the editor of the
// condition being edited, so that typing never moves the field. `problem`
// names a problem only the engine finds, such as a message pattern it
// rejected.
import {describeCondition} from './describe.js';
import {isEmptyCondition, conditionProblem, MAX_CONDITIONS} from './filter-model.js';

const LISTS = ['include', 'exclude'];
export function conditionSummary(c, format) {
  const parts = describeCondition(c, format).map(chip => chip.label);
  return parts.length ? parts.join(' · ') : '조건 없음';
}

// What a card says of a condition without rows, by list.
export function rowlessText(list, following=false) {
  const none = following ? '지금까지 이 로그에 이 조건과 맞는 줄이 없습니다' : '이 로그에 이 조건과 맞는 줄이 없습니다';
  return `${none} · ${list === 'include' ? '이 조건으로 보이는 줄이 없습니다' : '이 조건이 숨기는 줄이 없습니다'}`;
}

export function mountConditions({document, lists, hints, adders, editor, onSelect, onToggle, onRemove, onAdd, rowless=() => ({include:new Set(), exclude:new Set(), following:false}), problem:engineProblem=() => ''}) {
  let cards = {include:[], exclude:[]};
  for (const list of LISTS) adders[list].addEventListener('click', () => onAdd(list));
  function card(list, index) {
    const box = document.createElement('div'), head = document.createElement('div');
    const toggle = document.createElement('input'), summary = document.createElement('button'), remove = document.createElement('button');
    const alert = document.createElement('p');
    box.className = 'cond-card'; head.className = 'cond-head';
    toggle.type = 'checkbox'; toggle.className = 'cond-toggle';
    toggle.addEventListener('change', () => onToggle(list, index, toggle.checked));
    summary.type = 'button'; summary.className = 'cond-summary';
    summary.addEventListener('click', () => onSelect(list, index));
    remove.type = 'button'; remove.className = 'cond-remove'; remove.textContent = '×';
    remove.addEventListener('click', () => onRemove(list, index));
    alert.className = 'cond-alert'; alert.hidden = true;
    head.append(toggle, summary, remove); box.append(head, alert);
    return {box, toggle, summary, alert, remove};
  }
  // Rebuilds the cards and puts the editor in the active one.
  function build(conds, active, format) {
    for (const list of LISTS) {
      cards[list] = conds[list].map((_, index) => card(list, index));
      lists[list].replaceChildren(...cards[list].map(c => c.box));
    }
    const editing = cards[active.list]?.[active.index];
    editing?.box.insertBefore(editor, editing.alert);
    refresh(conds, active, format);
  }
  // Summaries, switches and states, without moving the editor.
  function refresh(conds, active, format) {
    const solo = conds.include.length === 1 && active.list === 'include', marks = rowless();
    for (const list of LISTS) {
      conds[list].forEach((c, index) => {
        const view = cards[list][index];
        if (!view) return;
        const current = active.list === list && active.index === index, text = conditionSummary(c, format);
        const kind = list === 'include' ? '보기' : '제외', name = `${kind} 조건 ${index + 1}`;
        const problem = conditionProblem(c) || engineProblem(list, index), none = !!marks[list]?.has(index), lone = solo && list === 'include';
        view.box.dataset.active = String(current);
        view.box.dataset.off = String(c.enabled === false);
        view.box.dataset.empty = String(isEmptyCondition(c));
        view.box.dataset.invalid = String(!!problem);
        view.box.dataset.rows = none ? 'none' : '';
        // A card's line is short, its title says what it means; a lone
        // condition has room for the sentence.
        view.alert.hidden = !none;
        view.alert.textContent = !none ? '' : lone ? `${rowlessText(list, marks.following)}.` : marks.following ? '지금까지 맞는 줄 없음' : '이 로그에 맞는 줄 없음';
        view.alert.title = none && !lone ? rowlessText(list, marks.following) : '';
        view.box.classList.toggle('solo', solo && list === 'include');
        view.toggle.checked = c.enabled !== false;
        view.toggle.setAttribute('aria-label', `${name} 켜기`);
        view.toggle.title = c.enabled === false ? '꺼진 조건 · 켜면 적용합니다' : '켜진 조건 · 끄면 적용하지 않습니다';
        view.summary.textContent = problem ? `${text} · 확인 필요` : text;
        view.summary.title = problem || (current ? `${name} · 편집 중` : `${name} · 눌러서 편집`);
        view.summary.setAttribute('aria-expanded', String(current));
        view.summary.setAttribute('aria-label', `${name}: ${text}${c.enabled === false ? ' (꺼짐)' : ''}${none ? ' (이 로그에 맞는 줄 없음)' : ''}`);
        view.remove.setAttribute('aria-label', `${name} 삭제`);
        view.remove.title = '조건 삭제';
      });
    }
    const n = conds.include.length, m = conds.exclude.length, full = n + m >= MAX_CONDITIONS;
    hints.include.textContent = n > 1 ? '하나라도 맞으면 보입니다' : '';
    hints.exclude.textContent = m > 1 ? '하나라도 맞으면 숨깁니다' : m ? '맞는 줄을 숨깁니다' : '';
    for (const list of LISTS) {
      adders[list].disabled = full;
      adders[list].title = full ? `조건은 ${MAX_CONDITIONS}개까지 쓸 수 있습니다` : list === 'include' ? '이 조건 또는 다른 보기 조건에 맞는 줄을 보입니다' : '맞는 줄을 숨기는 조건을 더합니다';
    }
  }
  return {build, refresh};
}
