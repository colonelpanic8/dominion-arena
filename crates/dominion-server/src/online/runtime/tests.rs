use super::*;
use dominion_online::{
    questions::{Question, QuestionDescription, QuestionKind, Story},
    transport::{LoginSuccess, NamedId},
    wire::{Reader, Writer},
};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog {
        cards: vec![],
        enums: BTreeMap::new(),
        labels: BTreeMap::new(),
    })
}
fn named(id: i32) -> NamedId {
    NamedId {
        id,
        name: format!("fixture-{id}"),
    }
}
fn table() -> TableDetails {
    TableDetails {
        table_id: 10,
        host_id: 42,
        players: vec![named(42)],
        ready: vec![false],
        bots: vec![named(-2)],
        spectators: vec![],
        reconnecting: vec![],
        rules: vec![
            (
                TableRule {
                    id: 5,
                    value: RuleValue::Group(Group::Nobody),
                },
                false,
            ),
            (
                TableRule {
                    id: 19,
                    value: RuleValue::Boolean(false),
                },
                false,
            ),
        ],
        division: String::new(),
        match_games: vec![],
        match_score: None,
        tournament: None,
    }
}
fn runtime() -> (TempDir, Runtime) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let runtime = Runtime {
        socket: None,
        needs_snapshot: false,
        request_game: false,
        resuming: false,
        endpoint: Endpoint::Alpha,
        session: None,
        table: Some(table()),
        bots: vec![-2],
        update: Update {
            reducer: Reducer::new(catalog(), 42),
            error: None,
            connected: true,
        },
        store,
    };
    (directory, runtime)
}
fn login(reconnecting: bool) -> LoginSuccess {
    let mut w = Writer::new();
    w.int(42);
    w.string("fixture-account");
    w.boolean(reconnecting);
    w.int(0);
    w.int(0);
    w.string("private-fixture-authority");
    w.int(0);
    for _ in 0..4 {
        w.int(0);
    }
    w.int(0);
    w.int(0);
    w.int(0);
    let bytes = w.finish();
    let mut reader = Reader::new(&bytes);
    let login = LoginSuccess::parse(&mut reader).unwrap();
    reader.finish().unwrap();
    login
}
fn snapshot() -> model::Snapshot {
    let player = |player_id| model::Player {
        player_id,
        card_frequencies: vec![],
        reminders: vec![],
        bought_projects: vec![],
    };
    model::Snapshot {
        reconnect: false,
        game_id: 123,
        state: model::GameState {
            players: vec![player(42), player(-2)],
            card_names: vec![],
            pile_names: vec![],
            card_counts: vec![],
            cards: vec![],
            zones: vec![],
            pile_markers: vec![],
            tokens: vec![],
            counters: vec![],
            turn: model::TurnDescription {
                owner_id: 0,
                turn_number: 1,
                turn_type: 0,
                controller_id: 0,
            },
            cost_reduction: model::CostReduction {
                generic: 0,
                by_type: vec![],
                by_card: vec![],
                cost_override: vec![],
            },
            temporary_effects: model::TemporaryEffects {
                renames: vec![],
                behaviour_changes: vec![],
                image_changes: vec![],
                card_type_changes: vec![],
            },
            associations: vec![],
            card_statuses: vec![],
        },
        autoplay_settings: vec![],
        table_options: model::TableOptions {
            players_see_spectator_chat: false,
            timer_settings: model::TimerSettings {
                settings: vec![],
                force_resign: false,
                preset: 4,
            },
        },
        events: vec![],
    }
}
fn question(index: i32) -> Question {
    let story = Story {
        question_id: 0,
        arguments: vec![],
        context: None,
    };
    Question {
        index,
        description: QuestionDescription {
            question_type: 11,
            association: -1,
            story: story.clone(),
            short_story: story,
        },
        kind: QuestionKind::Number {
            min: 0,
            max: 2,
            default: 0,
            decline: -1,
        },
    }
}
fn decision_id(runtime: &Runtime) -> String {
    runtime.update.reducer.view().unwrap().decision.unwrap().id
}
fn active_runtime() -> (TempDir, Runtime) {
    let (directory, mut runtime) = runtime();
    runtime.process(Message::Login(login(false))).unwrap();
    runtime.process(Message::Snapshot(snapshot())).unwrap();
    runtime.process(Message::Question(question(7))).unwrap();
    (directory, runtime)
}

#[test]
fn no_human_seats_or_silently_ignored_counts() {
    for (players, bots) in [(1, 0), (2, 0), (3, 1), (7, 6)] {
        assert!(
            NewGame {
                players: Some(players),
                bots: Some(bots),
                ..Default::default()
            }
            .rules(&catalog())
            .is_err()
        );
    }
    assert_eq!(NewGame::default().rules(&catalog()).unwrap().0, 1);
}
#[test]
fn table_gate_rejects_humans_visitors_reconnecting_players_and_foreign_host() {
    let (_directory, mut runtime) = runtime();
    assert!(runtime.safe_table().is_ok());
    let good = table();
    let mut bad = Vec::new();
    let mut human = good.clone();
    human.players.push(named(99));
    bad.push(human);
    let mut spectator = good.clone();
    spectator.spectators.push(named(99));
    bad.push(spectator);
    let mut reconnecting = good.clone();
    reconnecting.reconnecting.push(named(99));
    bad.push(reconnecting);
    let mut foreign_host = good.clone();
    foreign_host.host_id = 99;
    bad.push(foreign_host);
    let mut wrong_account = good.clone();
    wrong_account.players[0] = named(99);
    bad.push(wrong_account);
    for table in bad {
        runtime.table = Some(table);
        assert!(runtime.safe_table().unwrap_err().starts_with("REFUSED:"));
    }
}
#[test]
fn table_gate_requires_confirmed_unrated_and_no_spectator_rules() {
    let (_directory, mut runtime) = runtime();
    for id in [5, 19] {
        let mut missing = table();
        missing.rules.retain(|(r, _)| r.id != id);
        runtime.table = Some(missing);
        assert!(runtime.safe_table().is_err());
    }
    let mut rated = table();
    rated
        .rules
        .iter_mut()
        .find(|(r, _)| r.id == 19)
        .unwrap()
        .0
        .value = RuleValue::Boolean(true);
    runtime.table = Some(rated);
    assert!(runtime.safe_table().is_err());
    let mut observable = table();
    observable
        .rules
        .iter_mut()
        .find(|(r, _)| r.id == 5)
        .unwrap()
        .0
        .value = RuleValue::Group(Group::Everybody);
    runtime.table = Some(observable);
    assert!(runtime.safe_table().is_err());
}
#[test]
fn conflicting_duplicate_privacy_rules_do_not_prove_a_safe_table() {
    let (_directory, mut runtime) = runtime();
    runtime.table.as_mut().unwrap().rules.push((
        TableRule {
            id: 19,
            value: RuleValue::Boolean(true),
        },
        true,
    ));
    assert!(runtime.safe_table().is_err());
}
#[test]
fn initial_login_reporting_an_existing_game_does_not_acquire_authority() {
    let (directory, mut runtime) = runtime();
    let error = runtime.process(Message::Login(login(true))).unwrap_err();
    assert!(error.contains("no automatic takeover or reconnect"));
    assert!(runtime.session.is_none());
    assert!(runtime.socket.is_none());
    assert!(!directory.path().join("session.json").exists());
    assert!(runtime.update.reducer.game_id.is_none());
}
#[test]
fn persisted_login_metadata_does_not_contain_the_session_authority() {
    let (directory, mut runtime) = runtime();
    runtime.process(Message::Login(login(false))).unwrap();
    let metadata = std::fs::read_to_string(directory.path().join("session.json")).unwrap();
    assert!(metadata.contains("fixture-account"));
    assert!(!metadata.contains("private-fixture-authority"));
    assert!(runtime.session.is_some());
}
#[tokio::test]
async fn an_old_decision_cannot_consume_the_replacement_question() {
    let (_directory, mut runtime) = active_runtime();
    let stale = decision_id(&runtime);
    runtime.process(Message::Question(question(8))).unwrap();
    let current = decision_id(&runtime);
    assert_ne!(stale, current);
    let error = runtime
        .answer(Answer::indices(stale, vec![1]), None)
        .await
        .unwrap_err();
    assert!(error.contains("is not open"), "{error}");
    assert_eq!(decision_id(&runtime), current);
    assert!(runtime.update.error.is_none());
}
#[tokio::test]
async fn a_failed_send_does_not_leave_an_answer_available_for_replay() {
    let (_directory, mut runtime) = active_runtime();
    let answer = Answer::indices(decision_id(&runtime), vec![1]);
    let error = runtime.answer(answer.clone(), None).await.unwrap_err();
    assert!(error.contains("connection closed"), "{error}");
    assert!(runtime.update.reducer.view().unwrap().decision.is_none());
    let duplicate = runtime.answer(answer, None).await.unwrap_err();
    assert!(
        duplicate.contains("no open official decision"),
        "{duplicate}"
    );
}
#[tokio::test]
async fn snapshot_replacement_invalidates_an_answer_from_the_previous_epoch() {
    let (_directory, mut runtime) = active_runtime();
    let old_answer = Answer::indices(decision_id(&runtime), vec![1]);
    runtime.process(Message::Snapshot(snapshot())).unwrap();
    runtime.process(Message::Question(question(7))).unwrap();
    let current = decision_id(&runtime);
    let error = runtime.answer(old_answer, None).await.unwrap_err();
    assert!(error.contains("is not open"), "{error}");
    assert_eq!(decision_id(&runtime), current);
}

#[tokio::test]
async fn reconnect_barrier_does_not_consume_a_question_before_fresh_snapshot() {
    let (_directory, mut runtime) = active_runtime();
    let id = decision_id(&runtime);
    runtime.needs_snapshot = true;
    let error = runtime
        .answer(Answer::indices(id.clone(), vec![1]), None)
        .await
        .unwrap_err();
    assert!(error.contains("reconnect snapshot"), "{error}");
    assert_eq!(decision_id(&runtime), id);
}

#[tokio::test]
async fn changed_table_membership_blocks_an_existing_answer() {
    let (_directory, mut runtime) = active_runtime();
    let id = decision_id(&runtime);
    runtime.table.as_mut().unwrap().players.push(named(99));
    let error = runtime
        .answer(Answer::indices(id.clone(), vec![1]), None)
        .await
        .unwrap_err();
    assert!(error.starts_with("REFUSED:"), "{error}");
    assert_eq!(decision_id(&runtime), id);
}

#[tokio::test]
async fn receive_failure_closes_and_invalidates_before_a_later_action() {
    let (_directory, mut runtime) = active_runtime();
    let answer = Answer::indices(decision_id(&runtime), vec![1]);
    let error = runtime.receive().await.unwrap_err();
    assert!(error.contains("connection closed"), "{error}");
    assert!(!runtime.update.connected);
    assert!(runtime.update.error.is_some());
    assert!(runtime.update.reducer.view().unwrap().decision.is_none());
    assert!(runtime.answer(answer, None).await.is_err());
}

#[test]
fn automatic_reconnect_waits_for_the_server_without_requesting_a_second_snapshot() {
    let (_directory, mut runtime) = active_runtime();
    runtime.process(Message::Login(login(true))).unwrap();
    assert!(runtime.needs_snapshot);
    assert!(!runtime.request_game);
}

#[test]
fn interrupted_reconnect_requests_one_game_and_requires_owned_authority() {
    let (_directory, mut runtime) = runtime();
    let mut interrupted = login(false);
    interrupted.previous_reconnects = 1;
    assert!(
        runtime
            .process(Message::Login(interrupted.clone()))
            .is_err()
    );
    assert!(runtime.session.is_none());
    runtime.resuming = true;
    runtime.process(Message::Login(interrupted)).unwrap();
    assert!(runtime.needs_snapshot);
    assert!(runtime.request_game);
}

#[test]
fn leaving_own_finished_table_releases_membership_without_erasing_the_result() {
    let (_directory, mut runtime) = active_runtime();
    runtime.update.reducer.running = false;
    runtime
        .process(Message::PlayerLeft {
            table_id: 10,
            player_id: 42,
        })
        .unwrap();
    assert!(runtime.table.is_none());
    assert!(runtime.bots.is_empty());
    assert_eq!(runtime.update.reducer.game_id, Some(123));
}
#[tokio::test]
async fn new_game_does_not_leave_an_existing_unfinished_table() {
    let (_directory, mut runtime) = runtime();
    let error = runtime.new_game(NewGame::default()).await.unwrap_err();
    assert!(error.contains("unfinished"), "{error}");
    assert!(runtime.table.is_some());
}

#[test]
fn decision_log_replacement_does_not_erase_the_next_question() {
    let (_directory, mut runtime) = active_runtime();
    let id = decision_id(&runtime);
    runtime.update.reducer.logs.push(LogEntry {
        seq: 5,
        kind: "decision".into(),
        player: Some(1),
        card: None,
        text: "fixture".into(),
    });
    runtime
        .process(Message::Log(model::LogChange {
            start_index: 5,
            entries: vec![model::LogEntry::Decision {
                decision_index: 1,
                player: 1,
                decision: vec![0],
                autoplayed: false,
            }],
        }))
        .unwrap();
    assert_eq!(decision_id(&runtime), id);
    assert_eq!(runtime.update.reducer.logs.len(), 1);
}

#[tokio::test]
async fn resign_consumes_the_open_decision_even_if_transmission_fails() {
    let (_directory, mut runtime) = active_runtime();
    assert!(runtime.resign().await.is_err());
    assert!(runtime.update.reducer.view().unwrap().decision.is_none());
}
