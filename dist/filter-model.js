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
// Fields that hold one value, or with their `Regex` switch a pattern.
export const VALUE_FIELDS = ['tag', 'pid', 'tid', 'ecu', 'apid', 'ctid'];
export const REGEX_FIELDS = [...VALUE_FIELDS, 'message'];
export const regexKey = key => `${key}Regex`;
const trimmed = value => String(value ?? '').trim();
// Why a PID or TID value cannot apply. A comma says what it means to the
// person who typed a list; a pattern may be any text.
export function numberError(value) {
  const text = trimmed(value);
  if (!text || /^\d+$/.test(text)) return '';
  return /[\s,]/.test(text) ? 'PID와 TID는 값 하나만 입력합니다. 여러 값은 정규식으로 찾습니다.' : 'PID와 TID에는 숫자만 입력해 주세요.';
}
// What a comma in a text field's value means, shown under the field.
export const commaHint = (value, regex) => !regex && String(value ?? '').includes(',') ? '쉼표도 글자입니다. 여러 값은 정규식으로 찾습니다.' : '';

// Conditions saved before fields held one value: a tag took a comma-separated
// list and a PID or TID a list split by commas or spaces, each item matching
// as a value does. One item stays a value, several become a pattern that
// matches the same rows.
const escapePattern = text => text.replace(/[\\.+*?()|[\]{}^$#&~-]/g, '\\$&');
const items = (value, split) => [...new Set(String(value ?? '').split(split).map(item => item.trim()).filter(Boolean))];
const listed = (key, value) => items(value, key === 'pid' || key === 'tid' ? /[\s,]+/ : ',');
// The pattern of a list typed into a field: the items as they matched in a
// tag, or whole in a field of whole values.
const listPattern = (key, list) => {
  const body = list.map(escapePattern).join('|');
  return key === 'tag' ? body : `^(${body})$`;
};
// A field's value as a pattern when it lists several items, else ''.
export const patternOfList = (key, value) => {
  const list = listed(key, value);
  return list.length > 1 ? listPattern(key, list) : '';
};
export function upgradeCondition(c) {
  const out = {...c};
  for (const key of ['tag', 'pid', 'tid']) {
    const list = listed(key, c[key]);
    out[key] = list.length > 1 ? listPattern(key, list) : list[0] ?? '';
    out[regexKey(key)] = list.length > 1;
  }
  return out;
}

// A filter shows the rows that match one of its showing conditions (all rows
// without one) and none of its hiding conditions. A condition holds levels and
// the tag, PID, TID, DLT ID, time and message criteria a row must all match.
// `enabled` is the page's own switch. A condition without criteria is skipped.
export const MAX_CONDITIONS = 32;
const TEXT = 1000, TAG_TEXT = 4000;
// The engine's error for a condition's pattern names its field; only the
// engine knows the grammar.
const PATTERN_ERROR = /^Invalid or unsupported (tag|PID|TID|ECU|APID|CTID|message) pattern: /;
export const patternField = text => PATTERN_ERROR.exec(String(text ?? ''))?.[1]?.replace(/^[A-Z]+$/, name => name.toLowerCase()) ?? null;
const noValues = () => Object.fromEntries(REGEX_FIELDS.flatMap(key => [[key, ''], [regexKey(key), false]]));
export const emptyCondition = () => ({levels:[...LEVELS], ...noValues(), timeFrom:'', timeTo:'', enabled:true});
// A condition without `levels` has every level.
const levelsOf = c => Array.isArray(c.levels) ? LEVELS.filter(level => c.levels.includes(level)) : [...LEVELS];
export const isEmptyCondition = c => levelsOf(c).length === LEVELS.length && !VALUE_FIELDS.some(key => trimmed(c[key])) && !trimmed(c.timeFrom) && !trimmed(c.timeTo) && !String(c.message ?? '');
export const isActiveCondition = c => c.enabled !== false && !isEmptyCondition(c);
// A condition of known fields of the right type, or null.
export function readCondition(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const c = emptyCondition();
  if (Array.isArray(value.levels)) c.levels = LEVELS.filter(level => value.levels.includes(level));
  // A list saved before patterns grows when it becomes one, within the core's
  // bound for a tag.
  for (const key of [...REGEX_FIELDS, 'timeFrom', 'timeTo']) if (typeof value[key] === 'string') c[key] = value[key].slice(0, key === 'tag' ? TAG_TEXT : TEXT);
  for (const key of REGEX_FIELDS) c[regexKey(key)] = value[regexKey(key)] === true;
  c.enabled = value.enabled !== false;
  return c;
}
export const defaultFilters = () => ({include:[emptyCondition()], exclude:[], includeRaw:true, query:'', caseSensitive:false, recognizeLineNumbers:null, searchMode:'literal'});
// A fresh copy with every option, conditions read as `readCondition` does.
export function normalizeFilters(f = {}) {
  const d = defaultFilters(), list = (value, fallback) => Array.isArray(value) ? value.map(readCondition).filter(Boolean) : fallback;
  return {...d, ...f, include:list(f.include, d.include), exclude:list(f.exclude, d.exclude)};
}
// The filter switched off as a whole: no condition applies and every row
// shows. The search and the reading options stay.
export const withoutConditions = f => ({...f, include:[], exclude:[], includeRaw:true});
// One condition as the core reads it: values trimmed, a message as typed, and
// a field without a value has no mode.
function coreCondition(c) {
  const out = {levels:levelsOf(c), timeFrom:c.timeFrom ?? '', timeTo:c.timeTo ?? ''};
  for (const key of REGEX_FIELDS) {
    out[key] = key === 'message' ? c.message ?? '' : trimmed(c[key]);
    out[regexKey(key)] = !!out[key] && c[regexKey(key)] === true;
  }
  return out;
}
export const coreForm = coreCondition;
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
// The field that keeps a condition from applying, with why, or null.
export function conditionFault(c) {
  for (const key of ['pid', 'tid']) {
    const problem = c[regexKey(key)] === true ? '' : numberError(c[key]);
    if (problem) return {key, problem};
  }
  return isTimeBound(c.timeFrom) && isTimeBound(c.timeTo) ? null : {key:'time', problem:'시간 형식을 확인해 주세요.'};
}
export const conditionProblem = c => conditionFault(c)?.problem ?? '';
export function validateFilters(input = {}) {
  const conditions = [...(input.include ?? []), ...(input.exclude ?? [])];
  // The blank condition the panel keeps is no condition.
  if (conditions.filter(c => !isEmptyCondition(c)).length > MAX_CONDITIONS) throw new Error(`조건은 ${MAX_CONDITIONS}개까지 쓸 수 있습니다.`);
  for (const c of conditions) {
    const problem = conditionProblem(c);
    if (problem) throw new Error(problem);
  }
}
