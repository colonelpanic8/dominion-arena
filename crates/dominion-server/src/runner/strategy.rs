//! The opening a runner gives its player. Providers own the model conversation and tools.
use super::RunnerStrategy;
use crate::archive::{Archive, call};
use crate::client::{Client, Reply};
use crate::harness::{Sandbox, game_state, has_decision, is_clock_active, is_running};
use crate::house::Job;
use serde_json::json;

pub(crate) struct Opening {
    pub view: Reply,
    pub observed_at: tokio::time::Instant,
    pub clock_aware: bool,
    pub ratings: Option<String>,
}

pub(crate) trait ModelRunnerStrategy: Copy {
    fn initial_tool(self, job: &Job) -> (&'static str, serde_json::Value);
    fn clock_cues(self) -> bool;

    fn timed_opening(self, state: &serde_json::Value) -> bool {
        self.clock_cues() && is_clock_active(state) && is_running(state)
    }

    async fn prepare_opening(
        self,
        initial: Reply,
        client: &Client,
        archive: &mut Archive,
        job: &Job,
        sandbox: &Sandbox,
    ) -> Result<Opening, String>;
}

impl ModelRunnerStrategy for RunnerStrategy {
    fn initial_tool(self, job: &Job) -> (&'static str, serde_json::Value) {
        if job.official {
            return ("state", json!({}));
        }
        let key = match self {
            Self::Classic => "game_id",
            Self::ClockAware => "game",
        };
        ("join_game", json!({(key):job.game_id, "token":job.token}))
    }

    fn clock_cues(self) -> bool {
        match self {
            Self::Classic => false,
            Self::ClockAware => true,
        }
    }

    async fn prepare_opening(
        self,
        initial: Reply,
        client: &Client,
        archive: &mut Archive,
        job: &Job,
        sandbox: &Sandbox,
    ) -> Result<Opening, String> {
        let view = match self {
            RunnerStrategy::Classic => initial,
            RunnerStrategy::ClockAware => {
                wait_until_decision(client, archive, job, initial).await?
            }
        };
        let observed_at = tokio::time::Instant::now();
        let state = view.json.as_ref().ok_or("Dominion returned no game view")?;
        let clock_aware = self.timed_opening(state);
        let ratings = if clock_aware {
            prefetch_card_ratings(sandbox, job, state).await
        } else {
            None
        };
        Ok(Opening {
            view,
            observed_at,
            clock_aware,
            ratings,
        })
    }
}

pub(super) async fn run(
    provider: &mut impl super::provider::PlayerAdapter,
    seat: &super::provider::Seat<'_>,
    plan: Option<&super::provider::AnalysisPlan>,
    archive: &mut Archive,
) -> Result<Result<String, String>, String> {
    use super::{analyst, provider::AnalysisModel};
    use crate::harness::JOINED_OVER;
    let (spec, sandbox, job, client) = (seat.spec, seat.sandbox, seat.job, seat.client.as_ref());
    let advice = analyst::Advice::default();
    let model = AnalysisModel::new(spec, sandbox);
    let (tool, args) = job.strategy.initial_tool(job);
    let joined = call(client, archive, "dominion", tool, args).await?;
    if !joined.ok {
        return Err(joined.text);
    }
    let mut analysis = tokio::task::JoinSet::new();
    let mut analyzing = analyst::start(&mut analysis, &advice, plan, &model, job, &joined, archive);
    let Opening {
        mut view,
        observed_at,
        clock_aware: clocked,
        ratings,
    } = job
        .strategy
        .prepare_opening(joined, client, archive, job, sandbox)
        .await?;
    if !analyzing {
        analyzing = analyst::start(&mut analysis, &advice, plan, &model, job, &view, archive);
    }
    let state = view.json.as_ref().ok_or("Dominion returned no game view")?;
    if !is_running(state) {
        return Ok(Err(JOINED_OVER.into()));
    }
    let deadline = plan
        .filter(|_| analyzing)
        .and_then(|config| {
            analyst::planning_window(state, config.target_seconds, crate::hub::now_ms())
        })
        .map(|window| observed_at + window);
    let facts = if analyzing {
        serde_json::from_value(game_state(state).clone())
            .ok()
            .and_then(|view| analyst::opening_facts(&view))
    } else {
        None
    };
    let mut preliminary = None;
    if let Some(deadline) = deadline {
        let decision = game_state(state)["decision"]["id"].clone();
        let mut board = view.text.clone();
        if let Some(facts) = &facts {
            board.push_str(&format!("\n\n{facts}"));
        }
        if let Some(ratings) = &ratings {
            board.push_str(&format!("\n\nPre-fetched card ratings:\n{ratings}"));
        }
        if deadline > tokio::time::Instant::now() {
            preliminary =
                analyst::preliminary_plan(&model, &board, archive, &advice, &job.game_id, deadline)
                    .await?;
        }
        let _ = tokio::time::timeout_at(deadline, analysis.join_next()).await;
        let args = if job.official {
            json!({})
        } else {
            json!({"token": job.token})
        };
        let mut fresh = call(client, archive, "dominion", "state", args).await?;
        if !fresh.ok {
            return Err(fresh.text);
        }
        let state = fresh
            .json
            .as_ref()
            .ok_or("Dominion returned no game view")?;
        if !is_running(state) {
            return Ok(Err(JOINED_OVER.into()));
        }
        if game_state(state)["game"]["id"].as_str() != Some(job.game_id.as_str()) {
            return Err("the seat changed games during opening analysis".into());
        }
        if game_state(state)["decision"]["id"] != decision {
            preliminary = None;
        }
        complete_opening(&view, &view.model_prompts, &mut fresh);
        view = fresh;
    }
    let mut guidance = String::new();
    if analyzing {
        guidance.push_str(" A separate analyst is studying the kingdom. Use any opening plans that fit the current hand and budget. Choose your move from the current legal options.");
        if plan.is_some_and(|a| a.background) {
            guidance.push_str(" Later strategy notes can arrive between decisions. Keep playing while the analyst works.");
        }
        if deadline.is_some() {
            guidance.push_str(" The opening planning window has elapsed or the analyses finished early. Submit your first move promptly.");
        }
    }
    advice.deliver(&mut view);
    let mut message = provider.opening_message(job, spec);
    message.push_str(&format!("Initial view and card reference:\n{}", view.text));
    if let Some(ratings) = ratings {
        message.push_str(&format!(
            "\n\nPre-fetched card ratings (`python3 data/card-ratings.py`):\n{ratings}"
        ));
    }
    if let Some(facts) = view
        .json
        .as_ref()
        .filter(|_| analyzing)
        .and_then(|s| serde_json::from_value(game_state(s).clone()).ok())
        .and_then(|v| analyst::opening_facts(&v))
    {
        message.push_str(&format!("\n\n{facts}"));
    }
    if let Some(plan) = preliminary {
        message.push_str(&format!("\n\nYour preliminary opening plan: {plan}"));
    }
    provider
        .play_player(
            seat,
            archive,
            super::provider::PlayerOpening {
                message,
                guidance,
                clocked,
                advice,
            },
        )
        .await
}

fn extract_cards_lines(text: &str) -> Option<String> {
    let rest = text
        .strip_prefix("CARDS\n")
        .or_else(|| text.split_once("\nCARDS\n").map(|(_, r)| r))?;
    let mut lines = Vec::new();
    for line in rest.lines() {
        if line.starts_with("GAME OVER")
            || line.starts_with("RESULT")
            || line.starts_with("SPECTATING:")
            || line.starts_with("WAITING:")
            || line.starts_with("DECISION ")
        {
            break;
        }
        lines.push(line);
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn merge_initial_cards(initial_text: &str, fresh_text: &mut String) {
    let Some(initial_cards) = extract_cards_lines(initial_text) else {
        return;
    };
    if let Some(pos) = fresh_text.find("\nCARDS\n") {
        let insert_at = pos + "\nCARDS\n".len();
        fresh_text.insert_str(insert_at, &format!("{initial_cards}\n"));
    } else if fresh_text.starts_with("CARDS\n") {
        fresh_text.insert_str("CARDS\n".len(), &format!("{initial_cards}\n"));
    } else {
        fresh_text.push_str(&format!("\nCARDS\n{initial_cards}"));
    }
}

/// The session has already rendered these instructions in observations the player hasn't seen.
pub(crate) fn complete_opening(first: &Reply, already_shown: &[String], fresh: &mut Reply) {
    merge_initial_cards(&first.text, &mut fresh.text);
    let mut instructions = String::new();
    for (old, current) in already_shown.iter().zip(&fresh.model_prompts) {
        if old != current {
            break;
        }
        instructions.push_str(crate::mcp::play::GAMEPLAY_INSTRUCTION);
        instructions.push('\n');
        instructions.push_str(old);
        instructions.push('\n');
    }
    fresh.text.insert_str(0, &instructions);
}

/// On a clocked game whose initial observation has no decision yet (`WAITING`), waits
/// until this seat has its first actionable decision (or the game ends), then fetches a
/// fresh full `state` with the initial card reference preserved so the agent starts with
/// its actual hand, live clock, and first decision.
async fn wait_until_decision(
    client: &Client,
    archive: &mut Archive,
    job: &Job,
    initial: crate::client::Reply,
) -> Result<crate::client::Reply, String> {
    let Some(json) = initial.json.as_ref() else {
        return Ok(initial);
    };
    if !is_running(json) || !is_clock_active(json) || has_decision(json) {
        return Ok(initial);
    }
    let args = || {
        if job.official {
            json!({})
        } else {
            json!({"token": job.token})
        }
    };
    let already_shown = loop {
        let waited = call(client, archive, "dominion", "wait", args()).await?;
        if !waited.ok {
            return Err(waited.text);
        }
        let Some(json) = waited.json.as_ref() else {
            return Err("Dominion returned no game view".into());
        };
        if game_state(json)["game"]["id"].as_str() != Some(job.game_id.as_str()) {
            return Err("the seat changed games while waiting for the opening".into());
        }
        if !is_running(json) {
            return Ok(waited);
        }
        if has_decision(json) {
            break waited.model_prompts;
        }
    };
    let mut fresh = call(client, archive, "dominion", "state", args()).await?;
    if !fresh.ok {
        return Err(fresh.text);
    }
    if fresh
        .json
        .as_ref()
        .and_then(|v| game_state(v)["game"]["id"].as_str())
        != Some(job.game_id.as_str())
    {
        return Err("the seat changed games while refreshing the opening".into());
    }
    complete_opening(&initial, &already_shown, &mut fresh);
    Ok(fresh)
}

/// Non-basic Kingdom Supply cards and landscapes present in an observation's view.
fn board_card_names(reply: &serde_json::Value) -> Vec<String> {
    const BASICS: [&str; 9] = [
        "Copper", "Silver", "Gold", "Platinum", "Estate", "Duchy", "Province", "Colony", "Curse",
    ];
    let state = game_state(reply);
    let mut names = Vec::new();
    for field in ["supply", "landscapes"] {
        if let Some(items) = state.get(field).and_then(|v| v.as_array()) {
            for item in items {
                let Some(card) = item.get("card").and_then(|v| v.as_str()) else {
                    continue;
                };
                if !BASICS.contains(&card) && !names.iter().any(|n| n == card) {
                    names.push(card.to_string());
                }
            }
        }
    }
    names
}

/// Pre-runs the playbook's `data/card-ratings.py` for the board on clocked games so the
/// model starts with ratings in hand instead of spending Turn 1 clock on lookup churn.
async fn prefetch_card_ratings(
    sandbox: &Sandbox,
    job: &Job,
    reply: &serde_json::Value,
) -> Option<String> {
    if job.brief.is_some()
        || !super::analyst::available_time(reply, crate::hub::now_ms()).is_some_and(
            |(bank, available)| bank.min(available) > crate::mcp::play::CLOCK_PRESSURE_MS,
        )
        || game_state(reply)["turn"]["yourTurn"] != true
    {
        return None;
    }
    let playbook = sandbox.playbook.as_deref()?;
    if !playbook.join("data/card-ratings.py").is_file() {
        return None;
    }
    let names = board_card_names(reply);
    if names.is_empty() {
        return None;
    }
    let mut path: Vec<std::path::PathBuf> = sandbox.path.clone();
    path.extend(
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    let joined_path = std::env::join_paths(path).ok()?;
    let run = tokio::process::Command::new("python3")
        .arg("data/card-ratings.py")
        .args(&names)
        .current_dir(playbook)
        .env("PATH", joined_path)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(5), run)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Arena;
    use crate::hub::{Hub, NewGame};
    use dominion_protocol::lobby::Clock;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn strategies_play_the_same_timed_seat_through_mcp() {
        let hub = Arc::new(Hub::default());
        let arena = Arena::open(hub.clone(), Default::default()).unwrap();
        let service = StreamableHttpService::new(
            move || {
                Ok(crate::mcp::Session::new(
                    arena.clone(),
                    Duration::from_secs(2),
                    false,
                ))
            },
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        let created = hub
            .create(&NewGame {
                players: 2,
                seed: Some(7),
                kingdom: ["Village", "Smithy", "Market", "Militia", "Moat"]
                    .map(String::from)
                    .to_vec(),
                clock: Some(Clock::new(15, 0)),
                ..Default::default()
            })
            .unwrap();
        let job = Job {
            strategy: Default::default(),
            official: false,
            official_account: None,
            transcript: false,
            configuration: None,
            agent: "Astra".into(),
            brief: None,
            debrief: None,
            game_id: created.game_id.clone(),
            seat: 1,
            name: "P2".into(),
            token: created.seats[1].token.clone(),
        };
        hub.restore_model_host(&job.game_id, vec![false, true], "host".into())
            .unwrap();
        let prompt = |text: &str| {
            use dominion_protocol::api::{ModelAction, ModelRequest};
            hub.model_action(
                &job.game_id,
                "host",
                false,
                &ModelRequest {
                    game: job.game_id.clone(),
                    seat: 1,
                    action: ModelAction::Prompt { text: text.into() },
                },
            )
            .unwrap();
        };
        prompt("Watch Militia.");
        let classic = Client::connect(&url).await.unwrap();
        let aware = Client::connect(&url).await.unwrap().with_clock_cues(true);
        let dir = tempfile::tempdir().unwrap();
        let mut archive = Archive::open(&dir.path().join("archive")).unwrap();
        let sandbox = Sandbox {
            playbook: Some(dir.path().into()),
            ..Default::default()
        };
        std::fs::create_dir(dir.path().join("data")).unwrap();
        std::fs::write(
            dir.path().join("data/card-ratings.py"),
            "import sys\nprint(*sys.argv[1:])",
        )
        .unwrap();
        let (tool, args) = RunnerStrategy::Classic.initial_tool(&job);
        let initial = classic.call(tool, args).await.unwrap();
        assert!(initial.text.contains("=== CARD & PILE REFERENCE ==="));
        assert!(
            initial.ok && !has_decision(initial.json.as_ref().unwrap()),
            "{}",
            initial.text
        );
        let old = RunnerStrategy::Classic
            .prepare_opening(initial.clone(), &classic, &mut archive, &job, &sandbox)
            .await
            .unwrap();
        assert!(!old.clock_aware && old.ratings.is_none());
        assert_eq!(old.view.text, initial.text);
        assert_eq!(old.view.text.matches("Watch Militia.").count(), 1);

        let (tool, args) = RunnerStrategy::ClockAware.initial_tool(&job);
        let initial = aware.call(tool, args).await.unwrap();
        let mut untimed = initial.clone();
        untimed.json.as_mut().unwrap()["clockInfo"]["status"] = json!("off");
        let off = RunnerStrategy::ClockAware
            .prepare_opening(untimed, &aware, &mut archive, &job, &sandbox)
            .await
            .unwrap();
        assert!(
            !off.clock_aware
                && off.ratings.is_none()
                && !has_decision(off.view.json.as_ref().unwrap())
        );

        prompt("Consider Moat.");
        let first = hub
            .claim(&created.game_id, &created.seats[0].token)
            .unwrap();
        let (view, ..) = hub.view(&first).unwrap();
        let d = view.decision.unwrap();
        let end = d
            .options
            .iter()
            .find(|o| o.action == "end-phase")
            .unwrap()
            .index;
        hub.answer(&first, &dominion_agent::Answer::indices(d.id, vec![end]))
            .unwrap();
        let opening = RunnerStrategy::ClockAware
            .prepare_opening(initial, &aware, &mut archive, &job, &sandbox)
            .await
            .unwrap();
        assert!(opening.clock_aware && opening.ratings.is_none());
        let mut healthy = opening.view.json.clone().unwrap();
        healthy["clock"]["left"][1] = json!(300_000);
        let ratings = prefetch_card_ratings(&sandbox, &job, &healthy)
            .await
            .unwrap();
        assert!(ratings.contains("Smithy"));
        let old = classic.call("state", json!({})).await.unwrap();
        let new = opening.view;
        for note in ["Watch Militia.", "Consider Moat."] {
            assert_eq!(new.text.matches(note).count(), 1, "{}", new.text);
        }
        for (reply, cues) in [(&old, false), (&new, true)] {
            assert!(reply.ok && reply.text.contains("CLOCK ") && reply.text.contains("DECISION "));
            assert_eq!(reply.text.contains("LOW TIME"), cues, "{}", reply.text);
            assert_eq!(
                reply.json.as_ref().unwrap()["clockInfo"]["status"],
                "available"
            );
        }
        let decision = old.json.unwrap()["view"]["decision"].clone();
        assert!(decision.is_object());
        assert_eq!(new.json.unwrap()["view"]["decision"], decision);
        for (client, cues) in [(&classic, false), (&aware, true)] {
            let refused = client
                .call(
                    "step",
                    json!({"decision":"stale","indices":[0],"timeout_s":0}),
                )
                .await
                .unwrap();
            assert!(
                !refused.ok
                    && refused.text.contains("REJECTED")
                    && refused.text.contains(decision["id"].as_str().unwrap())
            );
            assert_eq!(refused.text.contains("LOW TIME"), cues, "{}", refused.text);
        }
        classic.close().await;
        aware.close().await;
    }
}
