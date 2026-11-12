//! The status bar (the turn player's Actions, Buys and coins over the prompt, the phase's
//! end button at its right end), the row of buttons above it, the column of ability
//! buttons at the right, and the window holding choices the table shows nowhere else:
//! revealed or looked-at cards, cards in decks and discard piles, named cards.

use super::layout::{Geometry, Rect};
use super::pieces::{CardName, TableCard, seat_class};
use super::{Spot, Ui, use_table};
use crate::app::use_cards;
use crate::assets::use_assets;
use crate::card::CardSize;
use crate::game::board::Board;
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
) -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let s = &board.status;
    let turn_player = s.player.as_ref().and_then(|w| {
        board
            .you
            .iter()
            .chain(&board.opponents)
            .find(|p| p.seat == w.seat)
    });
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
    let debt = s.debt.unwrap_or(0);
    let (coffers, villagers, favors) = (token("coffers"), token("villagers"), token("favors"));
    let ticker: Element = if !s.running {
        rsx! { "The game has ended." }
    } else if let Some(u) = ui.clone() {
        let text = wording::prompt(&u.decision, &cards);
        rsx! {
            span { "data-prompt": "{u.decision.purpose}", "{text}" }
        }
    } else {
        match &s.deciding {
            Some(w) => rsx! {
                "Waiting for "
                span { class: "dt-pc {seat_class(w.seat)}", "{w.name}" }
                "."
            },
            None => rsx! { "Waiting…" },
        }
    };
    // The phase's end, inside the bar's right end.
    let end = ui.as_ref().and_then(|u| {
        u.spots
            .iter()
            .enumerate()
            .find(|(i, sp)| **sp == Spot::Button && u.inter.targets[*i].action == "end-phase")
            .map(|(i, _)| i)
    });
    let (end_label, end_look) = wording::end_phase(s.phase.as_deref());
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
                    span { class: "dt-counter dt-icon dt-villagers", title: "Villagers", "{villagers}" }
                }
                span { class: "dt-sep" }
                span { class: "dt-counter dt-buys", class: if buys == 0 { "dt-zero" }, "{plural(buys, \"Buy\", \"Buys\")}" }
                span { class: "dt-sep" }
                span { class: "dt-counter dt-coin", title: "Coins", "{coins}" }
                if potions > 0 {
                    span { class: "dt-counter dt-icon dt-potion", title: "Potions", if potions > 1 { "{potions}" } }
                }
                if debt > 0 {
                    span { class: "dt-counter dt-icon dt-debt", title: "Debt", "{debt}" }
                }
                if coffers > 0 {
                    span { class: "dt-counter dt-icon dt-coffers", title: "Coffers", "{coffers}" }
                }
                if favors > 0 {
                    span { class: "dt-sep" }
                    span { class: "dt-counter dt-icon dt-favors", title: "Favors", "{favors}" }
                }
                span { class: "dt-sep" }
            }
            div { class: "dt-ticker",
                {ticker}
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
) -> Element {
    let ctx = use_table();
    let assets = use_assets();
    let cards = use_cards();
    let mut number = use_signal(|| 0usize);
    let Some(u) = ui else {
        return rsx! {};
    };
    let d = &u.decision;
    let main: Vec<usize> = (0..u.spots.len())
        .filter(|&i| u.spots[i] == Spot::Button && u.inter.targets[i].action != "end-phase")
        .collect();
    let side: Vec<usize> = (0..u.spots.len())
        .filter(|&i| u.spots[i] == Spot::Ability)
        .collect();
    let numbers = main.len() > MAX_NUMBER_BUTTONS
        && main.iter().all(|&i| u.inter.targets[i].action == "number");
    let picks = sel.picks().len();
    let mut row: Vec<Btn> = vec![];
    if !numbers {
        for &i in &main {
            let t = &u.inter.targets[i];
            let resolving =
                matches!(d.purpose.as_str(), "trigger" | "reaction") && t.action == "no";
            row.push(Btn {
                label: if resolving {
                    "Continue".into()
                } else {
                    t.label.clone()
                },
                icon: resolving.then_some("continue"),
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
        let icon = match d.purpose.as_str() {
            "discard" => Some("discard-icon"),
            "trash" => Some("trash-icon"),
            "top-deck" => Some("topdeck-icon"),
            _ => None,
        };
        row.push(Btn {
            label: wording::done(d).into(),
            icon,
            count: wording::done_count(d, picks),
            disabled: busy || !sel.is_legal(d),
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
    let width = |b: &Btn| if b.icon.is_some() { bw * 4.0 / 3.0 } else { bw };
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
    // With abilities to choose from, every button joins their column at the right.
    if !side.is_empty() {
        rows.clear();
        for (i, b) in row.iter().enumerate() {
            let w = width(b);
            let y = g.side_top + (side.len() + i) as f64 * (bh + g.vh);
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
    let side_w = 21.0 * g.vh;
    rsx! {
        for (i, r) in placed.iter() {
            {
                let b = row[*i].clone();
                let what = b.what.clone();
                let icon_url = b.icon.and_then(|k| {
                    let path = match k {
                        "treasures" => "images/buttons/autoplay-treasures.png",
                        "refuse" => "images/buttons/refuse.png",
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
                        span { class: "dt-btn-label", "{b.label}" }
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
                let r = Rect::new(g.side_right - side_w, g.side_top + k as f64 * (bh + g.vh), side_w, bh);
                let n = t.options.len();
                let art = cards.get(&card).is_some();
                rsx! {
                    button {
                        key: "s{i}",
                        class: "dt-btn dt-ability",
                        style: "{r.style()}",
                        "data-target": "{i}",
                        "data-options": "{n}",
                        title: "{t.label}",
                        onclick: move |e| { e.stop_propagation(); ctx.act(i); },
                        if art {
                            div { class: "dt-ability-art",
                                TableCard { name: card.clone(), size: CardSize::Mini }
                            }
                        }
                        span { class: "dt-btn-label", "{card}" }
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
pub fn DecisionWindow(g: Rc<Geometry>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let mut hidden = use_signal(|| None::<String>);
    let mut query = use_signal(String::new);
    let Some(u) = ui else {
        return rsx! {};
    };
    let tray: Vec<usize> = (0..u.spots.len())
        .filter(|&i| u.spots[i] == Spot::Tray)
        .collect();
    let show_order = u.inter.ordered && sel.picks().len() > 1;
    if tray.is_empty() && !u.inter.name_any && !show_order {
        return rsx! {};
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
    let wide: Vec<bool> = tray
        .iter()
        .map(|&i| {
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
    let row = g.window_row(&cards_box, tray.len(), &wide);
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
                onclick: move |_| hidden.set(Some(id.clone())),
                "✕"
            }
            for (k, &i) in tray.iter().enumerate() {
                {
                    let t = &u.inter.targets[i];
                    let mark = u.mark_target(&sel, i);
                    let label = match (&t.card, &t.zone) {
                        (Some(c), Some(z)) if &t.label == c && !matches!(z.as_str(), "deck" | "hand") => {
                            Some(zone_text(z).to_string())
                        }
                        (Some(c), _) if &t.label == c => None,
                        _ => Some(t.label.clone()),
                    };
                    let count = (t.options.len() > 1).then_some(t.options.len() as u32);
                    let w = if wide[k] { row.w * 2.0 } else { row.w };
                    let h = if wide[k] { w * super::layout::WIDE } else { row.h };
                    rsx! {
                        TableCard {
                            key: "{i}",
                            name: t.card.clone().unwrap_or_default(),
                            size: CardSize::Full,
                            back: t.card.is_none(),
                            count,
                            mark,
                            label,
                            at: Rect::new(row.xs[k] - win.x, row.y - win.y + (row.h - h) / 2.0, w, h),
                            class: "dt-tslot",
                        }
                    }
                }
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
