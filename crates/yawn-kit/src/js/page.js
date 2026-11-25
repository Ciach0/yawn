// The on-screen keyboard never covers the field being typed in: while it is up, the scrolling parent gets room below its content for
// the part of the window the keyboard hides, and is scrolled until the field is in the part that is left.
export function keepFocusVisible() {
  if (window.__yawnKeyboard || !window.visualViewport) return;
  window.__yawnKeyboard = 1;
  var vv = window.visualViewport, padded = null, saved = '';
  function editable(e) { return e && e.matches && e.matches('input, textarea, select, [contenteditable="true"]'); }
  function scroller(e) {
    for (var p = e.parentElement; p && p !== document.body; p = p.parentElement) {
      var o = getComputedStyle(p).overflowY;
      if (o === 'auto' || o === 'scroll') return p;
    }
    return null;
  }
  function release() { if (padded) { padded.style.paddingBottom = saved; padded = null; } }
  function fit() {
    var e = document.activeElement;
    var hidden = Math.max(0, window.innerHeight - vv.height - vv.offsetTop);
    if (!editable(e) || hidden < 80) { release(); return; }
    var s = scroller(e);
    if (s && padded !== s) {
      release();
      padded = s; saved = s.style.paddingBottom;
      s.dataset.keyboardBase = parseFloat(getComputedStyle(s).paddingBottom) || 0;
    }
    if (padded) padded.style.paddingBottom = (parseFloat(padded.dataset.keyboardBase) + hidden) + 'px';
    var r = e.getBoundingClientRect();
    var bottom = vv.offsetTop + vv.height - 16, top = vv.offsetTop + 72;
    var dy = r.bottom > bottom ? r.bottom - bottom : (r.top < top ? r.top - top : 0);
    if (dy) { if (s) s.scrollBy(0, dy); else window.scrollBy(0, dy); }
  }
  vv.addEventListener('resize', fit);
  vv.addEventListener('scroll', fit);
  document.addEventListener('focusin', function () { setTimeout(fit, 60); setTimeout(fit, 320); });
  document.addEventListener('focusout', function () { setTimeout(fit, 100); });
  document.addEventListener('input', fit, true);
}
export function viewportWidth() { return window.innerWidth; }
export function viewportHeight() { return window.innerHeight; }
export function copyText(text) { try { if (navigator.clipboard) navigator.clipboard.writeText(text); } catch (x) {} }
export function matchesMedia(query) { return !!(window.matchMedia && window.matchMedia(query).matches); }
var watches = [];
export function watchMedia(query, callback) {
  if (!window.matchMedia) return -1;
  var list = window.matchMedia(query);
  var listener = function (e) { callback(e.matches); };
  list.addEventListener('change', listener);
  watches.push({ list: list, listener: listener });
  return watches.length - 1;
}
export function unwatchMedia(id) {
  var w = watches[id]; if (!w) return;
  w.list.removeEventListener('change', w.listener); watches[id] = null;
}
export function underPointer(x, y, attr) {
  var e = document.elementFromPoint(x, y);
  while (e) { if (e.hasAttribute && e.hasAttribute(attr)) return e.getAttribute(attr); e = e.parentElement; }
  return null;
}
