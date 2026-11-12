use dominion_agent::{AgentDecision, Answer};
use dominion_engine::{Card, PlayerId};
use dominion_protocol::api::{
    AnimationSpeed, CardInfo, Created, Dealt, GameSummary, SeatInfo, SeatView, WebSettings,
};
use dominion_server::Options;
use dominion_server::client::Client;
use dominion_server::hub::{Hub, Seat};
use futures_util::StreamExt;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

struct Server {
    addr: SocketAddr,
    base: String,
    hub: Arc<Hub>,
    http: reqwest::Client,
    stop: CancellationToken,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn start_with(options: Options) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hub = Arc::new(Hub::default());
    let stop = CancellationToken::new();
    tokio::spawn(dominion_server::serve(
        listener,
        hub.clone(),
        options,
        stop.clone(),
    ));
    Server {
        addr,
        base: format!("http://{addr}"),
        hub,
        http: reqwest::Client::new(),
        stop,
    }
}

async fn start() -> Server {
    start_with(Options::default()).await
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

    /// The `error` of a JSON error body with `status`.
    fn error(&self, status: StatusCode) -> String {
        assert_eq!(self.status, status, "{}", self.text);
        let body: Value = self.json();
        body["error"].as_str().unwrap().to_string()
    }
}

impl Server {
    async fn send(&self, req: reqwest::RequestBuilder, token: Option<&str>) -> Reply {
        let req = match token {
            Some(t) => req.bearer_auth(t),
            None => req,
        };
        let res = req.send().await.unwrap();
        Reply {
            status: res.status(),
            headers: res.headers().clone(),
            text: res.text().await.unwrap(),
        }
    }

    async fn get(&self, path: &str, token: Option<&str>) -> Reply {
        self.send(self.http.get(format!("{}{path}", self.base)), token)
            .await
    }

    async fn post(&self, path: &str, token: Option<&str>, body: impl Into<String>) -> Reply {
        let req = self
            .http
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .body(body.into());
        self.send(req, token).await
    }

    async fn create(&self, req: Value) -> Created {
        let reply = self.post("/api/games", None, req.to_string()).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
        reply.json()
    }

    async fn summary(&self, id: &str) -> GameSummary {
        let games: Vec<GameSummary> = self.get("/api/games", None).await.json();
        games.into_iter().find(|g| g.game_id == id).unwrap()
    }

    async fn view(&self, id: &str, token: &str) -> SeatView {
        let reply = self
            .get(&format!("/api/games/{id}/view"), Some(token))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        reply.json()
    }

    async fn answer(&self, id: &str, token: &str, answer: &Answer) -> Reply {
        let body = serde_json::to_string(answer).unwrap();
        self.post(&format!("/api/games/{id}/answer"), Some(token), body)
            .await
    }

    /// The deciding seat's token and open decision.
    async fn deciding(&self, created: &Created) -> (String, AgentDecision) {
        let seat = self.summary(&created.game_id).await.deciding.unwrap();
        let token = created.seats[usize::from(seat)].token.clone();
        let sv = self.view(&created.game_id, &token).await;
        assert_eq!(sv.deciding, Some(seat));
        let other = &created.seats[(usize::from(seat) + 1) % created.seats.len()];
        let theirs = self.view(&created.game_id, &other.token).await;
        assert_eq!(theirs.deciding, Some(seat));
        assert!(theirs.view.decision.is_none());
        (token, sv.view.decision.unwrap())
    }

    /// Answers the open decision with its first option, over HTTP.
    async fn answer_first(&self, created: &Created) -> SeatView {
        let (token, d) = self.deciding(created).await;
        let reply = self
            .answer(&created.game_id, &token, &first_answer(&d))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        reply.json()
    }

    /// Sends `request` as it is, for what an HTTP client would refuse or rewrite.
    async fn raw(&self, request: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).await.unwrap();
        out
    }
}

fn first_answer(d: &AgentDecision) -> Answer {
    let min = d.min.unwrap_or(1).max(1);
    Answer::indices(
        &d.id,
        d.options
            .iter()
            .take(min as usize)
            .map(|o| o.index)
            .collect(),
    )
}

fn random_answer(d: &AgentDecision, rng: &mut StdRng) -> Answer {
    let (min, max) = (d.min.unwrap_or(1) as usize, d.max.unwrap_or(1) as usize);
    let mut indices: Vec<u32> = d.options.iter().map(|o| o.index).collect();
    indices.shuffle(rng);
    indices.truncate(rng.random_range(min..=max.min(indices.len())));
    Answer::indices(&d.id, indices)
}

/// The table extras agree with the view: every player's cards in play in some order, and
/// a top card exactly when the discard pile has cards.
fn check_extras(sv: &SeatView) {
    let players: Vec<_> = sv.view.you.iter().chain(&sv.view.opponents).collect();
    assert_eq!(sv.table.players.len(), players.len());
    for p in players {
        let t = sv.table.players.iter().find(|t| t.seat == p.index).unwrap();
        let mut counted: std::collections::BTreeMap<String, u32> = Default::default();
        for c in &t.in_play {
            *counted.entry(c.clone()).or_default() += 1;
        }
        assert_eq!(counted, p.in_play.known, "{}'s cards in play", p.name);
        assert_eq!(
            t.discard_top.is_some(),
            p.discard.count > 0,
            "{}'s discard",
            p.name
        );
    }
}

/// Plays a whole game over HTTP with random legal answers and returns seat 0's last view.
async fn play_out(server: &Server, req: Value, seed: u64) -> SeatView {
    let created = server.create(req).await;
    let id = &created.game_id;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut version = 0;
    for _ in 0..20_000 {
        let summary = server.summary(id).await;
        if !summary.running {
            let last = server.view(id, &created.seats[0].token).await;
            assert_eq!(last.version, version);
            assert_eq!(last.deciding, None);
            return last;
        }
        let (token, d) = server.deciding(&created).await;
        let reply = server
            .answer(id, &token, &random_answer(&d, &mut rng))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        let after: SeatView = reply.json();
        check_extras(&after);
        assert_eq!(after.version, version + 1);
        assert_eq!(after.deciding, server.summary(id).await.deciding);
        version = after.version;
    }
    panic!("the game does not end");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn whole_games_are_played_over_http() {
    let server = start().await;
    let base = json!({
        "players": 2,
        "names": ["Ada", "Bo"],
        "kingdom": ["Village", "Smithy", "Market", "Festival", "Laboratory",
                    "Workshop", "Gardens", "Moneylender", "Council Room", "Mine"],
        "seed": 4,
        "turn_limit": 30,
    });
    let random = json!({ "players": 3, "seed": 11, "turn_limit": 24 });
    for (req, seed) in [(base, 1), (random, 2)] {
        let last = play_out(&server, req, seed).await;
        assert!(!last.view.game.running);
        let result = last.view.game.result.expect("a finished game has a result");
        assert!(!result.players.is_empty());
        assert!(last.view.decision.is_none());
    }
}

#[tokio::test]
async fn errors_have_their_status_and_a_message() {
    let server = start().await;
    let req = json!({ "players": 2, "kingdom": ["Village"], "seed": 1 });
    let a = server.create(req.clone()).await;
    let b = server.create(req).await;
    let (ga, gb) = (&a.game_id, &b.game_id);
    let view = |id: &str| format!("/api/games/{id}/view");

    let missing = server.get(&view(ga), None).await;
    assert!(missing.error(StatusCode::UNAUTHORIZED).contains("token"));
    server
        .get(&view(ga), Some("nope"))
        .await
        .error(StatusCode::UNAUTHORIZED);
    server
        .get(&view(gb), Some(&a.seats[0].token))
        .await
        .error(StatusCode::UNAUTHORIZED);
    server
        .get(&view("g999"), Some(&a.seats[0].token))
        .await
        .error(StatusCode::NOT_FOUND);
    server
        .get(&format!("{}?logFrom=x", view(ga)), Some(&a.seats[0].token))
        .await
        .error(StatusCode::BAD_REQUEST);

    let (token, d) = server.deciding(&a).await;
    let other = a.seats.iter().find(|s| s.token != token).unwrap();
    let stale = server
        .answer(ga, &token, &Answer::indices("d999", vec![0]))
        .await;
    assert!(stale.error(StatusCode::CONFLICT).contains("not open"));
    let not_theirs = server.answer(ga, &other.token, &first_answer(&d)).await;
    assert!(
        not_theirs
            .error(StatusCode::CONFLICT)
            .contains("not deciding")
    );
    server
        .answer(ga, &token, &Answer::indices(&d.id, vec![9999]))
        .await
        .error(StatusCode::UNPROCESSABLE_ENTITY);
    server
        .post(
            &format!("/api/games/{ga}/answer"),
            Some(&token),
            "{not json",
        )
        .await
        .error(StatusCode::BAD_REQUEST);
    server
        .post("/api/games", None, r#"{"players": 2, "colour": "red"}"#)
        .await
        .error(StatusCode::BAD_REQUEST);
    server
        .post("/api/games", None, r#"{"players": 9}"#)
        .await
        .error(StatusCode::BAD_REQUEST);
    server
        .get("/api/seat", Some("nope"))
        .await
        .error(StatusCode::UNAUTHORIZED);
    server
        .get("/api/nope", None)
        .await
        .error(StatusCode::NOT_FOUND);
    server
        .post("/api/cards", None, "{}")
        .await
        .error(StatusCode::METHOD_NOT_ALLOWED);

    // Nothing above changed the game.
    assert_eq!(server.view(ga, &token).await.version, 0);
}

#[tokio::test]
async fn a_token_names_its_seat() {
    let server = start().await;
    let created = server
        .create(json!({ "players": 2, "names": ["Ada", "Bo"], "kingdom": ["Village"] }))
        .await;
    let seat: SeatInfo = server
        .get("/api/seat", Some(&created.seats[1].token))
        .await
        .json();
    assert_eq!(
        seat,
        SeatInfo {
            game_id: created.game_id.clone(),
            seat: 1,
            name: "Bo".into()
        }
    );
    let raw: Value = server
        .get("/api/seat", Some(&created.seats[1].token))
        .await
        .json();
    assert_eq!(raw["gameId"], json!(created.game_id));
}

#[tokio::test]
async fn games_say_which_seats_have_joined() {
    let server = start().await;
    let created = server
        .create(json!({ "players": 3, "kingdom": ["Village"] }))
        .await;
    let id = &created.game_id;
    assert_eq!(server.summary(id).await.joined, [false, false, false]);
    server.view(id, &created.seats[2].token).await;
    let wrong = server
        .get(&format!("/api/games/{id}/view"), Some("nope"))
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(server.summary(id).await.joined, [false, false, true]);
    // MCP's join_game claims the seat this way.
    server.hub.claim(id, &created.seats[0].token).unwrap();
    assert_eq!(server.summary(id).await.joined, [true, false, true]);
}

#[tokio::test]
async fn undo_takes_back_the_seats_last_answer() {
    let server = start().await;
    let created = server
        .create(json!({ "players": 2, "kingdom": ["Village"], "seed": 3 }))
        .await;
    let id = &created.game_id;
    let (token, d) = server.deciding(&created).await;
    let before: SeatView = server
        .get(&format!("/api/games/{id}/view?logFrom=0"), Some(&token))
        .await
        .json();
    assert!(!before.can_undo);
    let after = server.answer_first(&created).await;
    assert!(after.can_undo);

    let undo = |token: String| {
        let server = &server;
        async move {
            server
                .post(&format!("/api/games/{id}/undo"), Some(&token), "")
                .await
        }
    };
    let other = created
        .seats
        .iter()
        .find(|s| s.token != token)
        .unwrap()
        .token
        .clone();
    assert_eq!(undo(other).await.status, StatusCode::CONFLICT);

    let reply = undo(token.clone()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let undone: SeatView = reply.json();
    assert_eq!(undone.undos, 1);
    assert!(undone.version > after.version);
    assert!(!undone.can_undo);
    assert_eq!(
        undone.view.since_last_decision,
        before.view.since_last_decision
    );
    // The same position, under a new decision id: the old one is stale.
    let open = undone.view.decision.clone().unwrap();
    assert_ne!(open.id, d.id);
    assert_eq!(open.options, d.options);
    let stale = server.answer(id, &token, &first_answer(&d)).await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(undo(token).await.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn log_from_chooses_where_the_log_starts() {
    let server = start().await;
    let created = server
        .create(json!({ "players": 2, "kingdom": ["Village"], "seed": 3 }))
        .await;
    for _ in 0..6 {
        server.answer_first(&created).await;
    }
    let id = &created.game_id;
    let token = &created.seats[0].token;
    let whole: SeatView = server
        .get(&format!("/api/games/{id}/view?logFrom=0"), Some(token))
        .await
        .json();
    let log = &whole.view.since_last_decision;
    assert!(log.len() > 6);
    assert!(log.iter().enumerate().all(|(i, e)| e.seq == i));
    let usual = server.view(id, token).await.view.since_last_decision;
    assert!(usual.len() < log.len());
    assert_eq!(usual.last(), log.last());
    let tail: SeatView = server
        .get(&format!("/api/games/{id}/view?logFrom=4"), Some(token))
        .await
        .json();
    assert_eq!(tail.view.since_last_decision, log[4..]);
    let past: SeatView = server
        .get(&format!("/api/games/{id}/view?logFrom=100000"), Some(token))
        .await
        .json();
    assert!(past.view.since_last_decision.is_empty());
}

/// Reads `change` events off an SSE response.
struct Changes {
    body: futures_util::stream::BoxStream<'static, reqwest::Result<Vec<u8>>>,
    buf: String,
}

impl Changes {
    async fn open(server: &Server, id: &str, token: &str) -> Changes {
        let res = server
            .http
            .get(format!(
                "{}/api/games/{id}/events?token={token}",
                server.base
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()["content-type"].to_str().unwrap(),
            "text/event-stream"
        );
        Changes {
            body: res.bytes_stream().map(|r| r.map(|b| b.to_vec())).boxed(),
            buf: String::new(),
        }
    }

    async fn next(&mut self) -> u64 {
        timeout(Duration::from_secs(5), async {
            loop {
                while let Some(end) = self.buf.find("\n\n") {
                    let event: String = self.buf.drain(..end + 2).collect();
                    if !event.lines().any(|l| l == "event: change") {
                        continue;
                    }
                    let data = event.lines().find_map(|l| l.strip_prefix("data: "));
                    let data: Value = serde_json::from_str(data.unwrap()).unwrap();
                    return data["version"].as_u64().unwrap();
                }
                let chunk = self.body.next().await.unwrap().unwrap();
                self.buf.push_str(std::str::from_utf8(&chunk).unwrap());
            }
        })
        .await
        .expect("a change event")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_change_reaches_event_subscribers() {
    let server = start().await;
    let created = server
        .create(json!({ "players": 2, "kingdom": ["Village"], "seed": 2 }))
        .await;
    let id = &created.game_id;
    let missing = server.get(&format!("/api/games/{id}/events"), None).await;
    missing.error(StatusCode::UNAUTHORIZED);

    let mut changes = Changes::open(&server, id, &created.seats[1].token).await;
    assert_eq!(changes.next().await, 0);

    server.answer_first(&created).await;
    assert_eq!(changes.next().await, 1);

    let (token, d) = server.deciding(&created).await;
    let seat = server.hub.find(&token).unwrap();
    assert_eq!(seat.game, *id);
    server.hub.answer(&seat, &first_answer(&d)).unwrap();
    assert_eq!(changes.next().await, 2);

    let (token, d) = server.deciding(&created).await;
    let mcp = Client::connect(&format!("{}/mcp", server.base))
        .await
        .unwrap();
    let reply = mcp
        .call(
            "answer",
            json!({ "token": token, "decision_id": d.id, "indices": first_answer(&d).indices }),
        )
        .await
        .unwrap();
    assert!(reply.ok, "{}", reply.text);
    assert_eq!(changes.next().await, 3);
    mcp.close().await;

    // A subscriber that joins late starts at the current version.
    let mut late = Changes::open(&server, id, &created.seats[0].token).await;
    assert_eq!(late.next().await, 3);
    let seat0 = Seat {
        game: id.clone(),
        seat: PlayerId(0),
    };
    assert_eq!(server.hub.seat_view(&seat0, None).unwrap().version, 3);
}

#[tokio::test]
async fn an_event_stream_ends_when_the_server_stops() {
    let server = start().await;
    let created = server
        .create(json!({ "players": 2, "kingdom": ["Village"] }))
        .await;
    let mut changes = Changes::open(&server, &created.game_id, &created.seats[0].token).await;
    assert_eq!(changes.next().await, 0);
    server.stop.cancel();
    let end = timeout(Duration::from_secs(5), async {
        while let Some(Ok(_)) = changes.body.next().await {}
    })
    .await;
    assert!(end.is_ok(), "the stream outlived the server");
}

fn request(path: &str, host: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

fn status(response: &str) -> u16 {
    response.split(' ').nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn only_allowed_hosts_are_served() {
    let server = start_with(Options {
        allowed_hosts: vec!["box.tailnet".into(), "pinned.example:8080".into()],
        ..Options::default()
    })
    .await;
    let port = server.addr.port();
    for (host, allowed) in [
        ("evil.example".to_string(), false),
        (format!("evil.example:{port}"), false),
        (format!("127.0.0.1:{port}"), true),
        (format!("localhost:{port}"), true),
        ("LOCALHOST".to_string(), true),
        ("box.tailnet".to_string(), true),
        ("box.tailnet:9999".to_string(), true),
        ("pinned.example:8080".to_string(), true),
        ("pinned.example:8081".to_string(), false),
        ("pinned.example".to_string(), false),
    ] {
        for path in ["/api/games", "/assets/x", "/"] {
            let got = status(&server.raw(&request(path, &host)).await);
            assert_eq!(got == 403, !allowed, "{host} {path}: {got}");
        }
    }
}

struct Dirs {
    _root: tempfile::TempDir,
    assets: PathBuf,
    web: PathBuf,
}

fn dirs() -> Dirs {
    let root = tempfile::tempdir().unwrap();
    let assets = root.path().join("assets");
    let web = root.path().join("web");
    std::fs::create_dir_all(assets.join("images/cards")).unwrap();
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(assets.join("images/cards/village.webp"), "art").unwrap();
    std::fs::write(web.join("assets/client-1a2b.wasm"), "wasm").unwrap();
    std::fs::write(web.join("assets/manifest.json"), "shadowed").unwrap();
    std::fs::write(assets.join("manifest.json"), "{}").unwrap();
    std::fs::write(root.path().join("secret.txt"), "secret").unwrap();
    std::fs::write(web.join("index.html"), "<html>client</html>").unwrap();
    std::fs::write(web.join("app.js"), "main()").unwrap();
    Dirs {
        _root: root,
        assets,
        web,
    }
}

#[tokio::test]
async fn assets_and_the_client_are_served_from_their_directories() {
    let dirs = dirs();
    let server = start_with(Options {
        assets_dir: Some(dirs.assets.clone()),
        web_dir: Some(dirs.web.clone()),
        ..Options::default()
    })
    .await;
    let art = server.get("/assets/images/cards/village.webp", None).await;
    assert_eq!(art.status, StatusCode::OK);
    assert_eq!(art.text, "art");
    assert!(
        art.headers["cache-control"]
            .to_str()
            .unwrap()
            .contains("max-age=")
    );
    // The client's bundled files share /assets; the cache wins a clash.
    let bundled = server.get("/assets/client-1a2b.wasm", None).await;
    assert_eq!(
        (bundled.status, bundled.text.as_str()),
        (StatusCode::OK, "wasm")
    );
    assert_eq!(server.get("/assets/manifest.json", None).await.text, "{}");
    let missing = server.get("/assets/images/cards/nope.webp", None).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.headers.get("cache-control").is_none());
    assert_eq!(
        server.get("/assets/images/cards", None).await.status,
        StatusCode::NOT_FOUND
    );
    let host = format!("127.0.0.1:{}", server.addr.port());
    for path in ["/assets/../secret.txt", "/assets/%2e%2e/secret.txt"] {
        let res = server.raw(&request(path, &host)).await;
        assert!(!res.contains("secret"), "{path}: {res}");
        assert_ne!(status(&res), 200, "{path}");
    }

    for path in ["/", "/game/g1", "/index.html"] {
        let page = server.get(path, None).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert_eq!(page.text, "<html>client</html>", "{path}");
    }
    assert_eq!(server.get("/app.js", None).await.text, "main()");
    let api = server.get("/api/nope", None).await;
    assert!(api.error(StatusCode::NOT_FOUND).contains("route"));
}

#[tokio::test]
async fn without_a_client_the_root_says_where_the_mcp_endpoint_is() {
    let server = start_with(Options {
        assets_dir: Some(PathBuf::from("/nonexistent/dominion-assets")),
        ..Options::default()
    })
    .await;
    let root = server.get("/", None).await;
    assert_eq!(root.status, StatusCode::OK);
    assert!(root.text.contains("/mcp"), "{}", root.text);
    assert_eq!(root.text.lines().count(), 1);
    assert_eq!(
        server.get("/game/g1", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server.get("/assets/images/x.png", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server.get("/assets/manifest.json", None).await.status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn settings_serve_the_web_client_settings_without_a_token() {
    let server = start_with(Options {
        web: WebSettings {
            web_animations: false,
            web_animation_speed: AnimationSpeed::Speedy,
            web_sounds: true,
            web_volume: 0.25,
            ..WebSettings::default()
        },
        ..Options::default()
    })
    .await;
    let raw: Value = server.get("/api/settings", None).await.json();
    assert_eq!(
        raw,
        json!({
            "webAnimations": false,
            "webAnimationSpeed": "speedy",
            "webMoveKingdom": true,
            "webConfirmSingle": false,
            "webBackground": "inheritance",
            "webSounds": true,
            "webVolume": 0.25,
            "bannedCards": [],
            "registration": true,
            "guests": true
        })
    );
    let defaults: WebSettings = start().await.get("/api/settings", None).await.json();
    assert_eq!(defaults, WebSettings::default());
}

#[tokio::test]
async fn cards_lists_every_card_as_printed() {
    let server = start().await;
    let cards: Vec<CardInfo> = server.get("/api/cards", None).await.json();
    assert_eq!(cards.len(), Card::ALL.len());
    let card = |name: &str| cards.iter().find(|c| c.name == name).unwrap().clone();
    let raw: Vec<Value> = server.get("/api/cards", None).await.json();
    let village = raw.iter().find(|c| c["name"] == "Village").unwrap();
    assert_eq!(village["types"], json!(["Action"]));
    assert_eq!(
        village["cost"],
        json!({ "coins": 3, "potions": 0, "debt": 0 })
    );
    assert_eq!(village["kind"], json!("card"));
    assert_eq!(village["set"], json!(Card::Village.set().to_string()));
    assert_eq!(village["variableCost"], json!(false));
    assert!(village["kingdom"].as_bool().unwrap());

    let engineer = card("Engineer").cost.unwrap();
    assert_eq!((engineer.coins, engineer.debt), (0, 4));
    let possession = card("Possession").cost.unwrap();
    assert_eq!((possession.coins, possession.potions), (6, 1));
    let aqueduct = raw.iter().find(|c| c["name"] == "Aqueduct").unwrap();
    assert!(aqueduct.get("cost").is_none());
    assert_eq!(aqueduct["kind"], json!("landmark"));
    assert_eq!(card("Gold").coins, 3);
    assert_eq!(card("Province").vp, 6);
    assert!(card("Copper").basic);
    assert_eq!(card("Sir Martin").pile.as_deref(), Some("Knights"));
}

#[tokio::test]
async fn a_deal_is_what_a_game_with_the_same_rules_and_seed_is_set_up_with() {
    let server = start().await;
    let deal = |body: Value| {
        let server = &server;
        async move {
            let reply = server.post("/api/deal", None, body.to_string()).await;
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
            reply.json::<Dealt>()
        }
    };
    for seed in [1u64, 7, 42] {
        let sets = json!(["Base", "Seaside", "Adventures"]);
        let dealt = deal(json!({"players": 2, "sets": sets, "seed": seed})).await;
        assert_eq!(dealt.seed, seed);
        let random = server
            .create(json!({"players": 2, "sets": sets, "seed": seed}))
            .await;
        let random = server.summary(&random.game_id).await;
        assert_eq!(
            (&random.kingdom, &random.landscapes),
            (&dealt.kingdom, &dealt.landscapes)
        );

        let rules = json!({
            "players": 3,
            "sets": ["Base", "Prosperity"],
            "kingdom": ["Witch", "Smithy"],
            "fromSets": ["Intrigue"],
            "banned": ["Village", "Market", "Bank"],
            "slots": [{"kinds": ["event", "project"], "optional": false}],
            "seed": seed,
        });
        let dealt = deal(rules).await;
        assert_eq!(&dealt.kingdom[..2], ["Witch", "Smithy"]);
        assert_eq!(dealt.kingdom.len(), 10);
        assert!(
            !dealt
                .kingdom
                .iter()
                .any(|k| ["Village", "Market", "Bank"].contains(&k.as_str()))
        );
        let started = server
            .create(json!({
                "players": 3,
                "kingdom": dealt.kingdom,
                "landscapes": dealt.landscapes,
                "seed": dealt.seed,
            }))
            .await;
        let started = server.summary(&started.game_id).await;
        assert_eq!(started.kingdom, dealt.kingdom);
        assert!(
            dealt
                .landscapes
                .iter()
                .all(|l| started.landscapes.contains(l))
        );
    }
    let bad = server
        .post("/api/deal", None, r#"{"players": 2, "kingdom": ["Gold"]}"#)
        .await;
    assert!(bad.error(StatusCode::BAD_REQUEST).contains("Kingdom pile"));
    let bad = server
        .post(
            "/api/deal",
            None,
            r#"{"players": 2, "slots": [{"kinds": ["ally"], "optional": true}]}"#,
        )
        .await;
    assert!(bad.error(StatusCode::BAD_REQUEST).contains("slot kind"));
}
