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
mod chance;
mod decision;
mod event;
mod frame;
mod game;
mod ids;
mod journal;
mod knowledge;
mod scribe;
mod setup;
mod trace;
mod trigger;
mod words;

#[cfg(test)]
mod tests;

pub use agent::{
    Agent, CardDeckView, Note, PileView, PlayError, PlayerView, SetupRole, View, play,
};
pub use card::{Card, CardDef, CardRelations, Cost, Kind, Plus, Set, TYPE_NAMES, Types};
pub use chance::{Given, Outcomes, Refused, Site};
pub use decision::{Decision, DecisionKind, Move, Opt, Purpose, Response, ResponseError};
pub use event::{Counter, Dest, Event, Marker};
pub use game::{
    CardDeck, EndReason, ExtraLimit, ExtraTurn, Game, Lasting, Mat, Outcome, Phase, Pile,
    PileGroup, PileKind, PileStructure, Player, STALL_ROUNDS, Scratch, Token, Turn, Until, Zone,
};
pub use ids::{CardId, PileId, PlayerId};
pub use journal::{
    EffectEnd, EffectState, Gauge, PileProps, Spot, Step, TokenAt, TokenKind, TurnKind, TurnLimits,
    Verb, table_note,
};
pub use knowledge::{CardCount, GroupView, Knowledge, KnownCards, UncertainCards, ZoneKnowledge};
pub use setup::{
    ANY_SLOT_KIND, DEFAULT_SLOTS, DealRules, GameConfig, KINGDOM_PILES, LandscapeSlot, SLOT_KINDS,
    SetupError,
};
pub use trace::Trace;
pub use trigger::{Occasion, Trigger};
pub use words::amount;
