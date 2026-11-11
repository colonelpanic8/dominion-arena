use crate::{Card, PlayerId};
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
    /// A card went onto the deck. Which card is public only if it was revealed.
    TopDecked {
        player: PlayerId,
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
    /// A card moved somewhere not covered by the other events (a mat, Exile, back to its
    /// pile, ...). `card` is `None` when only its owner may know which card it was.
    Moved {
        player: PlayerId,
        card: Option<Card>,
        to: crate::Zone,
    },
}
