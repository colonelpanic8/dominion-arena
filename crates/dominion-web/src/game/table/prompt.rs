//! The status bar (the turn player's Actions, Buys and coins over the prompt, the phase's
//! end button at its right end), the row of buttons above it, the column of ability
//! buttons at the right, and the window holding choices the table shows nowhere else:
//! revealed or looked-at cards, cards in decks and discard piles, named cards.

use super::layout::{Geometry, Rect};
use super::pieces::{CardName, Reminder, TableCard};
use super::supply::Badges;
use super::{Spot, Ui, use_table};
use crate::app::use_cards;
use crate::assets::use_assets;
use crate::card::CardSize;
use crate::game::board::{self, Board};
use crate::game::interaction::Target;
use crate::game::state::Selection;
use crate::game::wording;
use dioxus::prelude::*;
use std::rc::Rc;

/// The cards whose choices dominion.games offers "Select All" for.
const SELECT_ALL: [&str; 11] = [
    "Cellar",
    "Vault",
    "Inn",
    "Storeroom",
    "Artificer",
    "Opulent Castle",
    "Shepherd",
    "Crypt",
    "Silos",
    "Hostelry",
    "Marchland",
];

/// The cards whose questions dominion.games' window shows stacked, like cards together;
/// it draws every other question's cards one by one.
const STACKED_WINDOWS: [&str; 25] = [
    "Harbinger",
    "Hermit",
    "Scavenger",
    "Annex",
    "Bustling Village",
    "Settlers",
    "Herald",
    "Inn",
    "Counting House",
    "The Moon's Gift",
    "Crypt",
    "Necromancer",
    "Mountain Village",
    "Star Chart",
    "March",
    "Order of Astrologers",
    "Order of Masons",
    "Herb Gatherer",
    "Courier",
    "Bury",
    "Prepare",
    "Orb",
    "Fated",
    "Avoid",
    "Joust",
];

/// Number decisions with more values than this get a slider instead of a button each.
const MAX_NUMBER_BUTTONS: usize = 6;

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[component]
pub fn StatusBar(
    board: Rc<Board>,
    g: Rc<Geometry>,
    ui: Option<Rc<Ui>>,
    error: Option<String>,
    /// The first frame: "Your starting cards:".
    #[props(default)]
    starting: bool,
) -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let forfeit = crate::game::use_forfeit();
    let s = &board.status;
    let turn_player = s
        .player
        .as_ref()
        .and_then(|w| board.players().find(|p| p.seat == w.seat));
    let token = |kind: &str| -> u32 {
        turn_player
            .and_then(|p| p.badges.iter().find(|b| b.kind == kind))
            .and_then(|b| b.count)
            .map_or(0, |n| n.max(0) as u32)
    };
    let actions = s.actions.unwrap_or(0);
    let buys = s.buys.unwrap_or(0);
    let coins = s.coins.unwrap_or(0);
    let potions = s.potions.unwrap_or(0);
    let debt = s.debt.unwrap_or_else(|| token("debt"));
    let (coffers, villagers, favors) = (token("coffers"), token("villagers"), token("favors"));
    // What else the turn player has, as dominion.games shows it after the counters: not
    // the Journey token, VP tokens or Pirate Ship tokens, which it shows only beside the
    // player's name, nor the -1 Card and -$1 tokens, on the deck.
    let shown = [
        "coffers",
        "villagers",
        "favors",
        "debt",
        "journey",
        "journey-down",
        "vp",
        "pirate-ship",
        "-card",
        "-coin",
    ];
    let badges: Vec<_> = turn_player
        .map(|p| {
            p.badges
                .iter()
                .filter(|b| !shown.contains(&b.kind.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let reminders: Vec<(String, u32)> =
        turn_player.map(|p| p.reminders.clone()).unwrap_or_default();
    let held = !badges.is_empty() || !reminders.is_empty();
    let ticker: Element = if starting {
        rsx! { span { "data-prompt": "starting", "Your starting cards:" } }
    } else if !s.running {
        rsx! {}
    } else if let Some(u) = ui.clone() {
        let asked = board.players().find(|p| Some(p.seat) == u.decision.player);
        let asking = wording::Asking {
            name: asked.map(|p| p.name.clone()),
            others: board
                .players()
                .filter(|p| Some(p.seat) != u.decision.player)
                .map(|p| p.name.clone())
                .collect(),
            pirate_ship: asked
                .and_then(|p| p.badges.iter().find(|b| b.kind == "pirate-ship"))
                .and_then(|b| b.count)
                .map_or(0, |n| n.max(0) as u32),
        };
        let parts = wording::prompt_parts(&u.decision, &cards, &asking);
        rsx! {
            span { "data-prompt": "{u.decision.purpose}",
                for (i, part) in parts.iter().enumerate() {
                    span { key: "{i}", if i > 0 { " " } "{part}" }
                }
            }
        }
    } else {
        // dominion.games says nothing while another player decides.
        let waiting = match &s.deciding {
            Some(_) => rsx! {},
            None => rsx! { "Waiting…" },
        };
        rsx! {
            if board.watching {
                span { class: "dt-watch-mark", "data-watching": "1", "Watching. " }
            }
            {waiting}
        }
    };
    let forfeit = forfeit.filter(|_| s.running && !starting);
    // The phase's end, inside the bar's right end.
    let end = ui.as_ref().and_then(|u| {
        u.spots
            .iter()
            .enumerate()
            .find(|(i, sp)| **sp == Spot::Button && u.inter.targets[*i].action == "end-phase")
            .map(|(i, _)| i)
    });
    let (end_label, end_look) = wording::end_phase(s.phase.as_deref());
    // Spending Coffers or a Villager, or paying off Debt: a click on its counter.
    let spend = |kind: &str| {
        let action = counter_action(kind)?;
        let u = ui.as_ref()?;
        (0..u.spots.len())
            .find(|&i| u.spots[i] == Spot::Button && u.inter.targets[i].action == action)
    };
    let (spend_villager, spend_debt, spend_coffers) =
        (spend("villagers"), spend("debt"), spend("coffers"));
    let local = Rect::new(
        g.end_button.x - g.status.x,
        g.end_button.y - g.status.y,
        g.end_button.w,
        g.end_button.h,
    );
    rsx! {
        div { class: "dt-status", style: "{g.status.style()}",
            div { class: "dt-counters",
                span { class: "dt-counter dt-actions", class: if actions == 0 { "dt-zero" }, "{plural(actions, \"Action\", \"Actions\")}" }
                if villagers > 0 {
                    SpendCounter { kind: "villagers", title: "Villagers", n: villagers, target: spend_villager }
                }
                span { class: "dt-sep" }
                span { class: "dt-counter dt-buys", class: if buys == 0 { "dt-zero" }, "{plural(buys, \"Buy\", \"Buys\")}" }
                span { class: "dt-sep" }
                span { class: "dt-counter dt-coin", title: "Coins", "{coins}" }
                if potions > 0 {
                    span { class: "dt-counter dt-icon dt-potion", title: "Potions", if potions > 1 { "{potions}" } }
                }
                if debt > 0 {
                    SpendCounter { kind: "debt", title: "Debt", n: debt, target: spend_debt }
                }
                if coffers > 0 {
                    SpendCounter { kind: "coffers", title: "Coffers", n: coffers, target: spend_coffers }
                }
                if favors > 0 {
                    span { class: "dt-sep" }
                    span { class: "dt-counter dt-icon dt-favors", title: "Favors", "{favors}" }
                }
                span { class: "dt-sep" }
                if held {
                    span { class: "dt-status-held", "data-held": "1",
                        Badges { badges }
                        for (c, n) in reminders.iter() {
                            Reminder { key: "{c}", name: c.clone(), count: *n }
                        }
                    }
                }
            }
            div { class: "dt-ticker",
                {ticker}
                if let Some((f, text)) = forfeit {
                    span {
                        class: "dt-forfeit",
                        "data-forfeit": "{f.seat}",
                        "data-seconds": "{f.seconds}",
                        " {text}"
                    }
                }
                if let Some(e) = error {
                    span { class: "dt-error", "data-error": "1", " {e}" }
                }
            }
            if let Some(i) = end {
                button {
                    class: "dt-end dt-end-{end_look}",
                    style: "{local.style()}",
                    "data-target": "{i}",
                    "data-options": "1",
                    onclick: move |e| { e.stop_propagation(); ctx.act(i); },
                    "{end_label}"
                }
            }
        }
    }
}

/// A button's label as dominion.games fits it on a `w` × `h` button: one line in 90% of
/// the width beside any icon, at most 0.405 of the height tall; on a button with an icon,
/// two lines of up to 0.3 of the height where one would be smaller than that.
fn label_lines(
    label: &str,
    w: f64,
    h: f64,
    icon: bool,
    m: &dyn crate::card::fit::Measure,
) -> Vec<(String, f64)> {
    let room = 0.9 * if icon { w - h } else { w };
    let fit = |text: &str, cap: f64| {
        let ems = f64::from(m.width(text, crate::card::fit::Font::Title)).max(0.01);
        cap.min(room / ems)
    };
    let one = fit(label, 0.405 * h);
    if !icon || one >= 0.3 * h {
        return vec![(label.to_string(), one)];
    }
    let words: Vec<&str> = label.split(' ').collect();
    let (a, b) = words.split_at(words.len() / 2);
    [a.join(" "), b.join(" ")]
        .into_iter()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let px = fit(&l, 0.3 * h);
            (l, px)
        })
        .collect()
}

/// The icon dominion.games draws on a button choosing a vanilla bonus.
fn mode_icon(label: &str) -> Option<&'static str> {
    Some(match label {
        "+1 Card" => "1-card",
        "+2 Cards" => "2-card",
        "+3 Cards" => "3-card",
        "+1 Action" => "action",
        "+2 Actions" => "2-action",
        "+1 Buy" => "buy",
        "+$1" => "1-coin",
        "+$2" => "2-coin",
        "+$3" => "3-coin",
        "Gain a Gold" => "gold",
        _ => return None,
    })
}

/// The option a counter answers when clicked, as dominion.games has no button for it.
fn counter_action(kind: &str) -> Option<&'static str> {
    match kind {
        "coffers" => Some("spend-coffers"),
        "villagers" => Some("spend-villager"),
        "debt" => Some("pay-debt"),
        _ => None,
    }
}

/// A token counter in the status bar; clicking it spends one where that is offered.
#[component]
fn SpendCounter(kind: &'static str, title: &'static str, n: u32, target: Option<usize>) -> Element {
    let ctx = use_table();
    rsx! {
        span {
            class: "dt-counter dt-icon dt-{kind}",
            class: if target.is_some() { "dt-click dt-spend" },
            title,
            "data-target": target.map(|i| i.to_string()),
            "data-options": target.map(|_| "1"),
            onclick: move |e| {
                if let Some(i) = target {
                    e.stop_propagation();
                    ctx.act(i);
                }
            },
            "{n}"
        }
    }
}

/// One button of the row: black, white Trajan letters, an icon at its right end.
#[derive(Clone, Debug, PartialEq)]
struct Btn {
    label: String,
    icon: Option<&'static str>,
    count: Option<String>,
    disabled: bool,
    on: bool,
    what: Act,
}

#[derive(Clone, Debug, PartialEq)]
enum Act {
    Target(usize),
    Treasures,
    SelectAll,
    Confirm,
    Decline,
}

#[component]
pub fn Buttons(
    g: Rc<Geometry>,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    busy: bool,
    can_treasures: bool,
    /// The question's window shows cards looked at, which may answer it.
    #[props(default)]
    looking: bool,
) -> Element {
    let ctx = use_table();
    let assets = use_assets();
    let cards = use_cards();
    let mut number = use_signal(|| 0usize);
    let Some(u) = ui else {
        return rsx! {};
    };
    let d = &u.decision;
    let by_cards = wording::answered_by_cards(d).filter(|_| looking);
    let main: Vec<usize> = (0..u.spots.len())
        .filter(|&i| u.spots[i] == Spot::Button && u.inter.targets[i].action != "end-phase")
        .filter(|&i| by_cards != Some(u.inter.targets[i].label.as_str()))
        .filter(|&i| {
            !["coffers", "villagers", "debt"]
                .iter()
                .any(|k| counter_action(k) == Some(u.inter.targets[i].action.as_str()))
        })
        .collect();
    // dominion.games lists abilities by the ids of their cards, which follow its list
    // of cards.
    let mut side: Vec<usize> = (0..u.spots.len())
        .filter(|&i| u.spots[i] == Spot::Ability)
        .collect();
    side.sort_by_key(|&i| board::id_order(u.inter.targets[i].card.as_deref().unwrap_or("")));
    let numbers = main.len() > MAX_NUMBER_BUTTONS
        && main.iter().all(|&i| u.inter.targets[i].action == "number");
    let picks = sel.picks().len();
    let mut row: Vec<Btn> = vec![];
    if !numbers {
        for &i in &main {
            let t = &u.inter.targets[i];
            let resolving =
                matches!(d.purpose.as_str(), "trigger" | "reaction") && t.action == "no";
            let named = wording::choice_button(d, &t.label);
            row.push(Btn {
                label: if resolving {
                    "Continue".into()
                } else if let Some((label, _)) = named {
                    label.into()
                } else {
                    t.label.clone()
                },
                icon: if resolving {
                    Some("continue")
                } else if let Some((_, icon)) = named {
                    Some(icon)
                } else {
                    mode_icon(&t.label)
                },
                count: None,
                disabled: busy,
                on: !sel.positions(&t.options).is_empty(),
                what: Act::Target(i),
            });
        }
    }
    if can_treasures {
        row.push(Btn {
            label: "Autoplay Treasures".into(),
            icon: Some("treasures"),
            count: None,
            disabled: busy,
            on: false,
            what: Act::Treasures,
        });
    }
    let card_targets = u.inter.targets.iter().filter(|t| t.card.is_some()).count();
    if !u.click_answers {
        let max = d.max.unwrap_or(1) as usize;
        let all_options: usize = u
            .inter
            .targets
            .iter()
            .filter(|t| t.card.is_some())
            .map(|t| t.options.len())
            .sum();
        let select_all_card = d.source.as_deref().is_some_and(|c| SELECT_ALL.contains(&c));
        if select_all_card
            && d.min == Some(0)
            && max > 1
            && max >= all_options
            && card_targets > 1
            && !d.ordered
            && picks < all_options
        {
            row.push(Btn {
                label: "Select All".into(),
                icon: None,
                count: None,
                disabled: busy,
                on: false,
                what: Act::SelectAll,
            });
        }
        let none = wording::done_with_none(d).filter(|_| picks == 0);
        // Cards to order, confirmed in the order they lie.
        let as_they_lie = picks == 0 && crate::game::state::default_order(d).is_some();
        let icon = match d.purpose.as_str() {
            _ if as_they_lie => Some("continue"),
            _ if none.is_some() => Some("refuse"),
            "discard" => Some("discard-icon"),
            "trash" => Some("trash-icon"),
            "top-deck" => Some("topdeck-icon"),
            _ => None,
        };
        row.push(Btn {
            label: none.unwrap_or_else(|| wording::done(d)).into(),
            icon,
            count: wording::done_count(d, picks).filter(|_| !as_they_lie),
            disabled: busy || !(sel.is_legal(d) || as_they_lie),
            on: false,
            what: Act::Confirm,
        });
    } else if d.min == Some(0) && picks == 0 && d.kind != "turn" {
        row.push(Btn {
            label: wording::decline(d).into(),
            icon: Some(if matches!(d.purpose.as_str(), "reaction" | "trigger") {
                "continue"
            } else {
                "refuse"
            }),
            count: None,
            disabled: busy,
            on: false,
            what: Act::Decline,
        });
    }
    let (bw, bh) = g.button;
    let width = |b: &Btn| {
        if b.icon.is_some() || b.count.is_some() {
            bw * 4.0 / 3.0
        } else {
            bw
        }
    };
    let gap = 2.0 * g.vh;
    // Rows centred on the button line, wrapping upwards past 56vw.
    let mut rows: Vec<Vec<usize>> = vec![vec![]];
    let mut used = 0.0;
    for (i, b) in row.iter().enumerate() {
        let w = width(b);
        if !rows.last().is_some_and(|r| r.is_empty()) && used + gap + w > 56.0 * g.vw {
            rows.push(vec![]);
            used = 0.0;
        }
        used += if rows.last().is_some_and(|r| r.is_empty()) {
            w
        } else {
            gap + w
        };
        rows.last_mut().expect("a row").push(i);
    }
    let mut placed: Vec<(usize, Rect)> = vec![];
    // Abilities are as tall as their art shows at the column's width.
    let side_w = 21.0 * g.vh;
    let side_h = side_w * 5.0 / 12.0;
    let below_side = g.side_top + side.len() as f64 * (side_h + g.vh);
    // With abilities to choose from, or five buttons or more, every button joins the
    // column at the right.
    if !side.is_empty() || row.len() >= 5 {
        rows.clear();
        for (i, b) in row.iter().enumerate() {
            let w = width(b);
            let y = below_side + i as f64 * (bh + g.vh);
            placed.push((i, Rect::new(g.side_right - w, y, w, bh)));
        }
    }
    for (ri, r) in rows.iter().enumerate() {
        let total: f64 =
            r.iter().map(|&i| width(&row[i])).sum::<f64>() + gap * r.len().saturating_sub(1) as f64;
        let mut x = g.buttons_cx - total / 2.0;
        let y = g.buttons_bottom - bh - ri as f64 * (bh + g.vh);
        for &i in r {
            let w = width(&row[i]);
            placed.push((i, Rect::new(x, y, w, bh)));
            x += w + gap;
        }
    }
    let undo = (!u.click_answers && picks > 0)
        .then(|| Rect::new(69.0 * g.vw, 60.5 * g.vh, 14.0 * g.vh, 5.0 * g.vh));
    let refuse = assets.file_url("images/buttons/refuse.png");
    let slider = numbers.then(|| {
        let w = 35.0 * g.vh;
        Rect::new(g.buttons_cx - w / 2.0, g.buttons_bottom - bh, w, bh)
    });
    let m = crate::card::measure::measurer(assets.fonts);
    rsx! {
        for (i, r) in placed.iter() {
            {
                let b = row[*i].clone();
                let what = b.what.clone();
                let icon_url = b.icon.and_then(|k| {
                    let path = match k {
                        "1-card" | "2-card" | "3-card" | "1-coin" | "2-coin" | "3-coin"
                        | "action" | "2-action" | "buy" | "gold" => {
                            return assets.file_url(&format!("images/buttons/{k}.png"));
                        }
                        "treasures" => "images/buttons/autoplay-treasures.png",
                        "refuse" => "images/buttons/refuse.png",
                        "put-in-hand" => "images/buttons/put-in-hand.png",
                        "continue" => "images/buttons/continue.png",
                        "discard-icon" => "images/buttons/discard.png",
                        "trash-icon" => "images/buttons/trash.png",
                        "topdeck-icon" => "images/buttons/topdeck.png",
                        _ => return None,
                    };
                    assets.file_url(path)
                });
                let (target, options) = match &b.what {
                    Act::Target(t) => (Some(t.to_string()), Some("1".to_string())),
                    _ => (None, None),
                };
                let action = match &b.what {
                    Act::Treasures => Some("play-treasures"),
                    Act::SelectAll => Some("select-all"),
                    Act::Confirm => Some("confirm"),
                    Act::Decline => Some("decline"),
                    Act::Target(_) => None,
                };
                rsx! {
                    button {
                        key: "{i}{b.label}",
                        class: "dt-btn",
                        class: if b.on { "dt-on" },
                        class: if b.icon.is_some() { "dt-btn-icon" },
                        style: "{r.style()}",
                        disabled: b.disabled,
                        "data-target": target,
                        "data-options": options,
                        "data-action": action,
                        onclick: move |e| {
                            e.stop_propagation();
                            match &what {
                                Act::Target(t) => ctx.act(*t),
                                Act::Treasures => ctx.play_treasures(),
                                Act::SelectAll => ctx.select_all(),
                                Act::Confirm => ctx.confirm(),
                                Act::Decline => ctx.decline(),
                            }
                        },
                        span { class: "dt-btn-label",
                            for (k, (line, px)) in label_lines(&b.label, r.w, r.h, b.icon.is_some() || b.count.is_some(), m.as_ref()).into_iter().enumerate() {
                                span { key: "{k}", class: "dt-btn-line", style: "font-size:{px:.1}px;", "{line}" }
                            }
                        }
                        if let Some(src) = icon_url {
                            span { class: "dt-btn-img",
                                img { src, alt: "", draggable: "false" }
                                if let Some(c) = b.count.clone() {
                                    span { class: "dt-btn-count", "{c}" }
                                }
                            }
                        } else if let Some(c) = b.count.clone() {
                            span { class: "dt-btn-count", "{c}" }
                        }
                    }
                }
            }
        }
        if let Some(r) = slider {
            {
                let values: Vec<usize> = main.clone();
                let n = values.len();
                let pick = number().min(n.saturating_sub(1));
                let chosen = values.get(pick).copied();
                let label = chosen.map(|i| u.inter.targets[i].label.clone()).unwrap_or_default();
                rsx! {
                    div { class: "dt-slider", style: "{r.style()}",
                        "data-target-group": "number",
                        "data-options": "{n}",
                        input {
                            r#type: "range",
                            min: "0",
                            max: "{n.saturating_sub(1)}",
                            value: "{pick}",
                            oninput: move |e| number.set(e.value().parse().unwrap_or(0)),
                        }
                        button {
                            class: "dt-btn dt-slider-go",
                            disabled: busy || chosen.is_none(),
                            "data-target": chosen.map(|i| i.to_string()),
                            onclick: move |e| {
                                e.stop_propagation();
                                if let Some(i) = chosen {
                                    ctx.act(i);
                                }
                            },
                            "{label}"
                        }
                    }
                }
            }
        }
        if let Some(r) = undo {
            button {
                class: "dt-btn dt-undo",
                style: "{r.style()}",
                "data-action": "clear",
                onclick: move |e| { e.stop_propagation(); ctx.clear(); },
                span { class: "dt-btn-label", "Undo" }
                if let Some(src) = refuse.clone() {
                    span { class: "dt-btn-img", img { src, alt: "", draggable: "false" } }
                }
            }
        }
        for (k, &i) in side.iter().enumerate() {
            {
                let t = &u.inter.targets[i];
                let card = t.card.clone().unwrap_or_default();
                let r = Rect::new(g.side_right - side_w, g.side_top + k as f64 * (side_h + g.vh), side_w, side_h);
                let n = t.options.len();
                let art = assets
                    .card(&card)
                    .and_then(|f| f.art.as_deref())
                    .map(crate::assets::url);
                rsx! {
                    button {
                        key: "s{i}",
                        class: "dt-btn dt-ability",
                        style: "{r.style()}",
                        "data-target": "{i}",
                        "data-options": "{n}",
                        title: "{t.label}",
                        onclick: move |e| { e.stop_propagation(); ctx.act(i); },
                        if let Some(src) = art {
                            div { class: "dt-ability-art", style: "background-image:url('{src}');" }
                        } else if cards.get(&card).is_some() {
                            div { class: "dt-ability-art dt-ability-mini",
                                TableCard { name: card.clone(), size: CardSize::Mini }
                            }
                        }
                        span { class: "dt-ability-name", "{card}" }
                        if n > 1 {
                            span { class: "dt-btn-count", "{n}" }
                        }
                    }
                }
            }
        }
    }
}

fn option_text(ui: &Ui, index: u32) -> String {
    ui.decision
        .options
        .iter()
        .find(|o| o.index == index)
        .map(|o| o.card.clone().unwrap_or_else(|| o.label.clone()))
        .unwrap_or_default()
}

/// Puts a window's cards in `order` when they are all one card each of the cards looked
/// at, which lie in that order.
fn in_look_order(slots: &mut Vec<(usize, Option<usize>)>, targets: &[Target], order: &[String]) {
    let looked_at = |&(i, copy): &(usize, Option<usize>)| {
        let t = &targets[i];
        t.zone.as_deref() == Some("looking") && (copy.is_some() || t.options.len() == 1)
    };
    if slots.len() != order.len() || !slots.iter().all(looked_at) {
        return;
    }
    let mut left = slots.clone();
    let mut out = Vec::with_capacity(slots.len());
    for name in order {
        let Some(k) = left
            .iter()
            .position(|&(i, _)| targets[i].card.as_deref() == Some(name.as_str()))
        else {
            return;
        };
        out.push(left.remove(k));
    }
    *slots = out;
}

fn zone_text(zone: &str) -> &str {
    match zone {
        "deck" => "from deck",
        "discard" => "from discard",
        "hand" => "from hand",
        "inPlay" => "in play",
        "setAside" => "set aside",
        "trash" => "from trash",
        "pile" => "from a pile",
        other => other,
    }
}

/// The window of choices the table shows nowhere else, as dominion.games opens one over
/// the table for revealed or looked-at cards. Its ✕ hides it until the next decision.
#[component]
pub fn DecisionWindow(
    g: Rc<Geometry>,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    /// Cards looked at while answering a question about something else.
    #[props(default)]
    looking: Rc<Vec<String>>,
    /// The seat's cards looked at or revealed, in the order they were taken.
    #[props(default)]
    look_order: Rc<Vec<String>>,
) -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let mut hidden = use_signal(|| None::<String>);
    let mut query = use_signal(String::new);
    let Some(u) = ui else {
        // Waiting while another player picks among the cards this seat revealed.
        if looking.is_empty() || hidden().as_deref() == Some("") {
            return rsx! {};
        }
        return rsx! {
            LookingWindow { g, cards: looking, answer: None, low: false, onhide: move |_| hidden.set(Some(String::new())) }
        };
    };
    // Naming a card, dominion.games shows its input box alone, even for cards on no pile.
    let tray: Vec<usize> = (0..u.spots.len())
        .filter(|&i| u.spots[i] == Spot::Tray && !u.inter.name_any)
        .collect();
    let show_order = u.inter.ordered && sel.picks().len() > 1;
    if tray.is_empty() && !u.inter.name_any && !show_order && !looking.is_empty() {
        if hidden().as_deref() == Some(u.decision.id.as_str()) {
            return rsx! {};
        }
        let answer = wording::answered_by_cards(&u.decision)
            .and_then(|label| u.inter.targets.iter().position(|t| t.label == label));
        let id = u.decision.id.clone();
        return rsx! {
            LookingWindow {
                g,
                cards: looking,
                answer,
                low: u.piles_clickable(),
                onhide: move |_| hidden.set(Some(id.clone())),
            }
        };
    }
    if tray.is_empty() && !u.inter.name_any && !show_order {
        return rsx! {};
    }
    // Naming a card with every choice on the table: dominion.games' input box alone.
    if tray.is_empty() && !show_order {
        let q = query().trim().to_lowercase();
        let results: Vec<String> = if q.len() >= 2 {
            cards
                .nameable()
                .into_iter()
                .filter(|n| n.to_lowercase().contains(&q))
                .take(8)
                .collect()
        } else {
            vec![]
        };
        return rsx! {
            div { class: "dt-name-input", "data-target-group": "name",
                onclick: move |e| e.stop_propagation(),
                input {
                    class: "dt-name-entry",
                    placeholder: "name a card",
                    value: "{query}",
                    oninput: move |e| query.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            let q = query();
                            ctx.name(q.trim());
                        }
                    },
                }
                if !results.is_empty() {
                    div { class: "dt-name-gallery",
                        for n in results.iter() {
                            {
                                let name = n.clone();
                                rsx! {
                                    div {
                                        key: "{n}",
                                        class: "dt-result",
                                        "data-name-result": "{n}",
                                        onclick: move |_| ctx.name(&name),
                                        TableCard { name: n.clone(), size: CardSize::Micro, class: "dt-target dt-tone-name" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
    }
    if hidden().as_deref() == Some(u.decision.id.as_str()) {
        let (bw, bh) = g.button;
        let r = Rect::new(
            g.buttons_cx - bw / 2.0,
            g.buttons_bottom - 2.0 * bh - g.vh,
            bw,
            bh,
        );
        return rsx! {
            button {
                class: "dt-btn dt-tray-show",
                style: "{r.style()}",
                "data-action": "show-tray",
                onclick: move |e| { e.stop_propagation(); hidden.set(None); },
                span { class: "dt-btn-label", "Show choices" }
            }
        };
    }
    // Where the cards are goes without saying when they are all in one place.
    let zones: std::collections::HashSet<_> = tray
        .iter()
        .map(|&i| u.inter.targets[i].zone.as_deref())
        .collect();
    let one_zone = zones.len() == 1;
    // Cards picked from the trash: dominion.games opens the trash's own window.
    let from_trash = one_zone && zones.contains(&Some("trash"));
    // Each card drawn alone, unless the question is one dominion.games stacks.
    let stacked = from_trash
        || u.decision
            .source
            .as_deref()
            .is_some_and(|c| STACKED_WINDOWS.contains(&c));
    let mut slots: Vec<(usize, Option<usize>)> = tray
        .iter()
        .flat_map(|&i| {
            let t = &u.inter.targets[i];
            let n = t.options.len();
            if t.card.is_some() && n > 1 && !stacked {
                (0..n).map(|k| (i, Some(k))).collect::<Vec<_>>()
            } else {
                vec![(i, None)]
            }
        })
        .collect();
    // Cards to put onto a deck in order lie as they will: its bottom at the left.
    let ordering = u.inter.ordered;
    if ordering {
        slots.reverse();
    } else {
        // Cards looked at lie as they were taken, as dominion.games shows their zone.
        in_look_order(&mut slots, &u.inter.targets, &look_order);
    }
    let wide: Vec<bool> = slots
        .iter()
        .map(|&(i, _)| {
            u.inter.targets[i]
                .card
                .as_deref()
                .and_then(|c| cards.get(c))
                .is_some_and(crate::card::is_landscape)
        })
        .collect();
    let units: f64 = wide
        .iter()
        .map(|&w| if w { 2.0 } else { 1.0 })
        .sum::<f64>()
        .max(1.0);
    let mut win = g.window(units, u.piles_clickable());
    if from_trash {
        let w = g.tab_window(slots.len()).w;
        win = Rect::new(win.x + (win.w - w) / 2.0, win.y, w, win.h);
    }
    let extra = if u.inter.name_any || show_order {
        9.0 * g.vh
    } else {
        0.0
    };
    if extra > 0.0 {
        win.h += extra;
        win.y = (win.y - extra).max(0.0);
    }
    let cards_box = Rect::new(win.x, win.y, win.w, win.h - extra);
    let row = g.window_row(
        &cards_box,
        slots.len(),
        &wide,
        if from_trash { 0.15 } else { 0.0 },
    );
    let q = query().trim().to_lowercase();
    let results: Vec<String> = if u.inter.name_any && q.len() >= 2 {
        cards
            .nameable()
            .into_iter()
            .filter(|n| n.to_lowercase().contains(&q))
            .take(8)
            .collect()
    } else {
        vec![]
    };
    let picks = sel.picks().to_vec();
    let id = u.decision.id.clone();
    // Cards taken off a deck to look at are seen arriving here.
    let anchor = u
        .decision
        .player
        .filter(|_| {
            tray.iter()
                .any(|&i| u.inter.targets[i].zone.as_deref() == Some("looking"))
        })
        .map(|p| format!("zone:{p}:looking"));
    rsx! {
        div {
            class: "dt-window dt-tray",
            style: "{win.style()}",
            "data-tray": "1",
            "data-anchor": anchor,
            onclick: move |e| e.stop_propagation(),
            if from_trash {
                div { class: "dt-window-title", "Trash" }
            }
            button {
                class: "dt-x",
                title: "Hide (to see the table under it)",
                "data-action": "hide-tray",
                onclick: move |_| hidden.set(Some(id.clone())),
                "✕"
            }
            for (k, &(i, copy)) in slots.iter().enumerate() {
                {
                    let t = &u.inter.targets[i];
                    let mark = match copy {
                        Some(c) => u.mark_copy(&sel, i, c),
                        None => u.mark_target(&sel, i),
                    };
                    let ways = u.ways_of(Some(i));
                    let label = match (&t.card, &t.zone) {
                        _ if from_trash => None,
                        (Some(c), Some(z))
                            if &t.label == c
                                && !matches!(z.as_str(), "deck" | "hand" | "looking")
                                && !(one_zone && matches!(z.as_str(), "discard" | "trash")) =>
                        {
                            Some(zone_text(z).to_string())
                        }
                        (Some(c), _) if &t.label == c => None,
                        _ => Some(t.label.clone()),
                    };
                    let count = (copy.is_none() && t.options.len() > 1).then_some(t.options.len() as u32);
                    let w = if wide[k] { row.w * 2.0 } else { row.w };
                    let h = if wide[k] { w * super::layout::WIDE } else { row.h };
                    rsx! {
                        TableCard {
                            key: "{i}:{copy:?}",
                            name: t.card.clone().unwrap_or_default(),
                            size: CardSize::Full,
                            back: t.card.is_none(),
                            count,
                            mark,
                            ways,
                            label,
                            at: Rect::new(row.xs[k] - win.x, row.y - win.y + (row.h - h) / 2.0, w, h),
                            class: "dt-tslot",
                        }
                    }
                }
            }
            if ordering {
                span { class: "dt-window-end dt-window-bottom", "Bottom" }
                span { class: "dt-window-end dt-window-top", "Top" }
            }
            if show_order {
                div { class: "dt-order", "data-order": "1",
                    span { class: "dt-order-label", "Order (first is on top):" }
                    for (pos, &p) in picks.iter().enumerate() {
                        div {
                            key: "{p}",
                            class: "dt-order-chip",
                            "data-pos": "{pos}",
                            button {
                                class: "dt-arrow",
                                disabled: pos == 0,
                                "data-move": "left",
                                onclick: move |_| ctx.reorder(pos, pos - 1),
                                "◀"
                            }
                            span { "{pos + 1}. " CardName { name: option_text(&u, p) } }
                            button {
                                class: "dt-arrow",
                                disabled: pos + 1 == picks.len(),
                                "data-move": "right",
                                onclick: move |_| ctx.reorder(pos, pos + 1),
                                "▶"
                            }
                        }
                    }
                }
            }
            if u.inter.name_any {
                div { class: "dt-name-any", "data-target-group": "name",
                    input {
                        class: "dt-search",
                        placeholder: "Name a card…",
                        value: "{query}",
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                let q = query();
                                ctx.name(q.trim());
                            }
                        },
                    }
                    div { class: "dt-results",
                        for n in results.iter() {
                            {
                                let name = n.clone();
                                rsx! {
                                    div {
                                        key: "{n}",
                                        class: "dt-result",
                                        "data-name-result": "{n}",
                                        onclick: move |_| ctx.name(&name),
                                        TableCard { name: n.clone(), size: CardSize::Micro, class: "dt-target dt-tone-name" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Cards looked at or revealed, shown in a window of their own; a click on one answers
/// `answer`, where that is how dominion.games answers.
#[component]
fn LookingWindow(
    g: Rc<Geometry>,
    cards: Rc<Vec<String>>,
    answer: Option<usize>,
    low: bool,
    onhide: EventHandler<()>,
) -> Element {
    let ctx = use_table();
    let n = cards.len();
    let win = g.window(n as f64, low);
    let row = g.window_row(&win, n, &vec![false; n], 0.0);
    rsx! {
        div {
            class: "dt-window dt-tray",
            style: "{win.style()}",
            "data-tray": "1",
            onclick: move |e| e.stop_propagation(),
            button {
                class: "dt-x",
                title: "Hide (to see the table under it)",
                "data-action": "hide-tray",
                onclick: move |_| onhide.call(()),
                "✕"
            }
            for (k, card) in cards.iter().enumerate() {
                div {
                    key: "{k}",
                    class: if answer.is_some() { "dt-click" },
                    "data-target": answer.map(|i| i.to_string()),
                    onclick: move |_| {
                        if let Some(i) = answer {
                            ctx.act(i);
                        }
                    },
                    TableCard {
                        name: card.clone(),
                        size: CardSize::Full,
                        at: Rect::new(row.xs[k] - win.x, row.y - win.y, row.w, row.h),
                        class: "dt-tslot",
                    }
                }
            }
        }
    }
}
