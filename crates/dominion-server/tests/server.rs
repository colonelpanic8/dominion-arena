use dominion_server::client::{Client, Reply};
use dominion_server::hub::Hub;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

async fn start() -> (String, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    tokio::spawn(dominion_server::serve(
        listener,
        Arc::new(Hub::default()),
        vec![],
        stop.clone(),
    ));
    (url, stop)
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
