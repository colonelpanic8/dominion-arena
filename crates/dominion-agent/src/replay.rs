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
///
/// A successful replay establishes that this engine build and this recorder, given the
/// stream's setup, its answers and its chance outcomes, write this stream, event for
/// event, but for `setup.config.seed`, which is provenance the replay neither uses nor
/// checks. It does not establish that the stream is what was played: any stream this
/// build would write for some answers and outcomes replays, whoever wrote it, and a
/// faithful recording from another build (other wording, option order, merging of moves,
/// card data) may not.
///
/// The game returned is the replayed position, for reading. It has used up the outcomes
/// it was given and its generator has drawn none of them, so a game continued from it
/// needs a source of chance of its own; and what a host keeps beside the stream (decision
/// ids, Way questions answered in advance, where each undoable command began) is not in
/// it.
///
/// Differences are reported in stream order: a malformed outcome is reported at its event
/// only if the game gets that far. Every round of the loop consumes stream events or
/// fails, so no stream can hang it.
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
    let (outcomes, malformed) = outcomes_of(events);
    let mut game = Game::new_given(config, outcomes)
        .map_err(|e| differ(0, None, format!("setup fails: {e}")))?;
    let seats = &setup.seats;
    let mut recorder = Recorder::new();
    let mut at = 0u64;
    let mut acted_at = None;
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
        let bad = |at: u64| {
            (malformed.as_ref())
                .filter(|(seq, _)| *seq <= at)
                .map(|(seq, why)| differ(*seq, None, why.clone()))
        };
        for event in produced {
            if let Some(bad) = bad(at) {
                return Err(bad);
            }
            if !same(events.get(at as usize), &event) {
                return Err(match refused {
                    Some((Some(seq), why)) if seq <= at => differ(seq, None, why),
                    Some((None, why)) => differ(at, Some(event), why),
                    _ => differ(at, Some(event), "the game goes differently".into()),
                });
            }
            at += 1;
        }
        if let Some(bad) = bad(at) {
            return Err(bad);
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
        // What was done at the last event must have written it, and more.
        if acted_at == Some(at) {
            return Err(differ(at, None, "the game made no progress".into()));
        }
        acted_at = Some(at);
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
                let seats = game.num_players();
                let seat = match end.result.iter().find(|r| r.resigned) {
                    Some(r) if usize::from(r.seat) < seats => r.seat,
                    Some(r) => {
                        let why = format!("seat {} resigns, of {seats}", r.seat);
                        return Err(differ(at, None, why));
                    }
                    None => {
                        let why = "a resignation with nobody resigning".into();
                        return Err(differ(at, None, why));
                    }
                };
                game.resign(PlayerId(seat));
            }
            _ => {
                let why = format!("the game waits on seat {}'s answer", d.player.0);
                return Err(differ(at, None, why));
            }
        }
    }
}

/// Whether the game wrote what the stream recorded there: equal, but for the seed.
fn same(recorded: Option<&Event>, produced: &Event) -> bool {
    match (recorded.map(|e| &e.what), &produced.what) {
        (Some(What::Setup(r)), What::Setup(p)) => {
            let mut p = p.clone();
            if let (Some(rc), Some(pc)) = (&r.config, &mut p.config) {
                pc.seed = rc.seed;
            }
            recorded.map(|e| (e.seq, e.by)) == Some((produced.seq, produced.by)) && **r == *p
        }
        _ => recorded == Some(produced),
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

/// Every chance outcome the stream holds, by what it decides, in the stream's order, up to
/// the first that is malformed: that one's event, and why.
fn outcomes_of(events: &[Event]) -> (Outcomes, Option<(u64, String)>) {
    let mut outcomes = Outcomes::default();
    for e in events {
        if let Err(why) = outcomes_in(e, &mut outcomes) {
            return (outcomes, Some((e.seq, why)));
        }
    }
    (outcomes, None)
}

/// Adds the chance outcomes event `e` records.
fn outcomes_in(e: &Event, outcomes: &mut Outcomes) -> Result<(), String> {
    let mut push = |site: Site, names: &[String]| -> Result<(), String> {
        outcomes.push(site, cards(names)?, Some(e.seq));
        Ok(())
    };
    match &e.what {
        What::Setup(setup) => {
            for chance in &setup.chance {
                push(site_of(chance)?, &chance.cards)?;
            }
        }
        What::Chance(chance) => push(site_of(chance)?, &chance.cards)?,
        // Fewer than two cards have one order: no outcome.
        What::Shuffle(m) if m.cards.len() < 2 => {}
        What::Shuffle(m) => {
            let site = match m.to {
                Zone::Seat(p, SeatZone::Deck) => Site::Deck(PlayerId(p)),
                Zone::Boons(Pack::Deck) => Site::Reshuffle { hexes: false },
                Zone::Hexes(Pack::Deck) => Site::Reshuffle { hexes: true },
                ref to => return Err(format!("a shuffle into {to}")),
            };
            let names = (m.cards.iter())
                .map(|c| match c {
                    CardRef::Name(name) => Ok(name.clone()),
                    CardRef::Hidden => {
                        Err("a card face down: only a full stream can be replayed".to_string())
                    }
                })
                .collect::<Result<Vec<String>, _>>()?;
            push(site, &names)?;
        }
        _ => {}
    }
    Ok(())
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
