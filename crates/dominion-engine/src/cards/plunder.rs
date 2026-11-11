//! Plunder: its Kingdom cards, the Loot pile, Events and Traits.
//!
//! Durations keep their future part in a lasting made by [`CardFrame::add_duration`]
//! (see Seaside). "Next time" Durations (Cage, Search, Secluded Shrine, Abundance,
//! Cutthroat, Flagship, Landing Party) wait in play until their event, and resolve as
//! their `WAITED` ability. Effects that wait record the log's length when they were set
//! up in `data.i`; "the next time" ones answer the first gain or play to begin after that
//! (`GainState::since`, `PlayState::since`; see [`next_gain`] and [`next_play`]). Cards
//! held until the end of the turn (Cage, Puzzle Box, Deliver, Trickster, Tireless) sit in
//! their owner's `SetAside` zone, listed in an `Until::EndOfTurn` lasting's `data.cards`.
//! Traits sit on their pile (`Pile::traits`); the Loot pile is a face-down non-Supply pile
//! named [`Card::Loot`].

use super::{CardFrame, Hook, SetRules, Step, duration_trigger, mode_mask, turn_durations};
use crate::decision::{Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, Value};
use crate::game::{ExtraLimit, ExtraTurn, GainState, Lasting, Phase, Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup, add_mixed_pile};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Set, Trace, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {}

pub(crate) struct Rules;

/// A Duration's (or set-aside cards') start-of-turn effect.
const START: u8 = 0;
/// A "next time" Duration's event happened.
const WAITED: u8 = 1;
/// End of turn: cards held until then go to the hand (Tireless: onto the deck).
const HELD: u8 = 2;
/// "When you gain this", and the Traits' when-gain effects.
const GAINED: u8 = 3;
/// A Reaction played or revealed from the hand.
const REACT: u8 = 4;
const TRASHED: u8 = 5;
/// A "this turn" or "until your next turn" effect (Insignia, Mining Road, Frigate, ...).
const THIS_TURN: u8 = 6;
const SHAMAN_GAIN: u8 = 8;
const TASKMASTER_ARM: u8 = 9;
/// A Trait's ability other than when-gain (Friendly, Shy, Tireless, ...).
const TRAIT: u8 = 10;
const INHERIT: u8 = 11;

const LOOT_GAINERS: &[Card] = &[
    Card::JewelledEgg,
    Card::Search,
    Card::Pickaxe,
    Card::WealthyVillage,
    Card::Cutthroat,
    Card::SackOfLoot,
    Card::Peril,
    Card::Foray,
    Card::Looting,
    Card::Invasion,
    Card::Prosper,
];

/// Cards whose lastings resolve at the start of their owner's turn.
fn has_start(card: Card) -> bool {
    matches!(
        card,
        Card::Grotto
            | Card::Siren
            | Card::Stowaway
            | Card::Taskmaster
            | Card::CabinBoy
            | Card::Gondola
            | Card::Rope
            | Card::BuriedTreasure
            | Card::Crew
            | Card::Enlarge
            | Card::Longship
            | Card::Quartermaster
            | Card::Amphora
            | Card::EndlessChalice
            | Card::Figurehead
            | Card::Jewels
            | Card::Hasty
            | Card::Patient
            | Card::Prepare
    )
}

/// Cards whose start-of-turn effect is "at the start of each of your turns for the rest of
/// the game": it resolves on every turn, the one it is played at the start of included.
fn permanent(card: Card) -> bool {
    matches!(card, Card::Quartermaster | Card::EndlessChalice)
}

/// Cards whose Duration waits for "the next time" something happens.
fn waits(card: Card) -> bool {
    matches!(
        card,
        Card::Cage
            | Card::Search
            | Card::SecludedShrine
            | Card::Abundance
            | Card::Cutthroat
            | Card::Flagship
            | Card::LandingParty
    )
}

/// Lastings that hold cards set aside until the end of the turn.
fn holds(card: Card) -> bool {
    matches!(
        card,
        Card::Cage | Card::PuzzleBox | Card::Deliver | Card::Trickster | Card::Tireless
    )
}

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (c, Hook::Ability(START)) if has_start(c) => start(g, f, input),
            (_, Hook::Ability(WAITED)) => waited(g, f, input),
            (_, Hook::Ability(HELD)) => held(g, f),
            (_, Hook::Ability(GAINED)) => gained(g, f, input),
            (_, Hook::Ability(REACT)) => react(g, f),
            (Card::JewelledEgg, Hook::Ability(TRASHED)) => once(f, |g, f| gain_loot(g, f.player, Dest::Discard), g),
            (_, Hook::Ability(THIS_TURN)) => this_turn(g, f, input),
            (Card::Shaman, Hook::Ability(SHAMAN_GAIN)) => shaman_gain(g, f, input),
            (Card::Taskmaster, Hook::Ability(TASKMASTER_ARM)) => {
                if let Some(l) = f.lasting.and_then(|id| g.lasting_mut(id)) {
                    l.until = Until::Removed;
                    let (id, keeps) = (l.id, l.data.cards.clone());
                    g.keep_in_play(id, keeps);
                }
                Flow::done()
            }
            (_, Hook::Ability(TRAIT)) => trait_ability(g, f, input),
            (Card::Inherited, Hook::Ability(INHERIT)) => inherited(g, f, input),
            (_, Hook::Ability(_)) => return None,
            // Kingdom cards
            (Card::Cage, _) => cage(g, f, input),
            (Card::Grotto, _) => grotto(g, f, input),
            (
                Card::Search
                | Card::SecludedShrine
                | Card::Abundance
                | Card::Flagship
                | Card::LandingParty
                | Card::Stowaway
                | Card::CabinBoy
                | Card::BuriedTreasure
                | Card::Crew
                | Card::Longship
                | Card::Quartermaster
                | Card::Figurehead
                | Card::Jewels
                | Card::Rope,
                _,
            ) => {
                duration(g, f);
                Flow::done()
            }
            (Card::Shaman, _) => trash_up_to(g, f, input, 0, 1),
            (Card::Siren, _) => siren(g, f),
            (Card::Taskmaster, _) => {
                plus_actions(g, f.player, 1);
                coins(g, f.player, 1);
                taskmaster_watch(g, f.player, f.this, f.keeps(g));
                Flow::done()
            }
            (Card::Crucible, _) => crucible(g, f, input),
            (Card::FortuneHunter, _) => fortune_hunter(g, f, input),
            (Card::Gondola, _) => now_or_next(g, f, input),
            (Card::Amphora, _) => now_or_next(g, f, input),
            (Card::HarborVillage | Card::MiningRoad | Card::Insignia, _) => {
                let data = stamped(g);
                g.add_lasting(f.card, f.player, f.this, vec![], Until::EndOfTurn, data);
                Flow::done()
            }
            (Card::Mapmaker, _) => mapmaker(g, f, input),
            (Card::Maroon, _) => maroon(g, f, input),
            (Card::SwampShacks, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                    let n = g.in_play(f.player).len() / 3;
                    return Some(Flow::Call(Frame::draw(f.player, n)));
                }
                Flow::done()
            }
            (Card::Tools, _) => tools(g, f, input),
            (Card::Cutthroat, _) => {
                let flow = discard_down(g, f, input, 3);
                if matches!(flow, Flow::Done(_)) {
                    duration(g, f);
                }
                flow
            }
            (Card::Enlarge, _) => {
                if f.pc == 0 {
                    duration(g, f);
                }
                remodel(g, f, input, 0, 2)
            }
            (Card::Figurine, _) => figurine(g, f, input),
            (Card::FirstMate, _) => first_mate(g, f, input),
            (Card::Frigate, _) => {
                if !f.targets.is_empty() {
                    let data = Scratch {
                        n: mask(&f.targets),
                        ..stamped(g)
                    };
                    g.add_lasting(
                        Card::Frigate,
                        f.player,
                        f.this,
                        f.keeps(g),
                        Until::StartOfTurn(f.player),
                        data,
                    );
                }
                Flow::done()
            }
            (Card::Pendant, _) => {
                let n = treasure_names_in_play(g, f.player);
                coins(g, f.player, n as u32);
                Flow::done()
            }
            (Card::Pickaxe, _) => pickaxe(g, f, input),
            (Card::Pilgrim, _) => pilgrim(g, f, input),
            (Card::SilverMine, _) => silver_mine(g, f, input),
            (Card::Trickster, _) => {
                if each_target_gains(g, f, Card::Curse) {
                    return Some(Flow::Call(Frame::gain(
                        f.targets[f.s.i - 1],
                        g.supply_pile(Card::Curse).expect("checked"),
                        Dest::Discard,
                    )));
                }
                g.add_lasting(
                    Card::Trickster,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::SackOfLoot, _) => once(f, |g, f| gain_loot(g, f.player, Dest::Discard), g),
            (Card::KingsCache, _) => kings_cache(g, f, input),
            // Loot
            (Card::EndlessChalice, _) => {
                coins(g, f.player, 1);
                plus_buys(g, f.player, 1);
                duration(g, f);
                Flow::done()
            }
            (Card::Hammer, _) => gain_up_to(g, f, input, 0, Cost::coins(4), Dest::Discard),
            (Card::Orb, _) => orb(g, f, input),
            (Card::PrizeGoat, _) => trash_up_to(g, f, input, 0, 1),
            (Card::PuzzleBox, _) => puzzle_box(g, f, input),
            (Card::Sextant, _) => sextant(g, f, input),
            (Card::SpellScroll, _) => spell_scroll(g, f, input),
            (Card::Staff, _) => play_from_hand(g, f, input, Types::ACTION),
            (Card::Sword, _) => discard_down(g, f, input, 4),
            // Events
            (Card::Bury, _) => bury(g, f, input),
            (Card::Avoid | Card::Rush | Card::Mirror, _) => {
                plus_buys(g, f.player, 1);
                let data = stamped(g);
                g.add_lasting(f.card, f.player, None, vec![], Until::EndOfTurn, data);
                Flow::done()
            }
            (Card::Deliver, _) => {
                plus_buys(g, f.player, 1);
                if this_turn_lasting(g, Card::Deliver, f.player).is_none() {
                    let data = stamped(g);
                    g.add_lasting(Card::Deliver, f.player, None, vec![], Until::EndOfTurn, data);
                }
                Flow::done()
            }
            (Card::Peril, _) => peril(g, f, input),
            (Card::Foray, _) => foray(g, f, input),
            (Card::Launch, _) => launch(g, f),
            (Card::Prepare, _) => {
                let p = f.player;
                let hand = g.hand(p).to_vec();
                if !hand.is_empty() {
                    for &c in &hand {
                        g.set_aside(c, p);
                    }
                    let data = Scratch {
                        cards: hand,
                        ..Scratch::default()
                    };
                    g.add_lasting(Card::Prepare, p, None, vec![], Until::Removed, data);
                }
                Flow::done()
            }
            (Card::Scrounge, _) => scrounge(g, f, input),
            (Card::Journey, _) => {
                let p = f.player;
                g.add_lasting(
                    Card::Journey,
                    p,
                    None,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                if p == g.turn.player && !g.third_in_a_row(p) {
                    g.extra_turns.push_back(ExtraTurn {
                        player: p,
                        source: Card::Journey,
                        controller: g.turn.controller,
                        limit: ExtraLimit::ThirdInARow,
                    });
                }
                Flow::done()
            }
            (Card::Maelstrom, _) => maelstrom(g, f, input),
            (Card::Looting, _) => once(f, |g, f| gain_loot(g, f.player, Dest::Discard), g),
            (Card::Invasion, _) => invasion(g, f, input),
            (Card::Prosper, _) => prosper(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in turn_durations(g, occ) {
            let due = permanent(l.card) || l.turn < g.player(l.owner).turns_taken;
            if due && l.card.set() == Set::Plunder && has_start(l.card) {
                out.push(duration_trigger(l, START));
            }
        }
        search_triggers(g, out);
        match *occ {
            Occasion::GameStart => {
                if let Some(pile) = trait_pile(g, Card::Inherited) {
                    for p in g.players_from(PlayerId(0)) {
                        let has = g
                            .player(p)
                            .owned()
                            .any(|c| g.home_pile(g.printed(c)) == Some(pile));
                        if has {
                            out.push(Trigger::new(p, Card::Inherited, INHERIT));
                        }
                    }
                }
            }
            Occasion::StartTurn { player } => {
                if setup::in_game(g, |c| c == Card::Shaman) {
                    out.push(Trigger::new(player, Card::Shaman, SHAMAN_GAIN));
                }
                if !traited_in_hand(g, player, Card::Shy).is_empty() {
                    out.push(Trigger::new(player, Card::Shy, TRAIT).optional());
                }
            }
            Occasion::StartCleanup { player } => {
                if !traited_in_hand(g, player, Card::Friendly).is_empty() {
                    out.push(Trigger::new(player, Card::Friendly, TRAIT).optional());
                }
                if !traited_in_hand(g, player, Card::Patient).is_empty() {
                    out.push(Trigger::new(player, Card::Patient, TRAIT).optional());
                }
                journey_trigger(g, player, out);
            }
            Occasion::DiscardFromPlay { player } => discard_from_play_triggers(g, player, out),
            Occasion::EndTurn { .. } => {
                for l in g.lasting() {
                    let waiting = l
                        .data
                        .cards
                        .iter()
                        .any(|&c| g.zone(c) == Zone::SetAside(l.owner));
                    if holds(l.card) && l.until == Until::EndOfTurn && waiting {
                        out.push(Trigger::new(l.owner, l.card, HELD).via_lasting(l.id));
                    }
                }
            }
            Occasion::AfterPlay { player, card } => after_play_triggers(g, player, card, out),
            Occasion::Attack { target, .. } => {
                for &c in g.hand(target) {
                    if g.card(c) == Card::Shield {
                        out.push(Trigger::new(target, Card::Shield, REACT).on(c).optional());
                    }
                }
            }
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            Occasion::Trash { player, card } => {
                if g.card(card) == Card::JewelledEgg {
                    out.push(Trigger::new(player, Card::JewelledEgg, TRASHED).on(card));
                }
            }
            Occasion::Shuffled { player, .. } => {
                for l in g.lasting() {
                    if l.card == Card::Avoid && l.owner == player {
                        out.push(Trigger::new(player, Card::Avoid, THIS_TURN).via_lasting(l.id));
                    }
                }
                if !shuffled_fated(g, player).is_empty() {
                    out.push(Trigger::new(player, Card::Fated, THIS_TURN).optional());
                }
            }
            _ => {}
        }
    }

    fn modify_cost(g: &Game, card: Card, _id: Option<CardId>, cost: &mut Cost) {
        if is_trait(g, card, Card::Cheap) {
            *cost = cost.minus_coins(1);
        }
    }

    fn extra_runs(g: &Game, _p: PlayerId, id: CardId) -> u32 {
        u32::from(is_traited(g, id, Card::Reckless))
    }

    /// Notes +$ a step gave on the play in progress of the card whose ability it is
    /// (Harbor Village). Way of the Chameleon's exchange (Menagerie's, which runs first)
    /// counts as what the step gave.
    fn after_step(g: &mut Game, f: &mut CardFrame, before: &Step, _flow: &mut Flow) {
        if f.player != g.turn.player || g.turn.coins <= before.coins {
            return;
        }
        let this = f.this.or_else(|| {
            let id = f.lasting?;
            g.lasting().iter().find(|l| l.id == id)?.source
        });
        if let Some(st) = g.plays.iter_mut().rev().find(|st| Some(st.card) == this)
            && (f.card == st.behavior || Some(f.card) == st.way)
        {
            st.gave_coins = true;
        }
    }

    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        card != Card::Launch || this_turn_lasting(g, Card::Launch, p).is_none()
    }

    fn ensure(g: &mut Game, _s: &mut Setup) {
        let gainer = setup::in_game(g, |c| LOOT_GAINERS.contains(&c))
            || g.landscapes.contains(&Card::Cursed);
        if gainer {
            add_loot_pile(g);
        }
    }

    fn setup(g: &mut Game, s: &mut Setup) {
        let traits: Vec<Card> = g
            .landscapes
            .iter()
            .copied()
            .filter(|&l| l.kind() == Kind::Trait && s.joined(l))
            .collect();
        for t in traits {
            let eligible: Vec<Card> = g
                .piles
                .iter()
                .filter(|p| {
                    p.in_supply
                        && p.card.def().kingdom
                        && p.traits.is_empty()
                        && p.card.is(Types::ACTION | Types::TREASURE)
                })
                .map(|p| p.card)
                .collect();
            if let Some(pile) = s.choose(g, t, eligible).and_then(|c| g.supply_pile(c)) {
                g.piles[pile.index()].traits.push(t);
            }
        }

        // Inherited: each player in turn order takes a card of its pile for their deck.
        if s.joined(Card::Inherited)
            && let Some(pile) = trait_pile(g, Card::Inherited)
        {
            for p in g.players_from(PlayerId(0)) {
                if let Some(top) = g.pile_top(pile) {
                    g.move_card(top, Zone::Deck(p), Pos::Top);
                }
            }
        }
    }

    fn modify_starting_deck(g: &Game, p: PlayerId, deck: &mut Vec<Card>) {
        let Some(pile) = trait_pile(g, Card::Inherited) else {
            return;
        };
        let inherited = g
            .player(p)
            .deck
            .iter()
            .any(|&c| g.home_pile(g.printed(c)) == Some(pile));
        if !inherited {
            return;
        }
        let default = default_replaced(deck.iter().copied());
        if let Some(i) = deck.iter().position(|&c| c == default) {
            deck.remove(i);
        }
    }
}

// ---- Helpers ----

/// The Loot pile: two of each Loot, shuffled face down, outside the Supply.
fn add_loot_pile(g: &mut Game) {
    if g.pile_of(Card::Loot).is_some() {
        return;
    }
    let mut loot: Vec<Card> = Card::Loot
        .pile_members()
        .into_iter()
        .flat_map(|c| [c; 2])
        .collect();
    g.shuffle(&mut loot);
    let pile = add_mixed_pile(g, Card::Loot, &loot, false);
    g.piles[pile.index()].hidden = true;
}

/// Scratch data stamped with the log's length now.
fn stamped(g: &Game) -> Scratch {
    Scratch {
        i: g.log().len(),
        ..Scratch::default()
    }
}

/// Runs `step` once, then finishes.
fn once(f: &mut CardFrame, step: impl FnOnce(&mut Game, &mut CardFrame) -> Flow, g: &mut Game) -> Flow {
    if f.pc == 0 {
        f.pc = 1;
        return step(g, f);
    }
    Flow::done()
}

/// Schedules this play's Duration effect, stamped (Search also records how many times
/// Supply piles have emptied).
fn duration(g: &mut Game, f: &mut CardFrame) {
    let mut data = stamped(g);
    if f.card == Card::Search {
        data.n = supply_piles_emptied(g);
    }
    f.add_duration(g, data);
}

fn supply_piles_emptied(g: &Game) -> i32 {
    g.piles().iter().map(|p| p.emptied as i32).sum()
}

fn mask(players: &[PlayerId]) -> i32 {
    players.iter().fold(0, |m, p| m | 1 << p.0)
}

fn in_mask(mask: i32, p: PlayerId) -> bool {
    mask & (1 << p.0) != 0
}

fn plus_actions(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.turn.actions += n;
    }
}

fn plus_buys(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.turn.buys += n;
    }
}

fn coins(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.add_coins(p, n);
    }
}

/// The gain of `card` in progress, if any.
fn gain_state(g: &Game, card: CardId) -> Option<&crate::game::GainState> {
    g.gains.iter().rev().find(|s| s.card == card)
}

/// Whether `card`, being gained, is still where it was gained to.
fn where_gained(g: &Game, card: CardId) -> bool {
    g.still_where_gained(card)
}

/// Whether the gain of `card` is the next one `wanted` accepts since lasting `l` was set
/// up: it began after that, and not while an earlier such gain was still being resolved
/// (that one is the next, whichever of its abilities are resolved first).
fn next_gain(g: &Game, l: &Lasting, card: CardId, wanted: impl Fn(&GainState) -> bool) -> bool {
    let Some(i) = g.gains.iter().rposition(|s| s.card == card) else {
        return false;
    };
    l.data.i <= g.gains[i].since
        && !g.gains[..i]
            .iter()
            .any(|s| l.data.i <= s.since && wanted(s))
}

/// [`next_gain`] for a card of `types` gained by `l`'s owner.
fn next_gain_of(g: &Game, l: &Lasting, card: CardId, types: Types) -> bool {
    next_gain(g, l, card, |s| s.player == l.owner && g.is(s.card, types))
}

/// Whether the play of `card` in progress is the next one by its player, of a card whose
/// types `wanted` accepts, since lasting `l` was set up: it began after that, and not
/// while an earlier such play was still being resolved (Throne Room, not the card it
/// plays).
pub(super) fn next_play(
    g: &Game,
    l: &Lasting,
    card: CardId,
    wanted: impl Fn(Types) -> bool,
) -> bool {
    let Some(i) = g.plays.iter().rposition(|s| s.card == card) else {
        return false;
    };
    let st = &g.plays[i];
    l.data.i <= st.since
        && wanted(g.types(card))
        && !g.plays[..i]
            .iter()
            .any(|s| s.player == st.player && l.data.i <= s.since && wanted(g.types(s.card)))
}

/// The play of `card` in progress (up to its AfterPlay window), if any.
fn play_state(g: &Game, card: CardId) -> Option<&crate::game::PlayState> {
    g.plays.iter().rev().find(|s| s.card == card)
}

/// `p`'s lasting of `card` made this turn (an `EndOfTurn` one), if any.
fn this_turn_lasting(g: &Game, card: Card, p: PlayerId) -> Option<&Lasting> {
    g.lasting()
        .iter()
        .find(|l| l.card == card && l.owner == p && l.until == Until::EndOfTurn)
}

/// The pile Trait `t` is on.
fn trait_pile(g: &Game, t: Card) -> Option<PileId> {
    if !g.has_landscape(t) {
        return None;
    }
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&p| g.pile(p).traits.contains(&t))
}

/// Whether cards named `card` are from the pile with Trait `t` (even once that pile is
/// out of the game).
fn is_trait(g: &Game, card: Card, t: Card) -> bool {
    trait_pile(g, t).is_some_and(|pile| g.origin_pile(card) == Some(pile))
}

fn is_traited(g: &Game, id: CardId, t: Card) -> bool {
    is_trait(g, g.card(id), t)
}

fn traited_in_hand(g: &Game, p: PlayerId, t: Card) -> Vec<CardId> {
    if trait_pile(g, t).is_none() {
        return vec![];
    }
    g.hand(p)
        .iter()
        .copied()
        .filter(|&c| is_traited(g, c, t))
        .collect()
}

fn loot_pile(g: &Game) -> Option<PileId> {
    g.pile_of(Card::Loot).filter(|&p| g.pile_top(p).is_some())
}

/// Gains a Loot (the top card of the Loot pile), as one step of an ability: with none
/// left, the ability goes on to its next step. Set `f.pc` first.
fn gain_loot(g: &Game, p: PlayerId, to: Dest) -> Flow {
    match loot_pile(g) {
        Some(pile) => Flow::Call(Frame::gain(p, pile, to)),
        None => Flow::Continue,
    }
}

/// Gains `card` from its Supply pile, as one step of an ability (see [`gain_loot`]).
fn gain_basic(g: &Game, p: PlayerId, card: Card) -> Flow {
    match g.supply_pile(card).filter(|&s| g.pile_top(s).is_some()) {
        Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
        None => Flow::Continue,
    }
}

/// The Supply pile whose top card is named `card`.
fn supply_pile_topped_by(g: &Game, card: Card) -> Option<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&id| g.pile(id).in_supply && g.pile_top(id).is_some_and(|t| g.card(t) == card))
}

/// The differently named Treasures `p` has in play.
fn treasure_names_in_play(g: &Game, p: PlayerId) -> usize {
    let mut names: Vec<Card> = g
        .in_play(p)
        .iter()
        .filter(|&&c| g.is(c, Types::TREASURE))
        .map(|&c| g.card(c))
        .collect();
    names.sort();
    names.dedup();
    names.len()
}

/// The starting card Inherited replaces unless its player chooses another.
fn default_replaced(cards: impl Iterator<Item = Card> + Clone) -> Card {
    [Card::Estate, Card::Hovel, Card::Copper]
        .into_iter()
        .find(|k| cards.clone().any(|c| c == *k))
        .unwrap_or(Card::Copper)
}

/// Asks `p` for a card from their hand to set aside face down.
fn set_aside_hidden(g: &mut Game, c: CardId, p: PlayerId) {
    g.move_card(c, Zone::SetAside(p), Pos::Top);
    g.emit(Event::Moved {
        player: p,
        card: None,
        to: Zone::SetAside(p),
    });
}

fn taskmaster_watch(g: &mut Game, p: PlayerId, source: Option<CardId>, keeps: Vec<CardId>) {
    let data = Scratch {
        cards: keeps,
        ..Scratch::default()
    };
    g.add_lasting(Card::Taskmaster, p, source, vec![], Until::EndOfTurn, data);
}

/// Each target, in turn, gains a `card`; returns whether a gain is due now (the next
/// target is `f.targets[f.s.i - 1]`).
fn each_target_gains(g: &Game, f: &mut CardFrame, card: Card) -> bool {
    while f.s.i < f.targets.len() {
        f.s.i += 1;
        if g.supply_pile(card).is_some_and(|p| g.pile_top(p).is_some()) {
            return true;
        }
    }
    false
}

// ---- Trigger producers ----

/// Search: "the next time a Supply pile empties" (one emptied since it was played).
fn search_triggers(g: &Game, out: &mut Vec<Trigger>) {
    let now = supply_piles_emptied(g);
    for l in g.lasting() {
        if l.card != Card::Search || l.until != Until::Removed {
            continue;
        }
        if now > l.data.n {
            out.push(duration_trigger(l, WAITED));
        }
    }
}

fn gain_triggers(g: &Game, gainer: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let kind = g.card(card);
    let types = g.types(card);
    // What it cost as it was gained (Taskmaster, Cutthroat).
    let cost = gain_state(g, card).map_or_else(|| g.cost_of(card), |s| s.cost);
    let on = |t: Trigger| t.on(card);

    // "When you gain this"
    match kind {
        Card::BuriedTreasure | Card::Siren | Card::Doubloons => {
            out.push(on(Trigger::new(gainer, kind, GAINED)));
        }
        Card::Gondola if !g.playable(gainer, Types::ACTION).is_empty() => {
            out.push(on(Trigger::new(gainer, kind, GAINED)).optional());
        }
        Card::WealthyVillage if treasure_names_in_play(g, gainer) >= 3 => {
            out.push(on(Trigger::new(gainer, kind, GAINED)));
        }
        _ => {}
    }

    // Traits
    for t in [Card::Cursed, Card::Hasty, Card::Nearby, Card::Rich] {
        if is_trait(g, kind, t) {
            out.push(on(Trigger::new(gainer, t, GAINED)));
        }
    }
    if is_trait(g, kind, Card::Pious) && !g.hand(gainer).is_empty() {
        out.push(on(Trigger::new(gainer, Card::Pious, GAINED)).optional());
    }
    if kind == Card::Province
        && let Some(pile) = trait_pile(g, Card::Fawning)
        && g.pile_top(pile).is_some()
    {
        out.push(on(Trigger::new(gainer, Card::Fawning, GAINED)));
    }

    // Effects waiting for a gain
    let mut rush = false;
    let mut mirror = false;
    for l in g.lasting() {
        let mine = l.owner == gainer;
        let next =
            |wanted| mine && types.contains(wanted) && next_gain_of(g, l, card, wanted);
        let via = |ability| duration_trigger(l, ability);
        match l.card {
            Card::Cage if l.until == Until::Removed && next(Types::VICTORY) => {
                out.push(via(WAITED));
            }
            Card::SecludedShrine if l.until == Until::Removed && next(Types::TREASURE) => {
                out.push(via(WAITED));
            }
            Card::Abundance if next(Types::ACTION) => {
                out.push(via(WAITED));
            }
            Card::Cutthroat
                if types.contains(Types::TREASURE)
                    && cost.coins >= 5
                    && next_gain(g, l, card, |s| {
                        g.is(s.card, Types::TREASURE) && s.cost.coins >= 5
                    }) =>
            {
                out.push(via(WAITED));
            }
            Card::Taskmaster
                if mine && l.until == Until::EndOfTurn && cost == Cost::coins(5) =>
            {
                out.push(
                    Trigger::new(gainer, Card::Taskmaster, TASKMASTER_ARM)
                        .via_lasting(l.id)
                        .quiet(),
                );
            }
            Card::Insignia
                if mine
                    && where_gained(g, card)
                    && !matches!(g.zone(card), Zone::Deck(_)) =>
            {
                out.push(via(THIS_TURN).optional());
            }
            Card::MiningRoad if mine && types.contains(Types::TREASURE) && where_gained(g, card) => {
                out.push(via(THIS_TURN).optional());
            }
            Card::Deliver if mine && where_gained(g, card) => {
                out.push(via(THIS_TURN));
            }
            Card::Rush if next(Types::ACTION) && !rush => {
                rush = true;
                out.push(via(THIS_TURN));
            }
            Card::Mirror if next(Types::ACTION) && !mirror => {
                mirror = true;
                out.push(via(THIS_TURN));
            }
            _ => {}
        }
    }

    // Reactions from the hand
    let victory = types.contains(Types::VICTORY);
    let duration = types.contains(Types::DURATION);
    if victory || duration {
        for p in g.players_from(g.turn.player) {
            for &c in g.hand(p) {
                let react = match g.card(c) {
                    Card::Mapmaker => victory,
                    Card::Stowaway => duration,
                    _ => false,
                };
                if react && super::can_play(g, p, c) {
                    out.push(Trigger::new(p, g.card(c), REACT).on(c).optional());
                }
            }
        }
    }
}

fn after_play_triggers(g: &Game, player: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let Some(st) = play_state(g, card) else {
        return;
    };
    let since = st.since;
    let types = g.types(card);
    let action = types.contains(Types::ACTION);
    for l in g.lasting() {
        let via = |ability| duration_trigger(l, ability);
        match l.card {
            Card::Flagship
                if l.owner == player
                    && l.until == Until::Removed
                    && next_play(g, l, card, |t| {
                        t.contains(Types::ACTION) && !t.contains(Types::COMMAND)
                    }) =>
            {
                out.push(via(WAITED));
            }
            Card::HarborVillage
                if l.owner == player && next_play(g, l, card, |t| t.contains(Types::ACTION)) =>
            {
                out.push(via(THIS_TURN));
            }
            Card::LandingParty
                if l.owner == player
                    && l.data.i <= since
                    && l.until == Until::Removed
                    && types.contains(Types::TREASURE)
                    && first_play_of_turn(g, player, since) =>
            {
                out.push(via(WAITED));
            }
            Card::Frigate
                if l.owner != player
                    && action
                    && in_mask(l.data.n, player)
                    && g.hand(player).len() > 4 =>
            {
                out.push(Trigger::new(player, Card::Frigate, THIS_TURN).via_lasting(l.id));
            }
            _ => {}
        }
    }
    if player == g.turn.player && is_traited(g, card, Card::Inspiring) {
        out.push(Trigger::new(player, Card::Inspiring, TRAIT).on(card).optional());
    }
}

/// Whether the play whose Played event is at `since` was `p`'s first play this turn.
fn first_play_of_turn(g: &Game, p: PlayerId, since: usize) -> bool {
    let log = g.log();
    let start = log
        .iter()
        .rposition(|e| matches!(e, Event::TurnStart { .. }))
        .unwrap_or(0);
    let end = since.min(log.len());
    start > end
        || !log[start..end]
            .iter()
            .any(|e| matches!(e, Event::Played { player, .. } if *player == p))
}

/// Journey keeps in play everything its buyer has there: asked at the start of Clean-up
/// (for effects about cards that would be discarded) and again as cards are discarded,
/// for cards played in between.
fn journey_trigger(g: &Game, player: PlayerId, out: &mut Vec<Trigger>) {
    if let Some(l) = this_turn_lasting(g, Card::Journey, player)
        && g.in_play(player).iter().any(|c| !l.keeps.contains(c))
    {
        out.push(
            Trigger::new(player, Card::Journey, THIS_TURN)
                .via_lasting(l.id)
                .quiet(),
        );
    }
}

fn discard_from_play_triggers(g: &Game, player: PlayerId, out: &mut Vec<Trigger>) {
    if g.cleaning_up(player) {
        journey_trigger(g, player, out);
    }
    let leaving = g.leaving_play(player);
    for &c in &leaving {
        for t in [Card::Tireless, Card::Reckless] {
            if is_traited(g, c, t) {
                out.push(Trigger::new(player, t, TRAIT).on(c));
            }
        }
    }
    if leaving.iter().any(|&c| g.is(c, Types::TREASURE)) {
        for l in g.lasting() {
            if l.card == Card::Trickster && l.owner == player && l.data.n == 0 {
                out.push(
                    Trigger::new(player, Card::Trickster, THIS_TURN)
                        .via_lasting(l.id)
                        .optional(),
                );
            }
        }
    }
}

// ---- Abilities ----

fn start(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        // A Taskmaster's effect watches this turn's gains again.
        let l = if permanent(f.card) || f.card == Card::Taskmaster {
            f.lasting
                .and_then(|id| g.lasting().iter().find(|l| l.id == id).cloned())
        } else {
            f.end_lasting(g)
        };
        let Some(l) = l else {
            return Flow::done();
        };
        f.s = l.data;
        f.pc = 1;
    }
    match f.card {
        Card::Grotto => match f.pc {
            1 => {
                let held: Vec<CardId> = f
                    .s
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| g.zone(c) == Zone::SetAside(p))
                    .collect();
                f.pc = 2;
                Flow::Call(Frame::discard(p, held))
            }
            2 => {
                let n = input.cards().len();
                f.pc = 3;
                Flow::Call(Frame::draw(p, n))
            }
            _ => Flow::done(),
        },
        Card::Siren => {
            if f.pc == 1 {
                f.pc = 2;
                return Flow::Call(Frame::draw_to(p, 8));
            }
            Flow::done()
        }
        Card::Stowaway | Card::Longship | Card::Figurehead => {
            if f.pc == 1 {
                f.pc = 2;
                return Flow::Call(Frame::draw(p, 2));
            }
            Flow::done()
        }
        Card::Taskmaster => {
            plus_actions(g, p, 1);
            coins(g, p, 1);
            // The same effect watches this turn's gains again.
            if let Some(l) = f.lasting.and_then(|id| g.lasting_mut(id)) {
                l.until = Until::EndOfTurn;
                l.keeps.clear();
            }
            Flow::done()
        }
        Card::CabinBoy => cabin_boy(g, f, input),
        Card::Gondola => {
            coins(g, p, 2);
            Flow::done()
        }
        Card::Amphora | Card::BuriedTreasure => {
            plus_buys(g, p, 1);
            coins(g, p, 3);
            Flow::done()
        }
        Card::EndlessChalice => {
            coins(g, p, 1);
            plus_buys(g, p, 1);
            Flow::done()
        }
        Card::Rope => match f.pc {
            1 => {
                f.pc = 2;
                Flow::Call(Frame::draw(p, 1))
            }
            2 => {
                f.pc = 3;
                f.ask_cards(g.hand(p).to_vec(), 0, 1, Purpose::Trash)
            }
            3 => {
                f.pc = 4;
                Flow::Call(Frame::trash(p, input.cards()))
            }
            _ => Flow::done(),
        },
        Card::Crew => {
            let c = f.this();
            if g.zone(c) == Zone::InPlay(p) {
                g.top_deck(c, p);
            }
            Flow::done()
        }
        Card::Jewels => {
            let c = f.this();
            if g.zone(c) == Zone::InPlay(p) {
                g.move_card(c, Zone::Deck(p), Pos::Bottom);
                g.emit(Event::Moved {
                    player: p,
                    card: Some(Card::Jewels),
                    to: Zone::Deck(p),
                });
            }
            Flow::done()
        }
        Card::Enlarge => remodel(g, f, input, 1, 2),
        Card::Quartermaster => quartermaster(g, f, input),
        Card::Hasty => {
            let c = f.this();
            if f.pc == 1 && g.zone(c) == Zone::SetAside(p) {
                f.pc = 2;
                return Flow::Call(Frame::play(p, c, true, vec![]));
            }
            Flow::done()
        }
        Card::Patient | Card::Prepare => play_held(g, f, input),
        _ => Flow::done(),
    }
}

/// Patient and Prepare: play the set-aside cards in any order (Prepare: its Actions and
/// Treasures, then discard the rest). Each is played once: one set aside again by its own
/// play (Encampment, Way of the Turtle) is no longer theirs.
fn play_held(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        1 => {
            let prepare = f.card == Card::Prepare;
            let left: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|&c| g.zone(c) == Zone::SetAside(p))
                .filter(|&c| !prepare || g.is(c, Types::ACTION | Types::TREASURE))
                .collect();
            if left.is_empty() {
                f.pc = 3;
                return Flow::Continue;
            }
            f.pc = 2;
            f.ask_cards(left, 1, 1, Purpose::Play)
        }
        2 => {
            f.pc = 1;
            match input.card() {
                Some(c) => {
                    f.s.cards.retain(|&x| x != c);
                    Flow::Call(Frame::play(p, c, true, vec![]))
                }
                None => Flow::Continue,
            }
        }
        3 => {
            let rest: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|&c| g.zone(c) == Zone::SetAside(p))
                .collect();
            f.pc = 4;
            Flow::Call(Frame::discard(p, rest))
        }
        _ => Flow::done(),
    }
}

fn cabin_boy(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        1 => {
            f.pc = 2;
            f.ask_modes(&["+$2", "Trash this to gain a Duration card"], 1, 1)
        }
        2 => {
            if input.opt() != Some(Opt::Mode(1)) {
                coins(g, p, 2);
                return Flow::done();
            }
            let c = f.this();
            if g.zone(c) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 3;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        3 => {
            let piles = g.supply_piles_where(|t, _| t.contains(Types::DURATION));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 4;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        4 => {
            f.pc = 5;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

/// Whether an effect other than a Quartermaster holds set-aside card `c`: one that set it
/// aside again as it was gained (Hasty, Rapid Expansion, ...), so it is no longer on the
/// Quartermaster.
fn held_elsewhere(g: &Game, c: CardId) -> bool {
    g.lasting().iter().any(|l| {
        l.card != Card::Quartermaster
            && (l.source == Some(c) || l.data.pick == Some(c) || l.data.cards.contains(&c))
    })
}

fn quartermaster(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let this = f.this();
    let held = |g: &Game| -> Vec<CardId> {
        g.lasting()
            .iter()
            .filter(|l| l.card == Card::Quartermaster && l.source == Some(this))
            .flat_map(|l| l.data.cards.iter().copied())
            .filter(|&c| g.zone(c) == Zone::SetAside(p))
            .collect()
    };
    match f.pc {
        1 => {
            f.pc = 2;
            f.ask_modes(
                &[
                    "Gain a card costing up to $4, setting it aside on this",
                    "Put a card from this into your hand",
                ],
                1,
                1,
            )
        }
        2 => {
            if input.opt() == Some(Opt::Mode(1)) {
                let cards = held(g);
                if cards.is_empty() {
                    return Flow::done();
                }
                f.pc = 5;
                return f.ask_cards(cards, 1, 1, Purpose::Move);
            }
            let piles = g.piles_costing_up_to(Cost::coins(4));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::SetAside)),
                None => Flow::done(),
            }
        }
        4 => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::SetAside(p)
                && !held_elsewhere(g, c)
                && let Some(l) = f.lasting.and_then(|id| g.lasting_mut(id))
            {
                l.data.cards.push(c);
            }
            Flow::done()
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::SetAside(p)
            {
                g.put_in_hand(c, p);
                for l in g.lasting.iter_mut() {
                    l.data.cards.retain(|&x| x != c);
                }
            }
            Flow::done()
        }
    }
}

fn waited(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        let Some(l) = f.end_lasting(g) else {
            return Flow::done();
        };
        f.s = l.data;
        f.s.cards.retain(|&c| g.zone(c) == Zone::SetAside(p));
        if f.card == Card::Flagship {
            f.s.pick = match f.occ {
                Some(Occasion::AfterPlay { card, .. }) => Some(card),
                _ => None,
            };
            f.s.player = Some(p);
        }
        f.pc = 1;
    }
    let trash_this = |g: &Game, f: &mut CardFrame, next: u8| -> Flow {
        f.pc = next;
        match f.this {
            Some(c) if g.zone(c) == Zone::InPlay(p) => Flow::Call(Frame::trash(p, vec![c])),
            _ => Flow::Continue,
        }
    };
    match f.card {
        Card::Cage => match f.pc {
            1 => trash_this(g, f, 2),
            _ => {
                let cards = std::mem::take(&mut f.s.cards);
                if !cards.is_empty() {
                    let data = Scratch {
                        cards,
                        ..Scratch::default()
                    };
                    g.add_lasting(Card::Cage, p, None, vec![], Until::EndOfTurn, data);
                }
                Flow::done()
            }
        },
        Card::Search => match f.pc {
            1 => trash_this(g, f, 2),
            2 => {
                f.pc = 3;
                gain_loot(g, p, Dest::Discard)
            }
            _ => Flow::done(),
        },
        Card::SecludedShrine => match f.pc {
            1 => {
                f.pc = 2;
                f.ask_cards(g.hand(p).to_vec(), 0, 2, Purpose::Trash)
            }
            2 => {
                f.pc = 3;
                Flow::Call(Frame::trash(p, input.cards()))
            }
            _ => Flow::done(),
        },
        Card::Abundance => {
            plus_buys(g, p, 1);
            coins(g, p, 3);
            Flow::done()
        }
        Card::Cutthroat => {
            if f.pc == 1 {
                f.pc = 2;
                return gain_loot(g, p, Dest::Discard);
            }
            Flow::done()
        }
        Card::Flagship => {
            if f.pc == 1
                && let Some(c) = f.s.pick
            {
                f.pc = 2;
                return Flow::Call(Frame::play(p, c, false, f.chain_for_child()));
            }
            Flow::done()
        }
        Card::LandingParty => {
            if let Some(c) = f.this
                && g.zone(c) == Zone::InPlay(p)
            {
                g.top_deck(c, p);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn held(g: &mut Game, f: &mut CardFrame) -> Flow {
    let Some(l) = f.end_lasting(g) else {
        return Flow::done();
    };
    for c in l.data.cards {
        if g.zone(c) == Zone::SetAside(l.owner) {
            if l.card == Card::Tireless {
                g.top_deck(c, l.owner);
            } else {
                g.put_in_hand(c, l.owner);
            }
        }
    }
    Flow::done()
}

fn gained(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(Occasion::Gain { player, card, .. }) = f.occ else {
        return Flow::done();
    };
    match f.card {
        Card::BuriedTreasure => {
            if f.pc == 0 && where_gained(g, card) {
                f.pc = 1;
                return Flow::Call(Frame::play(player, card, true, vec![]));
            }
            Flow::done()
        }
        Card::Gondola => play_from_hand(g, f, input, Types::ACTION),
        Card::Siren => match f.pc {
            0 => {
                let actions = g.cards_in_hand_where(player, Types::ACTION);
                if actions.is_empty() {
                    f.pc = 2;
                    return Flow::Continue;
                }
                f.pc = 1;
                f.ask_cards(actions, 0, 1, Purpose::Trash)
            }
            1 => {
                let chosen = input.cards();
                if chosen.is_empty() {
                    f.pc = 2;
                    return Flow::Continue;
                }
                f.pc = 3;
                Flow::Call(Frame::trash(player, chosen))
            }
            2 => {
                f.pc = 3;
                if where_gained(g, card) {
                    return Flow::Call(Frame::trash(player, vec![card]));
                }
                Flow::done()
            }
            _ => Flow::done(),
        },
        Card::Doubloons => once(f, |g, _| gain_basic(g, player, Card::Gold), g),
        Card::WealthyVillage => once(f, |g, _| gain_loot(g, player, Dest::Discard), g),
        Card::Cursed => match f.pc {
            0 => {
                f.pc = 1;
                gain_loot(g, player, Dest::Discard)
            }
            1 => {
                f.pc = 2;
                gain_basic(g, player, Card::Curse)
            }
            _ => Flow::done(),
        },
        Card::Fawning => once(
            f,
            |g, _| match trait_pile(g, Card::Fawning).filter(|&s| g.pile_top(s).is_some()) {
                Some(pile) => Flow::Call(Frame::gain(player, pile, Dest::Discard)),
                None => Flow::done(),
            },
            g,
        ),
        Card::Hasty => {
            if where_gained(g, card) {
                g.set_aside(card, player);
                g.add_lasting(
                    Card::Hasty,
                    player,
                    Some(card),
                    vec![],
                    Until::Removed,
                    Scratch::default(),
                );
            }
            Flow::done()
        }
        Card::Nearby => {
            plus_buys(g, player, 1);
            Flow::done()
        }
        Card::Pious => trash_up_to(g, f, input, 0, 1),
        Card::Rich => once(f, |g, _| gain_basic(g, player, Card::Silver), g),
        _ => Flow::done(),
    }
}

fn react(g: &mut Game, f: &mut CardFrame) -> Flow {
    let c = f.this();
    if f.card == Card::Shield {
        if let Some(c) = f.this {
            g.show(c);
        }
        g.emit(Event::Revealed {
            player: f.player,
            card: Card::Shield,
        });
        return Flow::Done(Value::Bool(true));
    }
    if f.pc == 0 && g.zone(c) == Zone::Hand(f.player) {
        f.pc = 1;
        return Flow::Call(Frame::play(f.player, c, true, vec![]));
    }
    Flow::done()
}

fn this_turn(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let gained = match f.occ {
        Some(Occasion::Gain { card, .. }) => Some(card),
        _ => None,
    };
    match f.card {
        Card::Insignia => {
            if let Some(c) = gained
                && where_gained(g, c)
            {
                g.top_deck(c, p);
            }
            Flow::done()
        }
        Card::MiningRoad => {
            if f.pc == 0
                && let Some(c) = gained
                && where_gained(g, c)
            {
                f.end_lasting(g);
                f.pc = 1;
                return Flow::Call(Frame::play(p, c, true, vec![]));
            }
            Flow::done()
        }
        Card::Deliver => {
            if let Some(c) = gained
                && where_gained(g, c)
            {
                g.set_aside(c, p);
                if let Some(l) = f.lasting.and_then(|id| g.lasting_mut(id)) {
                    l.data.cards.push(c);
                }
            }
            Flow::done()
        }
        Card::Rush | Card::Mirror => {
            let Some(c) = gained else {
                return Flow::done();
            };
            if f.pc == 0 {
                // Every Rush (Mirror) waiting for this gain is used up by it.
                let used: Vec<u32> = g
                    .lasting()
                    .iter()
                    .filter(|l| {
                        l.card == f.card && l.owner == p && next_gain_of(g, l, c, Types::ACTION)
                    })
                    .map(|l| l.id)
                    .collect();
                for id in &used {
                    g.remove_lasting(*id);
                }
                f.s.n = used.len() as i32;
                f.pc = 1;
            }
            if f.card == Card::Rush {
                if f.pc == 1 && where_gained(g, c) {
                    f.pc = 2;
                    return Flow::Call(Frame::play(p, c, true, vec![]));
                }
                return Flow::done();
            }
            if (f.s.i as i32) < f.s.n
                && let Some(pile) = supply_pile_topped_by(g, g.card(c))
            {
                f.s.i += 1;
                return Flow::Call(Frame::gain(p, pile, Dest::Discard));
            }
            Flow::done()
        }
        Card::HarborVillage => {
            let Some(Occasion::AfterPlay { card, .. }) = f.occ else {
                return Flow::done();
            };
            f.end_lasting(g);
            if play_state(g, card).is_some_and(|st| st.gave_coins) {
                coins(g, p, 1);
            }
            Flow::done()
        }
        Card::Frigate => discard_down_one(g, f, input, p, 4),
        Card::Trickster => match f.pc {
            0 => {
                let treasures: Vec<CardId> = g
                    .leaving_play(p)
                    .into_iter()
                    .filter(|&c| g.is(c, Types::TREASURE))
                    .collect();
                if treasures.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_cards(treasures, 1, 1, Purpose::SetAside)
            }
            _ => {
                if let Some(c) = input.card()
                    && g.zone(c) == Zone::InPlay(p)
                {
                    g.set_aside(c, p);
                    if let Some(l) = f.lasting.and_then(|id| g.lasting_mut(id)) {
                        l.data.n = 1;
                        l.data.cards.push(c);
                    }
                }
                Flow::done()
            }
        },
        Card::Avoid => {
            if f.pc == 0 {
                f.end_lasting(g);
                f.pc = 1;
                let mut shuffled = g.shuffled(p).to_vec();
                shuffled.sort();
                return f.ask_cards(shuffled, 0, 3, Purpose::Move);
            }
            for c in input.cards() {
                if g.zone(c) == Zone::Deck(p) {
                    g.move_card(c, Zone::Discard(p), Pos::Top);
                }
            }
            Flow::done()
        }
        Card::Fated => fated(g, f, input),
        Card::Journey => {
            let all = g.in_play(p).to_vec();
            if let Some(l) = f.lasting.and_then(|id| g.lasting_mut(id)) {
                l.keeps = all;
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn shuffled_fated(g: &Game, p: PlayerId) -> Vec<CardId> {
    if trait_pile(g, Card::Fated).is_none() {
        return vec![];
    }
    let mut fated: Vec<CardId> = g
        .shuffled(p)
        .iter()
        .copied()
        .filter(|&c| is_traited(g, c, Card::Fated))
        .collect();
    fated.sort();
    fated
}

/// Fated: put any of the Fated cards just shuffled on top of or under the shuffled cards.
/// Those put under can go among the cards the shuffle put on the bottom (Shadow cards).
fn fated(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let shuffled = g.shuffled(p).to_vec();
    match f.pc {
        0 => {
            let fated = shuffled_fated(g, p);
            let n = fated.len();
            f.s.cards = fated.clone();
            f.pc = 1;
            f.ask_cards(fated, 0, n, Purpose::TopDeck)
        }
        1 => {
            let top = input.cards();
            f.s.cards.retain(|c| !top.contains(c));
            let rest = std::mem::take(&mut f.s.cards);
            f.s.cards = top;
            let n = rest.len();
            f.pc = 2;
            f.ask_cards(rest, 0, n, Purpose::Move)
        }
        2 => {
            let mut bottom = input.cards();
            let top = f.s.cards.len();
            if !bottom.is_empty() {
                // Top first, so the shuffle's bottom cards go under the Fated ones.
                let mut low: Vec<CardId> = shuffled
                    .iter()
                    .copied()
                    .take_while(|&c| super::bottom_when_shuffled(g, p, c))
                    .filter(|c| !bottom.contains(c) && !f.s.cards.contains(c))
                    .collect();
                low.reverse();
                bottom.extend(low);
            }
            f.s.m = bottom.len() as i32;
            f.s.cards.extend(bottom);
            f.pc = 3;
            if top > 1 {
                return f.ask_order(f.s.cards[..top].to_vec(), Purpose::TopDeck);
            }
            Flow::Continue
        }
        3 => {
            let top = f.s.cards.len() - f.s.m as usize;
            if let Input::Response(_) = input {
                f.s.cards.splice(..top, input.cards());
            }
            f.pc = 4;
            let under = f.s.cards[top..].to_vec();
            if under.iter().any(|&c| g.card(c) != g.card(under[0])) {
                return f.ask_order(under, Purpose::Move);
            }
            Flow::Continue
        }
        _ => {
            let top = f.s.cards.len() - f.s.m as usize;
            if let Input::Response(_) = input {
                f.s.cards.splice(top.., input.cards());
            }
            let in_deck = |c: &CardId| g.zone(*c) == Zone::Deck(p);
            let on_top: Vec<CardId> = f.s.cards[..top].iter().copied().filter(in_deck).collect();
            let under: Vec<CardId> = f.s.cards[top..].iter().copied().filter(in_deck).collect();
            for &c in on_top.iter().chain(&under) {
                if is_traited(g, c, Card::Fated) {
                    g.reveal(p, c);
                }
            }
            let rest = shuffled.len() - shuffled.iter().filter(|c| f.s.cards.contains(c)).count();
            f.s.cards.clear();
            let deck = &mut g.players[p.index()].deck;
            deck.retain(|c| !on_top.contains(c) && !under.contains(c));
            let mut new: Vec<CardId> = under.iter().rev().copied().collect();
            new.extend(&deck[..rest]);
            new.extend(on_top.iter().rev());
            new.extend(&deck[rest..]);
            *deck = new;
            Flow::done()
        }
    }
}

fn shaman_gain(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let cards: Vec<CardId> = g
                .trash()
                .iter()
                .copied()
                .filter(|&c| g.cost_of(c).at_most(Cost::coins(6)))
                .collect();
            if cards.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(cards, 1, 1, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.card() {
                Some(c) if g.zone(c) == Zone::Trash => {
                    Flow::Call(Frame::gain_card(f.player, c, Dest::Discard))
                }
                _ => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn trait_ability(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.card {
        Card::Friendly => match f.pc {
            0 => {
                f.pc = 1;
                f.ask_cards(traited_in_hand(g, p, Card::Friendly), 1, 1, Purpose::Discard)
            }
            1 => {
                f.pc = 2;
                Flow::Call(Frame::discard(p, input.cards()))
            }
            2 => {
                f.pc = 3;
                if input.cards().is_empty() {
                    return Flow::done();
                }
                match trait_pile(g, Card::Friendly).filter(|&s| g.pile_top(s).is_some()) {
                    Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                    None => Flow::done(),
                }
            }
            _ => Flow::done(),
        },
        Card::Patient => match f.pc {
            0 => {
                let cards = traited_in_hand(g, p, Card::Patient);
                let n = cards.len();
                f.pc = 1;
                f.ask_cards(cards, 1, n, Purpose::SetAside)
            }
            _ => {
                let cards = input.cards();
                for &c in &cards {
                    g.set_aside(c, p);
                }
                if !cards.is_empty() {
                    let data = Scratch {
                        cards,
                        ..Scratch::default()
                    };
                    g.add_lasting(Card::Patient, p, None, vec![], Until::Removed, data);
                }
                Flow::done()
            }
        },
        Card::Shy => match f.pc {
            0 => {
                f.pc = 1;
                f.ask_cards(traited_in_hand(g, p, Card::Shy), 1, 1, Purpose::Discard)
            }
            1 => {
                f.pc = 2;
                Flow::Call(Frame::discard(p, input.cards()))
            }
            2 => {
                f.pc = 3;
                if input.cards().is_empty() {
                    return Flow::done();
                }
                Flow::Call(Frame::draw(p, 2))
            }
            _ => Flow::done(),
        },
        Card::Tireless => {
            let c = f.this();
            if g.zone(c) == Zone::InPlay(p) {
                g.set_aside(c, p);
                let data = Scratch {
                    cards: vec![c],
                    ..Scratch::default()
                };
                g.add_lasting(Card::Tireless, p, None, vec![], Until::EndOfTurn, data);
            }
            Flow::done()
        }
        Card::Reckless => {
            let c = f.this();
            if g.zone(c) == Zone::InPlay(p) {
                g.return_to_pile(c);
            }
            Flow::done()
        }
        Card::Inspiring => match f.pc {
            0 => {
                let names: Vec<Card> = g.in_play(p).iter().map(|&c| g.card(c)).collect();
                let actions: Vec<CardId> = g
                    .playable(p, Types::ACTION)
                    .into_iter()
                    .filter(|&c| !names.contains(&g.card(c)))
                    .collect();
                if actions.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_cards(actions, 1, 1, Purpose::Play)
            }
            1 => {
                f.pc = 2;
                match input.card() {
                    Some(c) => Flow::Call(Frame::play(p, c, true, vec![])),
                    None => Flow::done(),
                }
            }
            _ => Flow::done(),
        },
        _ => Flow::done(),
    }
}

/// Inherited: before the first turn, the player chooses which starting card the Inherited
/// card replaces (an Estate unless they choose otherwise), then draws a new hand.
fn inherited(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let pile = trait_pile(g, Card::Inherited);
    let starting: Vec<CardId> = g
        .player(p)
        .owned()
        .filter(|&c| g.home_pile(g.printed(c)) != pile)
        .collect();
    let has_shelters = starting
        .iter()
        .any(|&c| g.printed(c).is(Types::SHELTER));
    let default = if has_shelters {
        Card::Hovel
    } else {
        Card::Estate
    };
    match f.pc {
        0 => {
            for c in g.hand(p).to_vec() {
                g.move_card(c, Zone::Deck(p), Pos::Top);
            }
            let mut kinds = vec![default];
            for &c in &starting {
                let k = g.printed(c);
                if !kinds.contains(&k) {
                    kinds.push(k);
                }
            }
            let labels = kinds.iter().map(|k| k.to_string()).collect();
            let options = kinds.into_iter().map(Opt::Card).collect();
            f.pc = 1;
            f.ask_choose_for(p, options, labels, 1, 1, Purpose::Other)
        }
        _ => {
            if let Some(Opt::Card(k)) = input.opt()
                && k != default
                && let Some(&x) = starting.iter().find(|&&c| g.printed(c) == k)
            {
                if k == Card::Copper && g.return_to_pile(x) {
                    g.create(default, Zone::Deck(p));
                } else {
                    // The chosen card goes back to the box and the default one comes back.
                    let from = g.cards[x.index()];
                    g.cards[x.index()] = default;
                    g.record(|| Trace::Changed {
                        card: x,
                        from,
                        to: default,
                    });
                }
            }
            let mut deck = std::mem::take(&mut g.players[p.index()].deck);
            g.shuffle(&mut deck);
            let mixed = deck.clone();
            g.record(|| Trace::Mixed { cards: mixed });
            g.players[p.index()].deck = deck;
            for _ in 0..5 {
                let Some(&top) = g.players[p.index()].deck.last() else {
                    break;
                };
                g.move_card(top, Zone::Hand(p), Pos::Top);
            }
            Flow::done()
        }
    }
}

// ---- Kingdom cards ----

/// Trash `min..=max` cards from hand.
fn trash_up_to(g: &mut Game, f: &mut CardFrame, input: Input, min: usize, max: usize) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, min, max, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            f.pc = 2;
            if chosen.is_empty() {
                return Flow::done();
            }
            Flow::Call(Frame::trash(f.player, chosen))
        }
        _ => Flow::done(),
    }
}

/// From `base`: gain a card costing up to `limit`.
fn gain_up_to(g: &mut Game, f: &mut CardFrame, input: Input, base: u8, limit: Cost, to: Dest) -> Flow {
    match f.pc - base {
        0 => {
            let piles = g.piles_costing_up_to(limit);
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = base + 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = base + 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, to)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

/// From `base`: trash a card from hand and gain one costing up to $`extra` more.
fn remodel(g: &mut Game, f: &mut CardFrame, input: Input, base: u8, extra: u8) -> Flow {
    match f.pc - base {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = base + 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.cost = g.cost_of(c);
            f.pc = base + 2;
            Flow::Call(Frame::trash(f.player, vec![c]))
        }
        _ => {
            let limit = f.s.cost.plus_coins(extra);
            gain_up_to(g, f, input, base + 2, limit, Dest::Discard)
        }
    }
}

/// Each target discards down to `n` cards in hand.
fn discard_down(g: &mut Game, f: &mut CardFrame, input: Input, n: usize) -> Flow {
    match f.pc {
        0 => {
            while let Some(&target) = f.targets.get(f.s.i) {
                let hand = g.hand(target).to_vec();
                if hand.len() > n {
                    let k = hand.len() - n;
                    f.pc = 1;
                    return f.ask_cards_for(target, hand, k, k, Purpose::Discard);
                }
                f.s.i += 1;
            }
            Flow::done()
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()))
        }
        _ => {
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
    }
}

/// `target` discards down to `n` cards in hand.
fn discard_down_one(g: &mut Game, f: &mut CardFrame, input: Input, target: PlayerId, n: usize) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(target).to_vec();
            if hand.len() <= n {
                return Flow::done();
            }
            let k = hand.len() - n;
            f.pc = 1;
            f.ask_cards_for(target, hand, k, k, Purpose::Discard)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(target, input.cards()))
        }
        _ => Flow::done(),
    }
}

/// You may play a card of `types` from your hand.
fn play_from_hand(g: &mut Game, f: &mut CardFrame, input: Input, types: Types) -> Flow {
    match f.pc {
        0 => {
            let cards = g.playable(f.player, types);
            if cards.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            let min = usize::from(f.hook != Hook::Main);
            f.ask_cards(cards, min, 1, Purpose::Play)
        }
        1 => {
            f.pc = 2;
            match input.card() {
                Some(c) if g.playable(f.player, types).contains(&c) => {
                    Flow::Call(Frame::play(f.player, c, true, vec![]))
                }
                _ => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn cage(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            f.pc = 1;
            f.ask_cards(hand, 0, 4, Purpose::SetAside)
        }
        _ => {
            let cards = input.cards();
            for &c in &cards {
                set_aside_hidden(g, c, p);
            }
            let data = Scratch {
                cards,
                ..stamped(g)
            };
            f.add_duration(g, data);
            Flow::done()
        }
    }
}

fn grotto(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 0, 4, Purpose::SetAside)
        }
        _ => {
            let cards = input.cards();
            if !cards.is_empty() {
                for &c in &cards {
                    set_aside_hidden(g, c, p);
                }
                let data = Scratch {
                    cards,
                    ..stamped(g)
                };
                f.add_duration(g, data);
            }
            Flow::done()
        }
    }
}

fn siren(g: &mut Game, f: &mut CardFrame) -> Flow {
    if each_target_gains(g, f, Card::Curse) {
        let curse = g.supply_pile(Card::Curse).expect("checked");
        return Flow::Call(Frame::gain(f.targets[f.s.i - 1], curse, Dest::Discard));
    }
    duration(g, f);
    Flow::done()
}

fn crucible(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.cost = g.cost_of(c);
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, vec![c]))
        }
        _ => {
            coins(g, f.player, f.s.cost.coins as u32);
            Flow::done()
        }
    }
}

fn fortune_hunter(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 3))
        }
        1 => {
            let cards = input.cards();
            for &c in &cards {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
            }
            let treasures: Vec<CardId> = cards
                .iter()
                .copied()
                .filter(|&c| g.is(c, Types::TREASURE))
                .collect();
            f.s.cards = cards;
            f.pc = 2;
            f.ask_cards(treasures, 0, 1, Purpose::Play)
        }
        2 => {
            f.pc = 3;
            match input.card() {
                Some(c) => Flow::Call(Frame::play(p, c, true, vec![])),
                None => Flow::Continue,
            }
        }
        3 => {
            let rest: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|&c| g.zone(c) == Zone::SetAside(p))
                .collect();
            f.pc = 4;
            f.ask_order(rest, Purpose::TopDeck)
        }
        _ => {
            for &c in input.cards().iter().rev() {
                if g.zone(c) == Zone::SetAside(p) {
                    g.move_card(c, Zone::Deck(p), Pos::Top);
                }
            }
            Flow::done()
        }
    }
}

/// Gondola and Amphora: their bonus now, or at the start of the next turn.
fn now_or_next(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        f.pc = 1;
        return f.ask_either(&["Now", "At the start of your next turn"]);
    }
    if input.opt() == Some(Opt::Mode(1)) {
        duration(g, f);
    } else if f.card == Card::Gondola {
        coins(g, f.player, 2);
    } else {
        plus_buys(g, f.player, 1);
        coins(g, f.player, 3);
    }
    Flow::done()
}

fn mapmaker(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 4))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            f.ask_cards(f.s.cards.clone(), 2, 2, Purpose::Move)
        }
        2 => {
            let chosen = input.cards();
            for &c in &chosen {
                if g.zone(c) == Zone::Deck(p) {
                    g.put_in_hand(c, p);
                }
            }
            let rest: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|&c| g.zone(c) == Zone::Deck(p))
                .collect();
            f.pc = 3;
            Flow::Call(Frame::discard(p, rest))
        }
        _ => Flow::done(),
    }
}

fn maroon(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.n = g.types(c).names().count() as i32;
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, vec![c]))
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::draw(f.player, 2 * f.s.n as usize))
        }
        _ => Flow::done(),
    }
}

fn tools(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let in_play: Vec<Card> = (0..g.num_players())
                .flat_map(|p| g.in_play(PlayerId(p as u8)).iter().map(|&c| g.card(c)))
                .collect();
            let piles: Vec<PileId> = (0..g.piles().len())
                .map(|i| PileId(i as u8))
                .filter(|&id| {
                    g.pile(id).in_supply
                        && g.pile_top(id).is_some_and(|t| in_play.contains(&g.card(t)))
                })
                .collect();
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn figurine(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.cards_in_hand_where(p, Types::ACTION);
            if actions.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        _ => {
            if !input.cards().is_empty() {
                plus_buys(g, p, 1);
                coins(g, p, 1);
            }
            Flow::done()
        }
    }
}

fn first_mate(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let name = f.s.card;
            let actions: Vec<CardId> = g
                .playable(p, Types::ACTION)
                .into_iter()
                .filter(|&c| name.is_none_or(|n| g.card(c) == n))
                .collect();
            f.pc = 1;
            if actions.is_empty() {
                f.pc = 2;
                return Flow::Continue;
            }
            f.ask_cards(actions, 0, 1, Purpose::Play)
        }
        1 => match input.card() {
            Some(c) => {
                f.s.card.get_or_insert(g.card(c));
                f.pc = 0;
                Flow::Call(Frame::play(p, c, true, vec![]))
            }
            None => {
                f.pc = 2;
                Flow::Continue
            }
        },
        2 => {
            f.pc = 3;
            Flow::Call(Frame::draw_to(p, 6))
        }
        _ => Flow::done(),
    }
}

fn pickaxe(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.cost = g.cost_of(c);
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, vec![c]))
        }
        2 => {
            f.pc = 3;
            if f.s.cost.coins >= 3 {
                return gain_loot(g, f.player, Dest::Hand);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn pilgrim(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::TopDeck)
        }
        _ => {
            for c in input.cards() {
                g.move_card(c, Zone::Deck(f.player), Pos::Top);
                g.emit(Event::Moved {
                    player: f.player,
                    card: None,
                    to: Zone::Deck(f.player),
                });
            }
            Flow::done()
        }
    }
}

fn silver_mine(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let limit = g.cost_of(f.this());
            let piles =
                g.supply_piles_where(|t, cost| t.contains(Types::TREASURE) && cost.less_than(limit));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Hand)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn kings_cache(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let treasures = g.playable(p, Types::TREASURE);
            if treasures.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(treasures, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.s.n = 1;
            f.pc = 2;
            Flow::Call(Frame::play(p, c, true, f.chain_for_child()))
        }
        _ => {
            if f.s.n < 3 {
                f.s.n += 1;
                let c = f.s.pick.expect("set in step 1");
                return Flow::Call(Frame::play(p, c, false, f.chain_for_child()));
            }
            Flow::done()
        }
    }
}

// ---- Loot ----

fn orb(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            for c in g.player(p).discard.clone() {
                g.show_to(c, p);
            }
            f.pc = 1;
            f.ask_modes(
                &["Play an Action or Treasure from your discard pile", "+1 Buy and +$3"],
                1,
                1,
            )
        }
        1 => {
            f.s.m = mode_mask(input);
            if f.s.m & 1 == 0 {
                f.pc = 3;
                return Flow::Continue;
            }
            let cards: Vec<CardId> = g
                .player(p)
                .discard
                .iter()
                .copied()
                .filter(|&c| g.is(c, Types::ACTION | Types::TREASURE))
                .collect();
            f.pc = 3;
            if cards.is_empty() {
                return Flow::Continue;
            }
            f.pc = 2;
            f.ask_cards(cards, 1, 1, Purpose::Play)
        }
        2 => {
            f.pc = 3;
            match input.card() {
                Some(c) if g.zone(c) == Zone::Discard(p) => {
                    Flow::Call(Frame::play(p, c, true, vec![]))
                }
                _ => Flow::Continue,
            }
        }
        3 => {
            f.pc = 4;
            if f.s.m & 2 != 0 {
                plus_buys(g, p, 1);
                coins(g, p, 3);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn puzzle_box(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 0, 1, Purpose::SetAside)
        }
        _ => {
            let cards = input.cards();
            if !cards.is_empty() {
                for &c in &cards {
                    set_aside_hidden(g, c, p);
                }
                let data = Scratch {
                    cards,
                    ..Scratch::default()
                };
                g.add_lasting(Card::PuzzleBox, p, f.this, vec![], Until::EndOfTurn, data);
            }
            Flow::done()
        }
    }
}

fn sextant(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 5))
        }
        1 => {
            let cards = input.cards();
            for &c in &cards {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
            }
            let n = cards.len();
            f.s.cards = cards.clone();
            f.pc = 2;
            f.ask_cards(cards, 0, n, Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(p, input.cards()))
        }
        3 => {
            let rest: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|&c| g.zone(c) == Zone::SetAside(p))
                .collect();
            f.pc = 4;
            f.ask_order(rest, Purpose::TopDeck)
        }
        _ => {
            for &c in input.cards().iter().rev() {
                if g.zone(c) == Zone::SetAside(p) {
                    g.move_card(c, Zone::Deck(p), Pos::Top);
                }
            }
            Flow::done()
        }
    }
}

fn spell_scroll(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let this = f.this();
    match f.pc {
        0 => {
            if g.zone(this) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.s.cost = g.cost_of(this);
            f.pc = 1;
            Flow::Call(Frame::trash(p, vec![this]))
        }
        1 => {
            let limit = f.s.cost;
            let piles = g.supply_piles_where(|_, cost| cost.less_than(limit));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        2 => {
            f.pc = 3;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        3 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            if g.zone(c) != Zone::Discard(p) || !g.is(c, Types::ACTION | Types::TREASURE) {
                return Flow::done();
            }
            f.s.pick = Some(c);
            f.pc = 4;
            f.ask_yes_no(Purpose::Play)
        }
        4 => {
            f.pc = 5;
            let c = f.s.pick.expect("set in step 3");
            if input.is_yes() && g.zone(c) == Zone::Discard(p) {
                return Flow::Call(Frame::play(p, c, true, vec![]));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

// ---- Events ----

fn bury(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        plus_buys(g, p, 1);
        let discard = g.player(p).discard.clone();
        if discard.is_empty() {
            return Flow::done();
        }
        f.pc = 1;
        return f.ask_cards(discard, 1, 1, Purpose::Move);
    }
    for c in input.cards() {
        if g.zone(c) == Zone::Discard(p) {
            g.move_card(c, Zone::Deck(p), Pos::Bottom);
            g.emit(Event::Moved {
                player: p,
                card: None,
                to: Zone::Deck(p),
            });
        }
    }
    Flow::done()
}

fn peril(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.cards_in_hand_where(p, Types::ACTION);
            if actions.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(p, chosen))
        }
        2 => {
            f.pc = 3;
            gain_loot(g, p, Dest::Discard)
        }
        _ => Flow::done(),
    }
}

fn foray(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            f.pc = 1;
            f.ask_cards(hand, 3, 3, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            for &c in &chosen {
                g.reveal(p, c);
            }
            let mut names: Vec<Card> = chosen.iter().map(|&c| g.card(c)).collect();
            names.sort();
            names.dedup();
            f.s.n = names.len() as i32;
            f.s.m = chosen.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        2 => {
            f.pc = 3;
            if f.s.m == 3 && f.s.n == 3 {
                return gain_loot(g, p, Dest::Discard);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn launch(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            g.add_lasting(
                Card::Launch,
                p,
                None,
                vec![],
                Until::EndOfTurn,
                Scratch::default(),
            );
            if p != g.turn.player || g.turn.phase != Phase::Buy {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::return_to_action(p))
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::draw(p, 1))
        }
        _ => {
            g.turn.actions += 1;
            g.turn.buys += 1;
            Flow::done()
        }
    }
}

fn scrounge(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Trash a card from your hand",
                    "Gain an Estate from the trash, and if you did, gain a card costing up to $5",
                ],
                1,
                1,
            )
        }
        1 => {
            if input.opt() != Some(Opt::Mode(1)) {
                let hand = g.hand(p).to_vec();
                if hand.is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                return f.ask_cards(hand, 1, 1, Purpose::Trash);
            }
            let estate = g.trash().iter().copied().find(|&c| g.card(c) == Card::Estate);
            match estate {
                Some(c) => {
                    f.pc = 4;
                    Flow::Call(Frame::gain_card(p, c, Dest::Discard))
                }
                None => Flow::done(),
            }
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        3 => Flow::done(),
        4 => {
            if input.cards().is_empty() {
                return Flow::done();
            }
            f.pc = 5;
            Flow::Continue
        }
        _ => gain_up_to(g, f, input, 5, Cost::coins(5), Dest::Discard),
    }
}

fn maelstrom(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            f.pc = 1;
            f.ask_cards(hand, 3, 3, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        2 => {
            let others = g.others(p);
            while let Some(&other) = others.get(f.s.i) {
                let hand = g.hand(other).to_vec();
                if hand.len() >= 5 {
                    f.pc = 3;
                    return f.ask_cards_for(other, hand, 1, 1, Purpose::Trash);
                }
                f.s.i += 1;
            }
            Flow::done()
        }
        3 => {
            let other = g.others(p)[f.s.i];
            f.pc = 4;
            Flow::Call(Frame::trash(other, input.cards()))
        }
        _ => {
            f.s.i += 1;
            f.pc = 2;
            Flow::Continue
        }
    }
}

fn invasion(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let attacks = g.playable(p, Types::ATTACK);
            f.pc = 2;
            if attacks.is_empty() {
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(attacks, 0, 1, Purpose::Play)
        }
        1 => {
            f.pc = 2;
            match input.card() {
                Some(c) => Flow::Call(Frame::play(p, c, true, vec![])),
                None => Flow::Continue,
            }
        }
        2 => {
            f.pc = 3;
            gain_basic(g, p, Card::Duchy)
        }
        3 => {
            let piles = g.supply_piles_where(|t, _| t.contains(Types::ACTION));
            f.pc = 5;
            if piles.is_empty() {
                return Flow::Continue;
            }
            f.pc = 4;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        4 => {
            f.pc = 5;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::DeckTop)),
                None => Flow::Continue,
            }
        }
        5 => {
            f.pc = 6;
            gain_loot(g, p, Dest::Discard)
        }
        6 => {
            f.pc = 7;
            match input.card() {
                Some(c) if g.zone(c) == Zone::Discard(p) => {
                    Flow::Call(Frame::play(p, c, true, vec![]))
                }
                _ => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn prosper(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            gain_loot(g, p, Dest::Discard)
        }
        1 => {
            let names: Vec<Card> = f.s.cards.iter().map(|&c| g.printed(c)).collect();
            let piles: Vec<PileId> = g
                .supply_piles_where(|t, _| t.contains(Types::TREASURE))
                .into_iter()
                .filter(|&id| {
                    let top = g.pile_top(id).expect("non-empty");
                    !names.contains(&g.card(top))
                })
                .collect();
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_pile(piles, true, Purpose::Gain)
        }
        2 => {
            let Some(pile) = input.pile() else {
                return Flow::done();
            };
            f.pc = 3;
            Flow::Call(Frame::gain(p, pile, Dest::Discard))
        }
        _ => {
            f.s.cards.extend(input.cards());
            f.pc = 1;
            Flow::Continue
        }
    }
}
