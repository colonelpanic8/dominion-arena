//! Every card-shaped object, drawn by the card renderer, with filters, a size switch and
//! a width slider; clicking a card shows it large with its data.

use crate::app::use_cards;
use crate::assets::use_assets;
use crate::card::{CardSize, CardView, is_landscape, layout};
use dioxus::prelude::*;
use dominion_protocol::api::CardInfo;
use std::collections::BTreeSet;

const LANDSCAPE_WIDTH: f64 = 994.0 / 630.0;

#[component]
pub fn GalleryPage() -> Element {
    let cards = use_cards();
    let mut set = use_signal(String::new);
    let mut kind = use_signal(String::new);
    let mut ty = use_signal(String::new);
    let mut query = use_signal(String::new);
    let mut size = use_signal(|| CardSize::Full);
    let mut width = use_signal(|| 200.0f64);
    let mut selected = use_signal(|| None::<String>);

    let sets: BTreeSet<String> = cards.all().iter().map(|c| c.set.clone()).collect();
    let kinds: BTreeSet<String> = cards.all().iter().map(|c| c.kind.clone()).collect();
    let types: BTreeSet<String> = cards
        .all()
        .iter()
        .flat_map(|c| c.types.iter().cloned())
        .collect();
    let q = query.read().to_lowercase();
    let shown: Vec<&CardInfo> = cards
        .all()
        .iter()
        .filter(|c| set.read().is_empty() || c.set == *set.read())
        .filter(|c| kind.read().is_empty() || c.kind == *kind.read())
        .filter(|c| ty.read().is_empty() || c.types.contains(&ty.read()))
        .filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q))
        .collect();
    let w = *width.read();
    let sz = *size.read();
    let detail = selected.read().clone().and_then(|n| cards.get(&n).cloned());

    rsx! {
        section { class: "panel gallery-page",
            h2 { "Gallery" }
            div { class: "gallery-controls",
                select {
                    onchange: move |e| set.set(e.value()),
                    option { value: "", "All sets" }
                    for s in sets.iter() {
                        option { key: "{s}", value: "{s}", "{s}" }
                    }
                }
                select {
                    onchange: move |e| kind.set(e.value()),
                    option { value: "", "All kinds" }
                    for k in kinds.iter() {
                        option { key: "{k}", value: "{k}", "{k}" }
                    }
                }
                select {
                    onchange: move |e| ty.set(e.value()),
                    option { value: "", "All types" }
                    for t in types.iter() {
                        option { key: "{t}", value: "{t}", "{t}" }
                    }
                }
                input {
                    r#type: "search",
                    placeholder: "Name",
                    value: "{query}",
                    oninput: move |e| query.set(e.value()),
                }
                span { class: "gallery-sizes",
                    for (label , s) in [("Full", CardSize::Full), ("Mini", CardSize::Mini), ("Micro", CardSize::Micro)] {
                        button {
                            key: "{label}",
                            class: if sz == s { "toggle on" } else { "toggle" },
                            onclick: move |_| size.set(s),
                            "{label}"
                        }
                    }
                }
                label { class: "gallery-width",
                    "Width "
                    input {
                        r#type: "range",
                        min: "40",
                        max: "600",
                        value: "{w}",
                        oninput: move |e| {
                            if let Ok(v) = e.value().parse() {
                                width.set(v);
                            }
                        },
                    }
                    span { class: "mono", "{w:.0}px" }
                }
                span { class: "muted", "{shown.len()} cards" }
            }
            if cards.is_empty() {
                p { class: "muted", "Loading cards…" }
            }
            div { class: "gallery-grid",
                for c in shown {
                    div {
                        key: "{c.name}",
                        class: "gallery-item",
                        style: "width:{item_width(c, sz, w):.0}px",
                        onclick: {
                            let name = c.name.clone();
                            move |_| selected.set(Some(name.clone()))
                        },
                        CardView { name: c.name.clone(), size: sz }
                    }
                }
            }
        }
        if let Some(c) = detail {
            div { class: "gallery-modal", onclick: move |_| selected.set(None),
                div {
                    class: "gallery-detail panel",
                    onclick: move |e| e.stop_propagation(),
                    div {
                        class: "gallery-detail-card",
                        style: "width:{item_width(&c, CardSize::Full, 400.0):.0}px",
                        CardView { name: c.name.clone(), size: CardSize::Full }
                    }
                    CardData { card: c.clone() }
                }
            }
        }
    }
}

/// Landscapes are as tall as portrait cards of width `w` at full size.
fn item_width(c: &CardInfo, size: CardSize, w: f64) -> f64 {
    if is_landscape(c) && size != CardSize::Micro {
        w * LANDSCAPE_WIDTH
    } else {
        w
    }
}

#[component]
fn CardData(card: CardInfo) -> Element {
    let assets = use_assets();
    let files = assets.card(&card.name).cloned();
    let cost = card.cost.map(|c| {
        let mut s = format!("${}", c.coins);
        if c.potions > 0 {
            s.push_str(&format!(" {}P", c.potions));
        }
        if c.debt > 0 {
            s.push_str(&format!(" {}D", c.debt));
        }
        if card.variable_cost {
            s.push('*');
        }
        if card.overpay {
            s.push('+');
        }
        s
    });
    let mut rows: Vec<(&str, String)> = vec![
        ("Set", card.set.clone()),
        ("Kind", card.kind.clone()),
        ("Types", layout::type_line(&card)),
    ];
    if let Some(c) = cost {
        rows.push(("Cost", c));
    }
    if card.coins > 0 {
        rows.push(("Coins", card.coins.to_string()));
    }
    if card.potions > 0 {
        rows.push(("Potions", card.potions.to_string()));
    }
    if card.vp != 0 {
        rows.push(("VP", card.vp.to_string()));
    }
    let flags: Vec<&str> = [
        (card.kingdom, "Kingdom"),
        (card.basic, "basic"),
        (card.removed, "removed"),
    ]
    .into_iter()
    .filter_map(|(on, s)| on.then_some(s))
    .collect();
    if !flags.is_empty() {
        rows.push(("", flags.join(", ")));
    }
    if let Some(p) = &card.pile {
        rows.push(("Pile", p.clone()));
    }
    if let Some(f) = files {
        for (k, v) in f.fields() {
            rows.push((k, v.unwrap_or("—").to_string()));
        }
    } else {
        rows.push(("Assets", "none".into()));
    }
    rsx! {
        div { class: "gallery-data",
            h3 { "{card.name}" }
            table {
                for (k , v) in rows {
                    tr { key: "{k}{v}",
                        th { "{k}" }
                        td { "{v}" }
                    }
                }
            }
            pre { class: "gallery-text", "{card.text}" }
        }
    }
}
