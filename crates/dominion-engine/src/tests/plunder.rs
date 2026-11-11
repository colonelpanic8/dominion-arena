use super::*;
use rand::seq::SliceRandom;

fn turn_starts(g: &Game) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count()
}

/// Ends phases until the next turn starts.
fn next_turn(g: &mut Game) {
    let n = turn_starts(g);
    while turn_starts(g) == n {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

/// Ends the Action phase if it is still on.
fn to_buy(g: &mut Game) {
    if phase(g) == Some(Phase::Action) {
        end_phase(g);
    }
}

fn phase(g: &Game) -> Option<Phase> {
    match g.pending().map(|d| &d.kind) {
        Some(DecisionKind::Turn { phase, .. }) => Some(*phase),
        _ => None,
    }
}

/// Resolves pending triggers in the order offered.
fn resolve_triggers(g: &mut Game) {
    while g.pending().is_some_and(|d| d.purpose == Purpose::Trigger) {
        respond(g, Response::option(Opt::Trigger(0)));
    }
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn aside_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).set_aside)
}

fn trash_kinds(g: &Game) -> Vec<Card> {
    kinds(g, g.trash())
}

fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

fn pick(g: &mut Game, card: Card) {
    respond(g, Response::option(Opt::Card(card)));
}

fn buy_event(g: &mut Game, card: Card) {
    respond(g, Response::Move(Move::BuyLandscape(card)));
}

fn gain_count(g: &Game, p: PlayerId, card: Card) -> usize {
    g.log()
        .iter()
        .filter(
            |e| matches!(e, Event::Gained { player, card: c, .. } if *player == p && *c == card),
        )
        .count()
}

fn loot_gained(g: &Game, p: PlayerId) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::Gained { player, card, .. } if *player == p && card.is(Types::LOOT)))
        .count()
}

/// Gives `p` coins and buys this turn (for buying in tests).
fn money(g: &mut Game, coins: u32, buys: u32) {
    g.turn.coins += coins;
    g.turn.buys += buys - 1;
}

/// Puts Trait `t` on `card`'s pile (and no other).
fn set_trait(g: &mut Game, t: Card, card: Card) {
    for pile in g.piles.iter_mut() {
        pile.traits.retain(|&x| x != t);
    }
    if !g.landscapes.contains(&t) {
        g.landscapes.push(t);
    }
    let pile = g.pile_of(card).expect("pile exists");
    g.piles[pile.index()].traits.push(t);
}

/// Replaces the top of the Loot pile with a new `card`.
fn loot_top(g: &mut Game, card: Card) -> CardId {
    let pile = g.pile_of(Card::Loot).expect("a Loot pile");
    g.create(card, Zone::Pile(pile))
}

/// An unstarted game with `landscapes` too.
fn setup_l(kingdom: &[Card], landscapes: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: landscapes.to_vec(),
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    })
}

fn play_games(cards: &[Card], landscapes: Vec<Card>, players: u8, seed: u64, extra: &[Card]) {
    let mut config = GameConfig::new(players, cards.to_vec(), seed);
    config.landscapes = landscapes;
    config.turn_limit = Some(60);
    let mut g = setup_with(config);
    for p in 0..players {
        let p = PlayerId(p);
        for &c in cards.iter().chain(extra) {
            if c.kind() == Kind::Card {
                g.create(c, Zone::Deck(p));
            }
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
}

/// Random games over Plunder with Throne Room and Moat, each player starting with a copy
/// of every Kingdom card and some Loot, with Events and Traits in play.
#[test]
fn random_plunder_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Plunder)
        .filter(|c| c.def().kingdom)
        .collect();
    let loot = Card::Loot.pile_members();
    let landscapes: Vec<Card> = Card::in_set(Set::Plunder)
        .filter(|c| matches!(c.kind(), Kind::Event | Kind::Trait))
        .collect();
    for seed in 0..150 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let mut ls = landscapes.clone();
        ls.shuffle(&mut rng);
        ls.truncate(4);
        let mut extra = loot.clone();
        extra.shuffle(&mut rng);
        extra.truncate(3);
        extra.extend([Card::Gold, Card::Gold, Card::Gold]);
        let players = rng.random_range(2..=4);
        play_games(&cards, ls, players, seed, &extra);
    }
}
