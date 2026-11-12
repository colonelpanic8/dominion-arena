//! Where the table draws each target of a decision, and how the cards in play divide
//! into this turn's and those kept from earlier turns. Plain data, tested natively.

use crate::game::board::{Board, PileAt, Stack, Zone};
use crate::game::interaction::{Interaction, Place};
use dominion_protocol::LogEntry;

/// The one thing on the table a target is clicked on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    /// A stack of the hand, by card.
    Hand(String),
    /// A stack of the seat's own cards in play, by card.
    InPlay(String),
    Pile(PileAt),
    Landscape(String),
    Tray,
    Button,
    Ability,
}

/// The spot of each of `inter.targets`, in order. A target whose place the board
/// doesn't show, or whose spot another target already took, goes to the tray.
pub fn spots(board: &Board, inter: &Interaction) -> Vec<Spot> {
    let you = board.you.as_ref();
    let holds =
        |z: Option<&Zone>, card: &str| z.is_some_and(|z| z.known.iter().any(|s| s.card == card));
    let piles: Vec<PileAt> = board
        .supply
        .basic
        .iter()
        .chain(board.supply.kingdom.iter().flatten())
        .chain(&board.non_supply)
        .map(|p| p.at)
        .collect();
    let mut taken: Vec<Spot> = vec![];
    inter
        .targets
        .iter()
        .map(|t| {
            let card = t.card.clone().unwrap_or_default();
            let want = match t.place {
                Place::Hand if holds(you.map(|y| &y.hand), &card) => Some(Spot::Hand(card)),
                Place::InPlay if holds(you.map(|y| &y.in_play), &card) => Some(Spot::InPlay(card)),
                Place::Supply => t.pile.filter(|p| piles.contains(p)).map(Spot::Pile),
                Place::Landscape if board.landscapes.iter().any(|l| l.card == card) => {
                    Some(Spot::Landscape(card))
                }
                Place::Button => return Spot::Button,
                Place::Ability => return Spot::Ability,
                _ => None,
            };
            match want {
                Some(s) if !taken.contains(&s) => {
                    taken.push(s.clone());
                    s
                }
                _ => Spot::Tray,
            }
        })
        .collect()
}

/// The turn player's cards in play split into those played this turn and those kept in
/// play from earlier turns (Durations): a card counts as this turn's for each time the
/// log shows it played since the turn began.
pub fn split_play(cards: &Zone, log: &[LogEntry], player: u8) -> (Vec<Stack>, Vec<Stack>) {
    let start = log
        .iter()
        .rposition(|e| e.kind == "turn-start")
        .map_or(0, |i| i + 1);
    let played = |card: &str| -> u32 {
        log[start..]
            .iter()
            .filter(|e| {
                e.kind == "played" && e.player == Some(player) && e.card.as_deref() == Some(card)
            })
            .count() as u32
    };
    let mut now = vec![];
    let mut earlier = vec![];
    for s in &cards.known {
        let fresh = played(&s.card).min(s.count);
        if fresh > 0 {
            now.push(Stack {
                card: s.card.clone(),
                count: fresh,
            });
        }
        if s.count > fresh {
            earlier.push(Stack {
                card: s.card.clone(),
                count: s.count - fresh,
            });
        }
    }
    (now, earlier)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(seq: usize, kind: &str, player: u8, card: Option<&str>) -> LogEntry {
        LogEntry {
            seq,
            kind: kind.into(),
            player: Some(player),
            card: card.map(Into::into),
            text: String::new(),
        }
    }

    fn zone(cards: &[(&str, u32)]) -> Zone {
        Zone {
            count: cards.iter().map(|c| c.1).sum(),
            known: cards
                .iter()
                .map(|&(card, count)| Stack {
                    card: card.into(),
                    count,
                })
                .collect(),
            unknown: 0,
        }
    }

    #[test]
    fn cards_not_played_this_turn_were_kept_from_earlier() {
        let log = vec![
            entry(0, "turn-start", 0, None),
            entry(1, "played", 0, Some("Haven")),
            entry(2, "turn-start", 1, None),
            entry(3, "turn-start", 0, None),
            entry(4, "played", 0, Some("Haven")),
            entry(5, "played", 0, Some("Copper")),
            entry(6, "played", 1, Some("Moat")),
        ];
        let (now, earlier) = split_play(&zone(&[("Copper", 1), ("Haven", 2)]), &log, 0);
        assert_eq!(now, zone(&[("Copper", 1), ("Haven", 1)]).known);
        assert_eq!(earlier, zone(&[("Haven", 1)]).known);
    }

    #[test]
    fn a_card_played_twice_is_counted_once_per_copy() {
        let log = vec![
            entry(0, "turn-start", 0, None),
            entry(1, "played", 0, Some("Throne Room")),
            entry(2, "played", 0, Some("Village")),
            entry(3, "played", 0, Some("Village")),
        ];
        let (now, earlier) = split_play(&zone(&[("Throne Room", 1), ("Village", 1)]), &log, 0);
        assert_eq!(now.len(), 2);
        assert!(earlier.is_empty());
    }
}
