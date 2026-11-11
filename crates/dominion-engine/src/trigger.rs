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
    /// Before the first turn, for setup choices players make (Inherited).
    GameStart,
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
    /// `attacker` played `card`, an Attack, and `target` may answer it, even if the play was
    /// replaced or blocked. A trigger that finishes with `Value::Bool(true)` makes `target`
    /// unaffected.
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
    /// Just shuffled `count` cards, now the bottom `count` cards of the deck.
    Shuffled {
        player: PlayerId,
        count: usize,
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
    /// Resolves before every trigger without this ("At the start of your next turn,
    /// first, ...").
    pub first: bool,
    /// Order never matters for it (bookkeeping, or a bonus nothing else can interact
    /// with): it resolves before the owner's other triggers without asking.
    pub quiet: bool,
    pub data: i32,
    /// What the choice is called when `card`'s name doesn't say it: a rule applied to
    /// `card` rather than its own ability, or which card a shared ability is on.
    pub label: Option<String>,
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
            first: false,
            quiet: false,
            data: 0,
            label: None,
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

    pub(crate) fn first(mut self) -> Trigger {
        self.first = true;
        self
    }

    pub(crate) fn quiet(mut self) -> Trigger {
        self.quiet = true;
        self
    }

    pub(crate) fn data(mut self, data: i32) -> Trigger {
        self.data = data;
        self
    }

    pub(crate) fn labelled(mut self, label: String) -> Trigger {
        self.label = Some(label);
        self
    }
}

/// Resolves every trigger for one occasion, or for several that happen together (cards
/// trashed at once), whose triggers the players then order together. Finishes with
/// `Value::Bool(true)` if any resolved trigger finished with `Value::Bool(true)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Window {
    occs: Vec<Occasion>,
    /// Resolved or declined triggers, with the index of their occasion.
    done: Vec<(usize, Trigger)>,
    asked: Vec<(usize, Trigger)>,
    result: bool,
}

impl Window {
    pub(crate) fn open(occ: Occasion) -> Frame {
        Window::open_all(vec![occ])
    }

    pub(crate) fn open_all(occs: Vec<Occasion>) -> Frame {
        Frame::Window(Window {
            occs,
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
                    return self.run(g, t);
                }
                _ => {
                    let declined: Vec<(usize, Trigger)> = self
                        .asked
                        .iter()
                        .filter(|(_, t)| t.optional)
                        .cloned()
                        .collect();
                    self.done.extend(declined);
                }
            },
            Input::Return(Value::Bool(true)) => self.result = true,
            _ => {}
        }

        let mut all = vec![];
        for (i, occ) in self.occs.iter().enumerate() {
            let mut found = vec![];
            cards::triggers(g, occ, &mut found);
            all.extend(found.into_iter().map(|t| (i, t)));
        }
        all.retain(|t| !self.done.contains(t));
        if all.iter().any(|(_, t)| t.first) {
            all.retain(|(_, t)| t.first);
        }
        if all.is_empty() {
            return Flow::Done(Value::Bool(self.result));
        }
        let order = g.players_from(g.turn.player);
        let owner = *order
            .iter()
            .find(|p| all.iter().any(|(_, t)| t.owner == **p))
            .expect("every trigger has an owner");
        let mine: Vec<(usize, Trigger)> =
            all.into_iter().filter(|(_, t)| t.owner == owner).collect();
        if let Some(t) = mine.iter().find(|(_, t)| t.quiet && !t.optional) {
            return self.run(g, t.clone());
        }
        // Copies of one mandatory ability for occasions that happen together (Tomb for
        // each card trashed together, two Rats) resolve in occasion order.
        let same = |t: &Trigger| {
            let u = &mine[0].1;
            (t.card, t.ability, t.optional, t.lasting, t.data)
                == (u.card, u.ability, false, u.lasting, u.data)
        };
        if self.occs.len() > 1 && mine.iter().all(|(_, t)| same(t)) {
            return self.run(g, mine[0].clone());
        }
        if !mine[0].1.optional && mine.iter().all(|(_, t)| same_effect(g, t, &mine[0].1)) {
            return self.run(g, mine[0].clone());
        }
        let mut options: Vec<Opt> = (0..mine.len()).map(|i| Opt::Trigger(i as u16)).collect();
        let mut labels: Vec<String> = mine
            .iter()
            .map(|(_, t)| {
                let name = t.label.clone().unwrap_or_else(|| t.card.to_string());
                if t.optional {
                    format!("{name} (optional)")
                } else {
                    name
                }
            })
            .collect();
        if mine.iter().all(|(_, t)| t.optional) {
            options.push(Opt::No);
            labels.push("Done".to_string());
        }
        let source = mine.first().map(|(_, t)| t.card);
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

    /// The pending triggers the decision being asked is about.
    pub(crate) fn asked(&self) -> impl Iterator<Item = &Trigger> {
        self.asked.iter().map(|(_, t)| t)
    }

    fn run(&mut self, g: &Game, (i, t): (usize, Trigger)) -> Flow {
        self.done.push((i, t.clone()));
        let mut f = CardFrame::new(t.card, Hook::Ability(t.ability), t.owner, t.source);
        f.occ = Some(self.occs[i].clone());
        f.lasting = t.lasting;
        // A Duration's later ability is still played by what kept it in play.
        if let Some(l) = t
            .lasting
            .and_then(|id| g.lasting().iter().find(|l| l.id == id))
            && t.source.is_some()
            && l.keeps.first() == t.source.as_ref()
        {
            f.chain = l.keeps[1..].iter().rev().copied().collect();
        }
        f.data = t.data;
        Flow::Call(Frame::Card(Box::new(f)))
    }
}

/// Whether resolving `a` or `b` makes no difference: the same ability with the same data,
/// from lasting effects (if any) holding the same data. Which physical card it is on does
/// not matter.
fn same_effect(g: &Game, a: &Trigger, b: &Trigger) -> bool {
    let lasting = |t: &Trigger| {
        t.lasting
            .and_then(|id| g.lasting().iter().find(|l| l.id == id))
            .map(|l| (l.card, &l.data))
    };
    (a.card, a.ability, a.data, a.optional) == (b.card, b.ability, b.data, b.optional)
        && lasting(a) == lasting(b)
}
