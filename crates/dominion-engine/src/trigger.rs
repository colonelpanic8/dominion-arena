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
use crate::game::{Mat, Scratch, Zone};
use crate::{Card, CardId, Cost, Game, PlayerId, Types};
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
    /// unaffected. One window holds the occasions of all the players attacked.
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
    /// `card` left a Supply pile other than by being gained or trashed: Exiled (Invest,
    /// Transport, ...) or set aside (Inheritance). The window [`Game::answers_emptying`]
    /// of a pile that emptied so (Search).
    LeaveSupply {
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// What resolving it does, said to its owner ("reveal it to be unaffected"): every
    /// optional one, and a mandatory one where its card's name doesn't say.
    #[serde(default)]
    pub does: Option<String>,
    /// All it does is make the attacked player unaffected: not offered once they are.
    #[serde(default)]
    pub protects: bool,
    /// A Reaction card's own Reaction ability: when used, the other players' declined
    /// Reactions are offered again.
    #[serde(default)]
    pub reaction: bool,
    /// An ability every player has in games using `card` ("In games using this", Shaman),
    /// not one of a card of the owner's.
    #[serde(default)]
    pub game_wide: bool,
    /// Resolving it starts its owner playing an Action card they see, before anything else
    /// is asked (a Reaction that plays itself, Innovation): a Way question may follow. Not
    /// part of what the trigger is.
    #[serde(default)]
    pub plays: bool,
}

impl PartialEq for Trigger {
    fn eq(&self, other: &Trigger) -> bool {
        let Trigger {
            owner,
            card,
            ability,
            source,
            lasting,
            optional,
            first,
            quiet,
            data,
            label,
            does,
            protects,
            reaction,
            game_wide,
            plays: _,
        } = self;
        (owner, card, ability, source, lasting, optional, first)
            == (
                &other.owner,
                &other.card,
                &other.ability,
                &other.source,
                &other.lasting,
                &other.optional,
                &other.first,
            )
            && (quiet, data, label, does, protects, reaction, game_wide)
                == (
                    &other.quiet,
                    &other.data,
                    &other.label,
                    &other.does,
                    &other.protects,
                    &other.reaction,
                    &other.game_wide,
                )
    }
}

impl Eq for Trigger {}

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
            reaction: false,
            game_wide: false,
            plays: false,
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

    /// What a mandatory one does, where its card's name doesn't say: another player's
    /// lasting effect on its owner ("gain a Curse").
    pub(crate) fn doing(mut self, does: impl Into<String>) -> Trigger {
        self.does = Some(does.into());
        self
    }

    /// A Reaction card's "you may" Reaction ability, saying what it does.
    pub(crate) fn reaction(mut self, does: impl Into<String>) -> Trigger {
        self.reaction = true;
        self.optional(does)
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

    pub(crate) fn game_wide(mut self) -> Trigger {
        self.game_wide = true;
        self
    }

    /// Resolving it starts its owner playing `card`, which they see: a Way question may
    /// follow if it is an Action (see [`Trigger::plays`]).
    pub(crate) fn plays(self, g: &Game, card: CardId) -> Trigger {
        self.plays_one_of(g, &[card])
    }

    /// It plays one of `cards`, which its owner sees, asking which first if there are
    /// several (a question that says itself that a Way question may follow).
    pub(crate) fn plays_one_of(mut self, g: &Game, cards: &[CardId]) -> Trigger {
        self.plays = cards.iter().any(|&c| g.is(c, Types::ACTION));
        self
    }

    /// It plays the card lasting effect `l` set aside (`Scratch::pick`), if it still is.
    pub(crate) fn plays_set_aside(self, g: &Game, l: &crate::game::Lasting) -> Trigger {
        match l.data.pick {
            Some(c) if g.zone(c) == Zone::SetAside(l.owner) => self.plays(g, c),
            _ => self,
        }
    }

    pub(crate) fn labelled(mut self, label: String) -> Trigger {
        self.label = Some(label);
        self
    }
}

/// Resolves every trigger for one occasion, or for several that happen together (cards
/// trashed or discarded at once, the players an Attack is played against), whose triggers
/// the players then order together. Finishes with `Value::Flags`: for each occasion,
/// whether a resolved trigger finished with `Value::Bool(true)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Window {
    occs: Vec<Occasion>,
    /// Resolved triggers, with the index of their occasion.
    done: Vec<(usize, Trigger)>,
    /// Optional triggers their owner declined. A declined Reaction is offered again once
    /// another player has used one.
    #[serde(default)]
    declined: Vec<(usize, Trigger)>,
    /// One trigger per option of the pending decision.
    asked: Vec<(usize, Trigger)>,
    /// Every trigger the pending decision is about, interchangeable ones included.
    #[serde(default)]
    offered: Vec<(usize, Trigger)>,
    /// The occasions for which a resolved trigger finished with `Value::Bool(true)`.
    #[serde(default)]
    granted: Vec<usize>,
}

impl Window {
    pub(crate) fn open(occ: Occasion) -> Frame {
        Window::open_all(vec![occ])
    }

    pub(crate) fn open_all(occs: Vec<Occasion>) -> Frame {
        Frame::Window(Box::new(Window {
            occs,
            done: vec![],
            declined: vec![],
            asked: vec![],
            offered: vec![],
            granted: vec![],
        }))
    }

    pub(crate) fn resume(&mut self, g: &mut Game, input: Input) -> Flow {
        if let Input::Return(value) = &input {
            g.end_resolving();
            if matches!(value, Value::Bool(true))
                && let Some(&(i, _)) = self.done.last()
                && !self.granted.contains(&i)
            {
                self.log_unaffected(g);
                self.granted.push(i);
            }
        }
        if let Input::Response(Response::Options(chosen)) = input {
            match chosen.first() {
                Some(Opt::Trigger(i)) => {
                    let t = self.asked[*i as usize].clone();
                    return self.run(g, t);
                }
                _ => {
                    let declined = self.offered.iter().filter(|(_, t)| t.optional);
                    self.declined.extend(declined.cloned());
                }
            }
        }

        let mut all = vec![];
        for (i, occ) in self.occs.iter().enumerate() {
            let mut found = vec![];
            cards::triggers(g, occ, &mut found);
            all.extend(found.into_iter().map(|t| (i, t)));
        }
        all.retain(|t| {
            !self.done.contains(t)
                && !self.declined.contains(t)
                && !(self.granted.contains(&t.0) && t.1.protects)
        });
        if all.iter().any(|(_, t)| t.first) {
            all.retain(|(_, t)| t.first);
        }
        if all.is_empty() {
            let flags = (0..self.occs.len()).map(|i| self.granted.contains(&i));
            return Flow::Done(Value::Flags(flags.collect()));
        }
        let order = g.players_from(g.turn.player);
        let owner = *order
            .iter()
            .find(|p| all.iter().any(|(_, t)| t.owner == **p))
            .expect("every trigger has an owner");
        let mine: Vec<(usize, Trigger)> =
            all.into_iter().filter(|(_, t)| t.owner == owner).collect();
        // A mandatory one that only makes its player unaffected (Lighthouse) needs no
        // order either: the others that would do the same drop out.
        if let Some(t) = mine
            .iter()
            .find(|(_, t)| (t.quiet || t.protects) && !t.optional)
        {
            return self.run(g, t.clone());
        }
        // Copies of one mandatory ability, also for occasions that happen together (Tomb
        // for each card trashed together, two Rats), resolve in occasion order.
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
        let plain: Vec<String> = asked.iter().map(|(_, t)| label(g, t, false)).collect();
        let mut labels: Vec<String> = asked
            .iter()
            .zip(&plain)
            .map(|((i, t), plain_label)| {
                // Two that differ under one name say what tells them apart.
                let alone = plain.iter().filter(|&l| l == plain_label).count() == 1;
                let mut label = label(g, t, !alone);
                if several {
                    label = format!("{label} [{}]", moment(g, &self.occs[*i], owner));
                }
                label
            })
            .collect();
        let plays = (asked.iter().zip(&options))
            .filter(|((_, t), _)| t.plays)
            .map(|(_, &o)| o)
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
        } else if (asked.iter()).any(|(_, t)| lasting_owner(g, t).is_some_and(|o| o != owner)) {
            "choose which ability resolves next (the others stay pending)"
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
            public_choice: false,
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
            plays,
        })
    }

    /// Whether the owner can't tell `a` and `b` apart and resolving either makes no
    /// difference.
    fn interchangeable(&self, g: &Game, a: &(usize, Trigger), b: &(usize, Trigger)) -> bool {
        alike(g, &self.occs[a.0], &self.occs[b.0])
            && same_effect(g, &a.1, &b.1)
            && core_label(g, &a.1, false) == core_label(g, &b.1, false)
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
        // "If another player uses a Reaction, any previous players in turn order get
        // another chance to react": only to react, and only until everyone has declined.
        if t.reaction {
            self.declined
                .retain(|(_, d)| d.owner == t.owner || !d.reaction);
        }
        self.done.push((i, t.clone()));
        // A label can say what only its owner knows.
        let player = t
            .lasting
            .and_then(|id| g.lasting().iter().find(|l| l.id == id))
            .map_or(t.owner, |l| l.owner);
        let header = t.label.is_none().then_some(match t.game_wide {
            true => crate::Event::ResolvedFor {
                player,
                card: t.card,
            },
            false => crate::Event::Resolved {
                player,
                card: t.card,
            },
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
                let mut l = l.clone();
                cards::plain_effect(&mut l, t.owner);
                let rest = Scratch {
                    cards: vec![],
                    pick: None,
                    ..l.data.clone()
                };
                let cards: Vec<_> = l.data.cards.iter().map(look).collect();
                (l.card, l.owner, rest, cards, l.data.pick.as_ref().map(look))
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
/// set aside that the owner can see, which tell two of one Duration apart. With `detail`,
/// also what else tells it apart from the same ability with another effect: where its
/// card is if not in play, its lasting effect's other cards, and its count (a Garrison's
/// tokens).
fn label(g: &Game, t: &Trigger, detail: bool) -> String {
    match lasting_owner(g, t).filter(|&o| o != t.owner) {
        Some(o) => format!("{o}'s {}", core_label(g, t, detail)),
        None => core_label(g, t, detail),
    }
}

/// Who made the lasting effect `t` comes from, if any.
fn lasting_owner(g: &Game, t: &Trigger) -> Option<PlayerId> {
    let id = t.lasting?;
    g.lasting().iter().find(|l| l.id == id).map(|l| l.owner)
}

/// `label` without whose lasting effect it is: abilities alike but for that are one
/// option (two players' Swamp Hags).
fn core_label(g: &Game, t: &Trigger, detail: bool) -> String {
    let name = t.label.clone().unwrap_or_else(|| {
        let view = g.view(t.owner);
        let lasting = t
            .lasting
            .and_then(|id| g.lasting().iter().find(|l| l.id == id));
        let cards = || {
            lasting
                .into_iter()
                .flat_map(|l| l.data.cards.iter().chain(&l.data.pick))
        };
        let on = t.source.and_then(|c| view.card(c)).filter(|&c| c != t.card);
        let mut held: Vec<String> = on
            .into_iter()
            .chain(
                cards()
                    .filter(|&&c| matches!(g.zone(c), Zone::SetAside(_)))
                    .filter_map(|&c| view.card(c)),
            )
            .map(|c| c.name().to_string())
            .collect();
        if detail {
            let away = |c: CardId| away(g.zone(c), t.owner);
            if let Some(c) = t.source.filter(|&c| view.card(c).is_some()) {
                held.extend(away(c));
            }
            for &c in cards().filter(|&&c| !matches!(g.zone(c), Zone::SetAside(_))) {
                let Some(card) = view.card(c).filter(|_| Some(c) != t.source) else {
                    continue;
                };
                held.push(match (card == t.card, away(c)) {
                    (true, Some(z)) => z,
                    (true, None) => continue,
                    (false, z) => [card.name().to_string()]
                        .into_iter()
                        .chain(z)
                        .collect::<Vec<_>>()
                        .join(" "),
                });
            }
            if let Some(l) = lasting.filter(|l| l.data.n != 0) {
                held.push(format!("{}", l.data.n));
            }
        }
        if held.is_empty() {
            t.card.to_string()
        } else {
            format!("{} ({})", t.card, held.join(", "))
        }
    });
    match &t.does {
        Some(does) if t.optional => format!("{name}: {does} (optional)"),
        Some(does) => format!("{name}: {does}"),
        None => name,
    }
}

/// Where a card in `zone` is, as `me` is told, unless it is in `me`'s play area.
fn away(zone: Zone, me: PlayerId) -> Option<String> {
    let whose = |p: PlayerId| cards::whose(me, p);
    Some(match zone {
        Zone::InPlay(p) if p == me => return None,
        Zone::InPlay(p) => format!("in {} play area", whose(p)),
        Zone::Deck(p) => format!("in {} deck", whose(p)),
        Zone::Hand(p) => format!("in {} hand", whose(p)),
        Zone::Discard(p) => format!("in {} discard pile", whose(p)),
        Zone::SetAside(_) | Zone::Looking(_) => "set aside".into(),
        Zone::Mat(p, mat) => {
            let mat = match mat {
                Mat::Tavern => "Tavern",
                Mat::Island => "Island",
                Mat::NativeVillage => "Native Village",
                Mat::Exile => "Exile",
            };
            format!("on {} {mat} mat", whose(p))
        }
        Zone::Pile(_) => "in the Supply".into(),
        Zone::Trash => "in the trash".into(),
    })
}

/// When `occ` happens, as `me` is told: "P1 plays Militia", "Start of your turn".
fn moment(g: &Game, occ: &Occasion, me: PlayerId) -> String {
    let view = g.view(me);
    let name = |c: CardId| match occ {
        Occasion::Gain { .. }
        | Occasion::Trash { .. }
        | Occasion::Discard { .. }
        | Occasion::LeaveSupply { .. } => g.card(c).name(),
        _ => view.card(c).map_or("a card", |c| c.name()),
    };
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
        Occasion::LeaveSupply { player, card } => {
            let c = name(*card);
            who(
                *player,
                &format!("take {c} from the Supply"),
                &format!("takes {c} from the Supply"),
            )
        }
        Occasion::Discard { player, card } => {
            let c = name(*card);
            who(*player, &format!("discard {c}"), &format!("discards {c}"))
        }
        Occasion::WouldShuffle { player } => who(*player, "would shuffle", "would shuffle"),
        Occasion::Shuffled { player, .. } => who(*player, "shuffled", "shuffled"),
    }
}
