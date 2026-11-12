//! The MCP tools: one `Session` per MCP session, sharing the `Hub`.

use crate::hub::{Hub, HubError, NewGame, Seat, panic_message};
use dominion_agent::{AgentView, Answer, WayChoice};
use futures_util::FutureExt;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The longest `wait_for_decision` waits, whatever it is asked for.
pub const MAX_WAIT: Duration = Duration::from_secs(600);

/// How a view is returned as text content.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// The compact text rendering.
    #[default]
    Text,
    /// The view's JSON.
    Json,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JoinParams {
    pub game_id: String,
    /// The seat token create_game returned.
    pub token: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewParams {
    /// Your seat token (optional in an MCP session that has called join_game).
    pub token: Option<String>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitParams {
    pub token: Option<String>,
    /// How long to wait, in milliseconds (default 30000, at most 600000).
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerParams {
    pub token: Option<String>,
    /// The open decision's id, from the view.
    pub decision_id: String,
    /// Option indices: one for most decisions; for ordered decisions, top first.
    pub indices: Option<Vec<u32>>,
    /// A card name, instead of indices, when the decision allows naming any card.
    pub name: Option<String>,
    /// Answers the Way question in advance: "none" plays the card as itself, a Way's name
    /// uses that Way. Applies to each Way question this answer leads to (a Throne Room's
    /// replays too) and is ignored when none does. A Way that isn't in the game rejects
    /// the answer.
    pub way: Option<String>,
    #[serde(default)]
    pub format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardParams {
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("properties" = {}))]
pub struct NoParams {}

#[derive(Clone)]
pub struct Session {
    hub: Arc<Hub>,
    joined: Arc<Mutex<Option<Seat>>>,
    max_wait: Duration,
}

fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn structured(value: serde_json::Value, text: String) -> CallToolResult {
    let mut result = CallToolResult::structured(value);
    result.content = vec![ContentBlock::text(text)];
    result
}

fn view_result(view: &AgentView, text: String, format: Format) -> CallToolResult {
    let json = serde_json::to_value(view).expect("views serialize");
    let text = match format {
        Format::Text => text,
        Format::Json => serde_json::to_string_pretty(&json).expect("views serialize"),
    };
    structured(json, text)
}

impl Session {
    /// `max_wait` caps `wait_for_decision`; it must be shorter than the time an idle
    /// session is kept, or a waiting call would outlive its session.
    pub fn new(hub: Arc<Hub>, max_wait: Duration) -> Session {
        Session {
            hub,
            joined: Arc::new(Mutex::new(None)),
            max_wait: max_wait.min(MAX_WAIT),
        }
    }

    fn joined(&self) -> std::sync::MutexGuard<'_, Option<Seat>> {
        self.joined.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn seat(&self, token: Option<&str>) -> Result<Seat, HubError> {
        match token {
            Some(token) => self.hub.find(token),
            None => self.joined().clone().ok_or_else(|| {
                HubError::BadRequest(
                    "this MCP session has not joined a seat (a join lasts until the session \
                     ends: a server restart or a long idle spell ends it): pass your seat \
                     token, or call join_game again"
                        .into(),
                )
            }),
        }
    }

    fn view_of(&self, seat: &Seat, format: Format) -> CallToolResult {
        match self.hub.view(seat) {
            Ok((view, text)) => view_result(&view, text, format),
            Err(e) => failure(e.to_string()),
        }
    }
}

#[tool_router]
impl Session {
    #[tool(
        description = "Create a game. Give `players` and optionally `kingdom` (card names), `landscapes`, `sets` (to deal a random kingdom from), `seed`, `turn_limit`, `names`. Returns the game id and one secret token per seat; hand each token only to whoever plays that seat."
    )]
    async fn create_game(
        &self,
        Parameters(req): Parameters<NewGame>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.hub.create(&req) {
            Ok(created) => {
                let mut text = format!("Created game {}.", created.game_id);
                for s in &created.seats {
                    text.push_str(&format!(
                        "\nseat {} ({}): token {}",
                        s.seat, s.name, s.token
                    ));
                }
                structured(serde_json::to_value(&created).unwrap(), text)
            }
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(
        description = "Bind this MCP session to the seat a token claims, so later calls in it may omit the token. The binding ends with the session (a server restart, or a long idle spell); then join again or pass the token. Returns that seat's view."
    )]
    async fn join_game(
        &self,
        Parameters(p): Parameters<JoinParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.hub.claim(&p.game_id, &p.token) {
            Ok(seat) => {
                *self.joined() = Some(seat.clone());
                self.view_of(&seat, Format::Text)
            }
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(description = "Your seat's current view of the game (agent-view@2).")]
    async fn view(
        &self,
        Parameters(p): Parameters<ViewParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match self.seat(p.token.as_deref()) {
            Ok(seat) => self.view_of(&seat, p.format),
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(
        description = "Wait until your seat must decide or the game ends (or the timeout passes), then return your view. Its `decision` is null if you don't have to decide yet."
    )]
    async fn wait_for_decision(
        &self,
        Parameters(p): Parameters<WaitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(30_000)).min(self.max_wait);
        if let Err(e) = self.hub.wait(&seat, timeout).await {
            return Ok(failure(e.to_string()));
        }
        Ok(self.view_of(&seat, p.format))
    }

    #[tool(
        description = "Answer your open decision with option indices (or `name` where any card may be named). In a game with Ways, `way` (\"none\" or a Way's name) also answers the \"play it as itself, or use a Way\" question for the card your answer plays, saving a call. Returns your next view, or why the answer was rejected."
    )]
    async fn answer(
        &self,
        Parameters(p): Parameters<AnswerParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = match self.seat(p.token.as_deref()) {
            Ok(seat) => seat,
            Err(e) => return Ok(failure(e.to_string())),
        };
        let way = match p.way.as_deref().map(WayChoice::parse).transpose() {
            Ok(way) => way,
            Err(e) => return Ok(failure(format!("rejected: {e}"))),
        };
        let answer = Answer {
            decision: p.decision_id,
            indices: p.indices,
            name: p.name,
        };
        Ok(match self.hub.answer_with(&seat, &answer, way) {
            Ok(answered) => {
                let mut result = self.view_of(&seat, p.format);
                if let Some(note) = answered.note() {
                    result.content.insert(0, ContentBlock::text(note));
                }
                result
            }
            Err(e) => failure(format!("rejected: {e}")),
        })
    }

    #[tool(description = "A card's official text, types, cost and set, by name.")]
    async fn card(
        &self,
        Parameters(p): Parameters<CardParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(match dominion_agent::glossary(&p.name) {
            Some(entry) => {
                let text = crate::card_text(&entry);
                structured(serde_json::to_value(&entry).unwrap(), text)
            }
            None => failure(format!("no card is named {:?}", p.name)),
        })
    }

    #[tool(description = "The games on this server (no tokens).")]
    async fn list_games(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let games = self.hub.list();
        let text = if games.is_empty() {
            "No games.".to_string()
        } else {
            games
                .iter()
                .map(|g| {
                    let state = match (g.running, g.deciding) {
                        (false, _) => "over".to_string(),
                        (true, Some(p)) => format!("seat {p} to decide"),
                        (true, None) => "running".to_string(),
                    };
                    format!(
                        "{}: {} ({state}); kingdom {}",
                        g.game_id,
                        g.names.join(", "),
                        g.kingdom.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(structured(serde_json::json!({ "games": games }), text))
    }
}

#[tool_handler(
    name = "dominion",
    instructions = "Play Dominion. create_game returns a token per seat; a seat's \
token is the only way to see or act for it. Pass `token` to view, wait_for_decision and \
answer. (join_game binds a seat to this MCP session so later calls may leave the token \
out, but only until the session ends: keep the token.) Loop: \
wait_for_decision, read the view, then answer the decision with option indices (in order \
when the decision is ordered, or `name` when it says any card may be named; `way` answers \
a Way question in advance). Views are \
dominion/agent-view@2: the JSON is the structured content, the text rendering the text \
content."
)]
impl ServerHandler for Session {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = request.name.to_string();
        let router = Self::tool_router();
        answered(
            &tool,
            router.call(ToolCallContext::new(self, request, context)),
        )
        .await
    }
}

/// Runs a tool call. rmcp leaves a request whose handler panics unanswered, so a panic
/// becomes an error result.
async fn answered(
    tool: &str,
    call: impl Future<Output = Result<CallToolResponse, ErrorData>>,
) -> Result<CallToolResponse, ErrorData> {
    match AssertUnwindSafe(call).catch_unwind().await {
        Ok(result) => result,
        Err(panic) => Ok(failure(format!(
            "{tool} failed inside the server ({}). This is a bug in the server",
            panic_message(&*panic)
        ))
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_documents_the_way_argument() {
        let router = Session::tool_router();
        let tool = router.get("answer").unwrap();
        assert!(tool.description.as_deref().unwrap().contains("`way`"));
        let way = &tool.input_schema["properties"]["way"];
        assert!(
            way["description"].as_str().unwrap().contains("\"none\""),
            "{way}"
        );
    }

    #[tokio::test]
    async fn a_tool_call_that_panics_is_answered_with_an_error() {
        let answer = answered("view", async { panic!("boom") }).await;
        let Ok(CallToolResponse::Complete(result)) = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().unwrap().text.clone();
        assert_eq!(
            text,
            "view failed inside the server (boom). This is a bug in the server"
        );
    }
}
