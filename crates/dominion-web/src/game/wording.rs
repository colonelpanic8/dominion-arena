//! What dominion.games says about a decision: the short prompt under the status bar's
//! counters ("You may buy cards", "Discard down to 3"), and its buttons' names ("End
//! Buys", "Confirm Discarding", "Don't Throne"). Pure, tested natively.

use crate::cards::CardDb;
use dominion_protocol::{AgentDecision, AgentOption, NoteView};

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

/// A decision's own prompt as shown to a player, without agent-only Way hints.
pub fn own_prompt(d: &AgentDecision) -> &str {
    if is_sinister_plot(d) {
        return "Sinister Plot";
    }
    without_way_hint(&d.prompt)
}

fn is_sinister_plot(d: &AgentDecision) -> bool {
    d.source.as_deref() == Some("Sinister Plot") && matches!(d.purpose.as_str(), "either" | "mode")
}

pub(crate) fn option_label(d: &AgentDecision, o: &AgentOption, notes: &[NoteView]) -> String {
    if is_sinister_plot(d) && o.action == "mode" {
        if matches!(o.label.as_str(), "Add a token" | "Add token") {
            return "Add token".into();
        }
        if (o.label == "Draw" || o.label.starts_with("Remove your "))
            && let Some(player) = d.player
            && let Some(count) = notes
                .iter()
                .find(|n| n.source == "Sinister Plot" && n.player == Some(player))
                .and_then(|n| u32::try_from(n.count).ok())
        {
            return format!("Draw {}", cards_word(count));
        }
    }
    o.label.clone()
}

const WAY_HINT: &str = " (to pre-answer the Way question";

pub fn without_way_hint(prompt: &str) -> &str {
    match prompt.find(WAY_HINT) {
        Some(at) => &prompt[..at],
        None => prompt,
    }
}

/// Whether answering `d` may play an Action card in a game with Ways, so that a Way
/// question may follow: the server says so in the prompt.
pub fn may_ask_way(d: &AgentDecision) -> bool {
    d.prompt.contains(WAY_HINT)
}

/// Whether `d` asks whether to play a card as itself or use a Way instead.
fn is_way(d: &AgentDecision, cards: &CardDb) -> bool {
    d.kind == "choose"
        && d.options.iter().any(|o| o.action == "select")
        && d.options.iter().all(|o| match o.action.as_str() {
            "no" => true,
            "select" => o
                .card
                .as_deref()
                .and_then(|c| cards.get(c))
                .is_some_and(|c| c.kind == "way"),
            _ => false,
        })
}

/// A Buy phase in which Treasures can still be played: dominion.games asks that first.
pub fn treasures_first(d: &AgentDecision) -> bool {
    d.kind == "turn"
        && d.phase.as_deref() == Some("buy")
        && d.options.iter().any(|o| o.action == "play")
}

/// dominion.games' short description of each of its questions (client 2.3.7), by its
/// question id: `REMODEL_GAIN`, `PAWN`, ...
const QUESTIONS: &str = include_str!("question_prompts.tsv");

/// The question id words a decision's purpose ends in.
fn purpose_suffixes(purpose: &str) -> &'static [&'static str] {
    match purpose {
        "trash" => &["TRASH"],
        "discard" => &["DISCARD"],
        "gain" => &["GAIN"],
        "top-deck" => &["TOPDECK"],
        "mode" | "either" => &["MODE", "CHOICE", "CHOOSE"],
        "play" => &["PLAY"],
        "reveal" => &["REVEAL"],
        "set-aside" => &["SET_ASIDE"],
        "exile" => &["EXILE"],
        "return" => &["RETURN"],
        "pass" => &["PASS"],
        "overpay" => &["OVERPAY"],
        "number" => &["INSERT"],
        // Moving a token onto a pile (Plan, Lost Arts, ...).
        "other" => &["MOVE", "NEW"],
        "move" => &["SHUFFLE"],
        _ => &[],
    }
}

/// What a prompt may name besides the decision: the players, by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asking {
    /// The player asked.
    pub name: Option<String>,
    /// Everyone else.
    pub others: Vec<String>,
    /// Coin tokens on the asked player's Pirate Ship mat.
    pub pirate_ship: u32,
}

/// dominion.games' own prompt for the question its source card asks, in parts (an
/// amount is a part of its own, as it draws it): the question named after the card
/// alone, or after the card and the decision's purpose.
pub fn question_prompt(d: &AgentDecision, cards: &CardDb, asking: &Asking) -> Option<Vec<String>> {
    if d.kind == "turn" || matches!(d.purpose.as_str(), "way" | "trigger" | "reaction") {
        return None;
    }
    let source = d.source.as_deref()?;
    let key: String = source
        .chars()
        .filter_map(|c| match c {
            ' ' | '-' => Some('_'),
            c if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase()),
            _ => None,
        })
        .collect();
    let find = |id: &str| {
        QUESTIONS
            .lines()
            .find_map(|l| l.split_once('\t').filter(|(k, _)| *k == id).map(|(_, t)| t))
    };
    let (min, max) = bounds(d);
    // Whose cards the question is about: the asked player's own, or another's.
    let own = own_prompt(d).contains("your");
    let whose = if own { "YOU" } else { "OTHER" };
    // Torturer's choice is asked as its discard, with taking the Curse its other button.
    let key = if source == "Torturer" && matches!(d.purpose.as_str(), "mode" | "either") {
        "TORTURER_MAY_DISCARD".to_string()
    } else {
        key
    };
    let text = find(&key).or_else(|| {
        purpose_suffixes(&d.purpose)
            .iter()
            .flat_map(|s| {
                [
                    (min == 0).then(|| format!("{key}_MAY_{s}")),
                    Some(format!("{key}_{s}")),
                ]
            })
            .flatten()
            .chain([format!("{key}_{whose}")])
            .find_map(|id| find(&id))
    })?;
    let amount = d
        .options
        .iter()
        .filter_map(|o| o.cost.as_ref())
        .filter(|c| c.potions == 0 && c.debt == 0)
        .map(|c| c.coins)
        .max()
        .or((source == "Pirate Ship").then_some(asking.pirate_ship));
    let player = |name: &str| {
        asking
            .others
            .iter()
            .any(|o| o == name)
            .then(|| name.to_string())
    };
    // Another player a question names: "... to Bob", "Bob's Copper", else the only one.
    let other = || {
        let prompt = own_prompt(d);
        prompt
            .rsplit_once(" to ")
            .and_then(|(_, n)| player(n))
            .or_else(|| {
                prompt
                    .split_whitespace()
                    .find_map(|w| w.strip_suffix("'s").and_then(player))
            })
            .or_else(|| match asking.others.as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            })
    };
    let mut parts = vec![String::new()];
    let mut rest = text;
    loop {
        let next = ["AMOUNT0", "NUMBER0", "PLAYER0", "PLAYER1", "CARDS0"]
            .into_iter()
            .filter_map(|p| rest.find(p).map(|i| (i, p)))
            .min();
        let last = parts.last_mut().expect("a part");
        let Some((i, p)) = next else {
            last.push_str(rest);
            break;
        };
        last.push_str(&rest[..i]);
        parts.push(match p {
            "AMOUNT0" => format!("{} Coins", amount?),
            "NUMBER0" if min == max => min.to_string(),
            "PLAYER0" => other()?,
            "PLAYER1" if own => asking.name.clone()?,
            "PLAYER1" => other()?,
            "CARDS0" => with_article(&subject_card(d, cards)?),
            _ => return None,
        });
        parts.push(String::new());
        rest = &rest[i + p.len()..];
    }
    let parts: Vec<String> = parts
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let unfilled = ["Argument", "CARDS", "PLAYER", "NUMBER", "AMOUNT", "["];
    if parts.iter().any(|p| unfilled.iter().any(|x| p.contains(x))) {
        return None;
    }
    Some(parts)
}

/// The prompt dominion.games shows for a decision.
pub fn prompt(d: &AgentDecision, cards: &CardDb) -> String {
    prompt_parts(d, cards, &Asking::default()).join(" ")
}

/// [`prompt`] in the parts dominion.games draws apart (an amount of coins).
pub fn prompt_parts(d: &AgentDecision, cards: &CardDb, asking: &Asking) -> Vec<String> {
    if is_sinister_plot(d) {
        return vec![own_prompt(d).into()];
    }
    if is_way(d, cards) {
        let card = d.source.as_deref().unwrap_or("it");
        return vec!["Consider the Way you play".into(), with_article(card)];
    }
    question_prompt(d, cards, asking).unwrap_or_else(|| vec![generic_prompt(d, cards)])
}

/// The one card a question is about: the card all its options name, else the card an
/// option's label ends with ("Set aside Vassal").
fn subject_card(d: &AgentDecision, cards: &CardDb) -> Option<String> {
    let mut named = d.options.iter().filter_map(|o| o.card.as_deref());
    if let Some(first) = named.next() {
        return named.all(|c| c == first).then(|| first.to_string());
    }
    d.options.iter().find_map(|o| {
        let words: Vec<&str> = o.label.split(' ').collect();
        (1..words.len())
            .map(|i| words[i..].join(" "))
            .find(|tail| cards.get(tail).is_some())
    })
}

/// "a Vassal", "an Estate".
fn with_article(card: &str) -> String {
    let article = if card.starts_with(['A', 'E', 'I', 'O', 'U']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {card}")
}

fn generic_prompt(d: &AgentDecision, cards: &CardDb) -> String {
    let (min, max) = bounds(d);
    let text = source_text(d, cards);
    let may = min == 0;
    match (d.kind.as_str(), d.purpose.as_str()) {
        ("turn", _) => match d.phase.as_deref() {
            Some("action") => "You may play Actions".into(),
            Some("buy") if treasures_first(d) => "You may play Treasures".into(),
            Some("buy") => "You may buy cards".into(),
            Some("night") => "You may play Night cards".into(),
            _ => own_prompt(d).into(),
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
        (_, "reaction") => "Do you want to react?".into(),
        // dominion.games asks every optional ability as a reaction.
        (_, "trigger") if may || d.options.iter().any(|o| o.action == "no") => {
            "Do you want to react?".into()
        }
        (_, "trigger") => "Resolve an effect".into(),
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
        _ => {
            let own = own_prompt(d);
            own.split_once(": ")
                .map_or(own.into(), |(_, rest)| capitalise(rest))
        }
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

/// The Done button of a decision that may pick nothing, while nothing is picked.
pub fn done_with_none(d: &AgentDecision) -> Option<&'static str> {
    if d.ordered || bounds(d).0 != 0 {
        return None;
    }
    Some(match d.purpose.as_str() {
        "trash" => "Don't Trash",
        "discard" => "Don't Discard",
        "top-deck" => "Don't Topdeck",
        "exile" => "Don't Exile",
        _ => return None,
    })
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

/// dominion.games' name and icon for a button whose own label says something else: its
/// card modes for a card's yes-or-no questions.
pub fn choice_button(d: &AgentDecision, label: &str) -> Option<(&'static str, &'static str)> {
    Some(match (d.source.as_deref()?, label) {
        ("Library", l) if l.starts_with("Set aside ") => ("Skip", "refuse"),
        ("Library", "Keep it") => ("Keep", "put-in-hand"),
        ("Navigator", "Discard them all") => ("Discard All", "refuse"),
        ("Oracle" | "Scrying Pool", l) if l.starts_with("Discard ") => ("Discard", "discard-icon"),
        ("Oracle" | "Scrying Pool", l) if l.starts_with("Put ") => ("Topdeck", "topdeck-icon"),
        _ => return None,
    })
}

/// The option of a yes-or-no question about cards looked at that dominion.games answers
/// by a click on those cards rather than a button of its own.
pub fn answered_by_cards(d: &AgentDecision) -> Option<&'static str> {
    match d.source.as_deref()? {
        "Navigator" => Some("Put them back in any order"),
        _ => None,
    }
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
        "trash" => "Confirm Trashing",
        "discard" => "Confirm Discarding",
        "top-deck" => "Don't Topdeck",
        "gain" => "Don't Gain",
        "reveal" => "Don't Reveal",
        "exile" => "Don't Exile",
        "set-aside" => "Don't Set Aside",
        "overpay" => "Don't Overpay",
        "reaction" | "trigger" => "Continue",
        "move" if source == "Inn" => "Don't",
        _ => "No thanks",
    }
}

/// Why the game ended, from the result's reason code.
pub fn end_reason(code: &str) -> String {
    match code {
        "provinces-gone" => "The Provinces ran out.".into(),
        "colonies-gone" => "The Colonies ran out.".into(),
        "piles-empty" => "Supply piles ran out.".into(),
        "turn-limit" => "The turn limit was reached.".into(),
        "resigned" => "A player resigned.".into(),
        "stalled" => "No one could do anything any more.".into(),
        other => other.into(),
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
            online_text: None,
            online_production: None,
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
            card("Vassal", "+$2\nDiscard the top card of your deck."),
            card("Estate", "1 VP"),
            card("Copper", "$1"),
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
        assert_eq!(p(turn.clone()), "You may buy cards");
        turn.options.push(dominion_protocol::AgentOption {
            index: 0,
            group: 0,
            action: "play".into(),
            label: "Copper".into(),
            card: Some("Copper".into()),
            zone: None,
            cost: None,
            value: None,
            player: None,
        });
        assert_eq!(p(turn), "You may play Treasures");
    }

    #[test]
    fn card_questions_read_as_dominion_games_words_them() {
        let cards = db();
        let asking = Asking {
            name: Some("Alice".into()),
            others: vec!["Bob".into()],
            pirate_ship: 2,
        };
        let p = |d: AgentDecision| prompt_parts(&d, &cards, &asking);
        assert_eq!(
            p(decision("choose", "mode", 2, 2, Some("Pawn"))),
            ["Choose how to play Pawn"]
        );
        assert_eq!(
            p(decision("cards", "discard", 0, 5, Some("Night Watchman"))),
            ["Discard any number"]
        );
        assert_eq!(
            p(decision("choose", "mode", 2, 2, Some("Courtier"))),
            ["Choose", "2", "options"]
        );
        let mut pass = decision("cards", "pass", 1, 1, Some("Masquerade"));
        pass.prompt = "Masquerade: choose a card to pass to Bob".into();
        assert_eq!(p(pass), ["Pass a card to", "Bob"]);
        let mut gain = decision("pile", "gain", 1, 1, Some("Remodel"));
        for (index, coins) in [(0, 2), (1, 4), (2, 3)] {
            gain.options.push(dominion_protocol::AgentOption {
                index,
                group: 0,
                action: "pile".into(),
                label: String::new(),
                card: None,
                zone: None,
                cost: Some(dominion_protocol::Cost {
                    coins,
                    ..Default::default()
                }),
                value: None,
                player: None,
            });
        }
        assert_eq!(p(gain), ["Gain a card up to", "4 Coins"]);
        // Its own words where the question names no amount it can fill in.
        assert_eq!(
            p(decision("cards", "trash", 0, 4, Some("Chapel"))),
            ["Trash up to 4 cards"]
        );
        assert_eq!(
            p(decision("choose", "reaction", 0, 1, None)),
            ["Do you want to react?"]
        );
    }

    fn option(index: u32, label: &str, card: Option<&str>) -> dominion_protocol::AgentOption {
        dominion_protocol::AgentOption {
            index,
            group: 0,
            action: "select".into(),
            label: label.into(),
            card: card.map(Into::into),
            zone: None,
            cost: None,
            value: None,
            player: None,
        }
    }

    #[test]
    fn questions_name_the_card_they_are_about() {
        let cards = db();
        let mut library = decision("choose", "set-aside", 1, 1, Some("Library"));
        library.options = vec![
            option(0, "Set aside Vassal", None),
            option(1, "Keep it", None),
        ];
        let asking = Asking {
            name: Some("Alice".into()),
            others: vec!["Bob".into()],
            pirate_ship: 2,
        };
        let p = |d: &AgentDecision| prompt_parts(d, &cards, &asking);
        assert_eq!(p(&library), ["You may skip", "a Vassal"]);
        assert_eq!(
            choice_button(&library, "Set aside Vassal"),
            Some(("Skip", "refuse"))
        );
        let mut diver = decision("cards", "top-deck", 0, 1, Some("Pearl Diver"));
        diver.options = vec![option(0, "Estate", Some("Estate"))];
        assert_eq!(p(&diver), ["Put", "an Estate", "on top or bottom"]);
        // Moving a token onto a pile.
        let plan = decision("pile", "other", 1, 1, Some("Plan"));
        assert_eq!(p(&plan), ["Move Trashing token"]);
        // The asked player's own cards, or another's.
        let mut pool = decision("choose", "discard", 1, 1, Some("Scrying Pool"));
        pool.prompt = "Scrying Pool: discard your Copper?".into();
        pool.options = vec![
            option(0, "Discard your Copper", None),
            option(1, "Put it back", None),
        ];
        assert_eq!(p(&pool), ["You: keep or discard", "a Copper"]);
        pool.prompt = "Scrying Pool: discard Bob's Copper?".into();
        pool.options[0].label = "Discard Bob's Copper".into();
        assert_eq!(p(&pool), ["Bob", ": keep or discard", "a Copper", "."]);
        let mut oracle = decision("choose", "discard", 1, 1, Some("Oracle"));
        oracle.prompt = "Oracle: discard your Copper, Oracle?".into();
        assert_eq!(p(&oracle), ["Make", "Alice", "discard?"]);
        let mut swindler = decision("pile", "gain", 1, 1, Some("Swindler"));
        swindler.prompt = "Swindler: choose a pile to gain from".into();
        assert_eq!(p(&swindler), ["Choose a card for", "Bob"]);
        assert_eq!(
            p(&decision("choose", "mode", 1, 1, Some("Pirate Ship"))),
            ["+", "2 Coins", "or attack"]
        );
        assert_eq!(
            p(&decision("cards", "set-aside", 0, 1, Some("Prince"))),
            ["You may set a card aside"]
        );
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
        assert_eq!(
            done_with_none(&decision("cards", "trash", 0, 4, Some("Chapel"))),
            Some("Don't Trash")
        );
        assert_eq!(
            done_with_none(&decision("cards", "trash", 1, 4, None)),
            None
        );
    }
}
