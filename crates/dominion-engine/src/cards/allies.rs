//! Allies: its Kingdom cards, the six split piles that rotate, Liaisons, Favors and the
//! 23 Allies.
//!
//! Favors are [`crate::Player::favors`]. Each Ally ability that spends Favors is an
//! optional trigger, offered only while its owner has enough of them; accepting it
//! spends them. Durations follow Seaside's pattern: a `START` ability resolves the
//! lasting at the start of the owner's next turn. "This turn" effects (Bauble, Galleria,
//! Guildmaster, Skirmisher, Garrison's tokens) are `Until::EndOfTurn` lastings; the
//! players an Attack affects are a bit mask in the lasting's `data.n`.

use super::{
    CardFrame, Hook, SetRules, due_durations, duration_trigger, looking, mode_mask, put_back,
    take_mode,
};
use crate::decision::{Decision, DecisionKind, Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input};
use crate::game::{ExtraLimit, ExtraTurn, Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup, add_mixed_pile};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Set, Types};
use crate::Note;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Family of Inventors' Favor tokens: how many are on each Supply pile.
    pub inventors: Vec<(PileId, u32)>,
}

pub(crate) struct Rules;

/// A Duration's start-of-turn effect.
const START: u8 = 0;
const HIGHWAYMAN_BLOCK: u8 = 1;
const GARRISON_TOKEN: u8 = 1;
/// A card's or Ally's triggered ability ("when you gain this", an Ally's timing, ...).
const REACT: u8 = 0;
const COASTAL_HAVEN_RETURN: u8 = 1;

/// A helper's `next` that finishes the ability instead of continuing.
const DONE: u8 = u8::MAX;
const CAVE_DWELLERS: &str = "discard a card, then draw a card";
const DESERT_GUIDES: &str = "discard your hand and draw 5 cards";

/// The split piles, each with its cards in pile order (top first), 4 copies each.
const SPLIT_PILES: [(Card, [Card; 4]); 6] = [
    (
        Card::Augurs,
        [Card::HerbGatherer, Card::Acolyte, Card::Sorceress, Card::Sibyl],
    ),
    (
        Card::Clashes,
        [Card::BattlePlan, Card::Archer, Card::Warlord, Card::Territory],
    ),
    (
        Card::Forts,
        [Card::Tent, Card::Garrison, Card::HillFort, Card::Stronghold],
    ),
    (
        Card::Odysseys,
        [
            Card::OldMap,
            Card::Voyage,
            Card::SunkenTreasure,
            Card::DistantShore,
        ],
    ),
    (
        Card::Townsfolk,
        [Card::TownCrier, Card::Blacksmith, Card::Miller, Card::Elder],
    ),
    (
        Card::Wizards,
        [Card::Student, Card::Conjurer, Card::Sorcerer, Card::Lich],
    ),
];

/// The cards whose lastings resolve as a Duration at the start of the next turn.
const DURATIONS: [Card; 8] = [
    Card::Importer,
    Card::Highwayman,
    Card::Warlord,
    Card::Contract,
    Card::RoyalGalley,
    Card::Conjurer,
    Card::Stronghold,
    Card::Garrison,
];

impl SetRules for Rules {
    fn notes(g: &Game, out: &mut Vec<Note>) {
        for &(pile, n) in &g.ext.allies.inventors {
            out.push(Note::new(Card::FamilyOfInventors).pile(pile).count(n as i32));
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (c, Hook::Ability(START)) if DURATIONS.contains(&c) => start(g, f, input),
            (Card::Highwayman, Hook::Ability(HIGHWAYMAN_BLOCK)) => {
                // Resolved after a replacement (Enlightenment, Enchantress), it overrides
                // it: the player picks which applies by ordering them.
                let c = f.this();
                if let Some(st) = g.plays.last_mut()
                    && st.card == c
                {
                    st.blocked = true;
                    st.replaced = None;
                }
                Flow::done()
            }
            (Card::Garrison, Hook::Ability(GARRISON_TOKEN)) => {
                if let Some(w) = f.lasting {
                    garrison_token(g, w);
                }
                Flow::done()
            }
            (Card::CoastalHaven, Hook::Ability(COASTAL_HAVEN_RETURN)) => {
                if let Some(l) = f.end_lasting(g) {
                    for c in l.data.cards {
                        if g.zone(c) == Zone::SetAside(f.player) {
                            g.put_in_hand(c, f.player);
                        }
                    }
                }
                Flow::done()
            }
            (c, Hook::Ability(REACT)) if c.kind() == Kind::Ally => ally(g, f, input),
            (Card::Sycophant, Hook::Ability(REACT)) => {
                g.players[f.player.index()].favors += 2;
                Flow::done()
            }
            (Card::MerchantCamp | Card::Tent, Hook::Ability(REACT)) => {
                let c = f.this();
                if g.zone(c) == Zone::InPlay(f.player) {
                    g.top_deck(c, f.player);
                }
                Flow::done()
            }
            (Card::Territory, Hook::Ability(REACT)) => {
                if f.pc == 0 {
                    f.pc = 1;
                    f.s.m = g.empty_supply_piles() as i32;
                }
                gain_copies(g, f, Card::Gold, f.s.m, Dest::Discard)
            }
            (Card::Lich, Hook::Ability(REACT)) => lich_trashed(g, f, input),
            (Card::Galleria, Hook::Ability(REACT)) => {
                plus_buys(g, f.player, 1);
                Flow::done()
            }
            (Card::Guildmaster, Hook::Ability(REACT)) => {
                g.players[f.player.index()].favors += 1;
                Flow::done()
            }
            (Card::Skirmisher, Hook::Ability(REACT)) => {
                let mask = g
                    .lasting()
                    .iter()
                    .find(|l| Some(l.id) == f.lasting)
                    .map_or(0, |l| l.data.n);
                if f.pc == 0 {
                    f.targets = g
                        .others_in_turn_order(f.player)
                        .into_iter()
                        .filter(|&p| in_mask(mask, p))
                        .collect();
                    f.pc = 1;
                }
                discard_down(g, f, input, 3, 1, DONE)
            }
            (Card::Bauble, Hook::Ability(REACT)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ
                    && g.still_where_gained(card)
                {
                    g.show(card);
                    g.top_deck(card, f.player);
                }
                Flow::done()
            }
            (_, Hook::Ability(_)) => return None,
            (Card::Acolyte, _) => acolyte(g, f, input),
            (Card::Archer, _) => archer(g, f, input),
            (Card::Barbarian, _) => barbarian(g, f, input),
            (Card::BattlePlan, _) => battle_plan(g, f, input),
            (Card::Bauble, _) => bauble(g, f, input),
            (Card::Blacksmith, _) => blacksmith(g, f, input),
            (Card::Broker, _) => broker(g, f, input),
            (Card::CapitalCity, _) => capital_city(g, f, input),
            (Card::Carpenter, _) => carpenter(g, f, input),
            (Card::Conjurer, _) => {
                if f.pc < 3 {
                    return Some(gain_step(g, f, input, &up_to(4), Dest::Discard, 0, 3));
                }
                if g.zone(f.this()) == Zone::InPlay(f.player) {
                    f.add_duration(g, Scratch::default());
                }
                Flow::done()
            }
            (Card::Contract, _) => contract(g, f, input),
            (Card::Courier, _) => courier(g, f, input),
            (Card::DistantShore, _) => gain_copies(g, f, Card::Estate, 1, Dest::Discard),
            (Card::Elder, _) => elder(g, f, input),
            (Card::Emissary, _) => {
                emissary(g, f);
                Flow::done()
            }
            (Card::Galleria | Card::Guildmaster, _) => {
                g.add_lasting(
                    f.card,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::Garrison, _) => {
                let data = Scratch {
                    cards: f.keeps(g),
                    ..Scratch::default()
                };
                g.add_lasting(
                    Card::Garrison,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    data,
                );
                Flow::done()
            }
            (Card::HerbGatherer, _) => herb_gatherer(g, f, input),
            (Card::Highwayman | Card::Warlord, _) => {
                let data = Scratch {
                    n: mask(&f.targets),
                    ..Scratch::default()
                };
                f.add_duration(g, data);
                Flow::done()
            }
            (Card::HillFort, _) => hill_fort(g, f, input),
            (Card::Hunter, _) => hunter(g, f, input),
            (Card::Importer, _) => {
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Innkeeper, _) => innkeeper(g, f, input),
            (Card::Lich, _) => {
                g.players[f.player.index()].skip_turns += 1;
                Flow::done()
            }
            (Card::Marquis, _) => marquis(g, f, input),
            (Card::Miller, _) => miller(g, f, input),
            (Card::Modify, _) => modify(g, f, input),
            (Card::OldMap, _) => old_map(g, f, input),
            (Card::RoyalGalley, _) => royal_galley(g, f, input),
            (Card::Sentinel, _) => sentinel(g, f, input),
            (Card::Sibyl, _) => sibyl(g, f, input),
            (Card::Skirmisher, _) => {
                let data = Scratch {
                    n: mask(&f.targets),
                    ..Scratch::default()
                };
                g.add_lasting(
                    Card::Skirmisher,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    data,
                );
                Flow::done()
            }
            (Card::Sorcerer, _) => sorcerer(g, f, input),
            (Card::Sorceress, _) => sorceress(g, f, input),
            (Card::Specialist, _) => specialist(g, f, input),
            (Card::Stronghold, _) => stronghold(g, f, input),
            (Card::Student, _) => student(g, f, input),
            (Card::SunkenTreasure, _) => sunken_treasure(g, f, input),
            (Card::Swap, _) => swap(g, f, input),
            (Card::Sycophant, _) => sycophant(g, f, input),
            (Card::Tent, _) => may_rotate(g, f, input, Card::Forts, 0, DONE),
            (Card::Town, _) => town(g, f, input),
            (Card::TownCrier, _) => town_crier(g, f, input),
            (Card::Voyage, _) => {
                voyage(g, f);
                Flow::done()
            }
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in due_durations(g, occ) {
            if DURATIONS.contains(&l.card) {
                out.push(duration_trigger(l, START));
            }
        }
        match *occ {
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            Occasion::Trash { player, card } => match g.card(card) {
                Card::Sycophant => out.push(Trigger::new(player, Card::Sycophant, REACT).on(card)),
                Card::Lich => out.push(Trigger::new(player, Card::Lich, REACT).on(card)),
                _ => {}
            },
            Occasion::Play { player, card } => {
                if g.is(card, Types::TREASURE)
                    && highwayman_attacks(g, player)
                    && !g.plays_before(player, card).any(|(_, t)| t.contains(Types::TREASURE))
                {
                    out.push(Trigger::new(player, Card::Highwayman, HIGHWAYMAN_BLOCK).on(card));
                }
            }
            Occasion::DiscardFromPlay { player } => {
                for c in g.leaving_play(player) {
                    let kind = g.card(c);
                    if matches!(kind, Card::MerchantCamp | Card::Tent) {
                        out.push(Trigger::new(player, kind, REACT).on(c).optional("put it onto your deck"));
                    }
                }
            }
            Occasion::EndTurn { player } => {
                for l in g.lasting() {
                    if l.card == Card::CoastalHaven && l.owner == player {
                        out.push(
                            Trigger::new(player, Card::CoastalHaven, COASTAL_HAVEN_RETURN)
                                .via_lasting(l.id),
                        );
                    }
                }
            }
            _ => {}
        }
        for ally in allies(g) {
            if let Some(t) = ally_trigger(g, ally, occ) {
                out.push(t);
            }
        }
    }

    fn modify_cost(g: &Game, card: Card, id: Option<CardId>, cost: &mut Cost) {
        let tokens = &g.ext.allies.inventors;
        if tokens.is_empty() {
            return;
        }
        let pile = match id.map(|c| g.zone(c)) {
            Some(Zone::Pile(pile)) => Some(pile),
            _ => g.home_pile(card),
        };
        if let Some(&(_, n)) = tokens.iter().find(|(p, _)| Some(*p) == pile) {
            *cost = cost.minus_coins(n.min(u8::MAX as u32) as u8);
        }
    }

    fn modify_decision(g: &Game, f: &CardFrame, d: &mut Decision) {
        if d.purpose != Purpose::Mode || f.hook != Hook::Main || d.player != f.player {
            return;
        }
        let Some(this) = f.this else {
            return;
        };
        // Only Elder's own play of it, not a replay (Royal Carriage) during that play.
        let play = g.turn.played_ids.iter().rposition(|&c| c == this);
        let elder = g.lasting().iter().any(|l| {
            l.card == Card::Elder
                && l.until == Until::Removed
                && l.data.pick == Some(this)
                && Some(l.data.i) == play
        });
        if let DecisionKind::Choose { options, max, .. } = &mut d.kind
            && elder
        {
            *max = (*max + 1).min(options.len());
        }
    }

    fn can_play(g: &Game, p: PlayerId, id: CardId) -> bool {
        let t = g.turn();
        if t.extra == Some(Card::Voyage) && t.player == p && t.hand_plays >= 3 {
            return false;
        }
        if g.is(id, Types::ACTION) && warlord_attacks(g, p) {
            let card = g.card(id);
            let copies = g.in_play(p).iter().filter(|&&c| g.card(c) == card).count();
            return copies < 2;
        }
        true
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::Territory).then(|| {
            let mut names: Vec<Card> = g
                .player(p)
                .owned()
                .filter(|&c| g.is(c, Types::VICTORY))
                .map(|c| g.card(c))
                .collect();
            names.sort();
            names.dedup();
            names.len() as i32
        })
    }

    fn bonus_vp(g: &Game, p: PlayerId) -> i32 {
        if !g.has_landscape(Card::PlateauShepherds) {
            return 0;
        }
        let twos = g
            .player(p)
            .owned()
            .filter(|&c| g.cost_of(c) == Cost::coins(2))
            .count() as u32;
        2 * twos.min(g.player(p).favors) as i32
    }

    fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
        let Some((_, members)) = SPLIT_PILES.iter().find(|(name, _)| *name == card) else {
            return false;
        };
        let cards: Vec<Card> = members.iter().flat_map(|&c| [c; 4]).collect();
        add_mixed_pile(g, card, &cards, true);
        true
    }

    fn ensure(g: &mut Game, _s: &mut Setup) {
        let liaison = setup::in_game(g, |c| c.is(Types::LIAISON));
        if liaison && allies(g).next().is_none() {
            add_random_ally(g);
        }
    }

    fn setup(g: &mut Game, s: &mut Setup) {
        let mut favors = 0;
        if s.joined_cards().iter().any(|c| c.kind() == Kind::Ally) {
            favors += 1;
        }
        if s.joined(Card::Importer) {
            favors += 4;
        }
        for pl in &mut g.players {
            pl.favors += favors;
        }
    }
}

// ---- Helpers ----

fn add_random_ally(g: &mut Game) {
    let all: Vec<Card> = Card::in_set(Set::Allies)
        .filter(|c| c.kind() == Kind::Ally)
        .collect();
    let ally = all[g.random_index(all.len())];
    g.landscapes.push(ally);
}

/// Continues the ability at `next`, or finishes it if `next` is [`DONE`].
fn goto(f: &mut CardFrame, next: u8) -> Flow {
    if next == DONE {
        return Flow::done();
    }
    f.pc = next;
    Flow::Continue
}

/// The Allies in this game.
fn allies(g: &Game) -> impl Iterator<Item = Card> + '_ {
    g.landscapes().iter().copied().filter(|c| c.kind() == Kind::Ally)
}

fn mask(players: &[PlayerId]) -> i32 {
    players.iter().fold(0, |m, p| m | 1 << p.0)
}

fn in_mask(mask: i32, p: PlayerId) -> bool {
    mask & (1 << p.0) != 0
}

fn plus_actions(g: &mut Game, p: PlayerId, n: u32) {
    g.add_actions(p, n);
}

fn plus_buys(g: &mut Game, p: PlayerId, n: u32) {
    g.add_buys(p, n);
}

fn favors(g: &Game, p: PlayerId) -> u32 {
    g.player(p).favors
}

fn spend(g: &mut Game, p: PlayerId, n: u32) {
    let pl = &mut g.players[p.index()];
    pl.favors = pl.favors.saturating_sub(n);
}

fn up_to(coins: u8) -> impl Fn(&Game) -> Vec<PileId> {
    move |g| g.piles_costing_up_to(Cost::coins(coins))
}

fn dest_zone(to: Dest, p: PlayerId) -> Zone {
    match to {
        Dest::Discard => Zone::Discard(p),
        Dest::Hand => Zone::Hand(p),
        Dest::DeckTop => Zone::Deck(p),
        Dest::SetAside => Zone::SetAside(p),
    }
}

/// Where `p` gained `card`: the first such gain logged since `since`.
fn gained_to(g: &Game, p: PlayerId, card: CardId, since: usize) -> Option<Zone> {
    let kind = g.card(card);
    g.log()[since..].iter().find_map(|e| match e {
        Event::Gained { player, card, to } if *player == p && *card == kind => {
            Some(dest_zone(*to, p))
        }
        _ => None,
    })
}

/// What a card being gained cost when it was gained.
fn gain_cost(g: &Game, card: CardId) -> Cost {
    g.gains
        .iter()
        .rev()
        .find(|st| st.card == card)
        .map_or_else(|| g.cost_of(card), |st| st.cost)
}

/// The Supply pile whose top card is `card`.
fn supply_pile_topped_by(g: &Game, card: Card) -> Option<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&id| g.pile(id).in_supply && g.pile_top(id).is_some_and(|t| g.card(t) == card))
}

/// " (top: Sorceress)": the card on top of the pile named `card`, for a label.
fn top_named(g: &Game, card: Card) -> String {
    g.pile_of(card)
        .and_then(|p| g.pile_top(p))
        .map(|c| format!(" (top: {})", g.card(c)))
        .unwrap_or_default()
}

fn top_of(g: &Game, card: Card) -> Option<PileId> {
    g.pile_of(card).filter(|&p| g.pile_top(p).is_some())
}

/// Puts `id` on top of `p`'s deck without revealing it.
fn top_deck_hidden(g: &mut Game, id: CardId, p: PlayerId) {
    g.move_card(id, Zone::Deck(p), Pos::Top);
}

/// Whether rotating `pile` would change it: it holds a card other than its top run.
fn rotatable(g: &Game, pile: PileId) -> bool {
    let cards = &g.pile(pile).cards;
    cards
        .last()
        .is_some_and(|&top| cards.iter().any(|&c| g.card(c) != g.card(top)))
}

/// Moves a pile's top card and the copies of it directly under it to its bottom.
pub(crate) fn rotate(g: &mut Game, p: PlayerId, pile: PileId) {
    let cards = &g.piles[pile.index()].cards;
    let Some(&top) = cards.last() else {
        return;
    };
    let top = g.card(top);
    let run = cards.iter().rev().take_while(|&&c| g.card(c) == top).count();
    let cards = &mut g.piles[pile.index()].cards;
    let moved = cards.split_off(cards.len() - run);
    cards.splice(0..0, moved);
    g.emit(Event::Rotated { player: p, pile });
}

/// "You may rotate the `name`" at pc `base` (asks) and `base + 1`, then `next`.
fn may_rotate(g: &mut Game, f: &mut CardFrame, input: Input, name: Card, base: u8, next: u8) -> Flow {
    let pile = g.pile_of(name).filter(|&p| rotatable(g, p));
    if f.pc == base {
        if pile.is_none() {
            return goto(f, next);
        }
        f.pc = base + 1;
        return f.ask_yes_no(Purpose::Other, format!("Rotate the {name}"));
    }
    if input.is_yes()
        && let Some(pile) = pile
    {
        rotate(g, f.player, pile);
    }
    goto(f, next)
}

/// Choose one of `piles(g)` and gain its top card to `dest`. Runs at pcs `base..base + 3`,
/// then continues at `next`.
fn gain_step(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    piles: &dyn Fn(&Game) -> Vec<PileId>,
    dest: Dest,
    base: u8,
    next: u8,
) -> Flow {
    match f.pc - base {
        0 => {
            let piles = piles(g);
            if piles.is_empty() {
                return goto(f, next);
            }
            f.pc = base + 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => match input.pile() {
            Some(pile) => {
                f.pc = base + 2;
                Flow::Call(Frame::gain(f.player, pile, dest))
            }
            None => goto(f, next),
        },
        _ => goto(f, next),
    }
}

/// Gain `n` copies of `card` from its pile, one at a time (counting in `f.s.n`); done after.
fn gain_copies(g: &mut Game, f: &mut CardFrame, card: Card, n: i32, dest: Dest) -> Flow {
    if f.s.n >= n {
        return Flow::done();
    }
    f.s.n += 1;
    match top_of(g, card) {
        Some(pile) => Flow::Call(Frame::gain(f.player, pile, dest)),
        None => Flow::done(),
    }
}

/// Each of `f.targets` (from `f.s.i`) discards down to `n` cards, in turn order. Runs at
/// pcs `base` and `base + 1`, then continues at `next`.
fn discard_down(g: &mut Game, f: &mut CardFrame, input: Input, n: usize, base: u8, next: u8) -> Flow {
    if f.pc == base + 1 {
        let target = f.targets[f.s.i];
        f.s.i += 1;
        f.pc = base;
        return Flow::Call(Frame::discard(target, input.cards()));
    }
    while let Some(&target) = f.targets.get(f.s.i) {
        let hand = g.hand(target).to_vec();
        if hand.len() > n {
            let k = hand.len() - n;
            f.pc = base + 1;
            return f.ask_cards_for(target, hand, k, k, Purpose::Discard);
        }
        f.s.i += 1;
    }
    f.s.i = 0;
    goto(f, next)
}

/// Each of `f.targets` gains a Curse, in turn order (counting in `f.s.i`); then `next`.
fn each_target_gains_curse(g: &mut Game, f: &mut CardFrame, next: u8) -> Flow {
    if let Some(&target) = f.targets.get(f.s.i) {
        f.s.i += 1;
        if let Some(pile) = top_of(g, Card::Curse) {
            return Flow::Call(Frame::gain(target, pile, Dest::Discard));
        }
        return Flow::Continue;
    }
    f.s.i = 0;
    goto(f, next)
}

/// At step `pc`, draws `n` cards and finishes.
fn draw_then_done(f: &mut CardFrame, pc: u8, n: usize) -> Flow {
    if f.pc == pc {
        f.pc = pc + 1;
        return Flow::Call(Frame::draw(f.player, n));
    }
    Flow::done()
}

/// A Highwayman's or Warlord's Attack on `p` is on: played by another player whose
/// next turn hasn't started yet.
fn attack_on(g: &Game, card: Card, p: PlayerId) -> bool {
    g.lasting().iter().any(|l| {
        l.card == card
            && l.until == Until::Removed
            && l.owner != p
            && in_mask(l.data.n, p)
            && g.before_next_turn(l)
    })
}

fn highwayman_attacks(g: &Game, p: PlayerId) -> bool {
    attack_on(g, Card::Highwayman, p)
}

fn warlord_attacks(g: &Game, p: PlayerId) -> bool {
    attack_on(g, Card::Warlord, p)
}

/// Adds a token to the Garrison whose lasting `watcher` watches this turn's gains.
fn garrison_token(g: &mut Game, watcher: u32) {
    let Some(w) = g.lasting().iter().find(|l| l.id == watcher).cloned() else {
        return;
    };
    let tokens = (w.data.n > 0)
        .then(|| (w.data.n - 1) as u32)
        .filter(|&t| g.lasting().iter().any(|l| l.id == t));
    match tokens {
        Some(t) => g.lasting_mut(t).expect("checked").data.n += 1,
        None => {
            let held = Scratch {
                n: 1,
                ..Scratch::default()
            };
            let t = g.continue_lasting(&w, w.data.cards.clone(), Until::Removed, held);
            g.lasting_mut(w.id).expect("watcher").data.n = t as i32 + 1;
        }
    }
}

fn gain_triggers(g: &Game, player: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let kind = g.card(card);
    if kind == Card::Sycophant {
        out.push(Trigger::new(player, Card::Sycophant, REACT).on(card));
    }
    if kind == Card::Territory {
        out.push(Trigger::new(player, Card::Territory, REACT).on(card));
    }
    let this_turn = |c: Card| {
        g.lasting()
            .iter()
            .filter(move |l| l.card == c && l.owner == player && l.until == Until::EndOfTurn)
    };
    let cost = gain_cost(g, card);
    if (3..=4).contains(&cost.coins) && cost.potions == 0 && cost.debt == 0 {
        for l in this_turn(Card::Galleria) {
            out.push(Trigger::new(player, Card::Galleria, REACT).via_lasting(l.id).quiet());
        }
    }
    for l in this_turn(Card::Guildmaster) {
        out.push(Trigger::new(player, Card::Guildmaster, REACT).via_lasting(l.id).quiet());
    }
    if g.is(card, Types::ATTACK) {
        for l in this_turn(Card::Skirmisher) {
            out.push(Trigger::new(player, Card::Skirmisher, REACT).via_lasting(l.id));
        }
    }
    for l in this_turn(Card::Garrison) {
        out.push(Trigger::new(player, Card::Garrison, GARRISON_TOKEN).via_lasting(l.id).quiet());
    }
    if let Some(l) = this_turn(Card::Bauble).next()
        && g.still_where_gained(card)
        && g.zone(card) != Zone::Deck(player)
    {
        out.push(
            Trigger::new(player, Card::Bauble, REACT)
                .via_lasting(l.id)
                .optional(format!("put {} onto your deck", g.card(card))),
        );
    }
}

// ---- Allies ----

/// The trigger `ally` offers for `occ`, if any.
fn ally_trigger(g: &Game, ally: Card, occ: &Occasion) -> Option<Trigger> {
    let (player, ok) = match *occ {
        Occasion::Gain { player, card, .. } => {
            let n = favors(g, player);
            let ok = match ally {
                Card::ArchitectsGuild => n >= 2 && !architects_piles(g, card).is_empty(),
                Card::BandOfNomads => n >= 1 && gain_cost(g, card).coins >= 3,
                Card::CityState => {
                    n >= 2
                        && player == g.turn.player
                        && g.is(card, Types::ACTION)
                        && g.still_where_gained(card)
                        && (g.zone(card) != Zone::Hand(player) || super::can_play(g, player, card))
                }
                Card::TrappersLodge => {
                    n >= 1 && g.still_where_gained(card) && g.zone(card) != Zone::Deck(player)
                }
                _ => false,
            };
            (player, ok)
        }
        Occasion::StartTurn { player } => {
            let n = favors(g, player);
            let ok = match ally {
                Card::CaveDwellers | Card::DesertGuides | Card::ForestDwellers => n >= 1,
                Card::CraftersGuild => n >= 2,
                Card::MountainFolk => n >= 5,
                Card::GangOfPickpockets => {
                    return Some(Trigger::new(player, ally, REACT));
                }
                _ => false,
            };
            (player, ok)
        }
        Occasion::StartBuyPhase { player } => {
            let n = favors(g, player);
            let ok = match ally {
                Card::FamilyOfInventors => n >= 1 && !inventors_piles(g).is_empty(),
                Card::LeagueOfBankers if n >= 4 => {
                    return Some(Trigger::new(player, ally, REACT));
                }
                Card::MarketTowns => n >= 1 && !g.playable(player, Types::ACTION).is_empty(),
                Card::PeacefulCult => n >= 1 && !g.hand(player).is_empty(),
                Card::WoodworkersGuild => {
                    n >= 1 && !g.cards_in_hand_where(player, Types::ACTION).is_empty()
                }
                _ => false,
            };
            (player, ok)
        }
        Occasion::AfterPlay { player, card } => {
            let n = favors(g, player);
            let liaison = g.is(card, Types::LIAISON);
            let ok = match ally {
                Card::CircleOfWitches => liaison && n >= 3,
                Card::FellowshipOfScribes => {
                    g.is(card, Types::ACTION) && g.hand(player).len() <= 4 && n >= 1
                }
                Card::LeagueOfShopkeepers if liaison && n >= 5 => {
                    return Some(Trigger::new(player, ally, REACT));
                }
                _ => false,
            };
            (player, ok)
        }
        Occasion::DiscardFromPlay { player } if ally == Card::CoastalHaven => (
            player,
            g.cleaning_up(player) && favors(g, player) >= 1 && !g.hand(player).is_empty(),
        ),
        Occasion::EndTurn { player } if ally == Card::IslandFolk => (
            player,
            favors(g, player) >= 5 && player == g.turn.player && !g.third_in_a_row(player),
        ),
        Occasion::Shuffled { player, .. }
            if matches!(ally, Card::OrderOfAstrologers | Card::OrderOfMasons) =>
        {
            (player, favors(g, player) >= 1)
        }
        _ => return None,
    };
    ok.then(|| Trigger::new(player, ally, REACT).optional(ally_does(ally)))
}

/// What using an optional Ally's ability does.
fn ally_does(ally: Card) -> String {
    match ally {
        Card::ArchitectsGuild => "spend 2 Favors to gain a cheaper non-Victory card".into(),
        Card::BandOfNomads => "spend a Favor for +1 Card, +1 Action or +1 Buy".into(),
        Card::CaveDwellers => format!("spend a Favor to {CAVE_DWELLERS}"),
        Card::CircleOfWitches => "spend 3 Favors so each other player gains a Curse".into(),
        Card::CityState => "spend 2 Favors to play the gained card".into(),
        Card::CoastalHaven => "spend Favors to keep that many cards in hand".into(),
        Card::CraftersGuild => "spend 2 Favors to gain a card costing up to $4 onto your deck".into(),
        Card::DesertGuides => format!("spend a Favor to {DESERT_GUIDES}"),
        Card::FamilyOfInventors => "put a Favor on a non-Victory Supply pile".into(),
        Card::FellowshipOfScribes => "spend a Favor for +1 Card".into(),
        Card::ForestDwellers => {
            "spend a Favor to look at your top 3 cards, discard any and put the rest back".into()
        }
        Card::IslandFolk => "spend 5 Favors to take an extra turn after this one".into(),
        Card::MarketTowns => "spend a Favor to play an Action card from your hand".into(),
        Card::MountainFolk => "spend 5 Favors for +3 Cards".into(),
        Card::OrderOfAstrologers => "spend Favors to pick a card per Favor to go on top".into(),
        Card::OrderOfMasons => {
            "spend Favors to put up to 2 cards per Favor into your discard pile".into()
        }
        Card::PeacefulCult => "spend Favors to trash that many cards from your hand".into(),
        Card::TrappersLodge => "spend a Favor to put the gained card onto your deck".into(),
        Card::WoodworkersGuild => {
            "spend a Favor to trash an Action card from your hand and gain an Action card".into()
        }
        _ => "use it".into(),
    }
}

/// The piles Architects' Guild could gain from for gaining `card`.
fn architects_piles(g: &Game, card: CardId) -> Vec<PileId> {
    let limit = g.cost_of(card);
    g.supply_piles_where(|types, cost| cost.less_than(limit) && !types.intersects(Types::VICTORY))
}

/// The non-Victory Supply piles (by their randomizer or card) Family of Inventors can use.
fn inventors_piles(g: &Game) -> Vec<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .filter(|&id| g.pile(id).in_supply && !g.pile(id).card.is(Types::VICTORY))
        .collect()
}

fn ally(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.card {
        Card::ArchitectsGuild => {
            let Some(Occasion::Gain { card, .. }) = f.occ else {
                return Flow::done();
            };
            match f.pc {
                0 => {
                    let piles = architects_piles(g, card);
                    if piles.is_empty() {
                        return Flow::done();
                    }
                    spend(g, p, 2);
                    f.pc = 1;
                    f.ask_pile(piles, false, Purpose::Gain)
                }
                1 => {
                    f.pc = 2;
                    match input.pile() {
                        Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                        None => Flow::done(),
                    }
                }
                _ => Flow::done(),
            }
        }
        Card::BandOfNomads => match f.pc {
            0 => {
                spend(g, p, 1);
                f.pc = 1;
                f.ask_either(&["+1 Card", "+1 Action", "+1 Buy"])
            }
            1 => {
                f.pc = 2;
                match input.opt() {
                    Some(Opt::Mode(0)) => Flow::Call(Frame::draw(p, 1)),
                    Some(Opt::Mode(1)) => {
                        plus_actions(g, p, 1);
                        Flow::done()
                    }
                    _ => {
                        plus_buys(g, p, 1);
                        Flow::done()
                    }
                }
            }
            _ => Flow::done(),
        },
        Card::CityState => {
            let Some(Occasion::Gain { card, .. }) = f.occ else {
                return Flow::done();
            };
            if f.pc == 0 && g.still_where_gained(card) {
                spend(g, p, 2);
                f.pc = 1;
                return Flow::Call(Frame::play(p, card, true, vec![]));
            }
            Flow::done()
        }
        Card::TrappersLodge => {
            if let Some(Occasion::Gain { card, .. }) = f.occ
                && g.still_where_gained(card)
            {
                spend(g, p, 1);
                g.show(card);
                g.top_deck(card, p);
            }
            Flow::done()
        }
        Card::CaveDwellers => match f.pc {
            0 => {
                spend(g, p, 1);
                let hand = g.hand(p).to_vec();
                f.pc = 1;
                f.ask_cards(hand, 1, 1, Purpose::Discard)
            }
            1 => {
                f.pc = 2;
                Flow::Call(Frame::discard(p, input.cards()))
            }
            2 => {
                f.pc = 3;
                Flow::Call(Frame::draw(p, 1))
            }
            3 => again(g, f, CAVE_DWELLERS),
            _ => {
                if input.is_yes() {
                    f.pc = 0;
                    return Flow::Continue;
                }
                Flow::done()
            }
        },
        Card::DesertGuides => match f.pc {
            0 => {
                spend(g, p, 1);
                f.pc = 1;
                Flow::Call(Frame::discard(p, g.hand(p).to_vec()))
            }
            1 => {
                f.pc = 2;
                Flow::Call(Frame::draw(p, 5))
            }
            2 => again(g, f, DESERT_GUIDES),
            _ => {
                if input.is_yes() {
                    f.pc = 0;
                    return Flow::Continue;
                }
                Flow::done()
            }
        },
        Card::CraftersGuild => {
            if f.pc == 0 {
                spend(g, p, 2);
                f.pc = 1;
            }
            gain_step(g, f, input, &up_to(4), Dest::DeckTop, 1, DONE)
        }
        Card::ForestDwellers => match f.pc {
            0 => {
                spend(g, p, 1);
                f.pc = 1;
                Flow::Call(Frame::look(p, 3))
            }
            1 => {
                f.s.cards = input.cards();
                f.pc = 2;
                let n = f.s.cards.len();
                f.ask_cards(f.s.cards.clone(), 0, n, Purpose::Discard)
            }
            2 => {
                let chosen = input.cards();
                f.s.cards.retain(|c| !chosen.contains(c));
                f.pc = 3;
                Flow::Call(Frame::discard(p, chosen))
            }
            3 => {
                f.s.cards = looking(g, p, &f.s.cards);
                f.pc = 4;
                f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
            }
            _ => {
                put_back(g, p, &input.cards());
                Flow::done()
            }
        },
        Card::GangOfPickpockets => match f.pc {
            0 => {
                if g.hand(p).len() <= 4 {
                    return Flow::done();
                }
                f.pc = 1;
                if favors(g, p) == 0 {
                    return Flow::Continue;
                }
                f.ask_yes_or(Purpose::Other, "Spend a Favor to keep your hand", "Discard down to 4 cards")
            }
            1 => {
                if input.is_yes() {
                    spend(g, p, 1);
                    return Flow::done();
                }
                f.targets = vec![p];
                f.pc = 2;
                Flow::Continue
            }
            _ => discard_down(g, f, input, 4, 2, DONE),
        },
        Card::MountainFolk => {
            if f.pc == 0 {
                spend(g, p, 5);
            }
            draw_then_done(f, 0, 3)
        }
        Card::FamilyOfInventors => match f.pc {
            0 => {
                let piles = inventors_piles(g);
                if piles.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_pile(piles, false, Purpose::Other)
                    .question("choose a non-Victory Supply pile for a Favor token")
            }
            _ => {
                if let Some(pile) = input.pile() {
                    spend(g, p, 1);
                    let tokens = &mut g.ext.allies.inventors;
                    match tokens.iter_mut().find(|(id, _)| *id == pile) {
                        Some((_, n)) => *n += 1,
                        None => tokens.push((pile, 1)),
                    }
                }
                Flow::done()
            }
        },
        Card::LeagueOfBankers => {
            g.add_coins(p, favors(g, p) / 4);
            Flow::done()
        }
        Card::MarketTowns => match f.pc {
            0 => {
                let actions = g.playable(p, Types::ACTION);
                if actions.is_empty() || favors(g, p) == 0 {
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_cards(actions, 0, 1, Purpose::Play)
            }
            1 => {
                let Some(c) = input.card() else {
                    return Flow::done();
                };
                spend(g, p, 1);
                f.pc = 0;
                Flow::Call(Frame::play(p, c, true, vec![]))
            }
            _ => Flow::done(),
        },
        Card::PeacefulCult => match f.pc {
            0 => {
                let n = favors(g, p) as usize;
                f.pc = 1;
                f.ask_cards(g.hand(p).to_vec(), 0, n, Purpose::Trash)
            }
            1 => {
                let chosen = input.cards();
                spend(g, p, chosen.len() as u32);
                f.pc = 2;
                Flow::Call(Frame::trash(p, chosen))
            }
            _ => Flow::done(),
        },
        Card::WoodworkersGuild => match f.pc {
            0 => {
                let actions = g.cards_in_hand_where(p, Types::ACTION);
                if actions.is_empty() {
                    return Flow::done();
                }
                spend(g, p, 1);
                f.pc = 1;
                f.ask_cards(actions, 1, 1, Purpose::Trash)
            }
            1 => {
                let chosen = input.cards();
                if chosen.is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                Flow::Call(Frame::trash(p, chosen))
            }
            _ => gain_step(
                g,
                f,
                input,
                &|g| g.supply_piles_where(|t, _| t.intersects(Types::ACTION)),
                Dest::Discard,
                2,
                DONE,
            ),
        },
        Card::CircleOfWitches => {
            if f.pc == 0 {
                spend(g, p, 3);
                f.targets = g.others_in_turn_order(p);
                f.pc = 1;
            }
            each_target_gains_curse(g, f, DONE)
        }
        Card::FellowshipOfScribes => {
            if f.pc == 0 {
                spend(g, p, 1);
            }
            draw_then_done(f, 0, 1)
        }
        Card::LeagueOfShopkeepers => {
            let n = favors(g, p);
            if n >= 5 {
                g.add_coins(p, 1);
            }
            if n >= 10 {
                plus_actions(g, p, 1);
                plus_buys(g, p, 1);
            }
            Flow::done()
        }
        Card::CoastalHaven => match f.pc {
            0 => {
                let hand = g.hand(p).to_vec();
                let n = favors(g, p) as usize;
                f.pc = 1;
                f.ask_cards(hand, 0, n, Purpose::SetAside)
            }
            _ => {
                let kept = input.cards();
                if kept.is_empty() {
                    return Flow::done();
                }
                spend(g, p, kept.len() as u32);
                for &c in &kept {
                    g.move_card(c, Zone::SetAside(p), Pos::Top);
                }
                let data = Scratch {
                    cards: kept,
                    ..Scratch::default()
                };
                g.add_lasting(Card::CoastalHaven, p, None, vec![], Until::Removed, data);
                Flow::done()
            }
        },
        Card::IslandFolk => {
            spend(g, p, 5);
            g.extra_turns.push_front(ExtraTurn {
                player: p,
                source: Card::IslandFolk,
                limit: ExtraLimit::ThirdInARow,
            });
            Flow::done()
        }
        Card::OrderOfAstrologers | Card::OrderOfMasons => order(g, f, input),
        _ => Flow::done(),
    }
}

/// Asks whether to spend another Favor and repeat ("Repeat as desired").
fn again(g: &Game, f: &mut CardFrame, what: &str) -> Flow {
    if favors(g, f.player) == 0 {
        return Flow::done();
    }
    f.pc = 4;
    f.ask_yes_no(Purpose::Other, format!("Spend another Favor to {what}"))
}

/// Order of Astrologers (cards onto the shuffled cards) and Order of Masons (cards
/// left out of the shuffle, into the discard pile).
fn order(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let astrologers = f.card == Card::OrderOfAstrologers;
    match f.pc {
        0 => {
            let mut shuffled = g.shuffled(p).to_vec();
            shuffled.sort();
            g.look_through_shuffled(p);
            let above = g.look_over_shuffled(p);
            let n = favors(g, p) as usize;
            let max = if astrologers { n } else { 2 * n };
            f.pc = 1;
            let (purpose, what) = if astrologers {
                (Purpose::TopDeck, "put on top of your deck, a Favor each")
            } else {
                (Purpose::Move, "put into your discard pile, a Favor per 2 cards")
            };
            f.ask_cards(shuffled, 0, max, purpose)
                .question(what)
                .about(above)
        }
        1 => {
            let chosen = input.cards();
            let k = chosen.len() as u32;
            spend(g, p, if astrologers { k } else { k.div_ceil(2) }.max(1));
            if !astrologers {
                for c in chosen {
                    g.show_to(c, p);
                    g.move_card(c, Zone::Discard(p), Pos::Top);
                }
                g.reshuffle_unplaced(p, &[]);
                return Flow::done();
            }
            f.pc = 2;
            f.ask_order(chosen, Purpose::TopDeck)
        }
        _ => {
            let chosen = input.cards();
            let base = g.shuffled(p).len();
            let deck = &mut g.players[p.index()].deck;
            let rest: Vec<CardId> = deck[..base]
                .iter()
                .copied()
                .filter(|c| !chosen.contains(c))
                .collect();
            let mut new: Vec<CardId> = rest;
            new.extend(chosen.iter().rev());
            new.extend(deck[base..].iter().copied());
            *deck = new;
            g.reshuffle_unplaced(p, &chosen);
            Flow::done()
        }
    }
}

// ---- Start of turn ----

fn start(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        let Some(l) = f.end_lasting(g) else {
            return Flow::done();
        };
        f.s = l.data;
        f.pc = 1;
    }
    let p = f.player;
    match f.card {
        Card::Importer => gain_step(g, f, input, &up_to(5), Dest::Discard, 1, DONE),
        Card::Highwayman => match f.pc {
            1 => {
                f.pc = 2;
                let this = f.this();
                if g.zone(this) == Zone::InPlay(p) {
                    return Flow::Call(Frame::discard_from_play(p, vec![this]));
                }
                Flow::Continue
            }
            2 => {
                f.pc = 3;
                Flow::Call(Frame::draw(p, 3))
            }
            _ => Flow::done(),
        },
        Card::Warlord => draw_then_done(f, 1, 2),
        Card::Stronghold => draw_then_done(f, 1, 3),
        Card::Garrison => {
            if f.pc == 1 && g.zone(f.this()) != Zone::InPlay(p) {
                return Flow::done();
            }
            draw_then_done(f, 1, f.s.n.max(0) as usize)
        }
        Card::Contract => match f.s.pick {
            Some(c) if f.pc == 1 && g.zone(c) == Zone::SetAside(p) => {
                f.pc = 2;
                Flow::Call(Frame::play(p, c, true, vec![]))
            }
            _ => Flow::done(),
        },
        Card::RoyalGalley => match f.s.pick {
            Some(c) if f.pc == 1 && g.zone(c) == Zone::InPlay(p) => {
                f.pc = 2;
                Flow::Call(Frame::play(p, c, false, vec![]))
            }
            _ => Flow::done(),
        },
        Card::Conjurer => {
            let this = f.this();
            if g.zone(this) == Zone::InPlay(p) {
                g.put_in_hand(this, p);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

// ---- Kingdom cards ----

fn bauble(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        f.pc = 1;
        return f.ask_modes(
            &[
                "+1 Buy",
                "+$1",
                "+1 Favor",
                "This turn, when you gain a card, you may put it onto your deck",
            ],
            2,
            2,
        );
    }
    let p = f.player;
    f.s.m = mode_mask(input);
    while let Some(m) = take_mode(f) {
        match m {
            0 => plus_buys(g, p, 1),
            1 => g.add_coins(p, 1),
            2 => g.players[p.index()].favors += 1,
            _ => {
                g.add_lasting(
                    Card::Bauble,
                    p,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
            }
        }
    }
    Flow::done()
}

fn sycophant(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            let n = hand.len().min(3);
            if n == 0 {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        _ => {
            if f.s.n > 0 {
                g.add_coins(p, 3);
            }
            Flow::done()
        }
    }
}

fn sentinel(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 5))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            f.ask_cards(f.s.cards.clone(), 0, 2, Purpose::Trash)
        }
        2 => {
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = 3;
            Flow::Call(Frame::trash(p, chosen))
        }
        3 => {
            f.s.cards = looking(g, p, &f.s.cards);
            f.pc = 4;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        _ => {
            put_back(g, p, &input.cards());
            Flow::done()
        }
    }
}

fn broker(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                f.pc = 2;
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            f.s.pick = chosen.first().copied();
            f.pc = 2;
            Flow::Call(Frame::trash(p, chosen))
        }
        2 => {
            f.pc = 3;
            let n = broker_count(g, f);
            f.ask_modes(
                &[
                    format!("+1 Card per $1 it costs (+{n})"),
                    format!("+1 Action per $1 it costs (+{n})"),
                    format!("+$1 per $1 it costs (+${n})"),
                    format!("+1 Favor per $1 it costs (+{n})"),
                ],
                1,
                1,
            )
        }
        3 => {
            f.s.m = mode_mask(input);
            f.pc = 4;
            Flow::Continue
        }
        _ => {
            while let Some(m) = take_mode(f) {
                let n = broker_count(g, f);
                match m {
                    0 => return Flow::Call(Frame::draw(p, n as usize)),
                    1 => plus_actions(g, p, n),
                    2 => g.add_coins(p, n),
                    _ => g.players[p.index()].favors += n,
                }
            }
            Flow::done()
        }
    }
}

/// What Broker's trashed card costs now, in $.
fn broker_count(g: &Game, f: &CardFrame) -> u32 {
    f.s.pick.map_or(0, |c| u32::from(g.cost_of(c).coins))
}

fn carpenter(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.empty_supply_piles() == 0 {
                plus_actions(g, p, 1);
                f.pc = 10;
                return Flow::Continue;
            }
            let hand = g.hand(p).to_vec();
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
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2..=4 => {
            let limit = g.cost_of(f.s.pick.expect("set in step 1")).plus_coins(2);
            gain_step(
                g,
                f,
                input,
                &move |g| g.piles_costing_up_to(limit),
                Dest::Discard,
                2,
                DONE,
            )
        }
        _ => gain_step(g, f, input, &up_to(4), Dest::Discard, 10, DONE),
    }
}

fn courier(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 1))
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(p, input.cards()))
        }
        2 => {
            g.look_through_discard(p);
            let cards: Vec<CardId> = g
                .player(p)
                .discard
                .iter()
                .copied()
                .filter(|&c| g.is(c, Types::ACTION | Types::TREASURE))
                .collect();
            f.pc = 3;
            f.ask_cards(cards, 0, 1, Purpose::Play)
        }
        3 => {
            f.pc = 4;
            match input.card() {
                Some(c) => Flow::Call(Frame::play(p, c, true, vec![])),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn innkeeper(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "+1 Card",
                    "+3 Cards, then discard 3 cards",
                    "+5 Cards, then discard 6 cards",
                ],
                1,
                1,
            )
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            Flow::Continue
        }
        2 => match take_mode(f) {
            None => Flow::done(),
            Some(m) => {
                let (draw, discard) = [(1, 0), (3, 3), (5, 6)][m as usize];
                f.s.n = discard;
                f.pc = 3;
                Flow::Call(Frame::draw(p, draw))
            }
        },
        3 => {
            let hand = g.hand(p).to_vec();
            let n = (f.s.n as usize).min(hand.len());
            f.pc = 4;
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        _ => {
            f.pc = 2;
            Flow::Call(Frame::discard(p, input.cards()))
        }
    }
}

fn royal_galley(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions: Vec<CardId> = g
                .playable(p, Types::ACTION)
                .into_iter()
                .filter(|&c| !g.is(c, Types::DURATION))
                .collect();
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::play(p, c, true, f.chain_for_child()))
        }
        _ => {
            let c = f.s.pick.expect("set in step 1");
            if g.zone(c) == Zone::InPlay(p) {
                let mut keeps = f.keeps(g);
                keeps.push(c);
                let data = Scratch {
                    pick: Some(c),
                    ..Scratch::default()
                };
                g.add_lasting(Card::RoyalGalley, p, f.this, keeps, Until::Removed, data);
            }
            Flow::done()
        }
    }
}

fn town(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["+1 Card and +2 Actions", "+1 Buy and +$2"], 1, 1)
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            if f.s.m & 1 != 0 {
                plus_actions(g, p, 2);
                return Flow::Call(Frame::draw(p, 1));
            }
            Flow::Continue
        }
        _ => {
            if f.s.m & 2 != 0 {
                plus_buys(g, p, 1);
                g.add_coins(p, 2);
            }
            Flow::done()
        }
    }
}

fn barbarian(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(&target) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(target, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 4;
                return Flow::Continue;
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::trash(target, vec![c]))
        }
        2 => {
            let c = f.s.pick.expect("set in step 1");
            let limit = g.cost_of(c);
            if limit.coins < 3 {
                f.pc = 4;
                return Flow::Continue;
            }
            let types = g.types(c);
            let piles = g.supply_piles_where(|t, cost| cost.less_than(limit) && t.intersects(types));
            if piles.is_empty() {
                f.pc = 5;
                return Flow::Continue;
            }
            f.pc = 3;
            f.ask_pile_for(target, piles, false, Purpose::Gain)
        }
        3 => {
            f.pc = 5;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(target, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        4 => {
            f.pc = 5;
            match top_of(g, Card::Curse) {
                Some(pile) => Flow::Call(Frame::gain(target, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        _ => {
            f.s.i += 1;
            f.s.pick = None;
            f.pc = 0;
            Flow::Continue
        }
    }
}

fn capital_city(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 4;
            if g.hand(p).is_empty() {
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Discard, "Discard 2 cards for +$2")
        }
        1 => {
            if !input.is_yes() {
                f.pc = 4;
                return Flow::Continue;
            }
            let hand = g.hand(p).to_vec();
            let n = hand.len().min(2);
            f.pc = 2;
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        2 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc = 3;
            Flow::Call(Frame::discard(p, chosen))
        }
        3 => {
            if f.s.n == 2 {
                g.add_coins(p, 2);
            }
            f.pc = 4;
            Flow::Continue
        }
        4 => {
            if p != g.turn.player || g.turn.coins < 2 {
                return Flow::done();
            }
            f.pc = 5;
            f.ask_yes_no(Purpose::Other, "Pay $2 for +2 Cards")
        }
        5 => {
            if !input.is_yes() {
                return Flow::done();
            }
            g.turn.coins -= 2;
            f.pc = 6;
            Flow::Call(Frame::draw(p, 2))
        }
        _ => Flow::done(),
    }
}

fn contract(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.cards_in_hand_where(p, Types::ACTION);
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::SetAside)
        }
        _ => {
            if let Some(c) = input.card() {
                g.set_aside(c, p);
                let data = Scratch {
                    pick: Some(c),
                    ..Scratch::default()
                };
                f.add_duration(g, data);
            }
            Flow::done()
        }
    }
}

/// +1 Action and +2 Favors if the +3 Cards it just gave made its player shuffle.
fn emissary(g: &mut Game, f: &mut CardFrame) {
    let p = f.player;
    let this = f.this();
    let run = g.plays.iter().rev().find(|st| st.card == this);
    let shuffled = run.is_some_and(|st| {
        g.log()[st.run_since..]
            .iter()
            .any(|e| matches!(e, Event::Shuffled { player } if *player == p))
    });
    if shuffled {
        plus_actions(g, p, 1);
        g.players[p.index()].favors += 2;
    }
}

fn hunter(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    const KINDS: [Types; 3] = [Types::ACTION, Types::TREASURE, Types::VICTORY];
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 3))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            Flow::Continue
        }
        2 => {
            while let Some(&kind) = KINDS.get(f.s.i) {
                let matching: Vec<CardId> = f
                    .s
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, kind) && g.zone(c) == Zone::Looking(p))
                    .collect();
                if !matching.is_empty() {
                    f.pc = 3;
                    return f.ask_cards(matching, 1, 1, Purpose::Move).question("put into your hand");
                }
                f.s.i += 1;
            }
            let rest = looking(g, p, &std::mem::take(&mut f.s.cards));
            f.pc = 4;
            Flow::Call(Frame::discard(p, rest))
        }
        3 => {
            for c in input.cards() {
                f.s.cards.retain(|&x| x != c);
                g.put_in_hand(c, p);
            }
            f.s.i += 1;
            f.pc = 2;
            Flow::Continue
        }
        _ => Flow::done(),
    }
}

fn marquis(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::draw(p, g.hand(p).len()))
        }
        1 => {
            let hand = g.hand(p).to_vec();
            if hand.len() <= 10 {
                return Flow::done();
            }
            let n = hand.len() - 10;
            f.pc = 2;
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(p, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn modify(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            f.pc = 2;
            if hand.is_empty() {
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            f.s.pick = chosen.first().copied();
            f.pc = 2;
            Flow::Call(Frame::trash(p, chosen))
        }
        2 => {
            f.pc = 3;
            f.ask_modes(
                &[
                    "+1 Card and +1 Action",
                    "Gain a card costing up to $2 more than the trashed card",
                ],
                1,
                1,
            )
        }
        3 => {
            f.s.m = mode_mask(input);
            f.pc = 4;
            Flow::Continue
        }
        4 => match take_mode(f) {
            Some(0) => {
                plus_actions(g, p, 1);
                Flow::Call(Frame::draw(p, 1))
            }
            Some(_) if f.s.pick.is_some() => {
                f.pc = 5;
                Flow::Continue
            }
            _ => Flow::done(),
        },
        _ => {
            let limit = g.cost_of(f.s.pick.expect("set in step 1")).plus_coins(2);
            gain_step(
                g,
                f,
                input,
                &move |g| g.piles_costing_up_to(limit),
                Dest::Discard,
                5,
                DONE,
            )
        }
    }
}

fn specialist(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let cards = g.playable(p, Types::ACTION | Types::TREASURE);
            f.pc = 1;
            f.ask_cards(cards, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.s.card = Some(g.card(c));
            f.s.n = g.next_lasting as i32;
            f.pc = 2;
            Flow::Call(Frame::play(p, c, true, vec![]))
        }
        2 => {
            f.pc = 3;
            let c = f.s.card.expect("set in step 1");
            f.ask_modes(&[format!("Play {c} again"), format!("Gain a copy of {c}")], 1, 1)
        }
        3 => {
            f.s.m = mode_mask(input);
            f.pc = 4;
            Flow::Continue
        }
        _ => match take_mode(f) {
            Some(0) => {
                let c = f.s.pick.expect("set in step 1");
                // Only a Duration played again keeps this in play with it, and then for
                // what its first play left to do as well.
                if g.is(c, Types::DURATION) {
                    let (first, keeps) = (f.s.n as u32, f.keeps(g));
                    let firsts: Vec<(u32, Vec<CardId>)> = g
                        .lasting
                        .iter()
                        .filter(|l| l.id >= first && l.keeps.last() == Some(&c))
                        .map(|l| (l.id, [&l.keeps[..], &keeps[..]].concat()))
                        .collect();
                    for (id, keeps) in firsts {
                        g.keep_in_play(id, keeps);
                    }
                }
                Flow::Call(Frame::play(p, c, false, f.chain_for_child()))
            }
            Some(_) => {
                let card = f.s.card.expect("set in step 1");
                match supply_pile_topped_by(g, card) {
                    Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                    None => Flow::Continue,
                }
            }
            None => Flow::done(),
        },
    }
}

fn swap(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions: Vec<CardId> = g
                .cards_in_hand_where(p, Types::ACTION)
                .into_iter()
                .filter(|&c| g.home_pile(g.printed(c)).is_some())
                .collect();
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::Return)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.card = Some(g.card(c));
            if !g.return_to_pile(c) {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Continue
        }
        _ => {
            let returned = f.s.card;
            gain_step(
                g,
                f,
                input,
                &move |g| {
                    let limit = Cost::coins(5);
                    (0..g.piles().len())
                        .map(|i| PileId(i as u8))
                        .filter(|&id| {
                            g.pile(id).in_supply
                                && g.pile_top(id).is_some_and(|t| {
                                    g.is(t, Types::ACTION)
                                        && g.cost_of(t).at_most(limit)
                                        && Some(g.card(t)) != returned
                                })
                        })
                        .collect()
                },
                Dest::Hand,
                2,
                DONE,
            )
        }
    }
}

fn town_crier(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["+$2", "Gain a Silver", "+1 Card and +1 Action"], 1, 1)
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            Flow::Continue
        }
        2 => match take_mode(f) {
            Some(0) => {
                g.add_coins(p, 2);
                Flow::Continue
            }
            Some(1) => match top_of(g, Card::Silver) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::Continue,
            },
            Some(_) => {
                plus_actions(g, p, 1);
                Flow::Call(Frame::draw(p, 1))
            }
            None => {
                f.pc = 3;
                Flow::Continue
            }
        },
        _ => may_rotate(g, f, input, Card::Townsfolk, 3, DONE),
    }
}

fn blacksmith(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Draw until you have 6 cards in hand",
                    "+2 Cards",
                    "+1 Card and +1 Action",
                ],
                1,
                1,
            )
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            Flow::Continue
        }
        _ => match take_mode(f) {
            Some(0) => Flow::Call(Frame::draw_to(p, 6)),
            Some(1) => Flow::Call(Frame::draw(p, 2)),
            Some(_) => {
                plus_actions(g, p, 1);
                Flow::Call(Frame::draw(p, 1))
            }
            None => Flow::done(),
        },
    }
}

fn miller(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 4))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            f.ask_cards(f.s.cards.clone(), 1, 1, Purpose::Move)
                .question("put into your hand (the rest are discarded)")
        }
        2 => {
            let chosen = input.cards();
            for &c in &chosen {
                g.put_in_hand(c, p);
            }
            let rest: Vec<CardId> = std::mem::take(&mut f.s.cards)
                .into_iter()
                .filter(|c| !chosen.contains(c))
                .collect();
            f.pc = 3;
            Flow::Call(Frame::discard(p, rest))
        }
        _ => Flow::done(),
    }
}

fn elder(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.playable(p, Types::ACTION);
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let data = Scratch {
                pick: Some(c),
                i: g.turn.played_ids.len(),
                ..Scratch::default()
            };
            f.s.n = g.add_lasting(Card::Elder, p, f.this, vec![], Until::Removed, data) as i32;
            f.pc = 2;
            Flow::Call(Frame::play(p, c, true, vec![]))
        }
        _ => {
            g.remove_lasting(f.s.n as u32);
            Flow::done()
        }
    }
}

fn acolyte(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let cards = g.cards_in_hand_where(p, Types::ACTION | Types::VICTORY);
            f.pc = 1;
            f.ask_cards(cards, 0, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            f.pc = 3;
            if chosen.is_empty() {
                return Flow::Continue;
            }
            f.pc = 2;
            Flow::Call(Frame::trash(p, chosen))
        }
        2 => {
            f.pc = 3;
            match top_of(g, Card::Gold) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        3 => {
            if g.zone(f.this()) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 4;
            f.ask_yes_no(Purpose::Trash, format!("Trash {} to gain an Augur{}", g.card(f.this()), top_named(g, Card::Augurs)))
        }
        4 => {
            let this = f.this();
            if !input.is_yes() || g.zone(this) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 5;
            Flow::Call(Frame::trash(p, vec![this]))
        }
        5 => {
            f.pc = 6;
            match top_of(g, Card::Augurs) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn sorceress(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_name()
        }
        1 => {
            f.s.card = input.name();
            f.pc = 2;
            Flow::Call(Frame::reveal(p, 1))
        }
        2 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            g.put_in_hand(c, p);
            if Some(g.card(c)) != f.s.card {
                return Flow::done();
            }
            f.pc = 3;
            Flow::Continue
        }
        _ => each_target_gains_curse(g, f, DONE),
    }
}

fn sibyl(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 | 2 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            let (purpose, what) = if f.pc == 0 {
                (Purpose::TopDeck, "put on top of your deck")
            } else {
                (Purpose::Move, "put on the bottom of your deck")
            };
            f.pc += 1;
            f.ask_cards(hand, 1, 1, purpose).question(what)
        }
        1 => {
            for c in input.cards() {
                top_deck_hidden(g, c, p);
            }
            f.pc = 2;
            Flow::Continue
        }
        _ => {
            for c in input.cards() {
                g.move_card(c, Zone::Deck(p), Pos::Bottom);
            }
            Flow::done()
        }
    }
}

fn herb_gatherer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let deck: Vec<CardId> = g.player(p).deck.iter().rev().copied().collect();
            for c in deck {
                g.move_card(c, Zone::Discard(p), Pos::Top);
            }
            g.look_through_discard(p);
            let treasures: Vec<CardId> = g
                .player(p)
                .discard
                .iter()
                .copied()
                .filter(|&c| g.is(c, Types::TREASURE))
                .collect();
            f.pc = 1;
            f.ask_cards(treasures, 0, 1, Purpose::Play)
        }
        1 => {
            f.pc = 2;
            match input.card() {
                Some(c) => Flow::Call(Frame::play(p, c, true, vec![])),
                None => Flow::Continue,
            }
        }
        _ => may_rotate(g, f, input, Card::Augurs, 2, DONE),
    }
}

fn battle_plan(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let attacks = g.cards_in_hand_where(p, Types::ATTACK);
            f.pc = 1;
            f.ask_cards(attacks, 0, 1, Purpose::Reveal)
        }
        1 => {
            f.pc = 2;
            if let Some(c) = input.card() {
                g.reveal(p, c);
                return Flow::Call(Frame::draw(p, 1));
            }
            Flow::Continue
        }
        2 => {
            let piles: Vec<PileId> = (0..g.piles().len())
                .map(|i| PileId(i as u8))
                .filter(|&id| g.pile(id).in_supply && rotatable(g, id))
                .collect();
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, true, Purpose::Other)
                .question("you may rotate a Supply pile")
        }
        _ => {
            if let Some(pile) = input.pile() {
                rotate(g, p, pile);
            }
            Flow::done()
        }
    }
}

fn archer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            while let Some(&target) = f.targets.get(f.s.i) {
                let hand = g.hand(target).to_vec();
                if hand.len() >= 5 {
                    f.pc = 1;
                    return f.ask_cards_for(target, hand, 1, 1, Purpose::Other)
                        .question("keep hidden (the others are revealed)");
                }
                f.s.i += 1;
            }
            Flow::done()
        }
        1 => {
            let target = f.targets[f.s.i];
            let hidden = input.cards();
            let revealed: Vec<CardId> = g
                .hand(target)
                .iter()
                .copied()
                .filter(|c| !hidden.contains(c))
                .collect();
            for &c in &revealed {
                g.reveal(target, c);
            }
            f.pc = 2;
            f.ask_cards(revealed, 1, 1, Purpose::Discard)
        }
        2 => {
            let target = f.targets[f.s.i];
            f.s.i += 1;
            f.pc = 0;
            Flow::Call(Frame::discard(target, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn hill_fort(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let piles = g.piles_costing_up_to(Cost::coins(4));
            if piles.is_empty() {
                f.pc = 3;
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            f.s.i = g.log().len();
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => {
                    f.pc = 3;
                    Flow::Continue
                }
            }
        }
        2 => {
            f.s.pick = input.card();
            f.pc = 3;
            Flow::Continue
        }
        3 => {
            f.pc = 4;
            let put = match f.s.pick {
                Some(c) => format!("Put {} into your hand", g.card(c)),
                None => "Put it into your hand".into(),
            };
            f.ask_modes(&[put, "+1 Card and +1 Action".into()], 1, 1)
        }
        4 => {
            let m = mode_mask(input);
            // The card is expected where it was gained to, which need not be the discard
            // pile.
            if m & 1 != 0
                && let Some(c) = f.s.pick
                && gained_to(g, p, c, f.s.i)
                    .is_some_and(|zone| zone != Zone::Hand(p) && g.still_where_put(c, zone))
            {
                g.show(c);
                g.put_in_hand(c, p);
            }
            if m & 2 != 0 {
                plus_actions(g, p, 1);
                f.pc = 5;
                return Flow::Call(Frame::draw(p, 1));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn stronghold(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        f.pc = 1;
        return f.ask_modes(&["+$3", "At the start of your next turn, +3 Cards"], 1, 1);
    }
    let m = mode_mask(input);
    if m & 1 != 0 {
        g.add_coins(f.player, 3);
    }
    if m & 2 != 0 {
        f.add_duration(g, Scratch::default());
    }
    Flow::done()
}

fn old_map(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Discard)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(p, input.cards()))
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::draw(p, 1))
        }
        _ => may_rotate(g, f, input, Card::Odysseys, 3, DONE),
    }
}

fn voyage(g: &mut Game, f: &mut CardFrame) {
    let p = f.player;
    if g.third_in_a_row(p) {
        return;
    }
    g.extra_turns.push_back(ExtraTurn {
        player: p,
        source: Card::Voyage,
        limit: ExtraLimit::ThirdInARow,
    });
    g.add_lasting(
        Card::Voyage,
        p,
        f.this,
        f.keeps(g),
        Until::StartOfTurn(p),
        Scratch::default(),
    );
}

fn sunken_treasure(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        let in_play: Vec<Card> = g.in_play(p).iter().map(|&c| g.card(c)).collect();
        let piles: Vec<PileId> = (0..g.piles().len())
            .map(|i| PileId(i as u8))
            .filter(|&id| {
                g.pile(id).in_supply
                    && g.pile_top(id)
                        .is_some_and(|t| g.is(t, Types::ACTION) && !in_play.contains(&g.card(t)))
            })
            .collect();
        if piles.is_empty() {
            return Flow::done();
        }
        f.pc = 1;
        return f.ask_pile(piles, false, Purpose::Gain);
    }
    gain_step(g, f, input, &|_| vec![], Dest::Discard, 0, DONE)
}

fn student(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 | 1 => may_rotate(g, f, input, Card::Wizards, 0, 2),
        2 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        3 => {
            let chosen = input.cards();
            f.s.n = chosen.iter().any(|&c| g.is(c, Types::TREASURE)) as i32;
            f.pc = 4;
            Flow::Call(Frame::trash(p, chosen))
        }
        _ => {
            if f.s.n != 0 {
                g.players[p.index()].favors += 1;
                let this = f.this();
                if g.zone(this) == Zone::InPlay(p) {
                    g.top_deck(this, p);
                }
            }
            Flow::done()
        }
    }
}

fn sorcerer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(&target) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_name_for(target)
        }
        1 => {
            f.s.card = input.name();
            f.pc = 2;
            Flow::Call(Frame::reveal(target, 1))
        }
        2 => {
            f.pc = 3;
            let Some(c) = input.card() else {
                return Flow::Continue;
            };
            put_back(g, target, &[c]);
            if Some(g.card(c)) != f.s.card
                && let Some(pile) = top_of(g, Card::Curse)
            {
                return Flow::Call(Frame::gain(target, pile, Dest::Discard));
            }
            Flow::Continue
        }
        _ => {
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
    }
}

fn lich_trashed(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let this = f.this();
    match f.pc {
        0 => {
            f.pc = 1;
            if g.zone(this) == Zone::Trash {
                return Flow::Call(Frame::discard(p, vec![this]));
            }
            Flow::Continue
        }
        1 => {
            let limit = g.cost_of(this);
            let cheaper: Vec<CardId> = g
                .trash()
                .iter()
                .copied()
                .filter(|&c| g.cost_of(c).less_than(limit))
                .collect();
            f.pc = 2;
            f.ask_cards(cheaper, 1, 1, Purpose::Gain)
        }
        2 => {
            f.pc = 3;
            match input.card() {
                Some(c) => Flow::Call(Frame::gain_card(p, c, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}
