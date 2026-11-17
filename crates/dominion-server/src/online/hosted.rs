use super::{Client, mcp, runtime::Update};
use crate::{arena::Arena, config::Config};
use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use dominion_protocol::api::{
    OnlineAccountInfo, OnlineAttach, OnlineManagement, OnlineMode, OnlineObservation, OnlineStart,
};
use dominion_protocol::lobby::UserKind;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

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
    control: Arc<tokio::sync::Mutex<()>>,
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
    let mut router = Router::new()
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
        }
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
                Ok(client) => hosted.push(Self::new(client, account.id, account.account, owner)),
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
        hosts: Vec<String>,
        idle: Option<Duration>,
        stop: CancellationToken,
        primary: bool,
    ) -> Router {
        let client = self.client.clone();
        let mut sessions = LocalSessionManager::default();
        sessions.session_config.keep_alive = idle;
        let service = StreamableHttpService::new(
            move || Ok(mcp::Session::managed(client.clone())),
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
            stop: stop.clone(),
        };
        let mut updates = self.client.updates.clone();
        let lifecycle_stop = stop.clone();
        tokio::spawn(async move {
            let web = lifecycle_web;
            loop {
                tokio::select! {_=lifecycle_stop.cancelled()=>break,changed=updates.changed()=>if changed.is_err(){break;}}
                let _guard = web.hosted.control.lock().await;
                let update = updates.borrow_and_update().clone();
                if update.error.is_some() || !update.connected {
                    web.set_job(None);
                } else if update.reducer.running {
                    if let Some(job) = web.job()
                        && job.game_id.is_empty()
                        && let Err(e) = assign(&web, job.agent)
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
            .route("/start", post(start_game))
            .route("/agent", post(attach_agent).delete(pause_agent))
            .route("/matchmaking", axum::routing::delete(cancel_matchmaking))
            .route("/reconnect", post(reconnect))
            .route("/resign", post(resign))
            .with_state(Web {
                hosted: self.clone(),
                access: auth.clone(),
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
        && (path.ends_with("/view") || path.ends_with("/events"));
    // Only this account's job token opens this account's MCP endpoint.
    let runner = path.starts_with("/mcp/online")
        && crate::http::bearer(request.headers()).is_some_and(|token| {
            access
                .arena
                .official_job(&access.id)
                .is_some_and(|job| !job.token.is_empty() && job.token == token)
        });
    if !watching
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
async fn no_cache(request: Request, next: Next) -> Response {
    let mut r = next.run(request).await;
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    r
}
fn observation(update: Update) -> OnlineObservation {
    let mut view = update.reducer.view().ok();
    if let Some(view) = &mut view {
        view.decision = None;
        view.spectating = true;
    }
    OnlineObservation {
        connected: update.connected,
        error: update.error,
        version: update.reducer.revision,
        epoch: update.reducer.epoch,
        table: update.reducer.table().unwrap_or_default(),
        view,
    }
}
#[derive(Clone)]
struct Web {
    hosted: Hosted,
    access: Access,
    stop: CancellationToken,
}
async fn snapshot(State(web): State<Web>) -> axum::Json<OnlineObservation> {
    axum::Json(observation(web.hosted.client.current()))
}
async fn events(
    State(web): State<Web>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let updates = web.hosted.client.updates.clone();
    let stream = futures_util::stream::unfold(
        (updates, true, web),
        |(mut updates, first, web)| async move {
            if web.stop.is_cancelled() {
                return None;
            }
            if !first {
                tokio::select! {
                    _=web.stop.cancelled()=>return None,
                    changed=updates.changed()=>if changed.is_err(){return None;},
                }
            }
            let update = updates.borrow_and_update().clone();
            let event = Event::default()
                .event("change")
                .json_data(observation(update))
                .expect("serializable observation");
            Some((Ok(event), (updates, false, web)))
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::new())
}
impl Web {
    fn job(&self) -> Option<crate::house::Job> {
        self.access.arena.official_job(&self.hosted.id)
    }
    fn set_job(&self, job: Option<crate::house::Job>) {
        self.access.arena.set_official_job(&self.hosted.id, job);
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
    OnlineManagement {
        connected: update.connected,
        error: update.error,
        id: web.hosted.id.clone(),
        account: update.reducer.names.get(&update.reducer.player_id).cloned(),
        agents: web.access.arena.official_agents(),
        agent: web.job().map(|j| j.agent),
        running: view.is_some_and(|v| v.game.running),
        searching: update.searching || web.job().is_some_and(|j| j.game_id.is_empty()),
        queue_feedback: update.queue_feedback,
        mode: update.mode,
    }
}
async fn manage(State(web): State<Web>) -> axum::Json<OnlineManagement> {
    axum::Json(management(&web))
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
fn assign(web: &Web, agent: String) -> Result<(), String> {
    let view = web
        .hosted
        .client
        .current()
        .view()?
        .ok_or("no official game")?;
    if !view.game.running {
        return Err("no running official game".into());
    }
    web.set_job(Some(crate::house::Job {
        official: true,
        official_account: Some(web.hosted.id.clone()),
        transcript: false,
        configuration: None,
        agent,
        game_id: view.game.id.ok_or("game has no id")?,
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
    axum::Json(start): axum::Json<OnlineStart>,
) -> Response {
    let _guard = web.hosted.control.lock().await;
    if start.mode == OnlineMode::Bots && !(1..=5).contains(&start.bots) {
        return refused("choose 1–5 server bots");
    }
    let reservation =
        match web
            .access
            .arena
            .reserve_official(&web.hosted.owner, &web.hosted.id, &start.agent)
        {
            Ok(r) => r,
            Err(e) => return refused(e),
        };
    if start.mode != OnlineMode::Bots {
        if !start.kingdom.is_empty() || !start.landscapes.is_empty() {
            return refused("public matchmaking does not accept required cards");
        }
        if let Err(e) = web
            .hosted
            .client
            .matchmaking(start.mode, start.matching)
            .await
        {
            return refused(e);
        }
        let update = web.hosted.client.current();
        if update.reducer.running {
            if let Err(e) = assign(&web, start.agent) {
                return refused(e);
            }
        } else if update.searching {
            web.set_job(Some(crate::house::Job {
                official: true,
                official_account: Some(web.hosted.id.clone()),
                transcript: false,
                configuration: None,
                agent: start.agent,
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
    if let Err(e) = assign(&web, start.agent) {
        return refused(e);
    }
    drop(reservation);
    axum::Json(management(&web)).into_response()
}
async fn attach_agent(
    State(web): State<Web>,
    axum::Json(body): axum::Json<OnlineAttach>,
) -> Response {
    let _guard = web.hosted.control.lock().await;
    let reservation =
        match web
            .access
            .arena
            .reserve_official(&web.hosted.owner, &web.hosted.id, &body.agent)
        {
            Ok(r) => r,
            Err(e) => return refused(e),
        };
    if let Err(e) = assign(&web, body.agent) {
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

async fn cancel_matchmaking(State(web): State<Web>) -> Response {
    let _guard = web.hosted.control.lock().await;
    match web.hosted.client.cancel_matchmaking().await {
        Ok(()) => {
            let update = web.hosted.client.current();
            if update.reducer.running {
                if let Some(job) = web.job()
                    && job.game_id.is_empty()
                    && let Err(e) = assign(&web, job.agent)
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
