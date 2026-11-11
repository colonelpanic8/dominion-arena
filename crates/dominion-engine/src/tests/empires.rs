use super::intrigue::{modes, random_games_with_copies, trigger};
use super::*;

/// Random games over Empires with copies of its cards in every deck, and two random
/// Events or Landmarks.
#[test]
fn random_empires_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Empires)
        .filter(|c| c.def().kingdom)
        .collect();
    let landscapes: Vec<Card> = Card::in_set(Set::Empires)
        .filter(|c| c.kind().is_landscape())
        .collect();
    let extra: Vec<Card> = Card::in_set(Set::Empires)
        .filter(|c| c.kind() == Kind::Card && !c.def().kingdom)
        .collect();
    random_games_with_copies(&pool, &extra, 0..fuzz_seeds(150), |seed| {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xe);
        let mut l = landscapes.clone();
        l.shuffle(&mut rng);
        l.truncate(3);
        l
    });
}
