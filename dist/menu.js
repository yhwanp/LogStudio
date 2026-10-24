// One positioned popup menu for row actions, recent files and saved filters.
// Items are [label, action, title?, options?]; null draws a separator and an
// item without an action is a note, read out as the menu's description.
// Options: `checked` marks one of a set of choices (focused first),
// `disabled` an item that can be reached but not chosen, `detail` words kept
// after a label that may be cut, `note` a second line, `key` what names the
// item when the menu is shown again while open (its focus stays there), and
// `group` a note that heads the items after it. Escape restores focus to the
// opener, which a menu shown again while open keeps; Tab, outside presses
// and window blur/resize close it. A menu opened on a list (`onList`: its
// rows or headers) also closes when a list scrolls; one from a button stays
// while a followed log scrolls under it.
export function createPopupMenu({menu, document, window, scrollers = []}) {
  let opener = null, notes = 0, onList = false;
  function close(restore = false) {
    if (menu.hidden) return;
    menu.hidden = true;
    menu.replaceChildren();
    menu.removeAttribute('aria-describedby');
    if (restore) opener?.focus({preventScroll:true});
    opener = null;
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
        close(true);
        action();
      });
    }
    return button;
  }
  function show(items, x, y, label, options = {}) {
    const again = !menu.hidden, focused = menu.contains(document.activeElement) ? document.activeElement.dataset?.key : undefined;
    onList = options.onList === true;
    if (!again) opener = document.activeElement;
    menu.setAttribute('aria-label', label);
    menu.replaceChildren(...items.map(entry));
    // Notes say what the items cannot: why a device is missing, for one.
    const described = [...menu.querySelectorAll('.menu-note')].map(note => note.id).join(' ');
    if (described) menu.setAttribute('aria-describedby', described);
    else menu.removeAttribute('aria-describedby');
    menu.hidden = false;
    // Keep the menu inside the window.
    const box = menu.getBoundingClientRect(), width = window.innerWidth || 0, height = window.innerHeight || 0;
    menu.style.left = `${Math.max(4, width ? Math.min(x, width - box.width - 4) : x)}px`;
    menu.style.top = `${Math.max(4, height ? Math.min(y, height - box.height - 4) : y)}px`;
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
  for (const scroller of scrollers) scroller.addEventListener('scroll', () => {if (onList) close(false);});
  return {show, close};
}
