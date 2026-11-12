//! Finished games: each lobby game's record, its players' ratings and tallies.

use super::{Arena, Error, HISTORY, Result, State, Stored};
use crate::hub::{Ended, lock, now_ms};
use crate::rating;
use dominion_protocol::lobby::{GameRecord, History, Leader, Leaderboard, Profile, RecordPlayer};

impl State {
    fn games_of(&self, user: &str, before: u64, limit: usize) -> Vec<GameRecord> {
        (self.history.iter().rev())
            .filter(|s| s.record.ended < before)
            .filter(|s| s.users.iter().any(|u| u.as_deref() == Some(user)))
            .take(limit)
            .map(|s| s.record.clone())
            .collect()
    }
}

impl Arena {
    /// Writes an ended game's record and counts it for its players. A game played only
    /// by seat tokens leaves none.
    pub(super) fn record(&self, ended: Ended) {
        let Ended { id, meta, info } = ended;
        let Some(result) = info.result else {
            return;
        };
        if meta.recorded || meta.owners.iter().all(Option::is_none) {
            return;
        }
        let now = meta.ended_at.unwrap_or_else(now_ms);
        let mut state = lock(&self.state);
        if let Some(table) = &meta.table {
            self.table_game_over(&mut state, table, &id);
        }
        if state.history.iter().any(|s| s.record.game_id == id) {
            drop(state);
            self.hub.set_recorded(&id);
            return;
        }
        let owner = |seat: u8| meta.owners.get(usize::from(seat)).cloned().flatten();
        let users: Vec<Option<String>> = (result.players.iter())
            .map(|p| owner(p.index).filter(|u| state.users.contains_key(u)))
            .collect();
        let rated = meta.rated && users.iter().all(Option::is_some);
        let before: Vec<rating::Rating> = (users.iter())
            .map(|u| {
                let user = u.as_ref().and_then(|u| state.users.get(u));
                user.map(|u| u.rating).unwrap_or_default()
            })
            .collect();
        let ranks: Vec<u32> = result.players.iter().map(|p| p.rank).collect();
        let after = rating::after_game(&before, &ranks, now / 1000);
        let winners = ranks.iter().filter(|&&r| r == 1).count();
        let mut players = vec![];
        for (i, p) in result.players.iter().enumerate() {
            let user = users[i].as_ref().and_then(|u| state.users.get_mut(u));
            let kind = user.as_ref().map(|u| u.kind());
            if let Some(user) = user {
                user.games += 1;
                // Everyone tied for first has no win to show for it.
                if p.rank == 1 && winners < ranks.len() {
                    user.wins += 1;
                }
                if rated {
                    user.rating = after[i];
                    user.rated_games += 1;
                }
                let user = user.clone();
                self.save_user(&user);
            }
            players.push(RecordPlayer {
                seat: p.index,
                name: p.name.clone(),
                kind,
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
            reason: (meta.ended.clone()).or(result.reason).unwrap_or_default(),
            kingdom: info.kingdom,
            landscapes: info.landscapes,
            players,
        };
        let stored = Stored { record, users };
        if let Some(store) = &self.store
            && let Err(e) = store.save(Some(HISTORY), &id, &stored)
        {
            eprintln!("dominion: saving the record of game {id}: {e}");
        }
        state.history.push(stored);
        drop(state);
        self.hub.set_recorded(&id);
        self.bump();
    }

    /// A player's page.
    pub fn profile(&self, name: &str) -> Result<Profile> {
        let state = lock(&self.state);
        let user = (state.by_name(name))
            .ok_or_else(|| Error::NotFound(format!("nobody is called {name}")))?;
        Ok(Profile {
            user: state.user_ref(user),
            rating: user.rating_info(),
            stats: user.stats(),
            recent: state.games_of(&user.id, u64::MAX, 20),
        })
    }

    /// Finished games that ended before `before`, newest first: `user`'s, or everyone's.
    pub fn history(
        &self,
        user: Option<&str>,
        before: Option<u64>,
        limit: usize,
    ) -> Result<History> {
        let state = lock(&self.state);
        let before = before.unwrap_or(u64::MAX);
        let limit = limit.clamp(1, 100);
        let games = match user {
            Some(name) => {
                let user = (state.by_name(name))
                    .ok_or_else(|| Error::NotFound(format!("nobody is called {name}")))?;
                state.games_of(&user.id, before, limit)
            }
            None => (state.history.iter().rev())
                .filter(|s| s.record.ended < before)
                .take(limit)
                .map(|s| s.record.clone())
                .collect(),
        };
        Ok(History { games })
    }

    /// The rated players, best first.
    pub fn leaderboard(&self, limit: usize) -> Leaderboard {
        let state = lock(&self.state);
        let mut rated: Vec<&super::User> = (state.users.values())
            .filter(|u| u.rated_games > 0)
            .collect();
        rated.sort_by(|a, b| b.rating.rating.total_cmp(&a.rating.rating));
        Leaderboard {
            players: (rated.iter().take(limit.clamp(1, 200)).enumerate())
                .map(|(i, u)| Leader {
                    rank: i as u32 + 1,
                    user: state.user_ref(u),
                    rating: u.rating_info(),
                    stats: u.stats(),
                })
                .collect(),
        }
    }
}
