//! The lobby: the server's games, creating a game, and taking a seat by token.

use crate::api;
use crate::app::{Cards, Route, use_cards};
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dominion_protocol::api::{Created, GameSummary, NewGame, SeatToken};
use std::collections::BTreeSet;

const REFRESH_MS: u32 = 3000;

#[component]
pub fn LobbyPage() -> Element {
    let created = use_signal(|| None::<Created>);
    rsx! {
        div { class: "lobby",
            GameList {}
            if let Some(c) = created() {
                TokensPanel { created: c }
            }
            div { class: "lobby-forms",
                NewGameForm { created }
                JoinForm {}
            }
        }
    }
}

#[component]
fn GameList() -> Element {
    let mut games = use_signal(|| None::<Result<Vec<GameSummary>, String>>);
    use_future(move || async move {
        loop {
            games.set(Some(api::games().await.map_err(|e| e.to_string())));
            platform::sleep(REFRESH_MS).await;
        }
    });
    let held = session::all();
    let nav = navigator();
    rsx! {
        section { class: "panel games",
            h2 { "Games" }
            match games() {
                None => rsx! { p { class: "muted", "Loading…" } },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                Some(Ok(list)) if list.is_empty() => rsx! { p { class: "muted", "No games yet." } },
                Some(Ok(list)) => rsx! {
                    table { class: "game-table",
                        thead {
                            tr {
                                th { "Game" }
                                th { "Players" }
                                th { "Status" }
                                th { "Kingdom" }
                                th { "" }
                            }
                        }
                        tbody {
                            for g in list {
                                {
                                    let mine = held.get(&g.game_id).cloned().unwrap_or_default();
                                    let status = match (g.running, g.deciding) {
                                        (false, _) => "finished".to_string(),
                                        (true, Some(s)) => format!(
                                            "{} to decide",
                                            g.names.get(s as usize).cloned().unwrap_or_else(|| format!("P{}", s + 1))
                                        ),
                                        (true, None) => "running".to_string(),
                                    };
                                    let id = g.game_id.clone();
                                    rsx! {
                                        tr { key: "{g.game_id}", class: if !mine.is_empty() { "mine" },
                                            td { class: "mono", "{g.game_id}" }
                                            td {
                                                for (i, n) in g.names.iter().enumerate() {
                                                    span {
                                                        class: if mine.iter().any(|s| s.seat as usize == i) { "seat held" } else { "seat" },
                                                        title: if mine.iter().any(|s| s.seat as usize == i) { "you hold this seat" },
                                                        "{n}"
                                                    }
                                                }
                                            }
                                            td { class: if g.running { "running" } else { "finished" }, "{status}" }
                                            td { class: "kingdom-cell", "{g.kingdom.join(\", \")}" }
                                            td {
                                                if !mine.is_empty() {
                                                    button { onclick: move |_| { nav.push(Route::Game { id: id.clone() }); }, "Open" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

/// A three-state yes / no / default choice.
#[component]
fn TriState(label: String, value: Signal<Option<bool>>) -> Element {
    let options = [(None, "default"), (Some(true), "yes"), (Some(false), "no")];
    rsx! {
        div { class: "tristate",
            span { class: "label", "{label}" }
            for (v, text) in options {
                button {
                    r#type: "button",
                    class: if value() == v { "toggle on" } else { "toggle" },
                    onclick: move |_| value.set(v),
                    "{text}"
                }
            }
        }
    }
}

/// A list of card names, added one by one from a text field with completion.
#[component]
fn CardPicker(
    label: String,
    list_id: String,
    choices: Vec<String>,
    value: Signal<Vec<String>>,
) -> Element {
    let mut input = use_signal(String::new);
    let db = use_context::<Cards>().0;
    let mut add = move || {
        let text = input();
        let name = db
            .read()
            .get(text.trim())
            .map(|c| c.name.clone())
            .unwrap_or_else(|| text.trim().to_string());
        if !name.is_empty() && !value.read().contains(&name) {
            value.write().push(name);
        }
        input.set(String::new());
    };
    let cards = use_cards();
    rsx! {
        div { class: "picker",
            label { "{label}" }
            div { class: "chips",
                for (i, name) in value().into_iter().enumerate() {
                    span { key: "{name}", class: "chip", title: cards.tooltip(&name),
                        "{name}"
                        button {
                            r#type: "button",
                            class: "chip-x",
                            onclick: move |_| { value.write().remove(i); },
                            "×"
                        }
                    }
                }
            }
            div { class: "row",
                input {
                    list: "{list_id}",
                    placeholder: "card name",
                    value: "{input}",
                    oninput: move |e| input.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            add();
                        }
                    },
                }
                button { r#type: "button", onclick: move |_| add(), "Add" }
            }
            datalist { id: "{list_id}",
                for c in choices {
                    option { key: "{c}", value: "{c}" }
                }
            }
        }
    }
}

#[component]
fn NewGameForm(created: Signal<Option<Created>>) -> Element {
    let cards = use_cards();
    let mut players = use_signal(|| 2u8);
    let mut names = use_signal(|| vec![String::new(); 6]);
    let mut take = use_signal(|| {
        let mut t = vec![false; 6];
        t[0] = true;
        t
    });
    let mut random = use_signal(|| true);
    let kingdom = use_signal(Vec::<String>::new);
    let mut sets = use_signal(BTreeSet::<String>::new);
    let landscapes = use_signal(Vec::<String>::new);
    let colonies = use_signal(|| None::<bool>);
    let shelters = use_signal(|| None::<bool>);
    let mut seed = use_signal(String::new);
    let mut turn_limit = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let nav = navigator();

    let submit = move |_| {
        let n = players() as usize;
        let parse = |s: String, what: &str| -> Result<Option<u64>, String> {
            let s = s.trim().to_string();
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse()
                    .map(Some)
                    .map_err(|_| format!("{what} must be a whole number"))
            }
        };
        let seed = match parse(seed(), "the seed") {
            Ok(s) => s,
            Err(e) => return error.set(Some(e)),
        };
        let turn_limit = match parse(turn_limit(), "the turn limit") {
            Ok(t) => t.map(|t| t as u32),
            Err(e) => return error.set(Some(e)),
        };
        let taken: Vec<u8> = (0..n as u8).filter(|&i| take.read()[i as usize]).collect();
        let req = NewGame {
            players: n as u8,
            names: (0..n)
                .map(|i| {
                    let name = names.read()[i].trim().to_string();
                    if name.is_empty() {
                        format!("P{}", i + 1)
                    } else {
                        name
                    }
                })
                .collect(),
            kingdom: if random() { vec![] } else { kingdom() },
            landscapes: landscapes(),
            sets: if random() {
                sets().into_iter().collect()
            } else {
                vec![]
            },
            seed,
            turn_limit,
            colonies: colonies(),
            shelters: shelters(),
        };
        busy.set(true);
        error.set(None);
        spawn(async move {
            match api::create_game(&req).await {
                Ok(c) => {
                    for s in c.seats.iter().filter(|s| taken.contains(&s.seat)) {
                        session::hold(
                            &c.game_id,
                            HeldSeat {
                                seat: s.seat,
                                name: s.name.clone(),
                                token: s.token.clone(),
                            },
                        );
                    }
                    if c.seats.iter().all(|s| taken.contains(&s.seat)) {
                        nav.push(Route::Game { id: c.game_id });
                    } else {
                        created.set(Some(Created {
                            game_id: c.game_id,
                            seats: c
                                .seats
                                .into_iter()
                                .filter(|s| !taken.contains(&s.seat))
                                .collect(),
                        }));
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };

    let n = players() as usize;
    rsx! {
        section { class: "panel new-game",
            h2 { "New game" }
            div { class: "field",
                label { "Players" }
                select {
                    value: "{players}",
                    onchange: move |e| {
                        if let Ok(p) = e.value().parse() {
                            players.set(p);
                        }
                    },
                    for p in 2..=6u8 {
                        option { value: "{p}", selected: players() == p, "{p}" }
                    }
                }
            }
            div { class: "seats",
                for i in 0..n {
                    div { key: "{i}", class: "seat-row",
                        span { class: "seat-no", "Seat {i + 1}" }
                        input {
                            placeholder: "P{i + 1}",
                            maxlength: "32",
                            value: "{names.read()[i]}",
                            oninput: move |e| names.write()[i] = e.value(),
                        }
                        label { class: "check",
                            input {
                                r#type: "checkbox",
                                checked: take.read()[i],
                                onchange: move |e| take.write()[i] = e.checked(),
                            }
                            "this browser plays it"
                        }
                    }
                }
            }
            div { class: "field",
                label { "Kingdom" }
                div { class: "tristate",
                    button { r#type: "button", class: if random() { "toggle on" } else { "toggle" }, onclick: move |_| random.set(true), "random" }
                    button { r#type: "button", class: if !random() { "toggle on" } else { "toggle" }, onclick: move |_| random.set(false), "choose cards" }
                }
            }
            if random() {
                div { class: "sets",
                    span { class: "muted", "Deal from (none checked: every set)" }
                    div { class: "set-grid",
                        for s in cards.sets() {
                            label { key: "{s}", class: "check",
                                input {
                                    r#type: "checkbox",
                                    checked: sets.read().contains(&s),
                                    onchange: {
                                        let s = s.clone();
                                        move |e: FormEvent| {
                                            if e.checked() {
                                                sets.write().insert(s.clone());
                                            } else {
                                                sets.write().remove(&s);
                                            }
                                        }
                                    },
                                }
                                "{s}"
                            }
                        }
                    }
                }
            } else {
                CardPicker { label: "Kingdom cards", list_id: "kingdom-cards", choices: cards.kingdom_names(), value: kingdom }
            }
            CardPicker { label: "Landscapes (optional)", list_id: "landscape-cards", choices: cards.landscape_names(), value: landscapes }
            TriState { label: "Colonies", value: colonies }
            TriState { label: "Shelters", value: shelters }
            div { class: "row",
                div { class: "field",
                    label { "Seed" }
                    input { placeholder: "random", value: "{seed}", oninput: move |e| seed.set(e.value()) }
                }
                div { class: "field",
                    label { "Turn limit" }
                    input { placeholder: "none", value: "{turn_limit}", oninput: move |e| turn_limit.set(e.value()) }
                }
            }
            div { class: "actions",
                button { class: "primary", disabled: busy(), onclick: submit, "Create game" }
            }
            if let Some(e) = error() {
                p { class: "error", "{e}" }
            }
        }
    }
}

/// The seats this browser didn't take, shown once after creating a game.
#[component]
fn TokensPanel(created: Created) -> Element {
    let origin = platform::origin();
    let id = created.game_id.clone();
    let held = !session::seats(&created.game_id).is_empty();
    rsx! {
        section { class: "panel tokens",
            h2 { "Game {created.game_id} created" }
            p { class: "warn", "These tokens are shown only now. Each one is the only way to play its seat." }
            for s in created.seats.iter() {
                SeatTokenRow { key: "{s.seat}", game: created.game_id.clone(), seat: s.clone(), origin: origin.clone() }
            }
            if held {
                div { class: "actions",
                    Link { class: "button primary", to: Route::Game { id }, "Open the game" }
                }
            }
        }
    }
}

#[component]
fn SeatTokenRow(game: String, seat: SeatToken, origin: String) -> Element {
    let link = format!(
        "{origin}/game/{game}#token={}",
        platform::encode(&seat.token)
    );
    let mcp = format!(
        "Connect to the dominion MCP server at {origin}/mcp and call join_game with {{\"game_id\": \"{game}\", \"token\": \"{}\"}}",
        seat.token
    );
    let token = seat.token.clone();
    let mcp_copy = mcp.clone();
    let link_copy = link.clone();
    rsx! {
        div { class: "token-row",
            h3 { "Seat {seat.seat + 1}: {seat.name}" }
            div { class: "row",
                code { class: "token", "{seat.token}" }
                button { onclick: move |_| platform::copy(&token), "Copy token" }
            }
            div { class: "row",
                code { class: "mcp", "{mcp}" }
                button { onclick: move |_| platform::copy(&mcp_copy), "Copy" }
            }
            div { class: "row",
                a { href: "{link}", target: "_blank", class: "mono", "{link}" }
                button { onclick: move |_| platform::copy(&link_copy), "Copy link" }
            }
        }
    }
}

#[component]
fn JoinForm() -> Element {
    let mut token = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let nav = navigator();
    let join = move |_| {
        let t = token().trim().to_string();
        if t.is_empty() {
            return;
        }
        error.set(None);
        spawn(async move {
            match api::seat(&t).await {
                Ok(info) => {
                    session::hold(
                        &info.game_id,
                        HeldSeat {
                            seat: info.seat,
                            name: info.name,
                            token: t,
                        },
                    );
                    nav.push(Route::Game { id: info.game_id });
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    rsx! {
        section { class: "panel join",
            h2 { "Join with a token" }
            div { class: "row",
                input {
                    class: "grow",
                    placeholder: "seat token",
                    value: "{token}",
                    oninput: move |e| token.set(e.value()),
                }
                button { class: "primary", onclick: join, "Join" }
            }
            if let Some(e) = error() {
                p { class: "error", "{e}" }
            }
        }
    }
}
