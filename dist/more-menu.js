// The top bar's ⋯ menu keeps what is needed now and then off the main screen.
// `items` lists it as it stands when opened; an item with more choices shows
// them in the menu's place (`sub`), with a way back. The row number box opens
// where the menu does, from its item or Cmd/Ctrl+L.
import {focusOwner} from './dom.js';

export function mountMoreMenu({ui, menu, items, document, window}) {
  const {moreButton:button, rowMenu, logsJumpForm:form, logsJump:input, jumpError:error} = ui;
  let shown = null, current = null, wasOpen = false, jumpOpen = false, returnTo = null;
  function anchor() {
    const box = button.getBoundingClientRect?.();
    return {x:box ? box.right : 0, y:box ? box.bottom + 4 : 0};
  }
  function show(list, label, focus) {
    const {x, y} = anchor(), open = {};
    current = open;
    // Opening a submenu runs the previous menu's onClose after the new show.
    const onClose = () => {if (current === open) button.setAttribute('aria-expanded', 'false');};
    shown = menu.show(list, x, y, label, {alignEnd:true, focus, opener:button, onClose});
    button.setAttribute('aria-expanded', 'true');
  }
  const main = from => show(items(), '메뉴', from);
  // `from` is the `key` of the item that opened these, focused on the way back.
  const sub = (label, list, from) => show([['‹ 메뉴', () => main(from), '메뉴로 돌아가기', {back:true}], null, ...list], label);
  // The outside press that closes the open menu must not open it again.
  window.addEventListener('pointerdown', event => {
    wasOpen = !rowMenu.hidden && menu.version() === shown && !!button.contains?.(event.target);
    if (jumpOpen && !form.contains(event.target)) closeJump();
  }, true);
  // A key's click has no press of its own, and a press released elsewhere
  // leaves `wasOpen` set.
  button.addEventListener('click', event => {
    const closed = wasOpen && event.detail !== 0;
    wasOpen = false;
    if (!closed) main();
  });

  function closeJump(restore = false) {
    if (!jumpOpen) return;
    jumpOpen = false;
    form.hidden = true;
    input.removeAttribute('aria-invalid');
    error.textContent = '';
    if (restore) (returnTo && returnTo !== document.body && returnTo.isConnected !== false ? returnTo : button).focus?.({preventScroll:true});
    returnTo = null;
  }
  function openJump() {
    if (!jumpOpen) {
      const {x, y} = anchor();
      jumpOpen = true;
      returnTo = focusOwner(document);
      form.hidden = false;
      form.style.right = `${Math.max(4, (window.innerWidth || 0) - x)}px`;
      form.style.top = `${y}px`;
    }
    input.focus();
    input.select?.();
  }
  form.addEventListener('keydown', event => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    closeJump(true);
  });
  form.addEventListener('focusout', event => {
    if (event.relatedTarget && !form.contains(event.relatedTarget)) closeJump();
  });
  window.addEventListener('resize', () => closeJump(true));
  return {sub, openJump, closeJump};
}
