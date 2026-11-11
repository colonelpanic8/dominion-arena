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
//! runs.

use crate::decision::{Decision, DecisionKind, Opt, Purpose};
use crate::frame::{Flow, Input};
use crate::game::{Lasting, Scratch, Until};
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

    /// Applies this set's cost modifiers to `card` (the physical `id`, if any).
    fn modify_cost(g: &Game, card: Card, id: Option<CardId>, cost: &mut Cost) {}

    /// Applies this set's type-changing effects to `id`.
    fn modify_types(g: &Game, id: CardId, types: &mut Types) {}

    /// Changes the vanilla bonuses `p` gets from playing `id`.
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

    /// Whether `p` may buy `card` (a card, Event or Project) when they can afford it.
    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        true
    }

    /// Builds the Kingdom pile named `card` if it isn't 10 (or 8/12) plain copies.
    /// Returns whether it did.
    fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
        false
    }

    /// Setup after the Supply is built: extra piles, decks of Boons, tokens, ...
    fn setup(g: &mut Game) {}

    /// Changes a player's starting deck (Heirlooms, ...).
    fn modify_starting_deck(g: &Game, p: PlayerId, deck: &mut Vec<Card>) {}
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
            match f.card.set() {
                $(Set::$set => <$module::Rules as SetRules>::run(g, f, input),)*
            }
            .unwrap_or_else(Flow::done)
        }

        pub(crate) fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
            $(<$module::Rules as SetRules>::triggers(g, occ, out);)*
        }

        pub(crate) fn modify_cost(g: &Game, card: Card, id: Option<CardId>, cost: &mut Cost) {
            $(<$module::Rules as SetRules>::modify_cost(g, card, id, cost);)*
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

        pub(crate) fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
            true $(&& <$module::Rules as SetRules>::can_buy(g, p, card))*
        }

        pub(crate) fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
            match card.set() {
                $(Set::$set => <$module::Rules as SetRules>::build_kingdom_pile(g, card),)*
            }
        }

        pub(crate) fn setup(g: &mut Game) {
            $(<$module::Rules as SetRules>::setup(g);)*
        }

        pub(crate) fn modify_starting_deck(g: &Game, p: PlayerId, deck: &mut Vec<Card>) {
            $(<$module::Rules as SetRules>::modify_starting_deck(g, p, deck);)*
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

/// The Duration effects (from [`CardFrame::add_duration`]) due now, if `occ` is a
/// `StartTurn`: the turn player's, made on an earlier turn of theirs (one played during
/// this start of turn waits for the next).
pub(crate) fn due_durations<'a>(g: &'a Game, occ: &Occasion) -> impl Iterator<Item = &'a Lasting> {
    let player = match *occ {
        Occasion::StartTurn { player } => Some(player),
        _ => None,
    };
    g.lasting().iter().filter(move |l| {
        Some(l.owner) == player
            && l.until == Until::Removed
            && l.turn < g.player(l.owner).turns_taken
    })
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
        }
    }

    /// The card this ability is on. Panics for abilities that have none.
    pub(crate) fn this(&self) -> CardId {
        self.this.expect("ability has a physical card")
    }

    /// The cards that stay in play while a Duration effect of this play lasts.
    pub(crate) fn keeps(&self) -> Vec<CardId> {
        self.this
            .into_iter()
            .chain(self.chain.iter().copied())
            .collect()
    }

    /// Schedules this play's Duration effect: a [`crate::Lasting`] that keeps this card
    /// (and whatever played it) in play until removed.
    pub(crate) fn add_duration(&self, g: &mut Game, data: Scratch) -> u32 {
        g.add_lasting(
            self.card,
            self.player,
            self.this,
            self.keeps(),
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
        self.keeps()
    }

    fn ask(&self, player: PlayerId, kind: DecisionKind, purpose: Purpose) -> Flow {
        Flow::Ask(Decision {
            player,
            kind,
            purpose,
            source: Some(self.card),
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
        self.ask_modes_for(self.player, labels, min, max)
    }

    pub(crate) fn ask_modes_for(
        &self,
        player: PlayerId,
        labels: &[&str],
        min: usize,
        max: usize,
    ) -> Flow {
        let options = (0..labels.len()).map(|i| Opt::Mode(i as u8)).collect();
        let labels = labels.iter().map(|s| s.to_string()).collect();
        self.ask_choose_for(player, options, labels, min, max, Purpose::Mode)
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
        self.ask(self.player, DecisionKind::NameCard, Purpose::Name)
    }

    pub(crate) fn ask_number(&self, min: i32, max: i32, purpose: Purpose) -> Flow {
        self.ask(self.player, DecisionKind::Number { min, max }, purpose)
    }
}
