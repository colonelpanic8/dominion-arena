//! Scripted seats for tests that play whole games: shared by the fold test and the
//! generator of the spec's example streams, whose games must not change when one does.

use dominion_engine::{Card, DecisionKind, Game, Move, Response};
use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};
use rand_chacha::ChaCha8Rng;

/// A random legal response.
pub fn random_response(game: &Game, rng: &mut ChaCha8Rng) -> Response {
    let d = game.pending().expect("a decision");
    let some = |n: usize, min: usize, max: usize, rng: &mut ChaCha8Rng| {
        let mut picks: Vec<usize> = (0..n).collect();
        picks.shuffle(rng);
        picks.truncate(rng.random_range(min..=max.min(n)));
        picks
    };
    match &d.kind {
        DecisionKind::Turn { moves, .. } => Response::Move(*moves.choose(rng).expect("a move")),
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ..
        } => Response::Cards(
            (some(candidates.len(), *min, *max, rng).into_iter())
                .map(|i| candidates[i])
                .collect(),
        ),
        DecisionKind::Pile { piles, optional } => {
            Response::Pile(match *optional && rng.random_bool(0.2) {
                true => None,
                false => piles.choose(rng).copied(),
            })
        }
        DecisionKind::Choose {
            options, min, max, ..
        } => Response::Options(
            (some(options.len(), *min, *max, rng).into_iter())
                .map(|i| options[i])
                .collect(),
        ),
        DecisionKind::NameCard => Response::Name(
            *[
                Card::Copper,
                Card::Silver,
                Card::Gold,
                Card::Estate,
                Card::Curse,
            ]
            .choose(rng)
            .expect("a name"),
        ),
        DecisionKind::Number { min, max } => Response::Number(rng.random_range(*min..=*max)),
    }
}

/// A response that moves a game on: Actions and Treasures played, the dearest thing
/// bought, so that games end on piles and Fleet's round is played.
pub fn eager_response(game: &Game, rng: &mut ChaCha8Rng) -> Response {
    let d = game.pending().expect("a decision");
    if let DecisionKind::Turn { moves, .. } = &d.kind
        && rng.random_bool(0.9)
    {
        let plays: Vec<&Move> = (moves.iter())
            .filter(|m| matches!(m, Move::Play(_)))
            .collect();
        if let Some(m) = plays.choose(rng) {
            return Response::Move(**m);
        }
        let price = |m: &Move| {
            let cost = match *m {
                Move::Buy(pile) => game.cost_of(*game.pile(pile).cards.last().expect("a card")),
                Move::BuyLandscape(card) => game.cost(card),
                _ => return None,
            };
            Some(cost.coins as u32 + cost.debt as u32 + 3 * cost.potions as u32)
        };
        if let Some(m) = (moves.iter())
            .filter(|m| price(m).is_some_and(|p| p > 1))
            .max_by_key(|m| (price(m), rng.random::<u8>()))
        {
            return Response::Move(*m);
        }
    }
    random_response(game, rng)
}
