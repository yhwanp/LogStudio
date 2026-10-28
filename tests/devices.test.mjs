import test from 'node:test';
import assert from 'node:assert/strict';
import {setImmediate as nextTurn} from 'node:timers/promises';
import {describeAdb, deviceProblem, deviceName, deviceMenuItems, appMenuItems, describeCapture, captureBreaks, breakLabel, clockTime, shortDate, installCommand, mountDevices} from '../dist/devices.js';
import {captureState} from '../dist/controller.js';

async function settle() {for (let i = 0; i < 20; i++) await nextTurn();}
const at = (h, m, s) => new Date(2026, 9, 24, h, m, s).getTime();

test('capture states from the engine are checked; other logs have none', () => {
  assert.equal(captureState(undefined), null);
  assert.equal(captureState({state:'unknown'}), null);
  const c = captureState({state:'waiting', device:'Pixel 8', serial:'emulator-5554', bytes:2048, error:7, lost:at(14, 2, 11), unsaved:true, recovered:'yes', breaks:[{row:3, lost:1, resumed:2, reboot:true}, {row:0}, {row:'x'}, null]});
  assert.deepEqual(c, {state:'waiting', device:'Pixel 8', serial:'emulator-5554', bytes:2048, error:'', breaks:[{row:3, lost:1, resumed:2, reboot:true, gap:false}], launches:[], lost:at(14, 2, 11), reason:'', unsaved:true, recovered:false, platform:'android'});
  assert.equal(captureState({state:'recording', platform:'ios'}).platform, 'ios');
  const started = captureState({state:'recording', platform:'ios', launches:[{row:40, at:at(14, 3, 0), app:'SampleApp', process:'SampleCore'}, {row:0, app:'x'}, null]});
  assert.deepEqual(started.launches, [{row:40, at:at(14, 3, 0), app:'SampleApp', process:'SampleCore'}]);
});

test('the device menu lists what adb and Apple\'s device service see, or why they see nothing', () => {
  const calls = [], actions = {refresh:() => calls.push('refresh'), pick:() => calls.push('pick'), help:() => calls.push('help'), appleHelp:() => calls.push('apple'), start:serial => calls.push(serial)};
  // An item as it reads: its label, what follows it, and its second line.
  const read = item => item && [item[0], item[3]?.detail].filter(Boolean).join(' · ') + (item[3]?.note ? ` / ${item[3].note}` : '');
  const labels = items => items.map(read);
  assert.deepEqual(labels(deviceMenuItems({loading:true}, actions)), ['기기를 찾는 중…']);
  // Without adb, iPhones are still listed, and one item leads to getting adb.
  const iphone = {serial:'QA0UDID0000000000000001', state:'device', model:'QA iPhone', product:'iPhone0,0', platform:'ios', version:'27.0'};
  const missing = deviceMenuItems({adb:{found:false}, list:[iphone], androidError:'adb was not found; install Android SDK Platform-Tools'}, actions);
  assert.deepEqual(labels(missing), ['기록할 기기를 선택해 주세요 · 지금부터 들어오는 로그를 기록합니다', 'QA iPhone · iOS 27.0', null, 'Android 기기 로그를 기록하려면 adb를 설치해 주세요…', null, '새로 고침']);
  assert.equal(missing[1][2], 'QA iPhone의 로그를 지금부터 기록합니다', 'an iPhone is not named by its UDID');
  assert.equal(missing[1][3].key, iphone.serial, 'focus follows it when the menu is listed again');
  missing[1][1](); missing[3][1](); assert.deepEqual(calls, [iphone.serial, 'help']);
  assert.deepEqual(labels(deviceMenuItems({adb:{found:false}, list:[]}, actions)).slice(0, 2), ['연결된 기기가 없습니다', 'iPhone·iPad를 케이블이나 Wi-Fi로 연결해 주세요']);
  const adb = {found:true, origin:'studio', version:'37.0.1'};
  assert.equal(describeAdb(adb), 'adb 37.0.1 · Android Studio');
  const none = deviceMenuItems({adb, list:[]}, actions);
  // Picking an adb is offered in the menu only while using a picked one.
  assert.deepEqual(labels(none), ['연결된 기기가 없습니다', 'Android 기기는 USB 디버깅을 켜고, iPhone·iPad는 케이블이나 Wi-Fi로 연결해 주세요', null, '새로 고침']);
  for (const origin of ['sdk', 'path', 'homebrew']) assert.ok(!labels(deviceMenuItems({adb:{...adb, origin}, list:[]}, actions)).includes('adb 위치 지정…'), origin);
  const picked = deviceMenuItems({adb:{...adb, origin:'chosen'}, list:[]}, actions);
  assert.deepEqual(labels(picked).slice(-2), ['새로 고침', 'adb 위치 지정…']);
  assert.equal(picked.at(-1)[2], '지금 쓰는 adb: adb 37.0.1 · 직접 지정');
  picked.at(-1)[1](); assert.deepEqual(calls.slice(-1), ['pick']);
  // Without Apple's device service (Windows without Apple Devices or
  // iTunes), one item leads to getting it; other failures are said as they are.
  const noService = deviceMenuItems({adb, list:[], iosError:'Apple device service not found; install Apple Devices or iTunes'}, actions);
  assert.deepEqual(labels(noService), ['연결된 기기가 없습니다', 'USB 디버깅을 켠 기기를 연결하거나 에뮬레이터를 실행해 주세요', null, 'iPhone·iPad 로그를 기록하려면 Apple Devices 앱을 설치해 주세요…', null, '새로 고침']);
  noService[3][1](); assert.deepEqual(calls.slice(-1), ['apple']);
  assert.deepEqual(labels(deviceMenuItems({adb, list:[], iosError:'Could not list iPhones or iPads; check the connection and retry'}, actions)).slice(3, 4), ['iPhone·iPad 목록을 가져오지 못했습니다. 연결을 확인하고 다시 시도해 주세요.']);
  // An iPad that has not trusted the computer says on a second line how it comes to.
  const ipad = {serial:'QA0UDID0000000000000002', state:'untrusted', model:'QA iPad', product:'iPad0,0', platform:'ios', version:'27.0'};
  const untrusted = deviceMenuItems({adb, list:[ipad]}, actions);
  assert.equal(read(untrusted[1]), 'QA iPad · iPadOS 27.0 / 기기 화면의 "이 컴퓨터를 신뢰하겠습니까?"에서 신뢰를 누르고 암호를 입력해 주세요');
  assert.equal(untrusted[1][1], null);
  assert.equal(untrusted[1][3].disabled, true);
  assert.match(untrusted[1][2], /케이블을 다시 연결해 주세요/);
  assert.equal(read(deviceMenuItems({adb, list:[{...ipad, state:'device'}]}, actions)[1]), 'QA iPad · iPadOS 27.0');
  assert.equal(read(deviceMenuItems({adb, list:[{...ipad, state:'locked'}]}, actions)[1]), 'QA iPad · iPadOS 27.0 / 기기 잠금을 풀어 주세요');
  // iPhones named alike (new ones are all "iPhone") show the end of their UDIDs.
  const twins = deviceMenuItems({adb, list:[{...iphone, model:'iPhone'}, {...iphone, model:'iPhone', serial:'QA0UDID00000000000000B2'}, {...ipad, state:'device'}]}, actions);
  assert.deepEqual(labels(twins).slice(1, 4), ['iPhone · iOS 27.0 · …0001', 'iPhone · iOS 27.0 · …00B2', 'QA iPad · iPadOS 27.0']);
  assert.equal(deviceProblem({state:'locked'}), '기기 잠금을 풀어 주세요');
  assert.equal(deviceName({serial:'S', model:'My_Phone', platform:'ios'}), 'My_Phone', 'an owner\'s name stays as it is');
  // Both kinds of device are listed under their kind.
  const list = [{serial:'emulator-5554', state:'device', model:'Pixel_8'}, {serial:'QA0SERIAL02', state:'unauthorized', model:''}, {serial:'X', state:'offline', model:'Tablet_X1'}];
  const items = deviceMenuItems({adb:{...adb, chosenFailed:true}, list:[...list, iphone]}, actions);
  assert.deepEqual(labels(items).slice(0, 8), ['지정한 adb를 실행할 수 없어 자동으로 찾은 adb를 씁니다', '기록할 기기를 선택해 주세요 · 지금부터 들어오는 로그를 기록합니다', 'Android', 'Pixel 8 · emulator-5554',
    'QA0SERIAL02 · QA0SERIAL02 / 기기 화면에서 USB 디버깅을 허용해 주세요', 'Tablet X1 · X / 오프라인 · 기기를 다시 연결해 주세요', 'iPhone·iPad', 'QA iPhone · iOS 27.0']);
  assert.deepEqual([items[2][3].group, items[6][3].group], [true, true]);
  assert.equal(items[4][1], null, 'a device not ready cannot be chosen');
  assert.equal(items[4][3].disabled, true, 'but its reason is reachable');
  items[3][1](); assert.deepEqual(calls.slice(-1), ['emulator-5554']);
  assert.ok(!labels(deviceMenuItems({adb, list}, actions)).includes('Android'), 'one kind needs no heading');
  assert.match(labels(deviceMenuItems({adb, error:'Device not found; refresh the device list'}, actions))[0], /기기 목록을 새로 고쳐/);
  assert.equal(deviceProblem({state:'no permissions'}), 'USB 권한이 없습니다');
  assert.equal(deviceProblem({state:'recovery'}), '준비되지 않음 (recovery)');
  assert.equal(deviceName({serial:'S', model:''}), 'S');
});

test('the capture bar names each state; breaks name where the device left and came back', () => {
  const base = {device:'Pixel 8', bytes:3 * 1048576, unsaved:true, breaks:[]};
  // Unsaved rows are named; a state leads with what to know or do, so a
  // narrow bar cuts the device name first.
  assert.equal(describeCapture({...base, state:'recording'}).text, '기록 중 · 저장 안 됨 · Pixel 8');
  assert.equal(describeCapture({...base, state:'recording', bytes:0}).text, '기록 중 · Pixel 8');
  const waiting = describeCapture({...base, state:'waiting', lost:at(14, 2, 11)});
  assert.equal(waiting.text, '연결 끊김 · 다시 연결을 기다리는 중 · Pixel 8');
  assert.equal(waiting.tone, 'waiting'); assert.match(waiting.title, /^14:02:11에 연결이 끊겼습니다/);
  // An iPhone that is away loses what it logs meanwhile, and the bar says
  // so; plugged in again it may need trust first.
  const away = describeCapture({...base, device:'QA iPhone', platform:'ios', state:'waiting', lost:at(14, 2, 11)});
  assert.equal(away.text, '연결 끊김 · 끊긴 동안 로그 기록 안 됨 · QA iPhone');
  const iphone = describeCapture({...base, device:'QA iPhone', platform:'ios', state:'waiting', lost:at(14, 2, 11), reason:'Trust this computer on the device, then retry'});
  assert.equal(iphone.text, '다시 연결됨 · 이 컴퓨터를 신뢰해 주세요 · QA iPhone');
  assert.match(iphone.title, /신뢰를 누르고 암호를 입력해 주세요.*준비되면 이어서 기록합니다\. 끊긴 동안의 로그는 기록되지 않습니다\.$/);
  // Offline is no return yet.
  assert.equal(describeCapture({...base, state:'waiting', lost:at(14, 2, 11), reason:'Device is offline; reconnect it and retry'}).text, '연결 끊김 · 기기가 오프라인입니다 · Pixel 8');
  const approve = describeCapture({...base, state:'waiting', lost:at(14, 2, 11), reason:'Allow USB debugging on the device, then retry'});
  assert.equal(approve.text, '다시 연결됨 · USB 디버깅을 허용해 주세요 · Pixel 8');
  // The app retries on its own: the tooltip says what to do, not to retry.
  assert.equal(approve.title, '14:02:11에 연결이 끊겼습니다. 기기 화면에서 USB 디버깅을 허용해 주세요. 준비되면 이어서 기록합니다.');
  assert.equal(waiting.title, '14:02:11에 연결이 끊겼습니다. 기기가 다시 연결되면 이어서 기록합니다.');
  assert.equal(describeCapture({...base, state:'stopped', unsaved:false}).text, '기록 중지됨 · 저장함 · Pixel 8');
  assert.equal(describeCapture({...base, state:'stopped'}).text, '기록 중지됨 · 저장 안 됨 · Pixel 8');
  assert.match(describeCapture({...base, state:'stopped'}).title, /지금까지 기록한 로그\(3\.0 MiB\)를 저장할 수 있습니다/);
  assert.equal(describeCapture({...base, state:'stopped', recovered:true}).text, '남은 기기 로그 · 저장 안 됨');
  assert.equal(describeCapture({...base, state:'stopped', recovered:true, unsaved:false}).text, '남은 기기 로그 · 저장함');
  // A failure's reason is a sentence: the bar keeps it to its title.
  const failed = describeCapture({...base, state:'failed', error:'Recording stopped: not enough disk space'});
  assert.equal(failed.text, '기록 중단 · 저장 안 됨 · Pixel 8');
  assert.match(failed.title, /^디스크 공간이 부족해 기록을 멈췄습니다\. 그때까지 기록한 로그는/);
  assert.equal(describeCapture(null), null);
  const breaks = captureBreaks({breaks:[{row:400, lost:at(15, 0, 0), resumed:at(15, 0, 40), reboot:true}, {row:120, lost:at(14, 2, 11), resumed:at(14, 2, 35), reboot:false}, {row:'x'}]});
  assert.deepEqual(breaks.map(b => b.row), [120, 400]);
  assert.equal(breakLabel(breaks, 119, 120), '14:02:11 연결 끊김 → 14:02:35 다시 연결');
  assert.equal(breakLabel(breaks, 120, 121), '');
  assert.equal(breakLabel(breaks, 399, 400), '15:00:00 연결 끊김 → 15:00:40 다시 연결 · 기기 재부팅');
  // A filter that hides a break's first row moves its mark to the next row
  // shown; breaks between two shown rows read as one.
  assert.equal(breakLabel(breaks, 100, 130), '14:02:11 연결 끊김 → 14:02:35 다시 연결');
  assert.equal(breakLabel(breaks, 0, 500), '14:02:11–15:00:40 연결 끊김 2번 · 기기 재부팅');
  // An iPhone's rows from while it was away were never recorded.
  const lossy = captureBreaks({breaks:[{row:9, lost:at(14, 2, 11), resumed:at(14, 2, 35), reboot:false, gap:true}]});
  assert.equal(breakLabel(lossy, 8, 9), '14:02:11 연결 끊김 → 14:02:35 다시 연결 · 끊긴 동안 로그 기록 안 됨');
  assert.deepEqual(captureBreaks(null), []);
  // An app started with its private values shown marks where its rows
  // begin, by the name its rows carry when that differs.
  const marked = captureBreaks({breaks:[{row:9, lost:at(14, 2, 11), resumed:at(14, 2, 35), reboot:false, gap:true}],
    launches:[{row:30, at:at(14, 3, 0), app:'SampleMaps', process:'SampleMapsCore'}, {row:9, at:at(14, 2, 40), app:'SampleApp', process:'SampleApp'}]});
  assert.deepEqual(marked.map(b => b.row), [9, 9, 30]);
  assert.equal(breakLabel(marked, 29, 30), '14:03:00 SampleMaps(SampleMapsCore) 실행 · 새 프로세스부터 <private> 없이 기록');
  assert.equal(breakLabel(marked, 8, 9), '14:02:11 연결 끊김 → 14:02:35 다시 연결 · 끊긴 동안 로그 기록 안 됨 · 14:02:40 SampleApp 실행 · 새 프로세스부터 <private> 없이 기록');
  assert.equal(breakLabel(marked.filter(b => b.kind === 'launch'), 0, 40), '14:02:40–14:03:00 앱 실행 2번 · 마지막 SampleMaps(SampleMapsCore) · 새 프로세스부터 <private> 없이 기록');
  // Hidden behind a filter, the two kinds read in the order they came.
  const later = captureBreaks({breaks:[{row:50, lost:at(14, 5, 0), resumed:at(14, 5, 9), reboot:false, gap:true}], launches:[{row:10, at:at(14, 0, 0), app:'SampleApp', process:'SampleApp'}]});
  assert.equal(breakLabel(later, 5, 60), '14:00:00 SampleApp 실행 · 새 프로세스부터 <private> 없이 기록 · 14:05:00 연결 끊김 → 14:05:09 다시 연결 · 끊긴 동안 로그 기록 안 됨');
  assert.equal(shortDate(at(9, 54, 32)), '10-24 09:54');
  assert.equal(shortDate(null), '');
  assert.equal(clockTime('x'), '');
  assert.equal(installCommand('MacIntel'), 'brew install --cask android-platform-tools');
  assert.equal(installCommand('Win32'), 'winget install Google.PlatformTools');
});

class Element {
  constructor(document, tag = 'div', id = '') {
    Object.assign(this, {ownerDocument:document, tagName:tag.toUpperCase(), id, listeners:new Map(), isConnected:true, hidden:false, disabled:false, textContent:'', title:'', dataset:{}, children:[], className:'', type:''});
  }
  addEventListener(name, fn) {(this.listeners.get(name) ?? this.listeners.set(name, []).get(name)).push(fn);}
  fire(name, event = {}) {let prevented = false; const full = {target:this, preventDefault() {prevented = true;}, stopPropagation() {}, ...event}; for (const fn of this.listeners.get(name) ?? []) fn(full); return {prevented};}
  focus() {this.ownerDocument.activeElement = this;}
  setAttribute(name, value) {(this.attributes ??= {})[name] = String(value);}
  getAttribute(name) {return this.attributes?.[name] ?? null;}
  get classList() {
    const names = () => new Set(this.className.split(' ').filter(Boolean));
    return {
      contains:name => names().has(name),
      toggle:(name, on) => {const set = names(); if (on ?? !set.has(name)) set.add(name); else set.delete(name); this.className = [...set].join(' '); return set.has(name);},
    };
  }
  append(...nodes) {this.children.push(...nodes);}
  replaceChildren(...nodes) {this.children = nodes;}
  getBoundingClientRect() {return {left:10, bottom:40};}
}
class Dialog extends Element {
  constructor(document, id) {super(document, 'dialog', id); this.open = false;}
  showModal() {this.open = true;}
  close() {if (!this.open) return; this.open = false; this.fire('close');}
}
// A page with the device parts, a scripted native side and log state.
function rig({state = {}, answers = {}, platform = 'MacIntel', unreport = null} = {}) {
  const document = {activeElement:null, body:null};
  const elements = new Map();
  document.getElementById = id => {
    if (!elements.has(id)) elements.set(id, ['captureDialog', 'adbDialog', 'appleDialog'].includes(id) ? new Dialog(document, id) : new Element(document, 'div', id));
    return elements.get(id);
  };
  document.createElement = tag => new Element(document, tag);
  document.body = new Element(document, 'body');
  const ui = Object.fromEntries(['deviceButton', 'emptyDevice', 'captureBar', 'captureState', 'captureText', 'captureHint', 'captureApp', 'captureAppStatus', 'captureStop', 'captureSave', 'captureLeftovers', 'leftoverList', 'rowMenu', 'openFile'].map(id => [id, document.getElementById(id)]));
  const calls = [], listeners = {}, menus = [], notes = [], reports = [];
  const native = {
    core:{invoke:async (command, args) => {
      calls.push([command, args]);
      const answer = answers[command];
      // Tauri rejects with the native error text.
      if (answer instanceof Error) throw answer.message;
      return typeof answer === 'function' ? answer(args) : answer;
    }},
    event:{listen:async (name, fn) => {listeners[name] = fn;}},
  };
  const logs = {state:{datasetId:7, closed:false, capture:null, ...state}, getState() {return this.state;}, start:async () => calls.push(['poll'])};
  const menu = {show:(items, x, y, label) => {ui.rowMenu.hidden = false; menus.push({items, x, y, label});}, close:() => {ui.rowMenu.hidden = true;}};
  const timers = [];
  const window = {navigator:{platform}, setTimeout:(fn, ms) => {timers.push(fn); return timers.length;}, clearTimeout:() => {}};
  const unreported = [];
  const devices = mountDevices({ui, document, window, native, menu, logs, isGone:() => false, report:m => reports.push(m), unreport:unreport ?? (m => unreported.push(m)), notify:m => notes.push(m)});
  return {document, ui, calls, listeners, menus, notes, reports, unreported, logs, devices, dialog:document.getElementById('captureDialog'), $:document.getElementById, timers};
}
const unsaved = {state:'recording', device:'Pixel 8', bytes:4096, unsaved:true, breaks:[]};

test('a capture with unsaved rows asks; others close without a question', async () => {
  let r = rig();
  assert.equal(await r.devices.guard(), 'none');
  r = rig({state:{capture:{...unsaved, unsaved:false}}});
  assert.equal(await r.devices.guard(), 'none');
  assert.deepEqual(r.calls.filter(c => c[0] !== 'capture_leftovers'), [], 'a saved capture is no question');
  // 저장 안 함 marks the file to go with the log.
  r = rig({state:{capture:unsaved}});
  let answer = r.devices.guard(); await settle();
  assert.equal(r.dialog.open, true);
  assert.match(r.$('captureDialogNote').textContent, /^Pixel 8에서 기록한 로그\(4\.0 KiB\)가 아직 저장되지 않았습니다\. 저장\u00a0안\u00a0함을 누르면 지워집니다\.$/);
  r.$('captureDiscard').fire('click');
  assert.equal(await answer, 'asked');
  assert.deepEqual(r.calls.at(-1), ['discard_capture', {datasetId:7, discard:true}]);
  // 취소 keeps everything.
  r = rig({state:{capture:unsaved}});
  answer = r.devices.guard(); await settle(); r.$('captureCancel').fire('click');
  assert.equal(await answer, 'cancel');
  assert.equal(r.calls.some(c => c[0] === 'discard_capture'), false);
  // 저장 saves first; a cancelled save dialog keeps the log.
  r = rig({state:{capture:unsaved}, answers:{save_capture:{cancelled:true}}});
  answer = r.devices.guard(); await settle(); r.$('captureForm').fire('submit');
  assert.equal(await answer, 'cancel');
  assert.equal(r.calls.some(c => c[0] === 'discard_capture'), false);
  r = rig({state:{capture:unsaved}, answers:{save_capture:{bytes:4096}}});
  answer = r.devices.guard(); await settle(); r.$('captureForm').fire('submit');
  assert.equal(await answer, 'asked');
  assert.deepEqual(r.calls.filter(c => c[0].endsWith('_capture')).map(c => c[0]), ['save_capture', 'discard_capture']);
  assert.match(r.notes.at(-1), /기기 로그\(4\.0 KiB\)를 저장했습니다/);
});

test('dropped files and closing the window wait for the answer', async () => {
  const r = rig({state:{capture:unsaved}});
  await settle();
  void r.listeners['drop-held']({payload:{token:3, count:2}}); await settle();
  r.$('captureCancel').fire('click'); await settle();
  assert.deepEqual(r.calls.at(-1), ['finish_drop', {token:3, open:false}]);
  void r.listeners['drop-held']({payload:{token:4, count:1}}); await settle();
  r.$('captureDiscard').fire('click'); await settle();
  assert.deepEqual(r.calls.at(-1), ['finish_drop', {token:4, open:true}]);
  void r.listeners['close-held']({payload:{}}); await settle();
  r.$('captureCancel').fire('click'); await settle();
  assert.deepEqual(r.calls.at(-1), ['answer_close', {close:false}]);
  r.logs.state = {...r.logs.state, capture:{...unsaved, unsaved:false}};
  void r.listeners['close-held']({payload:{}}); await settle();
  assert.deepEqual(r.calls.at(-1), ['answer_close', {close:true}]);
});

test('the device menu fills in as adb answers, and a failed start keeps the shown capture file', async () => {
  const r = rig({state:{capture:{...unsaved, unsaved:false}}, answers:{
    adb_status:{found:true, origin:'studio', version:'37.0.1'},
    list_devices:{devices:[{serial:'emulator-5554', state:'device', model:'Pixel_8'}], androidError:null, iosError:null},
    start_capture:new Error('Allow USB debugging on the device, then retry'),
  }});
  await r.devices.openMenu(r.ui.deviceButton);
  assert.deepEqual(r.menus.map(m => m.items[0][0]), ['기기를 찾는 중…', '기록할 기기를 선택해 주세요 · 지금부터 들어오는 로그를 기록합니다']);
  assert.deepEqual([r.menus[1].x, r.menus[1].y, r.menus[1].label], [10, 44, '기기 로그']);
  r.menus[1].items[1][1](); await settle();
  assert.deepEqual(r.calls.filter(c => ['start_capture', 'discard_capture'].includes(c[0])), [['start_capture', {serial:'emulator-5554'}], ['discard_capture', {datasetId:7, discard:false}]]);
  assert.equal(r.reports.at(-1), 'Allow USB debugging on the device, then retry');
  // Without adb the menu still lists iPhones and leads to getting adb.
  const none = rig({answers:{adb_status:{found:false}, list_devices:{devices:[], androidError:'adb was not found; install Android SDK Platform-Tools', iosError:null}}});
  await none.devices.openMenu(none.ui.emptyDevice);
  const items = none.menus.at(-1).items;
  assert.equal(items[0][0], '연결된 기기가 없습니다');
  items.find(item => item?.[0] === 'Android 기기 로그를 기록하려면 adb를 설치해 주세요…')[1]();
  assert.equal(none.$('adbDialog').open, true);
  assert.equal(none.$('adbCommand').textContent, 'brew install --cask android-platform-tools');
});

test('the capture bar shows the shown capture and its actions', () => {
  const r = rig({state:{capture:unsaved}});
  r.ui.openFile.className = 'button primary';
  r.devices.render();
  assert.equal(r.ui.captureBar.hidden, false);
  assert.equal(r.ui.captureBar.dataset.tone, 'live');
  assert.equal(r.ui.captureText.textContent, '기록 중 · 저장 안 됨 · Pixel 8');
  assert.equal(r.ui.captureStop.hidden, false);
  // Unsaved rows make saving the main action instead of opening a log.
  assert.equal(r.ui.captureSave.classList.contains('primary'), true);
  assert.equal(r.ui.openFile.classList.contains('primary'), false);
  assert.match(r.ui.captureSave.title, /\(⌘S\)$/);
  r.devices.render({datasetId:7, closed:false, capture:{...unsaved, state:'stopped', unsaved:false}});
  assert.equal(r.ui.captureStop.hidden, true);
  assert.equal(r.ui.captureSave.classList.contains('primary'), false);
  assert.equal(r.ui.openFile.classList.contains('primary'), true);
  r.devices.render({datasetId:7, closed:true, capture:unsaved});
  assert.equal(r.ui.captureBar.hidden, true);
  assert.equal(r.ui.openFile.classList.contains('primary'), true);
});

test('saving a capture can be stopped while it copies', async () => {
  let finish;
  const r = rig({state:{capture:unsaved}, answers:{save_capture:() => new Promise((_, reject) => {finish = reject;}), cancel_export:null}});
  const saved = r.devices.save();
  assert.equal(r.ui.captureSave.textContent, '저장 취소');
  r.ui.captureSave.fire('click'); await settle();
  assert.deepEqual(r.calls.filter(c => c[0] === 'cancel_export'), [['cancel_export', undefined]]);
  finish('Save cancelled');
  assert.equal(await saved, false);
  assert.deepEqual(r.notes, ['저장을 취소했습니다.'], 'a stopped save is news, not an error');
  assert.deepEqual(r.reports, []);
  assert.equal(r.ui.captureSave.textContent, '저장');
});

test('files left by an earlier run open, or go after a second press', async () => {
  const left = [{id:5, name:'logcat-Pixel_8-20261024-101500.log', size:2048, modified:at(10, 15, 0)}];
  const r = rig({answers:{capture_leftovers:() => left, delete_leftover:() => [], open_leftover:{}}});
  await settle();
  assert.equal(r.ui.captureLeftovers.hidden, false);
  const [row] = r.ui.leftoverList.children, [open, remove] = row.children;
  assert.equal(open.children[0].textContent, 'logcat-Pixel_8-20261024-101500.log');
  assert.equal(remove.getAttribute('aria-label'), 'logcat-Pixel_8-20261024-101500.log 삭제');
  assert.equal(open.children[1].textContent, '2.0 KiB · 10-24 10:15');
  remove.fire('click'); await settle();
  assert.equal(remove.textContent, '삭제 확인');
  assert.match(remove.getAttribute('aria-label'), /삭제 확인 · 한 번 더 누르면 지웁니다$/);
  assert.equal(r.calls.some(c => c[0] === 'delete_leftover'), false);
  remove.fire('click'); await settle();
  assert.deepEqual(r.calls.find(c => c[0] === 'delete_leftover'), ['delete_leftover', {id:5}]);
  open.fire('click'); await settle();
  assert.deepEqual(r.calls.find(c => c[0] === 'open_leftover'), ['open_leftover', {id:5}]);
});

test('a device waiting on the person is listed again while the menu is open', async () => {
  const ipad = {serial:'QA0UDID0000000000000002', state:'untrusted', model:'QA iPad', product:'iPad0,0', platform:'ios', version:'27.0'};
  let listed = {devices:[ipad], androidError:null, iosError:null};
  const r = rig({answers:{adb_status:{found:true, origin:'studio', version:'37.0.1'}, list_devices:() => listed}});
  const opened = r.devices.openMenu(r.ui.deviceButton);
  await settle();
  assert.equal(r.menus.at(-1).items[1][3].disabled, true);
  // Nothing new: the menu is not drawn again.
  const shown = r.menus.length;
  r.timers.shift()(); await settle();
  assert.equal(r.menus.length, shown);
  // Trusted: drawn again, the iPad can be chosen, and listing stops.
  listed = {devices:[{...ipad, state:'device'}], androidError:null, iosError:null};
  r.timers.shift()(); await settle(); await opened;
  assert.equal(r.menus.length, shown + 1);
  assert.equal(typeof r.menus.at(-1).items[1][1], 'function');
  assert.equal(r.timers.length, 0);
  // A closed menu stops listing.
  listed = {devices:[ipad], androidError:null, iosError:null};
  const again = r.devices.openMenu(r.ui.deviceButton);
  await settle();
  r.ui.rowMenu.hidden = true;
  const calls = r.calls.filter(c => c[0] === 'list_devices').length;
  r.timers.shift()(); await settle(); await again;
  assert.equal(r.calls.filter(c => c[0] === 'list_devices').length, calls);
});

test('without Apple\'s device service the menu leads to a dialog that finds it again', async () => {
  let listed = {devices:[], androidError:null, iosError:'Apple device service not found; install Apple Devices or iTunes'};
  const r = rig({answers:{adb_status:{found:true, origin:'studio', version:'37.0.1'}, list_devices:() => listed}});
  await r.devices.openMenu(r.ui.deviceButton);
  r.menus.at(-1).items.find(item => item?.[0]?.startsWith('iPhone·iPad 로그를 기록하려면'))[1]();
  assert.equal(r.$('appleDialog').open, true);
  assert.equal(r.$('appleUrl').textContent, 'https://apps.microsoft.com/detail/9np83lwlpz9k');
  r.$('appleRetry').fire('click'); await settle();
  assert.equal(r.$('appleDialogError').hidden, false, 'still missing');
  listed = {...listed, iosError:null};
  r.$('appleRetry').fire('click'); await settle();
  assert.equal(r.$('appleDialog').open, false);
  assert.match(r.notes.at(-1), /Apple 기기 서비스를 찾았습니다/);
});

test('the app menu lists the developer builds and says that a running one starts anew', () => {
  const calls = [], actions = {refresh:() => calls.push('refresh'), launch:app => calls.push(app.bundle)};
  assert.deepEqual(appMenuItems({loading:true}, actions), [['앱을 찾는 중…']]);
  const noXcode = appMenuItems({error:'Starting an app needs Xcode; install it from the App Store'}, {...actions, copyXcode:() => calls.push('xcode')});
  assert.deepEqual(noXcode.map(item => item?.[0] ?? null), ['앱을 실행하려면 Xcode가 필요합니다. App Store에서 Xcode를 설치해 주세요.', 'Xcode 설치 주소 복사', null, '새로 고침']);
  noXcode[1][1]();
  const failed = appMenuItems({error:'Could not start the app: The device is locked.'}, actions);
  assert.equal(failed[0][0], '앱을 실행하지 못했습니다: The device is locked.');
  // LogStudio's own reasons read in Korean.
  assert.equal(appMenuItems({error:'Xcode did not start the app within 90 s; check the device and retry'}, actions)[0][0], 'Xcode가 90초 안에 앱을 실행하지 못했습니다. 기기 상태를 확인하고 다시 시도해 주세요.');
  assert.equal(appMenuItems({error:"Could not run Xcode's devicectl; check the Xcode installation"}, actions)[0][0], 'Xcode의 devicectl을 실행하지 못했습니다. Xcode 설치를 확인해 주세요.');
  assert.deepEqual(appMenuItems({list:[]}, actions).map(item => item?.[0] ?? null), ['이 기기에 개발용으로 서명한 앱이 없습니다', null, '새로 고침']);
  const items = appMenuItems({list:[{bundle:'com.example.sample', name:'SampleApp', version:'1.4.0', executable:'SampleApp'}, {bundle:'com.example.maps', name:'SampleMaps', version:'2.0', executable:'SampleMapsCore'}, {bundle:'com.example.bare', name:''}, {name:'no bundle'}]}, actions);
  assert.deepEqual(items.slice(0, 2).map(item => item[0]), ['실행할 앱을 선택해 주세요 · 실행 중이면 종료하고 다시 시작합니다', '기기 잠금을 풀어 두어야 실행됩니다']);
  // An app whose rows carry another name says it.
  assert.deepEqual(items.slice(2, 5).map(item => [item[0], item[2], item[3]]), [
    ['SampleApp', 'SampleApp 1.4.0', {note:'com.example.sample', key:'com.example.sample'}],
    ['SampleMaps', 'SampleMaps 2.0 · 로그의 프로세스 이름: SampleMapsCore', {detail:'SampleMapsCore', note:'com.example.maps', key:'com.example.maps'}],
    ['com.example.bare', 'com.example.bare', {note:'com.example.bare', key:'com.example.bare'}],
  ]);
  assert.equal(items.length, 7, 'an entry without a bundle is left out');
  items[2][1]();
  assert.deepEqual(calls, ['xcode', 'com.example.sample']);
});

test('an iPhone being recorded on a Mac offers to start an app, which runs once at a time', async () => {
  const iphone = {...unsaved, device:'QA iPhone', platform:'ios'};
  let finish;
  const r = rig({state:{capture:iphone}, answers:{
    capture_apps:[{bundle:'com.example.sample', name:'SampleApp', version:'1.4.0'}],
    launch_capture_app:() => new Promise(resolve => {finish = resolve;}),
  }});
  r.devices.render();
  assert.equal(r.ui.captureApp.hidden, false);
  assert.equal(r.ui.captureApp.textContent, '앱 실행…');
  r.ui.captureApp.fire('click'); await settle();
  assert.deepEqual(r.menus.map(m => m.items[0][0]), ['앱을 찾는 중…', '실행할 앱을 선택해 주세요 · 실행 중이면 종료하고 다시 시작합니다']);
  assert.equal(r.menus.at(-1).label, '앱 실행');
  assert.deepEqual(r.calls.find(c => c[0] === 'capture_apps'), ['capture_apps', {datasetId:7}]);
  r.menus.at(-1).items[2][1](); await settle();
  assert.deepEqual(r.calls.find(c => c[0] === 'launch_capture_app'), ['launch_capture_app', {datasetId:7, bundle:'com.example.sample'}]);
  // The button keeps focus while the app starts: it is marked unavailable,
  // not disabled, and screen readers hear what happens.
  assert.equal(r.ui.captureApp.disabled, false);
  assert.equal(r.ui.captureApp.getAttribute('aria-disabled'), 'true');
  assert.equal(r.ui.captureApp.textContent, '앱 실행 중…');
  assert.equal(r.ui.captureAppStatus.textContent, 'SampleApp 앱을 실행하는 중입니다');
  // A press, or a second pick, while one starts is dropped.
  r.ui.captureApp.fire('click'); r.menus.at(-1).items[2][1](); await settle();
  assert.equal(r.calls.filter(c => c[0] === 'launch_capture_app').length, 1);
  assert.equal(r.calls.filter(c => c[0] === 'capture_apps').length, 1);
  // A start that takes long likely waits on a locked device.
  r.timers.at(-1)();
  assert.equal(r.notes.at(-1), 'SampleApp 앱을 실행하는 중입니다. 기기 잠금이 풀려 있는지 확인해 주세요.');
  finish({name:'SampleApp'}); await settle();
  assert.equal(r.ui.captureApp.getAttribute('aria-disabled'), 'false');
  assert.equal(r.ui.captureAppStatus.textContent, '');
  assert.equal(r.notes.at(-1), 'SampleApp 앱을 실행했습니다. 이번 실행의 로그는 <private> 없이 기록합니다.');
  // A failure is reported as the error bar reports others, and goes once
  // the next try starts.
  let answer = new Error('Unlock the device, then retry');
  const failing = rig({state:{capture:iphone}, answers:{capture_apps:[{bundle:'com.example.sample', name:'SampleApp'}], launch_capture_app:() => {if (answer instanceof Error) throw answer.message; return answer;}}});
  await failing.devices.openAppMenu(failing.ui.captureApp);
  failing.menus.at(-1).items[2][1](); await settle();
  assert.deepEqual(failing.reports, ['Unlock the device, then retry']);
  assert.deepEqual(failing.notes, []);
  answer = {name:'SampleApp'};
  failing.menus.at(-1).items[2][1](); await settle();
  assert.deepEqual(failing.unreported, ['', 'Unlock the device, then retry']);
  assert.equal(failing.notes.at(-1), 'SampleApp 앱을 실행했습니다. 이번 실행의 로그는 <private> 없이 기록합니다.');
  // Another capture shown while an app starts has its own button.
  let held;
  const other = rig({state:{capture:iphone}, answers:{capture_apps:[{bundle:'com.example.sample', name:'SampleApp'}], launch_capture_app:() => new Promise(resolve => {held = resolve;})}});
  await other.devices.openAppMenu(other.ui.captureApp);
  other.menus.at(-1).items[2][1](); await settle();
  assert.equal(other.ui.captureApp.getAttribute('aria-disabled'), 'true');
  other.devices.render({datasetId:8, closed:false, capture:iphone});
  assert.deepEqual([other.ui.captureApp.getAttribute('aria-disabled'), other.ui.captureApp.textContent], ['false', '앱 실행…']);
  other.devices.render();
  assert.equal(other.ui.captureApp.textContent, '앱 실행 중…');
  held({name:'SampleApp'}); await settle();
  // An answer for a capture no longer shown is dropped.
  let late;
  const closed = rig({state:{capture:iphone}, answers:{capture_apps:[{bundle:'com.example.sample', name:'SampleApp'}], launch_capture_app:() => new Promise((_, reject) => {late = reject;})}});
  await closed.devices.openAppMenu(closed.ui.captureApp);
  closed.menus.at(-1).items[2][1](); await settle();
  closed.logs.state = {...closed.logs.state, datasetId:8};
  late('Unlock the device, then retry'); await settle();
  assert.deepEqual([closed.reports, closed.notes], [[], []]);
  // Only while an iPhone records, and only on a Mac.
  for (const [capture, platform] of [[unsaved, 'MacIntel'], [{...iphone, state:'waiting'}, 'MacIntel'], [{...iphone, state:'stopped'}, 'MacIntel'], [iphone, 'Win32']]) {
    const other = rig({state:{capture}, platform});
    other.devices.render();
    assert.equal(other.ui.captureApp.hidden, true, `${capture.platform} ${capture.state} ${platform}`);
  }
});

test('a device menu answer that comes after the app menu opened is dropped', async () => {
  let listed;
  const r = rig({state:{capture:{...unsaved, platform:'ios'}}, answers:{
    adb_status:{found:true, origin:'studio', version:'37.0.1'},
    list_devices:() => new Promise(resolve => {listed = resolve;}),
    capture_apps:[{bundle:'com.example.sample', name:'SampleApp'}],
  }});
  const devices = r.devices.openMenu(r.ui.deviceButton); await settle();
  await r.devices.openAppMenu(r.ui.captureApp);
  listed({devices:[], androidError:null, iosError:null}); await devices;
  assert.equal(r.menus.at(-1).label, '앱 실행');
});
