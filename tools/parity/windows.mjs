// Clicks made in both clients before the screenshot (a scenario's `show`): opening a
// window, studying a card, picking cards for a decision. Each is found on our page by
// what it is; on theirs a card is found by its name, anything else at the place ours is.
//   { study: 'Village' }      right-click a card (the study view)
//   { click: 'Copper', nth }  click a card (the nth of that name, from 0; with `hand`,
//                             of the largest cards: the hand's)
//   { tab: 'Trash' }          click a tab on the table's edge (trash, a mat)
//   { at: '.dt-selector' }    click what is at our element's place, in both clients
//   { button: 'Ok' }          click the visible button reading this (the game's end notice)
export async function show(ours, theirs, actions) {
  for (const a of actions) {
    const pages = [
      [ours, 'ours'],
      [theirs, 'theirs'],
    ];
    if (a.study || a.click) {
      const button = a.study ? 'right' : 'left';
      for (const [page, side] of pages) await card(page, side, a.study ?? a.click, a.nth ?? 0, button, a.hand);
    } else if (a.button) {
      for (const [page, side] of pages) await button(page, side, a.button);
    } else if (a.tab || a.at) {
      // Found on our page before either is clicked: a click can take it away.
      const p = await place(ours, a.tab ? `.dt-tab[data-tab="${a.tab}"]` : a.at);
      for (const [page] of pages) {
        const vp = page.viewportSize();
        await page.mouse.click(Math.round(p.x * vp.width), Math.round(p.y * vp.height));
      }
    } else throw new Error(`unknown show action ${JSON.stringify(a)}`);
    await ours.waitForTimeout(400);
  }
}

async function button(page, side, text) {
  for (let i = 0; i < 50; i++) {
    const clicked = await page.evaluate((text) => {
      const el = [...document.querySelectorAll('button')].find(
        (e) => e.textContent.trim() === text && e.checkVisibility({ opacityProperty: true, visibilityProperty: true }),
      );
      el?.click();
      return !!el;
    }, text);
    if (clicked) return;
    await page.waitForTimeout(200);
  }
  throw new Error(`show: no button ${text} on ${side}`);
}

/// Where our page has `selector`, as fractions of the viewport.
async function place(page, selector) {
  for (let i = 0; i < 50; i++) {
    const r = await page.evaluate((sel) => {
      const el = [...document.querySelectorAll(sel)].find((e) => e.checkVisibility());
      if (!el) return null;
      const b = el.getBoundingClientRect();
      return { x: (b.left + b.width / 2) / innerWidth, y: (b.top + b.height / 2) / innerHeight };
    }, selector);
    if (r) return r;
    await page.waitForTimeout(200);
  }
  const tabs = await page.evaluate(() => [...document.querySelectorAll('[data-tab]')].map((e) => e.dataset.tab));
  throw new Error(`show: nothing on our page matches ${selector} (tabs: ${tabs.join(' | ')})`);
}

async function card(page, side, name, nth, button, hand = false) {
  const box = await page.evaluate(
    ({ side, name, nth, hand }) => {
      let els;
      if (side === 'ours') {
        els = [...document.querySelectorAll('.dt-c')].filter((e) => e.dataset.name === name);
      } else {
        // A card stack: the fixed box holding a name layer that reads `name`.
        els = [...document.querySelectorAll('.name-layer')]
          .filter((e) => e.textContent.trim() === name)
          .map((e) => e.parentElement);
      }
      els = els.filter((e) => e.checkVisibility({ opacityProperty: true, visibilityProperty: true }));
      // Left to right, then top to bottom, as a hand reads.
      let boxes = els.map((e) => e.getBoundingClientRect()).filter((b) => b.width > 4 && b.height > 4);
      // The hand's cards are the largest on the table.
      const tallest = Math.max(...boxes.map((b) => b.height));
      if (hand) boxes = boxes.filter((b) => b.height >= 0.9 * tallest);
      boxes.sort((a, b) => a.top - b.top || a.left - b.left);
      const b = boxes[nth];
      return b && { x: b.left + b.width / 2, y: b.top + b.height / 2 };
    },
    { side, name, nth, hand },
  );
  if (!box) throw new Error(`show: no card ${name} (${nth}) on ${side}`);
  await page.mouse.click(Math.round(box.x), Math.round(box.y), { button });
}
