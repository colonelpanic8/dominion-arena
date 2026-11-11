//! A thin terminal client: prints a seat's text view and reads option numbers.

use crate::hub::NewGame;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::common::client_side_sse::FixedInterval;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

/// A tool call's outcome: its text content, structured content, and whether it failed.
#[derive(Clone, Debug)]
pub struct Reply {
    pub ok: bool,
    pub text: String,
    pub json: Option<Value>,
}

/// Why a tool call got no [`Reply`].
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    /// The server wouldn't run the call: no such tool, or arguments it doesn't take.
    #[error("{0}")]
    Refused(String),
    /// The server couldn't be reached, or went away before it answered.
    #[error("{0}")]
    Lost(String),
}

pub struct Client {
    service: RunningService<RoleClient, ()>,
}

impl Client {
    pub async fn connect(url: &str) -> Result<Client, String> {
        // A response stream that breaks is resumed a few times, not forever, so a call to
        // a server that is gone fails.
        let mut retry = FixedInterval::default();
        retry.max_times = Some(3);
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
        config.retry_config = Arc::new(retry);
        let transport = StreamableHttpClientTransport::from_config(config);
        let service = ().serve(transport).await.map_err(|e| format!("connecting to {url}: {e}"))?;
        Ok(Client { service })
    }

    pub async fn call(&self, tool: &'static str, args: Value) -> Result<Reply, CallError> {
        let Value::Object(args) = args else {
            return Err(CallError::Refused(
                "tool arguments must be an object".into(),
            ));
        };
        let result = self
            .service
            .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
            .await
            .map_err(|e| match e {
                ServiceError::McpError(e) => CallError::Refused(format!("{tool}: {}", e.message)),
                e => CallError::Lost(format!("{tool}: {e}")),
            })?;
        let text = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Reply {
            ok: result.is_error != Some(true),
            text,
            json: result.structured_content,
        })
    }

    pub async fn close(self) {
        let _ = self.service.cancel().await;
    }
}

/// What the player typed.
#[derive(Debug, PartialEq, Eq)]
enum Input {
    Indices(Vec<u32>),
    Name(String),
    Quit,
}

fn parse(line: &str) -> Result<Input, String> {
    let line = line.trim();
    if line == "q" || line == "quit" {
        return Ok(Input::Quit);
    }
    if let Some(name) = line
        .strip_prefix("name ")
        .or_else(|| line.strip_prefix('='))
    {
        return Ok(Input::Name(name.trim().to_string()));
    }
    line.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse()
                .map_err(|_| format!("{s:?} is not an option number"))
        })
        .collect::<Result<_, _>>()
        .map(Input::Indices)
}

/// How long the terminal client waits for a reply; its longest call waits a minute.
const REPLY_TIMEOUT: Duration = Duration::from_secs(90);

/// One seat's connection, which outlives a server restart: every call carries the token,
/// so a new MCP session serves as well as the lost one.
struct Seat {
    url: String,
    client: Client,
    token: String,
    reconnect: Duration,
}

impl Seat {
    /// Connects again after a call failed with `lost`, trying for `reconnect` (games
    /// outlive a restarting server).
    async fn reconnect(&mut self, lost: &str) -> Result<(), String> {
        println!(
            "Lost the server ({lost}); trying to reach it for {}s...",
            self.reconnect.as_secs()
        );
        let deadline = Instant::now() + self.reconnect;
        loop {
            let error = match Client::connect(&self.url).await {
                Ok(client) => {
                    self.client = client;
                    println!("Connected again.");
                    return Ok(());
                }
                Err(e) => e,
            };
            if Instant::now() >= deadline {
                return Err(format!("lost the server: {error}"));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// One call. No reply in `REPLY_TIMEOUT` counts as a lost server, since a server cut
    /// off without a word would otherwise be waited on forever.
    async fn call_once(&self, tool: &'static str, args: Value) -> Result<Reply, CallError> {
        match tokio::time::timeout(REPLY_TIMEOUT, self.client.call(tool, args)).await {
            Ok(reply) => reply,
            Err(_) => Err(CallError::Lost(format!("{tool}: no reply"))),
        }
    }

    /// Calls a tool that may be called twice, again after reconnecting if the server
    /// went away.
    async fn call(&mut self, tool: &'static str, mut args: Value) -> Result<Reply, String> {
        args["token"] = json!(self.token);
        loop {
            match self.call_once(tool, args.clone()).await {
                Ok(reply) if reply.ok => return Ok(reply),
                Ok(reply) => return Err(reply.text),
                Err(CallError::Refused(e)) => return Err(e),
                Err(CallError::Lost(e)) => self.reconnect(&e).await?,
            }
        }
    }

    async fn view(&mut self) -> Result<Reply, String> {
        self.call("view", json!({})).await
    }

    /// The view once this seat must decide or the game is over.
    async fn wait(&mut self) -> Result<Reply, String> {
        loop {
            let reply = self
                .call("wait_for_decision", json!({ "timeout_ms": 60_000 }))
                .await?;
            if !running(&reply) || decision_id(&reply).is_some() {
                return Ok(reply);
            }
        }
    }
}

fn running(view: &Reply) -> bool {
    view.json.as_ref().map(|v| &v["game"]["running"]) != Some(&Value::Bool(false))
}

fn decision_id(view: &Reply) -> Option<String> {
    Some(view.json.as_ref()?["decision"]["id"].as_str()?.to_string())
}

/// Plays one seat from the terminal: `new` creates a game and takes its first seat;
/// otherwise `seat` is (game id, token). After losing the server it tries to reach it
/// again for `reconnect` before giving up.
pub async fn play(
    url: &str,
    new: Option<NewGame>,
    seat: Option<(String, String)>,
    reconnect: Duration,
) -> Result<(), String> {
    let client = Client::connect(url).await?;
    let ok = |reply: Result<Reply, CallError>| match reply {
        Ok(reply) if reply.ok => Ok(reply),
        Ok(reply) => Err(reply.text),
        Err(e) => Err(e.to_string()),
    };
    let (token, joined) = match (new, seat) {
        (Some(req), _) => {
            let args = serde_json::to_value(&req).unwrap();
            let reply = ok(client.call("create_game", args).await)?;
            let created = reply.json.ok_or("create_game returned no game")?;
            let seats = created["seats"].as_array().cloned().unwrap_or_default();
            println!(
                "Game {}. You are seat 0.",
                created["game_id"].as_str().unwrap_or("?")
            );
            for s in seats.iter().skip(1) {
                println!(
                    "  seat {} ({}) joins with: --game {} --token {}",
                    s["seat"],
                    s["name"].as_str().unwrap_or("?"),
                    created["game_id"].as_str().unwrap_or("?"),
                    s["token"].as_str().unwrap_or("?")
                );
            }
            let token = seats
                .first()
                .and_then(|s| s["token"].as_str())
                .ok_or("create_game returned no seats")?
                .to_string();
            (token, None)
        }
        (None, Some((game, token))) => {
            let args = json!({ "game_id": game, "token": token });
            (token, Some(ok(client.call("join_game", args).await)?))
        }
        (None, None) => return Err("give --new, or --game and --token".into()),
    };
    let mut seat = Seat {
        url: url.to_string(),
        client,
        token,
        reconnect,
    };
    let mut view = match joined {
        Some(view) => view,
        None => seat.view().await?,
    };
    let stdin = std::io::stdin();
    'game: loop {
        println!("\n{}", view.text);
        if !running(&view) {
            break;
        }
        let Some(id) = decision_id(&view) else {
            println!("Waiting for the other players...");
            view = seat.wait().await?;
            continue;
        };
        view = loop {
            print!("> ");
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            if stdin
                .lock()
                .read_line(&mut line)
                .map_err(|e| e.to_string())?
                == 0
            {
                break 'game;
            }
            let args = match parse(&line) {
                Ok(Input::Quit) => break 'game,
                Ok(Input::Indices(indices)) => {
                    json!({ "token": seat.token, "decision_id": id, "indices": indices })
                }
                Ok(Input::Name(name)) => {
                    json!({ "token": seat.token, "decision_id": id, "name": name })
                }
                Err(e) => {
                    println!("{e}; type option numbers (space separated), `name <card>`, or q");
                    continue;
                }
            };
            match seat.call_once("answer", args).await {
                Ok(reply) if reply.ok => break reply,
                Ok(reply) => println!("{}", reply.text),
                Err(CallError::Refused(e)) => return Err(e),
                // The answer may or may not have arrived; the view tells.
                Err(CallError::Lost(e)) => seat.reconnect(&e).await?,
            }
            // The decision may have been answered from elsewhere with this token.
            let now = seat.view().await?;
            if decision_id(&now).as_ref() != Some(&id) {
                break now;
            }
        };
    }
    seat.client.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_indices_names_and_quit() {
        assert_eq!(parse("2 0,1\n"), Ok(Input::Indices(vec![2, 0, 1])));
        assert_eq!(parse("\n"), Ok(Input::Indices(vec![])));
        assert_eq!(parse("name Gold"), Ok(Input::Name("Gold".into())));
        assert_eq!(parse("q"), Ok(Input::Quit));
        assert!(parse("two").is_err());
    }
}
