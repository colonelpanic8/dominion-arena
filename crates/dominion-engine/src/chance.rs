//! Chance: every random outcome a game draws, named by what it decides ([`Site`]).
//!
//! A game draws its outcomes from [`Game::rng`], or, started with [`Game::new_given`],
//! from outcomes it is given: then it takes each site's outcomes in order, refuses one
//! that doesn't fit what it is drawing ([`Game::chance_refused`]), and goes on with its
//! generator after a refusal. A journaled game journals every outcome no `shuffle` event
//! shows ([`crate::Step::Chance`]).

use crate::{Card, CardId, Game, PlayerId};
use rand::Rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// What a chance outcome decides. An outcome is cards: in order, top first for a deck or
/// pile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Site {
    /// Cards shuffled into a player's deck: the opening deck, a discard pile, a whole deck.
    Deck(PlayerId),
    /// The Boons' (or Hexes') discard pile shuffled into their deck.
    Reshuffle {
        hexes: bool,
    },
    /// The Boons' (or Hexes') deck as setup makes it.
    Fates {
        hexes: bool,
    },
    /// A shuffled pile as setup makes it, named by its card.
    Pile(Card),
    /// A choice setup makes at random, by the card that makes it.
    Choice(Card),
    Ally,
    Prophecy,
    /// The Kingdom piles Divine Wind sets up, in order.
    Kingdom,
}

impl Site {
    /// Whether a `shuffle` event shows its outcome.
    pub fn shuffled(self) -> bool {
        matches!(self, Site::Deck(_) | Site::Reshuffle { .. })
    }
}

/// An outcome to give a game, with where it was recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Given {
    pub cards: Vec<Card>,
    /// The event it was read from, for telling where a refused one came from.
    pub from: Option<u64>,
}

/// Outcomes to give a game instead of drawing them: each site's in the order the game
/// draws them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcomes {
    queues: Vec<(Site, VecDeque<Given>)>,
}

impl Outcomes {
    pub fn push(&mut self, site: Site, cards: Vec<Card>, from: Option<u64>) {
        let given = Given { cards, from };
        match self.queues.iter_mut().find(|(s, _)| *s == site) {
            Some((_, q)) => q.push_back(given),
            None => self.queues.push((site, VecDeque::from([given]))),
        }
    }

    fn take(&mut self, site: Site) -> Option<Given> {
        let (_, q) = self.queues.iter_mut().find(|(s, _)| *s == site)?;
        q.pop_front()
    }

    /// How many outcomes have not been drawn.
    pub fn left(&self) -> usize {
        self.queues.iter().map(|(_, q)| q.len()).sum()
    }
}

/// An outcome a game given its outcomes would not take, or one it found none of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    pub site: Site,
    /// The outcome refused; `None` when none was left for the site.
    pub given: Option<Given>,
    /// What the game was drawing: the cards being shuffled, or what it was picking from.
    pub drawing: Vec<Card>,
}

/// What a journaled or given game keeps of chance.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Chance {
    /// What setup decided: Platinum and Colony, Shelters.
    pub colonies: bool,
    pub shelters: bool,
    /// Outcomes drawn before the journal began.
    pub drawn: Vec<(Site, Vec<Card>)>,
    pub given: Option<Outcomes>,
    pub refused: Option<Refused>,
}

/// Something shuffled, known by its name.
pub(crate) trait Named: Copy {
    fn named(self, g: &Game) -> Card;
}

impl Named for CardId {
    fn named(self, g: &Game) -> Card {
        g.cards[self.index()]
    }
}

impl Named for Card {
    fn named(self, _: &Game) -> Card {
        self
    }
}

/// Whether `cards` is `pool`'s cards in some order.
fn same_cards(mut cards: Vec<Card>, mut pool: Vec<Card>) -> bool {
    cards.sort();
    pool.sort();
    cards == pool
}

/// Whether `out` is `want` different cards of `pool`.
pub(crate) fn picks(out: &[Card], pool: &[Card], want: usize) -> bool {
    out.len() == want
        && out.iter().all(|c| pool.contains(c))
        && out.iter().enumerate().all(|(i, c)| !out[..i].contains(c))
}

impl Game {
    /// Starts a journaled game whose chance outcomes are `outcomes` rather than drawn.
    pub fn new_given(
        config: crate::GameConfig,
        outcomes: Outcomes,
    ) -> Result<Game, crate::SetupError> {
        let chance = Chance {
            given: Some(outcomes),
            ..Chance::default()
        };
        let mut game = crate::setup::build_with(config, Some(chance))?;
        game.enable_journal();
        game.start();
        Ok(game)
    }

    /// The first outcome a game given its outcomes refused, or lacked.
    pub fn chance_refused(&self) -> Option<&Refused> {
        self.chance.as_ref()?.refused.as_ref()
    }

    /// The outcomes given and not yet drawn.
    pub fn chance_left(&self) -> Option<&Outcomes> {
        self.chance.as_ref()?.given.as_ref()
    }

    /// Whether setup used Platinum and Colony, and Shelters, for a journaled or given game.
    pub fn colonies_and_shelters(&self) -> Option<(bool, bool)> {
        self.chance.as_ref().map(|c| (c.colonies, c.shelters))
    }

    /// Whether the game takes its outcomes rather than drawing them.
    pub(crate) fn given_chance(&self) -> bool {
        self.chance.as_ref().is_some_and(|c| c.given.is_some())
    }

    /// The next outcome given for `site`: `None` for a game that draws its own, `Some(Err)`
    /// when none is left.
    fn next_given(&mut self, site: Site) -> Option<Result<Given, ()>> {
        let given = self.chance.as_mut()?.given.as_mut()?;
        Some(given.take(site).ok_or(()))
    }

    fn refuse(&mut self, site: Site, given: Option<Given>, drawing: Vec<Card>) {
        if let Some(chance) = &mut self.chance {
            // Once one is refused, the game is no longer the one given: it draws its own.
            chance.given = None;
            chance.refused.get_or_insert(Refused {
                site,
                given,
                drawing,
            });
        }
    }

    fn record_chance(&mut self, site: Site, cards: Vec<Card>) {
        if site.shuffled() {
            return;
        }
        if self.journal.is_some() {
            self.journal_chance(site, cards);
        } else if let Some(chance) = &mut self.chance {
            chance.drawn.push((site, cards));
        }
    }

    /// Shuffles `items` (listed bottom first, as the engine keeps decks) for `site`. Fewer
    /// than two have one order: nothing is drawn, given or recorded.
    pub(crate) fn shuffle_for<T: Named>(&mut self, site: Site, items: &mut [T]) {
        if items.len() < 2 {
            return;
        }
        let names = |g: &Game, items: &[T]| items.iter().rev().map(|i| i.named(g)).collect();
        match self.next_given(site) {
            None => self.rng_shuffle(items),
            Some(given) => {
                let drawing: Vec<Card> = names(self, items);
                match given {
                    Ok(given) if same_cards(given.cards.clone(), drawing.clone()) => {
                        let mut left: Vec<T> = items.to_vec();
                        let mut top_first = Vec::with_capacity(left.len());
                        for &card in &given.cards {
                            let i = (left.iter())
                                .position(|i| i.named(self) == card)
                                .expect("the same cards");
                            top_first.push(left.remove(i));
                        }
                        for (slot, item) in items.iter_mut().zip(top_first.into_iter().rev()) {
                            *slot = item;
                        }
                    }
                    given => {
                        self.refuse(site, given.ok(), drawing);
                        self.rng_shuffle(items);
                    }
                }
            }
        }
        if self.chance.is_some() {
            let cards = names(self, items);
            self.record_chance(site, cards);
        }
    }

    /// The outcome of `draw`, a random procedure, for `site`. Given its outcomes, the game
    /// takes the next one instead if it `fits`. A procedure on fewer than two cards
    /// (`drawing`) has one outcome: it is drawn, and neither given nor recorded.
    pub(crate) fn chance_of(
        &mut self,
        site: Site,
        fits: impl Fn(&[Card]) -> bool,
        drawing: Vec<Card>,
        draw: impl FnOnce(&mut Game) -> Vec<Card>,
    ) -> Vec<Card> {
        if drawing.len() < 2 {
            return draw(self);
        }
        let cards = match self.next_given(site) {
            None => draw(self),
            Some(Ok(given)) if fits(&given.cards) => given.cards,
            Some(given) => {
                self.refuse(site, given.ok(), drawing);
                draw(self)
            }
        };
        if self.chance.is_some() {
            self.record_chance(site, cards.clone());
        }
        cards
    }

    /// One of `cards` (at least one) at random, for `site`.
    pub(crate) fn chance_pick(&mut self, site: Site, cards: &[Card]) -> Card {
        if let [card] = cards {
            return *card;
        }
        let fits = |out: &[Card]| picks(out, cards, 1);
        let picked = self.chance_of(site, fits, cards.to_vec(), |g| {
            vec![cards[g.rng_index(cards.len())]]
        });
        picked[0]
    }

    /// Shuffles with the generator: only for drawing an outcome of a [`Site`].
    pub(crate) fn rng_shuffle<T>(&mut self, items: &mut [T]) {
        items.shuffle(&mut self.rng);
    }

    /// A number below `n` from the generator: only for drawing an outcome of a [`Site`].
    pub(crate) fn rng_index(&mut self, n: usize) -> usize {
        self.rng.random_range(0..n)
    }
}
