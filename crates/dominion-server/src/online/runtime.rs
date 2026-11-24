use super::recordings::{self, Recorder};
use crate::store::Store;
use dominion_online::{
    catalog::{Catalog, VERSION},
    record::Input,
    reducer::Reducer,
    transport::{
        Endpoint, Group, Message, RequiredSlot, RuleValue, TableDetails, TableRule, TimerUpdate,
        Transport,
        ancillary::{PlayerProfile, ProfileResult},
        command,
        outgoing::Command,
    },
};
use dominion_protocol::{
    AgentView, Answer, GlossaryEntry,
    api::{OnlineMatching, OnlineMode},
    lobby::{ChatKind, ChatMessage, ClockInfo, ClockStatus, ClockView},
};
mod matching;
mod pairing;
use matching::HumanTable;
pub use pairing::PairTable;
use pairing::Pairing;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, Debug, Default, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewGame {
    /// Two to six seats; every opponent must be an official server bot.
    pub players: Option<u8>,
    /// Must equal players minus one (default 1).
    pub bots: Option<u8>,
    #[serde(default)]
    pub kingdom: Vec<String>,
    #[serde(default)]
    pub landscapes: Vec<String>,
}
impl NewGame {
    fn rules(&self, catalog: &Catalog) -> Result<(u8, Vec<TableRule>), String> {
        let players = self.players.unwrap_or(2);
        let bots = self.bots.unwrap_or(1);
        if !(2..=6).contains(&players) || bots != players - 1 {
            return Err("REFUSED: official backend requires 2–6 players and bots = players - 1; human games are unsupported".into());
        }
        Ok((
            bots,
            table_rules(catalog, players, &self.kingdom, &self.landscapes)?,
        ))
    }
}
/// A table's rules for `players` seats, with `kingdom` and `landscapes` required.
fn table_rules(
    catalog: &Catalog,
    players: u8,
    kingdom: &[String],
    landscapes: &[String],
) -> Result<Vec<TableRule>, String> {
    {
        if kingdom.len() > 10 {
            return Err("REJECTED: at most ten required Kingdom piles".into());
        }
        let mut required = Vec::new();
        for (names, landscape) in [(kingdom, false), (landscapes, true)] {
            for name in names {
                let ordinal = catalog
                    .ordinal(name)
                    .ok_or_else(|| format!("REJECTED: unknown official card {name:?}"))?;
                let card = catalog.card(ordinal).map_err(|e| e.to_string())?;
                if if landscape {
                    !card.types.iter().any(|t| {
                        matches!(
                            t.as_str(),
                            "Event"
                                | "Landmark"
                                | "Project"
                                | "Way"
                                | "Ally"
                                | "Trait"
                                | "Prophecy"
                        )
                    })
                } else {
                    !card.kingdom
                } {
                    return Err(format!(
                        "REJECTED: {name} is not a {}",
                        if landscape {
                            "landscape"
                        } else {
                            "Kingdom pile"
                        }
                    ));
                }
                if required.contains(&RequiredSlot::Card(ordinal)) {
                    return Err(format!("REJECTED: duplicate required card {name}"));
                }
                required.push(RequiredSlot::Card(ordinal));
            }
        }
        Ok(vec![
            TableRule {
                id: 2,
                value: RuleValue::Required(required),
            },
            TableRule {
                id: 9,
                value: RuleValue::Int(i32::from(players)),
            },
            TableRule {
                id: 10,
                value: RuleValue::Int(i32::from(players)),
            },
        ])
    }
}
#[derive(Clone, Debug)]
pub(crate) struct TimerSample {
    pub update: TimerUpdate,
    pub received_ms: u64,
    pub received_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimerMode {
    Off,
    Active,
    Unknown,
}

#[derive(Clone)]
pub(crate) struct Update {
    pub reducer: Reducer,
    pub error: Option<String>,
    pub connected: bool,
    pub searching: bool,
    pub queue_feedback: Vec<String>,
    pub mode: OnlineMode,
    /// The official server's latest timer for each player, with local receipt timestamps.
    pub timers: Vec<TimerSample>,
    /// The official game snapshot's timer settings, where a game is loaded.
    pub timer_settings: Option<dominion_online::model::TimerSettings>,
    /// When the last game finished, in milliseconds since the epoch.
    pub finished_at: Option<u64>,
    /// The official server's ids of the bots at the table.
    pub bots: Vec<i32>,
    pub chat: TableChat,
    /// The game is against another account this server hosts.
    pub paired: bool,
}
/// What has been said at the account's table, its last lines; a table it joins starts it
/// afresh.
#[derive(Clone, Default)]
pub(crate) struct TableChat {
    table: Option<i64>,
    pub lines: Vec<ChatMessage>,
}
impl TableChat {
    const KEPT: usize = 200;
    /// The official client's senders of server lines that are no player.
    const MATCH_SUMMARY: &str = "!match";
    const TIEBREAK_STARTER: &str = "!tiebreak";

    fn at(&mut self, table: i64) {
        if self.table != Some(table) {
            *self = Self {
                table: Some(table),
                lines: Vec::new(),
            };
        }
    }
    /// A `Chat` message to `table`: `sender` is `p<player id>`, or names a server line.
    fn said(&mut self, table: &TableDetails, sender: &str, text: String, reducer: &Reducer) {
        self.at(table.table_id);
        let (from, kind, text) = match sender {
            "" | Self::MATCH_SUMMARY => (String::new(), ChatKind::System, text),
            Self::TIEBREAK_STARTER => (
                String::new(),
                ChatKind::System,
                format!("{text} selected to start game 7"),
            ),
            _ => {
                let Some(id) = sender.strip_prefix('p').and_then(|id| id.parse().ok()) else {
                    return;
                };
                let mut seated = table.players.iter().chain(&table.bots);
                let member = (seated.clone().chain(&table.spectators)).find(|p| p.id == id);
                let playing = seated.any(|p| p.id == id)
                    || (reducer.state().ok())
                        .is_some_and(|s| s.players.iter().any(|p| p.player_id == id));
                let name = (reducer.names.get(&id).or(member.map(|p| &p.name)).cloned())
                    .unwrap_or_else(|| format!("player {id}"));
                let kind = if playing {
                    ChatKind::Player
                } else {
                    ChatKind::Watcher
                };
                (name, kind, text)
            }
        };
        let seq = self.lines.last().map_or(1, |l| l.seq + 1);
        self.lines.push(ChatMessage {
            seq,
            at: crate::hub::now_ms(),
            from,
            kind,
            text,
        });
        if self.lines.len() > Self::KEPT {
            self.lines.remove(0);
        }
    }
}
impl Update {
    fn timer_mode(&self) -> TimerMode {
        let Some(settings) = &self.timer_settings else {
            return TimerMode::Unknown;
        };
        let Some(preset) = usize::try_from(settings.preset)
            .ok()
            .and_then(|i| self.reducer.catalog.enums.get("TimerPresetOptions")?.get(i))
        else {
            return TimerMode::Unknown;
        };
        if preset == "OFF" {
            TimerMode::Off
        } else {
            TimerMode::Active
        }
    }

    /// The game's timers and availability metadata at `now_ms`. When `now_instant` is
    /// provided, running elapsed time is measured monotonically from receipt so wall-clock
    /// jumps cannot distort countdowns; otherwise `now_ms` is used directly (for tests).
    pub fn clock_observation_at(
        &self,
        now_ms: u64,
        now_instant: Option<Instant>,
    ) -> (Option<ClockView>, Option<ClockInfo>) {
        let Ok(state) = self.reducer.state() else {
            return (None, None);
        };
        let unavailable = || {
            (
                None,
                Some(ClockInfo {
                    status: ClockStatus::Unavailable,
                    as_of_ms: now_ms,
                    sampled_at_ms: Vec::new(),
                }),
            )
        };
        match self.timer_mode() {
            TimerMode::Off => {
                if self.timers.is_empty() {
                    return (
                        None,
                        Some(ClockInfo {
                            status: ClockStatus::Off,
                            as_of_ms: now_ms,
                            sampled_at_ms: Vec::new(),
                        }),
                    );
                }
                return unavailable();
            }
            TimerMode::Active | TimerMode::Unknown => {}
        }
        if self.timers.is_empty() {
            return unavailable();
        }
        let players = &state.players;
        let mut clock = ClockView {
            left: vec![0; players.len()],
            free: vec![0; players.len()],
            running: None,
            increment: None,
        };
        let mut sampled_at_ms = vec![0; players.len()];
        let mut running_count = 0usize;
        for (seat, player) in players.iter().enumerate() {
            let Some(sample) = self
                .timers
                .iter()
                .find(|s| s.update.player == player.player_id)
            else {
                return unavailable();
            };
            let (mut left, mut free) = (
                sample.update.time.max(0) as u64,
                sample.update.free_time.max(0) as u64,
            );
            if sample.update.running {
                running_count += 1;
                if running_count > 1 {
                    return unavailable();
                }
                let elapsed = match now_instant {
                    Some(now) => now
                        .saturating_duration_since(sample.received_at)
                        .as_millis() as u64,
                    None => now_ms.saturating_sub(sample.received_ms),
                };
                let from_free = elapsed.min(free);
                free -= from_free;
                left = left.saturating_sub(elapsed - from_free);
                clock.running = u8::try_from(seat).ok();
            }
            clock.left[seat] = left;
            clock.free[seat] = free;
            sampled_at_ms[seat] = sample.received_ms;
        }
        if clock.free.iter().all(|&f| f == 0) {
            clock.free.clear();
        }
        (
            Some(clock),
            Some(ClockInfo {
                status: ClockStatus::Available,
                as_of_ms: now_ms,
                sampled_at_ms,
            }),
        )
    }

    /// The game's timers and freshness metadata as of right now (wall + monotonic).
    pub fn clock_now(&self) -> (Option<ClockView>, Option<ClockInfo>) {
        self.clock_observation_at(crate::hub::now_ms(), Some(Instant::now()))
    }

    /// The game's timers as they stand at `now`, by seat.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clock(&self, now: u64) -> Option<ClockView> {
        self.clock_observation_at(now, None).0
    }

    pub fn view(&self) -> Result<Option<AgentView>, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if !self.connected {
            return Err("official connection is closed; call online_reconnect".into());
        }
        if self.reducer.game_id.is_none() {
            return Ok(None);
        }
        self.reducer.view().map(Some).map_err(|e| e.to_string())
    }
}
enum Request {
    Answer(
        Answer,
        Option<Vec<String>>,
        oneshot::Sender<Result<(), String>>,
    ),
    NewGame(NewGame, oneshot::Sender<Result<(), String>>),
    Match(
        OnlineMode,
        OnlineMatching,
        oneshot::Sender<Result<(), String>>,
    ),
    CancelMatch(oneshot::Sender<Result<(), String>>),
    Reconnect(oneshot::Sender<Result<(), String>>),
    Resign(oneshot::Sender<Result<(), String>>),
    /// Says a line at the account's table.
    Say(String, oneshot::Sender<Result<(), String>>),
    Disconnect(oneshot::Sender<Result<(), String>>),
    /// A player's profile (this account's when none) under a `RatingTypes` ordinal.
    Profile(Option<i32>, i32, Reply<PlayerProfile>),
    Results(i32, Reply<Vec<ProfileResult>>),
    /// Opens a private unrated table for the account with that player id to join.
    PairOpen(i32, PairTable, Reply<i64>),
    /// Joins the other account's table, made by that host for this account to play at.
    PairJoin(i64, i32, u8, oneshot::Sender<Result<(), String>>),
    /// Starts the table's game once that player has joined it.
    PairStart(i32, oneshot::Sender<Result<(), String>>),
    /// Leaves a paired table whose game hasn't started.
    PairAbandon(oneshot::Sender<Result<(), String>>),
}
type Reply<T> = oneshot::Sender<Result<T, String>>;
/// How long dominion.games has to answer a player-page request (its own client's limit).
const PROFILE_TIMEOUT: Duration = Duration::from_secs(15);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long each step of setting up a paired table may take.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(30);
/// What the official player page asks for: a profile with 30 history points, and 25
/// results.
const PROFILE_PERIOD: i32 = 25;
const PROFILE_HISTORY: i32 = 30;
const RESULTS: i32 = 25;

/// Player-page requests waiting for the server's reply, by what they ask: a rating type,
/// with the player for a profile.
struct Waiting<K, T>(Vec<(K, Reply<T>)>);
impl<K, T> Default for Waiting<K, T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<K: PartialEq, T: Clone> Waiting<K, T> {
    /// Adds a request; true when none of `kind` was waiting already, so it must be sent.
    fn wait(&mut self, kind: K, reply: Reply<T>) -> bool {
        self.0.retain(|(_, r)| !r.is_closed());
        let first = !self.0.iter().any(|(k, _)| *k == kind);
        self.0.push((kind, reply));
        first
    }
    /// Answers the requests of `kind`, or all of them.
    fn answer(&mut self, kind: Option<K>, result: Result<T, String>) {
        let (answered, waiting) = std::mem::take(&mut self.0)
            .into_iter()
            .partition(|(k, _)| kind.as_ref().is_none_or(|kind| kind == k));
        self.0 = waiting;
        for (_, reply) in answered {
            let _ = reply.send(result.clone());
        }
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
#[derive(Clone)]
pub struct Client {
    tx: mpsc::Sender<Request>,
    pub(crate) updates: watch::Receiver<Update>,
}
impl Client {
    pub async fn login(
        catalog: Arc<Catalog>,
        endpoint: impl Into<Option<Endpoint>>,
        username: &str,
        password: &str,
        store: Store,
        resume: bool,
    ) -> Result<Self, String> {
        let pinned = endpoint.into();
        let endpoint = match pinned {
            Some(endpoint) => endpoint,
            None => super::auto_endpoint().await?,
        };
        let mut socket = Transport::connect(endpoint)
            .await
            .map_err(|e| e.to_string())?;
        socket
            .send(command::login(username, password, VERSION, 0))
            .await
            .map_err(|e| e.to_string())?;
        let saved: Option<TableDetails> = if resume {
            match std::fs::read(store.dir().join("own-table.json")) {
                Ok(bytes) => {
                    Some(serde_json::from_slice(&bytes).map_err(|_| "invalid own-table metadata")?)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            }
        } else {
            None
        };
        let human: Option<HumanTable> = if resume {
            match std::fs::read(store.dir().join("own-match.json")) {
                Ok(bytes) => {
                    Some(serde_json::from_slice(&bytes).map_err(|_| "invalid own-match metadata")?)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            }
        } else {
            None
        };
        let saved = human.as_ref().map(|h| h.table.clone()).or(saved);
        let mode = human.as_ref().map_or(OnlineMode::Bots, |h| h.mode);
        let paired = human.as_ref().is_some_and(|h| h.paired);
        let mut runtime = Runtime {
            human,
            search: None,
            queue_status: None,
            outbox: Vec::new(),
            pending_match: Vec::new(),
            pairing: None,
            timer_presets: Vec::new(),
            profiles: Waiting::default(),
            results: Waiting::default(),
            socket: Some(socket),
            endpoint,
            auto_endpoint: pinned.is_none(),
            session: None,
            credentials: Some(Credentials {
                username: username.to_string(),
                password: password.to_string(),
            }),
            reconnect_to: None,
            authentication: Some(Authentication::Password),
            needs_snapshot: false,
            request_game: false,
            leaving_finished: false,
            completed_table: None,
            resuming: saved.is_some(),
            bots: saved
                .as_ref()
                .map_or_else(Vec::new, |t| t.bots.iter().map(|b| b.id).collect()),
            table: saved,
            update: Update {
                reducer: {
                    let mut reducer = Reducer::new(catalog, -1);
                    reducer.epoch = rand::random::<u64>() >> 16;
                    reducer
                },
                error: None,
                connected: false,
                searching: false,
                queue_feedback: Vec::new(),
                timers: Vec::new(),
                timer_settings: None,
                finished_at: None,
                bots: Vec::new(),
                chat: TableChat::default(),
                paired,
                mode,
            },
            recorder: Recorder::new(recordings::dir(store.dir())),
            store,
        };
        tokio::time::timeout(Duration::from_secs(20), async {
            while runtime.session.is_none() {
                runtime.receive().await?;
            }
            if runtime.needs_snapshot {
                runtime.safe_table()?;
                while runtime.needs_snapshot {
                    runtime.receive().await?;
                }
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| "official login timed out")??;
        let (tx, mut rx) = mpsc::channel(16);
        let (publish, updates) = watch::channel(runtime.update.clone());
        tokio::spawn(async move {
            loop {
                let request = if let Some(socket) = &mut runtime.socket {
                    tokio::select! {
                        request=rx.recv()=>request,
                        frame=socket.receive()=>{
                            if let Err(error) = runtime.handle_frame(frame).await {
                                runtime.fail(error).await;
                            }
                            publish.send_replace(runtime.published());
                            continue;
                        }
                    }
                } else {
                    rx.recv().await
                };
                let Some(request) = request else {
                    runtime.close().await;
                    break;
                };
                let (result, reply, stop) = match request {
                    Request::Answer(answer, expect, reply) => {
                        let result = runtime.answer(answer, expect).await;
                        (result, reply, false)
                    }
                    Request::NewGame(options, reply) => {
                        let result = match tokio::time::timeout(
                            Duration::from_secs(60),
                            runtime.new_game(options),
                        )
                        .await
                        {
                            Ok(result) => result,
                            Err(_) => {
                                let error = "official game startup timed out; reconnect explicitly"
                                    .to_string();
                                runtime.fail(error.clone()).await;
                                Err(error)
                            }
                        };
                        (result, reply, false)
                    }
                    Request::Match(mode, options, reply) => {
                        let result = tokio::time::timeout(
                            Duration::from_secs(20),
                            runtime.start_match(mode, options),
                        )
                        .await;
                        let result = match result {
                            Ok(r) => r,
                            Err(_) => {
                                let e =
                                    "matchmaking acknowledgement timed out; reconnect explicitly"
                                        .to_string();
                                runtime.fail(e.clone()).await;
                                Err(e)
                            }
                        };
                        (result, reply, false)
                    }
                    Request::CancelMatch(reply) => {
                        let result =
                            tokio::time::timeout(Duration::from_secs(20), runtime.cancel_match())
                                .await;
                        let result = match result {
                            Ok(r) => r,
                            Err(_) => {
                                let e = "matchmaking cancellation timed out; reconnect explicitly"
                                    .to_string();
                                runtime.fail(e.clone()).await;
                                Err(e)
                            }
                        };
                        (result, reply, false)
                    }
                    Request::Reconnect(reply) => {
                        runtime.close().await;
                        publish.send_replace(runtime.published());
                        let result = runtime
                            .reconnect(super::auto_endpoint(), Transport::connect)
                            .await;
                        (result, reply, false)
                    }
                    Request::Resign(reply) => {
                        let result = runtime.resign().await;
                        (result, reply, false)
                    }
                    Request::Say(text, reply) => {
                        let result = runtime.say(text).await;
                        (result, reply, false)
                    }
                    Request::Disconnect(reply) => {
                        runtime.close().await;
                        (Ok(()), reply, true)
                    }
                    Request::Profile(player, kind, reply) => {
                        runtime.request_profile(player, kind, reply).await;
                        publish.send_replace(runtime.published());
                        continue;
                    }
                    Request::Results(kind, reply) => {
                        runtime.request_results(kind, reply).await;
                        publish.send_replace(runtime.published());
                        continue;
                    }
                    Request::PairOpen(partner, options, reply) => {
                        let result = match tokio::time::timeout(
                            PAIRING_TIMEOUT,
                            runtime.pair_open(partner, options),
                        )
                        .await
                        {
                            Ok(result) => result,
                            Err(_) => {
                                let error =
                                    "opening the table timed out; reconnect explicitly".to_string();
                                runtime.fail(error.clone()).await;
                                Err(error)
                            }
                        };
                        publish.send_replace(runtime.published());
                        let _ = reply.send(result);
                        continue;
                    }
                    Request::PairJoin(table, host, timer, reply) => {
                        let result = tokio::time::timeout(
                            PAIRING_TIMEOUT,
                            runtime.pair_join(table, host, timer),
                        )
                        .await;
                        (runtime.pairing_step(result).await, reply, false)
                    }
                    Request::PairStart(guest, reply) => {
                        let result =
                            tokio::time::timeout(PAIRING_TIMEOUT, runtime.pair_start(guest)).await;
                        (runtime.pairing_step(result).await, reply, false)
                    }
                    Request::PairAbandon(reply) => {
                        let result =
                            tokio::time::timeout(PAIRING_TIMEOUT, runtime.abandon_pairing()).await;
                        (runtime.pairing_step(result).await, reply, false)
                    }
                };
                publish.send_replace(runtime.published());
                let _ = reply.send(result);
                if stop {
                    break;
                }
            }
        });
        Ok(Self { tx, updates })
    }
    pub(crate) fn current(&self) -> Update {
        self.updates.borrow().clone()
    }
    async fn call(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<(), String>>) -> Request,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(make(tx))
            .await
            .map_err(|_| "official client stopped")?;
        rx.await
            .map_err(|_| "official client stopped".to_string())?
    }
    pub async fn answer(&self, answer: Answer, expect: Option<Vec<String>>) -> Result<(), String> {
        self.call(|r| Request::Answer(answer, expect, r)).await
    }
    pub async fn new_game(&self, options: NewGame) -> Result<(), String> {
        self.call(|r| Request::NewGame(options, r)).await
    }
    pub async fn matchmaking(
        &self,
        mode: OnlineMode,
        options: OnlineMatching,
    ) -> Result<(), String> {
        self.call(|r| Request::Match(mode, options, r)).await
    }
    pub async fn cancel_matchmaking(&self) -> Result<(), String> {
        self.call(Request::CancelMatch).await
    }
    /// Opens a private unrated table for the account with player id `partner` to join;
    /// its id.
    pub(crate) async fn pair_open(&self, partner: i32, options: PairTable) -> Result<i64, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Request::PairOpen(partner, options, tx))
            .await
            .map_err(|_| "official client stopped")?;
        rx.await
            .map_err(|_| "official client stopped".to_string())?
    }
    /// Joins table `table`, which player `host` opened for this account with timer preset
    /// `timer`, and says it is ready.
    pub(crate) async fn pair_join(&self, table: i64, host: i32, timer: u8) -> Result<(), String> {
        self.call(|r| Request::PairJoin(table, host, timer, r))
            .await
    }
    /// Starts the paired table's game once player `guest` has joined it.
    pub(crate) async fn pair_start(&self, guest: i32) -> Result<(), String> {
        self.call(|r| Request::PairStart(guest, r)).await
    }
    /// Leaves the paired table if its game hasn't started.
    pub(crate) async fn pair_abandon(&self) -> Result<(), String> {
        self.call(Request::PairAbandon).await
    }
    pub async fn reconnect(&self) -> Result<(), String> {
        self.call(Request::Reconnect).await
    }
    pub async fn resign(&self) -> Result<(), String> {
        if self.current().view()?.is_none_or(|v| !v.game.running) {
            return Err("NO_GAME: no running official game".into());
        }
        self.call(Request::Resign).await
    }
    /// Says `text` at the account's table on dominion.games, as the account.
    pub async fn say(&self, text: String) -> Result<(), String> {
        self.call(|r| Request::Say(text, r)).await
    }
    pub async fn disconnect(&self) -> Result<(), String> {
        self.call(Request::Disconnect).await
    }
    /// This account's official profile under `RatingTypes` ordinal `kind`.
    pub(crate) async fn profile(&self, kind: i32) -> Result<PlayerProfile, String> {
        self.ask(|reply| Request::Profile(None, kind, reply)).await
    }
    /// Player `player`'s official profile under `RatingTypes` ordinal `kind`.
    pub(crate) async fn player_profile(
        &self,
        player: i32,
        kind: i32,
    ) -> Result<PlayerProfile, String> {
        self.ask(|reply| Request::Profile(Some(player), kind, reply))
            .await
    }
    /// This account's recent official results under `RatingTypes` ordinal `kind`.
    pub(crate) async fn results(&self, kind: i32) -> Result<Vec<ProfileResult>, String> {
        self.ask(|reply| Request::Results(kind, reply)).await
    }
    async fn ask<T>(&self, request: impl FnOnce(Reply<T>) -> Request) -> Result<T, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(request(tx))
            .await
            .map_err(|_| "official client stopped")?;
        tokio::time::timeout(PROFILE_TIMEOUT, rx)
            .await
            .map_err(|_| "dominion.games did not answer in time".to_string())?
            .map_err(|_| "official client stopped".to_string())?
    }
    pub async fn idle_check(&self, duration: Duration) -> Result<(), String> {
        let mut updates = self.updates.clone();
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            updates.borrow().view()?;
            match tokio::time::timeout_at(deadline, updates.changed()).await {
                Err(_) => return Ok(()),
                Ok(Err(_)) => return Err("official client stopped".into()),
                Ok(Ok(())) => {}
            }
        }
    }
    pub(crate) async fn wait(&self, seconds: u64) -> Result<Update, String> {
        let mut updates = self.updates.clone();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds.min(600));
        loop {
            let update = updates.borrow_and_update().clone();
            match update.view()? {
                None => return Ok(update),
                Some(v) if !v.game.running || v.decision.is_some() => return Ok(update),
                _ => {}
            }
            match tokio::time::timeout_at(deadline, updates.changed()).await {
                Err(_) => return Ok(updates.borrow().clone()),
                Ok(Err(_)) => return Err("official client stopped".into()),
                Ok(Ok(())) => {}
            }
        }
    }
}
// Kinds of the official `Metagame` message: an opponent asks to undo; a player who left
// may be resigned (the official client's "permanently resignable" players).
const UNDO_REQUEST: i32 = 0;
const TIMEOUT_OFFER: i32 = 1;

struct SessionSecret {
    player: i32,
    token: String,
}
/// Kept in memory only, to log in again once dominion.games has dropped the session.
struct Credentials {
    username: String,
    password: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Authentication {
    Session,
    Password,
    SessionExpired,
}

struct Runtime {
    socket: Option<Transport>,
    endpoint: Endpoint,
    auto_endpoint: bool,
    session: Option<SessionSecret>,
    credentials: Option<Credentials>,
    reconnect_to: Option<Endpoint>,
    authentication: Option<Authentication>,
    needs_snapshot: bool,
    request_game: bool,
    leaving_finished: bool,
    completed_table: Option<i64>,
    resuming: bool,
    table: Option<TableDetails>,
    bots: Vec<i32>,
    human: Option<HumanTable>,
    search: Option<OnlineMatching>,
    queue_status: Option<bool>,
    outbox: Vec<Vec<u8>>,
    pending_match: Vec<Message>,
    pairing: Option<Pairing>,
    /// The timer presets dominion.games offers, as login gave them.
    timer_presets: Vec<(i32, dominion_online::transport::TimerSettings)>,
    profiles: Waiting<(i32, i32), PlayerProfile>,
    results: Waiting<i32, Vec<ProfileResult>>,
    update: Update,
    store: Store,
    recorder: Recorder,
}
impl Runtime {
    /// What to publish now that everything taken together has been: the recorder's frame.
    fn published(&mut self) -> Update {
        self.recorder.frame(&self.update.reducer);
        if let Some(table) = &self.table {
            self.update.chat.at(table.table_id);
        }
        self.update.paired = self.human.as_ref().is_some_and(|h| h.paired);
        self.update.clone()
    }
    async fn close(&mut self) {
        let cancel = self
            .search
            .as_ref()
            .and_then(|options| matching::criteria(self.update.mode, options, false).ok());
        self.update.reducer.invalidate();
        self.update.connected = false;
        self.leaving_finished = false;
        self.update.searching = false;
        self.search = None;
        self.queue_status = None;
        self.update.queue_feedback.clear();
        self.authentication = None;
        self.outbox.clear();
        self.request_game = false;
        self.pending_match.clear();
        self.answer_player_page("official connection closed".into());
        if let Some(mut socket) = self.socket.take() {
            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                if let Some(bytes) = cancel {
                    let _ = socket.send(bytes).await;
                }
                let _ = socket.disconnect().await;
            })
            .await;
        }
    }
    async fn fail(&mut self, error: String) {
        self.close().await;
        self.update.error = Some(error);
    }
    async fn send(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let result = self
            .socket
            .as_mut()
            .ok_or("official connection closed; call online_reconnect")?
            .send(bytes)
            .await
            .map_err(|e| e.to_string());
        if let Err(error) = &result {
            self.fail(error.clone()).await;
        }
        result
    }
    async fn receive(&mut self) -> Result<(), String> {
        let result = self.receive_inner().await;
        if let Err(error) = &result {
            self.fail(error.clone()).await;
        }
        result
    }
    async fn receive_inner(&mut self) -> Result<(), String> {
        let frame = self
            .socket
            .as_mut()
            .ok_or("official connection closed")?
            .receive()
            .await;
        self.handle_frame(frame).await
    }
    async fn handle_frame(
        &mut self,
        frame: dominion_online::Result<Vec<Message>>,
    ) -> Result<(), String> {
        let messages = frame.map_err(|e| e.to_string())?;
        let mut acknowledgements = 0;
        // With no length prefix, the transport decodes the entire frame before returning it.
        for message in messages {
            if self.process(message)? {
                acknowledgements += 1;
            }
        }
        self.update.bots.clone_from(&self.bots);
        if self.authentication.is_some() {
            return Ok(());
        }
        if self.request_game {
            self.safe_table()?;
            self.request_game = false;
            self.send(command::reconnect_game()).await?;
        }
        for bytes in std::mem::take(&mut self.outbox) {
            self.send(bytes).await?;
        }
        for _ in 0..acknowledgements {
            self.send(command::animation_ack()).await?;
        }
        if self.needs_snapshot {
            return Ok(());
        }
        self.answer_forced().await
    }
    fn process(&mut self, message: Message) -> Result<bool, String> {
        if self.authentication.is_some() {
            match &message {
                Message::Login(_)
                    if self.authentication != Some(Authentication::SessionExpired) => {}
                Message::Failed(_)
                | Message::ReconnectInstruction(_)
                | Message::TimeoutInstruction => {}
                // SUCCESSFULLY_CONNECTED_NEW_SERVER / SUCCESSFULLY_CONNECTED_OLD_SERVER.
                Message::Succeeded(9 | 10)
                    if self.authentication == Some(Authentication::Session) =>
                {
                    self.authenticated();
                    self.request_game = self.needs_snapshot;
                    return Ok(false);
                }
                Message::Pong | Message::LobbyNotice => return Ok(false),
                _ => return Err("unexpected official message before authentication".into()),
            }
        }
        // Login can repeat a result after the account has already left its table.
        if let Message::GameFinished { result, .. } = &message
            && self.update.reducer.result.is_some()
            && self.update.reducer.game_id == Some(result.game_id)
        {
            return Ok(false);
        }
        let awaiting_match = self.awaiting_match();
        if awaiting_match
            && matches!(
                &message,
                Message::Snapshot(_)
                    | Message::Event(_)
                    | Message::Question(_)
                    | Message::Log(_)
                    | Message::GameFinished { .. }
                    | Message::Metagame { .. }
            )
        {
            if self.pending_match.len() >= 256 {
                return Err("REFUSED: too much gameplay before match confirmation".into());
            }
            self.pending_match.push(message);
            return Ok(false);
        }
        let completed_table = self.holds_finished_table();
        let reducer = &mut self.update.reducer;
        match message {
            // An answer can cross the official result in flight. The result is final.
            Message::Failed(f)
                if f.reason == "COMMAND_REQUIRES_GAME"
                    && completed_table
                    && !reducer.running
                    && reducer.result.is_some()
                    && !self.needs_snapshot
                    && self.pairing.is_none()
                    && self.search.is_none() => {}
            Message::Failed(f)
                if f.reason == "COMMAND_REQUIRES_TABLE"
                    && self.leaving_finished
                    && completed_table =>
            {
                self.table = None;
                self.bots.clear();
                self.human = None;
                self.leaving_finished = false;
                self.completed_table = None;
            }
            Message::Failed(f)
                if f.reason == "RATING_SERVICE_DOWN"
                    && !(self.profiles.is_empty() && self.results.is_empty()) =>
            {
                self.answer_player_page("dominion.games' rating service is down".into());
            }
            Message::Failed(f)
                if matches!(f.reason, "LOGIN_REQUIRED" | "INVALID_SESSION_ID")
                    && self.authentication == Some(Authentication::Session)
                    && self.credentials.is_some() =>
            {
                self.authentication = Some(Authentication::SessionExpired);
            }
            Message::Failed(f) if self.pairing.is_some() && f.reason != "SERVER_IS_DOWN" => {
                if let Some(pairing) = &mut self.pairing {
                    pairing.refused = Some(f.error().to_string());
                }
            }
            Message::Failed(f) => {
                if f.reason == "SERVER_IS_DOWN" {
                    self.reconnect_to = None;
                }
                return Err(f.error().to_string());
            }
            Message::Login(login) => {
                let existing_game = login.reconnecting || login.previous_reconnects > 0;
                if existing_game && !(self.resuming || reducer.running || self.needs_snapshot) {
                    return Err("REFUSED: login reports an existing game/session; no automatic takeover or reconnect was attempted".into());
                }
                if self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.player != login.player_id)
                {
                    return Err("REFUSED: reconnect authenticated a different account".into());
                }
                if self.resuming
                    && self.table.as_ref().is_none_or(|t| {
                        !t.players
                            .iter()
                            .chain(&t.reconnecting)
                            .any(|p| p.id == login.player_id)
                    })
                {
                    return Err("REFUSED: saved table belongs to another account".into());
                }
                if existing_game {
                    self.needs_snapshot = true;
                    // isReconnecting means the server is already sending the game.
                    self.request_game = !login.reconnecting;
                } else if self.needs_snapshot {
                    self.request_game = true;
                } else if self.resuming {
                    self.store.remove("own-table").map_err(|e| e.to_string())?;
                    self.store.remove("own-match").map_err(|e| e.to_string())?;
                    self.table = None;
                    self.bots.clear();
                    self.resuming = false;
                    self.human = None;
                    self.update.mode = OnlineMode::Bots;
                } else if completed_table && self.authentication == Some(Authentication::Password) {
                    self.table = None;
                    self.bots.clear();
                    self.human = None;
                }
                reducer.player_id = login.player_id;
                self.timer_presets.clone_from(&login.timer_presets);
                reducer
                    .names
                    .insert(login.player_id, login.username.clone());
                self.session = Some(SessionSecret {
                    player: login.player_id,
                    token: login.session_id().to_string(),
                });
                self.store
                    .save(
                        "session",
                        &serde_json::json!({
                        "account":login.username,
                        "clientVersion":VERSION,
                        "endpoint":self.endpoint.url(),
                        "authority":"memory-only"}
                        ),
                    )
                    .map_err(|e| e.to_string())?;
                self.authenticated();
            }
            Message::Snapshot(snapshot) => {
                if self.session.is_none() {
                    return Err("snapshot before login".into());
                }
                if self.table.is_none() || completed_table {
                    return Err("REFUSED: snapshot has no current official table".into());
                }
                if self.needs_snapshot {
                    self.safe_table()?;
                    if self.update.reducer.running
                        && self.update.reducer.game_id != Some(snapshot.game_id)
                    {
                        return Err("REFUSED: reconnect snapshot belongs to another game".into());
                    }
                }
                let reducer = &mut self.update.reducer;
                let players: Vec<_> = snapshot.state.players.iter().map(|p| p.player_id).collect();
                if let Some(human) = &self.human {
                    human.validate_players(&players, reducer.player_id)?;
                } else if players
                    .iter()
                    .any(|p| *p != reducer.player_id && !self.bots.contains(p))
                {
                    return Err(
                        "REFUSED: snapshot contains an opponent not verified as a server bot"
                            .into(),
                    );
                }
                // Invalidate any prior timer samples on every new or replacement snapshot.
                self.update.timers.clear();
                self.update.timer_settings = Some(snapshot.table_options.timer_settings.clone());
                let recorded = snapshot.clone();
                reducer.replace(snapshot).map_err(|e| e.to_string())?;
                self.recorder.snapshot(reducer, recorded);
                self.needs_snapshot = false;
                self.request_game = false;
                self.resuming = false;
                self.update.searching = false;
                self.pairing = None;
            }
            Message::Event(event) => {
                reducer.apply(event.clone()).map_err(|e| e.to_string())?;
                self.recorder.input(Input::Event { event });
            }
            Message::Timers(timers) => {
                let received_ms = crate::hub::now_ms();
                let received_at = Instant::now();
                for timer in timers {
                    self.update
                        .timers
                        .retain(|s| s.update.player != timer.player);
                    self.update.timers.push(TimerSample {
                        update: timer,
                        received_ms,
                        received_at,
                    });
                }
            }
            Message::Question(question) => {
                if self.needs_snapshot {
                    return Err("question arrived before reconnect snapshot".into());
                }
                reducer.ask(question.clone()).map_err(|e| e.to_string())?;
                self.recorder.input(Input::Question {
                    question: Box::new(question),
                });
            }
            Message::Log(change) => {
                reducer.log(&change).map_err(|e| e.to_string())?;
                self.recorder.input(Input::Log { change });
            }
            // Details of a table other than ours are not ours: the official client ignores them too.
            Message::TableDetails(table)
                if self.table.as_ref().map(|t| t.table_id) != Some(table.table_id) => {}
            Message::TableJoined { table, .. } | Message::TableDetails(table) => {
                if self.needs_snapshot
                    && !self
                        .table
                        .as_ref()
                        .is_some_and(|t| t.table_id == table.table_id)
                {
                    return Err("REFUSED: reconnect joined a different official table".into());
                }
                for player in table.players.iter().chain(&table.bots) {
                    reducer.names.insert(player.id, player.name.clone());
                }
                if let Some(human) = &self.human
                    && (reducer.running || self.needs_snapshot || self.update.searching)
                {
                    human.validate(&table, reducer.player_id)?;
                }
                self.bots = table.bots.iter().map(|p| p.id).collect();
                self.table = Some(table);
            }
            Message::BotAdded { table_id, bot } => {
                if let Some(table) = &mut self.table
                    && table.table_id == table_id
                {
                    self.bots.push(bot.id);
                    reducer.names.insert(bot.id, bot.name.clone());
                    table.bots.push(bot);
                }
            }
            Message::PlayerJoined {
                table_id,
                player,
                is_player,
            } => {
                if let Some(table) = &mut self.table
                    && table.table_id == table_id
                {
                    if let Some(human) = &self.human
                        && is_player
                        && !human.table.players.iter().any(|p| p.id == player.id)
                    {
                        return Err("REFUSED: unexpected player joined matched table".into());
                    }
                    reducer.names.insert(player.id, player.name.clone());
                    table.reconnecting.retain(|p| p.id != player.id);
                    if is_player {
                        if !table.players.iter().any(|p| p.id == player.id) {
                            table.players.push(player);
                            table.ready.push(false);
                        }
                    } else if !table.spectators.iter().any(|p| p.id == player.id) {
                        table.spectators.push(player);
                    }
                }
            }
            Message::RoleChanged {
                player_id,
                is_player,
                ..
            } => {
                if !awaiting_match
                    && ((player_id == reducer.player_id && !is_player)
                        || (self.human.is_none()
                            && player_id != reducer.player_id
                            && self.pairing.as_ref().is_none_or(|p| p.partner != player_id)))
                {
                    return Err("REFUSED: official table role changed".into());
                }
            }
            Message::PlayerLeft {
                table_id,
                player_id,
            } => {
                if let Some(table) = &mut self.table
                    && table.table_id == table_id
                {
                    if let Some(index) = table.players.iter().position(|p| p.id == player_id) {
                        table.players.remove(index);
                        if index < table.ready.len() {
                            table.ready.remove(index);
                        }
                    }
                    table.spectators.retain(|p| p.id != player_id);
                    table.reconnecting.retain(|p| p.id != player_id);
                    table.bots.retain(|p| p.id != player_id);
                    self.bots.retain(|id| *id != player_id);
                }
                if player_id == reducer.player_id
                    && self.table.as_ref().is_some_and(|t| t.table_id == table_id)
                {
                    self.table = None;
                    self.bots.clear();
                    self.completed_table = None;
                }
            }
            Message::PlayerReplaced { .. } => {
                if !awaiting_match {
                    return Err(
                        "REFUSED: official table player replacement needs operator inspection"
                            .into(),
                    );
                }
            }
            Message::RuleChanged { table_id, rule } => {
                if let Some(table) = &mut self.table
                    && table.table_id == table_id
                {
                    table.rules.retain(|(r, _)| r.id != rule.id);
                    table.rules.push((rule, false));
                }
            }
            Message::HostChanged { table_id, host } => {
                if let Some(table) = &mut self.table
                    && table.table_id == table_id
                {
                    table.host_id = host;
                }
            }
            Message::GameFinished { table, result, .. }
                if !reducer.running && reducer.game_id != Some(result.game_id) =>
            {
                // A game that ended while nothing here followed it (sent on logging in):
                // leave its table rather than refuse the connection.
                if self
                    .table
                    .as_ref()
                    .is_some_and(|t| t.table_id != table.table_id)
                    || !table.players.iter().any(|p| p.id == reducer.player_id)
                    || table.table_id != result.table_id
                {
                    return Ok(false);
                }
                self.outbox.push(
                    dominion_online::transport::outgoing::Command::LeaveTable {
                        table: table.table_id,
                        player: reducer.player_id,
                    }
                    .encode()
                    .map_err(|e| e.to_string())?,
                );
            }
            Message::GameFinished { table, result, .. } => {
                if table.table_id != result.table_id
                    || self
                        .table
                        .as_ref()
                        .is_some_and(|t| t.table_id != table.table_id)
                {
                    return Err("REFUSED: game result belongs to another official table".into());
                }
                reducer.finish(&result).map_err(|e| e.to_string())?;
                self.recorder.finish(result);
                self.store.remove("own-table").map_err(|e| e.to_string())?;
                self.store.remove("own-match").map_err(|e| e.to_string())?;
                self.resuming = false;
                self.needs_snapshot = false;
                self.request_game = false;
                self.completed_table = Some(table.table_id);
                if self.table.is_some() {
                    self.table = Some(table);
                }
                self.update.finished_at = Some(crate::hub::now_ms());
            }
            Message::Chat {
                receiver,
                sender,
                text,
            } => {
                if let Some(table) = &self.table
                    && receiver == format!("t{}", table.table_id)
                {
                    self.update.chat.said(table, &sender, text, reducer);
                }
            }
            Message::AnimationCompensation => return Ok(true),
            Message::ReconnectInstruction(switch_server) => {
                let from = self.endpoint;
                let to = if switch_server { from.other() } else { from };
                self.reconnect_to = Some(to);
                return Err(format!(
                    "official server requested reconnect ({from:?} to {to:?}); pending answers invalidated; call online_reconnect"
                ));
            }
            Message::TimeoutInstruction => {
                self.reconnect_to = None;
                return Err(format!(
                    "official server {:?} timed out the connection; call online_reconnect",
                    self.endpoint
                ));
            }
            Message::Metagame {
                kind: TIMEOUT_OFFER,
                player,
                ..
            } => {
                if reducer.running && reducer.seat().is_some_and(|s| i32::from(s) != player) {
                    self.outbox.push(
                        dominion_online::transport::outgoing::Command::TimeoutRequest {
                            decision: -1,
                            player,
                        }
                        .encode()
                        .map_err(|e| e.to_string())?,
                    );
                }
            }
            Message::Metagame {
                kind,
                player,
                decision,
            } if self.human.is_some() => {
                if kind == UNDO_REQUEST {
                    self.outbox.push(
                        dominion_online::transport::outgoing::Command::UndoRequestDenied {
                            player,
                            decision,
                        }
                        .encode()
                        .map_err(|e| e.to_string())?,
                    );
                }
            }
            Message::Metagame { .. } => {
                reducer.invalidate();
                return Err("official metagame request requires operator inspection".into());
            }
            Message::TableTerminated(id) => {
                if self.table.as_ref().is_some_and(|t| t.table_id == id) {
                    reducer.invalidate();
                    self.table = None;
                }
            }
            Message::Ancillary(
                dominion_online::transport::ancillary::Message::AutomatchFound {
                    table,
                    is_bot_game,
                },
            ) => {
                let options = self
                    .search
                    .take()
                    .ok_or("REFUSED: unsolicited automatch result")?;
                if is_bot_game || self.update.mode == OnlineMode::Bots {
                    return Err("REFUSED: unexpected bot match".into());
                }
                let human = HumanTable {
                    mode: self.update.mode,
                    owner: reducer.player_id,
                    table: table.clone(),
                    paired: false,
                };
                if table.players.len() != usize::from(options.players) {
                    return Err("REFUSED: matched player count differs".into());
                }
                human.validate(&table, reducer.player_id)?;
                for p in &table.players {
                    reducer.names.insert(p.id, p.name.clone());
                }
                self.store
                    .save("own-match", &human)
                    .map_err(|e| e.to_string())?;
                self.table = Some(table);
                self.bots.clear();
                self.human = Some(human);
                self.needs_snapshot = true;
                self.update.searching = true;
                for message in std::mem::take(&mut self.pending_match) {
                    self.process(message)?;
                }
            }
            Message::Ancillary(dominion_online::transport::ancillary::Message::PlayerProfile(
                profile,
            )) => {
                let key = (profile.player.id, profile.rating_type);
                self.profiles.answer(Some(key), Ok(profile));
            }
            Message::Ancillary(dominion_online::transport::ancillary::Message::PlayerResults {
                rating_type,
                results,
            }) => {
                self.results.answer(Some(rating_type), Ok(results));
            }
            Message::Succeeded(6) if self.search.is_some() => {
                self.queue_status = Some(false);
                self.update.searching = false;
            }
            Message::AutomatchStatus(status) => {
                if self.search.is_some() {
                    if !status.searching {
                        eprintln!("dominion: official automatch status not searching: {status:?}");
                    }
                    self.queue_status = Some(status.searching);
                    self.update.searching = status.searching;
                    self.update.queue_feedback =
                        status.feedback.into_iter().map(|f| f.name).collect();
                }
            }
            Message::Ancillary(_)
            | Message::LobbyNotice
            | Message::Tables(_)
            | Message::PlayerStatus { .. }
            | Message::GameStarted { .. }
            | Message::TableRemoved
            | Message::Succeeded(_)
            | Message::Preferences(_)
            | Message::Relationships(_)
            | Message::PlayerDisconnected { .. }
            | Message::PlayerReconnected { .. }
            | Message::Ticker(_)
            | Message::Pong
            | Message::ServerState(_)
            | Message::Explanations { .. }
            | Message::FormResult { .. } => {}
        }
        Ok(false)
    }
    async fn request_profile(
        &mut self,
        player: Option<i32>,
        kind: i32,
        reply: Reply<PlayerProfile>,
    ) {
        let player = player.unwrap_or(self.update.reducer.player_id);
        if let Err(e) = self.connected() {
            let _ = reply.send(Err(e));
        } else if self.profiles.wait((player, kind), reply) {
            let command = Command::RequestPlayerProfile {
                player,
                rating: kind,
                period: PROFILE_PERIOD,
                offset: PROFILE_HISTORY,
            };
            if let Err(e) = self.send_command(command).await {
                self.profiles.answer(Some((player, kind)), Err(e));
            }
        }
    }
    async fn request_results(&mut self, kind: i32, reply: Reply<Vec<ProfileResult>>) {
        if let Err(e) = self.connected() {
            let _ = reply.send(Err(e));
        } else if self.results.wait(kind, reply) {
            let command = Command::RequestPlayerResults {
                player: self.update.reducer.player_id,
                rating: kind,
                count: RESULTS,
            };
            if let Err(e) = self.send_command(command).await {
                self.results.answer(Some(kind), Err(e));
            }
        }
    }
    fn connected(&self) -> Result<(), String> {
        if self.socket.is_none() || self.session.is_none() {
            return Err("official connection closed; reconnect first".into());
        }
        Ok(())
    }
    async fn say(&mut self, text: String) -> Result<(), String> {
        let table = (self.table.as_ref())
            .ok_or("REJECTED: the account is at no table on dominion.games")?
            .table_id;
        self.send_command(Command::SendChat {
            receiver: format!("t{table}"),
            sender: format!("p{}", self.update.reducer.player_id),
            text,
        })
        .await
    }
    async fn send_command(&mut self, command: Command) -> Result<(), String> {
        let bytes = command.encode().map_err(|e| e.to_string())?;
        self.send(bytes).await
    }
    /// Answers every player-page request waiting.
    fn answer_player_page(&mut self, error: String) {
        self.profiles.answer(None, Err(error.clone()));
        self.results.answer(None, Err(error));
    }
    async fn answer(&mut self, answer: Answer, expect: Option<Vec<String>>) -> Result<(), String> {
        self.update.view()?;
        self.safe_table()?;
        if self.needs_snapshot {
            return Err("REJECTED: waiting for a reconnect snapshot".into());
        }
        let (index, values) = self
            .update
            .reducer
            .answer(&answer, expect.as_deref())
            .map_err(|e| e.to_string())?;
        self.send(command::answer(index, &values)).await
    }
    async fn answer_forced(&mut self) -> Result<(), String> {
        let Some(view) = self.update.view()? else {
            return Ok(());
        };
        let Some(decision) = view.decision else {
            return Ok(());
        };
        if decision.groups.len() > 1 || decision.name_any {
            return Ok(());
        }
        let indices = if decision.min == Some(1)
            && decision.max == Some(1)
            && decision.options.len() == 1
        {
            Some(vec![0])
        } else if decision.min == Some(0) && decision.max == Some(0) && decision.options.is_empty()
        {
            Some(vec![])
        } else {
            None
        };
        if let Some(indices) = indices {
            self.answer(Answer::indices(decision.id, indices), None)
                .await?;
        }
        Ok(())
    }
    fn safe_table(&self) -> Result<&TableDetails, String> {
        if self.awaiting_match() {
            return Err("REFUSED: waiting for match confirmation".into());
        }
        let table = self
            .table
            .as_ref()
            .ok_or("official server has not returned a table")?;
        let id = self.update.reducer.player_id;
        if let Some(human) = &self.human {
            human.validate(table, id)?;
            return Ok(table);
        }
        if table.host_id != id
            || table.players.len() != 1
            || table.players[0].id != id
            || !table.spectators.is_empty()
            || !table.reconnecting.is_empty()
        {
            return Err(
                "REFUSED: official table is not exclusively this account and server bots".into(),
            );
        }
        if table.rules.iter().filter(|(r, _)| r.id == 5).count() != 1
            || table.rules.iter().filter(|(r, _)| r.id == 19).count() != 1
        {
            return Err("REFUSED: missing or conflicting privacy/rating rules".into());
        }
        if !table
            .rules
            .iter()
            .any(|(r, _)| r.id == 5 && r.value == RuleValue::Group(Group::Nobody))
            || !table
                .rules
                .iter()
                .any(|(r, _)| r.id == 19 && r.value == RuleValue::Boolean(false))
        {
            return Err(
                "REFUSED: official server did not confirm spectator prohibition and unrated play"
                    .into(),
            );
        }
        Ok(table)
    }
    async fn resign(&mut self) -> Result<(), String> {
        self.update.view()?;
        self.safe_table()?;
        if !self.update.reducer.running {
            return Err("NO_GAME: no running official game".into());
        }
        self.update.reducer.invalidate();
        self.send(command::resign()).await
    }
    async fn new_game(&mut self, options: NewGame) -> Result<(), String> {
        self.update.view()?;
        if self.update.reducer.running {
            return Err(
                "REFUSED: an official game is already running; new_game never resigns it".into(),
            );
        }
        if self.update.searching {
            return Err("REFUSED: cancel matchmaking before starting a bot game".into());
        }
        let (bots, rules) = options.rules(&self.update.reducer.catalog)?;
        let previous_game = self.update.reducer.game_id;
        self.leave_finished().await?;
        self.human = None;
        self.completed_table = None;
        self.update.mode = OnlineMode::Bots;
        self.send(command::private_table(&rules).map_err(|e| e.to_string())?)
            .await?;
        while self.table.is_none() {
            self.receive().await?;
        }
        for expected in 1..=bots {
            let id = self.safe_table()?.table_id;
            self.send(command::add_bot(id)).await?;
            while self.safe_table()?.bots.len() < usize::from(expected) {
                self.receive().await?;
            }
        }
        let table = self.safe_table()?;
        if table.bots.len() != usize::from(bots) {
            return Err("REFUSED: server bot count differs from requested settings".into());
        }
        for requested in &rules {
            if !table.rules.iter().any(|(rule, _)| rule == requested) {
                return Err(format!(
                    "REFUSED: server has not confirmed requested rule {}",
                    requested.id
                ));
            }
        }
        self.store
            .save("own-table", table)
            .map_err(|e| e.to_string())?;
        self.send(command::ready(table.table_id, true)).await?;
        while self.update.reducer.game_id == previous_game {
            self.receive().await?;
        }
        Ok(())
    }
    fn holds_finished_table(&self) -> bool {
        !self.update.reducer.running
            && self.update.reducer.result.is_some()
            && self
                .table
                .as_ref()
                .is_some_and(|t| Some(t.table_id) == self.completed_table)
    }

    fn authenticated(&mut self) {
        self.authentication = None;
        self.update.connected = true;
        self.update.error = None;
    }

    async fn leave_finished(&mut self) -> Result<(), String> {
        if self.table.is_some() {
            if !self.holds_finished_table() {
                return Err("REFUSED: an unfinished official table already exists".into());
            }
            let table_id = self.table.as_ref().expect("table checked").table_id;
            let bytes = dominion_online::transport::outgoing::Command::LeaveTable {
                table: table_id,
                player: self.update.reducer.player_id,
            }
            .encode()
            .map_err(|e| e.to_string())?;
            self.leaving_finished = true;
            self.send(bytes).await?;
            while self.table.is_some() {
                self.receive().await?;
            }
            self.leaving_finished = false;
        }
        Ok(())
    }
    async fn reconnect(
        &mut self,
        probe: impl Future<Output = Result<Endpoint, String>>,
        connect: impl AsyncFn(Endpoint) -> dominion_online::Result<Transport>,
    ) -> Result<(), String> {
        let result =
            tokio::time::timeout(RECONNECT_TIMEOUT, self.restore_connection(probe, connect))
                .await
                .unwrap_or_else(|_| {
                    Err("official reconnect timed out awaiting confirmation".into())
                });
        match &result {
            Ok(()) => self.reconnect_to = None,
            Err(error) => self.fail(error.clone()).await,
        }
        result
    }

    async fn restore_connection(
        &mut self,
        probe: impl Future<Output = Result<Endpoint, String>>,
        connect: impl AsyncFn(Endpoint) -> dominion_online::Result<Transport>,
    ) -> Result<(), String> {
        self.update.timers.clear();
        self.needs_snapshot |= self.update.reducer.running;
        let idle = !self.needs_snapshot && (self.table.is_none() || self.holds_finished_table());
        let destination = match self.reconnect_to {
            Some(to) if !self.auto_endpoint && to != self.endpoint => {
                return Err(format!(
                    "official server requested {to:?}, but the endpoint is pinned to {:?}",
                    self.endpoint
                ));
            }
            Some(to) => to,
            None if idle && self.auto_endpoint => probe.await?,
            None => self.endpoint,
        };
        if destination != self.endpoint && self.holds_finished_table() {
            // The finished table ID belongs to the server we left.
            self.table = None;
            self.human = None;
            self.bots.clear();
            self.completed_table = None;
        }
        self.endpoint = destination;
        if self.reconnect_to.is_some() || !idle || self.credentials.is_none() {
            let session = self.session.as_ref().ok_or("no owned official session")?;
            let bytes = command::reconnect_session(session.player, &session.token, VERSION);
            self.confirm_connection(&connect, Authentication::Session, bytes)
                .await?;
            if self.authentication != Some(Authentication::SessionExpired) {
                return Ok(());
            }
            self.close().await;
        }

        let credentials = self
            .credentials
            .as_ref()
            .ok_or("no official password available")?;
        let bytes = command::login(&credentials.username, &credentials.password, VERSION, 0);
        self.confirm_connection(&connect, Authentication::Password, bytes)
            .await
    }

    async fn confirm_connection(
        &mut self,
        connect: &impl AsyncFn(Endpoint) -> dominion_online::Result<Transport>,
        auth: Authentication,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        self.authentication = Some(auth);
        self.socket = Some(connect(self.endpoint).await.map_err(|e| e.to_string())?);
        self.send(bytes).await?;
        while self.authentication.is_some() || self.needs_snapshot {
            self.receive_inner().await?;
            if self.authentication == Some(Authentication::SessionExpired) {
                break;
            }
        }
        Ok(())
    }
}

pub(crate) fn glossary(reducer: &Reducer, names: &[String]) -> Result<Vec<GlossaryEntry>, String> {
    names
        .iter()
        .map(|name| {
            let id = reducer
                .catalog
                .ordinal(name)
                .ok_or_else(|| format!("unknown official card {name:?}"))?;
            let info = reducer.catalog.card(id).map_err(|e| e.to_string())?;
            let mut entry = dominion_agent::glossary(&info.name)
                .ok_or_else(|| format!("official text unavailable locally for {}", info.name))?;
            entry.cost = Some(if reducer.game_id.is_some() {
                reducer.cost(None, id).map_err(|e| e.to_string())?
            } else {
                info.cost.clone()
            });
            entry.types = info.types.clone();
            Ok(entry)
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests;
