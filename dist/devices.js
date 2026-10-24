// Android device captures: the device menu, the capture bar, how to get adb,
// the question before a capture's unsaved rows go, and the capture files an
// earlier run left. Native code owns adb, the devices and the files; the
// page names a device by the serial of the last listing and a left file by
// an ID. A capture's rows are a temporary file until the user saves them.
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
    default: return `준비되지 않음 (${String(device?.state ?? '').slice(0, 40)})`;
  }
}
// A device by its model when adb knows it, else by its serial.
export function deviceName(device) {
  return device?.model ? String(device.model).replaceAll('_', ' ') : String(device?.serial ?? '');
}
// The device menu's items (menu.js): what adb lists, or why it lists none.
export function deviceMenuItems({adb, list, error, loading}, actions) {
  if (loading) return [['기기를 찾는 중…']];
  if (adb && !adb.found) {
    return [
      ['adb를 찾지 못했습니다'],
      ['Android SDK Platform-Tools를 설치하면 기기 로그를 볼 수 있습니다'],
      null,
      ['설치 방법 보기…', actions.help],
      ['다시 찾기', actions.refresh],
      ['adb 위치 지정…', actions.pick],
    ];
  }
  const footer = [
    null,
    ['새로 고침', actions.refresh],
    ['adb 위치 지정…', actions.pick, adb?.found ? `지금 쓰는 adb: ${describeAdb(adb)}` : 'adb 실행 파일을 직접 선택합니다'],
  ];
  if (error) return [[describeLogError(error)], ...footer];
  const items = [];
  if (adb?.chosenFailed) items.push(['지정한 adb를 실행할 수 없어 자동으로 찾은 adb를 씁니다']);
  const devices = Array.isArray(list) ? list : [];
  if (devices.length) items.push(['기록할 기기를 선택해 주세요 · 지금부터 들어오는 로그를 기록합니다']);
  else items.push(['연결된 기기가 없습니다'], ['USB 디버깅을 켠 기기를 연결하거나 에뮬레이터를 실행해 주세요']);
  for (const device of devices) {
    const problem = deviceProblem(device), name = deviceName(device);
    if (problem) {
      // A device that cannot be picked still takes focus, so keyboard and
      // screen reader users learn why.
      items.push([`${name} · ${problem}`, null, undefined, {disabled:true}]);
    } else {
      items.push([`${name} · ${device.serial}`, () => actions.start(device.serial), `${name}(${device.serial})의 로그를 지금부터 기록합니다`]);
    }
  }
  return items.concat(footer);
}
// What a waiting capture needs of a device that is listed again: words for
// the bar, and what to do for its tooltip. The app retries on its own.
const WAITING_FOR = {
  'Allow USB debugging on the device, then retry':{need:'USB 디버깅을 허용해 주세요', hint:'기기 화면에서 USB 디버깅을 허용해 주세요.'},
  'Device is offline; reconnect it and retry':{need:'기기가 오프라인입니다', hint:'케이블이나 기기 연결을 확인해 주세요.'},
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
        text:`기록 중 · ${device}${kept}`,
        title:'기기에서 들어오는 로그를 임시 파일에 기록하고 있습니다. 저장하지 않고 닫으면 지워집니다.',
      };
    case 'waiting': {
      // A device listed again may still need the user, such as approving
      // USB debugging after a reboot.
      const wait = capture.reason ? WAITING_FOR[capture.reason] ?? NOT_READY : null;
      const lost = `${clockTime(capture.lost)}에 연결이 끊겼습니다.`;
      return {
        tone:'waiting',
        text:`연결 끊김 · ${wait ? wait.need : '다시 연결을 기다리는 중'} · ${device}`,
        title:wait ? `${lost} ${wait.hint} 준비되면 이어서 기록합니다.` : `${lost} 기기가 다시 연결되면 이어서 기록합니다.`,
      };
    }
    case 'failed':
      // The reason is a sentence; the error bar shows it whole.
      return {
        tone:'failed',
        text:`기록 중단 · ${device}${kept}`,
        title:`${describeLogError(capture.error)} 그때까지 기록한 로그는 그대로 볼 수 있습니다.`,
      };
    default:
      if (capture.recovered) {
        return {tone:'stopped', text:`남은 기기 로그${kept}`, title:`이전 실행이 기록을 마치지 못하고 남긴 로그입니다(${size}).`};
      }
      return {
        tone:'stopped',
        text:`기록 중지됨 · ${device}${kept}`,
        title:capture.unsaved ? `기록을 멈췄습니다. 지금까지 기록한 로그(${size})를 저장할 수 있습니다.` : `기록을 멈췄습니다. 기록한 로그(${size})를 저장했습니다.`,
      };
  }
}
// Where a capture lost its device and found it again: the first row after
// each break, in row order.
export function captureBreaks(capture) {
  return (capture?.breaks ?? []).filter(b => Number.isSafeInteger(b?.row)).sort((a, b) => a.row - b.row);
}
// The label of the breaks after row `after` up to and including `row`. A
// filtered list may hide a break's own first row; the next row shown carries
// it, and several breaks between two shown rows read as one.
export function breakLabel(breaks, after, row) {
  const between = breaks.filter(b => b.row > after && b.row <= row);
  if (!between.length) return '';
  const first = between[0], last = between[between.length - 1], reboot = between.some(b => b.reboot);
  const span = between.length === 1
    ? `${clockTime(first.lost)} 연결 끊김 → ${clockTime(first.resumed)} 다시 연결`
    : `${clockTime(first.lost)}–${clockTime(last.resumed)} 연결 끊김 ${between.length}번`;
  return `${span}${reboot ? ' · 기기 재부팅' : ''}`;
}
// How to get adb on this computer.
export function installCommand(platform) {
  return /Mac/.test(platform ?? '') ? 'brew install --cask android-platform-tools' : 'winget install Google.PlatformTools';
}
export const PLATFORM_TOOLS_URL = 'https://developer.android.com/tools/releases/platform-tools';
// Native code's word for a save the user stopped.
const SAVE_CANCELLED = 'Save cancelled';

// One modal question: 저장… (focused), 저장 안 함, or 취소 (also Escape and a
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

export function mountDevices({ui, document, window, native, menu, logs, isGone, report, notify}) {
  const invoke = (command, args) => native.core.invoke(command, args);
  const fail = error => report(String(error).slice(0, 500));
  const $ = id => document.getElementById(id);
  const dialog = mountCaptureDialog({document});
  const platform = window.navigator.platform ?? '';
  const mod = /Mac|iPhone|iPad/.test(platform) ? '⌘' : 'Ctrl+';
  let adb = null, busy = false, saving = false;

  // The device menu opens at once and fills in when adb answers.
  let menuAt = null, menuSequence = 0;
  function showMenu(content) {
    if (!menuAt) return;
    const actions = {refresh:() => void openMenu(menuAt.anchor), pick, help:openHelp, start};
    menu.show(deviceMenuItems({adb, ...content}, actions), menuAt.x, menuAt.y, 'Android 기기');
  }
  async function openMenu(anchor) {
    if (isGone()) return;
    const box = anchor.getBoundingClientRect();
    menuAt = {anchor, x:box.left, y:box.bottom + 4};
    const sequence = ++menuSequence;
    // A newer menu, a closed one or a closed page drops a late answer.
    const stale = () => sequence !== menuSequence || isGone() || ui.rowMenu.hidden;
    showMenu({loading:true});
    try {
      adb = await invoke('adb_status');
      if (stale()) return;
      if (!adb.found) {
        showMenu({});
        return;
      }
      const listed = await invoke('adb_devices');
      if (!stale()) showMenu({list:listed?.devices});
    } catch (error) {
      if (!stale()) showMenu({error:String(error).slice(0, 500)});
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
  // copies, 저장… becomes 저장 취소: a capture can be gigabytes.
  async function save(datasetId = logs.getState().datasetId) {
    if (saving || !logs.getState().capture) return false;
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
  ui.captureSave.addEventListener('click', () => {
    if (saving) void invoke('cancel_export').catch(() => {});
    else void save();
  });
  ui.captureStop.addEventListener('click', async () => {
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
    // The status region announces changes; the same words are not news.
    if (ui.captureText.textContent !== words.text) ui.captureText.textContent = words.text;
    ui.captureState.title = words.title;
    const live = capture.state === 'recording' || capture.state === 'waiting';
    ui.captureStop.hidden = !live;
    ui.captureStop.disabled = busy;
    ui.captureSave.classList.toggle('primary', unsaved && !saving);
    ui.captureSave.textContent = saving ? '저장 취소' : '저장…';
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

  return {render, guard, keep, save, loadLeftovers, openMenu, isAsking:() => dialog.isOpen || helpDialog.open};
}
