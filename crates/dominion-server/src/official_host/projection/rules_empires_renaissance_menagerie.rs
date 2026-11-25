//! Empires, Renaissance and Menagerie: every admitted card is admitted, its questions
//! project, and every answer offered resolves to a response the engine accepts.
use super::*;
use crate::official_host::profile::{self, KINGDOM};
use crate::official_host::questions::semantics::set_requirements;
use dominion_online::questions::{LogArgument, QuestionElements};

const SETS: [Set; 3] = [Set::Empires, Set::Renaissance, Set::Menagerie];

fn admitted() -> Vec<Card> {
    Card::ALL
        .iter()
        .copied()
        .filter(|c| SETS.contains(&c.set()) && set_requirements(*c).is_some())
        .collect()
}

fn symbol(table: &str, name: &str) -> i32 {
    catalog().symbol(table, name).unwrap()
}

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// Plays a random game with `card` in it, leaning towards playing and buying it, until
/// `stop` holds for the pending decision.
fn drive(card: Card, seed: u64, mut stop: impl FnMut(&Game) -> bool) -> Option<Game> {
    let mut kingdom = vec![
        Card::Village,
        Card::Smithy,
        Card::Market,
        Card::Cellar,
        Card::Militia,
        Card::Workshop,
        Card::Remodel,
        Card::Laboratory,
        Card::Festival,
    ];
    if card.def().kingdom {
        kingdom.push(card);
    } else {
        kingdom.push(Card::Moat);
    }
    let mut cfg = GameConfig::new(2, kingdom, seed);
    cfg.colonies = Some(false);
    cfg.shelters = Some(false);
    if card.kind().is_landscape() {
        cfg.landscapes.push(card);
    }
    let mut game = Game::new(cfg).ok()?;
    let mut rng = Rng(seed * 31 + 7);
    for _ in 0..600 {
        let d = game.pending()?;
        if stop(&game) {
            return Some(game);
        }
        // A table trashed down to nothing plays turns forever without a question.
        if game
            .view(d.player)
            .players()
            .any(|p| p.hand_size() + p.deck_size() + p.discard_size() + p.in_play().len() < 5)
        {
            return None;
        }
        let response = match &d.kind {
            DecisionKind::Turn { moves, .. } => {
                let mine: Vec<_> = moves
                    .iter()
                    .filter(|m| match m {
                        Move::Play(id) => game.card(*id) == card,
                        Move::BuyLandscape(l) => *l == card,
                        Move::Buy(p) => game.pile(*p).card == card,
                        _ => false,
                    })
                    .collect();
                let others: Vec<_> = moves.iter().filter(|m| **m != Move::EndPhase).collect();
                if !mine.is_empty() && rng.below(3) != 0 {
                    Response::Move(*mine[rng.below(mine.len())])
                } else if others.is_empty() || rng.below(4) == 0 {
                    Response::Move(Move::EndPhase)
                } else {
                    Response::Move(*others[rng.below(others.len())])
                }
            }
            _ => default_response(&game, rng.below(6)),
        };
        if game.respond(response).is_err() {
            game.respond(default_response(&game, 0)).ok()?;
        }
    }
    None
}

fn find(card: Card, stop: impl Fn(&Game) -> bool) -> Game {
    (0..40)
        .find_map(|seed| drive(card, seed, &stop))
        .unwrap_or_else(|| panic!("no game reached the {card} decision"))
}

fn question(game: &Game) -> Question {
    let seat = game.pending().unwrap().player.0;
    let u = update(game, seat, 2);
    assert_options(game, &u);
    u.question.unwrap().wire
}

fn story(q: &Question) -> i32 {
    q.description.story.question_id
}

#[test]
fn every_admitted_empires_renaissance_menagerie_card_is_admitted_alone() {
    let meta = hub::Meta::default();
    for card in admitted() {
        let mut cfg = GameConfig::new(2, KINGDOM.to_vec(), 5);
        cfg.colonies = Some(false);
        cfg.shelters = Some(false);
        if card.kind().is_landscape() {
            cfg.landscapes.push(card);
        } else if card.def().kingdom {
            cfg.kingdom[0] = card;
        } else {
            continue;
        }
        profile::check_supported(&cfg, &meta, catalog(), false)
            .unwrap_or_else(|e| panic!("{card}: {e}"));
    }
}

#[test]
fn every_admitted_empires_renaissance_menagerie_action_projects_and_every_option_resolves() {
    for card in admitted() {
        if !card.def().kingdom || !card.is(Types::ACTION) {
            continue;
        }
        for variant in 0..4 {
            let mut game = fixture(card, 2);
            play(&mut game, card);
            for step in 0..24 {
                let Some(d) = game.pending() else { break };
                let u = update(&game, d.player.0, step + 2);
                assert_options(&game, &u);
                if matches!(d.kind, DecisionKind::Turn { .. }) {
                    break;
                }
                game.respond(default_response(&game, variant)).unwrap();
            }
        }
    }
}

/// Random games for every admitted card, Events, Projects, Ways and Landmarks included:
/// every decision projects for every seat and a random wire answer is accepted.
#[test]
fn admitted_empires_renaissance_menagerie_cards_survive_random_games() {
    for card in admitted() {
        if !(card.def().kingdom || card.kind().is_landscape()) {
            continue;
        }
        let (_, errors) = super::super::survey::survey_card(card, 2);
        assert!(errors.is_empty(), "{card}: {errors:?}");
    }
}

#[test]
fn a_way_is_considered_beside_the_card_being_played() {
    let mut game = find(Card::WayOfTheGoat, |g| {
        g.pending().unwrap().purpose == Purpose::Way
    });
    let seat = game.pending().unwrap().player.0;
    let u = update(&game, seat, 2);
    let q = u.question.as_ref().unwrap();
    let wire = &q.wire;
    assert_eq!(
        wire.description.question_type,
        symbol("QuestionTypes", "COMPLEX_AND")
    );
    assert_eq!(story(wire), symbol("QuestionIds", "CONSIDER_WAY"));
    let QuestionKind::Complex(parts) = &wire.kind else {
        panic!("{wire:?}")
    };
    let [play, way] = parts.as_slice() else {
        panic!("{parts:?}")
    };
    assert_eq!(
        play.description.question_type,
        symbol("QuestionTypes", "PLAY")
    );
    assert_eq!(
        way.description.question_type,
        symbol("QuestionTypes", "WAY")
    );
    let QuestionKind::Choice {
        elements: QuestionElements::Cards(card),
        ..
    } = &play.kind
    else {
        panic!("{play:?}")
    };
    let QuestionKind::Choice {
        elements: QuestionElements::Cards(ways),
        affected,
        min: 0,
        max: 1,
        ..
    } = &way.kind
    else {
        panic!("{way:?}")
    };
    assert_eq!(affected, card);
    assert_eq!(ways.len(), 1);
    let LogArgument::CardNames(named) = &wire.description.story.arguments[0] else {
        panic!()
    };
    let played = game.way_card().unwrap();
    assert_eq!(named[0].name, catalog().card(game.card(played)).unwrap());
    assert_eq!(
        q.resolve_at(19, wire.index, &[1, 0, 0]).unwrap(),
        Response::option(Opt::No)
    );
    assert!(q.resolve_at(19, wire.index, &[0, 1, 0]).is_err());
    assert!(q.resolve_at(19, wire.index, &[1, 0, 1, 1]).is_err());
    let goat = q.resolve_at(19, wire.index, &[1, 0, 1, 0]).unwrap();
    assert_eq!(goat, Response::option(Opt::Card(Card::WayOfTheGoat)));
    game.respond(goat).unwrap();
    let d = game.pending().unwrap();
    if d.source == Some(Card::WayOfTheGoat) {
        assert_eq!(
            story(&question(&game)),
            symbol("QuestionIds", "WAY_OF_THE_GOAT")
        );
    }
}

#[test]
fn the_exile_rule_discards_by_the_gained_cards_name() {
    for card in [Card::Sanctuary, Card::BountyHunter, Card::CamelTrain] {
        let game = find(card, |g| {
            let d = g.pending().unwrap();
            d.purpose == Purpose::Trigger
                && g.view(d.player)
                    .pending_triggers()
                    .iter()
                    .any(|t| t.ability == 253)
        });
        let d = game.pending().unwrap();
        let view = game.view(d.player);
        let gained = view
            .pending_triggers()
            .iter()
            .find(|t| t.ability == 253)
            .unwrap()
            .card;
        let q = question(&game);
        let QuestionKind::Choice {
            elements: QuestionElements::Abilities(abilities),
            ..
        } = &q.kind
        else {
            panic!("{q:?}")
        };
        let exile = abilities
            .iter()
            .find(|a| a.name == symbol("AbilityNames", "DISCARD_FROM_EXILE"))
            .unwrap();
        assert!(exile.optional);
        assert_eq!(
            exile.arguments,
            [LogArgument::CardNames(vec![
                dominion_online::questions::CardFrequency {
                    name: catalog().card(gained).unwrap(),
                    frequency: 1,
                }
            ])]
        );
    }
}

#[test]
fn kiln_offers_its_copy_as_an_optional_ability() {
    let game = find(Card::Kiln, |g| {
        let d = g.pending().unwrap();
        d.source == Some(Card::Kiln) && d.purpose == Purpose::Gain
    });
    let q = question(&game);
    assert_eq!(
        q.description.question_type,
        symbol("QuestionTypes", "RESOLVE_ABILITY")
    );
    let QuestionKind::Choice {
        elements: QuestionElements::Abilities(abilities),
        decline,
        min: 0,
        ..
    } = &q.kind
    else {
        panic!("{q:?}")
    };
    assert_eq!(*decline, symbol("DeclineButtonIds", "DONE_REACTING"));
    assert_eq!(abilities[0].name, symbol("AbilityNames", "KILN"));
    assert_eq!(abilities[0].arguments.len(), 1);
}

#[test]
fn capital_repays_an_amount_and_charm_names_what_it_matches() {
    let game = find(Card::Capital, |g| {
        g.pending().unwrap().source == Some(Card::Capital)
    });
    let q = question(&game);
    assert_eq!(
        q.description.question_type,
        symbol("QuestionTypes", "HOW_MUCH_REPAY")
    );
    let QuestionKind::Number { min, max, .. } = q.kind else {
        panic!("{q:?}")
    };
    let seat = game.pending().unwrap().player.0;
    let u = update(&game, seat, 2);
    let p = u.question.unwrap();
    assert_eq!(
        p.resolve_at(19, p.wire.index, &[max]).unwrap(),
        Response::Number(max)
    );
    assert_eq!(
        p.resolve_at(19, p.wire.index, &[min]).unwrap(),
        Response::Number(min)
    );

    let game = find(Card::Charm, |g| {
        let d = g.pending().unwrap();
        d.source == Some(Card::Charm) && d.purpose == Purpose::Gain
    });
    let q = question(&game);
    assert_eq!(story(&q), symbol("QuestionIds", "CHARM_GAIN"));
    assert!(matches!(
        q.description.story.arguments.as_slice(),
        [LogArgument::CardNames(_), LogArgument::Amount { .. }]
    ));
}

#[test]
fn gladiators_neighbour_is_asked_about_the_revealed_card() {
    let game = find(Card::Gladiator, |g| {
        let d = g.pending().unwrap();
        d.source == Some(Card::Gladiator) && d.player != g.turn().player
    });
    let DecisionKind::Cards { candidates, .. } = &game.pending().unwrap().kind else {
        panic!()
    };
    let named = game.card(candidates[0]);
    let q = question(&game);
    assert_eq!(story(&q), symbol("QuestionIds", "GLADIATOR_BLOCK"));
    assert_eq!(
        q.description.story.arguments,
        [LogArgument::CardNames(vec![
            dominion_online::questions::CardFrequency {
                name: catalog().card(named).unwrap(),
                frequency: 1,
            }
        ])]
    );
}
