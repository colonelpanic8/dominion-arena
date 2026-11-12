//! A table without the UI: the rules it holds before a game (dominion.games' Kingdom
//! Options and Players), the kingdom as text, the card selector's search, and the deal
//! and game requests they become.

use crate::cards::{CardDb, LANDSCAPE_KINDS};
use dominion_protocol::api::{CardInfo, DealRequest, DealSlot, Dealt, NewGame};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_PLAYERS: usize = 6;
pub const KINGDOM_SLOTS: usize = 10;
/// Landscape slots and required landscapes together.
pub const MAX_LANDSCAPES: usize = 4;

/// The expansions as dominion.games orders them, with the file name of their symbol.
/// "Deprecated" holds the cards removed in second editions (only in the card selector).
pub const EXPANSIONS: [(&str, &str); 17] = [
    ("Base", "base"),
    ("Intrigue", "intrigue"),
    ("Seaside", "seaside"),
    ("Alchemy", "alchemy"),
    ("Prosperity", "prosperity"),
    ("Cornucopia & Guilds", "cornucopia-guilds"),
    ("Hinterlands", "hinterlands"),
    ("Dark Ages", "dark-ages"),
    ("Adventures", "adventures"),
    ("Empires", "empires"),
    ("Nocturne", "nocturne"),
    ("Renaissance", "renaissance"),
    ("Menagerie", "menagerie"),
    ("Allies", "allies"),
    ("Plunder", "plunder"),
    ("Rising Sun", "rising-sun"),
    ("Promos", "promos"),
];
pub const DEPRECATED: &str = "Deprecated";

/// The white symbol of an expansion (or of [`DEPRECATED`]) in the asset cache.
pub fn expansion_icon(set: &str) -> String {
    let file = EXPANSIONS
        .iter()
        .find(|(s, _)| *s == set)
        .map(|(_, f)| f.to_string())
        .unwrap_or_else(|| {
            set.to_lowercase()
                .replace([' ', '&'], "-")
                .replace("--", "-")
        });
    format!("images/elements/{file}-small-white.png")
}

/// The expansions a kingdom can be dealt from, in dominion.games' order.
pub fn expansions(db: &CardDb) -> Vec<String> {
    let have: BTreeSet<&str> = db
        .all()
        .iter()
        .filter(|c| c.kingdom && !c.removed)
        .map(|c| c.set.as_str())
        .collect();
    let mut out: Vec<String> = EXPANSIONS
        .iter()
        .filter(|(s, _)| have.contains(s))
        .map(|(s, _)| s.to_string())
        .collect();
    for s in have {
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    out
}

/// Colonies and Shelters: chosen at setup, or forced either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ternary {
    #[default]
    Random,
    Yes,
    No,
}

impl Ternary {
    /// dominion.games' order: Random, Yes, No.
    pub fn cycle(self) -> Ternary {
        match self {
            Ternary::Random => Ternary::Yes,
            Ternary::Yes => Ternary::No,
            Ternary::No => Ternary::Random,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Ternary::Random => "Random",
            Ternary::Yes => "Yes",
            Ternary::No => "No",
        }
    }

    pub fn value(self) -> Option<bool> {
        match self {
            Ternary::Random => None,
            Ternary::Yes => Some(true),
            Ternary::No => Some(false),
        }
    }
}

/// What a landscape slot may be dealt, as in the server's deal.
pub const SLOT_KINDS: [&str; 5] = ["Event", "Landmark", "Project", "Way", "Trait"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandscapeSlot {
    /// Bit `i` set: it may be a `SLOT_KINDS[i]`.
    pub kinds: u8,
    /// It may stay empty.
    pub empty: bool,
}

impl LandscapeSlot {
    pub const ANY: LandscapeSlot = LandscapeSlot {
        kinds: (1 << SLOT_KINDS.len()) - 1,
        empty: true,
    };
    pub const NO_WAY: LandscapeSlot = LandscapeSlot {
        kinds: ((1 << SLOT_KINDS.len()) - 1) & !(1 << 3),
        empty: true,
    };

    pub fn takes(&self, i: usize) -> bool {
        self.kinds & (1 << i) != 0
    }

    /// What it may be, as dominion.games' tooltip says it: "Event, Way or empty".
    pub fn describe(&self) -> String {
        let mut parts: Vec<&str> = (0..SLOT_KINDS.len())
            .filter(|&i| self.takes(i))
            .map(|i| SLOT_KINDS[i])
            .collect();
        if self.empty {
            parts.push("empty");
        }
        match parts.split_last() {
            None => String::new(),
            Some((last, [])) => last.to_string(),
            Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        }
    }

    /// Toggles a kind (or, with `None`, whether it may stay empty). Returns `false` when
    /// nothing would be left, which removes the slot on dominion.games.
    pub fn toggle(&mut self, kind: Option<usize>) -> bool {
        match kind {
            Some(i) => self.kinds ^= 1 << i,
            None => self.empty = !self.empty,
        }
        self.kinds != 0
    }
}

/// Who plays a seat. dominion.games' bots are seats someone else plays here: an agent
/// over MCP or a browser with the seat link; an empty slot left at the start is one of
/// those too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeatKind {
    /// The host: this browser.
    You,
    /// Also played in this browser.
    HotSeat,
    /// For an MCP agent (joins with the token).
    Agent,
    /// An empty slot: whoever opens the seat link or joins with the token.
    Open,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatSpec {
    pub name: String,
    pub kind: SeatKind,
}

impl SeatSpec {
    pub fn plays_here(&self) -> bool {
        matches!(self.kind, SeatKind::You | SeatKind::HotSeat)
    }
}

/// A Kingdom slot that isn't left to chance, or is left to one set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pick {
    Card(String),
    /// A random Kingdom pile from this set.
    FromSet(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRules {
    pub seats: Vec<SeatSpec>,
    pub randomize_order: bool,
    pub kingdom: Vec<Pick>,
    pub landscapes: Vec<String>,
    pub slots: Vec<LandscapeSlot>,
    /// The expansions random cards come from.
    pub sets: BTreeSet<String>,
    pub colonies: Ternary,
    pub shelters: Ternary,
    /// The Banned cards list applies.
    pub respect_banned: bool,
    /// Blank: a random seed.
    pub seed: String,
    /// Blank: none.
    pub turn_limit: String,
}

impl TableRules {
    /// A new table: you and an empty slot, every expansion, the default landscape slots.
    pub fn new(you: &str, sets: &[String]) -> TableRules {
        TableRules {
            seats: vec![
                SeatSpec {
                    name: you.to_string(),
                    kind: SeatKind::You,
                },
                SeatSpec {
                    name: String::new(),
                    kind: SeatKind::Open,
                },
            ],
            randomize_order: true,
            kingdom: vec![],
            landscapes: vec![],
            slots: vec![LandscapeSlot::ANY, LandscapeSlot::NO_WAY],
            sets: sets.iter().cloned().collect(),
            colonies: Ternary::Random,
            shelters: Ternary::Random,
            respect_banned: true,
            seed: String::new(),
            turn_limit: String::new(),
        }
    }

    /// Back to ten random slots, the default landscape slots, Random and Random, and
    /// every expansion ("Clear Selection").
    pub fn clear_selection(&mut self, sets: &[String]) {
        self.kingdom.clear();
        self.landscapes.clear();
        self.slots = vec![LandscapeSlot::ANY, LandscapeSlot::NO_WAY];
        self.colonies = Ternary::Random;
        self.shelters = Ternary::Random;
        self.sets = sets.iter().cloned().collect();
    }

    pub fn required_piles(&self) -> Vec<&str> {
        self.kingdom
            .iter()
            .filter_map(|p| match p {
                Pick::Card(c) => Some(c.as_str()),
                Pick::FromSet(_) => None,
            })
            .collect()
    }

    pub fn landscape_count(&self) -> usize {
        self.landscapes.len() + self.slots.len()
    }

    pub fn has(&self, name: &str) -> bool {
        self.landscapes.iter().any(|l| l == name) || self.required_piles().contains(&name)
    }

    /// Whether a card can be added: a Kingdom pile while a slot is free, a landscape
    /// while fewer than [`MAX_LANDSCAPES`] are set, and nothing already in.
    pub fn can_add(&self, db: &CardDb, name: &str) -> bool {
        let Some(c) = db.get(name) else {
            return false;
        };
        if self.has(&c.name) {
            return false;
        }
        if is_landscape(c) {
            let replaces = ONLY_ONE.contains(&c.kind.as_str())
                && self.landscapes.iter().any(|l| kind(db, l) == c.kind);
            replaces || self.landscape_count() < MAX_LANDSCAPES
        } else {
            c.kingdom && self.kingdom.len() < KINGDOM_SLOTS
        }
    }

    /// Adds a Kingdom pile or landscape if it can be added. A game has one Ally and one
    /// Prophecy, so either replaces one of its kind.
    pub fn add(&mut self, db: &CardDb, name: &str) -> bool {
        if !self.can_add(db, name) {
            return false;
        }
        let c = db.get(name).expect("can_add found it");
        if is_landscape(c) {
            if ONLY_ONE.contains(&c.kind.as_str()) {
                self.landscapes.retain(|l| kind(db, l) != c.kind);
            }
            self.landscapes.push(c.name.clone());
        } else {
            self.kingdom.push(Pick::Card(c.name.clone()));
        }
        true
    }

    pub fn add_from_set(&mut self, set: &str) -> bool {
        if self.kingdom.len() >= KINGDOM_SLOTS {
            return false;
        }
        self.kingdom.push(Pick::FromSet(set.to_string()));
        true
    }

    pub fn remove(&mut self, name: &str) {
        self.kingdom.retain(|p| p != &Pick::Card(name.to_string()));
        self.landscapes.retain(|l| l != name);
    }

    pub fn remove_pick(&mut self, i: usize) {
        if i < self.kingdom.len() {
            self.kingdom.remove(i);
        }
    }

    pub fn add_slot(&mut self) {
        if self.landscape_count() < MAX_LANDSCAPES {
            self.slots.push(LandscapeSlot::ANY);
        }
    }

    /// A click on one expansion: on, or off unless it is the last one.
    pub fn toggle_set(&mut self, set: &str) {
        if self.sets.contains(set) {
            if self.sets.len() > 1 {
                self.sets.remove(set);
            }
        } else {
            self.sets.insert(set.to_string());
        }
    }

    /// "All": none when any is on, else every one.
    pub fn toggle_all_sets(&mut self, all: &[String]) {
        if self.sets.is_empty() {
            self.sets = all.iter().cloned().collect();
        } else {
            self.sets.clear();
        }
    }

    /// The kingdom as dominion.games writes it under the cards: the chosen cards
    /// (Kingdom piles by cost then name, then landscapes) in lower case, then Shelters and
    /// Colonies when they aren't Random.
    pub fn kingdom_code(&self, db: &CardDb) -> String {
        let mut cards: Vec<&CardInfo> = self
            .required_piles()
            .into_iter()
            .chain(self.landscapes.iter().map(String::as_str))
            .filter_map(|n| db.get(n))
            .collect();
        cards.sort_by(|a, b| card_order(db, a, b));
        let mut parts: Vec<String> = cards.iter().map(|c| c.name.to_lowercase()).collect();
        match self.shelters {
            Ternary::Yes => parts.push("Shelters".into()),
            Ternary::No => parts.push("No Shelters".into()),
            Ternary::Random => {}
        }
        match self.colonies {
            Ternary::Yes => parts.push("Colonies".into()),
            Ternary::No => parts.push("No Colonies".into()),
            Ternary::Random => {}
        }
        parts.join(", ")
    }

    /// Applies a line typed into the card selector: comma-separated card names (each
    /// the best match for what was typed), and Colonies, No Colonies, Shelters, No
    /// Shelters. Returns what matched nothing.
    pub fn apply_text(&mut self, db: &CardDb, text: &str) -> Vec<String> {
        let mut unknown = vec![];
        for part in split_list(text) {
            match part.to_lowercase().as_str() {
                "colonies" => self.colonies = Ternary::Yes,
                "no colonies" => self.colonies = Ternary::No,
                "shelters" => self.shelters = Ternary::Yes,
                "no shelters" => self.shelters = Ternary::No,
                _ => {
                    let name = part.split('(').next().unwrap_or_default().trim();
                    let all: Vec<&CardInfo> = db.all().iter().filter(|c| selectable(c)).collect();
                    let found = search(db, &all, name)
                        .into_iter()
                        .find(|c| c.name.eq_ignore_ascii_case(name))
                        .or_else(|| search(db, &all, name).into_iter().next());
                    match found {
                        Some(c) => {
                            let name = c.name.clone();
                            self.add(db, &name);
                        }
                        None => unknown.push(part),
                    }
                }
            }
        }
        unknown
    }

    /// What `POST /api/deal` is asked for: these rules, with `banned` when the table
    /// respects the list. The random slots are the Kingdom slots without a card.
    pub fn deal_request(&self, banned: &[String]) -> Result<DealRequest, String> {
        if self.sets.is_empty() && self.kingdom.len() < KINGDOM_SLOTS {
            return Err("Choose at least one expansion to deal random cards from.".into());
        }
        Ok(DealRequest {
            players: self.seats.len() as u8,
            sets: self.sets.iter().cloned().collect(),
            kingdom: self
                .required_piles()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            from_sets: self
                .kingdom
                .iter()
                .filter_map(|p| match p {
                    Pick::FromSet(s) => Some(s.clone()),
                    Pick::Card(_) => None,
                })
                .collect(),
            landscapes: self.landscapes.clone(),
            slots: Some(
                self.slots
                    .iter()
                    .map(|s| DealSlot {
                        kinds: (0..SLOT_KINDS.len())
                            .filter(|&i| s.takes(i))
                            .map(|i| SLOT_KINDS[i].to_lowercase())
                            .collect(),
                        optional: s.empty,
                    })
                    .collect(),
            ),
            banned: if self.respect_banned {
                banned.to_vec()
            } else {
                vec![]
            },
            seed: parse_number::<u64>(&self.seed, "The random seed")?,
        })
    }

    /// The seat names in seat order, blank ones named as the server would (P1, P2, ...);
    /// an error when two are the same.
    pub fn names(&self) -> Result<Vec<String>, String> {
        let names: Vec<String> = self
            .seats
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let n = s.name.trim();
                if n.is_empty() {
                    format!("P{}", i + 1)
                } else {
                    n.to_string()
                }
            })
            .collect();
        for (i, n) in names.iter().enumerate() {
            if names[..i]
                .iter()
                .any(|m| m.to_lowercase() == n.to_lowercase())
            {
                return Err(format!("Two players are called {n}."));
            }
        }
        Ok(names)
    }

    /// The game a deal starts: its kingdom, landscapes and seed, these seats in `order`
    /// (seat indices, first player first).
    pub fn new_game(&self, dealt: &Dealt, order: &[usize]) -> Result<NewGame, String> {
        let names = self.names()?;
        Ok(NewGame {
            players: self.seats.len() as u8,
            names: order.iter().map(|&i| names[i].clone()).collect(),
            kingdom: dealt.kingdom.clone(),
            landscapes: dealt.landscapes.clone(),
            sets: vec![],
            seed: Some(dealt.seed),
            turn_limit: parse_number::<u32>(&self.turn_limit, "The turn limit")?,
            colonies: self.colonies.value(),
            shelters: self.shelters.value(),
        })
    }

    /// The order seats play in: as listed, or shuffled with `seed` when the table
    /// randomizes it.
    pub fn seat_order(&self, seed: u64) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.seats.len()).collect();
        if self.randomize_order {
            let mut state = seed;
            for i in (1..order.len()).rev() {
                state = splitmix(state);
                order.swap(i, (state % (i as u64 + 1)) as usize);
            }
        }
        order
    }

    /// Replaces the random Kingdom slots and landscape slots with what was dealt for
    /// them, so the table shows the kingdom it will start with.
    pub fn take_deal(&mut self, dealt: &Dealt) {
        self.kingdom = dealt.kingdom.iter().cloned().map(Pick::Card).collect();
        self.landscapes = dealt.landscapes.clone();
        self.slots.clear();
    }
}

const ONLY_ONE: [&str; 2] = ["ally", "prophecy"];

fn kind(db: &CardDb, name: &str) -> String {
    db.get(name).map(|c| c.kind.clone()).unwrap_or_default()
}

pub fn is_landscape(c: &CardInfo) -> bool {
    LANDSCAPE_KINDS.contains(&c.kind.as_str())
}

/// What the card selector offers: Kingdom piles and landscapes.
pub fn selectable(c: &CardInfo) -> bool {
    c.kingdom || is_landscape(c)
}

/// The set a card is listed under in the card selector.
pub fn listed_set(c: &CardInfo) -> &str {
    if c.removed { DEPRECATED } else { &c.set }
}

fn cost_key(db: &CardDb, c: &CardInfo) -> (u32, u32, u32) {
    let key = |c: &CardInfo| c.cost.map(|c| (c.coins, c.potions, c.debt));
    key(c)
        .or_else(|| {
            db.all()
                .iter()
                .filter(|m| m.pile.as_deref() == Some(c.name.as_str()))
                .filter_map(key)
                .min()
        })
        .unwrap_or_default()
}

/// dominion.games' card order: Kingdom cards before landscapes, then by cost ($, then
/// Potions, then Debt), then by name.
pub fn card_order(db: &CardDb, a: &CardInfo, b: &CardInfo) -> std::cmp::Ordering {
    is_landscape(a)
        .cmp(&is_landscape(b))
        .then_with(|| cost_key(db, a).cmp(&cost_key(db, b)))
        .then_with(|| a.name.cmp(&b.name))
}

/// The cards of one listed set (or all of them), in dominion.games' order.
pub fn set_cards<'a>(db: &'a CardDb, set: Option<&str>) -> Vec<&'a CardInfo> {
    let mut cards: Vec<&CardInfo> = db
        .all()
        .iter()
        .filter(|c| selectable(c) && set.is_none_or(|s| listed_set(c) == s))
        .collect();
    cards.sort_by(|a, b| card_order(db, a, b));
    cards
}

/// How well a name matches what was typed, as the card selector ranks it (lower is
/// better): the name itself, its initials, a Way's last word, a prefix, then (from four
/// letters) any part.
pub fn match_rank(name: &str, query: &str, way: bool) -> Option<u8> {
    let name = name.to_lowercase();
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    if name == q {
        return Some(1);
    }
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .collect();
    if initials == q {
        return Some(2);
    }
    if way && let Some(last) = name.split_whitespace().last() {
        if last == q {
            return Some(3);
        }
        if last.starts_with(&q) {
            return Some(4);
        }
    }
    if name.starts_with(&q) {
        return Some(4);
    }
    if q.chars().count() > 3 && name.contains(&q) {
        return Some(5);
    }
    None
}

/// `cards` matching `query`, best first (then in card order).
pub fn search<'a>(db: &CardDb, cards: &[&'a CardInfo], query: &str) -> Vec<&'a CardInfo> {
    let mut found: Vec<(u8, &CardInfo)> = cards
        .iter()
        .filter_map(|c| match_rank(&c.name, query, c.kind == "way").map(|r| (r, *c)))
        .collect();
    found.sort_by(|(ra, a), (rb, b)| ra.cmp(rb).then_with(|| card_order(db, a, b)));
    found.into_iter().map(|(_, c)| c).collect()
}

/// Comma-separated parts, keeping commas inside parentheses.
fn split_list(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// A whole number field: empty is `None`.
pub fn parse_number<T: std::str::FromStr>(text: &str, what: &str) -> Result<Option<T>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse()
        .map(Some)
        .map_err(|_| format!("{what} must be a whole number."))
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
