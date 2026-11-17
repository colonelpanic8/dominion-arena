//! Presentation and arguments for dominion/play@1; legacy tools keep their wire format.

use dominion_agent::turns::{self, TurnSpan, TurnSummary};
use dominion_agent::{AgentDecision, AgentView, GlossaryEntry};
use dominion_protocol::lobby::ClockView;
use serde::Deserialize;
use std::collections::BTreeSet;

#[cfg(test)]
mod conformance;
mod seat;
pub(crate) use seat::{Play, PlaySeat, failure, glossary_result, no_game, structured};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateParams {
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitParams {
    pub token: Option<String>,
    /// Seconds to wait (default 60, capped at 600 and the session's lifetime).
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepParams {
    pub token: Option<String>,
    pub decision: String,
    #[schemars(with = "Option<Vec<u32>>")]
    pub indices: Option<Vec<i64>>,
    pub name: Option<String>,
    /// Give each selected option's label or card, in index order, to detect mistakes.
    pub expect: Option<Vec<String>>,
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardsParams {
    pub names: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogParams {
    pub after: Option<usize>,
    /// Keep the newest entries after `after` (default 50), returned oldest first.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnsParams {
    /// Only this player's turns: their seat index, as in the view (0 is P1).
    pub player: Option<u8>,
    /// The first turn number to include: each player's own count, extra turns included.
    pub from: Option<u32>,
    /// The last turn number to include.
    pub to: Option<u32>,
    /// With neither from nor to: how many of the newest turns (default 6).
    pub last: Option<usize>,
    /// Also return each turn's log entries.
    #[serde(default)]
    pub detail: bool,
}

/// The turns `p` asks for, oldest first, with their summaries. The last turn of a
/// `running` game is the one in progress.
pub fn turns<'a>(
    log: &'a [dominion_agent::LogEntry],
    names: &[String],
    running: bool,
    p: &TurnsParams,
) -> Vec<(TurnSpan<'a>, TurnSummary)> {
    let all = turns::split(log);
    let current = all.last().map(|t| (t.player, t.number)).filter(|_| running);
    let mut spans: Vec<_> = all
        .into_iter()
        .filter(|t| p.player.is_none_or(|q| t.player == q))
        .filter(|t| p.from.is_none_or(|n| t.number >= n))
        .filter(|t| p.to.is_none_or(|n| t.number <= n))
        .collect();
    if p.from.is_none() && p.to.is_none() {
        spans.drain(..spans.len().saturating_sub(p.last.unwrap_or(6)));
    }
    spans
        .into_iter()
        .map(|t| {
            let ongoing = current == Some((t.player, t.number));
            (t, turns::summarize(t, names, ongoing))
        })
        .collect()
}

pub fn turns_text(turns: &[(TurnSpan<'_>, TurnSummary)], detail: bool) -> String {
    let mut text = String::from("TURNS");
    if turns.is_empty() {
        text.push_str("\n  none");
    }
    for (span, summary) in turns {
        text.push_str(&format!(
            "\n  [{}-{}] {}",
            summary.first_seq, summary.last_seq, summary.summary
        ));
        if detail {
            for e in span.entries {
                text.push_str(&format!("\n    {}: {}", e.seq, e.text));
            }
        }
    }
    text
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewGameParams {
    pub players: Option<u8>,
    /// Arena has no automatic bots. Only 0 (the default) is accepted; share the other
    /// seats' tokens with agents. Use create_game's explicit profiles for house agents.
    pub bots: Option<u8>,
    #[serde(default)]
    pub kingdom: Vec<String>,
    #[serde(default)]
    pub landscapes: Vec<String>,
    #[serde(default)]
    pub sets: Vec<String>,
    #[serde(default)]
    pub names: Vec<String>,
    pub seed: Option<u64>,
    pub turn_limit: Option<u32>,
    pub colonies: Option<bool>,
    pub shelters: Option<bool>,
    pub watchable: Option<bool>,
}

#[derive(Default)]
pub struct Presentation {
    pub seat: Option<crate::hub::Seat>,
    previous: Option<AgentView>,
    shown: BTreeSet<String>,
    last_log: Option<usize>,
    undos: u64,
}

fn key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn matches(expected: &str, option: &dominion_agent::AgentOption) -> bool {
    let expected = key(expected);
    !expected.is_empty()
        && (expected == key(&option.label)
            || expected == key(option.label.split(" (").next().unwrap_or(&option.label))
            || option.card.as_ref().is_some_and(|c| expected == key(c)))
}

pub fn validate_expect(d: &AgentDecision, p: &StepParams) -> Result<(), String> {
    if p.name.is_some() {
        return Ok(());
    }
    let Some(expected) = &p.expect else {
        return Ok(());
    };
    let indices = p.indices.as_deref().unwrap_or_default();
    if expected.len() != indices.len() {
        return Err("expect must have one entry per index, in the same order".into());
    }
    for (&index, expected) in indices.iter().zip(expected) {
        let Some(option) = d.options.iter().find(|o| i64::from(o.index) == index) else {
            return Err(format!(
                "option {index} does not exist (decision has {} options)",
                d.options.len()
            ));
        };
        if !matches(expected, option) {
            let hint = d
                .options
                .iter()
                .find(|o| matches(expected, o))
                .map(|o| format!(" (that is option {})", o.index))
                .unwrap_or_default();
            return Err(format!(
                "option {index} is {:?}, not {expected:?}{hint}",
                option.label
            ));
        }
    }
    Ok(())
}

pub fn glossary_text(entries: &[GlossaryEntry]) -> String {
    entries
        .iter()
        .map(|entry| crate::card_text(entry).replace('\n', " "))
        .collect::<Vec<_>>()
        .join("\n")
}

// Each top-level rendering line owns its indented details. Player headers include
// their seat, so identical zone lines in different players never cancel one another.
fn sections(text: &str) -> Vec<(String, Vec<String>)> {
    let mut result: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        if line.starts_with(' ') {
            if let Some((_, lines)) = result.last_mut() {
                lines.push(line.into());
            }
        } else {
            let section = if line.starts_with("UNCERTAIN ") {
                line.to_string()
            } else if line.starts_with("OPPONENT ") || line.starts_with("PLAYER ") {
                line.split(':')
                    .next()
                    .unwrap_or(line)
                    .split(" (")
                    .next()
                    .unwrap_or(line)
                    .to_string()
            } else {
                line.split_whitespace().next().unwrap_or(line).to_string()
            };
            result.push((section, vec![line.into()]));
        }
    }
    result
}

fn delta(previous: &AgentView, current: &AgentView) -> String {
    let old = sections(&dominion_agent::text::render(previous));
    let new = sections(&dominion_agent::text::render(current));
    let mut out = Vec::new();
    for (section, lines) in &new {
        if section == "LOG" || section == "RESULT" {
            continue;
        }
        let before = old.iter().find(|(k, _)| k == section).map(|(_, v)| v);
        if section == "GAME" || section == "DECISION" {
            out.extend(lines.iter().cloned());
            continue;
        }
        let Some(before) = before else {
            out.extend(lines.iter().cloned());
            continue;
        };
        if before != lines {
            let changed: Vec<_> = lines
                .iter()
                .skip(1)
                .filter(|l| !before.contains(l))
                .cloned()
                .collect();
            let gone: Vec<_> = before
                .iter()
                .skip(1)
                .filter(|l| !lines.contains(l))
                .map(|l| format!("  gone: {}", l.trim()))
                .collect();
            if section == "SUPPLY" || section == "NON-SUPPLY" {
                // Count changes replace a pile line rather than removing the pile.
                let piles = if section == "SUPPLY" {
                    &current.supply
                } else {
                    &current.non_supply
                };
                let previous_piles = if section == "SUPPLY" {
                    &previous.supply
                } else {
                    &previous.non_supply
                };
                let mut changes: Vec<String> =
                    changed.iter().map(|l| l.trim().to_string()).collect();
                changes.extend(
                    previous_piles
                        .iter()
                        .filter(|p| !piles.iter().any(|n| n.card == p.card))
                        .map(|p| format!("{} gone", p.card)),
                );
                out.push(format!("{section} (changed) {}", changes.join(", ")));
            } else {
                out.push(lines[0].clone());
                out.extend(changed);
                out.extend(gone);
            }
        }
    }
    for (section, lines) in old {
        if !matches!(section.as_str(), "LOG" | "DECISION" | "RESULT")
            && !new.iter().any(|(key, _)| key == &section)
        {
            out.push(format!("{} gone", lines[0]));
        }
    }
    out.join("\n")
}

/// Milliseconds as `m:ss` or `h:mm:ss`, rounded up.
fn clock_text(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// `CLOCK you 4:12 (running, +10s each turn) · Bo 6:30`: each player's time as of now.
fn clock_line(view: &AgentView, clock: &ClockView) -> String {
    let names = turns::names(view);
    let players: Vec<String> = (clock.left.iter().enumerate())
        .map(|(seat, &left)| {
            let who = if view.seat == Some(seat as u8) {
                "you".to_string()
            } else {
                names
                    .get(seat)
                    .cloned()
                    .unwrap_or_else(|| format!("P{}", seat + 1))
            };
            let mut notes = vec![];
            if clock.running == Some(seat as u8) {
                notes.push("running".to_string());
            }
            if let Some(&free) = clock.free.get(seat).filter(|&&f| f > 0) {
                notes.push(format!("{} free first", clock_text(free)));
            }
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join(", "))
            };
            format!("{who} {}{notes}", clock_text(left))
        })
        .collect();
    let increment = clock
        .increment
        .filter(|&ms| ms > 0)
        .map_or(String::new(), |ms| {
            format!("; +{}s at the start of each of your turns", ms / 1000)
        });
    format!(
        "CLOCK {}{increment}. Time runs only while you must decide; at 0:00 you forfeit.",
        players.join(" · ")
    )
}

impl Presentation {
    pub fn render(
        &mut self,
        seat: &crate::hub::Seat,
        observation: &crate::hub::PlayObservation,
        full: bool,
    ) -> (String, Vec<GlossaryEntry>) {
        let crate::hub::PlayObservation {
            view,
            log: logs,
            undos,
            glossary,
            note,
            clock,
        } = observation;
        let undos = *undos;
        if self.seat.as_ref() != Some(seat) {
            let same_game = self.seat.as_ref().is_some_and(|s| s.game == seat.game);
            let shown = if same_game {
                std::mem::take(&mut self.shown)
            } else {
                BTreeSet::new()
            };
            *self = Self {
                seat: Some(seat.clone()),
                shown,
                undos,
                ..Self::default()
            };
        } else if self.undos != undos {
            self.previous = None;
            self.last_log = None;
            self.undos = undos;
        }
        let full = full || self.previous.is_none();
        let note = note.as_ref().map(|n| format!("{n}\n")).unwrap_or_default();
        let mut text_view = view.clone();
        text_view.since_last_decision.clear();
        // LOG and RESULT are appended below, where their sequence and ending are explicit.
        text_view.game.result = None;
        text_view.decision = None;
        let mut text = if full {
            format!(
                "{note}STATUS game {}, seat {} of {}, opponents: {} other seats\n{}",
                seat.game,
                seat.seat.0,
                view.opponents.len() + 1,
                view.opponents.len(),
                dominion_agent::text::render(&text_view)
            )
        } else {
            note + &delta(self.previous.as_ref().unwrap(), &text_view)
        };
        if let Some(line) = clock
            .as_ref()
            .filter(|_| view.game.running)
            .map(|c| clock_line(view, c))
        {
            text.push('\n');
            text.push_str(&line);
        }
        let unseen: Vec<_> = logs
            .iter()
            .filter(|e| full || self.last_log.is_none_or(|seq| e.seq > seq))
            .collect();
        if unseen.len() > 50 {
            let (lo, hi) = (unseen[0].seq, unseen[unseen.len() - 51].seq);
            text.push_str(&format!(
                "\nLOG truncated: entries {lo}-{hi} omitted; call log or turns (detail) to reread them."
            ));
            let names = turns::names(view);
            let earlier: Vec<_> = turns::split(logs)
                .into_iter()
                .filter_map(|t| {
                    let continues = t.entries.last().is_some_and(|e| e.seq > hi);
                    let part = t.within(|e| (lo..=hi).contains(&e.seq))?;
                    Some(turns::summarize(part, &names, continues))
                })
                .collect();
            if !earlier.is_empty() {
                text.push_str("\nOMITTED, BY TURN");
                for t in earlier {
                    text.push_str(&format!(
                        "\n  [{}-{}] {}",
                        t.first_seq, t.last_seq, t.summary
                    ));
                }
            }
        }
        if !unseen.is_empty() {
            text.push_str("\nLOG");
            for entry in unseen.iter().skip(unseen.len().saturating_sub(50)) {
                text.push_str(&format!("\n  {}: {}", entry.seq, entry.text));
            }
        }
        self.last_log = logs.last().map(|e| e.seq);
        let rendered = dominion_agent::text::render(view);
        if let Some(index) = rendered.find("DECISION ") {
            text.push('\n');
            text.push_str(&rendered[index..]);
        }
        let fresh: Vec<_> = glossary
            .iter()
            .filter(|e| {
                !matches!(
                    e.name.as_str(),
                    "Copper"
                        | "Silver"
                        | "Gold"
                        | "Platinum"
                        | "Estate"
                        | "Duchy"
                        | "Province"
                        | "Colony"
                        | "Curse"
                )
            })
            .filter(|e| self.shown.insert(e.name.clone()))
            .cloned()
            .collect();
        if !fresh.is_empty() {
            text.push_str(&format!("\nCARDS\n{}", glossary_text(&fresh)));
        }
        if !view.game.running {
            text.push_str("\nGAME OVER");
            if let Some(result) = dominion_agent::text::render(view)
                .lines()
                .find(|l| l.starts_with("RESULT"))
            {
                text.push('\n');
                text.push_str(result);
            }
        } else if view.spectating {
            text.push_str("\nSPECTATING: nothing to answer.");
        } else if view.decision.is_none() {
            text.push_str("\nWAITING: no decision for you yet; call wait.");
        }
        self.previous = Some(text_view);
        (text, fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::{PlayObservation, Seat};
    use dominion_agent::{LogEntry, Table};
    use dominion_engine::{Card, GameConfig, PlayerId};

    fn observation() -> (Seat, PlayObservation) {
        let table = Table::new(
            "test",
            GameConfig::new(2, vec![Card::Village], 1),
            vec!["Ada".into(), "Bo".into()],
        )
        .unwrap();
        let view = table.view(PlayerId(0));
        let log = view.since_last_decision.clone();
        let glossary = dominion_agent::glossary_for(&view);
        (
            Seat {
                game: "test".into(),
                seat: PlayerId(0),
            },
            PlayObservation {
                view,
                log,
                glossary,
                undos: 0,
                note: None,
                clock: None,
            },
        )
    }

    #[test]
    fn every_rendering_of_a_clocked_game_says_each_players_time() {
        let (seat, mut observation) = observation();
        observation.clock = Some(ClockView {
            left: vec![252_000, 390_000],
            free: vec![],
            running: Some(0),
            increment: Some(10_000),
        });
        let mut presentation = Presentation::default();
        let line = "CLOCK you 4:12 (running) · Bo 6:30; +10s at the start of each of your \
                    turns. Time runs only while you must decide; at 0:00 you forfeit.";
        let (full, _) = presentation.render(&seat, &observation, false);
        assert!(full.contains(line), "{full}");
        let (delta, _) = presentation.render(&seat, &observation, false);
        assert!(delta.contains(line), "{delta}");
        observation.clock = None;
        let (plain, _) = presentation.render(&seat, &observation, false);
        assert!(!plain.contains("CLOCK"), "{plain}");
    }

    #[test]
    fn delta_reports_changed_and_removed_piles_and_lines_and_keeps_decision() {
        let (seat, mut observation) = observation();
        let mut presentation = Presentation::default();
        presentation.render(&seat, &observation, false);
        observation.view.supply[0].count -= 1;
        let removed = observation.view.supply.pop().unwrap();
        observation
            .view
            .you
            .as_mut()
            .unwrap()
            .counters
            .insert("marker".into(), 3);
        let (changed, cards) = presentation.render(&seat, &observation, false);
        assert!(changed.starts_with("GAME test"));
        assert!(
            changed.contains("SUPPLY (changed) Copper $0 x45, Village gone"),
            "{changed}"
        );
        assert!(changed.contains("marker 3") && changed.contains("DECISION "));
        assert!(cards.is_empty());
        assert_eq!(removed.card, "Village");
        observation.view.you.as_mut().unwrap().counters.clear();
        let (gone, _) = presentation.render(&seat, &observation, false);
        assert!(gone.contains("gone: marker 3"), "{gone}");
        assert!(!gone.contains("SUPPLY"));
    }

    #[test]
    fn log_truncation_is_explicit_cursors_do_not_repeat_and_undo_resets() {
        let (seat, mut observation) = observation();
        observation.log = (0..80)
            .map(|seq| LogEntry {
                seq,
                kind: if seq == 0 { "turn-start" } else { "played" }.into(),
                player: Some(0),
                card: Some("Village".into()),
                text: if seq == 0 {
                    "Ada starts turn 4".into()
                } else {
                    format!("entry {seq}")
                },
            })
            .collect();
        let mut presentation = Presentation::default();
        let (full, _) = presentation.render(&seat, &observation, false);
        assert!(
            full.contains("entries 0-29 omitted")
                && full.contains("[0-29] Ada T4: played Village x29 (so far)")
                && full.contains("30: entry 30")
                && full.contains("79: entry 79")
        );
        assert!(!full.contains("29: entry 29"));
        let (same, _) = presentation.render(&seat, &observation, false);
        assert!(!same.contains("LOG") && !same.contains("CARDS"));
        observation.log.truncate(2);
        observation.undos += 1;
        let (reset, cards) = presentation.render(&seat, &observation, false);
        assert!(reset.starts_with("STATUS game") && reset.contains("1: entry 1"));
        assert!(cards.is_empty());
    }

    #[test]
    fn turns_are_picked_by_player_and_number_or_newest_first() {
        let log: Vec<LogEntry> = (0..6)
            .map(|seq| LogEntry {
                seq,
                kind: "turn-start".into(),
                player: Some((seq % 2) as u8),
                card: None,
                text: format!("X starts turn {}", seq / 2 + 1),
            })
            .collect();
        let names = ["Ada".to_string(), "Bo".to_string()];
        let pick = |p: TurnsParams| -> Vec<(u8, u32)> {
            turns(&log, &names, false, &p)
                .into_iter()
                .map(|(t, _)| (t.player, t.number))
                .collect()
        };
        let all = || TurnsParams {
            player: None,
            from: None,
            to: None,
            last: None,
            detail: false,
        };
        let bo_from_2 = TurnsParams {
            player: Some(1),
            from: Some(2),
            ..all()
        };
        assert_eq!(pick(bo_from_2), vec![(1, 2), (1, 3)]);
        let newest = TurnsParams {
            last: Some(2),
            ..all()
        };
        assert_eq!(pick(newest), vec![(0, 3), (1, 3)]);
    }

    #[test]
    fn expectations_accept_normalized_labels_suffixless_labels_and_card_names() {
        let (_, observation) = observation();
        let d = observation.view.decision.unwrap();
        let option = &d.options[0];
        for expected in [
            option.label.to_uppercase(),
            option.label.split(" (").next().unwrap().to_string(),
            option.card.clone().unwrap(),
        ] {
            let params = StepParams {
                token: None,
                decision: d.id.clone(),
                indices: Some(vec![option.index.into()]),
                name: None,
                expect: Some(vec![expected]),
                timeout_s: None,
            };
            assert!(validate_expect(&d, &params).is_ok());
        }
    }
}
