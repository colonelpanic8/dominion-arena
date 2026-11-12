//! My Table: the table's rules before a game, as dominion.games lays them out
//! (Kingdom Options and Game Options on the left, Players on the right, Ready and Leave
//! Table below), and starting a game from them.

use super::rules::{
    LandscapeSlot, MAX_PLAYERS, Pick, SLOT_KINDS, SeatKind, SeatSpec, TableRules, Ternary,
    expansion_icon, expansions,
};
use super::selector::CardSelector;
use super::store::{self, MyTable};
use super::{
    Fieldset, LobbyFrame, OptionRow, Switch, Tab, TooltipLayer, flash, new_table, show_tooltip,
    use_banned, use_table, use_toast, use_tooltip,
};
use crate::api;
use crate::app::{Route, use_cards};
use crate::assets::use_assets;
use crate::card::{CardSize, CardView};
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dioxus::router::Navigator;
use dominion_protocol::api::SeatToken;

/// Changes the table's rules, if there is a table.
pub fn edit(mut table: Signal<Option<MyTable>>, f: impl FnOnce(&mut TableRules)) {
    if let Some(t) = table.write().as_mut() {
        f(&mut t.rules);
    }
}

/// Ready: deals the table's kingdom, creates the game and goes to it, or stays at the
/// table to hand out the seats played elsewhere.
pub async fn start(
    mut table: Signal<Option<MyTable>>,
    banned: Vec<String>,
    toast: Signal<Option<String>>,
    nav: Navigator,
) {
    let Some(rules) = table.peek().as_ref().map(|t| t.rules.clone()) else {
        return;
    };
    let mut toast = toast;
    let fail = move |m: String| flash(toast, m);
    let req = match rules.deal_request(&banned) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    if let Err(e) = rules.names() {
        return fail(e);
    }
    toast.set(Some("Loading game...".into()));
    let dealt = match api::deal(&req).await {
        Ok(d) => d,
        Err(e) => return fail(e.message),
    };
    let order = rules.seat_order(dealt.seed ^ platform::random_seed());
    let game = match rules.new_game(&dealt, &order) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let created = match api::create_game(&game).await {
        Ok(c) => c,
        Err(e) => return fail(e.message),
    };
    let mut links: Vec<SeatToken> = vec![];
    for (i, seat) in created.seats.iter().enumerate() {
        if rules.seats[order[i]].plays_here() {
            session::hold(
                &created.game_id,
                HeldSeat {
                    seat: seat.seat,
                    name: seat.name.clone(),
                    token: seat.token.clone(),
                },
            );
        } else {
            links.push(seat.clone());
        }
    }
    store::set_last_kingdom(
        &dealt
            .kingdom
            .iter()
            .chain(&dealt.landscapes)
            .map(|n| n.to_lowercase())
            .collect::<Vec<_>>()
            .join(", "),
    );
    if let Some(you) = rules.seats.first() {
        store::set_name(&you.name);
    }
    let id = created.game_id.clone();
    let local = links.is_empty();
    table.set(Some(MyTable {
        rules,
        game: Some(id.clone()),
        links,
    }));
    toast.set(None);
    if local {
        nav.push(Route::Game { id });
    } else {
        nav.push(Route::MyTable {});
    }
}

#[component]
pub fn MyTableTab() -> Element {
    let cards = use_cards();
    let table = use_table();
    let selector = use_signal(|| false);
    let study = use_signal(|| None::<String>);
    let _ = cards;
    let db = use_context::<crate::app::Cards>().0;
    use_effect(move || {
        let db = db.read().clone();
        if !db.is_empty() {
            new_table(table, &expansions(&db));
        }
    });
    rsx! {
        LobbyFrame { tab: Tab::MyTable,
            if table.read().is_some() {
                div { class: "lb-table-content",
                    div { class: "lb-table-main",
                        KingdomOptions { table, selector, study }
                        GameOptions { table }
                    }
                    div { class: "lb-table-side",
                        Players { table }
                        SeatLinks { table }
                    }
                }
                TableButtons { table }
                if selector() {
                    div { class: "lb-selector-shield", onclick: { let mut selector = selector; move |_| selector.set(false) } }
                    CardSelector { table, selector, study }
                }
                StudyView { study }
            }
            TooltipLayer {}
        }
    }
}

/// The card selector, for pages outside the lobby (the score page's Edit Table).
#[component]
pub fn CardSelectorFor(
    table: Signal<Option<MyTable>>,
    selector: Signal<bool>,
    study: Signal<Option<String>>,
) -> Element {
    rsx! { CardSelector { table, selector, study } }
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
pub fn KingdomOptions(
    table: Signal<Option<MyTable>>,
    selector: Signal<bool>,
    study: Signal<Option<String>>,
) -> Element {
    let cards = use_cards();
    let Some(t) = table.read().clone() else {
        return rsx! {};
    };
    let r = t.rules;
    let code = r.kingdom_code(&cards);
    let all = expansions(&cards);
    rsx! {
        Fieldset { title: "Kingdom Options", class: "lb-kingdom-options",
            KingdomDisplay { table, selector, study }
            if !code.is_empty() {
                div { class: "lb-kingdom-code", "data-kingdom-code": "{code}", "{code}" }
            }
            Expansions { table }
            div { class: "lb-special-rules",
                button {
                    class: if selector() { "lb-button lb-stone lb-selector-button activated" } else { "lb-button lb-stone lb-selector-button" },
                    "data-action": "select-kingdom",
                    onclick: move |e| {
                        e.stop_propagation();
                        selector.set(!selector());
                    },
                    "Select Kingdom Cards"
                }
                SpecialSelector { table, colonies: true, value: r.colonies }
                SpecialSelector { table, colonies: false, value: r.shelters }
            }
            OptionRow { label: "Respected Cards",
                div { class: "lb-multibutton",
                    div {
                        class: if r.respect_banned { "lb-multi active" } else { "lb-multi" },
                        "data-action": "respect-banned",
                        title: "Never deal the cards on the Banned cards list (Cardlists)",
                        onclick: move |_| edit(table, |r| r.respect_banned = !r.respect_banned),
                        "Banned"
                    }
                }
            }
            div { class: "lb-load-clear",
                div {
                    class: "lb-oval",
                    "data-action": "clear-selection",
                    onclick: move |_| {
                        let all = all.clone();
                        edit(table, move |r| r.clear_selection(&all));
                    },
                    "Clear Selection"
                }
            }
        }
    }
}

/// The ten Kingdom slots (filled from the bottom left) and the landscape slots.
#[component]
fn KingdomDisplay(
    table: Signal<Option<MyTable>>,
    selector: Signal<bool>,
    study: Signal<Option<String>>,
) -> Element {
    let cards = use_cards();
    let assets = use_assets();
    let tip = use_tooltip();
    let Some(t) = table.read().clone() else {
        return rsx! {};
    };
    let r = t.rules;
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
                                        edit(table, move |r| r.remove(&name));
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
                                    edit(table, move |r| r.remove_pick(i));
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
                            selector.set(true);
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
                                edit(table, move |r| r.remove(&name));
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
                    LandscapeSelector { key: "slot{i}-{slot.kinds}-{slot.empty}", table, index: i, slot }
                }
                if r.landscape_count() < super::rules::MAX_LANDSCAPES {
                    div {
                        class: "lb-lcard lb-add-landscape",
                        "data-action": "add-landscape",
                        onmouseenter: move |e| show_tooltip(tip, "Create a new slot for landscape cards and then select which types of landscapes can appear in it.".into(), &e),
                        onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                        onclick: move |_| { let mut tip = tip; tip.set(None); edit(table, |r| r.add_slot()); },
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
fn LandscapeSelector(table: Signal<Option<MyTable>>, index: usize, slot: LandscapeSlot) -> Element {
    let assets = use_assets();
    let mut tip = use_tooltip();
    let mut hovered = use_signal(|| false);
    let toggle = move |kind: Option<usize>| {
        edit(table, move |r| {
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
                    edit(table, move |r| { if index < r.slots.len() { r.slots.remove(index); } });
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
fn Expansions(table: Signal<Option<MyTable>>) -> Element {
    let cards = use_cards();
    let assets = use_assets();
    let tip = use_tooltip();
    let all = expansions(&cards);
    let Some(t) = table.read().clone() else {
        return rsx! {};
    };
    let on = t.rules.sets;
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
                            onclick: move |_| { let s = s3.clone(); edit(table, move |r| r.toggle_set(&s)); },
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
                        onclick: move |_| { let all = all2.clone(); edit(table, move |r| r.toggle_all_sets(&all)); },
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
fn SpecialSelector(table: Signal<Option<MyTable>>, colonies: bool, value: Ternary) -> Element {
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
            onclick: move |_| edit(table, move |r| {
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
pub fn GameOptions(table: Signal<Option<MyTable>>) -> Element {
    let Some(t) = table.read().clone() else {
        return rsx! {};
    };
    let r = t.rules;
    rsx! {
        Fieldset { title: "Game Options", class: "lb-game-options",
            OptionRow { label: "Specify random seed",
                input {
                    class: "lb-rule-input",
                    "data-field": "seed",
                    placeholder: "random",
                    value: "{r.seed}",
                    oninput: move |e| { let v = e.value(); edit(table, move |r| r.seed = v); },
                }
            }
            OptionRow { label: "Turn limit",
                input {
                    class: "lb-rule-input",
                    "data-field": "turn-limit",
                    placeholder: "none",
                    value: "{r.turn_limit}",
                    oninput: move |e| { let v = e.value(); edit(table, move |r| r.turn_limit = v); },
                }
            }
        }
    }
}

/// The seats: you (the host), agents, seats this browser also plays, empty slots.
#[component]
pub fn Players(table: Signal<Option<MyTable>>) -> Element {
    let assets = use_assets();
    let tip = use_tooltip();
    let Some(t) = table.read().clone() else {
        return rsx! {};
    };
    let r = t.rules;
    let filled = r.seats.iter().filter(|s| s.kind != SeatKind::Open).count();
    let expanded = t.links.is_empty();
    let has_add = assets.has("images/elements/add.png");
    let icon = move |name: &str, path: &str, glyph: &str| -> (String, Option<String>) {
        let has = assets.has(path);
        (
            format!("lb-icon lb-icon-{name}"),
            (!has).then(|| glyph.to_string()),
        )
    };
    let n = r.seats.len();
    rsx! {
        Fieldset {
            title: "Players ({filled}/{n})",
            class: if expanded { "lb-players expanded" } else { "lb-players" },
            div { class: "lb-players-content",
                for (i, seat) in r.seats.iter().cloned().enumerate() {
                    {
                        let line_class = if seat.kind == SeatKind::Open { "lb-person empty" } else { "lb-person" };
                        let (crown, crown_g) = icon("crown", "images/elements/crown.png", "♛");
                        let (bot, bot_g) = icon("bot", "images/elements/bot.png", "⚙");
                        let (add_bot, add_bot_g) = icon("add-bot", "images/elements/add-bot.png", "+⚙");
                        let (play, play_g) = icon("play", "images/elements/play.png", "▶");
                        let (exit, exit_g) = icon("exit", "images/elements/exit.png", "✕");
                        let (minus, minus_g) = icon("minus", "images/elements/minus.png", "−");
                        let ready = if seat.kind == SeatKind::You { "lb-icon lb-icon-ready" } else { "lb-icon lb-icon-ready active" };
                        let kind = seat.kind;
                        let up = i > 0 && !r.randomize_order;
                        let down = i + 1 < n && !r.randomize_order;
                        rsx! {
                            div { key: "{i}", class: line_class, "data-seat": "{i}", "data-kind": "{kind:?}",
                                div { class: "lb-person-name",
                                    if kind == SeatKind::Open {
                                        span { class: "lb-empty-label", "Empty Slot" }
                                    } else {
                                        input {
                                            class: "lb-person-input",
                                            maxlength: "32",
                                            placeholder: "P{i + 1}",
                                            value: "{seat.name}",
                                            oninput: move |e| {
                                                let v = e.value();
                                                edit(table, move |r| if let Some(s) = r.seats.get_mut(i) { s.name = v; });
                                            },
                                        }
                                    }
                                }
                                if up || down {
                                    div { class: "lb-order",
                                        if up { div { class: "lb-order-up", title: "Earlier", onclick: move |_| edit(table, move |r| r.seats.swap(i, i - 1)) } }
                                        if down { div { class: "lb-order-down", title: "Later", onclick: move |_| edit(table, move |r| r.seats.swap(i, i + 1)) } }
                                    }
                                }
                                match kind {
                                    SeatKind::You => rsx! {
                                        div { class: "{crown} c3", onmouseenter: move |e| show_tooltip(tip, "Host".into(), &e), onmouseleave: move |_| { let mut tip = tip; tip.set(None); }, if let Some(g) = crown_g { "{g}" } }
                                        div { class: "{ready} c4", onmouseenter: move |e| show_tooltip(tip, "Not Ready".into(), &e), onmouseleave: move |_| { let mut tip = tip; tip.set(None); } }
                                    },
                                    SeatKind::Agent | SeatKind::HotSeat => rsx! {
                                        if kind == SeatKind::Agent {
                                            div { class: "{bot} c3", onmouseenter: move |e| show_tooltip(tip, "Agent: joins over MCP with the seat's token".into(), &e), onmouseleave: move |_| { let mut tip = tip; tip.set(None); }, if let Some(g) = bot_g { "{g}" } }
                                        } else {
                                            div { class: "{play} c3", onmouseenter: move |e| show_tooltip(tip, "Played in this browser".into(), &e), onmouseleave: move |_| { let mut tip = tip; tip.set(None); }, if let Some(g) = play_g { "{g}" } }
                                        }
                                        div { class: "{ready} c4", onmouseenter: move |e| show_tooltip(tip, "Ready".into(), &e), onmouseleave: move |_| { let mut tip = tip; tip.set(None); } }
                                        div {
                                            class: "{exit} c7 clickable",
                                            "data-action": "kick",
                                            onmouseenter: move |e| show_tooltip(tip, "Kick".into(), &e),
                                            onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                                            onclick: move |_| { let mut tip = tip; tip.set(None); edit(table, move |r| if let Some(s) = r.seats.get_mut(i) { *s = SeatSpec { name: String::new(), kind: SeatKind::Open }; }); },
                                            if let Some(g) = exit_g { "{g}" }
                                        }
                                    },
                                    SeatKind::Open => rsx! {
                                        div {
                                            class: "{add_bot} c3 clickable",
                                            "data-action": "add-agent",
                                            onmouseenter: move |e| show_tooltip(tip, "Add Agent".into(), &e),
                                            onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                                            onclick: move |_| {
                                                let mut tip = tip;
                                                tip.set(None);
                                                edit(table, move |r| {
                                                    let k = r.seats.iter().filter(|s| s.kind == SeatKind::Agent).count() + 1;
                                                    if let Some(s) = r.seats.get_mut(i) {
                                                        *s = SeatSpec { name: format!("Agent {k}"), kind: SeatKind::Agent };
                                                    }
                                                });
                                            },
                                            if let Some(g) = add_bot_g { "{g}" }
                                        }
                                        div {
                                            class: "{play} c4 clickable",
                                            "data-action": "hot-seat",
                                            onmouseenter: move |e| show_tooltip(tip, "Play this seat in this browser too".into(), &e),
                                            onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                                            onclick: move |_| {
                                                let mut tip = tip;
                                                tip.set(None);
                                                edit(table, move |r| if let Some(s) = r.seats.get_mut(i) {
                                                    *s = SeatSpec { name: format!("P{}", i + 1), kind: SeatKind::HotSeat };
                                                });
                                            },
                                            if let Some(g) = play_g { "{g}" }
                                        }
                                        if n > 2 {
                                            div {
                                                class: "{minus} c5 clickable",
                                                "data-action": "remove-slot",
                                                onmouseenter: move |e| show_tooltip(tip, "Remove Slot".into(), &e),
                                                onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                                                onclick: move |_| { let mut tip = tip; tip.set(None); edit(table, move |r| { r.seats.remove(i); }); },
                                                if let Some(g) = minus_g { "{g}" }
                                            }
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
                if n < MAX_PLAYERS {
                    div { class: "lb-person fake",
                        div {
                            class: "lb-icon lb-icon-add c5 clickable",
                            "data-action": "add-slot",
                            onmouseenter: move |e| show_tooltip(tip, "Add Slot".into(), &e),
                            onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                            onclick: move |_| edit(table, |r| r.seats.push(SeatSpec { name: String::new(), kind: SeatKind::Open })),
                            if !has_add { "+" }
                        }
                    }
                }
            }
            div { class: "lb-row lb-randomize",
                div { class: "lb-row-label", "Randomize Player Order" }
                Switch { on: r.randomize_order, onclick: move |_| edit(table, |r| r.randomize_order = !r.randomize_order) }
            }
        }
    }
}

/// The seats of the started game that others play: their link, token and the line to
/// give an agent. Where dominion.games has the table chat.
#[component]
fn SeatLinks(table: Signal<Option<MyTable>>) -> Element {
    let Some(t) = table.read().clone() else {
        return rsx! {};
    };
    let (Some(game), false) = (t.game.clone(), t.links.is_empty()) else {
        return rsx! {};
    };
    let origin = platform::origin();
    rsx! {
        Fieldset { title: "Seat Links", class: "lb-links",
            div { class: "lb-links-content",
                p { class: "lb-links-note", "Each seat's token is the only way to play it." }
                for s in t.links.iter().cloned() {
                    {
                        let link = format!("{origin}/game/{game}#token={}", platform::encode(&s.token));
                        let mcp = format!(
                            "Connect to the dominion MCP server at {origin}/mcp and call join_game with {{\"game_id\": \"{game}\", \"token\": \"{}\"}}",
                            s.token
                        );
                        let token = s.token.clone();
                        let l2 = link.clone();
                        let m2 = mcp.clone();
                        rsx! {
                            div { key: "{s.seat}", class: "lb-link", "data-link-seat": "{s.seat}",
                                div { class: "lb-link-name", "{s.name}" }
                                div { class: "lb-link-line", "data-mcp": "1", "{mcp}" }
                                div { class: "lb-link-buttons",
                                    div { class: "lb-oval", onclick: move |_| platform::copy(&m2), "Copy for Agent" }
                                    div { class: "lb-oval", onclick: move |_| platform::copy(&l2), "Copy Link" }
                                    div { class: "lb-oval", onclick: move |_| platform::copy(&token), "Copy Token" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Ready (or Play, while the table's game is on) and Leave Table.
#[component]
fn TableButtons(table: Signal<Option<MyTable>>) -> Element {
    let toast = use_toast();
    let banned = use_banned();
    let nav = navigator();
    let game = table.read().as_ref().and_then(|t| t.game.clone());
    let mut running = use_signal(|| None::<bool>);
    use_future(move || async move {
        loop {
            let watched = table.peek().as_ref().and_then(|t| t.game.clone());
            match watched {
                Some(id) => {
                    if let Ok(list) = api::games().await {
                        running.set(Some(list.iter().any(|g| g.game_id == id && g.running)));
                    }
                }
                None => running.set(Some(false)),
            }
            platform::sleep(1500).await;
        }
    });
    let playing = game.is_some() && running() == Some(true);
    rsx! {
        div { class: "lb-table-buttons",
            if playing {
                button {
                    class: "lb-button lb-gold",
                    "data-action": "play",
                    onclick: move |_| {
                        if let Some(id) = game.clone() {
                            nav.push(Route::Game { id });
                        }
                    },
                    "Play"
                }
            } else {
                button {
                    class: "lb-button lb-gold",
                    "data-action": "ready",
                    onclick: move |_| { spawn(start(table, banned.clone(), toast, nav)); },
                    "Ready"
                }
            }
            button {
                class: "lb-button lb-copper",
                "data-action": "leave-table",
                onclick: move |_| {
                    let mut table = table;
                    table.set(None);
                    nav.push(Route::Tables {});
                },
                "Leave Table"
            }
        }
    }
}
