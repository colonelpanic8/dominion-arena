mod common;
use common::*;
use dominion_online::{
    model::GameResult,
    reducer::Reducer,
    server::{AttachedReceiver, ServerEncoder},
    transport::{Decoder, Message, Transport, command},
};
use dominion_protocol::Answer;
fn receiver() -> AttachedReceiver {
    AttachedReceiver::new(Reducer::new(catalog(), 10))
}
fn apply(r: &mut AttachedReceiver, messages: &[Message]) {
    let bytes = ServerEncoder::new().encode_batch(messages).unwrap();
    r.apply_batch(Decoder::new().decode(&bytes).unwrap())
        .unwrap();
}
fn terminal(result: GameResult) -> Message {
    Message::GameFinished {
        table: table(),
        result,
        continue_allowed: false,
        match_completed: true,
    }
}
#[test]
fn completion_retains_scores_turns_one_based_ranks_and_resignation() {
    let mut r = receiver();
    apply(
        &mut r,
        &[
            Message::TableDetails(table()),
            Message::Snapshot(snapshot()),
            Message::Question(choice(42)),
        ],
    );
    assert!(r.reducer.view().unwrap().decision.is_some());
    apply(&mut r, &[Message::Snapshot(snapshot()), terminal(result())]);
    let view = r.reducer.view().unwrap();
    assert!(!view.game.running);
    assert!(view.decision.is_none());
    let players = &view.game.result.unwrap().players;
    assert_eq!(
        players
            .iter()
            .map(|p| (&*p.name, p.vp, p.rank, p.turns, p.resigned))
            .collect::<Vec<_>>(),
        vec![("Alice", 20, 1, 12, false), ("Bob", 25, 2, 11, true)]
    );
    assert!(
        r.reducer
            .answer(&Answer::indices("old", vec![0]), None)
            .is_err()
    );
    let mut fresh = receiver();
    apply(
        &mut fresh,
        &[
            Message::TableDetails(table()),
            Message::Snapshot(snapshot()),
            terminal(result()),
        ],
    );
    assert_eq!(
        fresh.reducer.view().unwrap().game.result,
        r.reducer.view().unwrap().game.result
    );
}
#[test]
fn tied_ranks_are_preserved_in_snapshot_player_order() {
    let mut r = receiver();
    let mut result = result();
    for p in &mut result.players {
        p.rank = 1;
        p.score.total_points = 9;
        p.score.used_turns = 10;
        p.resignation_type = None;
        p.resign_index = -1;
    }
    result.players.reverse();
    apply(
        &mut r,
        &[
            Message::TableDetails(table()),
            Message::Snapshot(snapshot()),
            terminal(result),
        ],
    );
    let players = r.reducer.view().unwrap().game.result.unwrap().players;
    assert_eq!(
        players
            .iter()
            .map(|p| (p.index, p.rank, p.turns))
            .collect::<Vec<_>>(),
        vec![(0, 1, 10), (1, 1, 10)]
    );
}
#[test]
fn invalid_completion_rolls_back_entire_batch_and_pending_decision() {
    let mut r = receiver();
    apply(
        &mut r,
        &[
            Message::TableDetails(table()),
            Message::Snapshot(snapshot()),
            Message::Question(choice(42)),
        ],
    );
    let before = r.reducer.view().unwrap();
    for bad in 0..9 {
        let mut result = result();
        match bad {
            0 => {
                result.players.pop();
            }
            1 => {
                result.players[1].player_id = 10;
            }
            2 => {
                result.players[1].player_id = 30;
            }
            3 => result.game_id = 8,
            4 => result.table_id = 2,
            5 => result.players[0].rank = 0,
            6 => result.players[0].score.used_turns = -1,
            7 => result.players[0].resignation_type = Some(4),
            _ => result.players[0].rank = 3,
        }
        assert!(
            r.apply_batch(vec![Message::Snapshot(snapshot()), terminal(result)])
                .is_err()
        );
        assert_eq!(r.reducer.view().unwrap(), before, "case {bad}");
    }
    assert!(
        r.apply_batch(vec![Message::Snapshot(snapshot()), Message::LobbyNotice])
            .is_err()
    );
    assert_eq!(r.reducer.view().unwrap(), before);
}
#[test]
fn replacement_requires_final_result_and_invalidates_previous_answers() {
    let mut r = receiver();
    apply(
        &mut r,
        &[
            Message::TableDetails(table()),
            Message::Snapshot(snapshot()),
            Message::Question(choice(42)),
        ],
    );
    let old = r.reducer.view().unwrap().decision.unwrap().id;
    let mut replacement = snapshot();
    replacement.reconnect = true;
    apply(
        &mut r,
        &[
            Message::Snapshot(replacement.clone()),
            Message::Question(choice(43)),
        ],
    );
    assert!(
        r.reducer
            .answer(&Answer::indices(old, vec![0]), None)
            .is_err()
    );
    let id = r.reducer.view().unwrap().decision.unwrap().id;
    let (index, values) = r
        .reducer
        .answer(&Answer::indices(id, vec![0]), None)
        .unwrap();
    assert_eq!((index, values), (43, vec![0]));
    apply(
        &mut r,
        &[Message::Snapshot(replacement.clone()), terminal(result())],
    );
    let before = r.reducer.view().unwrap();
    assert!(
        r.apply_batch(vec![Message::Snapshot(replacement.clone())])
            .is_err()
    );
    assert_eq!(r.reducer.view().unwrap(), before);
    apply(
        &mut r,
        &[Message::Snapshot(replacement), terminal(result())],
    );
    assert!(!r.reducer.running);
}
#[test]
fn attachment_rejects_game_table_and_player_switches_atomically() {
    let mut r = receiver();
    apply(
        &mut r,
        &[
            Message::TableDetails(table()),
            Message::Snapshot(snapshot()),
        ],
    );
    let before = r.reducer.view().unwrap();
    let mut bad = snapshot();
    bad.game_id = 8;
    assert!(r.apply_batch(vec![Message::Snapshot(bad)]).is_err());
    let mut bad = table();
    bad.table_id = 2;
    assert!(r.apply_batch(vec![Message::TableDetails(bad)]).is_err());
    let mut bad = table();
    bad.players.reverse();
    assert!(r.apply_batch(vec![Message::TableDetails(bad)]).is_err());
    assert_eq!(r.reducer.view().unwrap(), before);
}
#[tokio::test]
#[allow(clippy::result_large_err)] // Tungstenite requires its HTTP error response by value.
async fn authorized_request_preserves_bearer_without_official_origin() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{Message as WsMessage, client::IntoClientRequest},
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(
                    request.headers()["Authorization"],
                    "Bearer synthetic-seat-token"
                );
                assert!(!request.headers().contains_key("Origin"));
                Ok(response)
            },
        )
        .await
        .unwrap();
        let mut encoder = ServerEncoder::new();
        let frame = encoder
            .encode_batch(&[
                Message::TableDetails(table()),
                Message::Snapshot(snapshot()),
                Message::Question(choice(42)),
            ])
            .unwrap();
        socket.send(WsMessage::Binary(frame.into())).await.unwrap();
        let answer = socket.next().await.unwrap().unwrap();
        assert_eq!(answer.into_data(), command::answer(42, &[0]));
        let frame = encoder
            .encode_batch(&[Message::Snapshot(snapshot()), terminal(result())])
            .unwrap();
        socket.send(WsMessage::Binary(frame.into())).await.unwrap();
        socket.close(None).await.unwrap();
    });
    let mut request = format!("ws://{addr}/fixed-game")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Authorization",
        "Bearer synthetic-seat-token".parse().unwrap(),
    );
    let mut transport = Transport::connect_request(request).await.unwrap();
    let mut r = receiver();
    r.apply_batch(transport.receive().await.unwrap()).unwrap();
    let id = r.reducer.view().unwrap().decision.unwrap().id;
    let (question, values) = r
        .reducer
        .answer(&Answer::indices(id, vec![0]), None)
        .unwrap();
    transport
        .send(command::answer(question, &values))
        .await
        .unwrap();
    r.apply_batch(transport.receive().await.unwrap()).unwrap();
    assert!(!r.reducer.view().unwrap().game.running);
    assert!(r.reducer.view().unwrap().decision.is_none());
    task.await.unwrap();
}
