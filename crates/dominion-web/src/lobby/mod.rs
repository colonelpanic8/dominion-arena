//! The lobby, laid out like dominion.games': the painting, the tab column on the left
//! and one framed window per tab. Automatch (Bot Game, Create Table, joining a seat),
//! My Table (the table's rules before a game), Tables (this server's games) and
//! Cardlists (the Banned cards list).

mod cardlists;
mod home;
pub mod my_table;
pub mod rules;
pub mod seat_link;
mod selector;
pub mod store;
mod tables;

use crate::app::{Route, Settings, use_cards};
use dioxus::prelude::*;
use store::MyTable;

pub use cardlists::CardlistsTab;
pub use home::AutomatchTab;
pub use my_table::MyTableTab;
pub use tables::TablesTab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Automatch,
    MyTable,
    Tables,
    Cardlists,
}

/// The table this browser sits at, kept in localStorage.
#[derive(Clone, Copy)]
pub struct TableState(pub Signal<Option<MyTable>>);

/// The status line at the bottom right ("Loading game...", errors).
#[derive(Clone, Copy)]
pub struct Toast(pub Signal<Option<String>>);

/// Provides [`TableState`] and [`Toast`] to the app.
pub fn use_provide_lobby() {
    let table = use_context_provider(|| TableState(Signal::new(store::load())));
    use_context_provider(|| Toast(Signal::new(None)));
    use_context_provider(|| Tooltip(Signal::new(None)));
    use_effect(move || store::save(table.0.read().as_ref()));
}

pub fn use_table() -> Signal<Option<MyTable>> {
    use_context::<TableState>().0
}

pub fn use_toast() -> Signal<Option<String>> {
    use_context::<Toast>().0
}

/// Shows `message` in the status line for a few seconds.
pub fn flash(mut toast: Signal<Option<String>>, message: String) {
    toast.set(Some(message.clone()));
    spawn(async move {
        crate::platform::sleep(4000).await;
        if toast.peek().as_deref() == Some(message.as_str()) {
            toast.set(None);
        }
    });
}

/// The Banned cards list from the server's settings.
pub fn use_banned() -> Vec<String> {
    use_context::<Settings>().0.read().banned_cards.clone()
}

/// Sits down at a new table (New Table), unless this browser already has one.
pub fn new_table(mut table: Signal<Option<MyTable>>, sets: &[String]) {
    if table.peek().is_none() {
        table.set(Some(MyTable {
            rules: rules::TableRules::new(&default_name(), sets),
            game: None,
            links: vec![],
        }));
    }
}

/// Your seat's name: the one you used last, else P1.
pub fn default_name() -> String {
    let n = store::name();
    if n.is_empty() { "P1".into() } else { n }
}

#[component]
pub fn LobbyFrame(tab: Tab, children: Element) -> Element {
    let cards = use_cards();
    let table = use_table();
    let toast = use_toast();
    let nav = navigator();
    let has_table = table.read().is_some();
    let class = match tab {
        Tab::Automatch => "lb-window lb-automatch",
        Tab::MyTable => "lb-window lb-my-table",
        Tab::Tables => "lb-window lb-tables",
        Tab::Cardlists => "lb-window lb-cardlists",
    };
    let sets = rules::expansions(&cards);
    rsx! {
        div { class: "lb-page",
            div { class: "lb-spear lb-spear-left" }
            div { class: "lb-spear lb-spear-right" }
            div {
                class: "lb-fullscreen",
                title: "Fullscreen",
                onclick: move |_| crate::platform::run_js(
                    "document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen()",
                ),
            }
            ul { class: "lb-tabs",
                li { class: if tab == Tab::Automatch { "lb-tab selected" } else { "lb-tab" },
                    onclick: move |_| { nav.push(Route::Lobby {}); },
                    button { class: "lb-tab-button", "Automatch" }
                }
                li { class: if tab == Tab::MyTable { "lb-tab selected" } else { "lb-tab" },
                    "data-tab": "table",
                    onclick: move |_| {
                        new_table(table, &sets);
                        nav.push(Route::MyTable {});
                    },
                    button { class: "lb-tab-button", if has_table { "My Table" } else { "New Table" } }
                }
                li { class: if tab == Tab::Tables { "lb-tab selected" } else { "lb-tab" },
                    onclick: move |_| { nav.push(Route::Tables {}); },
                    button { class: "lb-tab-button", "Tables" }
                }
                li { class: if tab == Tab::Cardlists { "lb-tab selected" } else { "lb-tab" },
                    onclick: move |_| { nav.push(Route::Cardlists {}); },
                    button { class: "lb-tab-button", "Cardlists" }
                }
            }
            div { class: "lb-window-container",
                div { class, {children} }
            }
            if let Some(m) = toast() {
                div { class: "lb-status", role: "status", "{m}" }
            }
        }
    }
}

/// dominion.games' fieldset: a title on a rule, framed left, right and below.
#[component]
pub fn Fieldset(
    title: String,
    #[props(default)] class: String,
    #[props(default = VNode::empty())] after_title: Element,
    children: Element,
) -> Element {
    rsx! {
        div { class: "lb-fieldset {class}",
            div { class: "lb-fieldset-title", span { "{title}" } {after_title} }
            {children}
        }
    }
}

/// A row of a fieldset: label, gold rule, control.
#[component]
pub fn OptionRow(label: String, children: Element) -> Element {
    rsx! {
        div { class: "lb-row",
            div { class: "lb-row-label", "{label}" }
            div { class: "lb-row-rule" }
            {children}
        }
    }
}

/// dominion.games' on/off switch.
#[component]
pub fn Switch(on: bool, onclick: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: if on { "lb-switch active" } else { "lb-switch" },
            role: "switch",
            "aria-checked": "{on}",
            onclick: move |_| onclick.call(()),
            div { class: "lb-switch-knob" }
        }
    }
}

/// The table-wide tooltip under the cursor's target.
#[derive(Clone, Copy)]
pub struct Tooltip(pub Signal<Option<(String, f64, f64)>>);

pub fn use_tooltip() -> Signal<Option<(String, f64, f64)>> {
    use_context::<Tooltip>().0
}

/// Shows `text` above the element under a mouse event.
pub fn show_tooltip(mut tip: Signal<Option<(String, f64, f64)>>, text: String, e: &MouseEvent) {
    let p = e.client_coordinates();
    tip.set(Some((text, p.x, p.y)));
}

#[component]
pub fn TooltipLayer() -> Element {
    let tip = use_tooltip();
    match tip() {
        Some((text, x, y)) => rsx! {
            div { class: "lb-tooltip", style: "left: {x}px; top: {y}px;", "{text}" }
        },
        None => rsx! {},
    }
}
