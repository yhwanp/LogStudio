// User-facing text for progress, native errors and active filters.
import {LEVELS, tagList, idList, isActiveCondition, MESSAGE_PATTERN_ERROR} from './filter-model.js';
import {BUSY} from './controller.js';

import {levelNames} from './columns.js';
export const LEVEL_NAMES = levelNames('text');
export const formatNumber = n => Number(n || 0).toLocaleString('ko-KR');
export const formatSize = n => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KiB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MiB` : `${(n / 1073741824).toFixed(2)} GiB`;
// Search exactness belongs to the current query, independently of optional
// whole-file statistics. Navigation is limited to currently found results.
export function describeLargeProgress(state, {omitDuplicateFileCount=false, compactSearch=false}={}) {
  if (state.closed || state.datasetId === null || state.status === 'empty') {
    return {exact:false, count:'0건', query:state.closed?'파일 닫힘':'', range:'', status:state.closed?'파일을 닫았습니다.':'파일을 열거나 로그를 붙여넣으세요.'};
  }
  const inputPending = !!state.inputPending && !state.closed;
  const exact = !inputPending && !!state.queryComplete && !!state.textComplete && state.queryStatus === 'complete' && !state.closed && !state.error && !['error', 'cancelled', 'closed'].includes(state.status);
  const count = compactSearch ? inputPending ? '—' : `${formatNumber(state.count)}건` : `${inputPending ? '이전 검색' : exact ? '전체' : '현재까지'} ${formatNumber(state.count)}건`;
  const stopped = state.queryStatus === 'cancelled' || state.status === 'cancelled';
  const failed = state.queryStatus === 'error' || state.status === 'error' || !!state.error;
  // A followed file's query has read every row so far and waits for more.
  const waiting = !!state.following && !!state.caughtUp && state.queryStatus === 'running' && !failed;
  const query = inputPending ? (state.inputInvalid ? '입력한 조건 미적용' : state.inputPaused ? compactSearch ? '검색 취소됨 · 부분 결과' : '검색 취소됨 · 새 조건 미적용 · Enter로 검색' : compactSearch ? '검색 중' : '새 조건 적용 대기') : exact ? '검색 완료' : stopped ? '검색 취소됨 · 부분 결과' : failed ? '검색 미완료 · 부분 결과' : waiting ? '따라가는 중' : compactSearch ? '검색 중' : '검색 중 · 부분 결과';
  // With results, the continuous viewport reports the visible range itself.
  const range = inputPending ? (compactSearch ? '' : '새 조건 적용 대기') : state.count ? '' : exact ? '검색 결과가 없습니다' : stopped ? '검색이 취소되었습니다' : failed ? '검색을 완료하지 못했습니다' : waiting ? '아직 결과가 없습니다 · 따라가는 중' : '검색 중…';
  const progress = state.size > 0 ? ` ${Math.min(99, Math.max(0, Math.floor(state.indexedBytes / state.size * 100)))}%` : '';
  // Only the upper unfiltered, exact log total represents every physical row.
  // Equal partial/filtered counts do not make the file-row label redundant.
  const duplicateFileCount = omitDuplicateFileCount && exact && state.fieldFiltering === false && Number.isSafeInteger(state.count) && state.count >= 0 && state.count === state.total;
  const source = state.textComplete ? duplicateFileCount ? '' : `파일 ${formatNumber(state.total)}행` : state.status === 'error' ? '파일 읽기 실패' : state.status === 'cancelled' ? '파일 읽기 취소됨' : `파일 읽는 중${progress}`;
  // Prefix recognition can own a partial summary after the original metadata
  // pass has completed. A stopped prefix query has no remaining summary worker.
  const summaryQueryStopped = state.analysis?.status === 'complete' && ['cancelled', 'error'].includes(state.queryStatus);
  const analysis = state.following && state.textComplete ? '따라가는 중' : state.summaryComplete ? '' : summaryQueryStopped ? state.queryStatus === 'cancelled' ? '레벨 집계 취소됨' : '레벨 집계 미완료' : state.textComplete ? '레벨 집계 중' : '';
  // Rows appended after opening wait until following reads them.
  const appended = !state.following && state.grown && state.textComplete ? '새 줄 있음' : '';
  return {exact, count, query, range, status:[source, analysis, appended].filter(Boolean).join(' · ')};
}

// A row of a combined log names its file and its row within that file.
export function rowSource(entry, files) {
  const file = Number.isSafeInteger(entry?.file) && Number.isSafeInteger(entry?.fileLine) ? files?.[entry.file] : null;
  return file ? {index:entry.file, name:file.name, line:entry.fileLine} : null;
}

// Translate known native errors at the presentation boundary. Controller
// errors stay intact for retry decisions and bounded diagnostic detail.
// A DLT log's messages are payloads, and its errors call them so.
export function describeLogError(value, format='text') {
  const text = String(value ?? '').slice(0, 500);
  const messages = {
    'Opening cancelled':'파일 읽기가 취소되었습니다.',
    'No log is open':'로그 파일을 열어 주세요.',
    'Source closed':'파일이 닫혔습니다. 다시 열어 주세요.',
    'Selected file was not found; select it again':'선택한 파일을 찾을 수 없습니다. 파일 위치를 확인하고 다시 선택해 주세요.',
    'Read permission denied for selected file':'선택한 파일의 읽기 권한이 없습니다. 권한을 확인해 주세요.',
    'Selected file is busy; retry':'다른 작업에서 파일을 사용 중입니다. 잠시 후 다시 열어 주세요.',
    'Could not open selected file; check availability and retry':'파일에 접근할 수 없습니다. 파일 위치와 상태를 확인하고 다시 열어 주세요.',
    'Source changed; reopen the file':'파일 내용이 변경되었습니다. 다시 열어 주세요.',
    'Source truncated':'파일 크기가 줄어들었습니다. 다시 열어 주세요.',
    'File changed or exceeds 8 GiB':'파일이 변경되었거나 8 GiB를 초과합니다. 파일을 확인하고 다시 열어 주세요.',
    'NUL is not supported; select a UTF-8 text log':'텍스트 로그나 DLT 파일을 선택해 주세요. 그 밖의 바이너리 파일은 지원하지 않습니다.',
    'UTF-16 text is not supported; save the log as UTF-8':'UTF-16으로 저장된 텍스트 파일입니다(Windows PowerShell의 > 로 저장한 로그 등). UTF-8로 변환한 뒤 열어 주세요.',
    'Compressed archives are not supported; extract the log first':'압축 파일(zip·gz 등)은 열 수 없습니다. 압축을 푼 로그 파일을 열어 주세요.',
    'DLT files and text logs cannot be opened together':'DLT 파일과 텍스트 로그는 함께 열 수 없습니다. 같은 형식의 파일끼리 열어 주세요.',
    'A selected DLT file contains no DLT messages':'선택한 DLT 파일에서 DLT 메시지를 찾지 못했습니다.',
    'DLT version 2 files are not supported':'DLT 버전 2 파일은 지원하지 않습니다.',
    'DLT data without storage headers is not supported':'저장 헤더가 없는 DLT 데이터는 열 수 없습니다. 메시지마다 저장 헤더가 있는 .dlt 파일을 열어 주세요.',
    'Could not read the DLT file':'DLT 파일을 읽지 못했습니다. 다시 열어 주세요.',
    'DLT files cannot be followed':'DLT 파일은 따라가기를 지원하지 않습니다.',
    'Invalid export format':'이 로그는 선택한 형식으로 내보낼 수 없습니다.',
    'A physical line exceeds 1 MiB':'한 줄이 1 MiB를 초과해 파일을 열 수 없습니다.',
    'Pasted text exceeds 25 MiB':'붙여넣기는 최대 25 MiB입니다. 큰 파일은 파일 선택으로 열어 주세요.',
    'Select a regular file; symbolic links are not supported':'로그 파일을 직접 선택해 주세요. 심볼릭 링크는 지원하지 않습니다.',
    'Select one log file':'로그 파일을 하나만 선택해 주세요.',
    'Select at least one log file':'로그 파일을 하나 이상 선택해 주세요.',
    'Select 1 to 64 log files':'로그 파일은 한 번에 64개까지 열 수 있습니다.',
    'The same file was selected more than once':'같은 파일이 두 번 이상 선택되었습니다. 파일마다 한 번씩만 선택해 주세요.',
    'Paste one log file at a time':'파일 붙여넣기는 한 번에 하나만 할 수 있습니다. 여러 파일은 로그 파일 열기나 끌어다 놓기로 열어 주세요.',
    'Paste grant expired; paste again':'붙여넣은 뒤 시간이 지나 열 수 없습니다. 다시 붙여넣어 주세요.',
    'Paste grant is unavailable':'붙여넣은 파일을 열 수 없습니다. 다시 붙여넣어 주세요.',
    'Paste requires the main window':'LogStudio 창을 선택한 뒤 다시 붙여넣어 주세요.',
    'Select a local file':'로그 파일을 선택해 주세요.',
    'File dialog failed':'파일 선택 창을 열지 못했습니다. 다시 시도해 주세요.',
    'Request failed':'작업을 완료하지 못했습니다. 다시 시도해 주세요.',
    'Paste failed':'붙여넣기에 실패했습니다. 다시 붙여넣거나 파일을 열어 주세요.',
    'File paste failed':'붙여넣기에 실패했습니다. 다시 붙여넣거나 파일을 열어 주세요.',
    'Could not read the copied file; use Open File':'복사한 파일을 읽지 못했습니다. 로그 파일 열기로 열어 주세요.',
    'Invalid copied file':'복사한 파일의 위치를 알 수 없습니다. 로그 파일 열기로 열어 주세요.',
    'Invalid copied file path':'복사한 파일의 위치를 알 수 없습니다. 로그 파일 열기로 열어 주세요.',
    'Copied file has no local path':'복사한 파일의 위치를 알 수 없습니다. 로그 파일 열기로 열어 주세요.',
    'Paste a local file':'이 컴퓨터에 있는 파일만 붙여넣을 수 있습니다.',
    'Paste a local file URL without a remote host':'이 컴퓨터에 있는 파일만 붙여넣을 수 있습니다.',
    'Cancel failed':'작업을 취소하지 못했습니다. 다시 시도해 주세요.',
    'Wait until the results are complete':'결과 집계가 끝난 뒤 내보낼 수 있습니다.',
    'Choose a file other than the open log':'열려 있는 로그 파일에는 내보낼 수 없습니다. 다른 파일을 선택해 주세요.',
    'An export is already running':'이미 내보내는 중입니다.',
    'Write permission denied for the export file':'선택한 위치에 쓸 권한이 없습니다. 다른 위치를 선택해 주세요.',
    'Not enough disk space for the export':'디스크 공간이 부족해 내보내지 못했습니다.',
    'Export failed':'내보내기에 실패했습니다. 다시 시도해 주세요.',
    'Stale query':'결과가 바뀌었습니다. 다시 시도해 주세요.',
    'Stale query ID':'결과가 바뀌었습니다. 다시 시도해 주세요.',
    'Search failed; retry':'검색하지 못했습니다. 다시 시도해 주세요.',
    'Could not read the log; reopen the file':'로그를 읽지 못했습니다. 파일을 다시 열어 주세요.',
    'Nothing to reopen; select the file again':'다시 열 파일이 없습니다. 파일을 다시 선택해 주세요.',
    'Physical row is not indexed yet; retry':'이 행을 아직 읽지 못했습니다. 잠시 후 다시 시도해 주세요.',
    'Physical row is not indexed':'이 행을 아직 읽지 못했습니다. 잠시 후 다시 시도해 주세요.',
    'Filter budget exceeded':'필터 조건이 너무 많거나 깁니다. 조건을 줄여 주세요.',
    'Invalid filter':'필터 조건이 올바르지 않습니다. 조건을 확인해 주세요.',
    'PID and TID must contain ASCII digits':'PID와 TID는 숫자로 입력해 주세요.',
    'Supported options: FILE, --large-log FILE':'실행 인자를 확인해 주세요. 로그 파일 경로나 --large-log 파일 경로만 받습니다.',
    '--large-log requires a file':'--large-log 뒤에 로그 파일 경로를 적어 주세요.',
    'Unsupported command-line arguments':'실행 인자를 확인해 주세요. 로그 파일 경로나 --large-log 파일 경로만 받습니다.',
    'Stale source':'파일이 바뀌었습니다. 다시 시도해 주세요.',
    'Invalid time filter':'시간 형식을 확인해 주세요.',
    'Recent file is no longer listed':'최근 파일 목록이 바뀌었습니다. 다시 선택해 주세요.',
    'Enter a filter name':'이름을 입력해 주세요.',
    'Filter names are limited to 60 characters':'이름은 60자까지 쓸 수 있습니다.',
    'A saved filter has this name':'같은 이름의 저장된 필터가 있습니다. 다른 이름을 입력해 주세요.',
    'Up to 100 filters can be saved':'필터는 100개까지 저장할 수 있습니다. 쓰지 않는 필터를 삭제해 주세요.',
    'Saved filter not found':'저장된 필터를 찾지 못했습니다. 다시 시도해 주세요.',
    'Saved filter is too large':'필터가 너무 커서 저장하지 못했습니다.',
    'Invalid saved filter':'필터를 저장하지 못했습니다. 다시 시도해 주세요.',
    'Could not save filters':'필터를 저장하지 못했습니다. 앱 데이터 폴더에 쓸 수 있는지 확인해 주세요.',
    'Only one opened log file can be followed':'따라가기는 로그 파일을 하나만 열었을 때 쓸 수 있습니다. 붙여넣은 로그나 여러 파일에는 쓸 수 없습니다.',
    'Wait until the file is read':'파일을 다 읽은 뒤에 따라가기를 켤 수 있습니다.',
    'Device not found; connect it and retry':'기기를 찾지 못했습니다. 기기를 연결한 뒤 다시 시도해 주세요.',
    'Allow USB debugging on the device, then retry':'기기 화면에서 USB 디버깅을 허용한 뒤 다시 시도해 주세요.',
    'Device is offline; reconnect it and retry':'기기가 오프라인입니다. 케이블이나 Wi-Fi 연결을 확인한 뒤 다시 시도해 주세요.',
    'Device is not ready for logs; check its connection':'기기가 로그를 보낼 준비가 되지 않았습니다. 연결 상태를 확인해 주세요.',
    "Two adb versions are replacing each other's server; use one adb for all tools":'버전이 다른 adb가 함께 실행되어 adb 서버가 계속 다시 시작됩니다. Android Studio 등 모든 도구가 같은 adb를 쓰도록 맞춘 뒤 다시 기록해 주세요.',
    'Device not found; refresh the device list':'기기를 찾지 못했습니다. 기기 목록을 새로 고쳐 주세요.',
    'Trust this computer on the device, then retry':'기기에서 이 컴퓨터를 신뢰한 뒤 다시 시도해 주세요. 신뢰를 묻는 창이 보이지 않으면 케이블을 다시 연결해 주세요.',
    'Unlock the device, then retry':'기기 잠금을 푼 뒤 다시 시도해 주세요.',
    'Apple device service not found; install Apple Devices or iTunes':'iPhone·iPad를 연결하려면 Apple Devices 앱이나 iTunes를 설치해 주세요.',
    'Could not reach the device; check its connection and retry':'기기에 연결하지 못했습니다. 케이블이나 Wi-Fi 연결을 확인하고 다시 시도해 주세요.',
    'Could not list iPhones or iPads; check the connection and retry':'iPhone·iPad 목록을 가져오지 못했습니다. 연결을 확인하고 다시 시도해 주세요.',
    'adb was not found; install Android SDK Platform-Tools':'adb를 찾지 못했습니다. Android SDK Platform-Tools를 설치해 주세요.',
    'adb failed; check the device connection and retry':'adb가 응답하지 않았습니다. 기기 연결을 확인하고 다시 시도해 주세요.',
    'The selected file is not adb':'선택한 파일은 adb가 아닙니다. platform-tools 폴더의 adb를 선택해 주세요.',
    'Could not save the adb location':'adb 위치를 저장하지 못했습니다. 앱 데이터 폴더에 쓸 수 있는지 확인해 주세요.',
    'Recording stopped at 8 GiB':'기록한 로그가 8 GiB에 도달해 기록을 멈췄습니다.',
    'Recording stopped: not enough disk space':'디스크 공간이 부족해 기록을 멈췄습니다.',
    'Recording stopped: the capture file could not be written':'임시 파일에 쓸 수 없어 기록을 멈췄습니다.',
    'Could not create a capture file':'기록할 임시 파일을 만들지 못했습니다.',
    'Capture files are unavailable':'기기 로그용 임시 폴더에 쓸 수 없습니다.',
    'No device capture is open':'열린 기기 로그가 없습니다.',
    'No device capture is recording':'기록 중인 기기 로그가 없습니다.',
    'Choose a file other than the capture':'기록 중인 임시 파일에는 저장할 수 없습니다. 다른 위치를 선택해 주세요.',
    'Save cancelled':'저장을 취소했습니다.',
    'Save failed':'저장하지 못했습니다. 다시 시도해 주세요.',
    'Write permission denied for the file':'선택한 위치에 쓸 권한이 없습니다. 다른 위치를 선택해 주세요.',
    'Not enough disk space to save':'디스크 공간이 부족해 저장하지 못했습니다.',
    'Not a capture file':'기기 로그 파일이 아닙니다.',
    'The capture file is in use':'다른 LogStudio 창에서 쓰고 있는 기기 로그입니다.',
    'The capture file is gone':'기기 로그 파일을 찾지 못했습니다. 목록을 다시 확인해 주세요.',
    'Could not delete the capture file':'기기 로그 파일을 지우지 못했습니다.',
    'The dropped files are gone; drop them again':'끌어다 놓은 파일을 다시 놓아 주세요.',
    'Starting an app needs Xcode; install it from the App Store':'앱을 실행하려면 Xcode가 필요합니다. App Store에서 Xcode를 설치해 주세요.',
    'Open Xcode once to finish installing its components, then retry':'Xcode를 한 번 열어 추가 구성 요소 설치를 마친 뒤 다시 시도해 주세요.',
    "Xcode cannot reach the device; connect it in Xcode's Devices and Simulators window, then retry":'Xcode가 이 기기에 연결하지 못했습니다. Xcode의 Devices and Simulators 창에서 기기를 연결한 뒤 다시 시도해 주세요.',
    'App not found; open the app list again':'앱을 찾지 못했습니다. 앱 목록을 다시 열어 주세요.',
    'Could not start the app':'앱을 실행하지 못했습니다. 기기와 Xcode 연결을 확인하고 다시 시도해 주세요.',
    'Xcode did not start the app within 90 s; check the device and retry':'Xcode가 90초 안에 앱을 실행하지 못했습니다. 기기 상태를 확인하고 다시 시도해 주세요.',
    "Could not run Xcode's devicectl; check the Xcode installation":'Xcode의 devicectl을 실행하지 못했습니다. Xcode 설치를 확인해 주세요.',
    'Could not list the apps; check the connection and retry':'앱 목록을 가져오지 못했습니다. 케이블이나 Wi-Fi 연결을 확인하고 다시 시도해 주세요.',
    'Apps start only while an iPhone or iPad is recording':'iPhone·iPad를 기록하는 동안에만 앱을 실행할 수 있습니다.',
    [BUSY]:'처리가 지연되고 있습니다. 잠시 후 다시 시도해 주세요.'
  };
  // devicectl's own reason, in English as Xcode gives it.
  if (text.startsWith('Could not start the app: ')) return `앱을 실행하지 못했습니다: ${text.slice('Could not start the app: '.length)}`;
  if (text.startsWith('Invalid or unsupported search pattern: ')) return `정규식을 확인해 주세요: ${text.slice('Invalid or unsupported search pattern: '.length)}`;
  if (text.startsWith(MESSAGE_PATTERN_ERROR)) return `${format === 'dlt' ? '페이로드' : '메시지'} 정규식을 확인해 주세요: ${text.slice(MESSAGE_PATTERN_ERROR.length)}`;
  // Engine text without a translation keeps its detail behind Korean context.
  return messages[text] ?? (text && !/[가-힣]/.test(text) ? `작업을 완료하지 못했습니다. (${text})` : text);
}

// An open of several files that failed on one of them names that file.
export function describeFileError(value, file) {
  const message = describeLogError(value);
  return file && message ? `${file}: ${message}` : message;
}

// Why following stopped by itself; the rows read until then stay usable.
export function describeFollowError(value) {
  const text = String(value ?? '').slice(0, 500);
  const reasons = {
    'NUL is not supported; select a UTF-8 text log':'추가된 내용에 NUL 바이트가 있어',
    'A physical line exceeds 1 MiB':'추가된 줄이 1 MiB를 넘어',
    'Following stopped at 8 GiB':'파일이 8 GiB에 도달해'
  };
  if (!text) return '';
  return `${reasons[text] ?? '추가된 내용을 읽지 못해'} 따라가기를 멈췄습니다. 그 앞까지 읽은 줄은 그대로 볼 수 있습니다.`;
}

// One condition's criteria as chips, one per field; a clipped list keeps
// every item in `full`. A DLT row's tags are its ECU, APID and CTID.
export function describeCondition(c={}, format='text') {
  const chips=[], levels=c.levels ?? LEVELS;
  if (levels.length !== LEVELS.length) {
    const set=new Set(levels), warnOrAbove=set.size===4&&['W','E','F','A'].every(level=>set.has(level));
    // A short list reads better than a long one: name the few hidden levels
    // when fewer are hidden than shown.
    const names=LEVELS.filter(level=>set.has(level)),hidden=LEVELS.filter(level=>!set.has(level)),full=list=>list.map(level=>levelNames(format)[level]).join('·');
    chips.push({key:'levels', label:`레벨: ${!names.length?'없음':warnOrAbove?'Warn 이상':names.length<=2?full(names):hidden.length<names.length?`${full(hidden)} 제외`:names.join('·')}`});
  }
  for (const [key, label, items] of [['tag',format==='dlt'?'ID':'태그',tagList(c.tag)],['pid','PID',idList(c.pid)],['tid','TID',idList(c.tid)]]) {
    const text=items.join(', ');
    if (text) chips.push({key, label:`${label}: ${text.length>40?`${text.slice(0,40)}…`:text}`, full:`${label}: ${text}`});
  }
  const from=String(c.timeFrom ?? '').trim(), to=String(c.timeTo ?? '').trim();
  if (from || to) chips.push({key:'time', label:`시간: ${from||'처음'} ~ ${to||'끝'}`});
  // A message keeps its spaces, so quotes show where it starts and ends.
  const message=String(c.message ?? '');
  if (message) {
    const name=`${format==='dlt'?'페이로드':'메시지'}${c.messageRegex===true?' 정규식':''}`, shown=message.length>40?`${message.slice(0,40)}…`:message;
    chips.push({key:'message', label:`${name}: “${shown}”`, full:`${name}: “${message}”`});
  }
  return chips;
}
// Applied criteria as chips, each naming its condition (`list`, `index`):
// those of enabled conditions, then the unparsed rows option.
export function describeActiveFilters(filters={}, format='text') {
  const chips=[];
  for (const list of ['include', 'exclude'])
    (filters[list] ?? []).forEach((c, index) => {if (isActiveCondition(c)) chips.push(...describeCondition(c, format).map(chip => ({...chip, list, index})));});
  if (filters.includeRaw === false) chips.push({key:'includeRaw', label:'미인식 줄 제외'});
  return chips;
}
// The applied filter as one line: showing conditions joined by 또는, then
// each hiding condition after 제외.
export function describeFilter(filters={}, format='text') {
  const group=c=>describeCondition(c, format).map(chip=>chip.full ?? chip.label);
  const shows=(filters.include ?? []).filter(isActiveCondition).map(group), hides=(filters.exclude ?? []).filter(isActiveCondition).map(group);
  const parts=[];
  if (shows.length) parts.push(shows.map(g=>shows.length>1&&g.length>1?`(${g.join(' · ')})`:g.join(' · ')).join(' 또는 '));
  for (const g of hides) parts.push(`제외 ${g.length>1?`(${g.join(' · ')})`:g.join(' · ')}`);
  if (filters.includeRaw === false) parts.push('미인식 줄 제외');
  return parts.join(' · ');
}
