//! Request and response bodies of `dominion serve`'s HTTP API under `/api`. Game
//! creation and listing share the MCP tools' snake_case types; the rest are camelCase.

use crate::view::AgentView;
use serde::{Deserialize, Serialize};

/// What a client asks for when creating a game.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NewGame {
    /// Number of players, 2 to 6.
    pub players: u8,
    /// Seat names, in turn order (default P1, P2, ...). Each is at most 32 characters on
    /// one line, and no two seats share a name.
    #[serde(default)]
    pub names: Vec<String>,
    /// Kingdom piles by name. Without it, a random kingdom is dealt from `sets`.
    #[serde(default)]
    pub kingdom: Vec<String>,
    /// Events, Landmarks, Projects, Ways, ... by name. With a random kingdom they replace
    /// the landscapes it would deal.
    #[serde(default)]
    pub landscapes: Vec<String>,
    /// Sets to deal a random kingdom from (default: every released set). Not with
    /// `kingdom`.
    #[serde(default)]
    pub sets: Vec<String>,
    pub seed: Option<u64>,
    /// End the game after this many turns in total.
    pub turn_limit: Option<u32>,
    pub colonies: Option<bool>,
    pub shelters: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SeatToken {
    pub seat: u8,
    pub name: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Created {
    pub game_id: String,
    pub seats: Vec<SeatToken>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GameSummary {
    pub game_id: String,
    pub names: Vec<String>,
    pub running: bool,
    /// The seat that must decide now.
    pub deciding: Option<u8>,
    pub kingdom: Vec<String>,
}

/// `GET /api/seat`: the seat a token holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatInfo {
    pub game_id: String,
    pub seat: u8,
    pub name: String,
}

/// A seat's view and the game's change counter, which only ever changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatView {
    pub view: AgentView,
    pub version: u64,
}

/// The data of an SSE `change` event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub version: u64,
}

/// Query of `GET /api/games/{id}/view`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewQuery {
    /// Log from this sequence number on, instead of from the seat's last answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_from: Option<usize>,
}

/// Every error's body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub error: String,
}

/// `GET /api/cards`: one card-shaped object as printed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardInfo {
    pub name: String,
    /// The set's display name.
    pub set: String,
    pub types: Vec<String>,
    /// `card`, `event`, `landmark`, `project`, `way`, `ally`, `trait`, `prophecy`,
    /// `study`, `boon`, `hex`, `state`, `artifact` or `randomizer`.
    pub kind: String,
    /// Absent for things with no cost (most landscapes, Boons, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CardCost>,
    /// The printed cost has a `*`.
    pub variable_cost: bool,
    /// The printed cost has a `+`.
    pub overpay: bool,
    /// $ a Treasure produces.
    pub coins: u32,
    pub potions: u32,
    pub vp: i32,
    pub kingdom: bool,
    pub basic: bool,
    pub removed: bool,
    /// The randomizer or top card of the mixed or split pile it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pile: Option<String>,
    /// Official text: lines separated by `\n`, `---` for the dividing line.
    pub text: String,
}

/// A printed cost, every component present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardCost {
    pub coins: u32,
    pub potions: u32,
    pub debt: u32,
}
