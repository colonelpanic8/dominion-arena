//! The table's building blocks: a card or pile that can be a target, badges, card names
//! that open the study view, and the study view itself.

use super::layout::Rect;
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

const CUBES: [&str; 6] = ["red", "blue", "green", "yellow", "orange", "purple"];

pub fn cube_icon(seat: u8) -> String {
    format!("images/cubes/cube_{}.png", CUBES[usize::from(seat % 6)])
}

/// The class colouring a card's name by its types, as the log does: the first of
/// Victory, Event, Duration, Reserve, Treasure, Reaction, Curse, Hex, Boon, State,
/// Project, Artifact, Landmark, Way, Trait and Prophecy it is, else white.
pub fn type_class(cards: &CardDb, name: &str) -> &'static str {
    let Some(c) = cards.get(name) else {
        return "dt-t-none";
    };
    let is = |t: &str| c.types.iter().any(|x| x == t);
    [
        ("Victory", "dt-t-victory"),
        ("Event", "dt-t-event"),
        ("Duration", "dt-t-duration"),
        ("Reserve", "dt-t-reserve"),
        ("Treasure", "dt-t-treasure"),
        ("Reaction", "dt-t-reaction"),
        ("Curse", "dt-t-curse"),
        ("Hex", "dt-t-hex"),
        ("Boon", "dt-t-boon"),
        ("State", "dt-t-state"),
        ("Project", "dt-t-project"),
        ("Artifact", "dt-t-artifact"),
        ("Landmark", "dt-t-landmark"),
        ("Way", "dt-t-way"),
        ("Trait", "dt-t-trait"),
        ("Prophecy", "dt-t-prophecy"),
    ]
    .into_iter()
    .find(|(t, _)| is(t))
    .map_or("dt-t-action", |(_, class)| class)
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
        "journey-down" => "images/tokens/journey-token-back.png",
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
        "journey" | "journey-down" => "J",
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
    if matches!(b.kind.as_str(), "trait" | "note" | "bane")
        && let Some(card) = b.card.clone()
    {
        let label = (b.kind == "bane").then(|| "Bane".to_string());
        return rsx! {
            span { class: "dt-badge dt-b-trait dt-b-{b.kind}", title: "{b.text}",
                CardName { name: card, label }
                if let Some(n) = b.count {
                    " {n}"
                }
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

/// A card name in text, coloured by type; clicking or right-clicking it studies the card.
#[component]
pub fn CardName(name: String, #[props(default)] label: Option<String>) -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let class = type_class(&cards, &name);
    let shown = label.unwrap_or_else(|| name.clone());
    let n1 = name.clone();
    let n2 = name.clone();
    rsx! {
        span {
            class: "dt-name {class}",
            onclick: move |e| {
                e.stop_propagation();
                ctx.study(&n1);
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                e.stop_propagation();
                ctx.study(&n2);
            },
            "{shown}"
        }
    }
}

/// One card on the table: a stack of identical cards, a pile or a choice. A `target` is
/// clicked to answer (see [`Mark`]); its border takes the colour of what that does.
#[component]
pub fn TableCard(
    name: String,
    size: CardSize,
    #[props(default)] cost: Option<Cost>,
    /// The count badge, when given.
    #[props(default)]
    count: Option<u32>,
    #[props(default)] mark: Mark,
    /// An empty pile: washed out and crossed.
    #[props(default)]
    empty: bool,
    #[props(default)] back: bool,
    #[props(default)] badges: Vec<Badge>,
    #[props(default)] cubes: Vec<(u8, String)>,
    #[props(default)] label: Option<String>,
    #[props(default)] opens: Option<Popup>,
    #[props(default)] class: String,
    /// Names the zone or pile it stands for, for cards seen moving to and from it.
    #[props(default)]
    anchor: Option<String>,
    /// Where it is on the table, and how high it sits.
    #[props(default)]
    at: Option<Rect>,
    #[props(default)] z: i32,
    /// A buyable or gainable pile: the plus sign.
    #[props(default)]
    plus: bool,
    /// A pile holding different cards: a back peeking out from under it.
    #[props(default)]
    split: bool,
    /// Upside down, as an opponent's cards across the table.
    #[props(default)]
    inverted: bool,
    /// The "All" chip: playing or picking every copy.
    #[props(default)]
    all: bool,
) -> Element {
    let ctx = use_table();
    let assets = use_assets();
    let Mark {
        target,
        options,
        picked,
        ordered,
        tone,
    } = mark;
    let n_menu = name.clone();
    let clickable = target.is_some() || opens.is_some();
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
    let cross_url = super::super::interaction::cross(tone)
        .and_then(|c| assets.file_url(&format!("images/elements/{c}.png")));
    let counter_url = assets.file_url("images/elements/card-counter.png");
    let plus_url = assets.file_url("images/elements/plus-sign.png");
    let red_cross = assets.file_url("images/elements/red-cross.png");
    let n_picked = picked.len() as u32;
    // Picked copies of a stack split off from it, tilted, unless the whole stack is.
    let stack_n = count.unwrap_or(1);
    let split_off = n_picked > 0 && !ordered && stack_n > n_picked && size == CardSize::Full;
    let shown_count = if split_off {
        count.map(|c| c - n_picked)
    } else {
        count
    };
    let whole_picked = n_picked > 0 && !split_off;
    let style = at.map(|r| r.style()).unwrap_or_default();
    let z_style = if z != 0 {
        format!("z-index:{z};")
    } else {
        String::new()
    };
    let face = |picked_copy: bool| {
        rsx! {
            if back {
                CardBack { size }
            } else {
                CardView { name: name.clone(), size, cost: cost.clone(), shifted: count.is_some() && size == CardSize::Mini }
            }
            if empty {
                div { class: "dt-empty-x",
                    if let Some(src) = red_cross.clone() {
                        img { src, alt: "", draggable: "false" }
                    }
                }
            }
            if picked_copy {
                div { class: "dt-cross",
                    if let Some(src) = cross_url.clone() {
                        img { src, alt: "picked", draggable: "false" }
                    } else {
                        span { class: "dt-cross-text", "✕" }
                    }
                }
            }
        }
    };
    let order_text = ordered.then(|| {
        picked
            .iter()
            .map(|p| (p + 1).to_string())
            .collect::<Vec<_>>()
            .join(",")
    });
    rsx! {
        div {
            class: "dt-c {size_class} dt-tone-{tone} {class}",
            class: if target.is_some() { "dt-target" },
            class: if whole_picked { "dt-picked" },
            class: if empty { "dt-empty" },
            class: if clickable { "dt-click" },
            class: if wide { "dt-wide" },
            class: if inverted { "dt-inverted" },
            class: if at.is_some() { "dt-at" },
            style: "{style}{z_style}",
            "data-name": "{name}",
            "data-anchor": anchor,
            "data-target": if target.is_some() { "{target_attr}" },
            "data-options": if target.is_some() { "{options}" },
            onclick: move |e| {
                if let Some(t) = target {
                    e.stop_propagation();
                    ctx.act(t);
                } else if let Some(p) = open.clone() {
                    e.stop_propagation();
                    ctx.open(p);
                }
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                e.stop_propagation();
                if !back {
                    ctx.study(&n_menu);
                }
            },
            if split {
                div { class: "dt-split-back", CardBack { size } }
            }
            div { class: "dt-face", {face(whole_picked)} }
            if split_off {
                div {
                    class: "dt-picked-copy",
                    "data-unpick": "{target_attr}",
                    onclick: move |e| {
                        e.stop_propagation();
                        if let Some(t) = target {
                            ctx.unpick(t);
                        }
                    },
                    {face(true)}
                    if n_picked > 1 {
                        span { class: "dt-count",
                            if let Some(src) = counter_url.clone() { img { src: src, alt: "", draggable: "false" } }
                            span { "{n_picked}" }
                        }
                    }
                }
            }
            div { class: "dt-ov",
                if let Some(n) = shown_count {
                    span { class: "dt-count",
                        if let Some(src) = counter_url.clone() {
                            img { src, alt: "", draggable: "false" }
                        }
                        span { "{n}" }
                    }
                }
                if let Some(t) = order_text.filter(|t| !t.is_empty()) {
                    span { class: "dt-order-n", "{t}" }
                }
                if plus {
                    span { class: "dt-plus",
                        if let Some(src) = plus_url.clone() {
                            img { src, alt: "+", draggable: "false" }
                        } else {
                            "+"
                        }
                    }
                }
                if all {
                    span {
                        class: "dt-all",
                        "data-all": "{target_attr}",
                        onclick: move |e| {
                            e.stop_propagation();
                            if let Some(t) = target {
                                ctx.act_all(t);
                            }
                        },
                        "All"
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
            }
            if let Some(l) = label {
                div { class: "dt-label", "{l}" }
            }
        }
    }
}

/// The study view: the card large in the middle of the screen over a dim backdrop, and
/// beside it the other cards of its pile. The next click closes it.
#[component]
pub fn StudyLayer() -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let Some(name) = ctx.studied.read().clone() else {
        return rsx! {};
    };
    let landscape = cards.get(&name).is_some_and(crate::card::is_landscape);
    let (vw, vh) = platform::viewport();
    let (h, ratio) = if landscape {
        (0.43 * vh, super::layout::WIDE)
    } else {
        (0.675 * vh, super::layout::FULL)
    };
    let w = h / ratio;
    let x = vw * 0.5 - w / 2.0;
    let y = vh * 0.5 - h / 2.0;
    // The pile's other cards, for split and mixed piles.
    let pile_of = cards.get(&name).and_then(|c| c.pile.clone());
    let mut members: Vec<String> = match &pile_of {
        Some(p) => cards
            .all()
            .iter()
            .filter(|c| c.pile.as_deref() == Some(p.as_str()) && c.name != name)
            .map(|c| c.name.clone())
            .collect(),
        None => vec![],
    };
    members.truncate(12);
    let mh = 0.25 * vh;
    let mw = mh / super::layout::FULL;
    let room = (x - 0.04 * vh).max(mw);
    let step = if members.len() > 1 {
        ((room - mw) / (members.len() - 1) as f64).min(mw + 0.02 * vh)
    } else {
        0.0
    };
    rsx! {
        div {
            class: "dt-study",
            "data-study": "{name}",
            onmousedown: move |e| {
                e.stop_propagation();
                ctx.unstudy();
            },
            for (i, m) in members.iter().enumerate() {
                div {
                    key: "{m}",
                    class: "dt-study-member",
                    style: "left:{0.02 * vh + i as f64 * step:.0}px;top:{vh * 0.5 - mh / 2.0:.0}px;width:{mw:.0}px;",
                    CardView { name: m.clone(), size: CardSize::Full }
                }
            }
            div {
                class: "dt-study-card",
                style: "left:{x:.0}px;top:{y:.0}px;width:{w:.0}px;",
                CardView { name: name.clone(), size: CardSize::Full }
            }
        }
    }
}
