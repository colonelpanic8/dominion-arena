use super::runtime::{Client, NewGame, glossary};
use crate::hub::{PlayObservation, Seat};
use crate::mcp::play::{
    self, CardsParams, LogParams, Play, PlaySeat, StateParams, StepParams, TurnsParams, WaitParams,
};
use dominion_protocol::{Answer, GlossaryEntry};
use rmcp::{
    ServerHandler, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_handler,
    tool_router,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Session {
    client: Client,
    managed: bool,
    presentation: Arc<Mutex<play::Presentation>>,
}

const MANAGED: &str = "Manage this connection through the Arena official-game controls";

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

    /// The full view once a lifecycle request has taken effect.
    async fn settled(&self, requested: Result<(), String>) -> CallToolResult {
        let waited = match requested {
            Ok(()) => self.client.wait(30).await.map(drop),
            Err(e) => Err(e),
        };
        match waited {
            Ok(()) => self.play().observation(true),
            Err(e) => self.play().failure(e),
        }
    }

    /// The play@1 tools of the account's seat, as this session is shown them.
    fn play(&self) -> Play<'_, Session> {
        Play {
            seat: self,
            shown: &self.presentation,
        }
    }
}

/// The account's seat in its official game: everything is relayed to dominion.games.
impl PlaySeat for Session {
    fn observe(&self) -> Result<Option<(Seat, PlayObservation)>, String> {
        let update = self.client.current();
        let Some(view) = update.view()? else {
            return Ok(None);
        };
        let Some(seat) = view.seat else {
            return Err("REFUSED: spectator gameplay is unsupported".into());
        };
        let names: Vec<_> = dominion_agent::glossary_for(&view)
            .into_iter()
            .map(|e| e.name)
            .collect();
        let seat = Seat {
            game: view.game.id.clone().unwrap_or_default(),
            seat: dominion_engine::PlayerId(seat),
        };
        let observation = PlayObservation {
            log: view.since_last_decision.clone(),
            glossary: glossary(&update.reducer, &names)?,
            undos: update.reducer.epoch,
            note: None,
            clock: update.clock(crate::hub::now_ms()),
            view,
        };
        Ok(Some((seat, observation)))
    }

    async fn answer(&self, answer: &Answer, expect: Option<&[String]>) -> Result<(), String> {
        let expect = expect.map(<[String]>::to_vec);
        self.client.answer(answer.clone(), expect).await
    }

    async fn wait(&self, timeout: Duration) -> Result<(), String> {
        self.client.wait(timeout.as_secs()).await.map(drop)
    }

    fn cards(&self, names: &[String]) -> Result<Vec<GlossaryEntry>, String> {
        glossary(&self.client.current().reducer, names)
    }

    fn no_game(&self) -> String {
        if self.managed {
            "its owner starts one from Arena's official-game page".into()
        } else {
            "call new_game for an unrated game against official server bots".into()
        }
    }
}

#[tool_router]
impl Session {
    #[tool(
        description = "Full official-server player view and current decision, through dominion/play@1. Call first. Token is ignored for this one-account backend."
    )]
    async fn state(&self, Parameters(_): Parameters<StateParams>) -> CallToolResult {
        self.play().observation(true)
    }
    #[tool(
        description = "Answer the official decision, then wait. Supply expect with each chosen option label/card. Ordered indices are top first. Stale or invalid answers change nothing. Token is ignored."
    )]
    async fn step(&self, Parameters(p): Parameters<StepParams>) -> CallToolResult {
        self.play().step(p).await
    }
    #[tool(
        description = "Wait for your next official decision without answering. timeout_s defaults to 45, maximum 600. Token is ignored."
    )]
    async fn wait(&self, Parameters(p): Parameters<WaitParams>) -> CallToolResult {
        self.play().wait(p.timeout_s).await
    }
    #[tool(
        description = "Start an unrated official-server game against server bots. players defaults to 2 and bots to 1; bots must equal players minus one. Optional kingdom and landscapes are required card names. No humans or spectators. Never replaces or resigns an existing game. Settings the backend cannot honor are refused."
    )]
    async fn new_game(&self, Parameters(p): Parameters<NewGame>) -> CallToolResult {
        if self.managed {
            return self.play().failure(MANAGED);
        }
        match self.client.new_game(p).await {
            Ok(()) => match self.client.wait(60).await {
                Ok(_) => self.play().observation(true),
                Err(e) => self.play().failure(e),
            },
            Err(e) => self.play().failure(e),
        }
    }
    #[tool(
        description = "Autoplay Treasures: in a Buy phase move, play the Treasures it offers one after another, instead of answering once per Treasure. Plays only the Treasures offered when called, and stops early when you are asked anything else (a Treasure's own question). Returns your next view. Token is ignored."
    )]
    async fn play_treasures(
        &self,
        Parameters(p): Parameters<crate::mcp::PlayTreasuresParams>,
    ) -> CallToolResult {
        self.play().play_treasures(&p.decision_id).await
    }
    #[tool(description = "Official card text and current costs for named cards.")]
    async fn cards(&self, Parameters(p): Parameters<CardsParams>) -> CallToolResult {
        self.play().cards(&p.names)
    }
    #[tool(
        description = "Reread the official game log. after is an exclusive sequence number. Keep newest limit entries (default 50, at most 1000), returned oldest first."
    )]
    async fn log(&self, Parameters(p): Parameters<LogParams>) -> CallToolResult {
        self.play().log(&p)
    }
    #[tool(
        description = "The official game by turn: one line per turn saying what each player played, bought, gained, trashed, discarded, ... in it, with its log seqs. Turn numbers are each player's own count (`from`, `to`; `player` is a seat index, 0 is P1); with neither from nor to, the newest `last` turns (default 6). `detail` adds each turn's log entries."
    )]
    async fn turns(&self, Parameters(p): Parameters<TurnsParams>) -> CallToolResult {
        self.play().turns(&p)
    }
    #[tool(
        description = "Explicitly resign this account's current official-server game. Token is ignored."
    )]
    async fn resign(&self, Parameters(_): Parameters<StateParams>) -> CallToolResult {
        if self.managed {
            return self.play().failure(MANAGED);
        }
        self.settled(self.client.resign().await).await
    }
    #[tool(
        description = "Reconnect only this process's own official-server session. Invalidates prior decisions, does not resend actions, and never imports a browser session or removes another login."
    )]
    async fn online_reconnect(&self) -> CallToolResult {
        if self.managed {
            return self.play().failure(MANAGED);
        }
        self.settled(self.client.reconnect().await).await
    }
}
#[tool_handler(
    name = "dominion",
    instructions = "Play through dominion/play@1: state, new_game, then step with decision and indices (top first) or name where nameAny; give expect with each selected label/card. When decision is null call wait. Results carry full {view,glossary?}; step/wait text shows changes. Stop at game.running=false. This is the official dominion.games backend, not Arena's hosted engine. It supports only this account versus server bots, unrated, spectators prohibited. Lifecycle authentication is configured by the operator before this MCP starts. No seat token is needed. Unsupported official protocol features fail clearly; never retry a sent answer after reconnect. online_reconnect only resumes this process's own session and replaces the snapshot. cards, log and turns provide references. play_treasures plays every Treasure offered in one call."
)]
impl ServerHandler for Session {}

#[cfg(test)]
mod tests {
    #[test]
    fn an_official_seat_has_every_tool_seats_are_offered() {
        let router = super::Session::tool_router();
        for tool in crate::harness::GAME_TOOLS {
            assert!(router.get(tool).is_some(), "{tool}");
        }
    }
}
