//! The right-hand column: the log at the top (under it, our seat and view controls and
//! the panel with everything else the board carries), and the end of the game: the "The
//! game has ended." notice, then the score screen.

use super::layout::Geometry;
use super::pieces::{CardName, TableCard, seat_class};
use crate::app::{Route, use_cards};
use crate::card::CardSize;
use crate::game::board::{Board, Stack};
use crate::game::logfmt::{LogLine, Span, initials, log_lines};
use dioxus::prelude::*;
use dominion_protocol::LogEntry;
use std::rc::Rc;

#[component]
fn LogRow(line: LogLine, short: Vec<String>) -> Element {
    let l = &line;
    let name = |p: u8| {
        short
            .get(usize::from(p))
            .cloned()
            .unwrap_or_else(|| format!("P{}", p + 1))
    };
    let pad = f64::from(l.depth) / (f64::from(l.depth) + 10.0) * 50.0;
    rsx! {
        li {
            class: "dt-log-line dt-log-{l.kind}",
            class: if l.is_turn() { "dt-log-turn" },
            style: if l.depth > 0 { format!("padding-left:{pad:.1}%;") } else { String::new() },
            "data-seq": "{l.seq}",
            for (i, s) in l.spans.iter().enumerate() {
                match s {
                    Span::Text(t) => rsx! { span { key: "{i}", "{t}" } },
                    Span::Card(c) => rsx! { CardName { key: "{i}", name: c.clone() } },
                    Span::Cards(c, label) => rsx! { CardName { key: "{i}", name: c.clone(), label: label.clone() } },
                    Span::Player(p) => rsx! { span { key: "{i}", class: "dt-lp {seat_class(*p)}", "{name(*p)}" } },
                    Span::FullPlayer(p, n) => rsx! { span { key: "{i}", class: "dt-lp-full", "data-seat": "{p}", "{n}" } },
                }
            }
        }
    }
}

/// The log, following its end unless scrolled up.
#[component]
pub fn Log(
    entries: Vec<LogEntry>,
    names: Vec<String>,
    g: Rc<Geometry>,
    game_id: Option<String>,
    board: Rc<Board>,
) -> Element {
    let cards = use_cards();
    let lines = use_memo(use_reactive!(|entries, names| {
        log_lines(&entries, &names, &cards)
    }));
    let short = initials(&names);
    let mut stick = use_signal(|| true);
    let len = entries.len();
    use_effect(use_reactive!(|len| {
        let _ = len;
        if *stick.peek() {
            document::eval(
                "requestAnimationFrame(() => { const e = document.getElementById('dt-log'); if (e) e.scrollTop = e.scrollHeight; });",
            );
        }
    }));
    rsx! {
        div { class: "dt-log-wrap", style: "{g.log.style()}",
            ol {
                id: "dt-log",
                class: "dt-log",
                onscroll: move |e| {
                    let end = e.scroll_top() + f64::from(e.client_height()) >= f64::from(e.scroll_height()) - 100.0;
                    if end != *stick.peek() {
                        stick.set(end);
                    }
                },
                li { class: "dt-log-line dt-log-meta",
                    if let Some(id) = &game_id { "Game {id}, unrated." } else { "Game, unrated." }
                }
                li { class: "dt-log-line dt-log-meta dt-log-meta-last", "Timer: Off" }
                for l in lines.read().iter() {
                    LogRow { key: "{l.seq}", line: l.clone(), short: short.clone() }
                }
            }
        }
    }
}

/// How many entries the "more" panel holds.
fn extra_count(b: &Board) -> usize {
    b.lasting.len()
        + b.extra_turns.len()
        + b.notes.len()
        + b.counters.len()
        + b.uncertain.len()
        + b.you.as_ref().map_or(0, |y| y.uncertain.len())
        + usize::from(b.boons.is_some())
        + usize::from(b.hexes.is_some())
        + b.artifacts.len()
}

/// Lasting effects, extra turns, notes, counters, uncertain cards, Boons, Hexes and
/// Artifacts in words.
#[component]
pub fn More(board: Rc<Board>) -> Element {
    let mut open = use_signal(|| true);
    let b = &board;
    let n = extra_count(b);
    if n == 0 {
        return rsx! {};
    }
    let yours = b
        .you
        .as_ref()
        .map(|y| y.uncertain.clone())
        .unwrap_or_default();
    rsx! {
        div { class: "dt-more", "data-more": "1",
            button {
                class: "dt-more-head",
                onclick: move |e| { e.stop_propagation(); open.toggle(); },
                if open() { "▾ " } else { "▸ " }
                "Effects and more ({n})"
            }
            if open() {
                ul { class: "dt-more-list",
                    for (i, l) in b.lasting.iter().enumerate() {
                        li { key: "l{i}",
                            CardName { name: l.card.clone() }
                            " "
                            span { class: "dt-pc {seat_class(l.owner.seat)}", "({l.owner.name})" }
                            " {l.until}"
                            if !l.cards.is_empty() {
                                ": "
                                for (j, c) in l.cards.iter().enumerate() {
                                    CardName { key: "{j}", name: c.clone() }
                                    " "
                                }
                            }
                            if let Some(c) = l.count { " [{c}]" }
                        }
                    }
                    for (i, e) in b.extra_turns.iter().enumerate() {
                        li { key: "x{i}",
                            "Extra turn for "
                            span { class: "dt-pc {seat_class(e.player.seat)}", "{e.player.name}" }
                            " from "
                            CardName { name: e.source.clone() }
                        }
                    }
                    for (i, note) in b.notes.iter().enumerate() {
                        li { key: "n{i}", CardName { name: note.source.clone() } " — {note.text}" }
                    }
                    for (k, v) in b.counters.iter() {
                        li { key: "c{k}", "{k}: {v}" }
                    }
                    for (i, u) in b.uncertain.iter().chain(yours.iter()).enumerate() {
                        li { key: "u{i}",
                            "{u.count} of "
                            for s in u.candidates.iter() {
                                CardName { key: "{s.card}", name: s.card.clone(), label: format!("{} {}", s.count, s.card) }
                                " "
                            }
                            "are among {u.text}"
                        }
                    }
                    for (label, deck) in [("Boons", b.boons.clone()), ("Hexes", b.hexes.clone())] {
                        if let Some(d) = deck {
                            li { key: "{label}",
                                "{label}: {d.count} to draw"
                                if !d.discard.is_empty() { ", discarded: " }
                                for (j, c) in d.discard.iter().enumerate() {
                                    CardName { key: "{j}", name: c.clone() }
                                    " "
                                }
                            }
                        }
                    }
                    for a in b.artifacts.iter() {
                        li { key: "a{a.card}",
                            CardName { name: a.card.clone() }
                            match &a.holder {
                                Some(w) => rsx! { " held by " span { class: "dt-pc {seat_class(w.seat)}", "{w.name}" } },
                                None => rsx! { " unclaimed" },
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Where a player's points come from: the cards worth a fixed amount, VP tokens, and the
/// rest put on the cards whose worth varies.
fn vp_sources(
    owned: &[Stack],
    tokens: i32,
    total: i32,
    cards: &crate::cards::CardDb,
) -> Vec<(String, u32, i32)> {
    let mut out: Vec<(String, u32, i32)> = vec![];
    let mut fixed = 0;
    let mut varying: Vec<&Stack> = vec![];
    for s in owned {
        let Some(c) = cards.get(&s.card) else {
            continue;
        };
        if c.vp != 0 {
            let vp = c.vp * s.count as i32;
            fixed += vp;
            out.push((s.card.clone(), s.count, vp));
        } else if c.types.iter().any(|t| t == "Victory") {
            varying.push(s);
        }
    }
    if tokens != 0 {
        fixed += tokens;
        out.push((String::new(), tokens.unsigned_abs(), tokens));
    }
    let rest = total - fixed;
    match varying.as_slice() {
        [one] => out.push((one.card.clone(), one.count, rest)),
        _ => {
            for s in varying {
                out.push((s.card.clone(), s.count, 0));
            }
            if rest != 0 {
                out.push(("Other".into(), 0, rest));
            }
        }
    }
    out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The end of the game: dominion.games' "The game has ended." notice, and after its Ok
/// the score screen.
#[component]
pub fn GameOver(board: Rc<Board>) -> Element {
    let mut stage = use_signal(|| 0u8);
    let cards = use_cards();
    let Some(players) = board.result.clone() else {
        return rsx! {};
    };
    let mut players = players;
    players.sort_by_key(|p| (p.rank, p.index));
    match stage() {
        0 => {
            return rsx! {
                div { class: "dt-ended", "data-gameover": "1",
                    p { "The game has ended." }
                    for p in players.iter().filter(|p| p.resigned) {
                        p { key: "{p.index}", "{p.name} has resigned." }
                    }
                    button {
                        class: "dt-ok",
                        "data-action": "ok",
                        onclick: move |_| {
                            crate::platform::run_js("window.__dtResultSound && window.__dtResultSound(); window.__dtResultSound = null;");
                            stage.set(1);
                        },
                        "Ok"
                    }
                }
            };
        }
        2 => {
            return rsx! {
                button {
                    class: "dt-btn dt-show-result",
                    onclick: move |_| stage.set(1),
                    span { class: "dt-btn-label", "Score" }
                }
            };
        }
        _ => {}
    }
    let me = board.you.as_ref().map(|y| y.seat);
    let headline = me
        .and_then(|s| players.iter().find(|p| p.index == s))
        .map(|p| {
            let ties = players.iter().filter(|q| q.rank == p.rank).count() > 1;
            if p.resigned {
                "You Resigned"
            } else if p.rank == 1 && ties {
                "You Tie"
            } else if p.rank == 1 {
                "You Win"
            } else {
                "You Lose"
            }
        });
    let entries: Vec<_> = board.you.iter().chain(&board.opponents).cloned().collect();
    let game_id = crate::game::use_game().game.read().clone();
    let table = crate::lobby::use_table();
    let toast = crate::lobby::use_toast();
    let banned = crate::lobby::use_banned();
    let nav = navigator();
    let at_table = table
        .read()
        .as_ref()
        .is_some_and(|t| t.game.as_deref() == Some(game_id.as_str()));
    let mut focus_score = use_signal(|| true);
    let painting = match me.and_then(|s| players.iter().find(|p| p.index == s)) {
        None => "watch",
        Some(p) if p.rank > 1 => "lose",
        Some(_) => "win",
    };
    let selector = use_signal(|| false);
    let study = use_signal(|| None::<String>);
    rsx! {
        div { class: "lb-page lb-score-page {painting}", "data-gameover": "1",
            div { class: "lb-spear lb-spear-left" }
            div { class: "lb-spear lb-spear-right" }
            ul { class: "lb-tabs",
                li {
                    class: if focus_score() { "lb-tab selected" } else { "lb-tab" },
                    onclick: move |_| focus_score.set(true),
                    button { class: "lb-tab-button", "Score" }
                }
                if at_table {
                    li {
                        class: if focus_score() { "lb-tab" } else { "lb-tab selected" },
                        "data-tab": "edit-table",
                        onclick: move |_| focus_score.set(false),
                        button { class: "lb-tab-button", "Edit Table" }
                    }
                }
            }
            if focus_score() {
                div { class: "lb-score-panel dt-score-panel",
                if let Some(h) = headline {
                        h1 { "{h}" }
                    }
                    if let Some(r) = &board.end_reason {
                        p { class: "dt-note", "{r}" }
                    }
                    div { class: "dt-score-rows",
                        for p in players.iter() {
                            div { key: "{p.index}", class: "dt-score-row",
                                span { class: "dt-rank", "{p.rank}" }
                                span { class: "dt-score-name", "{p.name}" }
                                span { class: "dt-score-line" }
                                span { class: "dt-score-vp", "{p.vp}" }
                                span { class: "dt-score-turns", "Turns: {p.turns}" }
                                if p.resigned { span { class: "dt-note", " (resigned)" } }
                            }
                        }
                    }
                    for p in players.iter() {
                        {
                            let entry = entries.iter().find(|e| e.seat == p.index);
                            let owned = entry.map(|e| e.owned.known.clone()).unwrap_or_default();
                            let tokens = entry
                                .and_then(|e| e.badges.iter().find(|b| b.kind == "vp"))
                                .and_then(|b| b.count)
                                .unwrap_or(0);
                            let sources = vp_sources(&owned, tokens, p.vp, &cards);
                            let half = owned.len().div_ceil(2);
                            let (left, right) = owned.split_at(half);
                            rsx! {
                                fieldset { key: "f{p.index}", class: "dt-score-box",
                                    legend { "{p.name}" }
                                    div { class: "dt-score-sources",
                                        for (i, (card, n, vp)) in sources.iter().enumerate() {
                                            div { key: "{i}", class: "dt-score-source",
                                                if card.is_empty() {
                                                    span { class: "dt-strip-n", "{n}" }
                                                    span { class: "dt-strip-text", "VP tokens" }
                                                } else if card == "Other" {
                                                    span { class: "dt-strip-text", "Other" }
                                                } else {
                                                    div { class: "dt-strip",
                                                        TableCard { name: card.clone(), size: CardSize::Micro, count: Some(*n) }
                                                    }
                                                }
                                                span { class: "dt-score-line" }
                                                span { class: "dt-score-vp", "{vp}" }
                                            }
                                        }
                                    }
                                    div { class: "dt-score-deck",
                                        for col in [left, right] {
                                            div { class: "dt-score-col",
                                                for s in col.iter() {
                                                    div { key: "{s.card}", class: "dt-strip",
                                                        TableCard { name: s.card.clone(), size: CardSize::Micro, count: Some(s.count) }
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
            } else {
                div { class: "lb-score-table",
                    crate::lobby::my_table::KingdomOptions { table, selector, study }
                    crate::lobby::my_table::GameOptions { table }
                }
                if selector() {
                    div { class: "lb-selector-shield lb-score-shield", onclick: { let mut selector = selector; move |_| selector.set(false) } }
                    crate::lobby::my_table::CardSelectorFor { table, selector, study }
                }
                crate::lobby::my_table::StudyView { study }
            }
            if at_table {
                div { class: "lb-score-side",
                    crate::lobby::my_table::Players { table }
                }
            }
            div { class: "lb-score-buttons",
                if at_table {
                    button {
                        class: "lb-button lb-gold",
                        "data-action": "ready",
                        onclick: move |_| { spawn(crate::lobby::my_table::start(table, banned.clone(), toast, nav, false)); },
                        "Ready"
                    }
                }
                button {
                    class: "lb-button lb-copper",
                    "data-action": "leave-table",
                    onclick: move |_| {
                        if at_table {
                            let mut table = table;
                            table.set(None);
                        }
                        nav.push(Route::Lobby {});
                    },
                    "Leave Table"
                }
                button { class: "lb-button lb-stone", "data-action": "see-table", onclick: move |_| stage.set(2), "See the Table" }
            }
            if let Some(m) = toast() {
                div { class: "lb-status", role: "status", "{m}" }
            }
            crate::lobby::TooltipLayer {}
        }
    }
}
