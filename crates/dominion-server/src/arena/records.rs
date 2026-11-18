//! Finished games: each lobby game's record, its players' ratings and tallies.

use super::db::{self, Db, Tally};
use super::history::{Ask, GameSource};
use super::house::Setups;
use super::{Arena, Error, Login, Result, User};
use crate::hub::{Ended, Hub, lock, now_ms};
use crate::rating::{self, Scale};
use dominion_protocol::lobby::{
    AgentRun, AgentSetup, Arranged, GameRecord, History, HistoryQuery, Leader, Leaderboard,
    PastGame, PastPlayer, PlayedBy, Profile, RecordPlayer, SeatAgent, Source, UserKind, Venue,
};
use rusqlite::Transaction;
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
            seed,
            info,
            setup,
        } = ended;
        let Some(result) = info.result else {
            return Ok(());
        };
        let now = meta.ended_at.unwrap_or_else(now_ms);
        let owner = |seat: u8| meta.owners.get(usize::from(seat)).cloned().flatten();
        let setups = lock(&self.state).house.setups();
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
                let reported = meta.runs.get(usize::from(p.index)).cloned().flatten();
                let (agent, bot_owner) = match &accounts[i] {
                    Some(user) => who_played(tx, user, &setups, reported)?,
                    None => (None, None),
                };
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
                    level: None,
                    agent,
                    owner: bot_owner,
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
                seed: Some(seed),
                version: crate::commit(),
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
        let mut recent = self.history.find(&ask, u64::MAX, 20)?.games;
        complete_agents(&mut recent, &lock(&self.state).house.setups());
        let agent = matches!(user.login, Login::House).then(|| {
            let setups = lock(&self.state).house.setups();
            let latest = recent.iter().find_map(|g| {
                let seat = g
                    .players
                    .iter()
                    .find(|p| p.account.as_deref() == Some(&user.name));
                match &seat?.played_by {
                    PlayedBy::Agent(a) if a.model.is_some() => Some(a.clone()),
                    _ => None,
                }
            });
            (setups.get(&user.name.to_lowercase()).map(SeatAgent::from))
                .or(latest)
                .unwrap_or_else(|| SeatAgent {
                    agent: user.name.clone(),
                    ..SeatAgent::default()
                })
        });
        Ok(Profile {
            user: self.user_ref(&user),
            rating: user.rating_info(self.skill_scale().as_ref()),
            stats: user.stats(),
            official_games: self.history.count(&user.id, Venue::Official)?,
            agent,
            recent,
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
        let mut found = (self.history).find(&ask, before, q.limit.unwrap_or(20).clamp(1, 100))?;
        complete_agents(&mut found.games, &lock(&self.state).house.setups());
        Ok(found)
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

/// What played a seat of account `user`'s, to keep in its record: one of the server's
/// agents as its runner `reported` it (its model's name from `setups`), else as it was last
/// offered; or a user's bot's owner.
pub(super) fn who_played(
    tx: &Transaction<'_>,
    user: &User,
    setups: &Setups,
    reported: Option<AgentRun>,
) -> Result<(Option<AgentRun>, Option<String>)> {
    Ok(match &user.login {
        Login::House => {
            let known = setups.get(&user.name.to_lowercase());
            let run = match reported {
                Some(mut run) => {
                    if run.setup.model_name.is_none() {
                        run.setup.model_name = known.and_then(|k| k.model_name.clone());
                    }
                    run
                }
                None => AgentRun {
                    setup: known.cloned().unwrap_or_else(|| AgentSetup {
                        agent: user.name.clone(),
                        ..AgentSetup::default()
                    }),
                    attempts: vec![],
                },
            };
            (Some(run), None)
        }
        Login::Bot { owner, .. } => (None, db::user_in(tx, owner)?.map(|o| o.name)),
        _ => (None, None),
    })
}

/// Fills in an agent seat kept with only its name (an official account's, or one recorded
/// before its runner offered it) from the agent of that name as it is offered now.
fn complete_agents(games: &mut [PastGame], setups: &Setups) {
    for p in games.iter_mut().flat_map(|g| &mut g.players) {
        if let PlayedBy::Agent(a) = &mut p.played_by
            && a.model.is_none()
            && let Some(setup) = setups.get(&a.agent.trim().to_lowercase())
        {
            *a = SeatAgent {
                agent: std::mem::take(&mut a.agent),
                ..SeatAgent::from(setup)
            };
        }
    }
}

/// What played a seat with no account here, or one recorded before what played it was
/// kept: what its account says now.
pub(super) type Recall<'a> = &'a dyn Fn(&str) -> Option<PlayedBy>;

/// What account `name` says played its seats: one of the server's agents, or a user's bot.
pub(super) fn recall(db: &Db, name: &str) -> Option<PlayedBy> {
    let user = db.user_by_name(name).ok()??;
    match &user.login {
        Login::House => Some(PlayedBy::Agent(SeatAgent {
            agent: user.name,
            ..SeatAgent::default()
        })),
        Login::Bot { owner, .. } => Some(PlayedBy::Bot {
            owner: db.user(owner).ok().flatten().map(|o| o.name),
        }),
        _ => None,
    }
}

/// A stored record as the history shows it.
pub(super) fn past_game(record: GameRecord, venue: Venue, recall: Recall<'_>) -> PastGame {
    let official = venue == Venue::Official;
    // A game made directly has accounts only in the seats the server's agents played.
    let agents = record.players.iter().any(|p| p.kind.is_some());
    PastGame {
        venue,
        arranged: match record.source {
            Source::Automatch => Some(Arranged::Automatch),
            Source::Table => Some(Arranged::Table),
            Source::Direct if agents => Some(Arranged::Agents),
            Source::Direct => Some(Arranged::Tokens),
            Source::Official => None,
        },
        official_id: None,
        open: false,
        replayable: false,
        started: record.started,
        ended: record.ended,
        rated: record.rated,
        reason: record.reason,
        kingdom: record.kingdom,
        landscapes: record.landscapes,
        others: record.others,
        players: (record.players.into_iter())
            .map(|p| {
                // An Arena seat's name is its account's; an official one's is the official
                // account's, whatever played it here.
                let account =
                    (p.user.clone()).or((p.kind.is_some() && !official).then(|| p.name.clone()));
                let played_by = match (&p.agent, p.kind) {
                    (Some(run), _) => PlayedBy::Agent(SeatAgent::from(&run.setup)),
                    (None, Some(UserKind::Registered)) => PlayedBy::Person,
                    (None, Some(UserKind::Guest)) => PlayedBy::Guest,
                    (None, Some(UserKind::Bot)) => match p.owner {
                        Some(owner) => PlayedBy::Bot { owner: Some(owner) },
                        None => (account.as_deref())
                            .and_then(recall)
                            .unwrap_or(PlayedBy::Bot { owner: None }),
                    },
                    (None, None) if official => PlayedBy::Official,
                    (None, None) => PlayedBy::Token,
                };
                PastPlayer {
                    account,
                    played_by,
                    seat: p.seat,
                    name: p.name,
                    vp: p.vp,
                    rank: p.rank,
                    turns: p.turns,
                    resigned: p.resigned,
                    rating: p.rating,
                    level: p.level,
                    agent: p.agent,
                }
            })
            .collect(),
        seed: record.seed,
        version: record.version,
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
            let mut game = past_game(r, Venue::Arena, &|name| recall(&self.db, name));
            game.open = self.hub.meta(&game.id).is_ok();
            game
        });
        Ok(games.collect())
    }

    fn count(&self, user: &str) -> Result<u32> {
        self.db.count_games(Venue::Arena, user)
    }
}
