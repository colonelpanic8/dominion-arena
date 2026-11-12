//! The lobby's kingdom builder, without the UI: the piles and landscapes a game can be
//! set up with, the tray of chosen ones, the client-side randomizer and the request it
//! all becomes.

use crate::cards::{CardDb, LANDSCAPE_KINDS};
use dominion_protocol::api::{CardInfo, NewGame};
use std::collections::BTreeSet;

/// Kingdom piles in a usual game, and slots in the tray.
pub const KINGDOM_SIZE: usize = 10;

/// Kinds a game has one of at most.
const ONLY_ONE: [&str; 2] = ["ally", "prophecy"];

/// The sets a kingdom can be dealt from, in release order (the order of `/api/cards`).
pub fn sets(db: &CardDb) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for c in db.all() {
        if c.kingdom && !c.removed && !out.contains(&c.set) {
            out.push(c.set.clone());
        }
    }
    out
}

/// A pile's cost for sorting and filtering: its card's, or for a randomizer the
/// cheapest card in the pile.
pub fn pile_cost(db: &CardDb, card: &CardInfo) -> Option<(u32, u32, u32)> {
    let key = |c: &CardInfo| c.cost.map(|c| (c.coins, c.potions, c.debt));
    key(card).or_else(|| {
        db.all()
            .iter()
            .filter(|c| c.pile.as_deref() == Some(card.name.as_str()))
            .filter_map(key)
            .min()
    })
}

fn set_rank(order: &[String], set: &str) -> usize {
    order.iter().position(|s| s == set).unwrap_or(order.len())
}

fn sorted<'a>(db: &'a CardDb, mut cards: Vec<&'a CardInfo>) -> Vec<&'a CardInfo> {
    let order = sets(db);
    cards.sort_by_cached_key(|c| {
        (
            set_rank(&order, &c.set),
            pile_cost(db, c),
            c.name.to_lowercase(),
        )
    });
    cards
}

/// Every Kingdom pile from `sets`, by set, cost and name; first-edition cards only with
/// `removed`.
pub fn kingdom_piles<'a>(
    db: &'a CardDb,
    sets: &BTreeSet<String>,
    removed: bool,
) -> Vec<&'a CardInfo> {
    let cards = db
        .all()
        .iter()
        .filter(|c| c.kingdom && sets.contains(&c.set) && (removed || !c.removed))
        .collect();
    sorted(db, cards)
}

/// Every landscape from `sets`, sorted like [`kingdom_piles`].
pub fn landscapes<'a>(db: &'a CardDb, sets: &BTreeSet<String>, removed: bool) -> Vec<&'a CardInfo> {
    let cards = db
        .all()
        .iter()
        .filter(|c| {
            LANDSCAPE_KINDS.contains(&c.kind.as_str())
                && sets.contains(&c.set)
                && (removed || !c.removed)
        })
        .collect();
    sorted(db, cards)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CostFilter {
    #[default]
    Any,
    /// Costs exactly this many $ (the last choice is "this many or more").
    Coins(u32),
    Potion,
    Debt,
}

/// The highest $ the cost filter lists on its own; it stands for "or more".
pub const MAX_COST_FILTER: u32 = 6;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub query: String,
    /// A type the card must have; empty for any.
    pub ty: String,
    pub cost: CostFilter,
}

impl Filter {
    pub fn matches(&self, db: &CardDb, card: &CardInfo) -> bool {
        let q = self.query.trim().to_lowercase();
        if !q.is_empty() && !card.name.to_lowercase().contains(&q) {
            return false;
        }
        if !self.ty.is_empty() {
            let in_pile = |ty: &str| {
                db.all()
                    .iter()
                    .filter(|c| c.pile.as_deref() == Some(card.name.as_str()))
                    .any(|c| c.types.iter().any(|t| t == ty))
            };
            if !card.types.contains(&self.ty) && !in_pile(&self.ty) {
                return false;
            }
        }
        let cost = pile_cost(db, card);
        match self.cost {
            CostFilter::Any => true,
            CostFilter::Coins(n) => cost.is_some_and(|(c, p, d)| {
                p == 0 && d == 0 && (c == n || (n == MAX_COST_FILTER && c > n))
            }),
            CostFilter::Potion => cost.is_some_and(|(_, p, _)| p > 0),
            CostFilter::Debt => cost.is_some_and(|(_, _, d)| d > 0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub name: String,
    /// Kept by the randomizer.
    pub locked: bool,
}

/// The chosen Kingdom piles and landscapes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tray {
    pub kingdom: Vec<Slot>,
    pub landscapes: Vec<String>,
}

impl Tray {
    pub fn has_pile(&self, name: &str) -> bool {
        self.kingdom.iter().any(|s| s.name == name)
    }

    /// Adds a pile, or removes it if it is already in the tray.
    pub fn toggle_pile(&mut self, name: &str) {
        match self.kingdom.iter().position(|s| s.name == name) {
            Some(i) => {
                self.kingdom.remove(i);
            }
            None => self.kingdom.push(Slot {
                name: name.to_string(),
                locked: false,
            }),
        }
    }

    pub fn remove_pile(&mut self, i: usize) {
        if i < self.kingdom.len() {
            self.kingdom.remove(i);
        }
    }

    pub fn toggle_lock(&mut self, i: usize) {
        if let Some(s) = self.kingdom.get_mut(i) {
            s.locked = !s.locked;
        }
    }

    /// Adds a landscape, or removes it if it is already in the tray. A game has one Ally
    /// and one Prophecy at most, so either replaces one of its kind.
    pub fn toggle_landscape(&mut self, db: &CardDb, name: &str) {
        if let Some(i) = self.landscapes.iter().position(|l| l == name) {
            self.landscapes.remove(i);
            return;
        }
        let kind = |n: &str| db.get(n).map(|c| c.kind.clone()).unwrap_or_default();
        let k = kind(name);
        if ONLY_ONE.contains(&k.as_str()) {
            self.landscapes.retain(|l| kind(l) != k);
        }
        self.landscapes.push(name.to_string());
    }

    /// Replaces every unlocked pile with a random one from `pool` that isn't in the
    /// tray, filling it to at least [`KINGDOM_SIZE`] piles while the pool lasts.
    /// Locked piles keep their places.
    pub fn randomize(&mut self, pool: &[String], rng: &mut Rng) {
        let target = self.kingdom.len().max(KINGDOM_SIZE);
        let locked: BTreeSet<&str> = self
            .kingdom
            .iter()
            .filter(|s| s.locked)
            .map(|s| s.name.as_str())
            .collect();
        let mut candidates: Vec<String> = pool
            .iter()
            .filter(|n| !locked.contains(n.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut next = Vec::with_capacity(target);
        for i in 0..target {
            match self.kingdom.get(i) {
                Some(s) if s.locked => next.push(s.clone()),
                _ if candidates.is_empty() => {}
                _ => {
                    let pick = candidates.swap_remove(rng.below(candidates.len()));
                    next.push(Slot {
                        name: pick,
                        locked: false,
                    });
                }
            }
        }
        self.kingdom = next;
    }

    pub fn clear_unlocked(&mut self) {
        self.kingdom.retain(|s| s.locked);
    }
}

/// What a kingdom of `n` chosen piles means, when it isn't the usual ten.
pub fn size_note(n: usize) -> Option<String> {
    match n {
        0 => None,
        KINGDOM_SIZE => None,
        n if n < KINGDOM_SIZE => Some(format!(
            "{n} of {KINGDOM_SIZE}: the game is set up with exactly these {n} piles, \
             unless you fill the rest (Randomize)."
        )),
        n => Some(format!(
            "{n} piles: more than the usual {KINGDOM_SIZE}. The game is set up with all of them."
        )),
    }
}

/// Who chooses the Kingdom piles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Deal {
    /// The server deals ten at random from the chosen sets.
    #[default]
    Server,
    /// The tray's piles.
    Tray,
}

/// The rest of the form.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub players: u8,
    /// One per seat; blank ones get the server's default name.
    pub names: Vec<String>,
    pub seed: Option<u64>,
    pub turn_limit: Option<u32>,
    pub colonies: Option<bool>,
    pub shelters: Option<bool>,
}

/// The game the form describes. A tray deal sends `kingdom` and never `sets`; a server
/// deal (or a tray deal with an empty tray) sends `sets` and never `kingdom`.
pub fn request(
    deal: Deal,
    tray: &Tray,
    sets: &[String],
    opts: &Options,
) -> Result<NewGame, String> {
    let kingdom: Vec<String> = match deal {
        Deal::Tray => tray.kingdom.iter().map(|s| s.name.clone()).collect(),
        Deal::Server => vec![],
    };
    if kingdom.is_empty() && sets.is_empty() {
        return Err("Choose at least one set to deal the kingdom from.".into());
    }
    let n = usize::from(opts.players);
    let names: Vec<String> = (0..n)
        .map(|i| {
            let given = opts.names.get(i).map(|s| s.trim()).unwrap_or_default();
            if given.is_empty() {
                format!("P{}", i + 1)
            } else {
                given.to_string()
            }
        })
        .collect();
    Ok(NewGame {
        players: opts.players,
        names,
        sets: if kingdom.is_empty() {
            sets.to_vec()
        } else {
            vec![]
        },
        kingdom,
        landscapes: tray.landscapes.clone(),
        seed: opts.seed,
        turn_limit: opts.turn_limit,
        colonies: opts.colonies,
        shelters: opts.shelters,
    })
}

/// A whole number field: empty is `None`.
pub fn parse_number<T: std::str::FromStr>(text: &str, what: &str) -> Result<Option<T>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse()
        .map(Some)
        .map_err(|_| format!("{what} must be a whole number"))
}

/// The randomizer's generator (SplitMix64): the deal is only a preview, so any seed does.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..n`; `n` must be positive.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// A file in the asset cache picturing `set`: its box art, or else its large banner.
pub fn set_art_paths(set: &str) -> [String; 3] {
    let flat: String = set
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    let mut dashed = String::new();
    for c in set.chars() {
        if c.is_alphanumeric() {
            dashed.extend(c.to_lowercase());
        } else if !dashed.is_empty() && !dashed.ends_with('-') {
            dashed.push('-');
        }
    }
    let dashed = dashed.trim_end_matches('-').to_string();
    [
        format!("images/box_art/{flat}.jpg"),
        format!("images/large/{flat}.webp"),
        format!("images/large/{dashed}.webp"),
    ]
}
