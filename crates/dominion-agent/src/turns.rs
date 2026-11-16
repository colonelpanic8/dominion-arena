//! A seat's log by turn: each turn summed up as what each player played, bought,
//! gained, trashed, ... in it, by card with counts.

use crate::{AgentView, LogEntry};
use serde::Serialize;

/// A turn's run of log entries: from its start up to the next turn's.
#[derive(Clone, Copy, Debug)]
pub struct TurnSpan<'a> {
    pub player: u8,
    /// The player's own turn count, extra turns included.
    pub number: u32,
    pub entries: &'a [LogEntry],
}

impl<'a> TurnSpan<'a> {
    /// The same turn, keeping only its entries `keep` accepts (a contiguous range).
    pub fn within(self, keep: impl Fn(&LogEntry) -> bool) -> Option<TurnSpan<'a>> {
        let start = self.entries.iter().position(&keep)?;
        let len = self.entries[start..]
            .iter()
            .position(|e| !keep(e))
            .unwrap_or(self.entries.len() - start);
        Some(TurnSpan {
            entries: &self.entries[start..start + len],
            ..self
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSummary {
    pub player: u8,
    pub turn: u32,
    pub first_seq: usize,
    pub last_seq: usize,
    pub summary: String,
}

/// Splits a log, oldest first, at each turn start. Entries before the first turn are
/// left out.
pub fn split(log: &[LogEntry]) -> Vec<TurnSpan<'_>> {
    let starts: Vec<usize> = (0..log.len())
        .filter(|&i| log[i].kind == "turn-start")
        .collect();
    starts
        .iter()
        .enumerate()
        .filter_map(|(k, &i)| {
            let start = &log[i];
            Some(TurnSpan {
                player: start.player?,
                // "<name> starts turn <n>" (project::log_entry).
                number: start.text.rsplit(' ').next()?.parse().ok()?,
                entries: &log[i..starts.get(k + 1).copied().unwrap_or(log.len())],
            })
        })
        .collect()
}

/// Players' names by seat.
pub fn names(view: &AgentView) -> Vec<String> {
    let mut players: Vec<_> = view.you.iter().chain(&view.opponents).collect();
    players.sort_by_key(|p| p.index);
    players.into_iter().map(|p| p.name.clone()).collect()
}

const VERBS: [(&str, &str); 15] = [
    ("played", "played"),
    ("used-way", "used"),
    ("bought", "bought"),
    ("bought-landscape", "bought"),
    ("gained", "gained"),
    ("exchanged", "exchanged"),
    ("trashed", "trashed"),
    ("discarded", "discarded"),
    ("top-decked", "put onto their deck"),
    ("set-aside", "set aside"),
    ("revealed", "revealed"),
    ("received", "received"),
    ("returned", "returned"),
    ("called", "called"),
    ("unaffected", "was unaffected thanks to"),
];

/// Entries whose text is kept as it is: tokens and counters.
const NOTED: [&str; 4] = ["counter", "token-moved", "marker", "card-counter"];

#[derive(Default)]
struct Doings {
    player: u8,
    verbs: Vec<(&'static str, Vec<(String, u32)>)>,
    notes: Vec<String>,
}

impl Doings {
    fn add(&mut self, verb: &'static str, card: String) {
        let at = match self.verbs.iter().position(|(v, _)| *v == verb) {
            Some(at) => at,
            None => {
                self.verbs.push((verb, vec![]));
                self.verbs.len() - 1
            }
        };
        let cards = &mut self.verbs[at].1;
        match cards.iter_mut().find(|(c, _)| *c == card) {
            Some((_, n)) => *n += 1,
            None => cards.push((card, 1)),
        }
    }

    fn text(&self) -> String {
        let mut parts: Vec<String> = self
            .verbs
            .iter()
            .map(|(verb, cards)| {
                let cards: Vec<String> = cards
                    .iter()
                    .map(|(c, n)| match n {
                        1 => c.clone(),
                        n => format!("{c} x{n}"),
                    })
                    .collect();
                format!("{verb} {}", cards.join(", "))
            })
            .collect();
        parts.extend(self.notes.iter().cloned());
        parts.join("; ")
    }
}

/// One line for a turn: the turn's player first, then anyone else something happened to.
/// A turn `ongoing` says so.
pub fn summarize(span: TurnSpan<'_>, names: &[String], ongoing: bool) -> TurnSummary {
    let name = |p: u8| {
        names
            .get(p as usize)
            .cloned()
            .unwrap_or_else(|| format!("P{}", p + 1))
    };
    let mut doings = vec![Doings {
        player: span.player,
        ..Doings::default()
    }];
    // A bought card's gain says nothing more.
    let mut bought: Vec<(u8, &Option<String>)> = vec![];
    for e in span.entries {
        let Some(p) = e.player else { continue };
        if e.kind == "bought" {
            bought.push((p, &e.card));
        } else if e.kind == "gained"
            && let Some(at) = bought.iter().position(|b| *b == (p, &e.card))
        {
            bought.remove(at);
            continue;
        }
        let verb = VERBS.iter().find(|(k, _)| *k == e.kind).map(|(_, v)| *v);
        let noted = NOTED.contains(&e.kind.as_str());
        if verb.is_none() && !noted {
            continue;
        }
        let at = match doings.iter().position(|d| d.player == p) {
            Some(at) => at,
            None => {
                doings.push(Doings {
                    player: p,
                    ..Doings::default()
                });
                doings.len() - 1
            }
        };
        match verb {
            Some(verb) => doings[at].add(verb, e.card.clone().unwrap_or("a card".into())),
            None => {
                let prefix = format!("{} ", name(p));
                let text = e.text.strip_prefix(&prefix).unwrap_or(&e.text);
                doings[at].notes.push(text.to_string());
            }
        }
    }
    let mut lines = vec![];
    for d in &doings {
        let text = d.text();
        if d.player == span.player {
            let text = match (text.is_empty(), ongoing) {
                (true, true) => "nothing yet".into(),
                (true, false) => "played and gained nothing".into(),
                (false, true) => format!("{text} (so far)"),
                (false, false) => text,
            };
            lines.push(format!("{} T{}: {text}", name(d.player), span.number));
        } else if !text.is_empty() {
            lines.push(format!("{}: {text}", name(d.player)));
        }
    }
    let first_seq = span.entries.first().map_or(0, |e| e.seq);
    TurnSummary {
        player: span.player,
        turn: span.number,
        first_seq,
        last_seq: span.entries.last().map_or(first_seq, |e| e.seq),
        summary: lines.join(". "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        seq: usize,
        kind: &str,
        player: Option<u8>,
        card: Option<&str>,
        text: &str,
    ) -> LogEntry {
        LogEntry {
            seq,
            kind: kind.into(),
            player,
            card: card.map(Into::into),
            text: text.into(),
        }
    }

    #[test]
    fn a_turn_sums_up_its_players_doings_in_order() {
        let log = vec![
            entry(0, "noted", None, None, "Setup"),
            entry(1, "turn-start", Some(0), None, "Ann starts turn 3"),
            entry(2, "played", Some(0), Some("Witch"), "Ann plays Witch"),
            entry(3, "drew", Some(0), None, "Ann draws 2 cards"),
            entry(4, "gained", Some(1), Some("Curse"), "Ben gains Curse"),
            entry(5, "played", Some(0), Some("Copper"), "Ann plays Copper"),
            entry(6, "played", Some(0), Some("Copper"), "Ann plays Copper"),
            entry(7, "counter", Some(0), None, "Ann takes 2 Coffers"),
            entry(8, "bought", Some(0), Some("Gold"), "Ann buys Gold"),
            entry(9, "gained", Some(0), Some("Gold"), "Ann gains Gold"),
            entry(10, "turn-start", Some(1), None, "Ben starts turn 3"),
            entry(11, "drew", Some(1), None, "Ben draws 5 cards"),
        ];
        let names = ["Ann".to_string(), "Ben".to_string()];
        let turns: Vec<_> = split(&log)
            .into_iter()
            .map(|t| summarize(t, &names, t.number == 3 && t.player == 1))
            .collect();
        assert_eq!(
            turns,
            vec![
                TurnSummary {
                    player: 0,
                    turn: 3,
                    first_seq: 1,
                    last_seq: 9,
                    summary: "Ann T3: played Witch, Copper x2; bought Gold; takes 2 Coffers. \
                              Ben: gained Curse"
                        .into(),
                },
                TurnSummary {
                    player: 1,
                    turn: 3,
                    first_seq: 10,
                    last_seq: 11,
                    summary: "Ben T3: nothing yet".into(),
                },
            ]
        );
    }
}
