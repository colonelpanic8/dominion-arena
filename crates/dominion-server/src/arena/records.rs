//! Finished games: each lobby game's record, its players' ratings and tallies.

use super::db::{self, Db, Tally};
use super::history::{Ask, GameSource};
use super::{Arena, Error, Result, User};
use crate::hub::{Ended, Hub, lock, now_ms};
use crate::rating::{self, Scale};
use dominion_protocol::lobby::{
    Arranged, GameRecord, History, HistoryQuery, Leader, Leaderboard, PastGame, PastPlayer,
    Profile, RecordPlayer, Source, Venue,
};
use std::sync::Arc;

/// The most rating deviation an anchor may have here to count, and the least spread of
/// the anchors' ratings: closer than that, they can't tell a slope.
const ANCHOR_DEVIATION: f64 = 100.0;

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
        let Ended {
            id,
            meta,
            info,
            setup,
        } = ended;
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
                    user: (accounts[i].as_ref())
                        .map(|u| u.name.clone())
                        .filter(|n| *n != p.name),
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
                others: setup,
                players,
            };
            let users: Vec<Option<String>> = (accounts.iter())
                .map(|u| u.as_ref().map(|u| u.id.clone()))
                .collect();
            db::add_record(tx, Venue::Arena, &record, &users)
        })
    }

    /// The line from ratings here to dominion.games skill, through the anchors whose
    /// rating here is settled.
    pub(super) fn skill_scale(&self) -> Option<Scale> {
        let now = now_ms() / 1000;
        let points: Vec<(f64, f64)> = (self.settings.skill_anchors.iter())
            .filter_map(|a| {
                let user = self.db.user_by_name(&a.player).ok().flatten()?;
                let settled =
                    user.rated_games > 0 && user.rating.deviation_at(now) <= ANCHOR_DEVIATION;
                settled.then_some((user.rating.rating, a.skill))
            })
            .collect();
        Scale::fit(&points, ANCHOR_DEVIATION)
    }

    fn named(&self, name: &str) -> Result<User> {
        (self.db.user_by_name(name)?)
            .ok_or_else(|| Error::NotFound(format!("nobody is called {name}")))
    }

    /// A player's page.
    pub fn profile(&self, name: &str) -> Result<Profile> {
        let user = self.named(name)?;
        let ask = Ask {
            user: Some(&user.id),
            ..Ask::default()
        };
        Ok(Profile {
            user: self.user_ref(&user),
            rating: user.rating_info(self.skill_scale().as_ref()),
            stats: user.stats(),
            official_games: self.history.count(&user.id, Venue::Official)?,
            recent: self.history.find(&ask, u64::MAX, 20)?.games,
        })
    }

    /// Finished games, newest first, as `q` asks.
    pub fn history(&self, q: &HistoryQuery) -> Result<History> {
        let user = q.user.as_deref().map(|name| self.named(name)).transpose()?;
        let ask = Ask {
            user: user.as_ref().map(|u| u.id.as_str()),
            venue: q.venue,
            rated: q.rated,
            text: q.q.as_deref(),
        };
        let before = q.before.unwrap_or(u64::MAX);
        self.history
            .find(&ask, before, q.limit.unwrap_or(20).clamp(1, 100))
    }

    /// The rated players, best first.
    pub fn leaderboard(&self, limit: usize) -> Result<Leaderboard> {
        let leaders = self.db.leaders(limit.clamp(1, 200))?;
        let scale = self.skill_scale();
        Ok(Leaderboard {
            players: (leaders.iter().enumerate())
                .map(|(i, u)| Leader {
                    rank: i as u32 + 1,
                    user: self.user_ref(u),
                    rating: u.rating_info(scale.as_ref()),
                    stats: u.stats(),
                })
                .collect(),
        })
    }
}

/// A stored record as the history shows it.
pub(super) fn past_game(record: GameRecord, venue: Venue) -> PastGame {
    let official = venue == Venue::Official;
    PastGame {
        venue,
        arranged: match record.source {
            Source::Automatch => Some(Arranged::Automatch),
            Source::Table => Some(Arranged::Table),
            Source::Direct => Some(Arranged::Tokens),
            Source::Official => None,
        },
        official_id: None,
        open: false,
        started: record.started,
        ended: record.ended,
        rated: record.rated,
        reason: record.reason,
        kingdom: record.kingdom,
        landscapes: record.landscapes,
        others: record.others,
        players: (record.players.into_iter())
            .map(|p| PastPlayer {
                // An Arena seat's name is its account's; an official one's is the official
                // account's, whatever played it here.
                account: p
                    .user
                    .or((p.kind.is_some() && !official).then(|| p.name.clone())),
                seat: p.seat,
                name: p.name,
                kind: p.kind,
                vp: p.vp,
                rank: p.rank,
                turns: p.turns,
                resigned: p.resigned,
                rating: p.rating,
            })
            .collect(),
        id: record.game_id,
    }
}

/// The records of Arena's own games, written when each ends.
pub(super) struct ArenaRecords {
    pub db: Arc<Db>,
    pub hub: Arc<Hub>,
}

impl GameSource for ArenaRecords {
    fn venue(&self) -> Venue {
        Venue::Arena
    }

    fn page(&self, user: Option<&str>, before: u64, limit: usize) -> Result<Vec<PastGame>> {
        let records = self.db.games(Venue::Arena, user, before, limit)?;
        let games = records.into_iter().map(|r| {
            let mut game = past_game(r, Venue::Arena);
            game.open = self.hub.meta(&game.id).is_ok();
            game
        });
        Ok(games.collect())
    }

    fn count(&self, user: &str) -> Result<u32> {
        self.db.count_games(Venue::Arena, user)
    }
}
