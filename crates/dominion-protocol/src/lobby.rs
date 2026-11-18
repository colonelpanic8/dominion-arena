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
    /// The same rating on dominion.games' scale, once the server has anchors to read it
    /// off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official: Option<OfficialSkill>,
}

/// A rating as dominion.games skill (its mu, not its level), read off a line through
/// players rated on both servers (`official_skill_anchors`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialSkill {
    pub skill: f64,
    /// The rating deviation on that scale.
    pub deviation: f64,
    /// Below or above every anchor: `skill` is then the nearest end of the line, and the
    /// rating is only known to lie beyond it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beyond: Option<Beyond>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Beyond {
    Below,
    Above,
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
    /// Anyone may watch the game (public information only).
    pub watchable: bool,
    /// A player who must decide and has left, or takes too long, forfeits.
    pub timed: bool,
    /// A chess clock: a player whose time runs out forfeits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<Clock>,
    /// Whether players may take back answers.
    pub undo: UndoRule,
}

/// A chess clock: each player has a bank of time, which runs only while they must decide
/// and grows at the start of each of their turns. Whoever runs out forfeits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Clock {
    /// Each player's time at the start, in seconds.
    pub initial: u32,
    /// Added at the start of each of a player's turns, in seconds.
    #[serde(default)]
    pub increment: u32,
}

impl Clock {
    /// The most time a clock can start with or add: a day.
    pub const MAX_SECONDS: u32 = 24 * 60 * 60;

    /// dominion.games' timer names, slowest first.
    pub const PRESETS: [(&'static str, Clock); 4] = [
        ("Patient", Clock::new(30 * 60, 30)),
        ("Friendly", Clock::new(15 * 60, 15)),
        ("Hasty", Clock::new(7 * 60, 10)),
        ("Reckless", Clock::new(3 * 60, 5)),
    ];

    pub const fn new(initial: u32, increment: u32) -> Clock {
        Clock { initial, increment }
    }

    /// Its preset's name, if it is one.
    pub fn preset(self) -> Option<&'static str> {
        Clock::PRESETS
            .iter()
            .find(|(_, c)| *c == self)
            .map(|(name, _)| *name)
    }

    /// "15:00 + 15s", with its preset's name if it has one.
    pub fn label(self) -> String {
        let time = format!(
            "{}:{:02} + {}s",
            self.initial / 60,
            self.initial % 60,
            self.increment
        );
        match self.preset() {
            Some(name) => format!("{name} ({time})"),
            None => time,
        }
    }

    /// Why no game can have this clock, if none can.
    pub fn check(self) -> Result<(), String> {
        if self.initial == 0 {
            return Err("a clock starts with some time".into());
        }
        if self.initial > Clock::MAX_SECONDS || self.increment > Clock::MAX_SECONDS {
            return Err("a clock holds at most a day".into());
        }
        Ok(())
    }
}

/// Whether players may take back answers, which shows them what would have happened.
/// Games made directly (`POST /api/games`) let anyone undo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UndoRule {
    /// Answers can't be taken back.
    Off,
    /// Every other player must allow each undo (the server's agents always do).
    Ask,
    /// Anyone may undo without asking: for a table of friends.
    #[default]
    Free,
}

impl UndoRule {
    pub const ALL: [UndoRule; 3] = [UndoRule::Off, UndoRule::Ask, UndoRule::Free];

    pub fn label(self) -> &'static str {
        match self {
            UndoRule::Off => "No undo",
            UndoRule::Ask => "Undo with permission",
            UndoRule::Free => "Undo without asking",
        }
    }
}

impl<'de> Deserialize<'de> for UndoRule {
    /// Also `true` (`free`) and `false` (`off`), as the rule once was.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<UndoRule, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Given {
            Old(bool),
            Rule(String),
        }
        match Given::deserialize(d)? {
            Given::Old(true) => Ok(UndoRule::Free),
            Given::Old(false) => Ok(UndoRule::Off),
            Given::Rule(r) => match r.as_str() {
                "off" => Ok(UndoRule::Off),
                "ask" => Ok(UndoRule::Ask),
                "free" => Ok(UndoRule::Free),
                _ => Err(serde::de::Error::custom(format!(
                    "{r:?} is no undo rule: off, ask or free"
                ))),
            },
        }
    }
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
            clock: None,
            undo: UndoRule::Off,
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
    /// One of the server's own agents (`Lobby::house`), which the server starts when the
    /// game does.
    Agent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSeat {
    pub kind: SeatKind,
    /// A `player` seat's player, an `agent` seat's agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<UserRef>,
    /// A `token` seat's name in the game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Token and agent seats always are.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ready: bool,
    /// A `player` seat's player is connected; an `agent` seat's agent can be started now.
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
/// player to keep (a seated player left out is removed from the table), an open seat, a
/// token seat, or one of the server's agents by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum SeatEdit {
    Player(String),
    Open,
    Token(String),
    Agent(String),
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
    /// With `opponents: agents`, the server's agent to play (`Lobby::house`) when no other
    /// agent is searching; any of them when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

impl Default for MatchRequest {
    fn default() -> MatchRequest {
        MatchRequest {
            players: vec![2],
            opponents: Opponents::Any,
            rated: true,
            agent: None,
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
    #[serde(default)]
    pub house: House,
    /// The dominion.games accounts the server keeps connected, and their games.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub official: Vec<OfficialAccount>,
}

/// A dominion.games account the server keeps connected: one of the server's agents plays
/// as it, sent by its owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialAccount {
    /// Its id in routes: `/api/online/<id>/...`.
    pub id: String,
    /// Its name on dominion.games.
    pub account: String,
    /// The Arena account that sends agents to play as it.
    pub owner: String,
    pub connected: bool,
    /// Searching for a public match.
    pub searching: bool,
    /// The agent playing as it, or waiting for its search to find a game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Its game: the running one, else the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<OfficialGame>,
}

/// An official account's game.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialGame {
    /// dominion.games' id.
    pub id: String,
    pub running: bool,
    pub rated: bool,
    /// The players, by seat.
    pub players: Vec<String>,
    /// The account's seat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat: Option<u8>,
    pub kingdom: Vec<String>,
    #[serde(default)]
    pub landscapes: Vec<String>,
    /// What it was set up with besides those: Platinum and Colony, Shelters, Heirlooms, ...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<String>,
}

/// The server's own agents: it starts one on its machine for a table's `agent` seat, or
/// for a registered player's search of agents that no other agent answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct House {
    /// Empty while the server has nothing to start them with. Only agents with an
    /// account; [`House::models`] has every one the server can start.
    pub agents: Vec<HouseAgent>,
    /// Every agent the server can start, by model.
    #[serde(default)]
    pub models: Vec<HouseModel>,
    /// How many more can play at once now.
    pub free: u32,
    /// How many more agent seats you may fill today (`House::UNLIMITED` when the server
    /// sets no daily limit); none when you may not fill any (not signed in, a guest, a bot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<u32>,
}

impl House {
    pub const UNLIMITED: u32 = u32::MAX;

    /// "N more today", or that there is no daily limit.
    pub fn left_today(left: u32) -> String {
        if left == House::UNLIMITED {
            "no daily limit".into()
        } else {
            format!("{left} more today")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HouseAgent {
    pub user: UserRef,
    /// What plays it ("Claude Code, model opus").
    pub about: String,
    /// Games it is playing now.
    pub playing: u32,
}

/// A model the server's agents play with, and its efforts to choose from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HouseModel {
    /// What runs it ("Claude Code").
    pub program: String,
    /// The provider's id for it ("claude-opus-5-5").
    pub model: String,
    /// Its name ("Opus 5.5").
    pub name: String,
    pub choices: Vec<HouseChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HouseChoice {
    /// Its reasoning effort; none where the model has none to choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The agent's name, which seats it: its account's.
    pub agent: String,
}

impl HouseModel {
    /// The choice seated when only the model is picked: high effort, else the first.
    pub fn default_choice(&self) -> Option<&HouseChoice> {
        (self.choices.iter())
            .find(|c| c.effort.as_deref() == Some("high"))
            .or(self.choices.first())
    }
}

/// The model of the agent `name` among `models`, and its choice.
pub fn house_choice<'a>(
    models: &'a [HouseModel],
    name: &str,
) -> Option<(&'a HouseModel, &'a HouseChoice)> {
    let key = name.trim().to_lowercase();
    models.iter().find_map(|m| {
        (m.choices.iter())
            .find(|c| c.agent.to_lowercase() == key)
            .map(|c| (m, c))
    })
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
    /// Played on dominion.games by an official account.
    Official,
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
    /// Its chess clock, as of `now`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<ClockView>,
    /// People watching now.
    pub watchers: u32,
    /// Why the game ended early, when it did: `resigned`, `abandoned` or `timeout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
    /// The server's clock when this was read, in milliseconds since the Unix epoch: what
    /// `forfeit_at` is to be compared with.
    #[serde(default)]
    pub now: u64,
}

/// A game's clock as it stands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockView {
    /// Per seat: milliseconds left, the running seat's counted down to when this was read.
    pub left: Vec<u64>,
    /// Per seat: milliseconds the seat may still take over its decision before its own
    /// time runs (dominion.games' free decision time). Empty where there is none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free: Vec<u64>,
    /// The seat whose time is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<u8>,
    /// Milliseconds added at the start of each of a player's turns, where that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub increment: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordPlayer {
    pub seat: u8,
    pub name: String,
    /// Absent for a seat played by token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<UserKind>,
    /// The account that played it, where that isn't `name`: the agent playing an
    /// official account. Absent from records written before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub vp: i32,
    pub rank: u32,
    pub turns: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub resigned: bool,
    /// In a rated game: their rating before and after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<(f64, f64)>,
    /// For one of the server's agents: what played the seat, when the game ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<SeatAgent>,
    /// For a user's own bot: its owner's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// One of the server's agents as it played a seat.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatAgent {
    /// Its account's name ("Opus 5.5 High").
    pub agent: String,
    /// What runs it ("Claude Code").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// The provider's id for its model ("claude-opus-5-5").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Its model's name ("Opus 5.5").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its reasoning effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
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
    /// What it was set up with besides those: Platinum and Colony, Shelters, Heirlooms, ...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<String>,
    pub players: Vec<RecordPlayer>,
}

/// `GET /api/users/{name}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub user: UserRef,
    pub rating: RatingInfo,
    pub stats: Stats,
    /// Games they played on dominion.games, which `stats` doesn't count.
    #[serde(default)]
    pub official_games: u32,
    /// For one of the server's agents: what it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<SeatAgent>,
    /// Their latest games, newest first.
    pub recent: Vec<PastGame>,
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

/// Where a game was played.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Venue {
    Arena,
    /// dominion.games, by an official account.
    Official,
}

/// `GET /api/history`'s query: which finished games, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    /// Games this account played.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Games that ended before this, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<Venue>,
    /// Rated games only.
    #[serde(default, skip_serializing_if = "is_false")]
    pub rated: bool,
    /// Games with a player, the account behind a seat or a card whose name contains this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
}

/// How an Arena game's players came together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arranged {
    Automatch,
    Table,
    /// Every seat played by its token.
    Tokens,
}

/// A finished game as the history shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastGame {
    /// Its id in the history.
    pub id: String,
    pub venue: Venue,
    /// How an Arena game's players came together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arranged: Option<Arranged>,
    /// dominion.games' own number for a game played there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official_id: Option<String>,
    /// The server still hosts it: the game page at `/game/{id}` opens it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub open: bool,
    /// Milliseconds since the Unix epoch.
    pub started: u64,
    pub ended: u64,
    pub rated: bool,
    /// How it ended: `provinces-gone`, `resigned`, `abandoned`, ...
    pub reason: String,
    pub kingdom: Vec<String>,
    pub landscapes: Vec<String>,
    /// What it was set up with besides its Kingdom piles and landscapes: Supply piles
    /// beyond the base cards (Platinum, Colony, Potion, ...), Shelters, Heirlooms.
    /// Empty for games recorded before it was kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<String>,
    /// By seat.
    pub players: Vec<PastPlayer>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastPlayer {
    pub seat: u8,
    /// Its name in the game.
    pub name: String,
    /// The account here that played it, if one did: its page is `/user/{account}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub played_by: PlayedBy,
    pub vp: i32,
    /// Lower is better; ties share a rank.
    pub rank: u32,
    pub turns: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub resigned: bool,
    /// Its rating here before and after a rated game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<(f64, f64)>,
}

/// What played a seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlayedBy {
    /// A registered player.
    Person,
    Guest,
    /// Its seat token, with no account.
    Token,
    /// A user's own bot.
    Bot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
    },
    /// One of the server's agents.
    Agent(SeatAgent),
    /// Someone on dominion.games, not from here.
    Official,
}

/// `GET /api/history`: finished games, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub games: Vec<PastGame>,
    /// Ask again with this as `before` for more; none when there are no more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
}
