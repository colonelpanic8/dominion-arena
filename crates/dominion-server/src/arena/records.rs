//! Finished games: each lobby game's record, its players' ratings and tallies.

use super::db::{self, Tally};
use super::{Arena, Error, Result, User};
use crate::hub::{Ended, lock, now_ms};
use crate::rating;
use dominion_protocol::lobby::{GameRecord, History, Leader, Leaderboard, Profile, RecordPlayer};

impl Arena {
    /// Writes an ended game's record and counts it for its players, all or nothing. A
    /// game played only by seat tokens leaves none.
    pub(super) fn record(&self, ended: Ended) {
        let id = ended.id.clone();
        if let Some(table) = &ended.meta.table {
            self.table_game_over(&mut lock(&self.state), table, &id);
        }
        self.dispatch();
        if ended.meta.recorded || ended.meta.owners.iter().all(Option::is_none) {
            return;
        }
        match self.write_record(ended) {
            Ok(()) => self.hub.set_recorded(&id),
            Err(e) => eprintln!("dominion: recording game {id}: {e}"),
        }
        self.bump();
    }

    fn write_record(&self, ended: Ended) -> Result<()> {
        let Ended { id, meta, info } = ended;
        let Some(result) = info.result else {
            return Ok(());
        };
        let now = meta.ended_at.unwrap_or_else(now_ms);
        let owner = |seat: u8| meta.owners.get(usize::from(seat)).cloned().flatten();
        self.db.transaction(|tx| {
            if db::recorded(tx, &id)? {
                return Ok(());
            }
            let mut accounts: Vec<Option<User>> = vec![];
            for p in &result.players {
                accounts.push(match owner(p.index) {
                    Some(user) => db::user_in(tx, &user)?,
                    None => None,
                });
            }
            let rated = meta.rated && accounts.iter().all(Option::is_some);
            let before: Vec<rating::Rating> = (accounts.iter())
                .map(|u| u.as_ref().map(|u| u.rating).unwrap_or_default())
                .collect();
            let ranks: Vec<u32> = result.players.iter().map(|p| p.rank).collect();
            let after = rating::after_game(&before, &ranks, now / 1000);
            // Everyone tied for first has no win to show for it.
            let won = ranks.iter().filter(|&&r| r == 1).count() < ranks.len();
            let mut players = vec![];
            for (i, p) in result.players.iter().enumerate() {
                if let Some(user) = &accounts[i] {
                    let tally = Tally {
                        user: &user.id,
                        win: p.rank == 1 && won,
                        rating: rated.then_some(after[i]),
                    };
                    db::tally(tx, &tally)?;
                }
                players.push(RecordPlayer {
                    seat: p.index,
                    name: p.name.clone(),
                    kind: accounts[i].as_ref().map(|u| u.kind()),
                    vp: p.vp,
                    rank: p.rank,
                    turns: p.turns,
                    resigned: p.resigned,
                    rating: rated.then(|| (before[i].rating, after[i].rating)),
                });
            }
            let record = GameRecord {
                game_id: id.clone(),
                started: meta.started,
                ended: now,
                source: meta.source,
                rated,
                reason: (meta.ended.clone())
                    .or(result.reason.clone())
                    .unwrap_or_default(),
                kingdom: info.kingdom.clone(),
                landscapes: info.landscapes.clone(),
                players,
            };
            let users: Vec<Option<String>> = (accounts.iter())
                .map(|u| u.as_ref().map(|u| u.id.clone()))
                .collect();
            db::add_record(tx, &record, &users)
        })
    }

    fn named(&self, name: &str) -> Result<User> {
        (self.db.user_by_name(name)?)
            .ok_or_else(|| Error::NotFound(format!("nobody is called {name}")))
    }

    /// A player's page.
    pub fn profile(&self, name: &str) -> Result<Profile> {
        let user = self.named(name)?;
        Ok(Profile {
            user: self.user_ref(&user),
            rating: user.rating_info(),
            stats: user.stats(),
            recent: self.db.games(Some(&user.id), u64::MAX, 20)?,
        })
    }

    /// Finished games that ended before `before`, newest first: `user`'s, or everyone's.
    pub fn history(
        &self,
        user: Option<&str>,
        before: Option<u64>,
        limit: usize,
    ) -> Result<History> {
        let user = user.map(|name| self.named(name)).transpose()?;
        let user = user.as_ref().map(|u| u.id.as_str());
        let before = before.unwrap_or(u64::MAX);
        Ok(History {
            games: self.db.games(user, before, limit.clamp(1, 100))?,
        })
    }

    /// The rated players, best first.
    pub fn leaderboard(&self, limit: usize) -> Result<Leaderboard> {
        let leaders = self.db.leaders(limit.clamp(1, 200))?;
        Ok(Leaderboard {
            players: (leaders.iter().enumerate())
                .map(|(i, u)| Leader {
                    rank: i as u32 + 1,
                    user: self.user_ref(u),
                    rating: u.rating_info(),
                    stats: u.stats(),
                })
                .collect(),
        })
    }
}
