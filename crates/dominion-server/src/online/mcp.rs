use super::runtime::{Client, NewGame, Update, glossary};
use crate::mcp::play::{self, CardsParams, LogParams, StateParams, StepParams, WaitParams};
use dominion_protocol::Answer;
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Session {
    client: Client,
    presentation: Arc<Mutex<play::Presentation>>,
}
impl Session {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            presentation: Arc::new(Mutex::new(play::Presentation::default())),
        }
    }
    fn failure(&self, message: String) -> CallToolResult {
        let mut text = message;
        if let Ok(Some(view)) = self.client.current().view() {
            let rendered = dominion_agent::text::render(&view);
            if let Some(index) = rendered.find("DECISION ") {
                text.push('\n');
                text.push_str(&rendered[index..]);
            }
        }
        CallToolResult::error(vec![ContentBlock::text(text)])
    }
    fn observation(&self, update: Update, full: bool) -> CallToolResult {
        let view = match update.view() {
            Ok(Some(view)) => view,
            Ok(None) => return structured(
                serde_json::json!({"view":null}),
                "STATUS no game; call new_game for an unrated game against official server bots."
                    .into(),
            ),
            Err(e) => return self.failure(e),
        };
        let Some(seat) = view.seat else {
            return self.failure("REFUSED: spectator gameplay is unsupported".into());
        };
        let names: Vec<_> = dominion_agent::glossary_for(&view)
            .into_iter()
            .map(|e| e.name)
            .collect();
        let cards = match glossary(&update.reducer, &names) {
            Ok(g) => g,
            Err(e) => return self.failure(e),
        };
        let observation = crate::hub::PlayObservation {
            log: view.since_last_decision.clone(),
            view: view.clone(),
            glossary: cards,
            undos: update.reducer.epoch,
        };
        let seat = crate::hub::Seat {
            game: view.game.id.clone().unwrap_or_default(),
            seat: dominion_engine::PlayerId(seat),
        };
        let (text, cards) = self
            .presentation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .render(&seat, &observation, full);
        structured(serde_json::json!({"view":view,"glossary":cards}), text)
    }
}
fn structured(value: serde_json::Value, text: String) -> CallToolResult {
    let mut result = CallToolResult::structured(value);
    result.content = vec![ContentBlock::text(text)];
    result
}
#[tool_router]
impl Session {
    #[tool(
        description = "Full official-server player view and current decision, through dominion/play@1. Call first. Token is ignored for this one-account backend."
    )]
    async fn state(&self, Parameters(_): Parameters<StateParams>) -> CallToolResult {
        self.observation(self.client.current(), true)
    }
    #[tool(
        description = "Answer the official decision, then wait. Supply expect with each chosen option label/card. Ordered indices are top first. Stale or invalid answers change nothing. Token is ignored."
    )]
    async fn step(&self, Parameters(p): Parameters<StepParams>) -> CallToolResult {
        let indices = match p
            .indices
            .map(|v| {
                v.into_iter()
                    .map(u32::try_from)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
        {
            Ok(v) => v,
            Err(_) => {
                return self.failure(
                    "REJECTED: option indices must be nonnegative 32-bit integers".into(),
                );
            }
        };
        let answer = Answer {
            decision: p.decision,
            indices,
            name: p.name,
        };
        if let Err(e) = self.client.answer(answer, p.expect).await {
            return self.failure(e);
        }
        match self.client.wait(p.timeout_s.unwrap_or(60)).await {
            Ok(update) => self.observation(update, false),
            Err(e) => self.failure(e),
        }
    }
    #[tool(
        description = "Wait for your next official decision without answering. timeout_s defaults to 60, maximum 600. Token is ignored."
    )]
    async fn wait(&self, Parameters(p): Parameters<WaitParams>) -> CallToolResult {
        match self.client.wait(p.timeout_s.unwrap_or(60)).await {
            Ok(update) => self.observation(update, false),
            Err(e) => self.failure(e),
        }
    }
    #[tool(
        description = "Start an unrated official-server game against server bots. players defaults to 2 and bots to 1; bots must equal players minus one. Optional kingdom and landscapes are required card names. No humans or spectators. Never replaces or resigns an existing game. Settings the backend cannot honor are refused."
    )]
    async fn new_game(&self, Parameters(p): Parameters<NewGame>) -> CallToolResult {
        if let Err(e) = self.client.new_game(p).await {
            return self.failure(e);
        }
        match self.client.wait(60).await {
            Ok(update) => self.observation(update, true),
            Err(e) => self.failure(e),
        }
    }
    #[tool(description = "Official card text and current costs for named cards.")]
    async fn cards(&self, Parameters(p): Parameters<CardsParams>) -> CallToolResult {
        match glossary(&self.client.current().reducer, &p.names) {
            Ok(entries) => {
                let text = entries
                    .iter()
                    .map(crate::card_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                structured(serde_json::json!({"glossary":entries}), text)
            }
            Err(e) => self.failure(e),
        }
    }
    #[tool(
        description = "Reread the official game log. after is an exclusive sequence number. Keep newest limit entries (default 50), returned oldest first."
    )]
    async fn log(&self, Parameters(p): Parameters<LogParams>) -> CallToolResult {
        let update = self.client.current();
        if let Err(e) = update.view() {
            return self.failure(e);
        }
        let entries: Vec<_> = update
            .reducer
            .logs
            .iter()
            .filter(|e| p.after.is_none_or(|after| e.seq > after))
            .collect();
        let entries = &entries[entries
            .len()
            .saturating_sub(p.limit.unwrap_or(50).min(1000))..];
        let text = entries
            .iter()
            .map(|e| format!("{}: {}", e.seq, e.text))
            .collect::<Vec<_>>()
            .join("\n");
        structured(serde_json::json!({"log":entries}), text)
    }
    #[tool(
        description = "Explicitly resign this account's current official-server game. Token is ignored."
    )]
    async fn resign(&self, Parameters(_): Parameters<StateParams>) -> CallToolResult {
        match self.client.resign().await {
            Ok(()) => match self.client.wait(30).await {
                Ok(update) => self.observation(update, true),
                Err(e) => self.failure(e),
            },
            Err(e) => self.failure(e),
        }
    }
    #[tool(
        description = "Reconnect only this process's own official-server session. Invalidates prior decisions, does not resend actions, and never imports a browser session or removes another login."
    )]
    async fn online_reconnect(&self) -> CallToolResult {
        match self.client.reconnect().await {
            Ok(()) => match self.client.wait(30).await {
                Ok(update) => self.observation(update, true),
                Err(e) => self.failure(e),
            },
            Err(e) => self.failure(e),
        }
    }
}
#[tool_handler(
    name = "dominion",
    instructions = "Play through dominion/play@1: state, new_game, then step with decision and indices (top first) or name where nameAny; give expect with each selected label/card. When decision is null call wait. Results carry full {view,glossary?}; step/wait text shows changes. Stop at game.running=false. This is the official dominion.games backend, not Arena's hosted engine. It supports only this account versus server bots, unrated, spectators prohibited. Lifecycle authentication is configured by the operator before this MCP starts. No seat token is needed. Unsupported official protocol features fail clearly; never retry a sent answer after reconnect. online_reconnect only resumes this process's own session and replaces the snapshot. cards and log provide references."
)]
impl ServerHandler for Session {}
