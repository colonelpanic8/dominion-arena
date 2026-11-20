//! Invite codes: the server's `admins` make them, and each lets a number of people
//! register, even where the server takes no new accounts otherwise.

use super::db::InviteRow;
use super::{Arena, Error, Ident, Result, User, bad};
use crate::hub::now_ms;
use dominion_protocol::lobby::{Invite, NewInvite, UserKind};
use rand::Rng;

const DAY: u64 = 86_400;
const DEFAULT_DAYS: u32 = 30;
const MAX_USES: u32 = 100;
const MAX_DAYS: u32 = 365;
const MAX_NOTE: usize = 100;
/// No 0/O or 1/I/L, which read alike.
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const GROUPS: usize = 3;
const GROUP: usize = 4;

fn code() -> String {
    let mut rng = rand::rng();
    let groups: Vec<String> = (0..GROUPS)
        .map(|_| {
            (0..GROUP)
                .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
                .collect()
        })
        .collect();
    groups.join("-")
}

/// A code as someone typed it, in the form it is kept: letters upper case, grouped by
/// dashes, whatever spaces or dashes were typed.
pub(super) fn normalize(typed: &str) -> String {
    let chars: Vec<char> = (typed.chars())
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let groups: Vec<String> = chars.chunks(GROUP).map(|g| g.iter().collect()).collect();
    groups.join("-")
}

impl Arena {
    pub(super) fn is_admin(&self, user: &User) -> bool {
        user.kind() == UserKind::Registered
            && (self.settings.admins.iter()).any(|a| a.trim().eq_ignore_ascii_case(&user.name))
    }

    fn admin(&self, ident: &Ident) -> Result<User> {
        let user = self.user(&ident.user)?;
        match !ident.agent && self.is_admin(&user) {
            true => Ok(user),
            false => Err(Error::Forbidden(
                "only the server's admins manage invites".into(),
            )),
        }
    }

    fn invite_of(&self, row: InviteRow) -> Invite {
        let name = |id: &str| self.db.user_ref(id).map(|u| u.name);
        Invite {
            by: row.creator.as_deref().and_then(name),
            used_by: row.users.iter().filter_map(|u| name(u)).collect(),
            code: row.code,
            created: row.created,
            expires: row.expires,
            uses: row.uses,
            note: row.note,
        }
    }

    /// Every invite code, newest first, for an admin.
    pub fn invites(&self, ident: &Ident) -> Result<Vec<Invite>> {
        self.admin(ident)?;
        let rows = self.db.invites()?;
        Ok(rows.into_iter().map(|r| self.invite_of(r)).collect())
    }

    /// A new invite code of an admin's.
    pub fn new_invite(&self, ident: &Ident, new: &NewInvite) -> Result<Invite> {
        let admin = self.admin(ident)?;
        let uses = new.uses.unwrap_or(1);
        if !(1..=MAX_USES).contains(&uses) {
            return bad(format!("an invite has 1 to {MAX_USES} uses"));
        }
        let days = new.days.unwrap_or(DEFAULT_DAYS);
        if days > MAX_DAYS {
            return bad(format!(
                "an invite lasts at most {MAX_DAYS} days (0: no limit)"
            ));
        }
        let note = (new.note.as_deref().map(str::trim)).filter(|n| !n.is_empty());
        if note.is_some_and(|n| n.chars().count() > MAX_NOTE) {
            return bad(format!("a note has at most {MAX_NOTE} characters"));
        }
        let created = now_ms() / 1000;
        let row = InviteRow {
            code: code(),
            creator: Some(admin.id),
            created,
            expires: (days > 0).then(|| created + u64::from(days) * DAY),
            uses,
            users: vec![],
            note: note.map(String::from),
        };
        self.db.add_invite(&row)?;
        Ok(self.invite_of(row))
    }

    /// Deletes an invite code; the accounts made with it stay.
    pub fn revoke_invite(&self, ident: &Ident, code: &str) -> Result<()> {
        self.admin(ident)?;
        match self.db.delete_invite(&normalize(code))? {
            true => Ok(()),
            false => Err(Error::NotFound("no such invite code".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_read_back_however_they_are_typed() {
        let made = code();
        assert_eq!(made.len(), GROUPS * GROUP + GROUPS - 1);
        assert_eq!(normalize(&made), made);
        assert_eq!(normalize(" abcd efgh-JKMN "), "ABCD-EFGH-JKMN");
    }
}
