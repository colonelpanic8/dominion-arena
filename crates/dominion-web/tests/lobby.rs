//! The table's rules to and from the server's, the deal they ask for, the seats as the
//! host changes them, the Tables tab's lines, the kingdom text and the card selector's
//! search, against the engine's card table.

mod common;

use common::card_db;
use dominion_protocol::api::{DealSlot, GameSummary};
use dominion_protocol::lobby::{
    Lobby, Me, MyTable, RatingInfo, Searching, SeatEdit, SeatKind, Source, Stats, TableInfo,
    TableSeat, TableSpec, UserRef,
};
use dominion_web::lobby::rules::{
    KINGDOM_SLOTS, LandscapeSlot, Pick, TableRules, TableStatus, Ternary, expansions,
    free_seat_name, search, seat_edits, selectable, waiting_for,
};
use dominion_web::lobby::tables::{Action, SeatMark, listing};

fn sets() -> Vec<String> {
    expansions(&card_db())
}

fn table() -> TableRules {
    TableRules::new(&sets())
}

#[test]
fn a_tables_rules_are_the_servers_spec_and_back() {
    let db = card_db();
    let all = sets();
    // A new table's rules say what the server's default does, slots spelled out.
    let fresh = table();
    let spec = fresh.spec(&all).unwrap();
    let two = vec![
        DealSlot {
            kinds: ["event", "landmark", "project", "way", "trait"]
                .map(String::from)
                .to_vec(),
            optional: true,
        },
        DealSlot {
            kinds: ["event", "landmark", "project", "trait"]
                .map(String::from)
                .to_vec(),
            optional: true,
        },
    ];
    assert_eq!(
        spec,
        TableSpec {
            slots: Some(two),
            ..TableSpec::default()
        }
    );
    assert_eq!(TableRules::from_spec(&spec, &all), fresh);

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
    t.colonies = Ternary::Yes;
    t.shelters = Ternary::No;
    t.turn_limit = " 30 ".into();
    t.randomize_order = false;
    t.respect_banned = false;
    t.rated = false;
    t.timed = false;
    let spec = t.spec(&all).unwrap();
    assert_eq!(spec.sets, ["Base", "Seaside"]);
    assert_eq!(spec.kingdom, ["Witch", "Smithy"]);
    assert_eq!(spec.from_sets, ["Prosperity"]);
    assert_eq!(spec.landscapes, ["Delve"]);
    let slots = spec.slots.clone().unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].kinds, ["event", "way"]);
    assert!(!slots[0].optional);
    assert_eq!((spec.colonies, spec.shelters), (Some(true), Some(false)));
    assert_eq!(spec.turn_limit, Some(30));
    assert!(!spec.randomize_order && !spec.respect_banned && !spec.rated && !spec.timed);
    assert!(spec.watchable);
    let back = TableRules::from_spec(&spec, &all);
    t.turn_limit = "30".into();
    assert_eq!(back, t);
    assert_eq!(back.spec(&all).unwrap(), spec);

    t.turn_limit = "soon".into();
    assert!(t.spec(&all).is_err());
    t.turn_limit.clear();
    t.sets.clear();
    assert!(t.spec(&all).is_err(), "random cards need an expansion");
}

#[test]
fn a_deal_asks_for_the_rules_and_the_ban_list_only_while_they_respect_it() {
    let db = card_db();
    let all = sets();
    let mut t = table();
    assert!(t.add(&db, "Witch"));
    let banned = vec!["Village".to_string(), "Moat".to_string()];
    let req = t.deal_request(&all, 3, &banned).unwrap();
    assert_eq!(req.players, 3);
    assert_eq!(req.kingdom, ["Witch"]);
    assert_eq!(req.banned, banned);
    assert_eq!(req.seed, None);
    t.respect_banned = false;
    assert!(t.deal_request(&all, 3, &banned).unwrap().banned.is_empty());
}

fn user(name: &str) -> UserRef {
    UserRef {
        name: name.into(),
        ..UserRef::default()
    }
}

fn player(name: &str, ready: bool) -> TableSeat {
    TableSeat {
        kind: SeatKind::Player,
        user: Some(user(name)),
        ready,
        here: true,
        ..TableSeat::default()
    }
}

fn token(name: &str) -> TableSeat {
    TableSeat {
        kind: SeatKind::Token,
        name: Some(name.into()),
        ready: true,
        ..TableSeat::default()
    }
}

fn info(id: &str, seats: Vec<TableSeat>) -> TableInfo {
    TableInfo {
        id: id.into(),
        host: (seats.iter())
            .find_map(|s| s.user.as_ref().map(|u| u.name.clone()))
            .unwrap_or_default(),
        spec: TableSpec::default(),
        seats,
        game: None,
        last_game: None,
    }
}

#[test]
fn the_host_sends_the_seats_back_and_names_new_ones_apart() {
    let seats = vec![
        player("Ada", true),
        TableSeat::default(),
        token("Agent 1"),
        player("agent 2", false),
        TableSeat {
            kind: SeatKind::Agent,
            ..player("Claude Opus", true)
        },
    ];
    assert_eq!(
        seat_edits(&seats),
        [
            SeatEdit::Player("Ada".into()),
            SeatEdit::Open,
            SeatEdit::Token("Agent 1".into()),
            SeatEdit::Player("agent 2".into()),
            SeatEdit::Agent("Claude Opus".into()),
        ]
    );
    assert_eq!(free_seat_name(&seats, "Agent"), "Agent 3");
    assert_eq!(free_seat_name(&seats, "Player"), "Player 1");
}

#[test]
fn a_table_says_what_it_waits_for() {
    let mut t = info(
        "t1",
        vec![player("Ada", false), TableSeat::default(), token("Agent 1")],
    );
    assert_eq!(TableStatus::of(&t), TableStatus::New);
    assert!(waiting_for(&t, "Ada").unwrap().contains("empty slot"));
    t.seats[1] = player("Bob", false);
    assert_eq!(
        waiting_for(&t, "Ada").unwrap(),
        "The game starts when everyone is ready."
    );
    t.seats[1].ready = true;
    assert_eq!(
        waiting_for(&t, "Ada").unwrap(),
        "The game starts when you are ready."
    );
    assert_eq!(
        waiting_for(&t, "Bob").unwrap(),
        "Waiting for Ada to be ready."
    );
    t.seats[0].ready = true;
    assert_eq!(waiting_for(&t, "Ada"), None);
    t.game = Some("g1".into());
    assert_eq!(TableStatus::of(&t), TableStatus::Running);
    assert_eq!(waiting_for(&t, "Ada"), None);
    t.last_game = t.game.take();
    assert_eq!(TableStatus::of(&t), TableStatus::PostGame);
}

fn game(id: &str, names: &[&str], running: bool, watchable: bool) -> GameSummary {
    GameSummary {
        game_id: id.into(),
        names: names.iter().map(|n| n.to_string()).collect(),
        running,
        deciding: None,
        joined: vec![],
        kingdom: vec![],
        landscapes: vec![],
        source: Source::Automatch,
        rated: false,
        watchable,
        users: names.iter().map(|n| Some(user(n))).collect(),
    }
}

#[test]
fn the_tables_tab_offers_what_each_line_allows() {
    let mut mine = info("t1", vec![player("Ada", false), TableSeat::default()]);
    mine.last_game = Some("g0".into());
    let open = info("t2", vec![player("Bob", false), TableSeat::default()]);
    let full = info("t3", vec![player("Cy", true), token("Agent 1")]);
    let mut playing = info("t4", vec![player("Di", false), player("Ed", false)]);
    playing.game = Some("g4".into());
    let mut hidden = info("t5", vec![player("Flo", false), player("Gus", false)]);
    hidden.game = Some("g5".into());
    let lobby = Lobby {
        version: 1,
        you: Some(Me {
            user: user("Ada"),
            rating: RatingInfo {
                rating: 1500.0,
                deviation: 350.0,
                games: 0,
            },
            stats: Stats { games: 0, wins: 0 },
            bots: vec![],
        }),
        online: 1,
        searching: Searching::default(),
        tables: vec![mine, open, full, playing, hidden],
        table: Some(MyTable {
            id: "t1".into(),
            tokens: vec![],
        }),
        queue: None,
        games: vec![],
        house: Default::default(),
    };
    let games = [
        game("g0", &["Ada", "Zed"], false, true),
        game("g4", &["Di", "Ed"], true, true),
        game("g5", &["Flo", "Gus"], true, false),
        game("g6", &["Hal", "Ada"], true, true),
        game("g7", &["Ida", "Jo"], true, true),
        game("g8", &["Kim", "Lu"], false, true),
        game("g9", &["Mo", "Ada"], false, true),
    ];
    let held = |g: &str| ["g0", "g6", "g9"].contains(&g);
    let lines = listing(&lobby, &games, &held);
    let by = |key: &str| {
        (lines.iter())
            .find(|l| l.table.as_deref() == Some(key) || l.game.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no line for {key}"))
    };
    assert_eq!(by("t1").action, Action::Open);
    assert_eq!(by("t1").seats, [SeatMark::You, SeatMark::Open]);
    assert_eq!(by("t1").status, TableStatus::PostGame);
    assert_eq!(by("t2").action, Action::Join);
    assert_eq!(by("t3").action, Action::None, "no open seat");
    assert_eq!(
        by("t3").seats,
        [
            SeatMark::Player("Cy".into()),
            SeatMark::Token("Agent 1".into())
        ]
    );
    assert_eq!(by("t4").action, Action::Watch);
    assert_eq!(by("t5").action, Action::None, "not watchable");
    assert_eq!(by("g6").action, Action::Play);
    assert_eq!(
        by("g6").seats,
        [SeatMark::Player("Hal".into()), SeatMark::You]
    );
    assert_eq!(by("g7").action, Action::Watch);
    assert_eq!(by("g9").status, TableStatus::PostGame);
    // A table's game is the table's line, and other people's finished games aren't listed.
    let listed = |key: &str| {
        lines
            .iter()
            .filter(|l| l.game.as_deref() == Some(key))
            .count()
    };
    assert_eq!((listed("g0"), listed("g4"), listed("g8")), (0, 1, 0));
    assert_eq!(lines.len(), 8);
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
