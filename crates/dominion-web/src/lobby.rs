//! The lobby: the server's games, creating a game with the kingdom builder, and taking a
//! seat by token.

use crate::api;
use crate::app::{Route, use_cards};
use crate::assets::use_assets;
use crate::card::{CardHover, CardSize, CardView};
use crate::kingdom::{
    self, CostFilter, Deal, Filter, KINGDOM_SIZE, MAX_COST_FILTER, Options, Rng, Tray,
};
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dominion_protocol::api::{Created, GameSummary, SeatToken};
use std::collections::BTreeSet;

const REFRESH_MS: u32 = 3000;
const MAX_PLAYERS: usize = 6;

#[component]
pub fn LobbyPage() -> Element {
    let created = use_signal(|| None::<(Created, Option<GameSummary>)>);
    rsx! {
        div { class: "lobby",
            div { class: "lobby-top",
                GameList { created }
                JoinForm {}
            }
            if let Some((c, dealt)) = created() {
                TokensPanel { created: c, dealt }
            }
            NewGameForm { created }
        }
    }
}

/// Card names as a row of cards, each showing the whole card on hover.
#[component]
fn CardRow(names: Vec<String>, size: CardSize, #[props(default)] class: String) -> Element {
    let cards = use_cards();
    rsx! {
        div { class: "card-row {class}",
            for name in names {
                {
                    let landscape = cards.get(&name).is_some_and(crate::card::is_landscape);
                    rsx! {
                        div {
                            key: "{name}",
                            class: if landscape && size != CardSize::Micro { "row-card landscape" } else { "row-card" },
                            CardHover { name: name.clone(),
                                CardView { name: name.clone(), size }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn GameList(created: ReadSignal<Option<(Created, Option<GameSummary>)>>) -> Element {
    let mut games = use_signal(|| None::<Result<Vec<GameSummary>, String>>);
    use_future(move || async move {
        loop {
            games.set(Some(api::games().await.map_err(|e| e.to_string())));
            platform::sleep(REFRESH_MS).await;
        }
    });
    use_effect(move || {
        if created.read().is_some() {
            spawn(async move {
                games.set(Some(api::games().await.map_err(|e| e.to_string())));
            });
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
                    ul { class: "game-list",
                        for g in list.into_iter().rev() {
                            {
                                let mine = held.get(&g.game_id).cloned().unwrap_or_default();
                                let (status, class) = match (g.running, g.deciding) {
                                    (false, _) => ("finished".to_string(), "status finished"),
                                    (true, Some(s)) => (
                                        format!(
                                            "{} to decide",
                                            g.names.get(s as usize).cloned().unwrap_or_else(|| format!("P{}", s + 1))
                                        ),
                                        "status running",
                                    ),
                                    (true, None) => ("running".to_string(), "status running"),
                                };
                                let id = g.game_id.clone();
                                let deciding = g.deciding.filter(|_| g.running);
                                rsx! {
                                    li { key: "{g.game_id}", class: if !mine.is_empty() { "game mine" } else { "game" },
                                        div { class: "game-head",
                                            span { class: "game-id mono", "{g.game_id}" }
                                            span { class: "seats-line",
                                                for (i, n) in g.names.iter().enumerate() {
                                                    span {
                                                        key: "{i}",
                                                        class: format!(
                                                            "seat{}{}",
                                                            if mine.iter().any(|s| s.seat as usize == i) { " held" } else { "" },
                                                            if deciding == Some(i as u8) { " deciding" } else { "" },
                                                        ),
                                                        title: if mine.iter().any(|s| s.seat as usize == i) { "you hold this seat" },
                                                        "{n}"
                                                    }
                                                }
                                            }
                                            span { class, "{status}" }
                                            if !mine.is_empty() {
                                                button {
                                                    class: "primary right",
                                                    onclick: move |_| { nav.push(Route::Game { id: id.clone() }); },
                                                    "Open"
                                                }
                                            }
                                        }
                                        CardRow { names: g.kingdom.clone(), size: CardSize::Micro, class: "micro-row" }
                                        if !g.landscapes.is_empty() {
                                            CardRow { names: g.landscapes.clone(), size: CardSize::Micro, class: "micro-row landscapes" }
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
fn TriState(label: String, hint: String, value: Signal<Option<bool>>) -> Element {
    let options = [(None, "default"), (Some(true), "yes"), (Some(false), "no")];
    rsx! {
        div { class: "tristate", title: "{hint}",
            span { class: "label", "{label}" }
            div { class: "segmented",
                for (v , text) in options {
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
}

/// Every dealable set as a toggle, with its box art when the cache has some.
#[component]
fn SetPicker(all: Vec<String>, chosen: Signal<Option<BTreeSet<String>>>) -> Element {
    let assets = use_assets();
    let current: BTreeSet<String> = chosen().unwrap_or_else(|| all.iter().cloned().collect());
    let every = all.clone();
    rsx! {
        div { class: "set-picker",
            div { class: "section-head",
                h3 { "Sets" }
                button { r#type: "button", class: "small", onclick: move |_| chosen.set(Some(every.iter().cloned().collect())), "All" }
                button { r#type: "button", class: "small", onclick: move |_| chosen.set(Some(BTreeSet::new())), "None" }
                span { class: "muted", "{current.len()} of {all.len()}" }
            }
            div { class: "set-chips",
                for s in all.iter().cloned() {
                    {
                        let on = current.contains(&s);
                        let art = kingdom::set_art_paths(&s).into_iter().find_map(|p| assets.file_url(&p));
                        let base = current.clone();
                        let name = s.clone();
                        rsx! {
                            button {
                                key: "{s}",
                                r#type: "button",
                                class: if on { "set-chip on" } else { "set-chip" },
                                "data-set": "{s}",
                                onclick: move |_| {
                                    let mut next = base.clone();
                                    if !next.remove(&name) {
                                        next.insert(name.clone());
                                    }
                                    chosen.set(Some(next));
                                },
                                if let Some(src) = art {
                                    img { src, alt: "", loading: "lazy" }
                                }
                                span { "{s}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The ten Kingdom slots and the landscapes.
#[component]
fn TrayView(tray: Signal<Tray>, deal: Signal<Deal>) -> Element {
    let cards = use_cards();
    let t = tray();
    let ignored = deal() == Deal::Server || t.kingdom.is_empty();
    let empty = KINGDOM_SIZE.saturating_sub(t.kingdom.len());
    rsx! {
        div { class: if ignored { "trays ignored" } else { "trays" },
            div { class: "tray kingdom-tray",
                for (i , slot) in t.kingdom.iter().enumerate() {
                    div {
                        key: "{slot.name}",
                        class: if slot.locked { "slot filled locked" } else { "slot filled" },
                        CardHover { name: slot.name.clone(),
                            CardView { name: slot.name.clone(), size: CardSize::Mini,
                                button {
                                    r#type: "button",
                                    class: if slot.locked { "slot-btn lock on" } else { "slot-btn lock" },
                                    title: if slot.locked { "Unlock: Randomize may replace it" } else { "Lock: Randomize keeps it" },
                                    onclick: move |e| {
                                        e.stop_propagation();
                                        tray.write().toggle_lock(i);
                                    },
                                    if slot.locked { "\u{1F512}" } else { "\u{1F513}" }
                                }
                                button {
                                    r#type: "button",
                                    class: "slot-btn remove",
                                    title: "Remove",
                                    onclick: move |e| {
                                        e.stop_propagation();
                                        tray.write().remove_pile(i);
                                    },
                                    "×"
                                }
                            }
                        }
                    }
                }
                for i in 0..empty {
                    div { key: "empty-{i}", class: "slot empty" }
                }
            }
            div { class: "tray landscape-tray",
                for name in t.landscapes.iter().cloned() {
                    div { key: "{name}", class: "slot filled landscape",
                        CardHover { name: name.clone(),
                            CardView { name: name.clone(), size: CardSize::Mini,
                                button {
                                    r#type: "button",
                                    class: "slot-btn remove",
                                    title: "Remove",
                                    onclick: {
                                        let name = name.clone();
                                        let cards = cards.clone();
                                        move |e: MouseEvent| {
                                            e.stop_propagation();
                                            tray.write().toggle_landscape(&cards, &name);
                                        }
                                    },
                                    "×"
                                }
                            }
                        }
                    }
                }
                if t.landscapes.len() < 2 {
                    for i in t.landscapes.len()..2 {
                        div { key: "lempty-{i}", class: "slot empty landscape",
                            span { "Event, Landmark, Project, Way, Ally, Trait or Prophecy" }
                        }
                    }
                }
            }
        }
    }
}

/// Every Kingdom pile or landscape from the chosen sets; clicking one adds it to the
/// tray or takes it out.
#[component]
fn Browser(
    sets: BTreeSet<String>,
    tray: Signal<Tray>,
    deal: Signal<Deal>,
    removed: Signal<bool>,
) -> Element {
    let cards = use_cards();
    let mut landscapes = use_signal(|| false);
    let mut filter = use_signal(Filter::default);
    let f = filter();
    let all = if landscapes() {
        kingdom::landscapes(&cards, &sets, removed())
    } else {
        kingdom::kingdom_piles(&cards, &sets, removed())
    };
    let types: BTreeSet<String> = all
        .iter()
        .flat_map(|c| c.types.iter().cloned())
        .filter(|t| t != "Randomizer")
        .collect();
    let shown: Vec<String> = all
        .iter()
        .filter(|c| f.matches(&cards, c))
        .map(|c| c.name.clone())
        .collect();
    let t = tray();
    let cost_value = match f.cost {
        CostFilter::Any => String::new(),
        CostFilter::Coins(n) => n.to_string(),
        CostFilter::Potion => "potion".into(),
        CostFilter::Debt => "debt".into(),
    };
    rsx! {
        div { class: "browser",
            div { class: "browser-controls",
                div { class: "segmented",
                    button {
                        r#type: "button",
                        class: if !landscapes() { "toggle on" } else { "toggle" },
                        onclick: move |_| landscapes.set(false),
                        "Kingdom piles"
                    }
                    button {
                        r#type: "button",
                        class: if landscapes() { "toggle on" } else { "toggle" },
                        onclick: move |_| landscapes.set(true),
                        "Landscapes"
                    }
                }
                input {
                    r#type: "search",
                    class: "browser-search",
                    placeholder: "Search by name",
                    value: "{f.query}",
                    oninput: move |e| filter.write().query = e.value(),
                }
                select {
                    value: "{f.ty}",
                    onchange: move |e| filter.write().ty = e.value(),
                    option { value: "", "Any type" }
                    for ty in types {
                        option { key: "{ty}", value: "{ty}", selected: f.ty == ty, "{ty}" }
                    }
                }
                if !landscapes() {
                    select {
                        value: "{cost_value}",
                        onchange: move |e| {
                            let v = e.value();
                            filter.write().cost = match v.as_str() {
                                "potion" => CostFilter::Potion,
                                "debt" => CostFilter::Debt,
                                s => s.parse().map(CostFilter::Coins).unwrap_or(CostFilter::Any),
                            };
                        },
                        option { value: "", "Any cost" }
                        for n in 0..=MAX_COST_FILTER {
                            option { key: "{n}", value: "{n}", selected: f.cost == CostFilter::Coins(n),
                                if n == MAX_COST_FILTER { "${n}+" } else { "${n}" }
                            }
                        }
                        option { value: "potion", selected: f.cost == CostFilter::Potion, "Potion" }
                        option { value: "debt", selected: f.cost == CostFilter::Debt, "Debt" }
                    }
                }
                label { class: "check", title: "Cards removed from second editions",
                    input {
                        r#type: "checkbox",
                        checked: removed(),
                        onchange: move |e| removed.set(e.checked()),
                    }
                    "First-edition cards"
                }
                span { class: "muted right", "{shown.len()} shown" }
            }
            if sets.is_empty() {
                p { class: "muted", "Choose a set above to browse its cards." }
            }
            div { class: if landscapes() { "browser-grid landscapes" } else { "browser-grid" },
                for name in shown {
                    {
                        let picked = if landscapes() { t.landscapes.contains(&name) } else { t.has_pile(&name) };
                        let is_landscape = landscapes();
                        let n = name.clone();
                        let db = cards.clone();
                        rsx! {
                            div {
                                key: "{name}",
                                class: if picked { "pick picked" } else { "pick" },
                                onclick: move |_| {
                                    if is_landscape {
                                        tray.write().toggle_landscape(&db, &n);
                                    } else {
                                        tray.write().toggle_pile(&n);
                                        deal.set(Deal::Tray);
                                    }
                                },
                                CardHover { name: name.clone(),
                                    CardView { name: name.clone(), size: CardSize::Mini }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NewGameForm(created: Signal<Option<(Created, Option<GameSummary>)>>) -> Element {
    let cards = use_cards();
    let mut players = use_signal(|| 2u8);
    let mut names = use_signal(|| vec![String::new(); MAX_PLAYERS]);
    let mut take = use_signal(|| {
        let mut t = vec![false; MAX_PLAYERS];
        t[0] = true;
        t
    });
    let chosen_sets = use_signal(|| None::<BTreeSet<String>>);
    let mut tray = use_signal(Tray::default);
    let mut deal = use_signal(Deal::default);
    let removed = use_signal(|| false);
    let colonies = use_signal(|| None::<bool>);
    let shelters = use_signal(|| None::<bool>);
    let mut seed = use_signal(String::new);
    let mut turn_limit = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let nav = navigator();

    let all_sets = kingdom::sets(&cards);
    let sets: BTreeSet<String> =
        chosen_sets().unwrap_or_else(|| all_sets.iter().cloned().collect());
    let ordered_sets: Vec<String> = all_sets
        .iter()
        .filter(|s| sets.contains(*s))
        .cloned()
        .collect();

    let pool: Vec<String> = kingdom::kingdom_piles(&cards, &sets, removed())
        .into_iter()
        .map(|c| c.name.clone())
        .collect();
    let randomize = move |_| {
        let mut rng = Rng::new(platform::random_seed());
        tray.write().randomize(&pool, &mut rng);
        deal.set(Deal::Tray);
    };

    let t = tray();
    let server_deal = deal() == Deal::Server || t.kingdom.is_empty();
    let blocked = server_deal && sets.is_empty();

    let submit_sets = ordered_sets.clone();
    let submit = move |_| {
        let seed = match kingdom::parse_number::<u64>(&seed(), "The seed") {
            Ok(s) => s,
            Err(e) => return error.set(Some(e)),
        };
        let turn_limit = match kingdom::parse_number::<u32>(&turn_limit(), "The turn limit") {
            Ok(t) => t,
            Err(e) => return error.set(Some(e)),
        };
        let n = players();
        let opts = Options {
            players: n,
            names: names.read()[..n as usize].to_vec(),
            seed,
            turn_limit,
            colonies: colonies(),
            shelters: shelters(),
        };
        let req = match kingdom::request(deal(), &tray.read(), &submit_sets, &opts) {
            Ok(r) => r,
            Err(e) => return error.set(Some(e)),
        };
        let taken: Vec<u8> = (0..n).filter(|&i| take.read()[i as usize]).collect();
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
                        let dealt = api::games()
                            .await
                            .ok()
                            .and_then(|gs| gs.into_iter().find(|g| g.game_id == c.game_id));
                        let rest = Created {
                            game_id: c.game_id,
                            seats: c
                                .seats
                                .into_iter()
                                .filter(|s| !taken.contains(&s.seat))
                                .collect(),
                        };
                        created.set(Some((rest, dealt)));
                        platform::scroll_to_top();
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };

    let n = players() as usize;
    let kingdom_note = if server_deal {
        if !t.kingdom.is_empty() {
            "The server deals ten Kingdom piles at random from the chosen sets; the piles in the tray are not sent.".to_string()
        } else {
            "The server deals ten Kingdom piles at random from the chosen sets, plus up to two landscapes. Add cards or Randomize to choose them here.".to_string()
        }
    } else {
        kingdom::size_note(t.kingdom.len()).unwrap_or_else(|| {
            "These ten piles. Cards that need another pile (Young Witch's Bane, ...) add it at setup.".to_string()
        })
    };
    let landscape_note = match (server_deal, t.landscapes.is_empty()) {
        (true, true) => "The deal may include up to two landscapes.",
        (true, false) => "These replace the landscapes the deal would include.",
        (false, true) => "No landscapes, unless the kingdom needs an Ally or a Prophecy.",
        (false, false) => {
            "Exactly these landscapes, plus an Ally or a Prophecy if the kingdom needs one."
        }
    };
    let locked = t.kingdom.iter().filter(|s| s.locked).count();

    rsx! {
        section { class: "panel new-game",
            h2 { "New game" }
            SetPicker { all: all_sets.clone(), chosen: chosen_sets }
            div { class: "kingdom-builder",
                div { class: "section-head",
                    h3 { "Kingdom" }
                    div { class: "segmented",
                        button {
                            r#type: "button",
                            class: if !server_deal { "toggle on" } else { "toggle" },
                            disabled: t.kingdom.is_empty(),
                            title: "Play the piles in the tray",
                            onclick: move |_| deal.set(Deal::Tray),
                            "Use the tray"
                        }
                        button {
                            r#type: "button",
                            class: if server_deal { "toggle on" } else { "toggle" },
                            onclick: move |_| deal.set(Deal::Server),
                            "Leave it to the server"
                        }
                    }
                    button {
                        r#type: "button",
                        class: "randomize",
                        disabled: sets.is_empty(),
                        title: "Fill the unlocked slots at random from the chosen sets",
                        onclick: randomize,
                        "Randomize"
                    }
                    button {
                        r#type: "button",
                        disabled: t.kingdom.len() == locked && t.landscapes.is_empty(),
                        onclick: move |_| {
                            let mut w = tray.write();
                            w.clear_unlocked();
                            w.landscapes.clear();
                        },
                        "Clear"
                    }
                    span { class: "muted", "{t.kingdom.len()} piles, {locked} locked" }
                }
                div { class: "tray-row",
                    TrayView { tray, deal }
                    div { class: "tray-notes",
                        h4 { "Kingdom" }
                        p { class: "note", "{kingdom_note}" }
                        h4 { "Landscapes" }
                        p { class: "note", "{landscape_note}" }
                        p { class: "note hint", "Click cards below to add them. Hover a card to read it; lock the ones Randomize must keep." }
                    }
                }
                Browser { sets: sets.clone(), tray, deal, removed }
            }
            div { class: "options",
                div { class: "opt-block",
                    h3 { "Players" }
                    div { class: "field inline",
                        label { "Players" }
                        select {
                            value: "{players}",
                            onchange: move |e| {
                                if let Ok(p) = e.value().parse() {
                                    players.set(p);
                                }
                            },
                            for p in 2..=MAX_PLAYERS as u8 {
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
                }
                div { class: "opt-block",
                    h3 { "Setup" }
                    TriState {
                        label: "Colonies",
                        hint: "Default: when a random Kingdom pile is from Prosperity",
                        value: colonies,
                    }
                    TriState {
                        label: "Shelters",
                        hint: "Default: when a random Kingdom pile is from Dark Ages",
                        value: shelters,
                    }
                    div { class: "field inline",
                        label { "Seed" }
                        input { placeholder: "random", value: "{seed}", oninput: move |e| seed.set(e.value()) }
                    }
                    div { class: "field inline",
                        label { "Turn limit" }
                        input { placeholder: "none", value: "{turn_limit}", oninput: move |e| turn_limit.set(e.value()) }
                    }
                }
            }
            div { class: "actions create",
                button {
                    class: "primary big",
                    disabled: busy() || blocked || cards.is_empty(),
                    onclick: submit,
                    "Create game"
                }
                if blocked {
                    span { class: "warn", "Choose at least one set for the server to deal from." }
                }
                if let Some(e) = error() {
                    span { class: "error form-error", role: "alert", "{e}" }
                }
            }
        }
    }
}

/// The seats this browser didn't take, shown once after creating a game, with the
/// kingdom it was dealt.
#[component]
fn TokensPanel(created: Created, dealt: Option<GameSummary>) -> Element {
    let origin = platform::origin();
    let id = created.game_id.clone();
    let held = !session::seats(&created.game_id).is_empty();
    rsx! {
        section { class: "panel tokens",
            h2 { "Game {created.game_id} created" }
            if let Some(g) = dealt {
                div { class: "dealt",
                    CardRow { names: g.kingdom.clone(), size: CardSize::Mini, class: "mini-row" }
                    if !g.landscapes.is_empty() {
                        CardRow { names: g.landscapes.clone(), size: CardSize::Mini, class: "mini-row" }
                    }
                }
            }
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
