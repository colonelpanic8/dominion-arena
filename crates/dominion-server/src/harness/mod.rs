//! Native harnesses have no game credentials or host tools. This bridge alone
//! validates and executes their structured game requests.

mod antigravity;
mod claude;
mod codex;
mod process;

use crate::archive::{Archive, call};
use crate::client::Client;
use crate::house::Job;
use crate::runner::AgentSpec;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

const INSTRUCTIONS: &str = "You play one seat of Dominion. Play to win. The bridge supplies your legal view and executes one requested game tool per reply. Use only the observations supplied here for game information. Do not inspect files, run commands, delegate, or use other tools. Never resign or ask a person for help.
Reply with exactly one JSON object (no prose), using one of these forms:
{\"tool\":\"answer\",\"decision_id\":\"d1\",\"indices\":[0]}
{\"tool\":\"answer\",\"decision_id\":\"d1\",\"name\":\"Copper\"}
{\"tool\":\"play_treasures\",\"decision_id\":\"d1\"}
{\"tool\":\"card\",\"name\":\"Village\"}
An answer may also include \"way\":\"none\" or a Way's name. Use the actual decision id and option indices in your view; ordered choices are top first. A card lookup returns official card text, then you may answer. The conversation continues throughout the game. When a tool returns an error, correct your request.";

#[derive(Debug, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Answer {
        decision_id: String,
        indices: Option<Vec<u32>>,
        name: Option<String>,
        way: Option<String>,
    },
    PlayTreasures {
        decision_id: String,
    },
    Card {
        name: String,
    },
}

fn action(text: &str) -> Result<Action, String> {
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|t| t.trim().strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    serde_json::from_str(text).map_err(|e| format!("reply with one game-tool JSON object: {e}"))
}

pub async fn play(
    spec: &AgentSpec,
    job: &Job,
    server: &str,
    archive: &mut Archive,
    stop: CancellationToken,
) -> Result<(), String> {
    spec.validate()?;
    archive.record("seat", json!({"agent":spec,"job":job,"server":server}))?;
    let client = match Client::connect(server).await {
        Ok(client) => client,
        Err(error) => {
            archive.record("seat_finished", json!({"result":{"Err":error}}))?;
            return Err(error);
        }
    };
    let result = async {
        let mut harness = process::Harness::new(spec, &archive.dir)?;
        tokio::select! {
            biased;
            _ = stop.cancelled() => Err("seat runner cancelled".into()),
            result = play_inner(&mut harness, &client, spec, job, archive) => result,
        }
    }
    .await;
    let final_view = call(
        &client,
        archive,
        "dominion",
        "view",
        json!({"token":job.token}),
    )
    .await;
    if let Ok(view) = final_view
        && let Some(state) = view.json
        && state["game"]["running"] == false
    {
        archive.record("result", state)?;
    }
    archive.record("seat_finished", json!({"result":result}))?;
    client.close().await;
    result
}

async fn play_inner(
    harness: &mut process::Harness,
    client: &Client,
    spec: &AgentSpec,
    job: &Job,
    archive: &mut Archive,
) -> Result<(), String> {
    let mut view = call(
        client,
        archive,
        "dominion",
        "join_game",
        json!({"game_id":job.game_id,"token":job.token}),
    )
    .await?;
    if !view.ok {
        return Err(view.text);
    }
    let mut errors = 0;
    loop {
        let state = view.json.as_ref().ok_or("Dominion returned no game view")?;
        if state["game"]["running"] == false {
            archive.record("result", state.clone())?;
            return Ok(());
        }
        if state["decision"].is_null() {
            view = call(
                client,
                archive,
                "dominion",
                "wait_for_decision",
                json!({"token":job.token,"timeout_ms":60_000}),
            )
            .await?;
            if !view.ok {
                return Err(view.text);
            }
            continue;
        }
        let mut message = format!(
            "Current view:\n{}",
            serde_json::to_string(state).expect("serializable")
        );
        for interaction in 0..32 {
            if interaction == 31 {
                return Err("agent exceeded requests for one decision".into());
            }
            let response = harness.reply(spec, &message, archive).await?;
            let (tool, mut args) = match action(&response) {
                Ok(Action::Answer {
                    decision_id,
                    indices,
                    name,
                    way,
                }) => (
                    "answer",
                    json!({"decision_id":decision_id,"indices":indices,"name":name,"way":way}),
                ),
                Ok(Action::PlayTreasures { decision_id }) => {
                    ("play_treasures", json!({"decision_id":decision_id}))
                }
                Ok(Action::Card { name }) => ("card", json!({"name":name})),
                Err(e) => {
                    errors += 1;
                    if errors >= 3 {
                        return Err(e);
                    }
                    message = e;
                    continue;
                }
            };
            if tool != "card" {
                args["token"] = json!(job.token);
            }
            let reply = call(client, archive, "dominion", tool, args).await?;
            if tool == "card" || !reply.ok {
                if !reply.ok {
                    errors += 1;
                    if errors >= 3 {
                        return Err(reply.text);
                    }
                }
                message = serde_json::to_string(&reply).expect("serializable");
                continue;
            }
            errors = 0;
            view = reply;
            break;
        }
    }
}
