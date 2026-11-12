use crate::game::Token;
use crate::{Card, PileId, PlayerId, Zone};
use serde::{Deserialize, Serialize};

/// Where a gained card goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dest {
    Discard,
    Hand,
    DeckTop,
    /// Set aside by the effect that gains it (Blockade, ...), which tracks it.
    SetAside,
}

/// Something that happened, in order. The log holds only public information; e.g.
/// [`Event::Drew`] records a count, not which cards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    TurnStart {
        player: PlayerId,
        turn: u32,
    },
    Played {
        player: PlayerId,
        card: Card,
    },
    /// Used a Way instead of following the played card's instructions.
    UsedWay {
        player: PlayerId,
        card: Card,
        way: Card,
    },
    /// Named a card (Wishing Well, Contraband, ...), which everyone hears.
    Named {
        player: PlayerId,
        card: Card,
    },
    Bought {
        player: PlayerId,
        card: Card,
    },
    BoughtLandscape {
        player: PlayerId,
        card: Card,
    },
    Overpaid {
        player: PlayerId,
        amount: u32,
        potions: u32,
    },
    Gained {
        player: PlayerId,
        card: Card,
        to: Dest,
    },
    Trashed {
        player: PlayerId,
        card: Card,
    },
    Discarded {
        player: PlayerId,
        card: Card,
    },
    /// A card went onto the deck: `card` if everyone knew which it was.
    TopDecked {
        player: PlayerId,
        #[serde(default)]
        card: Option<Card>,
    },
    SetAside {
        player: PlayerId,
        card: Card,
    },
    Revealed {
        player: PlayerId,
        card: Card,
    },
    RevealedHand {
        player: PlayerId,
        cards: Vec<Card>,
    },
    Drew {
        player: PlayerId,
        count: usize,
    },
    Shuffled {
        player: PlayerId,
    },
    /// Received a Boon, Hex, State or Artifact, or took a token.
    Received {
        player: PlayerId,
        card: Card,
    },
    /// A pile's top card and the copies of it right under it went to its bottom.
    Rotated {
        player: PlayerId,
        pile: crate::PileId,
    },
    /// Cards moved from one zone to another with no event of their own (onto a mat, into a
    /// hand from set aside, a deck into its discard pile, back to a pile, ...): `count` of
    /// them, of which `cards` are the ones everyone knew which they were. `player` is the
    /// owner of `from`, else of `to`. Logged by the game itself, for every card moved
    /// with no other event. (`from` is `None` only in logs saved before it was recorded.)
    Moved {
        player: PlayerId,
        #[serde(default)]
        from: Option<Zone>,
        to: Zone,
        #[serde(default)]
        cards: Vec<Card>,
        #[serde(default = "one")]
        count: usize,
        /// Under the cards already in `to`.
        #[serde(default)]
        bottom: bool,
    },
    /// `player` looked at the top `count` cards of their deck.
    Looked {
        player: PlayerId,
        count: usize,
    },
    /// A counter changed by `delta`: a player's (`player`), a pile's (`pile`) or the
    /// game's (neither).
    Counter {
        player: Option<PlayerId>,
        pile: Option<PileId>,
        counter: Counter,
        delta: i32,
    },
    /// `player`'s Adventures `token` went onto `pile`, or off its pile (`None`).
    TokenMoved {
        player: PlayerId,
        token: Token,
        pile: Option<PileId>,
    },
    /// `player` took (`on`) or lost a marker token.
    Marker {
        player: PlayerId,
        marker: Marker,
        on: bool,
    },
    /// `player` returned a State.
    Returned {
        player: PlayerId,
        card: Card,
    },
    /// An ability of `card` of `player`'s began to resolve (a Duration's, a Reaction's, a
    /// landscape's, ...): logged before the first thing it does.
    Resolved {
        player: PlayerId,
        card: Card,
    },
    /// +Actions, +Buys or +$ that `player` got other than from a card's own play (from a
    /// Duration's later part, a landscape, a Boon, ...), after what gave them.
    Bonus {
        player: PlayerId,
        actions: u32,
        buys: u32,
        coins: u32,
    },
    /// `player` is unaffected by `attack` because of `by`.
    Unaffected {
        player: PlayerId,
        attack: Card,
        by: Card,
    },
    /// `player` returned to their Action phase.
    ReturnedToAction {
        player: PlayerId,
    },
    /// `player` called `card` from their Tavern mat.
    Called {
        player: PlayerId,
        card: Card,
    },
}

fn one() -> usize {
    1
}

/// A count of tokens [`Event::Counter`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Counter {
    Coffers,
    Villagers,
    Favors,
    Debt,
    VpTokens,
    /// Coin tokens on a Pirate Ship mat.
    PirateShip,
    /// Embargo's Curse tokens.
    Embargo,
    /// Trade Route's coin token on a pile, or coin tokens on the Trade Route mat.
    TradeRoute,
    /// Sun tokens on the Prophecy.
    Sun,
}

/// A token a player has or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Marker {
    MinusCard,
    MinusCoin,
    /// The Journey token; `on` is face up.
    Journey,
}
