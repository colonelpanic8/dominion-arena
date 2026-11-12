//! A typed client for `dominion serve`'s HTTP API, on the page's own origin.

use crate::platform::{self, encode};
use dominion_protocol::Answer;
use dominion_protocol::api::{
    CardInfo, Change, Created, DealRequest, Dealt, GameSummary, NewGame, SeatInfo, SeatView,
    WebSettings,
};
use dominion_protocol::lobby::{
    BotKey, ChatLog, Credentials, GuestRequest, History, JoinTable, Leaderboard, Lobby,
    LobbyChange, MatchRequest, Me, MyGame, NewBot, NewPassword, NewTable, Profile, QueueStatus,
    Ready, Say, SeatEdit, TableInfo, TableSpec,
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

    /// Nobody is signed in (or the seat token is wrong).
    pub fn is_unauthorized(&self) -> bool {
        self.status == 401
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

/// Replaces your Banned cards list (signed in).
pub async fn set_banned_cards(cards: &[String]) -> Result<WebSettings> {
    imp::send("PUT", "/api/settings/banned-cards", None, Some(&cards)).await
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

/// Takes back the seat's last answer; the view returned has the whole log.
pub async fn undo(game: &str, token: &str) -> Result<SeatView> {
    let url = format!("/api/games/{}/undo", encode(game));
    imp::send("POST", &url, Some(token), None::<&()>).await
}

/// Follows a change feed until the future is dropped, calling `on_change` for every
/// `change` event (one on each connect). Reconnects with backoff.
async fn follow<T: serde::de::DeserializeOwned>(url: &str, mut on_change: impl FnMut(T)) {
    let mut delay = 500;
    loop {
        if imp::follow(url, &mut |data| {
            if let Ok(change) = serde_json::from_str(data) {
                delay = 500;
                on_change(change)
            }
        })
        .await
        .is_err()
        {
            platform::sleep(delay).await;
            delay = (delay * 2).min(30_000);
        }
    }
}

/// Follows a game's change feed for a seat.
pub async fn watch(game: &str, token: &str, on_change: impl FnMut(Change)) {
    let url = format!("/api/games/{}/events?token={}", encode(game), encode(token));
    follow(&url, on_change).await
}

/// Follows a game's change feed as someone watching (signed in).
pub async fn watch_unseated(game: &str, on_change: impl FnMut(Change)) {
    follow(&format!("/api/games/{}/events", encode(game)), on_change).await
}

/// The game as someone watching sees it (signed in; the game must be watchable).
pub async fn watch_view(game: &str, log_from: Option<usize>) -> Result<SeatView> {
    let mut url = format!("/api/games/{}/watch", encode(game));
    if let Some(n) = log_from {
        url.push_str(&format!("?logFrom={n}"));
    }
    imp::get(&url, None).await
}

pub async fn resign(game: &str, token: &str) -> Result<SeatView> {
    let url = format!("/api/games/{}/resign", encode(game));
    imp::send("POST", &url, Some(token), None::<&()>).await
}

// Accounts. A session is a cookie the browser keeps and sends by itself.

pub async fn me() -> Result<Me> {
    imp::get("/api/me", None).await
}

pub async fn register(name: &str, password: &str) -> Result<Me> {
    let credentials = Credentials {
        name: name.into(),
        password: password.into(),
    };
    imp::post("/api/auth/register", None, &credentials).await
}

pub async fn login(name: &str, password: &str) -> Result<Me> {
    let credentials = Credentials {
        name: name.into(),
        password: password.into(),
    };
    imp::post("/api/auth/login", None, &credentials).await
}

/// A guest account under `name`, or under a name the server makes up.
pub async fn guest(name: Option<&str>) -> Result<Me> {
    let request = GuestRequest {
        name: name.map(String::from),
    };
    imp::post("/api/auth/guest", None, &request).await
}

pub async fn logout() -> Result<()> {
    imp::send_empty("POST", "/api/auth/logout", None, None::<&()>).await
}

pub async fn set_password(current: &str, new: &str) -> Result<()> {
    let passwords = NewPassword {
        current: current.into(),
        new: new.into(),
    };
    imp::send_empty("PUT", "/api/me/password", None, Some(&passwords)).await
}

/// The seats this account plays, with their tokens.
pub async fn my_games() -> Result<Vec<MyGame>> {
    imp::get("/api/me/games", None).await
}

pub async fn new_bot(name: &str) -> Result<BotKey> {
    imp::post("/api/me/bots", None, &NewBot { name: name.into() }).await
}

pub async fn new_bot_key(name: &str) -> Result<BotKey> {
    let url = format!("/api/me/bots/{}/key", encode(name));
    imp::send("POST", &url, None, None::<&()>).await
}

pub async fn delete_bot(name: &str) -> Result<()> {
    let url = format!("/api/me/bots/{}", encode(name));
    imp::send_empty("DELETE", &url, None, None::<&()>).await
}

// The lobby.

pub async fn lobby() -> Result<Lobby> {
    imp::get("/api/lobby", None).await
}

/// Follows the lobby's change feed; while it is open this account counts as connected.
pub async fn watch_lobby(on_change: impl FnMut(LobbyChange)) {
    follow("/api/lobby/events", on_change).await
}

pub async fn new_table(table: &NewTable) -> Result<TableInfo> {
    imp::post("/api/tables", None, table).await
}

pub async fn table(id: &str) -> Result<TableInfo> {
    imp::get(&format!("/api/tables/{}", encode(id)), None).await
}

pub async fn set_table_spec(id: &str, spec: &TableSpec) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/spec", encode(id));
    imp::send("PUT", &url, None, Some(spec)).await
}

pub async fn set_table_seats(id: &str, seats: &[SeatEdit]) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/seats", encode(id));
    imp::send("PUT", &url, None, Some(&seats)).await
}

pub async fn join_table(id: &str, seat: Option<u8>) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/join", encode(id));
    imp::post(&url, None, &JoinTable { seat }).await
}

pub async fn leave_table(id: &str) -> Result<()> {
    let url = format!("/api/tables/{}/leave", encode(id));
    imp::send_empty("POST", &url, None, None::<&()>).await
}

/// Says whether you are ready; the table's `game` is set once that has started.
pub async fn ready(id: &str, ready: bool) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/ready", encode(id));
    imp::post(&url, None, &Ready { ready }).await
}

pub async fn search(request: &MatchRequest) -> Result<QueueStatus> {
    imp::post("/api/automatch", None, request).await
}

pub async fn cancel_search() -> Result<()> {
    imp::send_empty("DELETE", "/api/automatch", None, None::<&()>).await
}

/// A chat channel's messages numbered after `after`: `lobby`, `table-<id>` or
/// `game-<id>` (with `token`, as that seat; else as the signed-in account).
pub async fn chat(channel: &str, after: u64, token: Option<&str>) -> Result<ChatLog> {
    let url = format!("/api/chat/{}?after={after}", encode(channel));
    imp::get(&url, token).await
}

pub async fn say(channel: &str, text: &str, token: Option<&str>) -> Result<()> {
    let url = format!("/api/chat/{}", encode(channel));
    imp::send_empty("POST", &url, token, Some(&Say { text: text.into() })).await
}

pub async fn profile(name: &str) -> Result<Profile> {
    imp::get(&format!("/api/users/{}", encode(name)), None).await
}

pub async fn leaderboard() -> Result<Leaderboard> {
    imp::get("/api/leaderboard", None).await
}

/// Finished games that ended before `before`, newest first: `user`'s, or everyone's.
pub async fn history(user: Option<&str>, before: Option<u64>) -> Result<History> {
    let mut url = "/api/history?limit=20".to_string();
    if let Some(user) = user {
        url.push_str(&format!("&user={}", encode(user)));
    }
    if let Some(before) = before {
        url.push_str(&format!("&before={before}"));
    }
    imp::get(&url, None).await
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

    async fn perform(
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> Result<String> {
        let req = match method {
            "POST" => Request::post(url),
            "PUT" => Request::put(url),
            "DELETE" => Request::delete(url),
            _ => Request::get(url),
        };
        let req = auth(req, token);
        let req = match body {
            Some(body) => req.json(body),
            None => req.build(),
        }
        .map_err(unreachable)?;
        let resp = req.send().await.map_err(unreachable)?;
        let status = resp.status();
        let text = resp.text().await.map_err(unreachable)?;
        if !resp.ok() {
            return Err(error_message(status, &text));
        }
        Ok(text)
    }

    /// A request, with or without a body, whose answer is JSON.
    pub async fn send<T: DeserializeOwned>(
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let text = perform(method, url, token, body).await?;
        serde_json::from_str(&text).map_err(|e| ApiError {
            status: 0,
            message: format!("unreadable response: {e}"),
        })
    }

    /// A request whose answer has no body.
    pub async fn send_empty(
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> Result<()> {
        perform(method, url, token, body).await.map(|_| ())
    }

    pub async fn get<T: DeserializeOwned>(url: &str, token: Option<&str>) -> Result<T> {
        send("GET", url, token, None::<&()>).await
    }

    pub async fn post<T: DeserializeOwned>(
        url: &str,
        token: Option<&str>,
        body: &impl Serialize,
    ) -> Result<T> {
        send("POST", url, token, Some(body)).await
    }

    /// Reads one connection's events, giving each `change` event's data to `on_change`;
    /// returns when it drops.
    pub async fn follow(url: &str, on_change: &mut dyn FnMut(&str)) -> std::result::Result<(), ()> {
        let mut source = EventSource::new(url).map_err(|_| ())?;
        let mut changes = source.subscribe("change").map_err(|_| ())?;
        while let Some(Ok((_, event))) = changes.next().await {
            if let Some(data) = event.data().as_string() {
                on_change(&data);
            }
        }
        source.close();
        Err(())
    }
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
mod imp {
    use super::{ApiError, Result};
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    fn unavailable<T>() -> Result<T> {
        Err(ApiError {
            status: 0,
            message: "the API client only runs in a browser".into(),
        })
    }

    pub async fn send<T: DeserializeOwned>(
        _method: &str,
        _url: &str,
        _token: Option<&str>,
        _body: Option<&impl Serialize>,
    ) -> Result<T> {
        unavailable()
    }

    pub async fn send_empty(
        _method: &str,
        _url: &str,
        _token: Option<&str>,
        _body: Option<&impl Serialize>,
    ) -> Result<()> {
        unavailable()
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

    pub async fn follow(
        _url: &str,
        _on_change: &mut dyn FnMut(&str),
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
