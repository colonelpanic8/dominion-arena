//! What dominion.games says about a decision: the short prompt under the status bar's
//! counters ("You may buy cards", "Discard down to 3"), and its buttons' names ("End
//! Buys", "Confirm Discarding", "Don't Throne"). Pure, tested natively.

use crate::cards::CardDb;
use dominion_protocol::AgentDecision;

fn bounds(d: &AgentDecision) -> (u32, u32) {
    (d.min.unwrap_or(1), d.max.unwrap_or(1))
}

fn number_word(n: u32) -> String {
    match n {
        1 => "one".into(),
        2 => "two".into(),
        3 => "three".into(),
        4 => "four".into(),
        n => n.to_string(),
    }
}

fn cards_word(n: u32) -> String {
    if n == 1 {
        "a card".into()
    } else {
        format!("{n} cards")
    }
}

/// The source card's text, lower-cased, or nothing.
fn source_text(d: &AgentDecision, cards: &CardDb) -> String {
    d.source
        .as_deref()
        .and_then(|s| cards.get(s))
        .map(|c| c.text.to_lowercase())
        .unwrap_or_default()
}

/// "discards down to 3 cards" in a card's text: the 3.
fn down_to(text: &str) -> Option<u32> {
    let at = text.find("down to ")?;
    text[at + 8..].split_whitespace().next()?.parse().ok()
}

/// The prompt dominion.games shows for a decision.
pub fn prompt(d: &AgentDecision, cards: &CardDb) -> String {
    let (min, max) = bounds(d);
    let text = source_text(d, cards);
    let may = min == 0;
    match (d.kind.as_str(), d.purpose.as_str()) {
        ("turn", _) => match d.phase.as_deref() {
            Some("action") => "You may play Actions".into(),
            Some("buy") => "You may buy cards".into(),
            Some("night") => "You may play Night cards".into(),
            _ => d.prompt.clone(),
        },
        ("name", _) => "Name a card".into(),
        ("number", _) => "Choose a number".into(),
        (_, "discard") => {
            if let Some(n) = down_to(&text).filter(|_| min == max) {
                format!("Discard down to {n}")
            } else if text.contains("discard any number") && text.contains("draw") {
                "Discard, then draw".into()
            } else if may {
                "You may discard".into()
            } else {
                format!("Discard {}", cards_word(min))
            }
        }
        (_, "trash") => {
            if may && max > 1 && text.contains(&format!("up to {max}")) {
                format!("Trash up to {max} cards")
            } else if may {
                "You may trash".into()
            } else {
                format!("Trash {}", cards_word(min))
            }
        }
        (_, "gain") => "Gain a card".into(),
        (_, "play") => {
            if text.contains("three times") {
                "You may play an Action three times".into()
            } else if text.contains("twice") {
                "You may play an Action twice".into()
            } else if may {
                "You may play a card".into()
            } else {
                "Play a card".into()
            }
        }
        (_, "top-deck") => {
            if d.ordered {
                "Topdeck in any order".into()
            } else if may {
                "You may topdeck".into()
            } else {
                format!("Topdeck {}", cards_word(min))
            }
        }
        (_, "reaction") => "You may react".into(),
        (_, "trigger") => {
            if may {
                "You may resolve an effect".into()
            } else {
                "Resolve an effect".into()
            }
        }
        (_, "mode") | (_, "either") => {
            if min == max {
                format!("Choose {}", number_word(min))
            } else {
                "Choose".into()
            }
        }
        (_, "set-aside") => {
            if may {
                "You may set aside".into()
            } else {
                "Set aside".into()
            }
        }
        (_, "reveal") => {
            if may {
                "You may reveal".into()
            } else {
                "Reveal a card".into()
            }
        }
        (_, "exile") => "Exile a card".into(),
        (_, "return") => "Return a card".into(),
        (_, "pass") => "Pass a card".into(),
        (_, "overpay") => "You may overpay".into(),
        (_, "name") => "Name a card".into(),
        _ => d
            .prompt
            .split_once(": ")
            .map_or(d.prompt.clone(), |(_, rest)| capitalise(rest)),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// The button ending a phase, and the image it is drawn on.
pub fn end_phase(phase: Option<&str>) -> (&'static str, &'static str) {
    match phase {
        Some("action") => ("End Actions", "stone"),
        Some("buy") => ("End Buys", "gold"),
        _ => ("End Turn", "copper"),
    }
}

/// The button sending the picks of a decision answered by picking several things.
pub fn done(d: &AgentDecision) -> &'static str {
    let (min, max) = bounds(d);
    let single = min == 1 && max == 1;
    if d.ordered {
        return "Done Ordering";
    }
    match d.purpose.as_str() {
        "discard" => {
            if single {
                "Discard"
            } else {
                "Confirm Discarding"
            }
        }
        "trash" => {
            if single {
                "Trash"
            } else {
                "Confirm Trashing"
            }
        }
        "top-deck" if single => "Topdeck",
        "exile" => "Exile",
        "pass" if single => "Pass",
        "overpay" => "Overpay",
        "set-aside" => "Done Setting Aside",
        "return" => "Done Returning",
        "mode" | "either" => "Done Choosing",
        _ => "Done Selecting",
    }
}

/// The count on the Done button: how many picks are still needed, with a "+" when more
/// may follow; only for decisions needing more than one.
pub fn done_count(d: &AgentDecision, picked: usize) -> Option<String> {
    let (min, max) = bounds(d);
    if min <= 1 {
        return None;
    }
    let need = (min as usize).saturating_sub(picked);
    if need == 0 {
        return None;
    }
    Some(if max > min {
        format!("{need}+")
    } else {
        need.to_string()
    })
}

/// The button answering a one-pick decision with nothing.
pub fn decline(d: &AgentDecision) -> &'static str {
    let source = d.source.as_deref().unwrap_or_default();
    match d.purpose.as_str() {
        "play" => match source {
            "Throne Room" | "Royal Carriage" | "Disciple" | "Mastermind" | "Scepter" => {
                "Don't Throne"
            }
            "King's Court" => "Don't King",
            _ => "Don't Play",
        },
        "trash" => "Don't Trash",
        "discard" => "Don't Discard",
        "top-deck" => "Don't Topdeck",
        "gain" => "Don't Gain",
        "reveal" => "Don't Reveal",
        "exile" => "Don't Exile",
        "set-aside" => "Don't Set Aside",
        "overpay" => "Don't Overpay",
        "reaction" | "trigger" => "Continue",
        _ => "No thanks",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_protocol::DecisionGroup;

    fn decision(
        kind: &str,
        purpose: &str,
        min: u32,
        max: u32,
        source: Option<&str>,
    ) -> AgentDecision {
        AgentDecision {
            id: "d1".into(),
            player: Some(0),
            prompt: format!(
                "{}choose",
                source.map(|s| format!("{s}: ")).unwrap_or_default()
            ),
            kind: kind.into(),
            purpose: purpose.into(),
            source: source.map(Into::into),
            phase: None,
            ordered: false,
            name_any: false,
            min: Some(min),
            max: Some(max),
            groups: vec![DecisionGroup {
                group: 0,
                kind: kind.into(),
                prompt: String::new(),
                min,
                max,
            }],
            options: vec![],
        }
    }

    fn db() -> CardDb {
        let card = |name: &str, text: &str| dominion_protocol::api::CardInfo {
            name: name.into(),
            set: "Base".into(),
            types: vec!["Action".into()],
            kind: "card".into(),
            cost: None,
            variable_cost: false,
            overpay: false,
            coins: 0,
            potions: 0,
            vp: 0,
            kingdom: true,
            basic: false,
            removed: false,
            pile: None,
            text: text.into(),
        };
        CardDb::new(vec![
            card(
                "Militia",
                "+$2\nEach other player discards down to 3 cards in hand.",
            ),
            card(
                "Cellar",
                "+1 Action\nDiscard any number of cards, then draw that many.",
            ),
            card("Chapel", "Trash up to 4 cards from your hand."),
            card(
                "Throne Room",
                "You may play an Action card from your hand twice.",
            ),
        ])
    }

    #[test]
    fn prompts_read_like_dominion_games() {
        let cards = db();
        let p = |d: AgentDecision| prompt(&d, &cards);
        assert_eq!(
            p(decision("cards", "discard", 2, 2, Some("Militia"))),
            "Discard down to 3"
        );
        assert_eq!(
            p(decision("cards", "discard", 0, 5, Some("Cellar"))),
            "Discard, then draw"
        );
        assert_eq!(
            p(decision("cards", "trash", 0, 4, Some("Chapel"))),
            "Trash up to 4 cards"
        );
        assert_eq!(p(decision("cards", "trash", 0, 2, None)), "You may trash");
        assert_eq!(
            p(decision("cards", "play", 0, 1, Some("Throne Room"))),
            "You may play an Action twice"
        );
        let mut turn = decision("turn", "turn", 1, 1, None);
        turn.phase = Some("buy".into());
        assert_eq!(p(turn), "You may buy cards");
    }

    #[test]
    fn buttons_are_named_by_what_they_do() {
        assert_eq!(
            done(&decision("cards", "discard", 2, 2, None)),
            "Confirm Discarding"
        );
        assert_eq!(done(&decision("cards", "trash", 1, 1, None)), "Trash");
        assert_eq!(
            decline(&decision("cards", "play", 0, 1, Some("Throne Room"))),
            "Don't Throne"
        );
        assert_eq!(
            done_count(&decision("cards", "discard", 2, 2, None), 1).as_deref(),
            Some("1")
        );
        assert_eq!(
            done_count(&decision("cards", "discard", 2, 4, None), 0).as_deref(),
            Some("2+")
        );
        assert_eq!(
            done_count(&decision("cards", "discard", 0, 4, None), 0),
            None
        );
        assert_eq!(end_phase(Some("buy")), ("End Buys", "gold"));
    }
}
