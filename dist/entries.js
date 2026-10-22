// Multi-line log entries in row lists: a row without fields that continues
// a parsed entry carries that entry row's line and fields as `owner`.
import {LEVEL_NAMES, formatNumber, rowSource} from './describe.js';
import {textSpan} from './dom.js';

export function continuationOwner(entry) {
  const owner = entry?.kind !== 'parsed' ? entry?.owner : null;
  return owner && Number.isSafeInteger(owner.lineNumber) && owner.lineNumber > 0 && owner.lineNumber < entry.lineNumber ? owner : null;
}
// The entry row a list row belongs to: itself when parsed, else its owner.
export const recordLine = entry => entry?.kind === 'parsed' ? entry.lineNumber : continuationOwner(entry)?.lineNumber ?? null;
// Whether `next` is the following physical row of the same entry, so no
// separator splits the two rows.
export const joinsNext = (entry, next) => recordLine(entry) !== null && next?.lineNumber === entry.lineNumber + 1 && continuationOwner(next)?.lineNumber === recordLine(entry);
// Row classes of a continuation row: the entry's level tint and guide.
export const continuationClass = owner => `continuation-row level-row-${LEVEL_NAMES[owner.level] ? owner.level : 'U'}`;
// Tooltip naming the entry a continuation row belongs to, in its own file.
export function ownerLabel(entry, owner, files) {
  const source = rowSource(entry, files), line = source ? source.line - (entry.lineNumber - owner.lineNumber) : owner.lineNumber;
  const fields = [[owner.date, owner.time].filter(Boolean).join(' '), LEVEL_NAMES[owner.level] ?? owner.level, owner.tag, owner.pid && `PID ${owner.pid}`, owner.tid && `TID ${owner.tid}`].filter(Boolean).map(String);
  return `${source ? `${source.name} · ` : ''}${formatNumber(line)}행 로그에 이어지는 줄${fields.length ? `\n${fields.join(' · ')}` : ''}`;
}
// A continuation row's field cells: empty under its entry row in the logs;
// dimmed owner fields in sparse lists (search results, bookmarks) so a row
// shows which log printed it. Every cell names the entry in its tooltip.
export function ownerCells(document, entry, owner, files, show) {
  const text = value => show ? String(value ?? '') : '';
  const cells = [textSpan(document, text([owner.date, owner.time].filter(Boolean).join(' ')), 'timestamp owner-field'), textSpan(document, text(owner.pid), 'pid owner-field'), textSpan(document, text(owner.tid), 'tid owner-field')];
  const level = LEVEL_NAMES[owner.level] ? owner.level : 'U';
  cells.push(show ? textSpan(document, LEVEL_NAMES[level], `level level-${level} owner-field`) : textSpan(document, '', 'owner-field'), textSpan(document, text(owner.tag), 'tag owner-field'));
  const label = ownerLabel(entry, owner, files);
  for (const cell of cells) cell.title = label;
  return cells;
}
// The fields a stored row keeps of its owner.
export function ownerSnapshot(entry) {
  const owner = continuationOwner(entry);
  return owner ? {lineNumber:owner.lineNumber, date:owner.date, time:owner.time, level:owner.level, tag:owner.tag, pid:owner.pid, tid:owner.tid} : undefined;
}
