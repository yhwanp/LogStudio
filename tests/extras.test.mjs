import test from 'node:test';
import assert from 'node:assert/strict';
import {mountExtras} from '../dist/extras.js';
import {Points, describeTrack, describeSelected, describePass, describeTiles} from '../extras/web/track/track-model.js';

// A native bridge whose `extras` command answers from `answers` and records
// every request; `emit` sends an event the page listens to.
function fakeNative(answers) {
  const calls = [], listeners = {};
  return {
    calls,
    emit: (name, payload) => listeners[name]?.({payload}),
    core:{invoke: async (command, {request}) => {
      assert.equal(command, 'extras');
      calls.push(request);
      const answer = answers[request.action];
      // Tauri rejects with the command's error text.
      if (answer instanceof Error) throw answer.message;
      return typeof answer === 'function' ? answer(request) : answer;
    }},
    event:{listen: async (name, fn) => {listeners[name] = fn;}},
  };
}
function button() {
  const listeners = {};
  return {hidden:true, addEventListener: (k, fn) => {listeners[k] = fn;}, click: () => listeners.click(), getBoundingClientRect: () => ({left:10, bottom:20})};
}
const settle = () => new Promise(resolve => setTimeout(resolve, 0));
function mount(answers, more = {}) {
  const native = fakeNative(answers), b = button(), shown = [], notes = [], located = [], timers = [], focus = [];
  const extras = mountExtras({
    button:b, native, notify:message => notes.push(message), isGone:() => false,
    window:{addEventListener:(name, fn) => {if (name === 'focus') focus.push(fn);}},
    menu:{show: (items, x, y, label) => shown.push({items, x, y, label})},
    locate:(datasetId, line) => located.push([datasetId, line]),
    current:() => more.current ?? null,
    setTimer:fn => {timers.push(fn);return timers.length;}, clearTimer:id => {timers[id - 1] = null;},
  });
  const flush = () => {for (const fn of timers.splice(0)) fn?.();};
  return {native, button:b, shown, notes, located, extras, flush, focus:() => focus.forEach(fn => fn())};
}

test('a build without extensions keeps the button hidden and sends no rows', async () => {
  const m = mount({list:[]});
  await settle();
  assert.equal(m.button.hidden, true);
  m.extras.selected('logs', 1, 40);
  m.flush();
  assert.deepEqual(m.native.calls, [{action:'list'}]);
  // An older build without the command lists none either.
  const old = mount({list:new Error('Unknown command')});
  await settle();
  assert.equal(old.button.hidden, true);
});

test('the menu lists extensions, opens one, and names a broken one', async () => {
  const m = mount({list:[{id:'track', title:'차량 궤적'}, {id:'other', title:'다른 확장', error:'설정 파일이 올바른 JSON이 아닙니다 (2행)'}], open:{}, select:{}}, {current:{datasetId:3, line:9}});
  await settle();
  assert.equal(m.button.hidden, false);
  m.button.click();
  await settle();
  const [menu] = m.shown;
  assert.equal(menu.label, '확장 기능');
  assert.deepEqual(menu.items.map(item => item?.[0] ?? null), ['차량 궤적', '다른 확장 · 설정 오류', null, '설정 폴더 열기']);
  // A broken one cannot be chosen; what is wrong stays under it.
  const [, action, , options] = menu.items[1];
  assert.equal(action, null);
  assert.deepEqual(options, {disabled:true, note:'extras/other.json · 설정 파일이 올바른 JSON이 아닙니다 (2행)'});
  menu.items[0][1]();
  await settle();
  // A window that opens shows the row selected now.
  assert.deepEqual(m.native.calls.slice(-2), [{action:'open', id:'track'}, {action:'select', datasetId:3, line:9}]);
  menu.items[3][1]();
  await settle();
  assert.deepEqual(m.native.calls.at(-1), {action:'reveal'});
});

test('a settings file written while the app runs shows the button when the window comes back', async () => {
  let listing = [];
  const m = mount({list:() => listing});
  await settle();
  assert.equal(m.button.hidden, true);
  listing = [{id:'track', title:'궤적'}];
  m.focus();
  await settle();
  assert.equal(m.button.hidden, false);
});

test('a window that cannot open says why', async () => {
  const m = mount({list:[{id:'track', title:'궤적'}], open:new Error('확장 기능 창을 열 수 없습니다')});
  await settle();
  m.button.click();
  await settle();
  m.shown[0].items[0][1]();
  await settle();
  assert.deepEqual(m.notes, ['확장 기능 창을 열 수 없습니다']);
});

test('selected rows are sent once they settle, each pane on its own', async () => {
  const m = mount({list:[{id:'track', title:'궤적'}], select:{}});
  await settle();
  m.extras.selected('logs', 1, 10);
  m.extras.selected('logs', 1, 11);
  m.flush();
  assert.deepEqual(m.native.calls.slice(1), [{action:'select', datasetId:1, line:11}]);
  // The search pane's row is sent; the logs repainting with its own row
  // again sends nothing.
  m.extras.selected('search', 1, 50);
  m.flush();
  m.extras.selected('logs', 1, 11);
  m.flush();
  m.extras.selected('logs', 1, null);
  m.flush();
  assert.deepEqual(m.native.calls.slice(1), [{action:'select', datasetId:1, line:11}, {action:'select', datasetId:1, line:50}]);
});

test('a row an extension picks is located in the main window', async () => {
  const m = mount({list:[{id:'track', title:'궤적'}]});
  await settle();
  m.native.emit('extras-locate', {datasetId:2, line:700});
  m.native.emit('extras-locate', {datasetId:2, line:'700'});
  assert.deepEqual(m.located, [[2, 700]]);
});

test('points grow, find the row a place was logged at, and the passes near a click', () => {
  const p = new Points();
  const triples = [];
  for (let i = 0; i < 3000; i++) triples.push(10 + 3 * i, 37 + i / 1e4, 127 + i / 1e4);
  p.append(Float64Array.from(triples));
  assert.equal(p.count, 3000);
  assert.equal(p.at(9), -1);
  assert.equal(p.at(10), 0);
  assert.equal(p.at(12), 0);
  assert.equal(p.at(13), 1);
  assert.equal(p.at(1e9), 2999);
  const revision = p.revision;
  p.truncate(1000);
  assert.equal(p.at(1e9), 999);
  p.truncate(5000);
  assert.equal(p.count, 1000);
  p.append(Float64Array.from([5000, 1, 2]));
  assert.equal(p.count, 1001);
  assert.equal(p.lines[1000], 5000);
  assert.ok(p.revision > revision + 1);
});

test('a place the track passes several times is one pass per visit, nearest point each', () => {
  // Out along a line and back over it: points 0-4 and 10-14 lie near x=2.
  const p = new Points();
  const xs = [0, 1, 2, 3, 4, 9, 9, 9, 9, 9, 4, 3, 2, 1, 0];
  p.append(Float64Array.from(xs.flatMap((x, i) => [100 + i, 0, x])));
  const box = {south:-1, north:1, west:-180, east:180};
  const distance = i => Math.abs(p.lon[i] - 2.1);
  const passes = p.passes(box, distance, 1.5);
  assert.deepEqual(passes.map(pass => pass.index), [2, 12]);
  assert.equal(describePass(p, passes, 0), '102행 · 이곳을 2번 지남 (1/2)');
  assert.equal(describePass(p, passes, 1), '112행 · 이곳을 2번 지남 (2/2)');
  // The ends of the track say so.
  assert.equal(describePass(p, [{index:0}], 0), '시작 · 100행');
  assert.equal(describePass(p, [{index:14}], 0), '끝 · 114행');
  assert.deepEqual(p.passes({south:5, north:6, west:0, east:1}, distance, 1.5), []);
});

test('the status line says how reading stands', () => {
  const marker = 'onPositionChanged';
  assert.deepEqual(describeTrack({phase:'reading', count:1200, skipped:0}, {marker}), {text:'읽는 중… 지점 1,200개', failed:false, note:null});
  assert.equal(describeTrack({phase:'following', count:5, skipped:1}, {marker}).text, '새 줄을 기다리는 중 · 지점 5개 · 좌표를 읽지 못한 행 1개');
  assert.equal(describeTrack({phase:'complete', count:0, skipped:0}, {marker}).note, '이 로그에는 “onPositionChanged”가 있는 행이 없습니다.');
  assert.match(describeTrack({phase:'complete', count:0, skipped:4}, {marker}).note, /행 4개에서 좌표를 읽지 못했습니다/);
  assert.deepEqual(describeTrack({phase:'failed', error:'로그를 읽지 못했습니다.'}, {marker}), {text:'로그를 읽지 못했습니다.', failed:true, note:null});
  assert.equal(describeTrack({phase:'full', count:2000000, skipped:0}, {marker}).text, '지점 2,000,000개에서 멈춤 · 한 로그에 최대 2,000,000개까지 표시합니다');
});

test('the selected row says where it was logged, and when that is not known yet', () => {
  const p = new Points();
  p.append(Float64Array.from([10, 52.5, 13.4, 20, 52.50001, 13.40002]));
  const done = {phase:'complete', through:100};
  assert.deepEqual(describeSelected(p, 20, done), {text:'20행 위치 · 52.50001, 13.40002', mark:'exact'});
  assert.deepEqual(describeSelected(p, 25, done), {text:'25행 → 20행 지점 · 52.50001, 13.40002', mark:'before'});
  assert.deepEqual(describeSelected(p, 3, done), {text:'3행 앞에는 지점이 없습니다', mark:'none'});
  assert.deepEqual(describeSelected(p, null, done), {text:'', mark:'none'});
  // A row the reading has not reached may still get a nearer point.
  assert.deepEqual(describeSelected(p, 500, {phase:'reading', through:300}), {text:'500행은 아직 읽지 않았습니다 · 300행까지 읽음', mark:'pending'});
  assert.deepEqual(describeSelected(p, 500, {phase:'following', through:300}).mark, 'pending');
  assert.deepEqual(describeSelected(p, 250, {phase:'reading', through:300}).mark, 'before');
  assert.deepEqual(describeSelected(p, 500, {phase:'full', through:300}), {text:'500행은 표시 한도 밖입니다', mark:'none'});
});

test('missing tiles name the server to check', () => {
  assert.equal(describeTiles(0, 'https://tile.openstreetmap.org/{z}/{x}/{y}.png'), '');
  assert.equal(describeTiles(13, 'https://tile.openstreetmap.org/{z}/{x}/{y}.png'), '지도 타일 13개를 받지 못했습니다 · 네트워크 연결을 확인해 주세요');
  assert.equal(describeTiles(2, 'http://127.0.0.1:8080/{z}/{x}/{y}.png'), '지도 타일 2개를 받지 못했습니다 · 이 컴퓨터의 타일 서버를 확인해 주세요');
});
