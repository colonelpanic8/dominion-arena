//! Finished games: the History tab, and the list of games and a game's sheet that a
//! player's page shares with it. Everything here comes from `/api/history` and
//! `/api/users/{name}` as the protocol's `PastGame`s.

use super::rules::{Pick, TableRules, Ternary, is_landscape};
use super::who::{SeatIcon, agent_title, caption};
use super::{
    Fieldset, LobbyCtx, LobbyFrame, Oval, Tab, clock, create_table, day, flash, store, use_lobby,
    use_toast,
};
use crate::account::{Account, use_account};
use crate::api;
use crate::app::{Route, use_cards};
use crate::assets::{self, use_assets};
use crate::card::{CardHover, CardSize, CardView};
use crate::platform;
use dioxus::prelude::*;
use dioxus::router::Navigator;
use dominion_protocol::api::CardInfo;
use dominion_protocol::lobby::{
    Arranged, HeadToHeadQuery, HistoryQuery, Matchup, PastGame, PastPlayer, PlayedBy, Venue,
};

/// Games asked for at a time.
const PAGE: usize = 30;

#[component]
pub fn HistoryTab() -> Element {
    let account = use_account();
    let me = account.me.read().as_ref().map(|m| m.user.name.clone());
    rsx! {
        LobbyFrame { tab: Tab::History,
            GameBrowser { title: "Game History", me }
        }
    }
}

/// Finished games beside the sheet of the one picked: everyone's (`me` may narrow them
/// to their own), or only `user`'s, told from their side.
#[component]
pub fn GameBrowser(
    title: String,
    #[props(default)] me: Option<String>,
    #[props(default)] user: Option<String>,
    #[props(default = VNode::empty())] top: Element,
) -> Element {
    let mut venue = use_signal(|| None::<Venue>);
    let mut mine = use_signal(|| false);
    let mut rated = use_signal(|| false);
    let mut voided = use_signal(|| false);
    let mut text = use_signal(String::new);
    let mut picked = use_signal(|| None::<String>);
    let mut more = use_signal(Vec::<PastGame>::new);
    let mut more_next = use_signal(|| None::<Option<u64>>);
    // A player's page only: its Head to head tab, with a Rated switch and search of its
    // own, and the opponent a pick there narrows the games to (key, name).
    let mut rivals = use_signal(|| false);
    let mut rivals_rated = use_signal(|| true);
    let mut who = use_signal(String::new);
    let mut vs = use_signal(|| None::<(String, String)>);
    let fixed = user.clone();
    let me_now = me.clone();
    let query = use_memo(move || HistoryQuery {
        user: fixed.clone().or_else(|| me_now.clone().filter(|_| mine())),
        venue: venue(),
        rated: rated(),
        voided: voided(),
        q: Some(text()).filter(|t| !t.trim().is_empty()),
        vs: vs().map(|(key, _)| key),
        limit: Some(PAGE),
        before: None,
    });
    let first = use_resource(move || {
        let q = query();
        more.set(vec![]);
        more_next.set(None);
        async move { api::history(&q).await }
    });
    let (games, next, error) = match &*first.read() {
        Some(Ok(h)) => {
            let mut games = h.games.clone();
            games.extend(more.read().iter().cloned());
            (games, more_next().unwrap_or(h.next), None)
        }
        Some(Err(e)) => (vec![], None, Some(e.message.clone())),
        None => (vec![], None, None),
    };
    let load_more = move |_| {
        let Some(before) = next else {
            return;
        };
        let mut q = query();
        q.before = Some(before);
        spawn(async move {
            if let Ok(h) = api::history(&q).await {
                more.write().extend(h.games);
                more_next.set(Some(h.next));
            }
        });
    };
    let shown = (picked.read().as_ref())
        .and_then(|id| games.iter().find(|g| &g.id == id))
        .or(games.first())
        .cloned();
    let mut last_day = String::new();
    let items: Vec<(Option<String>, PastGame, bool)> = (games.iter())
        .map(|g| {
            let label = day(g.ended);
            let heading = (label != last_day).then(|| label.clone());
            last_day = label;
            let on = shown.as_ref().is_some_and(|s| s.id == g.id);
            (heading, g.clone(), on)
        })
        .collect();
    rsx! {
        div { class: "lb-history",
            div { class: "lb-history-side",
                {top}
                Fieldset {
                    class: "lb-history-games",
                    title: if user.is_some() { String::new() } else { title.clone() },
                    after_title: rsx! {
                        if user.is_some() {
                            span { class: "lb-pane-tabs", role: "tablist",
                                span {
                                    class: if rivals() { "lb-pane-tab" } else { "lb-pane-tab on" },
                                    role: "tab",
                                    onclick: move |_| rivals.set(false),
                                    "{title}"
                                }
                                span {
                                    class: if rivals() { "lb-pane-tab on" } else { "lb-pane-tab" },
                                    role: "tab",
                                    onclick: move |_| rivals.set(true),
                                    "Head to head"
                                }
                            }
                        }
                    },
                    div { class: "lb-history-filters",
                        div { class: "lb-multibutton",
                            for (v, label) in [(None, "All"), (Some(Venue::Arena), "Arena"), (Some(Venue::Official), "dominion.games")] {
                                Oval {
                                    key: "{label}",
                                    active: venue() == v,
                                    "data-venue": "{label}",
                                    onclick: move |_| venue.set(v),
                                    "{label}"
                                }
                            }
                        }
                    }
                    div { class: "lb-history-filters",
                        if user.is_none() && me.is_some() {
                            div { class: "lb-multibutton",
                                for (m, label) in [(true, "Mine"), (false, "Everyone")] {
                                    Oval {
                                        key: "{label}",
                                        active: mine() == m,
                                        onclick: move |_| mine.set(m),
                                        "{label}"
                                    }
                                }
                            }
                        }
                        if rivals() {
                            label { class: "lb-history-rated",
                                super::Switch { on: rivals_rated(), onclick: move |_| rivals_rated.toggle() }
                                "Rated"
                            }
                            input {
                                class: "lb-input lb-history-search",
                                r#type: "search",
                                placeholder: "Opponent",
                                "aria-label": "Find an opponent",
                                value: "{who}",
                                oninput: move |e| who.set(e.value()),
                            }
                        } else {
                            label { class: "lb-history-rated",
                                super::Switch { on: rated(), onclick: move |_| rated.toggle() }
                                "Rated"
                            }
                            label { class: "lb-history-rated", title: "Show voided games too",
                                super::Switch { on: voided(), onclick: move |_| voided.toggle() }
                                "Voided"
                            }
                            input {
                                class: "lb-input lb-history-search",
                                r#type: "search",
                                placeholder: "Player, agent or card",
                                "aria-label": "Find a player, agent or card",
                                value: "{text}",
                                oninput: move |e| text.set(e.value()),
                            }
                        }
                    }
                    if let (Some(name), false, Some((_, against))) = (&user, rivals(), vs()) {
                        div { class: "lb-h2h-chip", "data-vs": "{against}",
                            "{name} vs "
                            b { "{against}" }
                            button {
                                class: "lb-h2h-clear",
                                title: "Every game",
                                "aria-label": "Show every game",
                                onclick: move |_| {
                                    vs.set(None);
                                    picked.set(None);
                                },
                                "✕"
                            }
                        }
                    }
                    if let (Some(name), true) = (&user, rivals()) {
                        Rivals {
                            name: name.clone(),
                            venue: venue(),
                            rated: rivals_rated(),
                            who: who(),
                            onpick: move |m: Matchup| {
                                vs.set(Some((m.key, m.seat.name)));
                                rated.set(rivals_rated());
                                text.set(String::new());
                                picked.set(None);
                                rivals.set(false);
                            },
                        }
                    }
                    div { class: "lb-list lb-history-list", display: if rivals() { "none" },
                        if let Some(e) = error {
                            p { class: "lb-error", "{e}" }
                        } else if games.is_empty() && first.read().is_some() {
                            p { class: "lb-note", "No finished games match." }
                        }
                        for (heading, g, on) in items {
                            div { key: "{g.id}", class: "lb-history-item",
                                if let Some(h) = heading {
                                    div { class: "lb-list-head lb-history-day", "{h}" }
                                }
                                Entry {
                                    game: g.clone(),
                                    on,
                                    onpick: {
                                        let id = g.id.clone();
                                        move |_| picked.set(Some(id.clone()))
                                    },
                                }
                            }
                        }
                        if next.is_some() {
                            div { class: "lb-oval lb-history-more", onclick: load_more, "More games" }
                        }
                    }
                }
            }
            match shown {
                Some(g) => rsx! { Sheet { key: "{g.id}", game: g, side: user.clone() } },
                None => rsx! { div { class: "lb-sheet" } },
            }
        }
    }
}

/// Player `name`'s record against each opponent, most games together first; a pick
/// narrows the Games tab to the games against them.
#[component]
fn Rivals(
    name: String,
    venue: Option<Venue>,
    rated: bool,
    who: String,
    onpick: EventHandler<Matchup>,
) -> Element {
    let found = use_resource(use_reactive!(|name, venue, rated| async move {
        api::head_to_head(&name, &HeadToHeadQuery { venue, rated }).await
    }));
    let who = who.trim().to_lowercase();
    let shown: Vec<Matchup> = match &*found.read() {
        Some(Ok(h)) => (h.opponents.iter())
            .filter(|m| who.is_empty() || m.seat.name.to_lowercase().contains(&who))
            .cloned()
            .collect(),
        _ => vec![],
    };
    let total = match &*found.read() {
        Some(Ok(h)) => h.opponents.len(),
        _ => 0,
    };
    let note = match &*found.read() {
        None => None,
        Some(Err(e)) => Some(e.message.clone()),
        Some(Ok(_)) if total == 0 && rated => {
            Some("No rated games yet. Turn off Rated to count every game.".to_string())
        }
        Some(Ok(_)) if total == 0 => Some("No finished games against anyone yet.".to_string()),
        Some(Ok(_)) if shown.is_empty() => Some("No opponent matches.".to_string()),
        Some(Ok(_)) => None,
    };
    rsx! {
        div { class: "lb-h2h-line lb-h2h-head lb-list-head",
            span {}
            span { "Opponent" }
            span { "Won – Lost – Tied" }
            span { class: "lb-h2h-num", "Win %" }
            span { class: "lb-h2h-num", "Games" }
        }
        div { class: "lb-list lb-h2h-list",
            if let Some(n) = note {
                p { class: "lb-note", "{n}" }
            }
            for m in shown {
                Rival { key: "{m.key}", matchup: m.clone(), onpick }
            }
        }
        if total > 0 {
            div { class: "lb-h2h-foot",
                if total == 1 { "1 opponent" } else { "{total} opponents" }
            }
        }
    }
}

/// One opponent's row: who they are, the record as numbers and a bar, the share won.
#[component]
fn Rival(matchup: Matchup, onpick: EventHandler<Matchup>) -> Element {
    let Matchup {
        seat,
        games,
        wins,
        losses,
        ties,
        last,
        ..
    } = matchup.clone();
    let share = |n: u32| {
        format!(
            "width: {:.2}%",
            f64::from(n) * 100.0 / f64::from(games.max(1))
        )
    };
    let won = (f64::from(wins) + f64::from(ties) / 2.0) * 100.0 / f64::from(games.max(1));
    let tone = match won {
        w if w >= 60.0 => "lb-h2h-num lb-h2h-pct high",
        w if w <= 40.0 => "lb-h2h-num lb-h2h-pct low",
        _ => "lb-h2h-num lb-h2h-pct",
    };
    let met = format!("Last met {} at {}", day(last).to_lowercase(), clock(last));
    let pick = matchup.clone();
    let key = matchup.clone();
    rsx! {
        div {
            class: "lb-h2h-line lb-h2h-row",
            role: "button",
            tabindex: "0",
            title: "{met}",
            "data-opponent": "{matchup.key}",
            onclick: move |_| onpick.call(pick.clone()),
            onkeydown: move |e: KeyboardEvent| {
                if matches!(e.key(), Key::Enter) || e.key() == Key::Character(" ".into()) {
                    e.prevent_default();
                    onpick.call(key.clone());
                }
            },
            SeatIcon { played_by: seat.played_by.clone() }
            span { class: "lb-h2h-who",
                match seat.account.clone() {
                    Some(name) => rsx! {
                        span {
                            onclick: |e| e.stop_propagation(),
                            onkeydown: |e| e.stop_propagation(),
                            Link { class: "lb-h2h-name", to: Route::User { name }, "{seat.name}" }
                        }
                    },
                    None => rsx! { span { class: "lb-h2h-name", "{seat.name}" } },
                }
                span { class: "lb-caption", "{caption(&seat)}" }
            }
            span { class: "lb-h2h-record",
                span { class: "lb-h2h-score",
                    span { class: "won", "{wins}" }
                    span { class: "dash", "–" }
                    span { class: "lost", "{losses}" }
                    if ties > 0 {
                        span { class: "dash", "–" }
                        span { class: "tied", "{ties}" }
                    }
                }
                span { class: "lb-h2h-bar",
                    span { class: "won", style: share(wins) }
                    span { class: "tied", style: share(ties) }
                    span { class: "lost", style: share(losses) }
                }
            }
            span { class: tone, "{won.round()}%" }
            span { class: "lb-h2h-num lb-h2h-games", "{games}" }
        }
    }
}

/// The seat `name`'s account played.
pub fn seat_of<'a>(game: &'a PastGame, name: &str) -> Option<&'a PastPlayer> {
    let theirs = |p: &&PastPlayer| {
        p.account
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(name))
    };
    game.players.iter().find(theirs)
}

/// The players by place, ties by seat.
/// The players in seat order, first player first.
fn by_seat(game: &PastGame) -> Vec<&PastPlayer> {
    let mut players: Vec<&PastPlayer> = game.players.iter().collect();
    players.sort_by_key(|p| p.seat);
    players
}

fn standings(game: &PastGame) -> Vec<&PastPlayer> {
    let mut players: Vec<&PastPlayer> = game.players.iter().collect();
    players.sort_by_key(|p| (p.rank, p.seat));
    players
}

/// The seat an official account played the game from.
fn official_seat(game: &PastGame) -> Option<&PastPlayer> {
    let ours = |p: &&PastPlayer| p.played_by != PlayedBy::Official;
    (game.venue == Venue::Official)
        .then(|| game.players.iter().find(ours))
        .flatten()
}

/// How it ended, in words: the server's `provinces-gone` as "Provinces gone".
pub fn reason(code: &str) -> String {
    let words = code.replace('-', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

const VOIDED: &str = "Voided: taken out of the record, it counts for no rating or tally";

/// Where and how a game was played, in a few words.
fn how(game: &PastGame) -> String {
    let mut parts = vec![];
    match game.venue {
        Venue::Arena => parts.push("Arena".to_string()),
        Venue::Official => parts.push("dominion.games".to_string()),
    }
    match (game.arranged, official_seat(game)) {
        (Some(Arranged::Automatch), _) => parts.push("Automatch".into()),
        (Some(Arranged::Table), _) => parts.push("Table".into()),
        (Some(Arranged::Tokens), _) => parts.push("By token".into()),
        (Some(Arranged::Agents), _) => parts.push("Agents".into()),
        (None, Some(p)) => parts.push(format!("as {}", p.name)),
        (None, None) => {}
    }
    // A voided game says so in a badge of its own.
    match (game.rated, game.voided) {
        (_, Some(_)) => {}
        (true, None) => parts.push("Rated".into()),
        (false, None) => parts.push("Unrated".into()),
    }
    parts.join(" · ")
}

/// One game in the list: the winner and the score, or the side's result.
#[component]
fn Entry(game: PastGame, on: bool, onpick: EventHandler<()>) -> Element {
    let seated = by_seat(&game);
    let class = if on {
        "lb-history-entry on"
    } else {
        "lb-history-entry"
    };
    let (venue, venue_name) = match game.venue {
        Venue::Arena => ("arena", "Played on Arena"),
        Venue::Official => ("official", "Played on dominion.games"),
    };
    let (first, rest) = match seated.split_first() {
        Some((f, r)) => (Some((*f).clone()), r.iter().map(|p| (*p).clone()).collect()),
        None => (None, vec![]),
    };
    let rest: Vec<PastPlayer> = rest;
    let arena = game.venue == Venue::Arena;
    rsx! {
        div {
            class,
            role: "button",
            tabindex: "0",
            "data-game": "{game.id}",
            "data-venue": venue,
            "data-voided": game.voided.is_some().then_some("true"),
            onclick: move |_| onpick.call(()),
            onkeydown: move |e: KeyboardEvent| {
                if matches!(e.key(), Key::Enter) || e.key() == Key::Character(" ".into()) {
                    e.prevent_default();
                    onpick.call(());
                }
            },
            span { class: "lb-history-venue", title: venue_name }
            span { class: "lb-history-entry-body",
                span { class: "lb-history-entry-line",
                    span { class: "lb-entry-side",
                        if let Some(p) = first {
                            Seated { won: p.rank == 1, arena, player: p }
                        }
                    }
                    span { class: "lb-history-score",
                        for (i, p) in seated.iter().enumerate() {
                            span { key: "{p.seat}", class: if p.rank == 1 { "lb-score-won" } else { "lb-score-lost" },
                                if i > 0 { "–" }
                                "{p.vp}"
                            }
                        }
                    }
                    span { class: "lb-entry-side right",
                        for p in rest.iter() {
                            Seated { key: "{p.seat}", player: p.clone(), won: p.rank == 1, arena, right: true }
                        }
                    }
                }
                span { class: "lb-history-how",
                    span {
                        if game.voided.is_some() {
                            span { class: "lb-voided", title: VOIDED, "Voided" }
                        }
                        "{how(&game)}"
                    }
                    span { class: "lb-history-time", "{clock(game.ended)}" }
                }
                KingdomPeek {
                    kingdom: game.kingdom.clone(),
                    landscapes: game.landscapes.clone(),
                    others: game.others.clone(),
                }
            }
        }
    }
}

/// The colour a card's thumbnail is framed in without its art: its main type's.
fn tone(card: Option<&CardInfo>) -> &'static str {
    let Some(card) = card else {
        return "action";
    };
    if is_landscape(card) {
        return "landscape";
    }
    let has = |t: &str| card.types.iter().any(|x| x == t);
    if has("Night") {
        "night"
    } else if has("Duration") {
        "duration"
    } else if has("Reaction") {
        "reaction"
    } else if has("Treasure") && !has("Action") {
        "treasure"
    } else if has("Victory") && !has("Action") {
        "victory"
    } else {
        "action"
    }
}

/// A kingdom as a strip of tiny pictures of its cards' art, which shows the whole of it
/// while the pointer is over it.
#[component]
fn KingdomPeek(kingdom: Vec<String>, landscapes: Vec<String>, others: Vec<String>) -> Element {
    let cards = use_cards();
    let art = use_assets();
    let mut at = use_signal(|| None::<(f64, f64)>);
    let popup = at().map(|(x, y)| {
        let (vw, vh) = platform::viewport();
        let lu = (vh / 100.0).min(vw / 200.0);
        let margin = 8.0;
        let side = if landscapes.is_empty() {
            0.0
        } else {
            16.5 * lu
        };
        let width = 5.0 * 10.6 * lu + side + 3.0 * lu;
        let also = if others.is_empty() { 0.0 } else { 12.6 * lu };
        let height = 2.0 * 10.4 * lu + also + 3.0 * lu;
        let mut left = x + 24.0;
        if left + width > vw - margin {
            left = x - 24.0 - width;
        }
        let left = left.min(vw - width - margin).max(margin);
        let top = (y - height / 2.0).min(vh - height - margin).max(margin);
        format!("left:{left:.0}px;top:{top:.0}px;")
    });
    rsx! {
        span {
            class: "lb-thumbs",
            onmouseenter: move |e: MouseEvent| {
                let p = e.client_coordinates();
                at.set(Some((p.x, p.y)));
            },
            onmouseleave: move |_| at.set(None),
            for name in kingdom.iter().chain(&landscapes).chain(&others) {
                {
                    let card = cards.get(name);
                    let wide = card.is_some_and(is_landscape);
                    let picture = (art.card(name))
                        .and_then(|f| f.mini_art.as_deref().or(f.art.as_deref()))
                        .map(|path| format!("background-image: url('{}');", assets::url(path)))
                        .unwrap_or_default();
                    rsx! {
                        span {
                            key: "{name}",
                            class: if wide { "lb-thumb wide {tone(card)}" } else { "lb-thumb {tone(card)}" },
                            style: picture,
                        }
                    }
                }
            }
            if let Some(style) = popup {
                div { class: "lb-peek-pop", style,
                    KingdomCards {
                        kingdom: kingdom.clone(),
                        landscapes: landscapes.clone(),
                        others: others.clone(),
                    }
                }
            }
        }
    }
}

/// The winner's mark, before their name.
#[component]
fn Crown() -> Element {
    rsx! {
        svg { class: "lb-crown", view_box: "0 0 16 16", role: "img", "aria-label": "won",
            path { d: "M2.5 12.5h11M3 12.5 2 5l3.5 3L8 3l2.5 5L14 5l-1 7.5" }
        }
    }
}

/// A seat in the list: an icon for what played it, its name (linking to its Arena
/// account's page), and under it a caption.
#[component]
fn Seated(
    player: PastPlayer,
    #[props(default)] won: bool,
    #[props(default)] arena: bool,
    /// On the right of the line: aligned to it, its icon outermost.
    #[props(default)]
    right: bool,
) -> Element {
    let class = match (won, right) {
        (true, true) => "lb-seat won right",
        (true, false) => "lb-seat won",
        (false, true) => "lb-seat right",
        (false, false) => "lb-seat",
    };
    let account = player.account.clone().filter(|_| arena);
    rsx! {
        span { class,
            SeatIcon { played_by: player.played_by.clone() }
            span { class: "lb-seat-text",
                span { class: "lb-seat-name",
                    if won {
                        Crown {}
                    }
                    match account {
                        Some(name) => rsx! {
                            span {
                                onclick: |e| e.stop_propagation(),
                                onkeydown: |e| e.stop_propagation(),
                                Link { class: "lb-seat-link", to: Route::User { name }, "{player.name}" }
                            }
                        },
                        None => rsx! { "{player.name}" },
                    }
                }
                span { class: "lb-caption lb-seat-caption", "{caption(&player)}" }
            }
        }
    }
}

/// A kingdom as the table lays it out: its piles in two rows, its landscapes beside them
/// and what else it was set up with under them, headed "Also in the game"; each card in
/// full while hovered.
#[component]
fn KingdomCards(kingdom: Vec<String>, landscapes: Vec<String>, others: Vec<String>) -> Element {
    rsx! {
        div { class: "lb-setup-cards",
            div { class: "lb-sheet-kingdom",
                div { class: "lb-sheet-piles",
                    for name in kingdom.iter() {
                        KingdomCard { key: "{name}", name: name.clone() }
                    }
                }
                if !landscapes.is_empty() {
                    div { class: "lb-sheet-landscapes",
                        for name in landscapes.iter() {
                            KingdomCard { key: "{name}", name: name.clone(), wide: true }
                        }
                    }
                }
            }
            if !others.is_empty() {
                div { class: "lb-setup-also",
                    span { class: "lb-setup-label", "Also in the game" }
                    div { class: "lb-setup-others",
                        for name in others.iter() {
                            KingdomCard { key: "{name}", name: name.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn KingdomCard(name: String, #[props(default)] wide: bool) -> Element {
    let class = if wide { "lb-lcard" } else { "lb-kcard" };
    rsx! {
        CardHover { name: name.clone(),
            div { class, CardView { name, size: CardSize::Mini } }
        }
    }
}

/// Who played a seat, beyond its name, and the account to link to.
fn played_by(game: &PastGame, p: &PastPlayer) -> (String, Option<String>) {
    let official = game.venue == Venue::Official;
    match &p.played_by {
        PlayedBy::Agent(a) => (agent_title(a), Some(a.agent.clone())),
        PlayedBy::Bot { owner: Some(owner) } => (format!("{owner}'s bot"), Some(owner.clone())),
        PlayedBy::Bot { owner: None } => ("A user's bot".into(), None),
        PlayedBy::Person if official => match &p.account {
            Some(account) => (account.clone(), Some(account.clone())),
            None => ("Registered player".into(), None),
        },
        PlayedBy::Person => ("Registered player".into(), None),
        PlayedBy::Guest => ("Guest".into(), None),
        PlayedBy::Token => ("Seat token".into(), None),
        PlayedBy::Official => ("dominion.games player".into(), None),
    }
}

/// A game's sheet: what it was, its kingdom, how everyone did, and what can be done with it.
#[component]
fn Sheet(game: PastGame, side: Option<String>) -> Element {
    let ctx = use_lobby();
    let account = use_account();
    let toast = use_toast();
    let nav = navigator();
    let ranked = standings(&game);
    let mine = side.as_deref().and_then(|s| seat_of(&game, s));
    let winners: Vec<&PastPlayer> = ranked.iter().copied().filter(|p| p.rank == 1).collect();
    let losers = ranked
        .iter()
        .filter(|p| p.rank != 1)
        .map(|p| p.vp.to_string())
        .collect::<Vec<_>>();
    let title = match winners.first() {
        _ if winners.len() > 1 => {
            let names = winners
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(" and ");
            format!("{names} tied, {}", winners[0].vp)
        }
        Some(w) if losers.is_empty() => format!("{} won, {}", w.name, w.vp),
        Some(w) => format!("{} won, {} to {}", w.name, w.vp, losers.join(" and ")),
        None => String::new(),
    };
    let turns = game.players.iter().map(|p| p.turns).max().unwrap_or(0);
    let mut ending = reason(&game.reason);
    if turns > 0 {
        ending.push_str(&format!(", turn {turns}"));
    }
    let minutes = game.ended.saturating_sub(game.started) / 60_000;
    let mut meta = format!("{} {} · {}", day(game.ended), clock(game.ended), how(&game));
    if minutes > 0 {
        meta.push_str(&format!(" · {minutes} min"));
    }
    let venue_class = match game.venue {
        Venue::Arena => "lb-venue arena",
        Venue::Official => "lb-venue official",
    };
    let venue_name = match game.venue {
        Venue::Arena => "Arena",
        Venue::Official => "dominion.games",
    };
    let replay = {
        let setup = (
            game.kingdom.clone(),
            game.landscapes.clone(),
            game.others.clone(),
        );
        move |_| {
            spawn(play_kingdom(ctx, account, toast, nav, setup.clone()));
        }
    };
    rsx! {
        div { class: "lb-sheet", "data-sheet": "{game.id}",
            div { class: "lb-sheet-meta",
                span { class: venue_class, "{venue_name}" }
                if game.voided.is_some() {
                    span { class: "lb-venue voided", title: VOIDED, "Voided" }
                }
                span { "{meta}" }
            }
            h2 { class: "lb-sheet-title", "{title}" }
            div { class: "lb-sheet-ending", "{ending}" }
            KingdomCards {
                kingdom: game.kingdom.clone(),
                landscapes: game.landscapes.clone(),
                others: game.others.clone(),
            }
            div { class: "lb-sheet-line lb-sheet-head",
                span { class: "lb-sheet-place", "Place" }
                span { "Player" }
                span { "Played by" }
                span { class: "lb-sheet-num", "VP" }
                span { class: "lb-sheet-num", "Turns" }
                span { class: "lb-sheet-num", "Rating" }
            }
            for p in by_seat(&game) {
                {
                    let (by, link) = played_by(&game, p);
                    let own = p.account.clone().filter(|_| game.venue == Venue::Arena);
                    let row = if mine.is_some_and(|m| m.seat == p.seat) { "lb-sheet-line mine" } else { "lb-sheet-line" };
                    rsx! {
                        div { key: "{p.seat}", class: row,
                            span { class: "lb-sheet-place", "{p.rank}" }
                            span { class: if p.rank == 1 { "lb-sheet-name won" } else { "lb-sheet-name" },
                                SeatIcon { played_by: p.played_by.clone() }
                                if p.rank == 1 {
                                    Crown {}
                                }
                                match own {
                                    Some(a) => rsx! { Link { to: Route::User { name: a }, "{p.name}" } },
                                    None => rsx! { "{p.name}" },
                                }
                                if p.resigned {
                                    span { class: "lb-sheet-resigned", "resigned" }
                                }
                            }
                            span { class: "lb-sheet-by",
                                match link {
                                    Some(a) => rsx! { Link { to: Route::User { name: a }, "{by}" } },
                                    None => rsx! { "{by}" },
                                }
                            }
                            span { class: "lb-sheet-num", "{p.vp}" }
                            span { class: "lb-sheet-num", "{p.turns}" }
                            match (p.rating, p.level) {
                                (Some((before, after)), _) => rsx! {
                                    span { class: if after >= before { "lb-sheet-num up" } else { "lb-sheet-num down" },
                                        "{before.round()} → {after.round()}"
                                    }
                                },
                                (None, Some(level)) => rsx! {
                                    span { class: "lb-sheet-num", title: "dominion.games level as the game began", "{level:.2}" }
                                },
                                (None, None) => rsx! { span { class: "lb-sheet-num none", "—" } },
                            }
                        }
                    }
                }
            }
            div { class: "lb-sheet-actions",
                if game.open {
                    Link { class: "lb-button lb-gold", to: Route::Game { id: game.id.clone() }, "Review game" }
                }
                if game.open || game.replayable {
                    Link { class: "lb-button lb-copper", "data-action": "replay", to: Route::GameReplay { id: game.id.clone() }, "Replay" }
                }
                button { class: "lb-button lb-copper", "data-action": "play-kingdom", onclick: replay, "Play this kingdom" }
                if let Some(number) = game.official_id.clone() {
                    span { class: "lb-sheet-official",
                        "dominion.games game "
                        span { class: "lb-sheet-number", "#{number}" }
                    }
                    div {
                        class: "lb-oval",
                        onclick: move |_| {
                            let said = if platform::copy(&number) { "Copied" } else { "Couldn't copy it" };
                            flash(toast, said.into());
                        },
                        "Copy"
                    }
                }
            }
        }
    }
}

/// Sets up a table with a game's kingdom, Colonies and Shelters as it had them: the
/// host's own table if it has no game yet, else a new one from the rules hosted here last.
async fn play_kingdom(
    ctx: LobbyCtx,
    account: Account,
    toast: Signal<Option<String>>,
    nav: Navigator,
    (kingdom, landscapes, others): (Vec<String>, Vec<String>, Vec<String>),
) {
    let has = |names: &[&str]| others.iter().any(|o| names.contains(&o.as_str()));
    let colonies = Ternary::of(Some(has(&["Platinum", "Colony"])));
    let shelters = Ternary::of(Some(has(&["Hovel", "Necropolis", "Overgrown Estate"])));
    let take = move |r: &mut TableRules| {
        r.kingdom = kingdom.iter().cloned().map(Pick::Card).collect();
        r.landscapes = landscapes.clone();
        r.slots.clear();
        r.colonies = colonies;
        r.shelters = shelters;
    };
    match ctx.table_now() {
        Some(table) if ctx.is_host_now() && table.game.is_none() => {
            ctx.edit(take);
            nav.push(Route::MyTable {});
        }
        Some(_) => flash(toast, "Leave your table to play this kingdom.".into()),
        None => {
            let sets = ctx.sets.peek().clone();
            let mut rules = store::rules().unwrap_or_else(|| TableRules::new(&sets));
            take(&mut rules);
            store::set_rules(&rules);
            if create_table(ctx, account, toast, nav).await {
                nav.push(Route::MyTable {});
            }
        }
    }
}
