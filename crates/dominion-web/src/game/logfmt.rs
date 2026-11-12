//! The game log as display lines: turn headers, effects indented under the play or buy
//! they follow, text split into player, card and plain spans, and runs of identical
//! lines merged with a count.

use dominion_protocol::{AgentView, LogEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Card(String),
    Player(u8),
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
    /// How many identical entries in a row the line stands for.
    pub count: u32,
}

impl LogLine {
    pub fn is_turn(&self) -> bool {
        self.turn.is_some()
    }

    /// The line as plain text, with player names from `names`.
    pub fn text(&self, names: &[String]) -> String {
        let mut s: String = self
            .spans
            .iter()
            .map(|sp| match sp {
                Span::Text(t) | Span::Card(t) => t.clone(),
                Span::Player(p) => seat_name(names, *p),
            })
            .collect();
        if self.count > 1 {
            s.push_str(&format!(" ×{}", self.count));
        }
        s
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

fn push_text(spans: &mut Vec<Span>, t: &str) {
    if t.is_empty() {
        return;
    }
    if let Some(Span::Text(last)) = spans.last_mut() {
        last.push_str(t);
    } else {
        spans.push(Span::Text(t.into()));
    }
}

fn spans(e: &LogEntry, names: &[String]) -> Vec<Span> {
    let mut out = vec![];
    let mut rest = e.text.as_str();
    if let Some(p) = e.player
        && let Some(name) = names.get(usize::from(p))
        && let Some(r) = rest.strip_prefix(name.as_str())
    {
        out.push(Span::Player(p));
        rest = r;
    }
    if e.kind == "revealed-hand"
        && let Some((head, list)) = rest.split_once(": ")
        && list != "nothing"
    {
        push_text(&mut out, head);
        push_text(&mut out, ": ");
        for (i, card) in list.split(", ").enumerate() {
            if i > 0 {
                push_text(&mut out, ", ");
            }
            out.push(Span::Card(card.into()));
        }
        return out;
    }
    if let Some(card) = &e.card
        && let Some(at) = rest.find(card.as_str())
    {
        push_text(&mut out, &rest[..at]);
        out.push(Span::Card(card.clone()));
        push_text(&mut out, &rest[at + card.len()..]);
        return out;
    }
    push_text(&mut out, rest);
    out
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

pub fn log_lines(entries: &[LogEntry], names: &[String]) -> Vec<LogLine> {
    let depth = depths(entries);
    let mut out: Vec<LogLine> = vec![];
    for (e, depth) in entries.iter().zip(depth) {
        let turn = (e.kind == "turn-start")
            .then(|| e.text.rsplit(' ').next()?.parse().ok())
            .flatten();
        let line = LogLine {
            seq: e.seq,
            kind: e.kind.clone(),
            player: e.player,
            turn,
            depth,
            spans: spans(e, names),
            count: 1,
        };
        if let Some(last) = out.last_mut()
            && !line.is_turn()
            && last.kind == line.kind
            && last.player == line.player
            && last.depth == line.depth
            && last.spans == line.spans
        {
            last.count += 1;
            continue;
        }
        out.push(line);
    }
    out
}
