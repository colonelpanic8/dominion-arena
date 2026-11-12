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

/// `POST /api/deal`: a kingdom dealt by a table's rules without creating a game. Giving
/// `POST /api/games` the result (`kingdom`, `landscapes`, `seed`) sets up exactly that
/// kingdom; with only `sets` (or nothing) and the same seed, it is the kingdom
/// `POST /api/games` would deal from those sets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DealRequest {
    /// Number of players, 2 to 6.
    pub players: u8,
    /// Sets the random piles and landscapes come from (default: every released set).
    #[serde(default)]
    pub sets: Vec<String>,
    /// Kingdom piles the kingdom must have.
    #[serde(default)]
    pub kingdom: Vec<String>,
    /// One random Kingdom pile from each of these sets, in or out of `sets`.
    #[serde(default)]
    pub from_sets: Vec<String>,
    /// Landscapes the game must have, besides the slots.
    #[serde(default)]
    pub landscapes: Vec<String>,
    /// Slots for random landscapes (default: one of any kind, one of any kind but Way,
    /// both optional).
    #[serde(default)]
    pub slots: Option<Vec<DealSlot>>,
    /// Cards never dealt at random.
    #[serde(default)]
    pub banned: Vec<String>,
    pub seed: Option<u64>,
}

/// A place for one random landscape.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DealSlot {
    /// `event`, `landmark`, `project`, `way` and `trait`: what it may hold.
    pub kinds: Vec<String>,
    /// It may stay empty.
    pub optional: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dealt {
    pub kingdom: Vec<String>,
    pub landscapes: Vec<String>,
    pub seed: u64,
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
    /// Per seat: its token has been used (`join_game`, any call with it, or the HTTP
    /// API) since the server started.
    #[serde(default)]
    pub joined: Vec<bool>,
    pub kingdom: Vec<String>,
    /// Events, Landmarks, Projects, Ways, ... in the game.
    #[serde(default)]
    pub landscapes: Vec<String>,
}

/// `GET /api/seat`: the seat a token holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatInfo {
    pub game_id: String,
    pub seat: u8,
    pub name: String,
}

/// `GET /api/settings`: how the web client behaves, from the server's config file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebSettings {
    /// Cards are seen moving.
    pub web_animations: bool,
    /// How fast they move: dominion.games' Animation Type.
    pub web_animation_speed: AnimationSpeed,
    /// On other players' turns the kingdom drops and their play area goes above it.
    pub web_move_kingdom: bool,
    /// A single card to trash, discard, exile or put on the deck is confirmed with a
    /// button rather than sent on the click.
    pub web_confirm_single: bool,
    /// The painting under the table: a name in the asset cache's `images/large`
    /// (`inheritance`, `dark-wood`, ...), or empty for none.
    pub web_background: String,
    pub web_sounds: bool,
    /// 0.0 to 1.0.
    pub web_volume: f32,
    /// dominion.games' Banned cards list: a table that respects it never deals them.
    pub banned_cards: Vec<String>,
}

/// dominion.games' animation speeds, slowest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimationSpeed {
    Slow,
    #[default]
    Standard,
    Sprightly,
    Swift,
    Speedy,
    None,
}

impl Default for WebSettings {
    fn default() -> WebSettings {
        WebSettings {
            web_animations: true,
            web_animation_speed: AnimationSpeed::Standard,
            web_move_kingdom: true,
            web_confirm_single: false,
            web_background: "inheritance".into(),
            web_sounds: true,
            web_volume: 0.5,
            banned_cards: vec![],
        }
    }
}

/// A seat's view and the game's change counter, which only ever changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatView {
    pub view: AgentView,
    pub version: u64,
    /// The seat that must decide now, if any, as of the same version.
    #[serde(default)]
    pub deciding: Option<u8>,
    /// What the table shows that the view doesn't carry, as the seat may see it.
    #[serde(default)]
    pub table: TableExtras,
}

/// Public facts the graphical table draws beside the view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableExtras {
    /// Every player, by seat.
    pub players: Vec<PlayerTable>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTable {
    pub seat: u8,
    /// Their cards in play, in the order they came into play.
    pub in_play: Vec<String>,
    /// The face-up card on top of their discard pile.
    pub discard_top: Option<String>,
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
