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
    OnlineAttach, OnlineManagement, OnlineMode, OnlineObservation, OnlineStart,
};
use dominion_protocol::lobby::UserKind;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Hosted {
    pub client: Client,
    pub owner: String,
    control: Arc<tokio::sync::Mutex<()>>,
}
impl Hosted {
    pub(crate) fn new(client: Client, owner: String) -> Self {
        Self {
            client,
            owner,
            control: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    pub async fn start(config: &Config) -> Result<Self, String> {
        let owner = config
            .online_owner
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .ok_or("online_enabled requires online_owner (a registered Arena account name)")?
            .clone();
        Ok(Self::new(super::start(config).await?, owner))
    }
    pub(crate) fn router(
        &self,
        arena: Arc<Arena>,
        hosts: Vec<String>,
        idle: Option<Duration>,
        stop: CancellationToken,
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
        };
        let lifecycle_web = Web {
            hosted: self.clone(),
            access: auth.clone(),
            stop: stop.clone(),
        };
        let lifecycle_arena = auth.arena.clone();
        let mut updates = self.client.updates.clone();
        let lifecycle_stop = stop.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {_=lifecycle_stop.cancelled()=>break,changed=updates.changed()=>if changed.is_err(){break;}}
                let _guard = lifecycle_web.hosted.control.lock().await;
                let update = updates.borrow_and_update().clone();
                if update.error.is_some() || !update.connected {
                    lifecycle_arena.set_official_job(None);
                } else if update.reducer.running {
                    if let Some(job) = lifecycle_arena.official_job()
                        && job.game_id.is_empty()
                        && let Err(e) = assign(&lifecycle_web, job.agent)
                    {
                        eprintln!("dominion: assigning matched game: {e}");
                    }
                } else if !update.searching
                    && lifecycle_arena
                        .official_job()
                        .is_some_and(|j| !j.game_id.is_empty())
                {
                    lifecycle_arena.set_official_job(None);
                }
            }
            lifecycle_arena.set_official_job(None);
        });
        Router::new()
            .route("/api/online/view", get(snapshot))
            .route("/api/online/events", get(events))
            .route("/api/online/manage", get(manage))
            .route("/api/online/start", post(start_game))
            .route("/api/online/agent", post(attach_agent).delete(pause_agent))
            .route(
                "/api/online/matchmaking",
                axum::routing::delete(cancel_matchmaking),
            )
            .route("/api/online/reconnect", post(reconnect))
            .route("/api/online/resign", post(resign))
            .with_state(Web {
                hosted: self.clone(),
                access: auth.clone(),
                stop,
            })
            .nest_service("/mcp/online", service)
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
    let runner = request.uri().path().starts_with("/mcp/online")
        && crate::http::bearer(request.headers()).is_some_and(|token| {
            access
                .arena
                .official_job()
                .is_some_and(|job| !job.token.is_empty() && job.token == token)
        });
    if !runner && let Err(status) = access.allowed(request.headers()) {
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
    headers: HeaderMap,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let updates = web.hosted.client.updates.clone();
    let stream = futures_util::stream::unfold(
        (updates, true, web, headers),
        |(mut updates, first, web, headers)| async move {
            if web.stop.is_cancelled() {
                return None;
            }
            if !first {
                tokio::select! {
                    _=web.stop.cancelled()=>return None,
                    changed=updates.changed()=>if changed.is_err(){return None;},
                }
            }
            if web.access.allowed(&headers).is_err() {
                return None;
            }
            let update = updates.borrow_and_update().clone();
            let event = Event::default()
                .event("change")
                .json_data(observation(update))
                .expect("serializable observation");
            Some((Ok(event), (updates, false, web, headers)))
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::new())
}
impl std::fmt::Debug for Hosted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosted")
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
        account: update.reducer.names.get(&update.reducer.player_id).cloned(),
        agents: web.access.arena.official_agents(),
        agent: web.access.arena.official_job().map(|j| j.agent),
        running: view.is_some_and(|v| v.game.running),
        searching: update.searching
            || web
                .access
                .arena
                .official_job()
                .is_some_and(|j| j.game_id.is_empty()),
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
    web.access.arena.set_official_job(Some(crate::house::Job {
        official: true,
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
    let reservation = match web
        .access
        .arena
        .reserve_official(&web.hosted.owner, &start.agent)
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
            web.access.arena.set_official_job(Some(crate::house::Job {
                official: true,
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
    let reservation = match web
        .access
        .arena
        .reserve_official(&web.hosted.owner, &body.agent)
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
    web.access.arena.set_official_job(None);
    axum::Json(management(&web)).into_response()
}
async fn reconnect(State(web): State<Web>) -> Response {
    let _guard = web.hosted.control.lock().await;
    if management(&web).searching {
        return refused("cancel matchmaking before reconnecting");
    }
    web.access.arena.set_official_job(None);
    match web.hosted.client.reconnect().await {
        Ok(()) => axum::Json(management(&web)).into_response(),
        Err(e) => refused(e),
    }
}
async fn resign(State(web): State<Web>) -> Response {
    let _guard = web.hosted.control.lock().await;
    match web.hosted.client.resign().await {
        Ok(()) => {
            web.access.arena.set_official_job(None);
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
                if let Some(job) = web.access.arena.official_job()
                    && job.game_id.is_empty()
                    && let Err(e) = assign(&web, job.agent)
                {
                    return refused(e);
                }
            } else if !update.searching {
                web.access.arena.set_official_job(None);
            }
            axum::Json(management(&web)).into_response()
        }
        Err(e) => refused(e),
    }
}
