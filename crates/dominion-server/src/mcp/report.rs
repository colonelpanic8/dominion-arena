//! `game_report`: a game as a record, for whoever runs games to learn from them.

use super::play::{self, TurnsParams};
use crate::hub::Report;
use serde_json::{Value, json};
use std::fmt::Write as _;

/// The report's structured content and its text. Briefs and debriefs are in it only once
/// the game is over; `detail` adds each turn's log entries.
pub fn render(report: &Report, detail: bool) -> (Value, String) {
    let config = &report.config;
    let meta = &report.meta;
    let over = !report.running;
    let names =
        |cards: &[dominion_engine::Card]| -> Vec<&str> { cards.iter().map(|c| c.name()).collect() };
    let setup: serde_json::Map<String, Value> = (config.setup_choices.iter())
        .map(|(card, chosen)| (card.name().to_string(), json!(names(chosen))))
        .collect();
    let mut text = String::new();
    let reason = (report.result.as_ref())
        .and_then(|r| r.reason.clone())
        .or_else(|| meta.ended.clone());
    let _ = write!(text, "GAME {}", report.game_id);
    match (over, &reason) {
        (true, Some(reason)) => {
            let _ = write!(text, " over: {reason}");
        }
        (true, None) => text.push_str(" over"),
        (false, _) => text.push_str(" running"),
    }
    if over && !report.settled {
        text.push_str(" (debriefs still to come)");
    }
    text.push_str(
        "\nSETUP (create_game with these and the same seed deals each seat the same starting cards)",
    );
    let _ = write!(text, "\n  kingdom: {}", names(&config.kingdom).join(", "));
    if !config.landscapes.is_empty() {
        let _ = write!(
            text,
            "\n  landscapes: {}",
            names(&config.landscapes).join(", ")
        );
    }
    let _ = write!(text, "\n  seed: {}", config.seed);
    for (what, value) in [("colonies", config.colonies), ("shelters", config.shelters)] {
        if let Some(value) = value {
            let _ = write!(text, "\n  {what}: {value}");
        }
    }
    if let Some(limit) = config.turn_limit {
        let _ = write!(text, "\n  turn_limit: {limit}");
    }
    for (card, chosen) in &setup {
        let _ = write!(text, "\n  setup {card}: {chosen}");
    }

    let mut seats = vec![];
    text.push_str("\nSEATS");
    for (i, name) in report.names.iter().enumerate() {
        let user = report.users.get(i).cloned().flatten();
        let agent = meta.agents.get(i).copied().unwrap_or(false);
        let result = (report.result.as_ref())
            .and_then(|r| r.players.iter().find(|p| usize::from(p.index) == i));
        let _ = write!(text, "\n  {i} {name}");
        match (&user, agent) {
            (Some(user), true) => {
                let _ = write!(text, " (server agent {})", user.name);
            }
            (Some(user), false) if user.name != *name => {
                let _ = write!(text, " ({})", user.name);
            }
            _ => {}
        }
        if let Some(r) = result {
            let _ = write!(
                text,
                ": {} VP, place {}, {} turns{}",
                r.vp,
                r.rank,
                r.turns,
                if r.resigned { ", resigned" } else { "" }
            );
        }
        let mut seat = json!({
            "seat": i, "name": name, "account": user.as_ref().map(|u| &u.name),
            "server_agent": agent, "result": result,
        });
        if over {
            seat["brief"] = json!(meta.briefs.get(i).cloned().flatten());
            seat["debrief"] = json!(meta.debriefs.get(i).cloned().flatten());
            seat["deck"] = json!(report.decks.get(i));
        }
        seats.push(seat);
    }

    if over {
        text.push_str("\nDECKS");
        for (i, deck) in report.decks.iter().enumerate() {
            let count: u32 = deck.values().sum();
            let mut cards: Vec<_> = deck.iter().collect();
            cards.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let list: Vec<String> = cards.iter().map(|(n, c)| format!("{c} {n}")).collect();
            let _ = write!(
                text,
                "\n  {} ({count} cards): {}",
                report.names[i],
                list.join(", ")
            );
        }
        for (i, brief) in meta.briefs.iter().enumerate() {
            if let Some(brief) = brief {
                let _ = write!(
                    text,
                    "\nBRIEF seat {i} ({})\n{}",
                    report.names[i],
                    indent(brief)
                );
            }
        }
        if let Some(question) = &meta.debrief {
            let _ = write!(text, "\nDEBRIEF QUESTION\n{}", indent(question));
        }
        for (i, name) in report.names.iter().enumerate() {
            let debrief = meta.debriefs.get(i).cloned().flatten();
            let owed = meta.debrief.is_some() && meta.agents.get(i).copied().unwrap_or(false);
            match debrief {
                Some(debrief) => {
                    let _ = write!(text, "\nDEBRIEF seat {i} ({name})\n{}", indent(&debrief));
                }
                None if owed && !report.settled => {
                    let _ = write!(text, "\nDEBRIEF seat {i} ({name}): still to come");
                }
                None if owed => {
                    let _ = write!(text, "\nDEBRIEF seat {i} ({name}): none came");
                }
                None => {}
            }
        }
    }

    let spans = play::turns(
        &report.log,
        &report.names,
        report.running,
        &TurnsParams {
            player: None,
            from: Some(0),
            to: None,
            last: None,
            detail,
        },
    );
    text.push('\n');
    text.push_str(&play::turns_text(&spans, detail));
    let turns: Vec<_> = (spans.iter())
        .map(|(_, s)| {
            json!({"player": s.player, "turn": s.turn, "first_seq": s.first_seq,
                "last_seq": s.last_seq, "summary": s.summary})
        })
        .collect();
    let value = json!({
        "game": report.game_id,
        "running": report.running,
        "settled": report.settled,
        "reason": reason,
        "setup": {
            "kingdom": names(&config.kingdom),
            "landscapes": names(&config.landscapes),
            "seed": config.seed,
            "colonies": config.colonies,
            "shelters": config.shelters,
            "turn_limit": config.turn_limit,
            "setup": setup,
        },
        "seats": seats,
        "debrief_question": if over { json!(meta.debrief) } else { Value::Null },
        "turns": turns,
    });
    (value, text)
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
