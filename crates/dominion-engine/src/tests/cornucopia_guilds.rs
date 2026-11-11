use super::*;
use crate::game::Zone;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Random games with kingdoms from `sets` (removed cards included), each player starting
/// with a copy of every Kingdom card and of `extra` so that expensive cards get played.
/// Returns how often each card was played.
pub(crate) fn fuzz_sets(sets: &[Set], extra: &[Card], games: u64) -> Vec<(Card, usize)> {
    let pool: Vec<Card> = Card::kingdom_cards()
        .filter(|c| sets.contains(&c.set()))
        .collect();
    let mut plays = std::collections::BTreeMap::new();
    for seed in 0..games {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut kingdom = pool.clone();
        kingdom.shuffle(&mut rng);
        kingdom.truncate(10);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, kingdom.clone(), seed);
        config.turn_limit = Some(40);
        let mut g = setup_with(config);
        for p in 0..players {
            for &c in kingdom.iter().chain(extra) {
                g.create(c, Zone::Discard(PlayerId(p)));
            }
        }
        g.start();
        let mut agents: Vec<Chaos> = (0..players)
            .map(|p| Chaos(ChaCha8Rng::seed_from_u64(seed * 10 + p as u64)))
            .collect();
        let mut steps = 0;
        while let Some(d) = g.pending().cloned() {
            check_zones(&g);
            let r = agents[d.player.index()].decide(&g.view(d.player), &d);
            if let Err(e) = g.respond(r.clone()) {
                panic!("seed {seed}: random response {r:?} to {d:?} rejected: {e}");
            }
            steps += 1;
            assert!(steps < 100_000, "seed {seed}: game does not end");
        }
        check_zones(&g);
        for e in g.log() {
            if let Event::Played { card, .. } = e {
                *plays.entry(*card).or_insert(0) += 1;
            }
        }
    }
    plays.into_iter().collect()
}

#[test]
fn random_games_with_every_card() {
    let mut extra = vec![Card::Province, Card::Gold];
    extra.extend(Card::in_set(Set::CornucopiaGuilds).filter(|c| !c.def().kingdom));
    let plays = fuzz_sets(&[Set::CornucopiaGuilds], &extra, 300);
    for card in Card::in_set(Set::CornucopiaGuilds) {
        if !card.is(Types::VICTORY) || card.is(Types::ACTION) {
            assert!(plays.iter().any(|&(c, _)| c == card), "{card} never played");
        }
    }
}
