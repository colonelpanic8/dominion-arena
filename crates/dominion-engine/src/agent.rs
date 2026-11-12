//! The interface for anything that plays: bots, UIs, network clients.

use crate::decision::{Decision, DecisionKind, Move, Purpose, Response, ResponseError};
use crate::frame::Frame;
use crate::game::{EndReason, ExtraTurn, Lasting, Mat, Outcome, Pile, Player, Token, Turn, Zone};
use crate::trigger::Trigger;
use crate::{Card, CardId, Cost, Event, Game, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use thiserror::Error;

/// Answers decisions for one seat.
pub trait Agent {
    fn decide(&mut self, view: &View<'_>, decision: &Decision) -> Response;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlayError {
    #[error("game has {players} players but {agents} agents were given")]
    AgentCount { players: usize, agents: usize },
    #[error("agent for {player} gave an invalid response {response:?}: {error}")]
    InvalidResponse {
        player: PlayerId,
        response: Response,
        error: ResponseError,
    },
}

/// Runs `game` to the end, asking `agents[p]` for seat `p`'s decisions.
pub fn play<'g>(
    game: &'g mut Game,
    agents: &mut [&mut dyn Agent],
) -> Result<&'g Outcome, PlayError> {
    if agents.len() != game.num_players() {
        return Err(PlayError::AgentCount {
            players: game.num_players(),
            agents: agents.len(),
        });
    }
    while let Some(decision) = game.pending().cloned() {
        let player = decision.player;
        let response = agents[player.index()].decide(&game.view(player), &decision);
        game.respond(response.clone())
            .map_err(|error| PlayError::InvalidResponse {
                player,
                response,
                error,
            })?;
    }
    Ok(game
        .outcome()
        .expect("a game with no pending decision is over"))
}

/// The game as one player is allowed to see it: everything public, plus what that player
/// may look at (their hand, their face-down set-aside cards, the cards they are looking at,
/// their Native Village mat, cards offered to them in a decision).
///
/// A deck's order is never shown, nor which cards are in a discard pile under its top,
/// nor any card of a face-down pile (the Black Market deck, Loot).
#[derive(Clone, Copy)]
pub struct View<'a> {
    game: &'a Game,
    me: PlayerId,
}

impl Game {
    pub fn view(&self, player: PlayerId) -> View<'_> {
        View {
            game: self,
            me: player,
        }
    }
}

impl<'a> View<'a> {
    pub fn me(&self) -> PlayerId {
        self.me
    }

    pub fn num_players(&self) -> usize {
        self.game.num_players()
    }

    pub fn turn(&self) -> &'a Turn {
        self.game.turn()
    }

    /// The public log.
    pub fn log(&self) -> &'a [Event] {
        self.game.log()
    }

    /// The decision this player is being asked, if any.
    pub fn pending(&self) -> Option<&'a Decision> {
        self.game.pending().filter(|d| d.player == self.me)
    }

    /// The triggers a pending [`Purpose::Trigger`] decision's `Opt::Trigger(i)` options
    /// refer to (`[i]`).
    pub fn pending_triggers(&self) -> Vec<&'a Trigger> {
        match (self.pending(), self.game.stack.last()) {
            (Some(d), Some(Frame::Window(w))) if d.purpose == Purpose::Trigger => {
                w.asked().collect()
            }
            _ => vec![],
        }
    }

    /// What `id` is, if this player can see it: cards in play, in the trash, on public
    /// mats, set aside face up, on top of a discard pile or a face-up pile, their own
    /// hand and face-down cards, and cards offered to them in a decision or that it is about.
    pub fn card(&self, id: CardId) -> Option<Card> {
        self.visible(id).then(|| self.game.card(id))
    }

    /// Where `id` is, if this player can see it.
    pub fn zone(&self, id: CardId) -> Option<Zone> {
        self.visible(id).then(|| self.game.zone(id))
    }

    /// What `id` costs right now, if this player can see it.
    pub fn cost_of(&self, id: CardId) -> Option<Cost> {
        self.visible(id).then(|| self.game.cost_of(id))
    }

    /// The types `id` has right now, if this player can see it.
    pub fn types(&self, id: CardId) -> Option<Types> {
        self.visible(id).then(|| self.game.types(id))
    }

    /// What a card costs right now (the cards of a pile, an Event, ...).
    pub fn cost(&self, card: Card) -> Cost {
        self.game.cost(card)
    }

    fn visible(&self, id: CardId) -> bool {
        let g = self.game;
        let seen = match g.zone(id) {
            Zone::Hand(p) => p == self.me,
            Zone::SetAside(p) | Zone::Looking(p) => {
                p == self.me || g.player(p).face_up.contains(&id)
            }
            Zone::InPlay(_) | Zone::Trash => true,
            Zone::Discard(p) => g.player(p).discard.last() == Some(&id),
            Zone::Pile(pile) => !g.pile(pile).hidden && g.pile_top(pile) == Some(id),
            Zone::Mat(p, mat) => mat != Mat::NativeVillage || p == self.me,
            Zone::Deck(_) => false,
        };
        seen || self.offered(id)
    }

    fn offered(&self, id: CardId) -> bool {
        let Some(d) = self.pending() else {
            return false;
        };
        d.about.contains(&id)
            || match &d.kind {
                DecisionKind::Turn { moves, .. } => moves.contains(&Move::Play(id)),
                DecisionKind::Cards { candidates, .. } => candidates.contains(&id),
                _ => false,
            }
    }

    /// This player's hand.
    pub fn hand(&self) -> &'a [CardId] {
        self.game.hand(self.me)
    }

    pub fn player(&self, p: PlayerId) -> PlayerView<'a> {
        PlayerView {
            view: *self,
            id: p,
            player: self.game.player(p),
        }
    }

    /// Every player, in seat order.
    pub fn players(&self) -> impl Iterator<Item = PlayerView<'a>> + 'a {
        let v = *self;
        (0..v.num_players()).map(move |i| v.player(PlayerId(i as u8)))
    }

    pub fn pile(&self, id: PileId) -> PileView<'a> {
        PileView {
            id,
            pile: self.game.pile(id),
        }
    }

    /// Every pile, in or out of the Supply, including removed ones and setup holders.
    pub fn piles(&self) -> impl Iterator<Item = PileView<'a>> + 'a {
        let v = *self;
        (0..self.game.piles().len()).map(move |i| v.pile(PileId(i as u8)))
    }

    pub fn supply_pile(&self, card: Card) -> Option<PileId> {
        self.game.supply_pile(card)
    }

    pub fn empty_supply_piles(&self) -> usize {
        self.game.empty_supply_piles()
    }

    pub fn trash(&self) -> &'a [CardId] {
        self.game.trash()
    }

    /// Events, Landmarks, Projects, Ways, Allies, Traits and the Prophecy in the game.
    pub fn landscapes(&self) -> &'a [Card] {
        self.game.landscapes()
    }

    /// The Artifacts in the game and who holds each.
    pub fn artifacts(&self) -> &'a [(Card, Option<PlayerId>)] {
        self.game.artifacts()
    }

    /// The Prophecy and its remaining Sun tokens.
    pub fn prophecy(&self) -> Option<(Card, u32)> {
        self.game.prophecy()
    }

    pub fn boons(&self) -> CardDeckView<'a> {
        CardDeckView::of(self.game.boons())
    }

    pub fn hexes(&self) -> CardDeckView<'a> {
        CardDeckView::of(self.game.hexes())
    }

    /// Coin tokens on the Trade Route mat.
    pub fn trade_route_mat(&self) -> u32 {
        self.game.trade_route_mat()
    }

    /// Extra turns waiting to be taken, next first.
    pub fn extra_turns(&self) -> &'a VecDeque<ExtraTurn> {
        self.game.extra_turns()
    }

    /// Set once the game has ended while an extra round after its end is played (Fleet).
    pub fn ending(&self) -> Option<EndReason> {
        self.game.ending
    }

    /// Every lasting effect (Durations' future parts, "this turn" effects, ...), with the
    /// cards this player can't see left out of their data.
    pub fn lasting(&self) -> Vec<Lasting> {
        let seen = |c: &CardId| self.visible(*c);
        self.game
            .lasting()
            .iter()
            .map(|l| {
                let mut l = l.clone();
                l.source = l.source.filter(seen);
                l.tracker = l.tracker.filter(seen);
                l.keeps.retain(seen);
                l.data.cards.retain(seen);
                l.data.pick = l.data.pick.filter(seen);
                l.data.placed.clear();
                l
            })
            .collect()
    }

    /// Public state cards and landscapes keep outside the zones and tokens above.
    pub fn notes(&self) -> Vec<Note> {
        crate::cards::notes(self.game)
    }
}

/// One player's public state, plus what the viewer may see of their cards.
#[derive(Clone, Copy)]
pub struct PlayerView<'a> {
    view: View<'a>,
    id: PlayerId,
    player: &'a Player,
}

impl<'a> PlayerView<'a> {
    pub fn id(&self) -> PlayerId {
        self.id
    }

    /// Their hand, if the viewer may see it.
    pub fn hand(&self) -> Option<&'a [CardId]> {
        (self.id == self.view.me).then_some(&self.player.hand[..])
    }

    pub fn hand_size(&self) -> usize {
        self.player.hand.len()
    }

    pub fn deck_size(&self) -> usize {
        self.player.deck.len()
    }

    pub fn discard_size(&self) -> usize {
        self.player.discard.len()
    }

    pub fn discard_top(&self) -> Option<CardId> {
        self.player.discard.last().copied()
    }

    pub fn in_play(&self) -> &'a [CardId] {
        &self.player.in_play
    }

    /// Their set-aside cards the viewer may see (face up, or all if they are the viewer's).
    pub fn set_aside(&self) -> Vec<CardId> {
        let v = self.view;
        self.player
            .set_aside
            .iter()
            .copied()
            .filter(|&c| v.visible(c))
            .collect()
    }

    pub fn set_aside_size(&self) -> usize {
        self.player.set_aside.len()
    }

    /// The cards an effect is looking at or revealed from their deck that the viewer may
    /// see (revealed ones, or all if they are the viewer's).
    pub fn looking(&self) -> Vec<CardId> {
        let v = self.view;
        self.player
            .looking
            .iter()
            .copied()
            .filter(|&c| v.visible(c))
            .collect()
    }

    pub fn looking_size(&self) -> usize {
        self.player.looking.len()
    }

    /// The cards on one of their mats, if the viewer may see them (the Native Village mat
    /// is its owner's only).
    pub fn mat(&self, mat: Mat) -> Option<&'a [CardId]> {
        (mat != Mat::NativeVillage || self.id == self.view.me).then(|| self.player.mat(mat))
    }

    pub fn mat_size(&self, mat: Mat) -> usize {
        self.player.mat(mat).len()
    }

    pub fn vp_tokens(&self) -> i32 {
        self.player.vp_tokens
    }

    pub fn coffers(&self) -> u32 {
        self.player.coffers
    }

    pub fn villagers(&self) -> u32 {
        self.player.villagers
    }

    pub fn favors(&self) -> u32 {
        self.player.favors
    }

    pub fn debt(&self) -> u32 {
        self.player.debt
    }

    /// Coin tokens on their Pirate Ship mat.
    pub fn pirate_ship(&self) -> u32 {
        self.player.pirate_ship
    }

    /// Projects they have a cube on.
    pub fn projects(&self) -> &'a [Card] {
        &self.player.projects
    }

    /// States they have (Lost in the Woods, Deluded, ...).
    pub fn states(&self) -> &'a [Card] {
        &self.player.states
    }

    /// Boons they keep until Clean-up.
    pub fn boons_kept(&self) -> &'a [Card] {
        &self.player.boons_kept
    }

    /// Their Adventures tokens and the pile each is on.
    pub fn tokens(&self) -> &'a BTreeMap<Token, Option<PileId>> {
        &self.player.tokens
    }

    /// Their -1 Card token is on their deck.
    pub fn minus_card(&self) -> bool {
        self.player.minus_card
    }

    /// Their -$1 token is on them.
    pub fn minus_coin(&self) -> bool {
        self.player.minus_coin
    }

    pub fn journey_face_up(&self) -> bool {
        self.player.journey_face_up
    }

    /// Turns taken, including extra turns.
    pub fn turns_taken(&self) -> u32 {
        self.player.turns_taken
    }

    pub fn extra_turns_taken(&self) -> u32 {
        self.player.extra_turns_taken
    }

    /// Turns they will skip (Lich).
    pub fn skip_turns(&self) -> u32 {
        self.player.skip_turns
    }

    pub fn turns_skipped(&self) -> u32 {
        self.player.turns_skipped
    }

    /// Every card a player has gained is public, so their total is too.
    pub fn score(&self) -> i32 {
        self.view.game.score(self.id)
    }
}

/// A pile: everything about it is public but the cards under its top (and its top, if it
/// is face down).
#[derive(Clone, Copy)]
pub struct PileView<'a> {
    id: PileId,
    pile: &'a Pile,
}

impl<'a> PileView<'a> {
    pub fn id(&self) -> PileId {
        self.id
    }

    /// The card the pile is named after (its randomizer or top card).
    pub fn card(&self) -> Card {
        self.pile.card
    }

    pub fn size(&self) -> usize {
        self.pile.cards.len()
    }

    /// Its top card, unless the pile is face down.
    pub fn top(&self) -> Option<CardId> {
        self.pile
            .cards
            .last()
            .copied()
            .filter(|_| !self.pile.hidden)
    }

    pub fn in_supply(&self) -> bool {
        self.pile.in_supply
    }

    /// A Kingdom pile: a Supply pile of a Kingdom card, whether it was dealt at setup or
    /// added later (Young Witch's Bane, Divine Wind).
    pub fn kingdom(&self) -> bool {
        self.pile.is_kingdom()
    }

    pub fn vp_tokens(&self) -> i32 {
        self.pile.vp_tokens
    }

    pub fn debt_tokens(&self) -> i32 {
        self.pile.debt_tokens
    }

    /// Embargo's Curse tokens.
    pub fn embargo(&self) -> u32 {
        self.pile.embargo
    }

    /// Whether a Trade Route token is still on it.
    pub fn trade_route(&self) -> bool {
        self.pile.trade_route
    }

    /// Adventures tokens on it, by owner.
    pub fn tokens(&self) -> &'a [(PlayerId, Token)] {
        &self.pile.tokens
    }

    pub fn traits(&self) -> &'a [Card] {
        &self.pile.traits
    }

    /// Out of the game, or a holder for cards set aside in setup.
    pub fn removed(&self) -> bool {
        self.pile.removed
    }

    /// Face down: none of its cards is public.
    pub fn hidden(&self) -> bool {
        self.pile.hidden
    }
}

/// A deck of Boons or Hexes: how many are left to draw, and its face-up discard pile.
#[derive(Clone, Copy, Debug)]
pub struct CardDeckView<'a> {
    pub draw_size: usize,
    /// Top last.
    pub discard: &'a [Card],
}

impl<'a> CardDeckView<'a> {
    fn of(deck: &'a crate::CardDeck) -> CardDeckView<'a> {
        CardDeckView {
            draw_size: deck.draw.len(),
            discard: &deck.discard,
        }
    }
}

/// Public state a card or landscape (`source`) keeps outside the zones and tokens
/// [`View`] shows directly: Young Witch's Bane pile, Ferryman's and Obelisk's piles, the
/// Black Market deck, Way of the Mouse's and Riverboat's cards, Druid's Boons, the
/// card each player set aside with Inheritance, cards Exiled with Invest, VP on
/// Landmarks, Family of Inventors' Favors on piles, Sinister Plot's tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub source: Card,
    pub player: Option<PlayerId>,
    pub pile: Option<PileId>,
    pub card: Option<Card>,
    /// Tokens or VP, where it keeps some.
    pub count: i32,
}

impl Note {
    pub(crate) fn new(source: Card) -> Note {
        Note {
            source,
            player: None,
            pile: None,
            card: None,
            count: 0,
        }
    }

    pub(crate) fn player(mut self, p: PlayerId) -> Note {
        self.player = Some(p);
        self
    }

    pub(crate) fn pile(mut self, pile: PileId) -> Note {
        self.pile = Some(pile);
        self
    }

    pub(crate) fn card(mut self, card: Card) -> Note {
        self.card = Some(card);
        self
    }

    pub(crate) fn count(mut self, n: i32) -> Note {
        self.count = n;
        self
    }
}
