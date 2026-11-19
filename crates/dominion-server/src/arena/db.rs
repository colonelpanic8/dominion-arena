//! Accounts, sessions and finished games' records: a SQLite database beside the games
//! (`dominion.sqlite` in the state directory), or in memory for a server that keeps
//! nothing. Every change is one transaction.

use super::{Error, Login, Result, User};
use crate::hub::lock;
use crate::rating::Rating;
use dominion_protocol::lobby::{GameRecord, Source, UserKind, UserRef, Venue};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, Transaction, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub(super) const FILE: &str = "dominion.sqlite";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sequences (name TEXT PRIMARY KEY, value INTEGER NOT NULL);
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
    rated_wins INTEGER NOT NULL DEFAULT 0,
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
    record TEXT NOT NULL,
    -- Where it was played: `arena` or `official` (dominion.games).
    venue TEXT NOT NULL DEFAULT 'arena'
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS games_ended ON games (ended DESC);
CREATE TABLE IF NOT EXISTS agent_transcript (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    game TEXT NOT NULL,
    seat INTEGER NOT NULL,
    agent TEXT NOT NULL,
    attempt TEXT NOT NULL,
    offset INTEGER NOT NULL,
    activity TEXT NOT NULL,
    UNIQUE (game, seat, attempt, offset)
);
CREATE INDEX IF NOT EXISTS agent_transcript_game ON agent_transcript (game, sequence);
-- An account's picture: a small image its owner uploaded, PNG, JPEG or WebP.
CREATE TABLE IF NOT EXISTS avatars (
    user INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    image BLOB NOT NULL,
    -- When it was set (ms): its URL changes with it.
    updated INTEGER NOT NULL
);
-- The accounts that played each game. They outlive a guest's account.
CREATE TABLE IF NOT EXISTS game_players (
    user INTEGER NOT NULL,
    ended INTEGER NOT NULL,
    game TEXT NOT NULL REFERENCES games (id) ON DELETE CASCADE,
    PRIMARY KEY (user, ended, game)
) WITHOUT ROWID;
";

fn venue_name(venue: Venue) -> &'static str {
    match venue {
        Venue::Arena => "arena",
        Venue::Official => "official",
    }
}

/// Gives a database from before `games.venue` the column, read from each record.
fn add_venues(conn: &Connection) -> rusqlite::Result<()> {
    let has = conn
        .prepare("SELECT 1 FROM pragma_table_info('games') WHERE name = 'venue'")?
        .exists([])?;
    if has {
        return Ok(());
    }
    conn.execute_batch("ALTER TABLE games ADD COLUMN venue TEXT NOT NULL DEFAULT 'arena'")?;
    let official: Vec<String> = {
        let mut query = conn.prepare("SELECT id, record FROM games")?;
        let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut ids = vec![];
        for row in rows {
            let (id, record) = row?;
            let record = serde_json::from_str::<GameRecord>(&record);
            if record.is_ok_and(|r| r.source == Source::Official) {
                ids.push(id);
            }
        }
        ids
    };
    for id in official {
        conn.execute("UPDATE games SET venue = 'official' WHERE id = ?1", [id])?;
    }
    Ok(())
}

/// Gives a database from before `users.rated_wins` the column, counted from the records
/// of its rated games.
fn add_rated_wins(conn: &Connection) -> rusqlite::Result<()> {
    let has = conn
        .prepare("SELECT 1 FROM pragma_table_info('users') WHERE name = 'rated_wins'")?
        .exists([])?;
    if has {
        return Ok(());
    }
    conn.execute_batch("ALTER TABLE users ADD COLUMN rated_wins INTEGER NOT NULL DEFAULT 0")?;
    let mut wins: HashMap<i64, u32> = HashMap::new();
    {
        let mut query = conn.prepare(
            "SELECT u.id, lower(u.name), g.record FROM game_players p \
             JOIN users u ON u.id = p.user JOIN games g ON g.id = p.game \
             WHERE g.venue = 'arena'",
        )?;
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (user, name, record) = row?;
            let Ok(record) = serde_json::from_str::<GameRecord>(&record) else {
                continue;
            };
            if record.rated && rated_win(&record, &name) {
                *wins.entry(user).or_default() += 1;
            }
        }
    }
    for (user, n) in wins {
        conn.execute(
            "UPDATE users SET rated_wins = ?2 WHERE id = ?1",
            params![user, n],
        )?;
    }
    Ok(())
}

/// Whether the account called `name` (lowercased) won `record`: placed first, with
/// someone placed lower.
fn rated_win(record: &GameRecord, name: &str) -> bool {
    let seat = (record.players.iter())
        .find(|p| p.user.as_deref().unwrap_or(&p.name).to_lowercase() == name);
    let won = record.players.iter().any(|p| p.rank != 1);
    won && seat.is_some_and(|p| p.rank == 1)
}

/// Kept in the file (`PRAGMA user_version`), for the day the schema changes.
const SCHEMA_VERSION: i32 = 6;

const USER: &str = "id, name, kind, password, owner, api_key, rating, deviation, volatility, \
                    rated_at, rated_games, games, wins, banned_cards, rated_wins";

/// When account `id`'s picture was set, if it has one.
fn avatar_set(conn: &Connection, id: &str) -> rusqlite::Result<Option<u64>> {
    let mut query = conn.prepare_cached("SELECT updated FROM avatars WHERE user = ?1")?;
    query
        .query_row([row_of(id)], |r| r.get::<_, i64>(0))
        .optional()
        .map(|t| t.map(|t| t as u64))
}

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
        rated_wins: row.get(14)?,
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
        add_venues(&conn)?;
        add_rated_wins(&conn)?;
        conn.execute_batch("CREATE INDEX IF NOT EXISTS games_venue ON games (venue, ended DESC)")?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Db {
            conn: Mutex::new(conn),
            refs: Mutex::default(),
        })
    }

    pub fn append_transcript(
        &self,
        game: &str,
        seat: u8,
        agent: &str,
        activity: &dominion_protocol::transcript::TranscriptInput,
    ) -> Result<()> {
        let text = serde_json::to_string(activity).map_err(|e| Error::Storage(e.to_string()))?;
        lock(&self.conn).execute("INSERT OR IGNORE INTO agent_transcript (game,seat,agent,attempt,offset,activity) VALUES (?1,?2,?3,?4,?5,?6)", params![game,seat,agent,activity.attempt,activity.offset,text])?;
        Ok(())
    }

    pub fn transcript(
        &self,
        game: &str,
        after: u64,
    ) -> Result<dominion_protocol::transcript::TranscriptPage> {
        use dominion_protocol::transcript::{TranscriptEntry, TranscriptPage};
        let conn = lock(&self.conn);
        let mut query = conn.prepare_cached("SELECT sequence,seat,agent,activity FROM agent_transcript WHERE game=?1 AND sequence>?2 ORDER BY sequence LIMIT 201")?;
        let rows = query.query_map(params![game, after.min(i64::MAX as u64)], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, u8>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut entries = vec![];
        for row in rows {
            let (sequence, seat, agent, activity) = row?;
            let activity =
                serde_json::from_str(&activity).map_err(|e| Error::Storage(e.to_string()))?;
            entries.push(TranscriptEntry {
                sequence,
                seat,
                agent,
                activity,
            });
        }
        let more = entries.len() > 200;
        entries.truncate(200);
        let next = entries.last().map_or(after, |e| e.sequence);
        Ok(TranscriptPage {
            entries,
            next,
            more,
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
                avatar: avatar_set(&conn, id).ok().flatten(),
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

    /// Gives account `id` the picture `image` of media type `kind`, set at `now` (ms).
    pub fn set_avatar(&self, id: &str, kind: &str, image: &[u8], now: u64) -> Result<()> {
        lock(&self.conn).execute(
            "INSERT OR REPLACE INTO avatars (user, type, image, updated) VALUES (?1, ?2, ?3, ?4)",
            params![row_of(id), kind, image, now as i64],
        )?;
        self.changed();
        Ok(())
    }

    pub fn clear_avatar(&self, id: &str) -> Result<()> {
        lock(&self.conn).execute("DELETE FROM avatars WHERE user = ?1", [row_of(id)])?;
        self.changed();
        Ok(())
    }

    /// The picture of the account called `name`, and its media type.
    pub fn avatar_of(&self, name: &str) -> Result<Option<(String, Vec<u8>)>> {
        let conn = lock(&self.conn);
        let mut query = conn.prepare_cached(
            "SELECT a.type, a.image FROM avatars a JOIN users u ON u.id = a.user \
             WHERE u.name_key = ?1",
        )?;
        let key = name.trim().to_lowercase();
        Ok(query
            .query_row([key], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
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
        if now.saturating_sub(seen as u64) >= super::accounts::SESSION_DAYS * 86_400 {
            conn.execute("DELETE FROM sessions WHERE token = ?1", [token])?;
            return Ok(None);
        }
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

    /// Allocates a lobby identity that will never be reused after restart.
    pub fn reserve_table_numbers(&self, through: u64) -> Result<()> {
        self.transaction(|tx| {
            tx.execute("INSERT INTO sequences (name,value) VALUES ('table',?1) ON CONFLICT(name) DO UPDATE SET value = MAX(value,excluded.value)",[through.min(i64::MAX as u64) as i64])?;
            Ok(())
        })
    }

    pub fn next_table_number(&self) -> Result<u64> {
        self.transaction(|tx| {
            tx.execute("INSERT INTO sequences (name, value) VALUES ('table', 1) ON CONFLICT(name) DO UPDATE SET value = value + 1", [])?;
            let n: i64 = tx.query_row("SELECT value FROM sequences WHERE name = 'table'", [], |r| r.get(0))?;
            u64::try_from(n).map_err(|_| Error::Storage("table identity overflow".into()))
        })
    }

    /// The number of the last game recorded.
    pub fn last_game_number(&self) -> Result<u64> {
        let conn = lock(&self.conn);
        let last: Option<i64> =
            conn.query_row("SELECT MAX(number) FROM games", [], |r| r.get(0))?;
        Ok(last.unwrap_or(0) as u64)
    }

    /// Up to `limit` games played at `venue` that ended before `before`, newest first:
    /// account `user`'s, or everyone's.
    pub fn games(
        &self,
        venue: Venue,
        user: Option<&str>,
        before: u64,
        limit: usize,
    ) -> Result<Vec<GameRecord>> {
        let conn = lock(&self.conn);
        let before = before.min(i64::MAX as u64) as i64;
        let limit = limit.min(i64::MAX as usize) as i64;
        let venue = venue_name(venue);
        Ok(match user {
            Some(user) => records(
                &conn,
                "SELECT games.record FROM game_players JOIN games ON games.id = game_players.game \
                 WHERE game_players.user = ?1 AND games.venue = ?2 AND game_players.ended < ?3 \
                 ORDER BY game_players.ended DESC LIMIT ?4",
                params![row_of(user), venue, before, limit],
            )?,
            None => records(
                &conn,
                "SELECT record FROM games WHERE venue = ?1 AND ended < ?2 \
                 ORDER BY ended DESC LIMIT ?3",
                params![venue, before, limit],
            )?,
        })
    }

    /// How many games played at `venue` account `user` played.
    pub fn count_games(&self, venue: Venue, user: &str) -> Result<u32> {
        let conn = lock(&self.conn);
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM game_players JOIN games ON games.id = game_players.game \
             WHERE game_players.user = ?1 AND games.venue = ?2",
            params![row_of(user), venue_name(venue)],
            |r| r.get(0),
        )?)
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
             rated_games = rated_games + 1, rated_wins = rated_wins + ?6 WHERE id = ?1",
            params![
                row_of(t.user),
                r.rating,
                r.deviation,
                r.volatility,
                r.at as i64,
                t.win as i64
            ],
        )?;
    }
    Ok(())
}

/// Takes a voided game off one of its players' tallies of every game.
pub(super) fn untally(tx: &Transaction<'_>, user: &str, win: bool) -> Result<()> {
    tx.execute(
        "UPDATE users SET games = MAX(games - 1, 0), wins = MAX(wins - ?2, 0) WHERE id = ?1",
        params![row_of(user), win as i64],
    )?;
    Ok(())
}

/// Every Arena game's record in the order the games ended, with the accounts that
/// played it.
pub(super) fn arena_records(tx: &Transaction<'_>) -> Result<Vec<(GameRecord, Vec<String>)>> {
    let mut query = tx
        .prepare("SELECT id, record FROM games WHERE venue = 'arena' ORDER BY ended, number, id")?;
    let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut players = tx.prepare_cached("SELECT user FROM game_players WHERE game = ?1")?;
    let mut out = vec![];
    for row in rows {
        let (id, json) = row?;
        let record = serde_json::from_str(&json)
            .map_err(|e| Error::Storage(format!("game {id}'s record: {e}")))?;
        let users = players.query_map([&id], |r| r.get::<_, i64>(0))?;
        let users = users
            .map(|u| u.map(id_of))
            .collect::<rusqlite::Result<_>>()?;
        out.push((record, users));
    }
    Ok(out)
}

/// An account's rating, rated games and rated wins.
pub(super) struct Rated {
    pub id: String,
    pub name: String,
    pub rating: Rating,
    pub games: u32,
    pub wins: u32,
}

/// Every account's rating.
pub(super) fn ratings(tx: &Transaction<'_>) -> Result<Vec<Rated>> {
    let mut query = tx.prepare(
        "SELECT id, name, rating, deviation, volatility, rated_at, rated_games, rated_wins \
         FROM users",
    )?;
    let rows = query.query_map([], |r| {
        Ok(Rated {
            id: id_of(r.get(0)?),
            name: r.get(1)?,
            rating: Rating {
                rating: r.get(2)?,
                deviation: r.get(3)?,
                volatility: r.get(4)?,
                at: r.get::<_, i64>(5)? as u64,
            },
            games: r.get(6)?,
            wins: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Replaces a game's record.
pub(super) fn set_record(tx: &Transaction<'_>, record: &GameRecord) -> Result<()> {
    let json = serde_json::to_string(record).map_err(|e| Error::Storage(e.to_string()))?;
    tx.execute(
        "UPDATE games SET record = ?2 WHERE id = ?1",
        params![record.game_id, json],
    )?;
    Ok(())
}

/// Sets account `user`'s rating, rated games and rated wins.
pub(super) fn set_rating(
    tx: &Transaction<'_>,
    user: &str,
    r: Rating,
    games: u32,
    wins: u32,
) -> Result<()> {
    tx.execute(
        "UPDATE users SET rating = ?2, deviation = ?3, volatility = ?4, rated_at = ?5, \
         rated_games = ?6, rated_wins = ?7 WHERE id = ?1",
        params![
            row_of(user),
            r.rating,
            r.deviation,
            r.volatility,
            r.at as i64,
            games,
            wins
        ],
    )?;
    Ok(())
}

/// Writes a finished game's record, played at `venue`, for the accounts `users` that
/// played it.
pub(super) fn add_record(
    tx: &Transaction<'_>,
    venue: Venue,
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
        "INSERT INTO games (id, number, ended, record, venue) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            record.game_id,
            number.unwrap_or(0i64),
            ended,
            json,
            venue_name(venue)
        ],
    )?;
    for user in users.iter().flatten() {
        tx.execute(
            "INSERT OR IGNORE INTO game_players (user, ended, game) VALUES (?1, ?2, ?3)",
            params![row_of(user), ended, record.game_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod transcript_tests {
    use super::*;
    use dominion_protocol::transcript::{TranscriptInput, TranscriptKind};

    #[test]
    fn transcripts_survive_reopening_and_page_without_repeating_retries() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(Some(dir.path())).unwrap();
        for offset in 1..=220 {
            let entry = TranscriptInput {
                attempt: "attempt".into(),
                offset,
                time: offset,
                kind: TranscriptKind::Reasoning,
                block: "thought".into(),
                append: true,
                text: "x".into(),
            };
            db.append_transcript("g1", 0, "Agent", &entry).unwrap();
            db.append_transcript("g1", 0, "Agent", &entry).unwrap();
        }
        drop(db);
        let db = Db::open(Some(dir.path())).unwrap();
        let page = db.transcript("g1", 0).unwrap();
        assert_eq!(page.entries.len(), 200);
        assert!(page.more);
        let rest = db.transcript("g1", page.next).unwrap();
        assert_eq!(rest.entries.len(), 20);
        assert!(!rest.more);
        assert_eq!(rest.entries[0].activity.offset, 201);
        assert!(db.transcript("g2", 0).unwrap().entries.is_empty());
    }
}

#[cfg(test)]
mod venue_tests {
    use super::*;
    use dominion_protocol::lobby::{GameRecord, RecordPlayer};

    fn record(id: &str, source: Source, ended: u64) -> String {
        serde_json::to_string(&GameRecord {
            game_id: id.into(),
            started: 0,
            ended,
            source,
            rated: false,
            voided: None,
            reason: String::new(),
            kingdom: vec![],
            landscapes: vec![],
            others: vec![],
            players: vec![],
            seed: None,
            version: None,
        })
        .unwrap()
    }

    #[test]
    fn rated_wins_are_counted_from_the_records_of_a_database_kept_without_them() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(FILE)).unwrap();
            conn.execute_batch(&SCHEMA.replace("    rated_wins INTEGER NOT NULL DEFAULT 0,\n", ""))
                .unwrap();
            for (row, name) in [(1, "Ada"), (2, "Bo")] {
                conn.execute(
                    "INSERT INTO users (id, name, name_key, kind, created, rating, deviation, \
                     volatility, rated_at) VALUES (?1, ?2, lower(?2), 'registered', 0, 1500, \
                     350, .06, 0)",
                    params![row, name],
                )
                .unwrap();
            }
            let seat = |name: &str, rank: u32| RecordPlayer {
                seat: 0,
                name: name.into(),
                kind: Some(UserKind::Registered),
                user: None,
                vp: 0,
                rank,
                turns: 0,
                resigned: false,
                rating: None,
                level: None,
                agent: None,
                owner: None,
            };
            let games = [
                ("g1", true, [("Ada", 1), ("Bo", 2)]),
                ("g2", true, [("Ada", 1), ("Bo", 1)]),
                ("g3", false, [("Ada", 1), ("Bo", 2)]),
                ("g4", true, [("Ada", 2), ("Bo", 1)]),
            ];
            for (id, rated, seats) in games {
                let mut game: GameRecord =
                    serde_json::from_str(&record(id, Source::Table, 1)).unwrap();
                game.rated = rated;
                game.players = seats.iter().map(|&(n, r)| seat(n, r)).collect();
                conn.execute(
                    "INSERT INTO games (id, number, ended, record) VALUES (?1, 0, 1, ?2)",
                    params![id, serde_json::to_string(&game).unwrap()],
                )
                .unwrap();
                for user in [1, 2] {
                    conn.execute(
                        "INSERT INTO game_players (user, ended, game) VALUES (?1, 1, ?2)",
                        params![user, id],
                    )
                    .unwrap();
                }
            }
        }
        let db = Db::open(Some(dir.path())).unwrap();
        let wins = |name| db.user_by_name(name).unwrap().unwrap().rated_wins;
        assert_eq!((wins("Ada"), wins("Bo")), (1, 1));
    }

    #[test]
    fn records_from_before_venues_are_given_theirs() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(FILE)).unwrap();
            conn.execute_batch(
                "CREATE TABLE games (id TEXT PRIMARY KEY, number INTEGER NOT NULL, \
                 ended INTEGER NOT NULL, record TEXT NOT NULL) WITHOUT ROWID;",
            )
            .unwrap();
            for (id, source, ended) in [
                ("g1", Source::Table, 1),
                ("dominion.games-9", Source::Official, 2),
            ] {
                conn.execute(
                    "INSERT INTO games (id, number, ended, record) VALUES (?1, 0, ?2, ?3)",
                    params![id, ended, record(id, source, ended as u64)],
                )
                .unwrap();
            }
        }
        let db = Db::open(Some(dir.path())).unwrap();
        let ids = |venue| -> Vec<String> {
            let games = db.games(venue, None, u64::MAX, 10).unwrap();
            games.into_iter().map(|g| g.game_id).collect()
        };
        assert_eq!(ids(Venue::Arena), ["g1"]);
        assert_eq!(ids(Venue::Official), ["dominion.games-9"]);
    }

    #[test]
    fn a_record_kept_before_venues_keeps_its_seed_and_agents() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(FILE)).unwrap();
            conn.execute_batch(
                "CREATE TABLE games (id TEXT PRIMARY KEY, number INTEGER NOT NULL, \
                 ended INTEGER NOT NULL, record TEXT NOT NULL) WITHOUT ROWID;",
            )
            .unwrap();
            let record = r#"{"gameId":"g1","started":0,"ended":1,"source":"direct",
                "rated":true,"reason":"","kingdom":[],"landscapes":[],"seed":7,
                "version":"abc","players":[{"seat":0,"name":"Opus","kind":"bot","vp":3,
                "rank":1,"turns":9,"agent":{"agent":"Opus","program":"Claude Code",
                "attempts":[{"started":5,"archive":"a1"}]}}]}"#;
            conn.execute(
                "INSERT INTO games (id, number, ended, record) VALUES ('g1', 0, 1, ?1)",
                [record],
            )
            .unwrap();
        }
        let db = Db::open(Some(dir.path())).unwrap();
        let games = db.games(Venue::Arena, None, u64::MAX, 10).unwrap();
        let game = super::super::records::past_game(games[0].clone(), Venue::Arena, &|_| None);
        assert_eq!((game.seed, game.version.as_deref()), (Some(7), Some("abc")));
        let run = game.players[0].agent.as_ref().unwrap();
        assert_eq!((run.setup.agent.as_str(), run.attempts.len()), ("Opus", 1));
        assert_eq!(
            game.arranged,
            Some(dominion_protocol::lobby::Arranged::Agents)
        );
    }
}
