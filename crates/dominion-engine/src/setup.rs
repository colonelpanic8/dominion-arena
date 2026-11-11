//! Game configuration and setup.

use crate::cards::{self, Ext};
use crate::game::{CardDeck, Pile, Player, Turn, Zone};
use crate::{Card, Game, Kind, PileId, PlayerId, Set, Types};
use rand::SeedableRng;
use rand::seq::{IndexedRandom, SliceRandom};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::ops::RangeInclusive;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameConfig {
    pub players: u8,
    /// Kingdom piles, each named by its card, top card (split piles) or randomizer
    /// (Knights, Castles, Augurs, ...). Usually 10.
    pub kingdom: Vec<Card>,
    /// Events, Landmarks, Projects, Ways, Traits and other landscapes in the game. An Ally
    /// or Prophecy is added automatically when the kingdom needs one and none is given.
    pub landscapes: Vec<Card>,
    pub seed: u64,
    /// End the game once this many turns have been taken in total.
    pub turn_limit: Option<u32>,
    /// Use Platinum and Colony. `None` follows the usual rule (a random Kingdom card is
    /// from Prosperity).
    pub colonies: Option<bool>,
    /// Use Shelters instead of starting Estates. `None` follows the usual rule (a random
    /// Kingdom card is from Dark Ages).
    pub shelters: Option<bool>,
}

impl GameConfig {
    pub fn new(players: u8, kingdom: Vec<Card>, seed: u64) -> GameConfig {
        GameConfig {
            players,
            kingdom,
            landscapes: vec![],
            seed,
            turn_limit: None,
            colonies: None,
            shelters: None,
        }
    }

    /// Deals a random kingdom from `sets`: 10 Kingdom piles plus up to two landscapes,
    /// dealt from one shuffled stack of randomizers, as in the rulebook.
    pub fn random(players: u8, sets: &[Set], seed: u64) -> GameConfig {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x9e37_79b9_7f4a_7c15);
        let mut stack: Vec<Card> = Card::ALL
            .iter()
            .copied()
            .filter(|c| sets.contains(&c.set()) && !c.def().removed)
            .filter(|c| {
                c.def().kingdom
                    || matches!(
                        c.kind(),
                        Kind::Event | Kind::Landmark | Kind::Project | Kind::Way | Kind::Trait
                    )
            })
            .collect();
        stack.shuffle(&mut rng);
        let mut config = GameConfig::new(players, vec![], seed);
        for card in stack {
            if config.kingdom.len() == 10 {
                break;
            }
            if card.def().kingdom {
                config.kingdom.push(card);
            } else if config.landscapes.len() < 2
                && !(card.kind() == Kind::Way
                    && config.landscapes.iter().any(|l| l.kind() == Kind::Way))
            {
                config.landscapes.push(card);
            }
        }
        config
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SetupError {
    #[error("Dominion supports 2 to 6 players, got {0}")]
    PlayerCount(u8),
    #[error("{0} is not a Kingdom pile")]
    NotKingdom(Card),
    #[error("{0} is not a landscape")]
    NotLandscape(Card),
    #[error("{0} appears more than once")]
    Duplicate(Card),
}

/// Number of copies a Victory pile starts with.
pub(crate) fn victory_pile_size(players: usize) -> usize {
    if players == 2 { 8 } else { 12 }
}

/// Builds the starting position without running anything.
pub(crate) fn build(config: GameConfig) -> Result<Game, SetupError> {
    let n = config.players;
    if !(2..=6).contains(&n) {
        return Err(SetupError::PlayerCount(n));
    }
    for (i, card) in config.kingdom.iter().enumerate() {
        if !card.def().kingdom {
            return Err(SetupError::NotKingdom(*card));
        }
        if config.kingdom[..i].contains(card) {
            return Err(SetupError::Duplicate(*card));
        }
    }
    for (i, card) in config.landscapes.iter().enumerate() {
        if !card.kind().is_landscape() {
            return Err(SetupError::NotLandscape(*card));
        }
        if config.landscapes[..i].contains(card) {
            return Err(SetupError::Duplicate(*card));
        }
    }

    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let colonies = config.colonies.unwrap_or_else(|| {
        config
            .kingdom
            .choose(&mut rng)
            .is_some_and(|c| c.set() == Set::Prosperity)
    });
    let shelters = config.shelters.unwrap_or_else(|| {
        config
            .kingdom
            .choose(&mut rng)
            .is_some_and(|c| c.set() == Set::DarkAges)
    });

    let mut g = Game {
        rng,
        cards: vec![],
        zones: vec![],
        masks: BTreeMap::new(),
        players: vec![
            Player {
                journey_face_up: true,
                ..Player::default()
            };
            n as usize
        ],
        piles: vec![],
        trash: vec![],
        turn: Turn::new(PlayerId(0)),
        landscapes: config.landscapes.clone(),
        boons: CardDeck::default(),
        hexes: CardDeck::default(),
        artifacts: vec![],
        prophecy: None,
        trade_route_mat: 0,
        lasting: vec![],
        next_lasting: 0,
        extra_turns: VecDeque::new(),
        ending: None,
        final_round: VecDeque::new(),
        gains: vec![],
        trashing: vec![],
        plays: vec![],
        ext: Ext::default(),
        stack: vec![],
        pending: None,
        log: vec![],
        outcome: None,
        config,
    };

    let n = n as usize;
    let victory = victory_pile_size(n);
    let provinces = match n {
        2 => 8,
        3 | 4 => 12,
        5 => 15,
        _ => 18,
    };
    let mut basics = vec![
        (Card::Copper, 60 - 7 * n),
        (Card::Silver, 40),
        (Card::Gold, 30),
    ];
    if colonies {
        basics.push((Card::Platinum, 12));
    }
    basics.extend([
        (Card::Estate, victory),
        (Card::Duchy, victory),
        (Card::Province, provinces),
    ]);
    if colonies {
        basics.push((Card::Colony, victory));
    }
    basics.push((Card::Curse, 10 * (n - 1)));
    for (card, size) in basics {
        add_pile(&mut g, card, size, true);
    }
    for card in g.config.kingdom.clone() {
        add_kingdom_pile(&mut g, card);
    }

    complete(&mut g, Mark::default());

    let estates = if shelters {
        vec![Card::Hovel, Card::Necropolis, Card::OvergrownEstate]
    } else {
        vec![Card::Estate; 3]
    };
    let mut coppers_dealt = 0;
    for p in 0..n {
        let p = PlayerId(p as u8);
        let mut deck = vec![Card::Copper; 7];
        deck.extend(estates.iter().copied());
        cards::modify_starting_deck(&g, p, &mut deck);
        coppers_dealt += deck.iter().filter(|&&c| c == Card::Copper).count();
        for card in deck {
            g.create(card, Zone::Deck(p));
        }
        let mut cards = std::mem::take(&mut g.players[p.index()].deck);
        g.shuffle(&mut cards);
        g.players[p.index()].deck = cards;
        for _ in 0..5 {
            let top = g.players[p.index()].deck.pop().expect("10 starting cards");
            g.zones[top.index()] = Zone::Hand(p);
            g.players[p.index()].hand.push(top);
        }
    }
    // Starting Coppers something replaced (Heirlooms, Inherited) stay in the pile.
    if let Some(copper) = g.supply_pile(Card::Copper) {
        for _ in coppers_dealt..7 * n {
            g.create(Card::Copper, Zone::Pile(copper));
        }
    }
    Ok(g)
}

/// Where the cards and landscapes in the game ended at some point, so that setup can
/// tell what joined since.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Mark {
    cards: usize,
    landscapes: usize,
}

pub(crate) fn mark(g: &Game) -> Mark {
    Mark {
        cards: g.cards.len(),
        landscapes: g.landscapes.len(),
    }
}

/// Setup in progress: what joined the game.
#[derive(Debug, Default)]
pub(crate) struct Setup {
    joined: Vec<Card>,
}

impl Setup {
    /// Whether a copy of `card` (or the landscape `card`) joined the game in this setup.
    pub(crate) fn joined(&self, card: Card) -> bool {
        self.joined.contains(&card)
    }

    /// The cards and landscapes that joined the game in this setup.
    pub(crate) fn joined_cards(&self) -> &[Card] {
        &self.joined
    }

    /// The choice `by` makes among `candidates`, at random.
    pub(crate) fn choose(&mut self, g: &mut Game, by: Card, candidates: Vec<Card>) -> Option<Card> {
        self.choose_n(g, by, candidates, 1..=1).pop()
    }

    /// The different cards `by` chooses among `candidates`: as many as possible up to
    /// `n.end()`, at random.
    pub(crate) fn choose_n(
        &mut self,
        g: &mut Game,
        _by: Card,
        mut candidates: Vec<Card>,
        n: RangeInclusive<usize>,
    ) -> Vec<Card> {
        g.shuffle(&mut candidates);
        candidates.truncate(*n.end());
        candidates
    }
}

/// Completes setup for whatever joined the game since `since` (everything, at the start):
/// runs every set's `ensure` until nothing changes, then their `setup` once.
pub(crate) fn complete(g: &mut Game, since: Mark) -> Setup {
    let footprint = |g: &Game| {
        (
            g.cards.len(),
            g.piles.len(),
            g.landscapes.len(),
            g.artifacts.len(),
            g.boons.draw.len(),
            g.hexes.draw.len(),
            g.prophecy.is_some(),
        )
    };
    let mut s = Setup::default();
    for pass in 0.. {
        assert!(pass < 32, "setup does not settle");
        let before = footprint(g);
        cards::ensure(g, &mut s);
        if footprint(g) == before {
            break;
        }
    }
    let mut joined: Vec<Card> = g.cards[since.cards..]
        .iter()
        .chain(&g.landscapes[since.landscapes..])
        .copied()
        .collect();
    joined.sort();
    joined.dedup();
    s.joined = joined;
    cards::setup(g, &mut s);
    s
}

/// Whether a card matching `pred` is in the game, anywhere, or a landscape matches it.
pub(crate) fn in_game(g: &Game, pred: impl Fn(Card) -> bool) -> bool {
    g.cards.iter().chain(&g.landscapes).any(|&c| pred(c))
}

/// Whether neither a copy of the Kingdom card `card` nor its pile is in the game.
pub(crate) fn unused(g: &Game, card: Card) -> bool {
    let home = |c: Card| c.def().pile.unwrap_or(c);
    let name = home(card);
    !g.piles.iter().any(|p| p.card == name) && !g.cards.iter().any(|&c| home(c) == name)
}

/// Adds the Kingdom pile named `card` to the Supply.
pub(crate) fn add_kingdom_pile(g: &mut Game, card: Card) -> PileId {
    let id = PileId(g.piles.len() as u8);
    if !cards::build_kingdom_pile(g, card) {
        let size = if card.is(Types::VICTORY) {
            victory_pile_size(g.num_players())
        } else {
            10
        };
        add_pile(g, card, size, true);
    }
    id
}

/// Adds a pile of `size` copies of `card`.
pub(crate) fn add_pile(g: &mut Game, card: Card, size: usize, in_supply: bool) -> PileId {
    let id = PileId(g.piles.len() as u8);
    g.piles.push(Pile::new(card, in_supply));
    for _ in 0..size {
        g.create(card, Zone::Pile(id));
    }
    id
}

/// Adds a pile named `name` holding `cards`, listed top first.
pub(crate) fn add_mixed_pile(g: &mut Game, name: Card, cards: &[Card], in_supply: bool) -> PileId {
    let id = PileId(g.piles.len() as u8);
    g.piles.push(Pile::new(name, in_supply));
    for &card in cards.iter().rev() {
        g.create(card, Zone::Pile(id));
    }
    id
}
