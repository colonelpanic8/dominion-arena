//! The interface for anything that plays: bots, UIs, network clients.

use crate::decision::{Decision, DecisionKind, Response, ResponseError};
use crate::game::{Outcome, Turn, Zone};
use crate::{Card, CardId, Cost, Event, Game, PileId, PlayerId};
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

/// The game as one player is allowed to see it.
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

    /// What `id` is, if this player can see it: own hand, anything in play, the trash,
    /// pile tops, discard tops, own set-aside cards, and cards offered in a decision
    /// addressed to this player.
    pub fn card(&self, id: CardId) -> Option<Card> {
        let g = self.game;
        let visible = match g.zone(id) {
            Zone::Hand(p) | Zone::SetAside(p) => self.sees_for(p),
            Zone::InPlay(_) | Zone::Trash => true,
            Zone::Discard(p) => g.player(p).discard.last() == Some(&id),
            Zone::Pile(pile) => !g.pile(pile).hidden && g.pile_top(pile) == Some(id),
            Zone::Mat(p, mat) => self.sees_for(p) || mat != crate::Mat::NativeVillage,
            Zone::Deck(_) => false,
        };
        (visible || self.offered(id)).then(|| g.card(id))
    }

    /// Whether this player sees what `p` sees: their own cards, or those of a player
    /// whose turn they control (Possession).
    fn sees_for(&self, p: PlayerId) -> bool {
        let turn = self.game.turn();
        p == self.me || (turn.player == p && turn.controller == self.me)
    }

    fn offered(&self, id: CardId) -> bool {
        let Some(d) = self.game.pending().filter(|d| d.player == self.me) else {
            return false;
        };
        match &d.kind {
            DecisionKind::Turn { moves, .. } => moves.contains(&crate::Move::Play(id)),
            DecisionKind::Cards { candidates, .. } => candidates.contains(&id),
            _ => false,
        }
    }

    pub fn hand(&self) -> &'a [CardId] {
        self.game.hand(self.me)
    }

    pub fn in_play(&self, p: PlayerId) -> &'a [CardId] {
        &self.game.player(p).in_play
    }

    pub fn hand_size(&self, p: PlayerId) -> usize {
        self.game.player(p).hand.len()
    }

    pub fn deck_size(&self, p: PlayerId) -> usize {
        self.game.player(p).deck.len()
    }

    pub fn discard_size(&self, p: PlayerId) -> usize {
        self.game.player(p).discard.len()
    }

    pub fn discard_top(&self, p: PlayerId) -> Option<Card> {
        self.game
            .player(p)
            .discard
            .last()
            .map(|&c| self.game.card(c))
    }

    /// Every card each player has gained is public, so totals are too.
    pub fn score(&self, p: PlayerId) -> i32 {
        self.game.score(p)
    }

    pub fn num_piles(&self) -> usize {
        self.game.piles().len()
    }

    /// The card a pile is named after.
    pub fn pile_name(&self, id: PileId) -> Card {
        self.game.pile(id).card
    }

    pub fn pile_size(&self, id: PileId) -> usize {
        self.game.pile(id).cards.len()
    }

    pub fn pile_top(&self, id: PileId) -> Option<Card> {
        if self.game.pile(id).hidden {
            return None;
        }
        self.game.pile_top(id).map(|c| self.game.card(c))
    }

    pub fn supply_pile(&self, card: Card) -> Option<PileId> {
        self.game.supply_pile(card)
    }

    pub fn empty_supply_piles(&self) -> usize {
        self.game.empty_supply_piles()
    }

    pub fn trash(&self) -> impl Iterator<Item = Card> + 'a {
        let g = self.game;
        g.trash().iter().map(move |&c| g.card(c))
    }

    pub fn cost(&self, card: Card) -> Cost {
        self.game.cost(card)
    }
}
