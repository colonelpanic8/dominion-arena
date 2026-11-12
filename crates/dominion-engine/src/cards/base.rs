//! Dominion (2nd edition), its removed 1st-edition cards, and the basic cards.

use super::{CardFrame, Hook, SetRules, whose};
use crate::decision::Purpose;
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, Value};
use crate::game::{Pos, Scratch, Until, Zone};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, Cost, Game, PlayerId, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {}

pub(crate) struct Rules;

const MOAT_REACT: u8 = 0;
const MERCHANT_SILVER: u8 = 0;

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Moat, Hook::Ability(MOAT_REACT)) => {
                if let Some(c) = f.this {
                    g.show(c);
                }
                g.emit(Event::Revealed {
                    player: f.player,
                    card: Card::Moat,
                });
                Flow::Done(Value::Bool(true))
            }
            (Card::Merchant, Hook::Ability(MERCHANT_SILVER)) => {
                g.add_coins(f.player, 1);
                Flow::done()
            }
            (_, Hook::Ability(_)) => return None,
            (Card::Cellar, _) => cellar(g, f, input),
            (Card::Chapel, _) => chapel(g, f, input),
            (Card::Harbinger, _) => harbinger(g, f, input),
            (Card::Merchant, _) => {
                let silvers = g.turn.played_count(f.player, Card::Silver);
                let data = Scratch {
                    n: silvers as i32,
                    ..Scratch::default()
                };
                g.add_lasting(Card::Merchant, f.player, f.this, vec![], Until::EndOfTurn, data);
                Flow::done()
            }
            (Card::Vassal, _) => vassal(g, f, input),
            (Card::Workshop, _) => gain_up_to(g, f, input, 4, Dest::Discard),
            (Card::Bureaucrat, _) => bureaucrat(g, f, input),
            (Card::Militia, _) => militia(g, f, input),
            (Card::Moneylender, _) => moneylender(g, f, input),
            (Card::Poacher, _) => poacher(g, f, input),
            (Card::Remodel, _) => upgrade(g, f, input, None, 2, Dest::Discard),
            (Card::ThroneRoom, _) => throne_room(g, f, input),
            (Card::Bandit, _) => bandit(g, f, input),
            (Card::CouncilRoom, _) => council_room(g, f),
            (Card::Library, _) => library(g, f, input),
            (Card::Mine, _) => upgrade(g, f, input, Some(Types::TREASURE), 3, Dest::Hand),
            (Card::Sentry, _) => sentry(g, f, input),
            (Card::Witch, _) => each_target_gains(g, f, Card::Curse),
            (Card::Artisan, _) => artisan(g, f, input),
            (Card::Adventurer, _) => adventurer(g, f, input),
            (Card::Chancellor, _) => chancellor(g, f, input),
            (Card::Feast, _) => feast(g, f, input),
            (Card::Spy, _) => spy(g, f, input),
            (Card::Thief, _) => thief(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        match *occ {
            Occasion::Attack { target, .. } => {
                for &c in g.hand(target) {
                    if g.card(c) == Card::Moat {
                        out.push(Trigger::new(target, Card::Moat, MOAT_REACT).on(c).optional("reveal it to be unaffected").protects());
                    }
                }
            }
            Occasion::Play { player, card } => {
                let first_silver = g.card(card) == Card::Silver
                    && player == g.turn.player
                    && g.turn.played_count(player, Card::Silver) == 1;
                if first_silver {
                    for l in g.lasting() {
                        if l.card == Card::Merchant && l.owner == player && l.data.n == 0 {
                            out.push(
                                Trigger::new(player, Card::Merchant, MERCHANT_SILVER)
                                    .via_lasting(l.id),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::Gardens).then(|| g.player(p).owned().count() as i32 / 10)
    }
}

fn top_deck(g: &mut Game, p: PlayerId, cards: Vec<crate::CardId>) {
    for c in cards {
        g.top_deck(c, p);
    }
}

/// Those of `cards`, looked at or revealed from `p`'s deck, that are still there: an
/// ability resolving meanwhile may have moved some.
fn on_deck(g: &Game, p: PlayerId, mut cards: Vec<crate::CardId>) -> Vec<crate::CardId> {
    cards.retain(|&c| g.zone(c) == Zone::Deck(p));
    cards
}

fn cellar(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
            let n = hand.len();
            f.ask_cards(hand, 0, n, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            f.s.i = chosen.len();
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::draw(f.player, f.s.i))
        }
        _ => Flow::done(),
    }
}

fn chapel(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), 0, 4, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn harbinger(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            g.look_through_discard(f.player);
            let discard = g.player(f.player).discard.clone();
            f.ask_cards(discard, 0, 1, Purpose::TopDeck)
        }
        _ => {
            top_deck(g, f.player, input.cards());
            Flow::done()
        }
    }
}

fn vassal(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(f.player, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, vec![c]))
        }
        2 => {
            let c = f.s.pick.expect("set in step 1");
            if !g.is(c, Types::ACTION) || g.zone(c) != Zone::Discard(f.player) {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_yes_no(Purpose::Play, format!("Play {}", g.card(c)))
        }
        3 if input.is_yes() => {
            f.pc = 4;
            let c = f.s.pick.expect("set in step 1");
            Flow::Call(Frame::play(f.player, c, true, vec![]))
        }
        _ => Flow::done(),
    }
}

/// "Gain a card costing up to $`limit`" (Workshop, Feast, Artisan's first half).
fn gain_up_to(g: &mut Game, f: &mut CardFrame, input: Input, limit: u8, to: Dest) -> Flow {
    match f.pc {
        0 => {
            let piles = g.piles_costing_up_to(Cost::coins(limit));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, to)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn bureaucrat(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    loop {
        match f.pc {
            0 => {
                f.pc = 1;
                if let Some(silver) = g.supply_pile(Card::Silver) {
                    return Flow::Call(Frame::gain(f.player, silver, Dest::DeckTop));
                }
            }
            1 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                let victory = g.cards_in_hand_where(target, Types::VICTORY);
                if victory.is_empty() {
                    let cards = g.hand(target).iter().map(|&c| g.card(c)).collect();
                    g.emit(Event::RevealedHand {
                        player: target,
                        cards,
                    });
                    f.s.i += 1;
                    continue;
                }
                f.pc = 2;
                return f.ask_cards_for(target, victory, 1, 1, Purpose::TopDeck);
            }
            _ => {
                let target = f.targets[f.s.i];
                let chosen = input.cards();
                for &c in &chosen {
                    g.reveal(target, c);
                }
                top_deck(g, target, chosen);
                f.s.i += 1;
                f.pc = 1;
                return Flow::Continue;
            }
        }
    }
}

fn militia(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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

fn moneylender(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let coppers: Vec<_> = g
                .hand(f.player)
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::Copper)
                .collect();
            if coppers.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(coppers, 0, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, chosen))
        }
        _ => {
            g.add_coins(f.player, 3);
            Flow::done()
        }
    }
}

fn poacher(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            let n = g.empty_supply_piles().min(hand.len());
            if n == 0 {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

/// Trash a card from hand (of `types`, optionally), then gain one costing up to `extra`
/// more (of `types` too).
fn upgrade(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    types: Option<Types>,
    extra: u8,
    to: Dest,
) -> Flow {
    match f.pc {
        0 => {
            let candidates = match types {
                Some(t) => g.cards_in_hand_where(f.player, t),
                None => g.hand(f.player).to_vec(),
            };
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            let min = if types.is_some() { 0 } else { 1 };
            f.ask_cards(candidates, min, 1, Purpose::Trash)
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
            let limit = f.s.cost.plus_coins(extra);
            let piles = g.supply_piles_where(|card_types, cost| {
                cost.at_most(limit) && types.is_none_or(|t| card_types.intersects(t))
            });
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, to)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn throne_room(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            Flow::Call(Frame::play(f.player, c, true, f.chain_for_child()))
        }
        2 => {
            f.pc = 3;
            let c = f.s.pick.expect("set in step 1");
            Flow::Call(Frame::play(f.player, c, false, f.chain_for_child()))
        }
        _ => Flow::done(),
    }
}

fn bandit(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                f.pc = 1;
                if let Some(gold) = g.supply_pile(Card::Gold) {
                    return Flow::Call(Frame::gain(f.player, gold, Dest::Discard));
                }
            }
            1 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                f.pc = 2;
                return Flow::Call(Frame::reveal(target, 2));
            }
            2 => {
                let target = f.targets[f.s.i];
                f.s.cards = input.take().map(Input::cards).unwrap_or_default();
                let treasures: Vec<_> = f
                    .s
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, Types::TREASURE) && g.card(c) != Card::Copper)
                    .collect();
                if treasures.is_empty() {
                    f.pc = 4;
                    continue;
                }
                f.pc = 3;
                return f.ask_cards_for(target, treasures, 1, 1, Purpose::Trash);
            }
            3 => {
                let chosen = input.take().map(Input::cards).unwrap_or_default();
                f.s.cards.retain(|c| !chosen.contains(c));
                f.pc = 4;
                return Flow::Call(Frame::trash(f.targets[f.s.i], chosen));
            }
            4 => {
                f.pc = 5;
                let target = f.targets[f.s.i];
                let rest = on_deck(g, target, std::mem::take(&mut f.s.cards));
                return Flow::Call(Frame::discard(target, rest));
            }
            _ => {
                f.s.i += 1;
                f.pc = 1;
            }
        }
    }
}

fn council_room(g: &mut Game, f: &mut CardFrame) -> Flow {
    match g.others_in_turn_order(f.player).get(f.s.i) {
        Some(&other) => {
            f.s.i += 1;
            Flow::Call(Frame::draw(other, 1))
        }
        None => Flow::done(),
    }
}

fn library(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if g.hand(f.player).len() >= 7 {
                f.pc = 3;
                return Flow::Continue;
            }
            f.pc = 1;
            // One more card, even if the -1 Card token takes the place of a draw.
            Flow::Call(Frame::draw_to(f.player, g.hand(f.player).len() + 1))
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 3;
                return Flow::Continue;
            };
            if g.is(c, Types::ACTION) {
                f.s.pick = Some(c);
                f.pc = 2;
                return f.ask_yes_or(Purpose::SetAside, format!("Set aside {}", g.card(c)), "Keep it");
            }
            f.pc = 0;
            Flow::Continue
        }
        2 => {
            if input.is_yes() {
                let c = f.s.pick.expect("set in step 1");
                g.set_aside(c, f.player);
                f.s.cards.push(c);
            }
            f.pc = 0;
            Flow::Continue
        }
        3 => {
            f.pc = 4;
            Flow::Call(Frame::discard(f.player, std::mem::take(&mut f.s.cards)))
        }
        _ => Flow::done(),
    }
}

fn sentry(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(f.player, 2))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            f.ask_cards(f.s.cards.clone(), 0, f.s.cards.len(), Purpose::Trash)
        }
        2 => {
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = 3;
            Flow::Call(Frame::trash(f.player, chosen))
        }
        3 => {
            f.s.cards = on_deck(g, f.player, std::mem::take(&mut f.s.cards));
            f.pc = 4;
            f.ask_cards(f.s.cards.clone(), 0, f.s.cards.len(), Purpose::Discard)
        }
        4 => {
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = 5;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        5 => {
            f.s.cards = on_deck(g, f.player, std::mem::take(&mut f.s.cards));
            f.pc = 6;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        _ => {
            for &c in input.cards().iter().rev() {
                g.move_card(c, Zone::Deck(f.player), Pos::Top);
            }
            Flow::done()
        }
    }
}

/// Each target gains a `card` from the Supply, in turn order (Witch).
fn each_target_gains(g: &mut Game, f: &mut CardFrame, card: Card) -> Flow {
    let Some(&target) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    f.s.i += 1;
    match g.supply_pile(card) {
        Some(pile) => Flow::Call(Frame::gain(target, pile, Dest::Discard)),
        None => Flow::done(),
    }
}

fn artisan(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        let flow = gain_up_to(g, f, input, 5, Dest::Hand);
        if !matches!(flow, Flow::Done(_)) {
            return flow;
        }
        f.pc = 2;
        return Flow::Continue;
    }
    match f.pc {
        2 => {
            f.pc = 3;
            f.ask_cards(g.hand(f.player).to_vec(), 1, 1, Purpose::TopDeck)
        }
        _ => {
            top_deck(g, f.player, input.cards());
            Flow::done()
        }
    }
}

fn adventurer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if f.s.n >= 2 {
                f.pc = 2;
                return Flow::Continue;
            }
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 2;
                return Flow::Continue;
            };
            g.set_aside_revealed(c, f.player);
            f.s.cards.push(c);
            if g.is(c, Types::TREASURE) {
                f.s.n += 1;
            }
            f.pc = 0;
            Flow::Continue
        }
        2 => {
            let (treasures, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut f.s.cards)
                .into_iter()
                .partition(|&c| g.is(c, Types::TREASURE));
            for c in treasures {
                g.move_card(c, Zone::Hand(f.player), Pos::Top);
            }
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, rest))
        }
        _ => Flow::done(),
    }
}

fn chancellor(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if g.player(f.player).deck.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Discard, "Put your deck into your discard pile")
        }
        _ => {
            if input.is_yes() {
                let deck: Vec<_> = g.player(f.player).deck.iter().rev().copied().collect();
                for c in deck {
                    g.move_card(c, Zone::Discard(f.player), Pos::Top);
                }
            }
            Flow::done()
        }
    }
}

fn feast(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.s.n == 0 {
        f.s.n = 1;
        let this = f.this();
        if g.zone(this) == Zone::InPlay(f.player) {
            return Flow::Call(Frame::trash(f.player, vec![this]));
        }
        return Flow::Continue;
    }
    gain_up_to(g, f, input, 5, Dest::Discard)
}

fn spy(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let victims: Vec<PlayerId> = std::iter::once(f.player)
        .chain(f.targets.iter().copied())
        .collect();
    match f.pc {
        0 => {
            let Some(&target) = victims.get(f.s.i) else {
                return Flow::done();
            };
            f.pc = 1;
            Flow::Call(Frame::reveal(target, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            };
            f.s.pick = Some(c);
            f.pc = 2;
            f.ask_yes_or(Purpose::Discard, format!("Discard {} {}", whose(f.player, victims[f.s.i]), g.card(c)), "Put it back")
        }
        _ => {
            let target = victims[f.s.i];
            f.s.i += 1;
            f.pc = 0;
            if input.is_yes() {
                let c = f.s.pick.expect("set in step 1");
                return Flow::Call(Frame::discard(target, vec![c]));
            }
            Flow::Continue
        }
    }
}

fn thief(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                f.pc = 1;
                return Flow::Call(Frame::reveal(target, 2));
            }
            1 => {
                f.s.cards = input.take().map(Input::cards).unwrap_or_default();
                let treasures: Vec<_> = f
                    .s
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, Types::TREASURE))
                    .collect();
                if treasures.is_empty() {
                    f.pc = 4;
                    continue;
                }
                f.pc = 2;
                return f.ask_cards(treasures, 1, 1, Purpose::Trash);
            }
            2 => {
                let chosen = input.take().map(Input::cards).unwrap_or_default();
                f.s.cards.retain(|c| !chosen.contains(c));
                f.s.pick = chosen.first().copied();
                f.pc = 3;
                return Flow::Call(Frame::trash(f.targets[f.s.i], chosen));
            }
            3 => {
                let c = f.s.pick.expect("set in step 2");
                if g.zone(c) != Zone::Trash {
                    f.pc = 4;
                    continue;
                }
                f.pc = 5;
                return f.ask_yes_no(Purpose::Gain, format!("Gain the trashed {}", g.card(c)));
            }
            5 => {
                f.pc = 4;
                if input.take().is_some_and(|i| i.is_yes()) {
                    let c = f.s.pick.expect("set in step 2");
                    return Flow::Call(Frame::gain_card(f.player, c, Dest::Discard));
                }
            }
            4 => {
                f.pc = 6;
                let target = f.targets[f.s.i];
                let rest = on_deck(g, target, std::mem::take(&mut f.s.cards));
                return Flow::Call(Frame::discard(target, rest));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
            }
        }
    }
}
