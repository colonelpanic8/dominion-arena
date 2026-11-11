//! Engine tests and shared helpers. Each set's card tests live in `tests/<set>.rs`.

mod adventures;
mod alchemy;
mod allies;
mod base;
mod cornucopia_guilds;
mod dark_ages;
mod elder;
mod empires;
mod hinterlands;
mod intrigue;
mod knowledge;
mod menagerie;
mod nocturne;
mod plunder;
mod promos;
mod prosperity;
mod renaissance;
mod rising_sun;
mod seaside;
mod setup;

use crate::game::Zone;
use crate::*;
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Sets whose cards are implemented; the fuzz test deals kingdoms from these.
pub(crate) const IMPLEMENTED: &[Set] = &[
    Set::Base,
    Set::Intrigue,
    Set::Seaside,
    Set::Alchemy,
    Set::Prosperity,
    Set::CornucopiaGuilds,
    Set::Hinterlands,
    Set::DarkAges,
    Set::Adventures,
    Set::Allies,
    Set::Empires,
    Set::Nocturne,
    Set::Renaissance,
    Set::Menagerie,
    Set::RisingSun,
    Set::Plunder,
    Set::Promos,
];

pub(crate) const P0: PlayerId = PlayerId(0);
pub(crate) const P1: PlayerId = PlayerId(1);
pub(crate) const P2: PlayerId = PlayerId(2);

/// An unstarted 2-player game: set up hands and decks, then call `g.start()`.
pub(crate) fn setup(kingdom: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    })
}

pub(crate) fn setup_with(config: GameConfig) -> Game {
    crate::setup::build(config).unwrap()
}

/// Adds Kingdom piles mid-game, as Divine Wind does, with what they need.
pub(crate) fn add_kingdom_piles(g: &mut Game, cards: &[Card]) {
    let since = crate::setup::mark(g);
    for &card in cards {
        crate::setup::add_kingdom_pile(g, card);
    }
    crate::setup::complete(g, since);
}

/// Replaces the cards in `zone` with new copies of `cards`; old ones go to the trash.
pub(crate) fn replace(g: &mut Game, zone: Zone, cards: &[Card]) -> Vec<CardId> {
    let old = std::mem::take(g.zone_vec(zone));
    for c in old {
        g.zones[c.index()] = Zone::Trash;
        g.trash.push(c);
    }
    cards.iter().map(|&c| g.create(c, zone)).collect()
}

pub(crate) fn hand(g: &mut Game, p: PlayerId, cards: &[Card]) -> Vec<CardId> {
    replace(g, Zone::Hand(p), cards)
}

/// Sets a deck, listed top first.
pub(crate) fn deck(g: &mut Game, p: PlayerId, cards: &[Card]) -> Vec<CardId> {
    let ids = replace(g, Zone::Deck(p), cards);
    let mut stacked = ids.clone();
    stacked.reverse();
    g.players[p.index()].deck = stacked;
    ids
}

pub(crate) fn discard(g: &mut Game, p: PlayerId, cards: &[Card]) -> Vec<CardId> {
    replace(g, Zone::Discard(p), cards)
}

pub(crate) fn respond(g: &mut Game, r: Response) {
    if let Err(e) = g.respond(r.clone()) {
        panic!("{r:?} rejected: {e} (pending {:?})", g.pending());
    }
}

/// Plays the first `card` in the deciding player's hand.
pub(crate) fn play(g: &mut Game, card: Card) {
    let p = g.pending().expect("a decision is pending").player;
    let id = find(g, p, card);
    respond(g, Response::Move(Move::Play(id)));
}

pub(crate) fn end_phase(g: &mut Game) {
    respond(g, Response::Move(Move::EndPhase));
}

pub(crate) fn buy(g: &mut Game, card: Card) {
    let pile = g.supply_pile(card).expect("pile in the Supply");
    respond(g, Response::Move(Move::Buy(pile)));
}

/// P0 plays an Ambassador, revealing the Gondola that is the rest of their hand and
/// returning none; P1 gains one and plays the only Action in their hand with it.
pub(crate) fn play_on_another_players_turn(g: &mut Game) {
    play(g, Card::Ambassador);
    choose(g, &[]);
    let d = g.pending().expect("a decision is pending");
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    respond(g, Response::option(Opt::Trigger(0)));
}

pub(crate) fn yes(g: &mut Game) {
    respond(g, Response::option(Opt::Yes));
}

pub(crate) fn no(g: &mut Game) {
    respond(g, Response::option(Opt::No));
}

pub(crate) fn choose(g: &mut Game, cards: &[CardId]) {
    respond(g, Response::Cards(cards.to_vec()));
}

/// Chooses candidates of the pending `Cards` decision by card kind.
pub(crate) fn choose_kinds(g: &mut Game, kinds: &[Card]) {
    let mut left = candidates(g);
    let mut chosen = vec![];
    for &k in kinds {
        let i = left
            .iter()
            .position(|&c| g.card(c) == k)
            .unwrap_or_else(|| panic!("no {k} among the candidates"));
        chosen.push(left.remove(i));
    }
    choose(g, &chosen);
}

pub(crate) fn choose_pile(g: &mut Game, card: Card) {
    let pile = g.pile_of(card).expect("pile exists");
    respond(g, Response::Pile(Some(pile)));
}

pub(crate) fn candidates(g: &Game) -> Vec<CardId> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Cards { candidates, .. } => candidates.clone(),
        k => panic!("expected a Cards decision, got {k:?}"),
    }
}

pub(crate) fn purpose(g: &Game) -> Purpose {
    g.pending().expect("a decision is pending").purpose
}

pub(crate) fn find(g: &Game, p: PlayerId, card: Card) -> CardId {
    *g.hand(p)
        .iter()
        .find(|&&c| g.card(c) == card)
        .unwrap_or_else(|| panic!("no {card} in {p}'s hand"))
}

pub(crate) fn kinds(g: &Game, ids: &[CardId]) -> Vec<Card> {
    ids.iter().map(|&c| g.card(c)).collect()
}

pub(crate) fn hand_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.hand(p))
}

/// Deck, top first.
pub(crate) fn deck_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    g.player(p).deck.iter().rev().map(|&c| g.card(c)).collect()
}

/// Checks that every card is in exactly one zone, and that zones agree with `Game::zones`.
pub(crate) fn check_zones(g: &Game) {
    let mut seen = vec![0u8; g.card_count()];
    let mut mark = |ids: &[CardId], zone: Zone| {
        for &c in ids {
            assert_eq!(g.zone(c), zone, "{c:?} ({}) is in {zone:?}", g.card(c));
            seen[c.index()] += 1;
        }
    };
    for p in 0..g.num_players() {
        let id = PlayerId(p as u8);
        let pl = g.player(id);
        mark(&pl.deck, Zone::Deck(id));
        mark(&pl.hand, Zone::Hand(id));
        mark(&pl.discard, Zone::Discard(id));
        mark(&pl.in_play, Zone::InPlay(id));
        mark(&pl.set_aside, Zone::SetAside(id));
        for c in &pl.face_up {
            assert!(pl.set_aside.contains(c), "face-up {c:?} is not set aside");
        }
        for (&mat, cards) in &pl.mats {
            mark(cards, Zone::Mat(id, mat));
        }
    }
    for (i, pile) in g.piles().iter().enumerate() {
        mark(&pile.cards, Zone::Pile(PileId(i as u8)));
    }
    mark(g.trash(), Zone::Trash);
    for (i, &n) in seen.iter().enumerate() {
        assert_eq!(
            n,
            1,
            "card {i} ({}) is in {n} zones",
            g.printed(CardId(i as u32))
        );
    }
}

/// Answers every decision with a uniformly random legal response.
pub(crate) struct Chaos(pub ChaCha8Rng);

impl Agent for Chaos {
    fn decide(&mut self, _view: &View<'_>, d: &Decision) -> Response {
        let rng = &mut self.0;
        match &d.kind {
            DecisionKind::Turn { moves, .. } => {
                let end = Move::EndPhase;
                let pick = *moves.choose(rng).unwrap();
                if pick != end && rng.random_bool(0.85) {
                    Response::Move(pick)
                } else if rng.random_bool(0.5) {
                    Response::Move(end)
                } else {
                    Response::Move(*moves.choose(rng).unwrap())
                }
            }
            DecisionKind::Cards {
                candidates,
                min,
                max,
                ..
            } => {
                let mut chosen = candidates.clone();
                chosen.shuffle(rng);
                chosen.truncate(rng.random_range(*min..=*max));
                Response::Cards(chosen)
            }
            DecisionKind::Pile { piles, optional } => Response::Pile(
                piles
                    .choose(rng)
                    .copied()
                    .filter(|_| !*optional || rng.random_bool(0.8)),
            ),
            DecisionKind::Choose {
                options, min, max, ..
            } => {
                let mut chosen = options.clone();
                chosen.shuffle(rng);
                chosen.truncate(rng.random_range(*min..=*max));
                Response::Options(chosen)
            }
            DecisionKind::NameCard => {
                let cards: Vec<Card> = Card::ALL
                    .iter()
                    .copied()
                    .filter(|c| c.kind() == Kind::Card)
                    .collect();
                Response::Name(*cards.choose(rng).unwrap())
            }
            DecisionKind::Number { min, max } => Response::Number(rng.random_range(*min..=*max)),
        }
    }
}

/// How many seeds a fuzz test runs: `default`, or `$FUZZ_SEEDS` if set.
pub(crate) fn fuzz_seeds(default: u64) -> u64 {
    std::env::var("FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Plays a game of random decisions, checking invariants at every decision.
pub(crate) fn random_game(config: GameConfig, seed: u64) -> Game {
    drive_randomly(Game::new(config).unwrap(), seed)
}

/// Answers every decision of a started game randomly until it ends, checking invariants.
pub(crate) fn drive_randomly(mut g: Game, seed: u64) -> Game {
    let players = g.num_players();
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
    g
}

#[test]
fn random_games_keep_invariants() {
    for seed in 0..fuzz_seeds(200) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let players = rng.random_range(2..=6);
        let mut config = GameConfig::random(players, IMPLEMENTED, seed);
        config.turn_limit = Some(80);
        let g = random_game(config, seed);
        assert!(!g.outcome().unwrap().winners.is_empty());
    }
}

#[test]
fn play_drives_a_game_to_the_end() {
    let mut config = GameConfig::new(3, vec![Card::Witch, Card::Moat, Card::Library], 3);
    config.turn_limit = Some(90);
    let mut g = Game::new(config).unwrap();
    let mut a = Chaos(ChaCha8Rng::seed_from_u64(1));
    let mut b = Chaos(ChaCha8Rng::seed_from_u64(2));
    let mut c = Chaos(ChaCha8Rng::seed_from_u64(3));
    assert_eq!(
        crate::play(&mut g.clone(), &mut [&mut a]).unwrap_err(),
        PlayError::AgentCount {
            players: 3,
            agents: 1
        }
    );
    let outcome = crate::play(&mut g, &mut [&mut a, &mut b, &mut c]).unwrap();
    assert_eq!(outcome.scores.len(), 3);
}

#[test]
fn card_data_is_consistent() {
    for &card in Card::ALL {
        let def = card.def();
        assert_eq!(Card::from_name(def.name), Some(card), "{card:?}");
        if card.kind() == Kind::Card && card.set() != Set::Arcana {
            assert!(def.cost.is_some(), "{card} has no cost");
        }
        if let Some(pile) = def.pile {
            assert!(
                pile.def().kingdom || matches!(pile, Card::Ruins | Card::Loot),
                "{card}'s pile {pile}"
            );
        }
        // The dividing line is a line of its own, and no line is the wrapped rest of a
        // sentence or carries stray whitespace.
        for line in card.text().lines() {
            assert!(line == "---" || !line.contains("---"), "{card}: {line:?}");
            assert_eq!(line, line.trim(), "{card}");
            assert!(!line.starts_with(char::is_lowercase), "{card}: {line:?}");
        }
    }
    assert_eq!(Card::from_name("king's court"), Some(Card::KingsCourt));
    assert_eq!(Card::Peddler.type_names(), ["Action"]);
    assert!(Card::Moat.text().contains("unaffected"));
    assert_eq!(Card::WayOfTheMouse.kind(), Kind::Way);
    assert_eq!(Card::Knights.pile_members().len(), 10);
}

#[test]
fn invalid_responses_are_rejected_without_side_effects() {
    let mut g = setup(&[Card::Chapel]);
    hand(
        &mut g,
        P0,
        &[
            Card::Chapel,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    let extra = g.create(Card::Estate, Zone::Hand(P0));
    g.start();
    play(&mut g, Card::Chapel);
    let before = serde_json::to_string(&g).unwrap();
    let hand = g.hand(P0).to_vec();
    assert_eq!(
        g.respond(Response::Cards(hand.clone())),
        Err(ResponseError::WrongCount {
            got: 5,
            min: 0,
            max: 4
        })
    );
    assert_eq!(
        g.respond(Response::Cards(vec![extra, extra])),
        Err(ResponseError::Duplicate(extra))
    );
    assert_eq!(
        g.respond(Response::Move(Move::EndPhase)),
        Err(ResponseError::WrongKind)
    );
    assert_eq!(serde_json::to_string(&g).unwrap(), before);
}

#[test]
fn games_resume_identically_after_serialization() {
    let mut g = setup(&[Card::Cellar]);
    hand(
        &mut g,
        P0,
        &[Card::Cellar, Card::Estate, Card::Estate, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Cellar);
    let mut restored: Game = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
    let estates: Vec<_> = g
        .hand(P0)
        .iter()
        .copied()
        .filter(|&c| g.card(c) == Card::Estate)
        .collect();
    choose(&mut g, &estates);
    choose(&mut restored, &estates);
    assert_eq!(
        serde_json::to_string(&g).unwrap(),
        serde_json::to_string(&restored).unwrap()
    );
}

#[test]
fn view_hides_hidden_information() {
    let mut g = setup(&[Card::Harbinger]);
    hand(&mut g, P0, &[Card::Harbinger, Card::Copper]);
    deck(&mut g, P0, &[Card::Silver, Card::Gold]);
    let buried = discard(&mut g, P0, &[Card::Duchy, Card::Estate]);
    let theirs = g.hand(P1).to_vec();
    g.start();
    let v = g.view(P0);
    assert_eq!(v.card(theirs[0]), None);
    assert_eq!(v.card(buried[0]), None);
    assert_eq!(v.card(buried[1]), Some(Card::Estate));
    assert_eq!(v.card(g.player(P0).deck[0]), None);
    assert_eq!(g.view(P1).card(theirs[0]), Some(g.card(theirs[0])));

    play(&mut g, Card::Harbinger);
    assert_eq!(purpose(&g), Purpose::TopDeck);
    assert_eq!(g.view(P0).card(buried[0]), Some(Card::Duchy));
    assert_eq!(g.view(P1).card(buried[0]), None);
}

#[test]
fn buying_a_card_pays_and_gains_it() {
    let mut g = setup(&[Card::Smithy]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver]);
    g.start();
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Smithy);
    assert_eq!(g.count_owned(P0, Card::Smithy), 1);
    assert!(g.log().contains(&Event::Bought {
        player: P0,
        card: Card::Smithy
    }));
    // No Buys left, so the turn ended on its own.
    assert_eq!(g.pending().unwrap().player, P1);
}

#[test]
fn coffers_and_debt_can_be_used_in_every_phase() {
    let mut g = setup(&[Card::Smithy]);
    hand(&mut g, P0, &[]);
    g.players[0].coffers = 3;
    g.players[0].debt = 3;
    g.start();
    for phase in [Phase::Action, Phase::Buy, Phase::Night] {
        let d = g.pending().unwrap();
        assert_eq!(
            d.kind,
            DecisionKind::Turn {
                phase,
                moves: vec![Move::Coffers, Move::EndPhase]
            }
        );
        respond(&mut g, Response::Move(Move::Coffers));
        assert_eq!(g.turn().coins, 1);
        respond(&mut g, Response::Move(Move::PayDebt));
        if phase != Phase::Night {
            end_phase(&mut g);
        }
    }
    // Nothing left to do at Night, so the turn ended on its own.
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!((g.player(P0).coffers, g.player(P0).debt), (0, 0));
}

// ---- Shuffling ----

/// Resolves a pending Star Chart trigger, picking `card` to go on top of the shuffle.
fn star_chart(g: &mut Game, card: Card) {
    let d = g.pending().expect("a decision is pending");
    assert_eq!(
        (d.purpose, d.source),
        (Purpose::Trigger, Some(Card::StarChart))
    );
    respond(g, Response::option(Opt::Trigger(0)));
    choose_kinds(g, &[card]);
}

#[test]
fn inn_shuffling_the_deck_opens_the_shuffle_windows() {
    let mut g = setup(&[Card::Inn, Card::Village]);
    g.players[0].projects.push(Card::StarChart);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate, Card::Estate]);
    discard(&mut g, P0, &[Card::Village]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Inn);
    choose_kinds(&mut g, &[Card::Village]);
    star_chart(&mut g, Card::Village);
    assert_eq!(hand_kinds(&g, P0)[0], Card::Village);
}

#[test]
fn a_way_of_the_chameleon_draw_opens_the_shuffle_windows() {
    let mut g = setup(&[Card::Moneylender]);
    g.landscapes.push(Card::WayOfTheChameleon);
    g.players[0].projects.push(Card::StarChart);
    hand(&mut g, P0, &[Card::Moneylender, Card::Copper]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[Card::Estate, Card::Gold, Card::Estate, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Moneylender);
    respond(&mut g, Response::option(Opt::Card(Card::WayOfTheChameleon)));
    choose_kinds(&mut g, &[Card::Copper]);
    star_chart(&mut g, Card::Gold);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Estate, Card::Estate]);
}

#[test]
fn stash_and_star_chart_both_apply_to_one_shuffle() {
    let mut g = setup(&[Card::Smithy, Card::Stash]);
    g.players[0].projects.push(Card::StarChart);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Stash,
            Card::Gold,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Smithy);
    let d = g.pending().unwrap();
    let DecisionKind::Choose { labels, .. } = &d.kind else {
        panic!("expected the trigger order, got {d:?}");
    };
    assert_eq!(labels.len(), 3);
    star_chart(&mut g, Card::Gold);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    // Second from the top, under the Gold.
    respond(&mut g, Response::Number(1));
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Stash, Card::Estate]);
}

/// +Actions and +Buys are for the turn's player only: a card played on another player's
/// turn gives them to no one.
#[test]
fn actions_and_buys_do_nothing_on_another_players_turn() {
    let check = |card: Card, rest: &[Card], answer: fn(&mut Game)| {
        let mut g = setup(&[Card::Ambassador, Card::Gondola, card]);
        hand(&mut g, P0, &[Card::Ambassador, Card::Gondola]);
        hand(&mut g, P1, &[&[card], rest].concat());
        deck(&mut g, P1, &[Card::Copper; 5]);
        g.start();
        play_on_another_players_turn(&mut g);
        answer(&mut g);
        assert_eq!(kinds(&g, g.in_play(P1)), [card]);
        let turn = g.turn();
        assert_eq!((turn.player, turn.actions, turn.buys), (P0, 0, 1), "{card}");
    };
    check(Card::Pawn, &[], |g| intrigue::modes(g, &[1, 2]));
    check(Card::Squire, &[], |g| {
        respond(g, Response::option(Opt::Mode(1)))
    });
    check(Card::Hamlet, &[Card::Estate, Card::Curse], |g| {
        choose_kinds(g, &[Card::Estate]);
        choose_kinds(g, &[Card::Curse]);
    });
    check(Card::Stables, &[Card::Copper], |g| {
        choose_kinds(g, &[Card::Copper])
    });
}

#[test]
fn the_game_ends_when_provinces_run_out() {
    let mut g = setup(&[Card::Smithy]);
    let provinces = g.supply_pile(Card::Province).unwrap();
    let all = g.pile(provinces).cards.clone();
    for &c in &all[1..] {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    for _ in 0..3 {
        play(&mut g, Card::Gold);
    }
    buy(&mut g, Card::Province);
    let outcome = g.outcome().expect("game over");
    assert_eq!(outcome.reason, EndReason::ProvincesGone);
    assert_eq!(outcome.winners, [P0]);
}

#[test]
fn during_your_turns_effects_are_over_when_the_game_is_scored() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::Keep],
        turn_limit: Some(1),
        ..GameConfig::new(2, vec![Card::Militia], 7)
    });
    g.players[0].projects.push(Card::Capitalism);
    let militia = hand(&mut g, P0, &[Card::Militia])[0];
    g.start();
    // Keep: 5 VP for having the most copies of a Treasure, which Militia is for now.
    assert!(g.is(militia, Types::TREASURE));
    let during = g.score(P0);
    while g.outcome().is_none() {
        end_phase(&mut g);
    }
    let outcome = g.outcome().unwrap();
    assert!(!g.is(militia, Types::TREASURE));
    assert_eq!(outcome.scores[0], during - 5);
}

/// A random game in which every deck also starts with one of each Kingdom card (the top
/// card of each Kingdom pile), two Golds and, if the game has them, two Potions, so the
/// Kingdom cards actually get played.
pub(crate) fn random_game_with_kingdom_decks(config: GameConfig, seed: u64) -> Game {
    let mut g = setup_with(config);
    let mut extra: Vec<Card> = g
        .config()
        .kingdom
        .iter()
        .map(|&k| k.pile_members()[0])
        .collect();
    extra.extend([Card::Gold; 2]);
    if g.supply_pile(Card::Potion).is_some() {
        extra.extend([Card::Potion; 2]);
    }
    for p in 0..g.num_players() {
        for &card in &extra {
            g.create(card, Zone::Deck(PlayerId(p as u8)));
        }
        let mut deck = std::mem::take(&mut g.players[p].deck);
        g.shuffle(&mut deck);
        g.players[p].deck = deck;
    }
    g.start();
    drive_randomly(g, seed)
}

/// Kingdom cards of the implemented sets that later editions removed, which
/// `GameConfig::random` never deals.
fn removed_kingdom_cards() -> Vec<Card> {
    Card::ALL
        .iter()
        .copied()
        .filter(|c| c.def().kingdom && c.def().removed && IMPLEMENTED.contains(&c.set()))
        .collect()
}

/// Kingdoms (and landscapes) dealt from every implemented set, with every player's deck
/// seeded with the Kingdom cards so cross-set interactions actually happen. Up to three
/// Kingdom piles are swapped for removed cards.
#[test]
fn cross_set_games_keep_invariants() {
    let removed = removed_kingdom_cards();
    for seed in 0..fuzz_seeds(100) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xc0ffee);
        let players = rng.random_range(2..=6);
        let mut config = GameConfig::random(players, IMPLEMENTED, seed);
        for i in 0..rng.random_range(0..=3) {
            let card = *removed.choose(&mut rng).unwrap();
            if !config.kingdom.contains(&card) {
                config.kingdom[i] = card;
            }
        }
        config.turn_limit = Some(60);
        let g = random_game_with_kingdom_decks(config, seed);
        if std::env::var("FUZZ_STATS").is_ok() {
            let plays = g
                .log()
                .iter()
                .filter(|e| matches!(e, Event::Played { .. }))
                .count();
            let turns: u32 = g.outcome().unwrap().turns.iter().sum();
            eprintln!(
                "seed {seed}: {turns} turns, {plays} plays, {:?}",
                g.outcome().unwrap().reason
            );
        }
    }
}
