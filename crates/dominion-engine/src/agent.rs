//! The interface for anything that plays: bots, UIs, network clients.

use crate::decision::{Decision, DecisionKind, Move, Purpose, Response, ResponseError};
use crate::frame::Frame;
use crate::game::{
    EndReason, ExtraTurn, Lasting, Mat, Outcome, Pile, Player, Token, Turn, Until, Zone,
};
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
///
/// Someone watching without a seat ([`Game::watch`]) sees only what is public.
#[derive(Clone, Copy)]
pub struct View<'a> {
    game: &'a Game,
    /// `None` for someone watching.
    me: Option<PlayerId>,
}

impl Game {
    pub fn view(&self, player: PlayerId) -> View<'_> {
        View {
            game: self,
            me: Some(player),
        }
    }

    /// The game as someone who is no player sees it: what every player may see.
    pub fn watch(&self) -> View<'_> {
        View {
            game: self,
            me: None,
        }
    }
}

impl<'a> View<'a> {
    /// The player whose view this is; `None` for someone watching.
    pub fn me(&self) -> Option<PlayerId> {
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
        self.game.pending().filter(|d| Some(d.player) == self.me)
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

    /// The occasion each of [`View::pending_triggers`] answers (`[i]`).
    fn pending_occasions(&self) -> Vec<&'a crate::Occasion> {
        match (self.pending(), self.game.stack.last()) {
            (Some(d), Some(Frame::Window(w))) if d.purpose == Purpose::Trigger => {
                w.asked_occasions().collect()
            }
            _ => vec![],
        }
    }

    /// The card the occasion of pending trigger `i` is about, where this player knows it:
    /// one they gain, wherever it went, or one they can see.
    pub fn pending_occasion_card(&self, i: usize) -> Option<Card> {
        let me = self.me?;
        match **self.pending_occasions().get(i)? {
            crate::Occasion::Gain { player, card, .. } if player == me => {
                Some(self.game.card(card))
            }
            crate::Occasion::Gain { card, .. }
            | crate::Occasion::Buy { card, .. }
            | crate::Occasion::Play { card, .. }
            | crate::Occasion::AfterPlay { card, .. } => self.card(card),
            _ => None,
        }
    }

    fn pending_card_frame(&self) -> Option<&'a crate::cards::CardFrame> {
        let source = self.pending()?.source?;
        self.game.stack.iter().rev().find_map(|frame| match frame {
            Frame::Card(f) if f.card == source => Some(f.as_ref()),
            _ => None,
        })
    }

    /// The actual source of this choice, when its physical identity is visible.
    pub fn pending_source(&self) -> Option<CardId> {
        self.pending_card_frame()?
            .this
            .filter(|&id| self.visible(id))
    }

    /// The step the asking card ability resumes at: which of its questions this is.
    pub fn pending_step(&self) -> Option<u8> {
        Some(self.pending_card_frame()?.pc)
    }

    /// The asking ability's own counter (`Scratch::n`): which round of a repeated
    /// question this is, for the scripts that count rounds there (Remake, Stonemason).
    pub fn pending_counter(&self) -> Option<i32> {
        Some(self.pending_card_frame()?.s.n)
    }

    /// A card the asking ability names and everyone has seen, kept by name (`Scratch::card`):
    /// the Boon Blessed Village or Sacred Grove is giving.
    pub fn pending_named(&self) -> Option<Card> {
        let f = self.pending_card_frame()?;
        matches!(f.card, Card::BlessedVillage | Card::SacredGrove)
            .then_some(f.s.card)
            .flatten()
    }

    /// The card in play whose pending Way question this player is answering.
    pub fn pending_way_card(&self) -> Option<CardId> {
        self.pending()?;
        self.game.way_card().filter(|&id| self.visible(id))
    }

    /// A visible card this effect retained across its preceding steps.
    pub fn pending_reference(&self) -> Option<CardId> {
        self.pending_card_frame()?
            .s
            .pick
            .filter(|&id| self.visible(id))
    }

    /// The cost bound stated by the effect, before filtering available piles.
    pub fn pending_cost_limit(&self) -> Option<Cost> {
        self.pending_card_frame()?.choice_cost
    }

    /// The explicitly recorded affected player, which may differ from the decision maker.
    pub fn pending_subject(&self) -> Option<PlayerId> {
        let f = self.pending_card_frame()?;
        f.choice_subject
    }

    /// The original pile of a legally visible card, including split piles.
    pub fn origin_pile(&self, id: CardId) -> Option<PileId> {
        self.visible(id)
            .then(|| self.game.origin_pile(self.game.printed(id)))
            .flatten()
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
            Zone::Hand(p) => Some(p) == self.me,
            Zone::SetAside(p) | Zone::Looking(p) => {
                Some(p) == self.me || g.player(p).face_up.contains(&id)
            }
            Zone::InPlay(_) | Zone::Trash => true,
            Zone::Discard(p) => g.player(p).discard.last() == Some(&id),
            Zone::Pile(pile) => !g.pile(pile).hidden && g.pile_top(pile) == Some(id),
            Zone::Mat(p, mat) => mat != Mat::NativeVillage || Some(p) == self.me,
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
            || self
                .pending_triggers()
                .iter()
                .any(|t| t.source == Some(id) && t.owner == d.player)
    }

    /// This player's hand; no cards for someone watching.
    pub fn hand(&self) -> &'a [CardId] {
        self.me.map_or(&[], |me| self.game.hand(me))
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
            game: self.game,
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
                l.holds.retain(seen);
                l
            })
            .collect()
    }

    /// When lasting effect `l` ends, in its players' terms: a Duration's effect ends at the
    /// start of its owner's next turn, when its ability resolves (`Until::StartOfTurn`);
    /// `Until::Removed` is an effect that waits for something to happen.
    pub fn lasting_ends(&self, l: &Lasting) -> Until {
        let real = self.game.lasting().iter().find(|x| x.id == l.id);
        crate::cards::lasting_ends(self.game, real.unwrap_or(l))
    }

    /// The tokens on the card whose effect `l` is (Garrison's), if it keeps any.
    pub fn lasting_tokens(&self, l: &Lasting) -> Option<u32> {
        crate::cards::lasting_tokens(l)
    }

    /// Whether a lasting effect keeps card `id` in play past this turn's Clean-up.
    pub fn stays_in_play(&self, id: CardId) -> bool {
        self.game.lasting().iter().any(|l| l.keeps.contains(&id))
    }

    /// Whether card `id` has something of its own beside it: cards its lasting effect
    /// set aside (Haven's, seen or not), or tokens on it (Garrison's).
    pub fn holds(&self, id: CardId) -> bool {
        let g = self.game;
        let aside = |c: &CardId| matches!(g.zone(*c), Zone::SetAside(_));
        g.lasting()
            .iter()
            .filter(|l| l.source == Some(id))
            .any(|l| {
                crate::cards::lasting_tokens(l).is_some()
                    || l.data.pick.iter().chain(&l.data.cards).any(aside)
            })
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
        (Some(self.id) == self.view.me).then_some(&self.player.hand[..])
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
        (mat != Mat::NativeVillage || Some(self.id) == self.view.me).then(|| self.player.mat(mat))
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
    game: &'a Game,
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

    /// Its cards' names, top first, where anyone may look through it: every pile but a
    /// face-down or shuffled one (Knights, Ruins). Never their ids.
    pub fn names(&self) -> Option<Vec<Card>> {
        let shuffled = self
            .pile
            .structure
            .as_ref()
            .is_some_and(|s| s.kind == crate::PileKind::Shuffled);
        (!self.pile.hidden && !shuffled).then(|| {
            let g = self.game;
            self.pile.cards.iter().rev().map(|&id| g.card(id)).collect()
        })
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

    /// Blockades whose set-aside card is a copy of this pile's.
    pub fn blockades(&self) -> u32 {
        crate::cards::seaside::blockades(self.game, self.id)
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

    pub fn structure(&self) -> Option<&'a crate::PileStructure> {
        self.pile.structure.as_ref()
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
/// Landmarks, Family of Inventors' Favors on piles, Sinister Plot's tokens, Traits on piles
/// Divine Wind removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub source: Card,
    pub player: Option<PlayerId>,
    pub pile: Option<PileId>,
    pub card: Option<Card>,
    /// Tokens or VP, where it keeps some.
    pub count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupRole>,
}

/// A public setup relationship, distinct from a card's live state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SetupRole {
    SetAside,
    Bane,
    ExtraPile,
    ChosenPile,
    /// Public unordered contents, never the deck's order.
    Deck,
}

impl Note {
    pub(crate) fn new(source: Card) -> Note {
        Note {
            source,
            player: None,
            pile: None,
            card: None,
            count: 0,
            setup: None,
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

    pub(crate) fn setup(mut self, role: SetupRole) -> Note {
        self.setup = Some(role);
        self
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    use crate::tests::{P0, P1, choose_kinds, deck, hand, play, setup};

    #[test]
    fn exact_gain_bounds_and_visible_sources_survive_old_frame_serialization() {
        for (card, trashed, expected) in [
            (Card::Mine, Card::Silver, Cost::coins(6)),
            (Card::Remodel, Card::Silver, Cost::coins(5)),
            (Card::Upgrade, Card::Silver, Cost::coins(4)),
        ] {
            let mut g = setup(&[
                card,
                Card::Village,
                Card::Smithy,
                Card::Caravan,
                Card::Market,
            ]);
            hand(&mut g, P0, &[card, trashed]);
            g.start();
            play(&mut g, card);
            let source = g.view(P0).pending_source().unwrap();
            assert_eq!(g.card(source), card);
            if matches!(g.pending().unwrap().kind, DecisionKind::Cards { .. }) {
                choose_kinds(&mut g, &[trashed]);
            }
            assert_eq!(g.view(P0).pending_cost_limit(), Some(expected));
            let mut value = serde_json::to_value(&g).unwrap();
            for frame in value["stack"].as_array_mut().unwrap() {
                if let Some(f) = frame.get_mut("Card") {
                    f.as_object_mut().unwrap().remove("choice_cost");
                    f.as_object_mut().unwrap().remove("choice_subject");
                }
            }
            let old: Game = serde_json::from_value(value).unwrap();
            assert_eq!(old.view(P0).pending_cost_limit(), None);
            assert_eq!(old.pending(), g.pending());
            assert!(old.view(P1).pending_source().is_none());
        }
    }

    #[test]
    fn swindler_presentation_records_affected_seat_without_exposing_private_card() {
        let mut g = setup(&[Card::Swindler, Card::Village]);
        hand(&mut g, P0, &[Card::Swindler]);
        deck(&mut g, P1, &[Card::Silver, Card::Copper]);
        g.start();
        // Setup draws P1's opening hand before the attack, so replace the deck afterwards.
        deck(&mut g, P1, &[Card::Silver]);
        play(&mut g, Card::Swindler);
        assert_eq!(g.pending().unwrap().player, P0);
        assert_eq!(g.view(P0).pending_subject(), Some(P1));
        assert_eq!(g.view(P0).pending_cost_limit(), Some(Cost::coins(3)));
        assert!(g.view(P1).pending_cost_limit().is_none());
    }
}
