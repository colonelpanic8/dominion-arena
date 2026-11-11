//! An opt-in, id-level record of what happens to physical cards, for tracking what each
//! player can know (see [`crate::Knowledge`]). [`Game::new_traced`] turns it on; untraced
//! games record nothing.

use crate::game::{Player, Zone};
use crate::{Card, CardId, Game, PlayerId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trace {
    /// A card came into the game. Consecutive cards created in the same place (a player's
    /// starting cards, a pile) came in together.
    Created { card: CardId, is: Card, zone: Zone },
    /// A card moved. `top` says it was the top card of `from` (the last one, for a deck,
    /// discard pile or pile).
    Moved {
        card: CardId,
        from: Zone,
        to: Zone,
        top: bool,
    },
    /// These cards were rearranged where no one could follow them (a shuffle, or cards put
    /// back in an order their owner chose).
    Mixed { cards: Vec<CardId> },
    /// `card` was shown to everyone (`to: None`) or to one player (`Some`).
    Shown { card: CardId, to: Option<PlayerId> },
    /// A turn began: `player`'s.
    Turn { player: PlayerId },
    /// `card` became another card, publicly (Inherited's setup swap).
    Changed { card: CardId, from: Card, to: Card },
}

impl Game {
    /// Starts a game that records a [`Trace`], beginning with every card in it.
    pub fn new_traced(config: crate::GameConfig) -> Result<Game, crate::SetupError> {
        let mut game = crate::setup::build(config)?;
        game.enable_trace();
        game.start();
        Ok(game)
    }

    pub(crate) fn enable_trace(&mut self) {
        let players = self.players.iter().flat_map(Player::owned);
        let piles = self.piles.iter().flat_map(|p| p.cards.iter().copied());
        let trace = players
            .chain(piles)
            .chain(self.trash.iter().copied())
            .map(|card| Trace::Created {
                card,
                is: self.printed(card),
                zone: self.zone(card),
            })
            .collect();
        self.trace = Some(trace);
    }

    /// The trace recorded since it was last taken, if this game is traced.
    pub fn trace(&self) -> Option<&[Trace]> {
        self.trace.as_deref()
    }

    /// Takes the trace recorded so far, leaving it empty (and still recording).
    pub fn take_trace(&mut self) -> Vec<Trace> {
        self.trace.as_mut().map(std::mem::take).unwrap_or_default()
    }

    pub(crate) fn record_turn(&mut self) {
        let player = self.turn.player;
        self.record(|| Trace::Turn { player });
    }

    pub(crate) fn record(&mut self, entry: impl FnOnce() -> Trace) {
        if let Some(trace) = &mut self.trace {
            trace.push(entry());
        }
    }

    /// Records that `card` was revealed to everyone.
    pub(crate) fn show(&mut self, card: CardId) {
        self.record(|| Trace::Shown { card, to: None });
    }

    /// Records that `p` looked at `card`.
    pub(crate) fn show_to(&mut self, card: CardId, p: PlayerId) {
        self.record(|| Trace::Shown { card, to: Some(p) });
    }

    /// Records that `p` looked through their discard pile.
    pub(crate) fn look_through_discard(&mut self, p: PlayerId) {
        if self.trace.is_some() {
            for card in self.players[p.index()].discard.clone() {
                self.show_to(card, p);
            }
        }
    }

    /// Reveals `card` (one of `p`'s, or one they revealed): logs it and records it shown.
    pub(crate) fn reveal(&mut self, p: PlayerId, card: CardId) {
        self.show(card);
        self.emit(crate::Event::Revealed {
            player: p,
            card: self.card(card),
        });
    }
}
