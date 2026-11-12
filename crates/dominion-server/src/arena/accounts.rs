//! Accounts, how they sign in, and what they keep: sessions, bots, the Banned list.

use super::{Arena, Error, Ident, Login, Result, User, bad, conflict, limited};
use crate::hub::{lock, now_ms};
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

impl Arena {
    fn taken(&self, name: &str, but: Option<&str>) -> Result<()> {
        match self.db.name_free(name, but)? {
            true => Ok(()),
            false => conflict(format!("the name {name} is taken")),
        }
    }

    fn start_session(&self, user: &str) -> Result<String> {
        let token = secret(32);
        self.db.add_session(&sha256(&token), user, now())?;
        Ok(token)
    }

    /// Who `token` is: a session's token or a bot's API key.
    pub fn identify(&self, token: &str) -> Option<Ident> {
        let key = sha256(token);
        if token.starts_with(KEY_PREFIX) {
            let user = self.db.bot_by_key(&key).ok()??;
            return Some(Ident { user, agent: true });
        }
        let (user, kind) = self.db.session(&key, now()).ok()??;
        Some(Ident {
            user,
            agent: kind == UserKind::Bot,
        })
    }

    pub fn me(&self, ident: &Ident) -> Result<Me> {
        self.me_of(&self.user(&ident.user)?)
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
        let guest = current
            .and_then(|i| self.db.user(&i.user).ok().flatten())
            .filter(|u| u.kind() == UserKind::Guest);
        let name = match (credentials.name.trim(), &guest) {
            ("", Some(guest)) => guest.name.clone(),
            (name, _) => account_name(name)?,
        };
        self.taken(&name, guest.as_ref().map(|g| g.id.as_str()))?;
        limited(&self.limits.register, ip, "accounts registered from here")?;
        let hash = hash_password(&credentials.password)?;
        if let Some(guest) = guest {
            // The name is theirs unless someone took it meanwhile: then this fails.
            self.db.make_registered(&guest.id, &name, &hash)?;
            let me = self.me_of(&self.user(&guest.id)?)?;
            self.bump();
            return Ok((me, None));
        }
        let user = self.db.add_user(&name, &Login::Password(hash), now())?;
        let token = self.start_session(&user.id)?;
        Ok((self.me_of(&user)?, Some(token)))
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
        let found = self.db.user_by_name(&credentials.name)?;
        let hash = found.as_ref().and_then(|u| match &u.login {
            Login::Password(hash) => Some(hash.clone()),
            _ => None,
        });
        // The same work whether or not the name is an account's.
        let known = hash.as_deref().unwrap_or(DUMMY_HASH);
        let right = verify_password(&credentials.password, known) && hash.is_some();
        let Some(user) = found.filter(|_| right) else {
            return Err(Error::Unauthorized("wrong name or password".into()));
        };
        let token = self.start_session(&user.id)?;
        Ok((self.me_of(&user)?, token))
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
        if let Some(name) = &name {
            self.taken(name, None)?;
        }
        limited(&self.limits.guest, ip, "guests from here")?;
        let user = match name {
            Some(name) => self.db.add_user(&name, &Login::Guest, now())?,
            None => loop {
                let name = format!("Guest {}", rand::rng().random_range(1000..100_000));
                match self.db.add_user(&name, &Login::Guest, now()) {
                    Err(Error::Conflict(_)) => continue,
                    made => break made?,
                }
            },
        };
        let token = self.start_session(&user.id)?;
        Ok((self.me_of(&user)?, token))
    }

    /// Ends the session `token` holds. A guest's account goes with its last session once
    /// its games are over ([`Arena::sweep`]).
    pub fn logout(&self, token: &str) {
        let _ = self.db.end_session(&sha256(token));
    }

    /// Replaces a password, which ends the account's other sessions. Hashes: call it off
    /// the async threads.
    pub fn set_password(&self, ident: &Ident, token: &str, current: &str, new: &str) -> Result<()> {
        check_password(new)?;
        let hash = match self.user(&ident.user)?.login {
            Login::Password(hash) => hash,
            _ => return bad("this account has no password"),
        };
        if !verify_password(current, &hash) {
            return Err(Error::Unauthorized("wrong password".into()));
        }
        self.db.set_password(&ident.user, &hash_password(new)?)?;
        self.db.end_other_sessions(&ident.user, &sha256(token))
    }

    /// The id of `ident`'s account, which may own bots.
    fn owner(&self, ident: &Ident) -> Result<String> {
        match self.user(&ident.user)?.kind() {
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
        let owner = self.owner(ident)?;
        self.taken(&name, None)?;
        if self.db.bots_of(&owner)?.len() >= MAX_BOTS {
            return conflict(format!("an account has at most {MAX_BOTS} bots"));
        }
        let key = format!("{KEY_PREFIX}{}", secret(20));
        let login = Login::Bot {
            owner,
            key: sha256(&key),
        };
        let bot = self.db.add_user(&name, &login, now())?;
        Ok(BotKey {
            bot: self.user_ref(&bot),
            key,
        })
    }

    fn bot_of(&self, ident: &Ident, name: &str) -> Result<User> {
        let owner = self.owner(ident)?;
        (self.db.user_by_name(name)?)
            .filter(|b| b.owner() == Some(&owner))
            .ok_or_else(|| Error::NotFound(format!("you have no bot named {name}")))
    }

    /// A new API key for a bot; the old one stops working.
    pub fn new_bot_key(&self, ident: &Ident, name: &str) -> Result<BotKey> {
        let bot = self.bot_of(ident, name)?;
        let key = format!("{KEY_PREFIX}{}", secret(20));
        self.db.set_api_key(&bot.id, &sha256(&key))?;
        Ok(BotKey {
            bot: self.user_ref(&bot),
            key,
        })
    }

    /// Deletes an account that is in no game, taking it out of the lobby.
    fn delete_account(&self, id: &str) -> Result<()> {
        let mut state = lock(&self.state);
        state.leave_all(&self.db, id);
        state.presence.remove(id);
        drop(state);
        self.db.delete_user(id)
    }

    /// Deletes a bot that isn't in a game.
    pub fn delete_bot(&self, ident: &Ident, name: &str) -> Result<()> {
        let bot = self.bot_of(ident, name)?;
        if !self
            .hub
            .seats_of(&bot.id, std::time::Duration::ZERO)
            .is_empty()
        {
            return conflict(format!("{} is in a game", bot.name));
        }
        self.delete_account(&bot.id)?;
        self.bump();
        Ok(())
    }

    /// The Banned cards list that applies to `ident`: their own, else the server's.
    pub fn banned_cards(&self, ident: Option<&Ident>) -> Vec<String> {
        let user = ident.and_then(|i| self.db.user(&i.user).ok().flatten());
        user.and_then(|u| u.banned_cards)
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
        self.user(&ident.user)?;
        self.db.set_banned_cards(&ident.user, &cards)?;
        Ok(cards)
    }

    /// Ends the sessions nobody has used for too long, and deletes the guests left
    /// without one whose games are over.
    pub(super) fn expire_sessions(&self) {
        let now = now();
        let before = |days: u64| now.saturating_sub(days * DAY);
        let orphans = match self
            .db
            .expire_sessions(before(SESSION_DAYS), before(GUEST_DAYS))
        {
            Ok(orphans) => orphans,
            Err(e) => return eprintln!("dominion: expiring sessions: {e}"),
        };
        for id in orphans {
            let around = lock(&self.state).here(&id)
                || !self.hub.seats_of(&id, std::time::Duration::ZERO).is_empty();
            if !around && let Err(e) = self.delete_account(&id) {
                eprintln!("dominion: deleting guest {id}: {e}");
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
