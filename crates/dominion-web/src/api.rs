//! A typed client for `dominion serve`'s HTTP API, on the page's own origin.

use crate::platform::{self, encode};
use dominion_protocol::Answer;
use dominion_protocol::api::{
    CardInfo, Change, Created, DealRequest, Dealt, GameSummary, NewGame, SeatInfo, SeatView,
    WebSettings,
};
use std::fmt;

/// A failed call: the HTTP status (0 when the server couldn't be reached or answered
/// with something unreadable) and the server's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    /// The answer named a decision that is no longer open, or it isn't this seat's.
    pub fn is_stale(&self) -> bool {
        self.status == 409
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.status == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{} (HTTP {})", self.message, self.status)
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

pub async fn cards() -> Result<Vec<CardInfo>> {
    imp::get("/api/cards", None).await
}

pub async fn settings() -> Result<WebSettings> {
    imp::get("/api/settings", None).await
}

pub async fn games() -> Result<Vec<GameSummary>> {
    imp::get("/api/games", None).await
}

pub async fn create_game(req: &NewGame) -> Result<Created> {
    imp::post("/api/games", None, req).await
}

/// The kingdom a table's rules deal, without creating a game.
pub async fn deal(req: &DealRequest) -> Result<Dealt> {
    imp::post("/api/deal", None, req).await
}

/// Replaces the Banned cards list in the server's settings file.
pub async fn set_banned_cards(cards: &[String]) -> Result<WebSettings> {
    imp::put("/api/settings/banned-cards", &cards).await
}

pub async fn seat(token: &str) -> Result<SeatInfo> {
    imp::get("/api/seat", Some(token)).await
}

/// A seat's view; with `log_from` its log starts at that sequence number.
pub async fn view(game: &str, token: &str, log_from: Option<usize>) -> Result<SeatView> {
    let mut url = format!("/api/games/{}/view", encode(game));
    if let Some(n) = log_from {
        url.push_str(&format!("?logFrom={n}"));
    }
    imp::get(&url, Some(token)).await
}

pub async fn answer(game: &str, token: &str, answer: &Answer) -> Result<SeatView> {
    let url = format!("/api/games/{}/answer", encode(game));
    imp::post(&url, Some(token), answer).await
}

/// Follows a game's change feed until the future is dropped, calling `on_change` for
/// every `change` event (one on each connect). Reconnects with backoff.
pub async fn watch(game: &str, token: &str, mut on_change: impl FnMut(Change)) {
    let url = format!("/api/games/{}/events?token={}", encode(game), encode(token));
    let mut delay = 500;
    loop {
        if imp::follow(&url, &mut |c| {
            delay = 500;
            on_change(c)
        })
        .await
        .is_err()
        {
            platform::sleep(delay).await;
            delay = (delay * 2).min(30_000);
        }
    }
}

#[cfg_attr(not(all(feature = "web", target_arch = "wasm32")), allow(dead_code))]
fn error_message(status: u16, body: &str) -> ApiError {
    let message = serde_json::from_str::<dominion_protocol::api::ApiError>(body)
        .map(|e| e.error)
        .unwrap_or_else(|_| {
            if body.trim().is_empty() {
                format!("the server answered HTTP {status}")
            } else {
                body.trim().to_string()
            }
        });
    ApiError { status, message }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod imp {
    use super::{ApiError, Result, error_message};
    use dominion_protocol::api::Change;
    use futures_util::StreamExt;
    use gloo_net::eventsource::futures::EventSource;
    use gloo_net::http::{Request, RequestBuilder};
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    fn unreachable(e: impl std::fmt::Display) -> ApiError {
        ApiError {
            status: 0,
            message: format!("could not reach the server: {e}"),
        }
    }

    fn auth(req: RequestBuilder, token: Option<&str>) -> RequestBuilder {
        match token {
            Some(t) => req.header("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }

    async fn send<T: DeserializeOwned>(req: Request) -> Result<T> {
        let resp = req.send().await.map_err(unreachable)?;
        let status = resp.status();
        let body = resp.text().await.map_err(unreachable)?;
        if !resp.ok() {
            return Err(error_message(status, &body));
        }
        serde_json::from_str(&body).map_err(|e| ApiError {
            status: 0,
            message: format!("unreadable response: {e}"),
        })
    }

    pub async fn get<T: DeserializeOwned>(url: &str, token: Option<&str>) -> Result<T> {
        send(
            auth(Request::get(url), token)
                .build()
                .map_err(unreachable)?,
        )
        .await
    }

    pub async fn post<T: DeserializeOwned>(
        url: &str,
        token: Option<&str>,
        body: &impl Serialize,
    ) -> Result<T> {
        send(
            auth(Request::post(url), token)
                .json(body)
                .map_err(unreachable)?,
        )
        .await
    }

    pub async fn put<T: DeserializeOwned>(url: &str, body: &impl Serialize) -> Result<T> {
        send(Request::put(url).json(body).map_err(unreachable)?).await
    }

    /// Reads one connection's events; returns when it drops.
    pub async fn follow(
        url: &str,
        on_change: &mut dyn FnMut(Change),
    ) -> std::result::Result<(), ()> {
        let mut source = EventSource::new(url).map_err(|_| ())?;
        let mut changes = source.subscribe("change").map_err(|_| ())?;
        while let Some(Ok((_, event))) = changes.next().await {
            if let Some(change) = event
                .data()
                .as_string()
                .and_then(|d| serde_json::from_str::<Change>(&d).ok())
            {
                on_change(change);
            }
        }
        source.close();
        Err(())
    }
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
mod imp {
    use super::{ApiError, Result};
    use dominion_protocol::api::Change;
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    fn unavailable<T>() -> Result<T> {
        Err(ApiError {
            status: 0,
            message: "the API client only runs in a browser".into(),
        })
    }

    pub async fn get<T: DeserializeOwned>(_url: &str, _token: Option<&str>) -> Result<T> {
        unavailable()
    }

    pub async fn post<T: DeserializeOwned>(
        _url: &str,
        _token: Option<&str>,
        _body: &impl Serialize,
    ) -> Result<T> {
        unavailable()
    }

    pub async fn put<T: DeserializeOwned>(_url: &str, _body: &impl Serialize) -> Result<T> {
        unavailable()
    }

    pub async fn follow(
        _url: &str,
        _on_change: &mut dyn FnMut(Change),
    ) -> std::result::Result<(), ()> {
        std::future::pending().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_keep_the_server_message() {
        let e = error_message(
            409,
            r#"{"error":"decision d3 is not open; the open decision is d4"}"#,
        );
        assert_eq!(e.status, 409);
        assert!(e.is_stale());
        assert_eq!(
            e.message,
            "decision d3 is not open; the open decision is d4"
        );
        assert_eq!(error_message(502, "Bad Gateway").message, "Bad Gateway");
    }
}
