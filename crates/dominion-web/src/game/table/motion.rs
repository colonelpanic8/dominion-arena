//! Cards seen moving, as dominion.games moves them. Just before a new view is applied
//! the page notes where every zone and card is ([`before_change`]); once the table shows
//! the new view, the moves [inferred](crate::game::moves) between the two boards play
//! as ghost cards, each in the way its destination shows arrivals: a played card flies
//! to its slot, a drawn card fades in at its slot in the hand, a bought or gained card
//! rises from its pile and fades, a discarded card sinks out of the hand or slides off
//! the play row, a trashed card tilts and drops. Speed and pauses follow the Animation
//! Type setting's numbers for each kind of move; on the two slowest, moves along
//! different routes wait for each other. Ghosts ignore the pointer.

use super::super::moves::{self, Ghost, Loc};
use crate::app::Settings;
use crate::card::{CardBack, CardSize, CardView};
use crate::game::GameCtx;
use crate::game::board::Board;
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::api::AnimationSpeed;
use std::cell::RefCell;
use std::rc::Rc;

/// More moves than this arriving together are summarised, one ghost per pair of places.
const MAX_GHOSTS: usize = 16;

/// How a kind of move is shown: highlighted (held longer, blocking what follows), normal
/// or minimal (a short fade near the destination); its speed and pause in ms.
type Style = (&'static str, u32, u32);

/// The kinds of moves, as dominion.games classes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Your cards changing hands outside your turn (an attack's discard, a Curse).
    YoursOffTurn = 0,
    /// Cards changing hands that aren't yours (an opponent's gain or trash).
    TheirsChanging = 1,
    /// Your own gains.
    Gain = 4,
    Regular = 5,
}

/// Each preset's style for each [`Class`] (by its number), and whether moves along
/// different routes may overlap.
fn preset(speed: AnimationSpeed) -> Option<([Style; 7], bool)> {
    const H: &str = "h";
    const N: &str = "n";
    const M: &str = "m";
    Some(match speed {
        AnimationSpeed::Slow => (
            [
                (H, 350, 800),
                (N, 350, 800),
                (H, 300, 400),
                (H, 350, 0),
                (N, 300, 400),
                (N, 300, 200),
                (N, 200, 0),
            ],
            false,
        ),
        AnimationSpeed::Standard => (
            [
                (H, 350, 750),
                (N, 350, 750),
                (N, 300, 400),
                (H, 350, 0),
                (N, 300, 200),
                (N, 300, 200),
                (N, 200, 0),
            ],
            false,
        ),
        AnimationSpeed::Sprightly => (
            [
                (H, 350, 500),
                (N, 350, 500),
                (N, 300, 0),
                (N, 350, 0),
                (N, 300, 0),
                (N, 300, 150),
                (N, 200, 0),
            ],
            true,
        ),
        AnimationSpeed::Swift => (
            [
                (H, 300, 450),
                (N, 300, 450),
                (N, 200, 0),
                (N, 300, 0),
                (N, 200, 0),
                (M, 250, 150),
                (M, 200, 0),
            ],
            true,
        ),
        AnimationSpeed::Speedy => (
            [
                (H, 300, 300),
                (M, 300, 300),
                (M, 200, 0),
                (M, 300, 0),
                (M, 200, 0),
                (M, 200, 100),
                (M, 200, 0),
            ],
            true,
        ),
        AnimationSpeed::None => return None,
    })
}

fn class_of(g: &Ghost, you: Option<u8>, your_turn: bool) -> Class {
    let yours = |l: &Loc| you.is_some() && l.owner() == you;
    let from_pile = matches!(g.from, Loc::Pile(_));
    if your_turn {
        if from_pile {
            Class::Gain
        } else {
            Class::Regular
        }
    } else if yours(&g.from) || yours(&g.to) {
        Class::YoursOffTurn
    } else if from_pile || g.to == Loc::Trash {
        Class::TheirsChanging
    } else {
        Class::Regular
    }
}

const ANIM_JS: &str = r#"
(() => {
  const A = window.__dtAnim = {};
  const W = 200, RATIO = 994 / 630;
  const SMOOTH = 'cubic-bezier(.25,.25,.25,1)', EASE_IN = 'cubic-bezier(0,.3,.7,1)';
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
    for (const el of document.querySelectorAll('.dt-hand > .dt-slot, .dt-play > .dt-slot')) {
      slots.set(el, el.getBoundingClientRect());
    }
    A.snap = { anchors, slots };
  };
  const kind = k => (k || '').split(':')[0];
  const seat = k => { const m = /^[a-z]+:(\d+)/.exec(k); return m ? m[1] : null; };
  const now = k => document.querySelector(`[data-anchor="${esc(k)}"]`)
    || (seat(k) != null ? document.querySelector(`[data-anchor="player:${seat(k)}"]`) : null);
  const fit = r => Math.min(r.width, r.height / RATIO);
  // The transform drawing the ghost (W wide, scaled to w) centred on r, moved by dx, dy.
  const at = (r, w, dx = 0, dy = 0, extra = '') => {
    const s = w / W;
    return `translate(${r.left + r.width / 2 - W * s / 2 + dx}px, ${r.top + r.height / 2 - W * RATIO * s / 2 + dy}px) scale(${s}) ${extra}`;
  };
  // Keyframes and duration for one move, by where it goes and comes from.
  const script = (g, st, r0, w0, r1, w1, opp) => {
    const [type, S, D] = st;
    const h0 = w0 * RATIO;
    const from = kind(g.from), to = kind(g.to);
    if (type === 'm' && r1) {
      const mx = r0.left + (r1.left - r0.left) * 0.8, my = r0.top + (r1.top - r0.top) * 0.8;
      const mid = { left: mx, top: my, width: r1.width, height: r1.height };
      return { frames: [{ transform: at(mid, w1), opacity: 0, easing: SMOOTH }, { transform: at(r1, w1), opacity: 1 }], ms: S, landed: true };
    }
    if (to === 'hand' && r1) {
      const h1 = w1 * RATIO;
      const dy = opp ? -0.4 * h1 : 0.4 * h1;
      return { frames: [{ transform: at(r1, w1, -0.2 * w1, dy), opacity: 0, easing: SMOOTH }, { transform: at(r1, w1), opacity: 1 }], ms: S + 0.2 * D, landed: true };
    }
    if (from === 'pile') {
      if (type === 'h') {
        return { frames: [
          { transform: at(r0, w0), opacity: 1, easing: SMOOTH },
          { transform: at(r0, w0 * 1.2, 0, -0.4 * h0), opacity: 1, offset: 0.25 },
          { transform: at(r0, w0 * 1.2, 0, -0.4 * h0), opacity: 1, offset: 0.75, easing: SMOOTH },
          { transform: at(r0, w0 * 1.2, 0, -0.6 * h0), opacity: 0 },
        ], ms: 0.8 * S + D + S };
      }
      const t1 = 0.8 * S, t2 = 0.5 * S;
      return { frames: [
        { transform: at(r0, w0), opacity: 1, easing: SMOOTH },
        { transform: at(r0, w0 * 1.1, 0, -0.4 * h0), opacity: 1, offset: t1 / (t1 + t2), easing: SMOOTH },
        { transform: at(r0, w0 * 1.1, 0, -0.6 * h0), opacity: 0 },
      ], ms: t1 + t2 };
    }
    if (to === 'trash') {
      const a = (Math.random() * 40 - 20).toFixed(1);
      const t1 = 0.8 * S, t2 = 1.2 * S;
      return { frames: [
        { transform: at(r0, w0), opacity: 1, easing: SMOOTH },
        { transform: at(r0, w0, 0, 0.1 * h0, `rotate(${a}deg)`), opacity: 1, offset: t1 / (t1 + t2), easing: EASE_IN },
        { transform: at(r0, w0, 0, 0.3 * h0, `rotate(${a / 2}deg)`), opacity: 0 },
      ], ms: t1 + t2 };
    }
    if (to === 'discard' && from === 'hand') {
      return { frames: [
        { transform: at(r0, w0), opacity: 1, easing: EASE_IN },
        { transform: at(r0, w0 * 0.6, 0, 0.9 * h0), opacity: 0 },
      ], ms: 0.7 * S };
    }
    if (to === 'discard' && from === 'inplay') {
      return { frames: [
        { transform: at(r0, w0), opacity: 1, easing: EASE_IN },
        { transform: at(r0, w0, -0.3 * w0, 0), opacity: 0 },
      ], ms: 0.7 * S };
    }
    if (!r1 || to === 'deck' || to === 'zone') {
      return { frames: [
        { transform: at(r0, w0), opacity: 1, easing: SMOOTH },
        { transform: at(r0, w0, 0, -0.6 * h0), opacity: 0 },
      ], ms: S };
    }
    const spin = opp && from === 'hand' ? 'rotate(180deg)' : '';
    const fly = 1.5 * S, hold = (type === 'h' ? 0.5 : 0.2) * D;
    return { frames: [
      { transform: at(r0, w0, 0, 0, spin), opacity: 1, easing: SMOOTH },
      { transform: at(r1, w1), opacity: 1, offset: fly / (fly + hold) },
      { transform: at(r1, w1), opacity: 1 },
    ], ms: fly + hold, landed: true };
  };
  A.play = (ghosts, styles, concurrent, you) => {
    const snap = A.snap;
    A.snap = null;
    if (!snap || matchMedia('(prefers-reduced-motion: reduce)').matches || !styles) return;
    const relayout = styles[6][1];
    for (const [el, r0] of snap.slots) {
      if (!el.isConnected) continue;
      const r1 = el.getBoundingClientRect();
      const dx = r0.left - r1.left, dy = r0.top - r1.top;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
      el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration: relayout, easing: SMOOTH });
    }
    const pool = document.getElementById('dt-ghost-pool');
    if (!pool) return;
    // Moves along one route start together; on the slow presets a new route waits for
    // the last to end, and a highlighted move holds up everything after it.
    let clock = 0, groupEnd = 0, route = null, blockEnd = 0;
    ghosts.forEach(g => {
      const st = styles[g.cls];
      const src = snap.anchors[g.from] || (seat(g.from) != null && snap.anchors['player:' + seat(g.from)]);
      const dest = now(g.to);
      if (!src) return;
      const r0 = (g.card && src.cards[g.card]) || src.r;
      const w0 = g.card && src.cards[g.card] ? r0.width : fit(r0);
      const sel = g.card ? `.dt-c[data-name="${esc(g.card)}"]` : null;
      const destCard = dest && sel ? (dest.matches(sel) ? dest : dest.querySelector(sel)) : null;
      const r1raw = dest ? (destCard || dest).getBoundingClientRect() : null;
      const r1 = r1raw && r1raw.width > 0 && r1raw.bottom > 0 && r1raw.top < innerHeight ? r1raw : null;
      const w1 = r1 ? (destCard ? r1.width : fit(r1)) : w0;
      if (!(w0 > 0)) return;
      const opp = seat(g.to) != null && seat(g.to) !== String(you) && kind(g.to) === 'hand'
        || seat(g.from) != null && seat(g.from) !== String(you) && kind(g.from) === 'hand';
      const sc = script(g, st, r0, w0, r1, w1, opp);
      const key = g.from + '>' + g.to;
      let start;
      if (concurrent) start = blockEnd;
      else if (key === route) start = clock;
      else { start = Math.max(groupEnd, blockEnd); clock = start; route = key; }
      groupEnd = Math.max(groupEnd, start + sc.ms);
      if (st[0] === 'h') blockEnd = Math.max(blockEnd, start + sc.ms);
      const face = g.card ? pool.querySelector(`[data-ghost="${esc(g.card)}"]`) : pool.querySelector('[data-ghost-back]');
      if (!face) return;
      const ghost = face.cloneNode(true);
      ghost.removeAttribute('data-ghost');
      ghost.removeAttribute('data-ghost-back');
      ghost.classList.add('dt-ghost');
      ghost.style.cssText = `position:fixed;left:0;top:0;width:${W}px;margin:0;z-index:12000;pointer-events:none;transform-origin:0 0;`;
      if (g.count > 1) {
        const n = document.createElement('span');
        n.className = 'dt-ghost-n';
        n.textContent = '×' + g.count;
        ghost.appendChild(n);
      }
      (document.querySelector('.dt-root') || document.body).appendChild(ghost);
      const a = ghost.animate(sc.frames, { duration: Math.max(1, sc.ms), delay: start, fill: 'both' });
      a.onfinish = a.oncancel = () => ghost.remove();
      const was = snap.anchors[g.to];
      if (sc.landed && destCard && !(was && was.cards[g.card])) {
        destCard.animate([{ opacity: 0 }, { opacity: 0, offset: 0.99 }, { opacity: 1 }], { duration: start + sc.ms });
      }
    });
  };
})();
"#;

/// Notes where everything is on the table, before a new view changes it.
pub fn before_change() {
    platform::run_js("window.__dtAnim && window.__dtAnim.snapshot()");
}

/// The ghosts of one change and how they move.
#[derive(Clone, Debug, PartialEq)]
pub struct Flight {
    pub seq: u64,
    pub ghosts: Vec<(Ghost, Class)>,
    pub speed: AnimationSpeed,
    pub you: Option<u8>,
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
            && s.web_animation_speed != AnimationSpeed::None
        {
            let inf = moves::infer(old, new, &st.log[l.log_len..]);
            if !inf.moves.is_empty() {
                let you = new.you.as_ref().map(|y| y.seat);
                let your_turn = new.status.your_turn || old.status.your_turn;
                let ghosts = moves::ghosts(&inf.moves, MAX_GHOSTS)
                    .into_iter()
                    .map(|g| {
                        let c = class_of(&g, you, your_turn);
                        (g, c)
                    })
                    .collect();
                let seq = l.seq + 1;
                l.seq = seq;
                flight.set(Some(Flight {
                    seq,
                    ghosts,
                    speed: s.web_animation_speed,
                    you,
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
        let Some((styles, concurrent)) = preset(f.speed) else {
            return;
        };
        let ghosts: Vec<serde_json::Value> = f
            .ghosts
            .iter()
            .map(|(g, c)| {
                serde_json::json!({
                    "card": g.card,
                    "from": g.from.anchor(),
                    "to": g.to.anchor(),
                    "count": g.count,
                    "cls": *c as u8,
                })
            })
            .collect();
        let styles: Vec<serde_json::Value> = styles
            .iter()
            .map(|(t, s, d)| serde_json::json!([t, s, d]))
            .collect();
        let json = serde_json::Value::Array(ghosts).to_string();
        let styles = serde_json::Value::Array(styles).to_string();
        let you = f.you.map_or("null".to_string(), |y| y.to_string());
        platform::run_js(&format!(
            "window.__dtAnim && window.__dtAnim.play({json}, {styles}, {concurrent}, {you})"
        ));
    });
    flight
}

/// The faces ghosts are cloned from, out of sight.
#[component]
pub fn GhostPool(flight: Option<Flight>) -> Element {
    let mut names: Vec<String> = flight
        .iter()
        .flat_map(|f| f.ghosts.iter().filter_map(|(g, _)| g.card.clone()))
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
