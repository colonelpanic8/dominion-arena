//! The right-hand column: the log, a panel with everything else the board carries, and
//! the score panel when the game is over.

use super::pieces::{HoverName, seat_class};
use crate::app::Route;
use crate::game::board::Board;
use crate::game::logfmt::{LogLine, Span, log_lines};
use dioxus::prelude::*;
use dominion_protocol::LogEntry;
use std::rc::Rc;

#[component]
fn LogRow(line: LogLine, names: Vec<String>) -> Element {
    let l = &line;
    let player_class = l.player.map(seat_class).unwrap_or_default();
    let name = |p: u8| {
        names
            .get(usize::from(p))
            .cloned()
            .unwrap_or_else(|| format!("P{}", p + 1))
    };
    rsx! {
        li {
            class: "dt-log-{l.kind} dt-depth-{l.depth} {player_class}",
            class: if l.is_turn() { "dt-log-turn" },
            "data-seq": "{l.seq}",
            for (i, s) in l.spans.iter().enumerate() {
                match s {
                    Span::Text(t) => rsx! { span { key: "{i}", "{t}" } },
                    Span::Card(c) => rsx! { HoverName { key: "{i}", name: c.clone() } },
                    Span::Player(p) => rsx! { span { key: "{i}", class: "dt-lp {seat_class(*p)}", "{name(*p)}" } },
                }
            }
            if l.count > 1 {
                span { class: "dt-times", " ×{l.count}" }
            }
        }
    }
}

/// The log, following its end unless scrolled up.
#[component]
pub fn Log(entries: Vec<LogEntry>, names: Vec<String>) -> Element {
    let lines = use_memo(use_reactive!(|entries, names| log_lines(&entries, &names)));
    let mut stick = use_signal(|| true);
    let mut reading = use_signal(|| false);
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
        div { class: "dt-log-wrap",
            ol {
                id: "dt-log",
                class: "dt-log",
                // Only the reader scrolling up stops it following the end, not the log
                // reflowing when the window is resized.
                onscroll: move |e| {
                    let end = e.scroll_top() + f64::from(e.client_height()) >= f64::from(e.scroll_height()) - 12.0;
                    let unstick = !end && *reading.peek();
                    if end && !*stick.peek() {
                        stick.set(true);
                    } else if unstick && *stick.peek() {
                        stick.set(false);
                    }
                },
                onwheel: move |_| reading.set(true),
                onmousedown: move |_| reading.set(true),
                ontouchstart: move |_| reading.set(true),
                onkeydown: move |_| reading.set(true),
                onmouseleave: move |_| reading.set(false),
                for l in lines.read().iter() {
                    LogRow { key: "{l.seq}", line: l.clone(), names: names.clone() }
                }
            }
            if !stick() {
                button {
                    class: "dt-log-end",
                    onclick: move |_| {
                        stick.set(true);
                        document::eval("const e = document.getElementById('dt-log'); if (e) e.scrollTop = e.scrollHeight;");
                    },
                    "↓ latest"
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
                            HoverName { name: l.card.clone() }
                            " "
                            span { class: seat_class(l.owner.seat), "({l.owner.name})" }
                            " {l.until}"
                            if !l.cards.is_empty() {
                                ": "
                                for (j, c) in l.cards.iter().enumerate() {
                                    HoverName { key: "{j}", name: c.clone() }
                                    " "
                                }
                            }
                            if let Some(c) = l.count { " [{c}]" }
                        }
                    }
                    for (i, e) in b.extra_turns.iter().enumerate() {
                        li { key: "x{i}",
                            "Extra turn for "
                            span { class: seat_class(e.player.seat), "{e.player.name}" }
                            " from "
                            HoverName { name: e.source.clone() }
                        }
                    }
                    for (i, note) in b.notes.iter().enumerate() {
                        li { key: "n{i}", HoverName { name: note.source.clone() } " — {note.text}" }
                    }
                    for (k, v) in b.counters.iter() {
                        li { key: "c{k}", "{k}: {v}" }
                    }
                    for (i, u) in b.uncertain.iter().enumerate() {
                        li { key: "u{i}",
                            "{u.count} of "
                            for s in u.candidates.iter() {
                                HoverName { key: "{s.card}", name: s.card.clone(), label: format!("{} {}", s.count, s.card) }
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
                                    HoverName { key: "{j}", name: c.clone() }
                                    " "
                                }
                            }
                        }
                    }
                    for a in b.artifacts.iter() {
                        li { key: "a{a.card}",
                            HoverName { name: a.card.clone() }
                            match &a.holder {
                                Some(w) => rsx! { " held by " span { class: seat_class(w.seat), "{w.name}" } },
                                None => rsx! { " unclaimed" },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn GameOver(board: Rc<Board>) -> Element {
    let mut hidden = use_signal(|| false);
    let Some(players) = board.result.clone() else {
        return rsx! {};
    };
    let mut players = players;
    players.sort_by_key(|p| (p.rank, p.index));
    if hidden() {
        return rsx! {
            button {
                class: "dt-btn dt-btn-gold dt-show-result",
                onclick: move |_| hidden.set(false),
                "Show the result"
            }
        };
    }
    rsx! {
        div { class: "dt-gameover", "data-gameover": "1",
            div { class: "dt-gameover-panel",
                h2 { "Game over" }
                if let Some(r) = &board.end_reason {
                    p { class: "dt-note", "{r}" }
                }
                table {
                    thead { tr { th { "" } th { "Player" } th { "VP" } th { "Turns" } } }
                    tbody {
                        for p in players.iter() {
                            tr { key: "{p.index}", class: if p.rank == 1 { "dt-winner" },
                                td { class: "dt-rank", "{p.rank}" }
                                td { span { class: seat_class(p.index), "{p.name}" } if p.resigned { " (resigned)" } }
                                td { class: "dt-vpcell", "{p.vp}" }
                                td { "{p.turns}" }
                            }
                        }
                    }
                }
                div { class: "dt-gameover-actions",
                    button { class: "dt-btn dt-btn-stone", onclick: move |_| hidden.set(true), "See the table" }
                    Link { class: "dt-btn dt-btn-gold", to: Route::Lobby {}, "Back to the lobby" }
                }
            }
        }
    }
}
