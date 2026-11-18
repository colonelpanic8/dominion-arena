//! The card counts: a row per card any player owns, a column per player with how many of
//! it they own, and for two players the difference between them. The game's Decks window
//! and the lobby's look at a running game both draw it.

use super::board::{CountsKey, sort_counts};
use super::table::seat_class;
use crate::card::{CardSize, CardView};
use dioxus::prelude::*;

/// The order of the rows: the Supply's, or by a column's numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecksSort {
    pub by: Option<CountsKey>,
    pub ascending: bool,
}

impl Default for DecksSort {
    /// The difference, largest first: what the first player has more of on top, what the
    /// second has more of at the bottom. With other than two players, the Supply's order.
    fn default() -> Self {
        Self {
            by: Some(CountsKey::Delta),
            ascending: false,
        }
    }
}

impl DecksSort {
    const SUPPLY: Self = Self {
        by: None,
        ascending: false,
    };

    /// Clicking a column: largest first, then smallest first, then the Supply's order.
    pub fn toggle(self, key: CountsKey) -> Self {
        match self.by {
            Some(k) if k == key && !self.ascending => Self {
                by: Some(key),
                ascending: true,
            },
            Some(k) if k == key => Self::SUPPLY,
            _ => Self {
                by: Some(key),
                ascending: false,
            },
        }
    }
}

/// A player's column.
#[derive(Clone, Debug, PartialEq)]
pub struct CountsColumn {
    pub seat: u8,
    pub name: String,
    /// How many cards they own.
    pub cards: u32,
    /// Under the name: their cards, and anything else worth a glance.
    pub sub: String,
    /// A third line: what plays the seat.
    pub note: Option<String>,
}

/// `rows` are in the Supply's order, each with a count per column; `unknown` is each
/// column's cards whose names aren't known. Without `on_sort` the headers are no buttons.
#[component]
pub fn CardCounts(
    columns: Vec<CountsColumn>,
    rows: Vec<(String, Vec<u32>)>,
    #[props(default)] unknown: Vec<u32>,
    sort: DecksSort,
    #[props(default)] on_sort: Option<EventHandler<CountsKey>>,
    /// Draws a row's card; a plain card strip when not given.
    #[props(default)]
    card: Option<Callback<String, Element>>,
) -> Element {
    let pair = columns.len() == 2;
    let mut rows = rows;
    let sorted_by = sort.by.filter(|k| match k {
        CountsKey::Player(i) => *i < columns.len(),
        CountsKey::Delta => pair,
    });
    if let Some(key) = sorted_by {
        sort_counts(&mut rows, key, sort.ascending);
    }
    let sorted = move |key| (sorted_by == Some(key)).then_some(sort.ascending);
    let grid = format!(
        "grid-template-columns: calc(16 * var(--vh)) repeat({}, minmax(0, 1fr));",
        columns.len() + usize::from(pair)
    );
    let cell = |n: u32, most: bool| {
        rsx! {
            div {
                class: "dt-decks-n",
                class: if n == 0 { "none" },
                class: if most { "most" },
                if n == 0 { "–" } else { "{n}" }
            }
        }
    };
    let most = |counts: &[u32]| {
        let max = counts.iter().copied().max().unwrap_or(0);
        let differ = counts.iter().any(|&n| n != max);
        move |n: u32| differ && n == max
    };
    let seats: Vec<u8> = columns.iter().map(|c| c.seat).collect();
    let delta = |a: u32, b: u32| {
        let d = i64::from(a) - i64::from(b);
        let leader = match d.signum() {
            1 => seat_class(seats[0]),
            -1 => seat_class(seats[1]),
            _ => String::new(),
        };
        rsx! {
            div {
                class: "dt-decks-n dt-decks-delta {leader}",
                class: if d == 0 { "none" },
                if d == 0 { "–" } else { "{d:+}" }
            }
        }
    };
    let difference = pair.then(|| {
        let d = i64::from(columns[0].cards) - i64::from(columns[1].cards);
        if d == 0 {
            "even".to_string()
        } else {
            format!("{d:+} cards")
        }
    });
    let draw = move |name: String| match card {
        Some(c) => c.call(name),
        None => rsx! { CardView { name, size: CardSize::Micro } },
    };
    rsx! {
        div { class: "dt-decks-grid", style: "{grid}",
            div {}
            for (i, c) in columns.iter().enumerate() {
                CountsHeader {
                    key: "h{c.seat}",
                    class: seat_class(c.seat),
                    name: c.name.clone(),
                    sub: c.sub.clone(),
                    note: c.note.clone(),
                    title: format!("Sort by {}'s counts", c.name),
                    sorted: sorted(CountsKey::Player(i)),
                    onclick: on_sort.map(|f| Callback::new(move |_| f.call(CountsKey::Player(i)))),
                }
            }
            if let Some(sub) = difference {
                CountsHeader {
                    class: "dt-decks-diff",
                    name: "Difference",
                    sub,
                    title: format!("Sort by {} − {}", columns[0].name, columns[1].name),
                    sorted: sorted(CountsKey::Delta),
                    onclick: on_sort.map(|f| Callback::new(move |_| f.call(CountsKey::Delta))),
                }
            }
            for (name, counts) in rows.iter() {
                div { key: "c{name}", class: "dt-decks-card", {draw(name.clone())} }
                for (i, &n) in counts.iter().enumerate() {
                    Fragment { key: "{name}{i}", {cell(n, most(counts)(n))} }
                }
                if pair {
                    Fragment { key: "{name}d", {delta(counts[0], counts[1])} }
                }
            }
            if unknown.iter().any(|&n| n > 0) {
                div { class: "dt-decks-card dt-decks-label", "Unknown" }
                for (i, &n) in unknown.iter().enumerate() {
                    Fragment { key: "u{i}", {cell(n, false)} }
                }
                if pair {
                    div {}
                }
            }
        }
    }
}

/// A column's header: sorts the rows by it when clicked, if they can be sorted.
#[component]
fn CountsHeader(
    class: String,
    name: String,
    sub: String,
    #[props(default)] note: Option<String>,
    title: String,
    /// Whether the rows are sorted by this column, and if so whether smallest first.
    sorted: Option<bool>,
    onclick: Option<EventHandler<()>>,
) -> Element {
    let arrow = match sorted {
        Some(true) => " ▲",
        Some(false) => " ▼",
        None => "",
    };
    let aria_sort = match sorted {
        Some(true) => "ascending",
        Some(false) => "descending",
        None => "none",
    };
    let body = rsx! {
        span { class: "dt-decks-name", "{name}{arrow}" }
        span { class: "dt-decks-total", "{sub}" }
        if let Some(note) = note {
            span { class: "dt-decks-note", "{note}" }
        }
    };
    match onclick {
        Some(f) => rsx! {
            button {
                class: "dt-decks-player {class}",
                class: if sorted.is_some() { "sorted" },
                title: "{title}",
                "aria-sort": aria_sort,
                onclick: move |_| f.call(()),
                {body}
            }
        },
        None => rsx! {
            div { class: "dt-decks-player static {class}", "aria-sort": aria_sort, {body} }
        },
    }
}
