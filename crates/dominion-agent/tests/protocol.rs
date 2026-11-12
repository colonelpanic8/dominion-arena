use dominion_agent::{
    AgentDecision, AgentOption, Answer, AnswerError, Answered, JSON_SCHEMA, Table, WAY_HINT,
    WayChoice, glossary_for, text,
};
use dominion_engine::{
    Card, CardId, DecisionKind, GameConfig, Kind, Move, Phase, PlayerId, Set, Types,
};
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
            assert_cards_placed_where_it_tells_them_apart(&d);
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
            // Sometimes with a Way question answered in advance.
            let ways: Vec<Card> = (table.game().landscapes().iter())
                .copied()
                .filter(|l| l.kind() == Kind::Way)
                .collect();
            let way = (!ways.is_empty() && rng.random_bool(0.3)).then(|| {
                match ways.get(rng.random_range(0..=ways.len())) {
                    Some(&w) => WayChoice::Way(w),
                    None => WayChoice::None,
                }
            });
            if let Err(e) = table.answer_with(seat, &answer, way) {
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

/// Card options say where their cards are when not all are in one place, in words.
fn assert_cards_placed_where_it_tells_them_apart(d: &AgentDecision) {
    let cards: Vec<&AgentOption> = (d.options.iter())
        .filter(|o| matches!(o.action.as_str(), "select" | "play") && o.zone.is_some())
        .collect();
    let zones: std::collections::BTreeSet<_> = cards.iter().map(|o| &o.zone).collect();
    for o in &cards {
        let name = o.card.as_deref().unwrap_or("a card");
        let plain = [name.to_string(), format!("Play {name}")];
        assert!(
            zones.len() == 1 || !plain.contains(&o.label),
            "{o:?} in {}",
            d.prompt
        );
        assert!(
            !o.label.contains("(from ") || o.label.contains("(from the top of "),
            "{o:?}"
        );
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

/// The game and every seat's knowledge, which an undo must restore exactly.
fn state_of(t: &Table) -> (Value, Value) {
    (
        serde_json::to_value(t.game()).unwrap(),
        serde_json::to_value(t.knowledge()).unwrap(),
    )
}

#[test]
fn undo_takes_back_a_seats_answers_until_another_seat_answered() {
    for seed in 0..seeds(4) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut config = GameConfig::random(2, &released(), seed);
        config.turn_limit = Some(12);
        let mut table = Table::new(format!("g{seed}"), config, vec![]).unwrap();
        // The states before each answer of the current run of one seat's answers.
        let mut run: Vec<(PlayerId, (Value, Value))> = vec![];
        let mut undos = 0;
        while let Some(seat) = table.deciding() {
            let other = PlayerId(1 - seat.0);
            assert!(!table.can_undo(other) || run.last().is_some_and(|(s, _)| *s == other));
            if run.last().is_some_and(|(s, _)| *s == seat) && rng.random_bool(0.3) {
                let undone_id = table.decision_id().unwrap();
                let (_, before) = run.pop().unwrap();
                assert!(table.undo(seat));
                undos += 1;
                assert_eq!(table.undos(), undos);
                assert_eq!(state_of(&table), before, "seed {seed}");
                assert_ne!(table.decision_id().unwrap(), undone_id);
                assert_eq!(table.can_undo(seat), !run.is_empty());
                assert_eq!(table.undo_note(seat), None);
                let note = table.undo_note(other).expect("the other seat is told");
                assert!(note.contains(&format!("{seat} took back")), "{note}");
                continue;
            }
            let before = state_of(&table);
            let d = table.view(seat).decision.unwrap();
            table.answer(seat, &random_answer(&d, &mut rng)).unwrap();
            if run.last().is_some_and(|(s, _)| *s != seat) {
                run.clear();
            }
            run.push((seat, before));
            assert_eq!(table.undo_note(seat), None);
            assert!(table.can_undo(seat));
            assert!(!table.undo(other));
        }
        assert!(undos > 0, "seed {seed}");
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
        format!(
            "there is no option {n}: the options are numbered 0 to {}",
            n - 1
        )
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
        .looking
        .iter()
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

/// A table of ten base cards, Divine Wind and `landscapes`, whose seat 0 buys and plays
/// Poets until the last Sun is gone; seat 1 does nothing.
fn after_divine_wind(landscapes: &[Card], choices: &[(Card, Card)]) -> Table {
    let mut config = GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: [&[Card::DivineWind], landscapes].concat(),
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
    for &(by, choice) in choices {
        config.setup_choices.insert(by, vec![choice]);
    }
    let mut t = Table::new("t", config, vec![]).unwrap();
    let suns = |t: &Table| {
        let view = t.view(PlayerId(1));
        let wind = view.landscapes.iter().find(|l| l.card == "Divine Wind");
        wind.and_then(|l| l.suns).expect("Divine Wind's Sun tokens")
    };
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
    t
}

#[test]
fn divine_wind_replaces_the_kingdom_and_the_log_says_so() {
    let t = after_divine_wind(&[], &[]);
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
    let log: Vec<String> = t
        .view_from(PlayerId(1), 0)
        .since_last_decision
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert!(log.contains(&"The Poet pile is removed from the game".to_string()));
    for pile in view.supply.iter().filter(|p| p.kingdom) {
        let joined = format!("The {} pile joins the Supply (", pile.card);
        assert!(log.iter().any(|l| l.starts_with(&joined)), "{log:#?}");
    }
}

/// "If Divine Wind goes into effect, the pile with the Trait is removed from the supply,
/// but cards from that removed pile still have the Trait" (wiki, Trait): a note says so.
#[test]
fn a_trait_on_a_pile_divine_wind_removed_is_still_noted() {
    let t = after_divine_wind(&[Card::Cheap], &[(Card::Cheap, Card::Smithy)]);
    let view = t.view(PlayerId(1));
    assert!(!view.supply.iter().any(|p| p.card == "Smithy"));
    assert!(
        view.notes
            .iter()
            .any(|n| n.source == "Cheap" && n.pile.as_deref() == Some("Smithy")),
        "{:#?}",
        view.notes
    );
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

/// The text names seats P1, P2, ... in its sections, so each player's header says which
/// seat they are.
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
        1 => "Ada attacks you with Militia: you may use this ability",
        _ => "Ada attacks you with Militia: choose an ability to use next, or decline all",
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
            "Ada takes a Coffers",
            "Ada takes 1 Villager"
        ],
        "{lines:#?}"
    );
}

/// A Duration's effect lasts until its owner's next turn, which is when its start-of-turn
/// ability removes it.
#[test]
fn a_durations_effect_says_it_lasts_until_its_owners_next_turn() {
    let mut t = table(&[Card::Lighthouse], 3, &["Ada", "Bo"]);
    let waiting = |seat: PlayerId, _: &AgentDecision| seat.0 == 1;
    loop {
        let seat = play_to(&mut t, "Lighthouse", waiting).expect("the game goes on");
        let v = t.view(seat);
        if let Some(l) = v.lasting.iter().find(|l| l.card == "Lighthouse") {
            assert_eq!(
                (l.until.as_str(), l.until_player, l.count),
                ("start-of-turn", Some(0), None)
            );
            let text = text::render(&v);
            assert!(
                text.contains("Lighthouse (P1) until P1's next turn"),
                "{text}"
            );
            break;
        }
        let end = pick(&v.decision.unwrap(), |o| o.action == "end-phase").unwrap();
        t.answer(seat, &end).unwrap();
    }
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

/// A 2-player game with Throne Room, Laboratory and `landscapes`.
fn way_table(landscapes: &[Card], seed: u64) -> Table {
    let config = GameConfig {
        landscapes: landscapes.to_vec(),
        colonies: Some(false),
        shelters: Some(false),
        setup_choices: (landscapes.iter())
            .filter(|&&l| l == Card::WayOfTheMouse)
            .map(|&l| (l, vec![Card::Village]))
            .collect(),
        ..GameConfig::new(
            2,
            vec![
                Card::ThroneRoom,
                Card::Laboratory,
                Card::Cellar,
                Card::Market,
                Card::Mine,
                Card::Moat,
                Card::Remodel,
                Card::Smithy,
                Card::Workshop,
                Card::Festival,
            ],
            seed,
        )
    };
    Table::new("t", config, vec![]).unwrap()
}

fn plays(d: &AgentDecision, card: &str) -> Option<u32> {
    (d.options.iter())
        .find(|o| o.action == "play" && o.card.as_deref() == Some(card))
        .map(|o| o.index)
}

/// Seat 0 plays only Treasures and buys Throne Rooms and Laboratories; the other seat
/// does nothing. Stops at seat 0's first Action phase decision `stop` accepts.
fn wait_for_hand(t: &mut Table, stop: impl Fn(&AgentDecision) -> bool) -> AgentDecision {
    let mut buys = 0;
    for _ in 0..2000 {
        let seat = t.deciding().expect("the game goes on");
        let d = t.view(seat).decision.unwrap();
        let answer = if d.kind != "turn" {
            first(&d)
        } else if seat.0 != 0 {
            pick(&d, |o| o.action == "end-phase").unwrap()
        } else if d.phase.as_deref() == Some("action") {
            if stop(&d) {
                return d;
            }
            pick(&d, |o| o.action == "end-phase").unwrap()
        } else {
            let want = ["Buy Laboratory", "Buy Throne Room"][buys % 2];
            pick(&d, |o| o.action == "play")
                .or_else(|| pick(&d, |o| o.label.starts_with(want)).inspect(|_| buys += 1))
                .or_else(|| pick(&d, |o| o.action == "end-phase"))
                .unwrap()
        };
        t.answer(seat, &answer).unwrap();
    }
    panic!("seat 0 never gets the hand");
}

fn is_way_question(d: &AgentDecision) -> bool {
    d.purpose == "mode" && d.prompt.contains("use a Way instead")
}

fn assert_views_valid(t: &Table) {
    let schema = validator("view");
    for p in 0..2 {
        let json = serde_json::to_value(t.view(PlayerId(p))).unwrap();
        assert_valid(&schema, &json, "view");
    }
}

#[test]
fn a_play_with_way_none_needs_no_second_answer() {
    let mut t = way_table(&[Card::WayOfTheOx], 3);
    let d = wait_for_hand(&mut t, |d| {
        plays(d, "Throne Room").is_some() && plays(d, "Laboratory").is_some()
    });
    assert!(d.prompt.ends_with(dominion_agent::WAY_HINT), "{}", d.prompt);
    let play = Answer::indices(&d.id, vec![plays(&d, "Laboratory").unwrap()]);
    let mut separately = t.clone();

    let answered = t
        .answer_with(PlayerId(0), &play, Some(WayChoice::None))
        .unwrap();
    assert_eq!(
        answered,
        Answered {
            ways_answered: 1,
            way_not_offered: None
        }
    );
    assert_eq!(answered.note(), None);
    let after = t.view(PlayerId(0));
    let next = after.decision.clone().unwrap();
    assert_eq!(next.kind, "turn");
    assert!(
        after
            .since_last_decision
            .iter()
            .any(|e| e.text.ends_with("plays Laboratory"))
    );
    assert_views_valid(&t);

    separately.answer(PlayerId(0), &play).unwrap();
    let asked = separately.view(PlayerId(0)).decision.unwrap();
    assert!(is_way_question(&asked), "{asked:?}");
    assert_eq!(asked.source.as_deref(), Some("Laboratory"));
    assert_eq!(asked.options[0].label, "Play Laboratory");
    separately
        .answer(PlayerId(0), &Answer::indices(&asked.id, vec![0]))
        .unwrap();
    assert_eq!(
        serde_json::to_value(separately.game()).unwrap(),
        serde_json::to_value(t.game()).unwrap()
    );
    assert_eq!(separately.decision_id(), t.decision_id());

    // The next play, answered without one, is asked again.
    let throne = plays(&next, "Throne Room").unwrap();
    t.answer(PlayerId(0), &Answer::indices(&next.id, vec![throne]))
        .unwrap();
    assert!(is_way_question(&t.view(PlayerId(0)).decision.unwrap()));
}

#[test]
fn a_way_named_in_advance_is_used() {
    let mut t = way_table(&[Card::WayOfTheOx], 1);
    let d = wait_for_hand(&mut t, |d| plays(d, "Laboratory").is_some());
    let play = Answer::indices(&d.id, vec![plays(&d, "Laboratory").unwrap()]);
    let way = WayChoice::parse("way of the ox").unwrap();
    let answered = t.answer_with(PlayerId(0), &play, Some(way)).unwrap();
    assert_eq!(answered.ways_answered, 1);
    let view = t.view(PlayerId(0));
    assert!(
        (view.since_last_decision.iter())
            .any(|e| e.text.ends_with("uses Way of the Ox for Laboratory")),
        "{:?}",
        view.since_last_decision
    );
    // Way of the Ox's +2 Actions instead of Laboratory's +1 Action.
    assert_eq!(view.turn.actions, Some(2));
}

#[test]
fn a_way_answered_in_advance_answers_each_replay() {
    let mut t = way_table(&[Card::WayOfTheOx], 3);
    let d = wait_for_hand(&mut t, |d| {
        plays(d, "Throne Room").is_some() && plays(d, "Laboratory").is_some()
    });
    let play = Answer::indices(&d.id, vec![plays(&d, "Throne Room").unwrap()]);
    let answered = t
        .answer_with(PlayerId(0), &play, Some(WayChoice::None))
        .unwrap();
    assert_eq!(answered.ways_answered, 1);
    let choice = t.view(PlayerId(0)).decision.unwrap();
    assert_eq!(choice.source.as_deref(), Some("Throne Room"));
    assert!(
        choice.prompt.ends_with(dominion_agent::WAY_HINT),
        "{}",
        choice.prompt
    );
    let lab = pick(&choice, |o| o.card.as_deref() == Some("Laboratory")).unwrap();
    let answered = t
        .answer_with(PlayerId(0), &lab, Some(WayChoice::None))
        .unwrap();
    assert_eq!(answered.ways_answered, 2);
    let view = t.view(PlayerId(0));
    assert_eq!(view.decision.unwrap().kind, "turn");
    let labs = (view.since_last_decision.iter())
        .filter(|e| e.text.ends_with("plays Laboratory"))
        .count();
    assert_eq!(labs, 2);
    assert_views_valid(&t);
}

#[test]
fn way_pre_answer_leaves_the_mouses_different_card_open() {
    // Way of the Mouse plays the set-aside Village, which can't use Way of the Mouse.
    let mut t = way_table(&[Card::WayOfTheMouse, Card::WayOfTheOx], 1);
    let d = wait_for_hand(&mut t, |d| plays(d, "Laboratory").is_some());
    let play = Answer::indices(&d.id, vec![plays(&d, "Laboratory").unwrap()]);
    let mouse = Some(WayChoice::Way(Card::WayOfTheMouse));
    let answered = t.answer_with(PlayerId(0), &play, mouse).unwrap();
    assert_eq!(
        answered,
        Answered {
            ways_answered: 1,
            way_not_offered: None
        }
    );
    assert_eq!(answered.note(), None);
    let open = t.view(PlayerId(0)).decision.unwrap();
    assert!(is_way_question(&open), "{open:?}");
    assert_eq!(open.source.as_deref(), Some("Village"));
    assert_views_valid(&t);
}

#[test]
fn a_way_not_in_the_game_rejects_the_answer() {
    let mut t = way_table(&[Card::WayOfTheOx], 1);
    let d = wait_for_hand(&mut t, |d| plays(d, "Laboratory").is_some());
    let play = Answer::indices(&d.id, vec![plays(&d, "Laboratory").unwrap()]);
    let before = serde_json::to_value(t.game()).unwrap();
    let mole = Some(WayChoice::Way(Card::WayOfTheMole));
    let Err(AnswerError::Invalid(msg)) = t.answer_with(PlayerId(0), &play, mole) else {
        panic!("a Way not in the game is accepted");
    };
    assert_eq!(
        msg,
        "Way of the Mole is not in this game (its Ways: Way of the Ox)"
    );
    assert_eq!(serde_json::to_value(t.game()).unwrap(), before);
    assert!(WayChoice::parse("Way of the Moose").is_err());
    assert!(WayChoice::parse("Laboratory").is_err());
}

#[test]
fn a_way_answered_in_advance_is_ignored_when_no_way_question_follows() {
    let mut t = way_table(&[Card::WayOfTheOx], 1);
    let d = wait_for_hand(&mut t, |d| plays(d, "Laboratory").is_some());
    let end = pick(&d, |o| o.action == "end-phase").unwrap();
    let answered = t
        .answer_with(PlayerId(0), &end, Some(WayChoice::Way(Card::WayOfTheOx)))
        .unwrap();
    assert_eq!(answered, Answered::default());
    let buy = t.view(PlayerId(0)).decision.unwrap();
    assert_eq!(buy.phase.as_deref(), Some("buy"));
    assert!(!buy.prompt.contains("Way"), "{}", buy.prompt);

    let mut t = way_table(&[], 1);
    let d = wait_for_hand(&mut t, |d| plays(d, "Laboratory").is_some());
    assert!(!d.prompt.contains("Way"), "{}", d.prompt);
    let play = Answer::indices(&d.id, vec![plays(&d, "Laboratory").unwrap()]);
    let ox = Some(WayChoice::Way(Card::WayOfTheOx));
    let Err(AnswerError::Invalid(msg)) = t.clone().answer_with(PlayerId(0), &play, ox) else {
        panic!("a Way is accepted in a game without Ways");
    };
    assert_eq!(msg, "Way of the Ox is not in this game, which has no Ways");
    let answered = t
        .answer_with(PlayerId(0), &play, Some(WayChoice::None))
        .unwrap();
    assert_eq!(answered, Answered::default());
}

#[test]
fn a_card_returned_to_its_pile_is_logged_as_returned() {
    let mut t = table(&[Card::Experiment], 0, &[]);
    let seat_1_turn = |seat: PlayerId, d: &AgentDecision| seat.0 == 1 && d.kind == "turn";
    for _ in 0..30 {
        assert_eq!(
            play_to(&mut t, "Experiment", seat_1_turn),
            Some(PlayerId(1))
        );
        let view = t.view(PlayerId(1));
        let returned: Vec<&str> = view
            .since_last_decision
            .iter()
            .filter(|e| e.text.contains("Experiment") && e.text.contains("pile"))
            .map(|e| e.text.as_str())
            .collect();
        if !returned.is_empty() {
            assert!(
                returned.contains(&"P1 returns Experiment from play to the Experiment pile"),
                "{returned:?}"
            );
            return;
        }
        while t.deciding() == Some(PlayerId(1)) {
            let d = t.view(PlayerId(1)).decision.unwrap();
            let end = pick(&d, |o| o.action == "end-phase").unwrap_or_else(|| first(&d));
            t.answer(PlayerId(1), &end).unwrap();
        }
    }
    panic!("Experiment was never played");
}

/// Cards taken off a deck to be looked at are shown to the looker as what they are
/// looking at, and to the others only as how many there are.
#[test]
fn cards_being_looked_at_are_the_lookers_and_counted_for_the_others() {
    let mut t = table(&[Card::Navigator], 1, &[]);
    let asked = |_: PlayerId, d: &AgentDecision| d.source.as_deref() == Some("Navigator");
    assert_eq!(play_to(&mut t, "Navigator", asked), Some(PlayerId(0)));
    let looker = t.view(PlayerId(0));
    let zone = &looker.you.as_ref().unwrap().other_zones["looking"];
    assert_eq!((zone.count, zone.unknown), (5, 0));
    assert!(text::render(&looker).contains("\n  looking 5 ("));
    let other = t.view(PlayerId(1));
    let zone = &other.opponents[0].other_zones["looking"];
    assert_eq!((zone.count, zone.unknown), (5, 5));
    assert!(text::render(&other).contains("\n  looking 5 (5 unknown)"));
    let d = looker.decision.unwrap();
    let back = pick(&d, |o| o.label.starts_with("Put them back")).unwrap();
    t.answer(PlayerId(0), &back).unwrap();
    let looker = t.view(PlayerId(0));
    let d = looker.decision.as_ref().unwrap();
    assert!(
        d.ordered && d.options.iter().all(|o| o.card.is_some()),
        "{d:?}"
    );
    let text = text::render(&looker);
    assert!(!text.contains("(from "), "{text}");
}

#[test]
fn a_card_put_into_the_middle_of_a_deck_says_how_deep() {
    let mut t = table(&[Card::SecretPassage], 0, &[]);
    let asked = |_: PlayerId, d: &AgentDecision| {
        d.source.as_deref() == Some("Secret Passage")
            && d.kind == "number"
            && d.options.iter().any(|o| o.value == Some(2))
            && d.options.iter().any(|o| o.value == Some(3))
    };
    let seat = play_to(&mut t, "Secret Passage", asked).expect("Secret Passage was played");
    let d = t.view(seat).decision.unwrap();
    let two = pick(&d, |o| o.value == Some(2)).unwrap();
    t.answer(seat, &two).unwrap();
    let view = t.view(PlayerId(1 - seat.0));
    let lines: Vec<&str> = view
        .since_last_decision
        .iter()
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        lines.contains(&"P1 puts a card from their hand into their deck, under 2 cards"),
        "{lines:?}"
    );
}

/// Seat 0's deck, top first, by card name.
fn deck_names(t: &Table) -> Vec<String> {
    let game: Value = serde_json::to_value(t.game()).unwrap();
    let cards = game["cards"].as_array().unwrap();
    let deck = game["players"][0]["deck"].as_array().unwrap();
    deck.iter()
        .rev()
        .map(|id| {
            let card: Card =
                serde_json::from_value(cards[id.as_u64().unwrap() as usize].clone()).unwrap();
            card.name().to_string()
        })
        .collect()
}

#[test]
fn way_pre_answer_stops_at_a_different_physical_herald() {
    for seed in 0..40 {
        let config = GameConfig {
            landscapes: vec![Card::WayOfTheOx],
            colonies: Some(false),
            shelters: Some(false),
            ..GameConfig::new(
                2,
                vec![
                    Card::Herald,
                    Card::Cellar,
                    Card::Market,
                    Card::Mine,
                    Card::Moat,
                    Card::Remodel,
                    Card::Smithy,
                    Card::Workshop,
                    Card::Festival,
                    Card::Laboratory,
                ],
                seed,
            )
        };
        let mut t = Table::new("t", config, vec![]).unwrap();
        for _ in 0..3000 {
            let Some(seat) = t.deciding() else { break };
            let d = t.view(seat).decision.unwrap();
            let end = pick(&d, |o| o.action == "end-phase");
            let answer = if d.kind != "turn" {
                first(&d)
            } else if seat.0 != 0 {
                end.unwrap()
            } else if d.phase.as_deref() == Some("action") {
                let herald = pick(&d, |o| {
                    o.action == "play" && o.card.as_deref() == Some("Herald")
                });
                let deck = deck_names(&t);
                // +1 Card draws the top card; the next one is revealed.
                match herald {
                    Some(play) if deck.get(1).map(String::as_str) == Some("Herald") => {
                        let answered = t
                            .answer_with(PlayerId(0), &play, Some(WayChoice::None))
                            .unwrap();
                        let view = t.view(PlayerId(0));
                        let open = view.decision.unwrap();
                        assert!(
                            view.since_last_decision
                                .iter()
                                .any(|e| e.text.ends_with("reveals Herald")),
                            "{:?}",
                            view.since_last_decision
                        );
                        assert_eq!(
                            (answered.ways_answered, open.source.as_deref()),
                            (1, Some("Herald")),
                            "the revealed Herald's Way question was answered too: {}",
                            open.prompt
                        );
                        assert!(open.prompt.contains("use a Way instead"), "{}", open.prompt);
                        return;
                    }
                    _ => end.unwrap(),
                }
            } else {
                pick(&d, |o| o.action == "play")
                    .or_else(|| pick(&d, |o| o.label.starts_with("Buy Herald")))
                    .or_else(|| end.clone())
                    .unwrap()
            };
            t.answer(seat, &answer).unwrap();
        }
    }
    panic!("no game got a Herald in hand with another second from the top of the deck");
}

#[test]
fn way_pre_answer_waits_while_another_seat_reacts_to_the_attack() {
    let config = GameConfig {
        landscapes: vec![Card::WayOfTheOx],
        ..table(&[Card::Militia], 5, &[]).game().config().clone()
    };
    let mut t = Table::new("t", config, vec![]).unwrap();
    let moat_in_hand = |t: &Table| {
        let g = t.game();
        (g.player(PlayerId(1)).hand.iter()).any(|&c| g.card(c) == Card::Moat)
    };
    let mut bought = [false; 2];
    for _ in 0..3000 {
        let Some(seat) = t.deciding() else { break };
        let d = t.view(seat).decision.unwrap();
        let militia = pick(&d, |o| {
            o.action == "play" && o.card.as_deref() == Some("Militia")
        });
        if let Some(play) = militia.filter(|_| seat.0 == 0 && moat_in_hand(&t)) {
            let answered = t
                .answer_with(PlayerId(0), &play, Some(WayChoice::None))
                .unwrap();
            assert_eq!(answered.ways_answered, 0);
            let reveal = t.view(PlayerId(1)).decision.unwrap();
            assert_eq!(reveal.purpose, "trigger", "{}", reveal.prompt);
            // Revealing Moat plays nothing.
            assert!(!reveal.prompt.contains(WAY_HINT), "{}", reveal.prompt);
            // It survives a save, and another seat's answer with a Way doesn't drop it.
            let mut t: Table = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
            let moat = pick(&reveal, |o| o.card.as_deref() == Some("Moat")).unwrap();
            t.answer_with(PlayerId(1), &moat, Some(WayChoice::None))
                .unwrap();
            let next = t.view(PlayerId(0));
            let d = next.decision.unwrap();
            assert!(!is_way_question(&d), "{}", d.prompt);
            assert_eq!(d.kind, "turn");
            let log: Vec<&str> = next
                .since_last_decision
                .iter()
                .map(|e| e.text.as_str())
                .collect();
            assert!(log.iter().any(|e| e.ends_with("plays Militia")), "{log:?}");
            assert!(!log.iter().any(|e| e.contains("Way of the Ox")), "{log:?}");
            return;
        }
        let end = pick(&d, |o| o.action == "end-phase");
        let want = ["Buy Militia", "Buy Moat"][seat.index()];
        let answer = if d.kind != "turn" {
            first(&d)
        } else if d.phase.as_deref() == Some("action") {
            end.unwrap()
        } else {
            pick(&d, |o| o.action == "play")
                .or_else(|| {
                    pick(&d, |o| !bought[seat.index()] && o.label.starts_with(want))
                        .inspect(|_| bought[seat.index()] = true)
                })
                .or(end)
                .unwrap()
        };
        t.answer(seat, &answer).unwrap();
    }
    panic!("P1 never has Militia while P2 holds Moat");
}

#[test]
fn cards_offered_from_different_places_say_where_they_are() {
    let mut t = table(&[Card::Fishmonger, Card::Merchant], 2, &[]);
    let plays_from = |d: &AgentDecision, z: &str| {
        (d.options.iter()).any(|o| o.action == "play" && o.zone.as_deref() == Some(z))
    };
    let mut buys = 0;
    for _ in 0..3000 {
        let seat = t.deciding().expect("the game goes on");
        let d = t.view(seat).decision.unwrap();
        if seat.0 == 0 && plays_from(&d, "deck") && plays_from(&d, "hand") {
            for o in d.options.iter().filter(|o| o.action == "play") {
                let card = o.card.as_deref().unwrap();
                let place = match o.zone.as_deref() {
                    Some("deck") => "in your deck",
                    _ => "in your hand",
                };
                assert_eq!(o.label, format!("Play {card} ({place})"));
            }
            return;
        }
        let end = pick(&d, |o| o.action == "end-phase");
        let answer = if d.kind != "turn" {
            first(&d)
        } else if seat.0 != 0 || d.phase.as_deref() == Some("action") {
            end.unwrap()
        } else {
            let want = ["Buy Fishmonger", "Buy Merchant"][buys % 2];
            pick(&d, |o| o.action == "play")
                .or_else(|| pick(&d, |o| o.label.starts_with(want)).inspect(|_| buys += 1))
                .or(end)
                .unwrap()
        };
        t.answer(seat, &answer).unwrap();
    }
    panic!("P1 never may play a Shadow card from the deck and a card from the hand");
}

#[test]
fn an_ability_that_plays_the_gained_card_hints_at_the_way_question() {
    let config = GameConfig {
        landscapes: vec![Card::Innovation, Card::WayOfTheOx],
        ..table(&[], 3, &[]).game().config().clone()
    };
    let mut t = Table::new("t", config, vec![]).unwrap();
    let mut innovation = false;
    for _ in 0..3000 {
        let seat = t.deciding().expect("the game goes on");
        let d = t.view(seat).decision.unwrap();
        if d.purpose == "trigger" && pick(&d, |o| o.card.as_deref() == Some("Innovation")).is_some()
        {
            assert!(d.prompt.ends_with(WAY_HINT), "{}", d.prompt);
            let play = pick(&d, |o| o.card.as_deref() == Some("Innovation")).unwrap();
            let answered = t.answer_with(seat, &play, Some(WayChoice::None)).unwrap();
            assert_eq!(answered.ways_answered, 1);
            return;
        }
        let end = pick(&d, |o| o.action == "end-phase");
        let answer = if d.kind != "turn" {
            first(&d)
        } else if seat.0 != 0 || d.phase.as_deref() == Some("action") {
            end.unwrap()
        } else {
            pick(&d, |o| o.action == "play")
                .or_else(|| {
                    pick(&d, |o| {
                        !innovation && o.label.starts_with("Buy Project Innovation")
                    })
                    .inspect(|_| innovation = true)
                })
                .or_else(|| {
                    let want = if innovation {
                        "Buy Village"
                    } else {
                        "Buy Silver"
                    };
                    pick(&d, |o| o.label.starts_with(want))
                })
                .or(end)
                .unwrap()
        };
        t.answer(seat, &answer).unwrap();
    }
    panic!("Innovation is never asked about");
}

#[test]
fn black_markets_purchase_is_a_buy_and_shows_each_cards_cost() {
    let mut t = table(&[Card::BlackMarket], 4, &[]);
    let buying = |_: PlayerId, d: &AgentDecision| {
        d.source.as_deref() == Some("Black Market") && d.prompt.contains("to buy")
    };
    assert_eq!(play_to(&mut t, "Black Market", buying), Some(PlayerId(0)));
    let d = t.view(PlayerId(0)).decision.unwrap();
    assert_eq!(d.prompt, "Black Market: choose up to 1 card to buy");
    assert!(!d.options.is_empty());
    for o in &d.options {
        let cost = o.cost.as_ref().expect("a cost");
        let card = o.card.as_deref().unwrap();
        assert_eq!(o.label, format!("{card} (${})", cost.coins));
    }
}

/// `t` with the top card of `seat`'s deck swapped with a lower card that is `wanted`.
fn with_top_card(t: &Table, seat: usize, wanted: impl Fn(Card) -> bool) -> Option<Table> {
    let mut json: Value = serde_json::to_value(t).unwrap();
    let deck: Vec<usize> = json["game"]["players"][seat]["deck"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_u64().unwrap() as usize)
        .collect();
    let card = |json: &Value, id: usize| -> Card {
        serde_json::from_value(json["game"]["cards"][id].clone()).unwrap()
    };
    let &top = deck.last()?;
    let &other = deck.iter().rev().find(|&&id| wanted(card(&json, id)))?;
    for path in [["game", "cards"], ["knowledge", "is"]] {
        json[path[0]][path[1]]
            .as_array_mut()
            .unwrap()
            .swap(top, other);
    }
    Some(serde_json::from_value(json).unwrap())
}

/// Throne Room on Jester with three players: whether Jester asks P1 again before Throne
/// Room replays it depends on P3's top card, which P1 can't see, so the hint on Jester's
/// question can't either. Two tables differ only in that card.
#[test]
fn the_way_hint_does_not_depend_on_cards_the_seat_cannot_see() {
    let estate = |card: Card| card == Card::Estate;
    for seed in 0..60 {
        let config = GameConfig {
            landscapes: vec![Card::WayOfTheOx],
            colonies: Some(false),
            shelters: Some(false),
            ..GameConfig::new(
                3,
                vec![
                    Card::ThroneRoom,
                    Card::Jester,
                    Card::Cellar,
                    Card::Market,
                    Card::Mine,
                    Card::Remodel,
                    Card::Smithy,
                    Card::Workshop,
                    Card::Festival,
                    Card::Laboratory,
                ],
                seed,
            )
        };
        let mut t = Table::new("t", config, vec![]).unwrap();
        let mut bought = [0; 2];
        for _ in 0..2000 {
            let Some(seat) = t.deciding() else { break };
            let d = t.view(seat).decision.unwrap();
            let end = pick(&d, |o| o.action == "end-phase");
            let plays = |card: &str| {
                pick(&d, |o| {
                    o.action == "play" && o.card.as_deref() == Some(card)
                })
            };
            if seat.0 == 0
                && d.phase.as_deref() == Some("action")
                && let (Some(throne), Some(_)) = (plays("Throne Room"), plays("Jester"))
            {
                // P2's top card is no Victory card, so Jester asks P1 about it.
                let Some(base) = with_top_card(&t, 1, |c| !estate(c)) else {
                    break;
                };
                let [Some(estate), Some(copper)] = [
                    with_top_card(&base, 2, estate),
                    with_top_card(&base, 2, |c| !estate(c)),
                ] else {
                    break;
                };
                let asked = |mut t: Table| {
                    t.answer_with(PlayerId(0), &throne, Some(WayChoice::None))
                        .unwrap();
                    let d = t.view(PlayerId(0)).decision.unwrap();
                    let jester = pick(&d, |o| o.card.as_deref() == Some("Jester")).unwrap();
                    t.answer_with(PlayerId(0), &jester, Some(WayChoice::None))
                        .unwrap();
                    serde_json::to_value(t.view(PlayerId(0))).unwrap()
                };
                let (estate, copper) = (asked(estate), asked(copper));
                let prompt = estate["decision"]["prompt"].as_str().unwrap();
                assert!(prompt.starts_with("Jester:"), "{prompt}");
                assert_eq!(estate, copper);
                return;
            }
            let answer = if d.kind != "turn" {
                first(&d)
            } else if seat.0 != 0 || d.phase.as_deref() == Some("action") {
                end.unwrap()
            } else {
                let buy = |i: usize, label: &str| {
                    pick(&d, |o| bought[i] < 2 && o.label.starts_with(label))
                };
                if let Some(play) = pick(&d, |o| o.action == "play") {
                    play
                } else if let Some(jester) = buy(0, "Buy Jester") {
                    bought[0] += 1;
                    jester
                } else if let Some(throne) = buy(1, "Buy Throne Room") {
                    bought[1] += 1;
                    throne
                } else {
                    end.unwrap()
                }
            };
            t.answer(seat, &answer).unwrap();
        }
    }
    panic!("no game had Throne Room and Jester in hand together");
}

/// The Treasures `seat`'s open Buy phase move offers to play.
fn treasures_offered(t: &Table, seat: PlayerId) -> Vec<CardId> {
    let g = t.game();
    match g.pending() {
        Some(d) if d.player == seat => match &d.kind {
            DecisionKind::Turn {
                phase: Phase::Buy,
                moves,
            } => (moves.iter())
                .filter_map(|m| match *m {
                    Move::Play(c) if g.is(c, Types::TREASURE) => Some(c),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        },
        _ => vec![],
    }
}

#[test]
fn play_treasures_plays_only_what_was_offered_when_asked() {
    let (mut runs, mut drawn_left) = (0, 0);
    for seed in 0..10 {
        let mut t = table(&[Card::Figurine], seed, &[]);
        for _ in 0..1000 {
            let Some(seat) = t.deciding() else { break };
            let d = t.view(seat).decision.unwrap();
            assert!(matches!(
                t.play_treasures(seat, "d0"),
                Err(AnswerError::StaleDecision { .. })
            ));
            let before = treasures_offered(&t, seat);
            if before.is_empty() {
                assert!(matches!(
                    t.play_treasures(seat, &d.id),
                    Err(AnswerError::Invalid(_))
                ));
            } else {
                runs += 1;
                let played = t.play_treasures(seat, &d.id).unwrap();
                assert_eq!(played as usize, before.len());
                let after = treasures_offered(&t, seat);
                assert!(after.iter().all(|c| !before.contains(c)), "{after:?}");
                drawn_left += after.len();
                let mut back = t.clone();
                assert!(back.undo(seat), "one undo takes the run back");
                assert_eq!(treasures_offered(&back, seat), before);
                continue;
            }
            let answer = if d.kind != "turn" {
                first(&d)
            } else if seat.0 != 0 {
                pick(&d, |o| o.action == "end-phase").unwrap()
            } else {
                pick(&d, |o| o.label.starts_with("Buy Figurine"))
                    .or_else(|| pick(&d, |o| o.label.starts_with("Buy Silver")))
                    .or_else(|| pick(&d, |o| o.action == "end-phase"))
                    .unwrap()
            };
            t.answer(seat, &answer).unwrap();
        }
    }
    assert!(
        runs > 0 && drawn_left > 0,
        "runs {runs}, drawn left {drawn_left}"
    );
}
