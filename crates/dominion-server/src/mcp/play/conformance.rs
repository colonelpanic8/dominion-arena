//! What a seat keeps whichever backend has its game, checked on each through the tools.

use super::{LogParams, Play, PlaySeat, Presentation, StepParams, TurnsParams};
use crate::hub::{Hub, PlayObservation, Seat};
use crate::mcp::ArenaSeat;
use dominion_agent::{Answer, GlossaryEntry, LogEntry};
use dominion_protocol::api::NewGame;
use rmcp::model::{CallToolResult, ContentBlock};
use std::sync::Mutex;
use std::time::Duration;

fn text(result: &CallToolResult) -> String {
    let lines = result.content.iter().filter_map(|c| match c {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    });
    lines.collect::<Vec<_>>().join("\n")
}

fn failed(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}

fn open(seat: &impl PlaySeat) -> Option<String> {
    let (_, observation) = seat.observe().unwrap().unwrap();
    observation.view.decision.map(|d| d.id)
}

/// A seat with a decision open, on a log of at least two turns.
async fn keeps_the_contract(seat: &impl PlaySeat) {
    let shown = Mutex::new(Presentation::default());
    let play = Play {
        seat,
        shown: &shown,
        clock_cues: true,
    };
    let state = play.observation(true);
    assert!(!failed(&state), "{}", text(&state));
    assert!(text(&state).starts_with("STATUS game "), "{}", text(&state));
    let view = &state.structured_content.as_ref().unwrap()["view"];
    let id = view["decision"]["id"].as_str().unwrap().to_string();
    assert_eq!(open(seat), Some(id.clone()));

    let step = |decision: &str, indices: Vec<i64>, expect: Option<Vec<&str>>| StepParams {
        token: None,
        decision: decision.into(),
        indices: Some(indices),
        name: None,
        expect: expect.map(|e| e.into_iter().map(String::from).collect()),
        timeout_s: Some(0),
    };
    for (params, why) in [
        (step("stale", vec![0], None), "decision stale is not open"),
        (step(&id, vec![0], Some(vec!["no such"])), "not \"no such\""),
        (step(&id, vec![0], Some(vec![])), "one entry per index"),
        (step(&id, vec![-1], None), "nonnegative"),
        (step(&id, vec![99], None), "option 99 does not exist"),
    ] {
        let refused = play.step(params).await;
        let said = text(&refused);
        assert!(failed(&refused) && said.starts_with("REJECTED: "), "{said}");
        assert!(said.contains(why), "{said}");
        assert!(said.contains(&format!("\nDECISION {id}")), "{said}");
        assert_eq!(
            open(seat),
            Some(id.clone()),
            "a refused answer changed the game"
        );
    }

    let log = play.log(&LogParams {
        after: Some(0),
        limit: Some(2),
    });
    let entries = log.structured_content.as_ref().unwrap()["log"].clone();
    let entries: Vec<LogEntry> = serde_json::from_value(entries).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].seq > 0 && entries[0].seq < entries[1].seq);
    let lines: Vec<String> = (entries.iter())
        .map(|e| format!("  {}: {}", e.seq, e.text))
        .collect();
    assert_eq!(text(&log), format!("LOG\n{}", lines.join("\n")));

    let turns = play.turns(&TurnsParams {
        player: None,
        from: None,
        to: None,
        last: None,
        detail: false,
    });
    let said = text(&turns);
    assert!(said.starts_with("TURNS\n  ["), "{said}");
    let summaries = &turns.structured_content.as_ref().unwrap()["turns"];
    assert_eq!(
        summaries.as_array().unwrap().len(),
        said.lines().count() - 1
    );
}

fn arena_game(hub: &Hub) -> Seat {
    let created = hub
        .create(&NewGame {
            players: 2,
            kingdom: ["Village", "Smithy", "Market", "Militia", "Moat"]
                .map(String::from)
                .to_vec(),
            seed: Some(7),
            ..Default::default()
        })
        .unwrap();
    hub.claim(&created.game_id, &created.seats[0].token)
        .unwrap()
}

fn arena_seat<'a>(hub: &'a Hub, seat: &Seat) -> ArenaSeat<'a> {
    ArenaSeat {
        hub,
        seat: seat.clone(),
        max_wait: Duration::from_secs(1),
    }
}

#[tokio::test]
async fn an_arena_seat_keeps_the_contract() {
    let hub = Hub::default();
    let seat = arena_game(&hub);
    // Two turns pass, so the log has turns to tell.
    for _ in 0..2 {
        let (view, ..) = hub.view(&seat).unwrap();
        let d = view.decision.unwrap();
        let end = d.options.iter().find(|o| o.action == "end-phase").unwrap();
        hub.answer(&seat, &Answer::indices(d.id.clone(), vec![end.index]))
            .unwrap();
        let other = Seat {
            seat: dominion_engine::PlayerId(1),
            ..seat.clone()
        };
        let (view, ..) = hub.view(&other).unwrap();
        let d = view.decision.unwrap();
        let end = d.options.iter().find(|o| o.action == "end-phase").unwrap();
        hub.answer(&other, &Answer::indices(d.id.clone(), vec![end.index]))
            .unwrap();
    }
    keeps_the_contract(&arena_seat(&hub, &seat)).await;
}

#[tokio::test]
async fn an_official_seat_keeps_the_contract() {
    let entry = |seq, kind: &str, text: &str| LogEntry {
        seq,
        kind: kind.into(),
        player: Some((seq / 2 % 2) as u8),
        card: None,
        text: text.into(),
    };
    let logs = vec![
        entry(0, "turn-start", "fixture-42 starts turn 1"),
        entry(1, "played", "fixture-42 plays a Copper"),
        entry(2, "turn-start", "fixture--2 starts turn 1"),
        entry(3, "played", "fixture--2 plays a Copper"),
    ];
    let (_directory, client) = crate::online::fixture_client(logs);
    let seat = crate::online::Session::new(client);
    keeps_the_contract(&seat).await;

    // An answer the checks let through is the backend's to refuse: here it can't be sent.
    let shown = Mutex::new(Presentation::default());
    let id = open(&seat).unwrap();
    let sent = Play {
        seat: &seat,
        shown: &shown,
        clock_cues: true,
    }
    .step(StepParams {
        token: None,
        decision: id,
        indices: Some(vec![1]),
        name: None,
        expect: None,
        timeout_s: Some(0),
    })
    .await;
    assert!(
        failed(&sent) && text(&sent).contains("connection closed"),
        "{}",
        text(&sent)
    );
    assert_eq!(open(&seat), None);
}

/// An Arena seat whose Treasures go one answer at a time, as a relaying backend's must.
struct Relayed<'a>(ArenaSeat<'a>);

impl PlaySeat for Relayed<'_> {
    fn observe(&self) -> Result<Option<(Seat, PlayObservation)>, String> {
        self.0.observe()
    }
    async fn answer(&self, answer: &Answer, expect: Option<&[String]>) -> Result<(), String> {
        self.0.answer(answer, expect).await
    }
    async fn wait(&self, timeout: Duration) -> Result<(), String> {
        self.0.wait(timeout).await
    }
    fn cards(&self, names: &[String]) -> Result<Vec<GlossaryEntry>, String> {
        self.0.cards(names)
    }
    fn no_game(&self) -> String {
        self.0.no_game()
    }
}

#[tokio::test]
async fn treasures_played_one_by_one_end_where_the_tables_own_run_does() {
    let (whole, relayed) = (Hub::default(), Hub::default());
    let (a, b) = (arena_game(&whole), arena_game(&relayed));
    let whole = arena_seat(&whole, &a);
    let relayed = Relayed(arena_seat(&relayed, &b));
    let id = open(&whole).unwrap();
    assert_eq!(open(&relayed), Some(id.clone()));
    let played = whole.play_treasures(&id).await.unwrap();
    assert!(played > 0);
    assert_eq!(relayed.play_treasures(&id).await, Ok(played));
    let view = |seat: &dyn Fn() -> PlayObservation| {
        let mut view = seat().view;
        // The decision's id counts the answers taken, which differ.
        if let Some(d) = &mut view.decision {
            d.id.clear();
        }
        view.since_last_decision.clear();
        view
    };
    assert_eq!(
        view(&|| whole.observe().unwrap().unwrap().1),
        view(&|| relayed.observe().unwrap().unwrap().1)
    );
    for seat in [open(&whole), open(&relayed)] {
        let refused = relayed.play_treasures(&seat.unwrap()).await.unwrap_err();
        assert!(refused.starts_with("REJECTED: no Treasures"), "{refused}");
    }
}
