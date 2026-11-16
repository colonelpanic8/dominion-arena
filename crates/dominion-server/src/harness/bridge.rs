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

pub const TOOLS: [&str; 5] = [
    "view",
    "card",
    "answer",
    "play_treasures",
    "wait_for_decision",
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
        info.instructions = Some("Play your assigned Dominion seat through these tools. Seat credentials are supplied by the server.".into());
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
        if tool != "card" {
            args.insert("token".into(), json!(self.token));
        }
        self.record(
            "request",
            json!({"source":"dominion", "tool":tool, "arguments":args}),
        )
        .await?;
        let reply = match tokio::time::timeout(
            std::time::Duration::from_secs(90),
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
        self.record(
            "response",
            json!({"source":"dominion", "tool":tool, "reply":reply}),
        )
        .await?;
        let mut result = CallToolResult::success(vec![ContentBlock::text(reply.text)]);
        result.structured_content = reply.json;
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
    pub async fn start(server: &str, job: &Job) -> Result<Self, String> {
        let client = Arc::new(Client::connect(server).await?);
        let (journal, records) = mpsc::channel(32);
        let bridge = SeatBridge {
            client,
            token: job.token.clone(),
            journal,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let path = format!("/seat/{:032x}/mcp", rand::random::<u128>());
        let url = format!(
            "http://{}{path}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        let service = StreamableHttpService::new(
            move || Ok(bridge.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service(&path, service);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
