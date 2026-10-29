// Small DOM helpers shared by the view modules.
export function textSpan(document, text, className='') {const span = document.createElement('span'); span.className = className; span.textContent = text ?? ''; return span;}

// Where focus returns when a dialog opened now closes. Focus inside an open
// popup menu, which closes as the dialog takes focus, returns to the menu's
// opener instead.
export function focusOwner(document) {
  const active = document.activeElement;
  return active?.closest?.('[role=menu]')?.popupOpener ?? active;
}

// Prefer the async clipboard so slow multi-page copies keep the user gesture.
export async function writeClipboard(window, text) {
  const clipboard=window.navigator?.clipboard;
  if (clipboard?.write && window.ClipboardItem) {
    await clipboard.write([new window.ClipboardItem({'text/plain':Promise.resolve(text).then(value=>new Blob([value],{type:'text/plain'}))})]);
  } else if (clipboard?.writeText) await clipboard.writeText(await text);
  else throw new Error('Clipboard unavailable');
}
