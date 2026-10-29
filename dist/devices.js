// Device captures, of Android devices through adb and of iPhones and iPads
// through Apple's device service: the device menu, the capture bar, how to
// get adb, the question before a capture's unsaved rows go, starting a
// developer build on the iPhone being recorded, and the capture files an
// earlier run left. Native code owns adb, the devices, the apps and the
// files; the page names a device by the serial of the last listing (an
// iPhone's UDID), an app by its bundle in the last app listing and a left
// file by an ID. A capture's rows are a temporary file until the user saves
// them; clearing them records into a new file from then on.
import {formatSize, describeLogError} from './describe.js';
import {textSpan, writeClipboard} from './dom.js';

const ORIGINS = {chosen:'직접 지정', sdk:'ANDROID_HOME', studio:'Android Studio', path:'PATH', homebrew:'Homebrew'};
const pad = n => String(n).padStart(2, '0');
// A local wall-clock time, `14:02:11`.
export function clockTime(ms) {
  if (!Number.isSafeInteger(ms)) return '';
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}
// A local date and minute as logcat writes them, `10-24 09:54`.
export function shortDate(ms) {
  if (!Number.isSafeInteger(ms)) return '';
  const d = new Date(ms);
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
// The adb in use, as the device menu names it.
export function describeAdb(adb) {
  if (!adb?.found) return '';
  return `adb ${adb.version ?? ''} · ${ORIGINS[adb.origin] ?? adb.origin ?? ''}`.trim();
}
// Why a listed device cannot be captured; empty when it can.
export function deviceProblem(device) {
  switch (device?.state) {
    case 'device': return '';
    case 'unauthorized': case 'authorizing': return '기기 화면에서 USB 디버깅을 허용해 주세요';
    case 'offline': case 'connecting': return '오프라인 · 기기를 다시 연결해 주세요';
    case 'no permissions': return 'USB 권한이 없습니다';
    case 'untrusted': return '기기에서 이 컴퓨터를 신뢰해 주세요';
    case 'locked': return '기기 잠금을 풀어 주세요';
    default: return `준비되지 않음 (${String(device?.state ?? '').slice(0, 40)})`;
  }
}
// A device by its model when adb knows it, else by its serial. An iPhone's
// name, which its owner chose, stays as it is.
export function deviceName(device) {
  if (!device?.model) return String(device?.serial ?? '');
  return device.platform === 'ios' ? String(device.model) : String(device.model).replaceAll('_', ' ');
}
// What tells a device apart after its name: an Android device's serial, an
// iPhone's system version (its UDID means nothing to people), with the end
// of its UDID when another iPhone has its name (new ones are all "iPhone").
function deviceDetail(device, alike = false) {
  if (device?.platform !== 'ios') return String(device?.serial ?? '');
  const system = device.version ? `${/^iPad/.test(device.product ?? '') ? 'iPadOS' : 'iOS'} ${device.version}` : '';
  return [system, alike ? `…${String(device.serial ?? '').slice(-4)}` : ''].filter(Boolean).join(' · ');
}
// How an iPhone comes to trust this computer, shown under it in the menu.
const TRUST_STEPS = '기기 화면의 "이 컴퓨터를 신뢰하겠습니까?"에서 신뢰를 누르고 암호를 입력해 주세요';
const TRUST_HINT = `${TRUST_STEPS}. 창이 보이지 않으면 케이블을 다시 연결해 주세요.`;
// Native code's word for a computer without Apple's device service.
export const NO_APPLE_SERVICE = 'Apple device service not found; install Apple Devices or iTunes';
export const APPLE_DEVICES_URL = 'https://apps.microsoft.com/detail/9np83lwlpz9k';
// One device of the menu. One that cannot be picked yet still takes focus
// and says on a second line what it needs, so keyboard and screen reader
// users learn why.
function deviceItem(device, alike, actions) {
  const name = deviceName(device), detail = deviceDetail(device, alike), problem = deviceProblem(device);
  const key = String(device.serial ?? '');
  if (problem) {
    const untrusted = device.state === 'untrusted';
    return [name, null, untrusted ? TRUST_HINT : undefined, {disabled:true, detail, note:untrusted ? TRUST_STEPS : problem, key}];
  }
  const title = `${name}${device.platform === 'ios' ? '' : `(${device.serial})`}의 로그를 지금부터 기록합니다`;
  return [name, () => actions.start(device.serial), title, {detail, key}];
}
// The device menu's items (menu.js): the Android devices adb lists and the
// iPhones Apple's device service lists, under their kind when both are
// there, and what a missing kind needs. Without adb, iPhones are listed.
export function deviceMenuItems({adb, list, error, androidError, iosError, loading}, actions) {
  if (loading) return [['기기를 찾는 중…']];
  const footer = [null, ['새로 고침', actions.refresh]];
  // Offering to pick an adb that was found on its own reads as a step left
  // to do. Without adb, picking is part of getting one (actions.help).
  if (adb?.found && adb.origin === 'chosen') footer.push(['adb 위치 지정…', actions.pick, `지금 쓰는 adb: ${describeAdb(adb)}`]);
  if (error) return [[describeLogError(error)], ...footer];
  const noAdb = !!adb && !adb.found;
  const items = [];
  if (adb?.chosenFailed) items.push(['지정한 adb를 실행할 수 없어 자동으로 찾은 adb를 씁니다']);
  const devices = Array.isArray(list) ? list : [];
  if (devices.length) {
    items.push(['기록할 기기를 선택해 주세요 · 지금부터 들어오는 로그를 기록합니다']);
  } else {
    items.push(['연결된 기기가 없습니다']);
    if (!noAdb && !iosError) items.push(['Android 기기는 USB 디버깅을 켜고, iPhone·iPad는 케이블이나 Wi-Fi로 연결해 주세요']);
    else if (!noAdb) items.push(['USB 디버깅을 켠 기기를 연결하거나 에뮬레이터를 실행해 주세요']);
    else if (!iosError) items.push(['iPhone·iPad를 케이블이나 Wi-Fi로 연결해 주세요']);
  }
  const ios = devices.filter(d => d?.platform === 'ios'), android = devices.filter(d => d?.platform !== 'ios');
  const names = ios.map(deviceName);
  for (const [heading, group] of [['Android', android], ['iPhone·iPad', ios]]) {
    if (!group.length) continue;
    if (android.length && ios.length) items.push([heading, null, undefined, {group:true}]);
    for (const device of group) {
      const alike = group === ios && names.filter(n => n === deviceName(device)).length > 1;
      items.push(deviceItem(device, alike, actions));
    }
  }
  // What a missing kind of device needs, as an item that leads there.
  const missing = [];
  if (noAdb) missing.push(['Android 기기 로그를 기록하려면 adb를 설치해 주세요…', actions.help, 'adb 설치 방법과 adb 위치 지정을 안내합니다']);
  else if (androidError) missing.push([describeLogError(androidError)]);
  if (iosError === NO_APPLE_SERVICE) missing.push(['iPhone·iPad 로그를 기록하려면 Apple Devices 앱을 설치해 주세요…', actions.appleHelp, 'Apple Devices 앱이나 iTunes 설치를 안내합니다']);
  else if (iosError) missing.push([describeLogError(iosError)]);
  if (missing.length) items.push(null, ...missing);
  return items.concat(footer);
}
// Native code's word for a computer without Xcode, and where to get it.
export const NO_XCODE = 'Starting an app needs Xcode; install it from the App Store';
export const XCODE_URL = 'https://apps.apple.com/app/xcode/id497799835';
// The app menu's items (menu.js) for an iPhone or iPad being recorded: its
// developer builds. One started from here runs anew, and its rows keep the
// values they would otherwise give as <private>. An app's rows carry its
// executable's name, which may not be its name.
export function appMenuItems({list, error, loading}, actions) {
  if (loading) return [['앱을 찾는 중…']];
  const footer = [null, ['새로 고침', actions.refresh]];
  if (error === NO_XCODE) return [[describeLogError(error)], ['Xcode 설치 주소 복사', actions.copyXcode, 'App Store의 Xcode 페이지 주소를 복사합니다'], ...footer];
  if (error) return [[describeLogError(error)], ...footer];
  const apps = (Array.isArray(list) ? list : []).filter(app => typeof app?.bundle === 'string' && app.bundle);
  if (!apps.length) return [['이 기기에 개발용으로 서명한 앱이 없습니다'], ...footer];
  const items = [['실행할 앱을 선택해 주세요 · 실행 중이면 종료하고 다시 시작합니다'], ['기기 잠금을 풀어 두어야 실행됩니다']];
  for (const app of apps) {
    const name = String(app.name || app.bundle), version = app.version ? ` ${app.version}` : '';
    const process = typeof app.executable === 'string' && app.executable && app.executable !== name ? app.executable : '';
    const title = `${name}${version}${process ? ` · 로그의 프로세스 이름: ${process}` : ''}`;
    // The bundle, often long, takes a second line so the name stays whole.
    items.push([name, () => actions.launch(app), title, {...(process ? {detail:process} : {}), note:app.bundle, key:app.bundle}]);
  }
  return items.concat(footer);
}
// What a waiting capture needs of a device that is listed again: words for
// the bar, and what to do for its tooltip. The app retries on its own.
// `back`: the device is plugged in again and waits on the person.
const WAITING_FOR = {
  'Allow USB debugging on the device, then retry':{need:'USB 디버깅을 허용해 주세요', hint:'기기 화면에서 USB 디버깅을 허용해 주세요.', back:true},
  'Device is offline; reconnect it and retry':{need:'기기가 오프라인입니다', hint:'케이블이나 기기 연결을 확인해 주세요.'},
  'Trust this computer on the device, then retry':{need:'이 컴퓨터를 신뢰해 주세요', hint:TRUST_HINT, back:true},
  'Unlock the device, then retry':{need:'기기 잠금을 풀어 주세요', hint:'기기 잠금을 풀어 주세요.', back:true},
};
const NOT_READY = {need:'기기가 준비되기를 기다리는 중', hint:'기기가 아직 로그를 보낼 준비가 되지 않았습니다.'};
// The capture bar's words for a capture; `tone` picks its color. Each state
// leads with what the user should know or do, so a narrow bar cuts the
// device name first.
export function describeCapture(capture) {
  if (!capture) return null;
  const size = formatSize(capture.bytes ?? 0), device = capture.device || '기기';
  const kept = capture.bytes > 0 ? (capture.unsaved ? ' · 저장 안 됨' : ' · 저장함') : '';
  switch (capture.state) {
    case 'recording':
      return {
        tone:'live',
        text:`기록 중${kept} · ${device}`,
        title:'기기에서 들어오는 로그를 임시 파일에 기록하고 있습니다. 저장하지 않고 닫으면 지워집니다.',
      };
    case 'waiting': {
      // A device listed again may still need the user, such as approving
      // USB debugging after a reboot.
      const wait = capture.reason ? WAITING_FOR[capture.reason] ?? NOT_READY : null;
      const lost = `${clockTime(capture.lost)}에 연결이 끊겼습니다.`;
      // An iPhone sends only new rows: what it logs while away is lost, and
      // the bar says so while nothing more is needed of the person.
      const ios = capture.platform === 'ios', gap = ios ? ' 끊긴 동안의 로그는 기록되지 않습니다.' : '';
      let text = `연결 끊김 · ${ios ? '끊긴 동안 로그 기록 안 됨' : '다시 연결을 기다리는 중'} · ${device}`;
      if (wait) text = `${wait.back ? '다시 연결됨' : '연결 끊김'} · ${wait.need} · ${device}`;
      return {
        tone:'waiting',
        text,
        title:wait ? `${lost} ${wait.hint} 준비되면 이어서 기록합니다.${gap}` : `${lost} 기기가 다시 연결되면 이어서 기록합니다.${gap}`,
      };
    }
    case 'failed':
      // The reason is a sentence; the error bar shows it whole.
      return {
        tone:'failed',
        text:`기록 중단${kept} · ${device}`,
        title:`${describeLogError(capture.error)} 그때까지 기록한 로그는 그대로 볼 수 있습니다.`,
      };
    default:
      if (capture.recovered) {
        return {tone:'stopped', text:`남은 기기 로그${kept}`, title:`이전 실행이 기록을 마치지 못하고 남긴 로그입니다(${size}).`};
      }
      return {
        tone:'stopped',
        text:`기록 중지됨${kept} · ${device}`,
        title:capture.unsaved ? `기록을 멈췄습니다. 지금까지 기록한 로그(${size})를 저장할 수 있습니다.` : `기록을 멈췄습니다. 기록한 로그(${size})를 저장했습니다.`,
      };
  }
}
// Where a capture was cleared, where it lost its device and found it again,
// and where an app it started with its private values shown begins: the
// first row after each, in row order. A clear marks the first row.
export function captureBreaks(capture) {
  const away = (capture?.breaks ?? []).map(b => ({...b, kind:'away'})), launches = (capture?.launches ?? []).map(l => ({...l, kind:'launch'}));
  const cleared = Number.isSafeInteger(capture?.cleared) ? [{row:1, at:capture.cleared, kind:'clear'}] : [];
  return [...cleared, ...away, ...launches].filter(b => Number.isSafeInteger(b?.row)).sort((a, b) => a.row - b.row);
}
// The label of the breaks after row `after` up to and including `row`. A
// filtered list may hide a break's own first row; the next row shown carries
// it, and several breaks between two shown rows read as one.
export function breakLabel(breaks, after, row) {
  const between = breaks.filter(b => b.row > after && b.row <= row);
  if (!between.length) return '';
  // Each kind reads as one part, in the order its first mark came.
  const parts = [], away = between.filter(b => b.kind === 'away'), started = between.filter(b => b.kind === 'launch');
  const cleared = between.find(b => b.kind === 'clear');
  if (cleared) parts.push({row:cleared.row, text:`${clockTime(cleared.at)} 로그 비움`});
  if (away.length) {
    const first = away[0], last = away[away.length - 1], reboot = away.some(b => b.reboot), gap = away.some(b => b.gap);
    const span = away.length === 1
      ? `${clockTime(first.lost)} 연결 끊김 → ${clockTime(first.resumed)} 다시 연결`
      : `${clockTime(first.lost)}–${clockTime(last.resumed)} 연결 끊김 ${away.length}번`;
    // An iPhone's rows from while it was away were never recorded.
    parts.push({row:first.row, text:`${span}${reboot ? ' · 기기 재부팅' : ''}${gap ? ' · 끊긴 동안 로그 기록 안 됨' : ''}`});
  }
  if (started.length) {
    // The app's rows say its process, which may not be its name.
    const first = started[0], last = started[started.length - 1];
    const name = last.process && last.process !== last.app ? `${last.app}(${last.process})` : last.app || last.process;
    // The mark is where the start began: rows of a copy still running may
    // follow it until Xcode ends that copy.
    const kept = '새 프로세스부터 <private> 없이 기록';
    parts.push({row:first.row, text:started.length === 1 ? `${clockTime(last.at)} ${name} 실행 · ${kept}` : `${clockTime(first.at)}–${clockTime(last.at)} 앱 실행 ${started.length}번 · 마지막 ${name} · ${kept}`});
  }
  return parts.sort((a, b) => a.row - b.row).map(p => p.text).join(' · ');
}
// How to get adb on this computer.
export function installCommand(platform) {
  return /Mac/.test(platform ?? '') ? 'brew install --cask android-platform-tools' : 'winget install Google.PlatformTools';
}
export const PLATFORM_TOOLS_URL = 'https://developer.android.com/tools/releases/platform-tools';
// Native code's word for a save the user stopped.
const SAVE_CANCELLED = 'Save cancelled';
// What the trash button does, and the notice once it has, which stays as
// long as a notice with an action: the rows it names are gone.
const CLEAR_HINT = '지금까지 기록한 로그를 비우고 이어서 기록합니다';
export const clearedNotice = bytes => `기록한 로그(${formatSize(bytes)})를 비웠습니다. 지금부터 들어오는 로그를 기록합니다.`;
const CLEARED_MS = 6000;
// A second press of a double click, after a clear that ended at once, is
// not a second clear.
const CLEAR_REPEAT_MS = 600;

// One modal question: 저장 (focused), 저장 안 함, or 취소 (also Escape and a
// press on the backdrop). Resolves 'save', 'discard' or 'cancel'.
export function mountCaptureDialog({document = globalThis.document} = {}) {
  const $ = id => document.getElementById(id);
  const dialog = $('captureDialog'), note = $('captureDialogNote'), save = $('captureSaveAs');
  let answer = null, opener = null;
  function ask(text) {
    answer?.('cancel');
    note.textContent = text;
    if (!dialog.open) {
      opener = document.activeElement;
      dialog.showModal();
    }
    save.focus({preventScroll:true});
    return new Promise(resolve => {answer = resolve;});
  }
  function close(result) {
    const done = answer;
    answer = null;
    if (dialog.open) {
      dialog.close();
      const target = opener;
      opener = null;
      if (target && target !== document.body && target.isConnected && !target.hidden) target.focus({preventScroll:true});
    }
    done?.(result);
  }
  $('captureForm').addEventListener('submit', event => {
    event.preventDefault();
    close('save');
  });
  $('captureDiscard').addEventListener('click', () => close('discard'));
  $('captureCancel').addEventListener('click', () => close('cancel'));
  dialog.addEventListener('keydown', event => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    close('cancel');
  });
  dialog.addEventListener('cancel', event => {
    event.preventDefault();
    close('cancel');
  });
  dialog.addEventListener('click', event => {
    if (event.target === dialog) close('cancel');
  });
  dialog.addEventListener('close', () => {
    const done = answer;
    answer = null;
    done?.('cancel');
  });
  return {ask, dismiss:() => close('cancel'), get isOpen() {return dialog.open;}};
}

export function mountDevices({ui, document, window, native, menu, logs, isGone, report, unreport = () => {}, notify, keepInput = () => () => {}}) {
  const invoke = (command, args) => native.core.invoke(command, args);
  const fail = error => report(String(error).slice(0, 500));
  const $ = id => document.getElementById(id);
  const dialog = mountCaptureDialog({document});
  const platform = window.navigator.platform ?? '';
  const mod = /Mac|iPhone|iPad/.test(platform) ? '⌘' : 'Ctrl+';
  // Apps start through Xcode, which only a Mac has.
  const xcode = /Mac/.test(platform);
  // An app that has not started after this long likely waits on a locked
  // device, which devicectl gives up on after about 10 s.
  const SLOW_LAUNCH_MS = 2000;
  // Apps starting, by the dataset of the capture they start on: another
  // capture shown meanwhile has its own button. A start goes on under the
  // log that replaces its capture's when the capture is cleared.
  const starting = new Map();
  let adb = null, busy = false, saving = false, clearing = false, clearedAt = -Infinity, launchError = '';

  // The device menu opens at once and fills in when adb and the devices
  // answer.
  let menuAt = null, menuSequence = 0;
  function showMenu(content) {
    if (!menuAt) return;
    const actions = {refresh:() => void openMenu(menuAt.anchor), pick, help:openHelp, appleHelp:openAppleHelp, start};
    menu.show(deviceMenuItems({adb, ...content}, actions), menuAt.x, menuAt.y, '기기 로그');
  }
  const listing = listed => ({list:listed?.devices, androidError:listed?.androidError, iosError:listed?.iosError});
  // A device that waits on the person (USB debugging, trust, unlocking) is
  // listed again while the menu is open, so it can be chosen once ready.
  const waitsOnPerson = listed => (listed?.devices ?? []).some(device => deviceProblem(device));
  const pause = ms => new Promise(resolve => window.setTimeout(resolve, ms));
  async function openMenu(anchor) {
    if (isGone()) return;
    const box = anchor.getBoundingClientRect();
    menuAt = {anchor, x:box.left, y:box.bottom + 4};
    const sequence = ++menuSequence;
    // A newer menu, a closed one or a closed page drops a late answer.
    const stale = () => sequence !== menuSequence || isGone() || ui.rowMenu.hidden;
    showMenu({loading:true});
    let listed;
    try {
      [adb, listed] = await Promise.all([invoke('adb_status'), invoke('list_devices')]);
      if (stale()) return;
      showMenu(listing(listed));
    } catch (error) {
      if (!stale()) showMenu({error:String(error).slice(0, 500)});
      return;
    }
    let shown = JSON.stringify(listed);
    while (waitsOnPerson(listed)) {
      await pause(1500);
      if (stale()) return;
      try {listed = await invoke('list_devices');} catch {return;}
      if (stale()) return;
      const now = JSON.stringify(listed);
      if (now !== shown) showMenu(listing(listed));
      shown = now;
    }
  }
  // The developer builds of the iPhone being recorded, in the same menu: a
  // newer menu of either kind drops a late answer.
  async function openAppMenu(anchor) {
    const datasetId = logs.getState().datasetId;
    if (isGone() || !Number.isSafeInteger(datasetId)) return;
    const box = anchor.getBoundingClientRect(), sequence = ++menuSequence;
    menuAt = null;
    const stale = () => sequence !== menuSequence || isGone() || ui.rowMenu.hidden || logs.getState().datasetId !== datasetId;
    const copyXcode = () => {void writeClipboard(window, XCODE_URL).then(() => notify('Xcode 설치 주소를 복사했습니다. 브라우저에 붙여 넣어 App Store에서 설치해 주세요.'), () => {});};
    const actions = {refresh:() => void openAppMenu(anchor), launch:app => void launch(datasetId, app), copyXcode};
    const show = content => menu.show(appMenuItems(content, actions), box.left, box.bottom + 4, '앱 실행');
    show({loading:true});
    try {
      const list = await invoke('capture_apps', {datasetId});
      if (!stale()) show({list});
    } catch (error) {
      if (!stale()) show({error:String(error).slice(0, 500)});
    }
  }
  // Starts the app anew; Xcode may first mount its disk image on the device.
  // A failure from an earlier try goes once this one starts, and an answer
  // for a capture no longer shown is dropped.
  async function launch(datasetId, app) {
    if (starting.has(datasetId) || isGone()) return;
    const name = String(app.name || app.bundle), job = {datasetId, name};
    const shown = () => !isGone() && logs.getState().datasetId === job.datasetId;
    starting.set(datasetId, job);
    unreport(launchError);
    launchError = '';
    ui.captureAppStatus.textContent = `${name} 앱을 실행하는 중입니다`;
    render();
    const slow = window.setTimeout(() => {if (shown()) notify(`${name} 앱을 실행하는 중입니다. 기기 잠금이 풀려 있는지 확인해 주세요.`);}, SLOW_LAUNCH_MS);
    try {
      const result = await invoke('launch_capture_app', {datasetId, bundle:app.bundle});
      if (shown()) notify(`${result?.name || name} 앱을 실행했습니다. 이번 실행의 로그는 <private> 없이 기록합니다.`);
    } catch (error) {
      if (shown()) {
        launchError = String(error).slice(0, 500);
        fail(error);
      }
    } finally {
      window.clearTimeout(slow);
      starting.delete(job.datasetId);
      if (shown()) ui.captureAppStatus.textContent = '';
      render();
    }
  }
  async function pick() {
    try {
      const result = await invoke('pick_adb');
      if (result?.cancelled) return;
      adb = result;
      notify(`지정한 adb를 씁니다 (${describeAdb(adb)}).`);
      closeHelp();
    } catch (error) {fail(error);}
  }

  // How to get adb, for a computer without it.
  const helpDialog = $('adbDialog');
  function openHelp() {
    $('adbCommand').textContent = installCommand(platform);
    $('adbUrl').textContent = PLATFORM_TOOLS_URL;
    $('adbDialogError').hidden = true;
    if (!helpDialog.open) helpDialog.showModal();
    $('adbRetry').focus({preventScroll:true});
  }
  function closeHelp() {
    if (helpDialog.open) helpDialog.close();
  }
  const copy = text => {void writeClipboard(window, text).then(() => notify('복사했습니다.'), () => {});};
  $('adbCopy').addEventListener('click', () => copy($('adbCommand').textContent));
  $('adbUrlCopy').addEventListener('click', () => copy(PLATFORM_TOOLS_URL));
  $('adbPick').addEventListener('click', () => void pick());
  $('adbClose').addEventListener('click', closeHelp);
  $('adbRetry').addEventListener('click', async () => {
    try {
      adb = await invoke('adb_status');
      if (adb.found) {
        closeHelp();
        notify(`adb를 찾았습니다 (${describeAdb(adb)}). 기기 메뉴에서 기록할 기기를 선택해 주세요.`);
        return;
      }
      $('adbDialogError').textContent = '아직 adb를 찾지 못했습니다. 설치가 끝났는지 확인하거나 adb 위치를 지정해 주세요.';
      $('adbDialogError').hidden = false;
    } catch (error) {fail(error);}
  });
  // How to get Apple's device service, for Windows without it.
  const appleDialog = $('appleDialog');
  function openAppleHelp() {
    $('appleUrl').textContent = APPLE_DEVICES_URL;
    $('appleDialogError').hidden = true;
    if (!appleDialog.open) appleDialog.showModal();
    $('appleRetry').focus({preventScroll:true});
  }
  const closeAppleHelp = () => {if (appleDialog.open) appleDialog.close();};
  $('appleUrlCopy').addEventListener('click', () => copy(APPLE_DEVICES_URL));
  $('appleClose').addEventListener('click', closeAppleHelp);
  $('appleRetry').addEventListener('click', async () => {
    try {
      const listed = await invoke('list_devices');
      if (listed?.iosError !== NO_APPLE_SERVICE) {
        closeAppleHelp();
        notify('Apple 기기 서비스를 찾았습니다. 기기 메뉴에서 iPhone·iPad를 선택해 주세요.');
        return;
      }
      $('appleDialogError').textContent = '아직 Apple 기기 서비스를 찾지 못했습니다. 설치가 끝났는지 확인한 뒤 다시 찾아 주세요.';
      $('appleDialogError').hidden = false;
    } catch (error) {fail(error);}
  });
  appleDialog.addEventListener('click', event => {
    if (event.target === appleDialog) closeAppleHelp();
  });
  helpDialog.addEventListener('click', event => {
    if (event.target === helpDialog) closeHelp();
  });

  // A capture that replaces the shown log asks about its rows first.
  async function start(serial) {
    if (busy || isGone()) return;
    if (await guard() === 'cancel') return;
    const previous = logs.getState().datasetId;
    busy = true;
    render();
    try {
      await invoke('start_capture', {serial});
    } catch (error) {
      fail(error);
      keep(previous);
    } finally {
      busy = false;
      render();
    }
  }
  // The shown capture keeps its file after all: an open that was to
  // replace it did not.
  function keep(datasetId) {
    const state = logs.getState();
    if (!Number.isSafeInteger(datasetId) || state.datasetId !== datasetId || !state.capture) return;
    void invoke('discard_capture', {datasetId, discard:false}).catch(() => {});
  }
  // Before the shown log closes or another replaces it: a capture with rows
  // no save holds asks whether to save them. 'asked' when it asked and the
  // log may go, 'none' when nothing needed asking, 'cancel' to keep the log.
  async function guard() {
    const state = logs.getState(), capture = state.capture, datasetId = state.datasetId;
    if (!capture || state.closed || !Number.isSafeInteger(datasetId)) return 'none';
    // A file with no unsaved rows goes when its log closes (native code).
    if (!capture.unsaved) return 'none';
    try {
      const device = capture.device || '기기', size = formatSize(capture.bytes);
      // No-break spaces keep the button's name on one line.
      const answer = await dialog.ask(`${device}에서 기록한 로그(${size})가 아직 저장되지 않았습니다. 저장\u00a0안\u00a0함을 누르면 지워집니다.`);
      if (answer === 'cancel' || isGone()) return 'cancel';
      if (answer === 'save' && !await save(datasetId)) return 'cancel';
      await invoke('discard_capture', {datasetId, discard:true});
      return 'asked';
    } catch (error) {
      fail(error);
      return 'cancel';
    }
  }
  // Saves a copy of the rows so far; false when the user cancelled. While it
  // copies, 저장 becomes 저장 취소: a capture can be gigabytes.
  async function save(datasetId = logs.getState().datasetId) {
    if (saving || clearing || !logs.getState().capture) return false;
    saving = true;
    render();
    try {
      const result = await invoke('save_capture', {datasetId});
      if (result?.cancelled) return false;
      notify(`기기 로그(${formatSize(result?.bytes ?? 0)})를 저장했습니다.`);
      void logs.start();
      return true;
    } catch (error) {
      if (String(error) === SAVE_CANCELLED) notify(describeLogError(SAVE_CANCELLED));
      else fail(error);
      return false;
    } finally {
      saving = false;
      render();
    }
  }
  // Why the trash button cannot clear now; empty when it can.
  function unclearable(capture = logs.getState().capture) {
    if (saving) return '저장하는 동안에는 비울 수 없습니다';
    if (!(capture?.bytes > 0)) return '아직 비울 로그가 없습니다';
    return '';
  }
  // Discards the rows so far at once, unsaved or not: the capture records
  // into a new file from now on, shown as a new log that keeps the filters
  // and search. Bookmarks and marks went with the rows. The bar's status
  // stays quiet meanwhile: the notice says what happened, and the status
  // turning unsaved again as rows arrive is no news.
  async function clear() {
    const {datasetId, capture} = logs.getState();
    if (clearing || unclearable(capture) || Date.now() - clearedAt < CLEAR_REPEAT_MS || isGone() || !Number.isSafeInteger(datasetId)) return;
    clearing = true;
    render();
    const forget = keepInput(datasetId, clearedNotice(capture.bytes), CLEARED_MS);
    ui.captureState.setAttribute('aria-live', 'off');
    const speak = () => ui.captureState.setAttribute('aria-live', 'polite');
    try {
      const status = await invoke('clear_capture', {datasetId});
      const job = starting.get(datasetId);
      if (job && Number.isSafeInteger(status?.datasetId)) {
        starting.delete(datasetId);
        job.datasetId = status.datasetId;
        starting.set(job.datasetId, job);
      }
      window.setTimeout(speak, CLEARED_MS);
    } catch (error) {
      speak();
      forget();
      fail(error);
    } finally {
      clearing = false;
      clearedAt = Date.now();
      render();
    }
  }
  ui.captureClear.addEventListener('click', () => void clear());
  ui.captureSave.addEventListener('click', () => {
    if (saving) void invoke('cancel_export').catch(() => {});
    else void save();
  });
  ui.captureStop.addEventListener('click', async () => {
    if (clearing) return;
    const datasetId = logs.getState().datasetId;
    ui.captureStop.disabled = true;
    try {
      await invoke('stop_capture', {datasetId});
      void logs.start();
    } catch (error) {
      fail(error);
    } finally {
      render();
    }
  });
  for (const button of [ui.deviceButton, ui.emptyDevice]) button.addEventListener('click', () => void openMenu(button));
  // While an app starts the button keeps focus, so it is only marked
  // unavailable.
  ui.captureApp.addEventListener('click', () => {if (!starting.has(logs.getState().datasetId)) void openAppMenu(ui.captureApp);});

  // The capture bar follows the shown log's capture. Unsaved rows make
  // saving the main action; opening a log, which replaces them, steps back.
  function render(state = logs.getState()) {
    const capture = state.datasetId !== null && !state.closed ? state.capture : null, words = describeCapture(capture);
    const unsaved = !!words && !!capture.unsaved && capture.bytes > 0;
    ui.captureBar.hidden = !words;
    ui.deviceButton.disabled = busy;
    ui.openFile.classList.toggle('primary', !unsaved);
    if (!words) return;
    ui.captureBar.dataset.tone = words.tone;
    // The status region announces changes, the tooltip's words with them
    // (screen readers do not read a title); the same words are not news.
    if (ui.captureText.textContent !== words.text) {
      ui.captureText.textContent = words.text;
      ui.captureHint.textContent = words.title;
    }
    ui.captureState.title = words.title;
    const live = capture.state === 'recording' || capture.state === 'waiting';
    // The bar's buttons keep focus while they cannot act, so they are only
    // marked unavailable, and the trash button says why.
    ui.captureClear.hidden = !live;
    ui.captureClear.disabled = busy;
    const why = unclearable(capture);
    ui.captureClear.setAttribute('aria-disabled', clearing || why ? 'true' : 'false');
    // Rows no save holds cannot come back.
    const hint = why || (unsaved ? `${CLEAR_HINT}. 저장하지 않은 로그는 되살릴 수 없습니다` : CLEAR_HINT);
    if (ui.captureClear.title !== hint) {
      ui.captureClear.title = hint;
      ui.captureClearHint.textContent = hint;
    }
    ui.captureStop.hidden = !live;
    ui.captureStop.disabled = busy;
    ui.captureStop.setAttribute('aria-disabled', clearing ? 'true' : 'false');
    ui.captureSave.setAttribute('aria-disabled', clearing ? 'true' : 'false');
    // An app starts on an iPhone or iPad only while it is being recorded.
    ui.captureApp.hidden = !xcode || capture.platform !== 'ios' || capture.state !== 'recording';
    ui.captureApp.disabled = busy;
    const launching = starting.get(state.datasetId)?.name ?? '';
    ui.captureApp.setAttribute('aria-disabled', launching ? 'true' : 'false');
    ui.captureApp.textContent = launching ? '앱 실행 중…' : '앱 실행…';
    ui.captureApp.title = launching ? `${launching} 앱을 실행하는 중입니다` : '개발용 서명 앱을 다시 실행해 그 앱의 로그를 <private> 없이 기록합니다';
    ui.captureSave.classList.toggle('primary', unsaved && !saving);
    ui.captureSave.textContent = saving ? '저장 취소' : '저장';
    if (saving) ui.captureSave.title = '저장을 멈춥니다. 저장하던 파일은 남기지 않습니다';
    else if (capture.unsaved) ui.captureSave.title = `지금까지 기록한 로그를 원본 그대로 파일로 저장합니다 (${mod}S)`;
    else ui.captureSave.title = `기록한 로그를 이미 저장했습니다 · 다시 저장할 수 있습니다 (${mod}S)`;
  }

  // Files an earlier run left: shown with the empty state, each opened to
  // save it or deleted after a second press.
  function leftoverRow(item) {
    const row = document.createElement('li'), open = document.createElement('button'), remove = document.createElement('button');
    open.type = 'button';
    open.className = 'recent-open';
    open.title = '열어서 확인하고 저장할 수 있습니다';
    const name = document.createElement('strong');
    name.textContent = item.name;
    open.append(name, textSpan(document, [formatSize(item.size ?? 0), shortDate(item.modified)].filter(Boolean).join(' · ')));
    open.addEventListener('click', async () => {
      try {
        await invoke('open_leftover', {id:item.id});
      } catch (error) {
        fail(error);
      }
      void loadLeftovers();
    });
    // Each 삭제 names its file for screen readers, as the recent list does.
    function arm(on) {
      remove.textContent = on ? '삭제 확인' : '삭제';
      remove.title = on ? '4초 안에 한 번 더 누르면 지웁니다' : '이 로그 파일을 지웁니다';
      remove.setAttribute('aria-label', on ? `${item.name} 삭제 확인 · 한 번 더 누르면 지웁니다` : `${item.name} 삭제`);
      if (on) remove.dataset.armed = 'true';
      else delete remove.dataset.armed;
    }
    remove.type = 'button';
    remove.className = 'leftover-remove';
    arm(false);
    let armed = null;
    remove.addEventListener('click', async () => {
      if (!armed) {
        arm(true);
        armed = window.setTimeout(() => {
          armed = null;
          arm(false);
        }, 4000);
        return;
      }
      window.clearTimeout(armed);
      armed = null;
      try {
        await invoke('delete_leftover', {id:item.id});
      } catch (error) {
        fail(error);
      }
      void loadLeftovers();
    });
    row.append(open, remove);
    return row;
  }
  async function loadLeftovers() {
    let list = [];
    try {list = await invoke('capture_leftovers');} catch {}
    if (isGone()) return;
    list = Array.isArray(list) ? list.filter(item => Number.isSafeInteger(item?.id) && typeof item.name === 'string').slice(0, 20) : [];
    ui.captureLeftovers.hidden = !list.length;
    ui.leftoverList.replaceChildren(...list.map(leftoverRow));
  }
  void loadLeftovers();

  // Native code holds dropped files and closing the window while a capture's
  // log is shown, until the page has asked.
  void native.event.listen('drop-held', async ({payload}) => {
    if (!Number.isSafeInteger(payload?.token)) return;
    const answer = isGone() ? 'cancel' : await guard();
    const datasetId = logs.getState().datasetId;
    try {
      await invoke('finish_drop', {token:payload.token, open:answer !== 'cancel'});
    } catch (error) {
      fail(error);
      keep(datasetId);
    }
  }).catch(() => {});
  void native.event.listen('close-held', async () => {
    const answer = isGone() ? 'none' : await guard();
    try {await invoke('answer_close', {close:answer !== 'cancel'});} catch {}
  }).catch(() => {});
  void native.event.listen('capture-changed', ({payload}) => {
    if (!isGone() && payload?.datasetId === logs.getState().datasetId) void logs.start();
  }).catch(() => {});

  return {render, guard, keep, save, clear, loadLeftovers, openMenu, openAppMenu, isAsking:() => dialog.isOpen || helpDialog.open || appleDialog.open};
}
