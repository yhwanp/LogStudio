// One positioned popup menu for row actions, recent files and saved filters.
// Items are [label, action, title?, {checked, disabled}?]; null draws a
// separator, an item without an action is a note, `checked` marks one of a
// set of choices (focused first) and `disabled` an item that can be reached
// but not chosen. Escape restores focus to the opener, which a menu shown
// again while open keeps; Tab, outside presses, window blur/resize and list
// scrolling close it.
export function createPopupMenu({menu, document, window, scrollers = []}) {
  let opener = null;
  function close(restore = false) {
    if (menu.hidden) return;
    menu.hidden = true;
    menu.replaceChildren();
    if (restore) opener?.focus({preventScroll:true});
    opener = null;
  }
  function entry(item) {
    if (!item) {
      const line = document.createElement('hr');
      line.setAttribute('role', 'separator');
      return line;
    }
    const [label, action, title, options = {}] = item;
    if (!action && !options.disabled) {
      const note = document.createElement('p');
      note.className = 'menu-note';
      note.textContent = label;
      return note;
    }
    const button = document.createElement('button');
    button.type = 'button';
    button.tabIndex = -1;
    button.textContent = label;
    if (title) button.title = title;
    if ('checked' in options) {
      button.setAttribute('role', 'menuitemradio');
      button.setAttribute('aria-checked', String(!!options.checked));
    } else {
      button.setAttribute('role', 'menuitem');
    }
    if (options.disabled) {
      button.setAttribute('aria-disabled', 'true');
    } else {
      button.addEventListener('click', () => {
        close(true);
        action();
      });
    }
    return button;
  }
  function show(items, x, y, label) {
    if (menu.hidden) opener = document.activeElement;
    menu.setAttribute('aria-label', label);
    menu.replaceChildren(...items.map(entry));
    menu.hidden = false;
    // Keep the menu inside the window.
    const box = menu.getBoundingClientRect(), width = window.innerWidth || 0, height = window.innerHeight || 0;
    menu.style.left = `${Math.max(4, width ? Math.min(x, width - box.width - 4) : x)}px`;
    menu.style.top = `${Math.max(4, height ? Math.min(y, height - box.height - 4) : y)}px`;
    // A menu of notes only takes focus itself, so Escape still closes it.
    const first = menu.querySelector('[aria-checked=true]')
      ?? menu.querySelector('[role^=menuitem]:not([aria-disabled=true])')
      ?? menu.querySelector('[role^=menuitem]');
    if (first) {
      first.focus();
    } else {
      menu.tabIndex = -1;
      menu.focus();
    }
  }
  menu.addEventListener('keydown', event => {
    const items = [...menu.querySelectorAll('[role^=menuitem]')], at = items.indexOf(document.activeElement);
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close(true);
    } else if (event.key === 'Tab') {
      close(false);
    } else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key) && items.length) {
      event.preventDefault();
      let next;
      if (event.key === 'Home') next = 0;
      else if (event.key === 'End') next = items.length - 1;
      else next = (at + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
      items[next].focus();
    }
  });
  document.addEventListener('pointerdown', event => {
    if (!menu.hidden && !menu.contains(event.target)) close(false);
  }, true);
  window.addEventListener('blur', () => close(false));
  window.addEventListener('resize', () => close(false));
  for (const scroller of scrollers) scroller.addEventListener('scroll', () => close(false));
  return {show, close};
}
