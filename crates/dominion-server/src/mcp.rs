//! The MCP tools: one `Session` per MCP session, sharing the `Hub`.

pub(crate) mod play;
mod report;
use play::{Play, PlaySeat, failure, structured};

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
    /// Game id for dominion/play@1; returns structured {view, glossary?}.
    pub game: Option<String>,
    /// Legacy spelling; returns the original raw structured view.
    pub game_id: Option<String>,
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
    /// Include the game's grouped card reference in the response content, as on joining.
    #[serde(default)]
    pub include_cards: bool,
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
pub struct UndoParams {
    /// Your seat token (optional in an MCP session that has joined a seat).
    pub token: Option<String>,
    /// Go back to before the answer that led to this log entry (its seq), taking back
    /// every answer after it too, instead of to before your last answer.
    pub log_seq: Option<usize>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerUndoParams {
    /// Your seat token (optional in an MCP session that has joined a seat).
    pub token: Option<String>,
    /// True lets the undo happen; false refuses it.
    pub allow: bool,
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
    /// House agent to play when searching for agents.
    pub agent: Option<String>,
    /// The selected house agent's runner strategy (default classic).
    pub strategy: Option<dominion_protocol::lobby::RunnerStrategy>,
    pub speed: Option<dominion_protocol::lobby::InferenceSpeed>,
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
pub struct StepsParams {
    pub token: Option<String>,
    /// Steps separated by `;`: `play A, B x2` (`play A as Way of X` for a Way), `treasures`
    /// (play every Treasure in hand), `buy A, B`, `pay` (off Debt), `coffers N`,
    /// `villagers N`, `end` (the phase), `next` (end the turn), `pick A, B` or `pick none`
    /// (answer any question by card name, option text or option number), `name CARD`.
    pub steps: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebriefParams {
    /// Your seat token (optional in an MCP session that has joined a seat).
    pub token: Option<String>,
    /// Your debrief: what you would tell the game's creator about how it went.
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportParams {
    /// The game's id.
    pub game: String,
    /// Wait up to this many seconds (at most 600) for the game to be over and its server
    /// agents' debriefs to be in; the report says whether they are (`settled`).
    pub wait_s: Option<u64>,
    /// Also give each turn's log entries.
    #[serde(default)]
    pub detail: bool,
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
    connection: Arc<Mutex<Option<crate::hub::Connection>>>,
    /// The account the session signed in to.
    ident: Arc<Mutex<Option<Ident>>>,
    /// Where its last request came from.
    peer: Arc<Mutex<Option<IpAddr>>>,
    trust_proxy: bool,
    max_wait: Duration,
    presentation: Arc<Mutex<play::Presentation>>,
    clock_cues: bool,
}

const NO_GAME: &str = "call new_game (Arena has token seats, no automatic bots)";
const NOT_SEATED: &str = "NOT_SEATED: pass token to state, or call join_game first";

/// One of Arena's own seats, played through its hub.
pub(crate) struct ArenaSeat<'a> {
    pub hub: &'a Hub,
    pub seat: Seat,
    /// The longest this session may wait.
    pub max_wait: Duration,
}

impl PlaySeat for ArenaSeat<'_> {
    fn observe(&self) -> Result<Option<(Seat, crate::hub::PlayObservation)>, String> {
        match self.hub.play_observation(&self.seat) {
            Ok(observation) => Ok(Some((self.seat.clone(), observation))),
            Err(e) => Err(format!("NO_GAME: {e}; call new_game")),
        }
    }

    async fn answer(&self, answer: &Answer, _: Option<&[String]>) -> Result<(), String> {
        let answered = self.hub.answer_with(&self.seat, answer, None);
        answered.map(drop).map_err(|e| format!("REJECTED: {e}"))
    }

    async fn wait(&self, timeout: Duration) -> Result<(), String> {
        let waited = self.hub.wait(&self.seat, timeout.min(self.max_wait)).await;
        waited
            .map(drop)
            .map_err(|e| format!("NO_GAME: {e}; call new_game"))
    }

    fn cards(&self, names: &[String]) -> Result<Vec<dominion_agent::GlossaryEntry>, String> {
        (self.hub.card_entries(Some(&self.seat), names)).map_err(|e| e.to_string())
    }

    /// One recorded step of the table, which an undo takes back whole.
    async fn play_treasures(&self, decision: &str) -> Result<u32, String> {
        (self.hub.play_treasures(&self.seat, decision)).map_err(|e| format!("REJECTED: {e}"))
    }

    fn advise(&self, text: &str) -> Result<(), String> {
        (self.hub.advise(&self.seat, text)).map_err(|e| format!("REFUSED: {e}"))
    }

    fn no_game(&self) -> String {
        NO_GAME.into()
    }
}

const NOT_SIGNED_IN: &str = "sign in first: call sign_in with a bot's `api_key`, or with \
                             nothing to play as a guest";

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
            connection: Arc::new(Mutex::new(None)),
            ident: Arc::new(Mutex::new(None)),
            peer: Arc::new(Mutex::new(None)),
            max_wait: max_wait.min(MAX_WAIT),
            clock_cues: false,
            presentation: Arc::new(Mutex::new(play::Presentation::default())),
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
            model: false,
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
            model: false,
        };
        let mut result = self.view_with_cards(&seat, Format::Text, true);
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
        let seat = match token {
            Some(token) => self.hub.find(token),
            None => self.joined().clone().ok_or_else(|| {
                HubError::BadRequest(
                    "this MCP session has not joined a seat (a join lasts until the session \
                     ends: a server restart or a long idle spell ends it): pass your seat \
                     token, or call join_game again"
                        .into(),
                )
            }),
        }?;
        if !seat.model {
            self.hub.mark_remote_agent(&seat);
        }
        Ok(seat)
    }

    fn play_seat(&self, token: Option<&str>) -> Result<Seat, CallToolResult> {
        if token.is_none() && self.joined().is_none() && self.hub.list().is_empty() {
            return Err(failure("NO_GAME: no game is running; call new_game"));
        }
        self.seat(token)
            .map_err(|e| failure(format!("NOT_SEATED: {e}; pass token or call join_game")))
    }

    fn reference_seat(&self) -> Option<Seat> {
        self.presentation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .seat
            .clone()
            .or_else(|| self.joined().clone())
    }

    fn arena_seat(&self, seat: Seat) -> ArenaSeat<'_> {
        ArenaSeat {
            hub: &self.hub,
            seat,
            max_wait: self.max_wait,
        }
    }

    /// The play@1 tools of `seat`, as this session is shown them.
    fn play<'a>(&'a self, seat: &'a ArenaSeat<'a>) -> Play<'a, ArenaSeat<'a>> {
        Play {
            seat,
            shown: &self.presentation,
            clock_cues: self.clock_cues,
        }
    }

    fn play_result(&self, seat: &Seat, full: bool) -> CallToolResult {
        self.play(&self.arena_seat(seat.clone())).observation(full)
    }

    /// The session has been shown `seat` through play@1, so its results are deltas.
    fn plays(&self, seat: &Seat) -> bool {
        let shown = self.presentation.lock();
        shown.unwrap_or_else(PoisonError::into_inner).seat.as_ref() == Some(seat)
    }

    fn view_of(&self, seat: &Seat, format: Format) -> CallToolResult {
        self.view_with_cards(seat, format, false)
    }

    fn view_with_cards(&self, seat: &Seat, format: Format, include_cards: bool) -> CallToolResult {
        match self.hub.view(seat) {
            Ok((view, text, note)) => {
                let mut result = view_result(&view, text, format);
                let prompts = self.hub.model_prompts(seat);
                if !prompts.is_empty() {
                    for prompt in &prompts {
                        result.content.push(ContentBlock::text(format!(
                            "{}\n{prompt}",
                            play::GAMEPLAY_INSTRUCTION
                        )));
                    }
                    result
                        .meta
                        .get_or_insert_with(Default::default)
                        .insert("dominion/modelPrompts".into(), serde_json::json!(prompts));
                }
                if include_cards {
                    let cards = dominion_agent::glossary_for(&view);
                    let reference = match format {
                        Format::Text => dominion_agent::text::reference(&view, &cards),
                        Format::Json => {
                            serde_json::to_string_pretty(&serde_json::json!({"cards": cards}))
                                .expect("glossary entries serialize")
                        }
                    };
                    result.content.insert(0, ContentBlock::text(reference));
                }
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
        description = "dominion/play@1: full seat view and new card text in structured {view, glossary?}. Pass token or join_game first. With no games, tells you to call new_game."
    )]
    async fn state(
        &self,
        Parameters(p): Parameters<play::StateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.token.is_none() && self.joined().is_none() && self.hub.list().is_empty() {
            return Ok(play::no_game(NO_GAME));
        }
        Ok(match self.play_seat(p.token.as_deref()) {
            Ok(seat) => self.play_result(&seat, true),
            Err(e) => e,
        })
    }

    #[tool(
        description = "dominion/play@1: answer decision with indices or name (only for nameAny), then wait for your next decision or game end. Give expect with each selected option's label or card, in index order, to catch mistakes. Ordered indices are top first; [] chooses nothing. timeout_s defaults to 45, at most 600. Returns compact changes and full structured {view, glossary?}. Rejected/stale answers change nothing and include the open decision."
    )]
    async fn step(
        &self,
        Parameters(p): Parameters<play::StepParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.play_seat(p.token.as_deref()) {
            Ok(seat) => self.arena_seat(seat),
            Err(e) => return Ok(e),
        };
        Ok(self.play(&seat).step(p).await)
    }

    #[tool(
        description = "dominion/play@1: wait without answering until your next decision, an undo another player asks you to allow (an UNDO line: answer_undo), game end, or timeout_s (default 45, at most 600). Returns compact changes and full structured {view, glossary?}. WAITING means call wait again."
    )]
    async fn wait(
        &self,
        Parameters(p): Parameters<play::WaitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.play_seat(p.token.as_deref()) {
            Ok(seat) => self.arena_seat(seat),
            Err(e) => return Ok(e),
        };
        Ok(self.play(&seat).wait(p.timeout_s).await)
    }

    #[tool(
        description = "dominion/play@1: for a seat a person holds while you advise them (results say ADVISING): write them a short note on what they asked or the decision shown, then wait like wait. Refused when you hold the seat yourself."
    )]
    async fn advise(
        &self,
        Parameters(p): Parameters<play::AdviseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.play_seat(p.token.as_deref()) {
            Ok(seat) => self.arena_seat(seat),
            Err(e) => return Ok(e),
        };
        Ok(self.play(&seat).advise(&p.text, p.timeout_s).await)
    }

    #[tool(
        description = "dominion/play@1: official glossary entries for names, with current costs/types in the last observed seat's game (printed values before joining). Does not expose another seat's information."
    )]
    async fn cards(
        &self,
        Parameters(p): Parameters<play::CardsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.reference_seat() {
            Some(seat) => self.play(&self.arena_seat(seat)).cards(&p.names),
            None => play::glossary_result(
                (self.hub.card_entries(None, &p.names)).map_err(|e| e.to_string()),
            ),
        })
    }

    #[tool(
        description = "dominion/play@1: reread the last observed seat's visible game log after seq `after`, retaining the newest `limit` entries (default 50, at most 1000), oldest first. Join a seat first; no other seat's private entries are returned."
    )]
    async fn log(
        &self,
        Parameters(p): Parameters<play::LogParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.reference_seat() {
            Some(seat) => self.play(&self.arena_seat(seat)).log(&p),
            None => failure(NOT_SEATED),
        })
    }

    #[tool(
        description = "dominion/play@1: the last observed seat's game by turn: one line per turn saying what each player played, bought, gained, trashed, discarded, ... in it, with its log seqs. Turn numbers are each player's own count (`from`, `to`; `player` is a seat index, 0 is P1); with neither from nor to, the newest `last` turns (default 6). `detail` adds each turn's log entries. Join a seat first."
    )]
    async fn turns(
        &self,
        Parameters(p): Parameters<play::TurnsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.reference_seat() {
            Some(seat) => self.play(&self.arena_seat(seat)).turns(&p),
            None => failure(NOT_SEATED),
        })
    }

    #[tool(
        description = "dominion/play@1: create a game, join seat 0, return its full view and all secret seat tokens. players defaults to 2. Arena has no automatic bot opponents: bots must be 0 (default); other seats need agents with their tokens. For explicit registered house-agent profiles use create_game instead. kingdom/landscapes are required inclusions, filled randomly. Extras: sets, names, seed, turn_limit, colonies, shelters, watchable (false makes the game private; anyone may watch otherwise). Never replaces or resigns your previous game."
    )]
    async fn new_game(
        &self,
        Parameters(p): Parameters<play::NewGameParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.bots.unwrap_or(0) != 0 {
            return Ok(failure(
                "REFUSED: Arena has no automatic bots; use bots: 0 and share the other seats' tokens, or use create_game with explicit registered house-agent profiles.",
            ));
        }
        let dealt = match crate::hub::deal(&dominion_protocol::api::DealRequest {
            players: p.players.unwrap_or(2),
            kingdom: p.kingdom,
            landscapes: p.landscapes,
            sets: p.sets,
            seed: p.seed,
            ..Default::default()
        }) {
            Ok(dealt) => dealt,
            Err(e) => return Ok(failure(format!("REFUSED: {e}"))),
        };
        let request = NewGame {
            players: p.players.unwrap_or(2),
            kingdom: dealt.kingdom,
            landscapes: dealt.landscapes,
            seed: Some(dealt.seed),
            names: p.names,
            turn_limit: p.turn_limit,
            colonies: p.colonies,
            shelters: p.shelters,
            watchable: p.watchable,
            ..Default::default()
        };
        let ident = self.ident().ok();
        let created = match self.arena.create_game(ident.as_ref(), &request) {
            Ok(created) => created,
            Err(e) => return Ok(failure(format!("REFUSED: {e}"))),
        };
        let seat = self
            .hub
            .claim(&created.game_id, &created.seats[0].token)
            .expect("new seat exists");
        let connection = match self.hub.connect(&seat) {
            Ok(connection) => connection,
            Err(e) => return Ok(failure(e.to_string())),
        };
        *self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(connection);
        *self.joined() = Some(seat.clone());
        let mut result = self.play_result(&seat, true);
        if let Some(value) = &mut result.structured_content {
            value["game_id"] = serde_json::json!(created.game_id);
            value["seats"] = serde_json::to_value(&created.seats).unwrap();
        }
        let tokens = created
            .seats
            .iter()
            .map(|s| format!("seat {} ({}): token {}", s.seat, s.name, s.token))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(ContentBlock::Text(content)) = result.content.first_mut() {
            let at = content.text.find('\n').unwrap_or(content.text.len());
            content.text.insert_str(at, &format!("\nSEATS (secret; session joined to seat 0; share other tokens only with their players)\n{tokens}"));
        }
        Ok(result)
    }

    #[tool(
        description = "Create a game. Give `players` and optionally `kingdom`, `landscapes`, `sets`, `seed`, `turn_limit`, `names`, `clock` (a chess clock: {initial, increment} in seconds; a seat whose time runs out forfeits). `agents` assigns a profile name (Classic) or {agent, strategy?, speed?} to each seat (null for a token seat); two agents play each other. Requires a registered account to seat agents. Anyone may watch a game unless `watchable` is false. `briefs` gives each seat's player text to read before the game (null for none): a plan to play, notes from earlier games; a server agent gets it with its first view, a token seat on joining. `debrief` is a question every server agent answers once the game is over, before it leaves; game_report collects the answers. `rated: true` counts a game with a different server agent in every seat for their ratings (no undo). Returns the game id and secret tokens only for seats without agents."
    )]
    async fn create_game(
        &self,
        Parameters(req): Parameters<NewGame>,
    ) -> Result<CallToolResult, ErrorData> {
        let ident = self.ident().ok();
        Ok(match self.arena.create_game(ident.as_ref(), &req) {
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
        description = "Bind this MCP session to the seat a token claims, so later calls in it may omit the token. The binding ends with the session (a server restart, or a long idle spell); then join again or pass the token. Use game for the common structured {view, glossary?} and full text; legacy game_id returns the raw view and grouped card reference, including related non-Supply and split-pile cards."
    )]
    async fn join_game(
        &self,
        Parameters(p): Parameters<JoinParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (game, common) = match (&p.game, &p.game_id) {
            (Some(game), None) => (game, true),
            (None, Some(game)) => (game, false),
            _ => {
                return Ok(failure(
                    "give exactly one of game or legacy game_id, plus token",
                ));
            }
        };
        Ok(match self.hub.claim(game, &p.token) {
            Ok(seat) => {
                let connection = match self.hub.connect(&seat) {
                    Ok(connection) => connection,
                    Err(error) => return Ok(failure(error.to_string())),
                };
                *self
                    .connection
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(connection);
                *self.joined() = Some(seat.clone());
                let mut result = if common {
                    self.play_result(&seat, true)
                } else {
                    self.view_with_cards(&seat, Format::Text, true)
                };
                if let Some(brief) = self.hub.brief(&seat) {
                    if let Some(ContentBlock::Text(content)) = result.content.first_mut() {
                        content.text = format!(
                            "BRIEF (from the game's creator: read it before your first decision)\n{brief}\n\n{}",
                            content.text
                        );
                    }
                    if common && let Some(value) = &mut result.structured_content {
                        value["brief"] = serde_json::json!(brief);
                    }
                }
                result
            }
            Err(e) => failure(format!("NOT_SEATED: {e}; check game and token")),
        })
    }

    #[tool(
        description = "Your seat's current view of the game (agent-view@2). Set include_cards to retrieve the grouped card reference again, including cards introduced during play. The reference is additional response content; the structured view is unchanged."
    )]
    async fn view(
        &self,
        Parameters(p): Parameters<ViewParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.seat(p.token.as_deref()) {
            Ok(seat) => self.view_with_cards(&seat, p.format, p.include_cards),
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(
        description = "Wait until your seat must decide, another player's undo asks you to allow it, or the game ends (or the timeout passes), then return your view. Its `decision` is null if you don't have to decide yet."
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
        description = "Autoplay Treasures: in a Buy phase move, play the Treasures it offers one after another, instead of answering once per Treasure. Plays only the Treasures offered when called (not ones drawn meanwhile, as by Figurine), and stops early when you are asked anything else (a Treasure's own question). Returns your next view: as step does in a session playing through state/step/wait, else as answer does."
    )]
    async fn play_treasures(
        &self,
        Parameters(p): Parameters<PlayTreasuresParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        if self.plays(&seat) {
            let seat = self.arena_seat(seat);
            return Ok(self.play(&seat).play_treasures(&p.decision_id).await);
        }
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
        description = "Take back your last answer (or, with log_seq, the answer that led to that log entry), with every answer after it. In games where undos need permission, the other players must allow it first: you get your view with an UNDO line saying whom it waits for, and answering your decision withdraws it. Returns your view."
    )]
    async fn undo(
        &self,
        Parameters(p): Parameters<UndoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        Ok(match self.hub.undo(&seat, p.log_seq) {
            Ok(done) => {
                let mut result = self.view_of(&seat, p.format);
                if done {
                    let note = "Undone: the game is back to before that answer.";
                    result.content.insert(0, ContentBlock::text(note));
                }
                result
            }
            Err(e) => failure(format!("rejected: {e}")),
        })
    }

    #[tool(
        description = "Allow (allow: true) or refuse (allow: false) the undo another player asked for, when your view has an UNDO line asking you. Once everyone asked allows it, the game goes back. Returns your view."
    )]
    async fn answer_undo(
        &self,
        Parameters(p): Parameters<AnswerUndoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        Ok(match self.hub.answer_undo(&seat, p.allow) {
            Ok(done) => {
                let mut result = self.view_of(&seat, p.format);
                if done {
                    let note = "Undone: the game is back to before the answers taken back.";
                    result.content.insert(0, ContentBlock::text(note));
                }
                result
            }
            Err(e) => failure(format!("rejected: {e}")),
        })
    }

    #[tool(description = "Withdraw the undo you asked for. Returns your view.")]
    async fn cancel_undo(
        &self,
        Parameters(p): Parameters<TokenParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        Ok(match self.hub.cancel_undo(&seat) {
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
        description = "Automatch: search for a game against other players (after sign_in) and wait for it. When one is found this session is joined to your seat and you get its view; then loop state / step / wait. If the wait times out you are still searching: call find_match again to keep waiting, or cancel_match. Lobby games are timed: a player who must decide and has gone away for some minutes forfeits."
    )]
    async fn find_match(
        &self,
        Parameters(p): Parameters<FindMatchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let ident = match self.ident() {
            Ok(ident) => ident,
            Err(e) => return Ok(failure(e)),
        };
        let asked = p.players.is_some()
            || p.opponents.is_some()
            || p.rated.is_some()
            || p.agent.is_some()
            || p.strategy.is_some()
            || p.speed.is_some();
        let defaults = MatchRequest::default();
        let request = MatchRequest {
            players: p.players.unwrap_or(defaults.players),
            opponents: p.opponents.unwrap_or_default(),
            options: dominion_protocol::lobby::LaunchOptions {
                strategy: p.strategy.unwrap_or_default(),
                speed: p.speed,
            },
            rated: p.rated.unwrap_or(defaults.rated),
            agent: p.agent,
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

    #[tool(
        description = "Play several decisions at once by naming them: `play Village, Smithy x2; treasures; buy Gold, Silver; next`. Steps after a card that draws or reveals are chosen before you see what came, so end the call there when what comes next depends on it. Stops at the first step that can't be done (the ones before it stand) or when it's no longer your decision. Returns what happened and your view; then call wait_for_decision for your next decision. Use `answer` for anything steps can't express."
    )]
    async fn steps(
        &self,
        Parameters(p): Parameters<StepsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        let stepped = match self.hub.steps(&seat, &p.steps) {
            Ok(s) => s,
            Err(e) => return Ok(failure(e.to_string())),
        };
        let (mut view, _, _) = match self.hub.view(&seat) {
            Ok(v) => v,
            Err(e) => return Ok(failure(e.to_string())),
        };
        view.since_last_decision.clear();
        let mut out = vec![];
        if !stepped.log.is_empty() {
            out.push("DONE".to_string());
            out.extend(stepped.log.iter().map(|l| format!("  {l}")));
        }
        if let Some((step, why)) = &stepped.stopped {
            out.push(format!("STOPPED at {step:?}: {why}"));
        }
        out.push(dominion_agent::text::render(&view));
        Ok(CallToolResult::success(vec![ContentBlock::text(
            out.join("\n"),
        )]))
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

    #[tool(
        description = "Once your game is over: record your debrief for the game's creator (answering its debrief question, if it set one), in place of any you gave before. game_report shows it."
    )]
    async fn debrief(
        &self,
        Parameters(p): Parameters<DebriefParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        if let Err(e) = self.hub.submit_debrief(&seat, &p.text) {
            return Ok(failure(e.to_string()));
        }
        // A server agent's job ends with its debrief.
        self.arena.dispatch();
        Ok(structured(
            serde_json::json!({"game": seat.game, "seat": seat.seat.0}),
            format!(
                "Debrief recorded for seat {} of game {}.",
                seat.seat.0, seat.game
            ),
        ))
    }

    #[tool(
        description = "A game as a record, for running and comparing games: its setup (kingdom, landscapes, seed: pass them to create_game to deal the same kingdom and starting cards again; swap the agents and briefs to swap the seats), its seats with their accounts and results, a summary line per turn (detail adds the log entries) and, once it is over, every player's final deck, the briefs, the debrief question and each seat's debrief. wait_s waits for the game to end and the server agents' debriefs to arrive (settled). For a watchable game, one you created, or one you play."
    )]
    async fn game_report(
        &self,
        Parameters(p): Parameters<ReportParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let meta = match self.hub.meta(&p.game) {
            Ok(meta) => meta,
            Err(e) => return Ok(failure(e.to_string())),
        };
        let user = self.ident().ok().map(|i| i.user);
        let mine = user.as_deref().is_some_and(|u| {
            meta.host.as_deref() == Some(u) || meta.owners.iter().any(|o| o.as_deref() == Some(u))
        });
        let seated = self.joined().as_ref().is_some_and(|s| s.game == p.game);
        if !(meta.watchable || mine || seated) {
            return Ok(failure(format!(
                "game {} isn't watchable, and you neither created nor play it",
                p.game
            )));
        }
        let wait = Duration::from_secs(p.wait_s.unwrap_or(0)).min(MAX_WAIT);
        let deadline = tokio::time::Instant::now() + wait;
        let report = loop {
            let version = match self.hub.version(&p.game) {
                Ok(version) => version,
                Err(e) => return Ok(failure(e.to_string())),
            };
            let report = match self.hub.report(&p.game) {
                Ok(report) => report,
                Err(e) => return Ok(failure(e.to_string())),
            };
            let now = tokio::time::Instant::now();
            if report.settled || now >= deadline {
                break report;
            }
            // Debriefs stop being waited for at a deadline no change marks: look again.
            let next = (deadline - now).min(Duration::from_secs(10));
            self.hub.changed_or(&p.game, version, next).await;
        };
        let (value, text) = report::render(&report, p.detail);
        Ok(structured(value, text))
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
    instructions = "Play Dominion through dominion/play@1 (agent-view@2). Call state to orient. Arena hosts secret token seats: new_game joins seat 0 and returns tokens for all seats; bots defaults to 0 because there are no automatic bot opponents. Share the other tokens with their players. To join an existing seat, call join_game with game and token (legacy game_id returns the original raw view), or pass token to state/step/wait. Keep your token: a binding lasts only for this MCP session. Loop: read the returned view, step with its decision, indices (ordered: top first) or name when nameAny; give expect with each chosen label/card. step waits for your next decision and returns compact changes plus structured {view, glossary?}. When decision is null call wait; finish when view.game.running is false. state refreshes the full view; cards looks up official text/current costs; log rereads entries by seq. Forced decisions are answered by the engine. Legacy view/answer/wait_for_decision/card/create_game remain available with their original wire formats; answer supports way and play_treasures batches Treasures. For lobby play: sign_in (bot API key, connection bearer token, or guest), then find_match or list_tables/join_table. Those bind your seat when a game starts. House agents require explicit profiles via create_game and registered-account authorization; its briefs and debrief give them plans and collect what they learned, and game_report gives any game you may see as a record. resign gives up your seat."
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
        let session = Self {
            clock_cues: play::show_clock_cues(&context.meta),
            ..self.clone()
        };
        let router = Self::tool_router();
        answered(
            &tool,
            router.call(ToolCallContext::new(&session, request, context)),
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

pub(crate) fn tool_definition(name: &str) -> Option<rmcp::model::Tool> {
    Session::tool_router().get(name).cloned()
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
    async fn a_join_holds_presence_until_the_session_is_dropped() {
        let hub = Arc::new(Hub::default());
        let game = hub
            .create(&crate::hub::NewGame {
                players: 2,
                ..Default::default()
            })
            .unwrap();
        let arena = Arena::open(hub.clone(), Default::default()).unwrap();
        let session = Session::new(arena, Duration::from_secs(60), false);
        let joined = session
            .join_game(Parameters(JoinParams {
                game: None,
                game_id: Some(game.game_id.clone()),
                token: game.seats[0].token.clone(),
            }))
            .await
            .unwrap();
        assert_ne!(joined.is_error, Some(true));
        let seat = Seat {
            game: game.game_id.clone(),
            seat: PlayerId(0),
            model: false,
        };
        hub.forget_last_call(&seat);
        assert!(hub.seat_view(&seat, None).unwrap().meta.connected[0]);
        let connection = Arc::downgrade(&session.connection);
        assert!(session.connection.lock().unwrap().is_some());
        let clone = session.clone();
        drop(session);
        assert!(connection.upgrade().is_some());
        drop(clone);
        assert!(connection.upgrade().is_none());
        hub.forget_last_call(&seat);
        assert!(!hub.seat_view(&seat, None).unwrap().meta.connected[0]);
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

    #[tokio::test]
    async fn the_model_attached_to_a_seat_advises_its_player_and_answers_nothing() {
        use crate::hub::{Meta, NewGame};
        use dominion_protocol::api::{ModelAction, ModelRequest};
        let hub = Arc::new(Hub::default());
        let config = crate::hub::config(&NewGame {
            players: 2,
            kingdom: vec!["Village".into()],
            seed: Some(1),
            ..Default::default()
        })
        .unwrap();
        let meta = Meta {
            owners: vec![Some("ann".into()), Some("bo".into())],
            ..Default::default()
        };
        let game = hub.host(config, vec!["Ann".into(), "Bo".into()], meta);
        let game = game.unwrap();
        let id = game.game_id.clone();
        let person = (game.seats.iter())
            .map(|s| hub.find(&s.token).unwrap())
            .find(|s| hub.view(s).unwrap().0.decision.is_some())
            .unwrap();
        let who = ["ann", "bo"][person.seat.index()];
        let model = ("u-astra".to_string(), "Astra".to_string());
        hub.attach_model(&id, person.seat.0, who, model, Default::default())
            .unwrap();
        let token = Some(hub.attached().remove(0).0.token);
        let arena = Arena::open(hub.clone(), Default::default()).unwrap();
        let session = Session::new(arena, Duration::from_secs(60), false);
        let text = |result: &CallToolResult| match result.content.first() {
            Some(ContentBlock::Text(content)) => content.text.clone(),
            _ => String::new(),
        };

        let params = play::StateParams {
            token: token.clone(),
        };
        let state = session.state(Parameters(params)).await.unwrap();
        assert!(
            text(&state).contains("No note is wanted now"),
            "{}",
            text(&state)
        );
        let decision = hub.view(&person).unwrap().0.decision.unwrap();
        let step = play::StepParams {
            token: token.clone(),
            decision: decision.id.clone(),
            indices: Some(vec![i64::from(decision.options[0].index)]),
            name: None,
            expect: None,
            timeout_s: Some(0),
        };
        let refused = session.step(Parameters(step)).await.unwrap();
        assert_eq!(refused.is_error, Some(true));
        assert!(text(&refused).contains("you are advising them"));
        assert!(hub.meta(&id).unwrap().remote_agents.iter().all(|a| !a));
        assert_eq!(
            hub.view(&person).unwrap().0.decision.unwrap().id,
            decision.id
        );

        let request = ModelRequest {
            game: id.clone(),
            seat: person.seat.0,
            action: ModelAction::Request,
        };
        hub.model_action(&id, who, None, &request).unwrap();
        let wait = play::WaitParams {
            token: token.clone(),
            timeout_s: Some(1),
        };
        let waited = session.wait(Parameters(wait)).await.unwrap();
        assert!(text(&waited).contains("ADVICE WANTED"), "{}", text(&waited));
        let advise = play::AdviseParams {
            token,
            text: "Village first.".into(),
            timeout_s: Some(0),
        };
        let advised = session.advise(Parameters(advise)).await.unwrap();
        assert!(text(&advised).contains("No note is wanted now"));
        let seat = hub.model_seats(&id, who, None).unwrap().remove(0);
        assert_eq!(seat.notes[0].text, "Village first.");
    }
}
