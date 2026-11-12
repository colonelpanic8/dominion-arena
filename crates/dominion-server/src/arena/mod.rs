//! Who is playing: accounts and their sessions, the lobby (tables, the automatch queue,
//! chat, who is connected), finished games' records and ratings. Games themselves are
//! the [`Hub`]'s; a lobby game's seats are still played by seat token, which their
//! accounts are handed.
//!
//! Accounts, sessions and records are the database's ([`db`]); `state` holds only what
//! is live: tables, the queue, who is connected, chat.
//!
//! Lock order: `state`, then the database, then anything of the hub's. The hub calls
//! back (a game ended, naming accounts) with no lock of its own held; the first of those
//! callbacks locks `state`, so no hub method that can end a game is called with `state`
//! locked.

mod accounts;
mod chat;
mod db;
mod matching;
mod records;
mod tables;

use crate::hub::{Hub, HubError, Timing, lock, now_ms};
use crate::limits::Limiter;
use crate::rating::Rating;
use db::Db;
use dominion_protocol::lobby::{
    ChatMessage, Lobby, Me, RatingInfo, Searching, Stats, UserKind, UserRef,
};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub use accounts::SESSION_COOKIE;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    BadRequest(String),
    /// Not signed in, or with the wrong name or password.
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    /// Too many of something lately; try again after the duration.
    #[error("{0}")]
    TooMany(String, Duration),
    /// The database failed.
    #[error("the server could not read or keep its records ({0})")]
    Storage(String),
    #[error(transparent)]
    Hub(#[from] HubError),
}

pub type Result<T> = std::result::Result<T, Error>;

fn bad<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::BadRequest(message.into()))
}

fn conflict<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Conflict(message.into()))
}

/// What the server's settings say about accounts and the lobby.
#[derive(Clone, Debug)]
pub struct Settings {
    /// New accounts can be registered.
    pub registration: bool,
    /// People can play without an account.
    pub guests: bool,
    /// The Banned cards list of anyone who hasn't set their own.
    pub banned_cards: Vec<String>,
    pub timing: Timing,
    pub max_games: usize,
    /// How long a finished game is kept.
    pub keep_finished: Duration,
    /// How long an unfinished game nobody touches is kept.
    pub keep_idle: Duration,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            registration: true,
            guests: true,
            banned_cards: vec![],
            timing: Timing::default(),
            max_games: crate::hub::DEFAULT_MAX_GAMES,
            keep_finished: Duration::from_secs(7 * 86_400),
            keep_idle: Duration::from_secs(30 * 86_400),
        }
    }
}

/// Who a request comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    /// The account's id.
    pub user: String,
    /// Connected as an agent: a bot, or anyone over MCP.
    pub agent: bool,
}

#[derive(Clone, Debug)]
enum Login {
    /// An argon2 hash in PHC form.
    Password(String),
    Guest,
    /// The owner's account id, and the SHA-256 of the API key.
    Bot {
        owner: String,
        key: String,
    },
}

/// An account, as the database has it.
#[derive(Clone, Debug)]
struct User {
    /// `u<row>`.
    id: String,
    name: String,
    login: Login,
    rating: Rating,
    rated_games: u32,
    games: u32,
    wins: u32,
    /// Their own Banned cards list, once they have set one.
    banned_cards: Option<Vec<String>>,
}

impl User {
    fn kind(&self) -> UserKind {
        match self.login {
            Login::Password(_) => UserKind::Registered,
            Login::Guest => UserKind::Guest,
            Login::Bot { .. } => UserKind::Bot,
        }
    }

    fn owner(&self) -> Option<&str> {
        match &self.login {
            Login::Bot { owner, .. } => Some(owner),
            _ => None,
        }
    }

    fn rating_info(&self) -> RatingInfo {
        RatingInfo {
            rating: self.rating.rating,
            deviation: self.rating.deviation_at(now_ms() / 1000),
            games: self.rated_games,
        }
    }

    fn stats(&self) -> Stats {
        Stats {
            games: self.games,
            wins: self.wins,
        }
    }
}

/// Who is connected: lobby feeds and waiting agents count as streams.
struct Presence {
    streams: u32,
    seen: Instant,
}

struct Chat {
    next: u64,
    messages: VecDeque<ChatMessage>,
}

#[derive(Default)]
struct State {
    tables: BTreeMap<String, tables::Table>,
    next_table: u64,
    /// The table each account sits at.
    seated: HashMap<String, String>,
    queue: Vec<matching::Entry>,
    /// Accounts an automatch found a game for, until they have seen it.
    matched: HashMap<String, matching::Matched>,
    presence: HashMap<String, Presence>,
    chats: HashMap<String, Chat>,
}

impl State {
    fn here(&self, user: &str) -> bool {
        self.presence
            .get(user)
            .is_some_and(|p| p.streams > 0 || p.seen.elapsed() < HERE)
    }

    fn touch(&mut self, user: &str) {
        let presence = self.presence.entry(user.to_string()).or_insert(Presence {
            streams: 0,
            seen: Instant::now(),
        });
        presence.seen = Instant::now();
    }
}

/// A player counts as connected this long after their last call.
const HERE: Duration = Duration::from_secs(45);

struct Limits {
    register: Limiter<IpAddr>,
    guest: Limiter<IpAddr>,
    login: Limiter<IpAddr>,
    login_name: Limiter<String>,
    create: Limiter<IpAddr>,
    deal: Limiter<IpAddr>,
    chat: Limiter<String>,
    lobby: Limiter<String>,
}

impl Default for Limits {
    fn default() -> Limits {
        let minutes = |m: u64| Duration::from_secs(60 * m);
        Limits {
            register: Limiter::new(5, minutes(60)),
            guest: Limiter::new(20, minutes(60)),
            login: Limiter::new(20, minutes(5)),
            login_name: Limiter::new(10, minutes(5)),
            create: Limiter::new(30, minutes(10)),
            deal: Limiter::new(60, minutes(1)),
            chat: Limiter::new(8, Duration::from_secs(10)),
            lobby: Limiter::new(60, minutes(1)),
        }
    }
}

fn limited<K: std::hash::Hash + Eq>(limiter: &Limiter<K>, key: K, what: &str) -> Result<()> {
    limiter.take(key).map_err(|wait| {
        let secs = wait.as_secs().max(1);
        Error::TooMany(format!("too many {what}; try again in {secs}s"), wait)
    })
}

pub struct Arena {
    pub hub: Arc<Hub>,
    db: Db,
    settings: Settings,
    state: Mutex<State>,
    /// Bumped whenever what `Arena::lobby` returns may have changed.
    lobby: watch::Sender<u64>,
    /// Bumped whenever something is said in the lobby or at a table.
    chat: watch::Sender<u64>,
    limits: Limits,
    /// When the slow part of [`Arena::sweep`] last ran.
    swept: Mutex<Option<Instant>>,
}

/// Counts an account as connected until dropped.
pub struct Attendance {
    arena: Weak<Arena>,
    user: String,
}

impl Drop for Attendance {
    fn drop(&mut self) {
        if let Some(arena) = self.arena.upgrade() {
            let mut state = lock(&arena.state);
            if let Some(p) = state.presence.get_mut(&self.user) {
                p.streams = p.streams.saturating_sub(1);
                p.seen = Instant::now();
            }
            drop(state);
            arena.bump();
        }
    }
}

impl Arena {
    /// The accounts and records kept with `hub`'s games (in memory if it keeps none),
    /// with the hub told about them.
    pub fn open(hub: Arc<Hub>, settings: Settings) -> std::io::Result<Arc<Arena>> {
        let store = hub.store();
        let failed = |e: Error| std::io::Error::other(e.to_string());
        let db = Db::open(store.as_deref().map(|s| s.dir())).map_err(failed)?;
        // A finished game's id stays its record's, after the game itself is deleted.
        hub.reserve_ids(db.last_game_number().map_err(failed)?);
        hub.set_timing(settings.timing);
        hub.set_max_games(settings.max_games);
        let arena = Arc::new(Arena {
            hub: hub.clone(),
            db,
            settings,
            state: Mutex::default(),
            lobby: watch::channel(0).0,
            chat: watch::channel(0).0,
            limits: Limits::default(),
            swept: Mutex::new(None),
        });
        let (on_end, users) = (Arc::downgrade(&arena), Arc::downgrade(&arena));
        hub.set_hooks(
            move |ended| {
                if let Some(arena) = on_end.upgrade() {
                    arena.record(ended);
                }
            },
            move |owners| match users.upgrade() {
                Some(arena) => arena.user_refs(owners),
                None => vec![None; owners.len()],
            },
        );
        for ended in hub.unrecorded() {
            arena.record(ended);
        }
        Ok(arena)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn bump(&self) {
        self.lobby.send_modify(|v| *v += 1);
    }

    /// The lobby's version and its chat's counter, for the lobby feed.
    pub fn subscribe(&self) -> (watch::Receiver<u64>, watch::Receiver<u64>) {
        (self.lobby.subscribe(), self.chat.subscribe())
    }

    /// Account `id`, which whoever asks is signed in as.
    fn user(&self, id: &str) -> Result<User> {
        let gone = || Error::Unauthorized("that account no longer exists".into());
        self.db.user(id)?.ok_or_else(gone)
    }

    fn user_ref(&self, user: &User) -> UserRef {
        self.db.user_ref(&user.id).unwrap_or_else(|| UserRef {
            name: user.name.clone(),
            kind: user.kind(),
            rating: None,
            owner: None,
        })
    }

    fn me_of(&self, user: &User) -> Result<Me> {
        let bots = self.db.bots_of(&user.id)?;
        Ok(Me {
            user: self.user_ref(user),
            rating: user.rating_info(),
            stats: user.stats(),
            bots: bots.iter().map(|b| self.user_ref(b)).collect(),
        })
    }

    fn user_refs(&self, owners: &[Option<String>]) -> Vec<Option<UserRef>> {
        let who = |owner: &Option<String>| self.db.user_ref(owner.as_deref()?);
        owners.iter().map(who).collect()
    }

    /// Counts `ident` as connected until the result is dropped.
    pub fn attend(self: &Arc<Self>, ident: &Ident) -> Attendance {
        {
            let mut state = lock(&self.state);
            state.touch(&ident.user);
            if let Some(p) = state.presence.get_mut(&ident.user) {
                p.streams += 1;
            }
        }
        self.bump();
        Attendance {
            arena: Arc::downgrade(self),
            user: ident.user.clone(),
        }
    }

    /// Notes that `ident` is around.
    pub fn touch(&self, ident: &Ident) {
        lock(&self.state).touch(&ident.user);
    }

    /// The lobby as `ident` sees it.
    pub fn lobby(&self, ident: Option<&Ident>) -> Lobby {
        let games = match ident {
            Some(ident) => self.hub.seats_of(&ident.user, Duration::from_secs(3600)),
            None => vec![],
        };
        let user = ident.and_then(|i| self.db.user(&i.user).ok().flatten());
        let you = user.as_ref().and_then(|u| self.me_of(u).ok());
        let version = *self.lobby.borrow();
        let mut state = lock(&self.state);
        if let Some(user) = &user {
            state.touch(&user.id);
        }
        let state = &*state;
        let agents = state.queue.iter().filter(|e| e.agent).count() as u32;
        let mine = user.as_ref().map(|u| u.id.as_str());
        Lobby {
            version,
            you,
            online: state.presence.keys().filter(|u| state.here(u)).count() as u32,
            searching: Searching {
                humans: state.queue.len() as u32 - agents,
                agents,
            },
            tables: (state.tables.values())
                .map(|t| t.info(state, &self.db))
                .collect(),
            table: mine.and_then(|id| {
                let table = state.tables.get(state.seated.get(id)?)?;
                Some(table.mine(id))
            }),
            queue: mine.and_then(|id| state.queue_status(id)),
            games,
        }
    }

    /// What happens on its own: forfeits, people who left, matches, things that expire.
    /// Called every few seconds.
    pub fn sweep(&self) {
        self.hub.notice_presence();
        for seat in self.hub.overdue() {
            let _ = self.hub.resign(&seat, "abandoned");
        }
        let changed = {
            let mut state = lock(&self.state);
            let left = state.drop_absent(&self.db);
            let seen = state.forget_matches();
            left || seen
        };
        if changed {
            self.bump();
        }
        self.run_matching();
        let mut swept = lock(&self.swept);
        if swept.is_some_and(|at| at.elapsed() < Duration::from_secs(600)) {
            return;
        }
        *swept = Some(Instant::now());
        drop(swept);
        self.expire_sessions();
        let expired = (self.hub).expire(self.settings.keep_finished, self.settings.keep_idle);
        if expired > 0 {
            let games: std::collections::HashSet<String> =
                (self.hub.list().into_iter().map(|g| g.game_id)).collect();
            lock(&self.state).chats.retain(|channel, _| {
                channel
                    .strip_prefix("game-")
                    .is_none_or(|id| games.contains(id))
            });
        }
        let l = &self.limits;
        l.register.sweep();
        l.guest.sweep();
        l.login.sweep();
        l.login_name.sweep();
        l.create.sweep();
        l.deal.sweep();
        l.chat.sweep();
        l.lobby.sweep();
    }

    /// Whether `ip` may create another game by hand now.
    pub fn may_create(&self, ip: IpAddr) -> Result<()> {
        limited(&self.limits.create, ip, "games created")
    }

    /// Whether `ip` may have another kingdom dealt now.
    pub fn may_deal(&self, ip: IpAddr) -> Result<()> {
        limited(&self.limits.deal, ip, "kingdoms dealt")
    }
}
