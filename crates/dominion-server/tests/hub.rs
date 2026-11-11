use dominion_engine::PlayerId;
use dominion_server::hub::{Hub, HubError, NewGame, Seat};

fn two_players() -> NewGame {
    NewGame {
        players: 2,
        kingdom: vec!["Village".into()],
        seed: Some(1),
        ..NewGame::default()
    }
}

#[test]
fn an_engine_panic_stays_in_its_game() {
    let hub = Hub::default();
    let broken = hub.create(&two_players()).unwrap();
    let healthy = hub.create(&two_players()).unwrap();
    // A seat the game doesn't have makes the engine panic, as a bug in it would.
    let nobody = Seat {
        game: broken.game_id.clone(),
        seat: PlayerId(9),
    };
    let failed = hub.view(&nobody).map(|_| ());
    assert!(matches!(failed, Err(HubError::Engine(_))), "{failed:?}");

    assert_eq!(hub.list().len(), 2);
    for game in [&broken, &healthy] {
        let seat = hub.find(&game.seats[0].token).unwrap();
        assert_eq!(
            hub.claim(&game.game_id, &game.seats[0].token).unwrap(),
            seat
        );
        let (view, _) = hub.view(&seat).unwrap();
        assert!(view.decision.is_some());
    }
}

#[test]
fn a_token_claims_its_seat_in_its_own_game_only() {
    let hub = Hub::default();
    let first = hub.create(&two_players()).unwrap();
    let second = hub.create(&two_players()).unwrap();
    let token = &second.seats[1].token;
    let seat = hub.find(token).unwrap();
    assert_eq!((seat.game.as_str(), seat.seat.0), ("g2", 1));
    assert!(matches!(
        hub.claim(&first.game_id, token),
        Err(HubError::BadToken(_))
    ));
    assert!(matches!(
        hub.claim("g9", token),
        Err(HubError::UnknownGame(_))
    ));
    assert!(hub.find("guess").is_err());
}
