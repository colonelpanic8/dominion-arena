use super::{Advice, Note, analyze};
use crate::runner::provider::AnalysisModel;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::sync::watch;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct Position {
    pub turn: u64,
    seat: u64,
    #[serde(rename = "activePlayer")]
    active_player: Option<u64>,
    buys: Option<u64>,
    coins: Option<u64>,
    scores: BTreeMap<u64, Option<i64>>,
    piles: BTreeMap<String, u64>,
    #[serde(rename = "recentPublicBuys")]
    recent_buys: Vec<PublicBuy>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct PublicBuy {
    seat: u64,
    card: String,
}

impl Position {
    pub fn read(view: &Value) -> Option<Self> {
        let players = std::iter::once(&view["you"]).chain(view["opponents"].as_array()?.iter());
        Some(Self {
            turn: view["turn"]["number"].as_u64()?,
            seat: view["seat"].as_u64()?,
            active_player: view["turn"]["player"].as_u64(),
            buys: view["turn"]["buys"].as_u64(),
            coins: view["turn"]["coins"].as_u64(),
            recent_buys: recent_buys(&view["sinceLastDecision"]),
            scores: players
                .map(|p| Some((p["index"].as_u64()?, p["vp"].as_i64())))
                .collect::<Option<_>>()?,
            piles: view["supply"]
                .as_array()?
                .iter()
                .map(|p| Some((p["card"].as_str()?.to_owned(), p["count"].as_u64()?)))
                .collect::<Option<_>>()?,
        })
    }
    fn endgame_changed(&self, current: &Self) -> bool {
        self.piles
            .iter()
            .any(|(card, count)| current.piles.get(card).map(|v| v.min(&2)) != Some(count.min(&2)))
    }
    pub fn still_relevant(&self, current: &Self) -> bool {
        self.seat == current.seat
            && (self.turn..=self.turn.saturating_add(1)).contains(&current.turn)
            && self.scores == current.scores
            && !self.endgame_changed(current)
    }
    fn due(&self, previous: Option<&Self>) -> bool {
        self.turn >= 5
            && previous.is_none_or(|p| {
                self.turn >= p.turn.saturating_add(4)
                    || (self.turn > p.turn && p.endgame_changed(self))
            })
    }
}

pub(super) fn strategic_decision(view: &Value) -> bool {
    view["turn"]["yourTurn"] == true
        && view["decision"]["options"]
            .as_array()
            .is_some_and(|options| {
                options
                    .iter()
                    .any(|o| matches!(o["action"].as_str(), Some("play" | "buy")))
            })
}

fn recent_buys(log: &Value) -> Vec<PublicBuy> {
    let mut buys: Vec<_> = log
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .filter_map(|e| {
            if !matches!(
                e["type"].as_str()?,
                "buy" | "official-buy" | "official-buy-and-gain"
            ) {
                return None;
            }
            Some(PublicBuy {
                seat: e["player"].as_u64()?,
                card: e["card"].as_str()?.to_owned(),
            })
        })
        .take(24)
        .collect();
    buys.reverse();
    buys
}

const STRATEGY_BRIEF: &str = "Reassess the strategy for position.seat using this public position. Give the player a strategic update if the position changes the plan. Current turn coins exclude unplayed Treasures. Attribute recent public buys to their seats. The list can be partial. Buying a card does not prove it is in play. VP can be unknown. You cannot see hands or available moves. Return STRATEGY NOTE: followed by at most 60 words of conditional strategic advice. Return only NO CHANGE if no update is worth the player's attention.";

pub(super) async fn run(
    mut updates: watch::Receiver<Option<Position>>,
    advice: &Advice,
    game: &str,
    setup: &Value,
    model: &AnalysisModel,
    dir: &Path,
) {
    let mut previous = None;
    let mut last_note = advice.note_for(game);
    while updates.changed().await.is_ok() {
        let Some(position) = updates.borrow_and_update().clone() else {
            continue;
        };
        if !position.due(previous.as_ref()) {
            continue;
        }
        let prompt = json!({"setup":setup,"position":position,"previousAdvice":last_note});

        let logdir = dir.with_file_name(format!(
            "{}-turn-{}",
            dir.file_name().unwrap_or_default().to_string_lossy(),
            position.turn
        ));
        match analyze(model, &logdir, prompt, STRATEGY_BRIEF).await {
            Ok(note) if note.trim_end_matches(['.', '!']).trim() != "NO CHANGE" => {
                let current = updates.borrow().clone();
                if current.as_ref().is_some_and(|p| position.still_relevant(p)) {
                    last_note = Some(note.clone());
                    advice.publish(Note {
                        game: game.into(),
                        text: note,
                        position: Some(position.clone()),
                    });
                }
            }
            Err(e) => eprintln!("dominion agents: strategy update for {game}: {e}"),
            _ => {}
        }
        previous = Some(position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserve_strategy_notes_for_the_players_own_play_or_buy() {
        let mut view = json!({"turn":{"yourTurn":false},"decision":{"options":[{"action":"buy"}]}});
        assert!(!strategic_decision(&view));
        view["turn"]["yourTurn"] = json!(true);
        assert!(strategic_decision(&view));
        view["decision"]["options"][0]["action"] = json!("select");
        assert!(!strategic_decision(&view));
    }

    #[test]
    fn only_public_purchases_reach_the_strategy_worker() {
        let buys = recent_buys(&json!([
            {"type":"official-draw","player":1,"card":"Secret"},
            {"type":"official-buy-and-gain","player":0,"card":"Search","text":"unused"},
            {"type":"buy","player":1,"card":"Riverboat"}]));
        assert_eq!(
            serde_json::to_value(buys).unwrap(),
            json!([{"seat":0,"card":"Search"},{"seat":1,"card":"Riverboat"}])
        );
    }

    #[test]
    fn cadence_and_staleness_follow_turns_scores_and_piles() {
        let p = Position {
            recent_buys: Vec::new(),
            turn: 5,
            seat: 0,
            active_player: Some(0),
            buys: Some(1),
            coins: Some(0),
            scores: BTreeMap::from([(0, Some(3))]),
            piles: BTreeMap::from([("Province".into(), 3)]),
        };
        let mut next = p.clone();
        assert!(p.due(None));
        assert!(!next.due(Some(&p)));
        next.turn = 6;
        assert!(p.still_relevant(&next));
        assert!(!next.due(Some(&p)));
        next.scores.insert(0, Some(9));
        assert!(!p.still_relevant(&next));
        next.piles.insert("Province".into(), 1);
        assert!(next.due(Some(&p)));
        next = p.clone();
        next.turn = 9;
        assert!(next.due(Some(&p)));
        assert!(!p.still_relevant(&next));
    }
}
