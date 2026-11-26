//! Arena lifecycle translated by one bounded socket actor.
pub(crate) mod context;
mod rules;

use super::{
    Host,
    identity::{self, ProjectionRequest},
    profile, projection,
    questions::PreparedQuestion,
};
use crate::{
    arena::{self, Arena, Attendance, Ident, StartCheck},
    hub::{Connection, Seat},
    limits::Limiter,
};
use axum::extract::ws::{CloseFrame, Message as WsMessage, WebSocket};
use dominion_online::{
    catalog::VERSION,
    server::{
        ClientCommand, LifecycleCommand, ServerEncoder, decode_lifecycle_command, encode_probe,
    },
    transport::{self, Message},
};
use dominion_protocol::lobby::{ChatKind, Credentials, NewTable, UserKind};
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::{
    sync::watch,
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

const AUTH_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE: Duration = Duration::from_secs(30);
const SEND: Duration = Duration::from_secs(5);
const HEARTBEAT: Duration = Duration::from_secs(5);
struct Failure(u16, &'static str);
type Result<T> = std::result::Result<T, Failure>;
struct Principal {
    ident: Ident,
    token: String,
    _attendance: Attendance,
    _check: Arc<StartCheck>,
}
struct Playing {
    seat: Seat,
    connected: Option<Connection>,
    feed: watch::Receiver<u64>,
    stamp: Option<(Option<String>, u64, bool)>,
    initialized: bool,
    version: u64,
}
struct Session {
    arena: Arc<Arena>,
    host: Arc<Host>,
    ip: IpAddr,
    stop: CancellationToken,
    encoder: ServerEncoder,
    principal: Option<Principal>,
    table: Option<String>,
    table_details: Option<transport::TableDetails>,
    /// The last refusal explained, so a refresh doesn't repeat it.
    refused: Option<String>,
    playing: Option<Playing>,
    active: Option<PreparedQuestion>,
    question_index: i32,
    epoch: u64,
    chat_cursors: std::collections::HashMap<String, u64>,
    update_kind: i32,
}
impl Session {
    fn authorized(&self) -> Result<&Principal> {
        let p = self
            .principal
            .as_ref()
            .ok_or(Failure(1008, "authentication required"))?;
        if self.arena.identify(&p.token).as_ref() != Some(&p.ident) {
            return Err(Failure(1008, "authorization expired"));
        }
        Ok(p)
    }
    async fn send(&self, socket: &mut WebSocket, message: WsMessage) -> Result<()> {
        if self.principal.is_some() {
            self.authorized()?;
        }
        tokio::select! {
            _=self.stop.cancelled()=>Err(Failure(1001,"server stopping")),
            sent=timeout(SEND,socket.send(message))=>match sent {Ok(Ok(()))=>Ok(()),_=>Err(Failure(1001,"socket send failed or timed out"))}
        }
    }
    async fn messages(&mut self, socket: &mut WebSocket, messages: &[Message]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let bytes = self
            .encoder
            .encode_batch(messages)
            .map_err(|_| Failure(1011, "protocol encoding failed"))?;
        self.send(socket, WsMessage::Binary(bytes.into())).await
    }
    async fn reject(&mut self, socket: &mut WebSocket, reason: &'static str) -> Result<()> {
        self.messages(
            socket,
            &[Message::Failed(transport::Failure { reason, card: None })],
        )
        .await
    }
    async fn explain(
        &mut self,
        socket: &mut WebSocket,
        reason: &'static str,
        detail: String,
    ) -> Result<()> {
        self.messages(
            socket,
            &[
                Message::Failed(transport::Failure { reason, card: None }),
                Message::FormResult {
                    form: "arena-compatibility".into(),
                    succeeded: false,
                    error: Some(detail),
                },
            ],
        )
        .await
    }

    /// [`Self::explain`], unless this refusal was the last one explained.
    async fn refuse(
        &mut self,
        socket: &mut WebSocket,
        reason: &'static str,
        detail: String,
    ) -> Result<()> {
        if self.refused.as_ref() == Some(&detail) {
            return Ok(());
        }
        self.refused = Some(detail.clone());
        self.explain(socket, reason, detail).await
    }

    async fn authenticate(
        &mut self,
        socket: &mut WebSocket,
        token: String,
        player: Option<i32>,
    ) -> Result<()> {
        let Some(ident) = self.arena.identify(&token) else {
            return self.reject(socket, "INVALID_SESSION_ID").await;
        };
        let me = self
            .arena
            .me(&ident)
            .map_err(|_| Failure(1008, "account unavailable"))?;
        let id = identity::account(&ident.user, me.user.kind == UserKind::Bot)
            .map_err(|_| Failure(1008, "identity unavailable"))?;
        if player.is_some_and(|p| p != id) {
            return self.reject(socket, "INVALID_SESSION_ID").await;
        }
        let preferences = self
            .host
            .preferences
            .read(&ident.user)
            .map_err(|_| Failure(1011, "account preferences unavailable"))?;
        let host = self.host.clone();
        let check: Arc<StartCheck> = Arc::new(move |config, meta| {
            if meta.undo != dominion_protocol::lobby::UndoRule::Off {
                return Err("official lifecycle requires Arena's undo-off policy".into());
            }
            profile::check_supported(config, meta, &host.catalog, host.unreviewed_cards)
                .map_err(|e| e.to_string())
        });
        let check = self.arena.check_starts(&ident, check);
        let attendance = self.arena.attend(&ident);
        self.principal = Some(Principal {
            ident,
            token: token.clone(),
            _attendance: attendance,
            _check: check,
        });
        let mut login = transport::LoginSuccess::new(id, me.user.name, token);
        let presets = (self.host.catalog.timer_presets())
            .map_err(|_| Failure(1011, "timer presets unavailable"))?;
        for (preset, timer) in presets {
            if let Some(old) = login.timer_presets.iter_mut().find(|(p, _)| *p == preset) {
                old.1 = timer;
            }
        }
        login.reconnecting = self
            .arena
            .hub
            .seats_of(&self.authorized()?.ident.user, Duration::from_secs(3600))
            .iter()
            .any(|g| g.running);
        for preference in preferences.preferences() {
            if let Some(old) = login.preferences.iter_mut().find(|p| p.id == preference.id) {
                *old = preference;
            } else {
                login.preferences.push(preference);
            }
        }
        self.messages(socket, &[Message::Login(login)]).await?;
        self.refresh(socket, true).await
    }
    async fn refresh(&mut self, socket: &mut WebSocket, force: bool) -> Result<()> {
        let ident = self.authorized()?.ident.clone();
        let lobby = self.arena.lobby(Some(&ident));
        let table_id = lobby.table.as_ref().map(|t| t.id.clone());
        let table = lobby
            .tables
            .iter()
            .find(|t| Some(&t.id) == table_id.as_ref());
        let left_table = self.table.is_some() && table_id != self.table;
        if table_id != self.table {
            self.active = None;
            if left_table {
                self.playing = None;
            }
            self.epoch = self
                .epoch
                .checked_add(1)
                .ok_or(Failure(1011, "identity epoch exhausted"))?;
            if let Some(previous) = &self.table {
                let table_id = identity::number(previous, 't')
                    .map_err(|_| Failure(1008, "table identity unavailable"))?;
                self.messages(
                    socket,
                    &[Message::PlayerLeft {
                        table_id,
                        player_id: self.own_id()?,
                    }],
                )
                .await?;
            }
            self.table = table_id;
            self.table_details = None;
            self.refused = None;
        }
        // A table their client can't show (a card it doesn't have) is refused, and so is
        // its game: their lobby says the kingdom couldn't be set up.
        let details = match table.map(|t| rules::details(&self.arena, t, &self.host)) {
            Some(Err(error)) => {
                self.refuse(socket, "COULD_NOT_CREATE_KINGDOM", error)
                    .await?;
                None
            }
            details => details.and_then(std::result::Result::ok),
        };
        let representable = table.is_none() || details.is_some();
        if let Some(details) = details {
            if self.table_details.is_none() {
                self.messages(
                    socket,
                    &[Message::TableJoined {
                        table: details.clone(),
                        show: true,
                    }],
                )
                .await?;
            } else if force || self.table_details.as_ref() != Some(&details) {
                self.messages(socket, &[Message::TableDetails(details.clone())])
                    .await?;
            }
            self.table_details = Some(details);
        }
        let desired = if !representable {
            None
        } else if let Some(table) = table {
            table
                .game
                .clone()
                .or_else(|| {
                    self.playing
                        .as_ref()
                        .filter(|p| Some(&p.seat.game) == table.last_game.as_ref())
                        .map(|p| p.seat.game.clone())
                })
                .or_else(|| if force { table.last_game.clone() } else { None })
        } else if self.playing.is_none() && force && !left_table {
            lobby
                .games
                .iter()
                .find(|g| g.running)
                .or_else(|| lobby.games.first())
                .map(|g| g.game_id.clone())
        } else if self.table.is_none() {
            self.playing.as_ref().map(|p| p.seat.game.clone())
        } else {
            None
        };
        if desired.as_deref() != self.playing.as_ref().map(|p| p.seat.game.as_str()) {
            self.active = None;
            self.playing = None;
            self.epoch = self
                .epoch
                .checked_add(1)
                .ok_or(Failure(1011, "identity epoch exhausted"))?;
            if let Some(game) = desired {
                let seat = self
                    .arena
                    .hub
                    .owned_in(&game, &ident.user)
                    .into_iter()
                    .next()
                    .ok_or(Failure(1008, "game seat unavailable"))?;
                let supported =
                    self.arena
                        .hub
                        .admission(&game, |config, meta| {
                            if meta.undo != dominion_protocol::lobby::UndoRule::Off {
                                return Err("official lifecycle requires Arena's undo-off policy"
                                    .to_string());
                            }
                            profile::check_supported(
                                config,
                                meta,
                                &self.host.catalog,
                                self.host.unreviewed_cards,
                            )
                            .map_err(|e| e.to_string())
                        })
                        .map_err(|_| Failure(1008, "game unavailable"))?;
                if let Err(error) = supported {
                    return self
                        .refuse(socket, "COULD_NOT_CREATE_KINGDOM", error.to_string())
                        .await;
                }
                self.refused = None;
                let feed = self
                    .arena
                    .hub
                    .subscribe(&game)
                    .map_err(|_| Failure(1008, "game unavailable"))?
                    .version;
                let connected = self
                    .arena
                    .hub
                    .connect(&seat)
                    .map_err(|_| Failure(1008, "game unavailable"))?;
                let context = context::context(&self.arena, &seat, &self.host)
                    .map_err(|_| Failure(1008, "identity unavailable"))?;
                if self.table.is_none() {
                    self.messages(
                        socket,
                        &[Message::TableJoined {
                            table: context.table_details.clone(),
                            show: true,
                        }],
                    )
                    .await?;
                }
                self.messages(
                    socket,
                    &[Message::GameStarted {
                        table_id: context.wire_ids.table_id,
                        game_id: context.wire_ids.game_id,
                    }],
                )
                .await?;
                self.playing = Some(Playing {
                    seat,
                    connected: Some(connected),
                    feed,
                    stamp: None,
                    initialized: false,
                    version: 0,
                });
            }
        }
        self.game_update(socket, force).await?;
        if self.update_kind == 3 || force {
            let tables = lobby
                .tables
                .iter()
                .map(|t| rules::summary(&self.arena, t))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Failure(1008, "lobby identities unavailable"))?;
            self.messages(socket, &[Message::Tables(tables)]).await?;
        }
        self.chats(socket).await
    }
    async fn game_update(&mut self, socket: &mut WebSocket, force: bool) -> Result<()> {
        let Some(playing) = &self.playing else {
            return Ok(());
        };
        let seat = playing.seat.clone();
        let initialized = playing.initialized;
        let context = context::context(&self.arena, &seat, &self.host)
            .map_err(|_| Failure(1008, "game context unavailable"))?;
        if context.table_options.timed
            || context.table_options.undo != dominion_protocol::lobby::UndoRule::Off
        {
            self.active = None;
            let playing = self.playing.as_mut().unwrap();
            playing.version = *playing.feed.borrow();
            return self.reject(socket, "ILLEGAL_REQUEST").await;
        }
        let question_index = self
            .question_index
            .checked_add(1)
            .ok_or(Failure(1011, "question index exhausted"))?;
        let previous = playing.stamp.clone();
        let (version, stamp, projected) = self
            .arena
            .hub
            .project_seat(&seat, |o| {
                let stamp = (
                    o.decision_id.map(str::to_string),
                    o.undos,
                    o.outcome.is_some(),
                );
                let version = o.version;
                let update = if !force && previous.as_ref() == Some(&stamp) {
                    None
                } else {
                    Some(projection::project(
                        o,
                        &context,
                        &self.host.catalog,
                        ProjectionRequest {
                            question_index,
                            reconnect: initialized,
                            epoch: self.epoch,
                        },
                    ))
                };
                (version, stamp, update)
            })
            .map_err(|_| Failure(1008, "game unavailable"))?;
        if let Some(playing) = &mut self.playing {
            playing.version = version;
        }
        let Some(projected) = projected else {
            return Ok(());
        };
        self.active = None;
        let update = match projected {
            Ok(update) => update,
            Err(error) => {
                self.playing.as_mut().unwrap().stamp = Some(stamp);
                return self
                    .explain(socket, "ILLEGAL_REQUEST", error.to_string())
                    .await;
            }
        };
        let _ = version;
        self.question_index = question_index;
        let mut messages = vec![Message::Snapshot(update.snapshot)];
        if !update.timers.is_empty() {
            messages.push(Message::Timers(update.timers));
        }
        if let Some(result) = update.finished {
            if force || previous.as_ref().is_none_or(|s| !s.2) {
                messages.push(Message::GameFinished {
                    table: context.table_details,
                    result,
                    continue_allowed: self.table.is_some(),
                    match_completed: true,
                });
            }
        } else if let Some(question) = &update.question {
            messages.push(Message::Question(question.wire.clone()));
        }
        self.messages(socket, &messages).await?;
        self.active = update.question;
        if let Some(playing) = &mut self.playing {
            playing.initialized = !stamp.2;
            if stamp.2 {
                playing.connected = None;
            }
            playing.stamp = Some(stamp);
        }
        Ok(())
    }
    async fn chats(&mut self, socket: &mut WebSocket) -> Result<()> {
        let mut channels = vec!["lobby".to_string()];
        if let Some(table) = &self.table {
            channels.push(format!("table-{table}"));
        }
        if let Some(game) = &self.playing {
            channels.push(format!("game-{}", game.seat.game));
        }
        let mut messages = vec![];
        for channel in channels {
            let cursor = *self.chat_cursors.get(&channel).unwrap_or(&0);
            for message in self.arena.chat_log(&channel, cursor).messages {
                self.chat_cursors.insert(channel.clone(), message.seq);
                let receiver = if let Some(table) = channel.strip_prefix("table-") {
                    table.to_string()
                } else if channel.starts_with("game-") {
                    self.playing
                        .as_ref()
                        .and_then(|p| context::context(&self.arena, &p.seat, &self.host).ok())
                        .map(|c| format!("t{}", c.wire_ids.table_id))
                        .unwrap_or_else(|| "lobby".into())
                } else {
                    channel.clone()
                };
                let sender = if message.from.is_empty() {
                    String::new()
                } else {
                    self.arena
                        .account_id(&message.from)
                        .ok()
                        .and_then(|(id, kind)| identity::account(&id, kind == UserKind::Bot).ok())
                        .map(|id| format!("p{id}"))
                        .unwrap_or(message.from)
                };
                messages.push(Message::Chat {
                    receiver,
                    sender,
                    text: message.text,
                });
            }
        }
        self.messages(socket, &messages).await
    }
    async fn command(&mut self, socket: &mut WebSocket, bytes: &[u8]) -> Result<bool> {
        let command = match decode_lifecycle_command(bytes) {
            Ok(c) => c,
            Err(dominion_online::Error::Unsupported(_)) => {
                self.reject(socket, "ILLEGAL_REQUEST").await?;
                return Ok(true);
            }
            Err(_) => return Err(Failure(1002, "malformed binary command")),
        };
        if self.principal.is_none() {
            match command {
                LifecycleCommand::RequestServerState { version, .. } => {
                    let state = if version == VERSION {
                        transport::ServerState::Normal
                    } else {
                        transport::ServerState::BadClientVersion
                    };
                    let bytes = encode_probe(0, state)
                        .map_err(|_| Failure(1011, "probe encoding failed"))?;
                    self.send(socket, WsMessage::Binary(bytes.into())).await?;
                    return Ok(false);
                }
                LifecycleCommand::Login {
                    username,
                    password,
                    version,
                    ..
                } => {
                    if version != VERSION {
                        self.reject(socket, "BAD_CLIENT_VERSION").await?;
                        return Ok(true);
                    }
                    let arena = self.arena.clone();
                    let ip = self.ip;
                    let login = tokio::task::spawn_blocking(move || {
                        arena.login(
                            ip,
                            &Credentials {
                                name: username,
                                password: password.expose().to_string(),
                                invite: None,
                            },
                        )
                    })
                    .await
                    .map_err(|_| Failure(1011, "login task failed"))?;
                    match login {
                        Ok((_, token)) => self.authenticate(socket, token, None).await?,
                        Err(_) => self.reject(socket, "WRONG_PASSWORD").await?,
                    }
                }
                LifecycleCommand::LoginWithSession {
                    player,
                    session,
                    version,
                    ..
                }
                | LifecycleCommand::ReconnectSession {
                    player,
                    session,
                    version,
                } => {
                    if version != VERSION {
                        self.reject(socket, "BAD_CLIENT_VERSION").await?;
                    } else {
                        self.authenticate(socket, session.expose().into(), Some(player))
                            .await?;
                    }
                }
                LifecycleCommand::LoginWithCode { code, version } => {
                    if version != VERSION {
                        self.reject(socket, "BAD_CLIENT_VERSION").await?;
                    } else {
                        let token = self.host.redeem(code.expose(), &self.arena);
                        match token {
                            Some(token) => self.authenticate(socket, token, None).await?,
                            None => self.reject(socket, "NO_SUCH_CODE").await?,
                        }
                    }
                }
                _ => return Err(Failure(1008, "authentication required")),
            }
            return Ok(true);
        }
        let ident = self.authorized()?.ident.clone();
        self.arena.touch(&ident);
        let result: arena::Result<()> = match command {
            LifecycleCommand::Game(command) => {
                return self.game_command(socket, command).await.map(|_| true);
            }
            LifecycleCommand::Logout => {
                let token = self.authorized()?.token.clone();
                self.arena.logout(&token);
                return Ok(false);
            }
            LifecycleCommand::NewTableRequest { rules: requested } => {
                let mut spec = rules::defaults();
                let seats = requested
                    .iter()
                    .find_map(|r| match (&r.value, r.id) {
                        (transport::RuleValue::Int(n), 10) if (2..=6).contains(n) => {
                            Some(*n as usize)
                        }
                        _ => None,
                    })
                    .unwrap_or(2);
                let applied = requested
                    .iter()
                    .try_for_each(|rule| rules::apply(&self.host, &mut spec, rule, seats));
                applied
                    .and_then(|_| {
                        self.arena.new_table(
                            &ident,
                            &NewTable {
                                spec,
                                seats: Some(vec![
                                    dominion_protocol::lobby::SeatEdit::Open;
                                    seats - 1
                                ]),
                            },
                        )
                    })
                    .map(|_| ())
            }
            LifecycleCommand::JoinTable {
                table,
                as_player: true,
            }
            | LifecycleCommand::SitDown { table } => identity::canonical(table, 't')
                .map_err(arena::Error::BadRequest)
                .and_then(|id| self.arena.join_table(&ident, &id, None))
                .map(|_| ()),
            LifecycleCommand::JoinTable {
                as_player: false, ..
            }
            | LifecycleCommand::StandUp { .. } => Err(arena::Error::BadRequest(
                "Arena has no spectator table membership".into(),
            )),
            LifecycleCommand::ChangeTableRule { table, rule } => identity::canonical(table, 't')
                .map_err(arena::Error::BadRequest)
                .and_then(|id| match (&rule.value, rule.id) {
                    (transport::RuleValue::Int(n), 10) if (2..=6).contains(n) => {
                        self.arena.resize_table(&ident, &id, *n as usize)
                    }
                    _ => self.arena.transform_table_spec(&ident, &id, |spec, seats| {
                        rules::apply(&self.host, spec, &rule, seats)
                    }),
                })
                .map(|_| ()),
            LifecycleCommand::ReadyToStart { table, ready } => identity::canonical(table, 't')
                .map_err(arena::Error::BadRequest)
                .and_then(|id| self.arena.ready(&ident, &id, ready))
                .map(|_| ()),
            LifecycleCommand::LeaveTable { table, player } => {
                let own = self.own_id()?;
                if player != own {
                    self.table_player(table, player)
                        .and_then(|(table, target)| {
                            self.arena.remove_player(&ident, &table, &target)
                        })
                        .map(|_| ())
                } else if identity::canonical(table, 't').ok().as_ref() != self.table.as_ref() {
                    Err(arena::Error::Forbidden(
                        "only your current membership can be left".into(),
                    ))
                } else {
                    self.arena.leave_table(&ident);
                    Ok(())
                }
            }
            LifecycleCommand::ChangeHost { table, player } => self
                .table_player(table, player)
                .and_then(|(table, target)| self.arena.transfer_host(&ident, &table, &target))
                .map(|_| ()),
            LifecycleCommand::RequestUpdate { kind } => {
                self.update_kind = kind;
                match kind {
                    0 | 3 => Ok(()),
                    2 => {
                        self.messages(
                            socket,
                            &[Message::Relationships(transport::Relationships {
                                following: vec![],
                                friends: vec![],
                                followers: vec![],
                                blacklisted: vec![],
                            })],
                        )
                        .await?;
                        Ok(())
                    }
                    _ => Err(arena::Error::BadRequest("automatch is not mapped".into())),
                }
            }
            LifecycleCommand::Unsupported { opcode: 27 } => {
                self.messages(
                    socket,
                    &[Message::Ancillary(
                        transport::ancillary::Message::MailSummaries(vec![]),
                    )],
                )
                .await?;
                Ok(())
            }
            LifecycleCommand::Unsupported { opcode: 26 } => {
                self.messages(
                    socket,
                    &[Message::Ancillary(transport::ancillary::Message::Mails(
                        vec![],
                    ))],
                )
                .await?;
                Ok(())
            }
            LifecycleCommand::RequestPlayerAvatars { .. } => {
                self.messages(
                    socket,
                    &[Message::Ancillary(
                        transport::ancillary::Message::PlayerAvatars(vec![]),
                    )],
                )
                .await?;
                Ok(())
            }
            LifecycleCommand::ChangeUserProperty { preference }
            | LifecycleCommand::PersistUserProperty { preference } => {
                match self.host.preferences.update(&ident.user, &preference) {
                    Ok(()) => {
                        self.messages(socket, &[Message::Preferences(vec![preference])])
                            .await?;
                        Ok(())
                    }
                    Err(e) => Err(arena::Error::BadRequest(e)),
                }
            }
            LifecycleCommand::SendChat {
                receiver,
                sender: _,
                text,
            } => {
                let channel = match receiver.as_str() {
                    "lobby" => Some("lobby".to_string()),
                    "table" => self.table.as_ref().map(|t| format!("table-{t}")),
                    "game" => self
                        .playing
                        .as_ref()
                        .map(|p| format!("game-{}", p.seat.game)),
                    _ if receiver.starts_with('t') => {
                        let table = identity::number(&receiver, 't').ok();
                        let game = self
                            .playing
                            .as_ref()
                            .filter(|p| p.stamp.as_ref().is_none_or(|s| !s.2))
                            .and_then(|p| {
                                context::context(&self.arena, &p.seat, &self.host)
                                    .ok()
                                    .map(|c| (p, c))
                            });
                        if let Some((playing, context)) = game
                            && Some(context.wire_ids.table_id) == table
                        {
                            Some(format!("game-{}", playing.seat.game))
                        } else {
                            Some(format!("table-{receiver}"))
                        }
                    }
                    _ => Some(receiver),
                };
                match channel {
                    Some(channel) if channel.starts_with("game-") => {
                        if let Some(playing) = &self.playing
                            && channel == format!("game-{}", playing.seat.game)
                        {
                            let name = self
                                .arena
                                .name_of(&ident)
                                .map_err(|_| Failure(1008, "account unavailable"))?;
                            self.arena.say_in_game(
                                &playing.seat.game,
                                &ident.user,
                                &name,
                                ChatKind::Player,
                                &text,
                            )
                        } else {
                            Err(arena::Error::Forbidden("not your game channel".into()))
                        }
                    }
                    Some(channel) => self.arena.say(&ident, &channel, &text),
                    None => Err(arena::Error::Forbidden("channel unavailable".into())),
                }
            }
            // A player's dominion.games profile (the score page asks for each player's):
            // Arena keeps none, so there is nothing to send.
            LifecycleCommand::Unsupported { opcode: 57 } => Ok(()),
            LifecycleCommand::Unsupported { opcode } => Err(arena::Error::BadRequest(format!(
                "official operation {opcode} is not implemented"
            ))),
            _ => Err(arena::Error::BadRequest(
                "official operation is not implemented".into(),
            )),
        };
        if let Err(error) = result {
            let detail = match error {
                arena::Error::BadRequest(message)
                | arena::Error::Forbidden(message)
                | arena::Error::Conflict(message)
                | arena::Error::NotFound(message) => message,
                _ => "Arena could not complete this operation".into(),
            };
            self.explain(socket, "ILLEGAL_REQUEST", detail).await?;
        }
        self.refresh(socket, false).await?;
        Ok(true)
    }
    fn table_player(&self, table: i64, player: i32) -> arena::Result<(String, String)> {
        let id = identity::canonical(table, 't').map_err(arena::Error::BadRequest)?;
        let table = self.arena.table(&id)?;
        for seat in &table.seats {
            if let Some(user) = &seat.user {
                let (account, kind) = self.arena.account_id(&user.name)?;
                if identity::account(&account, kind == UserKind::Bot)
                    .map_err(arena::Error::BadRequest)?
                    == player
                {
                    return Ok((id, account));
                }
            }
        }
        Err(arena::Error::NotFound("player is not at this table".into()))
    }
    fn own_id(&self) -> Result<i32> {
        let p = self.authorized()?;
        let me = self
            .arena
            .me(&p.ident)
            .map_err(|_| Failure(1008, "account unavailable"))?;
        identity::account(&p.ident.user, me.user.kind == UserKind::Bot)
            .map_err(|_| Failure(1008, "identity unavailable"))
    }
    async fn game_command(&mut self, socket: &mut WebSocket, command: ClientCommand) -> Result<()> {
        match command {
            ClientCommand::Ping => self.messages(socket, &[Message::Pong]).await,
            ClientCommand::AnimationCompensationAck => Ok(()),
            ClientCommand::ReconnectGame => self.refresh(socket, true).await,
            ClientCommand::AnswerQuestion {
                question, values, ..
            } => {
                let response = self
                    .active
                    .as_ref()
                    .and_then(|p| p.resolve_at(self.epoch, question, &values).ok());
                let result = match (&self.playing, &self.active, response) {
                    (Some(game), Some(plan), Some(response)) => {
                        self.arena
                            .hub
                            .respond(&game.seat, &plan.decision_id, response)
                    }
                    _ => return self.reject(socket, "ILLEGAL_ANSWER").await,
                };
                if result.is_err() {
                    self.reject(socket, "ILLEGAL_ANSWER").await?;
                } else {
                    self.active = None;
                }
                self.refresh(socket, false).await
            }
            ClientCommand::Resign => {
                if let Some(game) = &self.playing {
                    self.arena
                        .hub
                        .resign(&game.seat, "resigned")
                        .map_err(|_| Failure(1008, "game unavailable"))?;
                } else {
                    return self.reject(socket, "COMMAND_REQUIRES_GAME").await;
                }
                self.active = None;
                self.refresh(socket, false).await
            }
        }
    }
    async fn drive(&mut self, socket: &mut WebSocket) -> Result<()> {
        let (mut lobby, mut chat) = self.arena.subscribe();
        let mut tick = tokio::time::interval(HEARTBEAT);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        let connected = Instant::now();
        let mut received = Instant::now();
        let commands = Limiter::new(256, Duration::from_secs(1));
        loop {
            if let Some(game) = &mut self.playing {
                let notified = *game.feed.borrow_and_update();
                if notified != game.version {
                    self.game_update(socket, false).await?;
                    continue;
                }
            }
            tokio::select! {
                _=self.stop.cancelled()=>return Err(Failure(1001,"server stopping")),
                _=lobby.changed(),if self.principal.is_some()=>self.refresh(socket,false).await?,
                _=chat.changed(),if self.principal.is_some()=>self.chats(socket).await?,
                _=async {match &mut self.playing {Some(game)=>{let _=game.feed.changed().await;},None=>std::future::pending::<()>().await}}=>self.game_update(socket,false).await?,
                _=tick.tick()=>{
                    if self.principal.is_none() && connected.elapsed()>=AUTH_TIMEOUT {return Err(Failure(1008,"authentication deadline exceeded"));}
                    if received.elapsed()>=IDLE{return Err(Failure(1001,"heartbeat timed out"));}
                    if self.principal.is_some(){let ident=self.authorized()?.ident.clone();self.arena.touch(&ident);self.refresh(socket,false).await?;}
                    self.send(socket,WsMessage::Ping(vec![].into())).await?;
                }
                incoming=socket.recv()=>{
                    let Some(incoming)=incoming else{return Ok(())};let incoming=incoming.map_err(|_|Failure(1002,"invalid WebSocket frame"))?;
                    received=Instant::now();commands.take(()).map_err(|_|Failure(1008,"command rate exceeded"))?;
                    match incoming {
                        WsMessage::Binary(bytes)=>if !self.command(socket,&bytes).await?{return Ok(())},
                        WsMessage::Ping(bytes)=>self.send(socket,WsMessage::Pong(bytes)).await?,WsMessage::Pong(_)=>{},WsMessage::Close(_)=>return Ok(()),WsMessage::Text(_)=>return Err(Failure(1008,"binary commands required")),
                    }
                }
            }
        }
    }
}
pub(crate) async fn run(
    mut socket: WebSocket,
    arena: Arc<Arena>,
    host: Arc<Host>,
    ip: IpAddr,
    stop: CancellationToken,
) {
    let mut session = Session {
        arena,
        host,
        ip,
        stop,
        encoder: ServerEncoder::new(),
        principal: None,
        table: None,
        table_details: None,
        refused: None,
        playing: None,
        active: None,
        question_index: 0,
        epoch: 0,
        chat_cursors: Default::default(),
        update_kind: 0,
    };
    let failure = session.drive(&mut socket).await.err();
    let (code, reason) = failure.map_or((1000, "session closed"), |Failure(code, reason)| {
        (code, reason)
    });
    let _ = timeout(
        SEND,
        socket.send(WsMessage::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
}
