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
    managed: bool,
    presentation: Arc<Mutex<play::Presentation>>,
}
impl Session {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            managed: false,
            presentation: Arc::new(Mutex::new(play::Presentation::default())),
        }
    }
    pub(crate) fn managed(client: Client) -> Self {
        Self {
            managed: true,
            ..Self::new(client)
        }
    }
    fn treasure_plays(
        &self,
        update: &Update,
        d: &dominion_protocol::AgentDecision,
    ) -> Vec<(u32, String)> {
        let names: Vec<String> = d.options.iter().filter_map(|o| o.card.clone()).collect();
        let cards = glossary(&update.reducer, &names).unwrap_or_default();
        treasure_plays(d, |name| {
            cards
                .iter()
                .any(|e| e.name == name && e.types.iter().any(|t| t == "Treasure"))
        })
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
            note: None,
            clock: update.clock(crate::hub::now_ms()),
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
        let mut value = serde_json::json!({"view":view,"glossary":cards});
        if let Some(clock) = &observation.clock {
            value["clock"] = serde_json::json!(clock);
        }
        structured(value, text)
    }
}
/// The Treasures a Buy phase move offers to play, as (option index, card name). The
/// official view has no phase: a decision offering buys is a Buy phase move.
fn treasure_plays(
    d: &dominion_protocol::AgentDecision,
    is_treasure: impl Fn(&str) -> bool,
) -> Vec<(u32, String)> {
    if !d.options.iter().any(|o| o.action == "buy") {
        return vec![];
    }
    d.options
        .iter()
        .filter(|o| o.action == "play")
        .filter_map(|o| Some((o.index, o.card.clone()?)))
        .filter(|(_, card)| is_treasure(card))
        .collect()
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
        if self.managed {
            return self
                .failure("Manage this connection through the Arena official-game controls".into());
        }
        if let Err(e) = self.client.new_game(p).await {
            return self.failure(e);
        }
        match self.client.wait(60).await {
            Ok(update) => self.observation(update, true),
            Err(e) => self.failure(e),
        }
    }
    #[tool(
        description = "Autoplay Treasures: in a Buy phase move, play the Treasures it offers one after another, instead of answering once per Treasure. Plays only the Treasures offered when called, and stops early when you are asked anything else (a Treasure's own question). Returns your next view. Token is ignored."
    )]
    async fn play_treasures(
        &self,
        Parameters(p): Parameters<crate::mcp::PlayTreasuresParams>,
    ) -> CallToolResult {
        let mut update = self.client.current();
        let mut left = match update.view() {
            Ok(Some(view)) => match view.decision.as_ref() {
                Some(d) if d.id == p.decision_id => self.treasure_plays(&update, d),
                Some(d) => {
                    return self.failure(format!(
                        "REJECTED: decision {} is not open; the open one is {}",
                        p.decision_id, d.id
                    ));
                }
                None => return self.failure("REJECTED: no decision is open for you".into()),
            },
            Ok(None) => return self.failure("REJECTED: no game".into()),
            Err(e) => return self.failure(e),
        };
        if left.is_empty() {
            return self.failure(
                "REJECTED: no Treasures to play: the decision is not a Buy phase move offering one"
                    .into(),
            );
        }
        let mut played = 0;
        while let Ok(Some(view)) = update.view() {
            let Some(d) = view.decision.as_ref() else {
                break;
            };
            let Some((index, card)) = self
                .treasure_plays(&update, d)
                .into_iter()
                .find(|(_, card)| left.iter().any(|(_, c)| c == card))
            else {
                break;
            };
            let at = left
                .iter()
                .position(|(_, c)| *c == card)
                .expect("found above");
            left.remove(at);
            let answer = Answer {
                decision: d.id.clone(),
                indices: Some(vec![index]),
                name: None,
            };
            if let Err(e) = self.client.answer(answer, Some(vec![card])).await {
                return self.failure(e);
            }
            update = match self.client.wait(30).await {
                Ok(update) => update,
                Err(e) => return self.failure(e),
            };
            played += 1;
        }
        let mut result = self.observation(update, false);
        let note = format!(
            "Played {}.",
            dominion_engine::amount(played, "Treasure", "Treasures")
        );
        result.content.insert(0, ContentBlock::text(note));
        result
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
        if self.managed {
            return self
                .failure("Manage this connection through the Arena official-game controls".into());
        }
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
        if self.managed {
            return self
                .failure("Manage this connection through the Arena official-game controls".into());
        }
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

#[cfg(test)]
mod tests {
    use dominion_protocol::{AgentDecision, AgentOption};

    fn option(index: u32, action: &str, card: &str) -> AgentOption {
        AgentOption {
            index,
            group: 0,
            action: action.into(),
            label: format!("{action} {card}"),
            card: Some(card.into()),
            zone: None,
            cost: None,
            value: None,
            player: None,
        }
    }

    fn decision(options: Vec<AgentOption>) -> AgentDecision {
        AgentDecision {
            id: "d1".into(),
            player: None,
            prompt: String::new(),
            kind: "choose".into(),
            purpose: String::new(),
            source: None,
            phase: None,
            ordered: false,
            name_any: false,
            min: None,
            max: None,
            groups: vec![],
            options,
        }
    }

    #[test]
    fn treasures_are_played_only_from_a_buy_phase_move() {
        let treasure = |c: &str| matches!(c, "Copper" | "Silver" | "Crown");
        let buy = decision(vec![
            option(0, "play", "Copper"),
            option(1, "play", "Silver"),
            option(2, "buy", "Province"),
        ]);
        assert_eq!(
            super::treasure_plays(&buy, treasure),
            [(0, "Copper".to_string()), (1, "Silver".to_string())]
        );
        // An Action phase move playing Crown, an Action-Treasure, offers no buys.
        let action = decision(vec![
            option(0, "play", "Crown"),
            option(1, "play", "Village"),
        ]);
        assert!(super::treasure_plays(&action, treasure).is_empty());
    }
}
