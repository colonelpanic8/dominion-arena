use dominion_engine::{Card, PlayerId};
use dominion_server::hub::{self, Hub, HubError, NewGame, Seat};

fn two_players() -> NewGame {
    NewGame {
        players: 2,
        kingdom: vec!["Village".into()],
        seed: Some(1),
        ..NewGame::default()
    }
}

fn refusal(req: NewGame) -> String {
    match Hub::default().create(&req) {
        Err(HubError::BadRequest(e)) => e,
        Err(e) => panic!("{req:?} failed with {e}"),
        Ok(_) => panic!("{req:?} was accepted"),
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
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

#[test]
fn create_checks_options_it_does_not_use() {
    let unknown_landscape = refusal(NewGame {
        players: 2,
        landscapes: strings(&["No Such Landscape"]),
        ..NewGame::default()
    });
    assert!(unknown_landscape.contains("No Such Landscape"));
    let unknown_set = refusal(NewGame {
        sets: strings(&["No Such Set"]),
        ..two_players()
    });
    assert!(unknown_set.contains("No Such Set"), "{unknown_set}");
    let both = refusal(NewGame {
        sets: strings(&["Seaside"]),
        ..two_players()
    });
    assert!(both.contains("not both"), "{both}");
}

#[test]
fn a_random_kingdom_takes_the_landscapes_asked_for() {
    let config = hub::config(&NewGame {
        players: 2,
        sets: strings(&["dark ages", "Cornucopia & Guilds"]),
        landscapes: strings(&["Alms", "Way of the Mouse"]),
        seed: Some(7),
        ..NewGame::default()
    })
    .unwrap();
    assert_eq!(config.landscapes, [Card::Alms, Card::WayOfTheMouse]);
    assert_eq!(config.kingdom.len(), 10);
}

#[test]
fn a_kingdom_is_only_dealt_from_sets_that_have_one() {
    for name in ["Arcana", "Basee"] {
        let refused = refusal(NewGame {
            players: 2,
            sets: strings(&[name]),
            ..NewGame::default()
        });
        assert!(refused.contains(name), "{refused}");
        assert!(
            refused.contains("Base, Intrigue, Seaside") && !refused.contains("Arcana,"),
            "the sets to choose from are missing: {refused}"
        );
    }
    let all = hub::config(&NewGame {
        players: 2,
        ..NewGame::default()
    })
    .unwrap();
    assert_eq!(all.kingdom.len(), 10);
}

#[test]
fn seat_names_are_single_lines_that_tell_seats_apart() {
    let named = |names: &[&str]| NewGame {
        names: strings(names),
        ..two_players()
    };
    for names in [
        &["Ada", "ada"][..],
        &["Ada", " Ada "],
        &["P2"],
        &[""],
        &["Ada", "  "],
        &["Ada\ng9: Bo (over)"],
        &["Ada\u{1b}[2J"],
        &["Ada\u{2028}Bo"],
        &["An exceedingly long name for a seat"],
        &["Ada", "Bo", "Cy"],
    ] {
        refusal(named(names));
    }
    let created = Hub::default().create(&named(&[" Ada "])).unwrap();
    let names: Vec<&str> = created.seats.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Ada", "P2"]);
    let renamed = Hub::default().create(&named(&["P2", "P1"])).unwrap();
    assert_eq!(renamed.seats[0].name, "P2");
}
