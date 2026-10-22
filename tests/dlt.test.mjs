import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {PROFILES, dltCells, dltLevelTitle, levelNames, profileOf} from '../dist/columns.js';
import {describeActiveFilters, describeFileError, describeLogError} from '../dist/describe.js';
import {markOf} from '../dist/bookmarks.js';

// Native page rows of a DLT file (core/tests/dlt.rs writes the fixture).
const page = JSON.parse(await readFile(new URL('./dlt-page.json', import.meta.url), 'utf8'));

test('DLT rows fill their columns from the native page fields', () => {
  const [row] = page.rows;
  const cells = dltCells(row, (text, className) => ({text, className}), {level:row.level});
  assert.deepEqual(cells.map(c => c.className ?? 'level'), ['timestamp', 'uptime', 'ecu', 'apid', 'ctid', 'level', 'message']);
  assert.deepEqual(cells.map(c => c.text ?? c.level), ['2026-03-14 09:26:53.589793', '1234.5678', 'ECU1', 'NAV', 'RTE', 'W', 'Route recalculated: 3 km']);
  assert.equal(PROFILES.dlt.labels.length, cells.length + 1, 'one column per cell after the row number');
  for (const p of Object.values(PROFILES)) {
    assert.equal(p.labels[p.level], '레벨');
    for (const key of ['defaults', 'min', 'max', 'fit']) assert.equal(p[key].length + 1, p.labels.length, key);
  }
});

test('messages without a log level read as 기타 with their type in the tooltip', () => {
  const control = page.rows.find(r => r.dlt?.kind === 'control:response');
  assert.equal(levelNames('dlt')[control.level], '기타');
  assert.equal(levelNames('text').U, 'Unknown');
  assert.equal(dltLevelTitle(control), 'control:response');
  assert.equal(dltLevelTitle(page.rows[0]), '');
  assert.equal(profileOf('dlt'), 'dlt');
  assert.equal(profileOf(undefined), 'text');
});

test('DLT filter chips and errors read in Korean', () => {
  assert.deepEqual(describeActiveFilters({include:[{tag:'NAV, ECU1'}]}, 'dlt').map(c => c.label), ['ID: NAV, ECU1']);
  assert.deepEqual(describeActiveFilters({include:[{tag:'NAV'}]}).map(c => c.label), ['태그: NAV']);
  for (const error of ['DLT files and text logs cannot be opened together', 'A selected DLT file contains no DLT messages', 'DLT version 2 files are not supported',
    'DLT files cannot be followed', 'Invalid export format', 'Could not read the DLT file', 'DLT data without storage headers is not supported'])
    assert.ok(!describeLogError(error).includes(error), error);
});

test('an open of several files names the file that failed', () => {
  assert.equal(describeFileError('DLT version 2 files are not supported', 'b.dlt'), 'b.dlt: DLT 버전 2 파일은 지원하지 않습니다.');
  assert.equal(describeFileError('DLT version 2 files are not supported', ''), 'DLT 버전 2 파일은 지원하지 않습니다.');
  // The named message passes the later translation unchanged.
  const named = describeFileError('NUL is not supported; select a UTF-8 text log', 'trace.log');
  assert.equal(describeLogError(named), named);
  assert.equal(describeFileError('', 'b.dlt'), '');
});

test("a bookmark keeps a DLT row's fields", () => {
  assert.deepEqual(markOf(page.rows[1]).dlt, page.rows[1].dlt);
  assert.equal(markOf({lineNumber:1, raw:'x', kind:'raw'}).dlt, undefined);
});
