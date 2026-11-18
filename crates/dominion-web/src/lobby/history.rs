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
    parts.push(if game.rated { "Rated" } else { "Unrated" }.into());
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
    rsx! {
        button { class, "data-game": "{game.id}", "data-venue": venue, onclick: move |_| onpick.call(()),
            span { class: "lb-history-venue", title: venue_name }
            span { class: "lb-history-entry-body",
                span { class: "lb-history-entry-line",
                    span { class: "lb-entry-side",
                        if let Some(p) = first {
                            Seated { won: p.rank == 1, player: p }
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
                            Seated { key: "{p.seat}", player: p.clone(), won: p.rank == 1, right: true }
                        }
                    }
                }
                span { class: "lb-history-how",
                    span { "{how(&game)}" }
                    span { class: "lb-history-time", "{clock(game.ended)}" }
                }
            }
        }
    }
}

/// What played a seat, in a few words under its name: "Claude Code · high", "rook's bot".
/// An agent whose seat isn't named after its model (an official account's) is named.
pub fn caption(p: &PastPlayer) -> String {
    match &p.played_by {
        PlayedBy::Person => "Registered".into(),
        PlayedBy::Guest => "Guest".into(),
        PlayedBy::Token => "Seat token".into(),
        PlayedBy::Bot { owner: Some(owner) } => format!("{owner}'s bot"),
        PlayedBy::Bot { owner: None } => "Bot".into(),
        PlayedBy::Official => "dominion.games".into(),
        PlayedBy::Agent(a) => {
            let product = (a.name.clone())
                .or(a.model.clone())
                .unwrap_or(a.agent.clone());
            match &a.effort {
                Some(effort) => format!("{product} · {effort}"),
                None => product,
            }
        }
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

/// Who makes an agent's model, by its id, else what runs it, else its name: their logo
/// goes before its seat's name. Claude Code and the Claude API share Claude's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Maker {
    Claude,
    OpenAi,
    Gemini,
}

pub fn maker(a: &SeatAgent) -> Option<Maker> {
    let by_word = |text: &str| {
        let text = text.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
        if has(&["claude", "opus", "sonnet", "haiku", "fable"]) {
            Some(Maker::Claude)
        } else if has(&["gpt", "codex", "openai"]) {
            Some(Maker::OpenAi)
        } else if has(&["gemini", "antigravity"]) {
            Some(Maker::Gemini)
        } else {
            None
        }
    };
    (a.model.as_deref().and_then(by_word))
        .or(a.program.as_deref().and_then(by_word))
        .or(a.name.as_deref().and_then(by_word))
        .or(by_word(&a.agent))
}

/// The class of the icon before a seat's name.
fn icon_class(by: &PlayedBy) -> &'static str {
    match by {
        PlayedBy::Person => "lb-seat-mark person",
        PlayedBy::Guest => "lb-seat-mark guest",
        PlayedBy::Token => "lb-seat-mark token",
        PlayedBy::Bot { .. } => "lb-seat-mark bot",
        PlayedBy::Agent(a) => match maker(a) {
            Some(Maker::Claude) => "lb-seat-mark logo claude",
            Some(Maker::OpenAi) => "lb-seat-mark logo openai",
            Some(Maker::Gemini) => "lb-seat-mark logo gemini",
            None => "lb-seat-mark agent",
        },
        PlayedBy::Official => "lb-seat-mark official",
    }
}

/// A person, a machine, a token or an agent's maker's logo, drawn small before a seat's
/// name: the drawings fill the same box, 2.5 to 13.5 across and 2 to 14.5 down of 16, and
/// a logo the same share of its own.
#[component]
pub(crate) fn SeatIcon(played_by: PlayedBy) -> Element {
    let class = icon_class(&played_by);
    let logo = match &played_by {
        PlayedBy::Agent(a) => maker(a).map(|m| match m {
            Maker::Claude => CLAUDE,
            Maker::OpenAi => OPENAI,
            Maker::Gemini => GEMINI,
        }),
        _ => None,
    };
    let machine = matches!(played_by, PlayedBy::Bot { .. } | PlayedBy::Agent(_));
    let token = played_by == PlayedBy::Token;
    if let Some(d) = logo {
        return rsx! {
            svg { class, view_box: "-3 -3 30 30", "aria-hidden": "true",
                path { d }
            }
        };
    }
    rsx! {
        svg { class, view_box: "0 0 16 16", "aria-hidden": "true",
            if machine {
                path { d: "M8 2v1.5M4 3.5h8a1.5 1.5 0 0 1 1.5 1.5v5.5a1.5 1.5 0 0 1-1.5 1.5H4a1.5 1.5 0 0 1-1.5-1.5V5A1.5 1.5 0 0 1 4 3.5zM6 7v1M10 7v1M5.5 14.5h5" }
            } else if token {
                path { d: "M2.5 3.5h11v9h-11zM5.5 8h5" }
            } else {
                path { d: "M8 2a3 3 0 1 1 0 6a3 3 0 1 1 0-6zM2.5 14.5c0-3.3 2.5-5.2 5.5-5.2s5.5 1.9 5.5 5.2" }
            }
        }
    }
}

// The makers' marks, from Simple Icons (CC0), in a 24 by 24 box.
const CLAUDE: &str = "m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z";
const OPENAI: &str = "M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z";
const GEMINI: &str = "M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81";

/// The winner's mark, before their name.
#[component]
fn Crown() -> Element {
    rsx! {
        svg { class: "lb-crown", view_box: "0 0 16 16", role: "img", "aria-label": "won",
            path { d: "M2.5 12.5h11M3 12.5 2 5l3.5 3L8 3l2.5 5L14 5l-1 7.5" }
        }
    }
}

/// A seat in the list: an icon for what played it, its name, and under it a caption.
#[component]
fn Seated(
    player: PastPlayer,
    #[props(default)] won: bool,
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
    rsx! {
        span { class,
            SeatIcon { played_by: player.played_by.clone() }
            span { class: "lb-seat-text",
                span { class: "lb-seat-name",
                    if won {
                        Crown {}
                    }
                    "{player.name}"
                }
                span { class: "lb-seat-caption", "{caption(&player)}" }
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
