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
    routing::get,
};
use dominion_protocol::api::OnlineObservation;
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
}
impl Hosted {
    pub async fn start(config: &Config) -> Result<Self, String> {
        let owner = config
            .online_owner
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .ok_or("online_enabled requires online_owner (a registered Arena account name)")?
            .clone();
        Ok(Self {
            client: super::start(config).await?,
            owner,
        })
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
            move || Ok(mcp::Session::new(client.clone())),
            Arc::new(sessions),
            StreamableHttpServerConfig::default()
                .with_allowed_hosts(hosts.clone())
                .with_cancellation_token(stop.child_token()),
        );
        let auth = Access {
            arena,
            owner: self.owner.clone(),
        };
        Router::new()
            .route("/api/online/view", get(snapshot))
            .route("/api/online/events", get(events))
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
    if let Err(status) = access.allowed(request.headers()) {
        return (
            status,
            axum::Json(dominion_protocol::api::ApiError {
                error: "sign in as the configured official-game owner".into(),
            }),
        )
            .into_response();
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
