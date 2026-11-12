//! The graphical table, laid out and behaving like dominion.games' game screen: player
//! bars and basic piles on the left, the kingdom with the play row under it, the status
//! bar and buttons, your hand at the bottom and the log on the right, every zone a box
//! measured from the window ([`layout`]). It draws the [`board`](super::board) model and
//! answers through the [`interaction`](super::interaction) model: every target of the
//! open decision has one [`Spot`] where it is clicked.

pub mod layout;
pub mod model;
mod motion;
mod pieces;
mod players;
mod prompt;
mod side;
mod sound;
mod supply;

use super::board::{Board, Stack, board};
use super::interaction::{Interaction, TreasureRun, interaction, tone};
use super::state::{Selection, answers_on_click};
use super::use_game;
use crate::app::{Cards, Settings};
use crate::assets::use_assets;
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::{AgentDecision, Answer};
pub use model::Spot;
pub use motion::before_change;
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
    /// What a click does, for the border's colour: see [`tone`].
    pub tone: &'static str,
}

impl Ui {
    pub fn mark_target(&self, sel: &Selection, t: usize) -> Mark {
        let target = &self.inter.targets[t];
        let picked = sel.positions(&target.options);
        // Once the picks are complete, only the picked cards keep their colour.
        let full = !self.click_answers
            && sel.picks().len() >= self.inter.max as usize
            && picked.is_empty();
        Mark {
            target: Some(t),
            options: target.options.len(),
            picked,
            ordered: self.inter.ordered,
            tone: if full {
                ""
            } else {
                tone(&self.decision, target)
            },
        }
    }

    pub fn mark(&self, sel: &Selection, spot: &Spot) -> Mark {
        self.by_spot
            .get(spot)
            .map(|&t| self.mark_target(sel, t))
            .unwrap_or_default()
    }

    /// Whether any pile is to be clicked: windows then open low, over the hand.
    pub fn piles_clickable(&self) -> bool {
        self.spots.iter().any(|s| matches!(s, Spot::Pile(_)))
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

/// What an opened pile or mat shows.
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
    /// The card shown in the study view.
    pub studied: Signal<Option<String>>,
    /// The window a tab or pile opened, and whether a click keeps it open.
    pub popup: Signal<Option<(Popup, bool)>>,
    /// The opponent whose hand, deck and discard peek in from the top, when chosen.
    pub chosen_opponent: Signal<Option<u8>>,
    run: Signal<Option<Run>>,
    cards: Signal<Rc<crate::cards::CardDb>>,
    game: super::GameCtx,
}

pub fn use_table() -> TableCtx {
    use_context::<TableCtx>()
}

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

    fn set_sel(&self, id: &str, sel: Selection) {
        let mut s = self.sel;
        s.set(Sel {
            id: id.to_string(),
            sel,
        });
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
        // A click picks one more copy; on a stack whose copies are all picked it takes
        // one back. With the picks full it does nothing.
        if !sel.click(d, &target.options) && sel.count(&target.options) == target.options.len() {
            sel.unclick(&target.options);
        }
        self.set_sel(&d.id, sel);
    }

    /// The "All" chip on target `t`: plays every copy, one after another, or picks (or
    /// unpicks) every copy.
    pub fn act_all(&self, t: usize) {
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
            let card = target.card.clone();
            let view = self.game.state.peek().view.clone();
            let cards = self.cards.peek().clone();
            if let (Some(card), Some(view)) = (card, view)
                && let Some(run) = TreasureRun::start_card(&view, &cards, &card)
            {
                let mut r = self.run;
                r.set(Some(Run { run, sent: None }));
            } else {
                self.act(t);
            }
            return;
        }
        let mut sel = self.edit_sel(&d.id);
        if sel.positions(&target.options).len() == target.options.len() {
            for _ in 0..target.options.len() {
                sel.unclick(&target.options);
            }
        } else {
            while sel.click(d, &target.options) {}
        }
        self.set_sel(&d.id, sel);
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
        self.set_sel(&ui.decision.id, sel);
    }

    pub fn reorder(&self, from: usize, to: usize) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        let mut sel = self.edit_sel(&ui.decision.id);
        sel.reorder(from, to);
        self.set_sel(&ui.decision.id, sel);
    }

    pub fn clear(&self) {
        let mut s = self.sel;
        s.set(Sel::default());
    }

    /// Takes back this seat's last answer, stopping an Autoplay Treasures run.
    pub fn undo(&self) {
        if !self.game.state.peek().can_undo || self.game.state.peek().answering {
            return;
        }
        let mut run = self.run;
        run.set(None);
        self.clear();
        self.game.undo.call(());
    }

    /// Picks every option a target in `places` stands for ("Select All").
    pub fn select_all(&self) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        let d = &ui.decision;
        let mut sel = self.edit_sel(&d.id);
        for t in &ui.inter.targets {
            if t.card.is_some() {
                while sel.click(d, &t.options) {}
            }
        }
        self.set_sel(&d.id, sel);
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

    /// Answers with nothing picked: "Don't …", "Continue".
    pub fn decline(&self) {
        let Some(ui) = self.ui.peek().clone() else {
            return;
        };
        if self.busy() {
            return;
        }
        if let Some(a) = Selection::default().answer(&ui.decision) {
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

    /// "Autoplay Treasures".
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

    pub fn study(&self, name: &str) {
        let mut s = self.studied;
        s.set(Some(name.to_string()));
    }

    pub fn unstudy(&self) {
        let mut s = self.studied;
        if s.peek().is_some() {
            s.set(None);
        }
    }

    /// Opens a window; a click on what is already open closes it.
    pub fn open(&self, p: Popup) {
        let mut popup = self.popup;
        let same = popup
            .peek()
            .as_ref()
            .is_some_and(|(q, locked)| *q == p && *locked);
        popup.set(if same { None } else { Some((p, true)) });
    }

    /// Shows a window while the pointer is over its tab, unless one is kept open.
    pub fn peek_open(&self, p: Popup) {
        let mut popup = self.popup;
        if popup.peek().as_ref().is_none_or(|(_, locked)| !locked) {
            popup.set(Some((p, false)));
        }
    }

    pub fn peek_close(&self) {
        let mut popup = self.popup;
        if popup.peek().as_ref().is_some_and(|(_, locked)| !locked) {
            popup.set(None);
        }
    }

    pub fn close(&self) {
        let mut popup = self.popup;
        popup.set(None);
    }

    pub fn choose_opponent(&self, seat: u8) {
        let mut c = self.chosen_opponent;
        c.set(Some(seat));
    }

    fn key(&self, key: &str) {
        match key {
            "Enter" => self.confirm(),
            "Escape" => {
                self.clear();
                self.close();
                self.unstudy();
            }
            "e" | "E" => self.press("end-phase"),
            "t" | "T" => self.play_treasures(),
            "undo" => self.undo(),
            _ => {}
        }
    }
}

const VIEWPORT_JS: &str = r#"
if (window.__dtViewport) window.removeEventListener('resize', window.__dtViewport);
window.__dtViewport = () => { try { dioxus.send([window.innerWidth, window.innerHeight]); } catch (_) {} };
window.addEventListener('resize', window.__dtViewport);
window.__dtViewport();
"#;

const KEYS_JS: &str = r#"
if (window.__dtKeys) document.removeEventListener('keydown', window.__dtKeys);
window.__dtKeys = (e) => {
  const t = e.target;
  const typing = t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT');
  if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && (e.key === 'z' || e.key === 'Z')) {
    if (typing || !document.querySelector('.dt-root')) return;
    e.preventDefault();
    try { dioxus.send('undo'); } catch (_) {}
    return;
  }
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  if (typing && e.key !== 'Escape') return;
  if (!['Enter', 'Escape', 'e', 'E', 't', 'T'].includes(e.key)) return;
  if (!document.querySelector('.dt-root')) return;
  e.preventDefault();
  try { dioxus.send(e.key); } catch (_) {}
};
document.addEventListener('keydown', window.__dtKeys);
"#;

/// The images the table uses besides cards, as CSS variables when the cache has them.
fn table_css(assets: &crate::assets::AssetDb, background: &str) -> String {
    let mut css = String::from(".dt-root{");
    let bg = format!("images/large/{background}.webp");
    for (name, path) in [
        ("bg", bg.as_str()),
        ("btn-copper", "images/materials/copper_button.jpg"),
        ("btn-gold", "images/materials/gold_button.jpg"),
        ("btn-stone", "images/materials/stone_button.png"),
        ("tab", "images/elements/tab-bg.png"),
        ("tab-active", "images/elements/active-tab-bg.png"),
        ("trash-mat", "images/mats/trash_mat.png"),
        ("tavern-mat", "images/mats/tavern_mat.png"),
        ("exile-mat", "images/mats/exile_mat.png"),
        ("counter", "images/elements/card-counter.png"),
        ("coin", "images/elements/coin.png"),
        ("potion", "images/elements/potion-square.png"),
        ("debt", "images/elements/debt_130.png"),
        ("coffers", "images/elements/coffers.png"),
        ("villagers", "images/elements/villagers.png"),
        ("favors", "images/elements/favors.png"),
        ("vp", "images/elements/victory_150.png"),
        ("treasures", "images/buttons/autoplay-treasures.png"),
        ("refuse", "images/buttons/refuse.png"),
        ("continue", "images/buttons/continue.png"),
        ("discard-icon", "images/buttons/discard.png"),
        ("trash-icon", "images/buttons/trash.png"),
        ("topdeck-icon", "images/buttons/topdeck.png"),
    ] {
        if !path.ends_with("/.webp") && assets.has(path) {
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

/// The opponent whose hand, deck and discard pile peek in from the top: the one whose
/// turn it is, else the one chosen, else the next in turn order.
fn active_opponent(b: &Board, chosen: Option<u8>) -> Option<u8> {
    if let Some(p) = &b.status.player
        && b.opponents.iter().any(|o| o.seat == p.seat)
    {
        return Some(p.seat);
    }
    chosen
        .filter(|c| b.opponents.iter().any(|o| o.seat == *c))
        .or_else(|| b.opponents.first().map(|o| o.seat))
}

/// The layout's inputs for a board.
/// `deciding`: the shown seat has a decision open.
fn shape(
    b: &Board,
    settings: &dominion_protocol::api::WebSettings,
    deciding: bool,
) -> layout::Shape {
    let (victory, treasures) = supply::basic_columns(b);
    layout::Shape {
        opponents: b.opponents.len(),
        victory: victory.len(),
        treasures: treasures.len(),
        kingdom: b.supply.kingdom.iter().map(Vec::len).sum(),
        landscapes: supply::wides(b).len(),
        non_supply: b.non_supply.len(),
        lowered: settings.web_move_kingdom
            && b.status.running
            && b.you.is_some()
            && b.status.player.is_some()
            && !b.status.your_turn
            && !deciding
            && !at_start(b),
    }
}

/// The game page's table. `controls` (seat switching, the view toggle) go under the log.
#[component]
pub fn TableGame(controls: Element) -> Element {
    let game = use_game();
    let cards = use_context::<Cards>().0;
    let settings = use_context::<Settings>().0;
    let board_memo: Memo<Option<Rc<Board>>> = use_memo(move || {
        let st = game.state.read();
        let v = st.view.as_ref()?;
        Some(Rc::new(board(v, st.deciding, &st.table, &cards.read())))
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
            .filter(|(_, s)| !matches!(s, Spot::Tray | Spot::Button | Spot::Ability))
            .map(|(i, s)| (s.clone(), i))
            .collect();
        let confirm_single = settings.read().web_confirm_single;
        Some(Rc::new(Ui {
            click_answers: (answers_on_click(d)
                || (d.min == Some(0) && d.max == Some(1) && d.groups.len() <= 1))
                && !(confirm_single
                    && matches!(
                        d.purpose.as_str(),
                        "discard" | "trash" | "exile" | "top-deck"
                    )),
            decision: d.clone(),
            inter,
            spots,
            by_spot,
        }))
    });
    let ctx = use_context_provider(|| TableCtx {
        ui,
        sel: Signal::new(Sel::default()),
        studied: Signal::new(None),
        popup: Signal::new(None),
        chosen_opponent: Signal::new(None),
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
        let mut next = r.run.clone();
        match next.take(view, &cards.read()) {
            Some(a) if r.sent.as_deref() != Some(a.decision.as_str()) => {
                run.set(Some(Run {
                    run: next,
                    sent: Some(a.decision.clone()),
                }));
                game.submit.call(a);
            }
            Some(_) => {}
            None => run.set(None),
        }
    });

    let flight = motion::use_motion(board_memo, game);
    let muted = use_signal(sound::session_muted);
    let mut started = use_signal(|| false);
    sound::use_sound(board_memo, game, muted);
    let mut viewport = use_signal(platform::viewport);
    use_future(move || async move {
        let mut eval = document::eval(VIEWPORT_JS);
        while let Ok(v) = eval.recv::<Vec<f64>>().await {
            if let [w, h] = v[..] {
                let now = (w, h);
                if *viewport.peek() != now {
                    viewport.set(now);
                }
            }
        }
    });
    let geometry = use_memo(move || {
        let (w, h) = viewport();
        let b = board_memo.read();
        let deciding = ui.read().is_some();
        let s = b
            .as_ref()
            .map(|b| shape(b, &settings.read(), deciding))
            .unwrap_or_default();
        Rc::new(layout::geometry(w, h, &s))
    });

    use_future(move || async move {
        let mut eval = document::eval(KEYS_JS);
        while let Ok(key) = eval.recv::<String>().await {
            ctx.key(&key);
        }
    });

    let assets = use_assets();
    let extra_css = table_css(&assets, &settings.read().web_background);
    let st = game.state.read();
    let error = st.error.clone();
    let answering = st.answering || run.read().is_some();
    let can_undo = st.can_undo;
    let log = st.log.clone();
    let names = st
        .view
        .as_ref()
        .map(super::logfmt::names)
        .unwrap_or_default();
    let seat = st.view.as_ref().and_then(|v| v.seat);
    let game_id = st.view.as_ref().and_then(|v| v.game.id.clone());
    drop(st);
    let b = board_memo.read().clone();
    let Some(b) = b else {
        return rsx! {
            document::Style { "{CSS}" }
            div { class: "dt-root dt-loading",
                p { class: "dt-loading-text",
                    if let Some(e) = error { "{e}" } else { "Loading the game…" }
                }
                div { class: "dt-below-log dt-below-log-loading", {controls} }
            }
        };
    };
    let start_key = format!(
        "dominion.started.{}.{}",
        game_id.clone().unwrap_or_default(),
        seat.map(|s| s.to_string()).unwrap_or_default()
    );
    let starting = !started() && at_start(&b) && platform::session_get(&start_key).is_none();
    let b = if starting {
        Rc::new(starting_board(&b))
    } else {
        b
    };
    let sel = ctx.selection();
    let ui_now = if starting { None } else { ui.read().clone() };
    let g = geometry();
    let active = active_opponent(&b, *ctx.chosen_opponent.read());
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
            style: "--vh:{g.vh:.3}px;--vw:{g.vw:.3}px;",
            "data-busy": if answering { "1" },
            "data-decision": ui_now.as_ref().map(|u| u.decision.id.clone()).unwrap_or_default(),
            "data-seat": seat.map(|s| s.to_string()).unwrap_or_default(),
            onclick: move |_| {
                let locked = ctx.popup.peek().as_ref().is_some_and(|(_, l)| *l);
                if locked {
                    ctx.close();
                }
            },
            div { class: "dt-bg" }
            supply::SupplyArea { board: b.clone(), g: g.clone(), ui: ui_now.clone(), sel: sel.clone() }
            players::Opponents { board: b.clone(), g: g.clone(), active }
            players::PlayArea { board: b.clone(), g: g.clone(), ui: ui_now.clone(), sel: sel.clone() }
            players::YourArea { board: b.clone(), g: g.clone(), ui: ui_now.clone(), sel: sel.clone() }
            prompt::StatusBar {
                board: b.clone(),
                g: g.clone(),
                ui: ui_now.clone(),
                error,
                starting,
            }
            if starting {
                button {
                    class: "dt-btn dt-start-game",
                    style: "left:{g.buttons_cx:.1}px;top:{g.buttons_bottom:.1}px;",
                    "data-action": "start-game",
                    onclick: move |_| {
                        platform::session_set(&start_key, "1");
                        started.set(true);
                    },
                    "Start Game"
                }
            }
            prompt::Buttons {
                g: g.clone(),
                ui: ui_now.clone(),
                sel: sel.clone(),
                busy: answering,
                can_treasures,
            }
            prompt::DecisionWindow { g: g.clone(), ui: ui_now.clone(), sel: sel.clone() }
            if !starting {
                super::WaitingNotice {
                    style: format!(
                        "left:{:.1}px;top:{:.1}px;width:{:.1}px;",
                        g.play_x,
                        g.play_y + g.vh,
                        g.play_right - g.play_x - 22.0 * g.vh,
                    ),
                }
            }
            if let Some((p, _)) = ctx.popup.read().clone() {
                PopupView { popup: p, g: g.clone() }
            }
            side::Log { entries: log, names, g: g.clone(), game_id, board: b.clone() }
            div { class: "dt-below-log", style: "{g.below_log.style()}",
                div { class: "dt-controls", {controls}
                    if can_undo {
                        button {
                            "data-action": "undo",
                            title: "Take back your last answer (Ctrl+Z)",
                            disabled: answering,
                            onclick: move |_| ctx.undo(),
                            "Undo"
                        }
                    }
                    sound::MuteToggle { muted }
                }
                side::More { board: b.clone() }
            }
            if b.result.is_some() {
                side::GameOver { board: b.clone() }
            }
            pieces::StudyLayer {}
            motion::GhostPool { flight: flight() }
        }
    }
}

/// Nothing has happened yet: every player still owns just their starting cards, with
/// nothing in play or discarded. (The trash may start with cards: Necromancer's Zombies.)
fn at_start(b: &Board) -> bool {
    b.status.running
        && b.play_area.cards.count == 0
        && b.you.as_ref().is_some_and(|y| y.owned.count > 0)
        && b.you.iter().chain(&b.opponents).all(|p| {
            p.in_play.count == 0 && p.owned.count == STARTING_CARDS && p.discard.count == 0
        })
}

const STARTING_CARDS: u32 = 10;

/// dominion.games' first frame: your starting cards in the hand, nothing to do yet.
fn starting_board(b: &Board) -> Board {
    let mut b = b.clone();
    if let Some(y) = b.you.as_mut() {
        y.hand = y.owned.clone();
        y.deck = Default::default();
        y.discard = Default::default();
        y.discard_top = None;
    }
    b.status.actions = Some(0);
    b.status.buys = Some(0);
    b.status.coins = Some(0);
    b.status.your_turn = false;
    b
}

/// A window listing a pile's or mat's cards, as dominion.games opens them from tabs.
#[component]
fn PopupView(popup: Popup, g: Rc<layout::Geometry>) -> Element {
    let ctx = use_table();
    let p = &popup;
    let n = p.stacks.len() + usize::from(p.unknown > 0);
    let win = g.window(n.max(1) as f64, false);
    let row = g.window_row(&win, n, &vec![false; n]);
    rsx! {
        div {
            class: "dt-window dt-popup",
            style: "{win.style()}",
            onclick: move |e| e.stop_propagation(),
            div { class: "dt-window-title", "{p.title}" }
            button { class: "dt-x", "data-close": "popup", onclick: move |_| ctx.close(), "✕" }
            if let Some(note) = &p.note {
                p { class: "dt-note", "{note}" }
            }
            for (i, s) in p.stacks.iter().enumerate() {
                pieces::TableCard {
                    key: "{s.card}",
                    name: s.card.clone(),
                    size: crate::card::CardSize::Full,
                    count: (s.count > 1).then_some(s.count),
                    at: layout::Rect::new(row.xs[i] - win.x, row.y - win.y, row.w, row.h),
                }
            }
            if p.unknown > 0 {
                pieces::TableCard {
                    name: "",
                    size: crate::card::CardSize::Full,
                    back: true,
                    count: Some(p.unknown),
                    at: layout::Rect::new(row.xs[n - 1] - win.x, row.y - win.y, row.w, row.h),
                }
            }
            if p.stacks.is_empty() && p.unknown == 0 {
                p { class: "dt-window-empty", "Empty." }
            }
        }
    }
}
