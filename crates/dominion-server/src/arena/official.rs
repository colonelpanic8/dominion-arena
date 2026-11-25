//! The dominion.games accounts `dominion serve` keeps connected, as the lobby lists them,
//! and their finished games' records.

use super::db::{self, Db};
use super::history::GameSource;
use super::records::{past_game, recall, who_played};
use super::{Arena, Result};
use crate::hub::{lock, now_ms};
use dominion_protocol::lobby::{
    GameRecord, LaunchOptions, OFFICIAL_GAME_PREFIX, OfficialAccount, OfficialGame, PastGame,
    RecordPlayer, Source, Venue, official_game_id,
};
use dominion_protocol::view::GameResult;
use std::sync::{Arc, OnceLock};

/// What the lobby knows of an official account beyond what it lists.
#[derive(Default)]
pub(super) struct Official {
    pub info: OfficialAccount,
    /// When its current game was first seen running (ms).
    started: Option<(String, u64)>,
    /// The agent last seen playing its current game.
    played_by: Option<(String, String, LaunchOptions)>,
    /// The last game recorded.
    recorded: Option<String>,
}

/// Whether the official game a history id names has a recording to replay.
pub type Replayable = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The records of the official accounts' finished games.
pub(super) struct OfficialRecords {
    pub db: Arc<Db>,
    pub replayable: Arc<OnceLock<Replayable>>,
}

impl GameSource for OfficialRecords {
    fn venue(&self) -> Venue {
        Venue::Official
    }

    fn page(&self, user: Option<&str>, before: u64, limit: usize) -> Result<Vec<PastGame>> {
        let records = self.db.games(Venue::Official, user, before, limit)?;
        let games = records.into_iter().map(|r| {
            let mut game = past_game(r, Venue::Official, &|name| recall(&self.db, name));
            game.official_id = game
                .id
                .strip_prefix(OFFICIAL_GAME_PREFIX)
                .map(str::to_string);
            game.replayable = self.replayable.get().is_some_and(|has| has(&game.id));
            game
        });
        Ok(games.collect())
    }

    fn count(&self, user: &str) -> Result<u32> {
        self.db.count_games(Venue::Official, user)
    }
}

/// Notes that `job` plays official account `account`'s game.
pub(super) fn played_by(state: &mut super::State, account: &str, job: &crate::house::Job) {
    if !job.game_id.is_empty() {
        let entry = state.official.entry(account.into()).or_default();
        entry.played_by = Some((job.game_id.clone(), job.agent.clone(), job.options));
    }
}

impl Arena {
    /// Notes official account `info.id` as it stands, `result` being how its game ended.
    /// A game seen finished for the first time is recorded.
    pub(crate) fn set_official(&self, mut info: OfficialAccount, result: Option<&GameResult>) {
        let game = info.game.as_ref().map(|g| g.id.clone());
        let finished = {
            let mut state = lock(&self.state);
            let job = state.house.official_job(&info.id);
            let entry = state.official.entry(info.id.clone()).or_default();
            if let Some(game) = &game {
                if info.game.as_ref().is_some_and(|g| g.running)
                    && entry.started.as_ref().is_none_or(|(g, _)| g != game)
                {
                    entry.started = Some((game.clone(), now_ms()));
                }
                if let Some(job) = job.filter(|j| &j.game_id == game) {
                    entry.played_by = Some((game.clone(), job.agent, job.options));
                }
            }
            info.agent = None;
            let changed = entry.info != info;
            entry.info = info;
            let finished = game
                .filter(|_| result.is_some())
                .filter(|g| entry.recorded.as_ref() != Some(g));
            if finished.is_some() {
                entry.recorded = finished.clone();
            }
            let started = finished.as_ref().and_then(|g| {
                (entry.started.as_ref())
                    .filter(|(s, _)| s == g)
                    .map(|s| s.1)
            });
            let agent = finished.as_ref().and_then(|g| {
                (entry.played_by.as_ref())
                    .filter(|(p, ..)| p == g)
                    .map(|p| (p.1.clone(), p.2))
            });
            let owner = entry.info.owner.clone();
            let game = entry.info.game.clone();
            drop(state);
            if changed {
                self.bump();
            }
            finished.and(game).map(|game| (game, started, agent, owner))
        };
        if let (Some((game, started, agent, owner)), Some(result)) = (finished, result) {
            match self.record_official(&game, result, started, agent, &owner) {
                Ok(true) => self.bump(),
                Ok(false) => {}
                Err(e) => eprintln!("dominion: recording an official game: {e}"),
            }
        }
    }

    /// Lets the history tell which official games `replayable` can replay. Only the first
    /// call counts.
    pub fn set_official_replays(&self, replayable: Replayable) {
        let _ = self.official_replays.set(replayable);
    }

    /// The agent that has played official account `account`'s game `game`: the only one
    /// that may go on playing it.
    pub(crate) fn official_game_agent(&self, account: &str, game: &str) -> Option<String> {
        let state = lock(&self.state);
        let played = state.official.get(account)?.played_by.as_ref()?;
        (played.0 == game).then(|| played.1.clone())
    }

    /// Starts draining for a restart (`on`), or stops: while draining, official accounts
    /// start no new games, and the games they play run to their end. The official games
    /// still running, as `(account, game)`.
    pub fn drain(&self, on: bool) -> Vec<(String, String)> {
        (self.draining).store(on, std::sync::atomic::Ordering::SeqCst);
        let state = lock(&self.state);
        (state.official.values())
            .filter_map(|o| {
                let game = o.info.game.as_ref().filter(|g| g.running)?;
                Some((o.info.id.clone(), game.id.clone()))
            })
            .collect()
    }

    /// Whether the server is draining for a restart.
    pub(crate) fn draining(&self) -> bool {
        self.draining.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn official_game_options(&self, account: &str, game: &str) -> Option<LaunchOptions> {
        let state = lock(&self.state);
        let played = state.official.get(account)?.played_by.as_ref()?;
        (played.0 == game).then_some(played.2)
    }

    /// The official accounts, each with the agent sent to play as it.
    pub(super) fn official_accounts(&self, state: &super::State) -> Vec<OfficialAccount> {
        (state.official.values())
            .map(|o| OfficialAccount {
                agent: state.house.official_job(&o.info.id).map(|j| j.agent),
                ..o.info.clone()
            })
            .collect()
    }

    /// Records a finished official game for the agent that played it (its account), else
    /// for the official account's owner. Nobody's stats or rating here change. Whether it
    /// was recorded now.
    fn record_official(
        &self,
        game: &OfficialGame,
        result: &GameResult,
        started: Option<u64>,
        agent: Option<(String, LaunchOptions)>,
        owner: &str,
    ) -> Result<bool> {
        let id = official_game_id(&game.id);
        let strategy = agent
            .as_ref()
            .map(|(_, strategy)| *strategy)
            .unwrap_or_default();
        let player = match agent
            .as_ref()
            .map(|(name, _)| self.db.user_by_name(name))
            .transpose()?
            .flatten()
        {
            Some(user) => Some(user),
            None => self.db.user_by_name(owner)?,
        };
        let ended = now_ms();
        let setups = lock(&self.state).house.setups();
        self.db.transaction(|tx| {
            if db::recorded(tx, &id)? {
                return Ok(false);
            }
            let (agent, bot_owner) = match &player {
                Some(user) => who_played(tx, user, &setups, None, strategy)?,
                None => (None, None),
            };
            let mut users = vec![];
            let players = (result.players.iter())
                .map(|p| {
                    let user = player.as_ref().filter(|_| Some(p.index) == game.seat);
                    users.push(user.map(|u| u.id.clone()));
                    RecordPlayer {
                        seat: p.index,
                        name: p.name.clone(),
                        kind: user.map(|u| u.kind()),
                        user: user.map(|u| u.name.clone()).filter(|n| *n != p.name),
                        vp: p.vp,
                        rank: p.rank,
                        turns: p.turns,
                        resigned: p.resigned,
                        rating: None,
                        level: game.levels.get(usize::from(p.index)).copied().flatten(),
                        agent: user.and(agent.clone()),
                        owner: user.and(bot_owner.clone()),
                    }
                })
                .collect();
            let record = GameRecord {
                game_id: id.clone(),
                started: started.unwrap_or(ended),
                ended,
                source: Source::Official,
                rated: game.rated,
                voided: None,
                reason: result.reason.clone().unwrap_or_default(),
                kingdom: game.kingdom.clone(),
                landscapes: game.landscapes.clone(),
                others: game.others.clone(),
                players,
                seed: None,
                version: crate::commit(),
            };
            db::add_record(tx, Venue::Official, &record, &users)?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use dominion_protocol::lobby::{Credentials, HistoryQuery, PlayedBy};
    use dominion_protocol::view::ResultPlayer;
    use std::sync::Arc;

    #[test]
    fn a_finished_official_game_is_listed_then_recorded_once_for_the_owner() {
        let arena = Arena::open(Arc::new(Hub::default()), Default::default()).unwrap();
        let credentials = Credentials {
            name: "Owner".into(),
            password: "a long enough password".into(),
            invite: None,
        };
        arena
            .register("127.0.0.1".parse().unwrap(), &credentials, None)
            .unwrap();
        let game = OfficialGame {
            id: "123".into(),
            running: true,
            rated: true,
            players: vec!["Rival".into(), "OwnerOnline".into()],
            seat: Some(1),
            kingdom: vec!["Village".into()],
            landscapes: vec![],
            others: vec!["Colony".into()],
            levels: vec![Some(41.25), None],
        };
        let mut info = OfficialAccount {
            id: "main".into(),
            account: "OwnerOnline".into(),
            owner: "Owner".into(),
            connected: true,
            searching: false,
            agent: None,
            game: Some(game),
        };
        arena.set_official(info.clone(), None);
        assert_eq!(arena.lobby(None).official, [info.clone()]);

        info.game.as_mut().unwrap().running = false;
        let player = |index: u8, name: &str, rank: u32| ResultPlayer {
            index,
            name: name.into(),
            vp: 10,
            rank,
            turns: 15,
            resigned: false,
        };
        let result = GameResult {
            reason: Some("provinces-gone".into()),
            players: vec![player(0, "Rival", 2), player(1, "OwnerOnline", 1)],
        };
        arena.set_official(info.clone(), Some(&result));
        arena.set_official(info, Some(&result));

        let games = arena.history(&HistoryQuery::default()).unwrap().games;
        assert_eq!(games.len(), 1);
        let record = &games[0];
        assert_eq!(record.venue, Venue::Official);
        assert_eq!(record.official_id.as_deref(), Some("123"));
        assert!(record.rated && !record.open);
        assert_eq!(record.kingdom, ["Village"]);
        assert_eq!(record.others, ["Colony"]);
        assert_eq!(record.players[1].account.as_deref(), Some("Owner"));
        assert_eq!(record.players[1].played_by, PlayedBy::Person);
        assert_eq!(record.players[0].played_by, PlayedBy::Official);
        assert_eq!(record.players[0].account, None);
        assert!(record.players.iter().all(|p| p.rating.is_none()));
        assert_eq!(record.players[0].level, Some(41.25));
        assert_eq!(record.players[1].level, None);
        let owner = arena.profile("Owner").unwrap();
        assert_eq!(
            owner.stats.games, 0,
            "Arena's stats count only its own games"
        );
        assert_eq!(owner.official_games, 1);
    }
}
