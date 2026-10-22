// Column sets by log format. Text logs (Android, iOS) and DLT files show
// different fields; the layout, row cells and level names follow the open
// log's format.
export const PROFILES = {
  text: {
    labels: ['행', '날짜 · 시간', 'PID', 'TID', '레벨', '태그', '메시지'],
    defaults: [52, 180, 56, 56, 76, 128],
    min: [36, 100, 40, 40, 56, 64],
    max: [200, 640, 256, 256, 200, 1024],
    fit: [96, 260, 96, 96, 100, 320],
    level: 4,
  },
  dlt: {
    labels: ['행', '시간', '타임스탬프', 'ECU', 'APID', 'CTID', '레벨', '페이로드'],
    // The uptime fits its widest value, 429496.7295 (a 32-bit count of
    // 0.1 ms), with room for wider monospace fonts.
    defaults: [52, 200, 104, 56, 56, 56, 76],
    min: [36, 100, 48, 40, 40, 40, 56],
    max: [200, 640, 256, 128, 128, 128, 200],
    fit: [96, 260, 140, 96, 96, 96, 100],
    level: 6,
  },
};
export const profileOf = format => format === 'dlt' ? 'dlt' : 'text';

const NAMES = {V:'Verbose', D:'Debug', I:'Info', W:'Warn', E:'Error', F:'Fatal', A:'Assert', U:'Unknown'};
// DLT messages other than logs (control, traces) have no level: `U`.
const DLT_NAMES = {...NAMES, U:'기타'};
export const levelNames = format => format === 'dlt' ? DLT_NAMES : NAMES;

// A DLT message's own fields; the rows' tags are its ECU, APID and CTID.
export const DLT_IDS = [['ecu', 'ECU'], ['apid', 'APID'], ['ctid', 'CTID']];
// Field cells of a DLT row after its number: time, uptime, IDs, level and
// payload. `cell(text, className)` builds a field cell, `level` the badge.
export function dltCells(entry, cell, level) {
  const dlt = entry.dlt ?? {};
  return [
    cell(`${entry.date ?? ''} ${entry.time ?? ''}`, 'timestamp'),
    cell(dlt.uptime ?? '', 'uptime'),
    ...DLT_IDS.map(([key]) => cell(dlt[key] ?? '', key)),
    level,
    cell(entry.message ?? '', 'message'),
  ];
}
// Tooltip of a DLT level badge: the message type of messages without a log
// level, such as `control:response`.
export const dltLevelTitle = entry => entry.level === 'U' && entry.dlt?.kind ? entry.dlt.kind : '';
