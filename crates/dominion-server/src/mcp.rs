//! The MCP tools: one `Session` per MCP session, sharing the `Hub`.

use crate::arena::{Arena, Ident};
use crate::hub::{Hub, HubError, NewGame, Seat, panic_message};
use axum::extract::ConnectInfo;
use axum::http::request::Parts;
use dominion_agent::{AgentView, Answer, WayChoice};
use dominion_engine::PlayerId;
use dominion_protocol::lobby::{MatchRequest, MyGame, Opponents, QueueState, SeatKind};
use futures_util::FutureExt;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The longest `wait_for_decision` waits, whatever it is asked for.
pub const MAX_WAIT: Duration = Duration::from_secs(600);

/// How a view is returned as text content.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// The compact text rendering.
    #[default]
    Text,
    /// The view's JSON.
    Json,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JoinParams {
    pub game_id: String,
    /// The seat token create_game returned.
    pub token: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewParams {
    /// Your seat token (optional in an MCP session that has called join_game).
    pub token: Option<String>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitParams {
    pub token: Option<String>,
    /// How long to wait, in milliseconds (default 30000, at most 600000).
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerParams {
    pub token: Option<String>,
    /// The open decision's id, from the view.
    pub decision_id: String,
    /// Option indices: one for most decisions; for ordered decisions, top first.
    // Signed, so a negative index is refused with the other bad answers.
    #[schemars(with = "Option<Vec<u32>>")]
    pub indices: Option<Vec<i64>>,
    /// A card name, instead of indices, when the decision allows naming any card.
    pub name: Option<String>,
    /// Answers the Way question in advance: "none" plays the card as itself, a Way's name
    /// uses that Way. Applies to the Way questions of the card this answer plays (a Throne
    /// Room's replays too), also when other seats react first, and is ignored when none
    /// comes before you are asked something else. A Way that isn't in the game rejects
    /// the answer.
    pub way: Option<String>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayTreasuresParams {
    pub token: Option<String>,
    /// The open decision's id, from the view: a Buy phase move offering Treasures.
    pub decision_id: String,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenParams {
    /// Your seat token (optional in an MCP session that has joined a seat).
    pub token: Option<String>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignInParams {
    /// A bot's API key (made on the web client's account page). Without one you play as
    /// a guest.
    pub api_key: Option<String>,
    /// A guest's name (default: Guest and a number).
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindMatchParams {
    /// The player counts you accept, each 2 to 6 (default [2]).
    pub players: Option<Vec<u8>>,
    /// Who you may be paired with: "any" (default), "humans" or "agents".
    pub opponents: Option<Opponents>,
    /// Accept a rated game (default true; guests' games are never rated).
    pub rated: Option<bool>,
    /// How long to wait for a game, in milliseconds (default 30000, at most 600000).
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JoinTableParams {
    pub table_id: String,
    /// How long to wait for the table's game to start, in milliseconds (default 30000).
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitGameParams {
    /// How long to wait, in milliseconds (default 30000, at most 600000).
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardParams {
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("properties" = {}))]
pub struct NoParams {}

#[derive(Clone)]
pub struct Session {
    arena: Arc<Arena>,
    hub: Arc<Hub>,
    joined: Arc<Mutex<Option<Seat>>>,
    /// The account the session signed in to.
    ident: Arc<Mutex<Option<Ident>>>,
    /// Where its last request came from.
    peer: Arc<Mutex<Option<IpAddr>>>,
    trust_proxy: bool,
    max_wait: Duration,
}

const NOT_SIGNED_IN: &str = "sign in first: call sign_in with a bot's `api_key`, or with \
                             nothing to play as a guest";

fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn structured(value: serde_json::Value, text: String) -> CallToolResult {
    let mut result = CallToolResult::structured(value);
    result.content = vec![ContentBlock::text(text)];
    result
}

fn view_result(view: &AgentView, text: String, format: Format) -> CallToolResult {
    let json = serde_json::to_value(view).expect("views serialize");
    let text = match format {
        Format::Text => text,
        Format::Json => serde_json::to_string_pretty(&json).expect("views serialize"),
    };
    structured(json, text)
}

impl Session {
    /// `max_wait` caps `wait_for_decision`; it must be shorter than the time an idle
    /// session is kept, or a waiting call would outlive its session.
    pub fn new(arena: Arc<Arena>, max_wait: Duration, trust_proxy: bool) -> Session {
        Session {
            trust_proxy,
            hub: arena.hub.clone(),
            arena,
            joined: Arc::new(Mutex::new(None)),
            ident: Arc::new(Mutex::new(None)),
            peer: Arc::new(Mutex::new(None)),
            max_wait: max_wait.min(MAX_WAIT),
        }
    }

    fn ident(&self) -> Result<Ident, String> {
        let ident = self.ident.lock().unwrap_or_else(PoisonError::into_inner);
        ident.clone().ok_or_else(|| NOT_SIGNED_IN.to_string())
    }

    fn peer(&self) -> IpAddr {
        let peer = self.peer.lock().unwrap_or_else(PoisonError::into_inner);
        peer.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    /// Reads who an HTTP request of the session comes from: its address, and the account
    /// of an API key sent as a bearer token.
    fn greet(&self, parts: &Parts) {
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>();
        let peer = peer.map(|ConnectInfo(addr)| *addr);
        let ip = crate::http::client_ip(&parts.headers, peer, self.trust_proxy);
        *self.peer.lock().unwrap_or_else(PoisonError::into_inner) = Some(ip);
        let mut ident = self.ident.lock().unwrap_or_else(PoisonError::into_inner);
        if ident.is_some() {
            return;
        }
        let header = parts.headers.get(axum::http::header::AUTHORIZATION);
        let token = header
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim());
        *ident = token
            .and_then(|t| self.arena.identify(t))
            .map(|i| Ident { agent: true, ..i });
    }

    fn timeout(&self, ms: Option<u64>) -> Duration {
        Duration::from_millis(ms.unwrap_or(30_000)).min(self.max_wait)
    }

    /// The game a search or a table has started for `ident`, with their seat in it.
    fn started(&self, ident: &Ident) -> Option<MyGame> {
        let lobby = self.arena.lobby(Some(ident));
        let matched = lobby
            .queue
            .filter(|q| q.state == QueueState::Matched)
            .and_then(|q| q.game_id);
        let seated = lobby.table.and_then(|mine| {
            let table = lobby.tables.iter().find(|t| t.id == mine.id)?;
            table.game.clone()
        });
        let game = matched.or(seated)?;
        let mut games = lobby.games.into_iter();
        games.find(|g| g.game_id == game && g.running)
    }

    /// Waits for a game to start for `ident`, then binds the session to their seat.
    async fn await_game(&self, ident: &Ident, timeout: Duration) -> Option<MyGame> {
        let _here = self.arena.attend(ident);
        let (mut version, _) = self.arena.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        let game = loop {
            if let Some(game) = self.started(ident) {
                break game;
            }
            match tokio::time::timeout_at(deadline, version.changed()).await {
                Ok(Ok(())) => continue,
                _ => break self.started(ident)?,
            }
        };
        *self.joined() = Some(Seat {
            game: game.game_id.clone(),
            seat: PlayerId(game.seat),
        });
        self.arena.cancel_search(ident);
        Some(game)
    }

    /// What a tool that waited for a game answers.
    fn game_result(&self, game: Option<MyGame>, waiting: &str) -> CallToolResult {
        let Some(game) = game else {
            return CallToolResult::success(vec![ContentBlock::text(waiting.to_string())]);
        };
        let seat = Seat {
            game: game.game_id.clone(),
            seat: PlayerId(game.seat),
        };
        let mut result = self.view_of(&seat, Format::Text);
        let note = format!(
            "Game {}: you are {} (seat {}) against {}. Your seat token is {}; this session \
             is joined to the seat.",
            game.game_id,
            game.name,
            game.seat,
            game.opponents.join(", "),
            game.token
        );
        result.content.insert(0, ContentBlock::text(note));
        result
    }

    fn joined(&self) -> std::sync::MutexGuard<'_, Option<Seat>> {
        self.joined.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn seat(&self, token: Option<&str>) -> Result<Seat, HubError> {
        match token {
            Some(token) => self.hub.find(token),
            None => self.joined().clone().ok_or_else(|| {
                HubError::BadRequest(
                    "this MCP session has not joined a seat (a join lasts until the session \
                     ends: a server restart or a long idle spell ends it): pass your seat \
                     token, or call join_game again"
                        .into(),
                )
            }),
        }
    }

    fn view_of(&self, seat: &Seat, format: Format) -> CallToolResult {
        match self.hub.view(seat) {
            Ok((view, text, note)) => {
                let mut result = view_result(&view, text, format);
                if let Some(note) = note {
                    result.content.insert(0, ContentBlock::text(note));
                }
                result
            }
            Err(e) => failure(e.to_string()),
        }
    }
}

#[tool_router]
impl Session {
    #[tool(
        description = "Create a game. Give `players` and optionally `kingdom` (card names), `landscapes`, `sets` (to deal a random kingdom from), `seed`, `turn_limit`, `names`. Returns the game id and one secret token per seat; hand each token only to whoever plays that seat."
    )]
    async fn create_game(
        &self,
        Parameters(req): Parameters<NewGame>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.hub.create(&req) {
            Ok(created) => {
                let mut text = format!("Created game {}.", created.game_id);
                for s in &created.seats {
                    text.push_str(&format!(
                        "\nseat {} ({}): token {}",
                        s.seat, s.name, s.token
                    ));
                }
                structured(serde_json::to_value(&created).unwrap(), text)
            }
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(
        description = "Bind this MCP session to the seat a token claims, so later calls in it may omit the token. The binding ends with the session (a server restart, or a long idle spell); then join again or pass the token. Returns that seat's view."
    )]
    async fn join_game(
        &self,
        Parameters(p): Parameters<JoinParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.hub.claim(&p.game_id, &p.token) {
            Ok(seat) => {
                *self.joined() = Some(seat.clone());
                self.view_of(&seat, Format::Text)
            }
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(description = "Your seat's current view of the game (agent-view@2).")]
    async fn view(
        &self,
        Parameters(p): Parameters<ViewParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.seat(p.token.as_deref()) {
            Ok(seat) => self.view_of(&seat, p.format),
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(
        description = "Wait until your seat must decide or the game ends (or the timeout passes), then return your view. Its `decision` is null if you don't have to decide yet."
    )]
    async fn wait_for_decision(
        &self,
        Parameters(p): Parameters<WaitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        if let Err(e) = self.hub.wait(&seat, self.timeout(p.timeout_ms)).await {
            return Ok(failure(e.to_string()));
        }
        Ok(self.view_of(&seat, p.format))
    }

    #[tool(
        description = "Answer your open decision with option indices (or `name` where any card may be named). In a game with Ways, `way` (\"none\" or a Way's name) also answers the \"play it as itself, or use a Way\" question for the card your answer plays, saving a call. Returns your next view, or why the answer was rejected."
    )]
    async fn answer(
        &self,
        Parameters(p): Parameters<AnswerParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        let way = match p.way.as_deref().map(WayChoice::parse).transpose() {
            Ok(way) => way,
            Err(e) => return Ok(failure(format!("rejected: {e}"))),
        };
        let indices = match p.indices {
            Some(v) => match v.iter().find(|&&i| u32::try_from(i).is_err()) {
                Some(i) => {
                    return Ok(failure(format!(
                        "rejected: there is no option {i}: options are numbered from 0"
                    )));
                }
                None => Some(v.into_iter().map(|i| i as u32).collect()),
            },
            None => None,
        };
        let answer = Answer {
            decision: p.decision_id,
            indices,
            name: p.name,
        };
        Ok(match self.hub.answer_with(&seat, &answer, way) {
            Ok(answered) => {
                let mut result = self.view_of(&seat, p.format);
                if let Some(note) = answered.note() {
                    result.content.insert(0, ContentBlock::text(note));
                }
                result
            }
            Err(e) => failure(format!("rejected: {e}")),
        })
    }

    #[tool(
        description = "Autoplay Treasures: in a Buy phase move, play the Treasures it offers one after another, instead of answering once per Treasure. Plays only the Treasures offered when called (not ones drawn meanwhile, as by Figurine), and stops early when you are asked anything else (a Treasure's own question). Returns your next view."
    )]
    async fn play_treasures(
        &self,
        Parameters(p): Parameters<PlayTreasuresParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        Ok(match self.hub.play_treasures(&seat, &p.decision_id) {
            Ok(played) => {
                let mut result = self.view_of(&seat, p.format);
                let note = format!(
                    "Played {}.",
                    dominion_engine::amount(played, "Treasure", "Treasures")
                );
                result.content.insert(0, ContentBlock::text(note));
                result
            }
            Err(e) => failure(format!("rejected: {e}")),
        })
    }

    #[tool(
        description = "Resign: the game ends at once and you finish last. For a game you can't or won't finish."
    )]
    async fn resign(
        &self,
        Parameters(p): Parameters<TokenParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        Ok(match self.hub.resign(&seat, "resigned") {
            Ok(()) => self.view_of(&seat, p.format),
            Err(e) => failure(format!("rejected: {e}")),
        })
    }

    #[tool(
        description = "Sign in to the lobby, to find games against other players: with a bot's `api_key`, or with nothing (or a `name`) as a guest. Lasts for this MCP session. Not needed to play a seat you hold a token for."
    )]
    async fn sign_in(
        &self,
        Parameters(p): Parameters<SignInParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let signed_in = match p.api_key.as_deref() {
            Some(key) => self
                .arena
                .identify(key)
                .ok_or_else(|| "that is not a bot's API key".to_string()),
            None => self
                .arena
                .guest(self.peer(), p.name.as_deref())
                .map_err(|e| e.to_string())
                .and_then(|(_, token)| {
                    (self.arena.identify(&token)).ok_or_else(|| "the sign-in failed".to_string())
                }),
        };
        let ident = match signed_in {
            Ok(ident) => Ident {
                agent: true,
                ..ident
            },
            Err(e) => return Ok(failure(e)),
        };
        let me = match self.arena.me(&ident) {
            Ok(me) => me,
            Err(e) => return Ok(failure(e.to_string())),
        };
        *self.ident.lock().unwrap_or_else(PoisonError::into_inner) = Some(ident);
        let text = format!(
            "Signed in as {} ({:?}). find_match pairs you with an opponent; list_tables \
             and join_table sit you at a table.",
            me.user.name, me.user.kind
        );
        Ok(structured(serde_json::to_value(&me).unwrap(), text))
    }

    #[tool(
        description = "Automatch: search for a game against other players (after sign_in) and wait for it. When one is found this session is joined to your seat and you get its view; then loop wait_for_decision / answer as usual. If the wait times out you are still searching: call find_match again to keep waiting, or cancel_match. Lobby games are timed: a player who must decide and has gone away for some minutes forfeits."
    )]
    async fn find_match(
        &self,
        Parameters(p): Parameters<FindMatchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let ident = match self.ident() {
            Ok(ident) => ident,
            Err(e) => return Ok(failure(e)),
        };
        let asked = p.players.is_some() || p.opponents.is_some() || p.rated.is_some();
        let defaults = MatchRequest::default();
        let request = MatchRequest {
            players: p.players.unwrap_or(defaults.players),
            opponents: p.opponents.unwrap_or_default(),
            rated: p.rated.unwrap_or(defaults.rated),
            agent: None,
        };
        let searching = (self.arena.queue_status(&ident))
            .is_some_and(|q| q.state == QueueState::Searching && (!asked || q.request == request));
        if !searching && self.started(&ident).is_none() {
            let (arena, who) = (self.arena.clone(), ident.clone());
            let search = tokio::task::spawn_blocking(move || arena.search(&who, &request));
            match search.await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Ok(failure(e.to_string())),
                Err(_) => return Ok(failure("the search failed inside the server")),
            }
        }
        let game = self.await_game(&ident, self.timeout(p.timeout_ms)).await;
        Ok(self.game_result(
            game,
            "Still searching: nobody to play yet. Call find_match again to keep waiting \
             (stop calling and the search ends within a minute), or cancel_match.",
        ))
    }

    #[tool(description = "Stop searching for a game.")]
    async fn cancel_match(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.ident() {
            Ok(ident) => {
                self.arena.cancel_search(&ident);
                CallToolResult::success(vec![ContentBlock::text("No longer searching.")])
            }
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "The lobby's tables: who hosts each, its seats, and whether its game is running. join_table sits you at one with an open seat."
    )]
    async fn list_tables(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let ident = self.ident().ok();
        let lobby = self.arena.lobby(ident.as_ref());
        let lines: Vec<String> = (lobby.tables.iter())
            .map(|t| {
                let seats: Vec<String> = (t.seats.iter())
                    .map(|s| match (&s.kind, &s.user, &s.name) {
                        (SeatKind::Player, Some(u), _) => u.name.clone(),
                        (SeatKind::Token, _, Some(n)) => format!("{n} (by token)"),
                        (SeatKind::Agent, Some(u), _) => format!("{} (the server's agent)", u.name),
                        _ => "open".to_string(),
                    })
                    .collect();
                let state = match &t.game {
                    Some(game) => format!("playing {game}"),
                    None => "waiting".to_string(),
                };
                format!(
                    "{}: host {}; seats {}; {state}",
                    t.id,
                    t.host,
                    seats.join(", ")
                )
            })
            .collect();
        let text = match lines.is_empty() {
            true => "No tables.".to_string(),
            false => lines.join("\n"),
        };
        Ok(structured(
            serde_json::json!({ "tables": lobby.tables }),
            text,
        ))
    }

    #[tool(
        description = "Sit down at a lobby table (after sign_in), say you are ready, and wait for its game to start: it does once every seat is taken and everyone is ready. Then this session is joined to your seat and you get its view. If the wait times out you are still seated: call wait_for_game to keep waiting, or leave_table."
    )]
    async fn join_table(
        &self,
        Parameters(p): Parameters<JoinTableParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let ident = match self.ident() {
            Ok(ident) => ident,
            Err(e) => return Ok(failure(e)),
        };
        let (arena, who, id) = (self.arena.clone(), ident.clone(), p.table_id.clone());
        let sat = tokio::task::spawn_blocking(move || {
            arena.join_table(&who, &id, None)?;
            arena.ready(&who, &id, true)
        });
        match sat.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Ok(failure(e.to_string())),
            Err(_) => return Ok(failure("sitting down failed inside the server")),
        }
        let game = self.await_game(&ident, self.timeout(p.timeout_ms)).await;
        Ok(self.game_result(
            game,
            "Seated and ready; the game hasn't started yet. Call wait_for_game to keep \
             waiting (stop calling and you leave the table within a few minutes), or \
             leave_table.",
        ))
    }

    #[tool(
        description = "Keep waiting for the game of the table you sit at, or of your search, to start."
    )]
    async fn wait_for_game(
        &self,
        Parameters(p): Parameters<WaitGameParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let ident = match self.ident() {
            Ok(ident) => ident,
            Err(e) => return Ok(failure(e)),
        };
        let game = self.await_game(&ident, self.timeout(p.timeout_ms)).await;
        Ok(self.game_result(game, "No game has started for you yet."))
    }

    #[tool(description = "Leave the lobby table you sit at.")]
    async fn leave_table(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.ident() {
            Ok(ident) => {
                self.arena.leave_table(&ident);
                CallToolResult::success(vec![ContentBlock::text("You left the table.")])
            }
            Err(e) => failure(e),
        })
    }

    #[tool(description = "A card's official text, types, cost and set, by name.")]
    async fn card(
        &self,
        Parameters(p): Parameters<CardParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match dominion_agent::glossary(&p.name) {
            Some(entry) => {
                let text = crate::card_text(&entry);
                structured(serde_json::to_value(&entry).unwrap(), text)
            }
            None => failure(format!("no card is named {:?}", p.name)),
        })
    }

    #[tool(description = "The games on this server (no tokens).")]
    async fn list_games(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let games = self.hub.list();
        let text = if games.is_empty() {
            "No games.".to_string()
        } else {
            games
                .iter()
                .map(|g| {
                    let state = match (g.running, g.deciding) {
                        (false, _) => "over".to_string(),
                        (true, Some(p)) => format!("seat {p} to decide"),
                        (true, None) => "running".to_string(),
                    };
                    format!(
                        "{}: {} ({state}); kingdom {}",
                        g.game_id,
                        g.names.join(", "),
                        g.kingdom.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(structured(serde_json::json!({ "games": games }), text))
    }
}

#[tool_handler(
    name = "dominion",
    instructions = "Play Dominion. create_game returns a token per seat; a seat's \
token is the only way to see or act for it. Pass `token` to view, wait_for_decision and \
answer. (join_game binds a seat to this MCP session so later calls may leave the token \
out, but only until the session ends: keep the token.) Loop: \
wait_for_decision, read the view, then answer the decision with option indices (in order \
when the decision is ordered, or `name` when it says any card may be named; `way` answers \
a Way question in advance; play_treasures plays all your Treasures in one call). Views are \
dominion/agent-view@2: the JSON is the structured content, the text rendering the text \
content. To play other people instead of a game you create: sign_in (a bot's API key, \
also accepted as this connection's bearer token, or as a guest), then find_match, or \
list_tables and join_table; each returns your seat once a game starts. resign gives a \
game up."
)]
impl ServerHandler for Session {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = request.name.to_string();
        if let Some(parts) = context.extensions.get::<Parts>() {
            self.greet(parts);
        }
        let router = Self::tool_router();
        answered(
            &tool,
            router.call(ToolCallContext::new(self, request, context)),
        )
        .await
    }
}

/// Runs a tool call. rmcp leaves a request whose handler panics unanswered, so a panic
/// becomes an error result.
async fn answered(
    tool: &str,
    call: impl Future<Output = Result<CallToolResponse, ErrorData>>,
) -> Result<CallToolResponse, ErrorData> {
    match AssertUnwindSafe(call).catch_unwind().await {
        Ok(result) => result,
        Err(panic) => Ok(failure(format!(
            "{tool} failed inside the server ({}). This is a bug in the server",
            panic_message(&*panic)
        ))
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_documents_the_way_argument() {
        let router = Session::tool_router();
        let tool = router.get("answer").unwrap();
        assert!(tool.description.as_deref().unwrap().contains("`way`"));
        let way = &tool.input_schema["properties"]["way"];
        assert!(
            way["description"].as_str().unwrap().contains("\"none\""),
            "{way}"
        );
    }

    #[tokio::test]
    async fn a_tool_call_that_panics_is_answered_with_an_error() {
        let answer = answered("view", async { panic!("boom") }).await;
        let Ok(CallToolResponse::Complete(result)) = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().unwrap().text.clone();
        assert_eq!(
            text,
            "view failed inside the server (boom). This is a bug in the server"
        );
    }
}
