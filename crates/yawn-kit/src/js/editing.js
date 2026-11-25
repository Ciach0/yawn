// A box is a <textarea>/<input>, or an element with contenteditable="true" holding one <div> per line: its text is the lines joined by \n.
function rich(e) { return e && e.isContentEditable && typeof e.value !== 'string'; }
function lines(e) { return Array.prototype.slice.call(e.childNodes); }
function richText(e) { return lines(e).map(function (n) { return n.textContent; }).join('\n'); }
function lineStart(e, index) { var t = 0, l = lines(e); for (var i = 0; i < index && i < l.length; i++) t += l[i].textContent.length + 1; return t; }
function pointOffset(e, node, off) {
  if (node === e) return lineStart(e, off);
  var line = node; while (line && line.parentNode !== e) line = line.parentNode;
  if (!line) return richText(e).length;
  var r = document.createRange(); r.selectNodeContents(line); r.setEnd(node, off);
  return lineStart(e, lines(e).indexOf(line)) + r.toString().length;
}
function offsetPoint(e, offset) {
  var l = lines(e);
  for (var i = 0; i < l.length; i++) {
    var len = l[i].textContent.length;
    if (offset <= len || i === l.length - 1) {
      if (l[i].nodeType === 3) return [l[i], Math.min(offset, len)];
      var left = Math.min(offset, len), walker = document.createTreeWalker(l[i], NodeFilter.SHOW_TEXT), n;
      while ((n = walker.nextNode())) {
        var fixed = n.parentElement && n.parentElement.closest('[contenteditable="false"]');
        if (fixed && l[i].contains(fixed)) {
          if (left === 0) return [fixed.parentNode, Array.prototype.indexOf.call(fixed.parentNode.childNodes, fixed)];
          left -= n.nodeValue.length; if (left < 0) left = 0; continue;
        }
        if (left <= n.nodeValue.length) return [n, left]; left -= n.nodeValue.length;
      }
      return [l[i], l[i].childNodes.length > 0 && l[i].lastChild.nodeName !== 'BR' ? l[i].childNodes.length : 0];
    }
    offset -= len + 1;
  }
  return [e, 0];
}
function richSelect(e, a, b) {
  var s = window.getSelection(), r = document.createRange(), p = offsetPoint(e, a), q = offsetPoint(e, b);
  r.setStart(p[0], p[1]); r.setEnd(q[0], q[1]); s.removeAllRanges(); s.addRange(r);
}
export function boxValue(id) { var e = document.getElementById(id); if (rich(e)) return richText(e); return e && typeof e.value === 'string' ? e.value : null; }
export function boxSelection(id) {
  var e = document.getElementById(id); if (!e) return new Float64Array(0);
  if (rich(e)) {
    var s = window.getSelection();
    if (!s.rangeCount || !e.contains(s.anchorNode)) { var n = richText(e).length; return new Float64Array([n, n]); }
    var a = pointOffset(e, s.anchorNode, s.anchorOffset), b = pointOffset(e, s.focusNode, s.focusOffset);
    return new Float64Array([Math.min(a, b), Math.max(a, b)]);
  }
  if (e.selectionStart == null) return new Float64Array(0); return new Float64Array([e.selectionStart, e.selectionEnd]);
}
export function boxSetSelection(id, a, b) { var e = document.getElementById(id); if (!e) return; if (rich(e)) richSelect(e, a, b); else if (e.setSelectionRange) e.setSelectionRange(a, b); }
export function boxInsert(id, text) {
  var e = document.getElementById(id); if (!e) return false;
  e.focus({ preventScroll: true });
  // execCommand keeps the box's own undo history; setRangeText is the fallback where it is gone
  var ok = false;
  try { ok = document.execCommand && document.execCommand('insertText', false, text); } catch (x) {}
  if (!ok && e.setRangeText) { e.setRangeText(text, e.selectionStart, e.selectionEnd, 'end'); e.dispatchEvent(new Event('input', { bubbles: true })); }
  return true;
}
export function boxAutosize(id) { var e = document.getElementById(id); if (!e || rich(e)) return; e.style.height = 'auto'; e.style.height = e.scrollHeight + 'px'; }
export function boxSetHtml(id, html, a, b, focus) {
  var e = document.getElementById(id); if (!e) return;
  e.innerHTML = html;
  if (a >= 0) { if (focus) e.focus({ preventScroll: true }); richSelect(e, a, b); }
}
export function caretRect(id) {
  var e = document.getElementById(id); if (!e) return new Float64Array(0);
  var s = window.getSelection();
  if (!s.rangeCount || !e.contains(s.anchorNode)) return new Float64Array(0);
  var r = s.getRangeAt(0).cloneRange(); r.collapse(false);
  var rs = r.getClientRects();
  if (rs.length) { var b = rs[rs.length - 1]; return new Float64Array([b.left, b.top, b.right, b.bottom]); }
  var n = r.startContainer;
  if (n.nodeType === 1 && r.startOffset > 0) {
    var prev = n.childNodes[r.startOffset - 1];
    if (prev && prev.getBoundingClientRect) { var p = prev.getBoundingClientRect(); return new Float64Array([p.right, p.top, p.right, p.bottom]); }
  }
  var x = r.getBoundingClientRect();
  if (x.width || x.height) return new Float64Array([x.left, x.top, x.right, x.bottom]);
  var q = e.getBoundingClientRect(); return new Float64Array([q.left, q.top, q.left, q.bottom]);
}
export function scrollBox(id, dy) { var e = document.getElementById(id); if (e) e.scrollTop += dy; }
export function autoScroll(x, y, edge, step) {
  var vh = window.innerHeight;
  var dy = y < edge ? -step : (y > vh - edge ? step : 0);
  if (!dy) return;
  var e = document.elementFromPoint(x, y);
  while (e && e !== document.body && e !== document.documentElement) {
    var o = getComputedStyle(e).overflowY;
    if ((o === 'auto' || o === 'scroll') && e.scrollHeight > e.clientHeight) { e.scrollTop += dy; return; }
    e = e.parentElement;
  }
  window.scrollBy(0, dy);
}
export function moveLine(id, selector, delta) {
  var from = document.getElementById(id); if (!from) return false;
  var line = from.closest(selector); if (!line) return false;
  var all = [].slice.call(document.querySelectorAll(selector)), i = all.indexOf(line) + delta;
  if (i < 0 || i >= all.length) return false;
  var sel = window.getSelection(), col = 0;
  if (sel.rangeCount && line.contains(sel.anchorNode)) { var r = document.createRange(); r.selectNodeContents(line); r.setEnd(sel.anchorNode, sel.anchorOffset); col = r.toString().length; }
  var to = all[i], boxes = to.isContentEditable && !to.querySelector('[contenteditable]') ? [to] : [].slice.call(to.querySelectorAll('[contenteditable]'));
  if (!boxes.length) return false;
  var target = boxes[0], at = 0;
  for (var k = 0; k < boxes.length; k++) {
    var q = document.createRange(); q.selectNodeContents(to); q.setEndBefore(boxes[k]);
    var start = to === boxes[k] ? 0 : q.toString().length, len = boxes[k].textContent.length;
    if (col >= start) { target = boxes[k]; at = Math.min(col - start, len); }
  }
  target.focus(); richSelect(target, at, at); return true;
}
var heldTouch = false, heldInstalled = false;
export function holdTouchScroll(on) {
  heldTouch = on;
  if (on && !heldInstalled) { heldInstalled = true; document.addEventListener('touchmove', function (e) { if (heldTouch && e.cancelable) e.preventDefault(); }, { passive: false }); }
}
export function releaseFocus() {
  var a = document.activeElement; if (a && a !== document.body && a.blur) a.blur();
  var s = window.getSelection && window.getSelection(); if (s) s.removeAllRanges();
}
export function textSelects(x, y) {
  var el = document.elementFromPoint(x, y);
  var editor = el && el.closest('[contenteditable="true"]');
  if (!editor || (!editor.contains(document.activeElement) && editor !== document.activeElement)) return false;
  var range = document.createRange(); range.selectNodeContents(editor);
  var rects = range.getClientRects();
  for (var i = 0; i < rects.length; i++) { var r = rects[i]; if (x >= r.left - 2 && x <= r.right + 2 && y >= r.top && y <= r.bottom) return true; }
  return false;
}
