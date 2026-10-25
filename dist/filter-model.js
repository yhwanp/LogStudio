// UI input limits and filter validation. Parsing and search run in the Rust core.
export const MAX_BYTES = 25 * 1024 * 1024;
export const LEVELS = ['V', 'D', 'I', 'W', 'E', 'F', 'A', 'U'];
// Mirrors the core grammar: [YYYY-MM-DD | MM-DD] [HH[:MM[:SS[.fraction]]]].
const DATE = /^(?:\d{4}-)?(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])$/;
const CLOCK = /^([01]\d|2[0-3])(?::[0-5]\d(?::[0-5]\d(?:\.\d{1,9})?)?)?$/;
export function isTimeBound(value) {
  const words = String(value ?? '').trim().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  if (words.length === 2) return DATE.test(words[0]) && CLOCK.test(words[1]);
  return words.length === 1 && (words[0].includes('-') ? DATE.test(words[0]) : CLOCK.test(words[0]));
}
// True when both bounds carry a date and the end comes before the start under
// the core's comparison, so no row can match. Dates are compared by year only
// when both bounds have one; mixed bounds may still match dated rows.
export function isEmptyTimeRange(from, to) {
  if (!isTimeBound(from) || !isTimeBound(to)) return false;
  const point = (value, upper) => {
    const [date, clock = ''] = String(value ?? '').trim().split(/\s+/);
    if (!date || !date.includes('-')) return null;
    const parts = date.split('-'), fill = upper ? '23:59:59.999999999' : '00:00:00.000000000';
    const time = clock + fill.slice(clock.length);
    return {year:parts.length === 3 ? parts[0] : null, key:`${parts.at(-2)}-${parts.at(-1)} ${time}`};
  };
  const start = point(from, false), end = point(to, true);
  if (!start || !end || (start.year === null) !== (end.year === null)) return false;
  return `${start.year ?? ''}${start.key}` > `${end.year ?? ''}${end.key}`;
}
// Filter lists as the core reads them: trimmed, empty items dropped, the
// first of exact duplicates kept. Tags may contain spaces, so only commas
// separate them; IDs are digits, so whitespace separates them too.
export const MAX_LIST_ITEMS = 32;
const unique = items => [...new Set(items.filter(Boolean))];
export const tagList = value => unique(String(value ?? '').split(',').map(item => item.trim()));
export const idList = value => unique(String(value ?? '').split(/[\s,]+/));
export function idListError(value) {
  const ids = idList(value);
  if (ids.some(id => !/^\d+$/.test(id))) return 'PID와 TID에는 숫자만 쉼표나 공백으로 구분해 입력해 주세요.';
  return ids.length > MAX_LIST_ITEMS ? `PID와 TID는 각각 최대 ${MAX_LIST_ITEMS}개까지 입력할 수 있습니다.` : '';
}
export const tagListError = value => tagList(value).length > MAX_LIST_ITEMS ? `태그는 조건마다 최대 ${MAX_LIST_ITEMS}개까지 입력할 수 있습니다.` : '';

// A filter shows the rows that match one of its showing conditions (all rows
// without one) and none of its hiding conditions. A condition holds levels and
// the tag, PID, TID, time and message criteria a row must all match; the
// message is text the row's message contains, a pattern with `messageRegex`.
// `enabled` is the page's own switch. A condition without criteria is skipped.
export const MAX_CONDITIONS = 32;
const TEXT = 1000;
// The engine's error for a condition's message pattern starts so; only the
// engine knows its grammar.
export const MESSAGE_PATTERN_ERROR = 'Invalid or unsupported message pattern: ';
export const emptyCondition = () => ({levels:[...LEVELS], tag:'', pid:'', tid:'', timeFrom:'', timeTo:'', message:'', messageRegex:false, enabled:true});
// A condition without `levels` has every level.
const levelsOf = c => Array.isArray(c.levels) ? LEVELS.filter(level => c.levels.includes(level)) : [...LEVELS];
export const isEmptyCondition = c => levelsOf(c).length === LEVELS.length && !tagList(c.tag).length && !idList(c.pid).length && !idList(c.tid).length && !String(c.timeFrom ?? '').trim() && !String(c.timeTo ?? '').trim() && !String(c.message ?? '');
export const isActiveCondition = c => c.enabled !== false && !isEmptyCondition(c);
// A condition of known fields of the right type, or null.
export function readCondition(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const c = emptyCondition();
  if (Array.isArray(value.levels)) c.levels = LEVELS.filter(level => value.levels.includes(level));
  for (const key of ['tag', 'pid', 'tid', 'timeFrom', 'timeTo', 'message']) if (typeof value[key] === 'string') c[key] = value[key].slice(0, TEXT);
  c.messageRegex = value.messageRegex === true;
  c.enabled = value.enabled !== false;
  return c;
}
export const defaultFilters = () => ({include:[emptyCondition()], exclude:[], includeRaw:true, query:'', caseSensitive:false, recognizeLineNumbers:null, searchMode:'literal'});
// A fresh copy with every option, conditions read as `readCondition` does.
export function normalizeFilters(f = {}) {
  const d = defaultFilters(), list = (value, fallback) => Array.isArray(value) ? value.map(readCondition).filter(Boolean) : fallback;
  return {...d, ...f, include:list(f.include, d.include), exclude:list(f.exclude, d.exclude)};
}
// One condition as the core reads it; a message is used as typed, and
// without one its mode means nothing.
const coreCondition = c => ({levels:levelsOf(c), tag:c.tag ?? '', pid:c.pid ?? '', tid:c.tid ?? '', timeFrom:c.timeFrom ?? '', timeTo:c.timeTo ?? '', message:c.message ?? '', messageRegex:!!c.message && c.messageRegex === true});
// The filter as the core reads it: enabled conditions with criteria.
export function coreFilters(f) {
  return {include:(f.include ?? []).filter(isActiveCondition).map(coreCondition), exclude:(f.exclude ?? []).filter(isActiveCondition).map(coreCondition), includeRaw:f.includeRaw !== false,
    query:f.query ?? '', caseSensitive:!!f.caseSensitive, searchMode:f.searchMode === 'regex' ? 'regex' : 'literal', recognizeLineNumbers:f.recognizeLineNumbers ?? null};
}
// Positions in `conds` of the conditions no row of the log matches. The
// engine reports a state for each condition a query sent (`sent`, as
// `coreFilters` sent them); a condition is marked only while it still reads
// as the one sent at its place among the active ones, so an edit not yet
// applied marks nothing.
export function conditionsWithoutRows(conds, sent, states) {
  const out = {include:new Set(), exclude:new Set()};
  for (const list of ['include', 'exclude']) {
    let k = 0;
    (conds?.[list] ?? []).forEach((c, index) => {
      if (!isActiveCondition(c)) return;
      const was = sent?.[list]?.[k], state = states?.[list]?.[k];
      k++;
      if (state === 'none' && was && JSON.stringify(coreCondition(c)) === JSON.stringify(was)) out[list].add(index);
    });
  }
  return out;
}
// Why a condition cannot be applied, or ''.
export function conditionProblem(c) {
  const problem = idListError(c.pid) || idListError(c.tid) || tagListError(c.tag);
  if (problem) return problem;
  return isTimeBound(c.timeFrom) && isTimeBound(c.timeTo) ? '' : '시간 형식을 확인해 주세요.';
}
export function validateFilters(input = {}) {
  const conditions = [...(input.include ?? []), ...(input.exclude ?? [])];
  if (conditions.length > MAX_CONDITIONS) throw new Error(`조건은 ${MAX_CONDITIONS}개까지 쓸 수 있습니다.`);
  for (const c of conditions) {
    const problem = conditionProblem(c);
    if (problem) throw new Error(problem);
  }
}
