//! The server's own agents: seated at tables and in searches of agents, within the
//! runner's room and each player's day, with the runner told the seats they hold.

use dominion_protocol::lobby::{
    Credentials, HistoryQuery, MatchRequest, NewTable, Opponents, PlayedBy, QueueState, SeatAgent,
    SeatEdit, SeatKind, TableSpec, UserKind,
};
use dominion_server::arena::{Arena, Error, Ident, Settings};
use dominion_server::house::{Job, Offer, OfferedModel, ToRunner};
use dominion_server::hub::Hub;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

const IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

fn arena(per_day: u32) -> Arc<Arena> {
    let settings = Settings {
        agent_seats_per_day: per_day,
        ..Settings::default()
    };
    Arena::open(Arc::new(Hub::default()), settings).unwrap()
}

fn register(arena: &Arena, name: &str) -> Ident {
    let credentials = Credentials {
        name: name.into(),
        password: "correct horse".into(),
    };
    let (_, token) = arena.register(IP, &credentials, None).unwrap();
    arena.identify(&token.unwrap()).unwrap()
}

fn runner(arena: &Arena, names: &[&str], capacity: u32) -> UnboundedReceiver<ToRunner> {
    let (send, told) = unbounded_channel();
    let offers = (names.iter())
        .map(|n| Offer {
            name: n.to_string(),
            about: "Claude Code (opus)".into(),
            ..Default::default()
        })
        .collect();
    arena.runner_connected(offers, capacity, send);
    told
}

/// The last list of jobs the runner was told, if it was told anything since.
fn jobs(told: &mut UnboundedReceiver<ToRunner>) -> Option<Vec<Job>> {
    let mut last = None;
    while let Ok(message) = told.try_recv() {
        if let ToRunner::Jobs { jobs } = message {
            last = Some(jobs);
        }
    }
    last
}

fn with_agent(agent: &str) -> NewTable {
    NewTable {
        spec: TableSpec::default(),
        seats: Some(vec![SeatEdit::Agent(agent.into())]),
    }
}

/// Ends `game`: its first seat resigns.
fn end(arena: &Arena, game: &str) {
    let seat = dominion_server::hub::Seat {
        game: game.into(),
        seat: dominion_engine::PlayerId(0),
    };
    arena.hub.resign(&seat, "resigned").unwrap();
}

#[test]
fn a_table_seats_an_agent_and_the_runner_is_told_its_seat_until_the_game_ends() {
    let arena = arena(5);
    let mut told = runner(&arena, &["Claude Opus"], 2);
    assert_eq!(jobs(&mut told), Some(vec![]));
    let ada = register(&arena, "Ada");
    let house = arena.lobby(Some(&ada)).house;
    assert_eq!(
        (house.agents.len(), house.free, house.left),
        (1, 2, Some(5))
    );
    assert_eq!(house.agents[0].user.kind, UserKind::Bot);

    let (_, token) = arena.guest(IP, Some("Gus")).unwrap();
    let gus = arena.identify(&token).unwrap();
    assert!(matches!(
        arena.new_table(&gus, &with_agent("Claude Opus")),
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(
        arena.new_table(&ada, &with_agent("Clippy")),
        Err(Error::BadRequest(_))
    ));

    let table = arena.new_table(&ada, &with_agent("claude opus")).unwrap();
    let seat = &table.seats[1];
    assert_eq!(seat.kind, SeatKind::Agent);
    assert_eq!(seat.user.as_ref().unwrap().name, "Claude Opus");
    assert!(seat.ready && seat.here);
    let game = arena.ready(&ada, &table.id, true).unwrap().game.unwrap();

    let jobs_now = jobs(&mut told).unwrap();
    let [job] = jobs_now.as_slice() else {
        panic!("{jobs_now:?}");
    };
    assert_eq!(
        (job.agent.as_str(), job.game_id.as_str()),
        ("Claude Opus", game.as_str())
    );
    let claimed = arena.hub.claim(&game, &job.token).unwrap();
    assert_eq!(arena.hub.seat_name(&claimed).unwrap(), "Claude Opus");
    let house = arena.lobby(Some(&ada)).house;
    assert_eq!(
        (house.free, house.left, house.agents[0].playing),
        (1, Some(4), 1)
    );
    assert!(
        arena
            .hub
            .list()
            .iter()
            .any(|g| g.game_id == game && g.rated)
    );

    end(&arena, &game);
    assert_eq!(jobs(&mut told), Some(vec![]));
    let profile = arena.profile("Claude Opus").unwrap();
    assert_eq!(profile.stats.games, 1);
}

#[test]
fn agents_play_only_as_many_games_as_the_runner_has_room_for_and_players_have_seats_left() {
    let arena = arena(1);
    let mut told = runner(&arena, &["Claude Opus", "Codex"], 1);
    let (ada, bo) = (register(&arena, "Ada"), register(&arena, "Bo"));
    let first = arena.new_table(&ada, &with_agent("Claude Opus")).unwrap();
    let game = arena.ready(&ada, &first.id, true).unwrap().game.unwrap();
    let second = arena.new_table(&bo, &with_agent("Codex")).unwrap();
    assert!(matches!(
        arena.ready(&bo, &second.id, true),
        Err(Error::Conflict(_))
    ));
    assert_eq!(arena.table(&second.id).unwrap().game, None);
    end(&arena, &game);
    jobs(&mut told);
    let game = arena.ready(&bo, &second.id, true).unwrap().game.unwrap();
    end(&arena, &game);
    // Ada filled her one seat of the day.
    arena.leave_table(&ada);
    let third = arena.new_table(&ada, &with_agent("Codex")).unwrap();
    assert!(matches!(
        arena.ready(&ada, &third.id, true),
        Err(Error::TooMany(..))
    ));
}

#[test]
fn a_search_of_agents_by_a_registered_player_gets_the_agent_it_asks_for() {
    let arena = arena(5);
    let mut told = runner(&arena, &["Claude Opus", "Codex"], 2);
    let ada = register(&arena, "Ada");
    let codex = MatchRequest {
        opponents: Opponents::Agents,
        agent: Some("codex".into()),
        ..MatchRequest::default()
    };
    let found = arena.search(&ada, &codex).unwrap();
    assert_eq!(found.state, QueueState::Matched);
    let game = found.game_id.unwrap();
    let jobs_now = jobs(&mut told).unwrap();
    assert_eq!(jobs_now.len(), 1);
    assert_eq!(
        (jobs_now[0].agent.as_str(), jobs_now[0].game_id.as_str()),
        ("Codex", game.as_str())
    );

    // A guest waits for agents that come by themselves.
    let (_, token) = arena.guest(IP, Some("Gus")).unwrap();
    let gus = arena.identify(&token).unwrap();
    let any_agent = MatchRequest {
        opponents: Opponents::Agents,
        ..MatchRequest::default()
    };
    let waiting = arena.search(&gus, &any_agent).unwrap();
    assert_eq!(waiting.state, QueueState::Searching);
    assert!(matches!(
        arena.search(
            &gus,
            &MatchRequest {
                agent: Some("Clippy".into()),
                ..any_agent
            }
        ),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn a_runner_that_comes_back_is_told_the_seats_its_agents_still_hold() {
    let arena = arena(5);
    let mut first = runner(&arena, &["Claude Opus"], 2);
    let ada = register(&arena, "Ada");
    let table = arena.new_table(&ada, &with_agent("Claude Opus")).unwrap();
    let game = arena.ready(&ada, &table.id, true).unwrap().game.unwrap();
    assert_eq!(jobs(&mut first).unwrap().len(), 1);
    // The newer runner takes the place of the older one.
    let mut second = runner(&arena, &["Claude Opus"], 2);
    let jobs_now = jobs(&mut second).unwrap();
    assert_eq!(jobs_now[0].game_id, game);
    assert!(first.try_recv().is_err());
}

#[test]
fn an_agent_named_like_someone_elses_account_is_refused() {
    let arena = arena(5);
    register(&arena, "Codex");
    let mut told = runner(&arena, &["Codex", "Claude Opus"], 2);
    let mut refused = vec![];
    while let Ok(message) = told.try_recv() {
        if let ToRunner::Refused { agent, .. } = message {
            refused.push(agent);
        }
    }
    assert_eq!(refused, ["Codex"]);
    let agents = arena.lobby(None).house.agents;
    let names: Vec<&str> = agents.iter().map(|a| a.user.name.as_str()).collect();
    assert_eq!(names, ["Claude Opus"]);
}

#[test]
fn no_daily_limit_when_seats_per_day_is_zero() {
    let arena = arena(0);
    let mut told = runner(&arena, &["Claude Opus"], 1);
    let ada = register(&arena, "Ada");
    assert_eq!(
        arena.lobby(Some(&ada)).house.left,
        Some(dominion_protocol::lobby::House::UNLIMITED)
    );
    for _ in 0..3 {
        let table = arena.new_table(&ada, &with_agent("Claude Opus")).unwrap();
        let game = arena.ready(&ada, &table.id, true).unwrap().game.unwrap();
        end(&arena, &game);
        jobs(&mut told);
        arena.leave_table(&ada);
    }
}

#[test]
fn an_agent_offered_on_demand_has_an_account_once_seated_and_after_its_runner_returns() {
    let arena = arena(5);
    let offer = |name: &str, effort: &str| Offer {
        name: name.into(),
        about: format!("Claude Code, claude-opus-5-5, {effort} effort"),
        model: Some(OfferedModel {
            program: "Claude Code".into(),
            id: "claude-opus-5-5".into(),
            name: "Opus 5.5".into(),
            effort: Some(effort.into()),
        }),
        on_demand: name != "Opus High",
    };
    let offers = vec![offer("Opus High", "high"), offer("Opus 5.5 Max", "max")];
    let (send, mut told) = unbounded_channel();
    let runner = arena.runner_connected(offers.clone(), 2, send);
    let ada = register(&arena, "Ada");
    let house = arena.lobby(Some(&ada)).house;
    let names: Vec<_> = house.agents.iter().map(|a| a.user.name.as_str()).collect();
    assert_eq!(names, ["Opus High"]);
    let [model] = house.models.as_slice() else {
        panic!("{:?}", house.models);
    };
    let choices: Vec<_> = (model.choices.iter())
        .map(|c| (c.effort.as_deref(), c.agent.as_str()))
        .collect();
    assert_eq!(
        (model.name.as_str(), choices),
        (
            "Opus 5.5",
            vec![(Some("high"), "Opus High"), (Some("max"), "Opus 5.5 Max")]
        )
    );
    assert!(arena.profile("Opus 5.5 Max").is_err());

    let table = arena.new_table(&ada, &with_agent("Opus 5.5 Max")).unwrap();
    assert_eq!(table.seats[1].user.as_ref().unwrap().name, "Opus 5.5 Max");
    let game = arena.ready(&ada, &table.id, true).unwrap().game.unwrap();
    let job = |told: &mut UnboundedReceiver<ToRunner>| {
        let jobs = jobs(told).unwrap();
        jobs.iter()
            .map(|j| (j.agent.clone(), j.game_id.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(job(&mut told), [("Opus 5.5 Max".into(), game.clone())]);

    // Offered again, it plays the game it holds a seat in.
    arena.runner_gone(runner);
    let (send, mut told) = unbounded_channel();
    let runner = arena.runner_connected(offers, 2, send);
    assert_eq!(job(&mut told), [("Opus 5.5 Max".into(), game.clone())]);

    // No longer offered, it can't be seated, but plays out the game it holds a seat in.
    arena.offers_changed(runner, vec![offer("Opus High", "high")]);
    assert_eq!(jobs(&mut told), None);
    let house = arena.lobby(Some(&ada)).house;
    assert_eq!((house.models[0].choices.len(), house.free), (1, 1));
    assert!(matches!(
        arena.new_table(&ada, &with_agent("Opus 5.5 Max")),
        Err(Error::BadRequest(_))
    ));
    end(&arena, &game);
    assert_eq!(job(&mut told), []);

    // Its record says what it was, though no longer offered when the game ended.
    let max = SeatAgent {
        agent: "Opus 5.5 Max".into(),
        program: Some("Claude Code".into()),
        model: Some("claude-opus-5-5".into()),
        name: Some("Opus 5.5".into()),
        effort: Some("max".into()),
    };
    let query = HistoryQuery {
        q: Some("claude-opus".into()),
        ..HistoryQuery::default()
    };
    let games = arena.history(&query).unwrap().games;
    let seat = |name: &str| {
        let player = games[0].players.iter().find(|p| p.name == name).unwrap();
        player.played_by.clone()
    };
    assert_eq!(seat("Ada"), PlayedBy::Person);
    assert_eq!(seat("Opus 5.5 Max"), PlayedBy::Agent(max.clone()));
    assert_eq!(arena.profile("Opus 5.5 Max").unwrap().agent, Some(max));
    assert_eq!(arena.profile("Ada").unwrap().agent, None);
}

#[test]
fn a_record_kept_with_only_an_agents_name_shows_what_it_is_offered_as_now() {
    let arena = arena(5);
    let (send, _told) = unbounded_channel();
    let bare = Offer {
        name: "Sol High".into(),
        about: "OpenAI".into(),
        ..Default::default()
    };
    let runner = arena.runner_connected(vec![bare], 1, send);
    let ada = register(&arena, "Ada");
    let table = arena.new_table(&ada, &with_agent("Sol High")).unwrap();
    let game = arena.ready(&ada, &table.id, true).unwrap().game.unwrap();
    end(&arena, &game);
    let sol = || {
        let games = arena.history(&HistoryQuery::default()).unwrap().games;
        let player = games[0].players.iter().find(|p| p.name == "Sol High");
        player.unwrap().played_by.clone()
    };
    assert_eq!(
        sol(),
        PlayedBy::Agent(SeatAgent {
            agent: "Sol High".into(),
            ..SeatAgent::default()
        })
    );

    let offered = Offer {
        name: "Sol High".into(),
        about: "OpenAI, gpt-6.2-sol, high effort".into(),
        model: Some(OfferedModel {
            program: "OpenAI".into(),
            id: "gpt-6.2-sol".into(),
            name: "GPT-6.2-Sol".into(),
            effort: Some("high".into()),
        }),
        on_demand: false,
    };
    arena.offers_changed(runner, vec![offered]);
    let full = SeatAgent {
        agent: "Sol High".into(),
        program: Some("OpenAI".into()),
        model: Some("gpt-6.2-sol".into()),
        name: Some("GPT-6.2-Sol".into()),
        effort: Some("high".into()),
    };
    assert_eq!(sol(), PlayedBy::Agent(full.clone()));
    assert_eq!(arena.profile("Sol High").unwrap().agent, Some(full));
}

#[test]
fn a_search_for_any_agent_gets_one_when_none_has_an_account_yet() {
    let arena = arena(5);
    let offer = |name: &str, effort: &str| Offer {
        name: name.into(),
        about: "Claude Code".into(),
        model: Some(OfferedModel {
            program: "Claude Code".into(),
            id: "claude-opus-5-5".into(),
            name: "Opus 5.5".into(),
            effort: Some(effort.into()),
        }),
        on_demand: true,
    };
    let (send, mut told) = unbounded_channel();
    arena.runner_connected(
        vec![offer("Opus 5.5 Low", "low"), offer("Opus 5.5 High", "high")],
        2,
        send,
    );
    let ada = register(&arena, "Ada");
    let request = MatchRequest {
        players: vec![2],
        opponents: Opponents::Agents,
        ..MatchRequest::default()
    };
    arena.search(&ada, &request).unwrap();
    let jobs = jobs(&mut told).unwrap();
    let agents: Vec<_> = jobs.iter().map(|j| j.agent.as_str()).collect();
    assert_eq!(agents, ["Opus 5.5 High"]);
}

fn setup(agent: &str, version: &str) -> dominion_protocol::lobby::AgentSetup {
    dominion_protocol::lobby::AgentSetup {
        agent: agent.into(),
        program: "Claude Code".into(),
        model: Some("claude-opus-5-5".into()),
        model_name: None,
        effort: Some("high".into()),
        version: Some(version.into()),
        playbook: Some(dominion_protocol::lobby::Playbook {
            path: "/playbook".into(),
            commit: Some("abc123".into()),
            dirty: false,
            digest: "d1".into(),
        }),
    }
}

fn agent_game(rated: bool, agents: &[Option<&str>]) -> dominion_server::hub::NewGame {
    dominion_server::hub::NewGame {
        players: agents.len() as u8,
        agents: agents.iter().map(|a| a.map(String::from)).collect(),
        seed: Some(7),
        rated: Some(rated),
        ..Default::default()
    }
}

#[test]
fn a_rated_game_has_a_different_server_agent_in_every_seat() {
    let arena = arena(0);
    let _told = runner(&arena, &["Opus", "Sol"], 4);
    let ada = register(&arena, "Ada");
    for agents in [
        &[Some("Opus"), None][..],
        &[Some("Opus"), Some("Opus")],
        &[None, None],
    ] {
        assert!(matches!(
            arena.create_game(Some(&ada), &agent_game(true, agents)),
            Err(Error::BadRequest(_))
        ));
    }
    let game = (arena.create_game(Some(&ada), &agent_game(true, &[Some("Opus"), Some("Sol")])))
        .unwrap()
        .game_id;
    let meta = arena.hub.meta(&game).unwrap();
    assert!(meta.rated);
    assert_eq!(meta.undo, dominion_protocol::lobby::UndoRule::Off);
}

#[test]
fn a_record_names_what_played_each_agent_seat_and_why_an_attempt_stopped() {
    let arena = arena(0);
    let _told = runner(&arena, &["Opus", "Sol"], 4);
    let ada = register(&arena, "Ada");
    let mixed = (arena.create_game(Some(&ada), &agent_game(false, &[Some("Opus"), None])))
        .unwrap()
        .game_id;
    assert!(
        (arena.hub)
            .agent_attempt(&mixed, 1, "x".into(), setup("Opus", "v1"))
            .is_err()
    );

    let game = (arena.create_game(Some(&ada), &agent_game(true, &[Some("Opus"), Some("Sol")])))
        .unwrap()
        .game_id;
    let hub = &arena.hub;
    hub.agent_attempt(&game, 0, "a1".into(), setup("Opus", "v1"))
        .unwrap();
    hub.agent_attempt(&game, 1, "b1".into(), setup("Sol", "v1"))
        .unwrap();
    hub.agent_failed(&game, 0, "a1", "429 rate limited")
        .unwrap();
    hub.agent_attempt(&game, 0, "a2".into(), setup("Opus", "v2"))
        .unwrap();
    end(&arena, &game);

    let rated = dominion_protocol::lobby::HistoryQuery {
        rated: true,
        ..Default::default()
    };
    let history = arena.history(&rated).unwrap();
    let record = history.games.iter().find(|r| r.id == game).unwrap();
    assert!(record.rated);
    assert_eq!(
        record.arranged,
        Some(dominion_protocol::lobby::Arranged::Agents)
    );
    assert_eq!(record.seed, Some(7));
    let opus = record.players.iter().find(|p| p.seat == 0).unwrap();
    assert!(opus.rating.is_some());
    let run = opus.agent.as_ref().unwrap();
    assert_eq!(run.setup, setup("Opus", "v1"));
    let attempts: Vec<_> = (run.attempts.iter())
        .map(|a| (a.archive.as_str(), a.error.as_deref(), a.setup.as_ref()))
        .collect();
    assert_eq!(
        attempts,
        [
            ("a1", Some("429 rate limited"), None),
            ("a2", None, Some(&setup("Opus", "v2"))),
        ]
    );
    let sol = record.players.iter().find(|p| p.seat == 1).unwrap();
    assert_eq!(sol.agent.as_ref().unwrap().attempts.len(), 1);
}
