//! Offline socket acceptance: `nix develop -c cargo test -p dominion-server --test official_host`.
use dominion_agent::{AgentView, Answer};
use dominion_engine::{Card, GameConfig};
use dominion_online::{
    catalog::{CardInfo, Catalog},
    reducer::Reducer,
    server::AttachedReceiver,
    transport::{Decoder, Message as ServerMessage, command},
};
use dominion_protocol::{
    Cost,
    api::{Created, NewGame},
    lobby::{HistoryQuery, Source},
};
use dominion_server::{
    Options,
    arena::{Arena, SESSION_COOKIE, Settings},
    hub::{Hub, Meta},
    official_host::Host,
};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    tungstenite::{
        self, Message,
        client::IntoClientRequest,
        http::{Request, StatusCode},
    },
};
use tokio_util::sync::CancellationToken;

const KINGDOM: [Card; 10] = [
    Card::Village,
    Card::Smithy,
    Card::Market,
    Card::Festival,
    Card::Laboratory,
    Card::Merchant,
    Card::CouncilRoom,
    Card::Cellar,
    Card::Chapel,
    Card::Workshop,
];
const WAIT: Duration = Duration::from_secs(5);

/// Deliberately synthetic, explicitly injected; no bundle/network is required.
fn catalog() -> Arc<Catalog> {
    let mut cards = vec![CardInfo {
        key: "BACK".into(),
        name: "Back".into(),
        cost: Cost::default(),
        types: vec![],
        kingdom: false,
    }];
    for card in [
        Card::Copper,
        Card::Silver,
        Card::Gold,
        Card::Estate,
        Card::Duchy,
        Card::Province,
        Card::Curse,
    ]
    .into_iter()
    .chain(KINGDOM)
    {
        let cost = card.cost().unwrap();
        cards.push(CardInfo {
            key: card.name().replace(' ', "").to_uppercase(),
            name: card.name().into(),
            cost: Cost {
                coins: cost.coins.into(),
                potions: cost.potions.into(),
                debt: cost.debt.into(),
            },
            types: card.types().names().map(String::from).collect(),
            kingdom: KINGDOM.contains(&card),
        });
    }
    let enums = [
        ("QuestionTypes", "PLAY GAIN DISCARD TRASH TRASH_UNIQUE SELECT_ZONE SELECT_TURN RESOLVE_ABILITY ORDER_CARDS BUY CHOOSE_MODE HOW_MUCH_REPAY HOW_MANY_SPEND TOPDECK BOTTOMDECK SET_ASIDE REVEAL PASS COMPLEX_OR COMPLEX_AND"),
        ("QuestionIds", "GAME_ACTION_PHASE GAME_MAY_PLAY_ACTION GAME_BUY_PHASE GAME_MAY_PLAY_TREASURES GAME_MAY_BUY GAME_MAY_START_BUYING CELLAR CHAPEL WORKSHOP"),
        ("DeclineButtonIds", "END_ACTION_PHASE END_BUY_PHASE END_NIGHT_PHASE END_TURN DONT_DISCARD DONT_TRASH DONT_GAIN"),
        ("CounterNames", "ACTIONS COINS POTIONS BUYS"),
        ("ExtraTurnTypes", "REGULAR"),
        ("Types", "Action Treasure Victory Curse"),
        ("ZoneNames", "HandZone DrawZone DiscardZone TrashZone SupplyCardZone BasicTreasureZone BasicVictoryZone Unused InPlayZone"),
        ("TimerPresetOptions", "RECKLESS HASTY FRIENDLY PATIENT OFF CUSTOM LEAGUE DOC_CHAMPIONSHIP"),
        ("TimeSettingsParameters", "INITIAL_TIMEOUT FREE_DECISION_TIME MAXIMUM_TIME START_OF_GAME_ADDED_TIME START_OF_GAME_MINIMUM_TIME START_OF_MY_TURN_ADDED_TIME START_OF_MY_TURN_MINIMUM_TIME START_OF_BUY_ADDED_TIME START_OF_BUY_MINIMUM_TIME NOT_MY_TURN_ADDED_TIME NOT_MY_TURN_MINIMUM_TIME BUY_DECISION_ADDED_TIME BUY_DECISION_MINIMUM_TIME DEFAULT_DECISION_ADDED_TIME DEFAULT_DECISION_MINIMUM_TIME START_OF_OTHER_TURN_ADDED_TIME START_OF_OTHER_TURN_MINIMUM_TIME LOW_THRESHOLD VERY_LOW_THRESHOLD"),
        ("ResignationTypes", "MANUAL MANUAL_FROM_RECONNECT FORCE_RESIGNED TIMED_OUT"),
    ].into_iter().map(|(name, entries)| (name.into(), entries.split_whitespace().map(String::from).collect())).collect();
    Arc::new(Catalog {
        cards,
        enums,
        labels: BTreeMap::new(),
    })
}

struct Listener {
    inner: TcpListener,
    tiny_buffer: bool,
}
impl axum::serve::Listener for Listener {
    type Io = TcpStream;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (socket, address) = self.inner.accept().await.unwrap();
        if self.tiny_buffer {
            socket2::SockRef::from(&socket)
                .set_send_buffer_size(1024)
                .unwrap();
        }
        (socket, address)
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

struct Server {
    addr: SocketAddr,
    hub: Arc<Hub>,
    arena: Arc<Arena>,
    catalog: Arc<Catalog>,
    stop: CancellationToken,
    http: reqwest::Client,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl Server {
    async fn start(enabled: bool) -> Self {
        Self::with_hub(Arc::new(Hub::default()), enabled, None).await
    }
    async fn with_hub(hub: Arc<Hub>, enabled: bool, public_url: Option<String>) -> Self {
        Self::with_catalog(hub, enabled, public_url, catalog()).await
    }
    async fn with_catalog(
        hub: Arc<Hub>,
        enabled: bool,
        public_url: Option<String>,
        catalog: Arc<Catalog>,
    ) -> Self {
        Self::with_buffers(hub, enabled, public_url, catalog, false).await
    }
    async fn with_buffers(
        hub: Arc<Hub>,
        enabled: bool,
        public_url: Option<String>,
        catalog: Arc<Catalog>,
        tiny_buffer: bool,
    ) -> Self {
        let arena = Arena::open(hub.clone(), Settings::default()).unwrap();
        let stop = CancellationToken::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let listener = Listener {
            inner: listener,
            tiny_buffer,
        };
        let options = Options {
            official_server: enabled.then(|| Host::new(catalog.clone()).unwrap()),
            public_url,
            ..Options::default()
        };
        let router = dominion_server::http::router(
            arena.clone(),
            &options,
            vec!["127.0.0.1".into()],
            stop.clone(),
        );
        let stopped = stop.clone();
        tokio::spawn(async move {
            axum::serve(listener, router.into_make_service())
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
                .unwrap();
        });
        Self {
            addr,
            hub,
            arena,
            catalog,
            stop,
            http: reqwest::Client::new(),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
    async fn create(&self, seed: u64) -> Created {
        let request = NewGame {
            players: 2,
            kingdom: KINGDOM.map(|card| card.name().into()).to_vec(),
            seed: Some(seed),
            ..NewGame::default()
        };
        self.http
            .post(self.url("/api/games"))
            .body(serde_json::to_vec(&request).unwrap())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    fn request(&self, game: &str, seat: u8, token: &str) -> Request<()> {
        let mut request = format!(
            "ws://{}/api/games/{game}/official/2.3.7?seat={seat}",
            self.addr
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        request
    }
    async fn attach(&self, created: &Created, seat: u8) -> Client {
        self.attach_request(
            self.request(&created.game_id, seat, &created.seats[seat as usize].token),
            seat,
        )
        .await
    }
    async fn attach_request(&self, request: Request<()>, seat: u8) -> Client {
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let mut decoder = Decoder::new();
        let messages = loop {
            match timeout(WAIT, socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
            {
                Message::Binary(bytes) => break decoder.decode(&bytes).unwrap(),
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                other => panic!("expected initial snapshot, got {other:?}"),
            }
        };
        let player_id = messages
            .iter()
            .find_map(|message| match message {
                ServerMessage::Snapshot(snapshot) => {
                    Some(snapshot.state.players[seat as usize].player_id)
                }
                _ => None,
            })
            .expect("initial seat snapshot");
        let mut receiver = AttachedReceiver::new(Reducer::new(self.catalog.clone(), player_id));
        receiver.apply_batch(messages.clone()).unwrap();
        let last_question = messages.iter().find_map(|m| {
            if let ServerMessage::Question(q) = m {
                Some(q.index)
            } else {
                None
            }
        });
        Client {
            socket,
            decoder,
            receiver,
            last_question,
        }
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
struct Client {
    socket: Socket,
    decoder: Decoder,
    receiver: AttachedReceiver,
    last_question: Option<i32>,
}
impl Client {
    fn view(&self) -> AgentView {
        self.receiver.reducer.view().unwrap()
    }
    async fn update(&mut self) -> Vec<ServerMessage> {
        loop {
            let message = timeout(WAIT, self.socket.next())
                .await
                .expect("socket update timed out")
                .expect("socket ended")
                .unwrap();
            match message {
                Message::Binary(bytes) => {
                    let messages = self.decoder.decode(&bytes).unwrap();
                    self.last_question = messages.iter().find_map(|m| {
                        if let ServerMessage::Question(q) = m {
                            Some(q.index)
                        } else {
                            None
                        }
                    });
                    self.receiver.apply_batch(messages.clone()).unwrap();
                    return messages;
                }
                Message::Ping(bytes) => self.socket.send(Message::Pong(bytes)).await.unwrap(),
                message => panic!("expected binary update, got {message:?}"),
            }
        }
    }
    fn encode(&mut self, answer: &Answer) -> Vec<u8> {
        let (index, values) = self.receiver.reducer.answer(answer, None).unwrap();
        command::answer(index, &values)
    }
    async fn send(&mut self, bytes: Vec<u8>) {
        self.socket
            .send(Message::Binary(bytes.into()))
            .await
            .unwrap();
    }
    async fn closed(&mut self, code: u16) {
        loop {
            let incoming = timeout(WAIT, self.socket.next())
                .await
                .expect("close timed out")
                .expect("no close frame")
                .unwrap();
            match incoming {
                Message::Close(Some(frame)) => {
                    assert_eq!(u16::from(frame.code), code, "{frame:?}");
                    return;
                }
                Message::Binary(bytes) => {
                    self.decoder.decode(&bytes).unwrap();
                }
                Message::Ping(bytes) => self.socket.send(Message::Pong(bytes)).await.unwrap(),
                _ => {}
            }
        }
    }
}

fn assert_shared(native: &AgentView, wire: &AgentView) {
    assert_eq!(native.game.running, wire.game.running);
    assert_eq!(native.seat, wire.seat);
    if native.game.running {
        assert_eq!(native.turn.player, wire.turn.player);
        assert_eq!(native.turn.actions, wire.turn.actions);
        assert_eq!(native.turn.buys, wire.turn.buys);
        assert_eq!(native.turn.coins, wire.turn.coins);
    }
    let native_you = native.you.as_ref().unwrap();
    let wire_you = wire.you.as_ref().unwrap();
    assert_eq!(native_you.hand, wire_you.hand);
    assert_eq!(native_you.deck.count, wire_you.deck.count);
    assert_eq!(native_you.discard.count, wire_you.discard.count);
    assert_eq!(native_you.in_play.known, wire_you.in_play.known);
    assert_eq!(native.opponents[0].hand.count, wire.opponents[0].hand.count);
    assert_eq!(wire.opponents[0].hand.unknown, wire.opponents[0].hand.count);
    assert_eq!(wire.opponents[0].hand.known, BTreeMap::new());
    let supply = |view: &AgentView| {
        view.supply
            .iter()
            .map(|pile| (pile.card.clone(), pile.count))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(supply(native), supply(wire));
    assert_eq!(native.decision.is_some(), wire.decision.is_some());
    if let Some(result) = &native.game.result {
        assert_eq!(result.players, wire.game.result.as_ref().unwrap().players);
    }
}

#[derive(Default)]
struct Responder {
    bought: BTreeMap<String, usize>,
    played: BTreeSet<String>,
    discarded: bool,
    trashed: bool,
    gained: bool,
}
impl Responder {
    fn answer(&mut self, view: &AgentView) -> Answer {
        let d = view.decision.as_ref().unwrap();
        let choose = |indices| Answer::indices(&d.id, indices);
        if d.purpose == "discard" || d.purpose == "trash" {
            let mut indices: Vec<_> = d
                .options
                .iter()
                .filter(|o| {
                    o.card
                        .as_deref()
                        .is_some_and(|name| matches!(name, "Estate" | "Curse"))
                })
                .map(|o| o.index)
                .take(d.max.unwrap_or(4) as usize)
                .collect();
            if !indices.is_empty() {
                if d.purpose == "discard" {
                    self.discarded = true;
                } else {
                    self.trashed = true;
                }
            }
            if indices.is_empty()
                && let Some(decline) = d
                    .options
                    .iter()
                    .find(|o| matches!(o.action.as_str(), "decline" | "end-phase"))
            {
                indices.push(decline.index);
            }
            return choose(indices);
        }
        if d.purpose == "gain" {
            self.gained = true;
            let option = d
                .options
                .iter()
                .find(|o| o.card.as_deref() == Some("Silver"))
                .unwrap_or(&d.options[0]);
            return choose(vec![option.index]);
        }
        // Play cantrips first, then extra Actions, then draw/terminal Actions.
        let priority = [
            "Village",
            "Festival",
            "Market",
            "Laboratory",
            "Merchant",
            "Cellar",
            "Chapel",
            "Workshop",
            "Council Room",
            "Smithy",
            "Gold",
            "Silver",
            "Copper",
        ];
        for name in priority {
            if let Some(option) = d
                .options
                .iter()
                .find(|o| o.action == "play" && o.card.as_deref() == Some(name))
            {
                self.played.insert(name.into());
                return choose(vec![option.index]);
            }
        }
        let order = [
            "Chapel",
            "Cellar",
            "Workshop",
            "Village",
            "Merchant",
            "Smithy",
            "Market",
            "Laboratory",
            "Festival",
            "Council Room",
        ];
        for name in order {
            if self.bought.get(name).copied().unwrap_or(0) == 0
                && let Some(o) = d
                    .options
                    .iter()
                    .find(|o| o.action == "buy" && o.card.as_deref() == Some(name))
            {
                *self.bought.entry(name.into()).or_default() += 1;
                return choose(vec![o.index]);
            }
        }
        for name in ["Province", "Gold", "Silver"] {
            if let Some(o) = d
                .options
                .iter()
                .find(|o| o.action == "buy" && o.card.as_deref() == Some(name))
            {
                return choose(vec![o.index]);
            }
        }
        let end = d
            .options
            .iter()
            .find(|o| o.action == "end-phase")
            .expect("legal end phase");
        choose(vec![end.index])
    }
}

async fn complete_game(mixed: bool, owned: bool) {
    complete_game_on(Server::start(true).await, mixed, owned).await;
}

async fn complete_game_on(server: Server, mixed: bool, owned: bool) {
    let created = if owned {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let (_, first) = server.arena.guest(ip, Some("First")).unwrap();
        let (_, second) = server.arena.guest(ip, Some("Second")).unwrap();
        let owners = [first, second]
            .iter()
            .map(|token| Some(server.arena.identify(token).unwrap().user))
            .collect();
        server
            .hub
            .host(
                GameConfig::new(2, KINGDOM.to_vec(), 7),
                vec!["First".into(), "Second".into()],
                Meta {
                    source: Source::Table,
                    owners,
                    undo: dominion_protocol::lobby::UndoRule::Off,
                    ..Meta::default()
                },
            )
            .unwrap()
    } else {
        server.create(7).await
    };
    let mut clients = [
        server.attach(&created, 0).await,
        server.attach(&created, 1).await,
    ];
    let seats: Vec<_> = created
        .seats
        .iter()
        .map(|s| server.hub.find(&s.token).unwrap())
        .collect();
    let mut responders = [Responder::default(), Responder::default()];
    let mut steps = 0;
    loop {
        for i in 0..2 {
            assert_shared(&server.hub.view(&seats[i]).unwrap().0, &clients[i].view());
        }
        let Some(i) = clients.iter().position(|c| c.view().decision.is_some()) else {
            break;
        };
        let answer = responders[i].answer(&clients[i].view());
        if mixed && i == 1 {
            let native = server.hub.view(&seats[i]).unwrap().0;
            let wire_decision = clients[i].view().decision.unwrap();
            let native_decision = native.decision.unwrap();
            let mut used = BTreeSet::new();
            let indices = answer
                .indices
                .as_ref()
                .unwrap()
                .iter()
                .filter_map(|index| {
                    let option = wire_decision
                        .options
                        .iter()
                        .find(|o| o.index == *index)
                        .unwrap();
                    if matches!(option.action.as_str(), "decline") {
                        return None;
                    }
                    let native_index = native_decision
                        .options
                        .iter()
                        .find(|o| {
                            (o.action == option.action || option.action == "select")
                                && o.card == option.card
                                && !used.contains(&o.index)
                        })
                        .unwrap_or_else(|| {
                            panic!("native equivalent missing for {option:?}: {native_decision:?}")
                        })
                        .index;
                    used.insert(native_index);
                    Some(native_index)
                })
                .collect();
            let native_answer = Answer::indices(&native_decision.id, indices);
            let response = server
                .http
                .post(server.url(&format!("/api/games/{}/answer", created.game_id)))
                .bearer_auth(&created.seats[i].token)
                .body(serde_json::to_vec(&native_answer).unwrap())
                .send()
                .await
                .unwrap();
            assert!(
                response.status().is_success(),
                "{}",
                response.text().await.unwrap()
            );
        } else {
            let bytes = clients[i].encode(&answer);
            clients[i].send(bytes).await;
        }
        for client in &mut clients {
            client.update().await;
        }
        steps += 1;
        assert!(steps < 6000, "game did not reach ordinary completion");
    }
    let final_view = clients[0].view();
    assert!(!final_view.game.running && final_view.decision.is_none());
    let result = final_view.game.result.as_ref().unwrap();
    assert_eq!(result.players.len(), 2);
    assert!(result.players.iter().all(|p| p.turns > 0 && !p.resigned));
    for card in KINGDOM {
        assert!(
            responders.iter().any(|r| r.played.contains(card.name())),
            "never played {card}"
        );
    }
    assert!(responders.iter().any(|r| r.discarded));
    assert!(responders.iter().any(|r| r.trashed));
    assert!(responders.iter().any(|r| r.gained));
    let log = server
        .hub
        .seat_view(&seats[0], Some(0))
        .unwrap()
        .view
        .since_last_decision;
    assert!(log.iter().any(|entry| entry.text.contains("shuffles")));
    assert!(
        log.iter()
            .any(|entry| entry.text.contains("Merchant resolves"))
    );
    if owned {
        assert!(server.hub.meta(&created.game_id).unwrap().recorded);
        assert_eq!(server.arena.profile("First").unwrap().stats.games, 1);
        assert_eq!(
            server
                .arena
                .history(&HistoryQuery::default())
                .unwrap()
                .games
                .len(),
            1
        );
    }
    let attached = server.attach(&created, 0).await;
    assert_eq!(attached.view().game.result, final_view.game.result);
    clients[0].send(command::reconnect_game()).await;
    let messages = clients[0].update().await;
    assert!(matches!(
        messages.as_slice(),
        [
            ServerMessage::TableDetails(_),
            ServerMessage::Snapshot(_),
            ServerMessage::GameFinished { .. }
        ]
    ));
    assert_eq!(clients[0].view().game.result, final_view.game.result);
    if owned {
        assert_eq!(server.arena.profile("First").unwrap().stats.games, 1);
    }
}

#[tokio::test]
async fn two_binary_receivers_complete_an_ordinary_game() {
    complete_game(false, false).await;
}

#[tokio::test]
async fn http_and_binary_answers_share_an_ordinary_game() {
    complete_game(true, false).await;
}

#[tokio::test]
async fn owned_binary_game_records_once_through_the_existing_arena_hook() {
    complete_game(false, true).await;
}

async fn refused(request: Request<()>, status: StatusCode) {
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), status),
        other => panic!("expected HTTP {status}, got {other:?}"),
    }
}

#[tokio::test]
async fn disabled_route_and_unsupported_profiles_refuse_before_upgrade() {
    let disabled = Server::start(false).await;
    let game = disabled.create(1).await;
    refused(
        disabled.request(&game.game_id, 0, &game.seats[0].token),
        StatusCode::NOT_FOUND,
    )
    .await;
    let server = Server::start(true).await;
    for (config, meta) in [
        (GameConfig::new(2, vec![Card::Moat], 1), Meta::default()),
        (GameConfig::new(2, vec![Card::Goons], 1), Meta::default()),
        (
            GameConfig::new(2, KINGDOM.to_vec(), 1),
            Meta {
                rated: true,
                ..Meta::default()
            },
        ),
        (
            GameConfig::new(2, KINGDOM.to_vec(), 1),
            Meta {
                timed: true,
                ..Meta::default()
            },
        ),
    ] {
        let game = server.hub.host(config, vec![], meta).unwrap();
        refused(
            server.request(&game.game_id, 0, &game.seats[0].token),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }
}

#[tokio::test]
async fn seat_tokens_origin_and_game_boundaries_are_enforced() {
    let server = Server::start(true).await;
    let game = server.create(1).await;
    let other = server.create(2).await;
    refused(
        server.request(&game.game_id, 0, "unknown"),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    refused(
        server.request("missing", 0, &game.seats[0].token),
        StatusCode::NOT_FOUND,
    )
    .await;
    refused(
        server.request(&game.game_id, 0, &other.seats[0].token),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    for origin in [
        "https://dominion.games",
        "null",
        "garbage",
        "https://127.0.0.1.evil.test",
        "http://127.0.0.1:1",
        "ftp://127.0.0.1",
        "http://user@127.0.0.1",
    ] {
        let mut request = server.request(&game.game_id, 0, &game.seats[0].token);
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        refused(request, StatusCode::FORBIDDEN).await;
    }
    let mut request = server.request(&game.game_id, 0, &game.seats[0].token);
    request
        .headers_mut()
        .insert("host", "evil.test".parse().unwrap());
    refused(request, StatusCode::FORBIDDEN).await;
    // As with Api::seat, a seat token selects its bound seat, regardless of query.
    let client = server
        .attach_request(server.request(&game.game_id, 1, &game.seats[0].token), 0)
        .await;
    assert_eq!(client.view().seat, Some(0));
}

#[tokio::test]
async fn account_cookie_bearer_bot_key_and_ongoing_revocation_reuse_arena_auth() {
    use dominion_protocol::lobby::Credentials;
    let server = Server::with_hub(
        Arc::new(Hub::default()),
        true,
        Some("https://arena.example".into()),
    )
    .await;
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let (_owner, token) = server
        .arena
        .register(
            ip,
            &Credentials {
                name: "Owner".into(),
                password: "a long test password".into(),
            },
            None,
        )
        .unwrap();
    let token = token.unwrap();
    let (_, unrelated) = server.arena.guest(ip, Some("Unrelated")).unwrap();
    let ident = server.arena.identify(&token).unwrap();
    let bot = server.arena.new_bot(&ident, "TestBot").unwrap();
    let game = server
        .hub
        .host(
            GameConfig::new(2, KINGDOM.to_vec(), 1),
            vec![],
            Meta {
                source: Source::Table,
                owners: vec![
                    Some(ident.user.clone()),
                    Some(server.arena.identify(&bot.key).unwrap().user),
                ],
                ..Meta::default()
            },
        )
        .unwrap();
    refused(
        server.request(&game.game_id, 0, &unrelated),
        StatusCode::FORBIDDEN,
    )
    .await;
    refused(
        server.request(&game.game_id, 1, &token),
        StatusCode::FORBIDDEN,
    )
    .await;
    let mut bearer = server
        .attach_request(server.request(&game.game_id, 0, &token), 0)
        .await;
    let mut cookie_request = server.request(&game.game_id, 0, &token);
    cookie_request.headers_mut().remove("authorization");
    cookie_request.headers_mut().insert(
        "cookie",
        format!("{SESSION_COOKIE}={token}").parse().unwrap(),
    );
    refused(cookie_request.clone(), StatusCode::FORBIDDEN).await;
    cookie_request
        .headers_mut()
        .insert("origin", "https://dominion.games".parse().unwrap());
    refused(cookie_request.clone(), StatusCode::FORBIDDEN).await;
    cookie_request
        .headers_mut()
        .insert("origin", "https://arena.example".parse().unwrap());
    let mut cookie = server.attach_request(cookie_request, 0).await;
    let mut bot_client = server
        .attach_request(server.request(&game.game_id, 1, &bot.key), 1)
        .await;
    server.arena.new_bot_key(&ident, "TestBot").unwrap();
    bot_client.send(command::ping()).await;
    bot_client.closed(1008).await;
    server.arena.logout(&token);
    bearer.send(command::ping()).await;
    bearer.closed(1008).await;
    cookie.send(command::ping()).await;
    cookie.closed(1008).await;
}

#[tokio::test]
async fn reconnect_replaces_plans_ping_preserves_them_and_stale_replies_do_not_mutate() {
    let server = Server::start(true).await;
    let game = server.create(3).await;
    let mut client = server.attach(&game, 0).await;
    let before = server
        .hub
        .seat_view(&server.hub.find(&game.seats[0].token).unwrap(), None)
        .unwrap();
    let answer = Responder::default().answer(&client.view());
    let stale_bytes = client.encode(&answer);
    let first_index = client.last_question.unwrap();
    client.send(command::ping()).await;
    let messages = client.update().await;
    assert!(matches!(messages.as_slice(), [ServerMessage::Pong]));
    client.send(command::reconnect_game()).await;
    let messages = client.update().await;
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, ServerMessage::Snapshot(s) if s.reconnect))
    );
    assert!(client.last_question.unwrap() > first_index);
    client.send(stale_bytes).await;
    client.closed(1008).await;
    let after = server
        .hub
        .seat_view(&server.hub.find(&game.seats[0].token).unwrap(), None)
        .unwrap();
    assert_eq!(before.version, after.version);
    assert_eq!(before.view, after.view);
    let mut fresh = server.attach(&game, 0).await;
    assert_eq!(fresh.last_question, Some(1));
    let answer = Responder::default().answer(&fresh.view());
    let bytes = fresh.encode(&answer);
    fresh.send(command::ping()).await;
    fresh.update().await;
    fresh.send(bytes.clone()).await;
    fresh.update().await;
    let committed = server
        .hub
        .seat_view(&server.hub.find(&game.seats[0].token).unwrap(), None)
        .unwrap();
    fresh.send(bytes).await;
    fresh.closed(1008).await;
    let final_state = server
        .hub
        .seat_view(&server.hub.find(&game.seats[0].token).unwrap(), None)
        .unwrap();
    assert_eq!(committed.version, final_state.version);
    assert_eq!(committed.view, final_state.view);
}

#[tokio::test]
async fn malformed_unsupported_text_and_oversized_frames_never_mutate() {
    let server = Server::start(true).await;
    let game = server.create(1).await;
    let seat = server.hub.find(&game.seats[0].token).unwrap();
    let before = server.hub.seat_view(&seat, None).unwrap();
    for (message, code) in [
        (Message::Binary(vec![0, 0].into()), 1002),
        (Message::Binary(49_i32.to_be_bytes().to_vec().into()), 1008),
        (Message::Text("login".into()), 1008),
        (Message::Binary(vec![0; 65537].into()), 1002),
    ] {
        let mut client = server.attach(&game, 0).await;
        client.socket.send(message).await.unwrap();
        client.closed(code).await;
        let after = server.hub.seat_view(&seat, None).unwrap();
        assert_eq!(before.version, after.version);
        assert_eq!(before.view, after.view);
    }
}

#[tokio::test]
async fn external_undo_invalidates_the_wire_plan_and_external_resign_finishes_every_receiver() {
    let server = Server::start(true).await;
    let game = server.create(1).await;
    let seat = server.hub.find(&game.seats[0].token).unwrap();
    let mut clients = [server.attach(&game, 0).await, server.attach(&game, 1).await];
    let answer = Responder::default().answer(&clients[0].view());
    let bytes = clients[0].encode(&answer);
    clients[0].send(bytes).await;
    for client in &mut clients {
        client.update().await;
    }
    let stale_answer = Responder::default().answer(&clients[0].view());
    let stale = clients[0].encode(&stale_answer);
    server.hub.undo(&seat, None).unwrap();
    for client in &mut clients {
        client.update().await;
    }
    clients[0].send(stale).await;
    clients[0].closed(1008).await;
    let mut reattached = server.attach(&game, 0).await;
    server.hub.resign(&seat, "resigned").unwrap();
    clients[1].update().await;
    reattached.update().await;
    for view in [clients[1].view(), reattached.view()] {
        assert!(!view.game.running && view.decision.is_none());
        let result = view.game.result.unwrap();
        assert_eq!(result.players[0].rank, 2);
        assert!(result.players[0].resigned);
        assert_eq!(result.players[1].rank, 1);
    }
}

#[tokio::test]
async fn socket_and_http_race_commits_once_and_persistence_reattaches_without_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let (hub, _) = Hub::open(tmp.path(), Duration::from_secs(3600)).unwrap();
    let server = Server::with_hub(Arc::new(hub), true, None).await;
    let game = server.create(1).await;
    let seat = server.hub.find(&game.seats[0].token).unwrap();
    let mut socket = server.attach(&game, 0).await;
    let native = server.hub.seat_view(&seat, None).unwrap();
    let answer = Answer::indices(
        native.view.decision.as_ref().unwrap().id.clone(),
        vec![native.view.decision.as_ref().unwrap().options[0].index],
    );
    let wire_answer = Responder::default().answer(&socket.view());
    let bytes = socket.encode(&wire_answer);
    let request = server
        .http
        .post(server.url(&format!("/api/games/{}/answer", game.game_id)))
        .bearer_auth(&game.seats[0].token)
        .body(serde_json::to_vec(&answer).unwrap());
    let (http, _) = tokio::join!(request.send(), socket.send(bytes));
    let http = http.unwrap();
    assert!(http.status().is_success() || http.status() == reqwest::StatusCode::CONFLICT);
    // Pong is ordered after any accepted answer; a losing stale socket closes.
    socket.send(command::ping()).await;
    loop {
        let incoming = timeout(WAIT, socket.socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match incoming {
            Message::Binary(bytes) => {
                if socket
                    .decoder
                    .decode(&bytes)
                    .unwrap()
                    .iter()
                    .any(|m| matches!(m, ServerMessage::Pong))
                {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let after = server.hub.seat_view(&seat, None).unwrap();
    assert_eq!(after.version, native.version + 1);
    let mut fresh = server.attach(&game, 0).await;
    assert_shared(&after.view, &fresh.view());
    let new_answer = Responder::default().answer(&fresh.view());
    let bytes = fresh.encode(&new_answer);
    let mut changed = server.hub.subscribe(&game.game_id).unwrap().version;
    fresh.send(bytes).await;
    // Drop before reading delivery: the committed state remains authoritative.
    drop(fresh);
    let mut observer = server.attach(&game, 1).await;
    observer.send(command::ping()).await;
    observer.update().await;
    timeout(WAIT, async {
        while *changed.borrow_and_update() <= after.version {
            changed.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let saved = server.hub.seat_view(&seat, None).unwrap();
    assert_eq!(saved.version, after.version + 1);
    server.stop.cancel();
    drop(observer);
    drop(socket);
    drop(server);
    // Wait for only this test's cancelled socket tasks to release their Store Arcs.
    let mut restarted = None;
    for _ in 0..100 {
        tokio::task::yield_now().await;
        if let Ok((hub, _)) = Hub::open(tmp.path(), Duration::from_secs(3600)) {
            restarted = Some(hub);
            break;
        }
    }
    let restarted = Server::with_hub(
        Arc::new(restarted.expect("cancelled sockets released store")),
        true,
        None,
    )
    .await;
    let client = restarted.attach(&game, 0).await;
    assert_shared(&saved.view, &client.view());
}

#[tokio::test]
async fn deterministic_draw_discard_trash_gain_and_merchant_bonus_cross_the_socket() {
    let server = Server::start(true).await;
    let game = server.create(7).await;
    let mut clients = [server.attach(&game, 0).await, server.attach(&game, 1).await];
    let mut responders = [Responder::default(), Responder::default()];
    let mut covered = BTreeSet::new();
    for _ in 0..1500 {
        let i = clients
            .iter()
            .position(|c| c.view().decision.is_some())
            .expect("scenario remains running");
        let before = clients[i].view();
        let answer = responders[i].answer(&before);
        let decision = before.decision.as_ref().unwrap();
        let selected: Vec<_> = answer
            .indices
            .as_ref()
            .unwrap()
            .iter()
            .map(|index| decision.options.iter().find(|o| o.index == *index).unwrap())
            .collect();
        let playing = selected
            .first()
            .filter(|o| o.action == "play")
            .and_then(|o| o.card.as_deref());
        let own = before.you.as_ref().unwrap();
        let first_merchant_silver = playing == Some("Silver")
            && own.in_play.known.get("Silver").copied().unwrap_or(0) == 0
            && own.in_play.known.get("Merchant").copied().unwrap_or(0) > 0;
        let bytes = clients[i].encode(&answer);
        clients[i].send(bytes).await;
        for client in &mut clients {
            client.update().await;
        }
        let after = clients[i].view();
        let now = after.you.as_ref().unwrap();
        if playing == Some("Council Room") {
            assert_eq!(now.hand.count, own.hand.count + 3);
            assert_eq!(
                after.opponents[0].hand.count,
                before.opponents[0].hand.count + 1
            );
            covered.insert("council");
        }
        if first_merchant_silver {
            assert_eq!(
                after.turn.coins.unwrap(),
                before.turn.coins.unwrap() + 2 + own.in_play.known["Merchant"]
            );
            covered.insert("merchant");
        }
        let selected_cards = selected.iter().filter(|o| o.card.is_some()).count() as u32;
        match decision.purpose.as_str() {
            "discard" if selected_cards > 0 => {
                assert_eq!(now.hand.count, own.hand.count);
                covered.insert("cellar");
            }
            "trash" if selected_cards > 0 => {
                assert_eq!(now.hand.count, own.hand.count - selected_cards);
                assert_eq!(after.trash.count, before.trash.count + selected_cards);
                covered.insert("chapel");
            }
            "gain" => {
                let card = selected[0].card.as_ref().unwrap();
                let count =
                    |view: &AgentView| view.supply.iter().find(|p| &p.card == card).unwrap().count;
                assert_eq!(count(&after), count(&before) - 1);
                assert_eq!(now.discard.count, own.discard.count + 1);
                covered.insert("workshop");
            }
            _ => {}
        }
        if covered.len() == 5 {
            return;
        }
    }
    panic!("missing deterministic scenario coverage: {covered:?}");
}

#[tokio::test]
async fn drop_shutdown_and_expiry_release_connection_guards() {
    let server = Server::start(true).await;
    let first = server.create(1).await;
    let second = server.create(2).await;
    let first_client = server.attach(&first, 0).await;
    let mut second_client = server.attach(&second, 0).await;
    assert_eq!(server.hub.expire(Duration::ZERO, Duration::ZERO), 0);
    drop(first_client);
    timeout(WAIT, async {
        loop {
            if server.hub.expire(Duration::ZERO, Duration::ZERO) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    server.stop.cancel();
    second_client.closed(1001).await;
    timeout(WAIT, async {
        loop {
            if server.hub.expire(Duration::ZERO, Duration::ZERO) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

/// DOMINION_ONLINE_TEST_BUNDLE=/path/to/body-2.3.7.js nix develop -c cargo test
/// -p dominion-server --test official_host pinned_catalog_socket_smoke -- --ignored
#[tokio::test]
#[ignore = "requires the separately obtained hash-pinned catalog bundle"]
async fn pinned_catalog_socket_smoke() {
    let path =
        std::env::var_os("DOMINION_ONLINE_TEST_BUNDLE").expect("set DOMINION_ONLINE_TEST_BUNDLE");
    let config = dominion_server::config::Config {
        official_server_enabled: Some(true),
        official_server_catalog_file: Some(path.clone().into()),
        ..Default::default()
    };
    assert!(Host::from_config(&config).unwrap().is_some());
    let catalog = Arc::new(Catalog::from_bundle(&std::fs::read(path).unwrap()).unwrap());
    let server = Server::with_catalog(Arc::new(Hub::default()), true, None, catalog).await;
    complete_game_on(server, false, false).await;
}

#[tokio::test]
async fn heartbeat_revocation_and_unresponsive_peers_release_their_seats() {
    let server = Server::start(true).await;
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let (_, token) = server.arena.guest(ip, Some("Heartbeat")).unwrap();
    let owner = server.arena.identify(&token).unwrap();
    let game = server
        .hub
        .host(
            GameConfig::new(2, KINGDOM.to_vec(), 1),
            vec![],
            Meta {
                owners: vec![Some(owner.user), None],
                ..Meta::default()
            },
        )
        .unwrap();
    let mut account = server
        .attach_request(server.request(&game.game_id, 0, &token), 0)
        .await;
    account.send(command::ping()).await;
    account.update().await;
    server.arena.logout(&token);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    account.closed(1008).await;
    let mut seat = server.attach(&game, 0).await;
    seat.send(command::ping()).await;
    seat.update().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    seat.closed(1001).await;
    assert_eq!(server.hub.expire(Duration::ZERO, Duration::ZERO), 1);
}

#[tokio::test]
async fn slow_consumer_has_bounded_sends_and_shutdown_cancels_backpressure() {
    for cancel in [false, true] {
        let server =
            Server::with_buffers(Arc::new(Hub::default()), true, None, catalog(), true).await;
        let game = server.create(1).await;
        let mut client = server.attach(&game, 0).await;
        if let MaybeTlsStream::Plain(stream) = client.socket.get_ref() {
            socket2::SockRef::from(stream)
                .set_recv_buffer_size(1024)
                .unwrap();
        }
        // A bounded input burst fits in TCP while its snapshots cannot fit in the
        // deliberately tiny send/receive buffers. The peer stops reading entirely.
        for _ in 0..256 {
            client.send(command::reconnect_game()).await;
        }
        if cancel {
            server.stop.cancel();
        }
        timeout(Duration::from_secs(12), async {
            loop {
                if server.hub.expire(Duration::ZERO, Duration::ZERO) == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("backpressured sender retained its connection guard");
    }
}

#[tokio::test]
async fn control_ping_and_empty_animation_ack_preserve_the_current_question() {
    let server = Server::start(true).await;
    let game = server.create(1).await;
    let mut client = server.attach(&game, 0).await;
    let id = client.view().decision.unwrap().id;
    client
        .socket
        .send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();
    let reply = timeout(WAIT, client.socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(reply, Message::Pong(vec![1, 2, 3].into()));
    client.send(command::animation_ack()).await;
    client.send(command::ping()).await;
    let messages = client.update().await;
    assert!(matches!(messages.as_slice(), [ServerMessage::Pong]));
    assert_eq!(client.view().decision.unwrap().id, id);
    let answer = Responder::default().answer(&client.view());
    let bytes = client.encode(&answer);
    client.send(bytes).await;
    client.update().await;
}
