//! Game configuration and setup.

use crate::cards::{self, Ext};
use crate::game::{CardDeck, Pile, Player, Turn, Zone};
use crate::{Card, Game, Kind, PileId, PlayerId, Set, Types};
use rand::SeedableRng;
use rand::seq::{IndexedRandom, SliceRandom};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
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
        players: vec![Player::default(); n as usize],
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
    let potions = g.config.kingdom.iter().any(|&k| {
        k.pile_members()
            .iter()
            .chain(std::iter::once(&k))
            .any(|c| c.cost().is_some_and(|c| c.potions > 0))
    });
    if potions {
        basics.push((Card::Potion, 16));
    }
    for (card, size) in basics {
        add_pile(&mut g, card, size, true);
    }

    for card in g.config.kingdom.clone() {
        if !cards::build_kingdom_pile(&mut g, card) {
            let size = if card.is(Types::VICTORY) { victory } else { 10 };
            add_pile(&mut g, card, size, true);
        }
    }

    cards::setup(&mut g);

    let estates = if shelters {
        vec![Card::Hovel, Card::Necropolis, Card::OvergrownEstate]
    } else {
        vec![Card::Estate; 3]
    };
    for p in 0..n {
        let p = PlayerId(p as u8);
        let mut deck = vec![Card::Copper; 7];
        deck.extend(estates.iter().copied());
        cards::modify_starting_deck(&g, p, &mut deck);
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
    Ok(g)
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
