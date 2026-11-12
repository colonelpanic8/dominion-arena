use dominion_agent::{
    AgentDecision, AgentOption, Answer, AnswerError, JSON_SCHEMA, Table, glossary_for, text,
};
use dominion_engine::{Card, GameConfig, PlayerId, Set};
use jsonschema::Validator;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Value, json};

fn validator(def: &str) -> Validator {
    let mut schema: Value = serde_json::from_str(JSON_SCHEMA).unwrap();
    schema["$ref"] = json!(format!("#/$defs/{def}"));
    jsonschema::validator_for(&schema).unwrap()
}

fn assert_valid(v: &Validator, instance: &Value, what: &str) {
    let errors: Vec<String> = v
        .iter_errors(instance)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{what} is invalid: {errors:#?}");
}

fn seeds(default: u64) -> u64 {
    std::env::var("FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn released() -> Vec<Set> {
    Set::ALL
        .iter()
        .copied()
        .filter(|&s| s != Set::Arcana)
        .collect()
}

/// A random legal answer: a random number of options in the decision's bounds.
fn random_answer(d: &AgentDecision, rng: &mut ChaCha8Rng) -> Answer {
    let (min, max) = (d.min.unwrap_or(1) as usize, d.max.unwrap_or(1) as usize);
    let mut indices: Vec<u32> = d.options.iter().map(|o| o.index).collect();
    indices.shuffle(rng);
    indices.truncate(rng.random_range(min..=max.min(indices.len())));
    Answer::indices(&d.id, indices)
}

#[test]
fn random_games_produce_valid_views_and_answerable_options() {
    let view_schema = validator("view");
    let decision_schema = validator("decision");
    let answer_schema = validator("answer");
    let glossary_schema = validator("glossaryEntry");
    for seed in 0..seeds(12) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::random(players, &released(), seed);
        config.turn_limit = Some(30);
        let mut table = Table::new(format!("g{seed}"), config, vec![]).unwrap();
        let mut step = 0;
        while let Some(seat) = table.deciding() {
            let view = table.view(seat);
            let json = serde_json::to_value(&view).unwrap();
            assert_valid(
                &view_schema,
                &json,
                &format!("seed {seed} step {step} view"),
            );
            let d = view
                .decision
                .clone()
                .expect("the deciding seat has a decision");
            assert_valid(&decision_schema, &json["decision"], "decision");
            if step % 25 == 0 {
                for p in 0..players {
                    let json = serde_json::to_value(table.view(PlayerId(p))).unwrap();
                    assert_valid(&view_schema, &json, "a waiting seat's view");
                }
                for entry in glossary_for(&view) {
                    let json = serde_json::to_value(entry).unwrap();
                    assert_valid(&glossary_schema, &json, "glossary entry");
                }
            }
            let (min, max) = (d.min.unwrap(), d.max.unwrap());
            if (min..=max).contains(&1) {
                for o in &d.options {
                    let answer = Answer::indices(&d.id, vec![o.index]);
                    if let Err(e) = table.resolve(seat, &answer) {
                        panic!("seed {seed}: option {o:?} of {d:?} is rejected: {e}");
                    }
                }
            }
            if d.name_any {
                assert!(table.resolve(seat, &Answer::name(&d.id, "gold")).is_ok());
            }
            let answer = random_answer(&d, &mut rng);
            assert_valid(
                &answer_schema,
                &serde_json::to_value(&answer).unwrap(),
                "answer",
            );
            if let Err(e) = table.answer(seat, &answer) {
                panic!("seed {seed}: random answer {answer:?} to {d:?} rejected: {e}");
            }
            step += 1;
            assert!(step < 50_000, "seed {seed}: the game does not end");
        }
        let json = serde_json::to_value(table.view(PlayerId(0))).unwrap();
        assert_valid(&view_schema, &json, "the final view");
        assert!(json["game"]["result"].is_object());
    }
}

#[test]
fn a_restored_table_plays_on_exactly_like_the_original() {
    for seed in 0..seeds(6) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::random(players, &released(), seed);
        config.turn_limit = Some(20);
        let mut table = Table::new(format!("g{seed}"), config, vec![]).unwrap();
        let mut copy: Option<Table> = None;
        let mut step = 0;
        while let Some(seat) = table.deciding() {
            if step % 40 == 7 {
                let saved = serde_json::to_string(&table).unwrap();
                copy = Some(serde_json::from_str(&saved).unwrap());
            }
            if let Some(copy) = &copy {
                for p in 0..players {
                    assert_eq!(
                        serde_json::to_value(copy.view(PlayerId(p))).unwrap(),
                        serde_json::to_value(table.view(PlayerId(p))).unwrap(),
                        "seed {seed} step {step} seat {p}"
                    );
                }
            }
            let answer = random_answer(&table.view(seat).decision.unwrap(), &mut rng);
            table.answer(seat, &answer).unwrap();
            if let Some(copy) = &mut copy {
                copy.answer(seat, &answer).unwrap();
            }
            step += 1;
        }
        assert!(copy.is_none_or(|c| c.is_over()));
    }
}

fn base_table() -> Table {
    let kingdom = vec![
        Card::Cellar,
        Card::Market,
        Card::Militia,
        Card::Mine,
        Card::Moat,
        Card::Remodel,
        Card::Smithy,
        Card::Village,
        Card::Workshop,
        Card::Witch,
    ];
    let config = GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        turn_limit: Some(24),
        ..GameConfig::new(2, kingdom, 11)
    };
    Table::new("snap", config, vec!["Ada".into(), "Bo".into()]).unwrap()
}

/// Plays Actions, then Treasures, and buys Militia once, then Silver; a fixed script for
/// reaching positions, not a strategy.
fn scripted(d: &AgentDecision) -> Answer {
    let pick = |prefix: &str| d.options.iter().find(|o| o.label.starts_with(prefix));
    let choice = pick("Play Militia")
        .or_else(|| pick("Play Smithy"))
        .or_else(|| pick("Play Gold"))
        .or_else(|| pick("Play Silver"))
        .or_else(|| pick("Play Copper"))
        .or_else(|| pick("Buy Militia"))
        .or_else(|| pick("Buy Silver"))
        .or_else(|| pick("End your"));
    let indices = match choice {
        Some(o) => vec![o.index],
        None => {
            let min = d.min.unwrap_or(1) as usize;
            d.options.iter().take(min.max(1)).map(|o| o.index).collect()
        }
    };
    Answer::indices(&d.id, indices)
}

fn check_snapshot(name: &str, text: &str) {
    let path = format!("{}/tests/snapshots/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(&path, format!("{text}\n")).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        text.trim_end(),
        expected.trim_end(),
        "{name} changed; rerun with UPDATE_SNAPSHOTS=1 to accept"
    );
}

#[test]
fn text_rendering_snapshots() {
    let mut table = base_table();
    check_snapshot("start-deciding", &table.text(PlayerId(0)));
    check_snapshot("start-waiting", &table.text(PlayerId(1)));
    let mut militia = None;
    while let Some(seat) = table.deciding() {
        let d = table.view(seat).decision.unwrap();
        if militia.is_none() && d.source.as_deref() == Some("Militia") {
            militia = Some(table.text(seat));
        }
        table.answer(seat, &scripted(&d)).unwrap();
    }
    check_snapshot("militia-attack", &militia.expect("Militia is played"));
    check_snapshot("game-over", &table.text(PlayerId(1)));
}

#[test]
fn bad_answers_are_rejected_with_reasons() {
    let mut table = base_table();
    let d = table.view(PlayerId(0)).decision.unwrap();
    let n = d.options.len() as u32;
    let err = |table: &mut Table, seat: u8, answer: Answer| {
        table.answer(PlayerId(seat), &answer).unwrap_err()
    };
    assert!(matches!(
        err(&mut table, 1, Answer::indices(&d.id, vec![0])),
        AnswerError::NotYourDecision { .. }
    ));
    assert!(matches!(
        err(&mut table, 0, Answer::indices("d999", vec![0])),
        AnswerError::StaleDecision { .. }
    ));
    let AnswerError::Invalid(msg) = err(&mut table, 0, Answer::indices(&d.id, vec![n])) else {
        panic!("out-of-range index accepted");
    };
    assert_eq!(
        msg,
        format!("there is no option {n}: the options are numbered 0 to {}", n - 1)
    );
    let AnswerError::Invalid(msg) = err(&mut table, 0, Answer::indices(&d.id, vec![0, 0])) else {
        panic!("repeated index accepted");
    };
    assert_eq!(msg, "option 0 is chosen more than once");
    let AnswerError::Invalid(msg) = err(&mut table, 0, Answer::indices(&d.id, vec![0, 1])) else {
        panic!("two moves accepted");
    };
    assert_eq!(msg, "choose a move; you chose 2");
    let AnswerError::Invalid(msg) = err(&mut table, 0, Answer::indices(&d.id, vec![])) else {
        panic!("no move accepted");
    };
    assert_eq!(msg, "choose a move; you chose none");
    let AnswerError::Invalid(msg) = err(&mut table, 0, Answer::name(&d.id, "Gold")) else {
        panic!("name accepted for a turn decision");
    };
    assert!(msg.contains("doesn't take a name"), "{msg}");
    table
        .answer(PlayerId(0), &Answer::indices(&d.id, vec![0]))
        .unwrap();
    assert!(matches!(
        err(&mut table, 0, Answer::indices(&d.id, vec![0])),
        AnswerError::StaleDecision { .. }
    ));
}

/// A 2-player game of `kingdom` filled up with Base cards.
fn table(kingdom: &[Card], seed: u64, names: &[&str]) -> Table {
    let mut cards = kingdom.to_vec();
    for filler in [
        Card::Cellar,
        Card::Market,
        Card::Mine,
        Card::Moat,
        Card::Remodel,
        Card::Smithy,
        Card::Village,
        Card::Workshop,
        Card::Festival,
        Card::Laboratory,
    ] {
        if cards.len() < 10 {
            cards.push(filler);
        }
    }
    let config = GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(2, cards, seed)
    };
    Table::new("t", config, names.iter().map(|s| s.to_string()).collect()).unwrap()
}

fn pick(d: &AgentDecision, wanted: impl Fn(&AgentOption) -> bool) -> Option<Answer> {
    d.options
        .iter()
        .find(|o| wanted(o))
        .map(|o| Answer::indices(&d.id, vec![o.index]))
}

fn first(d: &AgentDecision) -> Answer {
    let min = d.min.unwrap_or(1) as usize;
    Answer::indices(&d.id, d.options.iter().take(min).map(|o| o.index).collect())
}

/// Seat 0 plays whatever it can and buys `card` once (Silver otherwise); the other seat
/// does nothing. Stops at the first decision `stop` accepts, returning its seat.
fn play_to(
    table: &mut Table,
    card: &str,
    stop: impl Fn(PlayerId, &AgentDecision) -> bool,
) -> Option<PlayerId> {
    let mut bought = false;
    for _ in 0..600 {
        let seat = table.deciding()?;
        let d = table.view(seat).decision.unwrap();
        if stop(seat, &d) {
            return Some(seat);
        }
        let end = pick(&d, |o| o.action == "end-phase");
        let answer = if d.kind != "turn" {
            first(&d)
        } else if seat.0 != 0 {
            end.unwrap()
        } else if let Some(play) = pick(&d, |o| o.action == "play") {
            play
        } else if let Some(buy) = pick(&d, |o| {
            !bought && o.label.starts_with(&format!("Buy {card}"))
        }) {
            bought = true;
            buy
        } else if let Some(silver) = pick(&d, |o| !bought && o.label.starts_with("Buy Silver")) {
            silver
        } else {
            end.unwrap()
        };
        table.answer(seat, &answer).unwrap();
    }
    None
}

#[test]
fn a_question_about_looked_at_cards_names_them() {
    let mut t = table(&[Card::Navigator], 1, &[]);
    let asked = |_: PlayerId, d: &AgentDecision| d.source.as_deref() == Some("Navigator");
    assert_eq!(play_to(&mut t, "Navigator", asked), Some(PlayerId(0)));
    let g = t.game();
    let top: Vec<&str> = g
        .player(PlayerId(0))
        .deck
        .iter()
        .rev()
        .take(5)
        .map(|&c| g.card(c).name())
        .collect();
    let d = t.view(PlayerId(0)).decision.unwrap();
    assert!(
        d.prompt
            .starts_with(&format!("Navigator (looking at {}): ", top.join(", "))),
        "{}",
        d.prompt
    );
}

#[test]
fn everyone_sees_the_card_named_for_contraband() {
    let mut t = table(&[Card::Contraband], 0, &[]);
    let naming = |_: PlayerId, d: &AgentDecision| d.name_any;
    assert_eq!(play_to(&mut t, "Contraband", naming), Some(PlayerId(1)));
    let d = t.view(PlayerId(1)).decision.unwrap();
    assert_eq!(d.source.as_deref(), Some("Contraband"));
    t.answer(PlayerId(1), &Answer::name(&d.id, "Duchy"))
        .unwrap();
    for seat in [PlayerId(0), PlayerId(1)] {
        let view = t.view(seat);
        assert!(
            view.notes
                .iter()
                .any(|n| n.source == "Contraband" && n.card.as_deref() == Some("Duchy")),
            "{seat}: {:?}",
            view.notes
        );
        assert!(
            view.since_last_decision
                .iter()
                .any(|e| e.kind == "named" && e.card.as_deref() == Some("Duchy")),
            "{seat}: {:?}",
            view.since_last_decision
        );
    }
}

#[test]
fn the_kingdom_is_the_kingdom_piles_in_the_game() {
    let t = table(&[Card::YoungWitch], 0, &[]);
    let view = t.view(PlayerId(0));
    let bane = view
        .notes
        .iter()
        .find(|n| n.source == "Young Witch")
        .and_then(|n| n.pile.clone())
        .expect("the Bane pile");
    let flagged: Vec<&str> = view
        .supply
        .iter()
        .filter(|p| p.kingdom)
        .map(|p| p.card.as_str())
        .collect();
    assert_eq!(flagged.len(), 11);
    assert!(flagged.contains(&bane.as_str()));
    assert_eq!(view.game.kingdom, flagged);
}

#[test]
fn divine_wind_replaces_the_kingdom() {
    let config = GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::DivineWind],
        ..GameConfig::new(
            2,
            vec![
                Card::Poet,
                Card::Cellar,
                Card::Market,
                Card::Mine,
                Card::Moat,
                Card::Remodel,
                Card::Smithy,
                Card::Village,
                Card::Workshop,
                Card::Festival,
            ],
            3,
        )
    };
    let mut t = Table::new("t", config, vec![]).unwrap();
    let suns = |t: &Table| {
        let view = t.view(PlayerId(1));
        let wind = view.landscapes.iter().find(|l| l.card == "Divine Wind");
        wind.and_then(|l| l.suns).expect("Divine Wind's Sun tokens")
    };
    // Seat 0 buys and plays Poets until the last Sun is gone; seat 1 does nothing.
    while suns(&t) > 0 {
        let seat = t.deciding().expect("the game goes on");
        let d = t.view(seat).decision.unwrap();
        let end = pick(&d, |o| o.action == "end-phase");
        let answer = if d.kind != "turn" {
            first(&d)
        } else if seat.0 != 0 {
            end.unwrap()
        } else {
            pick(&d, |o| o.action == "play")
                .or_else(|| pick(&d, |o| o.label.starts_with("Buy Poet")))
                .or(end)
                .unwrap()
        };
        t.answer(seat, &answer).unwrap();
    }
    let view = t.view(PlayerId(1));
    let flagged: Vec<&str> = view
        .supply
        .iter()
        .filter(|p| p.kingdom)
        .map(|p| p.card.as_str())
        .collect();
    assert_eq!(flagged.len(), 10);
    assert!(!flagged.contains(&"Poet"));
    assert_eq!(view.game.kingdom, flagged);
}

#[test]
fn the_black_market_deck_is_a_face_down_pile_of_known_cards() {
    let t = table(&[Card::BlackMarket], 4, &[]);
    let view = t.view(PlayerId(0));
    let deck = view
        .non_supply
        .iter()
        .find(|p| p.hidden)
        .expect("the Black Market deck");
    assert!(!deck.kingdom && deck.cost.is_none() && deck.types.is_empty());
    let mut listed: Vec<&str> = view
        .notes
        .iter()
        .filter(|n| n.source == "Black Market" && n.count == 1)
        .filter_map(|n| n.card.as_deref())
        .collect();
    let g = t.game();
    let pile = g.piles().iter().find(|p| p.hidden).unwrap();
    let mut cards: Vec<&str> = pile.cards.iter().map(|&c| g.card(c).name()).collect();
    listed.sort();
    cards.sort();
    assert_eq!(listed, cards);
    let rendered = text::render(&view);
    let line = rendered
        .lines()
        .find(|l| l.starts_with("NON-SUPPLY"))
        .unwrap();
    assert!(line.contains("Black Market (face down) x60"), "{line}");
}

/// The text names seats P1, P2, ... (as engine-made labels do), so each player's header
/// says which seat they are.
#[test]
fn text_introduces_the_seat_numbers_it_uses() {
    let t = table(&[Card::Militia], 11, &["Ada", "Bo"]);
    let mut view = t.view(PlayerId(1));
    view.lasting.push(dominion_agent::LastingView {
        card: "Haven".into(),
        owner: 0,
        until: "start-of-turn".into(),
        until_player: Some(0),
        cards: vec![],
        count: None,
    });
    let rendered = text::render(&view);
    assert!(rendered.contains("YOU P2: Bo") && rendered.contains("OPPONENT P1: Ada"));
    assert!(
        rendered.contains("Haven (P1) until P1's next turn"),
        "{rendered}"
    );
}

#[test]
fn a_yes_or_no_question_says_what_yes_does() {
    let mut t = table(&[Card::MiningVillage], 3, &[]);
    let asked = |_: PlayerId, d: &AgentDecision| {
        d.source.as_deref() == Some("Mining Village") && d.kind == "choose"
    };
    let seat = play_to(&mut t, "Mining Village", asked).expect("Mining Village asks");
    let d = t.view(seat).decision.unwrap();
    assert_eq!(d.prompt, "Mining Village: trash Mining Village for +$2?");
    let labels: Vec<&str> = d.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Trash Mining Village for +$2", "No"]);
}

#[test]
fn a_reaction_says_what_it_answers_and_does_and_names_no_single_source() {
    let mut t = table(&[Card::Militia], 5, &["Ada", "Bo"]);
    let mut bought = false;
    let mut asked = None;
    for _ in 0..2000 {
        let Some(seat) = t.deciding() else { break };
        let d = t.view(seat).decision.unwrap();
        if seat.0 == 1 && d.purpose == "trigger" {
            asked = Some(d);
            break;
        }
        let answer = if d.kind != "turn" {
            first(&d)
        } else if seat.0 == 0 {
            pick(&d, |o| o.action == "play")
                .or_else(|| {
                    pick(&d, |o| !bought && o.label.starts_with("Buy Militia"))
                        .inspect(|_| bought = true)
                })
                .or_else(|| pick(&d, |o| o.action == "end-phase"))
                .unwrap()
        } else {
            pick(&d, |o| {
                o.action == "play" && o.card.as_deref() != Some("Moat")
            })
            .or_else(|| pick(&d, |o| o.label.starts_with("Buy Moat")))
            .or_else(|| pick(&d, |o| o.action == "end-phase"))
            .unwrap()
        };
        t.answer(seat, &answer).unwrap();
    }
    let d = asked.expect("Bo is asked about Moat");
    assert_eq!(d.source, None);
    let (decline, moats) = d.options.split_last().unwrap();
    assert!(!moats.is_empty());
    let prompt = match moats.len() {
        1 => "P1 attacks you with Militia: you may use this ability",
        _ => "P1 attacks you with Militia: choose an ability to use next, or decline all",
    };
    assert_eq!(d.prompt, prompt);
    for o in moats {
        assert_eq!(o.label, "Moat: reveal it to be unaffected (optional)");
        assert_eq!(o.card.as_deref(), Some("Moat"));
    }
    assert!(decline.label.starts_with("Decline"), "{}", decline.label);
}

#[test]
fn the_result_lists_players_by_rank_and_says_why_the_game_ended() {
    let t = table(&[Card::Militia], 11, &["Ada", "Bo"]);
    let mut view = t.view(PlayerId(0));
    view.game.result = Some(dominion_agent::GameResult {
        reason: Some("provinces-gone".into()),
        players: vec![
            dominion_agent::ResultPlayer {
                index: 0,
                name: "Ada".into(),
                vp: 10,
                rank: 2,
                turns: 14,
                resigned: false,
            },
            dominion_agent::ResultPlayer {
                index: 1,
                name: "Bo".into(),
                vp: 20,
                rank: 1,
                turns: 14,
                resigned: false,
            },
        ],
    });
    let rendered = text::render(&view);
    assert!(
        rendered.contains(
            "RESULT (the Provinces ran out): 1. Bo 20 VP (14 turns), 2. Ada 10 VP (14 turns)"
        ),
        "{rendered}"
    );
}

#[test]
fn the_journey_token_is_shown_face_up_or_down_in_games_using_it() {
    let journey = |t: &Table| t.view(PlayerId(0)).you.unwrap().tokens.journey_face_up;
    assert_eq!(journey(&table(&[Card::Militia], 1, &[])), None);
    let t = table(&[Card::Ranger], 1, &[]);
    assert_eq!(journey(&t), Some(true));
    assert!(t.text(PlayerId(1)).contains("Journey token face up"));
    let mut view = t.view(PlayerId(0));
    view.you.as_mut().unwrap().tokens.journey_face_up = Some(false);
    assert!(text::render(&view).contains("Journey token face down"));
}

#[test]
fn the_log_says_which_ability_changed_which_tokens() {
    let mut t = table(&[Card::SilkMerchant], 2, &["Ada", "Bo"]);
    let waiting = |seat: PlayerId, _: &AgentDecision| seat.0 == 1;
    let bought = |t: &Table| t.game().count_owned(PlayerId(0), Card::SilkMerchant) > 0;
    while !bought(&t) {
        let seat = play_to(&mut t, "Silk Merchant", waiting).expect("the game goes on");
        if !bought(&t) {
            let d = t.view(seat).decision.unwrap();
            let end = pick(&d, |o| o.action == "end-phase").unwrap();
            t.answer(seat, &end).unwrap();
        }
    }
    let lines: Vec<String> = t
        .view(PlayerId(1))
        .since_last_decision
        .into_iter()
        .map(|e| e.text)
        .collect();
    let at = lines
        .iter()
        .position(|l| l == "Ada gains Silk Merchant")
        .unwrap_or_else(|| panic!("{lines:#?}"));
    assert_eq!(
        lines[at + 1..at + 4],
        [
            "Ada's Silk Merchant resolves",
            "Ada takes 1 Coffers",
            "Ada takes 1 Villager"
        ],
        "{lines:#?}"
    );
}

#[test]
fn the_final_view_knows_the_last_clean_ups_discard_top() {
    let config = GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        turn_limit: Some(2),
        ..GameConfig::new(2, vec![Card::Smithy], 3)
    };
    let mut t = Table::new("t", config, vec![]).unwrap();
    while let Some(seat) = t.deciding() {
        let d = t.view(seat).decision.unwrap();
        let answer = pick(&d, |o| o.action == "end-phase").unwrap_or_else(|| first(&d));
        t.answer(seat, &answer).unwrap();
    }
    let g = t.game();
    let last = PlayerId(1);
    let top = *g.player(last).discard.last().unwrap();
    let known = t.knowledge().known(g, PlayerId(0));
    let discard = known.zone(dominion_engine::Zone::Discard(last));
    assert!(discard.known.contains_key(&g.card(top)), "{discard:?}");
}
