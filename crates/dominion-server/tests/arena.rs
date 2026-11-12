//! Accounts, the lobby and finished games' records, driven through the `Arena`.

use dominion_agent::Answer;
use dominion_protocol::lobby::{
    Credentials, MatchRequest, NewTable, Opponents, QueueState, SeatEdit, SeatKind, Source,
    TableSpec, UserKind,
};
use dominion_server::arena::{Arena, Error, Ident, Settings};
use dominion_server::hub::{Hub, Seat, Timing};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

const IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

fn arena() -> Arc<Arena> {
    Arena::open(Arc::new(Hub::default()), Settings::default()).unwrap()
}

fn register(arena: &Arena, name: &str) -> Ident {
    let credentials = Credentials {
        name: name.into(),
        password: "correct horse".into(),
    };
    let (_, token) = arena.register(IP, &credentials, None).unwrap();
    arena.identify(&token.unwrap()).unwrap()
}

fn guest(arena: &Arena, name: &str) -> Ident {
    let (_, token) = arena.guest(IP, Some(name)).unwrap();
    arena.identify(&token).unwrap()
}

/// A table of `host`'s that `others` have joined, everyone ready: its game's id.
fn start_table(arena: &Arena, spec: TableSpec, host: &Ident, others: &[&Ident]) -> String {
    let seats = Some(vec![SeatEdit::Open; others.len()]);
    let table = arena.new_table(host, &NewTable { spec, seats }).unwrap();
    for other in others {
        arena.join_table(other, &table.id, None).unwrap();
        arena.ready(other, &table.id, true).unwrap();
    }
    let table = arena.ready(host, &table.id, true).unwrap();
    table.game.expect("the game started")
}

/// Plays `game` to its end with every decision's first option.
fn play_out(arena: &Arena, game: &str) {
    let hub = &arena.hub;
    for _ in 0..50_000 {
        let summary = hub.list().into_iter().find(|g| g.game_id == game).unwrap();
        let Some(deciding) = summary.deciding else {
            return;
        };
        let seat = Seat {
            game: game.into(),
            seat: dominion_engine::PlayerId(deciding),
        };
        let d = hub.view(&seat).unwrap().0.decision.unwrap();
        let min = d.min.unwrap_or(1).max(1) as usize;
        let picks = d.options.iter().take(min).map(|o| o.index).collect();
        hub.answer(&seat, &Answer::indices(&d.id, picks)).unwrap();
    }
    panic!("the game never ended");
}

fn my_seat(arena: &Arena, ident: &Ident, game: &str) -> Seat {
    let lobby = arena.lobby(Some(ident));
    let mine = lobby.games.iter().find(|g| g.game_id == game).unwrap();
    arena.hub.claim(game, &mine.token).unwrap()
}

#[test]
fn accounts_sign_in_with_their_password_and_names_are_one_persons() {
    let arena = arena();
    let ada = register(&arena, "Ada");
    assert!(!ada.agent);
    let me = arena.me(&ada).unwrap();
    assert_eq!(
        (me.user.name.as_str(), me.user.kind),
        ("Ada", UserKind::Registered)
    );
    assert_eq!(me.user.rating, None);
    let again = Credentials {
        name: "ada".into(),
        password: "another one".into(),
    };
    assert!(matches!(
        arena.register(IP, &again, None),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        arena.login(IP, &again),
        Err(Error::Unauthorized(_))
    ));
    let right = Credentials {
        name: " ADA ".into(),
        password: "correct horse".into(),
    };
    let (_, token) = arena.login(IP, &right).unwrap();
    assert_eq!(arena.identify(&token), Some(ada.clone()));
    let short = Credentials {
        name: "Bo".into(),
        password: "short".into(),
    };
    assert!(matches!(
        arena.register(IP, &short, None),
        Err(Error::BadRequest(_))
    ));
    arena.logout(&token);
    assert_eq!(arena.identify(&token), None);
}

#[test]
fn a_new_password_ends_the_other_sessions() {
    let arena = arena();
    let ada = register(&arena, "Ada");
    let login = Credentials {
        name: "Ada".into(),
        password: "correct horse".into(),
    };
    let (_, here) = arena.login(IP, &login).unwrap();
    let (_, there) = arena.login(IP, &login).unwrap();
    let wrong = arena.set_password(&ada, &here, "not it", "battery staple");
    assert!(matches!(wrong, Err(Error::Unauthorized(_))));
    arena
        .set_password(&ada, &here, "correct horse", "battery staple")
        .unwrap();
    assert!(arena.identify(&here).is_some());
    assert_eq!(arena.identify(&there), None);
    assert!(arena.login(IP, &login).is_err());
}

#[test]
fn a_guest_who_registers_keeps_their_account() {
    let arena = arena();
    let guest = guest(&arena, "Wanderer");
    assert_eq!(arena.me(&guest).unwrap().user.kind, UserKind::Guest);
    let credentials = Credentials {
        name: String::new(),
        password: "correct horse".into(),
    };
    let (me, token) = arena.register(IP, &credentials, Some(&guest)).unwrap();
    assert_eq!(token, None, "the guest's session goes on");
    assert_eq!(
        (me.user.name.as_str(), me.user.kind),
        ("Wanderer", UserKind::Registered)
    );
    let login = Credentials {
        name: "wanderer".into(),
        password: "correct horse".into(),
    };
    let (_, token) = arena.login(IP, &login).unwrap();
    assert_eq!(arena.identify(&token), Some(guest));
}

#[test]
fn closed_servers_take_no_accounts_or_guests() {
    let settings = Settings {
        registration: false,
        guests: false,
        ..Settings::default()
    };
    let arena = Arena::open(Arc::new(Hub::default()), settings).unwrap();
    let credentials = Credentials {
        name: "Ada".into(),
        password: "correct horse".into(),
    };
    assert!(matches!(
        arena.register(IP, &credentials, None),
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(arena.guest(IP, None), Err(Error::Forbidden(_))));
}

#[test]
fn one_address_makes_only_so_many_guests() {
    let arena = arena();
    let made = (0..40).filter(|_| arena.guest(IP, None).is_ok()).count();
    assert_eq!(made, 20);
    assert!(matches!(arena.guest(IP, None), Err(Error::TooMany(..))));
    let elsewhere: IpAddr = "10.0.0.7".parse().unwrap();
    assert!(arena.guest(elsewhere, None).is_ok());
}

#[test]
fn a_bot_signs_in_with_its_key_until_the_key_is_replaced() {
    let arena = arena();
    let ada = register(&arena, "Ada");
    let made = arena.new_bot(&ada, "Adabot").unwrap();
    assert_eq!(made.bot.kind, UserKind::Bot);
    assert_eq!(made.bot.owner.as_deref(), Some("Ada"));
    let bot = arena.identify(&made.key).unwrap();
    assert!(bot.agent);
    assert_eq!(arena.me(&ada).unwrap().bots.len(), 1);
    let replaced = arena.new_bot_key(&ada, "adabot").unwrap();
    assert_eq!(arena.identify(&made.key), None);
    assert_eq!(arena.identify(&replaced.key), Some(bot));
    let wanderer = guest(&arena, "Wanderer");
    assert!(matches!(
        arena.new_bot(&wanderer, "Wanderbot"),
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(
        arena.new_bot_key(&wanderer, "Adabot"),
        Err(Error::Forbidden(_))
    ));
    arena.delete_bot(&ada, "Adabot").unwrap();
    assert_eq!(arena.identify(&replaced.key), None);
}

#[test]
fn the_banned_list_is_each_accounts_own() {
    let settings = Settings {
        banned_cards: vec!["Possession".into()],
        ..Settings::default()
    };
    let arena = Arena::open(Arc::new(Hub::default()), settings).unwrap();
    let (ada, bo) = (register(&arena, "Ada"), register(&arena, "Bo"));
    assert_eq!(arena.banned_cards(Some(&ada)), ["Possession"]);
    let names = ["witch".to_string(), "Witch".into(), "Delve".into()];
    assert_eq!(
        arena.set_banned_cards(&ada, &names).unwrap(),
        ["Witch", "Delve"]
    );
    assert_eq!(arena.banned_cards(Some(&ada)), ["Witch", "Delve"]);
    assert_eq!(arena.banned_cards(Some(&bo)), ["Possession"]);
    assert_eq!(arena.banned_cards(None), ["Possession"]);
    assert!(arena.set_banned_cards(&ada, &["Copper".into()]).is_err());
}

#[test]
fn a_table_starts_its_game_once_every_seat_is_taken_and_ready() {
    let arena = arena();
    let (ada, bo, cy) = (
        register(&arena, "Ada"),
        register(&arena, "Bo"),
        register(&arena, "Cy"),
    );
    let table = arena.new_table(&ada, &NewTable::default()).unwrap();
    assert_eq!(table.host, "Ada");
    assert_eq!(table.seats.len(), 2);
    assert_eq!(table.seats[1].kind, SeatKind::Open);
    // Ready alone starts nothing: a seat is open.
    assert_eq!(arena.ready(&ada, &table.id, true).unwrap().game, None);
    let joined = arena.join_table(&bo, &table.id, None).unwrap();
    assert_eq!(joined.seats[1].user.as_ref().unwrap().name, "Bo");
    assert!(matches!(
        arena.join_table(&cy, &table.id, None),
        Err(Error::Conflict(_))
    ));
    let rules = TableSpec {
        kingdom: vec!["Village".into()],
        ..TableSpec::default()
    };
    assert!(matches!(
        arena.set_spec(&bo, &table.id, &rules),
        Err(Error::Forbidden(_))
    ));
    // New rules: nobody is ready for them yet.
    let changed = arena.set_spec(&ada, &table.id, &rules).unwrap();
    assert!(changed.seats.iter().all(|s| !s.ready));
    let none = TableSpec {
        kingdom: vec!["Copper".into()],
        ..TableSpec::default()
    };
    assert!(arena.set_spec(&ada, &table.id, &none).is_err());
    arena.ready(&ada, &table.id, true).unwrap();
    let started = arena.ready(&bo, &table.id, true).unwrap();
    let game = started.game.clone().expect("the game started");
    let summary = arena
        .hub
        .list()
        .into_iter()
        .find(|g| g.game_id == game)
        .unwrap();
    assert_eq!(summary.source, Source::Table);
    assert!(summary.rated && summary.watchable);
    assert!(summary.kingdom.contains(&"Village".to_string()));
    let mut names = summary.names.clone();
    names.sort();
    assert_eq!(names, ["Ada", "Bo"]);
    let users: Vec<&str> = (summary.users.iter())
        .map(|u| u.as_ref().unwrap().name.as_str())
        .collect();
    assert_eq!(users, summary.names);
    // Each account is handed its own seat's token, and nobody else's.
    for (ident, name) in [(&ada, "Ada"), (&bo, "Bo")] {
        let lobby = arena.lobby(Some(ident));
        assert_eq!(lobby.table.as_ref().unwrap().id, table.id);
        let [mine] = lobby.games.as_slice() else {
            panic!("{:?}", lobby.games);
        };
        assert_eq!(
            (mine.game_id.as_str(), mine.name.as_str()),
            (game.as_str(), name)
        );
        let seat = arena.hub.claim(&game, &mine.token).unwrap();
        assert_eq!(arena.hub.seat_name(&seat).unwrap(), name);
    }
    assert!(arena.lobby(Some(&cy)).games.is_empty());
    assert!(arena.lobby(None).games.is_empty());
    assert!(matches!(
        arena.set_spec(&ada, &table.id, &rules),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn the_host_hands_out_the_tokens_of_token_seats() {
    let arena = arena();
    let (ada, bo) = (register(&arena, "Ada"), register(&arena, "Bo"));
    let new = NewTable {
        spec: TableSpec::default(),
        seats: Some(vec![SeatEdit::Token("Robo".into()), SeatEdit::Open]),
    };
    let table = arena.new_table(&ada, &new).unwrap();
    arena.join_table(&bo, &table.id, None).unwrap();
    arena.ready(&bo, &table.id, true).unwrap();
    let game = arena.ready(&ada, &table.id, true).unwrap().game.unwrap();
    let summary = arena
        .hub
        .list()
        .into_iter()
        .find(|g| g.game_id == game)
        .unwrap();
    assert!(
        !summary.rated,
        "a seat without an account plays for no rating"
    );
    let tokens = arena.lobby(Some(&ada)).table.unwrap().tokens;
    let [robo] = tokens.as_slice() else {
        panic!("{tokens:?}");
    };
    assert_eq!(robo.name, "Robo");
    let seat = arena.hub.claim(&game, &robo.token).unwrap();
    assert_eq!(arena.hub.seat_name(&seat).unwrap(), "Robo");
    assert!(arena.lobby(Some(&bo)).table.unwrap().tokens.is_empty());
}

#[test]
fn the_host_arranges_the_seats_and_the_next_player_hosts_when_they_leave() {
    let arena = arena();
    let (ada, bo, cy) = (
        register(&arena, "Ada"),
        register(&arena, "Bo"),
        register(&arena, "Cy"),
    );
    let table = arena.new_table(&ada, &NewTable::default()).unwrap();
    arena.join_table(&bo, &table.id, None).unwrap();
    let seats = [
        SeatEdit::Open,
        SeatEdit::Player("Ada".into()),
        SeatEdit::Token("Robo".into()),
    ];
    // Bo is left out: removed from the table.
    let arranged = arena.set_seats(&ada, &table.id, &seats).unwrap();
    let kinds: Vec<SeatKind> = arranged.seats.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [SeatKind::Open, SeatKind::Player, SeatKind::Token]);
    assert_eq!(arena.lobby(Some(&bo)).table, None);
    assert!(
        arena
            .set_seats(&ada, &table.id, &[SeatEdit::Open, SeatEdit::Open])
            .is_err()
    );
    assert!(matches!(
        arena.set_seats(&bo, &table.id, &seats),
        Err(Error::Forbidden(_))
    ));
    arena.join_table(&cy, &table.id, Some(0)).unwrap();
    arena.leave_table(&ada);
    assert_eq!(arena.table(&table.id).unwrap().host, "Cy");
    // Sitting down elsewhere leaves this table, which goes with its last player.
    arena.new_table(&cy, &NewTable::default()).unwrap();
    assert!(matches!(arena.table(&table.id), Err(Error::NotFound(_))));
}

#[test]
fn automatch_pairs_searches_that_fit_each_other() {
    let arena = arena();
    let (ada, bo) = (register(&arena, "Ada"), register(&arena, "Bo"));
    let bot = {
        let key = arena.new_bot(&ada, "Adabot").unwrap().key;
        arena.identify(&key).unwrap()
    };
    let humans = MatchRequest {
        opponents: Opponents::Humans,
        ..MatchRequest::default()
    };
    let status = arena.search(&ada, &humans).unwrap();
    assert_eq!(status.state, QueueState::Searching);
    // A bot is no human, and plays its owner for nothing anyway.
    let waiting = arena.search(&bot, &MatchRequest::default()).unwrap();
    assert_eq!(waiting.state, QueueState::Searching);
    let lobby = arena.lobby(Some(&bo));
    assert_eq!((lobby.searching.humans, lobby.searching.agents), (1, 1));
    let found = arena.search(&bo, &humans).unwrap();
    assert_eq!(found.state, QueueState::Matched);
    let game = found.game_id.unwrap();
    let ada_status = arena.queue_status(&ada).unwrap();
    assert_eq!(ada_status.game_id.as_deref(), Some(game.as_str()));
    assert_eq!(
        arena.queue_status(&bot).unwrap().state,
        QueueState::Searching
    );
    let summary = arena
        .hub
        .list()
        .into_iter()
        .find(|g| g.game_id == game)
        .unwrap();
    assert_eq!(summary.source, Source::Automatch);
    assert!(summary.rated);
    for ident in [&ada, &bo] {
        assert_eq!(arena.lobby(Some(ident)).games.len(), 1);
    }
    // In a timed game: no second search.
    assert!(matches!(
        arena.search(&ada, &humans),
        Err(Error::Conflict(_))
    ));
    arena.cancel_search(&ada);
    assert_eq!(arena.queue_status(&ada), None);
}

#[test]
fn a_rated_game_moves_ratings_and_is_recorded() {
    let arena = arena();
    let (ada, bo) = (register(&arena, "Ada"), register(&arena, "Bo"));
    arena.search(&ada, &MatchRequest::default()).unwrap();
    let game = arena
        .search(&bo, &MatchRequest::default())
        .unwrap()
        .game_id
        .unwrap();
    arena
        .hub
        .resign(&my_seat(&arena, &bo, &game), "resigned")
        .unwrap();
    let (winner, loser) = (arena.me(&ada).unwrap(), arena.me(&bo).unwrap());
    assert!(winner.rating.rating > 1500.0 && loser.rating.rating < 1500.0);
    assert!(winner.rating.deviation < 350.0);
    assert_eq!((winner.stats.games, winner.stats.wins), (1, 1));
    assert_eq!((loser.stats.games, loser.stats.wins), (1, 0));
    assert_eq!(
        winner.user.rating,
        Some(winner.rating.rating.round() as i32)
    );
    let history = arena.history(Some("bo"), None, 10).unwrap();
    let [record] = history.games.as_slice() else {
        panic!("{history:?}");
    };
    assert_eq!(
        (record.game_id.as_str(), record.reason.as_str()),
        (game.as_str(), "resigned")
    );
    assert!(record.rated);
    let bo_row = record.players.iter().find(|p| p.name == "Bo").unwrap();
    assert!(bo_row.resigned && bo_row.rank == 2);
    let (before, after) = bo_row.rating.unwrap();
    assert_eq!((before, after), (1500.0, loser.rating.rating));
    let board = arena.leaderboard(10).unwrap();
    let names: Vec<&str> = board.players.iter().map(|p| p.user.name.as_str()).collect();
    assert_eq!(names, ["Ada", "Bo"]);
    assert_eq!(arena.profile("ADA").unwrap().recent.len(), 1);
    assert!(arena.history(Some("Nobody"), None, 10).is_err());
    // Free to search again.
    assert!(arena.search(&ada, &MatchRequest::default()).is_ok());
}

#[test]
fn a_guests_game_is_recorded_and_moves_no_rating() {
    let arena = arena();
    let (ada, wanderer) = (register(&arena, "Ada"), guest(&arena, "Wanderer"));
    let spec = TableSpec {
        turn_limit: Some(2),
        ..TableSpec::default()
    };
    let game = start_table(&arena, spec, &ada, &[&wanderer]);
    play_out(&arena, &game);
    let me = arena.me(&ada).unwrap();
    assert_eq!(
        (me.rating.games, me.rating.rating, me.stats.games),
        (0, 1500.0, 1)
    );
    let record = &arena.history(None, None, 10).unwrap().games[0];
    assert_eq!(
        (record.rated, record.reason.as_str()),
        (false, "turn-limit")
    );
    assert!(record.players.iter().all(|p| p.rating.is_none()));
    assert!(arena.leaderboard(10).unwrap().players.is_empty());
    // The table is free for another game.
    let table = arena.lobby(Some(&ada)).table.unwrap();
    let info = arena.table(&table.id).unwrap();
    assert_eq!((info.game, info.last_game), (None, Some(game)));
    assert!(info.seats.iter().all(|s| !s.ready));
}

#[test]
fn a_player_who_takes_too_long_in_a_timed_game_forfeits() {
    let settings = Settings {
        timing: Timing {
            away: Duration::ZERO,
            decision: Duration::from_millis(20),
        },
        ..Settings::default()
    };
    let arena = Arena::open(Arc::new(Hub::default()), settings).unwrap();
    let (ada, bo) = (register(&arena, "Ada"), register(&arena, "Bo"));
    let untimed = TableSpec {
        timed: false,
        ..TableSpec::default()
    };
    let slow = start_table(&arena, untimed, &ada, &[&bo]);
    std::thread::sleep(Duration::from_millis(40));
    arena.sweep();
    let running = |game: &str| {
        let games = arena.hub.list();
        games
            .into_iter()
            .find(|g| g.game_id == game)
            .unwrap()
            .running
    };
    assert!(
        running(&slow),
        "an untimed game waits for as long as it takes"
    );
    arena
        .hub
        .resign(&my_seat(&arena, &ada, &slow), "resigned")
        .unwrap();

    let timed = start_table(&arena, TableSpec::default(), &ada, &[&bo]);
    let view = arena
        .hub
        .seat_view(&my_seat(&arena, &ada, &timed), None)
        .unwrap();
    assert!(view.meta.timed && view.meta.forfeit_at.is_some());
    let deciding = view.deciding.unwrap();
    std::thread::sleep(Duration::from_millis(40));
    arena.sweep();
    assert!(!running(&timed));
    let record = &arena.history(None, None, 1).unwrap().games[0];
    assert_eq!(record.reason, "abandoned");
    let quitter = record.players.iter().find(|p| p.resigned).unwrap();
    assert_eq!(quitter.seat, deciding);
}

#[test]
fn chat_is_for_the_lobby_and_for_those_at_the_table() {
    let arena = arena();
    let (ada, bo) = (register(&arena, "Ada"), register(&arena, "Bo"));
    arena.say(&ada, "lobby", "  anyone for\na game? ").unwrap();
    let log = arena.chat_log("lobby", 0);
    assert_eq!(log.messages.len(), 1);
    assert_eq!(
        (log.messages[0].from.as_str(), log.messages[0].seq),
        ("Ada", 1)
    );
    assert_eq!(log.messages[0].text, "anyone for a game?");
    assert!(arena.chat_log("lobby", 1).messages.is_empty());
    assert!(matches!(
        arena.say(&ada, "lobby", "   "),
        Err(Error::BadRequest(_))
    ));
    let table = arena.new_table(&ada, &NewTable::default()).unwrap();
    let channel = format!("table-{}", table.id);
    assert!(matches!(
        arena.say(&bo, &channel, "hello"),
        Err(Error::Forbidden(_))
    ));
    arena.join_table(&bo, &table.id, None).unwrap();
    arena.say(&bo, &channel, "hello").unwrap();
    let texts: Vec<String> = (arena.chat_log(&channel, 0).messages.into_iter())
        .map(|m| m.text)
        .collect();
    assert_eq!(texts, ["Bo sat down.", "hello"]);
    // A burst is cut off.
    let said = (0..20)
        .filter(|_| arena.say(&ada, "lobby", "spam").is_ok())
        .count();
    assert_eq!(said, 7);
}

#[test]
fn accounts_sessions_and_records_outlive_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let open = || {
        let (hub, _) = Hub::open(dir.path(), Duration::from_secs(3600)).unwrap();
        Arena::open(Arc::new(hub), Settings::default()).unwrap()
    };
    let (token, game) = {
        let arena = open();
        let credentials = Credentials {
            name: "Ada".into(),
            password: "correct horse".into(),
        };
        let (_, token) = arena.register(IP, &credentials, None).unwrap();
        let ada = arena.identify(token.as_deref().unwrap()).unwrap();
        let bo = register(&arena, "Bo");
        arena.search(&ada, &MatchRequest::default()).unwrap();
        let game = arena
            .search(&bo, &MatchRequest::default())
            .unwrap()
            .game_id
            .unwrap();
        arena
            .hub
            .resign(&my_seat(&arena, &bo, &game), "resigned")
            .unwrap();
        (token.unwrap(), game)
    };
    // A server that stopped before noting the record written finds it there.
    let file = dir.path().join(format!("{game}.json"));
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains(r#""recorded":true"#));
    std::fs::write(
        &file,
        saved.replace(r#""recorded":true"#, r#""recorded":false"#),
    )
    .unwrap();
    let arena = open();
    let ada = arena.identify(&token).expect("the session is still good");
    let me = arena.me(&ada).unwrap();
    assert_eq!(me.user.name, "Ada");
    assert!(me.rating.rating > 1500.0);
    assert_eq!((me.rating.games, me.stats.games, me.stats.wins), (1, 1, 1));
    let history = arena.history(Some("Ada"), None, 10).unwrap();
    assert_eq!(
        history.games.len(),
        1,
        "recorded once, not again on reopening"
    );
    assert_eq!(history.games[0].game_id, game);
    let login = Credentials {
        name: "Bo".into(),
        password: "correct horse".into(),
    };
    let (_, bo) = arena.login(IP, &login).unwrap();
    let bo = arena.identify(&bo).unwrap();
    // The seat is still Bo's while the finished game is kept.
    assert_eq!(arena.lobby(Some(&bo)).games[0].game_id, game);
    let wanderer = guest(&arena, "Wanderer");
    assert_ne!(wanderer.user, ada.user);
    assert_ne!(wanderer.user, bo.user);
}
