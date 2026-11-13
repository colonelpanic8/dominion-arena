//! Accounts, sessions and finished games' records: a SQLite database beside the games
//! (`dominion.sqlite` in the state directory), or in memory for a server that keeps
//! nothing. Every change is one transaction.

use super::{Error, Login, Result, User};
use crate::hub::lock;
use crate::rating::Rating;
use dominion_protocol::lobby::{GameRecord, UserKind, UserRef};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, Transaction, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub(super) const FILE: &str = "dominion.sqlite";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    -- Lowercased: a name is one account's whatever its case.
    name_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('registered', 'guest', 'bot')),
    -- An argon2 hash in PHC form, of a registered account.
    password TEXT,
    -- A bot's owner; none for the server's own agents.
    owner INTEGER REFERENCES users (id) ON DELETE CASCADE,
    -- The SHA-256 of a bot's API key.
    api_key TEXT UNIQUE,
    created INTEGER NOT NULL,
    rating REAL NOT NULL,
    deviation REAL NOT NULL,
    volatility REAL NOT NULL,
    rated_at INTEGER NOT NULL,
    rated_games INTEGER NOT NULL DEFAULT 0,
    games INTEGER NOT NULL DEFAULT 0,
    wins INTEGER NOT NULL DEFAULT 0,
    -- Their own Banned cards list as a JSON array, once they have set one.
    banned_cards TEXT
);
CREATE INDEX IF NOT EXISTS users_owner ON users (owner);
CREATE INDEX IF NOT EXISTS users_rating ON users (rating DESC) WHERE rated_games > 0;
CREATE TABLE IF NOT EXISTS sessions (
    -- The SHA-256 of the session's token.
    token TEXT PRIMARY KEY,
    user INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created INTEGER NOT NULL,
    -- When it was last used, kept to the day.
    seen INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user);
CREATE TABLE IF NOT EXISTS games (
    id TEXT PRIMARY KEY,
    -- The number in the id.
    number INTEGER NOT NULL,
    ended INTEGER NOT NULL,
    -- The GameRecord as JSON.
    record TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS games_ended ON games (ended DESC);
-- The accounts that played each game. They outlive a guest's account.
CREATE TABLE IF NOT EXISTS game_players (
    user INTEGER NOT NULL,
    ended INTEGER NOT NULL,
    game TEXT NOT NULL REFERENCES games (id) ON DELETE CASCADE,
    PRIMARY KEY (user, ended, game)
) WITHOUT ROWID;
";

/// Kept in the file (`PRAGMA user_version`), for the day the schema changes.
const SCHEMA_VERSION: i32 = 1;

const USER: &str = "id, name, kind, password, owner, api_key, rating, deviation, volatility, \
                    rated_at, rated_games, games, wins, banned_cards";

fn id_of(row: i64) -> String {
    format!("u{row}")
}

/// The row of account `id` (`u<row>`); no row has -1.
fn row_of(id: &str) -> i64 {
    let row = id.strip_prefix('u').and_then(|n| n.parse().ok());
    row.unwrap_or(-1)
}

fn kind_name(kind: UserKind) -> &'static str {
    match kind {
        UserKind::Registered => "registered",
        UserKind::Guest => "guest",
        UserKind::Bot => "bot",
    }
}

fn read_user(row: &Row<'_>) -> rusqlite::Result<User> {
    let kind: String = row.get(2)?;
    let login = match kind.as_str() {
        "registered" => Login::Password(row.get::<_, Option<String>>(3)?.unwrap_or_default()),
        "bot" => match row.get::<_, Option<i64>>(4)? {
            Some(owner) => Login::Bot {
                owner: id_of(owner),
                key: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            },
            None => Login::House,
        },
        _ => Login::Guest,
    };
    let banned: Option<String> = row.get(13)?;
    Ok(User {
        id: id_of(row.get(0)?),
        name: row.get(1)?,
        login,
        rating: Rating {
            rating: row.get(6)?,
            deviation: row.get(7)?,
            volatility: row.get(8)?,
            at: row.get::<_, i64>(9)? as u64,
        },
        rated_games: row.get(10)?,
        games: row.get(11)?,
        wins: row.get(12)?,
        banned_cards: banned.and_then(|b| serde_json::from_str(&b).ok()),
    })
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Storage(e.to_string())
    }
}

fn taken(e: rusqlite::Error, name: &str) -> Error {
    match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::ConstraintViolation => {
            Error::Conflict(format!("the name {name} is taken"))
        }
        e => e.into(),
    }
}

/// How one player of a finished game is counted.
pub(super) struct Tally<'a> {
    pub user: &'a str,
    pub win: bool,
    /// Their rating after a rated game.
    pub rating: Option<Rating>,
}

pub(super) struct Db {
    conn: Mutex<Connection>,
    /// Accounts as others see them, by id: cleared whenever an account changes.
    refs: Mutex<HashMap<String, Option<UserRef>>>,
}

fn user(conn: &Connection, id: &str) -> rusqlite::Result<Option<User>> {
    let sql = format!("SELECT {USER} FROM users WHERE id = ?1");
    let mut query = conn.prepare_cached(&sql)?;
    query.query_row([row_of(id)], read_user).optional()
}

fn records(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Vec<GameRecord>> {
    let mut query = conn.prepare_cached(sql)?;
    let rows = query.query_map(params, |row| row.get::<_, String>(0))?;
    let mut out = vec![];
    for row in rows {
        if let Ok(record) = serde_json::from_str(&row?) {
            out.push(record);
        }
    }
    Ok(out)
}

impl Db {
    /// The database in `dir`, made if it isn't there; in memory without one.
    pub fn open(dir: Option<&Path>) -> Result<Db> {
        let conn = match dir {
            Some(dir) => {
                let path = dir.join(FILE);
                // It holds password hashes: its owner's alone from the start.
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create(true).truncate(false);
                #[cfg(unix)]
                std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                options
                    .open(&path)
                    .map_err(|e| Error::Storage(format!("{}: {e}", path.display())))?;
                let conn = Connection::open(&path)?;
                conn.pragma_update(None, "journal_mode", "WAL")?;
                conn
            }
            None => Connection::open_in_memory()?,
        };
        conn.pragma_update(None, "foreign_keys", true)?;
        // Someone looking into the file with another program holds it only briefly.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Db {
            conn: Mutex::new(conn),
            refs: Mutex::default(),
        })
    }

    fn changed(&self) {
        lock(&self.refs).clear();
    }

    pub fn user(&self, id: &str) -> Result<Option<User>> {
        Ok(user(&lock(&self.conn), id)?)
    }

    pub fn user_by_name(&self, name: &str) -> Result<Option<User>> {
        let conn = lock(&self.conn);
        let sql = format!("SELECT {USER} FROM users WHERE name_key = ?1");
        let mut query = conn.prepare_cached(&sql)?;
        let key = name.trim().to_lowercase();
        Ok(query.query_row([key], read_user).optional()?)
    }

    /// Account `id` as others see it.
    pub fn user_ref(&self, id: &str) -> Option<UserRef> {
        // Held while reading, so that a change made meanwhile clears what is put here.
        let mut refs = lock(&self.refs);
        if let Some(known) = refs.get(id) {
            return known.clone();
        }
        let conn = lock(&self.conn);
        let found = user(&conn, id).ok().flatten().map(|u| {
            let owner = u.owner().and_then(|o| user(&conn, o).ok().flatten());
            UserRef {
                kind: u.kind(),
                rating: (u.rated_games > 0).then(|| u.rating.rating.round() as i32),
                owner: owner.map(|o| o.name),
                name: u.name,
            }
        });
        drop(conn);
        if refs.len() >= 10_000 {
            refs.clear();
        }
        refs.insert(id.to_string(), found.clone());
        found
    }

    pub fn bots_of(&self, owner: &str) -> Result<Vec<User>> {
        let conn = lock(&self.conn);
        let sql = format!("SELECT {USER} FROM users WHERE owner = ?1 ORDER BY id");
        let mut query = conn.prepare_cached(&sql)?;
        let rows = query.query_map([row_of(owner)], read_user)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Whether `name` is nobody's, or only `but`'s.
    pub fn name_free(&self, name: &str, but: Option<&str>) -> Result<bool> {
        let holder = self.user_by_name(name)?;
        Ok(holder.is_none_or(|u| Some(u.id.as_str()) == but))
    }

    /// A new account. `now` in seconds since the Unix epoch.
    pub fn add_user(&self, name: &str, login: &Login, now: u64) -> Result<User> {
        let (kind, password, owner, key) = match login {
            Login::Password(hash) => (UserKind::Registered, Some(hash.as_str()), None, None),
            Login::Guest => (UserKind::Guest, None, None, None),
            Login::Bot { owner, key } => {
                (UserKind::Bot, None, Some(row_of(owner)), Some(key.as_str()))
            }
            Login::House => (UserKind::Bot, None, None, None),
        };
        let fresh = Rating::default();
        let conn = lock(&self.conn);
        conn.execute(
            "INSERT INTO users (name, name_key, kind, password, owner, api_key, created, \
             rating, deviation, volatility, rated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                name,
                name.to_lowercase(),
                kind_name(kind),
                password,
                owner,
                key,
                now as i64,
                fresh.rating,
                fresh.deviation,
                fresh.volatility,
                fresh.at as i64,
            ],
        )
        .map_err(|e| taken(e, name))?;
        let id = id_of(conn.last_insert_rowid());
        let added = user(&conn, &id)?;
        drop(conn);
        self.changed();
        added.ok_or_else(|| Error::Storage("the new account is missing".into()))
    }

    /// Makes account `id` a registered one called `name` with password hash `password`.
    pub fn make_registered(&self, id: &str, name: &str, password: &str) -> Result<()> {
        let conn = lock(&self.conn);
        conn.execute(
            "UPDATE users SET name = ?2, name_key = ?3, kind = 'registered', password = ?4 \
             WHERE id = ?1",
            params![row_of(id), name, name.to_lowercase(), password],
        )
        .map_err(|e| taken(e, name))?;
        drop(conn);
        self.changed();
        Ok(())
    }

    pub fn set_password(&self, id: &str, password: &str) -> Result<()> {
        let conn = lock(&self.conn);
        conn.execute(
            "UPDATE users SET password = ?2 WHERE id = ?1",
            params![row_of(id), password],
        )?;
        Ok(())
    }

    /// Replaces a bot's API key hash.
    pub fn set_api_key(&self, id: &str, key: &str) -> Result<()> {
        let conn = lock(&self.conn);
        conn.execute(
            "UPDATE users SET api_key = ?2 WHERE id = ?1",
            params![row_of(id), key],
        )?;
        Ok(())
    }

    pub fn set_banned_cards(&self, id: &str, cards: &[String]) -> Result<()> {
        let json = serde_json::to_string(cards).unwrap_or_default();
        let conn = lock(&self.conn);
        conn.execute(
            "UPDATE users SET banned_cards = ?2 WHERE id = ?1",
            params![row_of(id), json],
        )?;
        Ok(())
    }

    /// Deletes an account, with its sessions and its bots.
    pub fn delete_user(&self, id: &str) -> Result<()> {
        lock(&self.conn).execute("DELETE FROM users WHERE id = ?1", [row_of(id)])?;
        self.changed();
        Ok(())
    }

    /// The bot whose API key hashes to `key`.
    pub fn bot_by_key(&self, key: &str) -> Result<Option<String>> {
        let conn = lock(&self.conn);
        let mut query = conn.prepare_cached("SELECT id FROM users WHERE api_key = ?1")?;
        let row = query
            .query_row([key], |row| row.get::<_, i64>(0))
            .optional()?;
        Ok(row.map(id_of))
    }

    pub fn add_session(&self, token: &str, user: &str, now: u64) -> Result<()> {
        lock(&self.conn).execute(
            "INSERT INTO sessions (token, user, created, seen) VALUES (?1, ?2, ?3, ?3)",
            params![token, row_of(user), now as i64],
        )?;
        Ok(())
    }

    /// The account of the session whose token hashes to `token`, and whether it is a
    /// bot's. Notes that the session was used, to the day.
    pub fn session(&self, token: &str, now: u64) -> Result<Option<(String, UserKind)>> {
        let conn = lock(&self.conn);
        let mut query = conn.prepare_cached(
            "SELECT users.id, users.kind, sessions.seen FROM sessions \
             JOIN users ON users.id = sessions.user WHERE sessions.token = ?1",
        )?;
        let read = |row: &Row<'_>| {
            let (id, kind, seen): (i64, String, i64) = (row.get(0)?, row.get(1)?, row.get(2)?);
            Ok((id, kind, seen))
        };
        let Some((id, kind, seen)) = query.query_row([token], read).optional()? else {
            return Ok(None);
        };
        if now.saturating_sub(seen as u64) >= 86_400 {
            conn.execute(
                "UPDATE sessions SET seen = ?2 WHERE token = ?1",
                params![token, now as i64],
            )?;
        }
        let kind = match kind.as_str() {
            "registered" => UserKind::Registered,
            "bot" => UserKind::Bot,
            _ => UserKind::Guest,
        };
        Ok(Some((id_of(id), kind)))
    }

    pub fn end_session(&self, token: &str) -> Result<()> {
        lock(&self.conn).execute("DELETE FROM sessions WHERE token = ?1", [token])?;
        Ok(())
    }

    /// Ends `user`'s sessions but the one whose token hashes to `keep`.
    pub fn end_other_sessions(&self, user: &str, keep: &str) -> Result<()> {
        lock(&self.conn).execute(
            "DELETE FROM sessions WHERE user = ?1 AND token != ?2",
            params![row_of(user), keep],
        )?;
        Ok(())
    }

    /// Ends the sessions last used before `before` (a guest's: before `guests_before`).
    /// The guests left without a session.
    pub fn expire_sessions(&self, before: u64, guests_before: u64) -> Result<Vec<String>> {
        let conn = lock(&self.conn);
        conn.execute(
            "DELETE FROM sessions WHERE seen < ?1 OR (seen < ?2 AND user IN \
             (SELECT id FROM users WHERE kind = 'guest'))",
            params![before as i64, guests_before as i64],
        )?;
        let mut query = conn.prepare_cached(
            "SELECT id FROM users WHERE kind = 'guest' AND NOT EXISTS \
             (SELECT 1 FROM sessions WHERE sessions.user = users.id)",
        )?;
        let rows = query.query_map([], |row| row.get::<_, i64>(0))?;
        Ok(rows
            .map(|r| r.map(id_of))
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Runs `write` in a transaction: all of it happens, or none.
    pub fn transaction<T>(&self, write: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = lock(&self.conn);
        let tx = conn.transaction()?;
        let out = write(&tx)?;
        tx.commit()?;
        drop(conn);
        self.changed();
        Ok(out)
    }

    /// The number of the last game recorded.
    pub fn last_game_number(&self) -> Result<u64> {
        let conn = lock(&self.conn);
        let last: Option<i64> =
            conn.query_row("SELECT MAX(number) FROM games", [], |r| r.get(0))?;
        Ok(last.unwrap_or(0) as u64)
    }

    /// Finished games that ended before `before`, newest first: `user`'s, or everyone's.
    pub fn games(&self, user: Option<&str>, before: u64, limit: usize) -> Result<Vec<GameRecord>> {
        let conn = lock(&self.conn);
        let (before, limit) = (before.min(i64::MAX as u64) as i64, limit as i64);
        Ok(match user {
            Some(user) => records(
                &conn,
                "SELECT games.record FROM game_players JOIN games ON games.id = game_players.game \
                 WHERE game_players.user = ?1 AND game_players.ended < ?2 \
                 ORDER BY game_players.ended DESC LIMIT ?3",
                params![row_of(user), before, limit],
            )?,
            None => records(
                &conn,
                "SELECT record FROM games WHERE ended < ?1 ORDER BY ended DESC LIMIT ?2",
                params![before, limit],
            )?,
        })
    }

    /// The rated accounts, best first.
    pub fn leaders(&self, limit: usize) -> Result<Vec<User>> {
        let conn = lock(&self.conn);
        let sql =
            format!("SELECT {USER} FROM users WHERE rated_games > 0 ORDER BY rating DESC LIMIT ?1");
        let mut query = conn.prepare_cached(&sql)?;
        let rows = query.query_map([limit as i64], read_user)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// Account `id`, read inside a transaction.
pub(super) fn user_in(tx: &Transaction<'_>, id: &str) -> Result<Option<User>> {
    Ok(user(tx, id)?)
}

/// Whether game `id` has a record.
pub(super) fn recorded(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let mut query = tx.prepare_cached("SELECT 1 FROM games WHERE id = ?1")?;
    Ok(query.exists([id])?)
}

/// Counts a finished game for one of its players.
pub(super) fn tally(tx: &Transaction<'_>, t: &Tally<'_>) -> Result<()> {
    tx.execute(
        "UPDATE users SET games = games + 1, wins = wins + ?2 WHERE id = ?1",
        params![row_of(t.user), t.win as i64],
    )?;
    if let Some(r) = t.rating {
        tx.execute(
            "UPDATE users SET rating = ?2, deviation = ?3, volatility = ?4, rated_at = ?5, \
             rated_games = rated_games + 1 WHERE id = ?1",
            params![
                row_of(t.user),
                r.rating,
                r.deviation,
                r.volatility,
                r.at as i64
            ],
        )?;
    }
    Ok(())
}

/// Writes a finished game's record, for the accounts `users` that played it.
pub(super) fn add_record(
    tx: &Transaction<'_>,
    record: &GameRecord,
    users: &[Option<String>],
) -> Result<()> {
    let number = record
        .game_id
        .strip_prefix('g')
        .and_then(|n| n.parse().ok());
    let json = serde_json::to_string(record).map_err(|e| Error::Storage(e.to_string()))?;
    let ended = record.ended.min(i64::MAX as u64) as i64;
    tx.execute(
        "INSERT INTO games (id, number, ended, record) VALUES (?1, ?2, ?3, ?4)",
        params![record.game_id, number.unwrap_or(0i64), ended, json],
    )?;
    for user in users.iter().flatten() {
        tx.execute(
            "INSERT OR IGNORE INTO game_players (user, ended, game) VALUES (?1, ?2, ?3)",
            params![row_of(user), ended, record.game_id],
        )?;
    }
    Ok(())
}
