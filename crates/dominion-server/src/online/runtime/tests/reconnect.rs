use super::*;
use dominion_online::{server::ServerEncoder, transport::Failure};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    WebSocketStream, accept_async,
    tungstenite::{Message as WsMessage, client::IntoClientRequest},
};

struct Peer {
    socket: WebSocketStream<TcpStream>,
    encoder: ServerEncoder,
}

impl Peer {
    async fn accept(listener: &TcpListener) -> Self {
        let (stream, _) = listener.accept().await.unwrap();
        Self {
            socket: accept_async(stream).await.unwrap(),
            encoder: ServerEncoder::new(),
        }
    }

    async fn expect(&mut self, command: &[u8]) {
        let message = self.socket.next().await.unwrap().unwrap();
        assert_eq!(message.into_data().as_ref(), command);
    }

    async fn send(&mut self, messages: &[Message]) {
        let frame = self.encoder.encode_batch(messages).unwrap();
        self.socket
            .send(WsMessage::Binary(frame.into()))
            .await
            .unwrap();
    }
}

async fn connect_local(address: SocketAddr) -> dominion_online::Result<Transport> {
    Transport::connect_request(format!("ws://{address}").into_client_request().unwrap()).await
}

async fn no_probe() -> Result<Endpoint, String> {
    panic!("unexpected server probe")
}

fn finished(table: TableDetails) -> Message {
    let players = table
        .players
        .iter()
        .chain(&table.bots)
        .enumerate()
        .map(|(n, p)| model::PlayerResult {
            player_id: p.id,
            rank: n as i32 + 1,
            score: model::Score {
                total_points: 3,
                used_turns: 1,
                parts: vec![],
            },
            card_names: vec![],
            resign_index: -1,
            resignation_type: None,
        })
        .collect();
    Message::GameFinished {
        result: model::GameResult {
            table_id: table.table_id,
            game_id: 123,
            players,
            empty_piles: vec![],
            rating_type: None,
            auto_continue: false,
        },
        table,
        continue_allowed: false,
        match_completed: true,
    }
}

fn refused(reason: &'static str) -> Message {
    Message::Failed(Failure { reason, card: None })
}

fn queued(counter: i32) -> WsMessage {
    // AUTOMATCH_STATUS has no server-side encoder.
    let mut w = Writer::new();
    w.int(counter);
    w.int(20);
    w.int(0);
    w.boolean(true);
    w.int(0);
    w.int(0);
    w.long(0);
    for _ in 0..4 {
        w.double(0.0);
    }
    w.int(0);
    w.boolean(false);
    w.double(0.0);
    w.boolean(false);
    WsMessage::Binary(w.finish().into())
}

#[tokio::test]
async fn post_game_search_follows_the_directed_session_to_the_other_server() {
    for left_table in [false, true] {
        post_game_redirect(left_table).await;
    }
}

async fn post_game_redirect(left_table: bool) {
    let (_dir, mut runtime) = resuming_human_match();
    runtime.auto_endpoint = true;
    runtime.process(Message::Login(login(true))).unwrap();
    let mut game = snapshot();
    game.state.players[1].player_id = 99;
    runtime.process(Message::Snapshot(game)).unwrap();
    let end = finished(human_table(true));
    let alpha = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let beta = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let alpha_addr = alpha.local_addr().unwrap();
    let beta_addr = beta.local_addr().unwrap();
    let options = OnlineMatching {
        min_rating: 4,
        ..Default::default()
    };
    let criteria = matching::criteria(OnlineMode::Rated, &options, true).unwrap();
    let peer = tokio::spawn(async move {
        let mut first = Peer::accept(&alpha).await;
        first.send(std::slice::from_ref(&end)).await;
        first
            .expect(
                &Command::LeaveTable {
                    table: 10,
                    player: 42,
                }
                .encode()
                .unwrap(),
            )
            .await;
        if left_table {
            first
                .send(&[Message::PlayerLeft {
                    table_id: 10,
                    player_id: 42,
                }])
                .await;
            first.expect(&criteria).await;
        }
        first.send(&[Message::ReconnectInstruction(true)]).await;

        let mut next = Peer::accept(&beta).await;
        next.expect(&command::reconnect_session(
            42,
            "private-fixture-authority",
            VERSION,
        ))
        .await;
        next.send(&[Message::Succeeded(9), end]).await;
        // A repeated finish on the new socket must not recreate membership.
        next.expect(&criteria).await;
        next.socket.send(queued(2)).await.unwrap();
    });

    runtime.socket = Some(connect_local(alpha_addr).await.unwrap());
    runtime.receive().await.unwrap();
    let result = runtime.update.reducer.result.clone().unwrap();
    let error = runtime
        .start_match(OnlineMode::Rated, options.clone())
        .await
        .unwrap_err();
    assert!(error.contains("Alpha to Beta"), "{error}");
    assert!(!runtime.update.connected && !runtime.update.searching);
    runtime
        .reconnect(no_probe(), async |endpoint| {
            assert_eq!(endpoint, Endpoint::Beta);
            connect_local(beta_addr).await
        })
        .await
        .unwrap();
    assert!(runtime.update.connected && !runtime.update.searching);
    assert_eq!(runtime.update.reducer.result.as_ref(), Some(&result));
    runtime
        .start_match(OnlineMode::Rated, options)
        .await
        .unwrap();
    assert!(runtime.update.searching);
    peer.await.unwrap();
}

#[tokio::test]
async fn expired_session_uses_password_once_and_requires_a_fresh_owned_snapshot() {
    let (_dir, mut runtime) = active_runtime();
    let old = decision_id(&runtime);
    runtime.credentials = Some(Credentials {
        username: "fixture-account".into(),
        password: "pw".into(),
    });
    runtime.close().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let mut first = Peer::accept(&listener).await;
        first
            .expect(&command::reconnect_session(
                42,
                "private-fixture-authority",
                VERSION,
            ))
            .await;
        first.send(&[refused("LOGIN_REQUIRED")]).await;
        let mut next = Peer::accept(&listener).await;
        next.expect(&command::login("fixture-account", "pw", VERSION, 0))
            .await;
        let mut login = login(false);
        login.previous_reconnects = 1;
        next.send(&[Message::Login(login)]).await;
        next.expect(&command::reconnect_game()).await;
        next.send(&[
            Message::Snapshot(snapshot()),
            Message::Question(question(7)),
        ])
        .await;
    });
    runtime
        .reconnect(no_probe(), async |_| connect_local(addr).await)
        .await
        .unwrap();
    assert!(runtime.update.connected && !runtime.needs_snapshot);
    let current = decision_id(&runtime);
    assert_ne!(old, current);
    assert!(
        runtime
            .answer(Answer::indices(old, vec![1]), None)
            .await
            .is_err()
    );
    assert_eq!(decision_id(&runtime), current);
    peer.await.unwrap();
}

#[tokio::test]
async fn idle_password_reconnect_cannot_resume_an_unexpected_game_or_mask_a_rejection() {
    for reply in [
        Message::Login(login(true)),
        refused("SERVER_IS_DOWN"),
        Message::TimeoutInstruction,
        refused("LOGIN_REQUIRED"),
        Message::Login(login(false)),
    ] {
        let (_dir, mut runtime) = active_runtime();
        runtime.process(finished(table())).unwrap();
        runtime.credentials = Some(Credentials {
            username: "fixture-account".into(),
            password: "pw".into(),
        });
        runtime.close().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let idle = matches!(&reply, Message::Login(login) if !login.reconnecting);
        let peer = tokio::spawn(async move {
            let mut next = Peer::accept(&listener).await;
            next.expect(&command::login("fixture-account", "pw", VERSION, 0))
                .await;
            next.send(&[reply]).await;
        });
        let result = runtime
            .reconnect(no_probe(), async |_| connect_local(addr).await)
            .await;
        assert_eq!(result.is_ok(), idle, "{result:?}");
        assert_eq!(runtime.update.connected, idle);
        assert_eq!(runtime.table.is_none(), idle);
        assert!(runtime.update.reducer.result.is_some());
        assert_eq!(runtime.update.error, result.err());
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn a_second_directive_during_session_confirmation_fails_this_reconnect() {
    let (_dir, mut runtime) = active_runtime();
    runtime.process(finished(table())).unwrap();
    runtime
        .process(Message::ReconnectInstruction(false))
        .unwrap_err();
    runtime.close().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let mut peer = Peer::accept(&listener).await;
        peer.expect(&command::reconnect_session(
            42,
            "private-fixture-authority",
            VERSION,
        ))
        .await;
        peer.send(&[Message::Succeeded(10), Message::ReconnectInstruction(true)])
            .await;
    });
    let error = runtime
        .reconnect(no_probe(), async |endpoint| {
            assert_eq!(endpoint, Endpoint::Alpha);
            connect_local(addr).await
        })
        .await
        .unwrap_err();
    assert!(error.contains("Alpha to Beta"), "{error}");
    assert!(!runtime.update.connected && !runtime.update.searching);
    let error = runtime
        .reconnect(no_probe(), async |_| {
            panic!("a pinned account must not connect to Beta")
        })
        .await
        .unwrap_err();
    assert!(error.contains("pinned"), "{error}");
    peer.await.unwrap();
}

#[tokio::test]
async fn a_reconnect_without_confirmation_expires_and_revokes_the_decision() {
    let (_dir, mut runtime) = active_runtime();
    runtime.close().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let mut peer = Peer::accept(&listener).await;
        peer.expect(&command::reconnect_session(
            42,
            "private-fixture-authority",
            VERSION,
        ))
        .await;
        tokio::time::pause();
        tokio::time::advance(RECONNECT_TIMEOUT).await;
        peer
    });
    let error = runtime
        .reconnect(no_probe(), async |_| connect_local(addr).await)
        .await
        .unwrap_err();
    drop(peer.await.unwrap());
    assert!(error.contains("timed out"), "{error}");
    assert_eq!(runtime.update.error.as_deref(), Some(error.as_str()));
    assert!(!runtime.update.connected && !runtime.update.searching);
    assert!(runtime.update.reducer.view().unwrap().decision.is_none());
    assert!(runtime.needs_snapshot);
}

#[test]
fn a_reconnect_cannot_replace_the_owned_table_or_game() {
    let (_dir, mut runtime) = active_runtime();
    runtime.needs_snapshot = true;
    let mut other = table();
    other.table_id += 1;
    assert!(
        runtime
            .process(Message::TableJoined {
                table: other,
                show: true
            })
            .is_err()
    );
    let mut other = snapshot();
    other.game_id += 1;
    assert!(runtime.process(Message::Snapshot(other)).is_err());
    assert!(runtime.needs_snapshot);
    assert_eq!(runtime.table.unwrap().table_id, 10);
    assert_eq!(runtime.update.reducer.game_id, Some(123));
}

#[tokio::test]
async fn an_idle_timeout_probes_for_a_normal_server_before_logging_in() {
    let (_dir, mut runtime) = active_runtime();
    runtime.auto_endpoint = true;
    runtime.credentials = Some(Credentials {
        username: "fixture-account".into(),
        password: "pw".into(),
    });
    runtime.process(finished(table())).unwrap();
    let error = runtime.process(Message::TimeoutInstruction).unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    runtime.fail(error).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let mut peer = Peer::accept(&listener).await;
        peer.expect(&command::login("fixture-account", "pw", VERSION, 0))
            .await;
        peer.send(&[Message::Login(login(false))]).await;
    });
    runtime
        .reconnect(async { Ok(Endpoint::Beta) }, async |endpoint| {
            assert_eq!(endpoint, Endpoint::Beta);
            connect_local(addr).await
        })
        .await
        .unwrap();
    assert!(runtime.update.connected && !runtime.update.searching);
    assert!(runtime.table.is_none());
    assert!(runtime.update.reducer.result.is_some());
    peer.await.unwrap();
}
