//! What every producer of a view holds to beyond its JSON Schema: the names zones and
//! landscape kinds go by, counts that add up, and table extras that say no more than
//! the view. The table and the agents read a view by these, whoever made it: the
//! engine's projection and the dominion.games reducer both test theirs against
//! [`violations`].

use crate::api::TableExtras;
use crate::{AgentView, CardCount, PlayerView};
use std::collections::BTreeSet;

/// The zones a player's `otherZones` holds: set-aside cards, cards taken off a deck
/// to look at or reveal, and the mats.
pub const OTHER_ZONES: [&str; 6] = [
    "setAside",
    "looking",
    "tavern",
    "island",
    "nativeVillage",
    "exile",
];

/// The zones every view has, by the names an option's card or an uncertain group is
/// placed with.
pub const COMMON_ZONES: [&str; 6] = ["hand", "deck", "discard", "inPlay", "pile", "trash"];

/// Only an Event or a Project, which are bought, has a `cost`.
pub const LANDSCAPE_KINDS: [&str; 9] = [
    "event", "landmark", "project", "way", "ally", "trait", "prophecy", "study", "other",
];

fn is_zone(name: &str) -> bool {
    COMMON_ZONES.contains(&name) || OTHER_ZONES.contains(&name)
}

fn kebab(s: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    !s.is_empty() && s.chars().all(ok) && !s.starts_with('-') && !s.ends_with('-')
}

/// Everything `view`, and the `table` extras sent with it, get wrong, in words.
pub fn violations(view: &AgentView, table: Option<&TableExtras>) -> Vec<String> {
    let mut out = vec![];
    let mut zone = |what: String, count: u32, known: &CardCount, unknown: u32| {
        if known.values().sum::<u32>() + unknown != count {
            out.push(format!(
                "{what} holds {count} cards but accounts for another number"
            ));
        }
        if known.values().any(|&n| n == 0) {
            out.push(format!("{what} lists a card it has none of"));
        }
    };
    let players: Vec<&PlayerView> = view.you.iter().chain(&view.opponents).collect();
    zone(
        "the trash".into(),
        view.trash.count,
        &view.trash.known,
        view.trash.unknown,
    );
    for p in &players {
        let who = &p.name;
        zone(
            format!("{who}'s hand"),
            p.hand.count,
            &p.hand.known,
            p.hand.unknown,
        );
        zone(
            format!("{who}'s cards in play"),
            p.in_play.count,
            &p.in_play.known,
            p.in_play.unknown,
        );
        zone(
            format!("{who}'s deck"),
            p.deck.count,
            &p.deck.known,
            p.deck.unknown,
        );
        zone(
            format!("{who}'s discard pile"),
            p.discard.count,
            &p.discard.known,
            p.discard.unknown,
        );
        for (key, z) in &p.other_zones {
            zone(format!("{who}'s {key}"), z.count, &z.known, z.unknown);
        }
    }
    for p in &players {
        for (key, z) in &p.other_zones {
            if !OTHER_ZONES.contains(&key.as_str()) {
                out.push(format!("{}'s zone {key:?} has no such name", p.name));
            }
            if z.count == 0 {
                out.push(format!("{}'s zone {key:?} is listed empty", p.name));
            }
        }
        for u in &p.uncertain {
            for z in u.zones.iter().filter(|z| !is_zone(z)) {
                out.push(format!("{}'s uncertain cards are in {z:?}", p.name));
            }
        }
    }
    let seats: BTreeSet<u8> = players.iter().map(|p| p.index).collect();
    if seats.len() != players.len() {
        out.push("two players have one seat".into());
    }
    if let (Some(seat), Some(you)) = (view.seat, &view.you)
        && you.index != seat
    {
        out.push(format!(
            "the view is seat {seat}'s but `you` is seat {}",
            you.index
        ));
    }
    for u in &view.uncertain {
        for z in u.zones.iter().filter(|z| !is_zone(&z.zone)) {
            out.push(format!("uncertain cards are in {:?}", z.zone));
        }
    }
    for l in &view.landscapes {
        if !LANDSCAPE_KINDS.contains(&l.kind.as_str()) {
            out.push(format!("{} is a landscape of kind {:?}", l.card, l.kind));
        }
        if l.cost.is_some() && !matches!(l.kind.as_str(), "event" | "project" | "other") {
            out.push(format!("{} is a {} with a cost", l.card, l.kind));
        }
    }
    for e in &view.since_last_decision {
        if !kebab(&e.kind) {
            out.push(format!("log entry {} has type {:?}", e.seq, e.kind));
        }
    }
    for o in view.decision.iter().flat_map(|d| &d.options) {
        if let Some(z) = o.zone.as_deref().filter(|z| !is_zone(z)) {
            out.push(format!("option {} is in {z:?}", o.index));
        }
    }
    let Some(table) = table else { return out };
    if table.players.len() != players.len() {
        out.push("the table extras are not one per player".into());
    }
    for t in &table.players {
        let Some(p) = players.iter().find(|p| p.index == t.seat) else {
            out.push(format!("the table extras name seat {}", t.seat));
            continue;
        };
        let mut named = CardCount::new();
        for c in &t.in_play {
            *named.entry(c.card.clone()).or_default() += 1;
        }
        for (card, n) in &named {
            if p.in_play.known.get(card).copied().unwrap_or(0) < *n {
                out.push(format!(
                    "the table extras put {n} {card} in play for {}, more than the view knows",
                    p.name
                ));
            }
        }
        if t.discard_top.is_some() && p.discard.count == 0 {
            out.push(format!("{}'s empty discard pile has a top card", p.name));
        }
    }
    out
}
