use super::*;
use rand::seq::SliceRandom;

/// Every Boon and Hex is somewhere it can come back from.
fn check_boons_and_hexes(g: &Game) {
    let boons = g.boons();
    let held: usize = g.players.iter().map(|p| p.boons_kept.len()).sum();
    let saved = g
        .lasting()
        .iter()
        .filter(|l| l.card == Card::BlessedVillage)
        .count();
    let total = boons.draw.len() + boons.discard.len() + held + saved + g.ext.nocturne.druid.len();
    assert!(total == 0 || total == 12, "{total} Boons");
    let hexes = g.hexes().draw.len() + g.hexes().discard.len();
    assert!(hexes == 0 || hexes == 12, "{hexes} Hexes");
}

/// Random games over Nocturne with Throne Room and Moat, each player starting with a copy
/// of every Kingdom card, Heirloom and non-Supply card so that all of them get played.
#[test]
fn random_nocturne_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Nocturne)
        .filter(|c| c.def().kingdom)
        .collect();
    let extras = [
        Card::Bat,
        Card::Ghost,
        Card::Imp,
        Card::WillOWisp,
        Card::Wish,
        Card::CursedGold,
        Card::Goat,
        Card::HauntedMirror,
        Card::LuckyCoin,
        Card::MagicLamp,
        Card::Pasture,
        Card::Pouch,
    ];
    for seed in 0..150 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, cards.clone(), seed);
        config.turn_limit = Some(60);
        let mut g = setup_with(config);
        for p in 0..players {
            let p = PlayerId(p);
            for &c in cards.iter().chain(&extras) {
                g.create(c, Zone::Deck(p));
            }
            let mut deck = std::mem::take(&mut g.players[p.index()].deck);
            g.shuffle(&mut deck);
            g.players[p.index()].deck = deck;
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
        check_boons_and_hexes(&g);
    }
}
