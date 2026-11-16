//! Native harnesses have no game credentials or host tools. This bridge alone
//! validates and executes their structured game requests.

mod antigravity;
mod bridge;
mod claude;
mod codex;
mod process;

use crate::archive::{Archive, call};
use crate::client::Client;
use crate::house::Job;
use crate::runner::AgentSpec;
use serde_json::json;
use tokio_util::sync::CancellationToken;

const INSTRUCTIONS: &str = "You play one assigned seat of Dominion. Play to win using only the dominion MCP tools. Use view for your legal observations, card for official card text, and answer or play_treasures to submit a move. The initial observation includes official text for the game's cards. Use view with include_cards: true to retrieve that reference again, or card for one entry. You may look up cards before deciding. Keep playing this seat for the entire game in this conversation. After each move, use the tool result as your new observation. If no decision is available for you, call wait_for_decision and continue. Finish only when the returned game.running is false. Do not call view after a tool already returned the current view. Never print a JSON action in place of calling a tool. Do not inspect files, run commands, delegate, resign, or ask a person for help. The server supplies your seat credentials. When a tool rejects a request, correct it using the returned error.";

pub async fn play(
    spec: &AgentSpec,
    job: &Job,
    server: &str,
    archive: &mut Archive,
    stop: CancellationToken,
) -> Result<(), String> {
    spec.validate()?;
    archive.publish(server, job)?;
    archive.record("seat", json!({"agent":spec,"job":job,"server":server}))?;
    let client = match Client::connect(server).await {
        Ok(client) => client,
        Err(error) => {
            archive.record("seat_finished", json!({"result":{"Err":error}}))?;
            archive.finish_publish().await;
            return Err(error);
        }
    };
    let result = async {
        let mut harness = process::Harness::new(spec, &archive.dir)?;
        tokio::select! {
            biased;
            _ = stop.cancelled() => Err("seat runner cancelled".into()),
            result = play_inner(&mut harness, &client, spec, job, server, archive) => result,
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
    archive.finish_publish().await;
    result
}

async fn play_inner(
    harness: &mut process::Harness,
    client: &Client,
    spec: &AgentSpec,
    job: &Job,
    server: &str,
    archive: &mut Archive,
) -> Result<(), String> {
    let view = call(
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
    let mut bridge = bridge::Bridge::start(server, job).await?;
    harness.enable_mcp(
        spec,
        &bridge.url,
        bridge.records.take().expect("new bridge"),
    )?;
    let state = view.json.as_ref().ok_or("Dominion returned no game view")?;
    if state["game"]["running"] == false {
        return Ok(());
    }
    let message = format!("Initial view and card reference:\n{}", view.text);
    harness.reply(spec, &message, archive).await?;
    let view = call(
        client,
        archive,
        "dominion",
        "view",
        json!({"token":job.token}),
    )
    .await?;
    if !view.ok {
        return Err(view.text);
    }
    if view
        .json
        .as_ref()
        .is_some_and(|v| v["game"]["running"] == false)
    {
        Ok(())
    } else {
        Err("agent session ended before the game finished".into())
    }
}
