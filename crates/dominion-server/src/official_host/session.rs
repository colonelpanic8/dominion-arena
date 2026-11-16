//! One owner serializes snapshots, question plans, counters and socket writes.

use super::{
    Host, identity::ProjectionRequest, lifecycle::context::context, projection,
    questions::PreparedQuestion,
};
use crate::arena::Arena;
use crate::{
    hub::{Hub, HubError, Seat, SeatObservation},
    limits::Limiter,
};
use axum::extract::ws::{CloseFrame, Message as WsMessage, WebSocket};
use dominion_online::{
    server::{ClientCommand, ServerEncoder, decode_command},
    transport::Message,
};
use std::{sync::Arc, time::Duration};
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;

pub(crate) const MAX_INBOUND: usize = 64 * 1024;
const MAX_OUTBOUND: usize = 4 * 1024 * 1024;
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const HEARTBEAT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(PartialEq, Eq)]
struct Stamp {
    decision: Option<String>,
    undos: u64,
    finished: bool,
}

impl Stamp {
    fn of(observation: &SeatObservation<'_>) -> Self {
        Self {
            decision: observation.decision_id.map(String::from),
            undos: observation.undos,
            finished: observation.outcome.is_some(),
        }
    }
}

struct Failure(u16, &'static str);

impl From<HubError> for Failure {
    fn from(error: HubError) -> Self {
        match error {
            HubError::Engine(_) => Self(1011, "game engine failed"),
            _ => Self(1008, "game or decision is no longer available"),
        }
    }
}

struct Session {
    hub: Arc<Hub>,
    arena: Arc<Arena>,
    seat: Seat,
    host: Arc<Host>,
    encoder: ServerEncoder,
    index: i32,
    stamp: Option<Stamp>,
    active: Option<PreparedQuestion>,
    authorize: Box<dyn Fn() -> bool + Send + Sync>,
    stop: CancellationToken,
}

impl Session {
    fn authorized(&self) -> Result<(), Failure> {
        if (self.authorize)() {
            Ok(())
        } else {
            Err(Failure(1008, "authorization expired"))
        }
    }

    async fn send(&self, socket: &mut WebSocket, message: WsMessage) -> Result<(), Failure> {
        self.authorized()?;
        tokio::select! {
            _ = self.stop.cancelled() => Err(Failure(1001, "server stopping")),
            sent = timeout(SEND_TIMEOUT, socket.send(message)) => match sent {
                Ok(Ok(())) => Ok(()),
                _ => Err(Failure(1001, "socket send failed or timed out")),
            }
        }
    }

    async fn messages(
        &mut self,
        socket: &mut WebSocket,
        messages: &[Message],
    ) -> Result<(), Failure> {
        let bytes = self
            .encoder
            .encode_batch(messages)
            .map_err(|_| Failure(1011, "protocol encoding failed"))?;
        if bytes.len() > MAX_OUTBOUND {
            return Err(Failure(1011, "snapshot exceeds protocol limit"));
        }
        self.send(socket, WsMessage::Binary(bytes.into())).await
    }

    async fn update(&mut self, socket: &mut WebSocket, force: bool) -> Result<u64, Failure> {
        self.authorized()?;
        let index = self
            .index
            .checked_add(1)
            .ok_or(Failure(1011, "question index exhausted"))?;
        let context = context(&self.arena, &self.seat, &self.host)
            .map_err(|_| Failure(1008, "identity unavailable"))?;
        let (version, projected) = self.hub.project_seat(&self.seat, |observation| {
            let version = observation.version;
            let stamp = Stamp::of(&observation);
            let projected = if !force && self.stamp.as_ref() == Some(&stamp) {
                None
            } else {
                Some(
                    projection::project(
                        observation,
                        &context,
                        &self.host.catalog,
                        ProjectionRequest {
                            question_index: index,
                            reconnect: self.stamp.is_some(),
                            epoch: index as u64,
                        },
                    )
                    .map(|update| (stamp, update)),
                )
            };
            (version, projected)
        })?;
        let Some(projected) = projected else {
            return Ok(version);
        };
        let (stamp, update) = projected.map_err(|_| Failure(1008, "unsupported game state"))?;
        debug_assert_eq!(update.version, version);
        debug_assert_eq!(update.undos, stamp.undos);
        self.index = index;
        self.active = None;
        let mut messages = vec![
            Message::TableDetails(update.table.clone()),
            Message::Snapshot(update.snapshot),
        ];
        if let Some(result) = update.finished {
            messages.push(Message::GameFinished {
                table: update.table,
                result,
                continue_allowed: false,
                match_completed: true,
            });
        } else if let Some(question) = &update.question {
            messages.push(Message::Question(question.wire.clone()));
        }
        self.messages(socket, &messages).await?;
        self.active = update.question;
        self.stamp = Some(stamp);
        Ok(version)
    }

    async fn command(&mut self, socket: &mut WebSocket, bytes: &[u8]) -> Result<(), Failure> {
        self.authorized()?;
        let command = decode_command(bytes).map_err(|error| match error {
            dominion_online::Error::Unsupported(_) => Failure(1008, "unsupported command"),
            _ => Failure(1002, "malformed binary command"),
        })?;
        match command {
            ClientCommand::AnswerQuestion {
                question, values, ..
            } => {
                let active = self
                    .active
                    .as_ref()
                    .ok_or(Failure(1008, "no active question"))?;
                let response = active
                    .resolve_at(self.index as u64, question, &values)
                    .map_err(|_| Failure(1008, "stale or invalid answer"))?;
                self.hub
                    .respond(&self.seat, &active.decision_id, response)?;
                self.active = None;
                self.update(socket, true).await?;
            }
            ClientCommand::ReconnectGame => {
                self.update(socket, true).await?;
            }
            ClientCommand::Resign => {
                self.hub.resign(&self.seat, "resigned")?;
                self.active = None;
                self.update(socket, true).await?;
            }
            ClientCommand::Ping => self.messages(socket, &[Message::Pong]).await?,
            ClientCommand::AnimationCompensationAck => {}
        }
        Ok(())
    }

    async fn drive(&mut self, socket: &mut WebSocket) -> Result<(), Failure> {
        // Subscribe before projecting, and never mark a newer notification as observed
        // unless its version was included in the projected state.
        let mut feed = self.hub.subscribe(&self.seat.game)?.version;
        let mut projected_version = self.update(socket, true).await?;
        let mut tick = tokio::time::interval(HEARTBEAT);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        let mut received = Instant::now();
        let commands = Limiter::new(256, Duration::from_secs(1));
        loop {
            let notified = *feed.borrow_and_update();
            if notified != projected_version {
                projected_version = self.update(socket, false).await?;
                continue;
            }
            tokio::select! {
                _ = self.stop.cancelled() => return Err(Failure(1001, "server stopping")),
                changed = feed.changed() => {
                    changed.map_err(|_| Failure(1001, "game expired"))?;
                }
                _ = tick.tick() => {
                    self.authorized()?;
                    self.hub.meta(&self.seat.game)?;
                    if received.elapsed() >= IDLE_TIMEOUT {
                        return Err(Failure(1001, "heartbeat timed out"));
                    }
                    self.send(socket, WsMessage::Ping(Vec::new().into())).await?;
                }
                incoming = socket.recv() => {
                    let Some(incoming) = incoming else { return Ok(()) };
                    let incoming = incoming.map_err(|_| Failure(1002, "invalid WebSocket frame"))?;
                    self.authorized()?;
                    received = Instant::now();
                    commands.take(()).map_err(|_| Failure(1008, "command rate exceeded"))?;
                    match incoming {
                        WsMessage::Binary(bytes) => self.command(socket, &bytes).await?,
                        WsMessage::Ping(bytes) => self.send(socket, WsMessage::Pong(bytes)).await?,
                        WsMessage::Pong(_) => {},
                        WsMessage::Close(_) => return Ok(()),
                        WsMessage::Text(_) => return Err(Failure(1008, "binary commands required")),
                    }
                }
            }
        }
    }
}

pub(crate) async fn run(
    mut socket: WebSocket,
    arena: Arc<Arena>,
    seat: Seat,
    host: Arc<Host>,
    authorize: impl Fn() -> bool + Send + Sync + 'static,
    stop: CancellationToken,
) {
    let hub = arena.hub.clone();
    let Ok(_connected) = hub.connect(&seat) else {
        return;
    };
    let mut session = Session {
        hub,
        arena,
        seat,
        host,
        encoder: ServerEncoder::new(),
        index: 0,
        stamp: None,
        active: None,
        authorize: Box::new(authorize),
        stop,
    };
    if let Err(Failure(code, reason)) = session.drive(&mut socket).await {
        let _ = timeout(
            SEND_TIMEOUT,
            socket.send(WsMessage::Close(Some(CloseFrame {
                code,
                reason: reason.into(),
            }))),
        )
        .await;
    }
}
