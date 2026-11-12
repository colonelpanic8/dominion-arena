//! "Select Cards": dominion.games' card selector over the table. The expansions filter
//! the cards (the chosen one's card back adds a random card from it), the input
//! searches every card and Enter adds the typed list; a click adds a card, a right-click
//! studies it. Randomize (ours) deals the random slots now, to show the kingdom the game
//! will start with.

use super::my_table::SetTile;
use super::rules::{
    DEPRECATED, EXPANSIONS, expansion_icon, expansions, search, selectable, set_cards,
};
use super::store;
use super::{flash, show_tooltip, use_banned, use_lobby, use_toast, use_tooltip};
use crate::api;
use crate::app::use_cards;
use crate::assets::use_assets;
use crate::card::{CardSize, CardView};
use dioxus::prelude::*;
use dominion_protocol::api::CardInfo;

#[component]
pub fn CardSelector(selector: Signal<bool>, study: Signal<Option<String>>) -> Element {
    let cards = use_cards();
    let ctx = use_lobby();
    let assets = use_assets();
    let tip = use_tooltip();
    let toast = use_toast();
    let banned = use_banned();
    let mut active = use_signal(|| Some("Base".to_string()));
    let mut query = use_signal(String::new);
    let Some(rules) = ctx.rules.read().clone() else {
        return rsx! {};
    };
    let mut sets = expansions(&cards);
    if cards.all().iter().any(|c| c.removed && selectable(c)) {
        sets.push(DEPRECATED.to_string());
    }
    let q = query();
    let shown: Vec<&CardInfo> = if q.trim().is_empty() {
        set_cards(&cards, active().as_deref())
    } else {
        let all: Vec<&CardInfo> = cards.all().iter().filter(|c| selectable(c)).collect();
        search(&cards, &all, &q)
    };
    let set_tiles: Vec<String> = if q.trim().is_empty() {
        active().filter(|s| s != DEPRECATED).into_iter().collect()
    } else {
        let mut found: Vec<(u8, String)> = EXPANSIONS
            .iter()
            .filter(|(s, _)| sets.iter().any(|x| x == s))
            .filter_map(|(s, _)| {
                let (name, q) = (s.to_lowercase(), q.trim().to_lowercase());
                let rank = if name == q {
                    Some(1)
                } else if name.starts_with(&q) {
                    Some(4)
                } else if q.len() > 3 && name.contains(&q) {
                    Some(5)
                } else {
                    None
                };
                rank.map(|r| (r, s.to_string()))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, s)| s).collect()
    };
    let last = store::last_kingdom();
    let db = cards.clone();
    let mut submit = move || {
        let text = query();
        if text.trim().is_empty() {
            return;
        }
        let db = db.clone();
        let mut unknown = vec![];
        ctx.edit(|r| unknown = r.apply_text(&db, &text));
        if !unknown.is_empty() {
            flash(
                toast,
                format!("No cards matching \"{}\" were found.", unknown.join(", ")),
            );
        }
        query.set(String::new());
    };
    let db2 = cards.clone();
    let replay = move |_| {
        if let Some(text) = store::last_kingdom() {
            let db = db2.clone();
            ctx.edit(|r| {
                r.kingdom.clear();
                r.landscapes.clear();
                r.apply_text(&db, &text);
            });
        }
    };
    let all = expansions(&cards);
    let randomize = move |_| {
        let players = ctx.table().map_or(2, |t| t.seats.len());
        let Some(req) = (ctx.rules.peek().as_ref()).map(|r| r.deal_request(&all, players, &banned))
        else {
            return;
        };
        spawn(async move {
            let req = match req {
                Ok(r) => r,
                Err(e) => return flash(toast, e),
            };
            match api::deal(&req).await {
                Ok(dealt) => ctx.edit(|r| r.take_deal(&dealt)),
                Err(e) => flash(toast, e.message),
            }
        });
    };
    rsx! {
        div { class: "lb-selector", onclick: move |e| e.stop_propagation(),
          div { class: "lb-fieldset lb-card-selector",
            div { class: "lb-fieldset-title", span { "Select Cards" } }
            div { class: "lb-selector-content",
                div { class: "lb-selector-expansions",
                    for set in sets.iter().cloned() {
                        {
                            let on = active().as_deref() == Some(set.as_str());
                            let icon = assets.file_url(&expansion_icon(&set));
                            let s1 = set.clone();
                            let s2 = set.clone();
                            rsx! {
                                div {
                                    key: "{set}",
                                    class: if on { "lb-expansion active" } else { "lb-expansion" },
                                    "data-filter-set": "{set}",
                                    onmouseenter: move |e| show_tooltip(tip, s1.clone(), &e),
                                    onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                                    onclick: move |_| {
                                        let s = s2.clone();
                                        if active().as_deref() == Some(s.as_str()) {
                                            active.set(None);
                                        } else {
                                            active.set(Some(s));
                                        }
                                    },
                                    if let Some(src) = icon {
                                        img { class: "lb-expansion-icon", src, alt: "{set}" }
                                    } else {
                                        span { class: "lb-glyph", "{set.chars().next().unwrap_or('?')}" }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "lb-selector-row",
                    input {
                        class: "lb-selector-input",
                        placeholder: "Enter cards to include in your kingdom",
                        value: "{query}",
                        autofocus: true,
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                submit();
                            }
                        },
                    }
                    if last.is_some() {
                        div { class: "lb-button lb-copper lb-selector-side", "data-action": "replay-last", onclick: replay, "Replay Last Kingdom" }
                    }
                    div {
                        class: "lb-button lb-copper lb-selector-side",
                        "data-action": "randomize",
                        title: "Deal the random slots now, to see and change the kingdom before Ready",
                        onclick: randomize,
                        "Randomize"
                    }
                }
                div { class: "lb-selector-cards",
                    for set in set_tiles {
                        {
                            let s = set.clone();
                            let full = rules.kingdom.len() >= super::rules::KINGDOM_SLOTS;
                            rsx! {
                                div {
                                    key: "tile{set}",
                                    class: if full { "lb-scard disabled" } else { "lb-scard" },
                                    "data-add-set": "{set}",
                                    onmouseenter: move |e| show_tooltip(tip, format!("Click to add a random card from {s} to your kingdom."), &e),
                                    onmouseleave: move |_| { let mut tip = tip; tip.set(None); },
                                    onclick: {
                                        let set = set.clone();
                                        move |_| {
                                            let set = set.clone();
                                            ctx.edit(move |r| { r.add_from_set(&set); });
                                            query.set(String::new());
                                        }
                                    },
                                    SetTile { set: set.clone() }
                                }
                            }
                        }
                    }
                    for c in shown {
                        {
                            let name = c.name.clone();
                            let landscape = super::rules::is_landscape(c);
                            let ok = rules.can_add(&cards, &name);
                            let class = match (landscape, ok) {
                                (true, true) => "lb-scard landscape",
                                (true, false) => "lb-scard landscape disabled",
                                (false, true) => "lb-scard",
                                (false, false) => "lb-scard disabled",
                            };
                            let db = cards.clone();
                            let n1 = name.clone();
                            let n2 = name.clone();
                            rsx! {
                                div {
                                    key: "{name}",
                                    class,
                                    "data-card": "{name}",
                                    onclick: move |_| {
                                        let db = db.clone();
                                        let n = n1.clone();
                                        ctx.edit(move |r| { r.add(&db, &n); });
                                        query.set(String::new());
                                    },
                                    oncontextmenu: move |e| {
                                        e.prevent_default();
                                        study.set(Some(n2.clone()));
                                    },
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
}
