//! Accounts, how they sign in, and what they keep: sessions, bots, the Banned list.

use super::{
    Arena, Error, Ident, Login, Result, SESSIONS, Session, State, USERS, User, bad, conflict,
    limited,
};
use crate::hub::{lock, now_ms};
use crate::rating::Rating;
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use dominion_engine::Card;
use dominion_protocol::lobby::{BotKey, Credentials, Me, UserKind};
use rand::Rng;
use sha2::{Digest, Sha256};
use std::net::IpAddr;

/// The cookie a browser's session token is kept in.
pub const SESSION_COOKIE: &str = "dominion_session";
/// What an API key starts with.
const KEY_PREFIX: &str = "dk_";
const MAX_BOTS: usize = 5;
const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 200;
const DAY: u64 = 86_400;
/// How long a session lasts unused.
const SESSION_DAYS: u64 = 60;
const GUEST_DAYS: u64 = 30;

fn now() -> u64 {
    now_ms() / 1000
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn secret(bytes: usize) -> String {
    let mut rng = rand::rng();
    hex(&(0..bytes).map(|_| rng.random::<u8>()).collect::<Vec<_>>())
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::encode_b64(&rand::rng().random::<[u8; 16]>())
        .map_err(|e| Error::BadRequest(e.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| Error::BadRequest(e.to_string()))
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| {
        Argon2::default()
            .verify_password(password.as_bytes(), &h)
            .is_ok()
    })
}

fn check_password(password: &str) -> Result<()> {
    match password.chars().count() {
        n if n < MIN_PASSWORD => bad(format!("a password has at least {MIN_PASSWORD} characters")),
        n if n > MAX_PASSWORD => bad(format!("a password has at most {MAX_PASSWORD} characters")),
        _ => Ok(()),
    }
}

/// An account's name as given, trimmed: 2 to 20 letters, digits, spaces, `_`, `-` and
/// `.`, starting with a letter or digit. Seats are called P1, P2, ... when nobody names
/// them, so no account is.
pub fn account_name(given: &str) -> Result<String> {
    let name = given.trim();
    let len = name.chars().count();
    if !(2..=20).contains(&len) {
        return bad("a name has 2 to 20 characters");
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || " _-.".contains(c);
    if !name.chars().all(allowed) || !name.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return bad(
            "a name is made of letters, digits, spaces, _ - and . and starts with a letter \
             or digit",
        );
    }
    if name.contains("  ") {
        return bad("a name has no two spaces in a row");
    }
    let lower = name.to_lowercase();
    let seat = lower
        .strip_prefix('p')
        .is_some_and(|n| n.parse::<u32>().is_ok());
    if seat || ["you", "system", "server", "admin", "dominion"].contains(&lower.as_str()) {
        return bad(format!("the name {name} is reserved"));
    }
    Ok(name.to_string())
}

impl State {
    fn free_name(&self, name: &str, but: Option<&str>) -> Result<()> {
        match self.names.get(&name.to_lowercase()) {
            Some(id) if Some(id.as_str()) != but => conflict(format!("the name {name} is taken")),
            _ => Ok(()),
        }
    }

    fn add_user(&mut self, name: String, login: Login) -> User {
        self.next_user += 1;
        let user = User {
            id: format!("u{}", self.next_user),
            name,
            login,
            created: now(),
            rating: Rating::default(),
            rated_games: 0,
            games: 0,
            wins: 0,
            banned_cards: None,
        };
        self.names.insert(user.name.to_lowercase(), user.id.clone());
        if let Login::Bot { key, .. } = &user.login {
            self.keys.insert(key.clone(), user.id.clone());
        }
        self.users.insert(user.id.clone(), user.clone());
        user
    }

    fn remove_user(&mut self, id: &str) -> Option<User> {
        let user = self.users.remove(id)?;
        self.names.remove(&user.name.to_lowercase());
        if let Login::Bot { key, .. } = &user.login {
            self.keys.remove(key);
        }
        self.presence.remove(id);
        Some(user)
    }
}

impl Arena {
    fn start_session(&self, state: &mut State, user: &str) -> String {
        let token = secret(32);
        let session = Session {
            user: user.to_string(),
            created: now(),
            seen: now(),
        };
        let key = sha256(&token);
        if let Some(store) = &self.store
            && let Err(e) = store.save(Some(SESSIONS), &key, &session)
        {
            eprintln!("dominion: saving a session: {e}");
        }
        state.sessions.insert(key, session);
        token
    }

    fn end_session(&self, state: &mut State, key: &str) {
        state.sessions.remove(key);
        if let Some(store) = &self.store {
            let _ = store.remove(Some(SESSIONS), key);
        }
    }

    /// Who `token` is: a session's token or a bot's API key.
    pub fn identify(&self, token: &str) -> Option<Ident> {
        let key = sha256(token);
        let mut state = lock(&self.state);
        if token.starts_with(KEY_PREFIX) {
            let user = state.keys.get(&key)?.clone();
            return Some(Ident { user, agent: true });
        }
        let now = now();
        let session = state.sessions.get_mut(&key)?;
        let stale = now.saturating_sub(session.seen) >= DAY;
        if stale {
            session.seen = now;
        }
        let session = session.clone();
        let Some(user) = state.users.get(&session.user) else {
            self.end_session(&mut state, &key);
            return None;
        };
        if stale && let Some(store) = &self.store {
            let _ = store.save(Some(SESSIONS), &key, &session);
        }
        Some(Ident {
            user: user.id.clone(),
            agent: user.kind() == UserKind::Bot,
        })
    }

    pub fn me(&self, ident: &Ident) -> Result<Me> {
        let state = lock(&self.state);
        Ok(state.me(state.user(&ident.user)?))
    }

    /// A new account and a session for it. Asked by a guest, it is the guest's account
    /// from now on, under the name given or, with none, the guest's. This hashes a
    /// password: call it off the async threads.
    pub fn register(
        &self,
        ip: IpAddr,
        credentials: &Credentials,
        current: Option<&Ident>,
    ) -> Result<(Me, Option<String>)> {
        if !self.settings.registration {
            return Err(Error::Forbidden("this server takes no new accounts".into()));
        }
        check_password(&credentials.password)?;
        let guest = current.filter(|i| {
            let state = lock(&self.state);
            state
                .users
                .get(&i.user)
                .is_some_and(|u| u.kind() == UserKind::Guest)
        });
        let name = match (credentials.name.trim(), guest) {
            ("", Some(guest)) => lock(&self.state).user(&guest.user)?.name.clone(),
            (name, _) => account_name(name)?,
        };
        lock(&self.state).free_name(&name, guest.map(|g| g.user.as_str()))?;
        limited(&self.limits.register, ip, "accounts registered from here")?;
        let hash = hash_password(&credentials.password)?;
        let mut state = lock(&self.state);
        state.free_name(&name, guest.map(|g| g.user.as_str()))?;
        if let Some(guest) = guest
            && let Some(mut user) = state.users.get(&guest.user).cloned()
        {
            state.names.remove(&user.name.to_lowercase());
            user.name = name;
            user.login = Login::Password(hash);
            state
                .names
                .insert(user.name.to_lowercase(), user.id.clone());
            state.users.insert(user.id.clone(), user.clone());
            self.save_user(&user);
            let me = state.me(&user);
            drop(state);
            self.bump();
            return Ok((me, None));
        }
        let user = state.add_user(name, Login::Password(hash));
        self.save_user(&user);
        let token = self.start_session(&mut state, &user.id);
        Ok((state.me(&user), Some(token)))
    }

    /// A session for the account `credentials` name. This verifies a password: call it
    /// off the async threads.
    pub fn login(&self, ip: IpAddr, credentials: &Credentials) -> Result<(Me, String)> {
        limited(&self.limits.login, ip, "sign-in attempts from here")?;
        let name = credentials.name.trim().to_lowercase();
        limited(
            &self.limits.login_name,
            name,
            "sign-in attempts for that name",
        )?;
        let found = {
            let state = lock(&self.state);
            state
                .by_name(&credentials.name)
                .and_then(|u| match &u.login {
                    Login::Password(hash) => Some((u.id.clone(), hash.clone())),
                    _ => None,
                })
        };
        // The same work whether or not the name is an account's.
        let hash = found.as_ref().map_or(DUMMY_HASH, |(_, h)| h.as_str());
        let right = verify_password(&credentials.password, hash);
        let Some((id, _)) = found.filter(|_| right) else {
            return Err(Error::Unauthorized("wrong name or password".into()));
        };
        let mut state = lock(&self.state);
        let token = self.start_session(&mut state, &id);
        Ok((state.me(state.user(&id)?), token))
    }

    /// A guest account and its session.
    pub fn guest(&self, ip: IpAddr, name: Option<&str>) -> Result<(Me, String)> {
        if !self.settings.guests {
            return Err(Error::Forbidden(
                "this server needs an account to play".into(),
            ));
        }
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => Some(account_name(name)?),
            None => None,
        };
        let mut state = lock(&self.state);
        if let Some(name) = &name {
            state.free_name(name, None)?;
        }
        limited(&self.limits.guest, ip, "guests from here")?;
        let name = name.unwrap_or_else(|| {
            let mut rng = rand::rng();
            loop {
                let name = format!("Guest {}", rng.random_range(1000..100_000));
                if !state.names.contains_key(&name.to_lowercase()) {
                    return name;
                }
            }
        });
        let user = state.add_user(name, Login::Guest);
        self.save_user(&user);
        let token = self.start_session(&mut state, &user.id);
        Ok((state.me(&user), token))
    }

    /// Ends the session `token` holds. A guest's account goes with its last session once
    /// its games are over ([`Arena::sweep`]).
    pub fn logout(&self, token: &str) {
        let mut state = lock(&self.state);
        self.end_session(&mut state, &sha256(token));
    }

    /// Replaces a password, which ends the account's other sessions. Hashes: call it off
    /// the async threads.
    pub fn set_password(&self, ident: &Ident, token: &str, current: &str, new: &str) -> Result<()> {
        check_password(new)?;
        let hash = match &lock(&self.state).user(&ident.user)?.login {
            Login::Password(hash) => hash.clone(),
            _ => return bad("this account has no password"),
        };
        if !verify_password(current, &hash) {
            return Err(Error::Unauthorized("wrong password".into()));
        }
        let hash = hash_password(new)?;
        let mut state = lock(&self.state);
        let mut user = state.user(&ident.user)?.clone();
        user.login = Login::Password(hash);
        state.users.insert(user.id.clone(), user.clone());
        self.save_user(&user);
        let keep = sha256(token);
        let others: Vec<String> = (state.sessions.iter())
            .filter(|(key, s)| s.user == user.id && **key != keep)
            .map(|(key, _)| key.clone())
            .collect();
        for key in others {
            self.end_session(&mut state, &key);
        }
        Ok(())
    }

    fn owner(&self, state: &State, ident: &Ident) -> Result<String> {
        match state.user(&ident.user)?.kind() {
            UserKind::Registered => Ok(ident.user.clone()),
            _ => Err(Error::Forbidden(
                "bots belong to registered accounts".into(),
            )),
        }
    }

    /// A bot of `ident`'s: an account an agent signs in to with the API key returned,
    /// which is shown this once.
    pub fn new_bot(&self, ident: &Ident, name: &str) -> Result<BotKey> {
        let name = account_name(name)?;
        let mut state = lock(&self.state);
        let owner = self.owner(&state, ident)?;
        state.free_name(&name, None)?;
        if state
            .users
            .values()
            .filter(|u| u.owner() == Some(&owner))
            .count()
            >= MAX_BOTS
        {
            return conflict(format!("an account has at most {MAX_BOTS} bots"));
        }
        let key = format!("{KEY_PREFIX}{}", secret(20));
        let login = Login::Bot {
            owner,
            key: sha256(&key),
        };
        let user = state.add_user(name, login);
        self.save_user(&user);
        let bot = state.user_ref(&user);
        Ok(BotKey { bot, key })
    }

    fn bot_of(&self, state: &State, ident: &Ident, name: &str) -> Result<User> {
        let owner = self.owner(state, ident)?;
        (state.by_name(name))
            .filter(|b| b.owner() == Some(&owner))
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("you have no bot named {name}")))
    }

    /// A new API key for a bot; the old one stops working.
    pub fn new_bot_key(&self, ident: &Ident, name: &str) -> Result<BotKey> {
        let mut state = lock(&self.state);
        let mut bot = self.bot_of(&state, ident, name)?;
        let key = format!("{KEY_PREFIX}{}", secret(20));
        if let Login::Bot { key: old, .. } = &mut bot.login {
            state.keys.remove(old);
            *old = sha256(&key);
            state.keys.insert(old.clone(), bot.id.clone());
        }
        state.users.insert(bot.id.clone(), bot.clone());
        self.save_user(&bot);
        Ok(BotKey {
            bot: state.user_ref(&bot),
            key,
        })
    }

    /// Deletes a bot that isn't in a game.
    pub fn delete_bot(&self, ident: &Ident, name: &str) -> Result<()> {
        let bot = self.bot_of(&lock(&self.state), ident, name)?;
        if !self
            .hub
            .seats_of(&bot.id, std::time::Duration::ZERO)
            .is_empty()
        {
            return conflict(format!("{} is in a game", bot.name));
        }
        let mut state = lock(&self.state);
        state.leave_all(&bot.id);
        state.remove_user(&bot.id);
        if let Some(store) = &self.store {
            let _ = store.remove(Some(USERS), &bot.id);
        }
        drop(state);
        self.bump();
        Ok(())
    }

    /// The Banned cards list that applies to `ident`: their own, else the server's.
    pub fn banned_cards(&self, ident: Option<&Ident>) -> Vec<String> {
        let state = lock(&self.state);
        ident
            .and_then(|i| state.users.get(&i.user)?.banned_cards.clone())
            .unwrap_or_else(|| self.settings.banned_cards.clone())
    }

    /// Replaces `ident`'s Banned cards list. The list as kept.
    pub fn set_banned_cards(&self, ident: &Ident, names: &[String]) -> Result<Vec<String>> {
        let mut cards: Vec<String> = vec![];
        for name in names {
            let card = Card::from_name(name)
                .filter(|c| c.def().kingdom || c.kind().is_landscape())
                .ok_or_else(|| {
                    Error::BadRequest(format!("{name:?} is not a Kingdom pile or landscape"))
                })?;
            if !cards.iter().any(|c| c == card.name()) {
                cards.push(card.name().to_string());
            }
        }
        let mut state = lock(&self.state);
        let mut user = state.user(&ident.user)?.clone();
        user.banned_cards = Some(cards.clone());
        state.users.insert(user.id.clone(), user.clone());
        self.save_user(&user);
        Ok(cards)
    }

    /// Ends the sessions nobody has used for too long, and deletes the guests left
    /// without one whose games are over.
    pub(super) fn expire_sessions(&self) {
        let now = now();
        let orphans: Vec<String> = {
            let mut state = lock(&self.state);
            let old: Vec<String> = (state.sessions.iter())
                .filter(|(_, s)| {
                    let guest =
                        (state.users.get(&s.user)).is_none_or(|u| u.kind() == UserKind::Guest);
                    let days = if guest { GUEST_DAYS } else { SESSION_DAYS };
                    now.saturating_sub(s.seen) >= days * DAY
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in old {
                self.end_session(&mut state, &key);
            }
            let state = &*state;
            (state.users.values())
                .filter(|u| u.kind() == UserKind::Guest && !state.here(&u.id))
                .filter(|u| !state.sessions.values().any(|s| s.user == u.id))
                .map(|u| u.id.clone())
                .collect()
        };
        for id in orphans {
            if !self.hub.seats_of(&id, std::time::Duration::ZERO).is_empty() {
                continue;
            }
            let mut state = lock(&self.state);
            state.leave_all(&id);
            state.remove_user(&id);
            if let Some(store) = &self.store {
                let _ = store.remove(Some(USERS), &id);
            }
        }
    }
}

/// An argon2 hash of nothing anyone knows, verified when a sign-in names no account.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$\
                          bWFkZSB1cCwgbWF0Y2hlcyBubyBwYXNzd29yZCBhdCBhbGw";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_plain_and_not_a_seat_number() {
        assert_eq!(account_name("  Ada L. ").unwrap(), "Ada L.");
        for bad in [
            "a",
            "P2",
            "p10",
            "Ada\nBo",
            "-ada",
            "a  b",
            "Åsa",
            "you",
            &"x".repeat(21),
        ] {
            assert!(account_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_password_verifies_against_its_hash_only() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("battery staple", &hash));
        assert!(!verify_password("anything", DUMMY_HASH));
        assert!(PasswordHash::new(DUMMY_HASH).is_ok());
    }
}
