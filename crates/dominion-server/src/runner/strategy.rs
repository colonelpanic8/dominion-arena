//! The opening a runner gives its player. Providers own the model conversation and tools.
use super::RunnerStrategy;
use crate::archive::{Archive, call};
use crate::client::{Client, Reply};
use crate::harness::{Sandbox, game_state, is_clock_active, is_running};
use crate::house::Job;
use serde_json::json;

pub(crate) struct Opening {
    pub view: Reply,
    pub clock_aware: bool,
    pub ratings: Option<String>,
}

/// The first-move target a clock-aware player is given without an analyst's.
const OPENING_TARGET_S: u32 = 45;

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

    /// A clock-aware player starts at once, before its first decision if it has none
    /// yet: what it plans while others play costs it no time.
    async fn prepare_opening(
        self,
        initial: Reply,
        _client: &Client,
        _archive: &mut Archive,
        job: &Job,
        sandbox: &Sandbox,
    ) -> Result<Opening, String> {
        let state = initial
            .json
            .as_ref()
            .ok_or("Dominion returned no game view")?;
        let clock_aware = self.timed_opening(state);
        let ratings = if clock_aware {
            prefetch_card_ratings(sandbox, job, state).await
        } else {
            None
        };
        Ok(Opening {
            view: initial,
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
    let advice = analyst::Advice::with_notices(seat.notices.clone());
    let model = AnalysisModel::new(spec, sandbox);
    let (tool, args) = job.options.strategy.initial_tool(job);
    let joined = call(client, archive, "dominion", tool, args).await?;
    if !joined.ok {
        return Err(joined.text);
    }
    // The analyst works beside the player for as long as it plays.
    let mut analysis = tokio::task::JoinSet::new();
    let analyzing = analyst::start(&mut analysis, &advice, plan, &model, job, &joined, archive);
    let Opening {
        mut view,
        clock_aware: clocked,
        ratings,
    } = job
        .options
        .strategy
        .prepare_opening(joined, client, archive, job, sandbox)
        .await?;
    let state = view.json.as_ref().ok_or("Dominion returned no game view")?;
    if !is_running(state) {
        return Ok(Err(JOINED_OVER.into()));
    }
    let mut guidance = String::new();
    if clocked {
        let target = plan.map_or(OPENING_TARGET_S, |p| p.target_seconds);
        guidance.push_str(&opening_budget(target));
    }
    if analyzing {
        guidance.push_str(" A separate analyst is studying the kingdom; its opening plan reaches you beside a later decision. Do not wait for it. Use any opening plans that fit the current hand and budget. Choose your move from the current legal options.");
        if plan.is_some_and(|a| a.background) {
            guidance.push_str(" Later strategy notes can arrive between decisions. Keep playing while the analyst works.");
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
    let played = provider
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
        .await;
    analysis.abort_all();
    played
}

/// What a clock-aware player is told of its opening clock.
fn opening_budget(target: u32) -> String {
    format!(
        " OPENING BUDGET: aim to submit your first move within {target} seconds of your first decision. Plan before that decision and during other players' turns, while your clock is not running; call wait when no decision is open. If your bank runs out with a decision open, the harness answers it with the least it allows, so never spend your last seconds thinking."
    )
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
/// Skipped only when the seat's own clock is already running low.
async fn prefetch_card_ratings(
    sandbox: &Sandbox,
    job: &Job,
    reply: &serde_json::Value,
) -> Option<String> {
    if job.brief.is_some()
        || super::analyst::available_time(reply, crate::hub::now_ms()).is_some_and(
            |(bank, available)| bank.min(available) <= crate::mcp::play::CLOCK_PRESSURE_MS,
        )
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
    use crate::harness::has_decision;
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
            options: Default::default(),
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

        // A clock-aware player starts before its first decision, ratings in hand: it
        // plans on the other seat's time.
        let opening = RunnerStrategy::ClockAware
            .prepare_opening(initial.clone(), &aware, &mut archive, &job, &sandbox)
            .await
            .unwrap();
        assert!(opening.clock_aware && opening.ratings.unwrap().contains("Smithy"));
        assert!(!has_decision(opening.view.json.as_ref().unwrap()));
        assert_eq!(opening.view.text, initial.text);
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
        // Low on time, it is not held up for ratings.
        let low = aware.call("state", json!({})).await.unwrap();
        assert!(
            prefetch_card_ratings(&sandbox, &job, low.json.as_ref().unwrap())
                .await
                .is_none()
        );
        let old = classic.call("state", json!({})).await.unwrap();
        let new = low;
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
