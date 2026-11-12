//! The graphical table, laid out like dominion.games' game screen: opponents along the
//! top, the Supply and the play area in the middle, the status and prompt bar, your
//! cards at the bottom and the log on the right. It draws the [`board`](super::board)
//! model and answers through the [`interaction`](super::interaction) model: every
//! target of the open decision has one [`Spot`] where it glows and is clicked.

pub mod model;
mod pieces;
mod players;
mod prompt;
mod side;
mod supply;

use super::board::{Board, Stack, board};
use super::interaction::{Interaction, TreasureRun, interaction};
use super::state::{Selection, answers_on_click};
use super::use_game;
use crate::app::Cards;
use crate::assets::use_assets;
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::{AgentDecision, Answer};
pub use model::Spot;
use std::collections::HashMap;
use std::rc::Rc;

const CSS: &str = include_str!("../../../assets/table.css");

/// The open decision of the seat shown, placed on the table.
#[derive(Debug, PartialEq)]
pub struct Ui {
    pub decision: AgentDecision,
    pub inter: Interaction,
    pub spots: Vec<Spot>,
    pub by_spot: HashMap<Spot, usize>,
    pub click_answers: bool,
}

/// How a card on the table takes part in the open decision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mark {
    /// The index of the target clicked there.
    pub target: Option<usize>,
    /// How many options the target stands for.
    pub options: usize,
    /// The positions in the answer of the target's options picked so far.
    pub picked: Vec<usize>,
    pub ordered: bool,
}

impl Ui {
    pub fn mark_target(&self, sel: &Selection, t: usize) -> Mark {
        let options = &self.inter.targets[t].options;
        Mark {
            target: Some(t),
            options: options.len(),
            picked: sel.positions(options),
            ordered: self.inter.ordered,
        }
    }

    pub fn mark(&self, sel: &Selection, spot: &Spot) -> Mark {
        self.by_spot
            .get(spot)
            .map(|&t| self.mark_target(sel, t))
            .unwrap_or_default()
    }
}

/// The marks of the cards drawn at `spot`, or none when nothing is being decided.
pub fn mark_at(ui: &Option<Rc<Ui>>, sel: &Selection, spot: Spot) -> Mark {
    ui.as_ref().map(|u| u.mark(sel, &spot)).unwrap_or_default()
}

/// A selection with the decision it was made for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sel {
    pub id: String,
    pub sel: Selection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub shown: bool,
}

/// What an opened pile shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub title: String,
    pub stacks: Vec<Stack>,
    pub unknown: u32,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Run {
    run: TreasureRun,
    sent: Option<String>,
}

#[derive(Clone, Copy)]
pub struct TableCtx {
    pub ui: Memo<Option<Rc<Ui>>>,
    pub sel: Signal<Sel>,
    pub hover: Signal<Option<Hover>>,
    hover_seq: Signal<u64>,
    pub popup: Signal<Option<Popup>>,
    run: Signal<Option<Run>>,
    cards: Signal<Rc<crate::cards::CardDb>>,
    game: super::GameCtx,
}

pub fn use_table() -> TableCtx {
    use_context::<TableCtx>()
}

const HOVER_DELAY_MS: u32 = 400;

impl TableCtx {
    /// The selection for the open decision.
    pub fn selection(&self) -> Selection {
        let ui = self.ui.read();
        let s = self.sel.read();
        match ui.as_ref() {
            Some(u) if u.decision.id == s.id => s.sel.clone(),
            _ => Selection::default(),
        }
    }

    fn busy(&self) -> bool {
        self.game.state.peek().answering || self.run.peek().is_some()
    }

    fn submit(&self, a: Answer) {
        self.game.submit.call(a);
    }

    /// The selection to change, emptied first if it was for another decision.
    fn edit_sel(&self, id: &str) -> Selection {
        let s = self.sel.peek();
        if s.id == id {
            s.sel.clone()
        } else {
            Selection::default()
        }
    }

    /// A click on target `t`.
    pub fn act(&self, t: usize) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        if self.busy() {
            return;
        }
        let Some(target) = ui.inter.targets.get(t) else {
            return;
        };
        let d = &ui.decision;
        if ui.click_answers {
            if let Some(a) = Selection::new(vec![target.options[0]]).answer(d) {
                self.submit(a);
            }
            return;
        }
        let mut sel = self.edit_sel(&d.id);
        if !sel.click(d, &target.options) {
            sel.unclick(&target.options);
        }
        let mut s = self.sel;
        s.set(Sel {
            id: d.id.clone(),
            sel,
        });
    }

    /// Takes back the last pick of target `t`.
    pub fn unpick(&self, t: usize) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        let Some(target) = ui.inter.targets.get(t) else {
            return;
        };
        let mut sel = self.edit_sel(&ui.decision.id);
        sel.unclick(&target.options);
        let mut s = self.sel;
        s.set(Sel {
            id: ui.decision.id.clone(),
            sel,
        });
    }

    pub fn reorder(&self, from: usize, to: usize) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        let mut sel = self.edit_sel(&ui.decision.id);
        sel.reorder(from, to);
        let mut s = self.sel;
        s.set(Sel {
            id: ui.decision.id.clone(),
            sel,
        });
    }

    pub fn clear(&self) {
        let mut s = self.sel;
        s.set(Sel::default());
    }

    pub fn confirm(&self) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        if self.busy() {
            return;
        }
        if let Some(a) = self.edit_sel(&ui.decision.id).answer(&ui.decision) {
            self.submit(a);
        }
    }

    pub fn name(&self, card: &str) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        if self.busy() {
            return;
        }
        let cards = self.cards.peek().clone();
        if let Some(a) = super::state::name_answer(&ui.decision, &cards, card) {
            self.submit(a);
        }
    }

    /// Starts "Play Treasures".
    pub fn play_treasures(&self) {
        if self.busy() {
            return;
        }
        let game = self.game;
        let cards = self.cards.peek().clone();
        let run = game
            .state
            .peek()
            .view
            .as_ref()
            .and_then(|v| TreasureRun::start(v, &cards));
        if let Some(run) = run {
            let mut state = game.state;
            state.write().error = None;
            let mut r = self.run;
            r.set(Some(Run { run, sent: None }));
        }
    }

    /// Answers with the target whose option does `action` (a key for a button).
    pub fn press(&self, action: &str) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        if let Some(t) = ui.inter.targets.iter().position(|t| t.action == action) {
            self.act(t);
        }
    }

    pub fn hover_enter(&self, name: &str, x: f64, y: f64) {
        let mut seq = self.hover_seq;
        let n = *seq.peek() + 1;
        seq.set(n);
        let mut hover = self.hover;
        hover.set(Some(Hover {
            name: name.to_string(),
            x,
            y,
            shown: false,
        }));
        spawn(async move {
            platform::sleep(HOVER_DELAY_MS).await;
            if *seq.peek() == n
                && let Some(h) = hover.write().as_mut()
            {
                h.shown = true;
            }
        });
    }

    pub fn hover_now(&self, name: &str, x: f64, y: f64) {
        let mut seq = self.hover_seq;
        let n = *seq.peek() + 1;
        seq.set(n);
        let mut hover = self.hover;
        hover.set(Some(Hover {
            name: name.to_string(),
            x,
            y,
            shown: true,
        }));
    }

    pub fn hover_leave(&self) {
        let mut seq = self.hover_seq;
        let n = *seq.peek() + 1;
        seq.set(n);
        let mut hover = self.hover;
        if hover.peek().is_some() {
            hover.set(None);
        }
    }

    pub fn open(&self, p: Popup) {
        let mut popup = self.popup;
        let same = popup.peek().as_ref() == Some(&p);
        popup.set(if same { None } else { Some(p) });
    }

    pub fn close(&self) {
        let mut popup = self.popup;
        popup.set(None);
    }

    fn key(&self, key: &str) {
        match key {
            "Enter" => self.confirm(),
            "Escape" => {
                self.clear();
                self.close();
                self.hover_leave();
            }
            "e" | "E" => self.press("end-phase"),
            "t" | "T" => self.play_treasures(),
            _ => {}
        }
    }
}

const KEYS_JS: &str = r#"
if (window.__dtKeys) document.removeEventListener('keydown', window.__dtKeys);
window.__dtKeys = (e) => {
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  const t = e.target;
  const typing = t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT');
  if (typing && e.key !== 'Escape') return;
  if (!['Enter', 'Escape', 'e', 'E', 't', 'T'].includes(e.key)) return;
  if (!document.querySelector('.dt-root')) return;
  e.preventDefault();
  try { dioxus.send(e.key); } catch (_) {}
};
document.addEventListener('keydown', window.__dtKeys);
"#;

/// The images the table uses besides cards, as CSS variables when the cache has them.
fn table_css(assets: &crate::assets::AssetDb) -> String {
    let mut css = String::from(".dt-root{");
    for (name, path) in [
        ("btn", "images/materials/copper_button.jpg"),
        ("btn-gold", "images/materials/gold_button.jpg"),
        ("btn-stone", "images/materials/stone_button.png"),
        ("canvas", "images/materials/canvas_small.jpg"),
        ("divider", "images/materials/gold_divider.png"),
        ("trash-mat", "images/mats/trash_mat.png"),
        ("counter", "images/elements/card-counter.png"),
        ("pick", "images/elements/premove-counter.png"),
        ("treasures", "images/buttons/autoplay-treasures.png"),
    ] {
        if assets.has(path) {
            css.push_str(&format!("--dt-{name}:url('{}');", crate::assets::url(path)));
        }
    }
    css.push('}');
    css
}

fn view_mode_key() -> &'static str {
    "dominion.view"
}

/// Whether this browser shows the table (rather than the plain page) this session.
pub fn table_mode() -> bool {
    platform::session_get(view_mode_key()).as_deref() != Some("plain")
}

pub fn set_table_mode(on: bool) {
    platform::session_set(view_mode_key(), if on { "table" } else { "plain" });
}

/// The game page's table. `controls` (seat switching, the view toggle) go above the log.
#[component]
pub fn TableGame(controls: Element) -> Element {
    let game = use_game();
    let cards = use_context::<Cards>().0;
    let board_memo: Memo<Option<Rc<Board>>> = use_memo(move || {
        let st = game.state.read();
        let v = st.view.as_ref()?;
        Some(Rc::new(board(v, st.deciding, &cards.read())))
    });
    let ui = use_memo(move || {
        let st = game.state.read();
        let v = st.view.as_ref()?;
        let d = v.decision.as_ref()?;
        let b = board_memo.read();
        let b = b.as_ref()?;
        let inter = interaction(v, d, &cards.read());
        let spots = model::spots(b, &inter);
        let by_spot = spots
            .iter()
            .enumerate()
            .filter(|(_, s)| !matches!(s, Spot::Tray | Spot::Button))
            .map(|(i, s)| (s.clone(), i))
            .collect();
        Some(Rc::new(Ui {
            click_answers: answers_on_click(d),
            decision: d.clone(),
            inter,
            spots,
            by_spot,
        }))
    });
    let ctx = use_context_provider(|| TableCtx {
        ui,
        sel: Signal::new(Sel::default()),
        hover: Signal::new(None),
        hover_seq: Signal::new(0),
        popup: Signal::new(None),
        run: Signal::new(None),
        cards,
        game,
    });
    let mut run = ctx.run;

    use_effect(move || {
        let st = game.state.read();
        let Some(r) = run.read().clone() else { return };
        if st.answering {
            return;
        }
        if st.error.is_some() {
            run.set(None);
            return;
        }
        let Some(view) = st.view.as_ref() else { return };
        match r.run.next(view, &cards.read()) {
            Some(a) if r.sent.as_deref() != Some(a.decision.as_str()) => {
                run.set(Some(Run {
                    run: r.run.clone(),
                    sent: Some(a.decision.clone()),
                }));
                game.submit.call(a);
            }
            Some(_) => {}
            None => run.set(None),
        }
    });

    use_future(move || async move {
        let mut eval = document::eval(KEYS_JS);
        while let Ok(key) = eval.recv::<String>().await {
            ctx.key(&key);
        }
    });

    let assets = use_assets();
    let extra_css = table_css(&assets);
    let st = game.state.read();
    let error = st.error.clone();
    let answering = st.answering || run.read().is_some();
    let log = st.log.clone();
    let names = st
        .view
        .as_ref()
        .map(super::logfmt::names)
        .unwrap_or_default();
    let seat = st.view.as_ref().and_then(|v| v.seat);
    drop(st);
    let b = board_memo.read().clone();
    let Some(b) = b else {
        return rsx! {
            document::Style { "{CSS}" }
            div { class: "dt-root dt-loading",
                div { class: "dt-main",
                    p { class: "muted",
                        if let Some(e) = error { "{e}" } else { "Loading the game…" }
                    }
                }
                aside { class: "dt-side", {controls} }
            }
        };
    };
    let sel = ctx.selection();
    let ui_now = ui.read().clone();
    let can_treasures = ui_now
        .as_ref()
        .is_some_and(|u| !u.inter.treasures.is_empty());
    rsx! {
        document::Style { "{CSS}" }
        style { "{extra_css}" }
        div {
            class: "dt-root",
            class: if b.status.your_turn { "dt-your-turn" },
            class: if ui_now.is_some() { "dt-deciding" },
            "data-busy": if answering { "1" },
            "data-decision": ui_now.as_ref().map(|u| u.decision.id.clone()).unwrap_or_default(),
            "data-seat": seat.map(|s| s.to_string()).unwrap_or_default(),
            onclick: move |_| {
                if ctx.popup.peek().is_some() {
                    ctx.close();
                }
            },
            div { class: "dt-main",
                players::Opponents { opponents: b.opponents.clone(), ui: ui_now.clone(), sel: sel.clone() }
                div { class: "dt-middle",
                    supply::SupplyArea { board: b.clone(), ui: ui_now.clone(), sel: sel.clone() }
                    players::PlayArea { board: b.clone(), log: log.clone(), ui: ui_now.clone(), sel: sel.clone() }
                    prompt::Tray { ui: ui_now.clone(), sel: sel.clone() }
                    if let Some(p) = ctx.popup.read().clone() {
                        PopupView { popup: p }
                    }
                }
                prompt::Bar {
                    status: b.status.clone(),
                    ui: ui_now.clone(),
                    sel: sel.clone(),
                    error,
                    busy: answering,
                    can_treasures,
                }
                players::YourArea { board: b.clone(), ui: ui_now.clone(), sel: sel.clone() }
            }
            aside { class: "dt-side",
                div { class: "dt-controls", {controls} }
                side::More { board: b.clone() }
                side::Log { entries: log, names }
            }
            if b.result.is_some() {
                side::GameOver { board: b.clone() }
            }
            pieces::HoverLayer {}
        }
    }
}

#[component]
fn PopupView(popup: Popup) -> Element {
    let ctx = use_table();
    let p = &popup;
    rsx! {
        div {
            class: "dt-popup",
            onclick: move |e| e.stop_propagation(),
            div { class: "dt-popup-head",
                span { "{p.title}" }
                button { class: "dt-x", "data-close": "popup", onclick: move |_| ctx.close(), "×" }
            }
            if let Some(n) = &p.note {
                p { class: "dt-note", "{n}" }
            }
            div { class: "dt-popup-cards",
                for s in p.stacks.iter() {
                    pieces::TableCard {
                        key: "{s.card}",
                        name: s.card.clone(),
                        size: crate::card::CardSize::Full,
                        count: (s.count > 1).then_some(s.count),
                    }
                }
                if p.unknown > 0 {
                    pieces::TableCard {
                        name: "",
                        size: crate::card::CardSize::Full,
                        back: true,
                        count: Some(p.unknown),
                        label: "unknown",
                    }
                }
                if p.stacks.is_empty() && p.unknown == 0 {
                    p { class: "muted", "Empty." }
                }
            }
        }
    }
}
