//! Static card data: every card, landscape and other card-shaped object, with its official
//! text, cost and types. Generated at build time from `data/cards/*.json`.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! cards {
    ($($(#[$doc:meta])* $name:ident,)*) => {
        /// Every card-shaped object: Kingdom and basic cards, non-Supply cards, landscapes
        /// (Events, Landmarks, Projects, Ways, Allies, Traits, Prophecies), Boons, Hexes,
        /// States, Artifacts and pile randomizers. See [`Card::kind`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum Card {
            $($(#[$doc])* $name,)*
        }

        impl Card {
            pub const ALL: &[Card] = &[$(Card::$name,)*];
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/cards_data.rs"));

/// What sort of object a [`Card`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// A physical card that can be in a deck.
    Card,
    Event,
    Landmark,
    Project,
    Way,
    Ally,
    Trait,
    Prophecy,
    /// Arcana's Studies.
    Study,
    Boon,
    Hex,
    State,
    Artifact,
    /// A randomizer naming a mixed pile (Knights, Castles, Augurs, ...).
    Randomizer,
}

impl Kind {
    /// Landscapes sit beside the Supply rather than in it.
    pub fn is_landscape(self) -> bool {
        matches!(
            self,
            Kind::Event
                | Kind::Landmark
                | Kind::Project
                | Kind::Way
                | Kind::Ally
                | Kind::Trait
                | Kind::Prophecy
                | Kind::Study
        )
    }
}

/// A cost in coins ($), Potions and Debt. Every component is compared separately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cost {
    pub coins: u8,
    pub potions: u8,
    pub debt: u8,
}

impl Cost {
    pub const ZERO: Cost = Cost::coins(0);

    pub const fn coins(coins: u8) -> Cost {
        Cost {
            coins,
            potions: 0,
            debt: 0,
        }
    }

    /// "Costing up to `limit`": no component exceeds the limit's.
    pub fn at_most(self, limit: Cost) -> bool {
        self.coins <= limit.coins && self.potions <= limit.potions && self.debt <= limit.debt
    }

    /// "Costing less than `other`": at most it, and not equal.
    pub fn less_than(self, other: Cost) -> bool {
        self.at_most(other) && self != other
    }

    pub fn plus_coins(self, n: u8) -> Cost {
        Cost {
            coins: self.coins.saturating_add(n),
            ..self
        }
    }

    pub fn minus_coins(self, n: u8) -> Cost {
        Cost {
            coins: self.coins.saturating_sub(n),
            ..self
        }
    }
}

impl fmt::Display for Cost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.coins > 0 || (self.potions == 0 && self.debt == 0) {
            write!(f, "${}", self.coins)?;
        }
        for _ in 0..self.potions {
            f.write_str("P")?;
        }
        if self.debt > 0 {
            write!(f, "{}D", self.debt)?;
        }
        Ok(())
    }
}

/// Unconditional bonuses printed at the top of a card, applied before its script runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plus {
    pub cards: u8,
    pub actions: u8,
    pub buys: u8,
    pub coins: u8,
    pub villagers: u8,
    pub coffers: u8,
    pub favors: u8,
}

impl Plus {
    pub fn is_empty(&self) -> bool {
        *self == Plus::default()
    }
}

/// Static facts about a card, as printed.
#[derive(Clone, Copy, Debug)]
pub struct CardDef {
    pub name: &'static str,
    pub set: Set,
    /// `None` for things with no cost (most landscapes, Boons, ...).
    pub cost: Option<Cost>,
    /// The printed cost has a `*`: it changes by its own rules (Peddler, Fisherman, ...).
    pub variable_cost: bool,
    /// The printed cost has a `+`: it can be overpaid for (Guilds).
    pub overpay: bool,
    pub types: Types,
    /// Vanilla bonuses on the card's leading lines.
    pub plus: Plus,
    /// $ produced when played as a Treasure, for Treasures with a printed value.
    pub coins: u8,
    /// Potions produced when played as a Treasure.
    pub potions: u8,
    /// Printed VP. Variable VP (e.g. Gardens) is computed in scoring.
    pub vp: i8,
    /// Can be one of a game's Kingdom piles (as a pile's randomizer or top card).
    pub kingdom: bool,
    /// One of the basic Supply cards (Copper, Curse, Potion, Colony, ...).
    pub basic: bool,
    /// Removed in a later edition; still playable.
    pub removed: bool,
    /// The randomizer or top card of the mixed or split pile this card belongs to.
    pub pile: Option<Card>,
    /// Official text. Lines are separated by `\n`; a line `---` is the dividing line.
    pub text: &'static str,
}

impl Card {
    pub fn def(self) -> &'static CardDef {
        &DEFS[self as usize]
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn text(self) -> &'static str {
        self.def().text
    }

    pub fn set(self) -> Set {
        self.def().set
    }

    pub fn types(self) -> Types {
        self.def().types
    }

    /// Whether the card has any of `types`. For a physical card in a game, use
    /// [`crate::Game::is`], which applies effects that change types.
    pub fn is(self, types: Types) -> bool {
        self.def().types.intersects(types)
    }

    /// Printed cost. For a card in a game, use [`crate::Game::cost_of`].
    pub fn cost(self) -> Option<Cost> {
        self.def().cost
    }

    pub fn kind(self) -> Kind {
        let t = self.types();
        [
            (Types::EVENT, Kind::Event),
            (Types::LANDMARK, Kind::Landmark),
            (Types::PROJECT, Kind::Project),
            (Types::WAY, Kind::Way),
            (Types::ALLY, Kind::Ally),
            (Types::TRAIT, Kind::Trait),
            (Types::PROPHECY, Kind::Prophecy),
            (Types::STUDY, Kind::Study),
            (Types::BOON, Kind::Boon),
            (Types::HEX, Kind::Hex),
            (Types::STATE, Kind::State),
            (Types::ARTIFACT, Kind::Artifact),
            (Types::RANDOMIZER, Kind::Randomizer),
        ]
        .into_iter()
        .find(|(ty, _)| t.contains(*ty))
        .map_or(Kind::Card, |(_, k)| k)
    }

    /// The names of the card's types, in canonical order.
    pub fn type_names(self) -> Vec<&'static str> {
        self.types().names().collect()
    }

    /// Cards that can be chosen as a Kingdom pile.
    pub fn kingdom_cards() -> impl Iterator<Item = Card> {
        Card::ALL.iter().copied().filter(|c| c.def().kingdom)
    }

    pub fn in_set(set: Set) -> impl Iterator<Item = Card> {
        Card::ALL.iter().copied().filter(move |c| c.set() == set)
    }

    /// The cards of a mixed or split pile named by `self`, in pile order (top first).
    pub fn pile_members(self) -> Vec<Card> {
        Card::ALL
            .iter()
            .copied()
            .filter(|c| c.def().pile == Some(self) || (*c == self && self.kind() == Kind::Card))
            .collect()
    }

    /// Looks a card up by name, ignoring case, spaces and punctuation.
    pub fn from_name(name: &str) -> Option<Card> {
        let key = normalize(name);
        Card::ALL
            .iter()
            .copied()
            .find(|c| normalize(c.name()) == key)
    }
}

impl Types {
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        TYPE_NAMES
            .iter()
            .filter(move |(t, _)| self.contains(*t))
            .map(|(_, name)| *name)
    }

    /// Looks a type up by its printed name, ignoring case.
    pub fn parse(name: &str) -> Option<Types> {
        TYPE_NAMES
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(t, _)| *t)
    }
}

impl fmt::Display for Types {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.names().collect();
        f.write_str(&names.join(" - "))
    }
}

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for Set {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
