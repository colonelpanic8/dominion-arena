//! The dominion.games accounts `dominion serve` keeps connected, as the lobby lists them,
//! and their finished games' records.

use super::{Arena, Result, db};
use crate::hub::{lock, now_ms};
use dominion_protocol::lobby::{GameRecord, OfficialAccount, OfficialGame, RecordPlayer, Source};
use dominion_protocol::view::GameResult;

/// What the lobby knows of an official account beyond what it lists.
#[derive(Default)]
pub(super) struct Official {
    pub info: OfficialAccount,
    /// When its current game was first seen running (ms).
    started: Option<(String, u64)>,
    /// The agent last seen playing its current game.
    played_by: Option<(String, String)>,
    /// The last game recorded.
    recorded: Option<String>,
}

/// The id of an official game's record.
fn record_id(game: &str) -> String {
    format!("dominion.games-{game}")
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
                    entry.played_by = Some((game.clone(), job.agent));
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
                    .filter(|(p, _)| p == g)
                    .map(|p| p.1.clone())
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
            match self.record_official(&game, result, started, agent.as_deref(), &owner) {
                Ok(true) => self.bump(),
                Ok(false) => {}
                Err(e) => eprintln!("dominion: recording an official game: {e}"),
            }
        }
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
        agent: Option<&str>,
        owner: &str,
    ) -> Result<bool> {
        let id = record_id(&game.id);
        let player = match agent
            .map(|a| self.db.user_by_name(a))
            .transpose()?
            .flatten()
        {
            Some(user) => Some(user),
            None => self.db.user_by_name(owner)?,
        };
        let ended = now_ms();
        self.db.transaction(|tx| {
            if db::recorded(tx, &id)? {
                return Ok(false);
            }
            let mut users = vec![];
            let players = (result.players.iter())
                .map(|p| {
                    let user = player.as_ref().filter(|_| Some(p.index) == game.seat);
                    users.push(user.map(|u| u.id.clone()));
                    RecordPlayer {
                        seat: p.index,
                        name: p.name.clone(),
                        kind: user.map(|u| u.kind()),
                        vp: p.vp,
                        rank: p.rank,
                        turns: p.turns,
                        resigned: p.resigned,
                        rating: None,
                    }
                })
                .collect();
            let record = GameRecord {
                game_id: id.clone(),
                started: started.unwrap_or(ended),
                ended,
                source: Source::Official,
                rated: game.rated,
                reason: result.reason.clone().unwrap_or_default(),
                kingdom: game.kingdom.clone(),
                landscapes: game.landscapes.clone(),
                players,
            };
            db::add_record(tx, &record, &users)?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use dominion_protocol::lobby::Credentials;
    use dominion_protocol::view::ResultPlayer;
    use std::sync::Arc;

    #[test]
    fn a_finished_official_game_is_listed_then_recorded_once_for_the_owner() {
        let arena = Arena::open(Arc::new(Hub::default()), Default::default()).unwrap();
        let credentials = Credentials {
            name: "Owner".into(),
            password: "a long enough password".into(),
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

        let games = arena.history(Some("Owner"), None, 10).unwrap().games;
        assert_eq!(games.len(), 1);
        let record = &games[0];
        assert_eq!(record.source, Source::Official);
        assert!(record.rated);
        assert_eq!(record.kingdom, ["Village"]);
        assert!(record.players[1].kind.is_some() && record.players[0].kind.is_none());
        assert!(record.players.iter().all(|p| p.rating.is_none()));
        let owner = arena.profile("Owner").unwrap();
        assert_eq!(
            owner.stats.games, 0,
            "Arena's stats count only its own games"
        );
    }
}
