//! Voiding games' ratings: their records become unrated, and every rating is worked out
//! again from the rated records left, in the order the games ended.

use super::db;
use super::{Arena, Error, Result};
use crate::rating::{self, Rating};
use dominion_protocol::lobby::GameRecord;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// What a void changed, or would.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Voided {
    pub games: Vec<String>,
    /// Each account whose rating changes: its name, then its rating before and after.
    pub ratings: Vec<(String, f64, f64)>,
}

/// An account's standing as the rated records make it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Standing {
    rating: Rating,
    games: u32,
    wins: u32,
}

/// How far a rating kept may be from the one worked out again: the records keep them
/// as JSON, which may not give back the last bit.
const EPSILON: f64 = 1e-6;

impl Arena {
    /// Makes `games` unrated and every rating what the other rated games make it. Refuses
    /// unless the records as they stand give exactly the ratings kept, so that nothing
    /// the records don't say is lost. With `dry_run`, only says what would change.
    pub fn void_ratings(&self, games: &[String], dry_run: bool) -> Result<Voided> {
        let void: HashSet<&str> = games.iter().map(String::as_str).collect();
        let voided = self.db.transaction(|tx| {
            let mut played = db::arena_records(tx)?;
            for id in &void {
                match played.iter().find(|(r, _)| r.game_id == *id) {
                    Some((r, _)) if r.rated => {}
                    Some(_) => return Err(Error::Conflict(format!("{id} is not rated"))),
                    None => return Err(Error::NotFound(format!("no record of game {id}"))),
                }
            }
            let accounts = db::ratings(tx)?;
            let names: HashMap<&str, &str> = (accounts.iter())
                .map(|a| (a.id.as_str(), a.name.as_str()))
                .collect();
            let keys = (played.iter())
                .map(|(record, users)| seat_keys(record, users, &names))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::Conflict)?;
            let kept =
                replay(&mut played, &keys, &HashSet::new(), true).map_err(Error::Conflict)?;
            for a in &accounts {
                let stored = Standing {
                    rating: a.rating,
                    games: a.games,
                    wins: a.wins,
                };
                let expected = kept.get(&a.id).copied().unwrap_or_default();
                if !same(stored, expected) {
                    return Err(Error::Conflict(format!(
                        "{}'s rating is not what the records make it ({:.2} kept, {:.2} \
                         from the records): nothing changed",
                        a.name, stored.rating.rating, expected.rating.rating
                    )));
                }
            }
            let after = replay(&mut played, &keys, &void, false).map_err(Error::Conflict)?;
            let mut voided = Voided {
                games: games.to_vec(),
                ratings: vec![],
            };
            for a in &accounts {
                let (old, new) = (kept.get(&a.id), after.get(&a.id));
                if old == new {
                    continue;
                }
                let new = new.copied().unwrap_or_default();
                let old = old.copied().unwrap_or_default();
                voided
                    .ratings
                    .push((a.name.clone(), old.rating.rating, new.rating.rating));
                if !dry_run {
                    db::set_rating(tx, &a.id, new.rating, new.games, new.wins)?;
                }
            }
            if !dry_run {
                for (record, _) in &played {
                    db::set_record(tx, record)?;
                }
            }
            Ok(voided)
        })?;
        if !dry_run {
            for id in games {
                self.hub.unrate(id);
            }
            self.bump();
        }
        Ok(voided)
    }
}

fn same(a: Standing, b: Standing) -> bool {
    let close = |x: f64, y: f64| (x - y).abs() <= EPSILON;
    close(a.rating.rating, b.rating.rating)
        && close(a.rating.deviation, b.rating.deviation)
        && close(a.rating.volatility, b.rating.volatility)
        && a.rating.at == b.rating.at
        && (a.games, a.wins) == (b.games, b.wins)
}

/// Whose rating each seat of `record` moves: the account of `users` (those that played
/// it) with the seat's account name, else the one account left; a seat whose account is
/// gone keeps a rating of its own by name. A seat without an account has none.
fn seat_keys(
    record: &GameRecord,
    users: &[String],
    names: &HashMap<&str, &str>,
) -> std::result::Result<Vec<Option<String>>, String> {
    let account = |p: &dominion_protocol::lobby::RecordPlayer| {
        p.kind
            .map(|_| p.user.clone().unwrap_or_else(|| p.name.clone()))
    };
    let mut left: Vec<&String> = users.iter().collect();
    let mut keys: Vec<Option<String>> = record.players.iter().map(|_| None).collect();
    for (i, p) in record.players.iter().enumerate() {
        let Some(name) = account(p) else { continue };
        let found = left.iter().position(|u| {
            names
                .get(u.as_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(&name))
        });
        if let Some(at) = found {
            keys[i] = Some(left.remove(at).clone());
        }
    }
    let unmatched: Vec<usize> = (0..keys.len())
        .filter(|&i| keys[i].is_none() && account(&record.players[i]).is_some())
        .collect();
    match (unmatched.as_slice(), left.as_slice()) {
        ([], _) => {}
        ([seat], [user]) => keys[*seat] = Some((*user).clone()),
        (seats, []) => {
            for &i in seats {
                let name = account(&record.players[i]).unwrap_or_default();
                keys[i] = Some(format!("gone:{}", name.to_lowercase()));
            }
        }
        _ => {
            return Err(format!(
                "can't tell which account played which seat of {}",
                record.game_id
            ));
        }
    }
    Ok(keys)
}

/// Plays the rated records again, but `void`'s, from fresh ratings, giving each its
/// players' ratings before and after and taking the rating from `void`'s. With `check`,
/// changes nothing and fails at the first rating a record keeps that isn't the one
/// worked out. The standing of each account (and of each account gone, as `gone:name`).
fn replay(
    played: &mut [(GameRecord, Vec<String>)],
    keys: &[Vec<Option<String>>],
    void: &HashSet<&str>,
    check: bool,
) -> std::result::Result<HashMap<String, Standing>, String> {
    let mut standings: HashMap<String, Standing> = HashMap::new();
    for ((record, _), keys) in played.iter_mut().zip(keys) {
        if !record.rated {
            continue;
        }
        if void.contains(record.game_id.as_str()) {
            record.rated = false;
            for p in &mut record.players {
                p.rating = None;
            }
            continue;
        }
        let Some(keys) = keys.iter().cloned().collect::<Option<Vec<String>>>() else {
            return Err(format!(
                "{} is rated with a seat of no account",
                record.game_id
            ));
        };
        let before: Vec<Rating> = (keys.iter())
            .map(|k| standings.get(k).map(|s| s.rating).unwrap_or_default())
            .collect();
        let ranks: Vec<u32> = record.players.iter().map(|p| p.rank).collect();
        let after = rating::after_game(&before, &ranks, record.ended / 1000);
        let won = ranks.iter().filter(|&&r| r == 1).count() < ranks.len();
        for (i, p) in record.players.iter_mut().enumerate() {
            let pair = (before[i].rating, after[i].rating);
            if check {
                let close = |(a, b): (f64, f64)| {
                    (a - pair.0).abs() <= EPSILON && (b - pair.1).abs() <= EPSILON
                };
                if !p.rating.is_some_and(close) {
                    return Err(format!(
                        "{}'s record doesn't give {} the rating the records before it make: \
                         nothing changed",
                        record.game_id, p.name
                    ));
                }
            } else {
                p.rating = Some(pair);
            }
            let s = standings.entry(keys[i].clone()).or_default();
            s.rating = after[i];
            s.games += 1;
            s.wins += u32::from(p.rank == 1 && won);
        }
    }
    Ok(standings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Login;
    use crate::hub::Hub;
    use dominion_protocol::lobby::{RecordPlayer, Source, UserKind, Venue};
    use std::sync::Arc;

    fn seat(name: &str, rank: u32) -> RecordPlayer {
        RecordPlayer {
            seat: 0,
            name: name.into(),
            kind: Some(UserKind::Bot),
            user: None,
            vp: 0,
            rank,
            turns: 0,
            resigned: false,
            rating: None,
            level: None,
            agent: None,
            owner: None,
        }
    }

    /// Records rated games as the server does when they end: Ada and Bo, Bo and Cy, Ada
    /// and Cy, each won by the first named.
    fn arena() -> Arc<Arena> {
        let arena = Arena::open(Arc::new(Hub::default()), Default::default()).unwrap();
        let ids: Vec<String> = ["Ada", "Bo", "Cy"]
            .iter()
            .map(|n| arena.db.add_user(n, &Login::House, 0).unwrap().id)
            .collect();
        for (n, (a, b)) in [(0, 1), (1, 2), (0, 2)].into_iter().enumerate() {
            arena
                .db
                .transaction(|tx| {
                    let users = [&ids[a], &ids[b]];
                    let mut accounts = vec![];
                    for u in users {
                        accounts.push(db::user_in(tx, u)?.unwrap());
                    }
                    let before: Vec<Rating> = accounts.iter().map(|u| u.rating).collect();
                    let ended = (n as u64 + 1) * 86_400_000;
                    let after = rating::after_game(&before, &[1, 2], ended / 1000);
                    let mut record = GameRecord {
                        game_id: format!("g{}", n + 1),
                        started: 0,
                        ended,
                        source: Source::Direct,
                        rated: true,
                        reason: String::new(),
                        kingdom: vec![],
                        landscapes: vec![],
                        others: vec![],
                        players: vec![seat(&accounts[0].name, 1), seat(&accounts[1].name, 2)],
                        seed: None,
                        version: None,
                    };
                    for i in 0..2 {
                        record.players[i].rating = Some((before[i].rating, after[i].rating));
                        db::tally(
                            tx,
                            &db::Tally {
                                user: users[i],
                                win: i == 0,
                                rating: Some(after[i]),
                            },
                        )?;
                    }
                    let users: Vec<Option<String>> = users.map(|u| Some(u.clone())).to_vec();
                    db::add_record(tx, Venue::Arena, &record, &users)
                })
                .unwrap();
        }
        arena
    }

    fn standing(arena: &Arena, name: &str) -> (f64, u32, u32) {
        let u = arena.db.user_by_name(name).unwrap().unwrap();
        (u.rating.rating, u.rated_games, u.rated_wins)
    }

    #[test]
    fn voiding_a_game_rates_the_rest_as_if_it_never_was() {
        let arena = arena();
        let bo = standing(&arena, "Bo");
        let dry = arena.void_ratings(&["g1".into()], true).unwrap();
        assert_eq!(standing(&arena, "Bo"), bo);
        let voided = arena.void_ratings(&["g1".into()], false).unwrap();
        assert_eq!(voided, dry);
        // Without g1, Bo's first game is g2, from a fresh rating, and Ada's is g3.
        let (fresh, cy) = (Rating::default(), Rating::default());
        let g2 = rating::after_game(&[fresh, cy], &[1, 2], 2 * 86_400);
        let g3 = rating::after_game(&[fresh, g2[1]], &[1, 2], 3 * 86_400);
        assert_eq!(standing(&arena, "Bo"), (g2[0].rating, 1, 1));
        assert_eq!(standing(&arena, "Ada"), (g3[0].rating, 1, 1));
        assert_eq!(standing(&arena, "Cy"), (g3[1].rating, 2, 0));
        let games = arena.db.games(Venue::Arena, None, u64::MAX, 10).unwrap();
        let g1 = games.iter().find(|g| g.game_id == "g1").unwrap();
        assert!(!g1.rated && g1.players.iter().all(|p| p.rating.is_none()));
        let g3r = games.iter().find(|g| g.game_id == "g3").unwrap();
        assert_eq!(g3r.players[0].rating, Some((fresh.rating, g3[0].rating)));
        assert!(matches!(
            arena.void_ratings(&["g1".into()], false),
            Err(Error::Conflict(_))
        ));
    }

    #[test]
    fn a_rating_the_records_dont_explain_refuses_the_void() {
        let arena = arena();
        let bo = arena.db.user_by_name("Bo").unwrap().unwrap();
        arena
            .db
            .transaction(|tx| {
                db::set_rating(tx, &bo.id, Rating::default(), bo.rated_games, bo.rated_wins)
            })
            .unwrap();
        assert!(matches!(
            arena.void_ratings(&["g1".into()], false),
            Err(Error::Conflict(_))
        ));
        assert_eq!(standing(&arena, "Bo").0, Rating::default().rating);
    }
}
