//! The history read model: finished games for `/api/history` and players' pages, as the
//! protocol's `PastGame`s, from the sources that fill it. A source (`GameSource`) knows
//! only where its games are kept; what is asked of them (whose, where, rated, a name in
//! them) and paging across sources are this model's.

use super::Result;
use dominion_protocol::lobby::{History, Matchup, PastGame, PastPlayer, PlayedBy, Venue};
use std::collections::HashMap;

/// One place finished games are kept.
pub trait GameSource: Send + Sync {
    /// Where its games were played.
    fn venue(&self) -> Venue;

    /// Up to `limit` of its games that ended before `before`, newest first: account
    /// `user`'s, or everyone's.
    fn page(&self, user: Option<&str>, before: u64, limit: usize) -> Result<Vec<PastGame>>;

    /// How many games account `user` played.
    fn count(&self, user: &str) -> Result<u32>;
}

/// Which games to list.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ask<'a> {
    /// Played by this account (its id).
    pub user: Option<&'a str>,
    pub venue: Option<Venue>,
    /// Rated games only: none voided.
    pub rated: bool,
    /// Voided games too.
    pub voided: bool,
    /// A player, the account behind a seat, an agent's model or program, or a card whose
    /// name contains it, in any case.
    pub text: Option<&'a str>,
    /// With this opponent: a `Matchup::key`.
    pub vs: Option<&'a str>,
}

impl Ask<'_> {
    fn admits(&self, game: &PastGame, text: Option<&str>) -> bool {
        if game.voided.is_some() && (self.rated || !self.voided) {
            return false;
        }
        if self.rated && !game.rated {
            return false;
        }
        if let Some(vs) = self.vs
            && !game.players.iter().any(|p| key(p).eq_ignore_ascii_case(vs))
        {
            return false;
        }
        let Some(text) = text else {
            return true;
        };
        let has = |s: &str| s.to_lowercase().contains(text);
        let named = |p: &PastPlayer| {
            let mut names = vec![Some(&p.name), p.account.as_ref()];
            if let PlayedBy::Agent(a) = &p.played_by {
                names.extend([a.model.as_ref(), a.name.as_ref(), a.program.as_ref()]);
            }
            names.into_iter().flatten().any(|s| has(s))
        };
        game.players.iter().any(named)
            || game.kingdom.iter().chain(&game.landscapes).any(|s| has(s))
    }
}

/// Games read from one source at a time.
const PAGE: usize = 100;
/// The most games one answer reads from a source, however few of them match.
const SCAN: usize = 1000;

pub struct HistoryModel {
    sources: Vec<Box<dyn GameSource>>,
}

impl HistoryModel {
    pub fn new(sources: Vec<Box<dyn GameSource>>) -> HistoryModel {
        HistoryModel { sources }
    }

    /// Up to `limit` games `ask` admits that ended before `before`, newest first, and
    /// where to go on from.
    pub fn find(&self, ask: &Ask<'_>, before: u64, limit: usize) -> Result<History> {
        let text = ask
            .text
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty());
        let mut found = vec![];
        // The oldest moment every source has been read back to.
        let mut frontier: Option<u64> = None;
        for source in &self.sources {
            if ask.venue.is_some_and(|v| v != source.venue()) {
                continue;
            }
            let mut cursor = before;
            let (mut read, mut matched) = (0, 0);
            let complete = loop {
                let page = source.page(ask.user, cursor, PAGE.min(SCAN - read))?;
                let full = page.len() == PAGE.min(SCAN - read);
                read += page.len();
                for game in page {
                    cursor = game.ended;
                    if ask.admits(&game, text.as_deref()) {
                        found.push(game);
                        matched += 1;
                        if matched == limit {
                            break;
                        }
                    }
                }
                if !full {
                    break true;
                }
                if matched == limit || read >= SCAN {
                    break false;
                }
            };
            if !complete {
                frontier = Some(frontier.map_or(cursor, |f| f.max(cursor)));
            }
        }
        found.retain(|g| frontier.is_none_or(|f| g.ended >= f));
        found.sort_by(|a, b| b.ended.cmp(&a.ended).then_with(|| a.id.cmp(&b.id)));
        let next = if found.len() >= limit {
            found.truncate(limit);
            found.last().map(|g| g.ended)
        } else {
            frontier
        };
        Ok(History { games: found, next })
    }

    /// Every game `ask` admits, newest first.
    pub fn all(&self, ask: &Ask<'_>) -> Result<Vec<PastGame>> {
        let (mut games, mut before) = (vec![], u64::MAX);
        loop {
            let page = self.find(ask, before, PAGE)?;
            games.extend(page.games);
            match page.next {
                Some(next) => before = next,
                None => return Ok(games),
            }
        }
    }

    /// How many games account `user` played at `venue`.
    pub fn count(&self, user: &str, venue: Venue) -> Result<u32> {
        let sources = self.sources.iter().filter(|s| s.venue() == venue);
        sources.map(|s| s.count(user)).sum()
    }
}

/// Who played a seat, as `Matchup::key` and `Ask::vs` name them: its account, else its
/// name in the game.
fn key(p: &PastPlayer) -> &str {
    p.account.as_deref().unwrap_or(&p.name)
}

/// Account `name`'s record against each opponent in `games` (newest first), most games
/// together first, then the latest met. A game where they sat more than once counts
/// their first seat, and none against themselves; a voided game counts for no one.
pub fn head_to_head(games: &[PastGame], name: &str) -> Vec<Matchup> {
    let mut by_key: HashMap<String, Matchup> = HashMap::new();
    for game in games.iter().filter(|g| g.voided.is_none()) {
        let mine = |p: &PastPlayer| {
            p.account
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(name))
        };
        let Some(me) = game.players.iter().find(|p| mine(p)) else {
            continue;
        };
        for them in game.players.iter().filter(|p| !mine(p)) {
            let m = by_key
                .entry(key(them).to_lowercase())
                .or_insert_with(|| Matchup {
                    key: key(them).to_string(),
                    seat: PastPlayer {
                        agent: None,
                        ..them.clone()
                    },
                    games: 0,
                    wins: 0,
                    losses: 0,
                    ties: 0,
                    last: game.ended,
                });
            m.games += 1;
            match me.rank.cmp(&them.rank) {
                std::cmp::Ordering::Less => m.wins += 1,
                std::cmp::Ordering::Greater => m.losses += 1,
                std::cmp::Ordering::Equal => m.ties += 1,
            }
        }
    }
    let mut all: Vec<Matchup> = by_key.into_values().collect();
    all.sort_by(|a, b| {
        (b.games.cmp(&a.games))
            .then(b.last.cmp(&a.last))
            .then(a.key.cmp(&b.key))
    });
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_protocol::lobby::SeatAgent;

    struct Fake {
        venue: Venue,
        /// Newest first.
        games: Vec<PastGame>,
    }

    impl GameSource for Fake {
        fn venue(&self) -> Venue {
            self.venue
        }

        fn page(&self, user: Option<&str>, before: u64, limit: usize) -> Result<Vec<PastGame>> {
            let theirs = |g: &&PastGame| {
                user.is_none_or(|u| g.players.iter().any(|p| p.account.as_deref() == Some(u)))
            };
            let games = self
                .games
                .iter()
                .filter(theirs)
                .filter(|g| g.ended < before);
            Ok(games.take(limit).cloned().collect())
        }

        fn count(&self, user: &str) -> Result<u32> {
            Ok(self.page(Some(user), u64::MAX, usize::MAX)?.len() as u32)
        }
    }

    fn game(venue: Venue, ended: u64, players: &[&str], card: &str, rated: bool) -> PastGame {
        PastGame {
            id: format!("{venue:?}{ended}"),
            venue,
            arranged: None,
            official_id: None,
            open: false,
            replayable: false,
            started: ended - 1,
            ended,
            rated,
            voided: None,
            reason: String::new(),
            kingdom: vec![card.into()],
            landscapes: vec![],
            others: vec![],
            players: (players.iter().enumerate())
                .map(|(i, name)| PastPlayer {
                    seat: i as u8,
                    name: name.to_string(),
                    account: Some(name.to_lowercase()),
                    played_by: match *name {
                        "Rival" => PlayedBy::Agent(SeatAgent {
                            agent: "Rival".into(),
                            model: Some("claude-opus-5-5".into()),
                            ..SeatAgent::default()
                        }),
                        _ => PlayedBy::Person,
                    },
                    vp: 0,
                    rank: 1,
                    turns: 1,
                    resigned: false,
                    rating: None,
                    level: None,
                    agent: None,
                    model: None,
                })
                .collect(),
            seed: None,
            version: None,
        }
    }

    /// Arena games ending at 2, 4, ... 400 and official ones at 3, 9, ... 597: Ada plays
    /// every one, Bo every Arena game, Rival (an agent on claude-opus-5-5) the official
    /// ones; every tenth Arena game is rated, and every 50th of each has Witch.
    fn model() -> HistoryModel {
        let card = |i: u64| {
            if i.is_multiple_of(50) {
                "Witch"
            } else {
                "Village"
            }
        };
        let arena = (1..=200).rev().map(|i| {
            game(
                Venue::Arena,
                2 * i,
                &["Ada", "Bo"],
                card(i),
                i.is_multiple_of(10),
            )
        });
        let official = (1..=100).rev().map(|i| {
            game(
                Venue::Official,
                6 * i - 3,
                &["Rival", "Ada"],
                card(i),
                false,
            )
        });
        HistoryModel::new(vec![
            Box::new(Fake {
                venue: Venue::Arena,
                games: arena.collect(),
            }),
            Box::new(Fake {
                venue: Venue::Official,
                games: official.collect(),
            }),
        ])
    }

    fn ended(history: &History) -> Vec<u64> {
        history.games.iter().map(|g| g.ended).collect()
    }

    #[test]
    fn pages_merge_the_sources_newest_first_without_gaps_or_repeats() {
        let model = model();
        let mut seen = vec![];
        let mut before = u64::MAX;
        loop {
            let page = model.find(&Ask::default(), before, 30).unwrap();
            assert!(page.games.len() <= 30);
            seen.extend(ended(&page));
            match page.next {
                Some(next) => before = next,
                None => break,
            }
        }
        let mut all: Vec<u64> = (1..=200)
            .map(|i| 2 * i)
            .chain((1..=100).map(|i| 6 * i - 3))
            .collect();
        all.sort_by(|a, b| b.cmp(a));
        assert_eq!(seen, all);
    }

    #[test]
    fn filters_pick_by_venue_player_rating_and_name() {
        let model = model();
        let find = |ask: Ask<'_>| model.find(&ask, u64::MAX, 500).unwrap();
        let official = find(Ask {
            venue: Some(Venue::Official),
            ..Ask::default()
        });
        assert_eq!(official.games.len(), 100);
        assert!(official.games.iter().all(|g| g.venue == Venue::Official));
        assert_eq!(
            find(Ask {
                user: Some("bo"),
                ..Ask::default()
            })
            .games
            .len(),
            200
        );
        let rated = find(Ask {
            rated: true,
            ..Ask::default()
        });
        assert_eq!(ended(&rated)[..3], [400, 380, 360]);
        assert_eq!(rated.games.len(), 20);
        assert_eq!(
            ended(&find(Ask {
                text: Some(" wITCH "),
                ..Ask::default()
            })),
            [597, 400, 300, 297, 200, 100]
        );
        for text in ["rival", "OPUS-5-5"] {
            let ask = Ask {
                text: Some(text),
                ..Ask::default()
            };
            assert_eq!(find(ask).games.len(), 100, "{text}");
        }
        assert!(
            find(Ask {
                text: Some("Smithy"),
                ..Ask::default()
            })
            .games
            .is_empty()
        );
        assert_eq!(model.count("ada", Venue::Official).unwrap(), 100);
        assert_eq!(model.count("bo", Venue::Official).unwrap(), 0);
    }

    #[test]
    fn a_rare_match_is_found_by_going_on_from_where_reading_stopped() {
        let model = model();
        let ask = Ask {
            text: Some("witch"),
            venue: Some(Venue::Arena),
            ..Ask::default()
        };
        let first = model.find(&ask, u64::MAX, 2).unwrap();
        assert_eq!(ended(&first), [400, 300]);
        let rest = model.find(&ask, first.next.unwrap(), 10).unwrap();
        assert_eq!(ended(&rest), [200, 100]);
        assert_eq!(rest.next, None);
    }

    #[test]
    fn head_to_head_counts_places_against_each_opponent() {
        let placed = |ended: u64, seats: &[(&str, u32)]| {
            let names: Vec<&str> = seats.iter().map(|s| s.0).collect();
            let mut g = game(Venue::Arena, ended, &names, "Village", false);
            for (p, (_, rank)) in g.players.iter_mut().zip(seats) {
                p.rank = *rank;
            }
            g
        };
        let mut token = placed(1, &[("Ada", 1), ("Kit", 2)]);
        token.players[1].account = None;
        let mut voided = placed(4, &[("Ada", 1), ("Bo", 2)]);
        voided.voided = Some(4);
        let games = [
            placed(9, &[("Ada", 2), ("Bo", 1), ("Cy", 3)]),
            placed(8, &[("Bo", 1), ("Ada", 1)]),
            placed(7, &[("Ada", 1), ("Cy", 2)]),
            placed(6, &[("Ada", 1), ("Ada", 2)]),
            placed(5, &[("Bo", 1), ("Cy", 2)]),
            voided,
            token,
        ];
        let rows: Vec<_> = head_to_head(&games, "ADA")
            .into_iter()
            .map(|m| (m.key, m.games, m.wins, m.losses, m.ties, m.last))
            .collect();
        assert_eq!(
            rows,
            [
                ("bo".to_string(), 2, 0, 1, 1, 9),
                ("cy".to_string(), 2, 2, 0, 0, 9),
                ("Kit".to_string(), 1, 1, 0, 0, 1),
            ]
        );
        let vs = Ask {
            vs: Some("KIT"),
            ..Ask::default()
        };
        assert!(vs.admits(&games[6], None));
        assert!(!vs.admits(&games[0], None));
        let with_voided = Ask {
            voided: true,
            ..Ask::default()
        };
        let rated = Ask {
            rated: true,
            ..with_voided
        };
        let mut rated_voided = games[5].clone();
        rated_voided.rated = true;
        assert!(!Ask::default().admits(&games[5], None));
        assert!(with_voided.admits(&games[5], None));
        assert!(!rated.admits(&rated_voided, None));
    }
}
