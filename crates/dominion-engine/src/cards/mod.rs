//! Card behavior, one module per set.
//!
//! Each set module defines `pub(crate) struct Rules` implementing [`SetRules`] (every
//! method has a no-op default) and `pub(crate) struct State`, its own serializable game
//! state, reachable as `g.ext.<module>`.
//!
//! A card ability runs as a [`CardFrame`]: a function `(game, frame, input) -> Flow` that
//! matches on `frame.pc`. Each arm does some work, sets `pc` to the next step, and either
//! asks a decision, calls a child frame, or finishes. A played card's vanilla bonuses
//! ([`crate::CardDef::plus`], a Treasure's `coins`) are applied before its `Hook::Main`
//! runs, and after its [`prelude`].

use crate::decision::{Decision, DecisionKind, Opt, Purpose};
use crate::event::Event;
use crate::frame::{Flow, Frame, Input};
use crate::game::{Lasting, Scratch, Until};
use crate::setup::Setup;
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, PileId, PlayerId, Plus, Set, Types};
use serde::{Deserialize, Serialize};

/// What a card's rules can do. Every method defaults to doing nothing.
#[allow(unused_variables)]
pub(crate) trait SetRules {
    /// Runs one of a card's abilities: `f.hook` says which. `Hook::Main` is what the
    /// card does when played; for a landscape it is its effect when bought (Events,
    /// Projects), used (Ways) or received (Boons, Hexes).
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        None
    }

    /// Adds this set's triggered abilities that respond to `occ`.
    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {}

    /// Applies this set's cost modifiers to `card` (the physical `id`, if any). Not asked
    /// about landscapes: an Event or a Project costs what it says.
    fn modify_cost(g: &Game, card: Card, id: Option<CardId>, cost: &mut Cost) {}

    /// A cost `card` has instead of its printed cost and every modifier (Wayfarer).
    fn fixed_cost(g: &Game, card: Card, id: Option<CardId>) -> Option<Cost> {
        None
    }

    /// Whether `p` may pay for buying `card` some other way than its cost (Animal Fair).
    /// The card's `Hook::Ability(frame::PAY_OTHERWISE)` then offers it.
    fn pays_otherwise(g: &Game, p: PlayerId, card: Card) -> bool {
        false
    }

    /// Whether the play of `id` may use `way` (Way of the Mouse's own card cannot).
    fn can_use_way(g: &Game, id: CardId, way: Card) -> bool {
        true
    }

    /// Called after each step of ability `f`, with the turn as it was before the step;
    /// may adjust what the step did and the flow it returned (Way of the Chameleon).
    /// [`CardFrame::hold`] defers the step's flow until after a frame it calls instead.
    fn after_step(g: &mut Game, f: &mut CardFrame, before: &Step, flow: &mut Flow) {}

    /// Applies this set's type-changing effects to `id`.
    fn modify_types(g: &Game, id: CardId, types: &mut Types) {}

    /// Changes the vanilla bonuses `p` gets from following `id`'s own instructions.
    fn modify_bonus(g: &Game, p: PlayerId, id: CardId, bonus: &mut Bonus) {}

    /// How many more times a play of `id` by `p` follows its instructions (Reckless).
    fn extra_runs(g: &Game, p: PlayerId, id: CardId) -> u32 {
        0
    }

    /// The card whose abilities `id` uses when played, if not its own (Inheritance).
    fn behavior(g: &Game, id: CardId) -> Option<Card> {
        None
    }

    /// The VP `card` is worth to `p`, if it isn't its printed VP.
    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        None
    }

    /// VP `p` scores from things other than cards (Landmarks, ...).
    fn bonus_vp(g: &Game, p: PlayerId) -> i32 {
        0
    }

    /// Adds the public state this set keeps outside the generic structures (Young Witch's
    /// Bane pile, Druid's Boons, ...), for agents.
    fn notes(g: &Game, out: &mut Vec<crate::Note>) {}

    /// Changes a decision a card ability asks (Elder: one more pick from a "choose").
    fn modify_decision(g: &Game, f: &CardFrame, d: &mut Decision) {}

    /// Whether `p` may play `id` from their hand now.
    fn can_play(g: &Game, p: PlayerId, id: CardId) -> bool {
        true
    }

    /// Whether `p` may buy `card` (a card, Event or Project) when they can afford it.
    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        true
    }

    /// Builds the Kingdom pile named `card` if it isn't 10 (or 8/12) plain copies.
    /// Returns whether it did.
    fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
        false
    }

    /// Adds what the cards and landscapes in the game need: piles, decks, set-aside cards,
    /// an Ally, ... Look at the whole game (`setup::in_game`), not at who added what, and
    /// add only what is missing: this runs until nothing changes, at the start and again
    /// whenever cards join mid-game (Divine Wind).
    fn ensure(g: &mut Game, s: &mut Setup) {}

    /// One-shot setup for the cards and landscapes that joined (`s.joined`), once `ensure`
    /// has settled: tokens on piles, starting resources, choices among the piles, ...
    fn setup(g: &mut Game, s: &mut Setup) {}

    /// Whether `id`, being shuffled into `p`'s deck, goes on the bottom of the shuffled
    /// cards (Shadow cards). The player orders the cards that do.
    fn bottom_when_shuffled(g: &Game, p: PlayerId, id: CardId) -> bool {
        false
    }

    /// Adds the cards `p` may play as if they were in their hand (Shadow cards in the deck).
    fn playable_as_if_in_hand(g: &Game, p: PlayerId, out: &mut Vec<CardId>) {}

    /// Whether unused Action plays can be used as Buys.
    fn actions_are_buys(g: &Game) -> bool {
        false
    }

    /// Changes a player's starting deck (Heirlooms, ...).
    fn modify_starting_deck(g: &Game, p: PlayerId, deck: &mut Vec<Card>) {}

    /// Reacts to `event` as it is logged, for effects that only change counters and
    /// never move cards or ask anything (Patron's +1 Coffers when revealed).
    fn on_event(g: &mut Game, event: &Event) {}

    /// Whether `p` takes a turn in an extra round after the game ends (Fleet).
    fn final_round(g: &Game, p: PlayerId) -> bool {
        false
    }
}

/// The turn as it was before a step of an ability ran (see [`SetRules::after_step`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Step {
    pub coins: u32,
    pub actions: u32,
    /// The ability's player's -$1 token.
    pub minus_coin: bool,
}

/// The bonuses a play gives before the card's script runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bonus {
    pub plus: Plus,
    pub coins: u32,
    pub potions: u32,
}

macro_rules! sets {
    ($($set:ident => $module:ident,)*) => {
        $(pub(crate) mod $module;)*

        /// Each set's own game state.
        #[derive(Clone, Debug, Default, Serialize, Deserialize)]
        pub(crate) struct Ext {
            $(pub $module: $module::State,)*
        }

        pub(crate) fn resume(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
            if let Some(held) = f.held.take() {
                return *held;
            }
            let before = Step {
                coins: g.turn.coins,
                actions: g.turn.actions,
                minus_coin: g.players[f.player.index()].minus_coin,
            };
            // The Exile rule applies to cards of every set.
            let set = match f.hook {
                Hook::Ability(EXILE_DISCARD) => Set::Menagerie,
                _ => f.card.set(),
            };
            let mut flow = match set {
                $(Set::$set => <$module::Rules as SetRules>::run(g, f, input),)*
            }
            .unwrap_or_else(Flow::done);
            if g.turn.ignore_plus_actions && g.turn.actions > before.actions {
                g.turn.actions = before.actions;
            }
            $(<$module::Rules as SetRules>::after_step(g, f, &before, &mut flow);)*
            flow
        }

        pub(crate) fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
            $(<$module::Rules as SetRules>::triggers(g, occ, out);)*
        }

        pub(crate) fn modify_cost(g: &Game, card: Card, id: Option<CardId>, cost: &mut Cost) {
            $(<$module::Rules as SetRules>::modify_cost(g, card, id, cost);)*
        }

        pub(crate) fn fixed_cost(g: &Game, card: Card, id: Option<CardId>) -> Option<Cost> {
            None$(.or_else(|| <$module::Rules as SetRules>::fixed_cost(g, card, id)))*
        }

        pub(crate) fn pays_otherwise(g: &Game, p: PlayerId, card: Card) -> bool {
            false $(|| <$module::Rules as SetRules>::pays_otherwise(g, p, card))*
        }

        pub(crate) fn can_use_way(g: &Game, id: CardId, way: Card) -> bool {
            true $(&& <$module::Rules as SetRules>::can_use_way(g, id, way))*
        }

        pub(crate) fn modify_types(g: &Game, id: CardId, types: &mut Types) {
            $(<$module::Rules as SetRules>::modify_types(g, id, types);)*
        }

        pub(crate) fn modify_bonus(g: &Game, p: PlayerId, id: CardId, bonus: &mut Bonus) {
            $(<$module::Rules as SetRules>::modify_bonus(g, p, id, bonus);)*
        }

        pub(crate) fn extra_runs(g: &Game, p: PlayerId, id: CardId) -> u32 {
            0 $(+ <$module::Rules as SetRules>::extra_runs(g, p, id))*
        }

        pub(crate) fn behavior(g: &Game, id: CardId) -> Option<Card> {
            None$(.or_else(|| <$module::Rules as SetRules>::behavior(g, id)))*
        }

        pub(crate) fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
            match card.set() {
                $(Set::$set => <$module::Rules as SetRules>::vp(g, p, card),)*
            }
        }

        pub(crate) fn bonus_vp(g: &Game, p: PlayerId) -> i32 {
            0 $(+ <$module::Rules as SetRules>::bonus_vp(g, p))*
        }

        pub(crate) fn notes(g: &Game) -> Vec<crate::Note> {
            let mut out = vec![];
            $(<$module::Rules as SetRules>::notes(g, &mut out);)*
            out
        }

        pub(crate) fn modify_decision(g: &Game, f: &CardFrame, d: &mut Decision) {
            $(<$module::Rules as SetRules>::modify_decision(g, f, d);)*
        }

        pub(crate) fn can_play(g: &Game, p: PlayerId, id: CardId) -> bool {
            true $(&& <$module::Rules as SetRules>::can_play(g, p, id))*
        }

        pub(crate) fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
            true $(&& <$module::Rules as SetRules>::can_buy(g, p, card))*
        }

        pub(crate) fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
            match card.set() {
                $(Set::$set => <$module::Rules as SetRules>::build_kingdom_pile(g, card),)*
            }
        }

        pub(crate) fn ensure(g: &mut Game, s: &mut Setup) {
            $(<$module::Rules as SetRules>::ensure(g, s);)*
        }

        pub(crate) fn setup(g: &mut Game, s: &mut Setup) {
            $(<$module::Rules as SetRules>::setup(g, s);)*
        }

        pub(crate) fn bottom_when_shuffled(g: &Game, p: PlayerId, id: CardId) -> bool {
            false $(|| <$module::Rules as SetRules>::bottom_when_shuffled(g, p, id))*
        }

        pub(crate) fn playable_as_if_in_hand(g: &Game, p: PlayerId, out: &mut Vec<CardId>) {
            $(<$module::Rules as SetRules>::playable_as_if_in_hand(g, p, out);)*
        }

        pub(crate) fn actions_are_buys(g: &Game) -> bool {
            false $(|| <$module::Rules as SetRules>::actions_are_buys(g))*
        }

        pub(crate) fn modify_starting_deck(g: &Game, p: PlayerId, deck: &mut Vec<Card>) {
            $(<$module::Rules as SetRules>::modify_starting_deck(g, p, deck);)*
        }

        pub(crate) fn on_event(g: &mut Game, event: &Event) {
            $(<$module::Rules as SetRules>::on_event(g, event);)*
        }

        pub(crate) fn final_round(g: &Game, p: PlayerId) -> bool {
            false $(|| <$module::Rules as SetRules>::final_round(g, p))*
        }
    };
}

sets! {
    Base => base,
    Intrigue => intrigue,
    Seaside => seaside,
    Alchemy => alchemy,
    Prosperity => prosperity,
    CornucopiaGuilds => cornucopia_guilds,
    Hinterlands => hinterlands,
    DarkAges => dark_ages,
    Adventures => adventures,
    Empires => empires,
    Nocturne => nocturne,
    Renaissance => renaissance,
    Menagerie => menagerie,
    Allies => allies,
    Plunder => plunder,
    RisingSun => rising_sun,
    Arcana => arcana,
    Promos => promos,
}

/// The turn player's Duration effects (from [`CardFrame::add_duration`]), if `occ` is a
/// `StartTurn`: what happens "at the start of each of your turns" (Hireling) does so on
/// every one, the turn it is played at the start of included.
pub(crate) fn turn_durations<'a>(g: &'a Game, occ: &Occasion) -> impl Iterator<Item = &'a Lasting> {
    let player = match *occ {
        Occasion::StartTurn { player } => Some(player),
        _ => None,
    };
    g.lasting()
        .iter()
        .filter(move |l| Some(l.owner) == player && l.until == Until::Removed)
}

/// The Duration effects due "at the start of your next turn", if `occ` is a `StartTurn`:
/// the turn player's, made on an earlier turn of theirs (one played during this start of
/// turn waits for the next).
pub(crate) fn due_durations<'a>(g: &'a Game, occ: &Occasion) -> impl Iterator<Item = &'a Lasting> {
    turn_durations(g, occ).filter(|l| l.turn < g.player(l.owner).turns_taken)
}

/// The start-of-turn trigger that resolves Duration effect `l` as its card's `ability`.
/// The ability calls [`CardFrame::end_lasting`].
pub(crate) fn duration_trigger(l: &Lasting, ability: u8) -> Trigger {
    let t = Trigger::new(l.owner, l.card, ability).via_lasting(l.id);
    match l.source {
        Some(c) => t.on(c),
        None => t,
    }
}

/// Ability numbers any card can be run with, above every card's own. One enum, so that the
/// compiler rejects two that collide or one past 255.
#[repr(u8)]
enum Reserved {
    Sun = 252,
    ExileDiscard,
    PayOtherwise,
    Overpay,
}

/// An Omen's "+1 Sun" is resolved as the card's ability with this number.
pub(crate) const SUN: u8 = Reserved::Sun as u8;
/// The Exile rule ("when you gain a card, you may discard its copies from Exile"), as an
/// ability of the gained card, whatever its set: [`resume`] sends it to Menagerie.
pub(crate) const EXILE_DISCARD: u8 = Reserved::ExileDiscard as u8;
/// Paying for a card another way than with its cost (Animal Fair) is offered as the card's
/// ability with this number, which finishes with `Value::Bool(true)` if it was paid that way.
pub(crate) const PAY_OTHERWISE: u8 = Reserved::PayOtherwise as u8;
/// Overpay is resolved as the bought card's ability with this number.
pub(crate) const OVERPAY: u8 = Reserved::Overpay as u8;

/// What following `card`'s instructions does before anything else, its vanilla bonuses
/// included: an Omen's "+1 Sun".
pub(crate) fn prelude(card: Card, player: PlayerId, id: CardId) -> Option<Frame> {
    card.is(Types::OMEN).then(|| {
        let f = CardFrame::new(card, Hook::Ability(SUN), player, Some(id));
        Frame::Card(Box::new(f))
    })
}

/// The chosen `Opt::Mode`s as a bit mask, to resolve in printed order with [`take_mode`].
pub(crate) fn mode_mask(input: Input) -> i32 {
    input.options().iter().fold(0, |m, o| match o {
        Opt::Mode(i) => m | 1 << i,
        _ => m,
    })
}

/// Takes the first mode left in `f.s.m`, in printed order.
pub(crate) fn take_mode(f: &mut CardFrame) -> Option<u8> {
    if f.s.m == 0 {
        return None;
    }
    let i = f.s.m.trailing_zeros() as u8;
    f.s.m &= !(1 << i);
    Some(i)
}

/// Which ability of a card is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Hook {
    /// Played (cards), bought (Events, Projects), used (Ways), received (Boons, Hexes).
    Main,
    /// A triggered or other numbered ability, numbered by the card's set module.
    Ability(u8),
}

/// The suspended state of one card ability.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CardFrame {
    pub card: Card,
    pub hook: Hook,
    /// Whose ability it is.
    pub player: PlayerId,
    /// The physical card the ability is on (for a Way: the card played), if any.
    pub this: Option<CardId>,
    /// For Attacks: the other players affected, in turn order.
    pub targets: Vec<PlayerId>,
    /// The cards playing this one (Throne Room, ...), innermost last.
    pub chain: Vec<CardId>,
    /// For a triggered ability: what triggered it.
    pub occ: Option<Occasion>,
    /// For a triggered ability: the lasting effect it comes from.
    pub lasting: Option<u32>,
    /// For a triggered ability: its `Trigger::data`; for overpay: the amount.
    pub data: i32,
    pub pc: u8,
    /// Free for the script's use between steps.
    pub s: Scratch,
    /// A step's flow, deferred until the frame called in its place returns.
    pub held: Option<Box<Flow>>,
}

impl CardFrame {
    pub(crate) fn new(card: Card, hook: Hook, player: PlayerId, this: Option<CardId>) -> CardFrame {
        CardFrame {
            card,
            hook,
            player,
            this,
            targets: vec![],
            chain: vec![],
            occ: None,
            lasting: None,
            data: 0,
            pc: 0,
            s: Scratch::default(),
            held: None,
        }
    }

    /// Defers `flow` until this frame is next resumed (after the frame called instead).
    pub(crate) fn hold(&mut self, flow: Flow) {
        self.held = Some(Box::new(flow));
    }

    /// The card this ability is on. Panics for abilities that have none.
    pub(crate) fn this(&self) -> CardId {
        self.this.expect("ability has a physical card")
    }

    /// The cards that stay in play while a Duration effect of this play lasts, innermost
    /// first: this card, the card that played it, and further up the chain only while the
    /// card played is a Duration (a Throne Room that plays a Throne Room that plays a
    /// Duration doesn't stay; one that plays a Mastermind that plays a Duration does). A
    /// Command card doesn't stay for the Throne Room it played from where it was.
    pub(crate) fn keeps(&self, g: &Game) -> Vec<CardId> {
        let mut keeps: Vec<CardId> = self.this.into_iter().collect();
        for &c in self.chain.iter().rev() {
            if keeps
                .last()
                .is_some_and(|&inner| !g.is(inner, Types::DURATION))
            {
                break;
            }
            keeps.push(c);
        }
        keeps
    }

    /// Schedules this play's Duration effect: a [`crate::Lasting`] that keeps this card
    /// (and whatever played it) in play until removed.
    pub(crate) fn add_duration(&self, g: &mut Game, data: Scratch) -> u32 {
        let keeps = self.keeps(g);
        g.add_lasting(
            self.card,
            self.player,
            self.this,
            keeps,
            Until::Removed,
            data,
        )
    }

    /// Ends the lasting effect this triggered ability comes from, returning it.
    pub(crate) fn end_lasting(&self, g: &mut Game) -> Option<Lasting> {
        self.lasting.and_then(|id| g.remove_lasting(id))
    }

    /// The chain to pass when this card plays another card.
    pub(crate) fn chain_for_child(&self) -> Vec<CardId> {
        self.chain.iter().copied().chain(self.this).collect()
    }

    fn ask(&self, player: PlayerId, kind: DecisionKind, purpose: Purpose) -> Flow {
        Flow::Ask(Decision {
            player,
            kind,
            purpose,
            source: Some(self.card),
            about: vec![],
        })
    }

    pub(crate) fn ask_cards(
        &self,
        candidates: Vec<CardId>,
        min: usize,
        max: usize,
        purpose: Purpose,
    ) -> Flow {
        self.ask_cards_for(self.player, candidates, min, max, purpose)
    }

    /// Ask `player` to choose `min..=max` of `candidates` (clamped to how many there are).
    pub(crate) fn ask_cards_for(
        &self,
        player: PlayerId,
        candidates: Vec<CardId>,
        min: usize,
        max: usize,
        purpose: Purpose,
    ) -> Flow {
        let max = max.min(candidates.len());
        let min = min.min(max);
        self.ask(
            player,
            DecisionKind::Cards {
                candidates,
                min,
                max,
                ordered: false,
            },
            purpose,
        )
    }

    /// Ask for all of `cards` in an order, top first.
    pub(crate) fn ask_order(&self, cards: Vec<CardId>, purpose: Purpose) -> Flow {
        let n = cards.len();
        self.ask(
            self.player,
            DecisionKind::Cards {
                candidates: cards,
                min: n,
                max: n,
                ordered: true,
            },
            purpose,
        )
    }

    pub(crate) fn ask_pile(&self, piles: Vec<PileId>, optional: bool, purpose: Purpose) -> Flow {
        self.ask_pile_for(self.player, piles, optional, purpose)
    }

    pub(crate) fn ask_pile_for(
        &self,
        player: PlayerId,
        piles: Vec<PileId>,
        optional: bool,
        purpose: Purpose,
    ) -> Flow {
        self.ask(player, DecisionKind::Pile { piles, optional }, purpose)
    }

    pub(crate) fn ask_yes_no(&self, purpose: Purpose) -> Flow {
        self.ask_yes_no_for(self.player, purpose)
    }

    pub(crate) fn ask_yes_no_for(&self, player: PlayerId, purpose: Purpose) -> Flow {
        self.ask_choose_for(
            player,
            vec![Opt::Yes, Opt::No],
            vec!["Yes".into(), "No".into()],
            1,
            1,
            purpose,
        )
    }

    /// Ask to pick `min..=max` of the card's numbered choices; answers are `Opt::Mode(i)`.
    pub(crate) fn ask_modes(&self, labels: &[&str], min: usize, max: usize) -> Flow {
        self.ask_numbered(self.player, labels, min, max, Purpose::Mode)
    }

    /// Ask to pick one of alternatives that aren't a "choose" (`Purpose::Either`); answers
    /// are `Opt::Mode(i)`.
    pub(crate) fn ask_either(&self, labels: &[&str]) -> Flow {
        self.ask_numbered(self.player, labels, 1, 1, Purpose::Either)
    }

    pub(crate) fn ask_numbered(
        &self,
        player: PlayerId,
        labels: &[&str],
        min: usize,
        max: usize,
        purpose: Purpose,
    ) -> Flow {
        let options = (0..labels.len()).map(|i| Opt::Mode(i as u8)).collect();
        let labels = labels.iter().map(|s| s.to_string()).collect();
        self.ask_choose_for(player, options, labels, min, max, purpose)
    }

    pub(crate) fn ask_choose_for(
        &self,
        player: PlayerId,
        options: Vec<Opt>,
        labels: Vec<String>,
        min: usize,
        max: usize,
        purpose: Purpose,
    ) -> Flow {
        let max = max.min(options.len());
        let min = min.min(max);
        self.ask(
            player,
            DecisionKind::Choose {
                options,
                labels,
                min,
                max,
            },
            purpose,
        )
    }

    pub(crate) fn ask_name(&self) -> Flow {
        self.ask_name_for(self.player)
    }

    pub(crate) fn ask_name_for(&self, player: PlayerId) -> Flow {
        self.ask(player, DecisionKind::NameCard, Purpose::Name)
    }

    pub(crate) fn ask_number(&self, min: i32, max: i32, purpose: Purpose) -> Flow {
        self.ask(self.player, DecisionKind::Number { min, max }, purpose)
    }
}
