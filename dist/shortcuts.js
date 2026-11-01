// One modal dialog lists the app's keyboard shortcuts. The ⋯ menu's item or
// `?` outside a text field opens it; Escape, the close button or a press on
// the backdrop closes it and returns focus to the opener. Escape stops at the
// dialog, so it does not also close the raw detail or the filter panel.
import {isEditableTarget} from './clipboard.js';
import {focusOwner} from './dom.js';

export function mountShortcutHelp({document = globalThis.document} = {}) {
  const button = document.getElementById('moreButton'), dialog = document.getElementById('shortcutDialog'), closeButton = document.getElementById('shortcutClose');
  if (!dialog) return null;
  let opener = null;
  function open() {
    if (dialog.open) return;
    opener = focusOwner(document);
    if (typeof dialog.showModal === 'function') dialog.showModal(); else dialog.open = true;
    closeButton?.focus({preventScroll: true});
  }
  function close() {
    if (!dialog.open) return;
    if (typeof dialog.close === 'function') dialog.close(); else { dialog.open = false; restore(); }
  }
  function restore() {
    // A menu opened by mouse leaves no focused opener, only the body.
    const target = opener?.isConnected === false || opener === document.body ? button : opener ?? button;
    opener = null;
    target?.focus?.({preventScroll: true});
  }
  closeButton?.addEventListener('click', close);
  // Every way the dialog closes, including the native Escape, ends here.
  dialog.addEventListener('close', restore);
  dialog.addEventListener('cancel', event => { event.preventDefault(); close(); });
  dialog.addEventListener('keydown', event => {
    if (event.key !== 'Escape' && event.key !== '?') return;
    if (event.key === '?' && (event.ctrlKey || event.metaKey || event.altKey)) return;
    event.preventDefault(); event.stopPropagation(); close();
  });
  // The content fills the dialog box, so only the backdrop targets the dialog.
  dialog.addEventListener('click', event => { if (event.target === dialog) close(); });
  document.addEventListener('keydown', event => {
    if (event.key !== '?' || event.ctrlKey || event.metaKey || event.altKey || event.isComposing || event.defaultPrevented || dialog.open || isEditableTarget(event.target)) return;
    event.preventDefault(); open();
  });
  return {open, close, get isOpen() { return !!dialog.open; }};
}
