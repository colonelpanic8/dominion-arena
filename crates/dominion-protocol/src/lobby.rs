//! Accounts, the lobby (tables, automatch, chat) and game history: the bodies of the
//! routes under `/api` that need to know who is asking. All camelCase.

use crate::api::{DealSlot, SeatToken};
use serde::{Deserialize, Serialize};

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum UserKind {
    /// Signed in with a password.
    #[default]
    Registered,
    /// No password: the browser (or agent) that made it holds its only session.
    Guest,
    /// An agent's account, signed in with an API key its owner made.
    Bot,
}

/// A player as everyone sees them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct UserRef {
    pub name: String,
    pub kind: UserKind,
    /// Their rating, once they have played a rated game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<i32>,
    /// A bot's owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// A Glicko-2 rating on the Glicko scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingInfo {
    pub rating: f64,
    /// The rating deviation: 350 for a new player, less the more they play.
    pub deviation: f64,
    /// Rated games played.
    pub games: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Finished games against other players, rated or not.
    pub games: u32,
    pub wins: u32,
}

/// `GET /api/me`: who a session is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub user: UserRef,
    pub rating: RatingInfo,
    pub stats: Stats,
    /// Their bots.
    #[serde(default)]
    pub bots: Vec<UserRef>,
}

/// `POST /api/auth/register` and `/api/auth/login`. Registering from a guest session
/// keeps the guest's games; `name` may then be left out to keep its name too.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credentials {
    #[serde(default)]
    pub name: String,
    pub password: String,
}

/// `POST /api/auth/guest`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestRequest {
    /// Default: Guest and a number.
    #[serde(default)]
    pub name: Option<String>,
}

/// `PUT /api/me/password`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewPassword {
    pub current: String,
    pub new: String,
}

/// `POST /api/me/bots`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewBot {
    pub name: String,
}

/// A bot with its API key, shown once: when the bot is made or its key replaced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotKey {
    pub bot: UserRef,
    pub key: String,
}

/// What a lobby table deals and how its game runs: dominion.games' Kingdom Options and
/// Game Options. The server picks the seed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct TableSpec {
    /// Sets the random piles and landscapes come from (empty: every released set).
    pub sets: Vec<String>,
    /// Kingdom piles the kingdom must have.
    pub kingdom: Vec<String>,
    /// One random Kingdom pile from each of these sets.
    pub from_sets: Vec<String>,
    /// Landscapes the game must have, besides the slots.
    pub landscapes: Vec<String>,
    /// Slots for random landscapes (`None`: the rulebook's two).
    pub slots: Option<Vec<DealSlot>>,
    /// Cards on a seated player's Banned list are never dealt at random.
    pub respect_banned: bool,
    pub colonies: Option<bool>,
    pub shelters: Option<bool>,
    pub turn_limit: Option<u32>,
    /// Seats play in a random order rather than as listed.
    pub randomize_order: bool,
    /// The game counts for ratings when every seat is a different registered player or
    /// bot (and no bot plays its owner).
    pub rated: bool,
    /// Signed-in people may watch the game (public information only).
    pub watchable: bool,
    /// A player who must decide and has left, or takes too long, forfeits.
    pub timed: bool,
    /// Players may take back their own answers, which shows them what would have
    /// happened: for a table of friends.
    pub undo: bool,
}

impl Default for TableSpec {
    fn default() -> TableSpec {
        TableSpec {
            sets: vec![],
            kingdom: vec![],
            from_sets: vec![],
            landscapes: vec![],
            slots: None,
            respect_banned: true,
            colonies: None,
            shelters: None,
            turn_limit: None,
            randomize_order: true,
            rated: true,
            watchable: true,
            timed: true,
            undo: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeatKind {
    /// Empty: anyone signed in may sit down.
    #[default]
    Open,
    /// A signed-in player.
    Player,
    /// Played with a seat token the host hands out when the game starts (a second player
    /// at the host's screen, an agent without an account).
    Token,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSeat {
    pub kind: SeatKind,
    /// A `player` seat's player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<UserRef>,
    /// A `token` seat's name in the game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Token seats always are.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ready: bool,
    /// A `player` seat's player is connected.
    #[serde(default, skip_serializing_if = "is_false")]
    pub here: bool,
}

/// A table in the lobby.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub id: String,
    /// The host's name: the one who sets the rules and the seats.
    pub host: String,
    pub spec: TableSpec,
    pub seats: Vec<TableSeat>,
    /// The game it started, while that runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    /// The game it started last, once that is over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_game: Option<String>,
}

/// `POST /api/tables`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewTable {
    #[serde(default)]
    pub spec: TableSpec,
    /// The seats besides the host's (default: one open seat).
    #[serde(default)]
    pub seats: Option<Vec<SeatEdit>>,
}

/// One seat of `PUT /api/tables/{id}/seats`, which lists every seat in order: a seated
/// player to keep (a seated player left out is removed from the table), an open seat or
/// a token seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum SeatEdit {
    Player(String),
    Open,
    Token(String),
}

/// `POST /api/tables/{id}/join`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinTable {
    /// The seat to take (default: the first open one).
    #[serde(default)]
    pub seat: Option<u8>,
}

/// `POST /api/tables/{id}/ready`. The game starts once no seat is open and every player
/// is ready.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ready {
    pub ready: bool,
}

/// Who an automatch may pair you with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Opponents {
    #[default]
    Any,
    /// Only people (players not connected as agents).
    Humans,
    /// Only agents: bots, and anyone searching over MCP.
    Agents,
}

/// `POST /api/automatch`: search for a game, replacing any search in progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct MatchRequest {
    /// The player counts you accept, each 2 to 6 (default: 2).
    pub players: Vec<u8>,
    pub opponents: Opponents,
    /// Accept a rated game. One is rated when everyone in it accepts and is a registered
    /// player or bot.
    pub rated: bool,
}

impl Default for MatchRequest {
    fn default() -> MatchRequest {
        MatchRequest {
            players: vec![2],
            opponents: Opponents::Any,
            rated: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueState {
    Searching,
    /// A game was made: `game_id` names it, and `Lobby::games` has your seat. Shown until
    /// you search again or cancel, or for a couple of minutes.
    Matched,
}

/// Your automatch search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatus {
    pub state: QueueState,
    /// When the search began, in milliseconds since the Unix epoch.
    pub since: u64,
    pub request: MatchRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
}

/// A seat you hold through your account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyGame {
    pub game_id: String,
    pub seat: u8,
    pub name: String,
    /// The seat's token, for the game routes.
    pub token: String,
    pub running: bool,
    /// The other seats' names.
    pub opponents: Vec<String>,
}

/// How many are searching for an automatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Searching {
    pub humans: u32,
    pub agents: u32,
}

/// The table you sit at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyTable {
    pub id: String,
    /// For the host: the tokens of the token seats of the game it started last, to hand
    /// out.
    #[serde(default)]
    pub tokens: Vec<SeatToken>,
}

/// `GET /api/lobby`: the lobby as the asker sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lobby {
    /// Changes whenever anything below may have.
    pub version: u64,
    pub you: Option<Me>,
    /// Players connected now.
    pub online: u32,
    pub searching: Searching,
    pub tables: Vec<TableInfo>,
    pub table: Option<MyTable>,
    pub queue: Option<QueueStatus>,
    /// Your seats in running games, and in games that ended within the hour.
    pub games: Vec<MyGame>,
}

/// The data of the lobby feed's `change` events (`GET /api/lobby/events`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyChange {
    /// [`Lobby::version`].
    pub version: u64,
    /// Changes whenever a message is said in the lobby or at a table.
    pub chat: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatKind {
    #[default]
    Player,
    /// Said by someone watching a game.
    Watcher,
    /// Said by the server ("Ada sat down").
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// Counts up within a channel.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    /// The speaker's name; empty for the server.
    pub from: String,
    #[serde(default)]
    pub kind: ChatKind,
    pub text: String,
}

/// `GET /api/chat/{channel}[?after=seq]`. Channels: `lobby`, `table-<id>`, `game-<id>`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLog {
    pub messages: Vec<ChatMessage>,
}

/// `POST /api/chat/{channel}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Say {
    pub text: String,
}

/// Where a game came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// `POST /api/games` or `create_game`: every seat is played by its token.
    #[default]
    Direct,
    Table,
    Automatch,
}

/// What a game's page shows beside the seat's view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMeta {
    pub source: Source,
    pub rated: bool,
    pub watchable: bool,
    pub timed: bool,
    /// Per seat: the account playing it, if one is.
    pub users: Vec<Option<UserRef>>,
    /// Per seat: someone is connected for it now, or was within the last moments.
    pub connected: Vec<bool>,
    /// In a timed game: when the deciding seat forfeits unless it answers, in
    /// milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forfeit_at: Option<u64>,
    /// People watching now.
    pub watchers: u32,
    /// Why the game ended early, when it did: `resigned` or `abandoned`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
    /// The server's clock when this was read, in milliseconds since the Unix epoch: what
    /// `forfeit_at` is to be compared with.
    #[serde(default)]
    pub now: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordPlayer {
    pub seat: u8,
    pub name: String,
    /// Absent for a seat played by token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<UserKind>,
    pub vp: i32,
    pub rank: u32,
    pub turns: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub resigned: bool,
    /// In a rated game: their rating before and after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<(f64, f64)>,
}

/// A finished game.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRecord {
    pub game_id: String,
    /// Milliseconds since the Unix epoch.
    pub started: u64,
    pub ended: u64,
    pub source: Source,
    pub rated: bool,
    /// The view's end reason (`provinces-gone`, ...), or `resigned` / `abandoned`.
    pub reason: String,
    pub kingdom: Vec<String>,
    pub landscapes: Vec<String>,
    pub players: Vec<RecordPlayer>,
}

/// `GET /api/users/{name}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub user: UserRef,
    pub rating: RatingInfo,
    pub stats: Stats,
    /// Their latest games, newest first.
    pub recent: Vec<GameRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Leader {
    pub rank: u32,
    pub user: UserRef,
    pub rating: RatingInfo,
    pub stats: Stats,
}

/// `GET /api/leaderboard`: rated players, best first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Leaderboard {
    pub players: Vec<Leader>,
}

/// `GET /api/history[?user=name][&before=ended][&limit=n]`: finished games, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub games: Vec<GameRecord>,
}
