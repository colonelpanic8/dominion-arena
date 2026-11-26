//! Text renderings of agent views and their official card references.

use crate::project::cost_text;
use dominion_engine::{Card, Kind, Types};
use dominion_protocol::view::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

fn write_entry(out: &mut String, entry: &GlossaryEntry, indent: &str) {
    let _ = write!(
        out,
        "\n\n{indent}{} | {}",
        entry.name,
        entry.types.join(" - ")
    );
    if let Some(cost) = &entry.cost {
        let _ = write!(out, " | {}", cost_text(cost));
    }
    for line in entry.text.lines() {
        let _ = write!(out, "\n{indent}  {line}");
    }
}

fn describe_structure(pile: &PileView) -> Option<String> {
    let structure = pile.structure.as_ref()?;
    if structure.kind == PileKind::Stacked && structure.cards.len() == 1 {
        return None;
    }
    let groups = structure
        .cards
        .iter()
        .map(|group| format!("{} {}", group.count, group.card))
        .collect::<Vec<_>>()
        .join(", ");
    let mut s = match structure.kind {
        PileKind::Stacked => format!("stacked; initially top to bottom: {groups}."),
        PileKind::Rotating => format!(
            "rotating; initially top to bottom: {groups}. Rotation happens only when an effect says so: move the top card and consecutive copies directly under it to the bottom."
        ),
        PileKind::Shuffled => {
            let mut text = if let Some(size) = structure.sample_size {
                format!(
                    "shuffled; random sample of {size} cards without replacement from: {groups}. Exact selected counts are not revealed."
                )
            } else {
                format!("shuffled; initial contents: {groups}.")
            };
            text.push_str(if pile.hidden {
                " Order is hidden."
            } else {
                " Only the current top is exposed; order below it is hidden."
            });
            text
        }
    };
    if pile.hidden {
        s.push_str(" Face down, including the top card.");
    }
    Some(s)
}

fn present_names(active: &BTreeSet<Card>, pred: impl Fn(Card) -> bool) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = active
        .iter()
        .copied()
        .filter(|&c| pred(c))
        .map(Card::name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

fn pile_triggers(card: Card, view: &AgentView, active: &BTreeSet<Card>) -> Vec<String> {
    let mut notes = vec![];
    for note in &view.notes {
        if note.pile.as_deref() == Some(card.name()) {
            match note.setup {
                Some(SetupRole::Bane) => notes.push(format!("Bane pile for {}", note.source)),
                Some(SetupRole::ExtraPile) => {
                    let location = if view.supply.iter().any(|pile| pile.card == card.name()) {
                        "Supply"
                    } else {
                        "non-Supply"
                    };
                    notes.push(format!("{location} pile added by {}", note.source))
                }
                Some(SetupRole::ChosenPile) => {
                    notes.push(format!("chosen pile for {}", note.source))
                }
                _ => {}
            }
        }
    }
    if let Some(pile) = view
        .supply
        .iter()
        .chain(&view.non_supply)
        .find(|p| p.card == card.name())
    {
        for t in &pile.traits {
            notes.push(format!("has Trait: {t}"));
        }
    }
    let users = present_names(active, |source| source != card && source.uses_pile(card));
    if !users.is_empty() {
        notes.push(format!("used by {}", users.join(", ")));
    }
    for (kind, label) in [(Types::PRIZE, "Prize"), (Types::REWARD, "Reward")] {
        let piles: Vec<_> = card.related().piles.iter().filter(|c| c.is(kind)).collect();
        if !piles.is_empty() {
            let counts: Option<Vec<u32>> = piles
                .iter()
                .map(|c| {
                    let pile = view.non_supply.iter().find(|pile| pile.card == c.name())?;
                    let structure = pile.structure.as_ref()?;
                    Some(structure.cards.iter().map(|group| group.count).sum())
                })
                .collect();
            let copies = counts
                .filter(|counts| counts.iter().all(|&n| n == counts[0]))
                .map(|counts| {
                    format!(
                        " ({} each)",
                        dominion_engine::amount(counts[0], "copy", "copies")
                    )
                })
                .unwrap_or_default();
            notes.push(format!(
                "adds {} non-Supply {label} piles{copies}",
                piles.len()
            ));
        }
    }
    if let Some(heirloom) = card.related().heirloom {
        notes.push(format!(
            "replaces 1 starting Copper with Heirloom: {}",
            heirloom.name()
        ));
    }
    let artifacts: Vec<&'static str> = card.related().artifacts.iter().map(|c| c.name()).collect();
    if !artifacts.is_empty() {
        notes.push(format!("uses Artifact: {}", artifacts.join(", ")));
    }
    if !card.related().trash.is_empty() {
        let cards: Vec<_> = card.related().trash.iter().map(|c| c.name()).collect();
        notes.push(format!("puts {} into the trash at setup", cards.join(", ")));
    }
    notes.extend(setup_descriptions(card.name(), view));
    notes
}

fn setup_descriptions(source: &str, view: &AgentView) -> Vec<String> {
    let mut descriptions = Vec::new();
    let selected: Vec<_> = view
        .notes
        .iter()
        .filter(|note| note.source == source && note.setup == Some(SetupRole::SetAside))
        .filter_map(|note| note.card.as_deref())
        .collect();
    if !selected.is_empty() {
        let boons = selected
            .iter()
            .all(|name| Card::from_name(name).is_some_and(|c| c.kind() == Kind::Boon));
        let label = if boons {
            "set-aside Boons"
        } else {
            "set-aside cards"
        };
        descriptions.push(format!("{label}: {}", selected.join(", ")));
    }
    for note in view.notes.iter().filter(|note| note.source == source) {
        if let Some(pile) = &note.pile {
            let label = match note.setup {
                Some(SetupRole::Bane) => "Bane pile",
                Some(SetupRole::ExtraPile) => {
                    if view.supply.iter().any(|candidate| candidate.card == *pile) {
                        "extra Supply pile"
                    } else {
                        "extra non-Supply pile"
                    }
                }
                Some(SetupRole::ChosenPile) => "chosen pile",
                _ => continue,
            };
            descriptions.push(format!("{label}: {pile}"));
        }
    }
    descriptions
}

fn pile_member_cards(pile: &PileView, card: Card) -> Vec<Card> {
    let mut members = Vec::new();
    let mut push = |c: Card| {
        if !members.contains(&c) {
            members.push(c);
        }
    };
    if card.kind() == Kind::Randomizer {
        push(card);
    }
    if let Some(structure) = &pile.structure {
        for group in &structure.cards {
            if let Some(c) = Card::from_name(&group.card) {
                push(c);
            }
        }
    } else {
        for c in card.pile_members() {
            push(c);
        }
    }
    if members.is_empty() {
        members.push(card);
    }
    members
}

fn take_entry<'a>(
    entries: &BTreeMap<&str, &'a GlossaryEntry>,
    shown: &mut BTreeSet<&'a str>,
    name: &str,
) -> Option<&'a GlossaryEntry> {
    let entry = entries.get(name).copied()?;
    shown.insert(entry.name.as_str()).then_some(entry)
}

fn direct_companions(card: Card, view: &AgentView) -> Vec<Card> {
    let mut related: Vec<Card> = card
        .related()
        .piles
        .iter()
        .copied()
        .filter(|pile| pile.pile_sources().all(|source| source == card))
        .collect();
    related.extend(card.related().heirloom);
    related.extend(card.related().trash);
    related.extend(card.related().artifacts);
    related.extend(
        view.notes
            .iter()
            .filter(|note| note.source == card.name() && note.setup == Some(SetupRole::SetAside))
            .filter_map(|note| note.card.as_deref().and_then(Card::from_name)),
    );
    related
}

fn exclusive_companions(card: Card, view: &AgentView) -> Vec<Card> {
    let mut related = direct_companions(card, view);
    let mut index = 0;
    while index < related.len() {
        for child in direct_companions(related[index], view) {
            if child != card && !related.contains(&child) {
                related.push(child);
            }
        }
        index += 1;
    }
    related
}

/// A structured reference, grouping cards under their piles and mechanics while
/// preserving official wording and dividing lines.
pub fn reference(view: &AgentView, entries: &[GlossaryEntry]) -> String {
    let by_name: BTreeMap<&str, &GlossaryEntry> =
        entries.iter().map(|e| (e.name.as_str(), e)).collect();
    let active = crate::component_sources(view);
    let mut shown: BTreeSet<&str> = BTreeSet::new();
    let mut out = String::new();

    out.push_str(
        "=== CARD & PILE REFERENCE ===\n\
         Printed text and costs (initial pile composition, not remaining counts).\n\
         Only the top card of a Supply pile can be bought or gained; use the current view for availability and current costs.",
    );

    let basic_piles: Vec<&PileView> = view
        .supply
        .iter()
        .filter(|p| Card::from_name(&p.card).is_some_and(|c| c.is_basic()))
        .collect();
    let shelters: Vec<&GlossaryEntry> = Card::ALL
        .iter()
        .copied()
        .filter(|c| c.is(Types::SHELTER))
        .filter_map(|c| by_name.get(c.name()).copied())
        .collect();
    if !basic_piles.is_empty() || !shelters.is_empty() {
        out.push_str("\n\n=== BASIC SUPPLY ===");
        for pile in basic_piles {
            if let Some(card) = Card::from_name(&pile.card) {
                let triggers = pile_triggers(card, view, &active);
                if let Some(entry) = take_entry(&by_name, &mut shown, &pile.card) {
                    write_entry(&mut out, entry, "");
                    if !triggers.is_empty() {
                        let _ = write!(out, "\n  [Setup: {}]", triggers.join("; "));
                    }
                }
            }
        }
        if !shelters.is_empty() {
            out.push_str("\n\n--- Shelters (replace starting Estates) ---");
            for s in shelters {
                if let Some(entry) = take_entry(&by_name, &mut shown, &s.name) {
                    write_entry(&mut out, entry, "  ");
                }
            }
        }
    }

    let supply_piles: Vec<&PileView> = view
        .supply
        .iter()
        .filter(|p| !Card::from_name(&p.card).is_some_and(|c| c.is_basic()))
        .collect();
    if !supply_piles.is_empty() {
        out.push_str("\n\n=== KINGDOM & SUPPLY PILES ===");
        for pile in supply_piles {
            let Some(card) = Card::from_name(&pile.card) else {
                continue;
            };
            let structure_desc = describe_structure(pile);
            let triggers = pile_triggers(card, view, &active);
            let members = pile_member_cards(pile, card);

            let mut companions = exclusive_companions(card, view);
            companions.extend(pile.traits.iter().filter_map(|name| Card::from_name(name)));
            let companion_piles: Vec<&PileView> = view
                .notes
                .iter()
                .filter(|note| note.source == pile.card && note.setup == Some(SetupRole::ExtraPile))
                .filter_map(|note| note.pile.as_deref())
                .filter_map(|name| view.non_supply.iter().find(|pile| pile.card == name))
                .collect();
            for related in &companion_piles {
                if let Some(card) = Card::from_name(&related.card) {
                    for member in pile_member_cards(related, card) {
                        companions.push(member);
                        companions.extend(exclusive_companions(member, view));
                    }
                }
            }
            let is_group = structure_desc.is_some()
                || members.len() > 1
                || !companions.is_empty()
                || !triggers.is_empty();
            if is_group {
                let mut header = format!("\n\n--- {} Pile", pile.card);
                if !triggers.is_empty() {
                    let _ = write!(header, " ({})", triggers.join("; "));
                }
                header.push_str(" ---");
                out.push_str(&header);
                if let Some(desc) = structure_desc {
                    let _ = write!(out, "\n{}: {desc}", pile.card);
                }
                for m in members {
                    if let Some(entry) = take_entry(&by_name, &mut shown, m.name()) {
                        write_entry(&mut out, entry, "  ");
                    }
                }
                for related in companion_piles {
                    let _ = write!(out, "\n\n  Related non-Supply pile: {}", related.card);
                    if let Some(description) = describe_structure(related) {
                        let _ = write!(out, "\n  {}: {description}", related.card);
                    }
                }
                for comp in companions {
                    if let Some(entry) = take_entry(&by_name, &mut shown, comp.name()) {
                        write_entry(&mut out, entry, "  ");
                    }
                }
            } else if let Some(entry) = take_entry(&by_name, &mut shown, card.name()) {
                write_entry(&mut out, entry, "");
            }
        }
    }

    let mut landscapes_and_artifacts = Vec::new();
    for l in &view.landscapes {
        if by_name.contains_key(l.card.as_str()) && !shown.contains(l.card.as_str()) {
            landscapes_and_artifacts.push(l.card.as_str());
        }
    }
    for a in &view.artifacts {
        if by_name.contains_key(a.card.as_str()) && !shown.contains(a.card.as_str()) {
            landscapes_and_artifacts.push(a.card.as_str());
        }
    }
    if !landscapes_and_artifacts.is_empty() {
        out.push_str("\n\n=== LANDSCAPES & ARTIFACTS ===");
        for name in landscapes_and_artifacts {
            let Some(entry) = take_entry(&by_name, &mut shown, name) else {
                continue;
            };
            write_entry(&mut out, entry, "");
            if let Some(vp) = view
                .landscapes
                .iter()
                .find(|landscape| landscape.card == name)
                .and_then(|landscape| landscape.vp)
            {
                let _ = write!(out, "\n  Current: {vp} VP remaining");
            }
            if let Some(card) = Card::from_name(name) {
                match card.kind() {
                    Kind::Ally => {
                        let liaisons = present_names(&active, |c| c.is(Types::LIAISON));
                        if !liaisons.is_empty() {
                            let _ = write!(
                                out,
                                "\n  [Setup: added for Liaison: {}]",
                                liaisons.join(", ")
                            );
                        }
                    }
                    Kind::Prophecy => {
                        let omens = present_names(&active, |c| c.is(Types::OMEN));
                        if !omens.is_empty() {
                            let _ =
                                write!(out, "\n  [Setup: added for Omen: {}]", omens.join(", "));
                        }
                    }
                    Kind::Artifact => {
                        let srcs =
                            present_names(&active, |c| c.related().artifacts.contains(&card));
                        if !srcs.is_empty() {
                            let _ = write!(out, "\n  [Setup: used by {}]", srcs.join(", "));
                        }
                    }
                    Kind::Trait => {
                        let piles: Vec<&str> = view
                            .supply
                            .iter()
                            .filter(|p| p.traits.iter().any(|t| t == name))
                            .map(|p| p.card.as_str())
                            .collect();
                        if !piles.is_empty() {
                            let _ = write!(out, "\n  [Setup: on {} pile]", piles.join(", "));
                        }
                    }
                    _ => {}
                }
                let setup = setup_descriptions(name, view);
                if !setup.is_empty() {
                    let _ = write!(out, "\n  [Setup: {}]", setup.join("; "));
                }
                for companion in exclusive_companions(card, view) {
                    if let Some(entry) = take_entry(&by_name, &mut shown, companion.name()) {
                        write_entry(&mut out, entry, "  ");
                    }
                }
            }
        }
    }

    let mut non_supply_unshown = Vec::new();
    for pile in &view.non_supply {
        let Some(card) = Card::from_name(&pile.card) else {
            continue;
        };
        let members = pile_member_cards(pile, card);
        if members
            .iter()
            .any(|m| by_name.contains_key(m.name()) && !shown.contains(m.name()))
        {
            non_supply_unshown.push((pile, card, members));
        }
    }
    if !non_supply_unshown.is_empty() {
        out.push_str("\n\n=== NON-SUPPLY PILES ===");
        for (pile, card, members) in non_supply_unshown {
            let structure_desc = describe_structure(pile);
            let triggers = pile_triggers(card, view, &active);
            let is_group = structure_desc.is_some() || members.len() > 1 || !triggers.is_empty();
            if is_group {
                let mut header = format!("\n\n--- {}", pile.card);
                if !triggers.is_empty() {
                    let _ = write!(header, " ({})", triggers.join("; "));
                }
                header.push_str(" ---");
                out.push_str(&header);
                if let Some(desc) = structure_desc {
                    let _ = write!(out, "\n{}: {desc}", pile.card);
                }
                for m in members {
                    if let Some(entry) = take_entry(&by_name, &mut shown, m.name()) {
                        write_entry(&mut out, entry, "  ");
                    }
                }
            } else if let Some(entry) = take_entry(&by_name, &mut shown, card.name()) {
                write_entry(&mut out, entry, "");
            }
        }
    }

    let mut decks: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for note in view
        .notes
        .iter()
        .filter(|note| note.setup == Some(SetupRole::Deck))
    {
        if let Some(name) = note.card.as_deref()
            && by_name.contains_key(name)
            && !shown.contains(name)
        {
            decks.entry(&note.source).or_default().push(name);
        }
    }
    for (source, cards) in decks {
        let _ = write!(out, "\n\n--- {source} Deck Cards (unordered) ---");
        for name in cards {
            if let Some(entry) = take_entry(&by_name, &mut shown, name) {
                write_entry(&mut out, entry, "  ");
            }
        }
    }

    let boons: Vec<&GlossaryEntry> = entries
        .iter()
        .filter(|e| {
            !shown.contains(e.name.as_str())
                && Card::from_name(&e.name).is_some_and(|c| c.kind() == Kind::Boon)
        })
        .collect();
    if !boons.is_empty() {
        let fate = present_names(&active, Card::receives_boons);
        let mut header = String::from("\n\n=== BOONS");
        let mut details = Vec::new();
        if !fate.is_empty() {
            details.push(format!("used by Fate: {}", fate.join(", ")));
        }
        if !details.is_empty() {
            let _ = write!(header, " ({})", details.join("; "));
        }
        header.push_str(" ===");
        out.push_str(&header);
        for b in boons {
            if let Some(entry) = take_entry(&by_name, &mut shown, &b.name) {
                write_entry(&mut out, entry, "");
            }
        }
    }

    let hexes_and_states: Vec<&GlossaryEntry> = entries
        .iter()
        .filter(|e| {
            !shown.contains(e.name.as_str())
                && Card::from_name(&e.name)
                    .is_some_and(|c| c.kind() == Kind::Hex || c.is_hex_state())
        })
        .collect();
    if !hexes_and_states.is_empty() {
        let doom = present_names(&active, |c| c.is(Types::DOOM));
        let mut header = String::from("\n\n=== HEXES & STATES");
        if !doom.is_empty() {
            let _ = write!(header, " (used by Doom: {})", doom.join(", "));
        }
        header.push_str(" ===");
        out.push_str(&header);
        for h in hexes_and_states {
            if let Some(entry) = take_entry(&by_name, &mut shown, &h.name) {
                write_entry(&mut out, entry, "");
            }
        }
    }

    let remaining: Vec<&GlossaryEntry> = entries
        .iter()
        .filter(|e| !shown.contains(e.name.as_str()))
        .collect();
    if !remaining.is_empty() {
        out.push_str("\n\n=== OTHER CARDS ===");
        for r in remaining {
            if let Some(entry) = take_entry(&by_name, &mut shown, &r.name) {
                write_entry(&mut out, entry, "");
            }
        }
    }

    out
}

fn cards(count: &CardCount) -> String {
    count
        .iter()
        .map(|(card, &n)| {
            if n == 1 {
                card.clone()
            } else {
                format!("{n} {card}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// An uncertain group's candidates, with those the viewer can't name.
fn candidates(known: &CardCount, unknown: u32) -> String {
    match unknown {
        0 => cards(known),
        n => format!("{}, {n} unknown", cards(known)),
    }
}

fn zone(count: u32, known: &CardCount, unknown: u32) -> String {
    let mut parts = vec![];
    if !known.is_empty() {
        parts.push(cards(known));
    }
    if unknown > 0 {
        parts.push(format!("{unknown} unknown"));
    }
    if parts.is_empty() {
        count.to_string()
    } else {
        format!("{count} ({})", parts.join("; "))
    }
}

fn player(out: &mut Vec<String>, label: &str, p: &PlayerView) {
    let vp = p.vp.map(|vp| format!(" ({vp} VP)")).unwrap_or_default();
    // Seats are referred to as P1, P2, ... elsewhere (and in engine-made labels).
    let seat = format!("P{}", p.index + 1);
    let name = if p.name == seat {
        String::new()
    } else {
        format!(": {}", p.name)
    };
    out.push(format!("{label} {seat}{name}{vp}"));
    out.push(format!(
        "  hand {}",
        zone(p.hand.count, &p.hand.known, p.hand.unknown)
    ));
    if p.in_play.count > 0 {
        out.push(format!(
            "  in play {}",
            zone(p.in_play.count, &p.in_play.known, p.in_play.unknown)
        ));
    }
    out.push(format!(
        "  deck {}",
        zone(p.deck.count, &p.deck.known, p.deck.unknown)
    ));
    out.push(format!(
        "  discard {}",
        zone(p.discard.count, &p.discard.known, p.discard.unknown)
    ));
    for (name, z) in &p.other_zones {
        out.push(format!("  {name} {}", zone(z.count, &z.known, z.unknown)));
    }
    for u in &p.uncertain {
        let outside = if u.outside > 0 {
            format!(", {} of them elsewhere", u.outside)
        } else {
            String::new()
        };
        out.push(format!(
            "  {} hold {} of: {}{outside}",
            u.zones.join("+"),
            u.count,
            candidates(&u.candidates, u.unknown)
        ));
    }
    if !p.unlocated.is_empty() {
        out.push(format!("  unlocated {}", cards(&p.unlocated)));
    }
    let owned = cards(&p.owned.known);
    let unknown = if p.owned.unknown > 0 {
        format!("; {} unknown", p.owned.unknown)
    } else {
        String::new()
    };
    let owned = if owned.is_empty() {
        "nothing known".into()
    } else {
        owned
    };
    out.push(format!("  owns {owned}{unknown}"));
    let t = &p.tokens;
    let mut extras: Vec<String> = [
        ("coffers", t.coffers as i64),
        ("villagers", t.villagers as i64),
        ("favors", t.favors as i64),
        ("debt", t.debt as i64),
        ("VP tokens", t.vp_tokens as i64),
        ("pirate ship", t.pirate_ship as i64),
    ]
    .into_iter()
    .filter(|(_, n)| *n != 0)
    .map(|(name, n)| format!("{name} {n}"))
    .collect();
    for (flag, name) in [(t.minus_card, "-1 Card token"), (t.minus_coin, "-$1 token")] {
        if flag {
            extras.push(name.into());
        }
    }
    match t.journey_face_up {
        Some(true) => extras.push("Journey token face up".into()),
        Some(false) => extras.push("Journey token face down".into()),
        None => {}
    }
    for (token, pile) in &p.adventure_tokens {
        extras.push(match pile {
            Some(pile) => format!("{token} token on {pile}"),
            None => format!("{token} token"),
        });
    }
    for (name, list) in [
        ("projects", &p.projects),
        ("states", &p.states),
        ("boons", &p.boons),
        ("artifacts", &p.artifacts),
    ] {
        if !list.is_empty() {
            extras.push(format!("{name} {}", list.join(", ")));
        }
    }
    for (name, n) in &p.counters {
        extras.push(format!("{name} {n}"));
    }
    if !extras.is_empty() {
        out.push(format!("  {}", extras.join(", ")));
    }
}

fn pile(p: &PileView) -> String {
    let mut s = p.card.clone();
    if let Some(structure) = &p.structure
        && (structure.kind != PileKind::Stacked || structure.cards.len() > 1)
    {
        s.push_str(match structure.kind {
            PileKind::Stacked => " (stacked)",
            PileKind::Rotating => " (rotating)",
            PileKind::Shuffled => " (shuffled)",
        });
    }
    if let Some(top) = &p.top {
        let _ = write!(s, " (top {top})");
    }
    if p.hidden {
        s.push_str(" (face down)");
    }
    if let Some(c) = &p.cost {
        let _ = write!(s, " {}", cost_text(c));
    }
    let _ = write!(s, " x{}", p.count);
    if !p.members.is_empty() {
        let _ = write!(s, " (members: {})", p.members.join(" / "));
    }
    let t = &p.tokens;
    let mut tokens = vec![];
    if t.embargo > 0 {
        tokens.push(format!("{} embargo", t.embargo));
    }
    if t.blockade > 0 {
        tokens.push(format!("{} blockade", t.blockade));
    }
    if t.trade_route {
        tokens.push("trade route".into());
    }
    if t.vp != 0 {
        tokens.push(format!("{} VP", t.vp));
    }
    if t.debt != 0 {
        tokens.push(format!("{} debt", t.debt));
    }
    for pt in &t.players {
        tokens.push(format!("P{}'s {}", pt.player + 1, pt.token));
    }
    tokens.extend(p.traits.iter().cloned());
    if !tokens.is_empty() {
        let _ = write!(s, " [{}]", tokens.join(", "));
    }
    s
}

fn decision(out: &mut Vec<String>, d: &AgentDecision) {
    out.push(format!("DECISION {}: {}", d.id, d.prompt));
    let many = d.groups.len() > 1;
    for g in &d.groups {
        let bounds = if g.min == g.max {
            format!("choose {}", g.min)
        } else {
            format!("choose {}-{}", g.min, g.max)
        };
        let mut notes = vec![bounds];
        if d.ordered {
            notes.push("order matters: list them top first".into());
        }
        if d.name_any {
            notes.push("or answer with any card's name".into());
        }
        if many {
            out.push(format!(
                "  [{}] {} ({})",
                g.kind,
                g.prompt,
                notes.join("; ")
            ));
        } else {
            out.push(format!("  {}", notes.join("; ")));
        }
        for o in d.options.iter().filter(|o| o.group == g.group) {
            out.push(format!("    {}: {}", o.index, o.label));
        }
    }
}

/// Renders a view as compact text for language models.
pub fn render(v: &AgentView) -> String {
    let t = &v.turn;
    let mut status = vec![];
    if let Some(n) = t.number {
        status.push(format!("turn {n}"));
    }
    if let Some(active) = &t.active_player {
        status.push(if t.your_turn {
            "your turn".into()
        } else {
            format!("{active}'s turn")
        });
    }
    if let Some(extra) = &t.extra {
        status.push(format!("extra turn from {extra}"));
    }
    if let Some(phase) = &t.phase {
        status.push(format!("{phase} phase"));
    }
    // A finished game's view keeps its last turn's counters; they matter to nobody now.
    if v.game.running {
        for (name, value) in [("actions", t.actions), ("buys", t.buys), ("coins", t.coins)] {
            if let Some(n) = value {
                status.push(format!("{name} {n}"));
            }
        }
        for (name, value) in [("potions", t.potions), ("debt", t.debt)] {
            if let Some(n) = value.filter(|&n| n > 0) {
                status.push(format!("{name} {n}"));
            }
        }
    }
    let over = if v.game.running { "" } else { " (over)" };
    let id = v.game.id.as_deref().unwrap_or("?");
    let status = if status.is_empty() {
        String::new()
    } else {
        format!(": {}", status.join(", "))
    };
    let mut out = vec![format!("GAME {id}{over}{status}")];
    if let Some(r) = &v.game.result {
        let mut ranked: Vec<&ResultPlayer> = r.players.iter().collect();
        ranked.sort_by_key(|p| (p.rank, p.index));
        let players: Vec<String> = ranked
            .iter()
            .map(|p| {
                let resigned = if p.resigned { ", resigned" } else { "" };
                format!(
                    "{}. {} {} VP ({} turns{resigned})",
                    p.rank, p.name, p.vp, p.turns
                )
            })
            .collect();
        let reason = match r.reason.as_deref() {
            Some("provinces-gone") => " (the Provinces ran out)",
            Some("colonies-gone") => " (the Colonies ran out)",
            Some("piles-empty") => " (Supply piles ran out)",
            Some("turn-limit") => " (the turn limit was reached)",
            Some("resigned") => " (a player resigned)",
            _ => "",
        };
        out.push(format!("RESULT{reason}: {}", players.join(", ")));
    }
    if let Some(you) = &v.you {
        out.push(String::new());
        player(&mut out, "YOU", you);
    }
    let other = if v.spectating { "PLAYER" } else { "OPPONENT" };
    for o in &v.opponents {
        out.push(String::new());
        player(&mut out, other, o);
    }
    let supply: Vec<String> = v.supply.iter().map(pile).collect();
    out.push("\nSUPPLY".into());
    out.extend(supply.iter().map(|pile| format!("  {pile}")));
    if !v.non_supply.is_empty() {
        let piles: Vec<String> = v.non_supply.iter().map(pile).collect();
        out.push("\nNON-SUPPLY".into());
        out.extend(piles.iter().map(|pile| format!("  {pile}")));
    }
    if !v.landscapes.is_empty() {
        let ls: Vec<String> = v
            .landscapes
            .iter()
            .map(|l| {
                let mut s = format!("{} ({})", l.card, l.kind);
                if let Some(c) = &l.cost {
                    let _ = write!(s, " {}", cost_text(c));
                }
                if let Some(vp) = l.vp {
                    let _ = write!(s, " {vp} VP");
                }
                if let Some(suns) = l.suns {
                    let _ = write!(s, " {}", dominion_engine::amount(suns, "Sun", "Suns"));
                }
                if l.active == Some(true) {
                    s.push_str(" active");
                }
                if !l.owners.is_empty() {
                    let owners: Vec<String> =
                        l.owners.iter().map(|p| format!("P{}", p + 1)).collect();
                    let _ = write!(s, " owned by {}", owners.join(", "));
                }
                s
            })
            .collect();
        out.push("\nLANDSCAPES".into());
        out.extend(ls.iter().map(|landscape| format!("  {landscape}")));
    }
    let trash = cards(&v.trash.known);
    let unknown = if v.trash.unknown > 0 {
        format!("; {} unknown", v.trash.unknown)
    } else {
        String::new()
    };
    out.push(format!(
        "TRASH {}{unknown}",
        if trash.is_empty() {
            "empty".into()
        } else {
            trash
        }
    ));
    for u in &v.uncertain {
        let zones: Vec<String> = u
            .zones
            .iter()
            .map(|z| match (z.player, &z.pile) {
                (Some(p), _) => format!("P{} {}", p + 1, z.zone),
                (None, Some(pile)) => format!("{pile} pile"),
                _ => z.zone.clone(),
            })
            .collect();
        out.push(format!(
            "UNCERTAIN {} hold {} of: {}",
            zones.join("+"),
            u.count,
            candidates(&u.candidates, u.unknown)
        ));
    }
    for (label, deck) in [("BOONS", &v.boons), ("HEXES", &v.hexes)] {
        if let Some(d) = deck {
            let discard = if d.discard.is_empty() {
                String::new()
            } else {
                format!("; discarded {}", d.discard.join(", "))
            };
            out.push(format!("{label} {} to draw{discard}", d.count));
        }
    }
    if !v.artifacts.is_empty() {
        let list: Vec<String> = v
            .artifacts
            .iter()
            .map(|a| match a.holder {
                Some(p) => format!("{} (P{})", a.card, p + 1),
                None => a.card.clone(),
            })
            .collect();
        out.push(format!("ARTIFACTS {}", list.join(", ")));
    }
    if !v.lasting.is_empty() {
        out.push("LASTING".into());
        for l in &v.lasting {
            let until = match (l.until.as_str(), l.until_player) {
                ("start-of-turn", Some(p)) => format!("until P{}'s next turn", p + 1),
                ("end-of-turn", _) => "this turn".into(),
                _ => "for as long as its card says".into(),
            };
            let mut s = format!("  {} (P{}) {until}", l.card, l.owner + 1);
            if !l.cards.is_empty() {
                let _ = write!(s, ": {}", l.cards.join(", "));
            }
            if let Some(n) = l.count {
                let _ = write!(s, " [{n}]");
            }
            out.push(s);
        }
    }
    if !v.extra_turns.is_empty() {
        let list: Vec<String> = v
            .extra_turns
            .iter()
            .map(|e| format!("P{} ({})", e.player + 1, e.source))
            .collect();
        out.push(format!("EXTRA TURNS {}", list.join(", ")));
    }
    if !v.notes.is_empty() {
        // Consecutive notes of one source about the same player and pile share a head.
        let list: Vec<String> = v
            .notes
            .chunk_by(|a, b| (&a.source, a.player, &a.pile) == (&b.source, b.player, &b.pile))
            .map(|group| {
                let n = &group[0];
                let mut parts = vec![];
                if let Some(p) = n.player {
                    parts.push(format!("P{}", p + 1));
                }
                if let Some(pile) = &n.pile {
                    parts.push(format!("{pile} pile"));
                }
                let items: Vec<String> = group
                    .iter()
                    .map(|n| {
                        if n.source == "Voyage" {
                            let n = dominion_engine::amount(n.count, "more card", "more cards");
                            return format!("{n} from their hand");
                        }
                        let mut item: Vec<String> = n.card.iter().cloned().collect();
                        if n.count != 0 {
                            item.push(n.count.to_string());
                        }
                        item.join(" ")
                    })
                    .filter(|item| !item.is_empty())
                    .collect();
                if !items.is_empty() {
                    parts.push(items.join(", "));
                }
                let mut s = n.source.clone();
                if !parts.is_empty() {
                    let _ = write!(s, ": {}", parts.join(" "));
                }
                s
            })
            .collect();
        out.push("\nNOTES".into());
        out.extend(list.iter().map(|note| format!("  {note}")));
    }
    if !v.since_last_decision.is_empty() {
        out.push("\nLOG".into());
        out.extend(
            v.since_last_decision
                .iter()
                .map(|e| format!("  {}", e.text)),
        );
    }
    if let Some(d) = &v.decision {
        out.push(String::new());
        decision(&mut out, d);
    }
    out.join("\n")
}
