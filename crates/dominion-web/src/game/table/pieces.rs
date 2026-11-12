//! The table's building blocks: a card that can be a target, badges, hoverable names
//! and the large preview.

use super::{Mark, Popup, use_table};
use crate::app::use_cards;
use crate::assets::use_assets;
use crate::card::{CardBack, CardSize, CardView};
use crate::cards::CardDb;
use crate::game::board::Badge;
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::Cost;

/// A player's colour, by seat.
pub fn seat_class(seat: u8) -> String {
    format!("dt-p{}", seat % 6)
}

const CUBES: [&str; 6] = ["red", "blue", "green", "yellow", "purple", "orange"];

pub fn cube_icon(seat: u8) -> String {
    format!("images/cubes/cube_{}.png", CUBES[usize::from(seat % 6)])
}

/// The class colouring a card's name by its types, as the log and prompts do.
pub fn type_class(cards: &CardDb, name: &str) -> &'static str {
    let Some(c) = cards.get(name) else {
        return "dt-t-none";
    };
    if crate::card::is_landscape(c) {
        return "dt-t-landscape";
    }
    let is = |t: &str| c.types.iter().any(|x| x == t);
    [
        ("Curse", "dt-t-curse"),
        ("Ruins", "dt-t-ruins"),
        ("Shelter", "dt-t-shelter"),
        ("Reaction", "dt-t-reaction"),
        ("Duration", "dt-t-duration"),
        ("Night", "dt-t-night"),
        ("Treasure", "dt-t-treasure"),
        ("Victory", "dt-t-victory"),
        ("Action", "dt-t-action"),
    ]
    .into_iter()
    .find(|(t, _)| is(t))
    .map_or("dt-t-none", |(_, class)| class)
}

/// The cached image drawing a badge, by its kind.
pub fn badge_icon(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "coffers" => "images/elements/coffers.png",
        "villagers" => "images/elements/villagers.png",
        "favors" => "images/elements/favors.png",
        "debt" => "images/elements/debt_130.png",
        "vp" => "images/elements/victory_150.png",
        "pirate-ship" => "images/elements/pirate_130.png",
        "embargo" => "images/elements/embargo.png",
        "trade-route" => "images/elements/coin-token.png",
        "-card" => "images/tokens/minuscard.png",
        "-coin" => "images/tokens/minuscoin.png",
        "journey" => "images/tokens/journey-token.png",
        "sun" => "images/elements/rsun.png",
        "+card" => "images/tokens/card.png",
        "+action" => "images/tokens/action.png",
        "+buy" => "images/tokens/buy.png",
        "+coin" => "images/tokens/coin.png",
        "-cost" => "images/tokens/ferry.png",
        "trashing" => "images/tokens/trash.png",
        _ => return None,
    })
}

/// What a badge says without its image.
fn badge_short(kind: &str) -> &str {
    match kind {
        "coffers" => "Cof",
        "villagers" => "Vil",
        "favors" => "Fav",
        "debt" => "D",
        "vp" => "VP",
        "pirate-ship" => "PS",
        "embargo" => "Emb",
        "trade-route" => "TR",
        "journey" => "J",
        "sun" => "☀",
        "trashing" => "Tr",
        "estate" => "Est",
        "active" => "✓",
        other => other,
    }
}

#[component]
pub fn BadgeView(badge: Badge) -> Element {
    let assets = use_assets();
    let b = &badge;
    if b.kind == "trait"
        && let Some(card) = b.card.clone()
    {
        return rsx! {
            span { class: "dt-badge dt-b-trait", title: "{b.text}",
                HoverName { name: card }
            }
        };
    }
    let image = badge_icon(&b.kind).and_then(|p| assets.file_url(p));
    let owner = b.player.map(seat_class).unwrap_or_default();
    let on_disc = b.player.is_some() && b.kind != "pirate-ship";
    let count = b
        .count
        .filter(|_| !matches!(b.kind.as_str(), "-card" | "-coin"));
    rsx! {
        span {
            class: "dt-badge dt-b-{b.kind} {owner}",
            class: if on_disc { "dt-disc" },
            title: "{b.text}",
            "data-badge": "{b.kind}",
            if let Some(src) = image {
                img { src, alt: "{b.kind}", draggable: "false" }
            } else {
                span { class: "dt-badge-text", "{badge_short(&b.kind)}" }
            }
            if let Some(n) = count {
                span { class: "dt-badge-n", "{n}" }
            }
        }
    }
}

/// A player's cube on a Project, Artifact holder and the like.
#[component]
pub fn Cube(seat: u8, name: String) -> Element {
    let assets = use_assets();
    let image = assets.file_url(&cube_icon(seat));
    rsx! {
        span { class: "dt-cube {seat_class(seat)}", title: "{name}",
            if let Some(src) = image {
                img { src, alt: "", draggable: "false" }
            }
        }
    }
}

/// A card name that shows the card after a moment's hover or on right-click.
#[component]
pub fn HoverName(name: String, #[props(default)] label: Option<String>) -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let class = type_class(&cards, &name);
    let shown = label.unwrap_or_else(|| name.clone());
    let n1 = name.clone();
    let n2 = name.clone();
    rsx! {
        span {
            class: "dt-name {class}",
            onmouseenter: move |e| ctx.hover_enter(&n1, e.client_coordinates().x, e.client_coordinates().y),
            onmouseleave: move |_| ctx.hover_leave(),
            oncontextmenu: move |e| {
                e.prevent_default();
                ctx.hover_now(&n2, e.client_coordinates().x, e.client_coordinates().y);
            },
            "{shown}"
        }
    }
}

/// One card on the table: a stack of identical cards, a pile or a choice. A `target`
/// glows and is clicked to answer (see [`Mark`]).
#[component]
pub fn TableCard(
    name: String,
    size: CardSize,
    #[props(default)] cost: Option<Cost>,
    /// Shown as a count badge when more than one (or always, for piles).
    #[props(default)]
    count: Option<u32>,
    #[props(default)] mark: Mark,
    #[props(default)] dim: bool,
    #[props(default)] back: bool,
    #[props(default)] badges: Vec<Badge>,
    #[props(default)] cubes: Vec<(u8, String)>,
    #[props(default)] label: Option<String>,
    #[props(default)] opens: Option<Popup>,
    #[props(default)] class: String,
) -> Element {
    let ctx = use_table();
    let Mark {
        target,
        options,
        picked,
        ordered,
    } = mark;
    let n_enter = name.clone();
    let n_menu = name.clone();
    let clickable = target.is_some() || opens.is_some();
    let picked_text = if picked.is_empty() {
        String::new()
    } else if ordered {
        picked
            .iter()
            .map(|p| (p + 1).to_string())
            .collect::<Vec<_>>()
            .join(",")
    } else {
        picked.len().to_string()
    };
    let size_class = match size {
        CardSize::Full => "dt-full",
        CardSize::Mini => "dt-mini",
        CardSize::Micro => "dt-micro",
    };
    let target_attr = target.map(|t| t.to_string()).unwrap_or_default();
    let wide = !back
        && use_cards()
            .get(&name)
            .is_some_and(crate::card::is_landscape);
    let open = opens.clone();
    let overlay = rsx! {
        if let Some(n) = count {
            span { class: "dt-count", "{n}" }
        }
        if !picked.is_empty() {
            span {
                class: "dt-pick",
                title: "Unselect",
                "data-unpick": "{target_attr}",
                onclick: move |e| {
                    e.stop_propagation();
                    if let Some(t) = target {
                        ctx.unpick(t);
                    }
                },
                "{picked_text}"
            }
        }
        if !badges.is_empty() || !cubes.is_empty() {
            span { class: "dt-tokens",
                for (i, b) in badges.iter().enumerate() {
                    BadgeView { key: "{i}", badge: b.clone() }
                }
                for (seat, who) in cubes.iter() {
                    Cube { key: "c{seat}", seat: *seat, name: who.clone() }
                }
            }
        }
    };
    rsx! {
        div {
            class: "dt-c {size_class} {class}",
            class: if target.is_some() { "dt-target" },
            class: if !picked.is_empty() { "dt-picked" },
            class: if dim { "dt-dim" },
            class: if clickable { "dt-click" },
            class: if wide { "dt-wide" },
            "data-name": "{name}",
            "data-target": if target.is_some() { "{target_attr}" },
            "data-options": if target.is_some() { "{options}" },
            onclick: move |e| {
                if let Some(t) = target {
                    ctx.act(t);
                } else if let Some(p) = open.clone() {
                    e.stop_propagation();
                    ctx.open(p);
                }
            },
            onmouseenter: move |e| {
                if !back {
                    ctx.hover_enter(&n_enter, e.client_coordinates().x, e.client_coordinates().y);
                }
            },
            onmouseleave: move |_| ctx.hover_leave(),
            oncontextmenu: move |e| {
                e.prevent_default();
                if !back {
                    ctx.hover_now(&n_menu, e.client_coordinates().x, e.client_coordinates().y);
                }
            },
            if back {
                CardBack { size: if size == CardSize::Micro { CardSize::Mini } else { size } }
            } else {
                CardView { name: name.clone(), size, cost }
            }
            div { class: "dt-ov", {overlay} }
            if let Some(l) = label {
                div { class: "dt-label", "{l}" }
            }
        }
    }
}

/// The full card shown after a hover, beside the pointer and never under it.
#[component]
pub fn HoverLayer() -> Element {
    let ctx = use_table();
    let h = ctx.hover.read().clone();
    let Some(h) = h.filter(|h| h.shown) else {
        return rsx! {};
    };
    let cards = use_cards();
    let landscape = cards.get(&h.name).is_some_and(crate::card::is_landscape);
    let (vw, vh) = platform::viewport();
    let width = if landscape {
        (vh * 0.42 / 0.634).min(vw * 0.45)
    } else {
        (vh * 0.62 / 1.578).min(vw * 0.3)
    };
    let height = width * if landscape { 0.634 } else { 1.578 };
    let gap = 24.0;
    let left = if h.x > vw / 2.0 {
        h.x - gap - width
    } else {
        h.x + gap
    };
    let left = left.clamp(4.0, (vw - width - 4.0).max(4.0));
    let top = (h.y - height / 2.0).clamp(4.0, (vh - height - 4.0).max(4.0));
    rsx! {
        div {
            class: "dt-preview",
            style: "left:{left:.0}px;top:{top:.0}px;width:{width:.0}px;",
            CardView { name: h.name.clone(), size: CardSize::Full }
        }
    }
}
