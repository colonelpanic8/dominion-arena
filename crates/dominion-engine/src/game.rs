use crate::cards::{self, Ext};
use crate::decision::{Decision, Response, ResponseError};
use crate::event::Event;
use crate::frame::{Flow, Frame, Input};
use crate::setup::GameConfig;
use crate::{Card, CardId, Cost, Kind, PileId, PlayerId, Types};
use rand::Rng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// A player mat that holds cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Mat {
    Tavern,
    Island,
    NativeVillage,
    Exile,
}

/// Where a card currently is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Zone {
    Deck(PlayerId),
    Hand(PlayerId),
    Discard(PlayerId),
    InPlay(PlayerId),
    /// Set aside by an effect; the effect (usually a [`Lasting`]) tracks which cards.
    SetAside(PlayerId),
    Mat(PlayerId, Mat),
    Pile(PileId),
    Trash,
}

impl Zone {
    /// The player whose zone this is, if any.
    pub fn owner(self) -> Option<PlayerId> {
        match self {
            Zone::Deck(p)
            | Zone::Hand(p)
            | Zone::Discard(p)
            | Zone::InPlay(p)
            | Zone::SetAside(p)
            | Zone::Mat(p, _) => Some(p),
            Zone::Pile(_) | Zone::Trash => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pos {
    Top,
    Bottom,
}

/// Adventures tokens a player can place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Token {
    PlusCard,
    PlusAction,
    PlusBuy,
    PlusCoin,
    MinusCost,
    Trashing,
    Estate,
}

/// A player's cards and tokens. Deck and discard piles store the top card last.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Player {
    pub deck: Vec<CardId>,
    pub hand: Vec<CardId>,
    pub discard: Vec<CardId>,
    pub in_play: Vec<CardId>,
    pub set_aside: Vec<CardId>,
    pub mats: BTreeMap<Mat, Vec<CardId>>,
    pub vp_tokens: i32,
    pub coffers: u32,
    pub villagers: u32,
    pub favors: u32,
    pub debt: u32,
    /// Coin tokens on the Pirate Ship mat.
    pub pirate_ship: u32,
    /// Projects this player has a cube on.
    pub projects: Vec<Card>,
    /// States this player has (Lost in the Woods, Deluded, ...).
    pub states: Vec<Card>,
    /// Boons received and kept until Clean-up (The Field's Gift, ...).
    pub boons_kept: Vec<Card>,
    /// Adventures tokens and the pile each is on (`None` for ones not on a pile).
    pub tokens: BTreeMap<Token, Option<PileId>>,
    /// The -1 Card token: the next draw draws one less.
    pub minus_card: bool,
    /// The -$1 token: the next time this player gets $, they get $1 less.
    pub minus_coin: bool,
    pub journey_face_up: bool,
    pub turns_taken: u32,
    /// How many of `turns_taken` were extra turns, which don't count for the tiebreaker.
    pub extra_turns_taken: u32,
}

impl Player {
    pub fn mat(&self, mat: Mat) -> &[CardId] {
        self.mats.get(&mat).map_or(&[], Vec::as_slice)
    }

    /// Every card this player owns, wherever it is.
    pub fn owned(&self) -> impl Iterator<Item = CardId> + '_ {
        self.deck
            .iter()
            .chain(&self.hand)
            .chain(&self.discard)
            .chain(&self.in_play)
            .chain(&self.set_aside)
            .chain(self.mats.values().flatten())
            .copied()
    }
}

/// A stack of cards; the top card is last. Mixed and split piles are just stacks too.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pile {
    /// The card the pile is named after (its randomizer or top card).
    pub card: Card,
    pub cards: Vec<CardId>,
    pub in_supply: bool,
    pub vp_tokens: i32,
    pub debt_tokens: i32,
    /// Embargo's Curse tokens.
    pub embargo: u32,
    pub trade_route: bool,
    /// Adventures tokens on this pile, by owner.
    pub tokens: Vec<(PlayerId, Token)>,
    pub traits: Vec<Card>,
    /// Face down: no card in it is public, not even the top (the Black Market deck).
    pub hidden: bool,
}

impl Pile {
    pub(crate) fn new(card: Card, in_supply: bool) -> Pile {
        Pile {
            card,
            cards: vec![],
            in_supply,
            vp_tokens: 0,
            debt_tokens: 0,
            embargo: 0,
            trade_route: false,
            tokens: vec![],
            traits: vec![],
            hidden: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Start,
    Action,
    Buy,
    Night,
    Cleanup,
    End,
}

/// State that lasts for one turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Turn {
    pub player: PlayerId,
    /// Who makes this turn's decisions (differs under Possession).
    pub controller: PlayerId,
    /// The card that granted this as an extra turn, if it is one.
    pub extra: Option<Card>,
    pub phase: Phase,
    pub actions: u32,
    pub buys: u32,
    pub coins: u32,
    pub potions: u32,
    /// Bought anything (card, Event or Project) in this Buy phase (so no more Treasures).
    pub bought: bool,
    /// Go back to the Action phase from the Buy phase as soon as possible (Villa).
    pub return_to_action: bool,
    /// Every play this turn, in order. A card played twice appears twice.
    pub played: Vec<Card>,
    /// The physical cards of `played`, in the same order.
    pub played_ids: Vec<CardId>,
    pub gained: Vec<Card>,
    /// The cards in `gained` that were gained in the Buy phase.
    pub gained_in_buy_phase: Vec<Card>,
    pub bought_cards: Vec<Card>,
    pub trashed: Vec<Card>,
    /// Cards drawn in Clean-up.
    pub cleanup_draw: usize,
    /// Whether the player may buy cards (not Events or Projects) this turn.
    pub can_buy_cards: bool,
}

impl Turn {
    pub(crate) fn new(player: PlayerId) -> Turn {
        Turn {
            player,
            controller: player,
            extra: None,
            phase: Phase::Start,
            actions: 1,
            buys: 1,
            coins: 0,
            potions: 0,
            bought: false,
            return_to_action: false,
            played: vec![],
            played_ids: vec![],
            gained: vec![],
            gained_in_buy_phase: vec![],
            bought_cards: vec![],
            trashed: vec![],
            cleanup_draw: 5,
            can_buy_cards: true,
        }
    }
}

/// Free-form state an effect carries between steps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scratch {
    pub n: i32,
    pub m: i32,
    pub i: usize,
    pub cost: Cost,
    pub pick: Option<CardId>,
    pub cards: Vec<CardId>,
    pub card: Option<Card>,
    pub player: Option<PlayerId>,
    pub pile: Option<PileId>,
}

/// When a [`Lasting`] effect ends on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Until {
    /// At the end of this turn (after Clean-up).
    EndOfTurn,
    /// When that player's next turn starts (after start-of-turn effects).
    StartOfTurn(PlayerId),
    /// Only when an effect removes it (Durations remove themselves when they resolve).
    Removed,
}

/// An effect that outlives the play that created it: a Duration's future part, "this turn"
/// and "until your next turn" effects, and so on. Sets look these up to produce triggers,
/// cost changes and the like.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lasting {
    pub id: u32,
    pub card: Card,
    pub owner: PlayerId,
    /// The physical card that created it, if any.
    pub source: Option<CardId>,
    /// Cards that stay in play (are not discarded in Clean-up) while this lasts.
    pub keeps: Vec<CardId>,
    pub until: Until,
    /// The owner's [`Player::turns_taken`] when it was created.
    pub turn: u32,
    pub data: Scratch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraTurn {
    pub player: PlayerId,
    pub source: Card,
    /// Who controls it (Possession).
    pub controller: PlayerId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndReason {
    ProvincesGone,
    ColoniesGone,
    PilesEmpty,
    TurnLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub reason: EndReason,
    pub scores: Vec<i32>,
    /// Turns each player took, not counting extra turns (the tiebreaker).
    pub turns: Vec<u32>,
    /// More than one winner means a tie.
    pub winners: Vec<PlayerId>,
}

/// A deck of non-physical cards (Boons, Hexes), top last.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CardDeck {
    pub draw: Vec<Card>,
    pub discard: Vec<Card>,
}

/// A complete game: rules state plus the suspended effect stack.
///
/// Drive it by reading [`Game::pending`] and answering with [`Game::respond`] until
/// [`Game::outcome`] is set. The whole value is `Clone` and serializable at any point.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Game {
    pub(crate) config: GameConfig,
    pub(crate) cards: Vec<Card>,
    pub(crate) zones: Vec<Zone>,
    /// Cards that currently are another card (Band of Misfits, Overlord, ...), until they
    /// leave play.
    pub(crate) masks: BTreeMap<CardId, Card>,
    pub(crate) players: Vec<Player>,
    pub(crate) piles: Vec<Pile>,
    pub(crate) trash: Vec<CardId>,
    pub(crate) turn: Turn,
    pub(crate) landscapes: Vec<Card>,
    pub(crate) boons: CardDeck,
    pub(crate) hexes: CardDeck,
    /// Artifacts in the game and who holds each.
    pub(crate) artifacts: Vec<(Card, Option<PlayerId>)>,
    /// The game's Prophecy and its remaining Sun tokens.
    pub(crate) prophecy: Option<(Card, u32)>,
    pub(crate) trade_route_mat: u32,
    pub(crate) lasting: Vec<Lasting>,
    pub(crate) next_lasting: u32,
    pub(crate) extra_turns: VecDeque<ExtraTurn>,
    /// Set once the game has ended, while an extra round after its end is played.
    pub(crate) ending: Option<EndReason>,
    /// The turns of the extra round after the game ended (Fleet) still to be taken.
    pub(crate) final_round: VecDeque<PlayerId>,
    pub(crate) gains: Vec<GainState>,
    /// Cards whose Trash windows are pending or resolving, with the zone each was trashed from.
    pub(crate) trashing: Vec<(CardId, Zone)>,
    pub(crate) plays: Vec<PlayState>,
    pub(crate) ext: Ext,
    pub(crate) stack: Vec<Frame>,
    pub(crate) pending: Option<Decision>,
    pub(crate) rng: ChaCha8Rng,
    pub(crate) log: Vec<Event>,
    pub(crate) outcome: Option<Outcome>,
}

/// A gain in progress. "Would gain" triggers may change it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GainState {
    pub player: PlayerId,
    pub card: CardId,
    pub to: crate::Dest,
    pub bought: bool,
    /// Set by a replacement effect (Trader's exchange, ...): the card is not gained.
    pub cancelled: bool,
    /// Once gained by the turn's player: its position in [`Turn::gained`] ("the next card
    /// you gain", "your second gain this turn").
    pub turn_index: Option<usize>,
}

/// A play in progress. "When you play" triggers may change it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayState {
    pub player: PlayerId,
    pub card: CardId,
    /// The card whose abilities are used (differs for Inheritance-style effects).
    pub behavior: Card,
    /// Use this Way's effect instead of the card's own.
    pub way: Option<Card>,
    /// The card's own effect is replaced: do this instead (Enchantress: +1 Card +1 Action).
    pub replaced: Option<crate::Plus>,
    /// Its own effect does nothing (Highwayman).
    pub blocked: bool,
}

impl Game {
    pub fn new(config: GameConfig) -> Result<Game, crate::SetupError> {
        let mut game = crate::setup::build(config)?;
        game.start();
        Ok(game)
    }

    pub(crate) fn start(&mut self) {
        self.stack.push(Frame::Game {
            next: PlayerId(0),
            pc: 0,
        });
        self.run(Input::Start);
    }

    // ---- Public queries ----

    pub fn config(&self) -> &GameConfig {
        &self.config
    }

    pub fn pending(&self) -> Option<&Decision> {
        self.pending.as_ref()
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    pub fn log(&self) -> &[Event] {
        &self.log
    }

    /// What `id` currently is: its printed card, or what it is acting as.
    pub fn card(&self, id: CardId) -> Card {
        self.masks
            .get(&id)
            .copied()
            .unwrap_or(self.cards[id.index()])
    }

    /// What is printed on `id`.
    pub fn printed(&self, id: CardId) -> Card {
        self.cards[id.index()]
    }

    pub fn zone(&self, id: CardId) -> Zone {
        self.zones[id.index()]
    }

    /// Total number of physical cards in the game.
    pub fn card_count(&self) -> usize {
        self.cards.len()
    }

    pub fn num_players(&self) -> usize {
        self.players.len()
    }

    pub fn player(&self, p: PlayerId) -> &Player {
        &self.players[p.index()]
    }

    pub fn hand(&self, p: PlayerId) -> &[CardId] {
        &self.players[p.index()].hand
    }

    pub fn in_play(&self, p: PlayerId) -> &[CardId] {
        &self.players[p.index()].in_play
    }

    pub fn piles(&self) -> &[Pile] {
        &self.piles
    }

    pub fn pile(&self, id: PileId) -> &Pile {
        &self.piles[id.index()]
    }

    pub fn trash(&self) -> &[CardId] {
        &self.trash
    }

    pub fn turn(&self) -> &Turn {
        &self.turn
    }

    pub fn landscapes(&self) -> &[Card] {
        &self.landscapes
    }

    pub fn has_landscape(&self, card: Card) -> bool {
        self.landscapes.contains(&card)
    }

    pub fn lasting(&self) -> &[Lasting] {
        &self.lasting
    }

    pub fn artifacts(&self) -> &[(Card, Option<PlayerId>)] {
        &self.artifacts
    }

    pub fn artifact_holder(&self, artifact: Card) -> Option<PlayerId> {
        self.artifacts
            .iter()
            .find(|(a, _)| *a == artifact)
            .and_then(|(_, p)| *p)
    }

    pub fn prophecy(&self) -> Option<(Card, u32)> {
        self.prophecy
    }

    /// Whether the game's Prophecy is in effect (all its Sun tokens removed).
    pub fn prophecy_active(&self, card: Card) -> bool {
        self.prophecy == Some((card, 0))
    }

    pub fn boons(&self) -> &CardDeck {
        &self.boons
    }

    pub fn hexes(&self) -> &CardDeck {
        &self.hexes
    }

    pub fn trade_route_mat(&self) -> u32 {
        self.trade_route_mat
    }

    pub fn extra_turns(&self) -> &VecDeque<ExtraTurn> {
        &self.extra_turns
    }

    /// The pile named after `card`, in or out of the Supply.
    pub fn pile_of(&self, card: Card) -> Option<PileId> {
        self.piles
            .iter()
            .position(|p| p.card == card)
            .map(|i| PileId(i as u8))
    }

    /// The Supply pile named after `card`.
    pub fn supply_pile(&self, card: Card) -> Option<PileId> {
        self.piles
            .iter()
            .position(|p| p.in_supply && p.card == card)
            .map(|i| PileId(i as u8))
    }

    /// The pile `card` belongs in (where it returns to), in or out of the Supply.
    pub fn home_pile(&self, card: Card) -> Option<PileId> {
        let name = card.def().pile.unwrap_or(card);
        self.pile_of(name).or_else(|| self.pile_of(card))
    }

    /// Where `id` was trashed from, while its "when trashed" abilities resolve.
    pub fn trashed_from(&self, id: CardId) -> Option<Zone> {
        self.trashing
            .iter()
            .rev()
            .find(|(c, _)| *c == id)
            .map(|(_, z)| *z)
    }

    pub fn pile_top(&self, id: PileId) -> Option<CardId> {
        self.piles[id.index()].cards.last().copied()
    }

    /// What `id` costs right now, with every cost modifier applied.
    pub fn cost_of(&self, id: CardId) -> Cost {
        self.cost_with(self.card(id), Some(id))
    }

    /// What a card costs right now, e.g. the cards of a pile.
    pub fn cost(&self, card: Card) -> Cost {
        self.cost_with(card, None)
    }

    fn cost_with(&self, card: Card, id: Option<CardId>) -> Cost {
        let mut cost = card.cost().unwrap_or_default();
        cards::modify_cost(self, card, id, &mut cost);
        cost
    }

    /// The types `id` currently has, with every type-changing effect applied.
    pub fn types(&self, id: CardId) -> Types {
        let mut types = self.card(id).types();
        cards::modify_types(self, id, &mut types);
        types
    }

    pub fn is(&self, id: CardId, types: Types) -> bool {
        self.types(id).intersects(types)
    }

    pub fn empty_supply_piles(&self) -> usize {
        self.piles
            .iter()
            .filter(|p| p.in_supply && p.cards.is_empty())
            .count()
    }

    /// Supply piles whose top card's types and cost satisfy `pred`.
    pub fn supply_piles_where(&self, pred: impl Fn(Types, Cost) -> bool) -> Vec<PileId> {
        (0..self.piles.len())
            .map(|i| PileId(i as u8))
            .filter(|&id| {
                let pile = &self.piles[id.index()];
                pile.in_supply
                    && pile
                        .cards
                        .last()
                        .is_some_and(|&top| pred(self.types(top), self.cost_of(top)))
            })
            .collect()
    }

    /// Supply piles whose top card costs up to `limit`.
    pub fn piles_costing_up_to(&self, limit: Cost) -> Vec<PileId> {
        self.supply_piles_where(|_, cost| cost.at_most(limit))
    }

    /// The other players in turn order, starting to `p`'s left.
    pub fn others(&self, p: PlayerId) -> Vec<PlayerId> {
        let n = self.players.len() as u8;
        (1..n).map(|i| PlayerId((p.0 + i) % n)).collect()
    }

    /// All players in turn order, starting with `p`.
    pub fn players_from(&self, p: PlayerId) -> Vec<PlayerId> {
        let mut v = vec![p];
        v.extend(self.others(p));
        v
    }

    pub fn left_of(&self, p: PlayerId) -> PlayerId {
        PlayerId((p.0 + 1) % self.players.len() as u8)
    }

    pub fn right_of(&self, p: PlayerId) -> PlayerId {
        let n = self.players.len() as u8;
        PlayerId((p.0 + n - 1) % n)
    }

    pub fn count_owned(&self, p: PlayerId, card: Card) -> usize {
        self.player(p)
            .owned()
            .filter(|&c| self.card(c) == card)
            .count()
    }

    pub fn score(&self, p: PlayerId) -> i32 {
        let player = &self.players[p.index()];
        let cards: i32 = player
            .owned()
            .map(|c| {
                let card = self.card(c);
                cards::vp(self, p, card).unwrap_or(card.def().vp as i32)
            })
            .sum();
        cards + player.vp_tokens + cards::bonus_vp(self, p)
    }

    // ---- Driving the game ----

    pub fn respond(&mut self, response: Response) -> Result<(), ResponseError> {
        let decision = self
            .pending
            .as_ref()
            .ok_or(ResponseError::NoPendingDecision)?;
        decision.validate(&response)?;
        self.pending = None;
        self.run(Input::Response(response));
        Ok(())
    }

    /// Resumes frames until one needs a real decision or the stack empties.
    pub(crate) fn run(&mut self, mut input: Input) {
        while let Some(mut frame) = self.stack.pop() {
            match frame.resume(self, input) {
                Flow::Done(value) => input = Input::Return(value),
                Flow::Continue => {
                    self.stack.push(frame);
                    input = Input::Start;
                }
                Flow::Call(child) => {
                    self.stack.push(frame);
                    self.stack.push(child);
                    input = Input::Start;
                }
                Flow::Ask(mut decision) => {
                    self.stack.push(frame);
                    if decision.player == self.turn.player {
                        decision.player = self.turn.controller;
                    }
                    if let Some(response) = decision.forced(self) {
                        input = Input::Response(response);
                    } else {
                        self.pending = Some(decision);
                        return;
                    }
                }
            }
        }
    }

    // ---- Primitive state changes. These never trigger anything; frames do. ----

    pub(crate) fn create(&mut self, card: Card, zone: Zone) -> CardId {
        let id = CardId(self.cards.len() as u32);
        self.cards.push(card);
        self.zones.push(zone);
        self.zone_vec(zone).push(id);
        id
    }

    pub(crate) fn zone_vec(&mut self, zone: Zone) -> &mut Vec<CardId> {
        match zone {
            Zone::Deck(p) => &mut self.players[p.index()].deck,
            Zone::Hand(p) => &mut self.players[p.index()].hand,
            Zone::Discard(p) => &mut self.players[p.index()].discard,
            Zone::InPlay(p) => &mut self.players[p.index()].in_play,
            Zone::SetAside(p) => &mut self.players[p.index()].set_aside,
            Zone::Mat(p, mat) => self.players[p.index()].mats.entry(mat).or_default(),
            Zone::Pile(id) => &mut self.piles[id.index()].cards,
            Zone::Trash => &mut self.trash,
        }
    }

    pub(crate) fn move_card(&mut self, id: CardId, to: Zone, pos: Pos) {
        let from = self.zones[id.index()];
        let src = self.zone_vec(from);
        let i = src
            .iter()
            .rposition(|&c| c == id)
            .expect("card missing from its recorded zone");
        src.remove(i);
        if matches!(from, Zone::InPlay(_)) && !matches!(to, Zone::InPlay(_)) {
            self.masks.remove(&id);
        }
        self.zones[id.index()] = to;
        let dst = self.zone_vec(to);
        match pos {
            Pos::Top => dst.push(id),
            Pos::Bottom => dst.insert(0, id),
        }
    }

    pub(crate) fn emit(&mut self, event: Event) {
        cards::on_event(self, &event);
        self.log.push(event);
    }

    pub(crate) fn shuffle<T>(&mut self, items: &mut [T]) {
        items.shuffle(&mut self.rng);
    }

    pub(crate) fn random_index(&mut self, n: usize) -> usize {
        self.rng.random_range(0..n)
    }

    /// Shuffles `p`'s discard pile and puts it under their deck.
    pub(crate) fn shuffle_discard_into_deck(&mut self, p: PlayerId) {
        let player = &mut self.players[p.index()];
        let mut cards = std::mem::take(&mut player.discard);
        cards.shuffle(&mut self.rng);
        for &c in &cards {
            self.zones[c.index()] = Zone::Deck(p);
        }
        let player = &mut self.players[p.index()];
        cards.append(&mut player.deck);
        player.deck = cards;
        self.emit(Event::Shuffled { player: p });
    }

    pub(crate) fn cards_in_hand_where(&self, p: PlayerId, types: Types) -> Vec<CardId> {
        self.hand(p)
            .iter()
            .copied()
            .filter(|&c| self.is(c, types))
            .collect()
    }

    /// +$, respecting the -$1 token.
    pub(crate) fn add_coins(&mut self, p: PlayerId, n: u32) {
        if n == 0 {
            return;
        }
        let player = &mut self.players[p.index()];
        let n = if player.minus_coin {
            player.minus_coin = false;
            n - 1
        } else {
            n
        };
        if p == self.turn.player {
            self.turn.coins += n;
        }
    }

    /// `p` takes `n` Debt (or whoever takes it for them: Possession).
    pub(crate) fn add_debt(&mut self, p: PlayerId, n: u32) {
        let taker = cards::debt_taker(self, p).unwrap_or(p);
        self.players[taker.index()].debt += n;
    }

    /// Puts `id` on top of `p`'s deck.
    pub(crate) fn top_deck(&mut self, id: CardId, p: PlayerId) {
        self.move_card(id, Zone::Deck(p), Pos::Top);
        self.emit(Event::TopDecked {
            player: p,
            card: self.card(id),
        });
    }

    pub(crate) fn put_in_hand(&mut self, id: CardId, p: PlayerId) {
        self.move_card(id, Zone::Hand(p), Pos::Top);
    }

    /// Sets `id` aside for `p`; whoever set it aside must track it (usually in a Lasting).
    pub(crate) fn set_aside(&mut self, id: CardId, p: PlayerId) {
        self.move_card(id, Zone::SetAside(p), Pos::Top);
        self.emit(Event::SetAside {
            player: p,
            card: self.card(id),
        });
    }

    pub(crate) fn put_on_mat(&mut self, id: CardId, p: PlayerId, mat: Mat) {
        let public = mat != Mat::NativeVillage;
        self.move_card(id, Zone::Mat(p, mat), Pos::Top);
        self.emit(Event::Moved {
            player: p,
            card: public.then(|| self.card(id)),
            to: Zone::Mat(p, mat),
        });
    }

    /// Returns `id` to the pile it came from (no gain or trash happens). Returns whether
    /// it had a pile to go to.
    pub(crate) fn return_to_pile(&mut self, id: CardId) -> bool {
        let printed = self.printed(id);
        let Some(pile) = self.home_pile(printed) else {
            return false;
        };
        let owner = self.zone(id).owner();
        self.move_card(id, Zone::Pile(pile), Pos::Top);
        if let Some(player) = owner {
            self.emit(Event::Moved {
                player,
                card: Some(printed),
                to: Zone::Pile(pile),
            });
        }
        true
    }

    pub(crate) fn add_lasting(
        &mut self,
        card: Card,
        owner: PlayerId,
        source: Option<CardId>,
        keeps: Vec<CardId>,
        until: Until,
        data: Scratch,
    ) -> u32 {
        let id = self.next_lasting;
        self.next_lasting += 1;
        let turn = self.players[owner.index()].turns_taken;
        self.lasting.push(Lasting {
            id,
            card,
            owner,
            source,
            keeps,
            until,
            turn,
            data,
        });
        id
    }

    pub(crate) fn remove_lasting(&mut self, id: u32) -> Option<Lasting> {
        let i = self.lasting.iter().position(|l| l.id == id)?;
        Some(self.lasting.remove(i))
    }

    pub(crate) fn lasting_mut(&mut self, id: u32) -> Option<&mut Lasting> {
        self.lasting.iter_mut().find(|l| l.id == id)
    }

    /// Draws a Boon or Hex from its deck, reshuffling its discards if needed.
    pub(crate) fn draw_from(&mut self, hexes: bool) -> Option<Card> {
        let mut deck = std::mem::take(if hexes {
            &mut self.hexes
        } else {
            &mut self.boons
        });
        if deck.draw.is_empty() {
            deck.draw = std::mem::take(&mut deck.discard);
            deck.draw.shuffle(&mut self.rng);
        }
        let card = deck.draw.pop();
        *(if hexes {
            &mut self.hexes
        } else {
            &mut self.boons
        }) = deck;
        card
    }

    pub(crate) fn game_over_reason(&self) -> Option<EndReason> {
        let empty = |card| {
            self.supply_pile(card)
                .is_some_and(|p| self.piles[p.index()].cards.is_empty())
        };
        if empty(Card::Province) {
            return Some(EndReason::ProvincesGone);
        }
        if empty(Card::Colony) {
            return Some(EndReason::ColoniesGone);
        }
        let limit = if self.players.len() >= 5 { 4 } else { 3 };
        if self.empty_supply_piles() >= limit {
            return Some(EndReason::PilesEmpty);
        }
        let turns: u32 = self.players.iter().map(|p| p.turns_taken).sum();
        if self.config.turn_limit.is_some_and(|limit| turns >= limit) {
            return Some(EndReason::TurnLimit);
        }
        None
    }

    pub(crate) fn finish(&mut self, reason: EndReason) {
        let n = self.players.len();
        let scores: Vec<i32> = (0..n).map(|p| self.score(PlayerId(p as u8))).collect();
        let turns: Vec<u32> = self
            .players
            .iter()
            .map(|p| p.turns_taken - p.extra_turns_taken)
            .collect();
        let best = (0..n)
            .map(|p| (scores[p], std::cmp::Reverse(turns[p])))
            .max()
            .expect("at least two players");
        let winners = (0..n)
            .filter(|&p| (scores[p], std::cmp::Reverse(turns[p])) == best)
            .map(|p| PlayerId(p as u8))
            .collect();
        self.outcome = Some(Outcome {
            reason,
            scores,
            turns,
            winners,
        });
    }

    /// Whether `card` is a landscape, Boon, Hex or other non-physical object in play this game.
    pub(crate) fn in_game(&self, card: Card) -> bool {
        match card.kind() {
            Kind::Card | Kind::Randomizer => self.pile_of(card).is_some(),
            _ => {
                self.landscapes.contains(&card)
                    || self.artifacts.iter().any(|(a, _)| *a == card)
                    || self.prophecy.is_some_and(|(p, _)| p == card)
            }
        }
    }
}
