use crate::game::Phase;
use crate::{Card, CardId, Game, PileId, PlayerId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

/// A choice the engine is waiting on. Answer it with [`Game::respond`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Who decides: the player affected.
    pub player: PlayerId,
    pub kind: DecisionKind,
    pub purpose: Purpose,
    /// The card whose effect is asking, if any.
    pub source: Option<Card>,
    /// Cards the question is about that the deciding player may see though they aren't
    /// its options (cards they look at while asked what to do with them).
    #[serde(default)]
    pub about: Vec<CardId>,
}

/// Something a player can do in one of their turn's phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Move {
    /// Play a card from hand (an Action in the Action phase, a Treasure in the Buy phase,
    /// a Night card in the Night phase).
    Play(CardId),
    /// Buy the top card of a pile.
    Buy(PileId),
    /// Buy an Event or Project.
    BuyLandscape(Card),
    /// Spend a Coffers token for +$1.
    Coffers,
    /// Spend a Villager for +1 Action.
    Villager,
    /// Pay off one Debt with $1.
    PayDebt,
    EndPhase,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionKind {
    /// The main decision of a phase: pick one of `moves` with [`Response::Move`].
    Turn { phase: Phase, moves: Vec<Move> },
    /// Choose between `min` and `max` of `candidates` with [`Response::Cards`].
    /// When `ordered`, the response lists them in the order they should end up, top first.
    Cards {
        candidates: Vec<CardId>,
        min: usize,
        max: usize,
        ordered: bool,
    },
    /// Choose a pile with [`Response::Pile`]; `None` is allowed only when `optional`.
    Pile { piles: Vec<PileId>, optional: bool },
    /// Choose between `min` and `max` different `options` with [`Response::Options`].
    /// `labels[i]` describes `options[i]` for display.
    Choose {
        options: Vec<Opt>,
        labels: Vec<String>,
        min: usize,
        max: usize,
    },
    /// Name any card with [`Response::Name`].
    NameCard,
    /// Choose a number in `min..=max` with [`Response::Number`].
    Number { min: i32, max: i32 },
}

/// Why a decision is being asked, so agents can pick a strategy without parsing card text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Purpose {
    Turn,
    Discard,
    Trash,
    Gain,
    TopDeck,
    /// Put cards somewhere other than the deck top (hand, a mat, under a card, ...).
    Move,
    Play,
    SetAside,
    Reveal,
    Reaction,
    /// Order or accept triggered abilities.
    Trigger,
    /// Choose among a card's abilities (its "choose" options) or a way of playing a card.
    /// Elder may add a pick to these.
    Mode,
    /// Pick one of alternatives that aren't a "choose" ("either ... or", which Boon or
    /// Artifact to take, now or next turn). Elder doesn't add picks to these.
    Either,
    Exile,
    Return,
    Pass,
    Overpay,
    Name,
    Other,
}

/// An option in a [`DecisionKind::Choose`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Opt {
    Yes,
    No,
    /// The `n`th of the source card's numbered choices (e.g. Pawn's options, top to bottom).
    Mode(u8),
    /// A card or landscape, e.g. a Way, a Boon to keep, a pile's card.
    Card(Card),
    Player(PlayerId),
    /// The `n`th pending triggered ability.
    Trigger(u16),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Move(Move),
    Cards(Vec<CardId>),
    Pile(Option<PileId>),
    Options(Vec<Opt>),
    Name(Card),
    Number(i32),
}

impl Response {
    /// A single-option answer.
    pub fn option(opt: Opt) -> Response {
        Response::Options(vec![opt])
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResponseError {
    #[error("no decision is pending")]
    NoPendingDecision,
    #[error("response does not match the pending decision's kind")]
    WrongKind,
    #[error("move {0:?} is not available")]
    InvalidMove(Move),
    #[error("card {0:?} is not one of the choices")]
    NotACandidate(CardId),
    #[error("card {0:?} was chosen more than once")]
    Duplicate(CardId),
    #[error("chose {got} items, expected {min}..={max}")]
    WrongCount { got: usize, min: usize, max: usize },
    #[error("pile is not one of the choices")]
    InvalidPile,
    #[error("a pile must be chosen")]
    PileRequired,
    #[error("option {0:?} is not one of the choices")]
    InvalidOption(Opt),
    #[error("option {0:?} was chosen more than once")]
    DuplicateOption(Opt),
    #[error("{0} cannot be named")]
    InvalidName(Card),
    #[error("{got} is outside {min}..={max}")]
    OutOfRange { got: i32, min: i32, max: i32 },
}

fn ensure(cond: bool, err: ResponseError) -> Result<(), ResponseError> {
    if cond { Ok(()) } else { Err(err) }
}

fn check_count(got: usize, min: usize, max: usize) -> Result<(), ResponseError> {
    ensure(
        (min..=max).contains(&got),
        ResponseError::WrongCount { got, min, max },
    )
}

impl Decision {
    pub fn validate(&self, response: &Response) -> Result<(), ResponseError> {
        use DecisionKind as K;
        use Response as R;
        match (&self.kind, response) {
            (K::Turn { moves, .. }, R::Move(m)) => {
                ensure(moves.contains(m), ResponseError::InvalidMove(*m))
            }
            (
                K::Cards {
                    candidates,
                    min,
                    max,
                    ..
                },
                R::Cards(chosen),
            ) => {
                let mut seen = HashSet::new();
                for c in chosen {
                    ensure(candidates.contains(c), ResponseError::NotACandidate(*c))?;
                    ensure(seen.insert(*c), ResponseError::Duplicate(*c))?;
                }
                check_count(chosen.len(), *min, *max)
            }
            (K::Pile { piles, .. }, R::Pile(Some(p))) => {
                ensure(piles.contains(p), ResponseError::InvalidPile)
            }
            (K::Pile { optional, .. }, R::Pile(None)) => {
                ensure(*optional, ResponseError::PileRequired)
            }
            (
                K::Choose {
                    options, min, max, ..
                },
                R::Options(chosen),
            ) => {
                let mut seen = HashSet::new();
                for o in chosen {
                    ensure(options.contains(o), ResponseError::InvalidOption(*o))?;
                    ensure(seen.insert(*o), ResponseError::DuplicateOption(*o))?;
                }
                check_count(chosen.len(), *min, *max)
            }
            (K::NameCard, R::Name(card)) => ensure(
                card.kind() == crate::Kind::Card,
                ResponseError::InvalidName(*card),
            ),
            (K::Number { min, max }, R::Number(n)) => ensure(
                (*min..=*max).contains(n),
                ResponseError::OutOfRange {
                    got: *n,
                    min: *min,
                    max: *max,
                },
            ),
            _ => Err(ResponseError::WrongKind),
        }
    }

    /// The only meaningful answer, if there is just one. Such decisions are never surfaced.
    pub(crate) fn forced(&self, game: &Game) -> Option<Response> {
        match &self.kind {
            DecisionKind::Turn { moves, .. } if moves.len() == 1 => Some(Response::Move(moves[0])),
            DecisionKind::Cards {
                candidates,
                min,
                max,
                ordered,
            } => {
                if *max == 0 {
                    return Some(Response::Cards(vec![]));
                }
                let all_same = candidates
                    .iter()
                    .all(|c| game.card(*c) == game.card(candidates[0]));
                let take_all = *min == candidates.len();
                if (take_all && (!ordered || all_same)) || (min == max && all_same) {
                    return Some(Response::Cards(candidates[..*min].to_vec()));
                }
                None
            }
            DecisionKind::Pile { piles, optional } if !optional && piles.len() == 1 => {
                Some(Response::Pile(Some(piles[0])))
            }
            DecisionKind::Choose {
                options, min, max, ..
            } => {
                if *max == 0 {
                    Some(Response::Options(vec![]))
                } else if *min == options.len() {
                    Some(Response::Options(options.clone()))
                } else {
                    None
                }
            }
            DecisionKind::Number { min, max } if min == max => Some(Response::Number(*min)),
            _ => None,
        }
    }
}
