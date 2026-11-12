//! Cardlists: dominion.games' Banned cards list (its Liked and Disliked lists weight the
//! original's random deals, which the engine doesn't do). The list is the account's own:
//! a change signs a guest in where nobody is, and is saved with the account.

use super::rules::{expansion_icon, expansions, is_landscape, search, selectable, set_cards};
use super::{LobbyFrame, Tab, flash, signed_in, use_banned, use_toast};
use crate::account::use_account;
use crate::api;
use crate::app::{Route, Settings, use_cards};
use crate::assets::use_assets;
use crate::card::{CardSize, CardView};
use dioxus::prelude::*;
use dominion_protocol::api::CardInfo;
use dominion_protocol::lobby::UserKind;

#[component]
pub fn CardlistsTab() -> Element {
    let cards = use_cards();
    let assets = use_assets();
    let toast = use_toast();
    let account = use_account();
    let nav = navigator();
    let mut settings = use_context::<Settings>().0;
    let banned = use_banned();
    let mut active = use_signal(|| "Base".to_string());
    let mut query = use_signal(String::new);
    let sets = expansions(&cards);
    let q = query();
    let shown: Vec<&CardInfo> = if q.trim().is_empty() {
        set_cards(&cards, Some(&active()))
    } else {
        let all: Vec<&CardInfo> = cards.all().iter().filter(|c| selectable(c)).collect();
        search(&cards, &all, &q)
    };
    let save = move |next: Vec<String>| {
        spawn(async move {
            if !signed_in(account, toast, nav).await {
                return;
            }
            match api::set_banned_cards(&next).await {
                Ok(s) => settings.set(s),
                Err(e) => flash(toast, e.message),
            }
        });
    };
    let toggle = {
        let banned = banned.clone();
        move |name: String| {
            let mut next = banned.clone();
            if let Some(i) = next.iter().position(|n| *n == name) {
                next.remove(i);
            } else {
                next.push(name);
            }
            save(next);
        }
    };
    let db = cards.clone();
    let mut submit = {
        let toggle = toggle.clone();
        move || {
            let all: Vec<&CardInfo> = db.all().iter().filter(|c| selectable(c)).collect();
            if let Some(c) = search(&db, &all, &query()).first() {
                toggle(c.name.clone());
            }
            query.set(String::new());
        }
    };
    let owner = match account.me.read().as_ref() {
        Some(me) if me.user.kind == UserKind::Guest => {
            format!(
                "{}'s list, kept while this guest account is. Register (Account) to keep it.",
                me.user.name
            )
        }
        Some(me) => format!(
            "{}'s list: never dealt at random at a table you sit at that respects it, nor in your automatch games.",
            me.user.name
        ),
        None => "The server's list. Changing it makes it yours.".to_string(),
    };
    let mut listed: Vec<&CardInfo> = banned.iter().filter_map(|n| cards.get(n)).collect();
    listed.sort_by(|a, b| super::rules::card_order(&cards, a, b));
    rsx! {
        LobbyFrame { tab: Tab::Cardlists,
            div { class: "lb-cardlists",
                div { class: "lb-cardlist-buttons",
                    button { class: "lb-oval lb-cardlist-button active", "Banned cards" }
                    p { class: "lb-note lb-cardlist-owner", "{owner}" }
                }
                div { class: "lb-cardlist-body",
                    div { class: "lb-cardlist-sets",
                        for set in sets.iter().cloned() {
                            {
                                let icon = assets.file_url(&expansion_icon(&set));
                                let s = set.clone();
                                rsx! {
                                    div {
                                        key: "{set}",
                                        class: if active() == set { "lb-cardlist-set active" } else { "lb-cardlist-set" },
                                        onclick: move |_| { active.set(s.clone()); query.set(String::new()); },
                                        if let Some(src) = icon { img { src, alt: "" } }
                                        span { "{set}" }
                                    }
                                }
                            }
                        }
                        Link { class: "lb-cardlist-gallery", to: Route::Gallery {}, "Card Gallery" }
                    }
                    div { class: "lb-cardlist-main",
                        input {
                            class: "lb-selector-input",
                            placeholder: "Select Cards",
                            value: "{query}",
                            oninput: move |e| query.set(e.value()),
                            onkeydown: move |e| {
                                if e.key() == Key::Enter {
                                    submit();
                                }
                            },
                        }
                        div { class: "lb-selector-cards",
                            for c in shown.into_iter().filter(|c| c.kingdom || is_landscape(c)) {
                                {
                                    let name = c.name.clone();
                                    let on = banned.contains(&name);
                                    let landscape = is_landscape(c);
                                    let n = name.clone();
                                    let toggle = toggle.clone();
                                    rsx! {
                                        div {
                                            key: "{name}",
                                            class: if landscape { "lb-scard landscape" } else { "lb-scard" },
                                            "data-card": "{name}",
                                            "data-banned": "{on}",
                                            onclick: move |_| toggle(n.clone()),
                                            CardView { name: name.clone(), size: CardSize::Mini }
                                            if on { div { class: "lb-red-cross shown" } }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "lb-cardlist-list",
                        div { class: "lb-cardlist-count", "{listed.len()} banned" }
                        for c in listed {
                            {
                                let n = c.name.clone();
                                let toggle = toggle.clone();
                                rsx! {
                                    div {
                                        key: "{c.name}",
                                        class: "lb-cardlist-strip",
                                        title: "Remove from the list",
                                        onclick: move |_| toggle(n.clone()),
                                        CardView { name: c.name.clone(), size: CardSize::Micro }
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
