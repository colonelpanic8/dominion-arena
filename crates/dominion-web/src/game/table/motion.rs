//! Cards seen moving, as dominion.games moves them. Just before a new view is applied
//! the page notes where every zone and card is ([`before_change`]); once the table shows
//! the new view, the moves [inferred](crate::game::moves) between the two boards play
//! as ghost cards, each in the way its destination shows arrivals: a played card flies
//! to its slot, a drawn card fades in at its slot in the hand, a bought or gained card
//! rises from its pile and fades, a discarded card sinks out of the hand or slides off
//! the play row, a trashed card tilts and drops, a card looked at turns over into the
//! window showing it. Speed and pauses follow the Animation Type setting's numbers for
//! each kind of move, and moves queue across changes as dominion.games queues them: on
//! the two slowest, moves along different routes wait for each other. Shuffles aren't
//! shown. Ghosts ignore the pointer.

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
const MAX_GHOSTS: usize = 60;

/// How a kind of move is shown: highlighted (held longer, blocking what follows), normal
/// or minimal (a short fade near the destination); its speed and pause in ms.
type Style = (&'static str, u32, u32);

/// The kinds of moves, as dominion.games classes them. Cards change hands when they are
/// gained, trashed or returned to a pile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A player's cards changing hands outside their turn (a Curse from a Witch).
    OffTurn = 0,
    /// Another player's cards changing hands on their turn.
    TheirsChanging = 1,
    /// Your cards changing hands on your turn, other than gains (a trash).
    Changing = 2,
    /// Cards taken off a deck to look at or reveal.
    Reveal = 3,
    /// Your gains on your turn.
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

fn class_of(g: &Ghost, you: Option<u8>) -> Class {
    if matches!(&g.to, Loc::Mat(_, k) if k == "looking") {
        return Class::Reveal;
    }
    let changing = |l: &Loc| matches!(l, Loc::Pile(_) | Loc::Trash);
    if !changing(&g.from) && !changing(&g.to) {
        return Class::Regular;
    }
    let owner = g.to.owner().or(g.from.owner());
    if owner.is_some() && owner != g.turn {
        Class::OffTurn
    } else if owner != you {
        Class::TheirsChanging
    } else if matches!(g.from, Loc::Pile(_)) {
        Class::Gain
    } else {
        Class::Changing
    }
}

const ANIM_JS: &str = r#"
(() => {
  const A = window.__dtAnim = { q: [] };
  const W = 200, RATIO = 994 / 630;
  const SMOOTH = 'cubic-bezier(.25,.25,.25,1)', EASE_IN = 'cubic-bezier(0,.3,.7,1)';
  const esc = s => CSS.escape(s);
  const isWin = el => el.classList.contains('dt-window');
  A.snapshot = () => {
    if (!document.querySelector('.dt-root')) { A.snap = null; return; }
    const anchors = {};
    for (const el of document.querySelectorAll('[data-anchor]')) {
      const k = el.dataset.anchor;
      // An open window showing a zone stands for it rather than its tab.
      if (anchors[k] && (anchors[k].win || !isWin(el))) continue;
      const cards = {};
      let last = null;
      const named = [...el.querySelectorAll('.dt-c[data-name]')];
      if (el.matches('.dt-c[data-name]')) named.unshift(el);
      for (const c of named) {
        const n = c.dataset.name;
        const r = c.getBoundingClientRect();
        if (n) (cards[n] = cards[n] || []).push(r);
        if (!last || r.right > last.right) last = r;
      }
      anchors[k] = { r: el.getBoundingClientRect(), cards, last, win: isWin(el) };
    }
    const slots = new Map();
    for (const el of document.querySelectorAll('.dt-hand > .dt-slot, .dt-play > .dt-slot, .dt-c[data-anchor^="pile:"]')) {
      slots.set(el, el.getBoundingClientRect());
    }
    A.snap = { anchors, slots };
  };
  const kind = k => (k || '').split(':')[0];
  const seat = k => { const m = /^[a-z]+:(\d+)/.exec(k); return m ? m[1] : null; };
  const now = k => {
    const els = [...document.querySelectorAll(`[data-anchor="${esc(k)}"]`)];
    return els.find(isWin) || els[0]
      || (seat(k) != null ? document.querySelector(`[data-anchor="player:${seat(k)}"]`) : null);
  };
  const fit = r => Math.min(r.width, r.height / RATIO);
  // The place half a card after r, as a play row adds a card.
  const next = r => new DOMRect(r.left + r.width * 0.5, r.top, r.width, r.height);
  // The transform drawing the ghost (W wide, scaled to w about its centre) centred on r,
  // moved by dx, dy.
  const at = (r, w, dx = 0, dy = 0, extra = '') => {
    const s = w / W;
    return `translate(${r.left + r.width / 2 - W / 2 + dx}px, ${r.top + r.height / 2 - W * RATIO / 2 + dy}px) scale(${s}) ${extra}`;
  };
  const flat = 'rotate(0deg) rotateY(0deg)';
  const edge = 'rotate(-10deg) rotateY(90deg)';
  // Keyframes and duration for one move, by where it goes and comes from. `win` is the
  // open window it arrives in (the i-th of n arriving there), `back` that it is drawn
  // face down until it turns over (at `faceAt` ms).
  const script = (g, st, r0, w0, r1, w1, opp, win) => {
    const [type, S, D] = st;
    const h0 = w0 * RATIO;
    const from = kind(g.from), to = kind(g.to);
    if (win && r1) {
      if (type === 'h' && g.card) {
        const d = win.i * D * 0.3 / Math.max(1, 0.3 * win.n);
        const t1 = 0.6 * S, t2 = 1.2 * S, ms = d + t1 + t2 + D;
        return { frames: [
          { transform: at(r0, w0, 0, 0, flat), offset: 0 },
          { transform: at(r0, w0, 0, 0, flat), offset: d / ms, easing: SMOOTH },
          { transform: at(r1, w1, -0.5 * w1, 0, edge), offset: (d + t1) / ms, easing: SMOOTH },
          { transform: at(r1, w1, 0, 0, flat), offset: (d + t1 + t2) / ms },
          { transform: at(r1, w1, 0, 0, flat), offset: 1 },
        ], ms, landed: true, faceAt: d + t1 };
      }
      if (type === 'n' && g.card) {
        return { frames: [
          { transform: at(r1, w1, -0.5 * w1, 0, edge), easing: SMOOTH },
          { transform: at(r1, w1, 0, 0, flat) },
        ], ms: S, landed: true };
      }
      const t1 = 0.6 * S, ms = t1 + D;
      return { frames: [
        { transform: at(r1, w1, -0.5 * w1), opacity: 0, easing: SMOOTH },
        { transform: at(r1, w1), opacity: 1, offset: t1 / ms },
        { transform: at(r1, w1), opacity: 1 },
      ], ms, landed: true };
    }
    if (to === 'pile' && type !== 'h') {
      return { frames: [
        { transform: at(r0, w0), opacity: 1, easing: EASE_IN },
        { transform: at(r0, w0 * 0.6, 0, 0.9 * h0), opacity: 0 },
      ], ms: 0.7 * S };
    }
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
        { transform: at(r0, w0, 0, 0, 'rotate(0deg)'), opacity: 1, easing: SMOOTH },
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
    if (to === 'discard') {
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
    const spin = opp && from === 'hand' ? 'rotate(180deg)' : 'rotate(0deg)';
    const fly = 1.5 * S, hold = (type === 'h' ? 0.5 : 0.2) * D;
    return { frames: [
      { transform: at(r0, w0, 0, 0, spin), opacity: 1, easing: SMOOTH },
      { transform: at(r1, w1, 0, 0, 'rotate(0deg)'), opacity: 1, offset: fly / (fly + hold) },
      { transform: at(r1, w1, 0, 0, 'rotate(0deg)'), opacity: 1 },
    ], ms: fly + hold, landed: true };
  };
  // When a move may start, as dominion.games' AnimationDirector queues them: after every
  // highlighted move before it; on the slow presets after every earlier move along
  // another route, on the others after earlier moves bringing cards to where it takes
  // one from; and never before an earlier move taking cards from where it brings one.
  const when = (t0, g, concurrent) => {
    const key = g.from + '>' + g.to;
    let t = t0;
    for (const m of A.q) {
      if (m.hl || (concurrent ? m.to === g.from : m.key !== key)) t = Math.max(t, m.end);
      if (m.from === g.to) t = Math.max(t, m.start);
    }
    return t;
  };
  // A card taken off a deck to look at that no window showed, and then moved on, is seen
  // going straight from the deck.
  const unlook = (ghosts, snap) => {
    const shown = k => (snap.anchors[k] && snap.anchors[k].win) || isWin(now(k) || document.body);
    const out = [];
    ghosts.forEach(g => {
      if (kind(g.from) === 'zone' && g.from.endsWith(':looking') && !shown(g.from)) {
        const i = out.findIndex(h => h.to === g.from && !h.on && (!h.card || !g.card || h.card === g.card));
        if (i >= 0) {
          const h = out.splice(i, 1)[0];
          out.push(Object.assign({}, g, { from: h.from, card: g.card || h.card, on: true }));
          return;
        }
      }
      out.push(g);
    });
    return out;
  };
  A.play = (all, styles, concurrent, you) => {
    const snap = A.snap;
    A.snap = null;
    if (!snap || matchMedia('(prefers-reduced-motion: reduce)').matches || !styles) return;
    const ghosts = unlook(all, snap);
    const t0 = performance.now();
    A.q = A.q.filter(m => m.end > t0);
    const relayout = styles[6][1];
    for (const [el, r0] of snap.slots) {
      if (!el.isConnected) continue;
      const r1 = el.getBoundingClientRect();
      if (!(r1.width > 0)) continue;
      const dx = r0.left + r0.width / 2 - r1.left - r1.width / 2, dy = r0.top + r0.height / 2 - r1.top - r1.height / 2;
      const sx = r0.width / r1.width, sy = r0.height / r1.height;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1 && Math.abs(sx - 1) < 0.01 && Math.abs(sy - 1) < 0.01) continue;
      el.animate([{ transform: `translate(${dx}px, ${dy}px) scale(${sx}, ${sy})` }, { transform: 'none' }], { duration: relayout, easing: SMOOTH });
    }
    const pool = document.getElementById('dt-ghost-pool');
    if (!pool) return;
    const into = {}, going = {};
    for (const g of ghosts) {
      into[g.to] = (into[g.to] || 0) + 1;
      if (g.card) going[g.from + '|' + g.card] = (going[g.from + '|' + g.card] || 0) + 1;
    }
    const seen = {}, left = {}, landed = {};
    let group = null;
    ghosts.forEach(g => {
      const st = styles[g.cls];
      const src = snap.anchors[g.from] || (seat(g.from) != null && snap.anchors['player:' + seat(g.from)]);
      const live = document.querySelector(`[data-anchor="${esc(g.to)}"]`);
      const dest = now(g.to);
      if (!src) return;
      // A card a play row didn't show yet comes from after its cards.
      const after = kind(g.from) === 'inplay' && src.last ? next(src.last) : null;
      // Copies of a card leave from each of their places in turn, and arrive at the last
      // ones of their name there.
      const copies = g.card && src.cards[g.card];
      const from1 = copies ? copies[(left[g.from + '|' + g.card] = (left[g.from + '|' + g.card] ?? -1) + 1) % copies.length] : null;
      const r0 = from1 || after || src.r;
      const w0 = from1 || after ? r0.width : fit(r0);
      const sel = g.card ? `.dt-c[data-name="${esc(g.card)}"]` : null;
      const there = dest && sel ? (dest.matches(sel) ? [dest] : [...dest.querySelectorAll(sel)]) : [];
      const k = g.card ? (landed[g.to + '|' + g.card] = (landed[g.to + '|' + g.card] ?? -1) + 1) : 0;
      const was = snap.anchors[g.to];
      // The copies there before that stay: the cards after them are new.
      const stay = Math.max(0, ((was && was.cards[g.card]) || []).length - (going[g.to + '|' + g.card] || 0));
      const fresh = there[stay + k];
      const destCard = fresh || there[there.length - 1] || null;
      // A play row no longer shown (the turn has passed): where it was, after its cards.
      const old = !live && snap.anchors[g.to];
      const r1raw = old
        ? (old.last ? next(old.last) : old.r)
        : dest ? (destCard || dest).getBoundingClientRect() : null;
      const r1 = r1raw && r1raw.width > 0 && r1raw.bottom > 0 && r1raw.top < innerHeight ? r1raw : null;
      const w1 = r1 ? (destCard || (old && old.last) ? r1.width : fit(r1)) : w0;
      if (!(w0 > 0)) return;
      const opp = seat(g.to) != null && seat(g.to) !== String(you) && kind(g.to) === 'hand'
        || seat(g.from) != null && seat(g.from) !== String(you) && kind(g.from) === 'hand';
      const i = seen[g.to] = (seen[g.to] ?? -1) + 1;
      const win = dest && isWin(dest) && destCard ? { i, n: into[g.to] } : null;
      const sc = script(g, st, r0, w0, r1, w1, opp, win);
      // The cards of one move start together.
      const key = g.from + '>' + g.to;
      const t = group && group.key === key ? group.t : when(t0, g, concurrent);
      group = { key, t };
      A.q.push({ key: g.from + '>' + g.to, from: g.from, to: g.to, start: t, end: t + sc.ms, hl: st[0] === 'h' });
      const start = t - t0;
      const face = g.card ? pool.querySelector(`[data-ghost="${esc(g.card)}"]`) : pool.querySelector('[data-ghost-back]');
      if (!face) return;
      const ghost = face.cloneNode(true);
      ghost.removeAttribute('data-ghost');
      ghost.removeAttribute('data-ghost-back');
      ghost.classList.add('dt-ghost');
      ghost.style.cssText = `position:fixed;left:0;top:0;width:${W}px;margin:0;z-index:12000;pointer-events:none;transform-origin:50% 50%;`;
      if (g.count > 1) {
        const n = document.createElement('span');
        n.className = 'dt-ghost-n';
        n.textContent = '×' + g.count;
        ghost.appendChild(n);
      }
      if (sc.faceAt != null) {
        const back = pool.querySelector('[data-ghost-back]');
        if (back) {
          const b = back.cloneNode(true);
          b.removeAttribute('data-ghost-back');
          b.style.cssText = 'position:absolute;inset:0;margin:0;';
          ghost.appendChild(b);
          const k = sc.faceAt / sc.ms;
          b.animate([{ opacity: 1 }, { opacity: 1, offset: k }, { opacity: 0, offset: Math.min(1, k + 0.001) }, { opacity: 0 }],
            { duration: Math.max(1, sc.ms), delay: start, fill: 'both' });
        }
      }
      (document.querySelector('.dt-root') || document.body).appendChild(ghost);
      const a = ghost.animate(sc.frames, { duration: Math.max(1, sc.ms), delay: start, fill: 'both' });
      a.onfinish = a.oncancel = () => ghost.remove();
      if (sc.landed && fresh) {
        fresh.animate([{ opacity: 0 }, { opacity: 0, offset: 0.99 }, { opacity: 1 }], { duration: start + sc.ms });
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
                let ghosts = moves::ghosts(&inf.moves, MAX_GHOSTS)
                    .into_iter()
                    .map(|g| {
                        let c = class_of(&g, you);
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
