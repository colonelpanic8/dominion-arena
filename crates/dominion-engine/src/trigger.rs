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
use crate::game::{Scratch, Zone};
use crate::{Card, CardId, Cost, Game, PlayerId};
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
    /// Before `player` discards the cards of [`Game::leaving_play`] from play, in Clean-up
    /// or at another time: "when you discard this from play".
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
    /// `player` bought `card`, the top card of its pile, when it cost `cost`. The buy's own
    /// gain follows the window.
    Buy {
        player: PlayerId,
        card: CardId,
        #[serde(default)]
        cost: Cost,
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
    /// Just shuffled `count` cards, put under the deck: [`Game::shuffled`] is those still
    /// there.
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
    /// What resolving an optional one does, said to its owner ("reveal it to be
    /// unaffected").
    #[serde(default)]
    pub does: Option<String>,
    /// All it does is make the attacked player unaffected: not offered once they are.
    #[serde(default)]
    pub protects: bool,
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
            does: None,
            protects: false,
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

    /// "You may": `does` says what resolving it does ("exchange it for a Soldier").
    pub(crate) fn optional(mut self, does: impl Into<String>) -> Trigger {
        self.optional = true;
        self.does = Some(does.into());
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

    pub(crate) fn protects(mut self) -> Trigger {
        self.protects = true;
        self
    }

    pub(crate) fn labelled(mut self, label: String) -> Trigger {
        self.label = Some(label);
        self
    }
}

/// Resolves every trigger for one occasion, or for several that happen together (cards
/// trashed or discarded at once), whose triggers the players then order together.
/// Finishes with `Value::Bool(true)` if any resolved trigger finished with
/// `Value::Bool(true)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Window {
    occs: Vec<Occasion>,
    /// Resolved or declined triggers, with the index of their occasion.
    done: Vec<(usize, Trigger)>,
    /// One trigger per option of the pending decision.
    asked: Vec<(usize, Trigger)>,
    /// Every trigger the pending decision is about, interchangeable ones included.
    #[serde(default)]
    offered: Vec<(usize, Trigger)>,
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
            offered: vec![],
            result: false,
        })
    }

    pub(crate) fn resume(&mut self, g: &mut Game, input: Input) -> Flow {
        if let Input::Return(value) = &input {
            g.end_resolving();
            if matches!(value, Value::Bool(true)) && !self.result {
                self.log_unaffected(g);
            }
        }
        match input {
            Input::Response(Response::Options(chosen)) => match chosen.first() {
                Some(Opt::Trigger(i)) => {
                    let t = self.asked[*i as usize].clone();
                    return self.run(g, t);
                }
                _ => {
                    let declined: Vec<(usize, Trigger)> = self
                        .offered
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
        all.retain(|t| !self.done.contains(t) && !(self.result && t.1.protects));
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
        // Interchangeable triggers (two Moats, two Tunnels discarded together) are one
        // option.
        let mut asked: Vec<(usize, Trigger)> = vec![];
        for t in &mine {
            if !asked.iter().any(|u| self.interchangeable(g, t, u)) {
                asked.push(t.clone());
            }
        }
        let several = asked.iter().any(|(i, _)| *i != asked[0].0);
        let mut options: Vec<Opt> = (0..asked.len()).map(|i| Opt::Trigger(i as u16)).collect();
        let mut labels: Vec<String> = asked
            .iter()
            .map(|(i, t)| {
                let mut label = label(g, t);
                if several {
                    label = format!("{label} [{}]", moment(g, &self.occs[*i], owner));
                }
                label
            })
            .collect();
        let all_optional = mine.iter().all(|(_, t)| t.optional);
        if all_optional {
            options.push(Opt::No);
            labels.push(
                if asked.len() == 1 {
                    "Decline"
                } else {
                    "Decline all"
                }
                .to_string(),
            );
        }
        let ask = if all_optional && asked.len() == 1 {
            "you may use this ability"
        } else if all_optional {
            "choose an ability to use next, or decline all"
        } else {
            "choose which of your abilities resolves next (the others stay pending)"
        };
        let question = if several {
            format!("Several things happened: {ask}")
        } else {
            format!("{}: {ask}", moment(g, &self.occs[asked[0].0], owner))
        };
        self.asked = asked;
        self.offered = mine;
        Flow::Ask(Decision {
            player: owner,
            kind: DecisionKind::Choose {
                options,
                labels,
                min: 1,
                max: 1,
            },
            purpose: Purpose::Trigger,
            source: None,
            about: vec![],
            question: Some(question),
        })
    }

    /// Whether the owner can't tell `a` and `b` apart and resolving either makes no
    /// difference.
    fn interchangeable(&self, g: &Game, a: &(usize, Trigger), b: &(usize, Trigger)) -> bool {
        alike(g, &self.occs[a.0], &self.occs[b.0])
            && same_effect(g, &a.1, &b.1)
            && label(g, &a.1) == label(g, &b.1)
    }

    /// The pending triggers the decision being asked is about.
    pub(crate) fn asked(&self) -> impl Iterator<Item = &Trigger> {
        self.asked.iter().map(|(_, t)| t)
    }

    /// The trigger that just resolved made the target of an Attack unaffected.
    fn log_unaffected(&self, g: &mut Game) {
        let Some((i, t)) = self.done.last() else {
            return;
        };
        if let Occasion::Attack { card, target, .. } = self.occs[*i] {
            let attack = g.card(card);
            g.emit(crate::Event::Unaffected {
                player: target,
                attack,
                by: t.card,
            });
        }
    }

    fn run(&mut self, g: &mut Game, (i, t): (usize, Trigger)) -> Flow {
        self.done.push((i, t.clone()));
        // A label can say what only its owner knows.
        let header = t.label.is_none().then_some(crate::Event::Resolved {
            player: t.owner,
            card: t.card,
        });
        g.begin_resolving(header);
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
/// on cards (if any) and from lasting effects (if any) holding the same data, up to cards
/// no one can tell apart (the same card, printed and now, in the same zone). A Trait on
/// two differently named cards (Reckless returning both to a split pile) is two effects.
fn same_effect(g: &Game, a: &Trigger, b: &Trigger) -> bool {
    let look = |&c: &CardId| (g.printed(c), g.card(c), g.zone(c));
    let lasting = |t: &Trigger| {
        t.lasting
            .and_then(|id| g.lasting().iter().find(|l| l.id == id))
            .map(|l| {
                let rest = Scratch {
                    cards: vec![],
                    pick: None,
                    ..l.data.clone()
                };
                let cards: Vec<_> = l.data.cards.iter().map(look).collect();
                (l.card, rest, cards, l.data.pick.as_ref().map(look))
            })
    };
    (a.card, a.ability, a.data, a.optional) == (b.card, b.ability, b.data, b.optional)
        && a.source.as_ref().map(look) == b.source.as_ref().map(look)
        && lasting(a) == lasting(b)
}

/// Whether `a` and `b` are one occasion, or happened together to cards no one can tell
/// apart (two Tunnels discarded together).
fn alike(g: &Game, a: &Occasion, b: &Occasion) -> bool {
    let look = |c: CardId| (g.printed(c), g.card(c), g.zone(c));
    match (a, b) {
        (Occasion::Discard { player: p, card: c }, Occasion::Discard { player: q, card: d })
        | (Occasion::Trash { player: p, card: c }, Occasion::Trash { player: q, card: d }) => {
            p == q && look(*c) == look(*d)
        }
        _ => a == b,
    }
}

/// How the owner is offered `t`: its card (or label), what an optional one does, the card
/// it is on when that is another card (a Trait's), and the cards its lasting effect has
/// set aside that the owner can see, which tell two of one Duration apart.
fn label(g: &Game, t: &Trigger) -> String {
    let name = t.label.clone().unwrap_or_else(|| {
        let view = g.view(t.owner);
        let on = t.source.and_then(|c| view.card(c)).filter(|&c| c != t.card);
        let held: Vec<&str> = on
            .into_iter()
            .chain(
                t.lasting
                    .and_then(|id| g.lasting().iter().find(|l| l.id == id))
                    .into_iter()
                    .flat_map(|l| l.data.cards.iter().chain(&l.data.pick))
                    .filter(|&&c| matches!(g.zone(c), Zone::SetAside(_)))
                    .filter_map(|&c| view.card(c)),
            )
            .map(|c| c.name())
            .collect();
        if held.is_empty() {
            t.card.to_string()
        } else {
            format!("{} ({})", t.card, held.join(", "))
        }
    });
    match &t.does {
        Some(does) => format!("{name}: {does} (optional)"),
        None => name,
    }
}

/// When `occ` happens, as `me` is told: "P1 plays Militia", "Start of your turn".
fn moment(g: &Game, occ: &Occasion, me: PlayerId) -> String {
    let view = g.view(me);
    let name = |c: CardId| view.card(c).map_or("a card", |c| c.name());
    let who = |p: PlayerId, you: &str, they: &str| {
        if p == me {
            format!("You {you}")
        } else {
            format!("{p} {they}")
        }
    };
    let whose = |p: PlayerId| cards::whose(me, p);
    match occ {
        Occasion::GameStart => "Before the first turn".into(),
        Occasion::StartTurn { player } => format!("Start of {} turn", whose(*player)),
        Occasion::StartBuyPhase { player } => format!("Start of {} Buy phase", whose(*player)),
        Occasion::EndBuyPhase { player } => format!("End of {} Buy phase", whose(*player)),
        Occasion::StartNight { player } => format!("Start of {} Night phase", whose(*player)),
        Occasion::StartCleanup { player } => format!("Start of {} Clean-up", whose(*player)),
        Occasion::DiscardFromPlay { player } => who(
            *player,
            "discard cards from play",
            "discards cards from play",
        ),
        Occasion::EndTurn { player } => format!("End of {} turn", whose(*player)),
        Occasion::Play { player, card } => {
            let c = name(*card);
            who(*player, &format!("play {c}"), &format!("plays {c}"))
        }
        Occasion::AfterPlay { player, card } => {
            let c = name(*card);
            who(*player, &format!("played {c}"), &format!("played {c}"))
        }
        Occasion::Attack {
            attacker,
            card,
            target,
        } => {
            let c = name(*card);
            let target = if *target == me {
                "you".to_string()
            } else {
                target.to_string()
            };
            who(
                *attacker,
                &format!("attack {target} with {c}"),
                &format!("attacks {target} with {c}"),
            )
        }
        Occasion::Buy { player, card, .. } => {
            let c = name(*card);
            who(*player, &format!("buy {c}"), &format!("buys {c}"))
        }
        Occasion::BuyLandscape { player, card } => {
            who(*player, &format!("buy {card}"), &format!("buys {card}"))
        }
        Occasion::WouldGain { player, card } => {
            let c = name(*card);
            who(
                *player,
                &format!("would gain {c}"),
                &format!("would gain {c}"),
            )
        }
        Occasion::Gain { player, card, .. } => {
            let c = name(*card);
            who(*player, &format!("gain {c}"), &format!("gains {c}"))
        }
        Occasion::Trash { player, card } => {
            let c = name(*card);
            who(*player, &format!("trash {c}"), &format!("trashes {c}"))
        }
        Occasion::Discard { player, card } => {
            let c = name(*card);
            who(*player, &format!("discard {c}"), &format!("discards {c}"))
        }
        Occasion::WouldShuffle { player } => who(*player, "would shuffle", "would shuffle"),
        Occasion::Shuffled { player, .. } => who(*player, "shuffled", "shuffled"),
    }
}
