// Work kept across a reload and saved per opened file set: applied filters
// and search, the saved filter they came from, bookmarked physical rows, and
// the first visible and selected row of each list. Native code names the files; the page saves and asks only by
// dataset ID. No log text is saved: restored bookmarks read their rows again.
import {validateFilters, defaultFilters, readCondition, emptyCondition, tagList, MAX_CONDITIONS} from './filter-model.js';
import {sameFields} from './saved-filters.js';
import {MAX_BOOKMARKS, markOf} from './bookmarks.js';
import {formatNumber} from './describe.js';

export const WORK_VERSION = 1;
export const SAVE_DELAY_MS = 500;
// Rows one native `lines` request may name.
export const LINES_PER_REQUEST = 200;
const TEXT = 1000;
const line = value => Number.isSafeInteger(value) && value > 0 ? value : null;
export {defaultFilters};
// Conditions compare as the core reads them, then the search and options.
const sameFilters = (a, b) => sameFields(a, b) && ['query', 'caseSensitive', 'recognizeLineNumbers', 'searchMode'].every(key => (a[key] ?? null) === (b[key] ?? null));

// Known filter fields of the right type only. A set the filters would reject
// falls back to the defaults as a whole rather than half-applied. Work saved
// before conditions has a filter's fields: one showing condition and the
// excluded tags as one hiding condition.
export function readSavedFilters(value) {
  const filters = defaultFilters();
  if (!value || typeof value !== 'object') return filters;
  if (typeof value.query === 'string') filters.query = value.query.slice(0, TEXT);
  for (const key of ['caseSensitive', 'includeRaw']) if (typeof value[key] === 'boolean') filters[key] = value[key];
  if (value.recognizeLineNumbers === null || typeof value.recognizeLineNumbers === 'boolean') filters.recognizeLineNumbers = value.recognizeLineNumbers;
  if (value.searchMode === 'regex' || value.searchMode === 'literal') filters.searchMode = value.searchMode;
  const list = v => (Array.isArray(v) ? v : []).slice(0, MAX_CONDITIONS).map(readCondition).filter(Boolean);
  if (Array.isArray(value.include) || Array.isArray(value.exclude)) {
    [filters.include, filters.exclude] = [list(value.include), list(value.exclude)];
  } else {
    filters.include = [readCondition(value)];
    if (typeof value.excludeTags === 'string' && tagList(value.excludeTags).length) filters.exclude = [{...emptyCondition(), tag:value.excludeTags.slice(0, TEXT)}];
  }
  if (!filters.include.length) filters.include = [emptyCondition()];
  try {validateFilters(filters); return filters;} catch {return defaultFilters();}
}
const readPosition = value => ({top:line(value?.top), selected:line(value?.selected)});
// A state this page saved, or null.
export function readWork(value) {
  if (!value || typeof value !== 'object' || value.v !== WORK_VERSION) return null;
  const marks = Array.isArray(value.bookmarks) ? [...new Set(value.bookmarks.map(line).filter(Boolean))].sort((a, b) => a - b).slice(0, MAX_BOOKMARKS) : [];
  return {filters:readSavedFilters(value.filters), savedFilter:line(value.savedFilter), bookmarks:marks, upper:readPosition(value.upper), lower:readPosition(value.lower)};
}
// Work with nothing to restore is forgotten instead of saved.
export const isEmptyWork = work => sameFilters(work.filters, defaultFilters()) && !work.bookmarks.length && ![work.upper, work.lower].some(p => p?.top || p?.selected);
export const writeWork = ({filters, savedFilter=null, bookmarks, upper, lower}) => ({v:WORK_VERSION, filters, savedFilter, bookmarks, upper, lower});

// After a reload the marks read again replace their old rows. Rows past the
// new end are dropped; a row whose text differs is counted as changed. Marks
// added since the read began keep their rows.
export function mergeReloadMarks(current, old, rows, total) {
  const entries = []; let changed = 0, dropped = 0;
  for (const [at, mark] of current) {
    if (at > total) {dropped++; continue;}
    const row = rows.get(at);
    if (!row) {entries.push([at, mark]); continue;}
    const fresh = markOf(row), before = old.get(at);
    if (before && (before.raw !== fresh.raw || before.truncated !== fresh.truncated)) changed++;
    entries.push([at, fresh]);
  }
  return {entries, changed, dropped};
}
// Saved marks join the marks made since the file opened, up to the limit;
// lines the file no longer has are reported.
export function mergeSavedMarks(current, lines, rows) {
  const entries = [...current]; let dropped = 0;
  for (const at of lines) {
    if (current.has(at)) continue;
    const row = rows.get(at);
    if (row && entries.length < MAX_BOOKMARKS) entries.push([at, markOf(row)]); else dropped++;
  }
  return {entries:entries.sort((a, b) => a[0] - b[0]), dropped};
}

// Toast copy (kept short: toasts do not wrap). A reload says what it kept;
// changed content is reported once the bookmarked rows have been read again.
export const RELOADED = '파일을 다시 열었습니다. 필터·검색·북마크·위치를 유지합니다.';
export function describeReloadMarks({changed=0, dropped=0, shrank=false}) {
  const shifted = '위치가 밀렸을 수 있습니다.';
  if (dropped && changed) return `북마크 ${formatNumber(dropped)}개 해제·${formatNumber(changed)}개 내용 바뀜. 파일이 바뀌어 ${shifted}`;
  if (dropped) return `파일이 짧아져 북마크 ${formatNumber(dropped)}개를 해제했습니다. ${shifted}`;
  if (changed) return `북마크한 행 ${formatNumber(changed)}개의 내용이 바뀌었습니다. ${shifted}`;
  return shrank ? `파일이 이전보다 짧아졌습니다. ${shifted}` : '';
}
export function describeRestore(match, work) {
  const filtered = !sameFilters({...work.filters, query:'', caseSensitive:false, searchMode:'literal'}, defaultFilters());
  if (match === 'changed') return filtered || work.filters.query ? '파일이 지난번과 달라 필터와 검색만 복원했습니다.' : '파일이 지난번과 달라 북마크와 위치는 복원하지 않았습니다.';
  const parts = [filtered && '필터', work.filters.query && '검색', work.bookmarks.length && `북마크 ${formatNumber(work.bookmarks.length)}개`, (work.upper.top || work.upper.selected) && '위치'].filter(Boolean);
  return parts.length ? `지난번 작업을 복원했습니다 · ${parts.join(' · ')}` : '';
}
export const describeMissingMarks = dropped => dropped ? `북마크 ${formatNumber(dropped)}개는 파일에 그 행이 없어 복원하지 않았습니다.` : '';

// Saves the tracked dataset's work a moment after it changes, and at once on
// flush (before the file is replaced or closed, or the page goes away).
// Unchanged work is not sent again; native code skips unchanged writes too.
export function createWorkSaver({invoke, capture, setTimer=globalThis.setTimeout, clearTimer=globalThis.clearTimeout, delay=SAVE_DELAY_MS}) {
  let datasetId = null, timer = null, last = null;
  function flush() {
    if (timer !== null) clearTimer(timer); timer = null;
    if (datasetId === null) return;
    const work = capture(); if (!work) return;
    const state = isEmptyWork(work) ? null : writeWork(work), text = JSON.stringify(state);
    if (text === last) return; last = text;
    // The ID is taken now: a stop right after a flush must not rename it.
    const args = {datasetId, state};
    void Promise.resolve().then(() => invoke('save_work_state', args)).catch(() => {});
  }
  return {
    // Saves for `id` from now on; `saved` is the state native code holds.
    track(id, saved=null) {stop(); datasetId = id; last = JSON.stringify(saved);},
    schedule() {if (datasetId === null) return; if (timer !== null) clearTimer(timer); timer = setTimer(flush, delay);},
    flush, stop, tracking:() => datasetId,
  };
  function stop() {if (timer !== null) clearTimer(timer); timer = null; datasetId = null; last = null;}
}
