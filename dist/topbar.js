// The top bar's layout during a device capture. It keeps one row while the
// state text before the device name fits; only the device name shortens.
// Once one row would clip that text too, the capture bar takes a second row
// and moves after .top-actions in the DOM, so Tab visits the top row first.
// Checked again when the window, the zoom, the fonts or the bar's contents
// change, each time in the one-row layout and before the next paint.
export function mountTopbar(document, window) {
  const bar = document.getElementById('captureBar'), text = document.getElementById('captureText');
  const top = bar.parentElement, actions = top.querySelector('.top-actions');
  let last = null;
  const place = wrap => {
    top.classList.toggle('capture-wrap', wrap);
    if ((bar.nextElementSibling === actions) !== wrap) return;
    const focused = bar.contains(document.activeElement) ? document.activeElement : null;
    top.insertBefore(bar, wrap ? null : actions);
    focused?.focus({preventScroll:true});
  };
  // Whether the text before the device name (devices.js marks its length)
  // ends past the visible text. Measured unclipped: WebKit places the
  // characters an ellipsis hides at the ellipsis.
  const cut = () => {
    const node = text.firstChild, length = text.textContent.length, keep = Number(text.dataset.keep);
    if (!node) return false;
    const range = document.createRange();
    range.setStart(node, 0);
    range.setEnd(node, Number.isInteger(keep) && keep >= 0 && keep <= length ? keep : length);
    text.style.textOverflow = 'clip';
    const over = range.getBoundingClientRect().right > text.getBoundingClientRect().right + 0.5;
    text.style.textOverflow = '';
    return over;
  };
  const fit = () => {
    const shown = [...top.querySelectorAll('.capture-bar button, .top-actions button')].map(button => button.hidden);
    const key = [top.clientWidth, bar.hidden, bar.textContent, ...shown].join('|');
    if (key === last) return;
    last = key;
    if (bar.hidden) return place(false);
    top.classList.remove('capture-wrap');
    place(cut());
  };
  new window.MutationObserver(fit).observe(top, {subtree:true, childList:true, characterData:true, attributes:true, attributeFilter:['hidden']});
  // A resize is measured in the next frame, once the new width is laid out.
  window.addEventListener('resize', () => window.requestAnimationFrame(fit));
  document.fonts?.ready.then(() => {last = null; fit();});
  fit();
}
