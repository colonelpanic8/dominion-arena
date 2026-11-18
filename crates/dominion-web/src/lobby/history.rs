//! Finished games: the History tab, and the list of games and a game's sheet that a
//! player's page shares with it. Everything here comes from `/api/history` and
//! `/api/users/{name}` as the protocol's `PastGame`s.

use super::rules::{Pick, TableRules, Ternary};
use super::{
    LobbyCtx, LobbyFrame, Tab, clock, create_table, day, flash, store, use_lobby, use_toast,
};
use crate::account::{Account, use_account};
use crate::api;
use crate::app::Route;
use crate::card::{CardHover, CardSize, CardView};
use crate::platform;
use dioxus::prelude::*;
use dioxus::router::Navigator;
use dominion_protocol::lobby::{
    Arranged, HistoryQuery, PastGame, PastPlayer, PlayedBy, SeatAgent, Venue,
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
    let mut text = use_signal(String::new);
    let mut picked = use_signal(|| None::<String>);
    let mut more = use_signal(Vec::<PastGame>::new);
    let mut more_next = use_signal(|| None::<Option<u64>>);
    let fixed = user.clone();
    let me_now = me.clone();
    let query = use_memo(move || HistoryQuery {
        user: fixed.clone().or_else(|| me_now.clone().filter(|_| mine())),
        venue: venue(),
        rated: rated(),
        q: Some(text()).filter(|t| !t.trim().is_empty()),
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
    let side = user.clone();
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
                div { class: "lb-fieldset lb-history-games",
                    div { class: "lb-fieldset-title", span { "{title}" } }
                    div { class: "lb-history-filters",
                        div { class: "lb-multibutton",
                            for (v, label) in [(None, "All"), (Some(Venue::Arena), "Arena"), (Some(Venue::Official), "dominion.games")] {
                                div {
                                    key: "{label}",
                                    class: if venue() == v { "lb-oval active" } else { "lb-oval" },
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
                                    div {
                                        key: "{label}",
                                        class: if mine() == m { "lb-oval active" } else { "lb-oval" },
                                        onclick: move |_| mine.set(m),
                                        "{label}"
                                    }
                                }
                            }
                        }
                        label { class: "lb-history-rated",
                            super::Switch { on: rated(), onclick: move |_| rated.toggle() }
                            "Rated"
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
                    div { class: "lb-history-list",
                        if let Some(e) = error {
                            p { class: "lb-error", "{e}" }
                        } else if games.is_empty() && first.read().is_some() {
                            p { class: "lb-note", "No finished games match." }
                        }
                        for (heading, g, on) in items {
                            div { key: "{g.id}", class: "lb-history-item",
                                if let Some(h) = heading {
                                    div { class: "lb-history-day", "{h}" }
                                }
                                Entry {
                                    game: g.clone(),
                                    side: side.clone(),
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
        (None, Some(p)) => parts.push(format!("as {}", p.name)),
        (None, None) => {}
    }
    parts.push(if game.rated { "Rated" } else { "Unrated" }.into());
    parts.join(" · ")
}

/// A rating's change, rounded: "+15", "-13".
fn change((before, after): (f64, f64)) -> String {
    format!("{:+}", (after - before).round() as i64)
}

/// One game in the list: the winner and the score, or the side's result.
#[component]
fn Entry(game: PastGame, side: Option<String>, on: bool, onpick: EventHandler<()>) -> Element {
    let ranked = standings(&game);
    let mine = side.as_deref().and_then(|s| seat_of(&game, s));
    let class = if on {
        "lb-history-entry on"
    } else {
        "lb-history-entry"
    };
    let venue = match game.venue {
        Venue::Arena => "arena",
        Venue::Official => "official",
    };
    rsx! {
        button { class, "data-game": "{game.id}", "data-venue": venue, onclick: move |_| onpick.call(()),
            match mine {
                Some(me) => {
                    let won = me.rank == 1;
                    let tied = won && game.players.iter().filter(|p| p.rank == 1).count() > 1;
                    let (mark, result) = match (won, tied) {
                        (true, true) => ("T", "tie"),
                        (true, false) => ("W", "win"),
                        _ => ("L", "loss"),
                    };
                    let others: Vec<&PastPlayer> = ranked.iter().copied().filter(|p| p.seat != me.seat).collect();
                    let score = std::iter::once(me).chain(others.iter().copied()).map(|p| p.vp.to_string()).collect::<Vec<_>>().join("–");
                    rsx! {
                        span { class: "lb-history-result {result}", "{mark}" }
                        span { class: "lb-history-entry-body",
                            span { class: "lb-history-entry-line",
                                span { class: "lb-history-score", "{score}" }
                                span { class: "lb-history-names",
                                    "vs"
                                    for p in others.iter() {
                                        Seated { key: "{p.seat}", name: p.name.clone(), played_by: p.played_by.clone() }
                                    }
                                }
                                if let Some(r) = me.rating {
                                    span { class: if r.1 >= r.0 { "lb-history-delta up" } else { "lb-history-delta" }, "{change(r)}" }
                                }
                            }
                            span { class: "lb-history-how", "{clock(game.ended)} · {how(&game)}" }
                        }
                    }
                }
                None => {
                    let score = ranked.iter().map(|p| p.vp.to_string()).collect::<Vec<_>>().join("–");
                    rsx! {
                        span { class: "lb-history-venue" }
                        span { class: "lb-history-entry-body",
                            span { class: "lb-history-entry-line",
                                span { class: "lb-history-names",
                                    for (i, p) in ranked.iter().enumerate() {
                                        Seated {
                                            key: "{p.seat}",
                                            name: p.name.clone(),
                                            played_by: p.played_by.clone(),
                                            won: p.rank == 1,
                                        }
                                        if i == 0 {
                                            span { class: "lb-history-score", "{score}" }
                                        }
                                    }
                                }
                                span { class: "lb-history-time", "{clock(game.ended)}" }
                            }
                            span { class: "lb-history-how", "{how(&game)}" }
                        }
                    }
                }
            }
        }
    }
}

/// One of the server's agents in a few words: its model and effort, "Opus 5.5 · High".
pub fn agent_chip(a: &SeatAgent) -> String {
    let name = a.name.clone().unwrap_or_else(|| a.agent.clone());
    match &a.effort {
        Some(effort) => {
            let mut chars = effort.chars();
            let effort: String = (chars.next().into_iter())
                .flat_map(char::to_uppercase)
                .chain(chars)
                .collect();
            format!("{name} · {effort}")
        }
        None => name,
    }
}

/// One of the server's agents in full: "Claude Code · claude-opus-5-5 · high effort".
pub fn agent_line(a: &SeatAgent) -> String {
    let effort = a.effort.as_ref().map(|e| format!("{e} effort"));
    let parts: Vec<String> = [a.program.clone(), a.model.clone(), effort]
        .into_iter()
        .flatten()
        .collect();
    if parts.is_empty() {
        a.agent.clone()
    } else {
        parts.join(" · ")
    }
}

/// A seat's name with what played it: nothing for a person, else a tag, or a chip for one
/// of the server's agents.
#[component]
fn Seated(name: String, played_by: PlayedBy, #[props(default)] won: bool) -> Element {
    let (mark, chip) = match &played_by {
        PlayedBy::Person | PlayedBy::Official => (None, false),
        PlayedBy::Guest => (Some("guest".to_string()), false),
        PlayedBy::Token => (Some("token".to_string()), false),
        PlayedBy::Bot { owner: Some(owner) } => (Some(format!("{owner}'s bot")), false),
        PlayedBy::Bot { owner: None } => (Some("bot".to_string()), false),
        PlayedBy::Agent(a) => (Some(agent_chip(a)), true),
    };
    rsx! {
        span { class: if won { "lb-history-who won" } else { "lb-history-who" },
            span { class: "lb-history-who-name", "{name}" }
            if let Some(m) = mark {
                span { class: if chip { "lb-agent-chip" } else { "lb-user-kind" }, "{m}" }
            }
        }
    }
}

/// A kingdom as the table lays it out: its piles in two rows, its landscapes beside them
/// and what else it was set up with under them; each card in full while hovered.
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
                div { class: "lb-setup-others",
                    for name in others.iter() {
                        KingdomCard { key: "{name}", name: name.clone() }
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
        PlayedBy::Agent(a) => (agent_line(a), Some(a.agent.clone())),
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
    let title = match mine {
        Some(me) if winners.len() > 1 && me.rank == 1 => format!("Tied, {}", me.vp),
        Some(me) => {
            let others = ranked
                .iter()
                .filter(|p| p.seat != me.seat)
                .map(|p| p.vp.to_string());
            let word = if me.rank == 1 { "Won" } else { "Lost" };
            format!(
                "{word}, {} to {}",
                me.vp,
                others.collect::<Vec<_>>().join(" and ")
            )
        }
        None if winners.len() > 1 => {
            let names = winners
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(" and ");
            format!("{names} tied, {}", winners[0].vp)
        }
        None => match winners.first() {
            Some(w) if losers.is_empty() => format!("{} won, {}", w.name, w.vp),
            Some(w) => format!("{} won, {} to {}", w.name, w.vp, losers.join(" and ")),
            None => String::new(),
        },
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
                span { class: "lb-sheet-place", "#" }
                span { "Player" }
                span { "Played by" }
                span { class: "lb-sheet-num", "VP" }
                span { class: "lb-sheet-num", "Turns" }
                span { class: "lb-sheet-num", "Rating" }
            }
            for p in ranked.iter() {
                {
                    let (by, link) = played_by(&game, p);
                    let own = p.account.clone().filter(|_| game.venue == Venue::Arena);
                    let row = if mine.is_some_and(|m| m.seat == p.seat) { "lb-sheet-line mine" } else { "lb-sheet-line" };
                    rsx! {
                        div { key: "{p.seat}", class: row,
                            span { class: "lb-sheet-place", "{p.rank}" }
                            span { class: if p.rank == 1 { "lb-sheet-name won" } else { "lb-sheet-name" },
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
                            match p.rating {
                                Some((before, after)) => rsx! {
                                    span { class: if after >= before { "lb-sheet-num up" } else { "lb-sheet-num down" },
                                        "{before.round()} → {after.round()}"
                                    }
                                },
                                None => rsx! { span { class: "lb-sheet-num none", "—" } },
                            }
                        }
                    }
                }
            }
            div { class: "lb-sheet-actions",
                if game.open {
                    Link { class: "lb-button lb-gold", to: Route::Game { id: game.id.clone() }, "Review game" }
                    Link { class: "lb-button lb-gold", "data-action": "replay", to: Route::GameReplay { id: game.id.clone() }, "Replay" }
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
