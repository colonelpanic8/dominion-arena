//! The shared dominion/play@1 surface, exercised through real MCP sessions.
use dominion_server::client::{Client, Reply};
use dominion_server::hub::Hub;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

async fn server() -> (Client, Client, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    tokio::spawn(dominion_server::serve(
        listener,
        Arc::new(Hub::default()),
        Default::default(),
        stop.clone(),
    ));
    (
        Client::connect(&url).await.unwrap(),
        Client::connect(&url).await.unwrap(),
        stop,
    )
}

async fn call(client: &Client, tool: &'static str, args: Value) -> Reply {
    client.call(tool, args).await.unwrap()
}

async fn game(client: &Client, turns: u32) -> Reply {
    let reply = call(
        client,
        "new_game",
        json!({"sets":["Base"], "kingdom":["Village"], "seed":42, "turn_limit":turns}),
    )
    .await;
    assert!(reply.ok, "{}", reply.text);
    reply
}

#[tokio::test]
async fn shared_startup_preserves_seat_secrets_and_legacy_tools() {
    let (a, b, stop) = server().await;
    let empty = call(&a, "state", json!({})).await;
    assert!(empty.ok && empty.text.starts_with("STATUS no game"));
    assert!(!call(&a, "new_game", json!({"bots":1})).await.ok);
    assert!(
        call(&a, "list_games", json!({})).await.json.unwrap()["games"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!call(&a, "new_game", json!({"unsupported":true})).await.ok);
    let created = game(&a, 4).await;
    let created_json = created.json.unwrap();
    let view = &created_json["view"];
    assert_eq!(view["schema"], "dominion/agent-view@2");
    assert_eq!(view["seat"], 0);
    assert_eq!(view["game"]["kingdom"].as_array().unwrap().len(), 10);
    assert!(
        view["game"]["kingdom"]
            .as_array()
            .unwrap()
            .contains(&json!("Village"))
    );
    assert_eq!(view["opponents"][0]["hand"]["count"], 5);
    assert_eq!(view["opponents"][0]["hand"]["known"]["Copper"], 2);
    assert_eq!(view["opponents"][0]["hand"]["unknown"], 3);
    assert!(created.text.starts_with("STATUS game"));
    assert!(created.text.contains("\nCARDS\n") && created.text.contains("\nVillage | Action | $3"));
    assert!(!created.text.contains("\nCopper |"));
    let unseated = call(&b, "state", json!({})).await;
    assert!(!unseated.ok && unseated.text.starts_with("NOT_SEATED:"));
    let bad_token = call(&b, "state", json!({"token":"guess"})).await;
    assert!(!bad_token.ok && bad_token.text.starts_with("NOT_SEATED:"));
    let joined = call(
        &b,
        "join_game",
        json!({"game":created_json["game_id"],"token":created_json["seats"][1]["token"]}),
    )
    .await;
    assert!(joined.ok, "{}", joined.text);
    assert_eq!(joined.json.unwrap()["view"]["seat"], 1);
    let legacy = call(&a, "view", json!({})).await;
    assert_eq!(legacy.json.unwrap(), *view);
    let state = call(&a, "state", json!({})).await;
    assert!(!state.text.contains("\nCARDS\n"));
    let cards = call(&a, "cards", json!({"names":["village","Silver"]})).await;
    assert_eq!(cards.json.unwrap()["glossary"][0]["name"], "Village");
    assert!(
        !call(&a, "cards", json!({"names":["Made Up Card"]}))
            .await
            .ok
    );
    assert!(!call(&a, "wait", json!({"timeout_ms":1})).await.ok);
    let waiting = call(&b, "wait", json!({"timeout_s":0})).await;
    assert!(!waiting.text.starts_with("STATUS game")); // Common join established the baseline.
    assert!(
        waiting
            .text
            .ends_with("WAITING: no decision for you yet; call wait.")
    );
    let repeated = call(&b, "wait", json!({"timeout_s":0})).await;
    assert!(!repeated.text.contains("\nLOG") && !repeated.text.contains("\nCARDS"));
    a.close().await;
    b.close().await;
    stop.cancel();
}

#[tokio::test]
async fn shared_step_rejects_stale_illegal_and_mismatched_choices_without_mutating() {
    let (a, b, stop) = server().await;
    let created = game(&a, 4).await.json.unwrap();
    let before = &created["view"];
    let d = &before["decision"];
    let index = d["options"][0]["index"].clone();
    let label = d["options"][0]["label"].clone();
    for args in [
        json!({"decision":"d999999","indices":[index]}),
        json!({"decision":d["id"],"indices":[-1]}),
        json!({"decision":d["id"],"indices":[9999]}),
        json!({"decision":d["id"],"indices":[index,index]}),
        json!({"decision":d["id"],"indices":[index],"expect":["Buy Province"]}),
        json!({"decision":d["id"],"indices":[index],"expect":[]}),
        json!({"decision":d["id"],"indices":[index],"name":"Silver"}),
        json!({"decision":d["id"]}),
    ] {
        let rejected = call(&a, "step", args).await;
        assert!(
            !rejected.ok && rejected.text.starts_with("REJECTED:"),
            "{}",
            rejected.text
        );
        assert!(
            rejected
                .text
                .contains(&format!("DECISION {}:", d["id"].as_str().unwrap())),
            "{}",
            rejected.text
        );
        assert_eq!(call(&a, "view", json!({})).await.json.unwrap(), *before);
    }
    let accepted = call(
        &a,
        "step",
        json!({"decision":d["id"],"indices":[index],"expect":[label],"timeout_s":300}),
    )
    .await;
    assert!(accepted.ok, "{}", accepted.text);
    assert!(!accepted.text.contains("STATUS ") && !accepted.text.contains("SUPPLY"));
    assert!(accepted.text.contains("DECISION ") && accepted.text.contains("\nLOG\n"));
    let next = accepted.json.unwrap();
    assert_ne!(next["view"]["decision"]["id"], d["id"]);
    let last_seq = before["sinceLastDecision"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["seq"]
        .as_u64()
        .unwrap();
    let reread = call(&a, "log", json!({"after":last_seq,"limit":1000}))
        .await
        .json
        .unwrap();
    assert!(!reread["log"].as_array().unwrap().is_empty());
    for e in reread["log"].as_array().unwrap() {
        assert!(e["seq"].as_u64().unwrap() > last_seq);
        assert!(
            accepted
                .text
                .contains(&format!("{}: {}", e["seq"], e["text"].as_str().unwrap()))
        );
    }
    assert!(
        call(&a, "log", json!({"limit":0})).await.json.unwrap()["log"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    a.close().await;
    b.close().await;
    stop.cancel();
}

#[tokio::test]
async fn shared_loop_plays_both_seats_to_a_result() {
    let (a, b, stop) = server().await;
    let created = game(&a, 4).await.json.unwrap();
    assert!(
        call(
            &b,
            "join_game",
            json!({"game_id":created["game_id"],"token":created["seats"][1]["token"]})
        )
        .await
        .ok
    );
    let play = |client: Client| async move {
        let mut response = call(&client, "state", json!({})).await;
        for _ in 0..100 {
            assert!(response.ok, "{}", response.text);
            let view = &response.json.as_ref().unwrap()["view"];
            if view["game"]["running"] == false {
                assert!(
                    response.text.contains("\nGAME OVER\nRESULT"),
                    "{}",
                    response.text
                );
                client.close().await;
                return;
            }
            let d = &view["decision"];
            response = if d.is_null() {
                call(&client, "wait", json!({"timeout_s":2})).await
            } else {
                let options = d["options"].as_array().unwrap();
                let option = options
                    .iter()
                    .find(|o| o["action"] == "end-phase")
                    .unwrap_or(&options[0]);
                call(&client,"step",json!({"decision":d["id"],"indices":[option["index"]],"expect":[option["label"]],"timeout_s":2})).await
            };
        }
        panic!("game did not finish");
    };
    tokio::join!(play(a), play(b));
    stop.cancel();
}

#[tokio::test]
async fn cards_uses_current_costs_after_bridge_and_token_observations_set_reference_context() {
    let (a, b, stop) = server().await;
    let created = call(
        &a,
        "create_game",
        json!({"players":2,"kingdom":["Bridge"],"seed":4,"turn_limit":40}),
    )
    .await
    .json
    .unwrap();
    let mut discounted = false;
    for _ in 0..300 {
        for seat in 0..2 {
            let state = call(
                &a,
                "state",
                json!({"token":created["seats"][seat]["token"]}),
            )
            .await;
            assert!(state.ok, "{}", state.text);
            let view = &state.json.as_ref().unwrap()["view"];
            if view["game"]["running"] == false {
                break;
            }
            let d = &view["decision"];
            if d.is_null() {
                continue;
            }
            let options = d["options"].as_array().unwrap();
            let play_bridge = options
                .iter()
                .find(|o| o["action"] == "play" && o["card"] == "Bridge");
            let option = play_bridge
                .or_else(|| options.iter().find(|o| o["action"] == "play"))
                .or_else(|| {
                    options
                        .iter()
                        .find(|o| o["action"] == "buy" && o["card"] == "Bridge")
                })
                .or_else(|| options.iter().find(|o| o["action"] == "end-phase"))
                .unwrap_or(&options[0]);
            let stepped = call(&a,"step",json!({"token":created["seats"][seat]["token"],"decision":d["id"],"indices":[option["index"]],"expect":[option["label"]],"timeout_s":0})).await;
            assert!(stepped.ok, "{}", stepped.text);
            if play_bridge.is_some() {
                let cards = call(&a, "cards", json!({"names":["Silver","Bridge"]}))
                    .await
                    .json
                    .unwrap();
                assert_eq!(cards["glossary"][0]["cost"]["coins"], 2);
                assert_eq!(cards["glossary"][1]["cost"]["coins"], 3);
                // No join was required: log uses the seat last observed with its token.
                assert!(call(&a, "log", json!({"limit":1})).await.ok);
                discounted = true;
                break;
            }
        }
        if discounted {
            break;
        }
    }
    assert!(discounted, "never reached Bridge's cost modifier");
    a.close().await;
    b.close().await;
    stop.cancel();
}
