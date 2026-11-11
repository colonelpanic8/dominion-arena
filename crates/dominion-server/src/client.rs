//! A thin terminal client: prints a seat's text view and reads option numbers.

use crate::hub::NewGame;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// A tool call's outcome: its text content, structured content, and whether it failed.
#[derive(Clone, Debug)]
pub struct Reply {
    pub ok: bool,
    pub text: String,
    pub json: Option<Value>,
}

pub struct Client {
    service: RunningService<RoleClient, ()>,
}

impl Client {
    pub async fn connect(url: &str) -> Result<Client, String> {
        let transport = StreamableHttpClientTransport::from_uri(url.to_string());
        let service = ().serve(transport).await.map_err(|e| format!("connecting to {url}: {e}"))?;
        Ok(Client { service })
    }

    pub async fn call(&self, tool: &'static str, args: Value) -> Result<Reply, String> {
        let Value::Object(args) = args else {
            return Err("tool arguments must be an object".into());
        };
        let result = self
            .service
            .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
            .await
            .map_err(|e| format!("{tool}: {e}"))?;
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

/// Plays one seat from the terminal: `new` creates a game and takes its first seat;
/// otherwise `seat` is (game id, token).
pub async fn play(
    url: &str,
    new: Option<NewGame>,
    seat: Option<(String, String)>,
) -> Result<(), String> {
    let client = Client::connect(url).await?;
    let token = match (new, seat) {
        (Some(req), _) => {
            let reply = client
                .call("create_game", serde_json::to_value(&req).unwrap())
                .await?;
            if !reply.ok {
                return Err(reply.text);
            }
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
            seats
                .first()
                .and_then(|s| s["token"].as_str())
                .ok_or("create_game returned no seats")?
                .to_string()
        }
        (None, Some((game, token))) => {
            let reply = client
                .call("join_game", json!({ "game_id": game, "token": token }))
                .await?;
            if !reply.ok {
                return Err(reply.text);
            }
            token
        }
        (None, None) => return Err("give --new, or --game and --token".into()),
    };
    let stdin = std::io::stdin();
    let mut waiting_shown = false;
    loop {
        let reply = client
            .call(
                "wait_for_decision",
                json!({ "token": token, "timeout_ms": 60_000 }),
            )
            .await?;
        if !reply.ok {
            return Err(reply.text);
        }
        let view = reply.json.unwrap_or(Value::Null);
        if view["game"]["running"] == Value::Bool(false) {
            println!("{}", reply.text);
            break;
        }
        let Some(decision) = view["decision"].as_object() else {
            if !waiting_shown {
                println!("Waiting for the other players...");
                waiting_shown = true;
            }
            continue;
        };
        waiting_shown = false;
        let id = decision["id"].as_str().unwrap_or_default().to_string();
        println!("\n{}", reply.text);
        loop {
            print!("> ");
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            if stdin
                .lock()
                .read_line(&mut line)
                .map_err(|e| e.to_string())?
                == 0
            {
                client.close().await;
                return Ok(());
            }
            let args = match parse(&line) {
                Ok(Input::Quit) => {
                    client.close().await;
                    return Ok(());
                }
                Ok(Input::Indices(indices)) => {
                    json!({ "token": token, "decision_id": id, "indices": indices })
                }
                Ok(Input::Name(name)) => json!({ "token": token, "decision_id": id, "name": name }),
                Err(e) => {
                    println!("{e}; type option numbers (space separated), `name <card>`, or q");
                    continue;
                }
            };
            let reply = client.call("answer", args).await?;
            if reply.ok {
                break;
            }
            println!("{}", reply.text);
        }
    }
    client.close().await;
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
