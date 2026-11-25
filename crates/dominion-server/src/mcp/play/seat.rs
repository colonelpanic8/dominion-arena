//! dominion/play@1 over one seat, whatever keeps its game: Arena's hub, or an official
//! account's connection, which only relays dominion.games. A backend says how its seat
//! is observed, answered and waited for ([`PlaySeat`]); what the tools check, return and
//! say is here ([`Play`]), once for all of them.

use super::{LogParams, Presentation, StepParams, TurnsParams};
use crate::hub::{PlayObservation, Seat};
use dominion_agent::{AgentDecision, Answer, GlossaryEntry};
use rmcp::model::{CallToolResult, ContentBlock};
use std::collections::BTreeSet;
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

/// Cards as an agent reads a list of them: "5 Copper, 1 Silver", in the order offered.
fn tally<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let mut counts: Vec<(&str, u32)> = vec![];
    for name in names {
        match counts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, count)) => *count += 1,
            None => counts.push((name, 1)),
        }
    }
    counts
        .iter()
        .map(|(name, count)| format!("{count} {name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What printed $ the Treasures named give, and whether any gives an amount its text
/// decides instead.
fn printed_coins<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let (mut coins, mut more) = (0u32, false);
    for name in names {
        match dominion_engine::Card::from_name(name).map(|c| c.def()) {
            Some(def) if def.coins > 0 || def.plus.coins > 0 => {
                coins += u32::from(def.coins) + u32::from(def.plus.coins);
            }
            _ => more = true,
        }
    }
    if more {
        format!("${coins} or more")
    } else {
        format!("${coins}")
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DecisionConsequence {
    decision_id: String,
    closing_options: Vec<u32>,
    forgone_options: Vec<u32>,
    text: String,
}

/// What the options that close a window give up, said where the seat decides: a buy
/// ends Treasure plays for the turn, and ending a phase gives up the plays it still
/// offers. Both backends close these windows by the rules.
fn forecloses(
    d: &AgentDecision,
    is_treasure: impl Fn(&str) -> bool,
) -> Option<DecisionConsequence> {
    let end = d.options.iter().find(|o| o.action == "end-phase");
    let treasures = treasure_plays(d, is_treasure);
    if !treasures.is_empty() {
        let names = || treasures.iter().map(|(_, c)| c.as_str());
        let ending = end
            .map(|o| format!(" {} gives them up too.", o.label))
            .unwrap_or_default();
        return Some(DecisionConsequence {
            decision_id: d.id.clone(),
            closing_options: d
                .options
                .iter()
                .filter(|o| matches!(o.action.as_str(), "buy" | "end-phase"))
                .map(|o| o.index)
                .collect(),
            forgone_options: treasures.iter().map(|(i, _)| *i).collect(),
            text: format!(
                "Buying anything ends your Treasure plays for this turn: play the Treasures you want first (play_treasures plays them all). Unplayed Treasures: {} ({}).{ending}",
                tally(names()),
                printed_coins(names()),
            ),
        });
    }
    let end = end?;
    let plays: Vec<_> = d
        .options
        .iter()
        .filter(|o| o.action == "play" && o.card.is_some())
        .collect();
    (!plays.is_empty()).then(|| DecisionConsequence {
        decision_id: d.id.clone(),
        closing_options: vec![end.index],
        forgone_options: plays.iter().map(|o| o.index).collect(),
        text: format!(
            "{} ends this phase, giving up the plays still offered: {}.",
            end.label,
            tally(plays.iter().filter_map(|o| o.card.as_deref()))
        ),
    })
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
    pub clock_cues: bool,
}

impl<S: PlaySeat> Play<'_, S> {
    /// Adds to `d`'s prompt, for its text only, what each window-closing option gives up.
    fn annotate(&self, d: &mut AgentDecision) -> Option<DecisionConsequence> {
        let names: BTreeSet<String> = (d.options.iter())
            .filter(|o| o.action == "play")
            .filter_map(|o| o.card.clone())
            .collect();
        let names: Vec<String> = names.into_iter().collect();
        // Only a note: a lookup that fails leaves the decision as it was.
        let cards = if names.is_empty() {
            vec![]
        } else {
            self.seat.cards(&names).unwrap_or_default()
        };
        let is_treasure = |name: &str| {
            (cards.iter()).any(|e| e.name == name && e.types.iter().any(|t| t == "Treasure"))
        };
        let consequence = forecloses(d, is_treasure)?;
        d.prompt = format!("{} {}", d.prompt, consequence.text);
        Some(consequence)
    }

    /// The seat's view as a result: in full, or what changed since the session's last.
    pub fn observation(&self, full: bool) -> CallToolResult {
        let (seat, mut observation) = match self.seat.observe() {
            Ok(Some(observed)) => observed,
            Ok(None) => return no_game(&self.seat.no_game()),
            Err(e) => return self.failure(e),
        };
        let mut value = serde_json::json!({"view": observation.view});
        if let Some(note) = observation
            .view
            .decision
            .as_mut()
            .and_then(|d| self.annotate(d))
        {
            value["decisionConsequence"] = serde_json::json!(note);
        }
        let (text, glossary) = self
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .render(&seat, &observation, full, self.clock_cues);
        if let Some(clock) = &observation.clock {
            value["clock"] = serde_json::json!(clock);
        }
        if let Some(clock_info) = &observation.clock_info {
            value["clockInfo"] = serde_json::json!(clock_info);
        }
        if !glossary.is_empty() {
            value["glossary"] = serde_json::json!(glossary);
        }
        let mut result = structured(value, text);
        if !observation.prompts.is_empty() {
            result.meta.get_or_insert_with(Default::default).insert(
                "dominion/modelPrompts".into(),
                serde_json::json!(observation.prompts),
            );
        }
        result
    }

    /// A failed call: why, then the decision still open, so it can be answered again
    /// without another call.
    pub fn failure(&self, message: impl Into<String>) -> CallToolResult {
        let mut text = message.into();
        let mut observation = self.seat.observe().ok().flatten().map(|(_, obs)| obs);
        let consequence = observation
            .as_mut()
            .and_then(|obs| obs.view.decision.as_mut())
            .and_then(|d| self.annotate(d));
        let open = observation.as_ref().and_then(|obs| {
            let rendered = dominion_agent::text::render(&obs.view);
            let decision = rendered.find("DECISION ").map(|i| &rendered[i..])?;
            if !self.clock_cues {
                return Some(decision.to_owned());
            }
            let cue = super::clock_cue(&obs.view, obs.clock.as_ref(), obs.clock_info.as_ref());
            Some(match cue {
                Some(cue) => format!("{cue}\n{decision}"),
                None => decision.to_string(),
            })
        });
        match open {
            Some(open) => text = format!("{text}\n{open}"),
            None if observation.is_some() && text.starts_with("REJECTED") => {
                text.push_str("\nNo decision is open for you; call wait.");
            }
            None => {}
        }
        let mut result = failure(text);
        if let Some(note) = consequence {
            result.structured_content = Some(serde_json::json!({"decisionConsequence": note}));
        }
        result
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

    /// The Buy phase decision of the Scouting Party misplay: five Coppers unplayed.
    #[test]
    fn a_buy_says_it_ends_treasure_plays_and_what_is_unplayed() {
        let treasure = |c: &str| c == "Copper";
        let mut options = vec![
            option(0, "buy", "Coven"),
            option(1, "buy", "Scouting Party"),
        ];
        options.extend((2..7).map(|i| option(i, "play", "Copper")));
        let mut end = option(7, "end-phase", "");
        end.label = "End Buys".into();
        end.card = None;
        options.push(end);
        assert_eq!(
            super::forecloses(&decision(options), treasure)
                .unwrap()
                .text,
            "Buying anything ends your Treasure plays for this turn: play the Treasures you want first (play_treasures plays them all). Unplayed Treasures: 5 Copper ($5). End Buys gives them up too."
        );
    }

    #[test]
    fn ending_a_phase_says_which_plays_it_gives_up() {
        let mut end = option(1, "end-phase", "");
        end.label = "End Actions".into();
        let d = decision(vec![option(0, "play", "Vassal"), end.clone()]);
        assert_eq!(
            super::forecloses(&d, |_| false).unwrap().text,
            "End Actions ends this phase, giving up the plays still offered: 1 Vassal."
        );
        // Nothing is given up where nothing else is offered.
        assert!(super::forecloses(&decision(vec![end]), |_| false).is_none());
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
