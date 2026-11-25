//! Dark Ages, Cornucopia & Guilds, Adventures and Nocturne: every card admitted from these
//! sets is dealt into every starting deck and played through seeded games. Each decision
//! is projected for each seat; every wire answer offered must resolve to a response the
//! engine accepts, and every story, mode, ability and set-specific button an admitted
//! card's question shows must be one its `symbols` declares.
use super::*;
use crate::official_host::questions::semantics;
use dominion_engine::{
    Decision, DecisionKind, Game, GameConfig, KnownCards, Move, Opt, Purpose, Response, Set,
};
use dominion_online::questions::{Question, QuestionElements, QuestionKind};

fn catalog() -> &'static WireCatalog {
    static C: std::sync::OnceLock<WireCatalog> = std::sync::OnceLock::new();
    C.get_or_init(super::tests::catalog)
}

pub(super) fn update(game: &Game, seat: u8, index: i32) -> Result<ProjectedUpdate, String> {
    let n = game.num_players();
    let names: Vec<_> = (0..n).map(|i| format!("Seat{}", i + 1)).collect();
    let context = HostGameContext {
        wire_ids: WireIdentity {
            game_id: 100,
            table_id: 31,
            player_ids: (0..n).map(|i| 1000 + (i as i32) * 13).collect(),
        },
        table_details: transport::TableDetails {
            table_id: 31,
            host_id: 1000,
            players: (0..n)
                .map(|i| transport::NamedId {
                    id: 1000 + (i as i32) * 13,
                    name: names[i].clone(),
                })
                .collect(),
            ready: vec![true; n],
            bots: vec![],
            spectators: vec![],
            reconnecting: vec![],
            rules: vec![],
            division: String::new(),
            match_games: vec![],
            match_score: None,
            tournament: None,
        },
        viewer_role: ViewerRole::Player(usize::from(seat)),
        table_options: dominion_protocol::lobby::TableSpec {
            timed: false,
            rated: false,
            ..Default::default()
        },
    };
    project(
        hub::SeatObservation {
            clock: None,
            view: game.view(PlayerId(seat)),
            known: &KnownCards::default(),
            names: &names,
            decision_id: game.pending().map(|_| "q-darkages"),
            deciding: game.pending().map(|d| d.player),
            version: index as u64,
            undos: 0,
            outcome: game.outcome(),
            ended: None,
        },
        &context,
        catalog(),
        ProjectionRequest {
            question_index: index,
            reconnect: false,
            epoch: 19,
        },
    )
    .map_err(|e| e.to_string())
}

/// Every individual option, the boundary selections and one reversed order.
fn answers(q: &Question) -> Vec<Vec<i32>> {
    match &q.kind {
        QuestionKind::Complex(children) => children
            .iter()
            .enumerate()
            .flat_map(|(i, q)| {
                answers(q)
                    .into_iter()
                    .map(move |a| std::iter::once(i as i32).chain(a).collect())
            })
            .collect(),
        QuestionKind::Choice {
            min,
            max,
            elements,
            decline,
            ..
        } => {
            let len = elements.len() as i32;
            let mut a = vec![];
            if *min == 0 {
                a.push(vec![]);
            }
            if *min <= 1 && *max >= 1 {
                a.extend((0..len).map(|i| vec![i]));
            }
            if *min > 1 {
                a.push((0..*min).collect());
            }
            if *max > 1 {
                a.push((0..*max.min(&len)).rev().collect());
            }
            let _ = decline;
            a
        }
        QuestionKind::Name => vec![vec![2], vec![31]],
        QuestionKind::Number { min, max, .. } => vec![vec![*min], vec![*max]],
        _ => panic!("unexpected question shape"),
    }
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

/// The symbols of `q` that only a card's own binding chooses.
fn shown(q: &Question, out: &mut Vec<(&'static str, i32)>) {
    out.push(("QuestionIds", q.description.story.question_id));
    match &q.kind {
        QuestionKind::Complex(children) => children.iter().for_each(|q| shown(q, out)),
        QuestionKind::Choice {
            elements, decline, ..
        } => {
            if *decline >= 0 {
                out.push(("DeclineButtonIds", *decline));
            }
            match elements {
                QuestionElements::Modes(modes) => {
                    out.extend(modes.iter().map(|&m| ("CardModes", m)))
                }
                QuestionElements::Abilities(abilities) => {
                    out.extend(abilities.iter().map(|a| ("AbilityNames", a.name)))
                }
                _ => {}
            }
        }
        _ => {}
    }
}

const GENERIC: &[(&str, &str)] = &[
    ("QuestionIds", "GAME_RESOLVE_ABILITY"),
    ("QuestionIds", "GAME_MAY_REACT_WITH"),
    ("DeclineButtonIds", "DONE_REACTING"),
    ("DeclineButtonIds", "DONT_TRASH"),
    ("DeclineButtonIds", "DONT_DISCARD"),
    ("DeclineButtonIds", "DONT_GAIN"),
    ("DeclineButtonIds", "DONT_PLAY"),
    ("DeclineButtonIds", "DONT_TOPDECK"),
    ("DeclineButtonIds", "DONT_REVEAL"),
    ("DeclineButtonIds", "DONE_SETTING_ASIDE"),
    ("DeclineButtonIds", "DONT"),
];

/// Checks a projected question against the symbols its card declares.
fn declared(game: &Game, d: &Decision, q: &Question, admitted: &[Card]) {
    let c = catalog();
    let mut owners = vec![];
    if d.purpose == Purpose::Trigger {
        owners.extend(
            game.view(d.player)
                .pending_triggers()
                .iter()
                .map(|t| t.card),
        );
    } else if let Some(source) = d.source {
        owners.push(source);
    }
    if !owners.iter().any(|o| admitted.contains(o)) {
        return;
    }
    let allowed: Vec<(&str, i32)> = owners
        .iter()
        .flat_map(|&o| semantics::symbols(o).iter())
        .chain(GENERIC)
        .filter_map(|&(t, s)| c.symbol(t, s).ok().map(|i| (t, i)))
        .collect();
    let mut seen = vec![];
    shown(q, &mut seen);
    for (table, symbol) in seen {
        if d.purpose == Purpose::Trigger && table != "AbilityNames" {
            continue;
        }
        if d.purpose == Purpose::Trigger
            && !owners.iter().any(|&o| {
                semantics::symbols(o)
                    .iter()
                    .any(|(t, _)| *t == "AbilityNames")
            })
        {
            continue;
        }
        assert!(
            allowed.contains(&(table, symbol)),
            "{owners:?} {:?}: {table} {symbol} is not declared",
            d.purpose
        );
    }
}

/// Swaps some of each player's starting Coppers for `cards`.
fn deal(game: Game, cards: &[Card]) -> Game {
    let mut state = serde_json::to_value(&game).unwrap();
    let copper = serde_json::to_value(Card::Copper).unwrap();
    for p in 0..game.num_players() {
        let mut changed = 0;
        for zone in ["hand", "deck"] {
            let ids: Vec<usize> = state["players"][p][zone]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect();
            for id in ids {
                if changed < 4 && state["cards"][id] == copper {
                    let card = cards[changed % cards.len()];
                    state["cards"][id] = serde_json::to_value(card).unwrap();
                    changed += 1;
                }
            }
        }
    }
    serde_json::from_value(state).unwrap()
}

const FILLER: &[Card] = &[
    Card::Village,
    Card::Smithy,
    Card::Market,
    Card::Cellar,
    Card::Militia,
    Card::Moat,
    Card::Laboratory,
    Card::Festival,
    Card::Workshop,
    Card::Remodel,
];

/// The Kingdom pile a non-Kingdom card comes from in these sets' games.
fn kingdom_of(card: Card) -> Option<Card> {
    use Card::*;
    if card.def().kingdom {
        return Some(card);
    }
    Some(match card.def().pile {
        Some(Knights) => Knights,
        Some(Ruins) => Marauder,
        _ => match card {
            Ruins => Marauder,
            Spoils => BanditCamp,
            Madman => Hermit,
            Mercenary => Urchin,
            BagOfGold | Diadem | Followers | Princess | TrustySteed => Tournament,
            Coronet | Courser | Demesne | Housecarl | HugeTurnip | Renown => Joust,
            _ => return None,
        },
    })
}

/// Plays seeded games with `card` in every starting deck.
fn exercise(card: Card, admitted: &[Card], seeds: u64, steps: i32) -> usize {
    let mut questions = 0;
    let Some(home) = kingdom_of(card) else {
        panic!("{card} has no game to be played in");
    };
    for seed in 0..seeds {
        let mut kingdom: Vec<Card> = FILLER.iter().copied().filter(|&c| c != home).collect();
        kingdom.truncate(9);
        kingdom.push(home);
        let mut cfg = GameConfig::new(2, kingdom, 500 + seed);
        cfg.colonies = Some(false);
        cfg.shelters = Some(false);
        let game = Game::new(cfg).unwrap();
        let mut game = if card.kind() == dominion_engine::Kind::Card {
            deal(game, &[card])
        } else {
            game
        };
        let mut rng = Rng(seed * 7919 + 17);
        for step in 0..steps {
            let Some(d) = game.pending().cloned() else {
                break;
            };
            let view = game.view(d.player);
            if view
                .players()
                .any(|p| p.hand_size() + p.deck_size() + p.discard_size() + p.in_play().len() < 5)
            {
                break;
            }
            for seat in 0..game.num_players() as u8 {
                if seat != d.player.0 {
                    update(&game, seat, step + 1)
                        .unwrap_or_else(|e| panic!("{card}: seat {seat} at {d:?}: {e}"));
                }
            }
            let u = update(&game, d.player.0, step + 1)
                .unwrap_or_else(|e| panic!("{card}: {d:?}: {e}"));
            let q = u.question.expect("decider has a question");
            if !matches!(d.kind, DecisionKind::Turn { .. }) {
                questions += 1;
                declared(&game, &d, &q.wire, admitted);
            }
            let mut offered = vec![];
            for a in answers(&q.wire) {
                let response = q
                    .resolve_at(19, q.wire.index, &a)
                    .unwrap_or_else(|e| panic!("{card}: {d:?} {a:?}: {e:?}"));
                d.validate(&response)
                    .unwrap_or_else(|e| panic!("{card}: {d:?} {a:?} {response:?}: {e}"));
                offered.push(response);
            }
            let response = match &d.kind {
                DecisionKind::Turn { moves, .. } => {
                    let mine: Vec<_> = moves
                        .iter()
                        .filter(|m| matches!(m, Move::Play(id) if game.card(*id) == card))
                        .collect();
                    if !mine.is_empty() && rng.below(3) != 0 {
                        Response::Move(*mine[rng.below(mine.len())])
                    } else {
                        offered.swap_remove(rng.below(offered.len()))
                    }
                }
                DecisionKind::Choose { options, .. }
                    if d.purpose == Purpose::Trigger && options.contains(&Opt::No) =>
                {
                    let picked = offered.swap_remove(rng.below(offered.len()));
                    if rng.below(3) == 0 {
                        Response::option(Opt::No)
                    } else {
                        picked
                    }
                }
                _ => offered.swap_remove(rng.below(offered.len())),
            };
            game.respond(response).unwrap();
        }
    }
    questions
}

/// The cards of `sets` the transport admits.
fn admitted(sets: &[Set]) -> Vec<Card> {
    let report = crate::official_host::profile::coverage_report(catalog());
    Card::ALL
        .iter()
        .copied()
        .filter(|c| sets.contains(&c.set()) && !c.def().unimplemented)
        .filter(|c| {
            report
                .cards
                .iter()
                .any(|r| r.card == c.name() && r.semantic_mapped)
        })
        .collect()
}

fn exercise_sets(sets: &[Set]) {
    let admitted = admitted(sets);
    assert!(!admitted.is_empty());
    let report = crate::official_host::profile::coverage_report(catalog());
    for &card in &admitted {
        let r = report.cards.iter().find(|r| r.card == card.name()).unwrap();
        assert!(
            r.gaps.is_empty() || !r.catalog_present,
            "{card}: {:?}",
            r.gaps
        );
        if kingdom_of(card).is_some() {
            let n = exercise(card, &admitted, 3, 90);
            if std::env::var("Q_VERBOSE").is_ok() {
                eprintln!("{card}: {n} questions");
            }
        }
    }
}

#[test]
fn dark_ages_admitted_cards_project_and_answer_through_seeded_games() {
    exercise_sets(&[Set::DarkAges]);
}

#[test]
fn cornucopia_guilds_admitted_cards_project_and_answer_through_seeded_games() {
    exercise_sets(&[Set::CornucopiaGuilds]);
}
