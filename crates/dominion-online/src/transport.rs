//! Version-pinned binary messages and a non-replaying official-server websocket.
pub mod ancillary;
mod lobby;
pub mod outgoing;
use crate::{
    Error, Result,
    model::{Event, GameResult, LogChange, Snapshot, Ticker},
    questions::Question,
    wire::{Reader, Writer},
};
use futures_util::{SinkExt, StreamExt};
pub use lobby::*;
use std::time::Duration;
use tokio::{
    net::TcpStream,
    time::{Instant, timeout_at},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message as WsMessage, client::IntoClientRequest, http::HeaderValue},
};

const IDLE: Duration = Duration::from_secs(10);
const PONG_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Alpha,
    Beta,
}
impl Endpoint {
    pub fn other(self) -> Self {
        match self {
            Self::Alpha => Self::Beta,
            Self::Beta => Self::Alpha,
        }
    }
    pub fn url(self) -> &'static str {
        match self {
            Self::Alpha => "wss://prod-dominion-alpha.dominion.games:443/",
            Self::Beta => "wss://prod-dominion-beta.dominion.games:443/",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerState {
    AskingReconnect,
    Normal,
    NoNewGames,
    BadClientVersion,
    Down,
}
impl ServerState {
    fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(match r.int()? {
            0 => Self::AskingReconnect,
            1 => Self::Normal,
            2 => Self::NoNewGames,
            3 => Self::BadClientVersion,
            4 => Self::Down,
            x => return Err(Error::Unsupported(format!("server-state ordinal {x}"))),
        })
    }
}
#[derive(Clone, Debug)]
pub struct Failure {
    pub reason: &'static str,
    pub card: Option<i32>,
}
impl Failure {
    pub fn already_logged_in(&self) -> bool {
        self.reason == "ALREADY_LOGGED_IN"
    }
    pub fn error(&self) -> Error {
        Error::Rejected(if self.already_logged_in() {
            "ALREADY_LOGGED_IN: stopped; existing sessions were left untouched".into()
        } else {
            self.reason.into()
        })
    }
    fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let id = r.int()?;
        let reason = FAILURES
            .get(usize::try_from(id).unwrap_or(usize::MAX))
            .ok_or_else(|| Error::Unsupported(format!("failure ordinal {id}")))?;
        let card = r.int()?;
        if !(-1..881).contains(&card) {
            return Err(Error::Protocol("invalid failure card".into()));
        }
        Ok(Self {
            reason,
            card: if card == -1 { None } else { Some(card) },
        })
    }
}
#[derive(Clone, Debug)]
pub struct TimerUpdate {
    pub player: i32,
    pub time: i64,
    pub running: bool,
    pub free_time: i64,
    pub maximum: i64,
}
#[derive(Clone, Debug)]
pub enum Message {
    Chat {
        receiver: String,
        sender: String,
        text: String,
    },
    Failed(Failure),
    Login(LoginSuccess),
    Tables(Vec<TableSummary>),
    RuleChanged {
        table_id: i64,
        rule: TableRule,
    },
    HostChanged {
        table_id: i64,
        host: i32,
    },
    PlayerJoined {
        table_id: i64,
        player: NamedId,
        is_player: bool,
    },
    PlayerLeft {
        table_id: i64,
        player_id: i32,
    },
    PlayerStatus {
        player_id: i32,
        table_id: i64,
        status: i32,
    },
    GameStarted {
        table_id: i64,
        game_id: i64,
    },
    TableJoined {
        table: TableDetails,
        show: bool,
    },
    TableRemoved,
    RoleChanged {
        table_id: i64,
        player_id: i32,
        is_player: bool,
        ready: bool,
    },
    TableDetails(TableDetails),
    GameFinished {
        table: TableDetails,
        result: GameResult,
        continue_allowed: bool,
        match_completed: bool,
    },
    Succeeded(i32),
    Preferences(Vec<Preference>),
    Relationships(Relationships),
    AutomatchStatus(AutomatchStatus),
    BotAdded {
        table_id: i64,
        bot: NamedId,
    },
    TableTerminated(i64),
    PlayerReplaced {
        table_id: i64,
        old: NamedId,
        new: NamedId,
    },
    PlayerDisconnected {
        table_id: i64,
        player: NamedId,
    },
    PlayerReconnected {
        table_id: i64,
        player: NamedId,
    },
    Event(Event),
    Log(LogChange),
    Ticker(Ticker),
    Metagame {
        kind: i32,
        player: i32,
        decision: i32,
    },
    Pong,
    Question(Question),
    Snapshot(Snapshot),
    ReconnectInstruction(bool),
    TimeoutInstruction,
    Timers(Vec<TimerUpdate>),
    ServerState(ServerState),
    Explanations {
        values: Vec<(i32, i32, i32)>,
        projected_vp: i32,
    },
    FormResult {
        form: String,
        succeeded: bool,
        error: Option<String>,
    },
    AnimationCompensation,
    LobbyNotice,
    Ancillary(ancillary::Message),
}

/// A decoder is poisoned after any malformed frame: no partial frame is applied.
#[derive(Default)]
pub struct Decoder {
    next_counter: i32,
    probe: bool,
    poisoned: bool,
}
impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn probe() -> Self {
        Self {
            probe: true,
            ..Self::default()
        }
    }
    pub fn decode(&mut self, bytes: &[u8]) -> Result<Vec<Message>> {
        if self.poisoned {
            return Err(Error::Protocol("decoder is stopped".into()));
        }
        let result = self.decode_inner(bytes);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn decode_inner(&mut self, bytes: &[u8]) -> Result<Vec<Message>> {
        let mut r = Reader::new(bytes);
        let mut out = Vec::new();
        while r.remaining() > 0 {
            let counter = r.int()?;
            if counter != self.next_counter {
                return Err(Error::Protocol(format!(
                    "message counter {counter}, expected {}",
                    self.next_counter
                )));
            }
            self.next_counter = self
                .next_counter
                .checked_add(1)
                .ok_or_else(|| Error::Protocol("message counter overflow".into()))?;
            if self.probe {
                out.push(Message::ServerState(ServerState::parse(&mut r)?));
                r.finish()?;
                break;
            }
            out.push(parse_message(&mut r)?);
        }
        Ok(out)
    }
}
fn array<T>(r: &mut Reader<'_>, p: impl Fn(&mut Reader<'_>) -> Result<T>) -> Result<Vec<T>> {
    (0..r.count()?).map(|_| p(r)).collect()
}
fn checked(r: &mut Reader<'_>, n: i32, name: &str) -> Result<i32> {
    let x = r.int()?;
    if (0..n).contains(&x) {
        Ok(x)
    } else {
        Err(Error::Unsupported(format!("{name} ordinal {x}")))
    }
}
pub(crate) fn parse_message(r: &mut Reader<'_>) -> Result<Message> {
    Ok(match r.int()? {
        0 => Message::Chat {
            receiver: r.string()?,
            sender: r.string()?,
            text: r.string()?,
        },
        1 => Message::Failed(Failure::parse(r)?),
        2 => Message::Login(LoginSuccess::parse(r)?),
        3 => Message::Tables(array(r, TableSummary::parse)?),
        4 => Message::RuleChanged {
            table_id: r.long()?,
            rule: TableRule::parse(r)?,
        },
        5 => Message::HostChanged {
            table_id: r.long()?,
            host: r.int()?,
        },
        6 => Message::PlayerJoined {
            table_id: r.long()?,
            player: NamedId::parse(r)?,
            is_player: r.boolean()?,
        },
        7 => Message::PlayerLeft {
            table_id: r.long()?,
            player_id: r.int()?,
        },
        8 => Message::PlayerStatus {
            player_id: r.int()?,
            table_id: r.long()?,
            status: checked(r, 6, "activity")?,
        },
        9 => Message::GameStarted {
            table_id: r.long()?,
            game_id: r.long()?,
        },
        10 => Message::TableJoined {
            table: TableDetails::parse(r)?,
            show: r.boolean()?,
        },
        11 => Message::TableRemoved,
        12 => Message::RoleChanged {
            table_id: r.long()?,
            player_id: r.int()?,
            is_player: r.boolean()?,
            ready: r.boolean()?,
        },
        13 => Message::TableDetails(TableDetails::parse(r)?),
        14 => Message::GameFinished {
            table: TableDetails::parse(r)?,
            result: GameResult::parse(r)?,
            continue_allowed: r.boolean()?,
            match_completed: r.boolean()?,
        },
        15 => Message::Succeeded(checked(r, 11, "success")?),
        16 => Message::Preferences(array(r, Preference::parse)?),
        19 => Message::Relationships(Relationships::parse(r)?),
        20 => Message::AutomatchStatus(AutomatchStatus::parse(r)?),
        22 => Message::BotAdded {
            table_id: r.long()?,
            bot: NamedId::parse(r)?,
        },
        26 => Message::TableTerminated(r.long()?),
        27 => Message::PlayerReplaced {
            table_id: r.long()?,
            old: NamedId::parse(r)?,
            new: NamedId::parse(r)?,
        },
        28 => Message::PlayerDisconnected {
            table_id: r.long()?,
            player: NamedId::parse(r)?,
        },
        29 => Message::PlayerReconnected {
            table_id: r.long()?,
            player: NamedId::parse(r)?,
        },
        32 => Message::Event(Event::parse(r)?),
        33 => Message::Log(LogChange::parse(r)?),
        34 => Message::Ticker(Ticker::parse(r)?),
        35 => Message::Metagame {
            kind: checked(r, 4, "metagame")?,
            player: r.int()?,
            decision: r.int()?,
        },
        36 => Message::Pong,
        37 => Message::Question(Question::parse(r)?),
        38 => Message::Snapshot(Snapshot::parse(r)?),
        39 => Message::ReconnectInstruction(r.boolean()?),
        40 => Message::TimeoutInstruction,
        41 => Message::Timers(array(r, |r| {
            Ok(TimerUpdate {
                player: r.int()?,
                time: r.long()?,
                running: r.boolean()?,
                free_time: r.long()?,
                maximum: r.long()?,
            })
        })?),
        43 => Message::Explanations {
            values: array(r, |r| Ok((r.int()?, r.int()?, r.int()?)))?,
            projected_vp: r.int()?,
        },
        44 => Message::FormResult {
            form: r.string()?,
            succeeded: r.boolean()?,
            error: if r.boolean()? {
                Some(r.string()?)
            } else {
                None
            },
        },
        45 => Message::AnimationCompensation,
        opcode @ (17 | 18 | 21 | 23..=25 | 30 | 31 | 42 | 46..=61) => {
            Message::Ancillary(ancillary::Message::parse(opcode, r)?)
        }
        x => return Err(Error::Unsupported(format!("server opcode {x}"))),
    })
}

pub struct Transport {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    decoder: Decoder,
    last_inbound: Instant,
    pong_deadline: Option<Instant>,
    stopped: bool,
}
impl Transport {
    pub async fn connect(endpoint: Endpoint) -> Result<Self> {
        Self::connect_inner(endpoint, false).await
    }
    pub async fn connect_probe(endpoint: Endpoint) -> Result<Self> {
        Self::connect_inner(endpoint, true).await
    }
    async fn connect_inner(endpoint: Endpoint, probe: bool) -> Result<Self> {
        Self::connect_url(endpoint.url(), probe).await
    }
    async fn connect_url(url: &str, probe: bool) -> Result<Self> {
        let mut request = url.into_client_request().map_err(connection_error)?;
        request
            .headers_mut()
            .insert("Origin", HeaderValue::from_static("https://dominion.games"));
        let (socket, _) = connect_async(request).await.map_err(connection_error)?;
        Ok(Self::from_socket(socket, probe))
    }
    /// Connect to an already authorized endpoint without adding an official-site Origin.
    pub async fn connect_request(
        request: tokio_tungstenite::tungstenite::http::Request<()>,
    ) -> Result<Self> {
        let (socket, _) = connect_async(request).await.map_err(connection_error)?;
        Ok(Self::from_socket(socket, false))
    }
    fn from_socket(socket: WebSocketStream<MaybeTlsStream<TcpStream>>, probe: bool) -> Self {
        Self {
            socket,
            decoder: if probe {
                Decoder::probe()
            } else {
                Decoder::new()
            },
            last_inbound: Instant::now(),
            pong_deadline: None,
            stopped: false,
        }
    }
    pub async fn send(&mut self, bytes: Vec<u8>) -> Result<()> {
        if self.stopped {
            return Err(Error::Connection(
                "connection is stopped; command was not sent".into(),
            ));
        }
        if let Err(e) = self.socket.send(WsMessage::Binary(bytes.into())).await {
            self.stopped = true;
            return Err(connection_error(e));
        }
        Ok(())
    }
    /// Call continuously from one ordered reader task. No message is retransmitted.
    pub async fn receive(&mut self) -> Result<Vec<Message>> {
        if self.stopped {
            return Err(Error::Connection("connection is stopped".into()));
        }
        let result = self.receive_inner().await;
        if result.is_err() {
            self.stopped = true;
            let _ = self.socket.close(None).await;
        }
        result
    }
    async fn receive_inner(&mut self) -> Result<Vec<Message>> {
        loop {
            let deadline = self.pong_deadline.unwrap_or(self.last_inbound + IDLE);
            match timeout_at(deadline, self.socket.next()).await {
                Err(_) => {
                    if self.pong_deadline.is_some() {
                        return Err(Error::Connection(
                            "application PONG deadline exceeded".into(),
                        ));
                    }
                    self.send(command::ping()).await?;
                    self.pong_deadline = Some(Instant::now() + PONG_TIMEOUT);
                }
                Ok(Some(Ok(WsMessage::Binary(bytes)))) => {
                    self.last_inbound = Instant::now();
                    let messages = self.decoder.decode(&bytes)?;
                    if messages.iter().any(|m| matches!(m, Message::Pong)) {
                        self.pong_deadline = None;
                    }
                    return Ok(messages);
                }
                Ok(Some(Ok(WsMessage::Ping(bytes)))) => {
                    self.socket
                        .send(WsMessage::Pong(bytes))
                        .await
                        .map_err(connection_error)?;
                }
                Ok(Some(Ok(WsMessage::Pong(_)))) => {}
                Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => {
                    return Err(Error::Connection(
                        "websocket disconnected; commands were not replayed".into(),
                    ));
                }
                Ok(Some(Err(e))) => return Err(connection_error(e)),
                Ok(Some(Ok(_))) => {
                    return Err(Error::Protocol("expected binary websocket message".into()));
                }
            }
        }
    }
    pub async fn disconnect(&mut self) -> Result<()> {
        self.stopped = true;
        self.socket.close(None).await.map_err(connection_error)
    }
}
fn connection_error(e: impl std::fmt::Display) -> Error {
    Error::Connection(e.to_string())
}

/// Commands deliberately exclude session removal, takeover, and human matching.
pub mod command {
    use super::*;
    fn start(opcode: i32) -> Writer {
        let mut w = Writer::new();
        w.int(opcode);
        w
    }
    pub fn login(username: &str, password: &str, version: &str, language: i32) -> Vec<u8> {
        let mut w = start(1);
        w.string(username);
        w.string(password);
        w.string(version);
        w.int(language);
        w.finish()
    }
    pub fn server_state(version: &str, session: Option<(i32, &str)>) -> Vec<u8> {
        let mut w = start(45);
        w.string(version);
        w.boolean(session.is_some());
        if let Some((id, _)) = session {
            w.int(id);
        }
        w.boolean(session.is_some());
        if let Some((_, token)) = session {
            w.string(token);
        }
        w.finish()
    }
    pub fn login_session(player: i32, token: &str, version: &str, language: i32) -> Vec<u8> {
        let mut w = start(24);
        w.int(player);
        w.string(version);
        w.string(token);
        w.int(language);
        w.finish()
    }
    pub fn reconnect_session(player: i32, token: &str, version: &str) -> Vec<u8> {
        let mut w = start(46);
        w.int(player);
        w.string(version);
        w.string(token);
        w.finish()
    }
    pub fn reconnect_game() -> Vec<u8> {
        start(35).finish()
    }
    pub fn add_bot(table_id: i64) -> Vec<u8> {
        let mut w = start(23);
        w.long(table_id);
        w.finish()
    }
    pub fn ready(table_id: i64, ready: bool) -> Vec<u8> {
        let mut w = start(10);
        w.long(table_id);
        w.boolean(ready);
        w.finish()
    }
    pub fn answer(index: i32, answer: &[i32]) -> Vec<u8> {
        let mut w = start(37);
        w.int(index);
        w.ints(answer);
        w.boolean(false);
        w.finish()
    }
    pub fn animation_ack() -> Vec<u8> {
        start(51).finish()
    }
    pub fn resign() -> Vec<u8> {
        start(38).finish()
    }
    pub fn ping() -> Vec<u8> {
        start(44).finish()
    }
    pub fn change_rule(table_id: i64, rule: &TableRule) -> Result<Vec<u8>> {
        let mut w = start(4);
        w.long(table_id);
        rule.write(&mut w)?;
        Ok(w.finish())
    }
    /// Unrated and unobservable. The caller must verify that only its own seat and bots are present.
    pub fn private_table(rules: &[TableRule]) -> Result<Vec<u8>> {
        let mut w = start(9);
        let private = [
            TableRule {
                id: 5,
                value: RuleValue::Group(Group::Nobody),
            },
            TableRule {
                id: 19,
                value: RuleValue::Boolean(false),
            },
        ];
        if rules.iter().any(|r| [4, 5, 17, 19].contains(&r.id)) {
            return Err(Error::Rejected(
                "private-table access and rating rules cannot be overridden".into(),
            ));
        }
        w.int((rules.len() + private.len()) as i32);
        for r in private.iter().chain(rules) {
            r.write(&mut w)?;
        }
        Ok(w.finish())
    }
}

pub const FAILURES: &[&str] = &[
    "WRONG_PASSWORD",
    "NOT_ENOUGH_PLAYERS",
    "TOO_MANY_PLAYERS",
    "NEW_HOST_NOT_FOUND",
    "ALREADY_JOINED_A_TABLE",
    "NOT_ALLOWED_TO_JOIN",
    "BAD_CLIENT_VERSION",
    "UNKNOWN_RULE_INDEX",
    "RULE_NOT_REMOVABLE",
    "INVITEE_NOT_LOGGED_IN",
    "NOT_ALLOWED_TO_PLAY",
    "TABLE_CHAT_REQUIRES_TABLE",
    "SPECTATOR_CHAT_REQUIRES_TABLE",
    "LOGIN_REQUIRED",
    "COMMAND_REQUIRES_TABLE",
    "COMMAND_REQUIRES_HOST",
    "COMMAND_REQUIRES_PLAYER",
    "RULE_CONFLICTS",
    "NO_GAMESERVER_AVAILABLE",
    "SELECTED_CARD_NOT_OWNED",
    "SELECTED_CARD_NOT_KINGDOM_CARD",
    "SELECTED_TOO_MANY_EVENTS_OR_LANDMARKS",
    "THIS_IS_NOT_YOUR_TABLE",
    "ALREADY_LOGGED_IN",
    "CANNOT_RECONNECT_WHEN_STILL_CONNECTED",
    "KICK_FAILED",
    "ALREADY_STARTING",
    "ALREADY_STARTED",
    "NOTHING_TO_KICK",
    "INTERNAL_ERROR",
    "ILLEGAL_PROPERTY_CHANGE",
    "CODE_ALREADY_USED",
    "FAILED_CONFIRMING_EMAIL",
    "FAILED_RETRIEVING_USER",
    "EMAIL_NOT_CONFIRMED_YET",
    "FAILURE_SENDING_EMAIL",
    "CONFIRM_EMAIL_FIRST",
    "OLD_PASSWORD_INCORRECT",
    "INCORRECT_PRICE",
    "PURCHASE_IN_FUTURE",
    "PURCHASE_TOO_OLD",
    "FAILED_SETTING_UP_PURCHASE",
    "FAILED_STORING_PURCHASE",
    "DUPLICATE_USERNAME",
    "INVALID_USERNAME",
    "INVALID_PASSWORD",
    "INVALID_GENDER",
    "PURCHASE_TOO_SHORT",
    "PURCHASE_TOO_LONG",
    "METASERVER_INACTIVE",
    "NO_SUCH_TABLE",
    "NO_SUCH_USER",
    "NO_SUCH_CODE",
    "RESERVED_USERNAME",
    "PRICE_IS_ZERO",
    "NO_SUCH_USERNAME",
    "NO_SUCH_GAMELOG",
    "ILLEGAL_PLAYER_ORDER",
    "INVALID_SESSION_ID",
    "ACTION_CODE_EXPIRED",
    "BOT_GAMES_CANT_BE_RATED",
    "TOO_MANY_PLAYERS_FOR_RATED_GAME",
    "ALREADY_ON_BLACKLIST",
    "NOT_ON_BLACKLIST",
    "ALREADY_ON_FRIENDLIST",
    "NOT_ON_FRIENDLIST",
    "SEARCHING_FOR_MYSELF",
    "SEARCHED_FOR_BOT",
    "USER_IS_BANNED",
    "WRONG_BONUS_CODE_TYPE",
    "ILLEGAL_BONUS_CODE",
    "STORE_SERVICE_DOWN",
    "RATING_SERVICE_DOWN",
    "NOT_SUBSCRIBED_TO",
    "KINGDOM_HAS_TOO_MANY_CARDS",
    "NO_CARDS_AVAILABLE_TO_FILL_KINGDOM",
    "FAILED_TO_SELECT_BANE_CARD",
    "FAILED_TO_SELECT_MOUSE_CARD",
    "FAILED_TO_SELECT_FERRYMAN_CARD",
    "FAILED_TO_SELECT_PREVIEW_CARDS",
    "FAILED_TO_SELECT_ALLY",
    "FAILED_TO_SELECT_LIASON",
    "REQUIRING_RANDOM_CARD_FROM_DISABLED_EXPANSION",
    "NO_RECONNECT_AVAILABLE",
    "COMMAND_REQUIRES_GAME",
    "ILLEGAL_ANSWER",
    "ILLEGAL_REQUEST",
    "NO_SUCH_UNDO",
    "SERVER_DOESNT_ALLOW_NEW_GAMES",
    "SERVER_IS_DOWN",
    "OTHER_SERVER_IS_UNAVAILABLE",
    "ALREADY_IN_GAME",
    "OPPONENT_JUST_LEFT",
    "TOURNAMENT_MATCH_COMPLETED",
    "GAME_NOT_IN_MATCH",
    "COULD_NOT_PROVIDE_REQUIRED_CARD",
    "COULD_NOT_CREATE_KINGDOM",
    "SANDBOX_CARD_NAME_UNKNOWN",
    "SANDBOX_SEAT_OUT_OF_RANGE",
    "SANDBOX_HAND_NOT_IN_STARTING_CARDS",
    "SANDBOX_CARD_NOT_IN_SUPPLY",
    "SANDBOX_PILE_EXHAUSTED",
    "STATS_VIEW_LIMIT_REACHED",
    "GAME_ID_SHOULD_BE_NUMBER",
];

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::protocol::Role;

    fn packet(counter: i32, opcode: i32) -> Vec<u8> {
        let mut w = Writer::new();
        w.int(counter);
        w.int(opcode);
        w.finish()
    }
    #[test]
    fn exact_counters_multiple_messages_and_poisoning() {
        let mut bytes = packet(0, 36);
        bytes.extend(packet(1, 45));
        let mut decoder = Decoder::new();
        let messages = decoder.decode(&bytes).unwrap();
        assert!(matches!(
            messages.as_slice(),
            [Message::Pong, Message::AnimationCompensation]
        ));
        assert!(decoder.decode(&packet(3, 36)).is_err());
        assert!(decoder.decode(&packet(2, 36)).is_err());
        let mut decoder = Decoder::new();
        let mut bytes = packet(0, 36);
        bytes.extend(packet(1, 999));
        assert!(matches!(decoder.decode(&bytes), Err(Error::Unsupported(_))));
        assert!(decoder.decode(&packet(0, 36)).is_err());
        assert!(Decoder::new().decode(&[0, 0, 0]).is_err());
    }
    #[test]
    fn probe_is_counter_then_state_without_opcode() {
        assert!(matches!(
            Decoder::probe().decode(&packet(0, 1)).unwrap().as_slice(),
            [Message::ServerState(ServerState::Normal)]
        ));
        let mut bytes = packet(0, 1);
        bytes.extend([0]);
        assert!(Decoder::probe().decode(&bytes).is_err());
    }
    #[test]
    fn commands_match_upstream_answer_fixture_and_login_layout() {
        assert_eq!(
            command::answer(42, &[2, 0, 1]),
            [
                0, 0, 0, 37, 0, 0, 0, 42, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0
            ]
        );
        let bytes = command::login("fixture-user", "fixture-password", "2.3.7", -1);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.int().unwrap(), 1);
        assert_eq!(r.string().unwrap(), "fixture-user");
        assert_eq!(r.string().unwrap(), "fixture-password");
        assert_eq!(r.string().unwrap(), "2.3.7");
        assert_eq!(r.int().unwrap(), -1);
        r.finish().unwrap();
        let bytes = command::server_state("2.3.7", Some((42, "fixture-token")));
        let mut r = Reader::new(&bytes);
        assert_eq!(r.int().unwrap(), 45);
        assert_eq!(r.string().unwrap(), "2.3.7");
        assert!(r.boolean().unwrap());
        assert_eq!(r.int().unwrap(), 42);
        assert!(r.boolean().unwrap());
        assert_eq!(r.string().unwrap(), "fixture-token");
        r.finish().unwrap();
    }
    #[test]
    fn duplicate_login_has_no_takeover_action() {
        let mut w = Writer::new();
        w.int(0);
        w.int(1);
        w.int(23);
        w.int(-1);
        let messages = Decoder::new().decode(&w.finish()).unwrap();
        let Message::Failed(failure) = &messages[0] else {
            panic!("failure expected")
        };
        assert!(failure.already_logged_in());
        assert!(failure.error().to_string().contains("left untouched"));
    }
    #[test]
    fn login_token_is_redacted_from_debug() {
        let mut w = Writer::new();
        w.int(0);
        w.int(2);
        w.int(42);
        w.string("fixture-user");
        w.boolean(false);
        w.int(0);
        w.int(0);
        w.string("private-fixture-token");
        w.int(0);
        for _ in 0..4 {
            w.int(0);
        }
        w.int(0);
        w.int(0);
        w.int(0);
        let messages = Decoder::new().decode(&w.finish()).unwrap();
        assert!(!format!("{messages:?}").contains("private-fixture-token"));
        let Message::Login(login) = &messages[0] else {
            panic!("login expected")
        };
        assert_eq!(login.session_id(), "private-fixture-token");
    }
    #[test]
    fn private_table_does_not_enable_spectators_or_ratings() {
        let bytes = command::private_table(&[]).unwrap();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.int().unwrap(), 9);
        assert_eq!(r.count().unwrap(), 2);
        assert_eq!(
            TableRule::parse(&mut r).unwrap(),
            TableRule {
                id: 5,
                value: RuleValue::Group(Group::Nobody)
            }
        );
        assert_eq!(
            TableRule::parse(&mut r).unwrap(),
            TableRule {
                id: 19,
                value: RuleValue::Boolean(false)
            }
        );
        r.finish().unwrap();
        assert!(
            command::private_table(&[TableRule {
                id: 19,
                value: RuleValue::Boolean(true)
            }])
            .is_err()
        );
        assert!(
            command::change_rule(
                1,
                &TableRule {
                    id: 4,
                    value: RuleValue::Group(Group::Nobody)
                }
            )
            .is_err()
        );
    }
    async fn socket_pair() -> (Transport, WebSocketStream<TcpStream>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(address), listener.accept());
        let client = WebSocketStream::from_raw_socket(
            MaybeTlsStream::Plain(client.unwrap()),
            Role::Client,
            None,
        )
        .await;
        let server = WebSocketStream::from_raw_socket(server.unwrap().0, Role::Server, None).await;
        (Transport::from_socket(client, false), server)
    }
    #[tokio::test]
    async fn socket_heartbeat_requires_application_pong() {
        let (mut client, mut server) = socket_pair().await;
        tokio::time::pause();
        let reader = tokio::spawn(async move {
            let result = client.receive().await;
            (client, result)
        });
        tokio::time::advance(IDLE).await;
        let WsMessage::Binary(bytes) = server.next().await.unwrap().unwrap() else {
            panic!("application ping expected")
        };
        assert_eq!(bytes.as_ref(), command::ping());
        server
            .send(WsMessage::Binary(packet(0, 36).into()))
            .await
            .unwrap();
        let (client, result) = reader.await.unwrap();
        assert!(matches!(result.unwrap().as_slice(), [Message::Pong]));
        assert!(client.pong_deadline.is_none());
    }
    #[tokio::test]
    async fn other_traffic_does_not_cancel_explicit_pong_deadline() {
        let (mut client, mut server) = socket_pair().await;
        tokio::time::pause();
        let reader = tokio::spawn(async move {
            let result = client.receive().await;
            (client, result)
        });
        tokio::time::advance(IDLE).await;
        assert!(matches!(
            server.next().await.unwrap().unwrap(),
            WsMessage::Binary(_)
        ));
        server
            .send(WsMessage::Binary(packet(0, 45).into()))
            .await
            .unwrap();
        let (mut client, result) = reader.await.unwrap();
        assert!(result.is_ok());
        assert!(client.pong_deadline.is_some());
        let reader = tokio::spawn(async move { client.receive().await });
        tokio::time::advance(PONG_TIMEOUT).await;
        assert!(
            matches!(reader.await.unwrap(),Err(Error::Connection(s)) if s.contains("PONG deadline"))
        );
    }
    #[tokio::test]
    async fn disconnect_does_not_reconnect_or_replay() {
        let (mut client, mut server) = socket_pair().await;
        let command = command::answer(42, &[1]);
        client.send(command.clone()).await.unwrap();
        let WsMessage::Binary(bytes) = server.next().await.unwrap().unwrap() else {
            panic!("answer expected")
        };
        assert_eq!(bytes.as_ref(), command);
        server.close(None).await.unwrap();
        assert!(client.receive().await.is_err());
        assert!(client.send(command::answer(42, &[1])).await.is_err());
        assert!(client.receive().await.is_err());
    }
    #[tokio::test]
    #[allow(clippy::result_large_err)] // Tungstenite fixes the callback error type.
    async fn fake_handshake_has_official_origin() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                socket,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.headers()["origin"], "https://dominion.games");
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let message = socket.next().await.unwrap().unwrap();
            assert!(matches!(message, WsMessage::Close(_)));
        });
        let mut client = Transport::connect_url(&url, false).await.unwrap();
        client.disconnect().await.unwrap();
        server.await.unwrap();
    }

    #[test]
    fn all_default_table_rule_shapes_leave_the_next_message_aligned() {
        let mut w = Writer::new();
        w.int(0);
        w.int(10);
        w.long(100);
        w.int(42);
        w.int(1);
        w.int(42);
        w.string("fixture-user");
        w.int(1);
        w.boolean(false);
        w.int(0);
        w.int(0);
        w.int(0);
        w.int(25);
        for id in 0..25 {
            w.int(id);
            match id {
                0 | 13 | 19 | 20 | 23 => w.boolean(false),
                1 | 2 | 3 | 12 | 14 | 21 => w.int(0),
                4 | 5 | 17 => w.int(0),
                6 | 7 => w.double(0.0),
                8 => {
                    w.boolean(false);
                    w.long(0);
                }
                9 | 10 => w.int(2),
                11 => {
                    w.int(2);
                    w.int(2);
                    w.int(0);
                    w.int(0);
                }
                15 => w.int(1),
                16 => {
                    w.int(0);
                    w.boolean(false);
                    w.int(4);
                }
                18 => {
                    w.long(-1);
                    w.int(-1);
                    w.int(0);
                }
                22 => w.int(1),
                24 => w.string(""),
                _ => unreachable!(),
            }
            w.boolean(false);
        }
        w.string("");
        w.boolean(false);
        w.int(-1);
        w.boolean(true);
        w.int(1);
        w.int(22);
        w.long(100);
        w.int(-2);
        w.string("Lord Rattington");
        w.int(2);
        w.int(9);
        w.long(100);
        w.long(1000);
        let messages = Decoder::new().decode(&w.finish()).unwrap();
        assert!(
            matches!(&messages[0],Message::TableJoined{table,show:true} if table.rules.len()==25 && table.host_id==42 && table.players.len()==1 && table.bots.is_empty())
        );
        assert!(matches!(&messages[1],Message::BotAdded{table_id:100,bot} if bot.id == -2));
        assert!(matches!(
            &messages[2],
            Message::GameStarted {
                table_id: 100,
                game_id: 1000
            }
        ));
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;
    #[test]
    fn dispatch_handles_every_pinned_server_opcode_and_rejects_additions() {
        for opcode in 0_i32..62 {
            let bytes = opcode.to_be_bytes();
            let result = parse_message(&mut Reader::new(&bytes));
            if let Err(error) = result {
                assert!(
                    !error.to_string().contains("server opcode"),
                    "missing handler {opcode}"
                );
            }
        }
        let bytes = 62_i32.to_be_bytes();
        assert!(
            parse_message(&mut Reader::new(&bytes))
                .unwrap_err()
                .to_string()
                .contains("server opcode 62")
        );
    }
}
