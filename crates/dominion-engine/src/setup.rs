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
    /// or Prophecy is added automatically when the kingdom needs one and none is given; a
    /// game has one of each at most.
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
    /// Choices setup otherwise makes at random, by the card that makes them: the unused
    /// Kingdom card Young Witch (its Bane), Ferryman, Way of the Mouse, Riverboat and
    /// Approaching Army add; the Black Market deck (top first); Druid's three Boons; and
    /// the Kingdom pile Obelisk or each Trait goes on. Any card the rules allow can be
    /// configured, including the removed cards a random choice never adds. The Ally and
    /// Prophecy are chosen by listing them in `landscapes`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub setup_choices: BTreeMap<Card, Vec<Card>>,
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
            setup_choices: BTreeMap::new(),
        }
    }

    /// Deals a random kingdom from `sets`: 10 Kingdom piles plus up to two landscapes,
    /// dealt from one shuffled stack of randomizers, as in the rulebook.
    pub fn random(players: u8, sets: &[Set], seed: u64) -> GameConfig {
        GameConfig::deal(
            players,
            &DealRules {
                sets: sets.to_vec(),
                ..DealRules::default()
            },
            seed,
        )
    }

    /// Deals a kingdom by `rules`: the piles and landscapes they require, then one
    /// shuffled stack of randomizers from `rules.sets` fills the rest up to 10 Kingdom
    /// piles, a landscape going to a free slot that takes its kind as it comes up.
    pub fn deal(players: u8, rules: &DealRules, seed: u64) -> GameConfig {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x9e37_79b9_7f4a_7c15);
        let mut stack: Vec<Card> = Card::ALL
            .iter()
            .copied()
            .filter(|c| {
                (rules.sets.contains(&c.set()) || rules.from_sets.contains(&c.set()))
                    && !c.def().removed
            })
            .filter(|c| c.def().kingdom || SLOT_KINDS.contains(&c.kind()))
            .collect();
        stack.shuffle(&mut rng);
        let mut config = GameConfig::new(players, rules.kingdom.clone(), seed);
        config.landscapes = rules.landscapes.clone();
        let free = |c: &Card, config: &GameConfig| {
            !rules.banned.contains(c)
                && !config.kingdom.contains(c)
                && !config.landscapes.contains(c)
        };
        for &set in &rules.from_sets {
            if let Some(&card) = stack
                .iter()
                .find(|c| c.def().kingdom && c.set() == set && free(c, &config))
            {
                config.kingdom.push(card);
            }
        }
        let slots = rules.slots.as_deref().unwrap_or(&DEFAULT_SLOTS);
        let mut dealt: Vec<Card> = vec![];
        for card in stack {
            let full = config.kingdom.len() >= KINGDOM_PILES;
            if full && assign(&dealt, slots, true) {
                break;
            }
            if !rules.sets.contains(&card.set()) || !free(&card, &config) || dealt.contains(&card) {
                continue;
            }
            if card.def().kingdom {
                if !full {
                    config.kingdom.push(card);
                }
            } else {
                dealt.push(card);
                if !assign(&dealt, slots, false) {
                    dealt.pop();
                }
            }
        }
        config.landscapes.extend(dealt);
        config
    }
}

/// Kingdom piles in a dealt kingdom.
pub const KINGDOM_PILES: usize = 10;

/// The landscapes a slot can be dealt.
pub const SLOT_KINDS: [Kind; 5] = [
    Kind::Event,
    Kind::Landmark,
    Kind::Project,
    Kind::Way,
    Kind::Trait,
];

/// A slot's `kinds` taking every landscape a slot can.
pub const ANY_SLOT_KIND: u8 = (1 << SLOT_KINDS.len()) - 1;

/// The rulebook's two slots: any landscape, then any but a Way; either may stay empty.
/// Together: up to two landscapes, at most one of them a Way.
pub const DEFAULT_SLOTS: [LandscapeSlot; 2] = [
    LandscapeSlot {
        kinds: ANY_SLOT_KIND,
        optional: true,
    },
    LandscapeSlot {
        kinds: ANY_SLOT_KIND & !(1 << 3),
        optional: true,
    },
];

/// What a kingdom is dealt from and must contain.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DealRules {
    /// Sets the random piles and landscapes come from.
    pub sets: Vec<Set>,
    /// Kingdom piles the kingdom must have.
    pub kingdom: Vec<Card>,
    /// One random Kingdom pile from each of these sets, whether or not it is in `sets`.
    pub from_sets: Vec<Set>,
    /// Landscapes the game must have, besides the slots.
    pub landscapes: Vec<Card>,
    /// Slots for random landscapes; `None` is [`DEFAULT_SLOTS`].
    pub slots: Option<Vec<LandscapeSlot>>,
    /// Never dealt at random.
    pub banned: Vec<Card>,
}

/// A place for one random landscape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandscapeSlot {
    /// Bit `i` set: it takes `SLOT_KINDS[i]`.
    pub kinds: u8,
    /// It may stay empty; otherwise the deal goes on until it is filled.
    pub optional: bool,
}

impl LandscapeSlot {
    pub fn takes(&self, kind: Kind) -> bool {
        SLOT_KINDS
            .iter()
            .position(|&k| k == kind)
            .is_some_and(|i| self.kinds & (1 << i) != 0)
    }
}

/// Whether `cards` go to different slots that take them, leaving no slot that must be
/// filled empty when `fill` is set.
fn assign(cards: &[Card], slots: &[LandscapeSlot], fill: bool) -> bool {
    fn go(cards: &[Card], slots: &[LandscapeSlot], used: &mut Vec<bool>, fill: bool) -> bool {
        let Some((first, rest)) = cards.split_first() else {
            return !fill || slots.iter().zip(used.iter()).all(|(s, &u)| u || s.optional);
        };
        for i in 0..slots.len() {
            if !used[i] && slots[i].takes(first.kind()) {
                used[i] = true;
                if go(rest, slots, used, fill) {
                    return true;
                }
                used[i] = false;
            }
        }
        false
    }
    go(cards, slots, &mut vec![false; slots.len()], fill)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SetupError {
    #[error("Dominion supports 2 to 6 players, got {0}")]
    PlayerCount(u8),
    #[error("{0} is not a Kingdom pile")]
    NotKingdom(Card),
    #[error("{0} is not implemented and can't be in a game")]
    Unimplemented(Card),
    #[error("{0} is not a landscape")]
    NotLandscape(Card),
    #[error("{0} appears more than once")]
    Duplicate(Card),
    #[error("a game has one Ally and one Prophecy at most, got {0} and {1}")]
    OnlyOne(Card, Card),
    #[error("{choice} is not a legal setup choice for {by}")]
    InvalidChoice { by: Card, choice: Card },
    #[error("wrong number of setup choices for {0}")]
    ChoiceCount(Card),
    #[error("a setup choice is given for {0}, which makes none in this game")]
    UnusedChoice(Card),
    #[error("a game has at most {MAX_PILES} piles, Supply and non-Supply")]
    TooManyPiles,
}

/// The most piles setup makes: what joins mid-game (Divine Wind's new Kingdom piles and
/// the piles they need) must still fit in a [`PileId`].
pub(crate) const MAX_PILES: usize = 128;

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
        if card.def().unimplemented {
            return Err(SetupError::Unimplemented(*card));
        }
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
        let kind = card.kind();
        if matches!(kind, Kind::Ally | Kind::Prophecy)
            && let Some(other) = config.landscapes[..i].iter().find(|l| l.kind() == kind)
        {
            return Err(SetupError::OnlyOne(*other, *card));
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
        last_gain: None,
        trashing: vec![],
        plays: vec![],
        leaving: vec![],
        last_shuffle: vec![],
        holds: vec![],
        emptied_depth: 0,
        ext: Ext::default(),
        coins_as_cards: None,
        stack: vec![],
        pending: None,
        log: vec![],
        scribe: Default::default(),
        outcome: None,
        trace: None,
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
    // Five and six players use two sets of base Treasures.
    let sets = if n > 4 { 2 } else { 1 };
    let mut basics = vec![
        (Card::Copper, 60 * sets - 7 * n),
        (Card::Silver, 40 * sets),
        (Card::Gold, 30 * sets),
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
        if g.piles.len() >= MAX_PILES {
            return Err(SetupError::TooManyPiles);
        }
        add_kingdom_pile(&mut g, card);
    }

    let s = complete(&mut g, Mark::default());
    if let Some(err) = s.error {
        return Err(err);
    }
    if g.piles.len() > MAX_PILES {
        return Err(SetupError::TooManyPiles);
    }
    if let Some(&by) = g
        .config
        .setup_choices
        .keys()
        .find(|by| !s.made.contains(by))
    {
        return Err(SetupError::UnusedChoice(by));
    }

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

/// Setup in progress: what joined the game, and the configured choices made.
#[derive(Debug, Default)]
pub(crate) struct Setup {
    joined: Vec<Card>,
    made: Vec<Card>,
    error: Option<SetupError>,
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

    /// The choice `by` makes among `candidates`, every card the rules allow: the configured
    /// one, or a random one.
    pub(crate) fn choose(&mut self, g: &mut Game, by: Card, candidates: Vec<Card>) -> Option<Card> {
        self.choose_n(g, by, candidates, 1..=1).pop()
    }

    /// Like [`Setup::choose_n`], with only the candidates `random` accepts chosen at random.
    pub(crate) fn choose_or_random(
        &mut self,
        g: &mut Game,
        by: Card,
        candidates: Vec<Card>,
        n: RangeInclusive<usize>,
        random: impl Fn(Card) -> bool,
    ) -> Vec<Card> {
        let pool = candidates.iter().copied().filter(|&c| random(c)).collect();
        self.pick(g, by, &candidates, pool, n)
    }

    /// Rejects the configured choice `choice` of `by`, which breaks a rule between choices.
    pub(crate) fn reject(&mut self, by: Card, choice: Card) {
        self.error
            .get_or_insert(SetupError::InvalidChoice { by, choice });
    }

    /// The different cards `by` chooses among `candidates`: the configured ones (as many as
    /// `n` allows), or as many as possible up to `n.end()` at random.
    pub(crate) fn choose_n(
        &mut self,
        g: &mut Game,
        by: Card,
        candidates: Vec<Card>,
        n: RangeInclusive<usize>,
    ) -> Vec<Card> {
        let pool = candidates.clone();
        self.pick(g, by, &candidates, pool, n)
    }

    /// The configured choice of `by` if it is among `legal`, or random cards from `pool`.
    fn pick(
        &mut self,
        g: &mut Game,
        by: Card,
        legal: &[Card],
        mut pool: Vec<Card>,
        n: RangeInclusive<usize>,
    ) -> Vec<Card> {
        self.made.push(by);
        if let Some(chosen) = g.config.setup_choices.get(&by).cloned() {
            let err = if let Some(&c) = chosen.iter().find(|c| c.def().unimplemented) {
                Some(SetupError::Unimplemented(c))
            } else if n.contains(&chosen.len()) {
                chosen
                    .iter()
                    .enumerate()
                    .find(|&(i, c)| !legal.contains(c) || chosen[..i].contains(c))
                    .map(|(_, &choice)| SetupError::InvalidChoice { by, choice })
            } else {
                Some(SetupError::ChoiceCount(by))
            };
            match err {
                None => return chosen,
                Some(err) => {
                    self.error.get_or_insert(err);
                }
            }
        }
        // A random choice adds no card removed in a second edition (as no random kingdom
        // has one), and a card configured for another choice isn't free for it.
        let excluded = |c: &Card| {
            g.home_pile(*c).is_none()
                && (c.def().removed || g.config.setup_choices.values().flatten().any(|r| r == c))
        };
        pool.retain(|c| !excluded(c));
        g.shuffle(&mut pool);
        pool.truncate(*n.end());
        pool
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
    let id = PileId(g.piles.len().try_into().expect("setup bounds the piles"));
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
    let id = g.add_pile(Pile::new(card, in_supply));
    for _ in 0..size {
        g.create(card, Zone::Pile(id));
    }
    id
}

/// Adds a pile named `name` holding `cards`, listed top first.
pub(crate) fn add_mixed_pile(g: &mut Game, name: Card, cards: &[Card], in_supply: bool) -> PileId {
    let id = g.add_pile(Pile::new(name, in_supply));
    for &card in cards.iter().rev() {
        g.create(card, Zone::Pile(id));
    }
    id
}
