//! A headless rules engine for Dominion.
//!
//! The engine is a state machine: [`Game::pending`] returns the [`Decision`] it is
//! waiting on, and [`Game::respond`] answers it and runs until the next one. Decisions
//! with only one meaningful answer are resolved automatically. Card data (official text,
//! cost, types) is available through [`Card`] without a game.

// Helpers for card scripts are used by whichever sets need them.
#![allow(dead_code)]

mod agent;
mod card;
mod cards;
mod decision;
mod event;
mod frame;
mod game;
mod ids;
mod knowledge;
mod setup;
mod trace;
mod trigger;

#[cfg(test)]
mod tests;

pub use agent::{Agent, CardDeckView, Note, PileView, PlayError, PlayerView, View, play};
pub use card::{Card, CardDef, Cost, Kind, Plus, Set, TYPE_NAMES, Types};
pub use decision::{Decision, DecisionKind, Move, Opt, Purpose, Response, ResponseError};
pub use event::{Dest, Event};
pub use game::{
    CardDeck, EndReason, ExtraLimit, ExtraTurn, Game, Lasting, Mat, Outcome, Phase, Pile, Player,
    Scratch, Token, Turn, Until, Zone,
};
pub use ids::{CardId, PileId, PlayerId};
pub use knowledge::{CardCount, GroupView, Knowledge, KnownCards, UncertainCards, ZoneKnowledge};
pub use setup::{
    ANY_SLOT_KIND, DEFAULT_SLOTS, DealRules, GameConfig, KINGDOM_PILES, LandscapeSlot, SLOT_KINDS,
    SetupError,
};
pub use trace::Trace;
pub use trigger::{Occasion, Trigger};
