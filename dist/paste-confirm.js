// Pasting log text or a copied file replaces the open log, so a paste while
// a log is open asks first. One modal dialog says what would open and what
// becomes of the open log: 열기 (focused, so Enter) opens it; 취소, Escape
// or a press on the backdrop keeps the open log. Focus returns to where it
// was. A paste while the dialog shows replaces what it offers.
import {formatNumber, formatSize} from './describe.js';

// A paste replaces a log that is shown, even one still being read, or one
// being opened.
export const replacesLog = (state, opening = false) => opening || (state?.datasetId != null && !state.closed && state.status !== 'empty');

// What would open: text by its rows and size, a file by its base name.
export function describePaste(item) {
  if (item?.kind === 'text') return `로그 텍스트 ${formatNumber(item.lines)}줄 · ${formatSize(item.size)}`;
  const name = typeof item?.name === 'string' ? item.name.slice(0, 255) : '';
  return name ? `파일 ${name}` : '복사한 파일';
}

// What becomes of the open log. A file's work is saved as another log opens
// and restored when the file is opened again; pasted text and the sample
// cannot be opened again. Unknown says only that it closes.
export function describeClosing(reopenable) {
  if (reopenable === true) return '열려 있는 로그는 닫힙니다. 필터·검색·북마크는 저장되어 같은 파일을 다시 열면 복원됩니다.';
  if (reopenable === false) return '열려 있는 로그는 닫히며 다시 열 수 없습니다.';
  return '열려 있는 로그는 닫힙니다.';
}

// Pastes as the page receives them: text from the paste event, a copied
// file as a native grant. `replaces` says whether a paste would replace a
// log, `closing` what becomes of it; `openText` opens text, `report` shows
// an error, `isGone` says the page is going away and `inField` that a text
// field has focus (a file paste then stays the standard Paste). A file's
// grant is held while the dialog asks, and dropped, never pasted, when the
// user keeps the log or a newer paste arrives while it is being held.
// A shown device capture with unsaved rows asks its own question instead
// (`guard`, devices.js): 'asked' opens, 'cancel' keeps the log.
export function createPasteFlow({invoke, dialog, replaces, closing, openText, report, isGone, inField, guard = async () => 'none'}) {
  let ticket = 0;
  const drop = grant => {void Promise.resolve().then(() => invoke('finish_file_paste', {grant, action:'discard'})).catch(() => {});};
  async function finish(grant, action) {
    try {await invoke('finish_file_paste', {grant, action});} catch (error) {if (!isGone()) report(error);}
  }
  async function confirm(item) {
    const asked = await guard();
    const answer = asked === 'cancel' ? 'cancel' : asked === 'asked' ? 'open' : await dialog.ask({what:describePaste(item), note:closing()});
    if (answer === 'open' && !isGone()) return true;
    if (item.kind === 'file') drop(item.grant);
    return false;
  }
  return {
    async text(item) {
      ticket++;
      if (replaces() && !await confirm(item)) return;
      openText(item);
    },
    async file(payload) {
      const grant = payload?.grant;
      if (typeof grant !== 'string') return;
      if (isGone() || inField()) {await finish(grant, 'paste'); return;}
      const mine = ++ticket;
      if (!replaces()) {await finish(grant, 'open'); return;}
      try {await invoke('hold_file_paste', {grant});}
      catch (error) {drop(grant); if (mine === ticket && !isGone()) report(error); return;}
      if (mine !== ticket || isGone()) {drop(grant); return;}
      if (await confirm({kind:'file', grant, name:payload.name})) await finish(grant, 'open');
    },
  };
}

export function mountPasteConfirm({document = globalThis.document} = {}) {
  const $ = id => document.getElementById(id);
  const dialog = $('pasteDialog'), form = $('pasteForm'), what = $('pasteWhat'), note = $('pasteNote'), openButton = $('pasteOpen');
  let answer = null, opener = null;
  const settle = result => {const done = answer; answer = null; done?.(result);};
  // Resolves 'open' or 'cancel', or 'replaced' once a newer paste takes the dialog.
  function ask({what: text, note: closing}) {
    settle('replaced');
    what.textContent = text; note.textContent = closing;
    if (!dialog.open) {
      opener = document.activeElement;
      if (typeof dialog.showModal === 'function') dialog.showModal(); else dialog.open = true;
    }
    openButton.focus?.({preventScroll: true});
    return new Promise(resolve => {answer = resolve;});
  }
  function close(result) {
    // Taken first: closing fires `close`, which would answer 'cancel'.
    const done = answer; answer = null;
    if (dialog.open) {
      if (typeof dialog.close === 'function') dialog.close(); else dialog.open = false;
      const target = opener; opener = null;
      if (target && target !== document.body && target.isConnected !== false && !target.hidden && !dialog.contains?.(target)) target.focus?.({preventScroll: true});
    }
    done?.(result);
  }
  form.addEventListener('submit', event => {event.preventDefault(); close('open');});
  $('pasteCancel').addEventListener('click', () => close('cancel'));
  // Escape stops at the dialog, so it does not also close the raw detail.
  dialog.addEventListener('keydown', event => {if (event.key === 'Escape') {event.preventDefault(); event.stopPropagation(); close('cancel');}});
  dialog.addEventListener('cancel', event => {event.preventDefault(); close('cancel');});
  // The form fills the dialog box, so only the backdrop targets the dialog.
  dialog.addEventListener('click', event => {if (event.target === dialog) close('cancel');});
  // Closed any other way, nothing opens.
  dialog.addEventListener('close', () => settle('cancel'));
  return {ask, dismiss: () => close('cancel'), get isOpen() {return !!dialog.open;}};
}
