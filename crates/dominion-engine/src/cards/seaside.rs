//! Seaside (2nd edition) and its removed 1st-edition cards.
//!
//! Durations keep their future part in a [`crate::Lasting`] made by
//! [`CardFrame::add_duration`] (only when the play has something left to do); its
//! `START` ability resolves it at the start of the owner's next turn and ends it, so the
//! card (and whatever played it) is discarded in that turn's Clean-up. Cards a Duration
//! holds (Haven, Blockade) are set aside in the owner's `SetAside` zone and named by the
//! lasting's `data.pick`; the players an Attack-Duration affects are a bit mask in
//! `data.n`.

use super::{CardFrame, Hook, SetRules, due_durations, duration_trigger, mode_mask};
use crate::decision::{DecisionKind, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, Value};
use crate::game::{ExtraLimit, ExtraTurn, Mat, Pos, Scratch, Until, Zone};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, PileId, PlayerId, Set, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {}

pub(crate) struct Rules;

/// A Duration's start-of-turn effect.
const START: u8 = 0;
const LIGHTHOUSE_SHIELD: u8 = 1;
const MONKEY_DRAW: u8 = 1;
const CORSAIR_TRASH: u8 = 1;
const BLOCKADE_CURSE: u8 = 1;
const PIRATE_PLAY: u8 = 1;
const SAILOR_PLAY: u8 = 1;
const TREASURY_TOPDECK: u8 = 0;
const EMBARGO_CURSE: u8 = 0;

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (c, Hook::Ability(START)) if c.is(Types::DURATION) => start(g, f, input),
            (Card::Lighthouse, Hook::Ability(LIGHTHOUSE_SHIELD)) => Flow::Done(Value::Bool(true)),
            (Card::Monkey, Hook::Ability(MONKEY_DRAW)) => draw_then_done(f, 0, 1),
            (Card::Corsair, Hook::Ability(CORSAIR_TRASH)) => corsair_trash(g, f),
            (Card::Blockade, Hook::Ability(BLOCKADE_CURSE)) => blockade_curse(g, f),
            (Card::Pirate, Hook::Ability(PIRATE_PLAY)) => pirate_play(g, f),
            (Card::Sailor, Hook::Ability(SAILOR_PLAY)) => sailor_play(g, f),
            (Card::Treasury, Hook::Ability(TREASURY_TOPDECK)) => {
                let c = f.this();
                if g.zone(c) == Zone::InPlay(f.player) {
                    g.top_deck(c, f.player);
                }
                Flow::done()
            }
            (Card::Embargo, Hook::Ability(EMBARGO_CURSE)) => embargo_curse(g, f),
            (_, Hook::Ability(_)) => return None,
            (Card::Astrolabe, _) => {
                g.add_coins(f.player, 1);
                g.add_buys(f.player, 1);
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Blockade, _) => blockade(g, f, input),
            (
                Card::Caravan
                | Card::Corsair
                | Card::FishingVillage
                | Card::Monkey
                | Card::Pirate
                | Card::TidePools,
                _,
            ) => {
                let data = Scratch {
                    n: mask(&f.targets),
                    ..Scratch::default()
                };
                f.add_duration(g, data);
                Flow::done()
            }
            (Card::Cutpurse, _) => cutpurse(g, f, input),
            (Card::Haven, _) => haven(g, f, input),
            (Card::Island, _) => island(g, f, input),
            (Card::Lighthouse, _) => {
                g.add_coins(f.player, 1);
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Lookout, _) => lookout(g, f, input),
            (Card::MerchantShip, _) => {
                g.add_coins(f.player, 2);
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::NativeVillage, _) => native_village(g, f, input),
            (Card::Outpost, _) => outpost(g, f),
            (Card::Sailor, _) => {
                g.add_lasting(
                    Card::Sailor,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Salvager, _) => salvager(g, f, input),
            (Card::SeaChart, _) => sea_chart(g, f, input),
            (Card::SeaWitch, _) => sea_witch(g, f),
            (Card::Smugglers, _) => smugglers(g, f, input),
            (Card::Tactician, _) => tactician(g, f),
            (Card::TreasureMap, _) => treasure_map(g, f, input),
            (Card::Warehouse, _) => warehouse(g, f, input),
            (Card::Wharf, _) => {
                if f.pc == 0 {
                    g.add_buys(f.player, 1);
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, 2)));
                }
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Ambassador, _) => ambassador(g, f, input),
            (Card::Embargo, _) => embargo(g, f, input),
            (Card::Explorer, _) => explorer(g, f, input),
            (Card::GhostShip, _) => ghost_ship(g, f, input),
            (Card::Navigator, _) => navigator(g, f, input),
            (Card::PearlDiver, _) => pearl_diver(g, f, input),
            (Card::PirateShip, _) => pirate_ship(g, f, input),
            (Card::SeaHag, _) => sea_hag(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in due_durations(g, occ) {
            if l.card == Card::Outpost {
                out.push(duration_trigger(l, START).quiet());
            } else if l.card.set() == Set::Seaside {
                out.push(duration_trigger(l, START));
            }
        }
        match *occ {
            Occasion::Attack { target, .. } => {
                if active(g, Card::Lighthouse).any(|l| l.owner == target) {
                    out.push(Trigger::new(target, Card::Lighthouse, LIGHTHOUSE_SHIELD));
                }
            }
            Occasion::Play { player, card } => {
                let silver_or_gold = |c: &Card| matches!(c, Card::Silver | Card::Gold);
                let first = player == g.turn.player
                    && silver_or_gold(&g.card(card))
                    && g.turn.plays_by(player).filter(|(c, _)| silver_or_gold(c)).count() == 1;
                if first
                    && active(g, Card::Corsair)
                        .any(|l| l.owner != player && in_mask(l.data.n, player))
                {
                    out.push(Trigger::new(player, Card::Corsair, CORSAIR_TRASH).on(card));
                }
            }
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            Occasion::Buy { player, card, .. } => {
                // The pile bought from: the copy that was on top may have left it since.
                if let Some(pile) = g.home_pile(g.printed(card)) {
                    for token in 1..=g.pile(pile).embargo {
                        out.push(
                            Trigger::new(player, Card::Embargo, EMBARGO_CURSE)
                                .labelled(format!("Embargo token {token}")),
                        );
                    }
                }
            }
            Occasion::DiscardFromPlay { player } => {
                let victory = g
                    .turn
                    .gained_in_buy_phase_types
                    .iter()
                    .any(|t| t.contains(Types::VICTORY));
                // Other players, whose cards this Clean-up discards too, had no Buy phase.
                if victory && player == g.turn.player {
                    return;
                }
                for c in g.leaving_play(player) {
                    if g.card(c) == Card::Treasury {
                        out.push(
                            Trigger::new(player, Card::Treasury, TREASURY_TOPDECK)
                                .on(c)
                                .optional("put it onto your deck"),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

fn gain_triggers(g: &Game, gainer: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    for l in active(g, Card::Monkey) {
        if l.owner != gainer && g.right_of(l.owner) == gainer {
            out.push(Trigger::new(l.owner, Card::Monkey, MONKEY_DRAW).via_lasting(l.id));
        }
    }
    if gainer == g.turn.player {
        for l in active(g, Card::Blockade) {
            let copy = l.data.pick.is_some_and(|c| {
                g.zone(c) == Zone::SetAside(l.owner) && g.card(c) == g.card(card)
            });
            if copy && l.owner != gainer && in_mask(l.data.n, gainer) {
                out.push(Trigger::new(l.owner, Card::Blockade, BLOCKADE_CURSE).via_lasting(l.id));
            }
        }
    }
    if g.is(card, Types::TREASURE) {
        for p in g.players_from(g.turn.player) {
            for &c in g.hand(p) {
                if g.card(c) == Card::Pirate && super::can_play(g, p, c) {
                    out.push(Trigger::new(p, Card::Pirate, PIRATE_PLAY).on(c).optional("play it from your hand"));
                }
            }
        }
    }
    if g.is(card, Types::DURATION) && g.still_where_gained(card) {
        for l in g.lasting() {
            if l.card == Card::Sailor && l.until == Until::EndOfTurn && l.owner == gainer {
                out.push(
                    Trigger::new(gainer, Card::Sailor, SAILOR_PLAY)
                        .via_lasting(l.id)
                        .optional(format!("play {}", g.card(card))),
                );
            }
        }
    }
}

/// The Duration effects of `card` that haven't resolved yet.
fn active(g: &Game, card: Card) -> impl Iterator<Item = &crate::Lasting> {
    g.lasting()
        .iter()
        .filter(move |l| l.card == card && l.until == Until::Removed)
}

fn mask(players: &[PlayerId]) -> i32 {
    players.iter().fold(0, |m, p| m | 1 << p.0)
}

fn in_mask(mask: i32, p: PlayerId) -> bool {
    mask & (1 << p.0) != 0
}

/// At step `pc`, draws `n` cards and finishes.
fn draw_then_done(f: &mut CardFrame, pc: u8, n: usize) -> Flow {
    if f.pc == pc {
        f.pc = pc + 1;
        return Flow::Call(Frame::draw(f.player, n));
    }
    Flow::done()
}

/// The Supply pile whose top card is `card`.
fn supply_pile_topped_by(g: &Game, card: Card) -> Option<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&id| g.pile(id).in_supply && g.pile_top(id).is_some_and(|t| g.card(t) == card))
}

fn emit_hand(g: &mut Game, p: PlayerId) {
    let cards = g.hand(p).iter().map(|&c| g.card(c)).collect();
    g.emit(Event::RevealedHand { player: p, cards });
}

/// Puts `id` on top of `p`'s deck without revealing it.
fn top_deck_hidden(g: &mut Game, id: CardId, p: PlayerId) {
    g.move_card(id, Zone::Deck(p), Pos::Top);
    g.emit(Event::Moved {
        player: p,
        card: None,
        to: Zone::Deck(p),
    });
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
        Card::Astrolabe => {
            g.add_coins(p, 1);
            g.add_buys(p, 1);
            Flow::done()
        }
        Card::Blockade | Card::Haven => {
            if let Some(c) = f.s.pick
                && g.zone(c) == Zone::SetAside(p)
            {
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
        Card::Caravan | Card::Corsair | Card::Monkey => draw_then_done(f, 1, 1),
        Card::Outpost => {
            outpost_hand(g, p, f.this);
            Flow::done()
        }
        Card::FishingVillage => {
            g.add_actions(p, 1);
            g.add_coins(p, 1);
            Flow::done()
        }
        Card::Lighthouse => {
            g.add_coins(p, 1);
            Flow::done()
        }
        Card::MerchantShip => {
            g.add_coins(p, 2);
            Flow::done()
        }
        Card::Pirate => match f.pc {
            1 => {
                let piles = g.supply_piles_where(|t, c| {
                    t.intersects(Types::TREASURE) && c.at_most(Cost::coins(6))
                });
                if piles.is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                f.ask_pile(piles, false, Purpose::Gain)
            }
            2 => {
                f.pc = 3;
                match input.pile() {
                    Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Hand)),
                    None => Flow::done(),
                }
            }
            _ => Flow::done(),
        },
        Card::Sailor => match f.pc {
            1 => {
                g.add_coins(p, 2);
                f.pc = 2;
                f.ask_cards(g.hand(p).to_vec(), 0, 1, Purpose::Trash)
            }
            2 => {
                let chosen = input.cards();
                if chosen.is_empty() {
                    return Flow::done();
                }
                f.pc = 3;
                Flow::Call(Frame::trash(p, chosen))
            }
            _ => Flow::done(),
        },
        Card::SeaWitch => match f.pc {
            1 => {
                f.pc = 2;
                Flow::Call(Frame::draw(p, 2))
            }
            _ => discard_from_hand(g, f, input, 2, 2),
        },
        Card::Tactician => {
            if f.pc == 1 {
                g.add_actions(p, 1);
                g.add_buys(p, 1);
            }
            draw_then_done(f, 1, 5)
        }
        Card::TidePools => discard_from_hand(g, f, input, 1, 2),
        Card::Wharf => {
            if f.pc == 1 {
                g.add_buys(p, 1);
            }
            draw_then_done(f, 1, 2)
        }
        _ => Flow::done(),
    }
}

/// From step `pc`: discards `n` cards from hand (all of them if fewer), then finishes.
fn discard_from_hand(g: &mut Game, f: &mut CardFrame, input: Input, pc: u8, n: usize) -> Flow {
    if f.pc == pc {
        let hand = g.hand(f.player).to_vec();
        f.pc = pc + 1;
        return f.ask_cards(hand, n, n, Purpose::Discard);
    }
    if f.pc == pc + 1 {
        f.pc = pc + 2;
        return Flow::Call(Frame::discard(f.player, input.cards()));
    }
    Flow::done()
}

// ---- Triggered abilities ----

fn corsair_trash(g: &mut Game, f: &mut CardFrame) -> Flow {
    let c = f.this();
    if f.pc == 0 && g.zone(c) == Zone::InPlay(f.player) {
        f.pc = 1;
        return Flow::Call(Frame::trash(f.player, vec![c]));
    }
    Flow::done()
}

fn blockade_curse(g: &mut Game, f: &mut CardFrame) -> Flow {
    let Some(Occasion::Gain { player, .. }) = f.occ else {
        return Flow::done();
    };
    if f.pc == 0
        && let Some(curse) = g.supply_pile(Card::Curse)
    {
        f.pc = 1;
        return Flow::Call(Frame::gain(player, curse, Dest::Discard));
    }
    Flow::done()
}

fn pirate_play(g: &mut Game, f: &mut CardFrame) -> Flow {
    let c = f.this();
    if f.pc == 0 && g.zone(c) == Zone::Hand(f.player) && super::can_play(g, f.player, c) {
        f.pc = 1;
        return Flow::Call(Frame::play(f.player, c, true, vec![]));
    }
    Flow::done()
}

fn sailor_play(g: &mut Game, f: &mut CardFrame) -> Flow {
    let Some(Occasion::Gain { card, .. }) = f.occ else {
        return Flow::done();
    };
    if f.pc == 0 && !matches!(g.zone(card), Zone::InPlay(_) | Zone::Trash | Zone::Pile(_)) {
        f.end_lasting(g);
        f.pc = 1;
        return Flow::Call(Frame::play(f.player, card, true, vec![]));
    }
    Flow::done()
}

fn embargo_curse(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc == 0
        && let Some(curse) = g.supply_pile(Card::Curse)
    {
        f.pc = 1;
        return Flow::Call(Frame::gain(f.player, curse, Dest::Discard));
    }
    Flow::done()
}

// ---- Kingdom cards ----

fn blockade(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let piles = g.piles_costing_up_to(Cost::coins(4));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::SetAside)),
                None => Flow::done(),
            }
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::SetAside(f.player)
            {
                let data = Scratch {
                    n: mask(&f.targets),
                    pick: Some(c),
                    ..Scratch::default()
                };
                f.add_duration(g, data);
            }
            Flow::done()
        }
    }
}

fn cutpurse(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    loop {
        match f.pc {
            0 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                let coppers: Vec<_> = g
                    .hand(target)
                    .iter()
                    .copied()
                    .filter(|&c| g.card(c) == Card::Copper)
                    .collect();
                if coppers.is_empty() {
                    emit_hand(g, target);
                    f.s.i += 1;
                    continue;
                }
                f.pc = 1;
                return f.ask_cards_for(target, coppers, 1, 1, Purpose::Discard);
            }
            1 => {
                f.pc = 2;
                return Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
        }
    }
}

fn haven(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::SetAside)
        }
        _ => {
            let p = f.player;
            if let Some(c) = input.card() {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
                g.emit(Event::Moved {
                    player: p,
                    card: None,
                    to: Zone::SetAside(p),
                });
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

fn island(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let this = f.this();
            if g.zone(this) == Zone::InPlay(p) {
                g.put_on_mat(this, p, Mat::Island);
            }
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Move)
                .question("put onto your Island mat")
        }
        _ => {
            if let Some(c) = input.card() {
                g.put_on_mat(c, p, Mat::Island);
            }
            Flow::done()
        }
    }
}

fn lookout(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(f.player, 3))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            f.ask_cards(f.s.cards.clone(), 1, 1, Purpose::Trash)
        }
        2 => {
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = 3;
            Flow::Call(Frame::trash(f.player, chosen))
        }
        3 => {
            f.s.cards.retain(|&c| g.zone(c) == Zone::Deck(f.player));
            f.pc = 4;
            f.ask_cards(f.s.cards.clone(), 1, 1, Purpose::Discard)
        }
        4 => {
            f.pc = 5;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn native_village(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Put the top card of your deck on your mat",
                    "Put the cards from your mat into your hand",
                ],
                1,
                1,
            )
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            if f.s.m & 1 != 0 {
                return Flow::Call(Frame::look(p, 1));
            }
            Flow::Continue
        }
        _ => {
            if let Some(c) = input.card() {
                g.put_on_mat(c, p, Mat::NativeVillage);
            }
            if f.s.m & 2 != 0 {
                for c in g.player(p).mat(Mat::NativeVillage).to_vec() {
                    g.put_in_hand(c, p);
                }
            }
            Flow::done()
        }
    }
}

fn outpost(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    if p == g.turn.player {
        outpost_hand(g, p, f.this);
    } else {
        // Played on another player's turn: its player's next Clean-up draws 3.
        g.add_lasting(
            Card::Outpost,
            p,
            f.this,
            vec![],
            Until::Removed,
            Scratch::default(),
        );
    }
    if g.third_in_a_row(p) {
        return Flow::done();
    }
    g.extra_turns.push_back(ExtraTurn {
        player: p,
        source: Card::Outpost,
        limit: ExtraLimit::ThirdInARow,
    });
    g.add_lasting(
        Card::Outpost,
        p,
        f.this,
        f.keeps(g),
        Until::StartOfTurn(p),
        Scratch::default(),
    );
    Flow::done()
}

/// "You only draw 3 cards for your next hand", for this turn's Clean-up (once a turn).
fn outpost_hand(g: &mut Game, p: PlayerId, this: Option<CardId>) {
    let reduced = g
        .lasting()
        .iter()
        .any(|l| l.card == Card::Outpost && l.owner == p && l.until == Until::EndOfTurn);
    if !reduced {
        g.turn.cleanup_draw = g.turn.cleanup_draw.saturating_sub(2);
        g.add_lasting(
            Card::Outpost,
            p,
            this,
            vec![],
            Until::EndOfTurn,
            Scratch::default(),
        );
    }
}

fn salvager(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            g.add_coins(f.player, f.s.cost.coins as u32);
            Flow::done()
        }
    }
}

fn sea_chart(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        f.pc = 1;
        return Flow::Call(Frame::reveal(f.player, 1));
    }
    if let Some(c) = input.card() {
        let card = g.card(c);
        if g.in_play(f.player).iter().any(|&x| g.card(x) == card) {
            g.put_in_hand(c, f.player);
        }
    }
    Flow::done()
}

fn sea_witch(g: &mut Game, f: &mut CardFrame) -> Flow {
    if let Some(&target) = f.targets.get(f.s.i) {
        f.s.i += 1;
        if let Some(curse) = g.supply_pile(Card::Curse) {
            return Flow::Call(Frame::gain(target, curse, Dest::Discard));
        }
        return Flow::Continue;
    }
    f.add_duration(g, Scratch::default());
    Flow::done()
}

/// The cards `p` gained during their most recent turn (not one in progress).
fn gained_last_turn(g: &Game, p: PlayerId) -> Vec<Card> {
    let log = g.log();
    let is_start = |e: &Event| matches!(e, Event::TurnStart { .. });
    let mut end = log.len();
    if g.turn.player == p {
        end = log.iter().rposition(is_start).unwrap_or(0);
    }
    let Some(begin) = log[..end]
        .iter()
        .rposition(|e| matches!(e, Event::TurnStart { player, .. } if *player == p))
    else {
        return vec![];
    };
    log[begin + 1..]
        .iter()
        .take_while(|e| !is_start(e))
        .filter_map(|e| match e {
            Event::Gained { player, card, .. } if *player == p => Some(*card),
            _ => None,
        })
        .collect()
}

fn smugglers(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let gained = gained_last_turn(g, g.right_of(f.player));
            let piles = g.supply_piles_where(|_, _| true);
            let piles: Vec<PileId> = piles
                .into_iter()
                .filter(|&id| {
                    let top = g.pile_top(id).expect("non-empty");
                    gained.contains(&g.card(top)) && g.cost_of(top).at_most(Cost::coins(6))
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

fn tactician(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc == 0 {
        let hand = g.hand(f.player).to_vec();
        if hand.is_empty() {
            return Flow::done();
        }
        f.pc = 1;
        return Flow::Call(Frame::discard(f.player, hand));
    }
    f.add_duration(g, Scratch::default());
    Flow::done()
}

fn treasure_map(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let maps: Vec<_> = g
                .hand(p)
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::TreasureMap)
                .collect();
            f.pc = 1;
            if maps.is_empty() {
                return Flow::Continue;
            }
            f.ask_cards(maps, 1, 1, Purpose::Trash)
        }
        1 => {
            let this = f.this();
            let mut cards = input.cards();
            if g.zone(this) == Zone::InPlay(p) {
                cards.insert(0, this);
            }
            f.s.n = cards.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::trash(p, cards))
        }
        _ => {
            if f.s.n == 2
                && f.s.i < 4
                && let Some(gold) = g.supply_pile(Card::Gold)
            {
                f.s.i += 1;
                return Flow::Call(Frame::gain(p, gold, Dest::DeckTop));
            }
            Flow::done()
        }
    }
}

fn warehouse(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    discard_from_hand(g, f, input, 0, 3)
}

// ---- Removed 1st-edition cards ----

fn ambassador(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                let hand = g.hand(p).to_vec();
                if hand.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                return f.ask_cards(hand, 1, 1, Purpose::Reveal);
            }
            1 => {
                let Some(c) = input.take().and_then(Input::card) else {
                    return Flow::done();
                };
                let card = g.card(c);
                f.s.card = Some(card);
                g.reveal(p, c);
                let returnable = g
                    .home_pile(g.printed(c))
                    .is_some_and(|pile| g.pile(pile).in_supply);
                f.pc = 3;
                if returnable {
                    let copies: Vec<_> = g
                        .hand(p)
                        .iter()
                        .copied()
                        .filter(|&x| g.card(x) == card)
                        .collect();
                    f.pc = 2;
                    return f.ask_cards(copies, 0, 2, Purpose::Return).question("return to the Supply");
                }
            }
            2 => {
                for c in input.take().map(Input::cards).unwrap_or_default() {
                    if g.zone(c) == Zone::Hand(p) {
                        g.return_to_pile(c);
                    }
                }
                f.pc = 3;
                return Flow::Continue;
            }
            _ => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                f.s.i += 1;
                let card = f.s.card.expect("set in step 1");
                if let Some(pile) = supply_pile_topped_by(g, card) {
                    return Flow::Call(Frame::gain(target, pile, Dest::Discard));
                }
                return Flow::Continue;
            }
        }
    }
}

fn embargo(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let this = f.this();
    match f.pc {
        0 => {
            if g.zone(this) != Zone::InPlay(f.player) {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::trash(f.player, vec![this]))
        }
        1 => {
            let piles: Vec<PileId> = (0..g.piles().len())
                .map(|i| PileId(i as u8))
                .filter(|&id| g.pile(id).in_supply)
                .collect();
            f.pc = 2;
            f.ask_pile(piles, false, Purpose::Other)
                .question("choose a Supply pile for an Embargo token")
        }
        _ => {
            if let Some(pile) = input.pile() {
                g.piles[pile.index()].embargo += 1;
            }
            Flow::done()
        }
    }
}

fn explorer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let provinces: Vec<_> = g
                .hand(p)
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::Province)
                .collect();
            f.pc = 1;
            if provinces.is_empty() {
                return Flow::Continue;
            }
            f.ask_cards(provinces, 0, 1, Purpose::Reveal)
        }
        1 => {
            let card = if let Some(&province) = input.cards().first() {
                g.reveal(p, province);
                Card::Gold
            } else {
                Card::Silver
            };
            f.pc = 2;
            match g.supply_pile(card) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Hand)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn ghost_ship(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    loop {
        match f.pc {
            0 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                let hand = g.hand(target).to_vec();
                if hand.len() < 4 {
                    f.s.i += 1;
                    continue;
                }
                let n = hand.len() - 3;
                f.pc = 1;
                return f.ask(
                    target,
                    DecisionKind::Cards {
                        candidates: hand,
                        min: n,
                        max: n,
                        ordered: true,
                    },
                    Purpose::TopDeck,
                );
            }
            _ => {
                let target = f.targets[f.s.i];
                for &c in input.cards().iter().rev() {
                    top_deck_hidden(g, c, target);
                }
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
        }
    }
}

fn navigator(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 5))
        }
        1 => {
            f.s.cards = input.cards();
            if f.s.cards.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_either(&["Discard them all", "Put them back in any order"])
                .about(f.s.cards.clone())
        }
        2 => {
            if input.opt() == Some(crate::Opt::Mode(0)) {
                f.pc = 4;
                return Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)));
            }
            f.pc = 3;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        3 => {
            for &c in input.cards().iter().rev() {
                g.move_card(c, Zone::Deck(p), Pos::Top);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn pearl_diver(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            let pl = g.player(p);
            if pl.deck.is_empty() && !pl.discard.is_empty() {
                return Flow::Call(Frame::shuffle(p));
            }
            Flow::Continue
        }
        1 => {
            let Some(&bottom) = g.player(p).deck.first() else {
                return Flow::done();
            };
            f.pc = 2;
            f.ask_cards(vec![bottom], 0, 1, Purpose::TopDeck)
        }
        _ => {
            for c in input.cards() {
                if g.zone(c) == Zone::Deck(p) {
                    g.top_deck(c, p);
                }
            }
            Flow::done()
        }
    }
}

fn pirate_ship(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                f.pc = 1;
                return f.ask_modes(
                    &[
                        "+$1 per Coin token on your Pirate Ship mat",
                        "Each other player trashes a revealed Treasure",
                    ],
                    1,
                    1,
                );
            }
            1 => {
                let m = input.take().map(mode_mask).unwrap_or_default();
                if m & 1 != 0 {
                    let n = g.player(f.player).pirate_ship;
                    g.add_coins(f.player, n);
                }
                if m & 2 == 0 {
                    return Flow::done();
                }
                f.pc = 2;
            }
            2 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    if f.s.n > 0 {
                        g.players[f.player.index()].pirate_ship += 1;
                    }
                    return Flow::done();
                };
                f.pc = 3;
                return Flow::Call(Frame::reveal(target, 2));
            }
            3 => {
                f.s.cards = input.take().map(Input::cards).unwrap_or_default();
                let treasures: Vec<_> = f
                    .s
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, Types::TREASURE))
                    .collect();
                if treasures.is_empty() {
                    f.pc = 5;
                    continue;
                }
                f.pc = 4;
                return f.ask_cards(treasures, 1, 1, Purpose::Trash);
            }
            4 => {
                let chosen = input.take().map(Input::cards).unwrap_or_default();
                f.s.cards.retain(|c| !chosen.contains(c));
                f.s.n = 1;
                f.pc = 5;
                return Flow::Call(Frame::trash(f.targets[f.s.i], chosen));
            }
            5 => {
                f.pc = 6;
                let target = f.targets[f.s.i];
                let rest: Vec<_> = std::mem::take(&mut f.s.cards)
                    .into_iter()
                    .filter(|&c| g.zone(c) == Zone::Deck(target))
                    .collect();
                return Flow::Call(Frame::discard(target, rest));
            }
            _ => {
                f.s.i += 1;
                f.pc = 2;
            }
        }
    }
}

fn sea_hag(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                f.pc = 1;
                return Flow::Call(Frame::look(target, 1));
            }
            1 => {
                f.pc = 2;
                if let Some(c) = input.take().and_then(Input::card) {
                    return Flow::Call(Frame::discard(f.targets[f.s.i], vec![c]));
                }
            }
            2 => {
                f.pc = 3;
                if let Some(curse) = g.supply_pile(Card::Curse) {
                    return Flow::Call(Frame::gain(f.targets[f.s.i], curse, Dest::DeckTop));
                }
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
            }
        }
    }
}
