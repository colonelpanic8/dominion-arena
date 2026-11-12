//! The HTTP side of the server: the JSON API at `/api` (seats act through their token,
//! like MCP agents), the asset cache at `/assets` and the web client everywhere else.

use crate::Options;
use crate::hub::{Hub, HubError, Seat};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header, uri::Authority};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use dominion_agent::{Answer, AnswerError};
use dominion_engine::{Card, Kind};
use dominion_protocol::api::{
    ApiError, CardCost, CardInfo, Change, NewGame, SeatInfo, SeatView, ViewQuery,
};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};

/// A year: asset files never change under their path.
const ASSETS_CACHE: &str = "public, max-age=31536000, immutable";

/// The routes besides `/mcp`, all behind the Host check that guards `/mcp`.
pub fn router(
    hub: Arc<Hub>,
    options: &Options,
    allowed_hosts: Vec<String>,
    stop: CancellationToken,
) -> Router {
    let api = Router::new()
        .route("/cards", get(cards))
        .route("/games", get(list).post(create))
        .route("/seat", get(seat))
        .route("/games/{id}/view", get(view))
        .route("/games/{id}/answer", axum::routing::post(answer))
        .route("/games/{id}/events", get(events))
        .fallback(|| async { Error(StatusCode::NOT_FOUND, "no such route".into()) })
        .method_not_allowed_fallback(|| async {
            Error(StatusCode::METHOD_NOT_ALLOWED, "method not allowed".into())
        })
        .with_state(Api { hub, stop });
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
    .layer(middleware::map_response(cache_forever));
    let site = Router::new()
        .nest("/api", api)
        .nest_service("/assets", assets);
    let site = match &options.web_dir {
        Some(dir) => site
            .fallback_service(ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => site.route("/", get(no_client)),
    };
    site.layer(middleware::from_fn_with_state(
        Arc::new(allowed_hosts),
        check_host,
    ))
}

async fn no_client() -> &'static str {
    "dominion: the MCP endpoint is /mcp; the web client isn't installed here.\n"
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

async fn check_host(State(allowed): State<Arc<Vec<String>>>, req: Request, next: Next) -> Response {
    let host = match req.headers().get(header::HOST) {
        Some(h) => h.to_str().ok().and_then(|h| Authority::try_from(h).ok()),
        None => req.uri().authority().cloned(),
    };
    match host {
        None => Error(
            StatusCode::BAD_REQUEST,
            "missing or invalid Host header".into(),
        )
        .into_response(),
        Some(h) if !host_allowed(&h, &allowed) => {
            Error(StatusCode::FORBIDDEN, "Host header is not allowed".into()).into_response()
        }
        Some(_) => next.run(req).await,
    }
}

#[derive(Clone)]
struct Api {
    hub: Arc<Hub>,
    stop: CancellationToken,
}

struct Error(StatusCode, String);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, axum::Json(ApiError { error: self.1 })).into_response()
    }
}

impl From<HubError> for Error {
    fn from(e: HubError) -> Error {
        let status = match &e {
            HubError::BadRequest(_) => StatusCode::BAD_REQUEST,
            HubError::UnknownGame(_) => StatusCode::NOT_FOUND,
            HubError::BadToken(_) => StatusCode::UNAUTHORIZED,
            HubError::Answer(AnswerError::Invalid(_)) => StatusCode::UNPROCESSABLE_ENTITY,
            HubError::Answer(_) => StatusCode::CONFLICT,
            HubError::Engine(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Error(status, e.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

fn body<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| {
        Error(
            StatusCode::BAD_REQUEST,
            format!("invalid request body: {e}"),
        )
    })
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

fn token<'a>(headers: &'a HeaderMap, query: Option<&'a str>) -> Result<&'a str> {
    bearer(headers)
        .or(query)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| Error(StatusCode::UNAUTHORIZED, "missing seat token".into()))
}

fn claim(api: &Api, id: &str, token: &str) -> Result<Seat> {
    Ok(api.hub.claim(id, token)?)
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

async fn list(State(api): State<Api>) -> impl IntoResponse {
    axum::Json(api.hub.list())
}

async fn create(State(api): State<Api>, req: Bytes) -> Result<impl IntoResponse> {
    let req: NewGame = body(&req)?;
    let created = api.hub.create(&req)?;
    Ok((StatusCode::CREATED, axum::Json(created)))
}

async fn seat(State(api): State<Api>, headers: HeaderMap) -> Result<axum::Json<SeatInfo>> {
    let token = token(&headers, None)?;
    let seat = api
        .hub
        .find(token)
        .map_err(|_| Error(StatusCode::UNAUTHORIZED, "unknown seat token".into()))?;
    Ok(axum::Json(SeatInfo {
        name: api.hub.seat_name(&seat)?,
        game_id: seat.game,
        seat: seat.seat.0,
    }))
}

fn seat_view(api: &Api, seat: &Seat, log_from: Option<usize>) -> Result<axum::Json<SeatView>> {
    Ok(axum::Json(api.hub.seat_view(seat, log_from)?))
}

async fn view(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    query: std::result::Result<Query<ViewQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<axum::Json<SeatView>> {
    let Query(query) = query.map_err(|e| Error(StatusCode::BAD_REQUEST, e.body_text()))?;
    let seat = claim(&api, &id, token(&headers, None)?)?;
    seat_view(&api, &seat, query.log_from)
}

async fn answer(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Bytes,
) -> Result<axum::Json<SeatView>> {
    let seat = claim(&api, &id, token(&headers, None)?)?;
    let answer: Answer = body(&req)?;
    api.hub.answer(&seat, &answer)?;
    seat_view(&api, &seat, None)
}

#[derive(Deserialize)]
struct EventsQuery {
    token: Option<String>,
}

async fn events(
    State(api): State<Api>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let seat = claim(&api, &id, token(&headers, query.token.as_deref())?)?;
    let rx = api.hub.subscribe(&seat.game)?;
    let changes = futures_util::stream::unfold((rx, true), |(mut rx, first)| async move {
        if !first && rx.changed().await.is_err() {
            return None;
        }
        let version = *rx.borrow_and_update();
        let data = serde_json::to_string(&Change { version }).unwrap_or_default();
        let event = Event::default().event("change").data(data);
        Some((Ok(event), (rx, false)))
    });
    let stream = changes.take_until(api.stop.cancelled_owned());
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
