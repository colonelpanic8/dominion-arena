//! dominion/play@1 over one seat, whatever keeps its game: Arena's hub, or an official
//! account's connection, which only relays dominion.games. A backend says how its seat
//! is observed, answered and waited for ([`PlaySeat`]); what the tools check, return and
//! say is here ([`Play`]), once for all of them.

use super::{LogParams, Presentation, StepParams, TurnsParams};
use crate::hub::{PlayObservation, Seat};
use dominion_agent::{AgentDecision, Answer, GlossaryEntry};
use rmcp::model::{CallToolResult, ContentBlock};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// How long `wait` waits by default: under the 60 seconds MCP clients commonly allow a
/// tool call (Claude Code, Codex), so a wait with no timeout of its own returns first.
pub const DEFAULT_WAIT_S: u64 = 45;

/// Its errors are whole messages: where the contract gives one a code (`NO_GAME`,
/// `REJECTED`, `REFUSED`) the backend puts it first.
pub(crate) trait PlaySeat {
    /// The seat as it stands, with its whole visible log. None when its backend has no
    /// game for it.
    fn observe(&self) -> Result<Option<(Seat, PlayObservation)>, String>;

    /// Answers the open decision. A refused answer changes nothing.
    async fn answer(&self, answer: &Answer, expect: Option<&[String]>) -> Result<(), String>;

    /// Waits until the seat must decide, its game ends or `timeout` passes.
    async fn wait(&self, timeout: Duration) -> Result<(), String>;

    /// Glossary entries with the costs and types the seat's game gives them now.
    fn cards(&self, names: &[String]) -> Result<Vec<GlossaryEntry>, String>;

    /// Plays the Treasures the Buy phase move `decision` offers, and says how many.
    async fn play_treasures(&self, decision: &str) -> Result<u32, String> {
        treasures_one_by_one(self, decision).await
    }

    /// What a seat without a game is told to do: "call new_game ...".
    fn no_game(&self) -> String;
}

/// The Treasures a Buy phase move offers to play, as (option index, card name). A view
/// need not say the phase: a move offering buys is a Buy phase move.
fn treasure_plays(d: &AgentDecision, is_treasure: impl Fn(&str) -> bool) -> Vec<(u32, String)> {
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

fn offered(
    seat: &(impl PlaySeat + ?Sized),
    d: &AgentDecision,
) -> Result<Vec<(u32, String)>, String> {
    let names: Vec<String> = d.options.iter().filter_map(|o| o.card.clone()).collect();
    let cards = seat.cards(&names)?;
    Ok(treasure_plays(d, |name| {
        cards
            .iter()
            .any(|e| e.name == name && e.types.iter().any(|t| t == "Treasure"))
    }))
}

/// Autoplay Treasures for a backend that only takes answers: one answer per Treasure
/// offered when called, stopping when the seat is asked anything else.
async fn treasures_one_by_one(
    seat: &(impl PlaySeat + ?Sized),
    decision: &str,
) -> Result<u32, String> {
    let open = |seat: &_| -> Result<Option<AgentDecision>, String> {
        Ok(PlaySeat::observe(seat)?.and_then(|(_, o)| o.view.decision))
    };
    let Some(d) = open(seat)? else {
        return Err("REJECTED: no decision is open for you".into());
    };
    if d.id != decision {
        return Err(format!(
            "REJECTED: decision {decision} is not open; the open decision is {}",
            d.id
        ));
    }
    let mut left = offered(seat, &d)?;
    if left.is_empty() {
        return Err(
            "REJECTED: no Treasures to play: the decision is not a Buy phase move offering one"
                .into(),
        );
    }
    let mut played = 0;
    while let Some(d) = open(seat)? {
        let Some((index, card)) = offered(seat, &d)?
            .into_iter()
            .find(|(_, card)| left.iter().any(|(_, c)| c == card))
        else {
            break;
        };
        let at = left.iter().position(|(_, c)| *c == card).expect("found");
        left.remove(at);
        let answer = Answer {
            decision: d.id.clone(),
            indices: Some(vec![index]),
            name: None,
        };
        seat.answer(&answer, Some(&[card])).await?;
        seat.wait(Duration::from_secs(30)).await?;
        played += 1;
    }
    Ok(played)
}

pub(crate) fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

pub(crate) fn structured(value: serde_json::Value, text: String) -> CallToolResult {
    let mut result = CallToolResult::structured(value);
    result.content = vec![ContentBlock::text(text)];
    result
}

/// What `state` answers where there is no game: `advice` says what to call for one.
pub(crate) fn no_game(advice: &str) -> CallToolResult {
    structured(
        serde_json::json!({"view": null}),
        format!("STATUS no game; {advice}."),
    )
}

/// What `cards` answers.
pub(crate) fn glossary_result(entries: Result<Vec<GlossaryEntry>, String>) -> CallToolResult {
    match entries {
        Ok(entries) => structured(
            serde_json::json!({"glossary": entries}),
            super::glossary_text(&entries),
        ),
        Err(e) => failure(e),
    }
}

/// The play@1 tools of one seat, for the MCP session whose `shown` remembers what it
/// was last told.
pub(crate) struct Play<'a, S> {
    pub seat: &'a S,
    pub shown: &'a Mutex<Presentation>,
}

impl<S: PlaySeat> Play<'_, S> {
    /// The seat's view as a result: in full, or what changed since the session's last.
    pub fn observation(&self, full: bool) -> CallToolResult {
        let (seat, observation) = match self.seat.observe() {
            Ok(Some(observed)) => observed,
            Ok(None) => return no_game(&self.seat.no_game()),
            Err(e) => return self.failure(e),
        };
        let (text, glossary) = self
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .render(&seat, &observation, full);
        let mut value = serde_json::json!({"view": observation.view});
        if let Some(clock) = &observation.clock {
            value["clock"] = serde_json::json!(clock);
        }
        if !glossary.is_empty() {
            value["glossary"] = serde_json::json!(glossary);
        }
        structured(value, text)
    }

    /// A failed call: why, then the decision still open, so it can be answered again
    /// without another call.
    pub fn failure(&self, message: impl Into<String>) -> CallToolResult {
        let mut text = message.into();
        let decision = match self.seat.observe() {
            Ok(Some((_, observation))) => Some(observation.view),
            _ => None,
        };
        let open = decision.as_ref().and_then(|view| {
            let rendered = dominion_agent::text::render(view);
            rendered
                .find("DECISION ")
                .map(|i| rendered[i..].to_string())
        });
        match open {
            Some(open) => text = format!("{text}\n{open}"),
            None if decision.is_some() && text.starts_with("REJECTED") => {
                text.push_str("\nNo decision is open for you; call wait.");
            }
            None => {}
        }
        failure(text)
    }

    fn rejected(&self, message: impl std::fmt::Display) -> CallToolResult {
        self.failure(format!("REJECTED: {message}"))
    }

    fn observed(&self) -> Result<(Seat, PlayObservation), CallToolResult> {
        match self.seat.observe() {
            Ok(Some(observed)) => Ok(observed),
            Ok(None) => Err(failure(format!(
                "NO_GAME: no game is running; {}",
                self.seat.no_game()
            ))),
            Err(e) => Err(self.failure(e)),
        }
    }

    pub async fn step(&self, p: StepParams) -> CallToolResult {
        let observation = match self.observed() {
            Ok((_, observation)) => observation,
            Err(e) => return e,
        };
        let Some(d) = &observation.view.decision else {
            return self.rejected("you are not deciding now; call wait");
        };
        if d.id != p.decision {
            return self.rejected(format!(
                "decision {} is not open; the open decision is {}",
                p.decision, d.id
            ));
        }
        if let Err(e) = super::validate_expect(d, &p) {
            return self.rejected(e);
        }
        let indices: Result<Option<Vec<u32>>, _> = (p.indices.as_ref())
            .map(|indices| indices.iter().map(|i| u32::try_from(*i)).collect())
            .transpose();
        let Ok(indices) = indices else {
            return self.rejected("indices must be nonnegative option numbers");
        };
        let mut chosen = indices.iter().flatten();
        if let Some(missing) = chosen.find(|i| d.options.iter().all(|o| o.index != **i)) {
            return self.rejected(format!(
                "option {missing} does not exist (decision has {} options)",
                d.options.len()
            ));
        }
        let answer = Answer {
            decision: p.decision,
            indices,
            name: p.name,
        };
        if let Err(e) = self.seat.answer(&answer, p.expect.as_deref()).await {
            return self.failure(e);
        }
        self.wait(p.timeout_s).await
    }

    pub async fn wait(&self, timeout_s: Option<u64>) -> CallToolResult {
        let timeout =
            Duration::from_secs(timeout_s.unwrap_or(DEFAULT_WAIT_S)).min(crate::mcp::MAX_WAIT);
        match self.seat.wait(timeout).await {
            Ok(()) => self.observation(false),
            Err(e) => self.failure(e),
        }
    }

    pub fn cards(&self, names: &[String]) -> CallToolResult {
        glossary_result(self.seat.cards(names))
    }

    pub fn log(&self, p: &LogParams) -> CallToolResult {
        let observation = match self.observed() {
            Ok((_, observation)) => observation,
            Err(e) => return e,
        };
        let mut entries: Vec<_> = (observation.log.into_iter())
            .filter(|e| p.after.is_none_or(|seq| e.seq > seq))
            .collect();
        entries.drain(
            ..entries
                .len()
                .saturating_sub(p.limit.unwrap_or(50).min(1000)),
        );
        let lines: Vec<_> = (entries.iter())
            .map(|e| format!("  {}: {}", e.seq, e.text))
            .collect();
        structured(
            serde_json::json!({"log": entries}),
            format!("LOG\n{}", lines.join("\n")),
        )
    }

    pub fn turns(&self, p: &TurnsParams) -> CallToolResult {
        let observation = match self.observed() {
            Ok((_, observation)) => observation,
            Err(e) => return e,
        };
        let names = dominion_agent::turns::names(&observation.view);
        let running = observation.view.game.running;
        let turns = super::turns(&observation.log, &names, running, p);
        let json: Vec<_> = turns
            .iter()
            .map(|(span, summary)| {
                let mut v = serde_json::to_value(summary).unwrap_or_default();
                if p.detail {
                    v["entries"] = serde_json::json!(span.entries);
                }
                v
            })
            .collect();
        structured(
            serde_json::json!({"turns": json}),
            super::turns_text(&turns, p.detail),
        )
    }

    pub async fn play_treasures(&self, decision: &str) -> CallToolResult {
        match self.seat.play_treasures(decision).await {
            Ok(played) => {
                let mut result = self.observation(false);
                let note = format!(
                    "Played {}.",
                    dominion_engine::amount(played, "Treasure", "Treasures")
                );
                result.content.insert(0, ContentBlock::text(note));
                result
            }
            Err(e) => self.failure(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use dominion_agent::{AgentDecision, AgentOption};

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
