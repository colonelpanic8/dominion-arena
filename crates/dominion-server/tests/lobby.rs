//! The account and lobby routes of the HTTP API.

use dominion_protocol::api::{SeatView, WebSettings};
use dominion_protocol::lobby::{ChatLog, Lobby, Me, QueueState, QueueStatus, UserKind};
use dominion_server::Options;
use dominion_server::hub::Hub;
use futures_util::StreamExt;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

struct Server {
    base: String,
    http: reqwest::Client,
    stop: CancellationToken,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

struct Reply {
    status: StatusCode,
    headers: reqwest::header::HeaderMap,
    text: String,
}

impl Reply {
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|e| panic!("{e}: {} {}", self.status, self.text))
    }

    fn error(&self, status: StatusCode) -> String {
        assert_eq!(self.status, status, "{}", self.text);
        self.json::<Value>()["error"].as_str().unwrap().to_string()
    }

    fn ok(self) -> Reply {
        assert!(self.status.is_success(), "{} {}", self.status, self.text);
        self
    }

    /// The session token of the cookie the reply sets.
    fn session(&self) -> String {
        let cookie = self.headers["set-cookie"].to_str().unwrap();
        assert!(
            cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"),
            "{cookie}"
        );
        let (name, value) = cookie.split(';').next().unwrap().split_once('=').unwrap();
        assert_eq!(name, "dominion_session");
        value.to_string()
    }
}

async fn start() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    tokio::spawn(dominion_server::serve(
        listener,
        Arc::new(Hub::default()),
        Options::default(),
        stop.clone(),
    ));
    Server {
        base,
        http: reqwest::Client::new(),
        stop,
    }
}

impl Server {
    async fn call(&self, method: Method, path: &str, token: Option<&str>, body: Value) -> Reply {
        let mut req = self.http.request(method, format!("{}{path}", self.base));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        if !body.is_null() {
            req = req.body(body.to_string());
        }
        let res = req.send().await.unwrap();
        Reply {
            status: res.status(),
            headers: res.headers().clone(),
            text: res.text().await.unwrap(),
        }
    }

    async fn get(&self, path: &str, token: Option<&str>) -> Reply {
        self.call(Method::GET, path, token, Value::Null).await
    }

    async fn post(&self, path: &str, token: Option<&str>, body: Value) -> Reply {
        self.call(Method::POST, path, token, body).await
    }

    /// A registered account's session token.
    async fn register(&self, name: &str) -> String {
        let body = json!({ "name": name, "password": "correct horse" });
        let reply = self.post("/api/auth/register", None, body).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
        reply.session()
    }
}

#[tokio::test]
async fn a_session_is_a_cookie_or_a_bearer_token() {
    let server = start().await;
    server
        .get("/api/me", None)
        .await
        .error(StatusCode::UNAUTHORIZED);
    let ada = server.register("Ada").await;
    let me: Me = server.get("/api/me", Some(&ada)).await.json();
    assert_eq!(
        (me.user.name.as_str(), me.user.kind),
        ("Ada", UserKind::Registered)
    );
    let by_cookie = server
        .http
        .get(format!("{}/api/me", server.base))
        .header("cookie", format!("theme=dark; dominion_session={ada}"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_cookie.status(), StatusCode::OK);
    let wrong = json!({ "name": "Ada", "password": "not this one" });
    let refused = server.post("/api/auth/login", None, wrong).await;
    assert_eq!(
        refused.error(StatusCode::UNAUTHORIZED),
        "wrong name or password"
    );
    let right = json!({ "name": "ada", "password": "correct horse" });
    let second = server
        .post("/api/auth/login", None, right)
        .await
        .ok()
        .session();
    assert_ne!(second, ada);
    let out = server
        .post("/api/auth/logout", Some(&second), Value::Null)
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(
        out.headers["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    server
        .get("/api/me", Some(&second))
        .await
        .error(StatusCode::UNAUTHORIZED);
    let taken = json!({ "name": "ADA", "password": "another password" });
    let conflict = server.post("/api/auth/register", None, taken).await;
    assert!(conflict.error(StatusCode::CONFLICT).contains("taken"));
    let guest = server.post("/api/auth/guest", None, Value::Null).await;
    assert_eq!(guest.status, StatusCode::CREATED, "{}", guest.text);
    let me: Me = guest.json();
    assert_eq!(me.user.kind, UserKind::Guest);
    assert!(me.user.name.starts_with("Guest "), "{}", me.user.name);
}

#[tokio::test]
async fn another_sites_page_cannot_act_for_a_session() {
    let server = start().await;
    let ada = server.register("Ada").await;
    let from = |origin: &str| {
        server
            .http
            .post(format!("{}/api/tables", server.base))
            .header("cookie", format!("dominion_session={ada}"))
            .header("origin", origin.to_string())
            .send()
    };
    let foreign = from("https://evil.example").await.unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    let own = from(&server.base).await.unwrap();
    assert_eq!(own.status(), StatusCode::CREATED);
    // A development server on this machine, proxying here.
    let dev = from("http://localhost:8080").await.unwrap();
    assert_eq!(dev.status(), StatusCode::CREATED);
    let big = server
        .post(
            "/api/chat/lobby",
            Some(&ada),
            json!({ "text": "x".repeat(70_000) }),
        )
        .await;
    assert_eq!(big.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn the_banned_list_is_set_by_whoever_is_signed_in() {
    let server = start().await;
    async fn put(server: &Server, token: Option<&str>, body: Value) -> Reply {
        let path = "/api/settings/banned-cards";
        server.call(Method::PUT, path, token, body).await
    }
    let refused = put(&server, None, json!(["Witch"])).await;
    refused.error(StatusCode::UNAUTHORIZED);
    let ada = server.register("Ada").await;
    let set = put(&server, Some(&ada), json!(["witch", "Delve"])).await;
    let mine: WebSettings = set.ok().json();
    assert_eq!(mine.banned_cards, ["Witch", "Delve"]);
    let served: WebSettings = server.get("/api/settings", Some(&ada)).await.json();
    assert_eq!(served.banned_cards, ["Witch", "Delve"]);
    let anyone: WebSettings = server.get("/api/settings", None).await.json();
    assert!(anyone.banned_cards.is_empty());
    assert!(anyone.registration && anyone.guests);
    let bad = put(&server, Some(&ada), json!(["Copper"])).await;
    assert!(bad.error(StatusCode::BAD_REQUEST).contains("Copper"));
}

/// The data of the next `change` event of an open feed.
async fn next_change(
    feed: &mut (impl futures_util::Stream<Item = reqwest::Result<impl AsRef<[u8]>>> + Unpin),
) -> Value {
    let mut text = String::new();
    loop {
        let chunk = timeout(Duration::from_secs(5), feed.next()).await.unwrap();
        text.push_str(std::str::from_utf8(chunk.unwrap().unwrap().as_ref()).unwrap());
        if let Some(event) = text.split("\n\n").find(|e| e.contains("event: change")) {
            let data = event
                .lines()
                .find_map(|l| l.strip_prefix("data: "))
                .unwrap();
            return serde_json::from_str(data).unwrap();
        }
    }
}

#[tokio::test]
async fn two_people_are_matched_and_play_through_their_sessions() {
    let server = start().await;
    let (ada, bo) = (server.register("Ada").await, server.register("Bo").await);
    let mut feed = server
        .http
        .get(format!("{}/api/lobby/events", server.base))
        .bearer_auth(&ada)
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let first = next_change(&mut feed).await;
    let searching: QueueStatus = server
        .post("/api/automatch", Some(&ada), Value::Null)
        .await
        .json();
    assert_eq!(searching.state, QueueState::Searching);
    let lobby: Lobby = server.get("/api/lobby", Some(&bo)).await.json();
    assert_eq!((lobby.online, lobby.searching.humans), (2, 1));
    assert_eq!(lobby.you.unwrap().user.name, "Bo");
    let found: QueueStatus = server
        .post("/api/automatch", Some(&bo), json!({ "players": [2, 3] }))
        .await
        .json();
    let game = found.game_id.unwrap();
    let changed = next_change(&mut feed).await;
    assert!(
        changed["version"].as_u64() > first["version"].as_u64(),
        "{changed}"
    );
    let lobby: Lobby = server.get("/api/lobby", Some(&ada)).await.json();
    assert_eq!(lobby.queue.unwrap().game_id.as_deref(), Some(game.as_str()));
    assert_eq!(lobby.games[0].opponents, ["Bo"]);

    // The session stands in for the seat's token.
    let path = format!("/api/games/{game}");
    let mine: SeatView = server.get(&format!("{path}/view"), Some(&ada)).await.json();
    assert_eq!(mine.view.you.as_ref().unwrap().name, "Ada");
    let users: Vec<String> = (mine.meta.users.iter())
        .map(|u| u.as_ref().unwrap().name.clone())
        .collect();
    assert!(users.contains(&"Ada".to_string()) && users.contains(&"Bo".to_string()));
    assert!(mine.meta.rated && mine.meta.timed);
    let stranger = server.register("Cy").await;
    let refused = server.get(&format!("{path}/view"), Some(&stranger)).await;
    assert!(refused.error(StatusCode::FORBIDDEN).contains("no seat"));
    server
        .get(&format!("{path}/view"), None)
        .await
        .error(StatusCode::UNAUTHORIZED);

    let chat = format!("/api/chat/game-{game}");
    let said = server
        .post(&chat, Some(&bo), json!({ "text": "glhf" }))
        .await;
    assert_eq!(said.status, StatusCode::NO_CONTENT, "{}", said.text);
    let log: ChatLog = server.get(&chat, Some(&ada)).await.json();
    assert_eq!(
        (log.messages[0].from.as_str(), log.messages[0].text.as_str()),
        ("Bo", "glhf")
    );
    server
        .get(&chat, None)
        .await
        .error(StatusCode::UNAUTHORIZED);

    let over: SeatView = server
        .post(&format!("{path}/resign"), Some(&bo), Value::Null)
        .await
        .ok()
        .json();
    assert!(!over.view.game.running);
    assert_eq!(over.meta.ended.as_deref(), Some("resigned"));
    let board: Value = server.get("/api/leaderboard", None).await.json();
    assert_eq!(board["players"][0]["user"]["name"], json!("Ada"));
    let profile: Value = server.get("/api/users/bo", None).await.json();
    assert_eq!(profile["recent"][0]["gameId"], json!(game));
    assert_eq!(profile["stats"], json!({ "games": 1, "wins": 0 }));
    let history: Value = server
        .get("/api/history?user=Ada&limit=5", None)
        .await
        .json();
    assert_eq!(history["games"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_signed_in_onlooker_watches_a_lobby_game_and_sees_no_hands() {
    let server = start().await;
    let (ada, bo, cy) = (
        server.register("Ada").await,
        server.register("Bo").await,
        server.register("Cy").await,
    );
    server
        .post("/api/automatch", Some(&ada), Value::Null)
        .await
        .ok();
    let found: QueueStatus = server
        .post("/api/automatch", Some(&bo), Value::Null)
        .await
        .json();
    let path = format!("/api/games/{}", found.game_id.unwrap());
    server
        .get(&format!("{path}/watch"), None)
        .await
        .error(StatusCode::UNAUTHORIZED);
    let watched: SeatView = server.get(&format!("{path}/watch"), Some(&cy)).await.json();
    assert!(watched.view.spectating && watched.view.seat.is_none());
    assert!(watched.view.you.is_none() && watched.view.decision.is_none());
    assert_eq!(watched.view.opponents.len(), 2);
    for player in &watched.view.opponents {
        assert_eq!((player.hand.count, player.hand.unknown), (5, 5));
    }
    assert!(watched.deciding.is_some());
    // While the onlooker's feed is open, the players see that someone watches.
    let mut feed = server
        .http
        .get(format!("{}{path}/events", server.base))
        .bearer_auth(&cy)
        .send()
        .await
        .unwrap()
        .bytes_stream();
    next_change(&mut feed).await;
    let mine: SeatView = server.get(&format!("{path}/view"), Some(&ada)).await.json();
    assert_eq!(mine.meta.watchers, 1);
    let chat = path.replace("/api/games/", "/api/chat/game-");
    server
        .post(&chat, Some(&cy), json!({ "text": "nice hand" }))
        .await
        .ok();
    let log: ChatLog = server.get(&chat, Some(&ada)).await.json();
    assert_eq!(
        log.messages[0].kind,
        dominion_protocol::lobby::ChatKind::Watcher
    );
    // A game made by hand is its seats' only, unless it says otherwise.
    let made: Value = server
        .post("/api/games", None, json!({ "players": 2 }))
        .await
        .ok()
        .json();
    let private = format!("/api/games/{}/watch", made["game_id"].as_str().unwrap());
    server
        .get(&private, Some(&cy))
        .await
        .error(StatusCode::FORBIDDEN);
    let open: Value = server
        .post(
            "/api/games",
            None,
            json!({ "players": 2, "watchable": true }),
        )
        .await
        .ok()
        .json();
    let public = format!("/api/games/{}/watch", open["game_id"].as_str().unwrap());
    server.get(&public, Some(&cy)).await.ok();
}
