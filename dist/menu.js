// One positioned popup menu for row actions, recent files and saved filters.
// Items are [label, action, title?, options?]; null draws a separator and an
// item without an action is a note, read out as the menu's description.
// Options: `checked` marks one of a set of choices (focused first),
// `disabled` an item that can be reached but not chosen, `detail` words kept
// after a label that may be cut, `note` a second line, `key` what names the
// item when the menu is shown again while open (its focus stays there), and
// `group` a note that heads the items after it. Escape restores focus to the
// opener, which a menu shown again while open keeps; Tab, focus moving
// elsewhere, outside presses and window blur/resize close it. A menu opened
// on a list (`onList`: its rows or headers) also closes when the user
// scrolls a list (`moved`), and focus returns to its opener; rows added to a
// followed log close no menu. `onClose(press)` runs once when the menu
// closes or another menu replaces it, after a chosen item's action, and gets
// the outside press that closed it, if one did. `moveBy` moves an open menu
// along with the row it was opened on. `show` returns the menu's version,
// which every show and close changes: an owner whose answer comes late
// shows it only while `version()` is still the one it saw. The element
// holds `popupOpener` while open, so a dialog opened from the menu returns
// focus there (dom.js `focusOwner`).
export function createPopupMenu({menu, document, window}) {
  let opener = null, notes = 0, onList = false, onClose = null, at = null, version = 0;
  function close(restore = false, press = null) {
    if (menu.hidden) return;
    const told = take();
    version++;
    menu.hidden = true;
    menu.replaceChildren();
    menu.removeAttribute('aria-describedby');
    menu.popupOpener = null;
    if (restore) opener?.focus({preventScroll:true});
    opener = null;
    told?.(press);
  }
  function take() {
    const told = onClose;
    onClose = null;
    return told;
  }
  function text(tag, className, words) {
    const element = document.createElement(tag);
    element.className = className;
    element.textContent = words;
    return element;
  }
  function entry(item) {
    if (!item) {
      const line = document.createElement('hr');
      line.setAttribute('role', 'separator');
      return line;
    }
    const [label, action, title, options = {}] = item;
    if (!action && !options.disabled) {
      const note = text('p', options.group ? 'menu-note menu-group' : 'menu-note', label);
      note.id = `menu-note-${++notes}`;
      return note;
    }
    const button = document.createElement('button');
    button.type = 'button';
    button.tabIndex = -1;
    if (options.detail || options.note) {
      // Spaces between the parts keep them apart in the item's name.
      const line = document.createElement('span');
      line.className = 'menu-line';
      line.append(text('span', 'menu-label', label));
      if (options.detail) line.append(' ', text('span', 'menu-detail', `· ${options.detail}`));
      button.className = 'menu-rich';
      button.append(line);
      if (options.note) button.append(' ', text('span', 'menu-sub', options.note));
    } else {
      button.textContent = label;
    }
    if (title) button.title = title;
    if (options.key) button.dataset.key = options.key;
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
        const told = take();
        close(true);
        try {
          action();
        } finally {
          told?.(null);
        }
      });
    }
    return button;
  }
  // Keeps the menu inside the window.
  function place(x, y) {
    at = {x, y};
    const box = menu.getBoundingClientRect(), width = window.innerWidth || 0, height = window.innerHeight || 0;
    menu.style.left = `${Math.max(4, width ? Math.min(x, width - box.width - 4) : x)}px`;
    menu.style.top = `${Math.max(4, height ? Math.min(y, height - box.height - 4) : y)}px`;
  }
  function moveBy(dy) {if (!menu.hidden && at) place(at.x, at.y + dy);}
  function show(items, x, y, label, options = {}) {
    const again = !menu.hidden, focused = menu.contains(document.activeElement) ? document.activeElement.dataset?.key : undefined;
    onList = options.onList === true;
    if (!again) opener = document.activeElement;
    menu.popupOpener = opener;
    // A menu shown again in place of another is a new one to its owner.
    take()?.(null);
    onClose = options.onClose ?? null;
    version++;
    menu.setAttribute('aria-label', label);
    menu.replaceChildren(...items.map(entry));
    // Notes say what the items cannot: why a device is missing, for one.
    const described = [...menu.querySelectorAll('.menu-note')].map(note => note.id).join(' ');
    if (described) menu.setAttribute('aria-describedby', described);
    else menu.removeAttribute('aria-describedby');
    menu.hidden = false;
    place(x, y);
    // A menu of notes only takes focus itself, so Escape still closes it.
    const kept = again && focused ? [...menu.querySelectorAll('[role^=menuitem]')].find(item => item.dataset.key === focused) : null;
    const first = kept
      ?? menu.querySelector('[aria-checked=true]')
      ?? menu.querySelector('[role^=menuitem]:not([aria-disabled=true])')
      ?? menu.querySelector('[role^=menuitem]');
    if (first) {
      first.focus();
    } else {
      menu.tabIndex = -1;
      menu.focus();
    }
    return version;
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
    if (!menu.hidden && !menu.contains(event.target)) close(false, event);
  }, true);
  document.addEventListener('focusin', event => {
    if (!menu.hidden && !menu.contains(event.target)) close(false);
  });
  window.addEventListener('blur', () => close(false));
  window.addEventListener('resize', () => close(false));
  function moved() {if (onList) close(true);}
  // The element focused when the open menu was shown.
  return {show, close, moved, moveBy, opener: () => opener, version: () => version};
}
