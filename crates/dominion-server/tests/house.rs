//! The server's own agents: seated at tables and in searches of agents, within the
//! runner's room and each player's day, with the runner told the seats they hold.

use dominion_protocol::lobby::{
    Credentials, MatchRequest, NewTable, Opponents, QueueState, SeatEdit, SeatKind, TableSpec,
    UserKind,
};
use dominion_server::arena::{Arena, Error, Ident, Settings};
use dominion_server::house::{Job, Offer, ToRunner};
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
