use dominion_agent::{Answer, AnswerError, Table};
use dominion_engine::{Card, GameConfig, Move, PlayerId, Response};
use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;

fn state(table: &Table) -> serde_json::Value {
    serde_json::to_value(table).unwrap()
}

#[test]
fn typed_and_agent_answers_share_knowledge_log_cursors_and_undo() {
    for seed in 0..4 {
        let mut config = GameConfig::new(
            2,
            vec![
                Card::Cellar,
                Card::Chapel,
                Card::Workshop,
                Card::Village,
                Card::Smithy,
                Card::Merchant,
                Card::CouncilRoom,
            ],
            seed,
        );
        config.landscapes = vec![Card::WayOfTheOx];
        config.turn_limit = Some(20);
        let mut agent = Table::new("shared", config, vec![]).unwrap();
        let mut typed = agent.clone();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut steps = 0;
        while let Some(seat) = agent.deciding() {
            let decision = agent.view(seat).decision.unwrap();
            let min = decision.min.unwrap_or(1) as usize;
            let max = decision.max.unwrap_or(1) as usize;
            let mut indices: Vec<_> = decision.options.iter().map(|o| o.index).collect();
            indices.shuffle(&mut rng);
            indices.truncate(rng.random_range(min..=max.min(indices.len())));
            let answer = Answer::indices(&decision.id, indices);
            let response = agent.resolve(seat, &answer).unwrap();
            agent.answer(seat, &answer).unwrap();
            typed.respond(seat, &answer.decision, response).unwrap();
            assert_eq!(state(&agent), state(&typed));
            for player in [PlayerId(0), PlayerId(1)] {
                assert_eq!(agent.view(player), typed.view(player));
                assert_eq!(agent.undo_note(player), typed.undo_note(player));
            }
            if steps % 13 == 0 {
                let mut undo_agent = agent.clone();
                let mut undo_typed = typed.clone();
                undo_agent.undo(seat).unwrap();
                undo_typed.undo(seat).unwrap();
                assert_eq!(
                    serde_json::to_value(undo_agent.game()).unwrap(),
                    serde_json::to_value(undo_typed.game()).unwrap()
                );
                assert_eq!(undo_agent.answers(), undo_typed.answers());
                assert_eq!(undo_agent.decision_id(), undo_typed.decision_id());
                for player in [PlayerId(0), PlayerId(1)] {
                    assert_eq!(undo_agent.view(player), undo_typed.view(player));
                    assert_eq!(undo_agent.undo_note(player), undo_typed.undo_note(player));
                }
            }
            steps += 1;
            assert!(steps < 3000);
        }
        assert!(steps > 20);
    }
}

#[test]
fn typed_rejections_leave_the_entire_table_unchanged() {
    let mut table =
        Table::new("shared", GameConfig::new(2, vec![Card::Village], 1), vec![]).unwrap();
    let seat = table.deciding().unwrap();
    let id = table.decision_id().unwrap();
    let before = state(&table);
    assert!(matches!(
        table.respond(PlayerId(1 - seat.0), &id, Response::Move(Move::EndPhase)),
        Err(AnswerError::NotYourDecision { .. })
    ));
    assert!(matches!(
        table.respond(seat, "old", Response::Move(Move::EndPhase)),
        Err(AnswerError::StaleDecision { .. })
    ));
    assert!(matches!(
        table.respond(seat, &id, Response::Cards(vec![])),
        Err(AnswerError::Invalid(_))
    ));
    assert_eq!(state(&table), before);
    table
        .respond(seat, &id, Response::Move(Move::EndPhase))
        .unwrap();
    let after = state(&table);
    assert!(
        table
            .respond(seat, &id, Response::Move(Move::EndPhase))
            .is_err()
    );
    assert_eq!(state(&table), after);
    assert!(table.can_undo(seat));
    table.undo(seat).unwrap();
    assert_ne!(table.decision_id().unwrap(), id);
}
