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
use dominion_protocol::api::{CardCost, CardInfo};
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

/// What a card's cost box shows: `cost` in place of the printed cost, and nothing for a
/// card with none printed (Landmarks, Ways, Boons, ...), whatever `cost` says.
fn price(cost: Option<dominion_protocol::Cost>, printed: Option<CardCost>) -> Option<Price> {
    let printed = printed?;
    Some(match cost {
        Some(c) => Price {
            coins: c.coins,
            potions: c.potions,
            debt: c.debt,
        },
        None => Price {
            coins: printed.coins,
            potions: printed.potions,
            debt: printed.debt,
        },
    })
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
    /// A pile's name, moved right to clear its count badge in the top left corner.
    #[props(default)]
    shifted: bool,
    /// A Supply pile's markers (Bane, Split, ...): its mini card prints them, then the
    /// types its cards show on a pile (Attack, ...), as dominion.games does.
    #[props(default)]
    pile: Option<Vec<String>>,
    /// The Trait on a Supply pile, drawn across the top of its mini card.
    #[props(default)]
    attached: Option<String>,
    /// A Trait drawn across its pile (`attached`): its name fills the strip.
    #[props(default)]
    inset: bool,
    /// A name to print instead of the card's own (a Boon or Hex deck's strip).
    #[props(default)]
    title: Option<String>,
    /// Turned towards the player across the table, as dominion.games turns the active
    /// opponent's cards: the frame upside down, the name and type line swapped, the art,
    /// cost and words upright and the text left off.
    #[props(default)]
    inverted: bool,
    #[props(default = VNode::empty())] children: Element,
) -> Element {
    let cards = use_cards();
    let assets = use_assets();
    let info = cards.get(&name).cloned();
    let price = price(cost, info.as_ref().and_then(|c| c.cost));
    let m = measure::measurer(assets.fonts);
    let body = match &info {
        Some(info) => draw(
            info,
            size,
            price,
            Placing {
                shifted,
                pile: pile.as_deref(),
                attached: attached.as_deref().filter(|_| size == CardSize::Mini),
                inset,
                title: title.as_deref(),
                inverted: inverted && size == CardSize::Full,
            },
            &assets,
            m.as_ref(),
        ),
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
    let fill = match size {
        CardSize::Micro => 0.8,
        CardSize::Full if shape.landscape => 0.9,
        _ => 0.7,
    };
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

/// Where a card lies, as far as its face shows it (`CardView`'s props of that name).
struct Placing<'a> {
    shifted: bool,
    pile: Option<&'a [String]>,
    attached: Option<&'a str>,
    inset: bool,
    title: Option<&'a str>,
    inverted: bool,
}

fn draw(
    info: &CardInfo,
    size: CardSize,
    price: Option<Price>,
    placing: Placing<'_>,
    assets: &AssetDb,
    m: &dyn Measure,
) -> Element {
    let Placing {
        shifted,
        pile,
        attached,
        inset,
        title,
        inverted,
    } = placing;
    let shape = Shape::of(info);
    let inverted = inverted && !shape.landscape;
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
    // A pile with a Trait has its frame with room for it, where the cache has one.
    let trait_template = attached
        .and(template)
        .and_then(|t| t.strip_suffix(".webp"))
        .map(|t| format!("{t}-trait.webp"))
        .filter(|t| assets.has(t));
    let template = trait_template.as_deref().or(template);
    let fallback = template.is_none();
    let template_box = shape.template(size);
    let title = title_text(title.unwrap_or(&info.name), assets);
    let mut name_box = shape.name(size);
    // Beside a count: upstream's "shifted-name" box.
    if shifted && (size == CardSize::Micro || (size == CardSize::Mini && !shape.landscape)) {
        name_box.x = 0.24;
        name_box.w = 0.68;
    }
    if attached.is_some() && !shape.landscape {
        name_box.y = 0.16;
        name_box.h = 0.12;
    }
    if inset && size == CardSize::Micro {
        name_box = layout::ATTACHED_NAME;
    }
    if inverted {
        name_box = layout::INVERTED_NAME.flipped_from(name_box);
    }
    // Upstream prints no cost on a micro card.
    let price = price.filter(|_| size != CardSize::Micro);
    let fill = match size {
        CardSize::Micro => 0.8,
        CardSize::Full if shape.landscape => 0.9,
        _ => 0.7,
    };
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
    let corner_types = shape.landscape && size == CardSize::Full;
    let types = shape.types(size).map(|rect| {
        let rect = if inverted {
            layout::INVERTED_TYPES.flipped_from(rect)
        } else {
            rect
        };
        let line = title_text(&layout::type_line(info), assets);
        let s = fit_line(
            &line,
            Font::Title,
            rect.width_cqw(),
            rect.height_cqw(ratio),
            if corner_types { 0.75 } else { 0.6 },
            m,
        );
        (rect, line, s)
    });
    let pile_types = pile.filter(|_| size == CardSize::Mini).and_then(|markers| {
        let rect = shape.pile_types()?;
        let parts: Vec<&str> = markers.iter().map(String::as_str).collect();
        if parts.is_empty() {
            return None;
        }
        let line = title_text(&parts.join("/"), assets);
        let s = fit_line(
            &line,
            Font::Title,
            rect.width_cqw(),
            rect.height_cqw(ratio),
            0.6,
            m,
        );
        Some((rect, line, s))
    });
    // dominion.games' own lines where the server has them, else the official text wrapped.
    let online = info
        .online_text
        .as_deref()
        .filter(|t| size == CardSize::Full && !inverted && !shape.basic && !t.is_empty())
        .map(|t| {
            let lines = text::parse_online(t);
            let rect = shape.online_text();
            let s = fit::online_size(
                &lines,
                rect.width_cqw(),
                rect.height_cqw(ratio),
                shape.landscape,
                m,
            );
            (rect, s, lines)
        });
    let lines = match online {
        Some(_) => vec![],
        None if inverted => vec![],
        None => text::parse(&info.text),
    };
    let (heirloom, body): (Vec<Line>, Vec<Line>) = lines
        .into_iter()
        .partition(|l| l.kind() == Some(LineKind::Heirloom));
    let text_layout = match (size, shape.text()) {
        (CardSize::Full, Some(rect)) if !body.is_empty() && !inverted => {
            let max = rect.height_cqw(ratio) * if shape.landscape { 0.37 } else { 0.15 };
            Some((
                rect,
                fit_text(&body, rect.width_cqw(), rect.height_cqw(ratio), max, m),
            ))
        }
        _ => None,
    };
    let heirloom_layout = match (size, heirloom.first()) {
        (CardSize::Full, Some(line)) if !shape.landscape && !inverted => {
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
    // dominion.games' own list where the server has it, else the basic Treasures'.
    let production = (size == CardSize::Full && !inverted)
        .then(|| match &info.online_production {
            Some(p) => Some(p.clone()),
            None if shape.basic && shape.treasure_only => Some(if info.potions > 0 {
                "P".to_string()
            } else {
                info.coins.to_string()
            }),
            None => None,
        })
        .flatten();
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
                class: if inverted { "dc-inverted" },
                style: "{template_box.css()}background:{frame_bg};",
            }
            if let (CardSize::Full, Some(rect)) = (size, shape.text()) {
                div { class: "dc-fb-panel {dark}", style: rect.css() }
            }
        }
        if let Some(rect) = shape.art(size).map(|r| if inverted { r.flipped() } else { r }) {
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
                class: if inverted { "dc-inverted" },
                src: url(src),
                loading: "lazy",
                alt: "",
                style: template_box.css(),
            }
        }
        {name_node(&title, name_box, name_size, light_title)}
        if let Some(t) = attached {
            // A frame made for a Trait has its banner: the Trait's name alone goes on it.
            div {
                class: "dc-attached",
                class: if trait_template.is_some() { "dc-on-banner" },
                style: layout::ATTACHED.css(),
                CardView { name: t.to_string(), size: CardSize::Micro, inset: true }
            }
        }
        if let Some((rect, line, s)) = types {
            div {
                class: if corner_types { "dc-types dc-corner" } else { "dc-types" },
                class: if light_title && !corner_types { "dc-light" },
                style: rect.css(),
                span { style: "font-size:{s:.3}cqw;", "{line}" }
            }
        }
        if let Some((rect, line, s)) = pile_types {
            div {
                class: "dc-types dc-pile-types",
                class: if light_title { "dc-light" },
                style: rect.css(),
                span { style: "font-size:{s:.3}cqw;", "{line}" }
            }
        }
        if let Some((rect, s, lines)) = online {
            {online_text_node(rect, s, &lines, shape.landscape, shape.dark_text)}
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
                {cost_node(p)}
            }
        }
        if let Some(made) = production {
            for (i, rect) in shape.production().into_iter().enumerate() {
                div {
                    key: "{i}",
                    class: "dc-cost-box",
                    style: "{rect.css()}font-size:{rect.height_cqw(ratio):.3}cqw;",
                    if made == "P" {
                        span { class: "dc-cost",
                            span { class: "dc-i dc-potion" }
                        }
                    } else {
                        span { class: "dc-cost",
                            span { class: "dc-i dc-coin", "{made}" }
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

/// A cost as dominion.games draws it: no mark for a variable cost or overpaying.
fn cost_node(p: Price) -> Element {
    let coin = p.coins > 0 || (p.potions == 0 && p.debt == 0);
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

/// dominion.games' own lines, drawn as its TextLayer draws them: centred, one block per
/// piece, each line unbroken.
fn online_text_node(
    rect: Rect,
    size: f32,
    lines: &[Line],
    landscape: bool,
    light: bool,
) -> Element {
    let (separator, gap) = fit::online_spacing(landscape);
    rsx! {
        div { class: "dc-ol", class: if light { "dc-light" }, style: rect.css(),
            div { class: "dc-ol-lines", style: "font-size:{size:.3}cqw;",
                for (i, line) in lines.iter().enumerate() {
                    match line {
                        Line::Separator => rsx! {
                            div {
                                key: "{i}",
                                class: "dc-ol-line dc-ol-sep",
                                style: "height:{0.2 * separator:.3}em;margin:{0.5 * separator:.3}em 0 {0.3 * separator:.3}em;",
                            }
                        },
                        Line::Gap => rsx! {
                            div { key: "{i}", class: "dc-ol-line", style: "height:{gap}em;" }
                        },
                        Line::Text { kind, pieces } => rsx! {
                            div { key: "{i}", class: "dc-ol-line {online_line_class(*kind)}",
                                for p in pieces.iter() {
                                    {online_block(p)}
                                }
                            }
                        },
                    }
                }
            }
        }
    }
}

fn online_line_class(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Normal => "",
        LineKind::Bonus => "dc-ol-bold",
        LineKind::Huge => "dc-ol-huge",
        LineKind::Italic => "dc-ol-italic",
        LineKind::Heirloom => "dc-ol-heirloom",
    }
}

fn online_block(p: &Piece) -> Element {
    let (left, right) = text::block_margins(p);
    let margins = match (left, right) {
        (true, true) => "",
        (false, true) => "dc-nl",
        (true, false) => "dc-nr",
        (false, false) => "dc-nl dc-nr",
    };
    let n = |v: &Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
    match p {
        Piece::Text(t, style) => {
            let style = match style {
                Style::Normal => "",
                Style::Bold => "dc-b",
                Style::Italic => "dc-it",
            };
            rsx! { span { class: "dc-ol-block {margins} {style}", "{t}" } }
        }
        Piece::Coin(v) => rsx! {
            span { class: "dc-ol-block {margins}",
                span { class: "dc-ol-cost",
                    span { class: "dc-i dc-coin", "{n(v)}" }
                }
            }
        },
        Piece::Debt(v) => rsx! {
            span { class: "dc-ol-block {margins}",
                span { class: "dc-ol-cost",
                    span { class: "dc-i dc-debt", "{n(v)}" }
                }
            }
        },
        Piece::Potion => rsx! {
            span { class: "dc-ol-block {margins}",
                span { class: "dc-ol-cost",
                    span { class: "dc-i dc-potion" }
                }
            }
        },
        Piece::Vp(v) => rsx! {
            span { class: "dc-ol-vp",
                span { class: "dc-ol-vp-n",
                    if let Some(v) = v {
                        "{v}"
                    }
                }
                span { class: "dc-i dc-shield" }
            }
        },
        Piece::Sun => rsx! { span { class: "dc-i dc-sun" } },
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
                if let Some(v) = v {
                    span { class: "dc-vp-n", "{v}" }
                }
                span { class: "dc-i dc-shield" }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Price, price, set_slug};
    use dominion_protocol::{Cost, api::CardCost};

    #[test]
    fn set_names_become_icon_slugs() {
        assert_eq!(set_slug("Dark Ages"), "dark-ages");
        assert_eq!(set_slug("Cornucopia & Guilds"), "cornucopia-guilds");
        assert_eq!(set_slug("Base"), "base");
    }

    #[test]
    fn only_a_card_with_a_printed_cost_shows_one() {
        let zero = Cost {
            coins: 0,
            potions: 0,
            debt: 0,
        };
        assert_eq!(price(Some(zero.clone()), None), None);
        assert_eq!(price(None, None), None);
        let printed = CardCost {
            coins: 5,
            potions: 0,
            debt: 0,
        };
        let shown = |coins| Price {
            coins,
            potions: 0,
            debt: 0,
        };
        assert_eq!(price(None, Some(printed)), Some(shown(5)));
        assert_eq!(price(Some(zero), Some(printed)), Some(shown(0)));
    }
}
