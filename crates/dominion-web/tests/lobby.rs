//! The table's rules: the deal and game requests they make, the kingdom text, and the
//! card selector's search, against the engine's card table.

mod common;

use common::card_db;
use dominion_protocol::api::Dealt;
use dominion_web::lobby::rules::{
    KINGDOM_SLOTS, LandscapeSlot, Pick, SeatKind, SeatSpec, TableRules, Ternary, expansions,
    search, selectable,
};

fn table() -> TableRules {
    let db = card_db();
    TableRules::new("Ivan", &expansions(&db))
}

#[test]
fn a_table_asks_for_its_rules_and_the_ban_list_only_while_it_respects_it() {
    let db = card_db();
    let mut t = table();
    t.sets = ["Base", "Seaside"].map(String::from).into();
    assert!(t.add(&db, "Witch"));
    assert!(t.add(&db, "Smithy"));
    assert!(t.add_from_set("Prosperity"));
    assert!(t.add(&db, "Delve"));
    t.slots = vec![LandscapeSlot {
        kinds: 1 | (1 << 3),
        empty: false,
    }];
    t.seed = " 42 ".into();
    let banned = vec!["Village".to_string(), "Moat".to_string()];
    let req = t.deal_request(&banned).unwrap();
    assert_eq!(req.players, 2);
    assert_eq!(req.sets, ["Base", "Seaside"]);
    assert_eq!(req.kingdom, ["Witch", "Smithy"]);
    assert_eq!(req.from_sets, ["Prosperity"]);
    assert_eq!(req.landscapes, ["Delve"]);
    let slots = req.slots.unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].kinds, ["event", "way"]);
    assert!(!slots[0].optional);
    assert_eq!(req.banned, banned);
    assert_eq!(req.seed, Some(42));

    t.respect_banned = false;
    assert!(t.deal_request(&banned).unwrap().banned.is_empty());
    t.sets.clear();
    assert!(t.deal_request(&banned).is_err());
    t.seed = "soon".into();
    assert!(t.deal_request(&banned).is_err());
}

#[test]
fn a_dealt_kingdom_starts_with_the_seats_in_order_and_the_tables_choices() {
    let mut t = table();
    t.seats.push(SeatSpec {
        name: "  ".into(),
        kind: SeatKind::HotSeat,
    });
    t.colonies = Ternary::Yes;
    t.shelters = Ternary::No;
    t.turn_limit = "30".into();
    let dealt = Dealt {
        kingdom: vec!["Witch".into(), "Smithy".into()],
        landscapes: vec!["Delve".into()],
        seed: 9,
    };
    let game = t.new_game(&dealt, &[2, 0, 1]).unwrap();
    assert_eq!(game.players, 3);
    assert_eq!(game.names, ["P3", "Ivan", "P2"]);
    assert_eq!(game.kingdom, dealt.kingdom);
    assert_eq!(game.landscapes, dealt.landscapes);
    assert!(game.sets.is_empty());
    assert_eq!(game.seed, Some(9));
    assert_eq!(game.turn_limit, Some(30));
    assert_eq!((game.colonies, game.shelters), (Some(true), Some(false)));

    t.seats[1].name = "ivan".into();
    assert!(t.new_game(&dealt, &[0, 1, 2]).is_err());
}

#[test]
fn seat_order_is_a_shuffle_only_when_the_table_randomizes_it() {
    let mut t = table();
    for i in 0..4 {
        t.seats.push(SeatSpec {
            name: format!("S{i}"),
            kind: SeatKind::Agent,
        });
    }
    let orders: std::collections::BTreeSet<Vec<usize>> = (0..50).map(|s| t.seat_order(s)).collect();
    assert!(orders.len() > 10);
    for o in &orders {
        let mut sorted = o.clone();
        sorted.sort();
        assert_eq!(sorted, (0..6).collect::<Vec<_>>());
    }
    t.randomize_order = false;
    assert_eq!(t.seat_order(5), (0..6).collect::<Vec<_>>());
}

#[test]
fn the_kingdom_text_is_dominion_games_and_typing_it_rebuilds_the_table() {
    let db = card_db();
    let mut t = table();
    for c in ["Witch", "Smithy", "Delve", "Cellar"] {
        assert!(t.add(&db, c));
    }
    t.colonies = Ternary::Yes;
    t.shelters = Ternary::No;
    let code = t.kingdom_code(&db);
    assert_eq!(code, "cellar, smithy, witch, delve, No Shelters, Colonies");

    let mut u = table();
    assert!(u.apply_text(&db, &code).is_empty());
    let mut a = t.required_piles();
    let mut b = u.required_piles();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(u.landscapes, ["Delve"]);
    assert_eq!((u.colonies, u.shelters), (Ternary::Yes, Ternary::No));
    assert_eq!(u.apply_text(&db, "throne room, nonsense"), ["nonsense"]);
    assert!(u.required_piles().contains(&"Throne Room"));
}

#[test]
fn the_selector_ranks_names_initials_and_prefixes_as_dominion_games() {
    let db = card_db();
    let all: Vec<_> = db.all().iter().filter(|c| selectable(c)).collect();
    let names = |q: &str| -> Vec<String> {
        search(&db, &all, q)
            .into_iter()
            .map(|c| c.name.clone())
            .collect()
    };
    assert_eq!(&names("wit")[..2], ["Witch", "Witch's Hut"]);
    assert_eq!(names("cotr")[0], "Coin of the Realm");
    assert!(names("tr").iter().take(5).any(|n| n == "Throne Room"));
    assert_eq!(names("mouse")[0], "Way of the Mouse");
    assert!(names("itc").is_empty(), "a part needs four letters");
    assert!(names("itch").contains(&"Witch".to_string()));
}

#[test]
fn a_full_kingdom_takes_no_more_piles_and_landscapes_stop_at_four() {
    let db = card_db();
    let mut t = table();
    for c in [
        "Cellar",
        "Chapel",
        "Moat",
        "Harbinger",
        "Merchant",
        "Vassal",
        "Village",
        "Workshop",
        "Bureaucrat",
    ] {
        assert!(t.add(&db, c));
    }
    assert!(t.add_from_set("Seaside"));
    assert_eq!(t.kingdom.len(), KINGDOM_SLOTS);
    assert!(!t.can_add(&db, "Smithy"));
    assert!(!t.add_from_set("Base"));
    assert!(!t.can_add(&db, "Cellar"));
    assert!(t.add(&db, "Delve"));
    assert!(t.add(&db, "Alms"));
    assert!(!t.can_add(&db, "Bonfire"), "two slots and two landscapes");
    t.slots.pop();
    assert!(t.add(&db, "Bonfire"));
    t.remove("Cellar");
    assert!(!t.kingdom.contains(&Pick::Card("Cellar".into())));
    assert!(t.can_add(&db, "Smithy"));
}
