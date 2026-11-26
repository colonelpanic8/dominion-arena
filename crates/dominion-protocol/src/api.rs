//! Request and response bodies of `dominion serve`'s HTTP API under `/api`. Game
//! creation and listing share the MCP tools' snake_case types; the rest are camelCase.

use crate::lobby::{GameMeta, SeatAgent, Source, UndoRule, UserRef};
use crate::view::{AgentView, Answer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// Each seat accepts a profile name (Classic), an agent and strategy, or null for a token seat.
    /// When supplied, one entry per player. Seating agents requires a registered
    /// account and uses its agent-seat allowance.
    #[serde(default)]
    pub agents: Vec<Option<crate::lobby::AgentSelection>>,
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
    /// Let anyone watch the game, signed in or not (default true); false makes it
    /// private.
    pub watchable: Option<bool>,
    /// A chess clock: a seat whose time runs out forfeits.
    pub clock: Option<crate::lobby::Clock>,
    /// Choices setup otherwise makes at random, by the card that makes them: Young
    /// Witch's Bane, the pile a Trait or Obelisk goes on, Way of the Mouse's card, the
    /// Black Market deck (top first), Druid's Boons, ...
    #[serde(default)]
    pub setup: std::collections::BTreeMap<String, Vec<String>>,
    /// Per seat: what its player is told before the game (a plan to play, notes from
    /// earlier games), or null. When supplied, one entry per player. A server agent gets
    /// its brief with its first view, a token seat when it joins; nobody else sees a
    /// brief before the game is over.
    #[serde(default)]
    pub briefs: Vec<Option<String>>,
    /// A question each server agent answers once the game is over (its debrief), before
    /// it leaves the game. Token seats may answer it with the `debrief` tool.
    pub debrief: Option<String>,
    /// Count the game for ratings. Only where every seat is a different one of the
    /// server's agents; such a game allows no undo.
    pub rated: Option<bool>,
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
    /// `direct` (create_game), `table` or `automatch`.
    #[serde(default)]
    pub source: Source,
    /// It counts for its players' ratings.
    #[serde(default)]
    pub rated: bool,
    /// Anyone may watch it.
    #[serde(default)]
    pub watchable: bool,
    /// Per seat: the account playing it, if one is.
    #[serde(default)]
    pub users: Vec<Option<UserRef>>,
    /// Per seat: the server's agent playing it, if one is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<Option<SeatAgent>>,
    /// Where a running game stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
}

/// Where a running game stands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Progress {
    /// The seat whose turn it is.
    pub player: u8,
    /// That seat's turns taken, this one included.
    pub turn: u32,
    /// `start`, `action`, `buy`, `night` or `cleanup`.
    pub phase: String,
    /// Per seat, in a game anyone may watch: what it would score if the game ended now
    /// and the cards it owns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seats: Vec<SeatProgress>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SeatProgress {
    pub vp: i32,
    /// Every card it owns, wherever it is, by name.
    pub owned: BTreeMap<String, u32>,
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
    /// Your own when you are signed in and have set one, else the server's.
    pub banned_cards: Vec<String>,
    /// New accounts can be registered.
    pub registration: bool,
    /// People can play without an account.
    pub guests: bool,
    /// New accounts can be registered with an admin's invite code, even without
    /// `registration`.
    pub invites: bool,
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
            registration: true,
            guests: true,
            invites: false,
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
    /// How many answers have been undone. When it changes, the log may differ from any
    /// held before at every sequence number.
    #[serde(default)]
    pub undos: u64,
    /// The seat may ask to take back its last answer (`POST /api/games/{id}/undo`).
    #[serde(default)]
    pub can_undo: bool,
    /// The game's undo rule and the undo waiting to be allowed, if any.
    #[serde(default)]
    pub undo: UndoState,
    /// Who plays, who is connected, and what kind of game it is.
    #[serde(default)]
    pub meta: GameMeta,
    /// For a finished game watched at an earlier point (`step`): where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<Replay>,
}

/// Where a replay of a finished game stands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Replay {
    /// How many of the game's answers the view comes after.
    pub step: usize,
    /// How many answers the game had: after all of them, it is as it ended.
    pub steps: usize,
    /// Each turn's start, in order.
    pub turns: Vec<ReplayTurn>,
    /// The only seat it can be seen as: a dominion.games game, recorded as that seat saw
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayTurn {
    /// The step at which the turn has started.
    pub step: usize,
    pub player: u8,
    pub turn: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoState {
    pub rule: UndoRule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<UndoRequest>,
    /// The seat that refused the viewer's last request, until the viewer acts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused_by: Option<u8>,
}

/// An undo a seat asked for, waiting for the others to allow it
/// (`POST /api/games/{id}/undo/allow`, `/refuse`; the asking seat may `/cancel`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoRequest {
    pub by: u8,
    /// The answers it takes back, every seat's, counting to the latest.
    pub answers: u32,
    /// The seats yet to allow it.
    pub waiting: Vec<u8>,
}

/// `POST /api/games/{id}/answer`: an answer and, with `way` (`none` or a Way's name),
/// the Way question of the card it plays answered in advance, for that card's replays too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerRequest {
    #[serde(flatten)]
    pub answer: Answer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub way: Option<String>,
}

/// What `POST /api/games/{id}/undo` asks for: by default, to take back the seat's last
/// answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoAsk {
    /// Go back to before the answer that led to this log entry instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_seq: Option<usize>,
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
    pub in_play: Vec<InPlay>,
    /// The face-up card on top of their discard pile.
    pub discard_top: Option<String>,
}

/// A card in play.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InPlay {
    pub card: String,
    /// A lasting effect keeps it in play past this turn's Clean-up.
    #[serde(default)]
    pub stays: bool,
    /// Cards are set aside with it, or tokens lie on it.
    #[serde(default)]
    pub holds: bool,
}

/// The data of an SSE `change` event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub version: u64,
    /// Changes whenever something is said in the game's chat (`game-<id>`).
    #[serde(default)]
    pub chat: u64,
}

/// Query of `GET /api/games/{id}/view` and `/watch`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewQuery {
    /// Log from this sequence number on, instead of from the seat's last answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_from: Option<usize>,
    /// For `GET /api/games/{id}/watch`: the game as this seat sees it, without its
    /// decision, instead of with every player's cards visible.
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_seat: Option<u8>,
    /// For `GET /api/games/{id}/watch` of a finished game: the game as it was after this
    /// many answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<usize>,
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
    /// The text dominion.games' client sets on the card, when the server has its
    /// bundle: its fixed lines separated by `//`, in its markup (`|bold|`, `%italic%`,
    /// `[2]` coins, `{1}` VP, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online_text: Option<String>,
    /// What dominion.games prints in the coins beside the full card's name, when the
    /// server has its bundle: a number, `P` for a Potion, or its text (`?`, `x2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online_production: Option<String>,
}

/// A printed cost, every component present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardCost {
    pub coins: u32,
    pub potions: u32,
    pub debt: u32,
}

/// An official account's connection, and its game as anyone may watch it
/// (`GET /api/online/view`, and each `change` of `/api/online/events`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineObservation {
    pub connected: bool,
    pub error: Option<String>,
    /// The account's game, in the shape `GET /api/games/{id}/watch` gives one of Arena's,
    /// with the whole log. `undos` counts the snapshots dominion.games has sent: a view
    /// under another count shares nothing with one before it. None without a game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<SeatView>,
    /// What has been said at the account's table on dominion.games, its last lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chat: Vec<crate::lobby::ChatMessage>,
}

/// An official account `dominion serve` keeps connected (`GET /api/online/accounts`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineAccountInfo {
    /// Its id in routes: `/api/online/<id>/...`, `/mcp/online/<id>`.
    pub id: String,
    /// Its official-server name.
    pub account: String,
    /// Also served at the unprefixed `/api/online/...` and `/mcp/online`.
    pub primary: bool,
    pub connected: bool,
    pub running: bool,
}

/// One of dominion.games' four ratings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OnlineRatingType {
    #[default]
    #[serde(rename = "2p")]
    TwoPlayer,
    #[serde(rename = "3p")]
    ThreePlayer,
    #[serde(rename = "2p-blitz")]
    TwoPlayerBlitz,
    #[serde(rename = "3p-blitz")]
    ThreePlayerBlitz,
}

impl OnlineRatingType {
    pub const ALL: [Self; 4] = [
        Self::TwoPlayer,
        Self::ThreePlayer,
        Self::TwoPlayerBlitz,
        Self::ThreePlayerBlitz,
    ];

    /// Its ordinal in the official client's `RatingTypes`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    pub fn from_ordinal(ordinal: i32) -> Option<Self> {
        Self::ALL.get(usize::try_from(ordinal).ok()?).copied()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TwoPlayer => "2 players",
            Self::ThreePlayer => "3 players",
            Self::TwoPlayerBlitz => "2 players, blitz",
            Self::ThreePlayerBlitz => "3 players, blitz",
        }
    }
}

/// A connected official account's rating and record on dominion.games, as its player page
/// shows them (`GET /api/online/<id>/rating?type=2p`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineRating {
    /// The official account's name.
    pub account: String,
    #[serde(rename = "type")]
    pub rating_type: OnlineRatingType,
    /// The latest rating: the open rating period's, which counts every game finished so
    /// far, when it has results (`provisional`), else the last period's.
    pub level: f64,
    /// Skill (mu).
    pub skill: f64,
    pub deviation: f64,
    /// Rated games counted in it.
    pub games: u32,
    /// From the open rating period, not yet final.
    pub provisional: bool,
    /// The open rating period, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<OnlineRatingPeriod>,
    /// On the leaderboard; none while unranked. Updated when a period closes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    pub wins: u32,
    pub losses: u32,
    pub ties: u32,
    pub best_level: f64,
    /// Milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_level_date: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_rank: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_rank_date: Option<i64>,
    pub best_win_streak: u32,
    pub best_loss_streak: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub country: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Rating history, oldest first, ending with the open period's when it is provisional.
    #[serde(default)]
    pub history: Vec<OnlineRatingPoint>,
    /// Recent games of this rating type, newest first.
    #[serde(default)]
    pub results: Vec<OnlineResult>,
    /// When the official server sent it, in milliseconds since the Unix epoch.
    pub fetched: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineRatingPoint {
    /// Milliseconds since the Unix epoch.
    pub date: i64,
    pub level: f64,
    pub skill: f64,
    pub deviation: f64,
    pub games: u32,
}

/// The rating period still open: its games so far and the rating they make.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineRatingPeriod {
    /// Milliseconds since the Unix epoch.
    pub start: i64,
    pub results: u32,
    pub distinct_opponents: u32,
    /// Points scored (1 a win, ½ a tie) and expected from the ratings.
    pub score: f64,
    pub expected_score: f64,
    pub start_level: f64,
    pub start_skill: f64,
    pub start_deviation: f64,
}

/// One finished game on an official account's player page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineResult {
    /// The official game id.
    pub game: i64,
    /// Milliseconds since the Unix epoch.
    pub time: i64,
    pub rated: bool,
    pub outcome: OnlineOutcome,
    /// Lower is better; ties share a rank.
    pub rank: u32,
    pub score: i32,
    pub resigned: bool,
    /// The levels the official server records for the account and its opponents (of a
    /// rated game).
    pub level: f64,
    pub opponent_level: f64,
    pub expected_score: f64,
    pub opponents: Vec<OnlineResultOpponent>,
    /// Kingdom card names, as the official client names them.
    pub kingdom: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnlineOutcome {
    Win,
    Loss,
    Tie,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineResultOpponent {
    pub name: String,
    pub rank: u32,
    pub score: i32,
    pub resigned: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineManagement {
    #[serde(flatten)]
    pub options: crate::lobby::LaunchOptions,
    /// The official account's id.
    #[serde(default)]
    pub id: String,
    pub connected: bool,
    pub error: Option<String>,
    pub account: Option<String>,
    pub agents: Vec<String>,
    /// The agents in `agents`, by model.
    #[serde(default)]
    pub models: Vec<crate::lobby::HouseModel>,
    pub agent: Option<String>,
    /// The agent that has played the running game: only it may resume it.
    #[serde(default)]
    pub game_agent: Option<String>,
    /// The running game's id: what resigning it names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    pub running: bool,
    pub searching: bool,
    pub queue_feedback: Vec<String>,
    pub mode: OnlineMode,
    /// The lowest dominion.games level the account's public searches match, set in the
    /// server's settings: a search's range is raised to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_opponent_level: Option<f64>,
    /// The game is against another official account this server hosts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paired: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnlineStart {
    #[serde(flatten)]
    pub options: crate::lobby::LaunchOptions,
    pub agent: String,
    #[serde(default)]
    pub mode: OnlineMode,
    #[serde(default)]
    pub matching: OnlineMatching,
    #[serde(default)]
    pub kingdom: Vec<String>,
    #[serde(default)]
    pub landscapes: Vec<String>,
    #[serde(default = "one_bot")]
    pub bots: u8,
    /// Another official account this server hosts to play against at a private unrated
    /// table, instead of a search (`mode` unrated, two players). `matching.timer` is the
    /// table's timer, where 4 is Off; `kingdom` and `landscapes` apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub against: Option<OnlineOpponent>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineOpponent {
    #[serde(flatten)]
    pub options: crate::lobby::LaunchOptions,
    /// The other hosted account's id.
    pub account: String,
    /// The agent that plays it.
    pub agent: String,
}
fn one_bot() -> u8 {
    1
}
/// Resigning an official account's running game: which game, and why. The reason is
/// logged with who resigned.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineResign {
    pub game: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAttach {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<crate::lobby::RunnerStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<crate::lobby::InferenceSpeed>,
    pub agent: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnlineMode {
    #[default]
    Bots,
    Unrated,
    Rated,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct OnlineMatching {
    pub players: u8,
    pub min_rating: i32,
    pub max_rating: i32,
    /// Upstream timer preset: 0 Reckless, 1 Hasty, 2 Friendly, 3 Patient.
    pub timer: u8,
}
impl Default for OnlineMatching {
    fn default() -> Self {
        Self {
            players: 2,
            min_rating: -10,
            max_rating: 10,
            timer: 3,
        }
    }
}

impl Default for OnlineStart {
    fn default() -> Self {
        Self {
            agent: String::new(),
            options: Default::default(),
            mode: OnlineMode::Bots,
            matching: OnlineMatching::default(),
            bots: 1,
            kingdom: vec![],
            landscapes: vec![],
            against: None,
        }
    }
}

/// A seat with a model at it, as the caller may see it: a model's own seat, or a
/// person's seat with a model attached. One of the two holds it: plays it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSeat {
    pub seat: u8,
    pub name: String,
    /// A person holds the seat; else its model does.
    pub controlled: bool,
    /// The seat's hand and decision, for the admin holding a model's own seat.
    pub playing: Option<ModelHand>,
    pub prompts: Vec<String>,
    /// The model attached to a person's seat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The seat is the caller's own, with a model attached.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own: bool,
    /// The caller holds the seat.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub holding: bool,
    /// When the model writes about the holder's decisions unasked.
    #[serde(default)]
    pub advise: AdviseMode,
    /// The holder asked for a note on their next decision, and it isn't written yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub requested: bool,
    /// What the model wrote, oldest first: for whoever holds the seat, as it says what
    /// the seat's player holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<ModelNote>,
}

/// When a model writes about the decisions of the person holding its seat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AdviseMode {
    /// Only when asked.
    #[default]
    Off,
    /// Once a turn, when a buy is offered.
    Buys,
    /// At every decision.
    Always,
}

/// What a model wrote for the person holding its seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelNote {
    /// The decision it is about, if it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// The turn it was written in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u32>,
    pub text: String,
    /// Milliseconds since the Unix epoch.
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelHand {
    pub hand: crate::ZoneView,
    pub decision: Option<crate::AgentDecision>,
    /// The game's version this was read at: `decision` is the seat's as of it.
    #[serde(default)]
    pub version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRequest {
    pub game: String,
    pub seat: u8,
    pub action: ModelAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelAction {
    /// Something for the model to consider: a question, when a person holds the seat.
    Prompt {
        text: String,
    },
    /// Hold the seat: play it yourself.
    Take,
    /// Hand the seat to its model.
    Release,
    Answer {
        answer: Answer,
    },
    /// Attach one of the server's agents to your own seat: you go on holding it.
    Attach {
        agent: String,
        #[serde(default, flatten)]
        options: crate::lobby::LaunchOptions,
    },
    /// Send away the model attached to your seat.
    Detach,
    Advise {
        mode: AdviseMode,
    },
    /// Ask for a note on the next decision of the seat.
    Request,
}
