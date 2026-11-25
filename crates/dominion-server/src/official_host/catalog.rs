use std::{collections::BTreeMap, sync::Arc};

use dominion_engine::Card;
use dominion_online::catalog::Catalog;
use dominion_online::transport::TimerSettings;
use dominion_protocol::lobby::Clock;

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
            ("ResignationTypes", "TIMED_OUT", TIMED_OUT_RESIGNATION),
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

    fn timer_parameter(&self, name: &str) -> Result<i32, ProjectionError> {
        self.symbol("TimeSettingsParameters", name)
    }

    /// dominion.games' timer for `clock`: its preset where it is one (Custom otherwise),
    /// the bank as time added at the start of the game and the increment as time added
    /// at the start of each turn. Its client shows and explains the clock from these.
    pub(crate) fn timer(&self, clock: Option<Clock>) -> Result<TimerSettings, ProjectionError> {
        let Some(clock) = clock else {
            return Ok(TimerSettings {
                parameters: vec![],
                force_resign: false,
                preset: TIMER_OFF,
            });
        };
        let preset = match clock.preset() {
            Some(name) => self.symbol("TimerPresetOptions", &name.to_uppercase())?,
            None => self.symbol("TimerPresetOptions", "CUSTOM")?,
        };
        let parameters = [
            ("START_OF_GAME_ADDED_TIME", i64::from(clock.initial) * 1000),
            (
                "START_OF_MY_TURN_ADDED_TIME",
                i64::from(clock.increment) * 1000,
            ),
            ("LOW_THRESHOLD", LOW_MS),
            ("VERY_LOW_THRESHOLD", VERY_LOW_MS),
        ]
        .into_iter()
        .map(|(name, ms)| Ok((self.timer_parameter(name)?, ms)))
        .collect::<Result<_, ProjectionError>>()?;
        Ok(TimerSettings {
            parameters,
            force_resign: true,
            preset,
        })
    }

    /// Every preset's timer, as the client is told at login.
    pub(crate) fn timer_presets(&self) -> Result<Vec<(i32, TimerSettings)>, ProjectionError> {
        Clock::PRESETS
            .iter()
            .map(|&(_, clock)| {
                let timer = self.timer(Some(clock))?;
                Ok((timer.preset, timer))
            })
            .collect()
    }

    /// The clock a table rule's timer asks for (`None`: no clock), or `Err` where Arena
    /// has none like it: a Custom timer may set only what [`WireCatalog::timer`] sets.
    pub(crate) fn clock(&self, timer: &TimerSettings) -> Result<Option<Clock>, String> {
        let unmapped = || "this dominion.games timer has no equivalent Arena clock".to_string();
        let missing = |e: ProjectionError| e.to_string();
        if timer.preset == TIMER_OFF {
            return if timer.parameters.iter().all(|(_, v)| *v == 0) && !timer.force_resign {
                Ok(None)
            } else {
                Err(unmapped())
            };
        }
        for (name, clock) in Clock::PRESETS {
            if timer.preset
                == self
                    .symbol("TimerPresetOptions", &name.to_uppercase())
                    .map_err(missing)?
            {
                return Ok(Some(clock));
            }
        }
        if timer.preset
            != self
                .symbol("TimerPresetOptions", "CUSTOM")
                .map_err(missing)?
        {
            return Err(unmapped());
        }
        let initial = self
            .timer_parameter("START_OF_GAME_ADDED_TIME")
            .map_err(missing)?;
        let increment = self
            .timer_parameter("START_OF_MY_TURN_ADDED_TIME")
            .map_err(missing)?;
        let shown = [
            self.timer_parameter("LOW_THRESHOLD").map_err(missing)?,
            self.timer_parameter("VERY_LOW_THRESHOLD")
                .map_err(missing)?,
        ];
        let seconds = |id: i32| -> Result<u32, String> {
            let ms = timer
                .parameters
                .iter()
                .find(|(p, _)| *p == id)
                .map_or(0, |(_, v)| *v);
            (ms % 1000 == 0)
                .then(|| u32::try_from(ms / 1000).ok())
                .flatten()
                .ok_or_else(unmapped)
        };
        if timer
            .parameters
            .iter()
            .any(|&(p, v)| v != 0 && p != initial && p != increment && !shown.contains(&p))
        {
            return Err(unmapped());
        }
        let clock = Clock::new(seconds(initial)?, seconds(increment)?);
        clock.check()?;
        Ok(Some(clock))
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
    (
        "TimerPresetOptions",
        &["RECKLESS", "HASTY", "FRIENDLY", "PATIENT", "CUSTOM"],
    ),
    (
        "TimeSettingsParameters",
        &[
            "START_OF_GAME_ADDED_TIME",
            "START_OF_MY_TURN_ADDED_TIME",
            "LOW_THRESHOLD",
            "VERY_LOW_THRESHOLD",
        ],
    ),
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

// PileMarkerTypes {BANE, OBELISK, SPLIT, MIXED, FERRYMAN} in the same pinned body.
pub(super) fn pile_marker(marker: dominion_protocol::PileMarker) -> i32 {
    use dominion_protocol::PileMarker::*;
    match marker {
        Bane => 0,
        Obelisk => 1,
        Split => 2,
        Mixed => 3,
    }
}

// TimerPresetOptions.OFF and ResignationTypes.MANUAL in the same pinned body.
pub(super) const TIMER_OFF: i32 = 4;
pub(super) const MANUAL_RESIGNATION: i32 = 0;
pub(super) const FORCED_RESIGNATION: i32 = 2;
pub(super) const TIMED_OUT_RESIGNATION: i32 = 3;
/// Where the client marks a player's time as low, and very low.
const LOW_MS: i64 = 60_000;
const VERY_LOW_MS: i64 = 10_000;

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
