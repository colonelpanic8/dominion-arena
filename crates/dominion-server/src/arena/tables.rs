//! Lobby tables: a host sets the rules and the seats, players sit down and say they are
//! ready, and the game starts once every seat is taken and everyone is. Tables live in
//! memory only: a restart empties the lobby, and the games go on.

use super::db::Db;
use super::{Arena, Error, Ident, Result, State, User, bad, conflict, limited};
use crate::hub::{self, Meta, lock, seat_name};
use dominion_protocol::api::{DealRequest, SeatToken};
use dominion_protocol::lobby::{
    ChatKind, MyTable, NewTable, SeatEdit, SeatKind, Source, TableInfo, TableSeat, TableSpec,
    UserKind,
};
use rand::seq::SliceRandom;
use std::time::{Duration, Instant};

const MAX_TABLES: usize = 200;
/// A seated player who is gone this long, with no game of the table's running, has left.
const ABSENT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Seat {
    Open,
    Player {
        user: String,
        ready: bool,
    },
    Token(String),
    /// One of the server's agents, by its account's id.
    Agent(String),
}

pub(super) struct Table {
    id: String,
    /// The host's account id.
    host: String,
    spec: TableSpec,
    seats: Vec<Seat>,
    /// The game it started, while that runs.
    game: Option<String>,
    last_game: Option<String>,
    /// The token seats' tokens of the game it started last, for the host.
    tokens: Vec<SeatToken>,
    /// A game is being set up for it.
    starting: bool,
    /// When it last had a game running or a seat changed.
    idle_since: Instant,
}

impl Table {
    fn players(&self) -> impl Iterator<Item = &str> {
        self.seats.iter().filter_map(|s| match s {
            Seat::Player { user, .. } => Some(user.as_str()),
            _ => None,
        })
    }

    /// The server's agents seated, by account id.
    fn agents(&self) -> impl Iterator<Item = &str> {
        self.seats.iter().filter_map(|s| match s {
            Seat::Agent(user) => Some(user.as_str()),
            _ => None,
        })
    }

    fn unready(&mut self) {
        for seat in &mut self.seats {
            if let Seat::Player { ready, .. } = seat {
                *ready = false;
            }
        }
        self.idle_since = Instant::now();
    }

    pub(super) fn info(&self, state: &State, db: &Db) -> TableInfo {
        TableInfo {
            id: self.id.clone(),
            host: db.user_ref(&self.host).map(|u| u.name).unwrap_or_default(),
            spec: self.spec.clone(),
            seats: (self.seats.iter())
                .map(|seat| match seat {
                    Seat::Open => TableSeat::default(),
                    Seat::Player { user, ready } => TableSeat {
                        kind: SeatKind::Player,
                        user: db.user_ref(user),
                        name: None,
                        ready: *ready,
                        here: state.here(user),
                    },
                    Seat::Token(name) => TableSeat {
                        kind: SeatKind::Token,
                        user: None,
                        name: Some(name.clone()),
                        ready: true,
                        here: false,
                    },
                    Seat::Agent(user) => TableSeat {
                        kind: SeatKind::Agent,
                        user: db.user_ref(user),
                        name: None,
                        ready: true,
                        here: state.house.offers(user),
                    },
                })
                .collect(),
            game: self.game.clone(),
            last_game: self.last_game.clone(),
        }
    }

    pub(super) fn mine(&self, user: &str) -> MyTable {
        MyTable {
            id: self.id.clone(),
            tokens: if self.host == user {
                self.tokens.clone()
            } else {
                vec![]
            },
        }
    }
}

/// What a table's rules ask a deal for, for `players` seats.
fn deal_request(spec: &TableSpec, players: usize, banned: Vec<String>) -> DealRequest {
    DealRequest {
        players: players as u8,
        sets: spec.sets.clone(),
        kingdom: spec.kingdom.clone(),
        from_sets: spec.from_sets.clone(),
        landscapes: spec.landscapes.clone(),
        slots: spec.slots.clone(),
        banned,
        seed: None,
    }
}

/// Refuses rules no game can be set up with.
fn check_spec(spec: &TableSpec, players: usize) -> Result<()> {
    if spec.turn_limit == Some(0) {
        return bad("a turn limit is at least 1");
    }
    hub::deal(&deal_request(spec, players, vec![]))?;
    Ok(())
}

fn check_seats(seats: &[Seat]) -> Result<()> {
    if !(2..=6).contains(&seats.len()) {
        return bad("a table has 2 to 6 seats");
    }
    let names: Vec<String> = (seats.iter())
        .filter_map(|s| match s {
            Seat::Token(name) => Some(format!("token {}", name.to_lowercase())),
            Seat::Agent(user) => Some(format!("agent {user}")),
            _ => None,
        })
        .collect();
    if (1..names.len()).any(|i| names[..i].contains(&names[i])) {
        return bad("two seats have the same name");
    }
    Ok(())
}

/// A seat for the server's agent `name`, set by `host`: one the runner offers now, or
/// one of the table's seats already.
fn agent_seat(state: &State, db: &Db, host: &User, name: &str, old: &[Seat]) -> Result<Seat> {
    if host.kind() != UserKind::Registered {
        return Err(Error::Forbidden(
            "only registered players seat the server's agents".into(),
        ));
    }
    if let Some(agent) = state.house.agent_named(name) {
        return Ok(Seat::Agent(agent.user.clone()));
    }
    let key = name.trim().to_lowercase();
    let seated = old.iter().find(|s| match s {
        Seat::Agent(user) => db
            .user_ref(user)
            .is_some_and(|u| u.name.to_lowercase() == key),
        _ => false,
    });
    match seated {
        Some(seat) => Ok(seat.clone()),
        None => bad(format!("the server has no agent called {}", name.trim())),
    }
}

impl State {
    /// Takes `user` out of the table they sit at; the table goes with its last player,
    /// and the next one hosts when the host leaves.
    pub(super) fn leave_table(&mut self, db: &Db, user: &str) -> Option<String> {
        let id = self.seated.remove(user)?;
        let name = db.user_ref(user).map(|u| u.name);
        let table = self.tables.get_mut(&id)?;
        for seat in &mut table.seats {
            if matches!(seat, Seat::Player { user: u, .. } if u == user) {
                *seat = Seat::Open;
            }
        }
        table.unready();
        let next = table.players().next().map(str::to_string);
        match next {
            None => {
                self.tables.remove(&id);
                self.chats.remove(&format!("table-{id}"));
            }
            Some(next) => {
                if table.host == user {
                    table.host = next;
                    table.tokens.clear();
                }
                if let Some(name) = name {
                    self.say(
                        &format!("table-{id}"),
                        "",
                        ChatKind::System,
                        &format!("{name} left."),
                    );
                }
            }
        }
        Some(id)
    }

    /// Takes `user` out of the lobby: their table and their search.
    pub(super) fn leave_all(&mut self, db: &Db, user: &str) {
        self.leave_table(db, user);
        self.queue.retain(|e| e.user != user);
        self.matched.remove(user);
    }

    /// Removes the players who are gone from tables and the queue. Whether any were.
    pub(super) fn drop_absent(&mut self, db: &Db) -> bool {
        let gone: Vec<String> = (self.tables.values())
            .filter(|t| t.game.is_none() && !t.starting && t.idle_since.elapsed() >= ABSENT)
            .flat_map(|t| t.players())
            .filter(|u| {
                (self.presence.get(*u)).is_none_or(|p| p.streams == 0 && p.seen.elapsed() >= ABSENT)
            })
            .map(str::to_string)
            .collect();
        for user in &gone {
            self.leave_table(db, user);
        }
        let before = self.queue.len();
        let here: Vec<bool> = self.queue.iter().map(|e| self.here(&e.user)).collect();
        let mut here = here.into_iter();
        self.queue.retain(|_| here.next().unwrap_or(true));
        self.presence
            .retain(|_, p| p.streams > 0 || p.seen.elapsed() < Duration::from_secs(3600));
        !gone.is_empty() || self.queue.len() != before
    }

    fn table_of(&mut self, id: &str) -> Result<&mut Table> {
        self.tables
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("there is no table {id}")))
    }

    fn hosted(&mut self, id: &str, user: &str) -> Result<&mut Table> {
        let table = self.table_of(id)?;
        if table.host != user {
            return Err(Error::Forbidden("only the table's host does that".into()));
        }
        if table.game.is_some() || table.starting {
            return conflict("the table's game is running");
        }
        Ok(table)
    }
}

/// What a table's game is set up from, taken while the table is locked.
struct Start {
    table: String,
    host: String,
    spec: TableSpec,
    seats: Vec<Seat>,
    banned: Vec<String>,
    rated: bool,
}

impl Arena {
    fn table_info(&self, state: &State, id: &str) -> Result<TableInfo> {
        let table = state.tables.get(id);
        table
            .map(|t| t.info(state, &self.db))
            .ok_or_else(|| Error::NotFound(format!("there is no table {id}")))
    }

    pub fn table(&self, id: &str) -> Result<TableInfo> {
        self.table_info(&lock(&self.state), id)
    }

    /// A new table hosted by `ident`, who leaves the one they sat at.
    pub fn new_table(&self, ident: &Ident, new: &NewTable) -> Result<TableInfo> {
        limited(&self.limits.lobby, ident.user.clone(), "changes to tables")?;
        let host = self.user(&ident.user)?;
        let edits = new.seats.as_deref().unwrap_or(&[SeatEdit::Open]);
        let mut state = lock(&self.state);
        let mut seats = vec![Seat::Player {
            user: ident.user.clone(),
            ready: false,
        }];
        for seat in edits {
            seats.push(match seat {
                SeatEdit::Open => Seat::Open,
                SeatEdit::Token(name) => Seat::Token(
                    seat_name(name).map_err(|e| Error::BadRequest(format!("a seat's name {e}")))?,
                ),
                SeatEdit::Agent(name) => agent_seat(&state, &self.db, &host, name, &[])?,
                SeatEdit::Player(_) => return bad("a new table seats only its host"),
            });
        }
        check_seats(&seats)?;
        check_spec(&new.spec, seats.len())?;
        if state.tables.len() >= MAX_TABLES {
            return conflict("the lobby is full of tables; join one");
        }
        state.leave_all(&self.db, &ident.user);
        state.next_table += 1;
        let id = format!("t{}", state.next_table);
        let table = Table {
            id: id.clone(),
            host: ident.user.clone(),
            spec: new.spec.clone(),
            seats,
            game: None,
            last_game: None,
            tokens: vec![],
            starting: false,
            idle_since: Instant::now(),
        };
        state.tables.insert(id.clone(), table);
        state.seated.insert(ident.user.clone(), id.clone());
        state.touch(&ident.user);
        let info = self.table_info(&state, &id);
        drop(state);
        self.bump();
        info
    }

    /// Replaces a table's rules, which nobody is ready for yet.
    pub fn set_spec(&self, ident: &Ident, id: &str, spec: &TableSpec) -> Result<TableInfo> {
        limited(&self.limits.lobby, ident.user.clone(), "changes to tables")?;
        let seats = lock(&self.state).hosted(id, &ident.user)?.seats.len();
        check_spec(spec, seats)?;
        let mut state = lock(&self.state);
        let table = state.hosted(id, &ident.user)?;
        if table.spec != *spec {
            table.spec = spec.clone();
            table.unready();
        }
        let info = self.table_info(&state, id);
        drop(state);
        self.bump();
        info
    }

    /// Replaces a table's seats: the host's and any other seated player's to keep, by
    /// name, open ones and token ones. A seated player left out leaves the table.
    pub fn set_seats(&self, ident: &Ident, id: &str, edits: &[SeatEdit]) -> Result<TableInfo> {
        limited(&self.limits.lobby, ident.user.clone(), "changes to tables")?;
        let spec = lock(&self.state).hosted(id, &ident.user)?.spec.clone();
        check_spec(&spec, edits.len().clamp(2, 6))?;
        let host = self.user(&ident.user)?;
        let mut state = lock(&self.state);
        let old = state.hosted(id, &ident.user)?.seats.clone();
        let mut seats = vec![];
        for edit in edits {
            seats.push(match edit {
                SeatEdit::Open => Seat::Open,
                SeatEdit::Token(name) => Seat::Token(
                    seat_name(name).map_err(|e| Error::BadRequest(format!("a seat's name {e}")))?,
                ),
                SeatEdit::Agent(name) => agent_seat(&state, &self.db, &host, name, &old)?,
                SeatEdit::Player(name) => {
                    let user = self.db.user_by_name(name)?.map(|u| u.id);
                    let seat = old.iter().find(
                        |s| matches!(s, Seat::Player { user: u, .. } if Some(u) == user.as_ref()),
                    );
                    match seat {
                        Some(seat) if !seats.contains(seat) => seat.clone(),
                        _ => return bad(format!("{name} isn't sitting at the table")),
                    }
                }
            });
        }
        check_seats(&seats)?;
        let sits = |seats: &[Seat], user: &str| {
            (seats.iter()).any(|s| matches!(s, Seat::Player { user: u, .. } if u == user))
        };
        if !sits(&seats, &ident.user) {
            return bad("the host keeps a seat; leave the table instead");
        }
        let removed: Vec<String> = (old.iter())
            .filter_map(|s| match s {
                Seat::Player { user, .. } if !sits(&seats, user) => Some(user.clone()),
                _ => None,
            })
            .collect();
        for user in removed {
            state.seated.remove(&user);
            if let Some(gone) = self.db.user_ref(&user) {
                let line = format!("{} was removed from the table.", gone.name);
                state.say(&format!("table-{id}"), "", ChatKind::System, &line);
            }
        }
        let table = state.hosted(id, &ident.user)?;
        if table.seats.len() != seats.len() {
            table.unready();
        }
        table.seats = seats;
        table.idle_since = Instant::now();
        let info = self.table_info(&state, id);
        drop(state);
        self.bump();
        self.chat.send_modify(|v| *v += 1);
        info
    }

    /// Sits `ident` down at a table: at seat `seat`, or the first open one.
    pub fn join_table(&self, ident: &Ident, id: &str, seat: Option<u8>) -> Result<TableInfo> {
        limited(&self.limits.lobby, ident.user.clone(), "changes to tables")?;
        let name = self.user(&ident.user)?.name;
        let mut state = lock(&self.state);
        if state.seated.get(&ident.user).is_some_and(|t| t == id) {
            return self.table_info(&state, id);
        }
        let table = state.table_of(id)?;
        if table.game.is_some() || table.starting {
            return conflict("the table's game is running");
        }
        let open = |i: usize| table.seats.get(i) == Some(&Seat::Open);
        let at = match seat {
            Some(i) if open(usize::from(i)) => usize::from(i),
            Some(i) => return conflict(format!("seat {i} isn't open")),
            None => match (0..table.seats.len()).find(|&i| open(i)) {
                Some(i) => i,
                None => return conflict("the table has no open seat"),
            },
        };
        state.leave_all(&self.db, &ident.user);
        let table = state.table_of(id)?;
        if table.seats.get(at) != Some(&Seat::Open) {
            return conflict("the table has no open seat");
        }
        table.seats[at] = Seat::Player {
            user: ident.user.clone(),
            ready: false,
        };
        table.idle_since = Instant::now();
        state.seated.insert(ident.user.clone(), id.to_string());
        state.touch(&ident.user);
        state.say(
            &format!("table-{id}"),
            "",
            ChatKind::System,
            &format!("{name} sat down."),
        );
        let info = self.table_info(&state, id);
        drop(state);
        self.bump();
        self.chat.send_modify(|v| *v += 1);
        info
    }

    /// Takes `ident` out of the table they sit at.
    pub fn leave_table(&self, ident: &Ident) {
        let left = lock(&self.state).leave_table(&self.db, &ident.user);
        if left.is_some() {
            self.bump();
            self.chat.send_modify(|v| *v += 1);
        }
    }

    /// Says whether `ident` is ready. With everyone ready and no seat open, the table's
    /// game starts.
    pub fn ready(&self, ident: &Ident, id: &str, ready: bool) -> Result<TableInfo> {
        if ready && self.hub.in_timed_game(&ident.user) {
            return conflict("finish or resign the game you are in first");
        }
        let start = {
            let mut state = lock(&self.state);
            let table = state.table_of(id)?;
            if table.game.is_some() || table.starting {
                return conflict("the table's game is running");
            }
            let seat = table.seats.iter_mut().find_map(|s| match s {
                Seat::Player { user, ready } if *user == ident.user => Some(ready),
                _ => None,
            });
            let Some(seat) = seat else {
                return Err(Error::Forbidden("you aren't sitting at that table".into()));
            };
            *seat = ready;
            table.idle_since = Instant::now();
            let all = table.seats.iter().all(|s| match s {
                Seat::Open => false,
                Seat::Player { ready, .. } => *ready,
                Seat::Token(_) | Seat::Agent(_) => true,
            });
            if all {
                table.starting = true;
            }
            state.touch(&ident.user);
            all.then(|| self.starting(&state, id)).flatten()
        };
        self.bump();
        if let Some(start) = start {
            let table = start.table.clone();
            let started = self.start(start);
            let mut state = lock(&self.state);
            if let Some(table) = state.tables.get_mut(&table) {
                table.starting = false;
                table.unready();
            }
            drop(state);
            self.bump();
            started?;
        }
        self.table(id)
    }

    fn starting(&self, state: &State, id: &str) -> Option<Start> {
        let table = state.tables.get(id)?;
        let users: Vec<User> = (table.players().chain(table.agents()))
            .filter_map(|u| self.db.user(u).ok().flatten())
            .collect();
        let mut banned: Vec<String> = vec![];
        if table.spec.respect_banned {
            for user in users.iter().filter(|u| !table.agents().any(|a| a == u.id)) {
                let list = user.banned_cards.as_ref();
                for card in list.unwrap_or(&self.settings.banned_cards) {
                    if !banned.contains(card) {
                        banned.push(card.clone());
                    }
                }
            }
        }
        // A bot against its owner, or two bots of one owner, play for no rating.
        let owner = |u: &User| u.owner().unwrap_or(&u.id).to_string();
        let owners: std::collections::HashSet<String> = users.iter().map(owner).collect();
        let rated = table.spec.rated
            && users.len() == table.seats.len()
            && owners.len() == users.len()
            && users.iter().all(|u| u.kind() != UserKind::Guest);
        Some(Start {
            table: id.to_string(),
            host: table.host.clone(),
            spec: table.spec.clone(),
            seats: table.seats.clone(),
            banned,
            rated,
        })
    }

    /// Sets a table's game up and tells the table. No lock is held meanwhile.
    fn start(&self, start: Start) -> Result<()> {
        let busy = start.seats.iter().find_map(|s| match s {
            Seat::Player { user, .. } if self.hub.in_timed_game(user) => Some(user.clone()),
            _ => None,
        });
        if let Some(user) = busy {
            let name = self.db.user_ref(&user).map(|u| u.name);
            return conflict(format!("{} is still in a game", name.unwrap_or_default()));
        }
        let request = deal_request(&start.spec, start.seats.len(), start.banned);
        let (mut config, _) = hub::deal_config(&request)?;
        config.colonies = start.spec.colonies;
        config.shelters = start.spec.shelters;
        config.turn_limit = start.spec.turn_limit;
        let mut order: Vec<usize> = (0..start.seats.len()).collect();
        if start.spec.randomize_order {
            order.shuffle(&mut rand::rng());
        }
        let (names, owners): (Vec<String>, Vec<Option<String>>) = (order.iter())
            .map(|&i| match &start.seats[i] {
                Seat::Player { user, .. } => {
                    let name = self.db.user_ref(user).map(|u| u.name);
                    (name.unwrap_or_default(), Some(user.clone()))
                }
                Seat::Agent(user) => {
                    let name = self.db.user_ref(user).map(|u| u.name);
                    (name.unwrap_or_default(), Some(user.clone()))
                }
                Seat::Token(name) => (name.clone(), None),
                Seat::Open => (String::new(), None),
            })
            .unzip();
        if (1..names.len()).any(|i| {
            names[..i]
                .iter()
                .any(|n| n.to_lowercase() == names[i].to_lowercase())
        }) {
            return bad("two seats have the same name");
        }
        let meta = Meta {
            source: Source::Table,
            owners: owners.clone(),
            rated: start.rated,
            watchable: start.spec.watchable,
            timed: start.spec.timed,
            no_undo: !start.spec.undo,
            table: Some(start.table.clone()),
            ..Meta::default()
        };
        let agents = start
            .seats
            .iter()
            .filter(|s| matches!(s, Seat::Agent(_)))
            .count();
        let reservation = self.reserve(&start.host, agents as u32)?;
        let created = self.hub.host(config, names, meta)?;
        let mut state = lock(&self.state);
        if let Some(table) = state.tables.get_mut(&start.table) {
            table.game = Some(created.game_id.clone());
            table.tokens = (created.seats.iter())
                .filter(|s| owners[usize::from(s.seat)].is_none())
                .cloned()
                .collect();
        }
        drop(state);
        reservation.keep();
        Ok(())
    }

    /// Tells the table that started game `game` that it is over.
    pub(super) fn table_game_over(&self, state: &mut State, table: &str, game: &str) {
        if let Some(table) = state.tables.get_mut(table)
            && table.game.as_deref() == Some(game)
        {
            table.game = None;
            table.last_game = Some(game.to_string());
            table.unready();
            let players: Vec<String> = table.players().map(str::to_string).collect();
            for user in players {
                state.touch(&user);
            }
        }
    }
}
