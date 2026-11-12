//! The game log as dominion.games words it: "Turn 3 - Ann" headers, each line led by
//! the player's initial, "A plays 3 Coppers. (+$3)", "A buys and gains a Silver.",
//! "B trashes 2 Coppers and an Estate.", consecutive like entries merged into one list,
//! and what follows from a play or buy indented under it.

use super::entry::{self, Counter, CounterAt, Entry, GainTo, Marker, Moved, Place, seat_name};
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

/// Parts of a list joined as English joins them: "A and B", "A, B, and C".
fn join(out: &mut Vec<Span>, parts: Vec<Span>) {
    if parts.is_empty() {
        out.push(Span::Text("nothing".into()));
        return;
    }
    let n = parts.len();
    for (i, part) in parts.into_iter().enumerate() {
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
        out.push(part);
    }
}

fn card_part(card: &str, k: u32, landscape: bool) -> Span {
    let text = match (k, landscape) {
        (1, true) => card.to_string(),
        (1, false) => with_article(card),
        _ => format!("{k} {}", plural(card)),
    };
    Span::Cards(card.to_string(), text)
}

/// "a Silver and 3 Coppers", "2 Coppers, a Silver, and an Estate".
fn push_list(out: &mut Vec<Span>, items: &[(String, u32)], landscape: bool) {
    let parts = items
        .iter()
        .map(|(c, k)| card_part(c, *k, landscape))
        .collect();
    join(out, parts);
}

/// Cards by name, then how many more no one could tell: "a Copper and 2 other cards".
fn push_cards(out: &mut Vec<Span>, cards: &[String], hidden: u32) {
    let mut parts: Vec<Span> = tally(cards)
        .iter()
        .map(|(c, k)| card_part(c, *k, false))
        .collect();
    if hidden > 0 {
        parts.push(Span::Text(match (cards.is_empty(), hidden) {
            (true, 1) => "a card".into(),
            (true, n) => format!("{n} cards"),
            (false, 1) => "another card".into(),
            (false, n) => format!("{n} other cards"),
        }));
    }
    join(out, parts);
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

/// What follows the player's name in an entry's own text.
fn rest_of(e: &LogEntry, names: &[String]) -> String {
    let full = e.player.map(|p| seat_name(names, p)).unwrap_or_default();
    e.text
        .strip_prefix(full.as_str())
        .unwrap_or(&e.text)
        .trim_start()
        .to_string()
}

/// Kinds whose consecutive entries by one player merge into one list of cards, and how
/// a line of them reads around the list.
fn listed(e: &Entry) -> Option<(&'static str, &'static str, Option<&str>)> {
    Some(match e {
        Entry::Discarded(c) => ("discards ", ".", Some(c)),
        Entry::Trashed(c) => ("trashes ", ".", Some(c)),
        Entry::Revealed(c) => ("reveals ", ".", Some(c)),
        Entry::Received(c) => ("receives ", ".", Some(c)),
        Entry::SetAside(c) => ("sets ", " aside.", Some(c)),
        Entry::TopDecked(c) => ("topdecks ", ".", c.as_deref()),
        _ => return None,
    })
}

fn amount(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A place's words, from `p`'s side: "their hand", "B's discard pile", "the Silver pile".
fn place_spans(out: &mut Vec<Span>, place: &Place, p: u8) {
    let whose = |out: &mut Vec<Span>, q: u8| {
        if q == p {
            out.push(Span::Text("their ".into()));
        } else {
            out.push(Span::Player(q));
            out.push(Span::Text("'s ".into()));
        }
    };
    match place {
        Place::Pile(c) => {
            out.push(Span::Text("the ".into()));
            out.push(Span::Card(c.clone()));
            out.push(Span::Text(" pile".into()));
        }
        Place::Trash => out.push(Span::Text("the trash".into())),
        Place::InPlay(q) if *q == p => out.push(Span::Text("play".into())),
        Place::Zone(q, "looking") => {
            out.push(Span::Text("the top of ".into()));
            whose(out, *q);
            out.push(Span::Text("deck".into()));
        }
        Place::Hand(q)
        | Place::Deck(q)
        | Place::Discard(q)
        | Place::InPlay(q)
        | Place::Zone(q, _) => {
            whose(out, *q);
            out.push(Span::Text(
                match place {
                    Place::Hand(_) => "hand",
                    Place::Deck(_) => "deck",
                    Place::Discard(_) => "discard pile",
                    Place::InPlay(_) => "play area",
                    Place::Zone(_, k) => mat_name(k),
                    Place::Pile(_) | Place::Trash => unreachable!(),
                }
                .into(),
            ));
        }
    }
}

fn mat_name(key: &str) -> &'static str {
    match key {
        "tavern" => "Tavern mat",
        "island" => "Island mat",
        "nativeVillage" => "Native Village mat",
        "exile" => "Exile mat",
        "setAside" => "set-aside cards",
        _ => "mat",
    }
}

/// Cards moved with no event of their own, as dominion.games words each kind of move.
fn moved_spans(out: &mut Vec<Span>, m: &Moved, p: u8) {
    let text = |out: &mut Vec<Span>, t: &str| out.push(Span::Text(t.into()));
    let cards = |out: &mut Vec<Span>| push_cards(out, &m.cards, m.hidden);
    match (&m.from, &m.to) {
        (_, Place::Deck(q)) if m.back && *q == p => {
            text(out, "puts ");
            cards(out);
            text(
                out,
                if m.bottom {
                    " back on the bottom of their deck."
                } else {
                    " back onto their deck."
                },
            );
        }
        (Some(Place::Hand(f)), Place::Hand(q)) if *f == p && *q != p => {
            text(out, "passes ");
            cards(out);
            text(out, " to ");
            out.push(Span::Player(*q));
            text(out, ".");
        }
        (_, Place::Hand(q)) if *q == p => {
            text(out, "puts ");
            cards(out);
            text(out, " into their hand.");
        }
        (_, Place::Deck(q)) if *q == p && m.bottom => {
            text(out, "puts ");
            cards(out);
            text(out, " on the bottom of their deck.");
        }
        (_, Place::Deck(q)) if *q == p => {
            text(out, "topdecks ");
            cards(out);
            text(out, ".");
        }
        (Some(Place::Zone(f, "exile")), Place::Discard(q)) if *f == p && *q == p => {
            text(out, "discards ");
            cards(out);
            text(out, " from Exile.");
        }
        (_, Place::Discard(q)) if *q == p => {
            text(out, "puts ");
            cards(out);
            text(out, " into their discard pile.");
        }
        (_, Place::Zone(q, "setAside")) if *q == p => {
            text(out, "sets ");
            cards(out);
            text(out, " aside.");
        }
        (_, Place::Zone(q, "exile")) if *q == p => {
            text(out, "exiles ");
            cards(out);
            text(out, ".");
        }
        (_, Place::Zone(q, "looking")) if *q == p => {
            text(out, "looks at ");
            cards(out);
            text(out, ".");
        }
        (_, Place::Pile(_)) => {
            text(out, if m.bottom { "puts " } else { "returns " });
            cards(out);
            text(
                out,
                if m.bottom {
                    " on the bottom of "
                } else {
                    " to "
                },
            );
            place_spans(out, &m.to, p);
            text(out, ".");
        }
        (_, Place::Trash) => {
            text(out, "trashes ");
            cards(out);
            text(out, ".");
        }
        (_, Place::InPlay(q)) if *q == p => {
            text(out, "puts ");
            cards(out);
            text(out, " into play.");
        }
        (_, to) => {
            text(out, "puts ");
            cards(out);
            let prep = match to {
                Place::Deck(_) if m.bottom => " on the bottom of ",
                Place::Deck(_) => " onto ",
                Place::Zone(_, k) if *k != "setAside" => " on ",
                _ => " into ",
            };
            text(out, prep);
            place_spans(out, to, p);
            text(out, ".");
        }
    }
}

fn counter_word(c: Counter, n: u32) -> String {
    match c {
        Counter::Coffers => format!("{n} Coffers"),
        Counter::Villagers => amount(n, "Villager", "Villagers"),
        Counter::Favors => amount(n, "Favor", "Favors"),
        Counter::Debt => format!("{n} debt"),
        Counter::VpTokens => format!("{n} VP"),
        Counter::PirateShip => amount(n, "coin token", "coin tokens"),
        Counter::Embargo => amount(n, "Embargo token", "Embargo tokens"),
        Counter::TradeRoute => amount(n, "coin token", "coin tokens"),
        Counter::Sun => amount(n, "Sun token", "Sun tokens"),
        Counter::Other => amount(n, "token", "tokens"),
    }
}

/// A player's counter: "gets +2 Coffers.", "uses 1 Coffers. (+$1)", "repays 2 debt.".
fn player_counter(out: &mut Vec<Span>, c: Counter, delta: i32) {
    let n = delta.unsigned_abs();
    let what = counter_word(c, n);
    let up = delta > 0;
    out.push(Span::Text(match c {
        Counter::Coffers if up => format!("gets +{what}."),
        Counter::Coffers => format!("uses {what}. (+${n})"),
        Counter::Villagers | Counter::Favors if up => format!("gets +{what}."),
        Counter::Villagers => format!("uses {what}."),
        Counter::Favors => format!("spends {what}."),
        Counter::Debt if up => format!("takes {what}."),
        Counter::Debt => format!("repays {what}."),
        Counter::VpTokens if up => format!("gets {what}."),
        Counter::VpTokens => format!("loses {what}."),
        Counter::PirateShip if up => format!("adds {what} to their Pirate Ship mat."),
        Counter::PirateShip => format!("removes {what} from their Pirate Ship mat."),
        _ if up => format!("gets {what}."),
        _ => format!("loses {what}."),
    }));
}

fn counter_place(out: &mut Vec<Span>, at: &CounterAt) {
    match at {
        CounterAt::Pile(c) => place_spans(out, &Place::Pile(c.clone()), 0),
        CounterAt::TradeRouteMat => out.push(Span::Text("the Trade Route mat".into())),
        CounterAt::Prophecy(Some(c)) => out.push(Span::Card(c.clone())),
        CounterAt::Prophecy(None) => out.push(Span::Text("the Prophecy".into())),
    }
}

/// A pile's or the table's counter: "2 Debt tokens are put on the Silver pile.".
fn table_counter(out: &mut Vec<Span>, c: Counter, delta: i32, at: &CounterAt) {
    let n = delta.unsigned_abs();
    let what = match c {
        Counter::VpTokens => amount(n, "VP token", "VP tokens"),
        Counter::Debt => amount(n, "Debt token", "Debt tokens"),
        Counter::TradeRoute if matches!(at, CounterAt::Pile(_)) => {
            amount(n, "Trade Route token", "Trade Route tokens")
        }
        c => counter_word(c, n),
    };
    let what = match what.strip_prefix("1 ") {
        Some(one) => with_article(one),
        None => what,
    };
    let verb = if n == 1 { "is" } else { "are" };
    let how = if delta > 0 { "put on" } else { "removed from" };
    out.push(Span::Text(capitalise(&format!("{what} {verb} {how} "))));
    counter_place(out, at);
    out.push(Span::Text(".".into()));
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

pub fn log_lines(entries: &[LogEntry], names: &[String], cards: &CardDb) -> Vec<LogLine> {
    let depth = depths(entries);
    let parsed: Vec<Option<Entry>> = entries.iter().map(|e| entry::parse(e, names)).collect();
    let mut out: Vec<LogLine> = vec![];
    let mut i = 0;
    while i < entries.len() {
        let e = &entries[i];
        let d = depth[i];
        // The entry at `j`, if it is by the same player at the same depth.
        let next = |j: usize| -> Option<&Entry> {
            let f = entries.get(j)?;
            (f.player == e.player && depth[j] == d)
                .then_some(parsed[j].as_ref())
                .flatten()
        };
        // The entry right after, by the same player at any depth: what a play or buy did.
        let then = || -> Option<&Entry> {
            let f = entries.get(i + 1)?;
            (f.player == e.player)
                .then_some(parsed[i + 1].as_ref())
                .flatten()
        };
        let mut spans: Vec<Span> = vec![];
        if let Some(p) = e.player {
            spans.push(Span::Player(p));
            spans.push(Span::Text(" ".into()));
        }
        let text = |spans: &mut Vec<Span>, t: &str| spans.push(Span::Text(t.into()));
        let mut used = 1;
        let mut turn = None;
        let p = e.player.unwrap_or(0);
        match &parsed[i] {
            Some(Entry::TurnStart(n)) => {
                spans.clear();
                turn = Some(*n);
                text(&mut spans, &format!("Turn {n} - "));
                if let Some(p) = e.player {
                    spans.push(Span::FullPlayer(p, seat_name(names, p)));
                }
            }
            Some(Entry::Played(card)) if is_treasure(cards, card) => {
                let mut played = vec![card.clone()];
                while let Some(Entry::Played(c)) = next(i + used)
                    && is_treasure(cards, c)
                {
                    played.push(c.clone());
                    used += 1;
                }
                let items = tally(&played);
                text(&mut spans, "plays ");
                push_list(&mut spans, &items, false);
                text(&mut spans, ".");
                if let Some(c) = coins_of(cards, &items) {
                    text(&mut spans, &format!(" (+${c})"));
                }
            }
            Some(Entry::Played(card)) => {
                text(&mut spans, "plays ");
                push_list(&mut spans, &[(card.clone(), 1)], false);
                match then() {
                    Some(Entry::UsedWay { way, card: c }) if c == card => {
                        used = 2;
                        text(&mut spans, " using ");
                        spans.push(Span::Card(way.clone()));
                        text(&mut spans, ".");
                    }
                    _ => text(&mut spans, "."),
                }
            }
            Some(Entry::UsedWay { way, .. }) => {
                text(&mut spans, "uses ");
                spans.push(Span::Card(way.clone()));
                text(&mut spans, ".");
            }
            Some(Entry::Named(card)) => {
                text(&mut spans, "names ");
                spans.push(Span::Card(card.clone()));
                text(&mut spans, ".");
            }
            Some(Entry::Bought(card)) => {
                let landscape = e.kind == "bought-landscape";
                let gained = match then() {
                    Some(Entry::Gained(c, to)) if c == card => Some(*to),
                    _ => None,
                };
                if gained.is_some() {
                    used = 2;
                    text(&mut spans, "buys and gains ");
                } else {
                    text(&mut spans, "buys ");
                }
                push_list(&mut spans, &[(card.clone(), 1)], landscape);
                text(&mut spans, &format!("{}.", gained.map_or("", gain_tail)));
            }
            Some(Entry::Overpaid { coins, potions }) => {
                let potions = match potions {
                    0 => String::new(),
                    n => format!(" and {}", amount(*n, "Potion", "Potions")),
                };
                text(&mut spans, &format!("overpays ${coins}{potions}."));
            }
            Some(Entry::Gained(card, to)) => {
                text(&mut spans, "gains ");
                push_list(&mut spans, &[(card.clone(), 1)], false);
                text(&mut spans, &format!("{}.", gain_tail(*to)));
            }
            Some(Entry::Drew(n)) => {
                let mut n = *n;
                while let Some(Entry::Drew(k)) = next(i + used) {
                    n += k;
                    used += 1;
                }
                text(
                    &mut spans,
                    &if n == 1 {
                        "draws a card.".into()
                    } else {
                        format!("draws {n} cards.")
                    },
                );
            }
            Some(Entry::Shuffled) => text(&mut spans, "shuffles their deck."),
            Some(Entry::RevealedHand(shown)) => {
                text(&mut spans, "reveals their hand: ");
                push_list(&mut spans, &tally(shown), false);
                text(&mut spans, ".");
            }
            Some(
                first @ (Entry::Discarded(_)
                | Entry::Trashed(_)
                | Entry::Revealed(_)
                | Entry::Received(_)
                | Entry::SetAside(_)
                | Entry::TopDecked(_)),
            ) => {
                let (verb, end, card) = listed(first).unwrap_or_default();
                let same = |f: &Entry| std::mem::discriminant(f) == std::mem::discriminant(first);
                let mut named: Vec<String> = card.map(str::to_string).into_iter().collect();
                let mut hidden = u32::from(card.is_none());
                while let Some(f) = next(i + used).filter(|f| same(f)) {
                    match listed(f).and_then(|l| l.2) {
                        Some(c) => named.push(c.to_string()),
                        None => hidden += 1,
                    }
                    used += 1;
                }
                text(&mut spans, verb);
                if matches!(first, Entry::Received(_)) {
                    push_list(&mut spans, &tally(&named), true);
                } else {
                    push_cards(&mut spans, &named, hidden);
                }
                text(&mut spans, end);
            }
            Some(Entry::Rotated(pile)) => {
                text(&mut spans, "rotates ");
                place_spans(&mut spans, &Place::Pile(pile.clone()), p);
                text(&mut spans, ".");
            }
            Some(Entry::Moved(m)) => moved_spans(&mut spans, m, p),
            Some(Entry::Looked(n)) => text(
                &mut spans,
                &if *n == 1 {
                    "looks at the top card of their deck.".into()
                } else {
                    format!("looks at the top {n} cards of their deck.")
                },
            ),
            Some(Entry::PlayerCounter(c, delta)) => {
                // Tokens taken from a pile: "takes 2 VP from the Shaman pile."
                let from_pile = match next_table(&parsed, i + 1) {
                    Some(Entry::TableCounter(k, back, CounterAt::Pile(pile)))
                        if k == c
                            && *back == -delta
                            && *delta > 0
                            && matches!(c, Counter::VpTokens | Counter::Debt) =>
                    {
                        Some(pile.clone())
                    }
                    _ => None,
                };
                match from_pile {
                    Some(pile) => {
                        used = 2;
                        let what = counter_word(*c, delta.unsigned_abs());
                        text(&mut spans, &format!("takes {what} from "));
                        place_spans(&mut spans, &Place::Pile(pile), p);
                        text(&mut spans, ".");
                    }
                    None => player_counter(&mut spans, *c, *delta),
                }
            }
            Some(Entry::TableCounter(c, delta, at)) => table_counter(&mut spans, *c, *delta, at),
            Some(Entry::TokenMoved { token, pile }) => match pile {
                Some(pile) => {
                    text(&mut spans, &format!("moves their {token} token to "));
                    place_spans(&mut spans, &Place::Pile(pile.clone()), p);
                    text(&mut spans, ".");
                }
                None => text(&mut spans, &format!("takes back their {token} token.")),
            },
            Some(Entry::Marker { marker, on }) => text(
                &mut spans,
                match (marker, on) {
                    (Marker::MinusCard, true) => "takes the -1 Card token.",
                    (Marker::MinusCard, false) => "returns their -1 Card token.",
                    (Marker::MinusCoin, true) => "takes the -$1 token.",
                    (Marker::MinusCoin, false) => "returns their -$1 token.",
                    (Marker::Journey, true) => "flips their Journey token face up.",
                    (Marker::Journey, false) => "flips their Journey token face down.",
                },
            ),
            Some(Entry::Returned(card)) => {
                text(&mut spans, "returns ");
                spans.push(Span::Card(card.clone()));
                text(&mut spans, ".");
            }
            Some(Entry::Resolved(card)) => {
                spans.pop();
                text(&mut spans, "'s ");
                spans.push(Span::Card(card.clone()));
                text(&mut spans, " resolves.");
            }
            Some(Entry::Bonus {
                actions,
                buys,
                coins,
            }) => {
                let mut parts = vec![];
                if *actions > 0 {
                    parts.push(Span::Text(format!(
                        "+{}",
                        amount(*actions, "Action", "Actions")
                    )));
                }
                if *buys > 0 {
                    parts.push(Span::Text(format!("+{}", amount(*buys, "Buy", "Buys"))));
                }
                if *coins > 0 {
                    parts.push(Span::Text(format!("+${coins}")));
                }
                text(&mut spans, "gets ");
                join(&mut spans, parts);
                text(&mut spans, ".");
            }
            Some(Entry::Unaffected { by, .. }) => {
                text(&mut spans, "blocks with ");
                spans.push(Span::Card(by.clone()));
                text(&mut spans, ".");
            }
            Some(Entry::ReturnedToAction) => text(&mut spans, "returns to their Action phase."),
            Some(Entry::Called(card)) => {
                text(&mut spans, "calls ");
                push_list(&mut spans, &[(card.clone(), 1)], false);
                text(&mut spans, ".");
            }
            None => {
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

/// The entry at `j` if it is a table's counter (which has no player).
fn next_table(parsed: &[Option<Entry>], j: usize) -> Option<&Entry> {
    parsed
        .get(j)?
        .as_ref()
        .filter(|e| matches!(e, Entry::TableCounter(..)))
}

/// Where a gain went, after the card: "to their hand", "onto their drawpile".
fn gain_tail(to: GainTo) -> &'static str {
    match to {
        GainTo::Hand => " to their hand",
        GainTo::Deck => " onto their drawpile",
        GainTo::SetAside => " setting it aside",
        GainTo::Discard => "",
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
