//! The player owns the game bridge. Its analyst receives public strategy observations.
mod background;
mod board;
use super::provider::{AnalysisModel, AnalysisPlan};
use crate::harness::{game_state, has_decision, is_running};
use crate::runner::strategy::ModelRunnerStrategy;
use crate::{archive::Archive, client::Reply, house::Job};
use background::Position;
pub(super) use board::opening_facts;
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinSet;

struct Note {
    game: String,
    text: String,
    position: Option<Position>,
}

#[derive(Clone, Default)]
pub(crate) struct Advice {
    pending: Arc<Mutex<Option<Note>>>,
    updates: Arc<OnceLock<(String, watch::Sender<Option<Position>>)>>,
    notices: crate::runner::clock_guard::Notices,
}

impl Advice {
    /// Advice that also carries what the harness has to tell the player.
    pub(crate) fn with_notices(notices: crate::runner::clock_guard::Notices) -> Self {
        Self {
            notices,
            ..Self::default()
        }
    }

    fn publish(&self, note: Note) {
        *self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(note);
    }

    fn note_for(&self, game: &str) -> Option<String> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|note| note.game == game)
            .map(|note| note.text.clone())
    }

    pub(crate) fn deliver(&self, reply: &mut Reply) {
        if reply.json.is_some() {
            for notice in self.notices.take() {
                reply.text = format!("{notice}\n\n{}", reply.text);
            }
        }
        let Some(json) = &reply.json else {
            return;
        };
        if !reply.ok || !is_running(json) {
            return;
        }
        let view = game_state(json);
        let game = view["game"]["id"].as_str();
        let mut position = None;
        if let Some((expected, sender)) = self.updates.get()
            && Some(expected.as_str()) == game
        {
            position = Position::read(view);
            if position.is_some() {
                sender.send_if_modified(|old| {
                    if *old == position {
                        false
                    } else {
                        *old = position.clone();
                        true
                    }
                });
            }
        }
        if !has_decision(json) {
            return;
        }
        let mut slot = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(note) = slot.as_ref() else {
            return;
        };
        if Some(note.game.as_str()) != game {
            return;
        }
        if note.position.is_none() && view["turn"]["number"].as_u64().is_some_and(|turn| turn > 5) {
            *slot = None;
            return;
        }
        if note
            .position
            .as_ref()
            .is_some_and(|p| !position.as_ref().is_some_and(|v| p.still_relevant(v)))
        {
            *slot = None;
            return;
        }
        if note.position.is_some() && !background::strategic_decision(view) {
            return;
        }
        if !available_time(json, crate::hub::now_ms())
            .is_some_and(|(_, ms)| ms > crate::mcp::play::CLOCK_PRESSURE_MS)
        {
            return;
        }
        let label = note.position.as_ref().map_or_else(
            || "Opening analyst (initial kingdom)".to_string(),
            |p| format!("Strategy analyst (turn {})", p.turn),
        );
        reply.text = format!("{label}: {}\n\n{}", note.text, reply.text);
        *slot = None;
    }
}

// Native conversation storage uses the archive basename across all seats.
fn analysis_dir(archive: &Archive, role: &str) -> std::path::PathBuf {
    let attempt = archive
        .dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    archive.dir.join(format!("{role}-{attempt}"))
}

pub(super) fn start(
    tasks: &mut JoinSet<()>,
    advice: &Advice,
    plan: Option<&AnalysisPlan>,
    model: &AnalysisModel,
    job: &Job,
    reply: &Reply,
    archive: &Archive,
) -> bool {
    let (Some(config), Some(json)) = (plan, &reply.json) else {
        return false;
    };
    if job.brief.is_some() || !job.options.strategy.timed_opening(json) {
        return false;
    }
    let game = &game_state(json)["game"];
    let Some(_) = game["kingdom"].as_array().filter(|a| !a.is_empty()) else {
        return false;
    };
    if game["id"].as_str() != Some(job.game_id.as_str()) {
        return false;
    }
    let Ok(view) = serde_json::from_value::<dominion_agent::AgentView>(game_state(json).clone())
    else {
        return false;
    };
    let board = board::setup(&view);
    let analyst = model.for_model(&config.model);
    let dir = analysis_dir(archive, "analyst");
    let advice = advice.clone();
    let game = job.game_id.clone();
    let (done, opened) = oneshot::channel();
    if config.background {
        let (tx, rx) = watch::channel(None);
        let _ = advice.updates.set((game.clone(), tx));
        let (advice, game, board, analyst, dir) = (
            advice.clone(),
            game.clone(),
            board.clone(),
            analyst.clone(),
            dir.clone(),
        );
        tasks.spawn(async move {
            if opened.await.is_ok() {
                background::run(rx, &advice, &game, &board, &analyst, &dir).await;
            }
        });
    }
    tasks.spawn(async move {
        let brief = "Give the player an opening and game plan from this public setup. Return a single actionable STRATEGY NOTE: under 100 words.";
        match analyze(&analyst,&dir,board,brief).await {
            Ok(text) => advice.publish(Note {game, text, position:None}),
            Err(error) => eprintln!("dominion agents: opening analyst for {game}: {error}"),
        }
        let _ = done.send(());
    });
    true
}

pub(super) fn available_time(reply: &Value, now_ms: u64) -> Option<(u64, u64)> {
    let view = game_state(reply);
    let seat = view["seat"].as_u64()?;
    let clock = &reply["clock"];
    let info = &reply["clockInfo"];
    if clock["running"].as_u64() != Some(seat) || info["status"] != "available" {
        return None;
    }
    let seat = usize::try_from(seat).ok()?;
    let bank = clock["left"].get(seat)?.as_u64()?;
    let free = clock["free"].get(seat).and_then(Value::as_u64).unwrap_or(0);
    let as_of = info["asOfMs"].as_u64()?;
    if as_of > now_ms.saturating_add(1000) {
        return None;
    }
    let age = now_ms.saturating_sub(as_of);
    let available = bank.saturating_add(free).saturating_sub(age);
    Some((bank, available))
}

async fn analyze(
    model: &AnalysisModel,
    dir: &Path,
    board: Value,
    brief: &str,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let answer = tokio::time::timeout_at(
        deadline,
        model.analyze(dir, brief, &board.to_string(), deadline),
    )
    .await
    .map_err(|_| "analysis deadline elapsed".to_string())??;
    strategy_note(&answer)
}

fn strategy_note(answer: &str) -> Result<String, String> {
    let answer = answer.trim();
    let body = answer
        .rsplit_once("STRATEGY NOTE:")
        .map(|(_, note)| note.trim())
        .unwrap_or_else(|| {
            if answer.chars().count() > 1000 {
                answer
                    .rsplit("\n\n")
                    .find(|p| !p.trim().is_empty())
                    .unwrap_or("")
                    .trim()
            } else {
                answer
            }
        });
    let note: String = body.chars().take(1000).collect();
    if note.is_empty() {
        Err("empty opening analysis".into())
    } else {
        Ok(note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_long_analysis_delivers_its_conclusion() {
        let explanation = "Detailed evaluation. ".repeat(100);
        assert_eq!(
            strategy_note(&format!("{explanation} STRATEGY NOTE: Buy Chapel.")).unwrap(),
            "Buy Chapel."
        );
        assert_eq!(
            strategy_note(&format!("{explanation}\n\nBuy Chapel.")).unwrap(),
            "Buy Chapel."
        );
        assert!(strategy_note("STRATEGY NOTE: ").is_err());
    }

    #[test]
    fn a_late_note_reaches_one_open_decision_in_its_own_game() {
        let advice = Advice::default();
        let mut reply = Reply {
            model_prompts: vec![],
            ok: true,
            text: "decision text".into(),
            json: Some(
                json!({"clockInfo":{"status":"available","asOfMs":crate::hub::now_ms()},"clock":{"left":[120000],"running":0},"view":{"seat":0,
                    "game":{"id":"g1","running":true}, "decision":{"id":"d7","options":[3,4]}
                }}),
            ),
        };
        let decision = reply.json.clone();
        advice.deliver(&mut reply);
        assert_eq!(reply.text, "decision text");
        advice.publish(Note {
            game: "g1".into(),
            text: "Watch the Province pile.".into(),
            position: None,
        });
        for (id, running, open) in [("g2", true, true), ("g1", true, false), ("g1", false, true)] {
            let mut other = Reply {
                json: Some(json!({"view":{"game":{"id":id,"running":running},
                "decision":open.then(|| json!({"id":"d7"}))}})),
                ..reply.clone()
            };
            advice.deliver(&mut other);
            assert_eq!(other.text, "decision text");
        }
        let mut low = reply.clone();
        low.json.as_mut().unwrap()["clock"]["left"][0] = json!(5000);
        advice.deliver(&mut low);
        assert_eq!(low.text, "decision text");
        advice.deliver(&mut reply);
        assert!(
            reply
                .text
                .starts_with("Opening analyst (initial kingdom): Watch the Province pile.")
        );
        assert_eq!(reply.json, decision);
        reply.text = "next decision".into();
        advice.deliver(&mut reply);
        assert_eq!(reply.text, "next decision");
    }
}
