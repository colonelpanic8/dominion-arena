// What a page shows, read the same way from both clients: every visible picture (by
// its file name) and every visible run of text, with their boxes in viewport pixels.
// Elements matching `ignore` are left out, and their boxes returned as `ignored`; those
// matching `pixelsOnly` are left out but stay in the screenshot. Runs in the page.
export function extract({ ignore, pixelsOnly }) {
  const W = innerWidth;
  const H = innerHeight;
  const box = (r) => {
    const x0 = Math.max(0, r.left);
    const y0 = Math.max(0, r.top);
    const x1 = Math.min(W, r.right);
    const y1 = Math.min(H, r.bottom);
    if (x1 - x0 < 2 || y1 - y0 < 2) return null;
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  };
  const visible = (el) =>
    el.checkVisibility({ opacityProperty: true, visibilityProperty: true, contentVisibilityAuto: true });
  // Clipped away by an ancestor that hides its overflow.
  const clipped = (el, r) => {
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const cs = getComputedStyle(p);
      if (cs.overflow === 'visible' && cs.overflowX === 'visible' && cs.overflowY === 'visible') continue;
      const c = p.getBoundingClientRect();
      if (r.right <= c.left || r.left >= c.right || r.bottom <= c.top || r.top >= c.bottom) return true;
    }
    return false;
  };
  const images = [];
  const texts = [];
  const skip = [...ignore, ...pixelsOnly].join(',');
  const ignored = [...document.querySelectorAll(ignore.join(','))]
    .filter(visible)
    .map((el) => box(el.getBoundingClientRect()))
    .filter(Boolean);
  for (const el of document.body.querySelectorAll('*')) {
    if (el.closest(skip) || !visible(el)) continue;
    const cs = getComputedStyle(el);
    let url = null;
    if (el.tagName === 'IMG') url = el.currentSrc || el.src;
    else {
      const m = cs.backgroundImage.match(/url\("?([^")]+)"?\)/);
      if (m) url = m[1];
    }
    if (url && !url.startsWith('data:')) {
      const r = el.getBoundingClientRect();
      const b = box(r);
      const name = url.split('?')[0].split('/').pop().replace(/\.[a-z0-9]+$/i, '');
      if (b && name && !clipped(el, r)) images.push({ key: name.toLowerCase(), ...b });
    }
    for (const n of el.childNodes) {
      // 3: a text node (their client defines its own global `Node`).
      if (n.nodeType !== 3) continue;
      const text = n.textContent.replace(/\s+/g, ' ').trim();
      if (!text) continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      const r = range.getBoundingClientRect();
      const b = box(r);
      if (!b || clipped(el, r)) continue;
      texts.push({ key: text.toLowerCase(), text, size: parseFloat(cs.fontSize), ...b });
    }
  }
  // A run drawn twice in one place (their outlined text has a copy under it) is seen once.
  const once = (list) => {
    const seen = new Set();
    return list.filter((a) => {
      const k = [a.key, Math.round(a.x), Math.round(a.y), Math.round(a.w), Math.round(a.h)].join();
      return !seen.has(k) && seen.add(k);
    });
  };
  return { width: W, height: H, images: once(images), texts: once(texts), ignored };
}

/// Resolves once the page has stopped changing for `quiet` ms (or after `limit`).
export function settle({ quiet, limit }) {
  return new Promise((resolve) => {
    let timer;
    const done = () => {
      observer.disconnect();
      resolve();
    };
    const observer = new MutationObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(done, quiet);
    });
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
    timer = setTimeout(done, quiet);
    setTimeout(done, limit);
  });
}
