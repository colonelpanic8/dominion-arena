//! Cards drawn the way dominion.games draws them: a frame template chosen by type, the
//! art, and the name, cost, type line and rules text set over them. Without the asset
//! cache the same layout is drawn in CSS.
//!
//! A card fills its container's width and keeps its aspect ratio
//! ([`aspect_ratio`]); everything inside scales with that width.

pub mod fit;
pub mod layout;
pub mod measure;
pub mod text;

use crate::app::use_cards;
use crate::assets::{AssetDb, url, use_assets};
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::api::CardInfo;
use fit::{Font, Measure, fit_line, fit_text};
use layout::{Rect, Shape};
use text::{Line, LineKind, Piece, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardSize {
    /// The whole card.
    Full,
    /// A Supply pile on the table: art, name and cost.
    Mini,
    /// A one-line strip: name and cost.
    Micro,
}

impl CardSize {
    fn class(self) -> &'static str {
        match self {
            CardSize::Full => "dc-full",
            CardSize::Mini => "dc-mini",
            CardSize::Micro => "dc-micro",
        }
    }
}

/// Height over width of `card` drawn at `size` (landscapes are wider than tall).
pub fn aspect_ratio(card: Option<&CardInfo>, size: CardSize) -> f32 {
    layout::ratio(card.is_some_and(layout::is_landscape), size)
}

/// Whether a card is drawn horizontally: Events, Landmarks, Boons, Hexes, ...
pub fn is_landscape(card: &CardInfo) -> bool {
    layout::is_landscape(card)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Price {
    coins: u32,
    potions: u32,
    debt: u32,
}

/// One card. `cost` replaces the printed cost (cost reducers). `class` is added to the
/// card's root, for highlighting; `children` are drawn over the card (count badges,
/// tokens), in a layer covering it.
#[component]
pub fn CardView(
    name: String,
    size: CardSize,
    /// The cost to print, when it differs from the printed one (cost reducers).
    cost: Option<dominion_protocol::Cost>,
    #[props(default)] class: String,
    #[props(default = VNode::empty())] children: Element,
) -> Element {
    let cards = use_cards();
    let assets = use_assets();
    let info = cards.get(&name).cloned();
    let price = match (cost, info.as_ref().and_then(|c| c.cost)) {
        (Some(c), _) => Some(Price {
            coins: c.coins,
            potions: c.potions,
            debt: c.debt,
        }),
        (None, Some(c)) => Some(Price {
            coins: c.coins,
            potions: c.potions,
            debt: c.debt,
        }),
        (None, None) => None,
    };
    let m = measure::measurer(assets.fonts);
    let body = match &info {
        Some(info) => draw(info, size, price, &assets, m.as_ref()),
        None => draw_unknown(&name, size, &assets, m.as_ref()),
    };
    let ratio = info
        .as_ref()
        .map_or(layout::ratio(false, size), |c| Shape::of(c).ratio(size));
    let landscape = if info.as_ref().is_some_and(layout::is_landscape) {
        "dc-landscape"
    } else {
        "dc-portrait"
    };
    rsx! {
        div {
            class: "dc-card {size.class()} {landscape} {class}",
            style: "aspect-ratio: {1.0 / ratio};",
            "data-card": "{name}",
            {body}
            div { class: "dc-overlay", {children} }
        }
    }
}

/// A face-down card.
#[component]
pub fn CardBack(size: CardSize) -> Element {
    let assets = use_assets();
    let ratio = layout::ratio(false, size);
    let image = match size {
        CardSize::Full => assets.file_url("images/cards/templates/back.jpg"),
        CardSize::Mini => assets.file_url("images/cards/templates/back-mini.jpg"),
        CardSize::Micro => None,
    };
    rsx! {
        div {
            class: "dc-card dc-back {size.class()}",
            style: "aspect-ratio: {1.0 / ratio};",
            div { class: "dc-bg" }
            if let Some(src) = image {
                img { class: "dc-back-img", src, loading: "lazy", alt: "" }
            } else {
                div { class: "dc-back-fallback" }
            }
        }
    }
}

const HOVER_WIDTH: f64 = 280.0;
const HOVER_LANDSCAPE_WIDTH: f64 = 420.0;

/// Shows the full card beside the pointer while it is over `children`.
#[component]
pub fn CardHover(name: String, children: Element) -> Element {
    let mut at = use_signal(|| None::<(f64, f64)>);
    let cards = use_cards();
    let landscape = cards.get(&name).is_some_and(layout::is_landscape);
    let width = if landscape {
        HOVER_LANDSCAPE_WIDTH
    } else {
        HOVER_WIDTH
    };
    let height = width * layout::ratio(landscape, CardSize::Full) as f64;
    let place = move |e: MouseEvent| {
        let p = e.client_coordinates();
        at.set(Some((p.x, p.y)));
    };
    let popup = (*at.read()).map(|(x, y)| {
        let (vw, vh) = platform::viewport();
        let margin = 8.0;
        let mut left = x + 18.0;
        if left + width > vw - margin {
            left = x - 18.0 - width;
        }
        let left = left.min(vw - width - margin).max(margin);
        let top = (y - height / 2.0).min(vh - height - margin).max(margin);
        format!("left:{left:.0}px;top:{top:.0}px;width:{width:.0}px;")
    });
    rsx! {
        span {
            class: "dc-hover",
            onmouseenter: place,
            onmousemove: place,
            onmouseleave: move |_| at.set(None),
            {children}
            if let Some(style) = popup {
                div { class: "dc-hover-card", style,
                    CardView { name: name.clone(), size: CardSize::Full }
                }
            }
        }
    }
}

fn set_slug(set: &str) -> String {
    let mut slug = String::new();
    for c in set.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn title_text(name: &str, assets: &AssetDb) -> String {
    if assets.has_title_font() {
        name.to_string()
    } else {
        name.to_uppercase()
    }
}

fn draw_unknown(name: &str, size: CardSize, assets: &AssetDb, m: &dyn Measure) -> Element {
    let shape = Shape::plain();
    let ratio = shape.ratio(size);
    let name_box = shape.name(size);
    let title = title_text(name, assets);
    let fill = if size == CardSize::Micro { 0.8 } else { 0.7 };
    let name_size = fit_line(
        &title,
        Font::Title,
        name_box.width_cqw(),
        name_box.height_cqw(ratio),
        fill,
        m,
    );
    rsx! {
        div { class: "dc-bg" }
        div {
            class: "dc-frame",
            style: "{shape.template(size).css()}background:{layout::frame_background(&[])};",
        }
        if let Some(art) = shape.art(size) {
            div { class: "dc-art-ph", style: art.css() }
        }
        {name_node(&title, name_box, name_size, false)}
    }
}

fn name_node(title: &str, rect: Rect, size: f32, light: bool) -> Element {
    let color = if light { "dc-light" } else { "" };
    rsx! {
        div { class: "dc-name {color}", style: rect.css(),
            span { style: "font-size:{size:.3}cqw;", "{title}" }
        }
    }
}

fn draw(
    info: &CardInfo,
    size: CardSize,
    price: Option<Price>,
    assets: &AssetDb,
    m: &dyn Measure,
) -> Element {
    let shape = Shape::of(info);
    let ratio = shape.ratio(size);
    let files = assets.card(&info.name);
    let (art, template) = match size {
        CardSize::Full => (
            files.and_then(|f| f.art.as_deref()),
            files.and_then(|f| f.template.as_deref()),
        ),
        CardSize::Mini => (
            files.and_then(|f| f.mini_art.as_deref().or(f.art.as_deref())),
            files.and_then(|f| f.mini_template.as_deref()),
        ),
        CardSize::Micro => (None, files.and_then(|f| f.micro_template.as_deref())),
    };
    let fallback = template.is_none();
    let template_box = shape.template(size);
    let title = title_text(&info.name, assets);
    let mut name_box = shape.name(size);
    if size == CardSize::Micro
        && let Some(p) = price
    {
        let icons = [
            p.coins > 0 || (p.potions == 0 && p.debt == 0),
            p.potions > 0,
            p.debt > 0,
        ]
        .into_iter()
        .filter(|on| *on)
        .count() as f32;
        let start = 0.03 + 0.13 * icons + 0.01;
        name_box.w -= (start - name_box.x).max(0.0);
        name_box.x = name_box.x.max(start);
    }
    let fill = if size == CardSize::Micro { 0.8 } else { 0.7 };
    let name_size = fit_line(
        &title,
        Font::Title,
        name_box.width_cqw(),
        name_box.height_cqw(ratio),
        fill,
        m,
    );
    let light_title = match size {
        CardSize::Mini if shape.landscape => false,
        _ => shape.dark_title,
    };
    let types = shape.types(size).map(|rect| {
        let line = title_text(&layout::type_line(info), assets);
        let s = fit_line(
            &line,
            Font::Title,
            rect.width_cqw(),
            rect.height_cqw(ratio),
            0.6,
            m,
        );
        (rect, line, s)
    });
    let corner_types = shape.landscape && size == CardSize::Full;
    let lines = text::parse(&info.text);
    let (heirloom, body): (Vec<Line>, Vec<Line>) = lines
        .into_iter()
        .partition(|l| l.kind() == Some(LineKind::Heirloom));
    let text_layout = match (size, shape.text()) {
        (CardSize::Full, Some(rect)) if !body.is_empty() => {
            let max = rect.height_cqw(ratio) * if shape.landscape { 0.37 } else { 0.15 };
            Some((
                rect,
                fit_text(&body, rect.width_cqw(), rect.height_cqw(ratio), max, m),
            ))
        }
        _ => None,
    };
    let heirloom_layout = match (size, heirloom.first()) {
        (CardSize::Full, Some(line)) if !shape.landscape => {
            let rect = shape.heirloom_band();
            let l = fit_text(
                std::slice::from_ref(line),
                rect.width_cqw(),
                rect.height_cqw(ratio),
                rect.height_cqw(ratio) * 0.8,
                m,
            );
            Some((rect, l))
        }
        _ => None,
    };
    let cost = price.map(|p| (shape.cost(size), p));
    let production = (size == CardSize::Full && shape.basic && shape.treasure_only)
        .then_some((info.coins, info.potions));
    let symbol =
        (size == CardSize::Full && shape.basic && art.is_none()).then(|| basic_symbol(info));
    let set_icon = (size == CardSize::Full)
        .then(|| {
            assets.file_url(&format!(
                "images/elements/{}-small.png",
                set_slug(&info.set)
            ))
        })
        .flatten();
    let colors = layout::colors(info);
    let frame_bg = layout::frame_background(&colors);
    let dark = if shape.dark_text { "dc-dark" } else { "" };
    rsx! {
        div { class: "dc-bg" }
        if fallback {
            div {
                class: "dc-frame {dark}",
                style: "{template_box.css()}background:{frame_bg};",
            }
            if let (CardSize::Full, Some(rect)) = (size, shape.text()) {
                div { class: "dc-fb-panel {dark}", style: rect.css() }
            }
        }
        if let Some(rect) = shape.art(size) {
            if let Some(src) = art {
                img {
                    class: "dc-art",
                    src: url(src),
                    loading: "lazy",
                    alt: "",
                    style: rect.css(),
                }
            } else {
                div { class: "dc-art-ph", style: rect.css(),
                    if let Some(sym) = symbol {
                        div { class: "dc-symbol", {sym} }
                    }
                }
            }
        }
        if let Some(src) = template {
            img {
                class: "dc-template",
                src: url(src),
                loading: "lazy",
                alt: "",
                style: template_box.css(),
            }
        }
        {name_node(&title, name_box, name_size, light_title)}
        if let Some((rect, line, s)) = types {
            div {
                class: if corner_types { "dc-types dc-corner" } else { "dc-types" },
                class: if light_title && !corner_types { "dc-light" },
                style: rect.css(),
                span { style: "font-size:{s:.3}cqw;", "{line}" }
            }
        }
        if let Some((rect, l)) = text_layout {
            {text_node(rect, &l, shape.dark_text, "dc-text")}
        }
        if let Some((rect, l)) = heirloom_layout {
            {text_node(rect, &l, false, "dc-text dc-heirloom")}
        }
        if let Some((rect, p)) = cost {
            div {
                class: "dc-cost-box",
                style: "{rect.css()}font-size:{rect.height_cqw(ratio):.3}cqw;",
                {cost_node(p, info.variable_cost, info.overpay)}
            }
        }
        if let Some((coins, potions)) = production {
            for (i, rect) in shape.production().into_iter().enumerate() {
                div {
                    key: "{i}",
                    class: "dc-cost-box",
                    style: "{rect.css()}font-size:{rect.height_cqw(ratio):.3}cqw;",
                    if potions > 0 {
                        span { class: "dc-cost",
                            span { class: "dc-i dc-potion" }
                        }
                    } else {
                        span { class: "dc-cost",
                            span { class: "dc-i dc-coin", "{coins}" }
                        }
                    }
                }
            }
        }
        if let Some(src) = set_icon {
            img {
                class: "dc-set",
                src,
                loading: "lazy",
                alt: "",
                style: shape.set_icon().css(),
            }
        }
    }
}

/// What a basic card shows when its art (which carries it) is missing.
fn basic_symbol(info: &CardInfo) -> Element {
    if info.potions > 0 {
        rsx! { span { class: "dc-i dc-potion" } }
    } else if info.types.iter().any(|t| t == "Treasure") {
        rsx! { span { class: "dc-i dc-coin", "{info.coins}" } }
    } else {
        rsx! {
            span { class: "dc-vp",
                span { class: "dc-vp-n", "{info.vp}" }
                span { class: "dc-i dc-shield" }
            }
        }
    }
}

fn cost_node(p: Price, variable: bool, overpay: bool) -> Element {
    let coin = p.coins > 0 || (p.potions == 0 && p.debt == 0);
    let mark = match (variable, overpay) {
        (true, _) => "*",
        (false, true) => "+",
        _ => "",
    };
    rsx! {
        span { class: "dc-cost",
            if coin {
                span { class: "dc-i dc-coin", "{p.coins}" }
            }
            if p.potions > 0 {
                span { class: "dc-i dc-potion",
                    if p.potions > 1 {
                        "{p.potions}"
                    }
                }
            }
            if p.debt > 0 {
                span { class: "dc-i dc-debt", "{p.debt}" }
            }
            if !mark.is_empty() {
                span { class: "dc-mark", "{mark}" }
            }
        }
    }
}

fn text_node(rect: Rect, l: &fit::TextLayout, dark: bool, class: &str) -> Element {
    let dark = if dark { "dc-light" } else { "" };
    rsx! {
        div { class: "{class} {dark}", style: rect.css(),
            div { class: "dc-text-inner", style: "font-size:{l.size:.3}cqw;",
                for (i, line) in l.lines.iter().enumerate() {
                    if line.gap_before {
                        div { key: "g{i}", class: "dc-gap" }
                    }
                    if line.kind.is_none() {
                        div { key: "s{i}", class: "dc-sep" }
                    }
                    for (j, row) in line.rows.iter().enumerate() {
                        div { key: "r{i}-{j}", class: "dc-row {row_class(line.kind)}",
                            for p in row.iter() {
                                {piece_node(p)}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn row_class(kind: Option<LineKind>) -> &'static str {
    match kind {
        Some(LineKind::Bonus) => "dc-bonus",
        Some(LineKind::Huge) => "dc-huge",
        Some(LineKind::Italic) | Some(LineKind::Heirloom) => "dc-italic",
        _ => "",
    }
}

fn piece_node(p: &Piece) -> Element {
    let n = |v: &Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
    match p {
        Piece::Text(t, style) => {
            let class = match style {
                Style::Normal => "",
                Style::Bold => "dc-b",
                Style::Italic => "dc-it",
            };
            rsx! { span { class, "{t}" } }
        }
        Piece::Coin(v) => rsx! { span { class: "dc-i dc-coin", "{n(v)}" } },
        Piece::Debt(v) => rsx! { span { class: "dc-i dc-debt", "{n(v)}" } },
        Piece::Potion => rsx! { span { class: "dc-i dc-potion" } },
        Piece::Sun => rsx! { span { class: "dc-i dc-sun" } },
        Piece::Vp(v) => rsx! {
            span { class: "dc-vp",
                if v.is_some() {
                    span { class: "dc-vp-n", "{n(v)}" }
                }
                span { class: "dc-i dc-shield" }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::set_slug;

    #[test]
    fn set_names_become_icon_slugs() {
        assert_eq!(set_slug("Dark Ages"), "dark-ages");
        assert_eq!(set_slug("Cornucopia & Guilds"), "cornucopia-guilds");
        assert_eq!(set_slug("Base"), "base");
    }
}
