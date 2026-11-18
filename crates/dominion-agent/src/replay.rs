//! A game played again from its full `dominion/events@1` stream alone: set up from the
//! stream's `setup`, its chance outcomes taken from the stream, each seat's answers given
//! as recorded. The game records a stream of its own as it goes, which must be the same,
//! event for event.

use crate::events::Recorder;
use crate::project;
use dominion_engine::{Card, Game, GameConfig, Outcomes, PlayerId, Site};
use dominion_protocol::events::{
    CardRef, Chance, ChanceSite, Event, Pack, SeatZone, Setup, What, Zone,
};
use std::fmt;

/// A game replayed to the end of its stream: over, or waiting on the question the stream
/// ends with.
#[derive(Clone, Debug)]
pub struct Replay {
    pub game: Game,
    /// The recorder that wrote the stream again, to go on recording the game.
    pub recorder: Recorder,
}

/// Where a replay first went differently from the stream it replayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    /// The first event that differs.
    pub seq: u64,
    /// The stream's event there, if it has one.
    pub recorded: Option<Event>,
    /// What the game did there instead, if anything.
    pub produced: Option<Event>,
    pub why: String,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = |e: &Option<Event>| match e {
            Some(e) => serde_json::to_string(e).unwrap_or_else(|e| e.to_string()),
            None => "nothing".into(),
        };
        write!(
            f,
            "event {}: {}\nrecorded: {}\nproduced: {}",
            self.seq,
            self.why,
            json(&self.recorded),
            json(&self.produced)
        )
    }
}

impl std::error::Error for Difference {}

/// Plays the game `events` (a full stream, from its `setup`) records again: the game as
/// it stands after the last event, or the first event where the game goes differently.
pub fn replay(events: &[Event]) -> Result<Replay, Box<Difference>> {
    let differ = |seq: u64, produced: Option<Event>, why: String| {
        Box::new(Difference {
            seq,
            recorded: events.get(seq as usize).cloned(),
            produced,
            why,
        })
    };
    let setup = match events.first().map(|e| &e.what) {
        Some(What::Setup(setup)) => setup,
        _ => return Err(differ(0, None, "a stream begins with its setup".into())),
    };
    let config = config_of(setup).map_err(|why| differ(0, None, why))?;
    let outcomes = outcomes_of(events).map_err(|(seq, why)| differ(seq, None, why))?;
    let mut game = Game::new_given(config, outcomes)
        .map_err(|e| differ(0, None, format!("setup fails: {e}")))?;
    let seats = &setup.seats;
    let mut recorder = Recorder::new();
    let mut at = 0u64;
    loop {
        let produced = recorder
            .record(&mut game, seats)
            .map_err(|e| differ(e.seq, None, format!("the replay can't be recorded: {e}")))?;
        let refused = game.chance_refused().map(|r| {
            let seq = r.given.as_ref().and_then(|g| g.from);
            let given = r.given.as_ref().map(|g| names(&g.cards));
            let why = match given {
                Some(given) => format!(
                    "the outcome recorded for {:?}, {given}, doesn't fit what the game drew \
                     from: {}",
                    r.site,
                    names(&r.drawing)
                ),
                None => format!("the stream has no outcome for {:?}", r.site),
            };
            (seq, why)
        });
        for event in produced {
            if events.get(at as usize) != Some(&event) {
                return Err(match refused {
                    Some((Some(seq), why)) if seq <= at => differ(seq, None, why),
                    Some((None, why)) => differ(at, Some(event), why),
                    _ => differ(at, Some(event), "the game goes differently".into()),
                });
            }
            at += 1;
        }
        if let Some((seq, why)) = refused {
            return Err(differ(seq.unwrap_or(at), None, why));
        }
        if game.outcome().is_some() {
            if at < events.len() as u64 {
                return Err(differ(at, None, "the game is over".into()));
            }
            return Ok(Replay { game, recorder });
        }
        let Some(next) = events.get(at as usize) else {
            return Ok(Replay { game, recorder });
        };
        let d = game.pending().expect("a game not over waits on a decision");
        match &next.what {
            What::Answer(a) if a.seat == d.player.0 => {
                let response =
                    project::recorded_response(&game, d, seats, a.indices.clone(), a.name.clone())
                        .and_then(|r| d.validate(&r).map(|()| r).map_err(|e| e.to_string()))
                        .map_err(|why| {
                            differ(at, None, format!("the answer doesn't fit: {why}"))
                        })?;
                game.respond(response).expect("validated");
            }
            What::GameEnd(end) if end.reason == "resigned" => {
                let Some(seat) = end.result.iter().find(|r| r.resigned) else {
                    return Err(differ(
                        at,
                        None,
                        "a resignation with nobody resigning".into(),
                    ));
                };
                game.resign(PlayerId(seat.seat));
            }
            _ => {
                let why = format!("the game waits on seat {}'s answer", d.player.0);
                return Err(differ(at, None, why));
            }
        }
    }
}

fn card(name: &str) -> Result<Card, String> {
    Card::from_name(name)
        .filter(|c| c.name() == name)
        .ok_or_else(|| format!("no card is called {name:?}"))
}

fn cards(names: &[String]) -> Result<Vec<Card>, String> {
    names.iter().map(|n| card(n)).collect()
}

fn names(cards: &[Card]) -> String {
    let names: Vec<&str> = cards.iter().map(|c| c.name()).collect();
    format!("[{}]", names.join(", "))
}

/// The configuration `setup` says the game was set up from.
fn config_of(setup: &Setup) -> Result<GameConfig, String> {
    let c = (setup.config.as_ref())
        .ok_or("the setup doesn't say what the game was set up from (recorded before it did)")?;
    let players = u8::try_from(setup.seats.len()).map_err(|_| "too many seats")?;
    let mut config = GameConfig::new(players, cards(&setup.kingdom)?, c.seed.unwrap_or(0));
    config.landscapes = cards(&c.landscapes)?;
    config.turn_limit = c.turn_limit;
    config.colonies = Some(c.colonies);
    config.shelters = Some(c.shelters);
    for (by, chosen) in &c.choices {
        config.setup_choices.insert(card(by)?, cards(chosen)?);
    }
    Ok(config)
}

/// Every chance outcome the stream holds, by what it decides, in the stream's order.
fn outcomes_of(events: &[Event]) -> Result<Outcomes, (u64, String)> {
    let mut outcomes = Outcomes::default();
    for e in events {
        let fail = |why: String| (e.seq, why);
        let mut push = |site: Site, names: &[String]| -> Result<(), (u64, String)> {
            outcomes.push(site, cards(names).map_err(fail)?, Some(e.seq));
            Ok(())
        };
        match &e.what {
            What::Setup(setup) => {
                for chance in &setup.chance {
                    push(site_of(chance).map_err(fail)?, &chance.cards)?;
                }
            }
            What::Chance(chance) => push(site_of(chance).map_err(fail)?, &chance.cards)?,
            What::Shuffle(m) => {
                let site = match m.to {
                    Zone::Seat(p, SeatZone::Deck) => Site::Deck(PlayerId(p)),
                    Zone::Boons(Pack::Deck) => Site::Reshuffle { hexes: false },
                    Zone::Hexes(Pack::Deck) => Site::Reshuffle { hexes: true },
                    ref to => return Err(fail(format!("a shuffle into {to}"))),
                };
                let names = (m.cards.iter())
                    .map(|c| match c {
                        CardRef::Name(name) => Ok(name.clone()),
                        CardRef::Hidden => Err(fail(
                            "a card face down: only a full stream \
                                                     can be replayed"
                                .into(),
                        )),
                    })
                    .collect::<Result<Vec<String>, _>>()?;
                push(site, &names)?;
            }
            _ => {}
        }
    }
    Ok(outcomes)
}

fn site_of(chance: &Chance) -> Result<Site, String> {
    let named = || {
        let name = chance
            .card
            .as_deref()
            .ok_or("a chance outcome without its card")?;
        card(name)
    };
    Ok(match chance.site {
        ChanceSite::Boons => Site::Fates { hexes: false },
        ChanceSite::Hexes => Site::Fates { hexes: true },
        ChanceSite::Pile => Site::Pile(named()?),
        ChanceSite::Choice => Site::Choice(named()?),
        ChanceSite::Ally => Site::Ally,
        ChanceSite::Prophecy => Site::Prophecy,
        ChanceSite::Kingdom => Site::Kingdom,
    })
}
