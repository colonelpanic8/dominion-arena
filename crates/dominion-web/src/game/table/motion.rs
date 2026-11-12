//! Cards seen moving. Just before a new view is applied the page notes where every
//! zone and card is ([`before_change`]); once the table shows the new view, the moves
//! [inferred](crate::game::moves) between the two boards fly as ghost cards from where
//! each card was to where it is, and cards that stay in the hand or in play slide to
//! their new places. Ghosts ignore the pointer, so the table takes clicks throughout.

use super::super::moves::{self, Ghost};
use crate::app::Settings;
use crate::card::{CardBack, CardSize, CardView};
use crate::game::GameCtx;
use crate::game::board::Board;
use crate::platform;
use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// More moves than this arriving together are summarised, one ghost per pair of places.
const MAX_GHOSTS: usize = 12;

const ANIM_JS: &str = r#"
(() => {
  const A = window.__dtAnim = {};
  const W = 200, RATIO = 994 / 630;
  const esc = s => CSS.escape(s);
  A.snapshot = () => {
    if (!document.querySelector('.dt-root')) { A.snap = null; return; }
    const anchors = {};
    for (const el of document.querySelectorAll('[data-anchor]')) {
      const k = el.dataset.anchor;
      if (anchors[k]) continue;
      const cards = {};
      const named = [...el.querySelectorAll('.dt-c[data-name]')];
      if (el.matches('.dt-c[data-name]')) named.unshift(el);
      for (const c of named) {
        const n = c.dataset.name;
        if (n && !cards[n]) cards[n] = c.getBoundingClientRect();
      }
      anchors[k] = { r: el.getBoundingClientRect(), cards };
    }
    const slots = new Map();
    for (const el of document.querySelectorAll('.dt-hand > .dt-slot, .dt-play-row > .dt-slot')) {
      slots.set(el, el.getBoundingClientRect());
    }
    A.snap = { anchors, slots };
  };
  const seat = k => { const m = /^[a-z]+:(\d+)/.exec(k); return m ? m[1] : null; };
  const now = k => document.querySelector(`[data-anchor="${esc(k)}"]`)
    || (seat(k) != null ? document.querySelector(`[data-anchor="player:${seat(k)}"]`) : null);
  // The width of a card filling a box (a pile, a hand of backs, a panel).
  const fit = r => Math.min(r.width, r.height / RATIO);
  A.play = (ghosts, ms) => {
    const snap = A.snap;
    A.snap = null;
    if (!snap || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const ease = 'cubic-bezier(.25,.75,.35,1)';
    for (const [el, r0] of snap.slots) {
      if (!el.isConnected) continue;
      const r1 = el.getBoundingClientRect();
      const dx = r0.left - r1.left, dy = r0.top - r1.top;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
      el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration: ms, easing: ease });
    }
    const pool = document.getElementById('dt-ghost-pool');
    if (!pool) return;
    const stagger = ghosts.length > 1 ? Math.max(0, Math.min(90, (700 - ms) / (ghosts.length - 1))) : 0;
    ghosts.forEach((g, i) => {
      const src = snap.anchors[g.from] || (seat(g.from) != null && snap.anchors['player:' + seat(g.from)]);
      const dest = now(g.to);
      if (!src || !dest) return;
      const r0 = (g.card && src.cards[g.card]) || src.r;
      const w0 = g.card && src.cards[g.card] ? r0.width : fit(r0);
      const sel = g.card ? `.dt-c[data-name="${esc(g.card)}"]` : null;
      const destCard = sel ? (dest.matches(sel) ? dest : dest.querySelector(sel)) : null;
      const r1 = (destCard || dest).getBoundingClientRect();
      const w1 = destCard ? r1.width : fit(r1);
      if (!(w0 > 0 && w1 > 0 && r1.width > 0)) return;
      if (window.__dtAnimDebug) console.log('GHOST', JSON.stringify({ g, r0: [r0.left, r0.top, r0.width, r0.height], w0, r1: [r1.left, r1.top, r1.width, r1.height], w1, destCard: !!destCard }));
      const face = g.card ? pool.querySelector(`[data-ghost="${esc(g.card)}"]`) : pool.querySelector('[data-ghost-back]');
      if (!face) return;
      const ghost = face.cloneNode(true);
      ghost.removeAttribute('data-ghost');
      ghost.removeAttribute('data-ghost-back');
      ghost.classList.add('dt-ghost');
      ghost.style.cssText = `position:fixed;left:0;top:0;width:${W}px;margin:0;z-index:900;pointer-events:none;transform-origin:0 0;`;
      if (g.count > 1) {
        const n = document.createElement('span');
        n.className = 'dt-ghost-n';
        n.textContent = '×' + g.count;
        ghost.appendChild(n);
      }
      const at = (r, w) => {
        const s = w / W;
        return `translate(${r.left + r.width / 2 - W * s / 2}px, ${r.top + r.height / 2 - W * RATIO * s / 2}px) scale(${s})`;
      };
      (document.querySelector('.dt-root') || document.body).appendChild(ghost);
      const delay = i * stagger;
      // The flight eases; the fade once it has landed doesn't.
      const a = ghost.animate([
        { transform: at(r0, w0), opacity: 1, easing: ease },
        { transform: at(r1, w1), opacity: 1, offset: 0.85 },
        { transform: at(r1, w1), opacity: 0 },
      ], { duration: ms, delay, fill: 'both' });
      a.onfinish = a.oncancel = () => ghost.remove();
      const was = snap.anchors[g.to];
      if (destCard && !(was && was.cards[g.card])) {
        destCard.animate([{ opacity: 0 }, { opacity: 0, offset: 0.99 }, { opacity: 1 }], { duration: delay + ms * 0.85 });
      }
    });
  };
})();
"#;

/// Notes where everything is on the table, before a new view changes it.
pub fn before_change() {
    platform::run_js("window.__dtAnim && window.__dtAnim.snapshot()");
}

/// The ghosts of one change, with the time each takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Flight {
    pub seq: u64,
    pub ghosts: Vec<Ghost>,
    pub ms: u32,
}

#[derive(Default)]
struct Last {
    board: Option<Rc<Board>>,
    log_len: usize,
    seat: Option<u8>,
    seq: u64,
}

/// Animates the table's changes; the flight is for [`GhostPool`] to draw its cards.
pub fn use_motion(board: Memo<Option<Rc<Board>>>, game: GameCtx) -> Signal<Option<Flight>> {
    let settings = use_context::<Settings>().0;
    use_hook(|| platform::run_js(ANIM_JS));
    let mut flight = use_signal(|| None::<Flight>);
    let last = use_hook(|| Rc::new(RefCell::new(Last::default())));
    let last_a = last.clone();
    use_effect(move || {
        let b = board.read().clone();
        let st = game.state.peek();
        let seat = st.view.as_ref().and_then(|v| v.seat);
        let log_len = st.log.len();
        let mut l = last_a.borrow_mut();
        let s = settings.peek();
        if let (Some(old), Some(new)) = (&l.board, &b)
            && l.seat == seat
            && log_len > l.log_len
            && s.web_animations
        {
            let inf = moves::infer(old, new, &st.log[l.log_len..]);
            if !inf.moves.is_empty() {
                l.seq += 1;
                flight.set(Some(Flight {
                    seq: l.seq,
                    ghosts: moves::ghosts(&inf.moves, MAX_GHOSTS),
                    ms: s.web_animation_ms.max(1),
                }));
            }
        }
        l.board = b;
        l.log_len = log_len;
        l.seat = seat;
    });
    use_effect(move || {
        let Some(f) = flight.read().clone() else {
            return;
        };
        let ghosts: Vec<serde_json::Value> = f
            .ghosts
            .iter()
            .map(|g| {
                serde_json::json!({
                    "card": g.card,
                    "from": g.from.anchor(),
                    "to": g.to.anchor(),
                    "count": g.count,
                })
            })
            .collect();
        let json = serde_json::Value::Array(ghosts).to_string();
        platform::run_js(&format!(
            "window.__dtAnim && window.__dtAnim.play({json}, {})",
            f.ms
        ));
    });
    flight
}

/// The faces ghosts are cloned from, out of sight.
#[component]
pub fn GhostPool(flight: Option<Flight>) -> Element {
    let mut names: Vec<String> = flight
        .iter()
        .flat_map(|f| f.ghosts.iter().filter_map(|g| g.card.clone()))
        .collect();
    names.sort();
    names.dedup();
    rsx! {
        div { id: "dt-ghost-pool", "aria-hidden": "true",
            div { class: "dt-c dt-full", "data-ghost-back": "1",
                CardBack { size: CardSize::Full }
            }
            for n in names.iter() {
                div { key: "{n}", class: "dt-c dt-full", "data-ghost": "{n}",
                    CardView { name: n.clone(), size: CardSize::Full }
                }
            }
        }
    }
}
