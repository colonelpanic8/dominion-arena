use std::{collections::BTreeMap, sync::Arc};

use dominion_engine::Card;
use dominion_online::catalog::Catalog;

use super::projection::ProjectionError;

/// Semantic lookups in the hash-checked 2.3.7 catalog.
pub(crate) struct WireCatalog {
    pub(crate) catalog: Arc<Catalog>,
    cards: BTreeMap<Card, i32>,
}

impl WireCatalog {
    pub(crate) fn new(catalog: Arc<Catalog>) -> Result<Self, ProjectionError> {
        let mut cards = BTreeMap::new();
        for &card in Card::ALL {
            if let Some(ordinal) = catalog.ordinal(card.name()) {
                cards.insert(card, ordinal);
            }
        }
        let result = Self { catalog, cards };
        for (table, symbols) in REQUIRED_SYMBOLS {
            for symbol in *symbols {
                result.symbol(table, symbol)?;
            }
        }
        for (name, expected) in ZONE_SYMBOLS {
            if result.symbol("ZoneNames", name)? != *expected {
                return Err(ProjectionError::MissingSymbol(format!(
                    "2.3.7 ZoneNames.{name}"
                )));
            }
        }
        for (table, name, expected) in [
            ("TimerPresetOptions", "OFF", TIMER_OFF),
            ("ResignationTypes", "MANUAL", MANUAL_RESIGNATION),
            ("ResignationTypes", "FORCE_RESIGNED", FORCED_RESIGNATION),
        ] {
            if result.symbol(table, name)? != expected {
                return Err(ProjectionError::MissingSymbol(format!(
                    "2.3.7 {table}.{name}"
                )));
            }
        }
        Ok(result)
    }

    pub(crate) fn card(&self, card: Card) -> Result<i32, ProjectionError> {
        self.cards
            .get(&card)
            .copied()
            .ok_or_else(|| ProjectionError::MissingSymbol(format!("CardNames.{}", card.name())))
    }

    pub(crate) fn symbol(&self, table: &str, symbol: &str) -> Result<i32, ProjectionError> {
        self.catalog
            .enums
            .get(table)
            .and_then(|values| values.iter().position(|s| s == symbol))
            .and_then(|i| i32::try_from(i).ok())
            .ok_or_else(|| ProjectionError::MissingSymbol(format!("{table}.{symbol}")))
    }
}

pub(super) const REQUIRED_SYMBOLS: &[(&str, &[&str])] = &[
    (
        "QuestionTypes",
        &["PLAY", "BUY", "DISCARD", "TRASH", "GAIN", "COMPLEX_OR"],
    ),
    (
        "QuestionIds",
        &[
            "GAME_ACTION_PHASE",
            "GAME_MAY_PLAY_ACTION",
            "GAME_BUY_PHASE",
            "GAME_MAY_PLAY_TREASURES",
            "GAME_MAY_BUY",
            "GAME_MAY_START_BUYING",
            "CELLAR",
            "CHAPEL",
            "WORKSHOP",
        ],
    ),
    (
        "DeclineButtonIds",
        &[
            "END_ACTION_PHASE",
            "END_BUY_PHASE",
            "DONT_DISCARD",
            "DONT_TRASH",
            "DONT_GAIN",
        ],
    ),
    ("CounterNames", &["ACTIONS", "COINS", "POTIONS", "BUYS"]),
    ("ExtraTurnTypes", &["REGULAR"]),
];

// ZoneNames is an array of constructors, unlike the semantic objects above.
// These positions are pinned to catalog::BODY_SHA256; the offline proof test
// checks them directly against that array, without executing the bundle.
pub(super) mod zone {
    pub const HAND: i32 = 0;
    pub const DRAW: i32 = 1;
    pub const DISCARD: i32 = 2;
    pub const TRASH: i32 = 3;
    pub const SUPPLY: i32 = 4;
    pub const TREASURE: i32 = 5;
    pub const VICTORY: i32 = 6;
    pub const PLAY: i32 = 8;
}

// TimerPresetOptions.OFF and ResignationTypes.MANUAL in the same pinned body.
pub(super) const TIMER_OFF: i32 = 4;
pub(super) const MANUAL_RESIGNATION: i32 = 0;
pub(super) const FORCED_RESIGNATION: i32 = 2;

pub(super) const ZONE_SYMBOLS: &[(&str, i32)] = &[
    ("HandZone", zone::HAND),
    ("DrawZone", zone::DRAW),
    ("DiscardZone", zone::DISCARD),
    ("TrashZone", zone::TRASH),
    ("SupplyCardZone", zone::SUPPLY),
    ("BasicTreasureZone", zone::TREASURE),
    ("BasicVictoryZone", zone::VICTORY),
    ("InPlayZone", zone::PLAY),
];

#[cfg(test)]
#[path = "catalog/tests.rs"]
pub(super) mod tests;
