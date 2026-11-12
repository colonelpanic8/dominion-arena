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
    /// Its option labels (or number and question) may be announced in the public log.
    #[serde(default)]
    pub public_choice: bool,
    /// The card whose effect is asking, if any.
    pub source: Option<Card>,
    /// Cards the question is about that the deciding player may see though they aren't
    /// its options (cards they look at while asked what to do with them).
    #[serde(default)]
    pub about: Vec<CardId>,
    /// What is asked, in words, where the purpose and options don't say it: for a `Cards`
    /// decision what happens to the chosen cards ("put into your hand"), otherwise the
    /// whole question ("choose a Supply pile for an Embargo token", what a number means,
    /// the moment triggers are ordered at).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// The options whose answer starts its player playing an Action card they see, before
    /// anything else is asked (a Reaction that plays itself, "play it?"): those a Way
    /// question may follow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plays: Vec<Opt>,
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
    /// Choose among a card's abilities (its "choose" options). Elder may add a pick to
    /// these.
    Mode,
    /// Play a card as itself (`Opt::No`) or use one of the Ways offered (`Opt::Card`).
    Way,
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
    #[error("the pending decision asks for {expected}")]
    WrongKind { expected: &'static str },
    #[error("{0:?} is not one of the moves you can make")]
    InvalidMove(Move),
    #[error("{0:?} is not one of the cards to choose from")]
    NotACandidate(CardId),
    #[error("{0:?} is chosen more than once")]
    Duplicate(CardId),
    #[error("choose {}; you chose {got}", expected_count(*min, *max))]
    WrongCount { got: usize, min: usize, max: usize },
    #[error("that pile is not one of the piles to choose from")]
    InvalidPile,
    #[error("a pile must be chosen")]
    PileRequired,
    #[error("{0:?} is not one of the options")]
    InvalidOption(Opt),
    #[error("{0:?} is chosen more than once")]
    DuplicateOption(Opt),
    #[error("{0} is not a card that can be named")]
    InvalidName(Card),
    #[error("choose a number from {min} to {max}; {got} is not")]
    OutOfRange { got: i32, min: i32, max: i32 },
}

/// "exactly 1", "up to 3", "1 to 3".
fn expected_count(min: usize, max: usize) -> String {
    match (min, max) {
        (a, b) if a == b => format!("exactly {a}"),
        (0, b) => format!("up to {b}"),
        (a, b) => format!("{a} to {b}"),
    }
}

impl DecisionKind {
    /// What an answer to it is, for error messages.
    fn answer_kind(&self) -> &'static str {
        match self {
            DecisionKind::Turn { .. } => "a move",
            DecisionKind::Cards { .. } => "cards",
            DecisionKind::Pile { .. } => "a pile",
            DecisionKind::Choose { .. } => "options",
            DecisionKind::NameCard => "a card name",
            DecisionKind::Number { .. } => "a number",
        }
    }
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
            (kind, _) => Err(ResponseError::WrongKind {
                expected: kind.answer_kind(),
            }),
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
