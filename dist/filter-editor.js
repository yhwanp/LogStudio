// The condition editor: a window that floats over the log without blocking
// it, so results show while conditions change. It stays open on clicks
// elsewhere, moves by its heading, and opens where it was left, or beside the
// filter panel. The layout remembers the place; the view owns the fields.
const GAP = 8;
const MARGIN = 8;
const STEP = 16;

export function mountFilterEditor({document, window, layout, onClose}) {
  const $ = id => document.getElementById(id);
  const box = $('filterEditor'), head = $('filterEditorHead'), title = $('filterEditorTitle');
  // `wanted` is where it should be; `place` keeps it in the window, also as
  // help, a hint or an error makes it taller.
  let opener = null, drag = null, wanted = null;
  const isOpen = () => !box.hidden;
  const inView = (x, y) => {
    const width = box.offsetWidth, height = box.offsetHeight;
    return {x:Math.round(Math.max(MARGIN, Math.min(window.innerWidth - width - MARGIN, x))), y:Math.round(Math.max(MARGIN, Math.min(window.innerHeight - height - MARGIN, y)))};
  };
  function place(x, y) {
    const at = inView(x, y);
    box.style.left = `${at.x}px`; box.style.top = `${at.y}px`;
    return at;
  }
  // Beside the panel when it is open, else beside the rail, at the list's top.
  function beside() {
    const panel = $('filterPanel').getBoundingClientRect?.(), rail = document.querySelector?.('.filter-rail')?.getBoundingClientRect?.(), list = $('logsViewport').getBoundingClientRect?.();
    const edge = layout.isOpen?.() && panel?.width ? panel.right : rail?.right ?? 0;
    return {x:edge + GAP, y:list?.top ?? 0};
  }
  function position() {
    wanted = layout.editorPlace?.() ?? beside();
    place(wanted.x, wanted.y);
  }
  function moveTo(x, y) {
    wanted = inView(x, y);
    place(wanted.x, wanted.y);
    layout.setEditorPlace?.(wanted);
  }
  function open({heading, from=document.activeElement}={}) {
    if (heading) title.textContent = heading;
    if (isOpen()) return;
    opener = from; box.hidden = false; position();
  }
  function close({focus=true}={}) {
    if (!isOpen()) return false;
    box.hidden = true; drag = null;
    const back = opener; opener = null;
    const item = onClose();
    if (focus) {
      // What opened it may be gone, as a rebuilt list's item or a closed row
      // menu's is: its condition's item takes focus, then the log.
      back?.focus?.({preventScroll:true});
      if (back?.isConnected === false || document.activeElement === document.body || !document.activeElement) item?.focus?.({preventScroll:true});
      if (document.activeElement === document.body || !document.activeElement) $('logsViewport').focus?.({preventScroll:true});
    }
    return true;
  }
  head.addEventListener('pointerdown', event => {
    if (event.button !== 0 || event.target.closest?.('button')) return;
    const at = box.getBoundingClientRect();
    drag = {pointer:event.pointerId, dx:event.clientX - at.left, dy:event.clientY - at.top};
    head.setPointerCapture?.(event.pointerId);
    event.preventDefault();
  });
  head.addEventListener('pointermove', event => {
    if (drag?.pointer === event.pointerId) place(event.clientX - drag.dx, event.clientY - drag.dy);
  });
  const drop = event => {
    if (drag?.pointer !== event.pointerId) return;
    drag = null;
    moveTo(parseFloat(box.style.left), parseFloat(box.style.top));
  };
  head.addEventListener('pointerup', drop); head.addEventListener('pointercancel', drop);
  // The keyboard moves it too, by the arrows, faster with Shift; Home and a
  // double click put it back beside the panel.
  const backHome = () => {layout.setEditorPlace?.(null); position();};
  head.addEventListener('dblclick', event => {if (!event.target.closest?.('button')) backHome();});
  head.addEventListener('keydown', event => {
    if (event.target !== head) return;
    const step = event.shiftKey ? STEP * 4 : STEP, move = {ArrowLeft:[-step, 0], ArrowRight:[step, 0], ArrowUp:[0, -step], ArrowDown:[0, step]}[event.key];
    if (event.key === 'Home') backHome();
    else if (move) moveTo(parseFloat(box.style.left) + move[0], parseFloat(box.style.top) + move[1]);
    else return;
    event.preventDefault();
  });
  window.addEventListener('resize', () => {if (isOpen()) position();});
  if (window.ResizeObserver) new window.ResizeObserver(() => {if (isOpen() && wanted && !drag) place(wanted.x, wanted.y);}).observe(box);
  // Escape stops here, so it does not also close the raw detail or the panel.
  box.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    event.preventDefault(); event.stopPropagation(); close();
  });
  $('closeEditor').addEventListener('click', () => close());
  return {isOpen, open, close, contains:node => box.contains(node), setHeading(text) {title.textContent = text;}};
}
