//! Isolated Arena lifecycle sockets; no official service or credentials are used.
#[path = "common/official.rs"]
mod common;
use common::{KINGDOM, catalog};
use dominion_online::{
    catalog::Catalog,
    transport::{self, Decoder, Message as ServerMessage, outgoing::Command},
};
use dominion_protocol::lobby::{Credentials, NewTable, TableSpec};
use dominion_server::{
    Options,
    arena::{Arena, Settings},
    hub::Hub,
    official_host::Host,
};
use futures_util::{SinkExt, StreamExt};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
use tokio_util::sync::CancellationToken;
const WAIT: Duration = Duration::from_secs(10);
const VERSION: &str = "2.3.7";
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
struct Server {
    addr: SocketAddr,
    arena: Arc<Arena>,
    stop: CancellationToken,
    http: reqwest::Client,
    catalog: Arc<Catalog>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl Server {
    async fn new(catalog: Arc<Catalog>) -> Self {
        Self::with_hub(Arc::new(Hub::default()), catalog).await
    }
    async fn with_hub(hub: Arc<Hub>, catalog: Arc<Catalog>) -> Self {
        let arena = Arena::open(hub, Settings::default()).unwrap();
        let stop = CancellationToken::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let options = Options {
            official_server: Some(Host::new(catalog.clone()).unwrap()),
            ..Options::default()
        };
        let router = dominion_server::http::router(
            arena.clone(),
            &options,
            vec!["127.0.0.1".into()],
            stop.clone(),
        );
        let done = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router.into_make_service())
                .with_graceful_shutdown(done.cancelled_owned())
                .await
                .unwrap()
        });
        Self {
            addr,
            arena,
            stop,
            http: reqwest::Client::new(),
            catalog,
            task: Some(task),
        }
    }
    async fn close(mut self) {
        self.stop.cancel();
        timeout(WAIT, self.task.take().unwrap())
            .await
            .unwrap()
            .unwrap();
    }
    fn account(&self, name: &str) -> String {
        self.arena
            .register(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                &Credentials {
                    name: name.into(),
                    password: "synthetic-passphrase".into(),
                },
                None,
            )
            .unwrap()
            .1
            .unwrap()
    }
    async fn socket(&self, path: &str) -> Client {
        let mut request = format!("ws://{}{path}", self.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Origin", format!("http://{}", self.addr).parse().unwrap());
        let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        Client {
            socket,
            decoder: Decoder::new(),
        }
    }
    fn spec(&self) -> TableSpec {
        TableSpec {
            kingdom: KINGDOM.iter().map(|c| c.name().into()).collect(),
            sets: vec!["Base".into()],
            slots: Some(vec![]),
            colonies: Some(false),
            shelters: Some(false),
            rated: false,
            timed: false,
            undo: dominion_protocol::lobby::UndoRule::Off,
            ..TableSpec::default()
        }
    }
}
struct Client {
    socket: Socket,
    decoder: Decoder,
}
impl Client {
    async fn send(&mut self, command: Command) {
        self.socket
            .send(Message::Binary(command.encode().unwrap().into()))
            .await
            .unwrap();
    }
    async fn batch(&mut self) -> Vec<ServerMessage> {
        loop {
            match timeout(WAIT, self.socket.next())
                .await
                .expect("socket timed out")
                .expect("socket ended")
                .unwrap()
            {
                Message::Binary(bytes) => return self.decoder.decode(&bytes).unwrap(),
                Message::Ping(bytes) => self.socket.send(Message::Pong(bytes)).await.unwrap(),
                other => panic!("expected binary messages: {other:?}"),
            }
        }
    }
    async fn until(&mut self, predicate: impl Fn(&ServerMessage) -> bool) -> Vec<ServerMessage> {
        let mut all = vec![];
        loop {
            let batch = self.batch().await;
            let done = batch.iter().any(&predicate);
            all.extend(batch);
            if done {
                return all;
            }
        }
    }
    async fn login(&mut self, name: &str) -> transport::LoginSuccess {
        self.send(Command::Login {
            username: name.into(),
            password: "synthetic-passphrase".into(),
            version: VERSION.into(),
            language: 0,
        })
        .await;
        self.until(|m| matches!(m, ServerMessage::Login(_)))
            .await
            .into_iter()
            .find_map(|m| {
                if let ServerMessage::Login(login) = m {
                    Some(login)
                } else {
                    None
                }
            })
            .unwrap()
    }
}
fn required(server: &Server) -> transport::TableRule {
    transport::TableRule {
        id: 2,
        value: transport::RuleValue::Required(
            KINGDOM
                .iter()
                .map(|c| transport::RequiredSlot::Card(server.catalog.ordinal(c.name()).unwrap()))
                .collect(),
        ),
    }
}
#[tokio::test]
async fn root_and_versioned_probe_use_special_envelope_and_enforce_origin() {
    let server = Server::new(catalog()).await;
    for (path, version, expected) in [
        ("/", VERSION, 1_i32),
        ("/api/official/2.3.7", VERSION, 1),
        ("/", "wrong", 3),
    ] {
        let mut client = server.socket(path).await;
        client
            .send(Command::RequestServerState {
                version: version.into(),
                player: None,
                session: None,
            })
            .await;
        let bytes = match timeout(WAIT, client.socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Binary(bytes) => bytes,
            _ => panic!("probe response"),
        };
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[..4], &0_i32.to_be_bytes());
        assert_eq!(&bytes[4..], &expected.to_be_bytes());
    }
    for origin in [
        None,
        Some("https://dominion.games"),
        Some("https://evil.invalid"),
    ] {
        let mut request = format!("ws://{}/", server.addr)
            .into_client_request()
            .unwrap();
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("Origin", origin.parse().unwrap());
        }
        let err = tokio_tungstenite::connect_async(request).await.unwrap_err();
        assert!(
            matches!(err,tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==403)
        );
    }
    let response = server
        .http
        .get(format!("http://{}/", server.addr))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}
#[tokio::test]
async fn authentication_identity_resume_revocation_and_bridge_are_shared() {
    let server = Server::new(catalog()).await;
    let native = server.account("Alice");
    server.account("Bobby");
    let mut client = server.socket("/").await;
    let login = client.login("Alice").await;
    assert_eq!(login.player_id, 1);
    let mut resumed = server.socket("/api/official/2.3.7").await;
    resumed
        .send(Command::LoginWithSession {
            player: 2,
            session: login.session_id().into(),
            version: VERSION.into(),
            language: 0,
        })
        .await;
    assert!(
        resumed
            .until(|m| matches!(m, ServerMessage::Failed(_)))
            .await
            .iter()
            .any(|m| matches!(m,ServerMessage::Failed(f) if f.reason=="INVALID_SESSION_ID"))
    );
    resumed
        .send(Command::LoginWithSession {
            player: login.player_id,
            session: login.session_id().into(),
            version: VERSION.into(),
            language: 0,
        })
        .await;
    resumed
        .until(|m| matches!(m, ServerMessage::Login(_)))
        .await;
    let response = server
        .http
        .post(format!("http://{}/api/official/launch-code", server.addr))
        .bearer_auth(&native)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let code: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let mut bridge = server.socket("/").await;
    bridge
        .send(Command::LoginWithCode {
            code: code["code"].as_str().unwrap().into(),
            version: VERSION.into(),
        })
        .await;
    bridge.until(|m| matches!(m, ServerMessage::Login(_))).await;
    let mut again = server.socket("/").await;
    again
        .send(Command::LoginWithCode {
            code: code["code"].as_str().unwrap().into(),
            version: VERSION.into(),
        })
        .await;
    again
        .until(|m| matches!(m,ServerMessage::Failed(f) if f.reason=="NO_SUCH_CODE"))
        .await;
    server.arena.logout(login.session_id());
    resumed.send(Command::Ping {}).await;
    loop {
        match timeout(WAIT, resumed.socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), 1008);
                break;
            }
            Message::Binary(_) => {}
            Message::Ping(bytes) => resumed.socket.send(Message::Pong(bytes)).await.unwrap(),
            _ => {}
        }
    }
    assert!(server.arena.identify(&native).is_some());
    let revoked = server
        .http
        .post(format!("http://{}/api/official/launch-code", server.addr))
        .bearer_auth(&native)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let revoked: serde_json::Value =
        serde_json::from_slice(&revoked.bytes().await.unwrap()).unwrap();
    server.arena.logout(&native);
    let mut denied = server.socket("/").await;
    denied
        .send(Command::LoginWithCode {
            code: revoked["code"].as_str().unwrap().into(),
            version: VERSION.into(),
        })
        .await;
    denied
        .until(|m| matches!(m,ServerMessage::Failed(f) if f.reason=="NO_SUCH_CODE"))
        .await;
}
#[tokio::test]
async fn same_table_rules_readiness_chat_records_and_second_game_share_arena() {
    let server = Server::new(catalog()).await;
    let token_a = server.account("Alice");
    let token_b = server.account("Bobby");
    let a = server.arena.identify(&token_a).unwrap();
    let b = server.arena.identify(&token_b).unwrap();
    let mut client_a = server.socket("/").await;
    client_a.login("Alice").await;
    let mut client_b = server.socket("/").await;
    client_b.login("Bobby").await;
    client_a
        .send(Command::NewTableRequest {
            rules: vec![required(&server)],
        })
        .await;
    let joined = client_a
        .until(|m| matches!(m, ServerMessage::TableJoined { .. }))
        .await;
    let id = joined
        .iter()
        .find_map(|m| {
            if let ServerMessage::TableJoined { table, .. } = m {
                Some(table.table_id)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        server.arena.lobby(Some(&a)).table.unwrap().id,
        format!("t{id}")
    );
    client_b
        .send(Command::JoinTable {
            table: id,
            as_player: false,
        })
        .await;
    client_b
        .until(|m| matches!(m, ServerMessage::Failed(_)))
        .await;
    assert!(server.arena.lobby(Some(&b)).table.is_none());
    client_b
        .send(Command::JoinTable {
            table: id,
            as_player: true,
        })
        .await;
    client_b
        .until(|m| matches!(m, ServerMessage::TableJoined { .. }))
        .await;
    client_b
        .send(Command::ChangeTableRule {
            table: id,
            rule: transport::TableRule {
                id: 19,
                value: transport::RuleValue::Boolean(true),
            },
        })
        .await;
    client_b
        .until(|m| matches!(m, ServerMessage::Failed(_)))
        .await;
    assert!(!server.arena.table(&format!("t{id}")).unwrap().spec.rated);
    client_a
        .send(Command::SendChat {
            receiver: "table".into(),
            sender: "forged".into(),
            text: "hello native".into(),
        })
        .await;
    client_a
        .until(|m| matches!(m,ServerMessage::Chat{text,..} if text=="hello native"))
        .await;
    assert_eq!(
        server
            .arena
            .chat_log(&format!("table-t{id}"), 0)
            .messages
            .last()
            .unwrap()
            .from,
        "Alice"
    );
    server
        .arena
        .transform_spec(&a, &format!("t{id}"), |spec| {
            spec.randomize_order = false;
            Ok(())
        })
        .unwrap();
    let mut old_index = None;
    for round in 0..2 {
        client_a
            .send(Command::ReadyToStart {
                table: id,
                ready: true,
            })
            .await;
        client_a
            .until(|m| matches!(m,ServerMessage::TableDetails(t) if t.ready.first()==Some(&true)))
            .await;
        let ready_native = server.arena.ready(&b, &format!("t{id}"), true).unwrap();
        let game = ready_native.game.unwrap();
        let updates = client_a
            .until(|m| matches!(m, ServerMessage::Snapshot(_)))
            .await;
        assert!(updates.iter().any(|m|matches!(m,ServerMessage::GameStarted{game_id,..} if *game_id==game[1..].parse::<i64>().unwrap())));
        let current = updates.iter().find_map(|m| {
            if let ServerMessage::Question(q) = m {
                Some(q.index)
            } else {
                None
            }
        });
        if round == 1 {
            if let Some(index) = old_index {
                client_a
                    .send(Command::AnswerQuestion {
                        question: index,
                        values: vec![],
                        autoplay: false,
                    })
                    .await;
                client_a
                    .until(|m| matches!(m,ServerMessage::Failed(f) if f.reason=="ILLEGAL_ANSWER"))
                    .await;
            }
            if let (Some(old), Some(new)) = (old_index, current) {
                assert!(new > old);
            }
        }
        assert!(
            current.is_some(),
            "the deterministic first seat must have a question"
        );
        old_index = current.or(old_index);
        let seat = server.arena.hub.owned_in(&game, &b.user).pop().unwrap();
        server.arena.hub.resign(&seat, "resigned").unwrap();
        let final_messages = client_a
            .until(|m| matches!(m, ServerMessage::GameFinished { .. }))
            .await;
        assert!(final_messages.iter().any(|m|matches!(m,ServerMessage::Snapshot(s) if s.game_id==game[1..].parse::<i64>().unwrap())));
        assert!(
            server
                .arena
                .table(&format!("t{id}"))
                .unwrap()
                .game
                .is_none()
        );
        assert!(server.arena.hub.meta(&game).unwrap().recorded);
        client_a.send(Command::ReconnectGame {}).await;
        let final_again = client_a
            .until(|m| matches!(m, ServerMessage::GameFinished { .. }))
            .await;
        assert!(
            final_again
                .iter()
                .any(|m| matches!(m,ServerMessage::Snapshot(snapshot) if !snapshot.reconnect))
        );
        assert!(server.arena.hub.meta(&game).unwrap().recorded);
    }
    assert_eq!(server.arena.me(&a).unwrap().stats.games, 2);
}
#[tokio::test]
async fn native_start_runs_attached_capability_check_and_rule_edits_are_atomic() {
    let server = Server::new(catalog()).await;
    let token_a = server.account("Alice");
    let token_b = server.account("Bobby");
    let a = server.arena.identify(&token_a).unwrap();
    let b = server.arena.identify(&token_b).unwrap();
    let mut official = server.socket("/").await;
    official.login("Alice").await;
    let mut spec = server.spec();
    spec.timed = true;
    let table = server
        .arena
        .new_table(&a, &NewTable { spec, seats: None })
        .unwrap();
    server.arena.join_table(&b, &table.id, None).unwrap();
    server.arena.ready(&a, &table.id, true).unwrap();
    assert!(server.arena.ready(&b, &table.id, true).is_err());
    assert!(server.arena.table(&table.id).unwrap().game.is_none());
    server
        .arena
        .transform_spec(&a, &table.id, |spec| {
            spec.timed = false;
            Ok(())
        })
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let one = {
        let arena = server.arena.clone();
        let id = table.id.clone();
        let a = a.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            arena
                .transform_spec(&a, &id, |s| {
                    s.randomize_order = false;
                    Ok(())
                })
                .unwrap();
        })
    };
    barrier.wait();
    server
        .arena
        .transform_spec(&a, &table.id, |s| {
            s.watchable = false;
            Ok(())
        })
        .unwrap();
    one.join().unwrap();
    let spec = server.arena.table(&table.id).unwrap().spec;
    assert!(!spec.timed && !spec.randomize_order && !spec.watchable);
    server.arena.ready(&a, &table.id, true).unwrap();
    assert!(
        server
            .arena
            .ready(&b, &table.id, true)
            .unwrap()
            .game
            .is_some()
    );
}

#[tokio::test]
async fn restart_resumes_owned_game_without_recreating_old_lobby_membership() {
    let dir = tempfile::tempdir().unwrap();
    let (hub, _) = Hub::open(dir.path(), Duration::from_secs(86400)).unwrap();
    let server = Server::with_hub(Arc::new(hub), catalog()).await;
    let token = server.account("Alice");
    let other = server.account("Bobby");
    let a = server.arena.identify(&token).unwrap();
    let b = server.arena.identify(&other).unwrap();
    let table = server
        .arena
        .new_table(
            &a,
            &NewTable {
                spec: server.spec(),
                seats: None,
            },
        )
        .unwrap();
    server.arena.join_table(&b, &table.id, None).unwrap();
    server.arena.ready(&a, &table.id, true).unwrap();
    let game = server
        .arena
        .ready(&b, &table.id, true)
        .unwrap()
        .game
        .unwrap();
    server.close().await;
    let (hub, loaded) = Hub::open(dir.path(), Duration::from_secs(86400)).unwrap();
    assert_eq!(loaded.games, 1);
    let server = Server::with_hub(Arc::new(hub), catalog()).await;
    let mut client = server.socket("/").await;
    client
        .send(Command::ReconnectSession {
            player: 1,
            version: VERSION.into(),
            session: token.clone(),
        })
        .await;
    let messages = client
        .until(|m| matches!(m, ServerMessage::Snapshot(_)))
        .await;
    assert!(messages.iter().any(|m|matches!(m,ServerMessage::Snapshot(s) if !s.reconnect && s.game_id==game[1..].parse::<i64>().unwrap())));
    let lobby = server.arena.lobby(Some(&a));
    assert!(lobby.tables.is_empty() && lobby.table.is_none());
    assert_eq!(lobby.games.iter().filter(|g| g.game_id == game).count(), 1);
    let new = server
        .arena
        .new_table(
            &a,
            &NewTable {
                spec: server.spec(),
                seats: None,
            },
        )
        .unwrap();
    assert_ne!(new.id, table.id);
    client
        .send(Command::ReadyToStart {
            table: table.id[1..].parse().unwrap(),
            ready: true,
        })
        .await;
    client
        .until(|m| matches!(m, ServerMessage::Failed(_)))
        .await;
    assert!(server.arena.table(&new.id).unwrap().game.is_none());
}

#[tokio::test]
async fn unauthenticated_game_commands_and_invalid_packets_cannot_mutate() {
    let server = Server::new(catalog()).await;
    let token = server.account("Alice");
    let mut client = server.socket("/").await;
    client.send(Command::Resign {}).await;
    assert!(
        matches!(timeout(WAIT,client.socket.next()).await.unwrap().unwrap().unwrap(),Message::Close(Some(frame)) if u16::from(frame.code)==1008)
    );
    let mut client = server.socket("/").await;
    client.login("Alice").await;
    client
        .socket
        .send(Message::Binary(vec![0, 0, 0].into()))
        .await
        .unwrap();
    loop {
        match timeout(WAIT, client.socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), 1002);
                break;
            }
            Message::Binary(_) => {}
            Message::Ping(bytes) => client.socket.send(Message::Pong(bytes)).await.unwrap(),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        server
            .arena
            .lobby(server.arena.identify(&token).as_ref())
            .tables
            .is_empty()
    );
}

#[tokio::test]
async fn authentication_deadline_and_shutdown_close_only_owned_sockets() {
    let server = Server::new(catalog()).await;
    let mut waiting = server.socket("/").await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(16)).await;
    loop {
        match timeout(WAIT, waiting.socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), 1008);
                break;
            }
            Message::Ping(bytes) => waiting.socket.send(Message::Pong(bytes)).await.unwrap(),
            other => panic!("unexpected unauthenticated message {other:?}"),
        }
    }
    tokio::time::resume();
    server.account("Alice");
    let mut client = server.socket("/").await;
    client.login("Alice").await;
    server.stop.cancel();
    loop {
        match timeout(WAIT, client.socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), 1001);
                break;
            }
            Message::Binary(_) => {}
            Message::Ping(bytes) => client.socket.send(Message::Pong(bytes)).await.unwrap(),
            other => panic!("unexpected shutdown message {other:?}"),
        }
    }
}

#[tokio::test]
async fn expired_arena_sessions_cannot_be_revived_before_the_sweep() {
    let dir = tempfile::tempdir().unwrap();
    let (hub, _) = Hub::open(dir.path(), Duration::from_secs(86400)).unwrap();
    let server = Server::with_hub(Arc::new(hub), catalog()).await;
    let token = server.account("Alice");
    let db = rusqlite::Connection::open(dir.path().join("dominion.sqlite")).unwrap();
    db.execute("UPDATE sessions SET seen=0", []).unwrap();
    assert!(server.arena.identify(&token).is_none());
    let mut client = server.socket("/").await;
    client
        .send(Command::LoginWithSession {
            player: 1,
            version: VERSION.into(),
            session: token,
            language: 0,
        })
        .await;
    client
        .until(|m| matches!(m,ServerMessage::Failed(f) if f.reason=="INVALID_SESSION_ID"))
        .await;
}

#[tokio::test]
async fn unsupported_game_attachment_rejects_once_and_keeps_commands_responsive() {
    let server = Server::new(catalog()).await;
    let a = server.arena.identify(&server.account("Alice")).unwrap();
    let b = server.arena.identify(&server.account("Bobby")).unwrap();
    let mut spec = server.spec();
    spec.undo = dominion_protocol::lobby::UndoRule::Free;
    spec.randomize_order = false;
    let table = server
        .arena
        .new_table(&a, &NewTable { spec, seats: None })
        .unwrap();
    server.arena.join_table(&b, &table.id, None).unwrap();
    server.arena.ready(&a, &table.id, true).unwrap();
    let game = server
        .arena
        .ready(&b, &table.id, true)
        .unwrap()
        .game
        .unwrap();
    let mut client = server.socket("/").await;
    client.login("Alice").await;
    client
        .until(|m| matches!(m, ServerMessage::Failed(_)))
        .await;
    for _ in 0..2 {
        client.send(Command::Ping {}).await;
        let messages = client.until(|m| matches!(m, ServerMessage::Pong)).await;
        assert!(
            messages.len() < 20,
            "a rejected attachment must not retry in a tight loop"
        );
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, ServerMessage::Snapshot(_)))
        );
        let seat = server.arena.hub.owned_in(&game, &a.user).pop().unwrap();
        let view = server.arena.hub.view(&seat).unwrap().0;
        if let Some(d) = view.decision {
            server
                .arena
                .hub
                .answer(
                    &seat,
                    &dominion_agent::Answer::indices(&d.id, vec![d.options[0].index]),
                )
                .unwrap();
        }
    }
    client.send(Command::Logout {}).await;
    loop {
        match timeout(WAIT, client.socket.next()).await.unwrap() {
            None | Some(Ok(Message::Close(_))) => break,
            Some(Ok(Message::Ping(bytes))) => {
                client.socket.send(Message::Pong(bytes)).await.unwrap()
            }
            Some(Ok(Message::Binary(bytes))) => {
                assert!(client.decoder.decode(&bytes).unwrap().len() < 20);
            }
            other => panic!("unexpected logout response: {other:?}"),
        }
    }
}

#[tokio::test]
async fn retained_result_policy_is_independent_of_next_table_rules_and_leave_clears_membership() {
    let server = Server::new(catalog()).await;
    let a = server.arena.identify(&server.account("Alice")).unwrap();
    let b = server.arena.identify(&server.account("Bobby")).unwrap();
    let table = server
        .arena
        .new_table(
            &a,
            &NewTable {
                spec: server.spec(),
                seats: None,
            },
        )
        .unwrap();
    server.arena.join_table(&b, &table.id, None).unwrap();
    server.arena.ready(&a, &table.id, true).unwrap();
    let game = server
        .arena
        .ready(&b, &table.id, true)
        .unwrap()
        .game
        .unwrap();
    let seat = server.arena.hub.owned_in(&game, &b.user).pop().unwrap();
    server.arena.hub.resign(&seat, "resigned").unwrap();
    server
        .arena
        .transform_spec(&a, &table.id, |spec| {
            spec.undo = dominion_protocol::lobby::UndoRule::Free;
            spec.timed = true;
            spec.kingdom.reverse();
            Ok(())
        })
        .unwrap();
    let mut client = server.socket("/").await;
    let login = client.login("Alice").await;
    client
        .until(|m| matches!(m, ServerMessage::GameFinished { .. }))
        .await;
    client.send(Command::ReconnectGame {}).await;
    client
        .until(|m| matches!(m, ServerMessage::GameFinished { .. }))
        .await;
    assert_eq!(
        server.arena.table(&table.id).unwrap().spec.undo,
        dominion_protocol::lobby::UndoRule::Free
    );
    let id = table.id[1..].parse().unwrap();
    client
        .send(Command::LeaveTable {
            table: id,
            player: login.player_id,
        })
        .await;
    client.until(|m| matches!(m, ServerMessage::PlayerLeft { table_id, player_id } if *table_id == id && *player_id == login.player_id)).await;
    assert!(server.arena.lobby(Some(&a)).table.is_none());
    client.send(Command::Ping {}).await;
    let messages = client.until(|m| matches!(m, ServerMessage::Pong)).await;
    assert!(!messages.iter().any(|m| matches!(
        m,
        ServerMessage::GameFinished { .. } | ServerMessage::TableJoined { .. }
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the hash-pinned local original client bundle and Node dependencies"]
async fn original_consumer_lifecycle_and_rules() {
    let bundle = std::env::var("DOMINION_ORIGINAL_BUNDLE")
        .expect("set DOMINION_ORIGINAL_BUNDLE to the cached pinned body");
    let catalog = Arc::new(Catalog::from_bundle(&std::fs::read(&bundle).unwrap()).unwrap());
    let server = Server::new(catalog).await;
    server.account("fixture-a");
    server.account("fixture-b");
    let mixed = std::env::var("DOMINION_ORIGINAL_MIXED").is_ok_and(|v| v == "1");
    let fixture = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(fixture.path(),serde_json::to_vec(&serde_json::json!({"accounts":[{"username":"fixture-a","password":"synthetic-passphrase"},{"username":"fixture-b","password":"synthetic-passphrase"}],"steps":[],"originalNewTable":true,"chat":true,"bootstrap":true,"finalReconnect":true,"sessionResume":true,"lifecycleSafety":true,"mixedNative":if mixed {serde_json::json!(1)}else{serde_json::Value::Null}})).unwrap()).unwrap();
    let output = std::env::var("DOMINION_ORIGINAL_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("dominion-original-lifecycle-evidence"));
    let harness = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/protocol-proof/client-harness.mjs");
    let mut process = tokio::process::Command::new("node");
    process
        .arg(harness)
        .args([
            "--bundle",
            &bundle,
            "--endpoint",
            &format!("ws://{}/", server.addr),
            "--scenario",
            "lifecycle-and-rules",
            "--fixture",
        ])
        .arg(fixture.path())
        .arg("--output")
        .arg(&output)
        .kill_on_drop(true);
    let result = timeout(Duration::from_secs(120), process.output())
        .await
        .expect("original consumer harness exceeded deadline")
        .unwrap();
    assert!(
        result.status.success(),
        "original consumer failed: {}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    eprintln!("original consumer evidence: {}", output.display());
}
