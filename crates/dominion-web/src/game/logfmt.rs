//! The game log as dominion.games words it: "Turn 3 - Ann" headers, each line led by
//! the player's initial, "A plays 3 Coppers. (+$3)", "A buys and gains a Silver.",
//! "B trashes 2 Coppers and an Estate.", consecutive like entries merged into one list,
//! and what follows from a play or buy indented under it.

use crate::cards::CardDb;
use dominion_protocol::{AgentView, LogEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Text(String),
    /// A card's name as it stands.
    Card(String),
    /// A card, shown as given: "a Copper", "3 Coppers".
    Cards(String, String),
    /// A player by their initials, in their colour.
    Player(u8),
    /// A player by their full name.
    FullPlayer(u8, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The seq of the first entry merged into the line.
    pub seq: usize,
    pub kind: String,
    pub player: Option<u8>,
    /// For a turn header, the player's turn number.
    pub turn: Option<u32>,
    /// 0 for turn headers and what a player does on their own (plays, buys, Clean-up's
    /// draw), 1 for what follows from a play or buy.
    pub depth: u8,
    pub spans: Vec<Span>,
    /// How many entries the line stands for.
    pub count: u32,
}

impl LogLine {
    pub fn is_turn(&self) -> bool {
        self.turn.is_some()
    }

    /// The line as plain text, with players by `short` names.
    pub fn text(&self, short: &[String]) -> String {
        self.spans
            .iter()
            .map(|sp| match sp {
                Span::Text(t) | Span::Card(t) | Span::Cards(_, t) | Span::FullPlayer(_, t) => {
                    t.clone()
                }
                Span::Player(p) => seat_name(short, *p),
            })
            .collect()
    }
}

fn seat_name(names: &[String], p: u8) -> String {
    names
        .get(usize::from(p))
        .cloned()
        .unwrap_or_else(|| format!("P{}", p + 1))
}

/// The view's seat names, indexed by seat.
pub fn names(view: &AgentView) -> Vec<String> {
    let mut players: Vec<(u8, String)> = view
        .you
        .iter()
        .chain(&view.opponents)
        .map(|p| (p.index, p.name.clone()))
        .collect();
    if let Some(r) = &view.game.result {
        players.extend(r.players.iter().map(|p| (p.index, p.name.clone())));
    }
    let n = players
        .iter()
        .map(|(i, _)| usize::from(*i) + 1)
        .max()
        .unwrap_or(0);
    let mut out: Vec<String> = (0..n).map(|i| format!("P{}", i + 1)).collect();
    for (i, name) in players {
        out[usize::from(i)] = name;
    }
    out
}

/// Each name's shortest beginning that no other name shares (at least one letter), as
/// the log prints players.
pub fn initials(names: &[String]) -> Vec<String> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let chars: Vec<char> = n.chars().collect();
            for len in 1..=chars.len() {
                let prefix: String = chars[..len].iter().collect();
                let clash = names
                    .iter()
                    .enumerate()
                    .any(|(j, m)| j != i && m.starts_with(&prefix));
                if !clash {
                    return prefix;
                }
            }
            n.clone()
        })
        .collect()
}

/// A card's plural, as English makes it.
pub fn plural(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with('s') {
        name.to_string()
    } else if lower.ends_with('y')
        && !matches!(
            lower.chars().rev().nth(1),
            Some('a' | 'e' | 'i' | 'o' | 'u')
        )
    {
        format!("{}ies", &name[..name.len() - 1])
    } else if lower.ends_with("ch") || lower.ends_with("sh") || lower.ends_with('x') {
        format!("{name}es")
    } else {
        format!("{name}s")
    }
}

/// "a Copper", "an Estate".
pub fn with_article(name: &str) -> String {
    let vowel = name
        .chars()
        .next()
        .is_some_and(|c| "AEIOUaeiou".contains(c));
    format!("{} {name}", if vowel { "an" } else { "a" })
}

/// A list of cards with how many of each, in the order first seen.
fn tally(cards: &[String]) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = vec![];
    for c in cards {
        match out.iter_mut().find(|(n, _)| n == c) {
            Some((_, k)) => *k += 1,
            None => out.push((c.clone(), 1)),
        }
    }
    out
}

/// "a Silver and 3 Coppers", "2 Coppers, a Silver, and an Estate".
fn push_list(out: &mut Vec<Span>, items: &[(String, u32)], landscape: bool) {
    if items.is_empty() {
        out.push(Span::Text("nothing".into()));
        return;
    }
    let n = items.len();
    for (i, (card, k)) in items.iter().enumerate() {
        if i > 0 {
            out.push(Span::Text(
                match (n, i + 1 == n) {
                    (2, _) => " and ",
                    (_, true) => ", and ",
                    _ => ", ",
                }
                .into(),
            ));
        }
        let text = if *k == 1 {
            if landscape {
                card.clone()
            } else {
                with_article(card)
            }
        } else {
            format!("{k} {}", plural(card))
        };
        out.push(Span::Cards(card.clone(), text));
    }
}

fn is_own_move(kind: &str) -> bool {
    matches!(kind, "played" | "bought" | "bought-landscape")
}

/// Each entry's depth: effects after a turn's first play or buy go under it, except the
/// draws and shuffles that end a finished turn (its Clean-up).
fn depths(entries: &[LogEntry]) -> Vec<u8> {
    let mut out = vec![0; entries.len()];
    let starts: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].kind == "turn-start")
        .collect();
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(entries.len());
        let finished = k + 1 < starts.len();
        let player = entries[start].player;
        let mut cleanup = end;
        if finished {
            while cleanup > start + 1 {
                let e = &entries[cleanup - 1];
                if e.player != player || !matches!(e.kind.as_str(), "drew" | "shuffled") {
                    break;
                }
                cleanup -= 1;
            }
        }
        let mut moved = false;
        for (i, e) in entries.iter().enumerate().take(cleanup).skip(start + 1) {
            if is_own_move(&e.kind) && e.player == player {
                moved = true;
            } else if moved {
                out[i] = 1;
            }
        }
    }
    out
}

fn is_treasure(cards: &CardDb, card: &str) -> bool {
    cards
        .get(card)
        .is_some_and(|c| c.types.iter().any(|t| t == "Treasure"))
}

/// What the Treasures make, when each always makes the same.
fn coins_of(cards: &CardDb, played: &[(String, u32)]) -> Option<u32> {
    let mut sum = 0;
    for (card, k) in played {
        let c = cards.get(card)?;
        if c.coins == 0 {
            return None;
        }
        sum += c.coins * k;
    }
    Some(sum)
}

/// The number at the end of "X draws 3 card(s)".
fn drawn(e: &LogEntry) -> u32 {
    e.text
        .split_whitespace()
        .rev()
        .find_map(|w| w.parse().ok())
        .unwrap_or(1)
}

/// What follows the player's name in an entry's own text, for kinds worded as they are.
fn rest_of(e: &LogEntry, names: &[String]) -> String {
    let full = e.player.map(|p| seat_name(names, p)).unwrap_or_default();
    e.text
        .strip_prefix(full.as_str())
        .unwrap_or(&e.text)
        .trim_start()
        .to_string()
}

/// Kinds whose consecutive entries by one player merge into one list of cards.
fn listed(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "discarded" => "discards",
        "trashed" => "trashes",
        "revealed" => "reveals",
        "received" => "receives",
        _ => return None,
    })
}

pub fn log_lines(entries: &[LogEntry], names: &[String], cards: &CardDb) -> Vec<LogLine> {
    let depth = depths(entries);
    let mut out: Vec<LogLine> = vec![];
    let mut i = 0;
    while i < entries.len() {
        let e = &entries[i];
        let d = depth[i];
        let same = |j: usize, kind: &str| {
            entries
                .get(j)
                .is_some_and(|f| f.kind == kind && f.player == e.player && depth[j] == d)
        };
        let mut spans: Vec<Span> = vec![];
        if let Some(p) = e.player {
            spans.push(Span::Player(p));
            spans.push(Span::Text(" ".into()));
        }
        let mut used = 1;
        let mut turn = None;
        let card = e.card.clone().unwrap_or_default();
        match e.kind.as_str() {
            "turn-start" => {
                spans.clear();
                let n: Option<u32> = e.text.rsplit(' ').next().and_then(|n| n.parse().ok());
                turn = n.or(Some(0));
                let who = e.player.map(|p| seat_name(names, p)).unwrap_or_default();
                spans.push(Span::Text(format!("Turn {} - ", n.unwrap_or(0))));
                if let Some(p) = e.player {
                    spans.push(Span::FullPlayer(p, who));
                }
            }
            "played" if is_treasure(cards, &card) => {
                let mut played = vec![card.clone()];
                while same(i + used, "played")
                    && entries[i + used]
                        .card
                        .as_deref()
                        .is_some_and(|c| is_treasure(cards, c))
                {
                    played.push(entries[i + used].card.clone().unwrap_or_default());
                    used += 1;
                }
                let items = tally(&played);
                spans.push(Span::Text("plays ".into()));
                push_list(&mut spans, &items, false);
                spans.push(Span::Text(".".into()));
                if let Some(c) = coins_of(cards, &items) {
                    spans.push(Span::Text(format!(" (+${c})")));
                }
            }
            "played" => {
                spans.push(Span::Text("plays ".into()));
                push_list(&mut spans, &[(card.clone(), 1)], false);
                spans.push(Span::Text(".".into()));
            }
            "bought" | "bought-landscape" => {
                let landscape = e.kind == "bought-landscape";
                let gains = entries.get(i + 1).is_some_and(|f| {
                    f.kind == "gained" && f.player == e.player && f.card == e.card
                });
                if gains {
                    used = 2;
                    spans.push(Span::Text("buys and gains ".into()));
                } else {
                    spans.push(Span::Text("buys ".into()));
                }
                push_list(&mut spans, &[(card.clone(), 1)], landscape);
                let tail = if gains {
                    gain_tail(&entries[i + 1])
                } else {
                    ""
                };
                spans.push(Span::Text(format!("{tail}.")));
            }
            "gained" => {
                spans.push(Span::Text("gains ".into()));
                push_list(&mut spans, &[(card.clone(), 1)], false);
                spans.push(Span::Text(format!("{}.", gain_tail(e))));
            }
            "drew" => {
                let mut n = drawn(e);
                while same(i + used, "drew") {
                    n += drawn(&entries[i + used]);
                    used += 1;
                }
                spans.push(Span::Text(if n == 1 {
                    "draws a card.".into()
                } else {
                    format!("draws {n} cards.")
                }));
            }
            "shuffled" => spans.push(Span::Text("shuffles their deck.".into())),
            "top-decked" => spans.push(Span::Text("topdecks a card.".into())),
            "set-aside" => {
                spans.push(Span::Text("sets ".into()));
                push_list(&mut spans, &[(card.clone(), 1)], false);
                spans.push(Span::Text(" aside.".into()));
            }
            "revealed-hand" => {
                let list = rest_of(e, names);
                let list = list.split_once(": ").map_or("", |(_, l)| l);
                let shown: Vec<String> = if list == "nothing" || list.is_empty() {
                    vec![]
                } else {
                    list.split(", ").map(str::to_string).collect()
                };
                spans.push(Span::Text("reveals their hand: ".into()));
                push_list(&mut spans, &tally(&shown), false);
                spans.push(Span::Text(".".into()));
            }
            kind if listed(kind).is_some() => {
                let mut list = vec![card.clone()];
                while same(i + used, kind) {
                    list.push(entries[i + used].card.clone().unwrap_or_default());
                    used += 1;
                }
                let verb = listed(kind).unwrap_or_default();
                spans.push(Span::Text(format!("{verb} ")));
                push_list(&mut spans, &tally(&list), kind == "received");
                spans.push(Span::Text(".".into()));
            }
            _ => {
                let rest = rest_of(e, names);
                match e
                    .card
                    .as_ref()
                    .and_then(|c| rest.find(c.as_str()).map(|at| (c, at)))
                {
                    Some((c, at)) => {
                        spans.push(Span::Text(rest[..at].to_string()));
                        spans.push(Span::Card(c.clone()));
                        spans.push(Span::Text(format!("{}.", &rest[at + c.len()..])));
                    }
                    None => spans.push(Span::Text(format!("{rest}."))),
                }
            }
        }
        spans.retain(|s| !matches!(s, Span::Text(t) if t.is_empty()));
        out.push(LogLine {
            seq: e.seq,
            kind: e.kind.clone(),
            player: e.player,
            turn,
            depth: d,
            spans,
            count: used as u32,
        });
        i += used;
    }
    out
}

/// Where a gain went, after the card: "to their hand", "onto their drawpile".
fn gain_tail(e: &LogEntry) -> &'static str {
    if e.text.ends_with(" to their hand") {
        " to their hand"
    } else if e.text.ends_with(" onto their deck") {
        " onto their drawpile"
    } else if e.text.ends_with(", setting it aside") {
        " setting it aside"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_are_the_shortest_unshared_beginnings() {
        let n = |v: &[&str]| initials(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(n(&["colonelpanic8", "Lord Rattington"]), ["c", "L"]);
        assert_eq!(n(&["Bob", "Bobby", "Ann"]), ["Bob", "Bobb", "A"]);
    }

    #[test]
    fn plurals_and_articles() {
        assert_eq!(plural("Copper"), "Coppers");
        assert_eq!(plural("Duchy"), "Duchies");
        assert_eq!(plural("Witch"), "Witches");
        assert_eq!(plural("Gardens"), "Gardens");
        assert_eq!(plural("Lackeys"), "Lackeys");
        assert_eq!(with_article("Estate"), "an Estate");
        assert_eq!(with_article("Silver"), "a Silver");
    }

    #[test]
    fn lists_join_with_and_and_a_serial_comma() {
        let text = |items: &[(&str, u32)]| {
            let mut s = vec![];
            let items: Vec<(String, u32)> =
                items.iter().map(|(c, k)| (c.to_string(), *k)).collect();
            push_list(&mut s, &items, false);
            LogLine {
                seq: 0,
                kind: String::new(),
                player: None,
                turn: None,
                depth: 0,
                spans: s,
                count: 1,
            }
            .text(&[])
        };
        assert_eq!(text(&[("Copper", 3)]), "3 Coppers");
        assert_eq!(
            text(&[("Silver", 1), ("Copper", 3)]),
            "a Silver and 3 Coppers"
        );
        assert_eq!(
            text(&[("Copper", 2), ("Silver", 1), ("Estate", 1)]),
            "2 Coppers, a Silver, and an Estate"
        );
    }
}
