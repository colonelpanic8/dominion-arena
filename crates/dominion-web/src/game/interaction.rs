//! Where on the table each option of a decision is answered: the hand, the play area,
//! a pile, a landscape, the choice tray or a button. Every option lands in exactly one
//! [`Target`]; whatever the board can't place falls back to the tray or a button.

use super::board::PileAt;
use crate::cards::CardDb;
use dominion_protocol::{AgentDecision, AgentOption, AgentView, Answer};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Interaction {
    pub min: u32,
    pub max: u32,
    pub ordered: bool,
    pub name_any: bool,
    /// Every option index is in exactly one target, in the order of their first option.
    pub targets: Vec<Target>,
    /// The `play` options of Treasures in a Buy phase `turn` decision, which "Play
    /// Treasures" answers with one at a time.
    pub treasures: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub place: Place,
    /// Interchangeable options: the copies of one card in one zone, or a single option.
    pub options: Vec<u32>,
    pub label: String,
    pub action: String,
    pub card: Option<String>,
    pub zone: Option<String>,
    /// For [`Place::Supply`], the pile.
    pub pile: Option<PileAt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    /// A stack in the hand shown at the bottom.
    Hand,
    /// A stack in the deciding seat's play area.
    InPlay,
    /// A Supply or non-Supply pile on the board.
    Supply,
    /// A landscape on the board.
    Landscape,
    /// A card (or card back) in the choice tray above the hand.
    Tray,
    /// A labelled button in the prompt bar.
    Button,
}

impl Interaction {
    /// The target holding option `index`.
    pub fn target_of(&self, index: u32) -> Option<&Target> {
        self.targets.iter().find(|t| t.options.contains(&index))
    }

    pub fn at(&self, place: Place) -> impl Iterator<Item = &Target> {
        self.targets.iter().filter(move |t| t.place == place)
    }
}

/// Someone else's turn this seat is playing (Possession): the cards it decides about are
/// that player's, not the ones in its own hand and play area.
pub fn controlling_another(view: &AgentView) -> bool {
    view.seat.is_some() && view.turn.controller == view.seat && view.turn.player != view.seat
}

fn is_landscape(view: &AgentView, card: &str) -> bool {
    view.landscapes.iter().any(|l| l.card == card)
}

/// The visible pile whose top card is `card`.
fn pile_of(view: &AgentView, card: &str) -> Option<PileAt> {
    PileAt::all(view)
        .find(|(_, p)| !p.hidden && p.top.as_deref().unwrap_or(&p.card) == card)
        .map(|(at, _)| at)
}

fn is_treasure(cards: &CardDb, card: &str) -> bool {
    cards
        .get(card)
        .is_some_and(|c| c.types.iter().any(|t| t == "Treasure"))
}

/// What options share to be interchangeable: place, action, card, zone and, in the
/// tray, label.
type JoinKey = (Place, String, Option<String>, Option<String>, String);

pub fn interaction(view: &AgentView, d: &AgentDecision, cards: &CardDb) -> Interaction {
    let own_cards = !controlling_another(view);
    let you = view.you.as_ref();
    let held = |zone: &str, card: &str| -> u32 {
        let z = match zone {
            "hand" => you.map(|y| &y.hand),
            "inPlay" => you.map(|y| &y.in_play),
            _ => None,
        };
        z.and_then(|z| z.known.get(card)).copied().unwrap_or(0)
    };
    let zoned = |o: &AgentOption, zone: &str| -> Option<String> {
        let ok = match zone {
            "hand" => matches!(o.action.as_str(), "play" | "select"),
            _ => o.action == "select",
        };
        (own_cards && ok && o.zone.as_deref() == Some(zone))
            .then(|| o.card.clone())
            .flatten()
    };
    let mut wanted: BTreeMap<(&str, String, &str), u32> = BTreeMap::new();
    for o in &d.options {
        for zone in ["hand", "inPlay"] {
            if let Some(card) = zoned(o, zone) {
                *wanted.entry((o.action.as_str(), card, zone)).or_default() += 1;
            }
        }
    }
    let naming = |card: &str| {
        d.options
            .iter()
            .filter(|o| o.card.as_deref() == Some(card))
            .count()
    };

    let mut targets: Vec<Target> = vec![];
    // Targets that more options may join, by what they share.
    let mut joinable: BTreeMap<JoinKey, usize> = BTreeMap::new();
    for o in &d.options {
        let mut place = Place::Button;
        let mut pile = None;
        let mut join = false;
        let placed_in_zone = ["hand", "inPlay"].into_iter().find_map(|zone| {
            let card = zoned(o, zone)?;
            let n = wanted[&(o.action.as_str(), card.clone(), zone)];
            (held(zone, &card) >= n).then_some(zone)
        });
        if let Some(zone) = placed_in_zone {
            place = if zone == "hand" {
                Place::Hand
            } else {
                Place::InPlay
            };
            join = true;
        } else if let (Some(card), "buy" | "pile") = (&o.card, o.action.as_str())
            && let Some(p) = pile_of(view, card)
        {
            place = Place::Supply;
            pile = Some(p);
        } else if let Some(card) = &o.card
            && is_landscape(view, card)
            && (o.action == "buy-landscape"
                || (matches!(o.action.as_str(), "select" | "trigger") && naming(card) == 1))
        {
            place = Place::Landscape;
        } else if o.card.is_some() {
            place = Place::Tray;
            join = true;
        } else if o.zone.is_some() {
            place = Place::Tray;
        }
        let key = (
            place,
            o.action.clone(),
            o.card.clone(),
            o.zone.clone(),
            if place == Place::Tray {
                o.label.clone()
            } else {
                String::new()
            },
        );
        if join && let Some(&t) = joinable.get(&key) {
            targets[t].options.push(o.index);
            continue;
        }
        if join {
            joinable.insert(key, targets.len());
        }
        targets.push(Target {
            place,
            options: vec![o.index],
            label: o.label.clone(),
            action: o.action.clone(),
            card: o.card.clone(),
            zone: o.zone.clone(),
            pile,
        });
    }

    let treasures = if d.kind == "turn" && d.phase.as_deref() == Some("buy") {
        d.options
            .iter()
            .filter(|o| {
                o.action == "play" && o.card.as_deref().is_some_and(|c| is_treasure(cards, c))
            })
            .map(|o| o.index)
            .collect()
    } else {
        vec![]
    };
    Interaction {
        min: d.min.unwrap_or(1),
        max: d.max.unwrap_or(1),
        ordered: d.ordered,
        name_any: d.name_any,
        targets,
        treasures,
    }
}

/// "Play Treasures": a run of answers, each playing one Treasure, for as long as the
/// seat that started it is asked for a move in the same Buy phase with Treasures left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreasureRun {
    seat: u8,
    phase: String,
}

impl TreasureRun {
    /// A run from this view, if its decision offers Treasures to play.
    pub fn start(view: &AgentView, cards: &CardDb) -> Option<TreasureRun> {
        let run = TreasureRun {
            seat: view.seat?,
            phase: view.decision.as_ref()?.phase.clone()?,
        };
        run.next(view, cards).is_some().then_some(run)
    }

    /// The answer to send next, or none when the run is over: the view is another
    /// seat's, its decision isn't a move in the run's phase (a Treasure asked something,
    /// or the phase ended), or no Treasure is left to play.
    pub fn next(&self, view: &AgentView, cards: &CardDb) -> Option<Answer> {
        let d = view.decision.as_ref()?;
        if view.seat != Some(self.seat)
            || d.kind != "turn"
            || d.phase.as_deref() != Some(self.phase.as_str())
        {
            return None;
        }
        let index = *interaction(view, d, cards).treasures.first()?;
        Some(Answer::indices(d.id.clone(), vec![index]))
    }
}
