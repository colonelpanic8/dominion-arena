mod model;
use super::recordings::{Refused, Replays};
use super::{Client, mcp, runtime::Update};
use crate::{arena::Arena, config::Config};
use axum::{
    Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use dominion_online::{
    catalog::Catalog,
    transport::ancillary::{PlayerProfile, ProfileResult},
};
use dominion_protocol::api::{
    OnlineAccountInfo, OnlineAttach, OnlineManagement, OnlineMatching, OnlineMode,
    OnlineObservation, OnlineOpponent, OnlineOutcome, OnlineRating, OnlineRatingPeriod,
    OnlineRatingPoint, OnlineRatingType, OnlineResult, OnlineResultOpponent, OnlineStart, SeatView,
};
use dominion_protocol::lobby::{GameMeta, OfficialAccount, OfficialGame, SeatLevel, UserKind};
use model::{OfficialModel, model_action, model_seats};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// How long a rating read from dominion.games is served before it is asked again.
const RATING_FRESH_MS: u64 = 60_000;
/// How long after a game finishes dominion.games may still be counting it: a rating read
/// before then is served for `RATING_RECHECK_MS` only.
const RATING_SETTLE_MS: u64 = 20_000;
const RATING_RECHECK_MS: u64 = 5_000;

/// One official account kept connected by `dominion serve`.
#[derive(Clone)]
pub struct Hosted {
    pub client: Client,
    /// The account's id in routes ([`super::Account::id`]).
    pub id: String,
    /// Its official-server name, as configured.
    pub account: String,
    /// The registered Arena account that controls it.
    pub owner: String,
    pub(super) control: Arc<tokio::sync::Mutex<()>>,
    model: Arc<std::sync::Mutex<OfficialModel>>,
    ratings: Arc<std::sync::Mutex<HashMap<OnlineRatingType, OnlineRating>>>,
    levels: Arc<tokio::sync::watch::Sender<Levels>>,
    /// Where its games are recorded.
    recordings: Option<PathBuf>,
    /// Where official clients' code is cached (`online_cache_dir`).
    cache: Option<PathBuf>,
    /// The lowest dominion.games level its public searches match.
    min_opponent_level: Option<f64>,
}

/// The dominion.games levels of a game's players by seat, asked of the official server as
/// the game began.
#[derive(Clone, Debug, Default, PartialEq)]
struct Levels {
    game: Option<i64>,
    by_seat: Vec<Option<SeatLevel>>,
}

impl Levels {
    /// Game `game`'s levels, as far as they are known.
    fn of(&self, game: Option<i64>) -> Vec<Option<SeatLevel>> {
        if game.is_some() && self.game == game {
            self.by_seat.clone()
        } else {
            vec![]
        }
    }
}

/// The routes of every hosted account. Each has `/api/online/<id>/...` and
/// `/mcp/online/<id>`; the `online_*` account (else the first) also has the unprefixed
/// `/api/online/...` and `/mcp/online`. `GET /api/online/accounts` lists them.
pub fn router(
    hosted: &[Hosted],
    arena: Arc<Arena>,
    hosts: Vec<String>,
    idle: Option<Duration>,
    stop: CancellationToken,
) -> Router {
    let primary = hosted
        .iter()
        .position(|h| h.id == super::LEGACY_ID)
        .unwrap_or(0);
    let list: Vec<Hosted> = hosted.to_vec();
    let peers: Arc<[Hosted]> = hosted.into();
    let dirs: Vec<PathBuf> = hosted.iter().filter_map(|h| h.recordings.clone()).collect();
    let replays = match hosted.first() {
        Some(first) if !dirs.is_empty() => {
            let catalog = first.client.current().reducer.catalog.clone();
            let replays = Arc::new(Replays::new(dirs, first.cache.clone(), catalog));
            let has = replays.clone();
            arena.set_official_replays(Arc::new(move |id| has.has(id)));
            Router::new()
                .route("/api/online/replays/{id}", get(replay))
                .with_state(replays)
        }
        _ => Router::new(),
    };
    let mut router = replays
        .route(
            "/api/online/accounts",
            get(move || {
                let list = list.clone();
                async move {
                    axum::Json(
                        list.iter()
                            .enumerate()
                            .map(|(i, h)| h.info(i == primary))
                            .collect::<Vec<_>>(),
                    )
                }
            }),
        )
        .layer(middleware::from_fn(no_cache))
        .layer(middleware::from_fn_with_state(
            Arc::new(hosts.clone()),
            crate::http::check_host,
        ));
    for (i, h) in hosted.iter().enumerate() {
        router = router.merge(h.router(
            arena.clone(),
            peers.clone(),
            hosts.clone(),
            idle,
            stop.clone(),
            i == primary,
        ));
    }
    router
}

impl Hosted {
    pub(crate) fn new(client: Client, id: String, account: String, owner: String) -> Self {
        Self {
            client,
            id,
            account,
            owner,
            control: Arc::new(tokio::sync::Mutex::new(())),
            model: Arc::default(),
            ratings: Arc::default(),
            levels: Arc::new(tokio::sync::watch::Sender::new(Levels::default())),
            recordings: None,
            cache: None,
            min_opponent_level: None,
        }
    }

    /// The account records its games in `recordings`; official clients' code is cached
    /// in `cache`.
    pub(crate) fn recording(mut self, recordings: PathBuf, cache: Option<PathBuf>) -> Self {
        self.recordings = Some(recordings);
        self.cache = cache;
        self
    }

    pub(crate) fn min_opponent_level(mut self, level: Option<f64>) -> Self {
        self.min_opponent_level = level;
        self
    }

    /// The levels of `update`'s game's players.
    fn levels(&self, update: &Update) -> Vec<Option<SeatLevel>> {
        self.levels.borrow().of(update.reducer.game_id)
    }

    /// The account's rating and recent results on dominion.games. They are asked of the
    /// official server at most once a minute per rating type, and again soon after a game
    /// of the account's finishes.
    pub async fn rating(&self, kind: OnlineRatingType) -> Result<OnlineRating, String> {
        let finished = self.client.updates.borrow().finished_at;
        let cached = crate::hub::lock(&self.ratings).get(&kind).cloned();
        if let Some(rating) = cached.filter(|r| fresh(r.fetched, finished, crate::hub::now_ms())) {
            return Ok(rating);
        }
        let (profile, results) = tokio::join!(
            self.client.profile(kind.ordinal()),
            self.client.results(kind.ordinal())
        );
        let catalog = self.client.updates.borrow().reducer.catalog.clone();
        let rating = rating_of(&profile?, &results?, &catalog, kind, crate::hub::now_ms());
        crate::hub::lock(&self.ratings).insert(kind, rating.clone());
        Ok(rating)
    }

    /// Logs in every account `dominion serve` keeps. An account that fails to log in is
    /// reported and left out, unless none succeeds; a settings error stops them all.
    pub async fn start_all(config: &Config) -> Result<Vec<Self>, String> {
        let accounts: Vec<_> = config
            .online_accounts()?
            .into_iter()
            .filter(|a| a.serve)
            .collect();
        let mut owners = Vec::new();
        for account in &accounts {
            let owner = account
                .owner
                .clone()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "official account {} needs its owner (a registered Arena account name: online_owner, or owner in online_accounts)",
                        account.id
                    )
                })?;
            if !account.exclusive {
                return Err(format!(
                    "REFUSED: official account {}: set exclusive only while the account is idle and exclusive to this client",
                    account.id
                ));
            }
            owners.push(owner);
        }
        let mut hosted = Vec::new();
        let mut errors = Vec::new();
        for (account, owner) in accounts.into_iter().zip(owners) {
            match super::start(config, &account).await {
                Ok(client) => hosted.push(
                    Self::new(client, account.id, account.account, owner)
                        .recording(
                            super::recordings::dir(&account.state_dir),
                            config.online_cache_dir(),
                        )
                        .min_opponent_level(account.min_opponent_level),
                ),
                Err(e) => {
                    eprintln!("dominion: official account {}: {e}", account.id);
                    errors.push(format!("{}: {e}", account.id));
                }
            }
        }
        if hosted.is_empty() && !errors.is_empty() {
            return Err(errors.join("; "));
        }
        Ok(hosted)
    }

    fn info(&self, primary: bool) -> OnlineAccountInfo {
        let update = self.client.current();
        OnlineAccountInfo {
            id: self.id.clone(),
            account: self.account.clone(),
            primary,
            connected: update.connected,
            running: update.reducer.running,
        }
    }

    fn router(
        &self,
        arena: Arc<Arena>,
        peers: Arc<[Hosted]>,
        hosts: Vec<String>,
        idle: Option<Duration>,
        stop: CancellationToken,
        primary: bool,
    ) -> Router {
        let hosted = self.clone();
        let mut sessions = LocalSessionManager::default();
        sessions.session_config.keep_alive = idle;
        let service = StreamableHttpService::new(
            move || Ok(mcp::Session::managed(hosted.clone())),
            Arc::new(sessions),
            StreamableHttpServerConfig::default()
                .with_allowed_hosts(hosts.clone())
                .with_cancellation_token(stop.child_token()),
        );
        let auth = Access {
            arena,
            owner: self.owner.clone(),
            id: self.id.clone(),
        };
        let lifecycle_web = Web {
            hosted: self.clone(),
            access: auth.clone(),
            peers: peers.clone(),
            stop: stop.clone(),
        };
        let mut updates = self.client.updates.clone();
        let lifecycle_stop = stop.clone();
        tokio::spawn(async move {
            let web = lifecycle_web;
            let update = updates.borrow_and_update().clone();
            web.ask_levels(&update);
            web.publish(&update);
            loop {
                tokio::select! {_=lifecycle_stop.cancelled()=>break,changed=updates.changed()=>if changed.is_err(){break;}}
                let _guard = web.hosted.control.lock().await;
                let update = updates.borrow_and_update().clone();
                web.ask_levels(&update);
                web.publish(&update);
                if update.error.is_some() || !update.connected {
                    web.set_job(None);
                } else if update.reducer.running {
                    if let Some(job) = web.job()
                        && job.game_id.is_empty()
                        && let Err(e) = assign(&web, job.agent, job.strategy)
                    {
                        eprintln!(
                            "dominion: official account {}: assigning matched game: {e}",
                            web.hosted.id
                        );
                    }
                } else if !update.searching && web.job().is_some_and(|j| !j.game_id.is_empty()) {
                    web.set_job(None);
                }
            }
            web.set_job(None);
        });
        let api = Router::new()
            .route("/view", get(snapshot))
            .route("/events", get(events))
            .route("/manage", get(manage))
            .route("/rating", get(rating))
            .route("/start", post(start_game))
            .route("/model", get(model_seats).post(model_action))
            .route("/agent", post(attach_agent).delete(pause_agent))
            .route("/matchmaking", axum::routing::delete(cancel_matchmaking))
            .route("/reconnect", post(reconnect))
            .route("/resign", post(resign))
            .route("/chat", post(say))
            .route("/transcript", post(publish_transcript))
            .with_state(Web {
                hosted: self.clone(),
                access: auth.clone(),
                peers,
                stop,
            });
        let mut router = Router::new()
            .nest(&format!("/api/online/{}", self.id), api.clone())
            .route_service(&format!("/mcp/online/{}", self.id), service.clone());
        if primary {
            router = router
                .nest("/api/online", api)
                .route_service("/mcp/online", service);
        }
        router
            .layer(middleware::from_fn_with_state(auth, authorize))
            .layer(middleware::from_fn(no_cache))
            .layer(middleware::from_fn_with_state(
                Arc::new(hosts),
                crate::http::check_host,
            ))
    }
}
#[derive(Clone)]
struct Access {
    arena: Arc<Arena>,
    owner: String,
    /// The official account these routes serve.
    id: String,
}
impl Access {
    fn allowed(&self, headers: &HeaderMap) -> Result<(), StatusCode> {
        let token = crate::http::bearer(headers)
            .or_else(|| crate::http::cookie(headers))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let ident = self.arena.identify(token).ok_or(StatusCode::UNAUTHORIZED)?;
        let me = self
            .arena
            .me(&ident)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        if (me.user.kind == UserKind::Registered && me.user.name == self.owner)
            || (me.user.kind == UserKind::Bot && me.user.owner.as_deref() == Some(&self.owner))
        {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}
async fn authorize(State(access): State<Access>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let watching = request.method() == axum::http::Method::GET
        && path.starts_with("/api/online/")
        && (path.ends_with("/view") || path.ends_with("/events") || path.ends_with("/rating"));
    // Only this account's job token opens this account's MCP endpoint, or publishes its
    // transcript.
    let runner = (path.starts_with("/mcp/online")
        || (request.method() == axum::http::Method::POST
            && path.starts_with("/api/online/")
            && path.ends_with("/transcript")))
        && crate::http::bearer(request.headers()).is_some_and(|token| {
            access
                .arena
                .official_job(&access.id)
                .is_some_and(|job| !job.token.is_empty() && job.token == token)
        });
    let model = path.starts_with("/api/online/") && path.ends_with("/model");
    if !model
        && !watching
        && !runner
        && let Err(status) = access.allowed(request.headers())
    {
        return (
            status,
            axum::Json(dominion_protocol::api::ApiError {
                error: "sign in as the configured official-game owner".into(),
            }),
        )
            .into_response();
    }
    if request.uri().path().starts_with("/api/online/")
        && request.method() != axum::http::Method::GET
        && let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|h| h.to_str().ok())
    {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let origin_host = origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"))
            .unwrap_or("")
            .trim_end_matches('/');
        if origin_host != host {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    next.run(request).await
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplayQuery {
    step: usize,
    #[serde(default)]
    log_from: usize,
}

/// A recorded game at a step, as its account saw it.
async fn replay(
    State(replays): State<Arc<Replays>>,
    Path(id): Path<String>,
    Query(q): Query<ReplayQuery>,
) -> Response {
    let shown = tokio::task::spawn_blocking(move || replays.view(&id, q.step, q.log_from)).await;
    let (status, error) = match shown {
        Ok(Ok(view)) => return axum::Json(view).into_response(),
        Ok(Err(Refused::Missing(e))) => (StatusCode::NOT_FOUND, e),
        Ok(Err(Refused::BadRequest(e))) => (StatusCode::BAD_REQUEST, e),
        Ok(Err(Refused::Failed(e))) => (StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    (
        status,
        axum::Json(dominion_protocol::api::ApiError { error }),
    )
        .into_response()
}
async fn no_cache(request: Request, next: Next) -> Response {
    let mut r = next.run(request).await;
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    r
}
/// What anyone may watch: the account's view without its decision.
fn observation(update: Update, ratings: Vec<Option<SeatLevel>>) -> OnlineObservation {
    let now = crate::hub::now_ms();
    let (clock, _) = update.clock_observation_at(now, Some(std::time::Instant::now()));
    let game = update.reducer.view().ok().map(|mut view| {
        let deciding = view.decision.take().and(view.seat);
        view.spectating = true;
        SeatView {
            view,
            version: update.reducer.revision,
            deciding,
            table: update.reducer.table().unwrap_or_default(),
            undos: update.reducer.epoch,
            can_undo: false,
            undo: Default::default(),
            meta: GameMeta {
                rated: update.mode == OnlineMode::Rated,
                watchable: true,
                timed: clock.is_some(),
                clock,
                now,
                levels: ratings.iter().map(|r| r.map(|r| r.level)).collect(),
                online_ratings: ratings,
                ..Default::default()
            },
            replay: None,
        }
    });
    OnlineObservation {
        connected: update.connected,
        error: update.error,
        game,
        chat: update.chat.lines,
    }
}
#[derive(Clone)]
struct Web {
    hosted: Hosted,
    access: Access,
    /// Every account this server hosts, this one included.
    peers: Arc<[Hosted]>,
    stop: CancellationToken,
}
async fn snapshot(State(web): State<Web>) -> axum::Json<OnlineObservation> {
    let update = web.hosted.client.current();
    let levels = web.hosted.levels(&update);
    axum::Json(observation(update, levels))
}
async fn events(
    State(web): State<Web>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let updates = web.hosted.client.updates.clone();
    let levels = web.hosted.levels.subscribe();
    let stream = futures_util::stream::unfold(
        (updates, levels, true, web),
        |(mut updates, mut levels, first, web)| async move {
            if web.stop.is_cancelled() {
                return None;
            }
            if !first {
                tokio::select! {
                    _=web.stop.cancelled()=>return None,
                    changed=updates.changed()=>if changed.is_err(){return None;},
                    changed=levels.changed()=>if changed.is_err(){return None;},
                }
            }
            let update = updates.borrow_and_update().clone();
            let shown = levels.borrow_and_update().of(update.reducer.game_id);
            let event = Event::default()
                .event("change")
                .json_data(observation(update, shown))
                .expect("serializable observation");
            Some((Ok(event), (updates, levels, false, web)))
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::new())
}
impl Web {
    /// Asks the official server once for the levels of the players of `update`'s game, if
    /// it is running, and publishes them when they come.
    fn ask_levels(&self, update: &Update) {
        let Some(game) = update.reducer.game_id.filter(|_| update.reducer.running) else {
            return;
        };
        let Ok(state) = update.reducer.state() else {
            return;
        };
        let players: Vec<i32> = state.players.iter().map(|p| p.player_id).collect();
        let fresh = self.hosted.levels.send_if_modified(|levels| {
            if levels.game == Some(game) {
                return false;
            }
            *levels = Levels {
                game: Some(game),
                by_seat: vec![None; players.len()],
            };
            true
        });
        if !fresh {
            return;
        }
        let kind = level_type(players.len());
        let bots = update.bots.clone();
        let catalog = update.reducer.catalog.clone();
        let web = self.clone();
        tokio::spawn(async move {
            let asked = players.iter().map(|&player| {
                let client = web.hosted.client.clone();
                let catalog = catalog.clone();
                let bot = bots.contains(&player);
                async move {
                    if bot {
                        return None;
                    }
                    let profile = client.player_profile(player, kind.ordinal()).await.ok()?;
                    let rating = rating_of(&profile, &[], &catalog, kind, 0);
                    (rating.games > 0).then_some(SeatLevel {
                        level: rating.level,
                        skill: rating.skill,
                        deviation: rating.deviation,
                        games: rating.games,
                        rank: rating.rank,
                        wins: rating.wins,
                        losses: rating.losses,
                        ties: rating.ties,
                    })
                }
            });
            let by_seat = futures_util::future::join_all(asked).await;
            let changed = web.hosted.levels.send_if_modified(|levels| {
                let current = levels.game == Some(game) && levels.by_seat != by_seat;
                if current {
                    levels.by_seat = by_seat;
                }
                current
            });
            if changed {
                let _guard = web.hosted.control.lock().await;
                web.publish(&web.hosted.client.current());
            }
        });
    }

    fn job(&self) -> Option<crate::house::Job> {
        self.access.arena.official_job(&self.hosted.id)
    }
    fn set_job(&self, job: Option<crate::house::Job>) {
        self.access.arena.set_official_job(&self.hosted.id, job);
    }
    /// Tells the lobby how the account stands.
    fn publish(&self, update: &Update) {
        let view = update.view().ok().flatten();
        let game = view.as_ref().and_then(|v| {
            let mut players: Vec<_> = v.you.iter().chain(&v.opponents).collect();
            players.sort_by_key(|p| p.index);
            Some(OfficialGame {
                id: v.game.id.clone()?,
                running: v.game.running,
                rated: update.mode == OnlineMode::Rated,
                players: players.iter().map(|p| p.name.clone()).collect(),
                seat: v.seat,
                kingdom: v.game.kingdom.clone(),
                landscapes: v.game.landscapes.clone(),
                levels: (self.hosted.levels(update).iter())
                    .map(|r| r.map(|r| r.level))
                    .collect(),
                others: crate::setup_cards::setup_cards(v),
            })
        });
        let info = OfficialAccount {
            id: self.hosted.id.clone(),
            account: self.hosted.account.clone(),
            owner: self.hosted.owner.clone(),
            connected: update.connected,
            searching: update.searching,
            agent: None,
            game,
        };
        let result = view.as_ref().and_then(|v| v.game.result.as_ref());
        self.access.arena.set_official(info, result);
    }
}
impl std::fmt::Debug for Hosted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosted")
            .field("id", &self.id)
            .field("account", &self.account)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

fn management(web: &Web) -> OnlineManagement {
    let update = web.hosted.client.current();
    let view = update.reducer.view().ok();
    let (agents, models) = web.access.arena.official_agents();
    OnlineManagement {
        strategy: web
            .job()
            .map(|j| j.strategy)
            .or_else(|| {
                let game = &view.as_ref()?.game;
                if !game.running {
                    return None;
                }
                web.access
                    .arena
                    .official_game_strategy(&web.hosted.id, game.id.as_deref()?)
            })
            .unwrap_or_default(),
        connected: update.connected,
        error: update.error,
        id: web.hosted.id.clone(),
        account: update.reducer.names.get(&update.reducer.player_id).cloned(),
        agents,
        models,
        agent: web.job().map(|j| j.agent),
        game_agent: (view.as_ref())
            .filter(|v| v.game.running)
            .and_then(|v| v.game.id.as_deref())
            .and_then(|game| web.access.arena.official_game_agent(&web.hosted.id, game)),
        running: view.is_some_and(|v| v.game.running),
        searching: update.connected
            && (update.searching || web.job().is_some_and(|j| j.game_id.is_empty())),
        queue_feedback: update.queue_feedback,
        mode: update.mode,
        min_opponent_level: web.hosted.min_opponent_level,
        paired: update.paired,
    }
}
async fn manage(State(web): State<Web>) -> axum::Json<OnlineManagement> {
    axum::Json(management(&web))
}
/// Raises `matching`'s relative range so it matches no one below level `floor`, the
/// account being at level `own`.
pub(super) fn raise_to_floor(matching: &mut OnlineMatching, floor: f64, own: f64) {
    let lowest = ((floor - own).ceil() as i32).clamp(-100, 100);
    matching.min_rating = matching.min_rating.max(lowest);
    matching.max_rating = matching.max_rating.max(matching.min_rating);
}
/// The rating dominion.games rates a game of `players` players under.
fn level_type(players: usize) -> OnlineRatingType {
    if players <= 2 {
        OnlineRatingType::TwoPlayer
    } else {
        OnlineRatingType::ThreePlayer
    }
}
/// Whether a rating read at `fetched` can still be served at `now`, the account's last game
/// having finished at `finished`.
pub(super) fn fresh(fetched: u64, finished: Option<u64>, now: u64) -> bool {
    let age = now.saturating_sub(fetched);
    let settled = finished.is_none_or(|f| fetched >= f + RATING_SETTLE_MS);
    age < if settled {
        RATING_FRESH_MS
    } else {
        RATING_RECHECK_MS
    }
}
pub(super) fn rating_of(
    profile: &PlayerProfile,
    results: &[ProfileResult],
    catalog: &Catalog,
    kind: OnlineRatingType,
    fetched: u64,
) -> OnlineRating {
    let count = |n: i32| u32::try_from(n).unwrap_or(0);
    let rank = |n: i32| u32::try_from(n).ok().filter(|&n| n > 0);
    let date = |ms: i64| (ms > 0).then_some(ms);
    let summary = &profile.summary;
    let mut history: Vec<_> = (profile.rating_history.iter())
        .map(|p| OnlineRatingPoint {
            date: p.date_millis,
            level: p.level,
            skill: p.skill,
            deviation: p.deviation,
            games: count(p.games),
        })
        .collect();
    history.sort_by_key(|p| p.date);
    let mut level = summary.current_level;
    let mut skill = summary.current_skill;
    let mut deviation = summary.current_deviation;
    let mut games = history.last().map_or(0, |p| p.games);
    // The official player page's rule: the open period has a rating of its own once it has
    // results the history doesn't hold yet.
    let p = &profile.period;
    let open = p.available
        && p.results > 0
        && p.period_date_millis > 0
        && history.last().is_none_or(|h| h.date < p.period_date_millis);
    if open {
        (level, skill, deviation, games) = (
            p.new_level,
            p.new_skill,
            p.new_deviation,
            count(p.new_games),
        );
        history.push(OnlineRatingPoint {
            date: p.period_date_millis,
            level,
            skill,
            deviation,
            games,
        });
    }
    OnlineRating {
        account: profile.player.name.clone(),
        rating_type: kind,
        level,
        skill,
        deviation,
        games,
        provisional: open,
        period: open.then(|| OnlineRatingPeriod {
            start: p.period_start_millis,
            results: count(p.results),
            distinct_opponents: count(p.distinct_opponents),
            score: p.score,
            expected_score: p.expected_score,
            start_level: p.start_level,
            start_skill: p.start_skill,
            start_deviation: p.start_deviation,
        }),
        rank: rank(summary.current_rank),
        wins: count(summary.wins),
        losses: count(summary.losses),
        ties: count(summary.ties),
        best_level: summary.best_level,
        best_level_date: date(summary.best_level_date_millis),
        best_rank: rank(summary.best_rank),
        best_rank_date: date(summary.best_rank_date_millis),
        best_win_streak: count(summary.best_win_streak),
        best_loss_streak: count(summary.best_loss_streak),
        country: profile.country.clone(),
        title: profile.title.clone(),
        history,
        results: results.iter().map(|r| result_of(r, catalog)).collect(),
        fetched,
    }
}
pub(super) fn result_of(result: &ProfileResult, catalog: &Catalog) -> OnlineResult {
    let rank = |n: i32| u32::try_from(n).unwrap_or(0);
    let opponents = &result.opponents;
    let outcome = if opponents.iter().all(|o| result.my_rank < o.rank) {
        OnlineOutcome::Win
    } else if opponents.iter().all(|o| result.my_rank <= o.rank) {
        OnlineOutcome::Tie
    } else {
        OnlineOutcome::Loss
    };
    let mut kingdom: Vec<_> = (result.kingdom.iter())
        .filter_map(|&id| catalog.card(id).ok().map(|c| c.name.clone()))
        .collect();
    kingdom.sort();
    OnlineResult {
        game: result.game_id,
        time: result.time_millis,
        rated: result.rated,
        outcome,
        rank: rank(result.my_rank),
        score: result.my_score,
        resigned: result.my_resigned,
        level: result.my_level,
        opponent_level: result.opponent_level,
        expected_score: result.expected_score,
        opponents: (opponents.iter())
            .map(|o| OnlineResultOpponent {
                name: o.player.name.clone(),
                rank: rank(o.rank),
                score: o.score,
                resigned: o.resigned,
            })
            .collect(),
        kingdom,
    }
}
#[derive(serde::Deserialize)]
struct RatingQuery {
    #[serde(rename = "type", default)]
    kind: OnlineRatingType,
}
async fn rating(State(web): State<Web>, Query(query): Query<RatingQuery>) -> Response {
    match web.hosted.rating(query.kind).await {
        Ok(rating) => axum::Json(rating).into_response(),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(dominion_protocol::api::ApiError { error }),
        )
            .into_response(),
    }
}
fn refused(error: impl ToString) -> Response {
    (
        StatusCode::CONFLICT,
        axum::Json(dominion_protocol::api::ApiError {
            error: error.to_string(),
        }),
    )
        .into_response()
}
fn assign(
    web: &Web,
    agent: String,
    strategy: dominion_protocol::lobby::RunnerStrategy,
) -> Result<(), String> {
    let view = web
        .hosted
        .client
        .current()
        .view()?
        .ok_or("no official game")?;
    if !view.game.running {
        return Err("no running official game".into());
    }
    let game_id = view.game.id.ok_or("game has no id")?;
    if let Some(playing) = (web.access.arena).official_game_agent(&web.hosted.id, &game_id)
        && playing != agent
    {
        return Err(format!(
            "{playing} plays this game: its agent can't be changed mid-game"
        ));
    }
    if web
        .access
        .arena
        .official_game_strategy(&web.hosted.id, &game_id)
        .is_some_and(|s| s != strategy)
    {
        return Err("the running game must resume with the same strategy".into());
    }
    web.hosted.check_assignment(&game_id)?;
    web.set_job(Some(crate::house::Job {
        strategy,
        official: true,
        official_account: Some(web.hosted.id.clone()),
        transcript: true,
        configuration: None,
        agent,
        brief: None,
        debrief: None,
        game_id,
        seat: view.seat.ok_or("no playing seat")?,
        name: view.you.ok_or("no playing account")?.name,
        token: format!(
            "{:032x}{:032x}",
            rand::random::<u128>(),
            rand::random::<u128>()
        ),
    }));
    Ok(())
}
async fn start_game(
    State(web): State<Web>,
    headers: HeaderMap,
    axum::Json(start): axum::Json<OnlineStart>,
) -> Response {
    let _guard = web.hosted.control.lock().await;
    if let Some(against) = start.against.clone() {
        // Run to the end even if the request goes away, so no table is left half made.
        let web = web.clone();
        let paired =
            tokio::spawn(async move { start_paired(&web, &headers, start, against).await });
        return paired.await.unwrap_or_else(refused);
    }
    if start.mode == OnlineMode::Bots && !(1..=5).contains(&start.bots) {
        return refused("choose 1–5 server bots");
    }
    let reservation = match web.access.arena.reserve_official(
        &web.hosted.owner,
        &web.hosted.id,
        &start.agent,
        start.strategy,
    ) {
        Ok(r) => r,
        Err(e) => return refused(e),
    };
    if start.mode != OnlineMode::Bots {
        if !start.kingdom.is_empty() || !start.landscapes.is_empty() {
            return refused("public matchmaking does not accept required cards");
        }
        let current = management(&web);
        if let Some(playing) = current.game_agent
            && (playing != start.agent || current.strategy != start.strategy)
        {
            return refused(format!(
                "{playing} plays the running game: its agent and strategy are fixed"
            ));
        }
        let mut matching = start.matching;
        if let Some(floor) = web.hosted.min_opponent_level {
            let kind = level_type(usize::from(matching.players));
            let own = match web.hosted.rating(kind).await {
                Ok(rating) => rating.level,
                Err(e) => return refused(format!("reading the account's level: {e}")),
            };
            raise_to_floor(&mut matching, floor, own);
        }
        if let Err(e) = web.hosted.client.matchmaking(start.mode, matching).await {
            return refused(e);
        }
        let update = web.hosted.client.current();
        if update.reducer.running {
            if let Err(e) = assign(&web, start.agent, start.strategy) {
                return refused(e);
            }
        } else if update.searching {
            web.set_job(Some(crate::house::Job {
                strategy: start.strategy,
                official: true,
                official_account: Some(web.hosted.id.clone()),
                transcript: true,
                configuration: None,
                agent: start.agent,
                brief: None,
                debrief: None,
                game_id: String::new(),
                seat: 0,
                name: web.hosted.owner.clone(),
                token: String::new(),
            }));
        }
        drop(reservation);
        return axum::Json(management(&web)).into_response();
    }
    let options = super::NewGame {
        players: Some(start.bots + 1),
        bots: Some(start.bots),
        kingdom: start.kingdom,
        landscapes: start.landscapes,
    };
    if let Err(e) = web.hosted.client.new_game(options).await {
        return refused(e);
    }
    if let Err(e) = assign(&web, start.agent, start.strategy) {
        return refused(e);
    }
    drop(reservation);
    axum::Json(management(&web)).into_response()
}
/// How long a game against another hosted account waits for that account's control.
const PEER_BUSY: Duration = Duration::from_secs(5);

/// Starts a private unrated game between this account and `against`, another account this
/// server hosts that the caller also owns, each played by its agent.
async fn start_paired(
    web: &Web,
    headers: &HeaderMap,
    start: OnlineStart,
    against: OnlineOpponent,
) -> Response {
    if start.mode != OnlineMode::Unrated || start.matching.players != 2 {
        return refused("a game against another hosted account is unrated with two players");
    }
    let Some(peer) = web.peers.iter().find(|h| h.id == against.account).cloned() else {
        return refused(format!("no hosted official account {}", against.account));
    };
    if peer.id == web.hosted.id {
        return refused("choose another account to play against");
    }
    let peer = Web {
        access: Access {
            arena: web.access.arena.clone(),
            owner: peer.owner.clone(),
            id: peer.id.clone(),
        },
        hosted: peer,
        peers: web.peers.clone(),
        stop: web.stop.clone(),
    };
    if peer.access.allowed(headers).is_err() {
        return refused(format!("you don't own {}", peer.hosted.account));
    }
    let Ok(_peer_guard) = tokio::time::timeout(PEER_BUSY, peer.hosted.control.lock()).await else {
        return refused(format!("{} is busy", peer.hosted.account));
    };
    let arena = &web.access.arena;
    let reservations = (
        arena.reserve_official(
            &web.hosted.owner,
            &web.hosted.id,
            &start.agent,
            start.strategy,
        ),
        arena.reserve_official(
            &peer.hosted.owner,
            &peer.hosted.id,
            &against.agent,
            against.strategy,
        ),
    );
    let (Ok(_host_seat), Ok(_peer_seat)) = reservations else {
        let (a, b) = reservations;
        return refused(a.err().or(b.err()).expect("a reservation failed"));
    };
    let (host, guest) = (web.hosted.client.current(), peer.hosted.client.current());
    // `pair` checks these too; here they name the account.
    for (update, account) in [(&host, &web.hosted.account), (&guest, &peer.hosted.account)] {
        if !update.connected || update.error.is_some() {
            return refused(format!("{account} is not connected"));
        }
        if update.reducer.running || update.searching {
            return refused(format!("{account} is already playing or searching"));
        }
    }
    let options = super::PairTable {
        kingdom: start.kingdom,
        landscapes: start.landscapes,
        timer: start.matching.timer,
    };
    let paired = super::pair(&web.hosted.client, &peer.hosted.client, options).await;
    if let Err(e) = paired {
        return refused(e);
    }
    if let Err(e) = assign(web, start.agent, start.strategy) {
        return refused(e);
    }
    if let Err(e) = assign(&peer, against.agent, against.strategy) {
        return refused(e);
    }
    axum::Json(management(web)).into_response()
}
async fn attach_agent(
    State(web): State<Web>,
    axum::Json(body): axum::Json<OnlineAttach>,
) -> Response {
    let _guard = web.hosted.control.lock().await;
    let strategy = body.strategy.unwrap_or_else(|| management(&web).strategy);
    let reservation = match web.access.arena.reserve_official(
        &web.hosted.owner,
        &web.hosted.id,
        &body.agent,
        strategy,
    ) {
        Ok(r) => r,
        Err(e) => return refused(e),
    };
    if let Err(e) = assign(&web, body.agent, strategy) {
        return refused(e);
    }
    drop(reservation);
    axum::Json(management(&web)).into_response()
}
async fn pause_agent(State(web): State<Web>) -> Response {
    let _guard = web.hosted.control.lock().await;
    if management(&web).searching {
        return refused("cancel matchmaking before pausing the assigned agent");
    }
    web.set_job(None);
    axum::Json(management(&web)).into_response()
}
async fn reconnect(State(web): State<Web>) -> Response {
    let _guard = web.hosted.control.lock().await;
    if management(&web).searching {
        return refused("cancel matchmaking before reconnecting");
    }
    web.set_job(None);
    match web.hosted.client.reconnect().await {
        Ok(()) => axum::Json(management(&web)).into_response(),
        Err(e) => refused(e),
    }
}
async fn resign(State(web): State<Web>) -> Response {
    let _guard = web.hosted.control.lock().await;
    match web.hosted.client.resign().await {
        Ok(()) => {
            web.set_job(None);
            axum::Json(management(&web)).into_response()
        }
        Err(e) => refused(e),
    }
}
/// Its owner says a line at the account's table on dominion.games.
async fn say(
    State(web): State<Web>,
    axum::Json(say): axum::Json<dominion_protocol::lobby::Say>,
) -> Response {
    let text = match crate::arena::chat_line(&say.text) {
        Ok(text) => text,
        Err(e) => return crate::http::Error::from(e).into_response(),
    };
    match web.hosted.client.say(text).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => refused(e),
    }
}

/// The agent playing as the account adds to its transcript, with its job's token.
async fn publish_transcript(
    State(web): State<Web>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(job) = web.job().filter(|job| {
        !job.token.is_empty() && crate::http::bearer(&headers) == Some(job.token.as_str())
    }) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let input: dominion_protocol::transcript::TranscriptInput = match serde_json::from_slice(&body)
    {
        Ok(input) => input,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let arena = web.access.arena.clone();
    match tokio::task::spawn_blocking(move || arena.append_official_transcript(&job, &input)).await
    {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => crate::http::Error::from(e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn cancel_matchmaking(State(web): State<Web>) -> Response {
    let _guard = web.hosted.control.lock().await;
    match web.hosted.client.cancel_matchmaking().await {
        Ok(()) => {
            let update = web.hosted.client.current();
            if update.reducer.running {
                if let Some(job) = web.job()
                    && job.game_id.is_empty()
                    && let Err(e) = assign(&web, job.agent, job.strategy)
                {
                    return refused(e);
                }
            } else if !update.searching {
                web.set_job(None);
            }
            axum::Json(management(&web)).into_response()
        }
        Err(e) => refused(e),
    }
}
