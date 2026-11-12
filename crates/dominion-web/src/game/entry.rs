//! Log entries read back into what they say. The agent-view log carries an entry's kind,
//! player, card and text; the table needs what moved where and the facts to word it
//! dominion.games' way, so each kind's text is parsed in every form the log writes it.
//! An entry no rule here reads gives `None`.

use dominion_protocol::LogEntry;

/// A place cards are in, from the log's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Hand(u8),
    Deck(u8),
    Discard(u8),
    InPlay(u8),
    /// A player's other zone, by the view's key: `setAside`, `looking`, `tavern`,
    /// `island`, `nativeVillage`, `exile`.
    Zone(u8, &'static str),
    /// A pile, by the card it is named after.
    Pile(String),
    Trash,
}

impl Place {
    pub fn owner(&self) -> Option<u8> {
        match self {
            Place::Hand(p)
            | Place::Deck(p)
            | Place::Discard(p)
            | Place::InPlay(p)
            | Place::Zone(p, _) => Some(*p),
            Place::Pile(_) | Place::Trash => None,
        }
    }
}

/// Cards moved with no event of their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    /// `None` when the log doesn't say: a pile's card going to a pile, or a card put
    /// `back` onto the deck it was taken off (to look at it, or from the deck itself).
    pub from: Option<Place>,
    pub to: Place,
    /// Under the cards already there.
    pub bottom: bool,
    /// Under that many of the cards already there (into a deck), if not on top or at the
    /// bottom.
    pub under: u32,
    /// Put back onto the deck it came from.
    pub back: bool,
    /// The cards everyone knew, by name.
    pub cards: Vec<String>,
    /// How many more there were that the log doesn't name.
    pub hidden: u32,
}

impl Moved {
    pub fn count(&self) -> u32 {
        self.cards.len() as u32 + self.hidden
    }
}

/// Where a gained card went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GainTo {
    Discard,
    Hand,
    Deck,
    SetAside,
}

/// A count of tokens a player or the table has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    Coffers,
    Villagers,
    Favors,
    Debt,
    VpTokens,
    /// Coin tokens on a Pirate Ship mat.
    PirateShip,
    Embargo,
    /// Trade Route's tokens on a pile, or coin tokens on its mat.
    TradeRoute,
    Sun,
    /// Tokens the log doesn't name.
    Other,
}

/// Where a counter not a player's is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CounterAt {
    Pile(String),
    TradeRouteMat,
    /// The Prophecy, by its card when the log names it.
    Prophecy(Option<String>),
}

/// What a log entry says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    TurnStart(u32),
    Played(String),
    UsedWay {
        way: String,
        card: String,
    },
    Named(String),
    Chosen(String),
    CardCounter {
        card: String,
        pile: Option<String>,
        delta: i32,
    },
    Bought(String),
    Overpaid {
        coins: u32,
        potions: u32,
    },
    Gained(String, GainTo),
    Trashed(String),
    Discarded(String),
    /// A card returned to its pile for another, gained to the discard pile.
    Exchanged {
        card: String,
        into: String,
    },
    TopDecked(Option<String>),
    SetAside(String),
    Revealed(String),
    RevealedHand(Vec<String>),
    Drew(u32),
    Shuffled,
    Received(String),
    /// The pile rotated, by the card it is named after.
    Rotated(String),
    Moved(Moved),
    /// Looked at the top `n` cards of their deck.
    Looked(u32),
    /// A player's counter changed by `delta`.
    PlayerCounter(Counter, i32),
    /// A pile's or the table's counter changed by `delta`.
    TableCounter(Counter, i32, CounterAt),
    /// A player's Adventures token went onto a pile (by the card it is named after), or
    /// back to them.
    TokenMoved {
        token: String,
        pile: Option<String>,
    },
    /// A player took (`on`) or lost a -1 Card or -$1 token; a Journey token turned face
    /// up (`on`) or down.
    Marker {
        marker: Marker,
        on: bool,
    },
    Returned(String),
    Resolved(String),
    Bonus {
        actions: u32,
        buys: u32,
        coins: u32,
    },
    Unaffected {
        attack: String,
        by: String,
    },
    ReturnedToAction,
    Called(String),
    /// What is in the game changed during play (Divine Wind): a pile joined or left, or
    /// its setup added something. Worded as the log words it.
    Setup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    MinusCard,
    MinusCoin,
    Journey,
}

/// Seat `p`'s name as the log prints it.
pub fn seat_name(names: &[String], p: u8) -> String {
    names
        .get(usize::from(p))
        .cloned()
        .unwrap_or_else(|| format!("P{}", p + 1))
}

fn number(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// "1 Favor" or "2 Favors": the number, if `s` is `n` of `one` / `many`.
fn amount(s: &str, one: &str, many: &str) -> Option<u32> {
    let (n, what) = s.split_once(' ')?;
    let n = number(n)?;
    (what == if n == 1 { one } else { many }).then_some(n)
}

/// A card name as the log prints it: no list or place words in it.
fn card_name(s: &str) -> Option<String> {
    let bad = s.is_empty()
        || s.contains(", ")
        || [" from ", " they had ", " back onto ", " onto ", " into "]
            .iter()
            .any(|w| s.contains(w));
    (!bad).then(|| s.to_string())
}

/// `s` must be `e`'s card.
fn the_card(e: &LogEntry, s: &str) -> Option<String> {
    (e.card.as_deref() == Some(s)).then(|| s.to_string())
}

/// "their hand", "Bo's hand": whose, and the rest.
fn owned<'a>(s: &'a str, p: u8, names: &[String]) -> Option<(u8, &'a str)> {
    if let Some(rest) = s.strip_prefix("their ") {
        return Some((p, rest));
    }
    (0..names.len().max(p as usize + 1) as u8).find_map(|q| {
        let prefix = format!("{}'s ", seat_name(names, q));
        s.strip_prefix(prefix.as_str())
            .filter(|_| q != p)
            .map(|rest| (q, rest))
    })
}

/// A place as `p`'s log entry words it.
fn place(s: &str, p: u8, names: &[String]) -> Option<Place> {
    match s {
        "play" => return Some(Place::InPlay(p)),
        "set aside" => return Some(Place::Zone(p, "setAside")),
        "the trash" => return Some(Place::Trash),
        _ => {}
    }
    if let Some(rest) = s.strip_prefix("the top of ") {
        let (q, z) = owned(rest, p, names)?;
        return (z == "deck").then_some(Place::Zone(q, "looking"));
    }
    if let Some(pile) = s.strip_prefix("the ").and_then(|s| s.strip_suffix(" pile")) {
        return card_name(pile).map(Place::Pile);
    }
    let (q, z) = owned(s, p, names)?;
    Some(match z {
        "hand" => Place::Hand(q),
        "deck" => Place::Deck(q),
        "discard pile" => Place::Discard(q),
        "play area" if q != p => Place::InPlay(q),
        "set-aside cards" if q != p => Place::Zone(q, "setAside"),
        "Tavern mat" => Place::Zone(q, "tavern"),
        "Island mat" => Place::Zone(q, "island"),
        "Native Village mat" => Place::Zone(q, "nativeVillage"),
        "Exile mat" => Place::Zone(q, "exile"),
        _ => return None,
    })
}

/// A destination, possibly "the bottom of" one.
fn dest(s: &str, p: u8, names: &[String]) -> Option<(Place, bool)> {
    match s.strip_prefix("the bottom of ") {
        Some(rest) => place(rest, p, names).map(|pl| (pl, true)),
        None => place(s, p, names).map(|pl| (pl, false)),
    }
}

/// "Copper, Silver, 2 other cards": the named cards and how many more.
fn cards_list(s: &str) -> Option<(Vec<String>, u32)> {
    let mut cards = vec![];
    let mut hidden = 0;
    for item in s.split(", ") {
        let more = match item {
            "a card" | "a other card" => Some(1),
            _ => amount(item, "card", "cards")
                .or_else(|| amount(item, "other card", "other cards"))
                .filter(|&n| n > 1),
        };
        match more {
            Some(n) if hidden == 0 => hidden = n,
            Some(_) => return None,
            None if hidden == 0 => cards.push(card_name(item)?),
            None => return None,
        }
    }
    Some((cards, hidden))
}

/// "Copper from their hand", "2 cards they had set aside", "Estate".
fn cards_from(s: &str, p: u8, names: &[String]) -> Option<(Vec<String>, u32, Option<Place>)> {
    if let Some(what) = s.strip_suffix(" they had set aside") {
        let (cards, hidden) = cards_list(what)?;
        return Some((cards, hidden, Some(Place::Zone(p, "setAside"))));
    }
    for (at, _) in s.rmatch_indices(" from ") {
        if let (Some(from), Some((cards, hidden))) =
            (place(&s[at + 6..], p, names), cards_list(&s[..at]))
        {
            return Some((cards, hidden, Some(from)));
        }
    }
    let (cards, hidden) = cards_list(s)?;
    Some((cards, hidden, None))
}

/// The text of a `moved` entry after the player's name.
fn moved(s: &str, p: u8, names: &[String]) -> Option<Moved> {
    let make = |(cards, hidden, from): (Vec<String>, u32, Option<Place>), to, bottom, back| {
        Some(Moved {
            from,
            to,
            bottom,
            under: 0,
            back,
            cards,
            hidden,
        })
    };
    if let Some(rest) = s.strip_prefix("sets aside ") {
        return make(
            cards_from(rest, p, names)?,
            Place::Zone(p, "setAside"),
            false,
            false,
        );
    }
    if let Some(rest) = s.strip_prefix("returns ") {
        for (at, _) in rest.rmatch_indices(" to ") {
            if let Some((to @ Place::Pile(_), bottom)) = dest(&rest[at + 4..], p, names)
                && let Some(what) = cards_from(&rest[..at], p, names)
            {
                return make(what, to, bottom, false);
            }
        }
        return None;
    }
    let rest = s.strip_prefix("puts ")?;
    if let Some((rest, under)) = rest.rsplit_once(", under ") {
        let under = match under {
            "a card" => 1,
            n => amount(n, "card", "cards")?,
        };
        for (at, _) in rest.rmatch_indices(" into ") {
            if let Some((to @ Place::Deck(_), false)) = dest(&rest[at + 6..], p, names)
                && let Some(what) = cards_from(&rest[..at], p, names)
            {
                let m = make(what, to, false, false)?;
                return Some(Moved { under, ..m });
            }
        }
        return None;
    }
    for (at, _) in rest.rmatch_indices(" back onto ") {
        if let Some((to @ Place::Deck(_), bottom)) = dest(&rest[at + 11..], p, names)
            && let Some((cards, hidden)) = cards_list(&rest[..at])
        {
            return make((cards, hidden, None), to, bottom, true);
        }
    }
    for prep in [" onto ", " into ", " on "] {
        for (at, _) in rest.rmatch_indices(prep) {
            let Some((to, bottom)) = dest(&rest[at + prep.len()..], p, names) else {
                continue;
            };
            let fits = match prep {
                " onto " => matches!(to, Place::Deck(_)),
                " on " => matches!(to, Place::Zone(_, k) if !matches!(k, "setAside" | "looking")),
                _ => !matches!(to, Place::Deck(_) | Place::Pile(_)),
            };
            if fits && let Some(what) = cards_from(&rest[..at], p, names) {
                return make(what, to, bottom, false);
            }
        }
    }
    None
}

fn counter(s: &str) -> Option<(Counter, i32)> {
    let (verb, rest) = s.split_once(' ')?;
    if verb == "puts" || verb == "removes" {
        let (what, mat) = match verb {
            "puts" => rest.split_once(" on ")?,
            _ => rest.split_once(" from ")?,
        };
        let n = amount(what, "coin token", "coin tokens")? as i32;
        return (mat == "their Pirate Ship mat")
            .then_some((Counter::PirateShip, if verb == "puts" { n } else { -n }));
    }
    if let Some(rest) = s.strip_prefix("gets +") {
        let (n, what) = rest.split_once(' ')?;
        let c = match what {
            "Embargo" => Counter::Embargo,
            "TradeRoute" => Counter::TradeRoute,
            "Sun" => Counter::Sun,
            _ => return None,
        };
        return Some((c, number(n)? as i32));
    }
    if let Some(rest) = s.strip_prefix("pays off ") {
        let n = rest.strip_suffix(" Debt").and_then(number)?;
        return Some((Counter::Debt, -(n as i32)));
    }
    let up = match verb {
        "takes" => true,
        "spends" | "loses" => false,
        _ => return None,
    };
    let (n, c) = [
        (
            match rest {
                "a Coffers" => Some(1),
                _ => rest.strip_suffix(" Coffers").and_then(number),
            },
            Counter::Coffers,
        ),
        (rest.strip_suffix(" Debt").and_then(number), Counter::Debt),
        (amount(rest, "Villager", "Villagers"), Counter::Villagers),
        (amount(rest, "Favor", "Favors"), Counter::Favors),
        (amount(rest, "VP token", "VP tokens"), Counter::VpTokens),
    ]
    .into_iter()
    .find_map(|(n, c)| n.map(|n| (n, c)))?;
    let fits = match verb {
        "takes" => true,
        "spends" => matches!(c, Counter::Coffers | Counter::Villagers | Counter::Favors),
        _ => c == Counter::VpTokens,
    };
    let n = n as i32;
    fits.then_some((c, if up { n } else { -n }))
}

fn table_counter(s: &str) -> Option<(Counter, i32, CounterAt)> {
    let (what, at, up) = if let Some((what, at)) = s.split_once(" put on ") {
        (what, at, true)
    } else {
        let (what, at) = s.split_once(" removed from ")?;
        (what, at, false)
    };
    let (n, kind) = what.split_once(' ')?;
    let n = number(n)?;
    let thing = |one: &str, many: &str| kind == if n == 1 { one } else { many };
    let c = if thing("VP token", "VP tokens") {
        Counter::VpTokens
    } else if thing("Debt token", "Debt tokens") {
        Counter::Debt
    } else if thing("Embargo token", "Embargo tokens") {
        Counter::Embargo
    } else if thing("Trade Route token", "Trade Route tokens") || thing("coin token", "coin tokens")
    {
        Counter::TradeRoute
    } else if thing("Sun token", "Sun tokens") {
        Counter::Sun
    } else if thing("token", "tokens") {
        Counter::Other
    } else {
        return None;
    };
    let pile = at
        .strip_prefix("the ")
        .and_then(|s| s.strip_suffix(" pile"))
        .and_then(card_name);
    let at = match (pile, at) {
        (Some(pile), _) if c != Counter::Sun => CounterAt::Pile(pile),
        (_, "the Trade Route mat") if c == Counter::TradeRoute => CounterAt::TradeRouteMat,
        (_, "the Prophecy") => CounterAt::Prophecy(None),
        (None, card) => CounterAt::Prophecy(Some(card_name(card)?)),
        _ => return None,
    };
    let n = n as i32;
    Some((c, if up { n } else { -n }, at))
}

const TOKENS: [&str; 7] = [
    "+1 Card",
    "+1 Action",
    "+1 Buy",
    "+$1",
    "-$2 cost",
    "Trashing",
    "Estate",
];

fn bonus(s: &str) -> Option<(u32, u32, u32)> {
    let (mut actions, mut buys, mut coins) = (0, 0, 0);
    for part in s.split(", ") {
        if let Some(n) = part.strip_prefix("+$") {
            coins = number(n)?;
            continue;
        }
        let rest = part.strip_prefix('+')?;
        match amount(rest, "Action", "Actions") {
            Some(n) => actions = n,
            None => buys = amount(rest, "Buy", "Buys")?,
        }
    }
    (actions + buys + coins > 0).then_some((actions, buys, coins))
}

/// What `e` says, or `None` if no rule reads it. `names` are the seats' names.
pub fn parse(e: &LogEntry, names: &[String]) -> Option<Entry> {
    if matches!(
        e.kind.as_str(),
        "pile-added" | "pile-removed" | "added" | "noted"
    ) {
        return Some(Entry::Setup);
    }
    if e.kind == "card-counter" {
        let s = match e.player {
            Some(p) => e.text.strip_prefix(&format!("{}: ", seat_name(names, p)))?,
            None => &e.text,
        };
        let (count, rest) = s.split_once(' ')?;
        let n = number(count)? as i32;
        let rest = rest.strip_prefix(if n == 1 { "token " } else { "tokens " })?;
        let (delta, at) = if let Some(at) = rest.strip_prefix("put on ") {
            (n, at)
        } else {
            (-n, rest.strip_prefix("removed from ")?)
        };
        let (card, pile) = match at.strip_prefix("the ") {
            Some(rest) => {
                let (pile, card) = rest.strip_suffix(')')?.split_once(" pile (")?;
                (card, Some(card_name(pile)?))
            }
            None => (at, None),
        };
        return Some(Entry::CardCounter {
            card: the_card(e, card)?,
            pile,
            delta,
        });
    }
    if e.kind == "counter" && e.player.is_none() {
        let (c, delta, at) = table_counter(&e.text)?;
        return Some(Entry::TableCounter(c, delta, at));
    }
    let p = e.player?;
    let name = seat_name(names, p);
    if e.kind == "resolved" {
        let card = e
            .text
            .strip_prefix(format!("{name}'s ").as_str())?
            .strip_suffix(" resolves")?;
        return the_card(e, card).map(Entry::Resolved);
    }
    let s = e.text.strip_prefix(name.as_str())?.strip_prefix(' ')?;
    let after = |prefix: &str| s.strip_prefix(prefix);
    let card_after = |prefix: &str| after(prefix).and_then(|c| the_card(e, c));
    Some(match e.kind.as_str() {
        "turn-start" => Entry::TurnStart(after("starts turn ").and_then(number)?),
        "played" => Entry::Played(card_after("plays ")?),
        "used-way" => {
            let (way, card) = after("uses ")?.split_once(" for ")?;
            Entry::UsedWay {
                way: the_card(e, way)?,
                card: card_name(card)?,
            }
        }
        "named" => Entry::Named(card_after("names ")?),
        "chosen" => Entry::Chosen(after("chooses ")?.to_string()),
        "bought" | "bought-landscape" => Entry::Bought(card_after("buys ")?),
        "overpaid" => {
            let rest = after("overpays $")?;
            let (coins, potions) = match rest.split_once(" and ") {
                Some((c, pot)) => (c, amount(pot, "Potion", "Potions")?),
                None => (rest, 0),
            };
            Entry::Overpaid {
                coins: number(coins)?,
                potions,
            }
        }
        "gained" => {
            let rest = after("gains ")?;
            let (card, to) = if let Some(c) = rest.strip_suffix(" to their hand") {
                (c, GainTo::Hand)
            } else if let Some(c) = rest.strip_suffix(" onto their deck") {
                (c, GainTo::Deck)
            } else if let Some(c) = rest.strip_suffix(", setting it aside") {
                (c, GainTo::SetAside)
            } else {
                (rest, GainTo::Discard)
            };
            Entry::Gained(the_card(e, card)?, to)
        }
        "trashed" => Entry::Trashed(card_after("trashes ")?),
        "discarded" => Entry::Discarded(card_after("discards ")?),
        "exchanged" => {
            let (card, into) = after("exchanges ")?.split_once(" for ")?;
            Entry::Exchanged {
                card: the_card(e, card)?,
                into: card_name(into)?,
            }
        }
        "top-decked" => {
            let what = after("puts ")?.strip_suffix(" onto their deck")?;
            match (what, &e.card) {
                ("a card", None) => Entry::TopDecked(None),
                (c, Some(_)) => Entry::TopDecked(Some(the_card(e, c)?)),
                _ => return None,
            }
        }
        "set-aside" => Entry::SetAside(card_after("sets aside ")?),
        "revealed" => Entry::Revealed(card_after("reveals ")?),
        "revealed-hand" => {
            let list = after("reveals their hand: ")?;
            Entry::RevealedHand(match list {
                "nothing" => vec![],
                l => l.split(", ").map(card_name).collect::<Option<_>>()?,
            })
        }
        "drew" => Entry::Drew(amount(after("draws ")?, "card", "cards")?),
        "shuffled" if s == "shuffles" => Entry::Shuffled,
        "received" => Entry::Received(card_after("receives ")?),
        "rotated" => Entry::Rotated(card_name(after("rotates the ")?.strip_suffix(" pile")?)?),
        "moved" => {
            let m = moved(s, p, names)?;
            let named = (m.count() == 1).then(|| m.cards.first()).flatten();
            if named != e.card.as_ref() {
                return None;
            }
            Entry::Moved(m)
        }
        "looked" => {
            let rest = after("looks at the top ")?.strip_suffix(" of their deck")?;
            Entry::Looked(match rest {
                "card" => 1,
                r => r.strip_suffix(" cards").and_then(number)?,
            })
        }
        "counter" => {
            let (c, delta) = counter(s)?;
            Entry::PlayerCounter(c, delta)
        }
        "token-moved" => {
            if let Some(rest) = after("puts their ") {
                let (token, pile) = rest.split_once(" token on the ")?;
                let pile = card_name(pile.strip_suffix(" pile")?)?;
                TOKENS.contains(&token).then_some(())?;
                Entry::TokenMoved {
                    token: token.into(),
                    pile: Some(pile),
                }
            } else {
                let token = after("takes their ")?.strip_suffix(" token")?;
                TOKENS.contains(&token).then_some(())?;
                Entry::TokenMoved {
                    token: token.into(),
                    pile: None,
                }
            }
        }
        "marker" => {
            let (marker, on) = match s {
                "takes the -1 Card token" => (Marker::MinusCard, true),
                "removes their -1 Card token" => (Marker::MinusCard, false),
                "takes the -$1 token" => (Marker::MinusCoin, true),
                "removes their -$1 token" => (Marker::MinusCoin, false),
                "turns their Journey token face up" => (Marker::Journey, true),
                "turns their Journey token face down" => (Marker::Journey, false),
                _ => return None,
            };
            Entry::Marker { marker, on }
        }
        "returned" => Entry::Returned(card_after("returns ")?),
        "bonus" => {
            let (actions, buys, coins) = bonus(after("gets ")?)?;
            Entry::Bonus {
                actions,
                buys,
                coins,
            }
        }
        "unaffected" => {
            let rest = after("is unaffected by ")?.strip_suffix(')')?;
            let (attack, by) = rest.rsplit_once(" (")?;
            Entry::Unaffected {
                attack: card_name(attack)?,
                by: the_card(e, by)?,
            }
        }
        "returned-to-action" if s == "returns to their Action phase" => Entry::ReturnedToAction,
        "called" => Entry::Called(card_after("calls ")?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, player: Option<u8>, card: Option<&str>, text: &str) -> LogEntry {
        LogEntry {
            seq: 0,
            kind: kind.into(),
            player,
            card: card.map(Into::into),
            text: text.into(),
        }
    }

    fn names() -> Vec<String> {
        vec!["Ann".into(), "Bo on Top".into()]
    }

    fn moved_of(card: Option<&str>, text: &str) -> Moved {
        match parse(&entry("moved", Some(0), card, text), &names()) {
            Some(Entry::Moved(m)) => m,
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn moves_say_where_from_and_to() {
        let m = moved_of(
            Some("Island"),
            "Ann puts Island from play on their Island mat",
        );
        assert_eq!(
            (m.from, m.to, m.cards),
            (
                Some(Place::InPlay(0)),
                Place::Zone(0, "island"),
                vec!["Island".to_string()]
            )
        );
        let m = moved_of(
            None,
            "Ann puts Copper, 2 other cards from their hand into Bo on Top's hand",
        );
        assert_eq!(
            (m.from, m.to, m.cards, m.hidden),
            (
                Some(Place::Hand(0)),
                Place::Hand(1),
                vec!["Copper".to_string()],
                2
            )
        );
        let m = moved_of(None, "Ann puts 2 cards back onto their deck");
        assert!(m.back && m.from.is_none() && m.hidden == 2);
        let m = moved_of(
            None,
            "Ann puts a card from their discard pile onto the bottom of their deck",
        );
        assert_eq!(
            (m.from, m.to, m.bottom),
            (Some(Place::Discard(0)), Place::Deck(0), true)
        );
        let m = moved_of(
            Some("Humble Castle"),
            "Ann returns Humble Castle from their discard pile to the Castles pile",
        );
        assert_eq!(m.to, Place::Pile("Castles".into()));
        let m = moved_of(None, "Ann puts 2 cards they had set aside into their hand");
        assert_eq!(m.from, Some(Place::Zone(0, "setAside")));
        let m = moved_of(
            Some("Copper"),
            "Ann puts Copper from the top of their deck into their hand",
        );
        assert_eq!(m.from, Some(Place::Zone(0, "looking")));
        let m = moved_of(
            Some("Copper"),
            "Ann puts Copper from their hand into their deck, under 2 cards",
        );
        assert_eq!(
            (m.from, m.to, m.under),
            (Some(Place::Hand(0)), Place::Deck(0), 2)
        );
        let m = moved_of(None, "Ann sets aside a card from their hand");
        assert_eq!((m.to, m.hidden), (Place::Zone(0, "setAside"), 1));
        assert!(
            parse(
                &entry("moved", Some(0), None, "Ann puts Copper somewhere"),
                &names()
            )
            .is_none()
        );
    }

    #[test]
    fn counters_are_read_both_ways() {
        let p = |player, text: &str| parse(&entry("counter", player, None, text), &names());
        assert_eq!(
            p(Some(0), "Ann pays off 3 Debt"),
            Some(Entry::PlayerCounter(Counter::Debt, -3))
        );
        assert_eq!(
            p(Some(0), "Ann spends 1 Villager"),
            Some(Entry::PlayerCounter(Counter::Villagers, -1))
        );
        assert_eq!(
            p(None, "1 Sun token removed from Sickness"),
            Some(Entry::TableCounter(
                Counter::Sun,
                -1,
                CounterAt::Prophecy(Some("Sickness".into()))
            ))
        );
        assert_eq!(
            p(None, "2 Debt tokens put on the Silver pile"),
            Some(Entry::TableCounter(
                Counter::Debt,
                2,
                CounterAt::Pile("Silver".into())
            ))
        );
        assert_eq!(p(Some(0), "Ann spends 1 Debt"), None);
        assert_eq!(
            p(Some(0), "Ann spends a Coffers"),
            Some(Entry::PlayerCounter(Counter::Coffers, -1))
        );
        let on_pile = entry(
            "card-counter",
            Some(0),
            Some("Family of Inventors"),
            "Ann: 1 token put on the Odysseys pile (Family of Inventors)",
        );
        assert_eq!(
            parse(&on_pile, &names()),
            Some(Entry::CardCounter {
                card: "Family of Inventors".into(),
                pile: Some("Odysseys".into()),
                delta: 1
            })
        );
    }
}
