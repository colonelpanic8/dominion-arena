//! Rising Sun: Omens and the game's Prophecy, Shadow cards, Debt, Events.
//!
//! The Prophecy is `Game::prophecy` (card, Sun tokens left), dealt in setup when an Omen
//! is in the game. An Omen's "+1 Sun" runs first in its `Hook::Main` (its vanilla bonuses
//! come from the card data, applied just before). Effects that last are [`crate::Lasting`]s
//! named by their card: Durations (Riverboat, Samurai) and the Prophecies' set-aside cards
//! (Rapid Expansion, Biding Time) resolve through `due_durations`; Daimyo, River Shrine and
//! Foresight last until the end of the turn. Riverboat's card sits alone in a removed
//! holder pile, so it has no pile of its own.

use super::{CardFrame, Hook, SetRules, due_durations, duration_trigger};
use crate::decision::Purpose;
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, Value};
use crate::game::{Pile, Pos, Scratch, Until, Zone};
use crate::setup::{add_pile, victory_pile_size};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Plus, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Riverboat's card, set aside in setup.
    pub riverboat: Option<CardId>,
    /// Who last bought Continue, and on which of their turns.
    pub continued: Option<(PlayerId, u32)>,
}

pub(crate) struct Rules;

/// A Duration's or a set-aside card's start-of-turn effect.
const START: u8 = 0;
const SAMURAI_COIN: u8 = 1;
const DAIMYO_CLAIM: u8 = 0;
const DAIMYO_REPLAY: u8 = 1;
const RIVER_SHRINE_GAIN: u8 = 0;
const FORESIGHT_RETURN: u8 = 0;

const APPROACHING_ARMY: u8 = 0;
const BIDING_TIME_SET_ASIDE: u8 = 1;
const BUREAUCRACY: u8 = 0;
const ENLIGHTENMENT: u8 = 0;
const GOOD_HARVEST: u8 = 0;
const GREAT_LEADER: u8 = 0;
const GROWTH: u8 = 0;
const HARSH_WINTER: u8 = 0;
const KIND_EMPEROR_GAIN: u8 = 0;
const PANIC_BUYS: u8 = 0;
const PANIC_RETURN: u8 = 1;
const PROGRESS: u8 = 0;
const RAPID_EXPANSION_SET_ASIDE: u8 = 1;
const SICKNESS: u8 = 0;

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Riverboat | Card::RapidExpansion | Card::BidingTime, Hook::Ability(START)) => {
                start(g, f)
            }
            (Card::Samurai, Hook::Ability(SAMURAI_COIN)) => {
                g.add_coins(f.player, 1);
                Flow::done()
            }
            (Card::Daimyo, Hook::Ability(DAIMYO_CLAIM)) => {
                if let (Some(id), Some(Occasion::Play { card, .. })) = (f.lasting, &f.occ)
                    && let Some(l) = g.lasting_mut(id)
                {
                    l.data.pick = Some(*card);
                }
                Flow::done()
            }
            (Card::Daimyo, Hook::Ability(DAIMYO_REPLAY)) => daimyo_replay(g, f),
            (Card::RiverShrine, Hook::Ability(RIVER_SHRINE_GAIN)) => {
                if f.pc == 0 {
                    f.end_lasting(g);
                    f.pc = 1;
                }
                gain_where(g, f, input, 1, Dest::Discard, |_, c| c.at_most(Cost::coins(4)))
            }
            (Card::Foresight, Hook::Ability(FORESIGHT_RETURN)) => {
                if let Some(c) = f.end_lasting(g).and_then(|l| l.data.pick)
                    && g.zone(c) == Zone::SetAside(f.player)
                {
                    g.put_in_hand(c, f.player);
                }
                Flow::done()
            }
            (Card::ApproachingArmy, Hook::Ability(APPROACHING_ARMY)) => {
                g.add_coins(f.player, 1);
                Flow::done()
            }
            (Card::BidingTime, Hook::Ability(BIDING_TIME_SET_ASIDE)) => {
                biding_time_set_aside(g, f.player);
                Flow::done()
            }
            (Card::Bureaucracy, Hook::Ability(BUREAUCRACY)) => {
                if f.pc == 0
                    && let Some(copper) = g.supply_pile(Card::Copper)
                {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::gain(f.player, copper, Dest::Discard)));
                }
                Flow::done()
            }
            (Card::Enlightenment, Hook::Ability(ENLIGHTENMENT)) => {
                if let Some(st) = g.plays.last_mut() {
                    // Overrides a Highwayman resolved before it.
                    st.blocked = false;
                    st.replaced = Some(Plus {
                        cards: 1,
                        actions: 1,
                        ..Plus::default()
                    });
                }
                Flow::done()
            }
            (Card::GoodHarvest, Hook::Ability(GOOD_HARVEST)) => {
                plus_buys(g, f.player, 1);
                g.add_coins(f.player, 1);
                Flow::done()
            }
            (Card::GreatLeader, Hook::Ability(GREAT_LEADER)) => {
                plus_actions(g, f.player, 1);
                Flow::done()
            }
            (Card::Growth, Hook::Ability(GROWTH)) => growth(g, f, input),
            (Card::HarshWinter, Hook::Ability(HARSH_WINTER)) => {
                let pile = &mut g.piles[f.data as usize];
                if pile.debt_tokens > 0 {
                    let debt = std::mem::take(&mut pile.debt_tokens);
                    g.add_debt(f.player, debt as u32);
                } else {
                    pile.debt_tokens += 2;
                }
                Flow::done()
            }
            (Card::KindEmperor, Hook::Ability(KIND_EMPEROR_GAIN)) => gain_where(
                g,
                f,
                input,
                0,
                Dest::Hand,
                |t, _| t.intersects(Types::ACTION),
            ),
            (Card::Panic, Hook::Ability(PANIC_BUYS)) => {
                plus_buys(g, f.player, 2);
                Flow::done()
            }
            (Card::Panic, Hook::Ability(PANIC_RETURN)) => {
                for c in panic_returns(g, f.player) {
                    g.return_to_pile(c);
                }
                Flow::done()
            }
            (Card::Progress, Hook::Ability(PROGRESS)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ {
                    g.top_deck(card, f.player);
                }
                Flow::done()
            }
            (Card::RapidExpansion, Hook::Ability(RAPID_EXPANSION_SET_ASIDE)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ {
                    g.set_aside(card, f.player);
                    let data = Scratch {
                        pick: Some(card),
                        ..Scratch::default()
                    };
                    g.add_lasting(
                        Card::RapidExpansion,
                        f.player,
                        None,
                        vec![],
                        Until::Removed,
                        data,
                    );
                }
                Flow::done()
            }
            (Card::Sickness, Hook::Ability(SICKNESS)) => sickness(g, f, input),
            (_, Hook::Ability(_)) => return None,
            (c, Hook::Main) if f.pc == 0 && c.is(Types::OMEN) => {
                f.pc = 1;
                remove_sun(g, f.player)
            }
            (Card::Kitsune, _) => kitsune(g, f, input),
            (Card::MountainShrine, _) => mountain_shrine(g, f, input),
            (Card::Poet, _) => poet(g, f, input),
            (Card::RiverShrine, _) => river_shrine(g, f, input),
            (Card::RusticVillage, _) => rustic_village(g, f, input),
            (Card::TeaHouse, _) => Flow::done(),
            (Card::Alley, _) => alley(g, f, input),
            (Card::Aristocrat, _) => {
                if f.pc > 0 {
                    return Some(Flow::done());
                }
                f.pc = 1;
                let n = g
                    .in_play(f.player)
                    .iter()
                    .filter(|&&c| g.card(c) == Card::Aristocrat)
                    .count();
                let p = f.player;
                match n {
                    1 | 5 => plus_actions(g, p, 3),
                    2 | 6 => return Some(Flow::Call(Frame::draw(p, 3))),
                    3 | 7 => g.add_coins(p, 3),
                    4 | 8 => plus_buys(g, p, 3),
                    _ => {}
                }
                Flow::done()
            }
            (Card::Artist, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                    let cards: Vec<Card> = g.in_play(f.player).iter().map(|&c| g.card(c)).collect();
                    let singles = cards
                        .iter()
                        .filter(|c| cards.iter().filter(|d| d == c).count() == 1)
                        .count();
                    return Some(Flow::Call(Frame::draw(f.player, singles)));
                }
                Flow::done()
            }
            (Card::Change, _) => change(g, f, input),
            (Card::Craftsman, _) => {
                if f.pc == 0 {
                    g.add_debt(f.player, 2);
                }
                gain_where(g, f, input, 0, Dest::Discard, |_, c| {
                    c.at_most(Cost::coins(5))
                })
            }
            (Card::Daimyo, _) => {
                g.add_lasting(
                    Card::Daimyo,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::Fishmonger, _) => Flow::done(),
            (Card::GoldMine, _) => gold_mine(g, f, input),
            (Card::ImperialEnvoy, _) => {
                g.add_debt(f.player, 2);
                Flow::done()
            }
            (Card::Litter, _) => {
                g.add_debt(f.player, 1);
                Flow::done()
            }
            (Card::RootCellar, _) => {
                g.add_debt(f.player, 3);
                Flow::done()
            }
            (Card::Ninja, _) => discard_down_to_3(g, f, input),
            (Card::Rice, _) => {
                let types = g
                    .in_play(f.player)
                    .iter()
                    .fold(Types::empty(), |t, &c| t | g.types(c));
                g.add_coins(f.player, types.bits().count_ones());
                Flow::done()
            }
            (Card::RiceBroker, _) => rice_broker(g, f, input),
            (Card::Riverboat, _) => {
                if g.ext.rising_sun.riverboat.is_some() {
                    f.add_duration(g, Scratch::default());
                }
                Flow::done()
            }
            (Card::Ronin, _) => {
                if f.pc == 1 && input.cards().is_empty() {
                    return Some(Flow::done());
                }
                if g.hand(f.player).len() < 7 {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, 1)));
                }
                Flow::done()
            }
            (Card::Samurai, _) => {
                let flow = discard_down_to_3(g, f, input);
                if !matches!(flow, Flow::Done(_)) {
                    return Some(flow);
                }
                g.add_lasting(
                    Card::Samurai,
                    f.player,
                    f.this,
                    f.keeps(),
                    Until::Removed,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::SnakeWitch, _) => snake_witch(g, f, input),
            (Card::Tanuki, _) => upgrade(g, f, input, 2),
            (Card::Amass, _) => {
                if f.pc == 0 {
                    let actions = g.in_play(f.player).iter().any(|&c| g.is(c, Types::ACTION));
                    if actions {
                        return Some(Flow::done());
                    }
                }
                gain_where(g, f, input, 0, Dest::Discard, |t, c| {
                    t.intersects(Types::ACTION) && c.at_most(Cost::coins(5))
                })
            }
            (Card::Asceticism, _) => asceticism(g, f, input),
            (Card::Continue, _) => continue_event(g, f, input),
            (Card::Credit, _) => credit(g, f, input),
            (Card::Foresight, _) => foresight(g, f, input),
            (Card::Gather, _) => gather(g, f, input),
            (Card::Kintsugi, _) => kintsugi(g, f, input),
            (Card::Practice, _) => practice(g, f, input),
            (Card::ReceiveTribute, _) => receive_tribute(g, f, input),
            (Card::SeaTrade, _) => sea_trade(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in due_durations(g, occ) {
            match l.card {
                Card::Riverboat | Card::RapidExpansion | Card::BidingTime => {
                    out.push(duration_trigger(l, START));
                }
                Card::Samurai if l.source.is_some_and(|c| g.zone(c) == Zone::InPlay(l.owner)) => {
                    out.push(duration_trigger(l, SAMURAI_COIN).quiet());
                }
                _ => {}
            }
        }
        let active = |card| g.prophecy_active(card);
        let turn = g.turn.player;
        match *occ {
            Occasion::StartTurn { player } => {
                if active(Card::KindEmperor) {
                    out.push(Trigger::new(player, Card::KindEmperor, KIND_EMPEROR_GAIN));
                }
                if active(Card::Sickness) {
                    out.push(Trigger::new(player, Card::Sickness, SICKNESS));
                }
            }
            Occasion::StartCleanup { player } => {
                if active(Card::BidingTime) && !g.hand(player).is_empty() {
                    out.push(Trigger::new(player, Card::BidingTime, BIDING_TIME_SET_ASIDE));
                }
                if g.turn.gained_in_buy_phase.is_empty() && player == turn {
                    for l in this_turn(g, Card::RiverShrine, player) {
                        out.push(
                            Trigger::new(player, Card::RiverShrine, RIVER_SHRINE_GAIN)
                                .via_lasting(l.id),
                        );
                    }
                }
            }
            Occasion::EndTurn { player } => {
                for l in this_turn(g, Card::Foresight, player) {
                    out.push(
                        Trigger::new(player, Card::Foresight, FORESIGHT_RETURN).via_lasting(l.id),
                    );
                }
            }
            Occasion::DiscardFromPlay { player } => {
                if active(Card::Panic) && !panic_returns(g, player).is_empty() {
                    out.push(Trigger::new(player, Card::Panic, PANIC_RETURN));
                }
            }
            Occasion::Play { player, card } => {
                let action = g.is(card, Types::ACTION);
                if action && !g.is(card, Types::COMMAND) {
                    for l in this_turn(g, Card::Daimyo, player) {
                        if l.data.pick.is_none() {
                            out.push(
                                Trigger::new(player, Card::Daimyo, DAIMYO_CLAIM)
                                    .via_lasting(l.id)
                                    .quiet(),
                            );
                        }
                    }
                }
                if !g.is(card, Types::TREASURE) {
                    return;
                }
                if active(Card::Enlightenment) && g.turn.phase == crate::Phase::Action {
                    out.push(Trigger::new(player, Card::Enlightenment, ENLIGHTENMENT));
                }
                if active(Card::Panic) && player == turn {
                    out.push(Trigger::new(player, Card::Panic, PANIC_BUYS).quiet());
                }
                let name = g.card(card);
                let first = g.turn.played.iter().filter(|&&c| c == name).count() == 1;
                if active(Card::GoodHarvest) && player == turn && first {
                    out.push(Trigger::new(player, Card::GoodHarvest, GOOD_HARVEST).quiet());
                }
            }
            Occasion::AfterPlay { player, card } => {
                for l in this_turn(g, Card::Daimyo, player) {
                    if l.data.pick == Some(card) {
                        out.push(
                            Trigger::new(player, Card::Daimyo, DAIMYO_REPLAY).via_lasting(l.id),
                        );
                    }
                }
                if active(Card::GreatLeader) && player == turn && g.is(card, Types::ACTION) {
                    out.push(Trigger::new(player, Card::GreatLeader, GREAT_LEADER).quiet());
                }
                if active(Card::ApproachingArmy) && g.is(card, Types::ATTACK) {
                    out.push(Trigger::new(player, Card::ApproachingArmy, APPROACHING_ARMY));
                }
            }
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            _ => {}
        }
    }

    fn modify_cost(g: &Game, card: Card, _id: Option<CardId>, cost: &mut Cost) {
        if card.kind() == Kind::Card && g.prophecy_active(Card::FlourishingTrade) {
            *cost = cost.minus_coins(1);
        }
    }

    fn modify_types(g: &Game, _id: CardId, types: &mut Types) {
        if types.contains(Types::TREASURE) && g.prophecy_active(Card::Enlightenment) {
            *types |= Types::ACTION;
        }
    }

    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        card != Card::Continue
            || g.ext.rising_sun.continued != Some((p, g.player(p).turns_taken))
    }

    fn actions_are_buys(g: &Game) -> bool {
        g.prophecy_active(Card::FlourishingTrade)
    }

    fn arrange_shuffled(g: &Game, _p: PlayerId, cards: &mut Vec<CardId>) {
        let (mut shadows, rest): (Vec<_>, Vec<_>) =
            cards.iter().partition(|&&c| g.is(c, Types::SHADOW));
        shadows.extend(rest);
        *cards = shadows;
    }

    fn playable_as_if_in_hand(g: &Game, p: PlayerId, out: &mut Vec<CardId>) {
        out.extend(
            g.player(p)
                .deck
                .iter()
                .rev()
                .copied()
                .filter(|&c| g.is(c, Types::SHADOW)),
        );
    }

    fn setup_added_piles(g: &mut Game, cards: &[Card]) {
        if cards.contains(&Card::Riverboat) && g.ext.rising_sun.riverboat.is_none() {
            set_aside_riverboat_card(g);
        }
    }

    fn setup_kingdom(g: &mut Game) {
        if g.supply_pile(Card::Riverboat).is_some() {
            set_aside_riverboat_card(g);
        }
        let configured = g
            .landscapes
            .iter()
            .copied()
            .find(|l| l.kind() == Kind::Prophecy);
        let omen = g
            .piles
            .iter()
            .any(|pile| pile.cards.iter().any(|&c| g.printed(c).is(Types::OMEN)));
        let prophecy = configured.or_else(|| {
            omen.then(|| {
                let all: Vec<Card> = Card::ALL
                    .iter()
                    .copied()
                    .filter(|c| c.kind() == Kind::Prophecy)
                    .collect();
                let card = all[g.random_index(all.len())];
                g.landscapes.push(card);
                card
            })
        });
        let Some(prophecy) = prophecy else {
            return;
        };
        let suns = match g.num_players() {
            2 => 5,
            3 => 8,
            4 => 10,
            5 => 12,
            _ => 13,
        };
        g.prophecy = Some((prophecy, suns));
        if prophecy == Card::ApproachingArmy {
            let mut pool = unused_kingdom_cards(g);
            pool.retain(|c| c.is(Types::ATTACK));
            if !pool.is_empty() {
                let card = pool[g.random_index(pool.len())];
                add_kingdom_pile(g, card);
            }
        }
    }
}

// ---- Helpers ----

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

/// This turn's lasting effects of `card` owned by `p`.
fn this_turn(g: &Game, card: Card, p: PlayerId) -> impl Iterator<Item = &crate::Lasting> {
    g.lasting()
        .iter()
        .filter(move |l| l.card == card && l.owner == p && l.until == Until::EndOfTurn)
}

fn dest_zone(to: Dest, p: PlayerId) -> Zone {
    match to {
        Dest::Discard => Zone::Discard(p),
        Dest::Hand => Zone::Hand(p),
        Dest::DeckTop => Zone::Deck(p),
        Dest::SetAside => Zone::SetAside(p),
    }
}

/// From step `pc`: gain a card from a Supply pile whose top card satisfies `pred` (if any
/// does), then finish.
fn gain_where(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    pc: u8,
    to: Dest,
    pred: impl Fn(Types, Cost) -> bool,
) -> Flow {
    if f.pc == pc {
        let piles = g.supply_piles_where(pred);
        if piles.is_empty() {
            return Flow::done();
        }
        f.pc = pc + 1;
        return f.ask_pile(piles, false, Purpose::Gain);
    }
    if f.pc == pc + 1 {
        f.pc = pc + 2;
        return match input.pile() {
            Some(pile) => Flow::Call(Frame::gain(f.player, pile, to)),
            None => Flow::done(),
        };
    }
    Flow::Done(match input {
        Input::Return(v) => v,
        _ => Value::None,
    })
}

/// Kingdom cards not used in this game (no pile of theirs, not Riverboat's card).
fn unused_kingdom_cards(g: &Game) -> Vec<Card> {
    let riverboat = g.ext.rising_sun.riverboat.map(|c| home_name(g.printed(c)));
    Card::kingdom_cards()
        .filter(|c| !c.def().removed && Some(*c) != riverboat)
        .filter(|&c| !g.piles().iter().any(|p| p.card == c))
        .collect()
}

fn home_name(card: Card) -> Card {
    card.def().pile.unwrap_or(card)
}

/// Adds a Kingdom pile mid-setup (Approaching Army, Divine Wind).
fn add_kingdom_pile(g: &mut Game, card: Card) {
    if !super::build_kingdom_pile(g, card) {
        let size = if card.is(Types::VICTORY) {
            victory_pile_size(g.num_players())
        } else {
            10
        };
        add_pile(g, card, size, true);
    }
}

fn set_aside_riverboat_card(g: &mut Game) {
    let used = |g: &Game, c: Card| g.piles().iter().any(|p| p.card == home_name(c));
    let pool: Vec<Card> = Card::ALL
        .iter()
        .copied()
        .filter(|&c| {
            let def = c.def();
            c.kind() == Kind::Card
                && !def.removed
                && (def.kingdom || def.pile.is_some_and(|p| p.def().kingdom))
                && c.is(Types::ACTION)
                && !c.is(Types::DURATION)
                && def.cost == Some(Cost::coins(5))
                && !used(g, c)
        })
        .collect();
    if pool.is_empty() {
        return;
    }
    let card = pool[g.random_index(pool.len())];
    let pile = PileId(g.piles.len() as u8);
    let mut holder = Pile::new(card, false);
    holder.removed = true;
    g.piles.push(holder);
    let id = g.create(card, Zone::Pile(pile));
    g.ext.rising_sun.riverboat = Some(id);
    super::setup_added_piles(g, &[card]);
}

/// "+1 Sun": removes a Sun token; the last one makes the Prophecy happen.
fn remove_sun(g: &mut Game, p: PlayerId) -> Flow {
    let Some((prophecy, suns)) = g.prophecy else {
        return Flow::Continue;
    };
    if suns == 0 {
        return Flow::Continue;
    }
    g.prophecy = Some((prophecy, suns - 1));
    if suns > 1 {
        return Flow::Continue;
    }
    g.emit(Event::Received {
        player: p,
        card: prophecy,
    });
    match prophecy {
        Card::KindEmperor => Flow::Call(Frame::Card(Box::new(CardFrame::new(
            Card::KindEmperor,
            Hook::Ability(KIND_EMPEROR_GAIN),
            p,
            None,
        )))),
        Card::DivineWind => {
            divine_wind(g);
            Flow::Continue
        }
        _ => Flow::Continue,
    }
}

/// Removes the Kingdom card piles from the Supply and sets up 10 new random ones.
fn divine_wind(g: &mut Game) {
    for pile in &mut g.piles {
        if pile.in_supply && pile.card.def().kingdom {
            pile.in_supply = false;
            pile.removed = true;
        }
    }
    let mut pool = unused_kingdom_cards(g);
    g.shuffle(&mut pool);
    pool.truncate(10);
    for &card in &pool {
        add_kingdom_pile(g, card);
    }
    let potions = pool.iter().any(|&k| {
        k.pile_members()
            .iter()
            .chain(std::iter::once(&k))
            .any(|c| c.cost().is_some_and(|c| c.potions > 0))
    });
    if potions && g.supply_pile(Card::Potion).is_none() {
        add_pile(g, Card::Potion, 16, true);
    }
    super::setup_added_piles(g, &pool);
}

fn gain_triggers(g: &Game, player: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let Some((prophecy, 0)) = g.prophecy else {
        return;
    };
    let landed = g
        .gains
        .last()
        .filter(|st| st.card == card)
        .map(|st| dest_zone(st.to, st.player));
    let still_there = landed == Some(g.zone(card));
    match prophecy {
        Card::Bureaucracy if g.cost_of(card) != Cost::ZERO => {
            out.push(Trigger::new(player, Card::Bureaucracy, BUREAUCRACY));
        }
        Card::Growth if g.is(card, Types::TREASURE) => {
            out.push(Trigger::new(player, Card::Growth, GROWTH));
        }
        Card::HarshWinter if player == g.turn.player && g.turn.controller == player => {
            if let Some(pile) = g.home_pile(g.printed(card)) {
                out.push(Trigger::new(player, Card::HarshWinter, HARSH_WINTER).data(pile.0 as i32));
            }
        }
        Card::Progress if still_there && landed != Some(Zone::Deck(player)) => {
            out.push(Trigger::new(player, Card::Progress, PROGRESS));
        }
        Card::RapidExpansion
            if still_there && g.is(card, Types::ACTION | Types::TREASURE) =>
        {
            out.push(Trigger::new(
                player,
                Card::RapidExpansion,
                RAPID_EXPANSION_SET_ASIDE,
            ));
        }
        _ => {}
    }
}

/// The Treasures `p` is about to discard from play.
fn panic_returns(g: &Game, p: PlayerId) -> Vec<CardId> {
    g.in_play(p)
        .iter()
        .copied()
        .filter(|&c| {
            g.is(c, Types::TREASURE)
                && !g.lasting().iter().any(|l| l.keeps.contains(&c))
                && g.home_pile(g.printed(c)).is_some()
        })
        .collect()
}

// ---- Lasting effects ----

fn start(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    let Some(l) = f.end_lasting(g) else {
        return Flow::done();
    };
    let p = f.player;
    match f.card {
        Card::Riverboat => {
            let Some(c) = g.ext.rising_sun.riverboat else {
                return Flow::done();
            };
            f.pc = 1;
            Flow::Call(Frame::play(p, c, false, l.keeps))
        }
        Card::RapidExpansion => match l.data.pick {
            Some(c) if g.zone(c) == Zone::SetAside(p) => {
                f.pc = 1;
                Flow::Call(Frame::play(p, c, true, vec![]))
            }
            _ => Flow::done(),
        },
        Card::BidingTime => {
            for c in l.data.cards {
                if g.zone(c) == Zone::SetAside(p) {
                    g.put_in_hand(c, p);
                }
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn biding_time_set_aside(g: &mut Game, p: PlayerId) {
    let cards = g.hand(p).to_vec();
    for &c in &cards {
        g.move_card(c, Zone::SetAside(p), Pos::Top);
        g.emit(Event::Moved {
            player: p,
            card: None,
            to: Zone::SetAside(p),
        });
    }
    let data = Scratch {
        cards,
        ..Scratch::default()
    };
    g.add_lasting(Card::BidingTime, p, None, vec![], Until::Removed, data);
}

fn daimyo_replay(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    let Some(l) = f.end_lasting(g) else {
        return Flow::done();
    };
    let Some(card) = l.data.pick else {
        return Flow::done();
    };
    let chain = l
        .source
        .filter(|&d| g.zone(d) == Zone::InPlay(f.player))
        .into_iter()
        .collect();
    f.pc = 1;
    Flow::Call(Frame::play(f.player, card, false, chain))
}

fn growth(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        let Some(Occasion::Gain { card, .. }) = f.occ else {
            return Flow::done();
        };
        f.s.cost = g.cost_of(card);
    }
    let limit = f.s.cost;
    gain_where(g, f, input, 0, Dest::Discard, |_, c| c.less_than(limit))
}

fn sickness(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["Gain a Curse onto your deck", "Discard 3 cards"], 1, 1)
        }
        1 => {
            f.pc = 3;
            if input.opt() == Some(crate::Opt::Mode(0)) {
                return match g.supply_pile(Card::Curse) {
                    Some(curse) => Flow::Call(Frame::gain(f.player, curse, Dest::DeckTop)),
                    None => Flow::done(),
                };
            }
            f.pc = 2;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, 3, 3, Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

// ---- Kingdom cards ----

fn kitsune(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    loop {
        match f.pc {
            1 => {
                f.pc = 2;
                return f.ask_modes(
                    &[
                        "+$2",
                        "+2 Actions",
                        "Each other player gains a Curse",
                        "Gain a Silver",
                    ],
                    2,
                    2,
                );
            }
            2 => {
                let chosen = input.options();
                let has = |i| chosen.contains(&crate::Opt::Mode(i));
                f.s.n = (0..4).filter(|&i| has(i)).fold(0, |m, i| m | 1 << i);
                if has(0) {
                    g.add_coins(p, 2);
                }
                if has(1) {
                    plus_actions(g, p, 2);
                }
                f.pc = if has(2) { 3 } else { 4 };
                return Flow::Continue;
            }
            3 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    f.pc = 4;
                    continue;
                };
                f.s.i += 1;
                if let Some(curse) = g.supply_pile(Card::Curse) {
                    return Flow::Call(Frame::gain(target, curse, Dest::Discard));
                }
            }
            4 => {
                f.pc = 5;
                if f.s.n & 1 << 3 != 0
                    && let Some(silver) = g.supply_pile(Card::Silver)
                {
                    return Flow::Call(Frame::gain(p, silver, Dest::Discard));
                }
            }
            _ => return Flow::done(),
        }
    }
}

fn mountain_shrine(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        1 => {
            f.pc = 2;
            f.ask_cards(g.hand(f.player).to_vec(), 0, 1, Purpose::Trash)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        3 => {
            f.pc = 4;
            if g.trash().iter().any(|&c| g.is(c, Types::ACTION)) {
                return Flow::Call(Frame::draw(f.player, 2));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn poet(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        1 => {
            f.pc = 2;
            Flow::Call(Frame::reveal(f.player, 1))
        }
        _ => {
            if let Some(c) = input.card()
                && g.cost_of(c).at_most(Cost::coins(3))
            {
                g.put_in_hand(c, f.player);
            }
            Flow::done()
        }
    }
}

fn river_shrine(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        1 => {
            f.pc = 2;
            f.ask_cards(g.hand(f.player).to_vec(), 0, 2, Purpose::Trash)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => {
            if f.player == g.turn.player {
                g.add_lasting(
                    Card::RiverShrine,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
            }
            Flow::done()
        }
    }
}

fn rustic_village(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        1 => {
            if g.hand(f.player).len() < 2 {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_yes_no(Purpose::Discard)
        }
        2 => {
            if !input.is_yes() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_cards(g.hand(f.player).to_vec(), 2, 2, Purpose::Discard)
        }
        3 => {
            f.pc = 4;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        4 => {
            f.pc = 5;
            Flow::Call(Frame::draw(f.player, 1))
        }
        _ => Flow::done(),
    }
}

fn alley(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), 1, 1, Purpose::Discard)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn change(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.player(p).debt > 0 {
                g.add_coins(p, 3);
                return Flow::done();
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
        2 => {
            let trashed = g.cost_of(f.s.pick.expect("set in step 1")).coins;
            let piles = g.supply_piles_where(|_, c| c.coins > trashed);
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => {
            if let Some(gained) = input.card() {
                let trashed = g.cost_of(f.s.pick.expect("set in step 1")).coins;
                let diff = g.cost_of(gained).coins.abs_diff(trashed);
                g.add_debt(p, diff as u32);
            }
            Flow::done()
        }
    }
}

fn gold_mine(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if g.supply_pile(Card::Gold).and_then(|p| g.pile_top(p)).is_none() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match g.supply_pile(Card::Gold) {
                Some(gold) if input.is_yes() => Flow::Call(Frame::gain(f.player, gold, Dest::Discard)),
                _ => Flow::done(),
            }
        }
        _ => {
            if !input.cards().is_empty() {
                g.add_debt(f.player, 4);
            }
            Flow::done()
        }
    }
}

/// Militia's attack, from step 0; finishes at step 2.
fn discard_down_to_3(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            while let Some(&target) = f.targets.get(f.s.i) {
                let hand = g.hand(target).to_vec();
                if hand.len() > 3 {
                    let n = hand.len() - 3;
                    f.pc = 1;
                    return f.ask_cards_for(target, hand, n, n, Purpose::Discard);
                }
                f.s.i += 1;
            }
            f.pc = 2;
            Flow::done()
        }
        1 => {
            f.pc = 3;
            Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()))
        }
        3 => {
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
        _ => Flow::done(),
    }
}

fn rice_broker(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        2 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let mut n = 0;
            if g.is(c, Types::TREASURE) {
                n += 2;
            }
            if g.is(c, Types::ACTION) {
                n += 5;
            }
            f.pc = 3;
            Flow::Call(Frame::draw(f.player, n))
        }
        _ => Flow::done(),
    }
}

fn snake_witch(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand: Vec<Card> = g.hand(p).iter().map(|&c| g.card(c)).collect();
            let duplicates = hand
                .iter()
                .enumerate()
                .any(|(i, c)| hand[i + 1..].contains(c));
            if duplicates {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Reveal)
        }
        1 => {
            if !input.is_yes() {
                return Flow::done();
            }
            let cards = g.hand(p).iter().map(|&c| g.card(c)).collect();
            g.emit(Event::RevealedHand { player: p, cards });
            let this = f.this();
            if g.zone(this) != Zone::InPlay(p) || !g.return_to_pile(this) {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Continue
        }
        _ => {
            let Some(&target) = f.targets.get(f.s.i) else {
                return Flow::done();
            };
            f.s.i += 1;
            match g.supply_pile(Card::Curse) {
                Some(curse) => Flow::Call(Frame::gain(target, curse, Dest::Discard)),
                None => Flow::done(),
            }
        }
    }
}

/// Trash a card from hand, gain one costing up to `extra` more.
fn upgrade(g: &mut Game, f: &mut CardFrame, input: Input, extra: u8) -> Flow {
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
            let limit = f.s.cost.plus_coins(extra);
            gain_where(g, f, input, 2, Dest::Discard, |_, c| c.at_most(limit))
        }
    }
}

// ---- Events ----

fn asceticism(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let max = (g.turn.coins as usize).min(g.hand(p).len());
            if max == 0 {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_number(0, max as i32, Purpose::Trash)
        }
        1 => {
            let n = input.number().unwrap_or(0).max(0) as usize;
            if n == 0 {
                return Flow::done();
            }
            g.turn.coins -= n as u32;
            f.pc = 2;
            f.ask_cards(g.hand(p).to_vec(), n, n, Purpose::Trash)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn continue_event(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            g.ext.rising_sun.continued = Some((p, g.player(p).turns_taken));
            f.pc = 3;
            let piles = g.supply_piles_where(|t, c| {
                t.intersects(Types::ACTION) && !t.intersects(Types::ATTACK) && c.at_most(Cost::coins(4))
            });
            if piles.is_empty() {
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        2 => {
            f.s.pick = input.card();
            f.pc = 3;
            Flow::Continue
        }
        3 => {
            f.pc = 4;
            if g.turn.phase == crate::Phase::Buy && p == g.turn.player {
                return Flow::Call(Frame::window(Occasion::EndBuyPhase { player: p }));
            }
            Flow::Continue
        }
        4 => {
            if p == g.turn.player {
                g.turn.phase = crate::Phase::Action;
            }
            f.pc = 5;
            if let Some(c) = f.s.pick
                && matches!(g.zone(c), Zone::Discard(o) | Zone::Hand(o) if o == p)
            {
                return Flow::Call(Frame::play(p, c, true, vec![]));
            }
            Flow::Continue
        }
        _ => {
            plus_actions(g, p, 1);
            plus_buys(g, p, 1);
            Flow::done()
        }
    }
}

fn credit(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        return gain_where(g, f, input, 0, Dest::Discard, |t, c| {
            t.intersects(Types::ACTION | Types::TREASURE) && c.at_most(Cost::coins(8))
        });
    }
    if let Some(c) = input.card() {
        let debt = g.cost_of(c).coins;
        g.add_debt(f.player, debt as u32);
    }
    Flow::done()
}

fn foresight(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 2;
                return Flow::Continue;
            };
            g.set_aside_revealed(c, p);
            if g.is(c, Types::ACTION) {
                g.add_lasting(
                    Card::Foresight,
                    p,
                    None,
                    vec![],
                    Until::EndOfTurn,
                    Scratch {
                        pick: Some(c),
                        ..Scratch::default()
                    },
                );
                f.pc = 2;
            } else {
                f.s.cards.push(c);
                f.pc = 0;
            }
            Flow::Continue
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)))
        }
        _ => Flow::done(),
    }
}

fn gather(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        let amount = 3 + f.s.i as u8;
        if amount > 5 {
            return Flow::done();
        }
        match f.pc {
            0 => {
                let piles = g.supply_piles_where(|_, c| c == Cost::coins(amount));
                if piles.is_empty() {
                    f.s.i += 1;
                    continue;
                }
                f.pc = 1;
                return f.ask_pile(piles, false, Purpose::Gain);
            }
            1 => {
                f.pc = 2;
                if let Some(pile) = input.take().and_then(Input::pile) {
                    return Flow::Call(Frame::gain(f.player, pile, Dest::Discard));
                }
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
            }
        }
    }
}

fn kintsugi(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
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
            f.s.cost = g.cost_of(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        _ => {
            let gained_gold = g.log().iter().any(|e| {
                matches!(e, Event::Gained { player, card: Card::Gold, .. } if *player == p)
            });
            if f.pc == 2 && !gained_gold {
                return Flow::done();
            }
            let limit = f.s.cost.plus_coins(2);
            gain_where(g, f, input, 2, Dest::Discard, |_, c| c.at_most(limit))
        }
    }
}

fn practice(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let actions = g.playable(f.player, Types::ACTION);
            if actions.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::play(f.player, c, true, vec![]))
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::play(f.player, f.s.pick.expect("set in step 1"), false, vec![]))
        }
        _ => Flow::done(),
    }
}

fn receive_tribute(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.turn.gained.len() < 3 || f.s.i >= 3 {
                return Flow::done();
            }
            let taken: Vec<Card> = f.s.cards.iter().map(|&c| g.card(c)).collect();
            let in_play: Vec<Card> = g.in_play(p).iter().map(|&c| g.card(c)).collect();
            let piles: Vec<PileId> = g
                .supply_piles_where(|t, _| t.intersects(Types::ACTION))
                .into_iter()
                .filter(|&pile| {
                    let top = g.card(g.pile_top(pile).expect("non-empty"));
                    !taken.contains(&top) && !in_play.contains(&top)
                })
                .collect();
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, true, Purpose::Gain)
        }
        1 => {
            let Some(pile) = input.pile() else {
                return Flow::done();
            };
            f.pc = 2;
            Flow::Call(Frame::gain(p, pile, Dest::Discard))
        }
        _ => {
            f.s.cards.extend(input.cards());
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
    }
}

fn sea_trade(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let n = g
                .in_play(p)
                .iter()
                .filter(|&&c| g.is(c, Types::ACTION))
                .count();
            if n == 0 {
                return Flow::done();
            }
            f.s.n = n as i32;
            f.pc = 1;
            Flow::Call(Frame::draw(p, n))
        }
        1 => {
            f.pc = 2;
            f.ask_cards(g.hand(p).to_vec(), 0, f.s.n as usize, Purpose::Trash)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        _ => Flow::done(),
    }
}
