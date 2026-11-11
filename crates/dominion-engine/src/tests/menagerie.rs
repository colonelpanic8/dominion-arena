use super::intrigue::{modes, random_games_with_copies, trigger, trigger_labels};
use super::*;

fn landscapes_of(kind: Kind) -> Vec<Card> {
    Card::in_set(Set::Menagerie)
        .filter(|c| c.kind() == kind)
        .collect()
}

/// Random games over Menagerie, each with one of its Ways and one of its Events.
#[test]
fn random_menagerie_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Menagerie)
        .filter(|c| c.def().kingdom)
        .collect();
    let ways = landscapes_of(Kind::Way);
    let events = landscapes_of(Kind::Event);
    random_games_with_copies(&pool, &[Card::Horse], 0..160, |seed| {
        let i = seed as usize;
        vec![ways[i % ways.len()], events[(i / 2) % events.len()]]
    });
}
