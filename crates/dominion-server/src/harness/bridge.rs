use crate::runner::analyst::Advice;
use crate::{archive::Archive, client::Client, house::Job};
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// The dominion/play@1 tools a seat is offered. The legacy ones (view, answer, ...) return
/// the raw JSON view as their text, so they're left out.
pub const TOOLS: [&str; 8] = [
    "state",
    "step",
    "wait",
    "advise",
    "cards",
    "log",
    "turns",
    "play_treasures",
];

pub struct Record {
    kind: &'static str,
    data: Value,
    saved: oneshot::Sender<Result<(), String>>,
}

impl Record {
    pub fn pending_change(&self) -> i32 {
        match self.kind {
            "request" => 1,
            "response" | "transport_error" => -1,
            _ => 0,
        }
    }

    pub fn save(self, archive: &mut Archive) -> Result<(), String> {
        let result = archive.record(self.kind, self.data);
        let _ = self.saved.send(result.clone());
        result
    }
}

#[derive(Clone)]
struct SeatBridge {
    client: Arc<Client>,
    token: String,
    journal: mpsc::Sender<Record>,
    official: bool,
    advice: Advice,
}

impl SeatBridge {
    async fn record(&self, kind: &'static str, data: Value) -> Result<(), ErrorData> {
        let (saved, received) = oneshot::channel();
        self.journal
            .send(Record { kind, data, saved })
            .await
            .map_err(|_| ErrorData::internal_error("archive closed", None))?;
        received
            .await
            .map_err(|_| ErrorData::internal_error("archive closed", None))?
            .map_err(|e| ErrorData::internal_error(e, None))
    }
}

fn tools() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|name| {
            let mut tool = crate::mcp::tool_definition(name).expect("game tool");
            tool.output_schema = None;
            let schema = Arc::make_mut(&mut tool.input_schema);
            if let Some(Value::Object(properties)) = schema.get_mut("properties") {
                properties.remove("token");
            }
            if let Some(Value::Array(required)) = schema.get_mut("required") {
                required.retain(|v| v != "token");
            }
            tool
        })
        .collect()
}

impl ServerHandler for SeatBridge {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some("Play your assigned Dominion seat through dominion/play@1: state, then step with decision/indices/expect, or wait when decision is null. Results are text: what changed since your last decision, new card text, then your next decision; state shows the whole game. Stop at GAME OVER. cards, log and turns (what each turn did, by turn number) provide references. Seat credentials are supplied by the server; do not create or join another game.".into());
        info
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tools(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = TOOLS
            .iter()
            .copied()
            .find(|name| *name == request.name)
            .ok_or_else(|| ErrorData::invalid_params("tool unavailable for this seat", None))?;
        let mut args = request.arguments.unwrap_or_default();
        if args.contains_key("token") || args.contains_key("game_id") {
            return Err(ErrorData::invalid_params(
                "seat identity cannot be overridden",
                None,
            ));
        }
        if !self.official && !matches!(tool, "card" | "cards" | "log" | "turns") {
            args.insert("token".into(), json!(self.token));
        }
        self.record(
            "request",
            json!({"source":"dominion", "tool":tool, "arguments":args}),
        )
        .await?;
        let mut reply = match tokio::time::timeout(
            crate::mcp::MAX_WAIT + std::time::Duration::from_secs(30),
            self.client.call(tool, Value::Object(args)),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            result => {
                let error = match result {
                    Ok(Err(e)) => e.to_string(),
                    _ => "game tool timed out".into(),
                };
                self.record(
                    "transport_error",
                    json!({"source":"dominion", "tool":tool,"error":error}),
                )
                .await?;
                return Err(ErrorData::internal_error(error, None));
            }
        };
        self.advice.deliver(&mut reply);
        self.record(
            "response",
            json!({"source":"dominion", "tool":tool, "reply":reply}),
        )
        .await?;
        // Only the text reaches the model: the structured view repeats the whole game at
        // every step, about ten times the text's size. The archive above keeps both.
        let mut result = CallToolResult::success(vec![ContentBlock::text(reply.text)]);
        result.is_error = Some(!reply.ok);
        Ok(result.into())
    }
}

pub struct Bridge {
    pub url: String,
    pub records: Option<mpsc::Receiver<Record>>,
    task: tokio::task::JoinHandle<()>,
}

impl Bridge {
    /// Serves an already bound seat client on the sandbox socket or a loopback port.
    pub async fn start(
        client: Arc<Client>,
        job: &Job,
        socket: Option<&std::path::Path>,
        advice: Advice,
    ) -> Result<Self, String> {
        let (journal, records) = mpsc::channel(32);
        let bridge = SeatBridge {
            client: client.clone(),
            advice,
            token: job.token.clone(),
            journal,
            official: job.official,
        };
        let path = format!("/seat/{:032x}/mcp", rand::random::<u128>());
        let service = StreamableHttpService::new(
            move || Ok(bridge.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service(&path, service);
        let (url, task) = match socket {
            Some(socket) => {
                let listener = tokio::net::UnixListener::bind(socket)
                    .map_err(|e| format!("{}: {e}", socket.display()))?;
                let url = format!("http://127.0.0.1:{}{path}", super::sandbox::BRIDGE_PORT);
                (
                    url,
                    tokio::spawn(async move {
                        let _ = axum::serve(listener, router).await;
                    }),
                )
            }
            None => {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .map_err(|e| e.to_string())?;
                let url = format!(
                    "http://{}{path}",
                    listener.local_addr().map_err(|e| e.to_string())?
                );
                (
                    url,
                    tokio::spawn(async move {
                        let _ = axum::serve(listener, router).await;
                    }),
                )
            }
        };
        Ok(Self {
            url,
            records: Some(records),
            task,
        })
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}
