//! Triggered abilities.
//!
//! When something happens that cards can respond to, the engine opens a window for that
//! [`Occasion`]. The window asks every set's rules for the [`Trigger`]s that apply, lets the
//! players resolve them (the player whose turn it is first, then the others in turn order;
//! each orders their own), and asks again after each one resolves, so a trigger whose
//! condition no longer holds (its card moved away, ...) drops out. A trigger runs as a
//! [`crate::cards::CardFrame`] with `hook == Hook::Ability(trigger.ability)`.

use crate::cards::{self, CardFrame, Hook};
use crate::decision::{Decision, DecisionKind, Opt, Purpose, Response};
use crate::frame::{Flow, Frame, Input, Value};
use crate::{Card, CardId, Game, PlayerId};
use serde::{Deserialize, Serialize};

/// Something that happened, or is about to, that abilities can respond to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Occasion {
    /// Start of a turn, before the Action phase (Durations, ...).
    StartTurn {
        player: PlayerId,
    },
    StartBuyPhase {
        player: PlayerId,
    },
    EndBuyPhase {
        player: PlayerId,
    },
    StartNight {
        player: PlayerId,
    },
    StartCleanup {
        player: PlayerId,
    },
    /// In Clean-up, before cards are discarded from play: "when you discard this from play".
    DiscardFromPlay {
        player: PlayerId,
    },
    /// After Clean-up's draw.
    EndTurn {
        player: PlayerId,
    },
    /// A card was put into play (or played where it is), before its effect.
    Play {
        player: PlayerId,
        card: CardId,
    },
    /// A card's effect finished resolving.
    AfterPlay {
        player: PlayerId,
        card: CardId,
    },
    /// `card`, an Attack played by `attacker`, is about to affect `target`. A trigger that
    /// finishes with `Value::Bool(true)` makes `target` unaffected.
    Attack {
        attacker: PlayerId,
        card: CardId,
        target: PlayerId,
    },
    Buy {
        player: PlayerId,
        card: CardId,
    },
    BuyLandscape {
        player: PlayerId,
        card: Card,
    },
    /// Before a gained card moves. Replacement effects edit `Game::gains`.
    WouldGain {
        player: PlayerId,
        card: CardId,
    },
    Gain {
        player: PlayerId,
        card: CardId,
        bought: bool,
    },
    Trash {
        player: PlayerId,
        card: CardId,
    },
    /// A card was discarded other than in Clean-up.
    Discard {
        player: PlayerId,
        card: CardId,
    },
    /// About to shuffle a discard pile into a deck.
    WouldShuffle {
        player: PlayerId,
    },
    /// Right after a shuffle (whether or not anything was left to shuffle): cards held out
    /// of it at `WouldShuffle` go where they belong.
    AfterShuffle {
        player: PlayerId,
    },
}

/// One triggered ability that could resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    pub owner: PlayerId,
    /// The card (or landscape, State, ...) whose rules resolve it.
    pub card: Card,
    /// Which of that card's abilities, numbered by its set module.
    pub ability: u8,
    /// The physical card the ability is on, if any.
    pub source: Option<CardId>,
    /// The lasting effect it comes from, if any.
    pub lasting: Option<u32>,
    /// "You may": the owner can decline it.
    pub optional: bool,
    pub data: i32,
}

impl Trigger {
    pub(crate) fn new(owner: PlayerId, card: Card, ability: u8) -> Trigger {
        Trigger {
            owner,
            card,
            ability,
            source: None,
            lasting: None,
            optional: false,
            data: 0,
        }
    }

    pub(crate) fn on(mut self, source: CardId) -> Trigger {
        self.source = Some(source);
        self
    }

    pub(crate) fn via_lasting(mut self, lasting: u32) -> Trigger {
        self.lasting = Some(lasting);
        self
    }

    pub(crate) fn optional(mut self) -> Trigger {
        self.optional = true;
        self
    }

    pub(crate) fn data(mut self, data: i32) -> Trigger {
        self.data = data;
        self
    }
}

/// Resolves every trigger for one occasion. Finishes with `Value::Bool(true)` if any
/// resolved trigger finished with `Value::Bool(true)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Window {
    occ: Occasion,
    done: Vec<Trigger>,
    asked: Vec<Trigger>,
    result: bool,
}

impl Window {
    pub(crate) fn open(occ: Occasion) -> Frame {
        Frame::Window(Window {
            occ,
            done: vec![],
            asked: vec![],
            result: false,
        })
    }

    pub(crate) fn resume(&mut self, g: &mut Game, input: Input) -> Flow {
        match input {
            Input::Response(Response::Options(chosen)) => match chosen.first() {
                Some(Opt::Trigger(i)) => {
                    let t = self.asked[*i as usize].clone();
                    return self.run(t);
                }
                _ => {
                    let declined: Vec<Trigger> =
                        self.asked.iter().filter(|t| t.optional).cloned().collect();
                    self.done.extend(declined);
                }
            },
            Input::Return(Value::Bool(true)) => self.result = true,
            _ => {}
        }

        let mut all = vec![];
        cards::triggers(g, &self.occ, &mut all);
        all.retain(|t| !self.done.contains(t));
        if all.is_empty() {
            return Flow::Done(Value::Bool(self.result));
        }
        let order = g.players_from(g.turn.player);
        let owner = *order
            .iter()
            .find(|p| all.iter().any(|t| t.owner == **p))
            .expect("every trigger has an owner");
        let mine: Vec<Trigger> = all.into_iter().filter(|t| t.owner == owner).collect();
        if mine.len() == 1 && !mine[0].optional {
            return self.run(mine[0].clone());
        }
        let mut options: Vec<Opt> = (0..mine.len()).map(|i| Opt::Trigger(i as u16)).collect();
        let mut labels: Vec<String> = mine
            .iter()
            .map(|t| {
                if t.optional {
                    format!("{} (optional)", t.card)
                } else {
                    t.card.to_string()
                }
            })
            .collect();
        if mine.iter().all(|t| t.optional) {
            options.push(Opt::No);
            labels.push("Done".to_string());
        }
        let source = mine.first().map(|t| t.card);
        self.asked = mine;
        Flow::Ask(Decision {
            player: owner,
            kind: DecisionKind::Choose {
                options,
                labels,
                min: 1,
                max: 1,
            },
            purpose: Purpose::Trigger,
            source,
        })
    }

    fn run(&mut self, t: Trigger) -> Flow {
        self.done.push(t.clone());
        let mut f = CardFrame::new(t.card, Hook::Ability(t.ability), t.owner, t.source);
        f.occ = Some(self.occ.clone());
        f.lasting = t.lasting;
        f.data = t.data;
        Flow::Call(Frame::Card(Box::new(f)))
    }
}
