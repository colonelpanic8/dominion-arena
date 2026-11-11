use dominion_server::Options;
use dominion_server::client::{CallError, Client, Reply};
use dominion_server::hub::Hub;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

struct Server {
    url: String,
    stop: CancellationToken,
    done: JoinHandle<std::io::Result<()>>,
}

async fn start_on(addr: &str, options: Options) -> Server {
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    let done = tokio::spawn(dominion_server::serve(
        listener,
        Arc::new(Hub::default()),
        options,
        stop.clone(),
    ));
    Server { url, stop, done }
}

async fn start() -> (String, CancellationToken) {
    let server = start_on("127.0.0.1:0", Options::default()).await;
    (server.url, server.stop)
}

/// A server whose sessions close after `idle` without a call.
async fn start_with_idle(idle: Duration) -> Server {
    let options = Options {
        session_idle: Some(idle),
        ..Options::default()
    };
    start_on("127.0.0.1:0", options).await
}

/// A two-player game: its id and seat tokens.
async fn create(client: &Client) -> (String, Vec<String>) {
    let created = call(
        client,
        "create_game",
        json!({ "players": 2, "kingdom": ["Village"], "seed": 1 }),
    )
    .await;
    assert!(created.ok, "{}", created.text);
    let created = created.json.unwrap();
    let tokens = created["seats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["token"].as_str().unwrap().to_string())
        .collect();
    (created["game_id"].as_str().unwrap().to_string(), tokens)
}

async fn call(client: &Client, tool: &'static str, args: Value) -> Reply {
    client.call(tool, args).await.unwrap()
}

/// Plays `token`'s seat to the end, answering each decision with its first options.
async fn play_seat(client: Client, token: String, seat: u64) -> Value {
    loop {
        let reply = call(
            &client,
            "wait_for_decision",
            json!({ "token": token, "timeout_ms": 10_000 }),
        )
        .await;
        assert!(reply.ok, "{}", reply.text);
        let view = reply.json.unwrap();
        assert_eq!(view["seat"], json!(seat));
        assert_eq!(view["you"]["index"], json!(seat));
        if view["game"]["running"] == json!(false) {
            client.close().await;
            return view;
        }
        let Some(d) = view["decision"].as_object().cloned() else {
            continue;
        };
        let min = d["min"].as_u64().unwrap().max(1);
        let indices: Vec<u64> = (0..min).collect();
        let reply = call(
            &client,
            "answer",
            json!({ "token": token, "decision_id": d["id"], "indices": indices }),
        )
        .await;
        assert!(reply.ok, "{}", reply.text);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_clients_play_their_seats_to_the_end() {
    let (url, stop) = start().await;
    let a = Client::connect(&url).await.unwrap();
    let b = Client::connect(&url).await.unwrap();
    let created = call(
        &a,
        "create_game",
        json!({
            "players": 2,
            "names": ["Ada", "Bo"],
            "kingdom": ["Village", "Smithy", "Market", "Festival", "Laboratory",
                        "Woodcutter", "Gardens", "Moneylender", "Council Room", "Mine"],
            "seed": 4,
            "turn_limit": 16,
            "colonies": false,
            "shelters": false,
        }),
    )
    .await;
    assert!(created.ok, "{}", created.text);
    let created = created.json.unwrap();
    let game = created["game_id"].as_str().unwrap().to_string();
    let token = |i: usize| created["seats"][i]["token"].as_str().unwrap().to_string();

    let rejected = call(
        &b,
        "join_game",
        json!({ "game_id": game, "token": "guess" }),
    )
    .await;
    assert!(!rejected.ok);
    assert!(!call(&b, "view", json!({ "token": "guess" })).await.ok);
    assert!(!call(&b, "view", json!({})).await.ok);
    let joined = call(
        &b,
        "join_game",
        json!({ "game_id": game, "token": token(1) }),
    )
    .await;
    assert!(joined.ok, "{}", joined.text);
    let theirs = &joined.json.unwrap()["opponents"][0]["hand"];
    assert_eq!(theirs["unknown"], theirs["count"]);
    let bound = call(&b, "view", json!({})).await;
    assert!(bound.text.contains("YOU: Bo"), "{}", bound.text);

    let view = call(&a, "view", json!({ "token": token(0) })).await;
    assert!(view.text.contains("YOU: Ada"), "{}", view.text);
    let view = view.json.unwrap();
    let id = view["decision"]["id"].as_str().unwrap().to_string();
    let options = view["decision"]["options"].as_array().unwrap().len();

    let out_of_turn = call(
        &b,
        "answer",
        json!({ "token": token(1), "decision_id": id, "indices": [0] }),
    )
    .await;
    assert!(!out_of_turn.ok);
    assert!(
        out_of_turn.text.contains("not deciding"),
        "{}",
        out_of_turn.text
    );
    for (decision, indices) in [("d999", json!([0])), (id.as_str(), json!([options]))] {
        let bad = call(
            &a,
            "answer",
            json!({ "token": token(0), "decision_id": decision, "indices": indices }),
        )
        .await;
        assert!(!bad.ok);
        assert!(bad.text.starts_with("rejected:"), "{}", bad.text);
    }

    let seat_a = tokio::spawn(play_seat(a, token(0), 0));
    let seat_b = tokio::spawn(play_seat(b, token(1), 1));
    let (end_a, end_b) = tokio::try_join!(seat_a, seat_b).expect("a seat failed");
    assert_eq!(end_a["game"]["result"], end_b["game"]["result"]);
    assert_eq!(
        end_a["game"]["result"]["players"].as_array().unwrap().len(),
        2
    );

    let card = Client::connect(&url).await.unwrap();
    let moat = call(&card, "card", json!({ "name": "moat" })).await;
    assert!(
        moat.text.starts_with("Moat | Action - Reaction"),
        "{}",
        moat.text
    );
    let games = call(&card, "list_games", json!({})).await;
    assert!(games.text.contains(&game) && !games.text.contains(&token(0)));
    card.close().await;
    stop.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arguments_a_tool_does_not_take_are_refused() {
    let (url, stop) = start().await;
    let client = Client::connect(&url).await.unwrap();
    let (game, tokens) = create(&client).await;
    for (tool, unknown, args) in [
        (
            "create_game",
            "kingdoms",
            json!({ "players": 2, "kingdoms": ["Village"] }),
        ),
        (
            "create_game",
            "turnLimit",
            json!({ "players": 2, "turnLimit": 3 }),
        ),
        (
            "join_game",
            "seat",
            json!({ "game_id": game, "token": tokens[0], "seat": 0 }),
        ),
        ("view", "fmt", json!({ "token": tokens[0], "fmt": "json" })),
        (
            "wait_for_decision",
            "timeout",
            json!({ "token": tokens[1], "timeout": 5 }),
        ),
        (
            "answer",
            "index",
            json!({ "token": tokens[0], "decision_id": "d1", "index": 0 }),
        ),
        ("card", "set", json!({ "name": "Moat", "set": "Base" })),
        ("list_games", "running", json!({ "running": true })),
    ] {
        let refused = call(&client, tool, args).await;
        assert!(!refused.ok, "{tool} took `{unknown}`");
        assert!(
            refused.text.contains(&format!("unknown field `{unknown}`")),
            "{tool}: {}",
            refused.text
        );
    }
    let games = call(&client, "list_games", json!({})).await;
    assert_eq!(games.json.unwrap()["games"].as_array().unwrap().len(), 1);
    client.close().await;
    stop.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_join_lasts_until_its_session_ends() {
    let server = start_with_idle(Duration::from_secs(3)).await;
    let client = Client::connect(&server.url).await.unwrap();
    let (game, tokens) = create(&client).await;
    let join = json!({ "game_id": game, "token": tokens[0] });
    assert!(call(&client, "join_game", join.clone()).await.ok);
    assert!(call(&client, "view", json!({})).await.ok);

    // The idle session is closed; the client starts another without saying so.
    sleep(Duration::from_secs(5)).await;
    let unbound = call(&client, "view", json!({})).await;
    assert!(!unbound.ok);
    assert!(unbound.text.contains("join_game again"), "{}", unbound.text);
    assert!(
        call(&client, "view", json!({ "token": tokens[0] }))
            .await
            .ok
    );
    assert!(call(&client, "join_game", join).await.ok);
    assert!(call(&client, "view", json!({})).await.ok);
    client.close().await;
    server.stop.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wait_ends_before_its_session_would() {
    let server = start_with_idle(Duration::from_secs(2)).await;
    let client = Client::connect(&server.url).await.unwrap();
    let (_, tokens) = create(&client).await;
    let waiting = client.call(
        "wait_for_decision",
        json!({ "token": tokens[1], "timeout_ms": 30_000 }),
    );
    let waited = timeout(Duration::from_secs(10), waiting)
        .await
        .expect("the wait outlived its session and was never answered")
        .unwrap();
    assert!(waited.ok, "{}", waited.text);
    assert_eq!(waited.json.unwrap()["decision"], Value::Null);
    client.close().await;
    server.stop.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_fails_when_the_server_goes_away() {
    let server = start_on("127.0.0.1:0", Options::default()).await;
    let client = Client::connect(&server.url).await.unwrap();
    let (_, tokens) = create(&client).await;
    let waiting = tokio::spawn(async move {
        let wait = json!({ "token": tokens[1], "timeout_ms": 60_000 });
        client.call("wait_for_decision", wait).await
    });
    sleep(Duration::from_millis(500)).await;
    server.stop.cancel();
    timeout(Duration::from_secs(5), server.done)
        .await
        .expect("the server shuts down")
        .unwrap()
        .unwrap();
    let outcome = timeout(Duration::from_secs(20), waiting)
        .await
        .expect("the call neither returned nor failed after the server was gone")
        .unwrap();
    assert!(matches!(outcome, Err(CallError::Lost(_))), "{outcome:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_on_ipv6_loopback_is_reached_at_its_bracketed_address() {
    if tokio::net::TcpListener::bind("[::1]:0").await.is_err() {
        return;
    }
    let server = start_on("[::1]:0", Options::default()).await;
    assert!(server.url.starts_with("http://[::1]:"), "{}", server.url);
    let client = Client::connect(&server.url).await.unwrap();
    assert!(call(&client, "list_games", json!({})).await.ok);
    client.close().await;
    server.stop.cancel();
}
