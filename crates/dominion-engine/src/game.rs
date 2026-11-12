use crate::cards::{self, Ext};
use crate::decision::{Decision, DecisionKind, Response, ResponseError};
use crate::event::Event;
use crate::frame::{Flow, Frame, Input};
use crate::setup::GameConfig;
use crate::trigger::Occasion;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Zone {
    Deck(PlayerId),
    Hand(PlayerId),
    Discard(PlayerId),
    InPlay(PlayerId),
    /// Set aside by an effect; the effect (usually a [`Lasting`]) tracks which cards.
    SetAside(PlayerId),
    /// Taken from the deck to be looked at or revealed: off the deck while the effect
    /// resolves, and put back on top when it ends unless it moved them (`Frame::look`).
    Looking(PlayerId),
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
            | Zone::Looking(p)
            | Zone::Mat(p, _) => Some(p),
            Zone::Pile(_) | Zone::Trash => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pos {
    Top,
    Bottom,
    /// Under that many of the cards there.
    Under(usize),
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
    /// The cards an effect is looking at or revealed from the deck (see [`Zone::Looking`]).
    #[serde(default)]
    pub looking: Vec<CardId>,
    /// The cards in `set_aside` and `looking` that are face up (revealed); the rest only
    /// their owner may look at.
    pub face_up: Vec<CardId>,
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
    /// The -1 Card token: the next draw removes it instead of drawing its first card.
    pub minus_card: bool,
    /// The -$1 token: the next time this player gets $, they get $1 less.
    pub minus_coin: bool,
    /// The $ this player has got so far, on anyone's turn: "if it gave you +$".
    #[serde(default)]
    pub coins_got: u32,
    pub journey_face_up: bool,
    pub turns_taken: u32,
    /// How many of `turns_taken` were extra turns, which don't count for the tiebreaker.
    pub extra_turns_taken: u32,
    /// Turns this player will skip (Lich): the next ones they would take.
    pub skip_turns: u32,
    /// Regular turns skipped so far; they count for the tiebreaker.
    pub turns_skipped: u32,
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
            .chain(&self.looking)
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
    /// Not a pile any more (Divine Wind removed it from the game) or never one (a holder for
    /// cards set aside in setup, like Riverboat's and Way of the Mouse's): lookups by name
    /// skip it, so it is no card's pile.
    pub removed: bool,
    /// Face down: no card in it is public, not even the top (the Black Market deck).
    pub hidden: bool,
    /// How many times it became empty while in the Supply.
    pub emptied: u32,
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
            removed: false,
            hidden: false,
            emptied: 0,
        }
    }

    /// A Kingdom pile: a Supply pile of a Kingdom card (or randomizer).
    pub fn is_kingdom(&self) -> bool {
        self.in_supply && !self.removed && self.card.def().kingdom
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[default]
    Start,
    Action,
    Buy,
    Night,
    Cleanup,
    End,
}

impl Phase {
    /// The start of a turn is part of its Action phase.
    pub fn is_action(self) -> bool {
        matches!(self, Phase::Start | Phase::Action)
    }
}

/// State that lasts for one turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Turn {
    pub player: PlayerId,
    /// Who took the turn before this one (skipped turns aren't taken).
    pub previous: Option<PlayerId>,
    /// The card that granted this as an extra turn, if it is one.
    pub extra: Option<Card>,
    pub phase: Phase,
    pub actions: u32,
    pub buys: u32,
    pub coins: u32,
    pub potions: u32,
    /// Bought anything (card, Event or Project) in this Buy phase (so no more Treasures). A
    /// card bought through Black Market isn't: Treasures can still be played after it.
    pub bought: bool,
    /// Every play this turn, by any player, in order. A card played twice appears twice.
    pub played: Vec<Card>,
    /// Who made each of `played`.
    #[serde(default)]
    pub played_by: Vec<PlayerId>,
    /// How many of this turn's plays by its player were from their hand.
    pub hand_plays: u32,
    /// The physical cards of `played`, in the same order.
    pub played_ids: Vec<CardId>,
    /// For each of `played_ids`: when it was played, if it was in play then (see
    /// [`Game::stayed_in_play`]).
    #[serde(default)]
    pub(crate) played_at: Vec<Mark>,
    /// The types each of `played` had as it was played.
    #[serde(default)]
    pub played_types: Vec<Types>,
    pub gained: Vec<Card>,
    /// The types each of `gained` had as it was gained.
    #[serde(default)]
    pub gained_types: Vec<Types>,
    /// The cards in `gained` that were gained in a Buy phase.
    pub gained_in_buy_phase: Vec<Card>,
    /// The types each of `gained_in_buy_phase` had as it was gained.
    #[serde(default)]
    pub gained_in_buy_phase_types: Vec<Types>,
    /// How many of `gained_in_buy_phase` were gained in Buy phases the turn has since left
    /// (Villa).
    #[serde(default)]
    pub earlier_buy_phase_gains: usize,
    pub bought_cards: Vec<Card>,
    pub trashed: Vec<Card>,
    /// Cards drawn in Clean-up.
    pub cleanup_draw: usize,
    /// No more +Actions this turn (Snowy Village).
    pub ignore_plus_actions: bool,
}

impl Turn {
    pub(crate) fn new(player: PlayerId) -> Turn {
        Turn {
            player,
            previous: None,
            extra: None,
            phase: Phase::Start,
            actions: 1,
            buys: 1,
            coins: 0,
            potions: 0,
            bought: false,
            played: vec![],
            played_by: vec![],
            hand_plays: 0,
            played_ids: vec![],
            played_at: vec![],
            played_types: vec![],
            gained: vec![],
            gained_types: vec![],
            gained_in_buy_phase: vec![],
            gained_in_buy_phase_types: vec![],
            earlier_buy_phase_gains: 0,
            bought_cards: vec![],
            trashed: vec![],
            cleanup_draw: 5,
            ignore_plus_actions: false,
        }
    }

    /// The cards gained in the current Buy phase: none yet after a return to the Action
    /// phase.
    pub fn gained_this_buy_phase(&self) -> &[Card] {
        &self.gained_in_buy_phase[self.earlier_buy_phase_gains..]
    }

    /// `p`'s plays this turn, in order: each card and the types it had as it was played.
    pub fn plays_by(&self, p: PlayerId) -> impl Iterator<Item = (Card, Types)> + '_ {
        self.played
            .iter()
            .zip(&self.played_types)
            .zip(&self.played_by)
            .filter(move |(_, q)| **q == p)
            .map(|((&c, &t), _)| (c, t))
    }

    /// How many of `p`'s plays this turn were of a card that had any of `types` as it was
    /// played.
    pub fn played_of(&self, p: PlayerId, types: Types) -> usize {
        self.plays_by(p)
            .filter(|(_, t)| t.intersects(types))
            .count()
    }

    /// How many times `p` played a `card` this turn.
    pub fn played_count(&self, p: PlayerId, card: Card) -> usize {
        self.plays_by(p).filter(|&(c, _)| c == card).count()
    }

    /// How many of this turn's gains were of a card that had any of `types` as it was gained.
    pub fn gained_of(&self, types: Types) -> usize {
        count_of(&self.gained_types, types)
    }
}

fn count_of(history: &[Types], types: Types) -> usize {
    history.iter().filter(|t| t.intersects(types)).count()
}

/// A moment in the game, by its card movements: an effect that has put cards where it
/// expects to find them later takes one ([`Game::now`]) and asks then whether each
/// [`Game::stayed`]. The default is no moment: nothing has stayed since.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mark(u32);

/// A card in the place an effect put it or found it ([`Game::placed`]), for the effect to
/// ask later whether it is [`Game::still`] there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Placed {
    pub card: CardId,
    at: Mark,
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
    /// Cards the effect goes on to move if they stay where they are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) placed: Vec<Placed>,
}

/// When a [`Lasting`] effect ends on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Until {
    /// At the end of this turn (after Clean-up).
    EndOfTurn,
    /// When that player's next turn starts (before start-of-turn effects); for one made
    /// during the start of a turn, that is the turn after.
    StartOfTurn(PlayerId),
    /// Only when an effect removes it (Durations remove themselves when they resolve).
    Removed,
    /// When the extra turn it keeps cards in play for (Outpost, Voyage: the
    /// [`ExtraTurn::lasting`] naming it) starts, or doesn't happen.
    ExtraTurn,
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
    /// For a replay of `source` (Throne Room's second play, ...): the card that replayed
    /// it, which has to stay in play to keep track of this effect.
    #[serde(default)]
    pub tracker: Option<CardId>,
    /// A Duration's effect whose card, or `tracker`, isn't in play: it still works this
    /// turn and ends with it.
    #[serde(default)]
    pub ending: bool,
}

impl Lasting {
    /// Whether this is a Duration card's effect, which keeps that card in play and
    /// lasts only while it stays there.
    fn tied_to_play(&self) -> bool {
        self.source.is_some() && self.keeps.first() == self.source.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraTurn {
    pub player: PlayerId,
    pub source: Card,
    pub limit: ExtraLimit,
    /// The lasting effect that keeps its card in play for it (Outpost, Voyage).
    #[serde(default)]
    pub lasting: Option<u32>,
}

/// When an extra turn doesn't happen (each card's own "but not ..." clause).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtraLimit {
    /// Always happens (Seize the Day).
    None,
    /// Not as a third turn in a row (Outpost, Voyage, Mission, ...).
    ThirdInARow,
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
    /// The gain that finished last.
    #[serde(default)]
    pub(crate) last_gain: Option<GainState>,
    /// Cards whose Trash windows are pending or resolving.
    pub(crate) trashing: Vec<TrashState>,
    pub(crate) plays: Vec<PlayState>,
    /// The cards being discarded from play other than in Clean-up, innermost last (see
    /// [`Game::leaving_play`]).
    #[serde(default)]
    pub(crate) leaving: Vec<Vec<CardId>>,
    /// The cards of the latest shuffle (see [`Game::shuffled`]).
    #[serde(default)]
    pub(crate) last_shuffle: Vec<CardId>,
    /// The cards of it that "when you shuffle" choices put on top of or under the others.
    #[serde(default)]
    pub(crate) shuffle_placed: Vec<CardId>,
    /// Cards taken off decks to be looked at or revealed, innermost last.
    #[serde(default)]
    pub(crate) holds: Vec<Hold>,
    /// For each card, when it last moved (see [`Game::stayed`]); 0 for one that never has.
    #[serde(default)]
    pub(crate) moved: Vec<u32>,
    /// The latest of `moved`.
    #[serde(default)]
    pub(crate) clock: u32,
    /// The cards the trash, discard or play that finished last put where it did, and when
    /// (see [`Game::put`]).
    #[serde(default)]
    pub(crate) last_put: (Vec<CardId>, Mark),
    /// How deep the effect stack was when a Supply pile last emptied (see
    /// [`Game::answers_emptying`]).
    #[serde(default)]
    pub(crate) emptied_depth: usize,
    pub(crate) ext: Ext,
    /// While a step of an ability runs whose +$ its player gets as +Cards instead (Way of
    /// the Chameleon): that player, and the +$ the step has given so far.
    #[serde(skip)]
    pub(crate) coins_as_cards: Option<(PlayerId, u32)>,
    pub(crate) stack: Vec<Frame>,
    pub(crate) pending: Option<Decision>,
    pub(crate) rng: ChaCha8Rng,
    pub(crate) log: Vec<Event>,
    /// What the log hasn't shown yet.
    #[serde(default)]
    pub(crate) scribe: crate::scribe::Scribe,
    pub(crate) outcome: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) trace: Option<Vec<crate::Trace>>,
}

/// Cards a look or reveal took off `player`'s deck (top first), held in
/// [`Zone::Looking`] until the frame that asked for them (at `depth - 1` on the stack)
/// finishes: then those still there go back on top of the deck, in their order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Hold {
    pub player: PlayerId,
    pub cards: Vec<CardId>,
    pub depth: usize,
    /// When they were taken.
    #[serde(default)]
    pub at: Mark,
}

/// A trash whose "when trashed" abilities are pending or resolving.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TrashState {
    pub card: CardId,
    /// Where it was trashed from.
    pub from: Zone,
    /// When it was trashed.
    #[serde(default)]
    pub at: Mark,
}

/// A gain in progress. "Would gain" triggers may change it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GainState {
    pub player: PlayerId,
    pub card: CardId,
    pub to: crate::Dest,
    pub bought: bool,
    /// For a bought card: which buy it is for (the log's length once its `Bought` event
    /// was logged). Another buy may come between (Black Market).
    #[serde(default)]
    pub buy: Option<usize>,
    /// Set by a replacement effect (Trader's exchange, ...): the card is not gained.
    pub cancelled: bool,
    /// The log's length when the gain began: effects set up later ("the next time you
    /// gain ...") don't respond to it.
    pub since: usize,
    /// What the card cost at the moment it was gained (set as it moves).
    pub cost: Cost,
    /// The turn's phase at the moment it was gained (set as it moves).
    #[serde(default)]
    pub phase: Phase,
    /// When it was gained (set as it moves).
    #[serde(default)]
    pub(crate) at: Mark,
    /// Once gained by the turn's player: its position in [`Turn::gained`] ("the next card
    /// you gain", "your second gain this turn").
    pub turn_index: Option<usize>,
    /// Once gained by the turn's player in their Buy phase: its position among the cards
    /// gained in that Buy phase ("the first card you gain in your Buy phase").
    #[serde(default)]
    pub buy_index: Option<usize>,
}

impl GainState {
    /// Where the card is gained to.
    pub fn zone(&self) -> Zone {
        match self.to {
            crate::Dest::Discard => Zone::Discard(self.player),
            crate::Dest::Hand => Zone::Hand(self.player),
            crate::Dest::DeckTop => Zone::Deck(self.player),
            crate::Dest::SetAside => Zone::SetAside(self.player),
        }
    }
}

/// A play in progress, until its `AfterPlay` window closes. "When you play" triggers may
/// change it.
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
    /// The log's length when the play began: effects set up later ("the next time you
    /// play ...") don't respond to it.
    pub since: usize,
    /// Whether the card's own instructions (its bonuses and abilities, or those of the Way
    /// used instead) have given +$ during this play, whatever became of the $ since: "if it
    /// gave you +$". Not the +$ of cards it played, tokens or Boons.
    #[serde(default)]
    pub gave_coins: bool,
    /// The log's length when the card's instructions were last begun (they are followed
    /// more than once for a Reckless card): "if this made you shuffle".
    #[serde(default)]
    pub run_since: usize,
    /// For a replay (the card was already played, or isn't put into play): the card that
    /// replays it, if a card does.
    #[serde(default)]
    pub tracker: Option<CardId>,
    /// Its place in `Turn::played`.
    #[serde(default)]
    pub index: usize,
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
        self.stack.push(Frame::window(Occasion::GameStart));
        self.start_log();
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

    /// The log of `p`'s last finished turn ("their last turn"): never one in progress.
    pub(crate) fn last_turn_log(&self, p: PlayerId) -> &[Event] {
        let is_start = |e: &Event| matches!(e, Event::TurnStart { .. });
        let mut end = self.log.len();
        if self.turn.player == p {
            end = self.log.iter().rposition(is_start).unwrap_or(0);
        }
        let Some(begin) = self.log[..end]
            .iter()
            .rposition(|e| matches!(e, Event::TurnStart { player, .. } if *player == p))
        else {
            return &[];
        };
        let len = self.log[begin + 1..end]
            .iter()
            .position(is_start)
            .unwrap_or(end - begin - 1);
        &self.log[begin + 1..begin + 1 + len]
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

    /// The present moment, for [`Game::stayed`].
    pub(crate) fn now(&self) -> Mark {
        Mark(self.clock + 1)
    }

    /// Whether `id` has stayed where it was at `since`: the stop-moving rule lets an
    /// effect move a card it expects somewhere only then. A card that was moved (even
    /// back to the same place, or only taken off its deck to be looked at), shuffled, or
    /// covered up on top of its deck has not.
    pub(crate) fn stayed(&self, id: CardId, since: Mark) -> bool {
        self.moved.get(id.index()).copied().unwrap_or(0) < since.0
    }

    /// The physical cards of the turn's plays that were in play as they were played and
    /// have stayed there since, one per such play: a card that left play and came back
    /// has moved, and what an earlier play of it set up can't move or replay it.
    pub(crate) fn stayed_in_play(&self) -> impl Iterator<Item = CardId> + '_ {
        let t = &self.turn;
        let plays = t.played_ids.iter().zip(&t.played_at);
        plays
            .filter(|&(&c, &at)| self.stayed(c, at))
            .map(|(&c, _)| c)
    }

    /// `id` where it is now.
    pub(crate) fn placed(&self, id: CardId) -> Placed {
        Placed {
            card: id,
            at: self.now(),
        }
    }

    /// Whether the card is still where it was when `placed` was taken.
    pub(crate) fn still(&self, placed: Placed) -> bool {
        self.stayed(placed.card, placed.at)
    }

    /// `id` where the trash, discard or play that just finished put it (into the trash,
    /// the discard pile, play), if it has stayed there: for effects that go on to move a
    /// card they trashed, discarded or played.
    pub(crate) fn put(&self, id: CardId) -> Option<Placed> {
        let (cards, at) = &self.last_put;
        (cards.contains(&id) && self.stayed(id, *at)).then_some(Placed { card: id, at: *at })
    }

    /// Records that `id` moved.
    fn touch(&mut self, id: CardId) {
        self.clock += 1;
        if self.moved.len() < self.cards.len() {
            self.moved.resize(self.cards.len(), 0);
        }
        self.moved[id.index()] = self.clock;
    }

    /// Records that `cards` were shuffled: whatever expected one of them where it was has
    /// lost it, even one that ends up where it began.
    pub(crate) fn touch_shuffled(&mut self, cards: &[CardId]) {
        for &c in cards {
            self.touch(c);
        }
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

    /// Whose turn is in progress. No one's between turns and once the game is over, when
    /// "during your turns" and "this turn" effects apply to nobody (scoring).
    pub(crate) fn turn_player(&self) -> Option<PlayerId> {
        (self.turn.phase != Phase::End).then_some(self.turn.player)
    }

    /// `p`'s $ and Potions: they are the turn's, so none on another player's turn.
    pub(crate) fn funds(&self, p: PlayerId) -> (u32, u32) {
        if p == self.turn.player {
            (self.turn.coins, self.turn.potions)
        } else {
            (0, 0)
        }
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

    /// Whether it is an Action phase.
    pub fn in_action_phase(&self) -> bool {
        self.turn.phase.is_action()
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
            .position(|p| p.card == card && !p.removed)
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

    /// The pile `card` is from, even one no longer in the game (Divine Wind): its cards are
    /// still that pile's for what marks the pile (the Bane, Obelisk).
    pub fn origin_pile(&self, card: Card) -> Option<PileId> {
        self.home_pile(card).or_else(|| {
            let name = card.def().pile.unwrap_or(card);
            self.piles
                .iter()
                .position(|p| p.card == name)
                .map(|i| PileId(i as u8))
        })
    }

    /// What `id` cost as it was gained, while its gain is in progress ("when you gain a
    /// card costing ..." abilities look at this, not at its cost where it landed).
    pub fn gain_cost(&self, id: CardId) -> Option<Cost> {
        self.gains
            .iter()
            .rev()
            .find(|st| st.card == id)
            .map(|st| st.cost)
    }

    /// Where `p` gained `id` to, if that is the gain that just finished.
    pub(crate) fn gained_to(&self, p: PlayerId, id: CardId) -> Option<Zone> {
        let st = self.last_gain.as_ref()?;
        (st.card == id && st.player == p).then_some(st.zone())
    }

    /// `id` where `p` gained it to in the gain that just finished, if it has stayed there:
    /// for effects that go on to move the card they had `p` gain. It is expected where it
    /// was gained to, which need not be the discard pile.
    pub(crate) fn gained(&self, p: PlayerId, id: CardId) -> Option<Placed> {
        let st = self.last_gain.as_ref()?;
        (st.card == id && st.player == p && self.stayed(id, st.at)).then_some(Placed {
            card: id,
            at: st.at,
        })
    }

    /// Whether a window opening now answers the latest emptying of a Supply pile: it is the
    /// window of the gain or trash that emptied it, or one further out, not one opened by
    /// an effect resolving in that window.
    pub(crate) fn answers_emptying(&self) -> bool {
        self.stack.len() <= self.emptied_depth + 1
    }

    /// Whether `id`, whose gain is in progress, has stayed where it was gained to: the
    /// stop-moving rule lets an on-gain ability move the card only then.
    pub(crate) fn still_where_gained(&self, id: CardId) -> bool {
        self.gains
            .iter()
            .rev()
            .find(|st| st.card == id)
            .is_some_and(|st| self.stayed(id, st.at))
    }

    /// The phase `id` was gained in, while its gain is in progress ("when you gain a card in
    /// your Buy phase" abilities look at this: resolving the gain may change the phase).
    pub fn gain_phase(&self, id: CardId) -> Option<Phase> {
        self.gains
            .iter()
            .rev()
            .find(|st| st.card == id)
            .map(|st| st.phase)
    }

    /// Whether `l`'s "until your next turn" part is still on: its owner hasn't begun a turn
    /// since it was made. It ends the instant that turn starts, before anything at its start,
    /// while the Duration's "at the start of your next turn" part is still to come.
    pub fn before_next_turn(&self, l: &Lasting) -> bool {
        self.player(l.owner).turns_taken == l.turn
    }

    /// `p`'s plays this turn before the play of `id` being resolved ("the first Silver you
    /// play", "if this is the first time you played"): not the ones made while it resolves.
    pub fn plays_before(
        &self,
        p: PlayerId,
        id: CardId,
    ) -> impl Iterator<Item = (Card, Types)> + '_ {
        let end = self
            .plays
            .iter()
            .rev()
            .find(|st| st.card == id)
            .map_or(self.turn.played.len(), |st| st.index);
        self.plays_where(p, move |i| i < end)
    }

    /// `p`'s plays this turn that have finished resolving.
    pub fn finished_plays(&self, p: PlayerId) -> impl Iterator<Item = (Card, Types)> + '_ {
        self.plays_where(p, |i| !self.plays.iter().any(|st| st.index == i))
    }

    fn plays_where<'a>(
        &'a self,
        p: PlayerId,
        keep: impl Fn(usize) -> bool + 'a,
    ) -> impl Iterator<Item = (Card, Types)> + 'a {
        let t = &self.turn;
        (0..t.played.len())
            .filter(move |&i| t.played_by.get(i) == Some(&p) && keep(i))
            .map(move |i| (t.played[i], t.played_types[i]))
    }

    /// Where `id` was trashed from, while its "when trashed" abilities resolve.
    pub fn trashed_from(&self, id: CardId) -> Option<Zone> {
        self.trashing
            .iter()
            .rev()
            .find(|st| st.card == id)
            .map(|st| st.from)
    }

    /// Whether `id`, whose "when trashed" abilities are resolving, has stayed in the
    /// trash: only then can one of them move it.
    pub(crate) fn still_where_trashed(&self, id: CardId) -> bool {
        self.trashing
            .iter()
            .rev()
            .find(|st| st.card == id)
            .is_some_and(|st| self.stayed(id, st.at))
    }

    /// Whether `id`, whose "when discarded" abilities are resolving, has stayed in the
    /// discard pile: only then can one of them move it.
    pub(crate) fn still_where_discarded(&self, id: CardId) -> bool {
        self.stack
            .iter()
            .rev()
            .find_map(|f| match f {
                Frame::Discard { cards, at, .. } if cards.contains(&id) => Some(*at),
                _ => None,
            })
            .is_some_and(|at| self.stayed(id, at))
    }

    /// Whether `id` is one of the cards a look or reveal took off `p`'s deck and has
    /// stayed off it since ([`Zone::Looking`]).
    pub(crate) fn held(&self, p: PlayerId, id: CardId) -> bool {
        self.zone(id) == Zone::Looking(p)
            && self
                .holds
                .iter()
                .rev()
                .find(|h| h.player == p && h.cards.contains(&id))
                .is_some_and(|h| self.kept(h, id))
    }

    /// Whether `id` has stayed in `h`. A hold saved before holds had marks has none: a
    /// card it lists that is still off the deck has stayed.
    fn kept(&self, h: &Hold, id: CardId) -> bool {
        h.at == Mark::default() || self.stayed(id, h.at)
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
        // Only cards change cost: an Event or a Project costs what it says.
        if card.kind().is_landscape() {
            return cost;
        }
        if let Some(cost) = cards::fixed_cost(self, card, id) {
            return cost;
        }
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

    /// The players other than `p`, in turn order from the player whose turn it is: the
    /// order an effect on several players resolves in, also off-turn.
    pub fn others_in_turn_order(&self, p: PlayerId) -> Vec<PlayerId> {
        let mut v = self.players_from(self.turn.player);
        v.retain(|&q| q != p);
        v
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

    /// `p`'s VP if the game ended now. The final scores are taken after the last turn, so
    /// during a turn this turn's effects (Capitalism, Inheritance, Bridge, cards played
    /// as another card) don't count.
    pub fn score(&self, p: PlayerId) -> i32 {
        let ended = self.turn_player().is_none()
            && !self.lasting.iter().any(|l| l.until == Until::EndOfTurn);
        if ended {
            self.score_as_is(p)
        } else {
            self.after_turn().score_as_is(p)
        }
    }

    /// This game as scored if it ended now: no turn in progress, this turn's lasting
    /// effects over, and what leaves play in Clean-up no longer another card.
    fn after_turn(&self) -> Game {
        let mut g = Game {
            config: self.config.clone(),
            cards: self.cards.clone(),
            zones: self.zones.clone(),
            masks: self.masks.clone(),
            players: self.players.clone(),
            piles: self.piles.clone(),
            trash: self.trash.clone(),
            turn: self.turn.clone(),
            landscapes: self.landscapes.clone(),
            boons: self.boons.clone(),
            hexes: self.hexes.clone(),
            artifacts: self.artifacts.clone(),
            prophecy: self.prophecy,
            trade_route_mat: self.trade_route_mat,
            lasting: self.lasting.clone(),
            next_lasting: self.next_lasting,
            extra_turns: self.extra_turns.clone(),
            ending: self.ending,
            final_round: self.final_round.clone(),
            gains: vec![],
            last_gain: None,
            trashing: vec![],
            plays: vec![],
            leaving: vec![],
            last_shuffle: vec![],
            shuffle_placed: vec![],
            holds: vec![],
            moved: vec![],
            clock: 0,
            last_put: Default::default(),
            emptied_depth: 0,
            ext: self.ext.clone(),
            coins_as_cards: None,
            stack: vec![],
            pending: None,
            rng: self.rng.clone(),
            log: vec![],
            scribe: Default::default(),
            outcome: None,
            trace: None,
        };
        g.turn.phase = Phase::End;
        g.lasting.retain(|l| l.until != Until::EndOfTurn);
        let kept: Vec<CardId> = g.lasting.iter().flat_map(|l| l.keeps.clone()).collect();
        let zones = &g.zones;
        g.masks
            .retain(|id, _| !matches!(zones[id.index()], Zone::InPlay(_)) || kept.contains(id));
        g
    }

    fn score_as_is(&self, p: PlayerId) -> i32 {
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
        let player = decision.player;
        if let Response::Name(card) = response {
            self.emit(Event::Named { player, card });
        }
        self.pending = None;
        self.run(Input::Response(response));
        Ok(())
    }

    /// Resumes frames until one needs a real decision or the stack empties.
    pub(crate) fn run(&mut self, mut input: Input) {
        while let Some(mut frame) = self.stack.pop() {
            match frame.resume(self, input) {
                Flow::Done(value) => {
                    self.release_holds();
                    input = Input::Return(value);
                }
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
                    if let Frame::Card(f) = &frame {
                        cards::modify_decision(self, f, &mut decision);
                    }
                    self.stack.push(frame);
                    // Whatever the answer, the others can't follow the order.
                    if let DecisionKind::Cards {
                        candidates,
                        ordered: true,
                        ..
                    } = &decision.kind
                        && candidates.len() > 1
                    {
                        self.mixed(candidates.clone());
                    }
                    if let Some(response) = decision.forced(self) {
                        input = Input::Response(response);
                    } else {
                        self.sync_log(None);
                        self.forget_hidden_shows();
                        self.pending = Some(decision);
                        return;
                    }
                }
            }
        }
        self.sync_log(None);
    }

    /// Puts back on top of their decks the held cards of frames that have finished.
    fn release_holds(&mut self) {
        while let Some(i) = self.holds.iter().rposition(|h| h.depth > self.stack.len()) {
            let h = self.holds.remove(i);
            for &c in h.cards.iter().rev() {
                if self.zones[c.index()] == Zone::Looking(h.player) && self.kept(&h, c) {
                    self.move_card(c, Zone::Deck(h.player), Pos::Top);
                }
            }
        }
    }

    // ---- Primitive state changes. These never trigger anything; frames do. ----

    pub(crate) fn add_pile(&mut self, pile: Pile) -> PileId {
        let id = u8::try_from(self.piles.len()).expect("setup bounds how many piles a game has");
        self.piles.push(pile);
        PileId(id)
    }

    pub(crate) fn create(&mut self, card: Card, zone: Zone) -> CardId {
        let id = CardId(self.cards.len() as u32);
        self.cards.push(card);
        self.zones.push(zone);
        self.zone_vec(zone).push(id);
        self.record(|| crate::Trace::Created {
            card: id,
            is: card,
            zone,
        });
        id
    }

    pub(crate) fn zone_vec(&mut self, zone: Zone) -> &mut Vec<CardId> {
        match zone {
            Zone::Deck(p) => &mut self.players[p.index()].deck,
            Zone::Hand(p) => &mut self.players[p.index()].hand,
            Zone::Discard(p) => &mut self.players[p.index()].discard,
            Zone::InPlay(p) => &mut self.players[p.index()].in_play,
            Zone::SetAside(p) => &mut self.players[p.index()].set_aside,
            Zone::Looking(p) => &mut self.players[p.index()].looking,
            Zone::Mat(p, mat) => self.players[p.index()].mats.entry(mat).or_default(),
            Zone::Pile(id) => &mut self.piles[id.index()].cards,
            Zone::Trash => &mut self.trash,
        }
    }

    pub(crate) fn move_card(&mut self, id: CardId, to: Zone, pos: Pos) {
        if self.already_at(id, to, pos) {
            return;
        }
        self.note_move(id, to, pos);
        let from = self.zones[id.index()];
        let src = self.zone_vec(from);
        let i = src
            .iter()
            .rposition(|&c| c == id)
            .expect("card missing from its recorded zone");
        let top = i + 1 == src.len();
        src.remove(i);
        self.record(|| crate::Trace::Moved {
            card: id,
            from,
            to,
            top,
        });
        if let Zone::Pile(pile) = from {
            let pile = &mut self.piles[pile.index()];
            if pile.cards.is_empty() && pile.in_supply {
                pile.emptied += 1;
                self.emptied_depth = self.stack.len();
            }
        }
        if matches!(from, Zone::InPlay(_)) && !matches!(to, Zone::InPlay(_)) {
            self.masks.remove(&id);
            self.left_play(id);
        }
        if let Zone::SetAside(p) | Zone::Looking(p) = from {
            self.players[p.index()].face_up.retain(|&c| c != id);
        }
        self.zones[id.index()] = to;
        let dst = self.zone_vec(to);
        let top = dst.last().copied();
        match pos {
            Pos::Top => dst.push(id),
            Pos::Bottom => dst.insert(0, id),
            Pos::Under(n) => dst.insert(dst.len().saturating_sub(n), id),
        }
        let covers = dst.last() == Some(&id);
        self.touch(id);
        // The card that was on top of a deck is covered up: what expected it there can't
        // move it any more. (A card covered up in a discard pile can still be moved.)
        if let (Zone::Deck(_), true, Some(under)) = (to, covers, top) {
            self.touch(under);
        }
    }

    /// Whether putting `id` there moves nothing: a card put into the hand it is in, or
    /// onto the deck it is on top of, has not moved for what expects it where it is
    /// (Watchtower putting a Nomad Camp onto the deck it was gained to).
    fn already_at(&self, id: CardId, to: Zone, pos: Pos) -> bool {
        self.zones[id.index()] == to
            && match to {
                Zone::Hand(_) => true,
                Zone::Deck(p) => {
                    pos == Pos::Top && self.players[p.index()].deck.last() == Some(&id)
                }
                _ => false,
            }
    }

    pub(crate) fn emit(&mut self, event: Event) {
        if let Event::RevealedHand { player, .. } = event {
            for c in self.players[player.index()].hand.clone() {
                self.show(c);
            }
        }
        self.sync_log(Some(&event));
        cards::on_event(self, &event);
        self.log.push(event);
    }

    /// Records that no one but their owner can follow `cards` any more.
    pub(crate) fn mixed(&mut self, cards: Vec<CardId>) {
        self.note_mixed(&cards);
        self.record(|| crate::Trace::Mixed { cards });
    }

    pub(crate) fn shuffle<T>(&mut self, items: &mut [T]) {
        items.shuffle(&mut self.rng);
    }

    pub(crate) fn random_index(&mut self, n: usize) -> usize {
        self.rng.random_range(0..n)
    }

    /// Cards of `types` that `p` may play as if from their hand: their hand, plus cards
    /// elsewhere that can be played as if there (Shadow cards in the deck), less those
    /// `SetRules::can_play` forbids (Warlord, Voyage).
    pub(crate) fn playable(&self, p: PlayerId, types: Types) -> Vec<CardId> {
        let mut cards = self.hand(p).to_vec();
        cards::playable_as_if_in_hand(self, p, &mut cards);
        cards.retain(|&c| self.is(c, types) && cards::can_play(self, p, c));
        cards
    }

    /// The cards `p` is discarding from play while a `DiscardFromPlay` window is open: the
    /// ones being discarded now or, in Clean-up, all of theirs that no lasting effect keeps
    /// in play.
    pub(crate) fn leaving_play(&self, p: PlayerId) -> Vec<CardId> {
        let in_play = self.in_play(p).iter().copied();
        match self.leaving.last() {
            Some(cards) => in_play.filter(|c| cards.contains(c)).collect(),
            None => in_play
                .filter(|c| !self.lasting.iter().any(|l| l.keeps.contains(c)))
                .collect(),
        }
    }

    /// Whether the open `DiscardFromPlay` window is `p`'s own Clean-up, rather than cards
    /// leaving play at another time or another player's cards leaving in this Clean-up.
    pub(crate) fn cleaning_up(&self, p: PlayerId) -> bool {
        p == self.turn.player && self.turn.phase == Phase::Cleanup && self.leaving.is_empty()
    }

    /// The cards just shuffled into `p`'s deck that are still in it, while the `Shuffled`
    /// window is open: the bottom cards of the deck, fewer than were shuffled once some
    /// have been put elsewhere (Avoid, Order of Masons).
    pub(crate) fn shuffled(&self, p: PlayerId) -> &[CardId] {
        let left = self
            .last_shuffle
            .iter()
            .filter(|c| self.zones[c.index()] == Zone::Deck(p))
            .count();
        let deck = &self.players[p.index()].deck;
        &deck[..left.min(deck.len())]
    }

    pub(crate) fn cards_in_hand_where(&self, p: PlayerId, types: Types) -> Vec<CardId> {
        self.hand(p)
            .iter()
            .copied()
            .filter(|&c| self.is(c, types))
            .collect()
    }

    /// +Actions: nothing for a player whose turn it isn't.
    pub(crate) fn add_actions(&mut self, p: PlayerId, n: u32) {
        if p == self.turn.player && !self.turn.ignore_plus_actions {
            self.turn.actions += n;
            self.note_bonus(p, n, 0, 0);
        }
    }

    /// +Buys: nothing for a player whose turn it isn't.
    pub(crate) fn add_buys(&mut self, p: PlayerId, n: u32) {
        if p == self.turn.player {
            self.turn.buys += n;
            self.note_bonus(p, 0, n, 0);
        }
    }

    /// +$, respecting the -$1 token.
    pub(crate) fn add_coins(&mut self, p: PlayerId, n: u32) {
        if let Some((q, coins)) = &mut self.coins_as_cards
            && *q == p
        {
            *coins += n;
            return;
        }
        self.get_coins(p, n);
    }

    /// $ that isn't "+$" (Fortune's doubling): the -$1 token applies, and nothing
    /// exchanges it for cards.
    pub(crate) fn get_coins(&mut self, p: PlayerId, n: u32) {
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
        player.coins_got += n;
        if p == self.turn.player {
            self.turn.coins += n;
            self.note_bonus(p, 0, 0, n);
        }
    }

    /// `p` takes `n` Debt.
    pub(crate) fn add_debt(&mut self, p: PlayerId, n: u32) {
        self.players[p.index()].debt += n;
    }

    /// Puts `id` on top of `p`'s deck.
    pub(crate) fn top_deck(&mut self, id: CardId, p: PlayerId) {
        let card = self.is_public(id).then(|| self.card(id));
        self.move_card(id, Zone::Deck(p), Pos::Top);
        self.emit(Event::TopDecked { player: p, card });
    }

    pub(crate) fn put_in_hand(&mut self, id: CardId, p: PlayerId) {
        self.move_card(id, Zone::Hand(p), Pos::Top);
    }

    /// Sets `id` aside face up for `p`; whoever set it aside must track it (usually in a
    /// Lasting). Cards set aside face down are moved with `move_card` instead.
    pub(crate) fn set_aside(&mut self, id: CardId, p: PlayerId) {
        self.set_aside_revealed(id, p);
        self.emit(Event::SetAside {
            player: p,
            card: self.card(id),
        });
    }

    /// Sets aside a card that was just revealed: face up, with no event of its own.
    pub(crate) fn set_aside_revealed(&mut self, id: CardId, p: PlayerId) {
        self.show(id);
        self.move_unlogged(id, Zone::SetAside(p));
        self.players[p.index()].face_up.push(id);
    }

    pub(crate) fn put_on_mat(&mut self, id: CardId, p: PlayerId, mat: Mat) {
        self.move_card(id, Zone::Mat(p, mat), Pos::Top);
    }

    /// Exchanges `p`'s `id` for `new`, the top card of a pile: `id` returns to its pile and
    /// `new` goes to `p`'s discard pile (nothing is gained). Returns whether it happened.
    pub(crate) fn exchange(&mut self, p: PlayerId, id: CardId, new: CardId) -> bool {
        let card = self.printed(id);
        if !self.return_to_pile(id) {
            return false;
        }
        self.move_card(new, Zone::Discard(p), Pos::Top);
        let into = self.card(new);
        self.emit(Event::Exchanged {
            player: p,
            card,
            into,
        });
        true
    }

    /// Returns `id` to the pile it came from (no gain or trash happens). Returns whether
    /// it had a pile to go to.
    pub(crate) fn return_to_pile(&mut self, id: CardId) -> bool {
        let printed = self.printed(id);
        let Some(pile) = self.home_pile(printed) else {
            return false;
        };
        self.show(id);
        self.move_card(id, Zone::Pile(pile), Pos::Top);
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
        let tracker = self
            .plays
            .last()
            .filter(|st| Some(st.card) == source)
            .and_then(|st| st.tracker);
        self.push_lasting(card, owner, source, tracker, keeps, until, data)
    }

    /// Adds a later part of the effect `from` began (a card set aside by its "once this
    /// turn" part, ...): the same play's, so with the same card keeping track of it.
    pub(crate) fn continue_lasting(
        &mut self,
        from: &Lasting,
        keeps: Vec<CardId>,
        until: Until,
        data: Scratch,
    ) -> u32 {
        self.push_lasting(
            from.card,
            from.owner,
            from.source,
            from.tracker,
            keeps,
            until,
            data,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn push_lasting(
        &mut self,
        card: Card,
        owner: PlayerId,
        source: Option<CardId>,
        tracker: Option<CardId>,
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
            keeps: vec![],
            until,
            turn,
            data,
            tracker,
            ending: false,
        });
        self.keep_in_play(id, keeps);
        id
    }

    /// Makes lasting `id` keep `keeps` in play. A Duration's effect (it keeps its own
    /// card) whose card or `tracker` isn't in play ends with this turn instead: a card
    /// that never was in play is like one that left it.
    pub(crate) fn keep_in_play(&mut self, id: u32, keeps: Vec<CardId>) {
        let zones = &self.zones;
        let in_play = |c: &CardId| matches!(zones[c.index()], Zone::InPlay(_));
        let Some(l) = self.lasting.iter_mut().find(|l| l.id == id) else {
            return;
        };
        l.keeps = keeps;
        if l.tied_to_play()
            && !(l.source.as_ref().is_some_and(in_play) && l.tracker.as_ref().is_none_or(in_play))
        {
            l.ending = true;
            l.keeps.clear();
        }
        l.keeps.retain(in_play);
    }

    /// `id` left play: a Duration stops doing things after this turn, and so do the
    /// replays of one that `id` was keeping track of. Nothing stays in play for `id`.
    fn left_play(&mut self, id: CardId) {
        for l in &mut self.lasting {
            if l.tied_to_play() && (l.source == Some(id) || l.tracker == Some(id)) {
                l.ending = true;
                l.keeps.clear();
            } else {
                l.keeps.retain(|&c| c != id);
            }
        }
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
            .map(|p| p.turns_taken - p.extra_turns_taken + p.turns_skipped)
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

    /// Whether a turn of `p`'s right after this one would be their third in a row.
    pub(crate) fn third_in_a_row(&self, p: PlayerId) -> bool {
        self.turn.player == p && self.turn.previous == Some(p)
    }

    /// Whether `p`'s next turn (an extra turn if `extra`) is skipped; if so, records it.
    pub(crate) fn skips_turn(&mut self, p: PlayerId, extra: bool) -> bool {
        let pl = &mut self.players[p.index()];
        if pl.skip_turns == 0 {
            return false;
        }
        pl.skip_turns -= 1;
        if !extra {
            pl.turns_skipped += 1;
        }
        true
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
