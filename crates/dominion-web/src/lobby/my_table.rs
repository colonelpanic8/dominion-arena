//! My Table: the server's table this account sits at, as dominion.games lays one out
//! (Kingdom Options and Game Options on the left, Players and the table's chat on the
//! right, Ready and Leave Table below). Its host edits the rules and the seats; everyone
//! else sees them. While the table's game runs it shows that game, locked.

use super::chat::Chat;
use super::rules::{
    LandscapeSlot, MAX_PLAYERS, Pick, SLOT_KINDS, Ternary, expansion_icon, expansions,
    free_seat_name, seat_edits, waiting_for,
};
use super::seat_link::SeatLink;
use super::selector::CardSelector;
use super::store;
use super::{
    Fieldset, LobbyCtx, LobbyFrame, OptionRow, Switch, Tab, TooltipLayer, UserName, create_table,
    handed_out, show_tooltip, use_lobby, use_toast, use_tooltip,
};
use crate::account::use_account;
use crate::api;
use crate::app::{Route, use_cards};
use crate::assets::use_assets;
use crate::card::{CardSize, CardView};
use crate::platform;
use crate::session;
use dioxus::prelude::*;
use dominion_protocol::api::GameSummary;
use dominion_protocol::lobby::{SeatEdit, SeatKind, TableInfo, TableSeat};

/// The table's game as `/api/games` lists it, asked again every few seconds while the
/// game runs.
fn use_table_game(ctx: LobbyCtx) -> Signal<Option<GameSummary>> {
    let mut summary = use_signal(|| None::<GameSummary>);
    let watched = use_memo(move || ctx.table().and_then(|t| t.game));
    use_resource(move || {
        let id = watched();
        async move {
            let Some(id) = id else {
                return;
            };
            loop {
                if let Ok(list) = api::games().await {
                    let found = list.into_iter().find(|g| g.game_id == id);
                    if *summary.peek() != found {
                        summary.set(found);
                    }
                }
                platform::sleep(2000).await;
            }
        }
    });
    summary
}

#[component]
pub fn MyTableTab() -> Element {
    let ctx = use_lobby();
    let selector = use_signal(|| false);
    let study = use_signal(|| None::<String>);
    let loaded = ctx.lobby.read().is_some();
    let table = ctx.table();
    let host = ctx.is_host();
    let summary = use_table_game(ctx);
    let running = table.as_ref().and_then(|t| t.game.clone());
    let game = summary().filter(|g| Some(&g.game_id) == running.as_ref());
    rsx! {
        LobbyFrame { tab: Tab::MyTable,
            if let Some(t) = table {
                if running.is_some() {
                    div { class: "lb-table-content lb-locked", "data-table": "{t.id}",
                        div { class: "lb-table-main",
                            if let Some(g) = game.clone() {
                                LockedKingdom { game: g, study }
                            }
                        }
                        div { class: "lb-table-side",
                            if let Some(g) = game.clone() {
                                LockedPlayers { game: g.clone() }
                                SeatLinks { game: g }
                            }
                            TableChat { table: t.id.clone() }
                        }
                    }
                } else {
                    div {
                        class: if host { "lb-table-content" } else { "lb-table-content lb-readonly" },
                        "data-table": "{t.id}",
                        "data-host": "{host}",
                        div { class: "lb-table-main",
                            KingdomOptions { selector, study }
                            GameOptions {}
                        }
                        div { class: "lb-table-side",
                            Players {}
                            if let Some(last) = t.last_game.clone() {
                                p { class: "lb-last-game", "data-last-game": "{last}",
                                    "Game {last} has ended. "
                                    Link { to: Route::Game { id: last.clone() }, "Its score" }
                                }
                            }
                            TableChat { table: t.id.clone() }
                        }
                    }
                    if selector() && host {
                        div { class: "lb-selector-shield", onclick: { let mut selector = selector; move |_| selector.set(false) } }
                        CardSelector { selector, study }
                    }
                }
                TableButtons { table: t }
                StudyView { study }
            } else if loaded {
                NoTable {}
            }
            TooltipLayer {}
        }
    }
}

/// What My Table shows to someone who sits at none.
#[component]
fn NoTable() -> Element {
    let ctx = use_lobby();
    let account = use_account();
    let toast = use_toast();
    let nav = navigator();
    rsx! {
        div { class: "lb-no-table",
            p { "You aren't sitting at a table." }
            button {
                class: "lb-button lb-gold lb-starter-button",
                "data-action": "create-table",
                onclick: move |_| { spawn(async move { create_table(ctx, account, toast, nav).await; }); },
                "Create Table"
            }
            button {
                class: "lb-button lb-copper lb-starter-button",
                onclick: move |_| { nav.push(Route::Tables {}); },
                "Join a Table"
            }
        }
    }
}

/// The table's chat, where dominion.games has it: under the players.
#[component]
fn TableChat(table: String) -> Element {
    rsx! {
        for channel in [format!("table-{table}")] {
            Chat { key: "{channel}", channel, title: "Table Chat", class: "lb-table-chat" }
        }
    }
}

/// The card selector, for pages outside the lobby (the score page's Edit Table).
#[component]
pub fn CardSelectorFor(selector: Signal<bool>, study: Signal<Option<String>>) -> Element {
    rsx! { CardSelector { selector, study } }
}

/// A card large over a dim backdrop; any click closes it.
#[component]
pub fn StudyView(study: Signal<Option<String>>) -> Element {
    let Some(name) = study() else {
        return rsx! {};
    };
    rsx! {
        div { class: "lb-study", onclick: move |_| study.set(None),
            div { class: "lb-study-card",
                CardView { name, size: CardSize::Full }
            }
        }
    }
}

#[component]
pub fn KingdomOptions(selector: Signal<bool>, study: Signal<Option<String>>) -> Element {
    let cards = use_cards();
    let ctx = use_lobby();
    let host = ctx.is_host();
    let refused = ctx.refused.read().clone();
    let Some(r) = ctx.rules.read().clone() else {
        return rsx! {};
    };
    let code = r.kingdom_code(&cards);
    let all = expansions(&cards);
    rsx! {
        Fieldset { title: "Kingdom Options", class: "lb-kingdom-options",
            KingdomDisplay { selector, study }
            if !code.is_empty() {
                div { class: "lb-kingdom-code", "data-kingdom-code": "{code}", "{code}" }
            }
            Expansions {}
            div { class: "lb-special-rules",
                if host {
                    button {
                        class: if selector() { "lb-button lb-stone lb-selector-button activated" } else { "lb-button lb-stone lb-selector-button" },
                        "data-action": "select-kingdom",
                        onclick: move |e| {
                            e.stop_propagation();
                            selector.set(!selector());
                        },
                        "Select Kingdom Cards"
                    }
                }
                SpecialSelector { colonies: true, value: r.colonies }
                SpecialSelector { colonies: false, value: r.shelters }
            }
            OptionRow { label: "Respected Cards",
                div { class: "lb-multibutton",
                    div {
                        class: if r.respect_banned { "lb-multi active" } else { "lb-multi" },
                        "data-action": "respect-banned",
                        title: "Never deal the cards on the Banned cards lists (Cardlists) of the players at the table",
                        onclick: move |_| ctx.edit(|r| r.respect_banned = !r.respect_banned),
                        "Banned"
                    }
                }
            }
            if host {
                div { class: "lb-load-clear",
                    div {
                        class: "lb-oval",
                        "data-action": "clear-selection",
                        onclick: move |_| {
                            let all = all.clone();
                            ctx.edit(move |r| r.clear_selection(&all));
                        },
                        "Clear Selection"
                    }
                }
                if let Some(why) = refused {
                    p { class: "lb-refused", role: "alert", "Not saved: {why}" }
                }
            }
        }
    }
}

/// The ten Kingdom slots (filled from the bottom left) and the landscape slots.
#[component]
fn KingdomDisplay(selector: Signal<bool>, study: Signal<Option<String>>) -> Element {
    let cards = use_cards();
    let ctx = use_lobby();
    let host = ctx.is_host();
    let assets = use_assets();
    let tip = use_tooltip();
    let Some(r) = ctx.rules.read().clone() else {
        return rsx! {};
    };
    let blanks = super::rules::KINGDOM_SLOTS.saturating_sub(r.kingdom.len());
    let mut picks: Vec<(usize, Pick)> = r.kingdom.iter().cloned().enumerate().collect();
    picks.sort_by(|(_, a), (_, b)| match (a, b) {
        (Pick::Card(a), Pick::Card(b)) => match (cards.get(a), cards.get(b)) {
            (Some(x), Some(y)) => super::rules::card_order(&cards, x, y),
            _ => a.cmp(b),
        },
        (Pick::Card(_), Pick::FromSet(_)) => std::cmp::Ordering::Less,
        (Pick::FromSet(_), Pick::Card(_)) => std::cmp::Ordering::Greater,
        (Pick::FromSet(a), Pick::FromSet(b)) => a.cmp(b),
    });
    let narrow = r.landscape_count() < 2;
    rsx! {
        div { class: "lb-kingdom-display",
            div { class: "lb-kingdom-cards",
                for (i, pick) in picks {
                    match pick {
                        Pick::Card(name) => rsx! {
                            div {
                                key: "c{name}",
                                class: "lb-kcard clickable",
                                "data-required": "{name}",
                                onclick: {
                                    let name = name.clone();
                                    move |e: MouseEvent| {
                                        e.stop_propagation();
                                        let name = name.clone();
                                        ctx.edit(move |r| r.remove(&name));
                                    }
                                },
                                oncontextmenu: {
                                    let name = name.clone();
                                    move |e: MouseEvent| {
                                        e.prevent_default();
                                        study.set(Some(name.clone()));
                                    }
                                },
                                CardView { name: name.clone(), size: CardSize::Mini }
                                div { class: "lb-red-cross" }
                            }
                        },
                        Pick::FromSet(set) => rsx! {
                            div {
                                key: "s{i}{set}",
                                class: "lb-kcard clickable",
                                "data-from-set": "{set}",
                                onclick: move |e: MouseEvent| {
                                    e.stop_propagation();
                                    ctx.edit(move |r| r.remove_pick(i));
                                },
                                SetTile { set: set.clone() }
                                div { class: "lb-red-cross" }
                            }
                        },
                    }
                }
                for i in 0..blanks {
                    div {
                        key: "b{i}",
                        class: if selector() { "lb-kcard lb-blank clickable active" } else { "lb-kcard lb-blank clickable" },
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            if host {
                                selector.set(true);
                            }
                        },
                        div { class: "lb-back" }
                    }
                }
            }
            div { class: if narrow { "lb-landscapes narrow" } else { "lb-landscapes" },
                for name in r.landscapes.iter().cloned() {
                    div {
                        key: "l{name}",
                        class: "lb-lcard clickable",
                        "data-required": "{name}",
                        onclick: {
                            let name = name.clone();
                            move |e: MouseEvent| {
                                e.stop_propagation();
                                let name = name.clone();
                                ctx.edit(move |r| r.remove(&name));
                            }
                        },
                        oncontextmenu: {
                            let name = name.clone();
                            move |e: MouseEvent| {
                                e.prevent_default();
                                study.set(Some(name.clone()));
                            }
                        },
                        CardView { name: name.clone(), size: CardSize::Mini }
                        div { class: "lb-red-cross" }
                    }
                }
                for (i, slot) in r.slots.iter().copied().enumerate() {
                    LandscapeSelector { key: "slot{i}-{slot.kinds}-{slot.empty}", index: i, slot }
                }
                if host && r.landscape_count() < super::rules::MAX_LANDSCAPES {
                    div {
                        class: "lb-lcard lb-add-landscape",
                        "data-action": "add-landscape",
                        onmouseenter: move |e| show_tooltip(tip, "Create a new slot for landscape cards and then select which types of landscapes can appear in it.".into(), &e),
                        onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                        onclick: move |_| { let mut tip = tip; tip.set(None); ctx.edit(|r| r.add_slot()); },
                        if assets.has("images/elements/add.png") { div { class: "lb-add-inner" } } else { span { class: "lb-glyph", "+" } }
                    }
                }
            }
        }
    }
}

/// A card back with an expansion's name and symbol: a random card from it.
#[component]
pub fn SetTile(set: String) -> Element {
    let assets = use_assets();
    let icon = assets.file_url(&expansion_icon(&set));
    rsx! {
        div { class: "lb-back" }
        div { class: "lb-set-wash" }
        div { class: "lb-set-title", "{set}" }
        if let Some(src) = icon {
            img { class: "lb-set-icon", src, alt: "" }
        }
    }
}

/// A landscape slot: the strips of what it may be; hovering opens them, a click on one
/// toggles that kind, the dice whether it may stay empty, the minus removes it.
#[component]
fn LandscapeSelector(index: usize, slot: LandscapeSlot) -> Element {
    let ctx = use_lobby();
    let assets = use_assets();
    let mut tip = use_tooltip();
    let mut hovered = use_signal(|| false);
    let toggle = move |kind: Option<usize>| {
        ctx.edit(move |r| {
            if let Some(s) = r.slots.get_mut(index)
                && !s.toggle(kind)
            {
                r.slots.remove(index);
            }
        });
    };
    let describe = slot.describe();
    rsx! {
        div {
            class: if hovered() { "lb-lcard lb-lselector hovered" } else { "lb-lcard lb-lselector" },
            "data-slot": "{index}",
            onmouseenter: move |e| { hovered.set(true); show_tooltip(tip, describe.clone(), &e); },
            onmouseleave: move |_| { hovered.set(false); tip.set(None); },
            div {
                class: "lb-lselector-remove",
                title: "Remove",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    tip.set(None);
                    ctx.edit(move |r| { if index < r.slots.len() { r.slots.remove(index); } });
                },
            }
            for (i, kind) in SLOT_KINDS.iter().enumerate() {
                div { key: "{kind}", class: "lb-lselector-part-box",
                    div {
                        class: if slot.takes(i) { "lb-lselector-part active" } else { "lb-lselector-part" },
                        "data-kind": "{kind}",
                        style: match assets.file_url(&format!("images/cards/templates/{}-selector.webp", kind.to_lowercase())) {
                            Some(u) => format!("background-image: url('{u}')"),
                            None => String::new(),
                        },
                        onclick: move |e: MouseEvent| { e.stop_propagation(); toggle(Some(i)); },
                        div { class: "lb-lselector-text", "{kind}" }
                    }
                }
            }
            div {
                class: if slot.empty { "lb-lselector-none" } else { "lb-lselector-none inactive" },
                title: "May stay empty",
                onclick: move |e: MouseEvent| { e.stop_propagation(); toggle(None); },
            }
        }
    }
}

/// The expansions random cards come from, and All.
#[component]
fn Expansions() -> Element {
    let ctx = use_lobby();
    let cards = use_cards();
    let assets = use_assets();
    let tip = use_tooltip();
    let all = expansions(&cards);
    let Some(on) = ctx.rules.read().as_ref().map(|r| r.sets.clone()) else {
        return rsx! {};
    };
    let any = !on.is_empty();
    rsx! {
        div { class: "lb-expansions",
            for set in all.iter().cloned() {
                {
                    let active = on.contains(&set);
                    let icon = assets.file_url(&expansion_icon(&set));
                    let s2 = set.clone();
                    let s3 = set.clone();
                    rsx! {
                        div {
                            key: "{set}",
                            class: if active { "lb-expansion active" } else { "lb-expansion" },
                            "data-set": "{set}",
                            onmouseenter: move |e| show_tooltip(tip, s2.clone(), &e),
                            onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                            onclick: move |_| { let s = s3.clone(); ctx.edit(move |r| r.toggle_set(&s)); },
                            if let Some(src) = icon {
                                img { class: "lb-expansion-icon", src, alt: "{set}" }
                            } else {
                                span { class: "lb-glyph", "{set.chars().next().unwrap_or('?')}" }
                            }
                        }
                    }
                }
            }
            {
                let all2 = all.clone();
                rsx! {
                    div {
                        class: if any { "lb-expansion active" } else { "lb-expansion" },
                        "data-set": "All",
                        onmouseenter: move |e| show_tooltip(tip, "All".into(), &e),
                        onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                        onclick: move |_| { let all = all2.clone(); ctx.edit(move |r| r.toggle_all_sets(&all)); },
                        if let Some(src) = assets.file_url("images/elements/fullscreen-icon.png") {
                            img { class: "lb-expansion-icon", src, alt: "All" }
                        } else {
                            span { class: "lb-glyph", "All" }
                        }
                    }
                }
            }
        }
    }
}

/// Colonies (Colony and Platinum, or Province and Gold) or Shelters (Hovel, Necropolis
/// and Overgrown Estate, or three Estates), a dice while Random, and the oval saying
/// which. A click cycles Random, Yes, No.
#[component]
fn SpecialSelector(colonies: bool, value: Ternary) -> Element {
    let ctx = use_lobby();
    let shown: &[&str] = match (colonies, value) {
        (true, Ternary::No) => &["Province", "Gold"],
        (true, _) => &["Colony", "Platinum"],
        (false, Ternary::No) => &["Estate", "Estate", "Estate"],
        (false, _) => &["Hovel", "Necropolis", "Overgrown Estate"],
    };
    let label = format!(
        "{}: {}",
        if colonies { "Colonies" } else { "Shelters" },
        value.label()
    );
    rsx! {
        div {
            class: "lb-special",
            "data-special": if colonies { "colonies" } else { "shelters" },
            "data-value": "{value.label()}",
            onclick: move |_| ctx.edit(move |r| {
                if colonies {
                    r.colonies = r.colonies.cycle();
                } else {
                    r.shelters = r.shelters.cycle();
                }
            }),
            div { class: "lb-special-cards",
                for (i, name) in shown.iter().enumerate() {
                    div { key: "{i}", class: "lb-special-crop", style: "z-index: {i}",
                        div { class: "lb-kcard",
                            CardView { name: name.to_string(), size: CardSize::Mini }
                        }
                    }
                }
                if value == Ternary::Random {
                    div { class: "lb-dice" }
                }
            }
            div { class: if value == Ternary::Yes { "lb-oval lb-special-button active" } else { "lb-oval lb-special-button" }, "{label}" }
        }
    }
}

#[component]
pub fn GameOptions() -> Element {
    let ctx = use_lobby();
    let host = ctx.is_host();
    let Some(r) = ctx.rules.read().clone() else {
        return rsx! {};
    };
    rsx! {
        Fieldset { title: "Game Options", class: "lb-game-options",
            OptionRow { label: "Turn limit",
                input {
                    class: "lb-rule-input",
                    "data-field": "turn-limit",
                    placeholder: "none",
                    readonly: !host,
                    value: "{r.turn_limit}",
                    oninput: move |e| { let v = e.value(); ctx.edit(move |r| r.turn_limit = v); },
                }
            }
            div { class: "lb-row", "data-option": "rated", title: "The game counts for ratings when every seat is a different registered player or bot",
                div { class: "lb-row-label", "Rated game" }
                div { class: "lb-row-rule" }
                Switch { on: r.rated, fixed: !host, onclick: move |_| ctx.edit(|r| r.rated = !r.rated) }
            }
            div { class: "lb-row", "data-option": "watchable", title: "Anyone signed in may watch the game: what every player may know, nobody's hand",
                div { class: "lb-row-label", "Spectators allowed" }
                div { class: "lb-row-rule" }
                Switch { on: r.watchable, fixed: !host, onclick: move |_| ctx.edit(|r| r.watchable = !r.watchable) }
            }
            div { class: "lb-row", "data-option": "timed", title: "A player who must decide and has left, or takes too long, forfeits",
                div { class: "lb-row-label", "Timed: leaving forfeits" }
                div { class: "lb-row-rule" }
                Switch { on: r.timed, fixed: !host, onclick: move |_| ctx.edit(|r| r.timed = !r.timed) }
            }
        }
    }
}

/// Replaces the table's seats with what `f` makes of them.
fn change_seats(
    ctx: LobbyCtx,
    toast: Signal<Option<String>>,
    f: impl FnOnce(&TableInfo, &mut Vec<SeatEdit>),
) {
    let Some(table) = ctx.table_now() else {
        return;
    };
    let mut seats = seat_edits(&table.seats);
    f(&table, &mut seats);
    spawn(ctx.set_seats(toast, seats));
}

/// Notes whether the token seat `name` of `table` is played at this screen.
fn play_here(table: &str, name: &str, here: bool) {
    let mut names = store::here(table);
    names.retain(|n| n != name);
    if here {
        names.push(name.to_string());
    }
    store::set_here(table, &names);
}

/// A token seat's name, which its host may change: sent once the typing has stopped.
#[component]
fn TokenName(index: usize, name: String, editable: bool) -> Element {
    let ctx = use_lobby();
    let toast = use_toast();
    let mut draft = use_signal(|| None::<String>);
    let mut typed = use_signal(|| 0u32);
    if !editable {
        return rsx! { span { class: "lb-person-fixed", "{name}" } };
    }
    rsx! {
        input {
            class: "lb-person-input",
            maxlength: "32",
            value: draft().unwrap_or(name),
            oninput: move |e| {
                let text = e.value();
                draft.set(Some(text.clone()));
                let n = typed() + 1;
                typed.set(n);
                spawn(async move {
                    platform::sleep(600).await;
                    let new = text.trim().to_string();
                    if *typed.peek() != n || new.is_empty() {
                        return;
                    }
                    let Some(table) = ctx.table_now() else {
                        return;
                    };
                    let old = table.seats.get(index).and_then(|s| s.name.clone());
                    let mut seats = seat_edits(&table.seats);
                    if seats.get(index) != old.clone().map(SeatEdit::Token).as_ref() {
                        return;
                    }
                    seats[index] = SeatEdit::Token(new.clone());
                    let mut here = store::here(&table.id);
                    for n in here.iter_mut().filter(|n| Some(&**n) == old.as_ref()) {
                        *n = new.clone();
                    }
                    store::set_here(&table.id, &here);
                    ctx.set_seats(toast, seats).await;
                    if *typed.peek() == n {
                        draft.set(None);
                    }
                });
            },
        }
    }
}

/// The seats: the players sitting at the table with their rating, whether they are
/// here and whether they are ready; the seats played by token, at this screen or handed
/// out; the open ones. The host changes them.
#[component]
pub fn Players() -> Element {
    let ctx = use_lobby();
    let toast = use_toast();
    let assets = use_assets();
    let tip = use_tooltip();
    let mut redraw = use_signal(|| 0u32);
    let _ = redraw();
    let Some(t) = ctx.table() else {
        return rsx! {};
    };
    let host = ctx.is_host() && t.game.is_none();
    let me = ctx
        .lobby
        .read()
        .as_ref()
        .and_then(|l| l.you.as_ref().map(|y| y.user.name.clone()));
    let randomize = ctx.rules.read().as_ref().is_some_and(|r| r.randomize_order);
    let here = store::here(&t.id);
    let filled = t.seats.iter().filter(|s| s.kind != SeatKind::Open).count();
    let has_add = assets.has("images/elements/add.png");
    let icon = move |name: &str, path: &str, glyph: &str| -> (String, Option<String>) {
        let has = assets.has(path);
        (
            format!("lb-icon lb-icon-{name}"),
            (!has).then(|| glyph.to_string()),
        )
    };
    let hide = move |_: MouseEvent| {
        let mut tip = tip;
        tip.set(None);
    };
    let n = t.seats.len();
    rsx! {
        Fieldset { title: "Players ({filled}/{n})", class: "lb-players",
            div { class: "lb-players-content",
                for (i, seat) in t.seats.iter().cloned().enumerate() {
                    {
                        let line_class = if seat.kind == SeatKind::Open { "lb-person empty" } else { "lb-person" };
                        let (crown, crown_g) = icon("crown", "images/elements/crown.png", "♛");
                        let (person, person_g) = icon("person", "images/elements/person.png", "☺");
                        let (bot, bot_g) = icon("bot", "images/elements/bot.png", "⚙");
                        let (add_bot, add_bot_g) = icon("add-bot", "images/elements/add-bot.png", "+⚙");
                        let (play, play_g) = icon("play", "images/elements/play.png", "▶");
                        let (exit, exit_g) = icon("exit", "images/elements/exit.png", "✕");
                        let (minus, minus_g) = icon("minus", "images/elements/minus.png", "−");
                        let ready = if seat.ready { "lb-icon lb-icon-ready active" } else { "lb-icon lb-icon-ready" };
                        let kind = seat.kind;
                        let up = host && i > 0 && !randomize;
                        let down = host && i + 1 < n && !randomize;
                        let name = seat.name.clone().unwrap_or_default();
                        let user = seat.user.clone();
                        let user_name = user.as_ref().map(|u| u.name.clone()).unwrap_or_default();
                        let is_host = kind == SeatKind::Player && user_name == t.host;
                        let is_me = kind == SeatKind::Player && Some(&user_name) == me.as_ref();
                        let played_here = here.contains(&name);
                        let table_id = t.id.clone();
                        rsx! {
                            div {
                                key: "{i}",
                                class: line_class,
                                "data-seat": "{i}",
                                "data-kind": "{kind:?}",
                                "data-ready": "{seat.ready}",
                                "data-here": "{seat.here}",
                                div { class: "lb-person-name",
                                    match kind {
                                        SeatKind::Open => rsx! { span { class: "lb-empty-label", "Empty Slot" } },
                                        SeatKind::Token => rsx! { TokenName { index: i, name: name.clone(), editable: host } },
                                        SeatKind::Player => rsx! {
                                            if let Some(u) = user.clone() {
                                                UserName { user: u }
                                            }
                                            if !seat.here {
                                                span { class: "lb-person-away", "away" }
                                            }
                                        },
                                    }
                                }
                                if up || down {
                                    div { class: "lb-order",
                                        if up { div { class: "lb-order-up", title: "Earlier", onclick: move |_| change_seats(ctx, toast, move |_, s| s.swap(i, i - 1)) } }
                                        if down { div { class: "lb-order-down", title: "Later", onclick: move |_| change_seats(ctx, toast, move |_, s| s.swap(i, i + 1)) } }
                                    }
                                }
                                match kind {
                                    SeatKind::Player => rsx! {
                                        if is_host {
                                            div { class: "{crown} c3", onmouseenter: move |e| show_tooltip(tip, "Host".into(), &e), onmouseleave: hide, if let Some(g) = crown_g { "{g}" } }
                                        } else {
                                            div { class: "{person} c3", onmouseenter: move |e| show_tooltip(tip, "Player".into(), &e), onmouseleave: hide, if let Some(g) = person_g { "{g}" } }
                                        }
                                        div { class: "{ready} c4", onmouseenter: move |e| show_tooltip(tip, if seat.ready { "Ready" } else { "Not Ready" }.into(), &e), onmouseleave: hide }
                                        if host && !is_me {
                                            div {
                                                class: "{exit} c7 clickable",
                                                "data-action": "kick",
                                                onmouseenter: move |e| show_tooltip(tip, "Kick".into(), &e),
                                                onmouseleave: hide,
                                                onclick: move |_| {
                                                    let mut tip = tip;
                                                    tip.set(None);
                                                    change_seats(ctx, toast, move |_, s| s[i] = SeatEdit::Open);
                                                },
                                                if let Some(g) = exit_g { "{g}" }
                                            }
                                        }
                                    },
                                    SeatKind::Token => rsx! {
                                        if played_here {
                                            div {
                                                class: if host { "{play} c3 clickable" } else { "{play} c3" },
                                                "data-action": "hand-out",
                                                onmouseenter: move |e| show_tooltip(tip, "Played at this screen. Click to hand its token out instead.".into(), &e),
                                                onmouseleave: hide,
                                                onclick: {
                                                    let (table_id, name) = (table_id.clone(), name.clone());
                                                    move |_| if host { play_here(&table_id, &name, !played_here); redraw += 1; }
                                                },
                                                if let Some(g) = play_g { "{g}" }
                                            }
                                        } else {
                                            div {
                                                class: if host { "{bot} c3 clickable" } else { "{bot} c3" },
                                                "data-action": "play-here",
                                                onmouseenter: move |e| show_tooltip(tip, if host { "Its token is handed out when the game starts: to an agent, or another browser. Click to play it at this screen instead." } else { "Played with a token the host hands out" }.into(), &e),
                                                onmouseleave: hide,
                                                onclick: {
                                                    let (table_id, name) = (table_id.clone(), name.clone());
                                                    move |_| if host { play_here(&table_id, &name, !played_here); redraw += 1; }
                                                },
                                                if let Some(g) = bot_g { "{g}" }
                                            }
                                        }
                                        div { class: "{ready} c4", onmouseenter: move |e| show_tooltip(tip, "Ready".into(), &e), onmouseleave: hide }
                                        if host {
                                            div {
                                                class: "{exit} c7 clickable",
                                                "data-action": "kick",
                                                onmouseenter: move |e| show_tooltip(tip, "Kick".into(), &e),
                                                onmouseleave: hide,
                                                onclick: move |_| {
                                                    let mut tip = tip;
                                                    tip.set(None);
                                                    change_seats(ctx, toast, move |_, s| s[i] = SeatEdit::Open);
                                                },
                                                if let Some(g) = exit_g { "{g}" }
                                            }
                                        }
                                    },
                                    SeatKind::Open => rsx! {
                                        if host {
                                            div {
                                                class: "{add_bot} c3 clickable",
                                                "data-action": "add-agent",
                                                onmouseenter: move |e| show_tooltip(tip, "Add Agent: a seat played with a token you hand out".into(), &e),
                                                onmouseleave: hide,
                                                onclick: move |_| {
                                                    let mut tip = tip;
                                                    tip.set(None);
                                                    change_seats(ctx, toast, move |t, s| {
                                                        let name = free_seat_name(&t.seats, "Agent");
                                                        play_here(&t.id, &name, false);
                                                        s[i] = SeatEdit::Token(name);
                                                    });
                                                },
                                                if let Some(g) = add_bot_g { "{g}" }
                                            }
                                            div {
                                                class: "{play} c4 clickable",
                                                "data-action": "hot-seat",
                                                onmouseenter: move |e| show_tooltip(tip, "Play this seat at this screen too".into(), &e),
                                                onmouseleave: hide,
                                                onclick: move |_| {
                                                    let mut tip = tip;
                                                    tip.set(None);
                                                    change_seats(ctx, toast, move |t, s| {
                                                        let name = free_seat_name(&t.seats, "Player");
                                                        play_here(&t.id, &name, true);
                                                        s[i] = SeatEdit::Token(name);
                                                    });
                                                },
                                                if let Some(g) = play_g { "{g}" }
                                            }
                                            if n > 2 {
                                                div {
                                                    class: "{minus} c5 clickable",
                                                    "data-action": "remove-slot",
                                                    onmouseenter: move |e| show_tooltip(tip, "Remove Slot".into(), &e),
                                                    onmouseleave: hide,
                                                    onclick: move |_| {
                                                        let mut tip = tip;
                                                        tip.set(None);
                                                        change_seats(ctx, toast, move |_, s| { s.remove(i); });
                                                    },
                                                    if let Some(g) = minus_g { "{g}" }
                                                }
                                            }
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
                if host && n < MAX_PLAYERS {
                    div { class: "lb-person fake",
                        div {
                            class: "lb-icon lb-icon-add c5 clickable",
                            "data-action": "add-slot",
                            onmouseenter: move |e| show_tooltip(tip, "Add Slot".into(), &e),
                            onmouseleave: hide,
                            onclick: move |_| change_seats(ctx, toast, |_, s| s.push(SeatEdit::Open)),
                            if !has_add { "+" }
                        }
                    }
                }
            }
            div { class: "lb-row lb-randomize",
                div { class: "lb-row-label", "Randomize Player Order" }
                Switch { on: randomize, fixed: !host, onclick: move |_| ctx.edit(|r| r.randomize_order = !r.randomize_order) }
            }
        }
    }
}

/// The seats of the running game whose tokens the host hands out: their link, token and
/// the line to give an agent, and whether they have joined.
#[component]
fn SeatLinks(game: GameSummary) -> Element {
    let ctx = use_lobby();
    let links = ctx
        .lobby
        .read()
        .as_ref()
        .map(handed_out)
        .unwrap_or_default();
    if links.is_empty() {
        return rsx! {};
    }
    let waiting = game
        .deciding
        .filter(|&d| links.iter().any(|l| l.seat == d))
        .filter(|&d| !game.joined.get(usize::from(d)).copied().unwrap_or(true))
        .and_then(|d| game.names.get(usize::from(d)).cloned());
    rsx! {
        Fieldset { title: "Seat Links", class: "lb-links",
            div { class: "lb-links-content",
                if let Some(name) = waiting {
                    p { class: "lb-waiting", role: "status", "The game is waiting for {name}, who hasn't joined. Give its agent the line below." }
                } else {
                    p { class: "lb-links-note", "Each seat's token is the only way to play it." }
                }
                for s in links {
                    SeatLink {
                        key: "{s.seat}",
                        game: game.game_id.clone(),
                        joined: game.joined.get(usize::from(s.seat)).copied(),
                        seat: s,
                    }
                }
            }
        }
    }
}

/// The running game's kingdom, which can't change until it ends.
#[component]
fn LockedKingdom(game: GameSummary, study: Signal<Option<String>>) -> Element {
    let cards = use_cards();
    let mut kingdom: Vec<String> = game.kingdom.clone();
    kingdom.sort_by(|a, b| match (cards.get(a), cards.get(b)) {
        (Some(x), Some(y)) => super::rules::card_order(&cards, x, y),
        _ => a.cmp(b),
    });
    let code = game
        .kingdom
        .iter()
        .chain(&game.landscapes)
        .map(|n| n.to_lowercase())
        .collect::<Vec<_>>()
        .join(", ");
    rsx! {
        Fieldset { title: "Kingdom Options", class: "lb-kingdom-options",
            div { class: "lb-kingdom-display",
                div { class: "lb-kingdom-cards",
                    for name in kingdom {
                        div {
                            key: "{name}",
                            class: "lb-kcard",
                            "data-dealt": "{name}",
                            oncontextmenu: {
                                let name = name.clone();
                                move |e: MouseEvent| {
                                    e.prevent_default();
                                    study.set(Some(name.clone()));
                                }
                            },
                            CardView { name: name.clone(), size: CardSize::Mini }
                        }
                    }
                }
                div {
                    class: "lb-landscapes",
                    style: "width: calc({game.landscapes.len().div_ceil(2).max(1) * 15} * var(--lu));",
                    for name in game.landscapes.iter().cloned() {
                        div {
                            key: "{name}",
                            class: "lb-lcard",
                            "data-dealt": "{name}",
                            oncontextmenu: {
                                let name = name.clone();
                                move |e: MouseEvent| {
                                    e.prevent_default();
                                    study.set(Some(name.clone()));
                                }
                            },
                            CardView { name: name.clone(), size: CardSize::Mini }
                        }
                    }
                }
            }
            div { class: "lb-kingdom-code", "data-kingdom-code": "{code}", "{code}" }
            p { class: "lb-locked-note", role: "status",
                "Game {game.game_id} is being played with this kingdom. The table's rules can be changed once it ends."
            }
        }
    }
}

/// The running game's players in turn order: their accounts, who plays at this screen,
/// who plays by a token handed out and whether they have joined.
#[component]
fn LockedPlayers(game: GameSummary) -> Element {
    let ctx = use_lobby();
    let assets = use_assets();
    let tip = use_tooltip();
    let host = ctx.table().map(|t| t.host).unwrap_or_default();
    let held = session::seats(&game.game_id);
    let n = game.names.len();
    let hide = move |_: MouseEvent| {
        let mut tip = tip;
        tip.set(None);
    };
    rsx! {
        Fieldset { title: "Players ({n}/{n})", class: "lb-players",
            div { class: "lb-players-content",
                for (i, name) in game.names.iter().cloned().enumerate() {
                    {
                        let here = held.iter().any(|h| usize::from(h.seat) == i);
                        let user = game.users.get(i).cloned().flatten();
                        let joined = here || user.is_some() || game.joined.get(i).copied().unwrap_or(false);
                        let (icon, path, glyph, what) = match &user {
                            Some(u) if u.name == host => ("lb-icon lb-icon-crown c3", "crown", "♛", "Host"),
                            Some(_) => ("lb-icon lb-icon-person c3", "person", "☺", "Player"),
                            None if here => ("lb-icon lb-icon-play c3", "play", "▶", "Played at this screen"),
                            None => ("lb-icon lb-icon-bot c3", "bot", "⚙", "Played with the seat's token"),
                        };
                        let glyph = (!assets.has(&format!("images/elements/{path}.png"))).then_some(glyph);
                        let light = if joined { "lb-icon lb-icon-ready active c4" } else { "lb-icon lb-icon-ready c4" };
                        let state = if here { "Here" } else if joined { "Joined" } else { "Hasn't joined yet" };
                        let deciding = game.deciding == Some(i as u8);
                        rsx! {
                            div { key: "{i}", class: "lb-person", "data-seat": "{i}", "data-joined": "{joined}",
                                div { class: "lb-person-name",
                                    if let Some(u) = user.clone() {
                                        UserName { user: u }
                                    } else {
                                        span { class: "lb-person-fixed", "{name}" }
                                    }
                                    if deciding { span { class: "lb-person-turn", "to play" } }
                                }
                                div { class: icon, onmouseenter: move |e| show_tooltip(tip, what.into(), &e), onmouseleave: hide,
                                    if let Some(g) = glyph { "{g}" }
                                }
                                div { class: light, onmouseenter: move |e| show_tooltip(tip, state.into(), &e), onmouseleave: hide }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Ready, and Not Ready once you are: the game starts when everyone at the table is.
#[component]
pub fn ReadyButton() -> Element {
    let ctx = use_lobby();
    let toast = use_toast();
    let mut busy = use_signal(|| false);
    let me = ctx
        .lobby
        .read()
        .as_ref()
        .and_then(|l| l.you.as_ref().map(|y| y.user.name.clone()));
    let Some(t) = ctx.table() else {
        return rsx! {};
    };
    let mine = |s: &&TableSeat| s.user.as_ref().map(|u| &u.name) == me.as_ref();
    let ready = t.seats.iter().find(mine).is_some_and(|s| s.ready);
    rsx! {
        button {
            class: if ready { "lb-button lb-stone activated" } else { "lb-button lb-gold" },
            "data-action": "ready",
            "data-ready": "{ready}",
            disabled: busy(),
            onclick: move |_| {
                if busy() {
                    return;
                }
                busy.set(true);
                spawn(async move {
                    ctx.set_ready(toast, !ready).await;
                    busy.set(false);
                });
            },
            if ready { "Not Ready" } else { "Ready" }
        }
    }
}

/// Ready (Play while the table's game is running), Leave Table, and what the table is
/// waiting for.
#[component]
fn TableButtons(table: TableInfo) -> Element {
    let ctx = use_lobby();
    let nav = navigator();
    let me = ctx
        .lobby
        .read()
        .as_ref()
        .and_then(|l| l.you.as_ref().map(|y| y.user.name.clone()));
    let note = waiting_for(&table, me.as_deref().unwrap_or_default());
    rsx! {
        if let Some(note) = note {
            p { class: "lb-table-note", role: "status", "{note}" }
        }
        div { class: "lb-table-buttons",
            if let Some(game) = table.game.clone() {
                button {
                    class: "lb-button lb-gold",
                    "data-action": "play",
                    onclick: move |_| { nav.push(Route::Game { id: game.clone() }); },
                    "Play"
                }
            } else {
                ReadyButton {}
            }
            button {
                class: "lb-button lb-copper",
                "data-action": "leave-table",
                onclick: move |_| {
                    // These buttons go with the table.
                    dioxus::core::spawn_forever(async move {
                        ctx.leave_table().await;
                        nav.push(Route::Lobby {});
                    });
                },
                "Leave Table"
            }
        }
    }
}
