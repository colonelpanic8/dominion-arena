//! The compact text rendering of an agent view, derived only from the view's JSON shape.

use crate::project::cost_text;
use crate::types::*;
use std::fmt::Write;

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
            cards(&u.candidates)
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
    let t = &p.tokens;
    let mut tokens = vec![];
    if t.embargo > 0 {
        tokens.push(format!("{} embargo", t.embargo));
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
            let zone = match o.zone.as_deref() {
                Some(z) if o.action == "select" && !["hand", "looking"].contains(&z) => {
                    format!(" (from {z})")
                }
                _ => String::new(),
            };
            out.push(format!("    {}: {}{zone}", o.index, o.label));
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
            .map(|p| format!("{}. {} {} VP ({} turns)", p.rank, p.name, p.vp, p.turns))
            .collect();
        let reason = match r.reason.as_deref() {
            Some("provinces-gone") => " (the Provinces ran out)",
            Some("colonies-gone") => " (the Colonies ran out)",
            Some("piles-empty") => " (Supply piles ran out)",
            Some("turn-limit") => " (the turn limit was reached)",
            _ => "",
        };
        out.push(format!("RESULT{reason}: {}", players.join(", ")));
    }
    if let Some(you) = &v.you {
        player(&mut out, "YOU", you);
    }
    for o in &v.opponents {
        player(&mut out, "OPPONENT", o);
    }
    let supply: Vec<String> = v.supply.iter().map(pile).collect();
    out.push(format!("SUPPLY {}", supply.join(", ")));
    if !v.non_supply.is_empty() {
        let piles: Vec<String> = v.non_supply.iter().map(pile).collect();
        out.push(format!("NON-SUPPLY {}", piles.join(", ")));
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
                    let _ = write!(s, " {suns} suns");
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
        out.push(format!("LANDSCAPES {}", ls.join(", ")));
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
            cards(&u.candidates)
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
                _ => "until removed".into(),
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
        out.push(format!("NOTES {}", list.join("; ")));
    }
    if !v.since_last_decision.is_empty() {
        out.push("LOG".into());
        out.extend(
            v.since_last_decision
                .iter()
                .map(|e| format!("  {}", e.text)),
        );
    }
    if let Some(d) = &v.decision {
        decision(&mut out, d);
    }
    out.join("\n")
}
