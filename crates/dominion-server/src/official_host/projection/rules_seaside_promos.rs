//! Seaside, Alchemy, Prosperity, Hinterlands and Promos: every admitted card's questions
//! project, and every answer their client could give resolves to one the engine accepts.
use super::*;
use dominion_online::questions::{CardFrequency, LogArgument, QuestionElements};

const SETS: [Set; 5] = [
    Set::Seaside,
    Set::Alchemy,
    Set::Prosperity,
    Set::Hinterlands,
    Set::Promos,
];

fn admitted() -> Vec<Card> {
    let report = crate::official_host::profile::coverage_report(catalog());
    Card::ALL
        .iter()
        .copied()
        .filter(|c| SETS.contains(&c.set()) && (c.def().kingdom || c.kind().is_landscape()))
        .filter(|c| {
            report
                .cards
                .iter()
                .any(|r| r.card == c.name() && r.semantic_mapped && r.gaps.is_empty())
        })
        .collect()
}

fn story(u: &ProjectedUpdate) -> i32 {
    u.question.as_ref().unwrap().wire.description.story.question_id
}
fn symbol(table: &str, name: &str) -> i32 {
    catalog().symbol(table, name).unwrap()
}
fn names(cards: &[(Card, i32)]) -> LogArgument {
    LogArgument::CardNames(
        cards
            .iter()
            .map(|&(card, frequency)| CardFrequency {
                name: catalog().card(card).unwrap(),
                frequency,
            })
            .collect(),
    )
}

#[test]
fn seaside_to_promos_admits_what_it_binds() {
    let admitted = admitted();
    for card in [
        Card::Ambassador,
        Card::Navigator,
        Card::PirateShip,
        Card::ScryingPool,
        Card::Golem,
        Card::CrystalBall,
        Card::KingsCourt,
        Card::Vault,
        Card::Develop,
        Card::Duchess,
        Card::Oracle,
        Card::Trader,
        Card::Tunnel,
        Card::Church,
        Card::Governor,
        Card::Prince,
        Card::Sauna,
        Card::Summon,
    ] {
        assert!(admitted.contains(&card), "{card:?} not admitted");
    }
    // No story for what the engine asks (Watchtower's trash-or-topdeck, Stash's place in
    // the deck), or an ordering their client can't show (from the hand or play).
    for card in [
        Card::Watchtower,
        Card::Stash,
        Card::GhostShip,
        Card::Mandarin,
        Card::Embassy,
        Card::BlackMarket,
    ] {
        assert!(!admitted.contains(&card), "{card:?} admitted");
    }
}

/// Each admitted card played from the fixture hand, every decision it raises answered
/// with each of its bounds.
#[test]
fn every_seaside_to_promos_play_projects_and_every_offered_option_resolves() {
    for card in admitted() {
        if !card.def().kingdom || !card.is(Types::ACTION | Types::TREASURE) {
            continue;
        }
        for variant in 0..4 {
            let mut game = fixture(card, 2);
            play(&mut game, card);
            for step in 0..24 {
                let Some(d) = game.pending() else { break };
                if matches!(d.kind, DecisionKind::Turn { .. }) {
                    break;
                }
                let seat = d.player.0;
                let u = update(&game, seat, step + 2);
                assert_options(&game, &u);
                update(&game, 1 - seat, step + 2);
                let response = default_response(&game, variant);
                game.respond(response).unwrap();
            }
        }
    }
}

const FILLER: &[Card] = &[
    Card::Village,
    Card::Smithy,
    Card::Market,
    Card::Cellar,
    Card::Militia,
    Card::Moat,
    Card::Laboratory,
    Card::Workshop,
    Card::Remodel,
    Card::Chapel,
];

/// Deals copies of `card` in place of each player's first three Coppers.
fn deal(game: Game, card: Card) -> Game {
    let mut state = serde_json::to_value(&game).unwrap();
    let copper = serde_json::to_value(Card::Copper).unwrap();
    let with = serde_json::to_value(card).unwrap();
    for p in 0..game.num_players() {
        let mut changed = 0;
        for zone in ["deck", "hand"] {
            let ids: Vec<usize> = state["players"][p][zone]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect();
            for id in ids {
                if changed < 3 && state["cards"][id] == copper {
                    state["cards"][id] = with.clone();
                    changed += 1;
                }
            }
        }
    }
    serde_json::from_value(state).unwrap()
}

/// Random games with the card in every starting deck: at every decision both seats
/// project, and the decider answers with one of the wire answers, all of which resolve.
#[test]
fn seaside_to_promos_random_games_answer_through_the_wire() {
    let mut seen = std::collections::BTreeSet::new();
    for card in admitted() {
        for seed in 0..2u64 {
            let mut kingdom: Vec<Card> = FILLER.iter().copied().filter(|&c| c != card).collect();
            kingdom.truncate(9);
            let mut cfg;
            if card.kind().is_landscape() {
                cfg = GameConfig::new(2, kingdom, 500 + seed);
                cfg.landscapes.push(card);
            } else {
                kingdom.push(card);
                cfg = GameConfig::new(2, kingdom, 500 + seed);
            }
            cfg.colonies = Some(false);
            cfg.shelters = Some(false);
            let game = Game::new(cfg).unwrap();
            let mut game = if card.kind().is_landscape() {
                game
            } else {
                deal(game, card)
            };
            let mut rng = seed * 7919 + u64::from(card as u16);
            for step in 0..120 {
                let Some(d) = game.pending() else { break };
                // Random play can trash the table bare, then turns pass without a question.
                let view = game.view(d.player);
                if view.players().any(|p| {
                    p.hand_size() + p.deck_size() + p.discard_size() + p.in_play().len() < 5
                }) {
                    break;
                }
                let seat = d.player.0;
                let u = update(&game, seat, step + 1);
                update(&game, 1 - seat, step + 1);
                if d.source == Some(card) || d.purpose == Purpose::Trigger {
                    seen.insert((card, story(&u)));
                }
                let options = assert_options(&game, &u);
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let pick = (rng >> 33) as usize;
                // Lean towards playing the card under test and towards buying.
                let response = match &d.kind {
                    DecisionKind::Turn { moves, .. } => {
                        let plays: Vec<_> = moves
                            .iter()
                            .filter(|m| matches!(m, Move::Play(id) if game.card(*id) == card))
                            .collect();
                        if !plays.is_empty() && pick % 4 != 0 {
                            Response::Move(*plays[pick % plays.len()])
                        } else {
                            options[pick % options.len()].1.clone()
                        }
                    }
                    _ => options[pick % options.len()].1.clone(),
                };
                game.respond(response).unwrap();
            }
        }
    }
    assert!(seen.len() > 60, "{} stories seen", seen.len());
}

fn modes(u: &ProjectedUpdate) -> Vec<i32> {
    match &u.question.as_ref().unwrap().wire.kind {
        QuestionKind::Choice {
            elements: QuestionElements::Modes(m),
            ..
        } => m.clone(),
        other => panic!("not a mode choice: {other:?}"),
    }
}

/// Keep or discard: the engine's Yes and No are Duchess's own modes.
#[test]
fn duchess_yes_and_no_are_its_card_modes() {
    let mut game = fixture(Card::Duchess, 2);
    play(&mut game, Card::Duchess);
    let d = game.pending().unwrap();
    assert_eq!(d.player, PlayerId(0));
    let top = game.card(d.about[0]);
    let u = update(&game, 0, 2);
    assert_eq!(story(&u), symbol("QuestionIds", "DUCHESS"));
    assert_eq!(
        u.question.as_ref().unwrap().wire.description.story.arguments,
        vec![names(&[(top, 1)])]
    );
    assert_eq!(
        modes(&u),
        vec![
            symbol("CardModes", "DUCHESS_DISCARD"),
            symbol("CardModes", "DUCHESS_KEEP")
        ]
    );
    let q = u.question.as_ref().unwrap();
    assert_eq!(q.resolve(q.wire.index, &[0]), Ok(Response::option(Opt::Yes)));
    assert_eq!(q.resolve(q.wire.index, &[1]), Ok(Response::option(Opt::No)));
}

/// "Discard them all" and "put them back" are buttons of one COMPLEX_OR question.
#[test]
fn navigator_choice_is_two_buttons() {
    let mut game = fixture(Card::Navigator, 2);
    play(&mut game, Card::Navigator);
    assert_eq!(game.pending().unwrap().purpose, Purpose::Either);
    let u = update(&game, 0, 2);
    let q = u.question.as_ref().unwrap();
    assert_eq!(story(&u), symbol("QuestionIds", "NAVIGATOR"));
    assert_eq!(
        q.wire.description.question_type,
        symbol("QuestionTypes", "COMPLEX_OR")
    );
    let QuestionKind::Complex(children) = &q.wire.kind else {
        panic!("not complex")
    };
    let buttons: Vec<_> = children
        .iter()
        .map(|c| match &c.kind {
            QuestionKind::Choice { decline, .. } => *decline,
            _ => panic!(),
        })
        .collect();
    assert_eq!(
        buttons,
        vec![
            symbol("DeclineButtonIds", "DISCARD_ALL"),
            symbol("DeclineButtonIds", "CONTINUE")
        ]
    );
    assert_eq!(
        q.resolve(q.wire.index, &[0]),
        Ok(Response::option(Opt::Mode(0)))
    );
    assert_eq!(
        q.resolve(q.wire.index, &[1]),
        Ok(Response::option(Opt::Mode(1)))
    );
    game.respond(Response::option(Opt::Mode(1))).unwrap();
    let u = update(&game, 0, 3);
    assert_eq!(story(&u), symbol("QuestionIds", "NAVIGATOR"));
    assert_eq!(
        u.question.as_ref().unwrap().wire.description.question_type,
        symbol("QuestionTypes", "ORDER_CARDS")
    );
    assert_options(&game, &u);
}

/// Develop's first gain names both costs it may gain, the second the one left.
#[test]
fn develop_gains_name_their_costs() {
    let amount = |coins| LogArgument::Amount {
        coins,
        potions: 0,
        debt: 0,
    };
    let mut game = fixture(Card::Develop, 2);
    play(&mut game, Card::Develop);
    let silver = game
        .hand(PlayerId(0))
        .iter()
        .copied()
        .find(|&id| game.card(id) == Card::Silver)
        .unwrap();
    game.respond(Response::Cards(vec![silver])).unwrap();
    let u = update(&game, 0, 2);
    assert_eq!(story(&u), symbol("QuestionIds", "DEVELOP_GAIN1"));
    assert_eq!(
        u.question.as_ref().unwrap().wire.description.story.arguments,
        vec![amount(4), amount(2)]
    );
    let DecisionKind::Pile { piles, .. } = &game.pending().unwrap().kind else {
        panic!()
    };
    let four = *piles
        .iter()
        .find(|&&p| game.cost(game.pile(p).card).coins == 4)
        .unwrap();
    game.respond(Response::Pile(Some(four))).unwrap();
    let u = update(&game, 0, 3);
    assert_eq!(story(&u), symbol("QuestionIds", "DEVELOP_GAIN2"));
    assert_eq!(
        u.question.as_ref().unwrap().wire.description.story.arguments,
        vec![amount(2)]
    );
}

/// "You may discard a Curse": the Curse, declined with Don't Discard.
#[test]
fn mountebank_offers_the_curse_with_a_decline() {
    let game = fixture(Card::Mountebank, 2);
    let mut state = serde_json::to_value(&game).unwrap();
    take(&mut state, &game, Card::Curse, Zone::Hand(PlayerId(1)));
    let mut game: Game = serde_json::from_value(state).unwrap();
    play(&mut game, Card::Mountebank);
    if game.pending().unwrap().purpose == Purpose::Trigger {
        game.respond(Response::option(Opt::No)).unwrap();
    }
    let d = game.pending().unwrap();
    assert_eq!((d.player, d.source), (PlayerId(1), Some(Card::Mountebank)));
    let u = update(&game, 1, 2);
    assert_eq!(story(&u), symbol("QuestionIds", "MOUNTEBANK"));
    let q = u.question.as_ref().unwrap();
    let QuestionKind::Choice {
        elements: QuestionElements::Cards(cards),
        decline,
        ..
    } = &q.wire.kind
    else {
        panic!()
    };
    assert_eq!(cards.len(), 1);
    assert_eq!(*decline, symbol("DeclineButtonIds", "DONT_DISCARD"));
    assert_eq!(q.resolve(q.wire.index, &[0]), Ok(Response::option(Opt::Yes)));
    assert_eq!(q.resolve(q.wire.index, &[]), Ok(Response::option(Opt::No)));
}

/// Royal Seal's "topdeck Argument0" names the card gained.
#[test]
fn royal_seal_ability_names_the_gained_card() {
    let mut game = fixture(Card::RoyalSeal, 2);
    play(&mut game, Card::RoyalSeal);
    let silver = loop {
        let d = game.pending().unwrap();
        let DecisionKind::Turn { moves, .. } = &d.kind else {
            panic!("{d:?}")
        };
        if let Some(&m) = moves
            .iter()
            .find(|m| matches!(m, Move::Buy(p) if game.pile(*p).card == Card::Silver))
        {
            break m;
        }
        game.respond(Response::Move(Move::EndPhase)).unwrap();
    };
    game.respond(Response::Move(silver)).unwrap();
    let d = game.pending().unwrap();
    assert_eq!(d.purpose, Purpose::Trigger);
    let u = update(&game, 0, 2);
    let QuestionKind::Choice {
        elements: QuestionElements::Abilities(abilities),
        ..
    } = &u.question.as_ref().unwrap().wire.kind
    else {
        panic!()
    };
    assert_eq!(abilities[0].name, symbol("AbilityNames", "TOPDECK_REACTION"));
    assert_eq!(abilities[0].arguments, vec![names(&[(Card::Silver, 1)])]);
    assert_options(&game, &u);
}
