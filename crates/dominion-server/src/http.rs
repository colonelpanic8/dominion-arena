//! The HTTP side of the server: the JSON API at `/api`, the asset cache at `/assets` and
//! the web client everywhere else. A seat acts through its token, like MCP agents; an
//! account through its session (a cookie in a browser, or a bearer token), which also
//! acts for the seats it plays.

use crate::Options;
use crate::arena::{self, Arena, Ident, SESSION_COOKIE};
use crate::hub::{Hub, HubError, Seat};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header, uri::Authority};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use dominion_agent::{AnswerError, WayChoice};
use dominion_engine::{Card, Kind, PlayerId};
use dominion_protocol::api::{
    AnswerRequest, ApiError, CardCost, CardInfo, Change, DealRequest, Dealt, NewGame, SeatInfo,
    SeatView, UndoAsk, ViewQuery, WebSettings,
};
use dominion_protocol::lobby::{
    BotKey, ChatKind, ChatLog, Credentials, GuestRequest, History, HistoryQuery, JoinTable,
    Leaderboard, Lobby, LobbyChange, MatchRequest, Me, MyGame, NewBot, NewPassword, NewTable,
    Profile, QueueStatus, Ready, Say, SeatEdit, TableInfo, TableSpec,
};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};

/// A year: asset files never change under their path.
const ASSETS_CACHE: &str = "public, max-age=31536000, immutable";
/// No request body is longer.
const MAX_BODY: usize = 64 * 1024;
/// Sixty days, as long as a session lasts unused.
const COOKIE_AGE: u32 = 60 * 86_400;

/// The routes besides `/mcp`, all behind the Host check that guards `/mcp`.
pub fn router(
    arena: Arc<Arena>,
    options: &Options,
    allowed_hosts: Vec<String>,
    stop: CancellationToken,
) -> Router {
    let public = options
        .public_url
        .as_deref()
        .map(|u| u.trim_end_matches('/'));
    let state = Api {
        hub: arena.hub.clone(),
        arena,
        stop,
        web: Arc::new(options.web.clone()),
        secure: public.is_some_and(|u| u.starts_with("https://")),
        trust_proxy: options.trust_proxy,
        origin: public.map(|u| Arc::new(u.to_ascii_lowercase())),
        official_server: options.official_server.clone(),
        native_web_dir: options.web_dir.clone(),
    };
    let api = Router::new()
        .route("/cards", get(cards))
        .route("/settings", get(settings))
        .route("/settings/banned-cards", put(set_banned_cards))
        .route("/games", get(list).post(create))
        .route("/deal", post(deal))
        .route("/seat", get(seat))
        .route("/games/{id}/view", get(view))
        .route("/games/{id}/watch", get(watch))
        .route("/games/{id}/answer", post(answer))
        .route("/games/{id}/resign", post(resign))
        .route("/games/{id}/undo", post(undo))
        .route("/games/{id}/undo/allow", post(allow_undo))
        .route("/games/{id}/undo/refuse", post(refuse_undo))
        .route("/games/{id}/undo/cancel", post(cancel_undo))
        .route("/games/{id}/events", get(events))
        .route(
            "/games/{id}/transcript",
            get(transcript).post(publish_transcript),
        )
        .route("/games/{id}/transcript/events", get(transcript_events))
        .route("/games/{id}/recording", get(recording))
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/guest", post(guest))
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
        .route("/me/password", put(set_password))
        .route("/me/avatar", put(set_avatar).delete(clear_avatar))
        .route("/me/games", get(my_games))
        .route("/me/bots", post(new_bot))
        .route("/me/bots/{name}", delete(delete_bot))
        .route("/me/bots/{name}/key", post(new_bot_key))
        .route("/lobby", get(lobby))
        .route("/lobby/events", get(lobby_events))
        .route("/tables", post(new_table))
        .route("/tables/{id}", get(table))
        .route("/tables/{id}/spec", put(set_spec))
        .route("/tables/{id}/seats", put(set_seats))
        .route("/tables/{id}/join", post(join_table))
        .route("/tables/{id}/leave", post(leave_table))
        .route("/tables/{id}/ready", post(ready))
        .route("/automatch", post(search).delete(cancel_search))
        .route("/chat/{channel}", get(chat_log).post(say))
        .route("/users/{name}", get(profile))
        .route("/users/{name}/avatar", get(avatar))
        .route("/leaderboard", get(leaderboard))
        .route("/history", get(history))
        .fallback(|| async { Error::new(StatusCode::NOT_FOUND, "no such route") })
        .method_not_allowed_fallback(|| async {
            Error::new(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
        });
    let api = if options.official_server.is_some() {
        api.route("/games/{id}/official/2.3.7", get(official_socket))
            .route("/official/2.3.7", get(official_lifecycle_socket))
            .route("/official/launch-code", post(official_launch_code))
            .route("/official/coverage", get(official_coverage))
    } else {
        api
    };
    let api = api
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn_with_state(state.clone(), same_origin))
        .with_state(state.clone());
    // The web client's bundler puts its own (content-hashed) files under /assets too:
    // the cache answers first, then the client's.
    let dir = |d: &std::path::Path| ServeDir::new(d).append_index_html_on_directories(false);
    let client = options.web_dir.as_ref().map(|d| d.join("assets"));
    let assets = match (&options.assets_dir, &client) {
        (Some(cache), Some(client)) => {
            Router::new().fallback_service(dir(cache).fallback(dir(client)))
        }
        (Some(only), None) | (None, Some(only)) => Router::new().fallback_service(dir(only)),
        (None, None) => Router::new(),
    }
    .layer(middleware::map_response(cache_forever))
    .layer(middleware::from_fn(no_manifest));
    let site = Router::new()
        .nest("/api", api)
        .nest_service("/assets", assets);
    let site = match &options.web_dir {
        Some(dir) => site.fallback_service(
            Router::new()
                .fallback_service(
                    ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html"))),
                )
                .layer(middleware::from_fn(fresh_client)),
        ),
        None if options
            .official_server
            .as_ref()
            .is_some_and(|h| h.root_socket) =>
        {
            site
        }
        None => site.route("/", get(no_client)),
    };
    let site = if options
        .official_server
        .as_ref()
        .is_some_and(|h| h.root_socket)
    {
        site.route("/", get(official_root).with_state(state))
    } else {
        site
    };
    site.layer(middleware::from_fn_with_state(
        Arc::new(allowed_hosts),
        check_host,
    ))
}

async fn fresh_client(mut request: Request, next: Next) -> Response {
    request.headers_mut().remove(header::IF_MODIFIED_SINCE);
    request.headers_mut().remove(header::IF_NONE_MATCH);
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().remove(header::LAST_MODIFIED);
    response.headers_mut().remove(header::ETAG);
    response
}

async fn no_client() -> &'static str {
    "dominion: the MCP endpoint is /mcp; the web client isn't installed here.\n"
}

/// Without an asset cache the manifest is 204 No Content rather than 404, so that a
/// browser asking for it has no failed request to report.
async fn no_manifest(req: Request, next: Next) -> Response {
    let manifest = req.uri().path() == "/manifest.json";
    let res = next.run(req).await;
    if manifest && res.status() == StatusCode::NOT_FOUND {
        return StatusCode::NO_CONTENT.into_response();
    }
    res
}

async fn cache_forever(mut res: Response) -> Response {
    if res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED {
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(ASSETS_CACHE),
        );
    }
    res
}

/// rmcp's rule: a listed host matches with any port, a listed host:port only with it.
fn host_allowed(host: &Authority, allowed: &[String]) -> bool {
    let norm = |h: &str| h.trim_matches(['[', ']']).to_ascii_lowercase();
    let host_name = norm(host.host());
    allowed.is_empty()
        || allowed.iter().any(|a| {
            let a = a.trim();
            if a.is_empty() {
                return false;
            }
            let (name, port) = match Authority::try_from(a) {
                Ok(auth) => (norm(auth.host()), auth.port_u16()),
                Err(_) => (norm(a), None),
            };
            name == host_name && port.is_none_or(|p| host.port_u16() == Some(p))
        })
}

fn host_of(req: &Request) -> Option<Authority> {
    match req.headers().get(header::HOST) {
        Some(h) => h.to_str().ok().and_then(|h| Authority::try_from(h).ok()),
        None => req.uri().authority().cloned(),
    }
}

pub(crate) async fn check_host(
    State(allowed): State<Arc<Vec<String>>>,
    req: Request,
    next: Next,
) -> Response {
    match host_of(&req) {
        None => {
            Error::new(StatusCode::BAD_REQUEST, "missing or invalid Host header").into_response()
        }
        Some(h) if !host_allowed(&h, &allowed) => {
            Error::new(StatusCode::FORBIDDEN, "Host header is not allowed").into_response()
        }
        Some(_) => next.run(req).await,
    }
}

/// A browser says where a request that changes something comes from: only this site's
/// own pages may make one, since a session's cookie goes along with it.
async fn same_origin(State(api): State<Api>, req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && let Some(origin) = req.headers().get(header::ORIGIN) {
        let host = host_of(&req);
        if !origin
            .to_str()
            .is_ok_and(|origin| origin_allowed(&api, origin, host.as_ref(), true))
        {
            return Error::new(
                StatusCode::FORBIDDEN,
                "requests from other sites are refused",
            )
            .into_response();
        }
    }
    next.run(req).await
}

fn origin_allowed(api: &Api, origin: &str, host: Option<&Authority>, local_proxy: bool) -> bool {
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    let (Some(scheme), Some(from)) = (uri.scheme_str(), uri.authority()) else {
        return false;
    };
    if !matches!(scheme, "http" | "https")
        || from.as_str().contains('@')
        || origin != format!("{scheme}://{from}")
    {
        return false;
    }
    let origin = origin.to_ascii_lowercase();
    let local = |authority: &Authority| {
        let host = authority.host().trim_matches(['[', ']']);
        host.eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    };
    host.is_some_and(|host| host.as_str().eq_ignore_ascii_case(from.as_str()))
        || api.origin.as_deref().is_some_and(|own| *own == origin)
        || (local_proxy && local(from) && host.is_some_and(local))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfficialQuery {
    seat: Option<u8>,
}

async fn official_socket(
    State(api): State<Api>,
    Path(id): Path<String>,
    Query(query): Query<OfficialQuery>,
    Peer(peer): Peer,
    headers: HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Result<Response> {
    use crate::official_host::{
        identity::ProjectionRequest, lifecycle::context::context, profile, projection, session,
    };
    let host = api
        .official_server
        .clone()
        .ok_or_else(|| Error::new(StatusCode::NOT_FOUND, "no such route"))?;
    let authority = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| Authority::try_from(h).ok());
    let origin_ok = match headers.get(header::ORIGIN) {
        Some(origin) => origin
            .to_str()
            .is_ok_and(|origin| origin_allowed(&api, origin, authority.as_ref(), false)),
        None => bearer(&headers).is_some(),
    };
    if !origin_ok {
        return Err(Error::new(
            StatusCode::FORBIDDEN,
            "WebSocket Origin is missing or not allowed",
        ));
    }
    host.upgrades.sweep();
    host.upgrades
        .take(api.ip(&headers, peer))
        .map_err(|wait| Error {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "too many socket upgrades".into(),
            retry_after: Some(wait),
        })?;
    let permit = host.connections.clone().try_acquire_owned().map_err(|_| {
        Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "too many socket connections",
        )
    })?;
    let seat = api.seat(&id, &headers, None, query.seat)?;
    api.hub
        .admission(&id, |config, meta| {
            profile::check_supported(config, meta, &host.catalog)
        })?
        .map_err(|error| Error::new(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let context = context(&api.arena, &seat, &host)
        .map_err(|error| Error::new(StatusCode::UNPROCESSABLE_ENTITY, error))?;
    api.hub
        .project_seat(&seat, |observation| {
            projection::project(
                observation,
                &context,
                &host.catalog,
                ProjectionRequest {
                    question_index: 1,
                    reconnect: false,
                    epoch: 1,
                },
            )
        })?
        .map_err(|error| Error::new(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    // Keep only the credential actually selected by Api::seat, never arbitrary headers.
    let mut credential = HeaderMap::new();
    if bearer(&headers).is_some() {
        credential.insert(
            header::AUTHORIZATION,
            headers[header::AUTHORIZATION].clone(),
        );
    } else if let Some(token) = cookie(&headers) {
        let value = HeaderValue::from_str(&format!("{SESSION_COOKIE}={token}"))
            .map_err(|_| Error::new(StatusCode::UNAUTHORIZED, "invalid session cookie"))?;
        credential.insert(header::COOKIE, value);
    }
    let fixed = seat.clone();
    let arena = api.arena.clone();
    let stop = api.stop.clone();
    Ok(upgrade
        .max_message_size(session::MAX_INBOUND)
        .max_frame_size(session::MAX_INBOUND)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            session::run(
                socket,
                arena,
                seat,
                host,
                move || {
                    api.seat(&fixed.game, &credential, None, query.seat)
                        .is_ok_and(|seat| seat == fixed)
                },
                stop,
            )
            .await;
        }))
}

async fn official_lifecycle_socket(
    State(api): State<Api>,
    Peer(peer): Peer,
    headers: HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Result<Response> {
    lifecycle_upgrade(api, peer, headers, upgrade)
}

fn lifecycle_upgrade(
    api: Api,
    peer: Option<SocketAddr>,
    headers: HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Result<Response> {
    let host = api
        .official_server
        .clone()
        .ok_or_else(|| Error::new(StatusCode::NOT_FOUND, "no such route"))?;
    let authority = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| Authority::try_from(h).ok());
    if !headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|origin| origin_allowed(&api, origin, authority.as_ref(), false))
    {
        return Err(Error::new(
            StatusCode::FORBIDDEN,
            "WebSocket Origin is missing or not allowed",
        ));
    }
    host.upgrades.sweep();
    let ip = api.ip(&headers, peer);
    host.upgrades.take(ip).map_err(|wait| Error {
        status: StatusCode::TOO_MANY_REQUESTS,
        message: "too many socket upgrades".into(),
        retry_after: Some(wait),
    })?;
    let permit = host.connections.clone().try_acquire_owned().map_err(|_| {
        Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "too many socket connections",
        )
    })?;
    Ok(upgrade
        .max_message_size(crate::official_host::session::MAX_INBOUND)
        .max_frame_size(crate::official_host::session::MAX_INBOUND)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            crate::official_host::lifecycle::run(socket, api.arena, host, ip, api.stop).await;
        }))
}

async fn official_root(
    State(api): State<Api>,
    Peer(peer): Peer,
    headers: HeaderMap,
    upgrade: std::result::Result<
        axum::extract::ws::WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
) -> Result<Response> {
    if let Ok(upgrade) = upgrade {
        return lifecycle_upgrade(api, peer, headers, upgrade);
    }
    if let Some(dir) = &api.native_web_dir {
        let bytes = std::fs::read(dir.join("index.html"))
            .map_err(|_| Error::new(StatusCode::NOT_FOUND, "web client unavailable"))?;
        return Ok((
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response());
    }
    Ok(no_client().await.into_response())
}

async fn official_coverage(State(api): State<Api>) -> Result<axum::Json<serde_json::Value>> {
    let host = api
        .official_server
        .as_ref()
        .ok_or_else(|| Error::new(StatusCode::NOT_FOUND, "no such route"))?;
    Ok(axum::Json(host.coverage_report()))
}

async fn official_launch_code(State(api): State<Api>, headers: HeaderMap) -> Result<Response> {
    let host = api
        .official_server
        .as_ref()
        .ok_or_else(|| Error::new(StatusCode::NOT_FOUND, "no such route"))?;
    let ident = api
        .ident(&headers)
        .ok_or_else(|| Error::new(StatusCode::UNAUTHORIZED, "Arena authentication required"))?;
    let token = bearer(&headers)
        .or_else(|| cookie(&headers))
        .ok_or_else(|| Error::new(StatusCode::UNAUTHORIZED, "Arena authentication required"))?;
    if api.arena.identify(token).as_ref() != Some(&ident) {
        return Err(Error::new(
            StatusCode::UNAUTHORIZED,
            "invalid Arena session",
        ));
    }
    let code = host
        .issue_code(token, &api.arena)
        .map_err(|e| Error::new(StatusCode::TOO_MANY_REQUESTS, e))?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(serde_json::json!({"code":code,"expiresIn":30})),
    )
        .into_response())
}

#[derive(Clone)]
struct Api {
    arena: Arc<Arena>,
    hub: Arc<Hub>,
    stop: CancellationToken,
    web: Arc<WebSettings>,
    /// Session cookies are for HTTPS only.
    secure: bool,
    /// The client's address is the last one in `X-Forwarded-For`.
    trust_proxy: bool,
    /// The origin people reach the server at, when it isn't what `Host` says.
    origin: Option<Arc<String>>,
    official_server: Option<Arc<crate::official_host::Host>>,
    native_web_dir: Option<std::path::PathBuf>,
}

pub(crate) struct Error {
    status: StatusCode,
    message: String,
    retry_after: Option<Duration>,
}

impl Error {
    fn new(status: StatusCode, message: impl Into<String>) -> Error {
        Error {
            status,
            message: message.into(),
            retry_after: None,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let body = axum::Json(ApiError {
            error: self.message,
        });
        let mut res = (self.status, body).into_response();
        if let Some(wait) = self.retry_after {
            let secs = wait.as_secs().max(1);
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        res
    }
}

impl From<HubError> for Error {
    fn from(e: HubError) -> Error {
        let status = match &e {
            HubError::BadRequest(_) => StatusCode::BAD_REQUEST,
            HubError::UnknownGame(_) => StatusCode::NOT_FOUND,
            HubError::BadToken(_) => StatusCode::UNAUTHORIZED,
            HubError::Forbidden(_) => StatusCode::FORBIDDEN,
            HubError::Conflict(_) => StatusCode::CONFLICT,
            HubError::Busy(_) | HubError::Unsaved(_) => StatusCode::SERVICE_UNAVAILABLE,
            HubError::Answer(AnswerError::Invalid(_)) => StatusCode::UNPROCESSABLE_ENTITY,
            HubError::Answer(_) => StatusCode::CONFLICT,
            HubError::Engine(_) => StatusCode::INTERNAL_SERVER_ERROR,
            HubError::NotFound(_) => StatusCode::NOT_FOUND,
        };
        Error::new(status, e.to_string())
    }
}

impl From<arena::Error> for Error {
    fn from(e: arena::Error) -> Error {
        let message = e.to_string();
        let status = match e {
            arena::Error::Hub(e) => return e.into(),
            arena::Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            arena::Error::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            arena::Error::Forbidden(_) => StatusCode::FORBIDDEN,
            arena::Error::NotFound(_) => StatusCode::NOT_FOUND,
            arena::Error::Conflict(_) => StatusCode::CONFLICT,
            arena::Error::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
            arena::Error::TooMany(_, wait) => {
                return Error {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    message,
                    retry_after: Some(wait),
                };
            }
        };
        Error::new(status, message)
    }
}

type Result<T> = std::result::Result<T, Error>;
type Json<T> = Result<axum::Json<T>>;

/// The address a request is counted against: where it came from, or behind a trusted
/// proxy the last address of `X-Forwarded-For`, which the proxy wrote.
pub(crate) fn client_ip(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trust_proxy: bool,
) -> IpAddr {
    let forwarded = || {
        let value = headers.get("x-forwarded-for")?.to_str().ok()?;
        value.rsplit(',').next()?.trim().parse().ok()
    };
    (trust_proxy.then(forwarded).flatten())
        .or(peer.map(|p| p.ip()))
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn body<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| {
        Error::new(
            StatusCode::BAD_REQUEST,
            format!("invalid request body: {e}"),
        )
    })
}

/// A body that may be left out.
fn body_or_default<T: DeserializeOwned + Default>(bytes: &[u8]) -> Result<T> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    body(bytes)
}

pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

pub(crate) fn cookie(headers: &HeaderMap) -> Option<&str> {
    let cookies = headers.get_all(header::COOKIE).into_iter();
    cookies
        .filter_map(|c| c.to_str().ok())
        .flat_map(|c| c.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value)
}

impl Api {
    /// The session token or API key a request carries.
    fn credential<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        bearer(headers).or_else(|| cookie(headers))
    }

    /// The account a request is made for, if it is signed in.
    fn ident(&self, headers: &HeaderMap) -> Option<Ident> {
        self.arena.identify(self.credential(headers)?)
    }

    fn signed_in(&self, headers: &HeaderMap) -> Result<Ident> {
        self.ident(headers)
            .ok_or_else(|| Error::new(StatusCode::UNAUTHORIZED, "sign in first"))
    }

    fn ip(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> IpAddr {
        client_ip(headers, peer, self.trust_proxy)
    }

    fn session_cookie(&self, token: &str, age: u32) -> HeaderValue {
        let secure = if self.secure { "; Secure" } else { "" };
        let cookie = format!(
            "{SESSION_COOKIE}={token}; Path=/; Max-Age={age}; HttpOnly; SameSite=Lax{secure}"
        );
        HeaderValue::from_str(&cookie).unwrap_or(HeaderValue::from_static(""))
    }

    /// The seat a request acts for in game `id`: the one its seat token claims, else one
    /// its account plays (`seat` says which, when it plays several).
    fn seat(
        &self,
        id: &str,
        headers: &HeaderMap,
        query_token: Option<&str>,
        seat: Option<u8>,
    ) -> Result<Seat> {
        let given = bearer(headers).or(query_token.filter(|t| !t.is_empty()));
        let claimed = given.map(|token| self.hub.claim(id, token));
        if let Some(Ok(seat)) = claimed {
            return Ok(seat);
        }
        let ident = given
            .or(cookie(headers))
            .and_then(|t| self.arena.identify(t));
        let Some(ident) = ident else {
            return Err(match claimed {
                Some(Err(e)) => e.into(),
                _ => Error::new(StatusCode::UNAUTHORIZED, "missing seat token"),
            });
        };
        let owned = self.hub.owned_in(id, &ident.user);
        let found = match seat {
            Some(n) => owned.into_iter().find(|s| s.seat == PlayerId(n)),
            None => owned.into_iter().next(),
        };
        let seat = match found {
            Some(seat) => seat,
            None => {
                self.hub.meta(id)?;
                return Err(Error::new(
                    StatusCode::FORBIDDEN,
                    format!("you have no seat in game {id}"),
                ));
            }
        };
        self.hub.touch(&seat);
        Ok(seat)
    }
}

/// The address a request came from, when the server was told (`serve` is).
struct Peer(Option<SocketAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Peer {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> std::result::Result<Peer, Infallible> {
        let info = parts.extensions.get::<ConnectInfo<SocketAddr>>();
        Ok(Peer(info.map(|ConnectInfo(addr)| *addr)))
    }
}

fn peer(Peer(addr): Peer) -> Option<SocketAddr> {
    addr
}

/// Runs password hashing off the async threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> arena::Result<T> + Send + 'static,
) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => Ok(result?),
        Err(_) => Err(Error::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the server failed",
        )),
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Card => "card",
        Kind::Event => "event",
        Kind::Landmark => "landmark",
        Kind::Project => "project",
        Kind::Way => "way",
        Kind::Ally => "ally",
        Kind::Trait => "trait",
        Kind::Prophecy => "prophecy",
        Kind::Study => "study",
        Kind::Boon => "boon",
        Kind::Hex => "hex",
        Kind::State => "state",
        Kind::Artifact => "artifact",
        Kind::Randomizer => "randomizer",
    }
}

fn card_info(card: Card) -> CardInfo {
    let d = card.def();
    CardInfo {
        name: d.name.into(),
        set: d.set.to_string(),
        types: card.type_names().into_iter().map(String::from).collect(),
        kind: kind_name(card.kind()).into(),
        cost: d.cost.map(|c| CardCost {
            coins: c.coins.into(),
            potions: c.potions.into(),
            debt: c.debt.into(),
        }),
        variable_cost: d.variable_cost,
        overpay: d.overpay,
        coins: d.coins.into(),
        potions: d.potions.into(),
        vp: d.vp.into(),
        kingdom: d.kingdom,
        basic: d.basic,
        removed: d.removed,
        pile: d.pile.map(|p| p.name().into()),
        text: d.text.into(),
    }
}

async fn cards() -> axum::Json<Vec<CardInfo>> {
    axum::Json(Card::ALL.iter().map(|&c| card_info(c)).collect())
}

fn web_settings(api: &Api, ident: Option<&Ident>) -> WebSettings {
    let server = api.arena.settings();
    WebSettings {
        banned_cards: api.arena.banned_cards(ident),
        registration: server.registration,
        guests: server.guests,
        ..(*api.web).clone()
    }
}

async fn settings(State(api): State<Api>, headers: HeaderMap) -> axum::Json<WebSettings> {
    axum::Json(web_settings(&api, api.ident(&headers).as_ref()))
}

/// Replaces the asker's Banned cards list.
async fn set_banned_cards(
    State(api): State<Api>,
    headers: HeaderMap,
    req: Bytes,
) -> Json<WebSettings> {
    let ident = api.signed_in(&headers)?;
    let names: Vec<String> = body(&req)?;
    api.arena.set_banned_cards(&ident, &names)?;
    Ok(axum::Json(web_settings(&api, Some(&ident))))
}

async fn list(State(api): State<Api>) -> impl IntoResponse {
    axum::Json(api.hub.list())
}

async fn create(
    State(api): State<Api>,
    info: Peer,
    headers: HeaderMap,
    req: Bytes,
) -> Result<impl IntoResponse> {
    let req: NewGame = body(&req)?;
    api.arena.may_create(api.ip(&headers, peer(info)))?;
    let created = api.arena.create_game(api.ident(&headers).as_ref(), &req)?;
    Ok((StatusCode::CREATED, axum::Json(created)))
}

async fn deal(State(api): State<Api>, info: Peer, headers: HeaderMap, req: Bytes) -> Json<Dealt> {
    let req: DealRequest = body(&req)?;
    api.arena.may_deal(api.ip(&headers, peer(info)))?;
    Ok(axum::Json(crate::hub::deal(&req)?))
}

async fn seat(State(api): State<Api>, headers: HeaderMap) -> Json<SeatInfo> {
    let unknown = || Error::new(StatusCode::UNAUTHORIZED, "unknown seat token");
    let token = bearer(&headers)
        .ok_or_else(|| Error::new(StatusCode::UNAUTHORIZED, "missing seat token"))?;
    let seat = api.hub.find(token).map_err(|_| unknown())?;
    Ok(axum::Json(SeatInfo {
        name: api.hub.seat_name(&seat)?,
        game_id: seat.game,
        seat: seat.seat.0,
    }))
}

#[derive(Deserialize, Default)]
struct SeatQuery {
    /// A seat token, where no header can carry it (an event stream).
    token: Option<String>,
    /// Which of the account's seats, when it plays several.
    seat: Option<u8>,
}

type Queried<T> = std::result::Result<Query<T>, axum::extract::rejection::QueryRejection>;

fn query<T>(query: Queried<T>) -> Result<T> {
    let Query(query) = query.map_err(|e| Error::new(StatusCode::BAD_REQUEST, e.body_text()))?;
    Ok(query)
}

async fn view(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    view_query: Queried<ViewQuery>,
    seat_query: Queried<SeatQuery>,
) -> Json<SeatView> {
    let (view_query, seat_query) = (query(view_query)?, query(seat_query)?);
    let seat = api.seat(&id, &headers, None, seat_query.seat)?;
    Ok(axum::Json(api.hub.seat_view(&seat, view_query.log_from)?))
}

/// The game as someone watching sees it.
async fn watch(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    view_query: Queried<ViewQuery>,
) -> Json<SeatView> {
    let ident = api.ident(&headers);
    if let Some(ident) = &ident {
        api.arena.touch(ident);
    }
    let view_query = query(view_query)?;
    let from = view_query.log_from.unwrap_or(0);
    if let Some(step) = view_query.step {
        return Ok(axum::Json(api.hub.replay_view(
            &id,
            step,
            from,
            view_query.as_seat,
        )?));
    }
    let watched = api.hub.watch_view(&id, from, view_query.as_seat)?;
    if watched.view.game.running
        && ident.is_some_and(|ident| !api.hub.owned_in(&id, &ident.user).is_empty())
    {
        return Err(Error::new(
            StatusCode::FORBIDDEN,
            "players must use their seat view while the game is running",
        ));
    }
    Ok(axum::Json(watched))
}

async fn answer(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat_query: Queried<SeatQuery>,
    req: Bytes,
) -> Json<SeatView> {
    let seat = api.seat(&id, &headers, None, query(seat_query)?.seat)?;
    let req: AnswerRequest = body(&req)?;
    let way = req
        .way
        .as_deref()
        .map(WayChoice::parse)
        .transpose()
        .map_err(|e| Error::new(StatusCode::BAD_REQUEST, e))?;
    api.hub.answer_with(&seat, &req.answer, way)?;
    Ok(axum::Json(api.hub.seat_view(&seat, None)?))
}

async fn resign(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat_query: Queried<SeatQuery>,
) -> Json<SeatView> {
    let seat = api.seat(&id, &headers, None, query(seat_query)?.seat)?;
    api.hub.resign(&seat, "resigned")?;
    Ok(axum::Json(api.hub.seat_view(&seat, None)?))
}

/// Asks to take back the seat's last answer (or, with `logSeq`, the answer that led to
/// that log entry, and every answer after it). Where the game's rule is to ask, it waits
/// for the other seats to allow it. The view returned has the whole log.
async fn undo(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat_query: Queried<SeatQuery>,
    req: Bytes,
) -> Json<SeatView> {
    let seat = api.seat(&id, &headers, None, query(seat_query)?.seat)?;
    let ask: UndoAsk = body_or_default(&req)?;
    api.hub.undo(&seat, ask.log_seq)?;
    Ok(axum::Json(api.hub.seat_view(&seat, Some(0))?))
}

async fn allow_undo(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat_query: Queried<SeatQuery>,
) -> Json<SeatView> {
    let seat = api.seat(&id, &headers, None, query(seat_query)?.seat)?;
    api.hub.answer_undo(&seat, true)?;
    Ok(axum::Json(api.hub.seat_view(&seat, Some(0))?))
}

async fn refuse_undo(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat_query: Queried<SeatQuery>,
) -> Json<SeatView> {
    let seat = api.seat(&id, &headers, None, query(seat_query)?.seat)?;
    api.hub.answer_undo(&seat, false)?;
    Ok(axum::Json(api.hub.seat_view(&seat, None)?))
}

async fn cancel_undo(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat_query: Queried<SeatQuery>,
) -> Json<SeatView> {
    let seat = api.seat(&id, &headers, None, query(seat_query)?.seat)?;
    api.hub.cancel_undo(&seat)?;
    Ok(axum::Json(api.hub.seat_view(&seat, None)?))
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptQuery {
    #[serde(default)]
    after: u64,
}

fn transcript_access(api: &Api, id: &str, headers: &HeaderMap, q: &SeatQuery) -> Result<()> {
    if crate::arena::is_official(id) {
        return Ok(());
    }
    if api.seat(id, headers, q.token.as_deref(), q.seat).is_ok() {
        return Ok(());
    }
    if !api.hub.meta(id)?.watchable {
        return Err(Error::new(StatusCode::FORBIDDEN, "this game is private"));
    }
    Ok(())
}

async fn transcript(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    cursor: Queried<TranscriptQuery>,
    seat: Queried<SeatQuery>,
) -> Json<dominion_protocol::transcript::TranscriptPage> {
    transcript_access(&api, &id, &headers, &query(seat)?)?;
    let after = query(cursor)?.after;
    let arena = api.arena.clone();
    let page = blocking(move || arena.transcript(&id, after)).await?;
    Ok(axum::Json(page))
}

async fn publish_transcript(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Bytes,
) -> Result<StatusCode> {
    let seat = api.seat(&id, &headers, None, None)?;
    let input: dominion_protocol::transcript::TranscriptInput = body(&req)?;
    let arena = api.arena.clone();
    blocking(move || arena.append_transcript(&seat, &input)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn transcript_events(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat: Queried<SeatQuery>,
) -> Result<Events> {
    transcript_access(&api, &id, &headers, &query(seat)?)?;
    let changes = if crate::arena::is_official(&id) {
        api.arena.official_transcript_changes()
    } else {
        api.hub.transcript_changes(&id)?
    };
    let (held, idle) = tokio::sync::watch::channel(0);
    Ok(feed(&api, changes, idle, held, |version, _| Change {
        version,
        chat: 0,
    }))
}

/// A finished game's recorded `dominion/events@1` stream, as written: for its seats, and
/// for anyone when the game is watchable.
async fn recording(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat: Queried<SeatQuery>,
) -> Result<Response> {
    let q = query(seat)?;
    if api.seat(&id, &headers, q.token.as_deref(), q.seat).is_err() && !api.hub.meta(&id)?.watchable
    {
        return Err(Error::new(StatusCode::FORBIDDEN, "this game is private"));
    }
    let hub = api.hub.clone();
    let name = id.clone();
    let (encoding, bytes) = blocking(move || Ok(hub.recording(&name)?)).await?;
    let file = format!("attachment; filename=\"{id}{}\"", encoding.suffix());
    Ok((
        [
            (header::CONTENT_TYPE, encoding.media_type().to_string()),
            (header::CONTENT_DISPOSITION, file),
        ],
        bytes,
    )
        .into_response())
}

/// Waits for either counter to change. Whether its sender is gone.
async fn changed(
    version: &mut tokio::sync::watch::Receiver<u64>,
    chat: &mut tokio::sync::watch::Receiver<u64>,
) -> bool {
    tokio::select! {
        r = version.changed() => r.is_err(),
        r = chat.changed() => r.is_err(),
    }
}

type Events = Response;

/// A feed of `change` events: one on connect, then one whenever a counter changes, until
/// the server stops. `held` is kept for as long as the feed is open.
fn feed<T: serde::Serialize, H: Send + 'static>(
    api: &Api,
    version: tokio::sync::watch::Receiver<u64>,
    chat: tokio::sync::watch::Receiver<u64>,
    held: H,
    data: impl Fn(u64, u64) -> T + Send + 'static,
) -> Events {
    let state = (version, chat, true, held, data);
    let changes = futures_util::stream::unfold(state, |mut s| async move {
        if !s.2 {
            // A burst of changes is one event.
            tokio::time::sleep(Duration::from_millis(50)).await;
            if changed(&mut s.0, &mut s.1).await {
                return None;
            }
        }
        let change = (s.4)(*s.0.borrow_and_update(), *s.1.borrow_and_update());
        let data = serde_json::to_string(&change).unwrap_or_default();
        let event = Event::default().event("change").data(data);
        s.2 = false;
        Some((Ok(event), s))
    });
    let stream = changes.take_until(api.stop.clone().cancelled_owned());
    let stream: std::pin::Pin<
        Box<dyn Stream<Item = std::result::Result<Event, Infallible>> + Send>,
    > = Box::pin(stream);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// A game's change feed, for one of its seats or, without one, for someone watching.
async fn events(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    seat_query: Queried<SeatQuery>,
) -> Result<Events> {
    let q = query(seat_query)?;
    let change = |version, chat| Change { version, chat };
    let feed_of = |api: &Api| api.hub.subscribe(&id);
    match api.seat(&id, &headers, q.token.as_deref(), q.seat) {
        Ok(seat) => {
            let held = api.hub.connect(&seat)?;
            let f = feed_of(&api)?;
            Ok(feed(&api, f.version, f.chat, held, change))
        }
        Err(refused) => {
            let watchable = api.hub.meta(&id).is_ok_and(|m| m.watchable);
            if !watchable {
                return Err(refused);
            }
            let attendance = api.ident(&headers).map(|ident| api.arena.attend(&ident));
            let held = (api.hub.watching(&id)?, attendance);
            let f = feed_of(&api)?;
            Ok(feed(&api, f.version, f.chat, held, change))
        }
    }
}

fn with_session(api: &Api, me: Me, token: Option<String>, status: StatusCode) -> Response {
    let mut res = (status, axum::Json(me)).into_response();
    if let Some(token) = token {
        res.headers_mut()
            .insert(header::SET_COOKIE, api.session_cookie(&token, COOKIE_AGE));
    }
    res
}

async fn register(
    State(api): State<Api>,
    info: Peer,
    headers: HeaderMap,
    req: Bytes,
) -> Result<Response> {
    let credentials: Credentials = body(&req)?;
    let ip = api.ip(&headers, peer(info));
    let current = api.ident(&headers);
    let arena = api.arena.clone();
    let (me, token) = blocking(move || arena.register(ip, &credentials, current.as_ref())).await?;
    Ok(with_session(&api, me, token, StatusCode::CREATED))
}

async fn login(
    State(api): State<Api>,
    info: Peer,
    headers: HeaderMap,
    req: Bytes,
) -> Result<Response> {
    let credentials: Credentials = body(&req)?;
    let ip = api.ip(&headers, peer(info));
    let arena = api.arena.clone();
    let (me, token) = blocking(move || arena.login(ip, &credentials)).await?;
    Ok(with_session(&api, me, Some(token), StatusCode::OK))
}

async fn guest(
    State(api): State<Api>,
    info: Peer,
    headers: HeaderMap,
    req: Bytes,
) -> Result<Response> {
    let request: GuestRequest = body_or_default(&req)?;
    let ip = api.ip(&headers, peer(info));
    let (me, token) = api.arena.guest(ip, request.name.as_deref())?;
    Ok(with_session(&api, me, Some(token), StatusCode::CREATED))
}

async fn logout(State(api): State<Api>, headers: HeaderMap) -> Response {
    if let Some(token) = api.credential(&headers) {
        api.arena.logout(token);
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, api.session_cookie("", 0));
    res
}

async fn me(State(api): State<Api>, headers: HeaderMap) -> Json<Me> {
    let ident = api.signed_in(&headers)?;
    Ok(axum::Json(api.arena.me(&ident)?))
}

async fn set_password(
    State(api): State<Api>,
    headers: HeaderMap,
    req: Bytes,
) -> Result<StatusCode> {
    let ident = api.signed_in(&headers)?;
    let passwords: NewPassword = body(&req)?;
    let token = api.credential(&headers).unwrap_or_default().to_string();
    let arena = api.arena.clone();
    blocking(move || arena.set_password(&ident, &token, &passwords.current, &passwords.new))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_avatar(State(api): State<Api>, headers: HeaderMap, req: Bytes) -> Json<Me> {
    let ident = api.signed_in(&headers)?;
    Ok(axum::Json(api.arena.set_avatar(&ident, &req)?))
}

async fn clear_avatar(State(api): State<Api>, headers: HeaderMap) -> Json<Me> {
    let ident = api.signed_in(&headers)?;
    Ok(axum::Json(api.arena.clear_avatar(&ident)?))
}

/// An account's picture. Its URL names the version, so it is kept for good; and as
/// something anyone uploaded, it is never read as anything but the image it was checked to
/// be.
async fn avatar(State(api): State<Api>, Path(name): Path<String>) -> Result<Response> {
    let (kind, image) = api.arena.avatar(&name)?;
    let headers = [
        (header::CONTENT_TYPE, kind),
        (
            header::CACHE_CONTROL,
            "public, max-age=31536000, immutable".into(),
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; sandbox".into(),
        ),
    ];
    Ok((headers, image).into_response())
}

async fn my_games(State(api): State<Api>, headers: HeaderMap) -> Json<Vec<MyGame>> {
    let ident = api.signed_in(&headers)?;
    let games = api.hub.seats_of(&ident.user, Duration::from_secs(3600));
    Ok(axum::Json(games))
}

async fn new_bot(State(api): State<Api>, headers: HeaderMap, req: Bytes) -> Result<Response> {
    let ident = api.signed_in(&headers)?;
    let bot: NewBot = body(&req)?;
    let key = api.arena.new_bot(&ident, &bot.name)?;
    Ok((StatusCode::CREATED, axum::Json(key)).into_response())
}

async fn delete_bot(
    State(api): State<Api>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let ident = api.signed_in(&headers)?;
    api.arena.delete_bot(&ident, &name)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn new_bot_key(
    State(api): State<Api>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Json<BotKey> {
    let ident = api.signed_in(&headers)?;
    Ok(axum::Json(api.arena.new_bot_key(&ident, &name)?))
}

async fn lobby(State(api): State<Api>, headers: HeaderMap) -> axum::Json<Lobby> {
    axum::Json(api.arena.lobby(api.ident(&headers).as_ref()))
}

/// The lobby's change feed. While it is open, its account is connected.
async fn lobby_events(State(api): State<Api>, headers: HeaderMap) -> Events {
    let held = api.ident(&headers).map(|ident| api.arena.attend(&ident));
    let (version, chat) = api.arena.subscribe();
    let change = |version, chat| LobbyChange { version, chat };
    feed(&api, version, chat, held, change)
}

async fn new_table(State(api): State<Api>, headers: HeaderMap, req: Bytes) -> Result<Response> {
    let ident = api.signed_in(&headers)?;
    let new: NewTable = body_or_default(&req)?;
    let table = api.arena.new_table(&ident, &new)?;
    Ok((StatusCode::CREATED, axum::Json(table)).into_response())
}

async fn table(State(api): State<Api>, Path(id): Path<String>) -> Json<TableInfo> {
    Ok(axum::Json(api.arena.table(&id)?))
}

async fn set_spec(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Bytes,
) -> Json<TableInfo> {
    let ident = api.signed_in(&headers)?;
    let spec: TableSpec = body(&req)?;
    Ok(axum::Json(api.arena.set_spec(&ident, &id, &spec)?))
}

async fn set_seats(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Bytes,
) -> Json<TableInfo> {
    let ident = api.signed_in(&headers)?;
    let seats: Vec<SeatEdit> = body(&req)?;
    Ok(axum::Json(api.arena.set_seats(&ident, &id, &seats)?))
}

async fn join_table(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Bytes,
) -> Json<TableInfo> {
    let ident = api.signed_in(&headers)?;
    let join: JoinTable = body_or_default(&req)?;
    Ok(axum::Json(api.arena.join_table(&ident, &id, join.seat)?))
}

async fn leave_table(
    State(api): State<Api>,
    Path(_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let ident = api.signed_in(&headers)?;
    api.arena.leave_table(&ident);
    Ok(StatusCode::NO_CONTENT)
}

async fn ready(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Bytes,
) -> Json<TableInfo> {
    let ident = api.signed_in(&headers)?;
    let ready: Ready = body(&req)?;
    let arena = api.arena.clone();
    // Setting a game up takes a moment.
    blocking(move || arena.ready(&ident, &id, ready.ready))
        .await
        .map(axum::Json)
}

async fn search(State(api): State<Api>, headers: HeaderMap, req: Bytes) -> Json<QueueStatus> {
    let ident = api.signed_in(&headers)?;
    let request: MatchRequest = body_or_default(&req)?;
    let arena = api.arena.clone();
    blocking(move || arena.search(&ident, &request))
        .await
        .map(axum::Json)
}

async fn cancel_search(State(api): State<Api>, headers: HeaderMap) -> Result<StatusCode> {
    let ident = api.signed_in(&headers)?;
    api.arena.cancel_search(&ident);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct ChatQuery {
    /// Only the messages numbered after this.
    #[serde(default)]
    after: u64,
    token: Option<String>,
    seat: Option<u8>,
}

/// Who a request is in a game's chat: a seat, or someone watching.
enum Speaker {
    Seat(Seat),
    Watcher(Ident),
}

fn speaker(api: &Api, game: &str, headers: &HeaderMap, q: &ChatQuery) -> Result<Speaker> {
    match api.seat(game, headers, q.token.as_deref(), q.seat) {
        Ok(seat) => Ok(Speaker::Seat(seat)),
        Err(refused) => {
            let watchable = api.hub.meta(game).is_ok_and(|m| m.watchable);
            let watcher = api.ident(headers).filter(|_| watchable);
            watcher.map(Speaker::Watcher).ok_or(refused)
        }
    }
}

async fn chat_log(
    State(api): State<Api>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    chat_query: Queried<ChatQuery>,
) -> Json<ChatLog> {
    let q = query(chat_query)?;
    if let Some(game) = channel.strip_prefix("game-")
        && !api.hub.meta(game)?.watchable
    {
        speaker(&api, game, &headers, &q)?;
    }
    Ok(axum::Json(api.arena.chat_log(&channel, q.after)))
}

async fn say(
    State(api): State<Api>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    chat_query: Queried<ChatQuery>,
    req: Bytes,
) -> Result<StatusCode> {
    let q = query(chat_query)?;
    let said: Say = body(&req)?;
    match channel.strip_prefix("game-") {
        None => api
            .arena
            .say(&api.signed_in(&headers)?, &channel, &said.text)?,
        Some(game) => match speaker(&api, game, &headers, &q)? {
            Speaker::Seat(seat) => {
                let who = format!("{game}/{}", seat.seat.0);
                let name = api.hub.seat_name(&seat)?;
                (api.arena).say_in_game(game, &who, &name, ChatKind::Player, &said.text)?
            }
            Speaker::Watcher(ident) => {
                let name = api.arena.name_of(&ident)?;
                (api.arena).say_in_game(game, &ident.user, &name, ChatKind::Watcher, &said.text)?
            }
        },
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn profile(State(api): State<Api>, Path(name): Path<String>) -> Json<Profile> {
    Ok(axum::Json(api.arena.profile(&name)?))
}

#[derive(Deserialize, Default)]
struct ListQuery {
    limit: Option<usize>,
}

async fn leaderboard(State(api): State<Api>, list: Queried<ListQuery>) -> Json<Leaderboard> {
    let limit = query(list)?.limit.unwrap_or(50);
    Ok(axum::Json(api.arena.leaderboard(limit)?))
}

async fn history(State(api): State<Api>, q: Queried<HistoryQuery>) -> Json<History> {
    Ok(axum::Json(api.arena.history(&query(q)?)?))
}
