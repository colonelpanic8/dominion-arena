//! Intrigue (2nd edition) and its removed 1st-edition cards.

use super::{Bonus, CardFrame, Hook, SetRules, mode_mask, take_mode};
use crate::decision::{Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input};
use crate::game::{Pos, Scratch, Until, Zone};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Reveals of Diplomat and Secret Chamber so far. Tags their triggers, so a Reaction
    /// can be revealed again to the same Attack.
    pub reveals: i32,
}

pub(crate) struct Rules;

const DIPLOMAT_REACT: u8 = 0;
const SECRET_CHAMBER_REACT: u8 = 0;

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Diplomat, Hook::Ability(DIPLOMAT_REACT)) => diplomat_react(g, f, input),
            (Card::SecretChamber, Hook::Ability(SECRET_CHAMBER_REACT)) => {
                secret_chamber_react(g, f, input)
            }
            (_, Hook::Ability(_)) => return None,
            (Card::Baron, _) => baron(g, f, input),
            (Card::Bridge, _) | (Card::Coppersmith, _) => {
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
            (Card::Conspirator, _) => conspirator(g, f),
            (Card::Courtier, _) => courtier(g, f, input),
            (Card::Courtyard, _) => courtyard(g, f, input),
            (Card::Diplomat, _) => {
                if g.hand(f.player).len() <= 5 {
                    g.turn.actions += 2;
                }
                Flow::done()
            }
            (Card::Ironworks, _) => ironworks(g, f, input),
            (Card::Lurker, _) => lurker(g, f, input),
            (Card::Masquerade, _) => masquerade(g, f, input),
            (Card::Mill, _) => mill(g, f, input),
            (Card::MiningVillage, _) => mining_village(g, f, input),
            (Card::Minion, _) => minion(g, f, input),
            (Card::Nobles, _) => nobles(g, f, input),
            (Card::Patrol, _) => reveal_and_take(g, f, input, 4, Types::VICTORY | Types::CURSE),
            (Card::Scout, _) => reveal_and_take(g, f, input, 4, Types::VICTORY),
            (Card::Pawn, _) => pawn(g, f, input),
            (Card::Replace, _) => replace(g, f, input),
            (Card::SecretPassage, _) => secret_passage(g, f, input),
            (Card::ShantyTown, _) => shanty_town(g, f),
            (Card::Steward, _) => steward(g, f, input),
            (Card::Swindler, _) => swindler(g, f, input),
            (Card::Torturer, _) => torturer(g, f, input),
            (Card::TradingPost, _) => trading_post(g, f, input),
            (Card::Upgrade, _) => {
                let p = f.player;
                trash_and_gain(g, f, input, 0, p, GainRule::Exactly(1), false)
                    .unwrap_or_else(Flow::done)
            }
            (Card::WishingWell, _) => wishing_well(g, f, input),
            (Card::Saboteur, _) => saboteur(g, f, input),
            (Card::SecretChamber, _) => secret_chamber(g, f, input),
            (Card::Tribute, _) => tribute(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        if let Occasion::Attack { target, .. } = *occ {
            let tag = g.ext.intrigue.reveals;
            let big_hand = g.hand(target).len() >= 5;
            for &c in g.hand(target) {
                let ability = match g.card(c) {
                    Card::Diplomat if big_hand => DIPLOMAT_REACT,
                    Card::SecretChamber => SECRET_CHAMBER_REACT,
                    _ => continue,
                };
                out.push(
                    Trigger::new(target, g.card(c), ability)
                        .on(c)
                        .optional()
                        .data(tag),
                );
            }
        }
    }

    fn modify_cost(g: &Game, _card: Card, _id: Option<CardId>, cost: &mut Cost) {
        let bridges = g.lasting().iter().filter(|l| l.card == Card::Bridge).count();
        if bridges > 0 {
            *cost = cost.minus_coins(bridges.min(255) as u8);
        }
    }

    fn modify_bonus(g: &Game, p: PlayerId, id: CardId, bonus: &mut Bonus) {
        if g.card(id) == Card::Copper {
            let n = g
                .lasting()
                .iter()
                .filter(|l| l.card == Card::Coppersmith && l.owner == p)
                .count();
            bonus.coins += n as u32;
        }
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::Duke).then(|| g.count_owned(p, Card::Duchy) as i32)
    }
}

/// How the cost of a gained card relates to the cost of the trashed one.
#[derive(Clone, Copy, Debug)]
pub(crate) enum GainRule {
    /// Costing up to $n more.
    UpTo(u8),
    /// Costing exactly $n more.
    Exactly(u8),
    /// Cheaper, if the trashed card costs $1 or more (Dismantle).
    Cheaper,
}

impl GainRule {
    pub(crate) fn piles(self, g: &Game, cost: Cost) -> Vec<PileId> {
        match self {
            GainRule::UpTo(n) => g.piles_costing_up_to(cost.plus_coins(n)),
            GainRule::Exactly(n) => {
                let want = cost.plus_coins(n);
                g.supply_piles_where(|_, c| c == want)
            }
            GainRule::Cheaper if cost.coins == 0 => vec![],
            GainRule::Cheaper => g.supply_piles_where(|_, c| c.less_than(cost)),
        }
    }
}

/// Trash a card from `who`'s hand, then gain a Supply card whose cost follows `rule`.
/// Uses `f.pc` from `base` to `base + 5`. Returns `None` once finished, with `f.s.n` = 1 if
/// a card was trashed (0 if not), `f.s.cost` its cost and `f.s.pick` the gained card.
pub(crate) fn trash_and_gain(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    base: u8,
    who: PlayerId,
    rule: GainRule,
    optional: bool,
) -> Option<Flow> {
    let step = f.pc - base;
    match step {
        0 => {
            f.s.n = 0;
            f.s.pick = None;
            let hand = g.hand(who).to_vec();
            if hand.is_empty() {
                f.pc = base + 5;
                return None;
            }
            f.pc = base + 1;
            Some(f.ask_cards_for(who, hand, usize::from(!optional), 1, Purpose::Trash))
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = base + 5;
                return None;
            };
            f.s.n = 1;
            f.s.cost = g.cost_of(c);
            f.pc = base + 2;
            Some(Flow::Call(Frame::trash(who, vec![c])))
        }
        2 => {
            let piles = rule.piles(g, f.s.cost);
            if piles.is_empty() {
                f.pc = base + 5;
                return None;
            }
            f.pc = base + 3;
            Some(f.ask_pile_for(who, piles, false, Purpose::Gain))
        }
        3 => match input.pile() {
            Some(pile) => {
                f.pc = base + 4;
                Some(Flow::Call(Frame::gain(who, pile, Dest::Discard)))
            }
            None => {
                f.pc = base + 5;
                None
            }
        },
        4 => {
            f.s.pick = input.card();
            f.pc = base + 5;
            None
        }
        _ => None,
    }
}

/// Puts `id` (in `p`'s hand or elsewhere) into `p`'s deck with `pos` cards above it.
pub(crate) fn put_into_deck(g: &mut Game, id: CardId, p: PlayerId, pos: usize) {
    g.move_card(id, Zone::Deck(p), Pos::Top);
    let deck = &mut g.players[p.index()].deck;
    let card = deck.pop().expect("just put there");
    let at = deck.len().saturating_sub(pos);
    deck.insert(at, card);
    g.emit(Event::Moved {
        player: p,
        card: None,
        to: Zone::Deck(p),
    });
}

fn gain_from_supply(g: &Game, p: PlayerId, card: Card, to: Dest) -> Option<Frame> {
    g.supply_pile(card).map(|pile| Frame::gain(p, pile, to))
}

fn chosen_modes(input: Input) -> Vec<u8> {
    input
        .options()
        .into_iter()
        .filter_map(|o| match o {
            Opt::Mode(i) => Some(i),
            _ => None,
        })
        .collect()
}

fn baron(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let estates: Vec<_> = g
                .hand(f.player)
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::Estate)
                .collect();
            f.pc = 1;
            if estates.is_empty() {
                return Flow::Continue;
            }
            f.ask_cards(estates, 0, 1, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                f.pc = 3;
                return match gain_from_supply(g, f.player, Card::Estate, Dest::Discard) {
                    Some(frame) => Flow::Call(frame),
                    None => Flow::done(),
                };
            }
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        2 => {
            g.add_coins(f.player, 4);
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn conspirator(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    f.pc = 1;
    if g.turn.played_of(Types::ACTION) >= 3 {
        g.turn.actions += 1;
        return Flow::Call(Frame::draw(f.player, 1));
    }
    Flow::done()
}

fn courtier(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Reveal)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            g.reveal(f.player, c);
            let n = g.types(c).bits().count_ones() as usize;
            f.pc = 2;
            f.ask_modes(&["+1 Action", "+1 Buy", "+$3", "Gain a Gold"], n, n)
        }
        2 => {
            let modes = chosen_modes(input);
            for &m in &modes {
                match m {
                    0 => g.turn.actions += 1,
                    1 => g.turn.buys += 1,
                    2 => g.add_coins(f.player, 3),
                    _ => {}
                }
            }
            f.pc = 3;
            if modes.contains(&3)
                && let Some(frame) = gain_from_supply(g, f.player, Card::Gold, Dest::Discard)
            {
                return Flow::Call(frame);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn courtyard(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), 1, 1, Purpose::TopDeck)
        }
        _ => {
            for c in input.cards() {
                g.top_deck(c, f.player);
            }
            Flow::done()
        }
    }
}

fn diplomat_react(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            g.ext.intrigue.reveals += 1;
            if let Some(c) = f.this {
                g.show(c);
            }
            g.emit(Event::Revealed {
                player: f.player,
                card: Card::Diplomat,
            });
            f.pc = 1;
            Flow::Call(Frame::draw(f.player, 2))
        }
        1 => {
            f.pc = 2;
            f.ask_cards(g.hand(f.player).to_vec(), 3, 3, Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn ironworks(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let piles = g.piles_costing_up_to(Cost::coins(4));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => match input.pile() {
            Some(pile) => {
                f.pc = 2;
                Flow::Call(Frame::gain(f.player, pile, Dest::Discard))
            }
            None => Flow::done(),
        },
        2 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let types = g.types(c);
            if types.contains(Types::ACTION) {
                g.turn.actions += 1;
            }
            if types.contains(Types::TREASURE) {
                g.add_coins(f.player, 1);
            }
            f.pc = 3;
            if types.contains(Types::VICTORY) {
                return Flow::Call(Frame::draw(f.player, 1));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn lurker(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Trash an Action card from the Supply",
                    "Gain an Action card from the trash",
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
            Some(0) => {
                let piles = g.supply_piles_where(|t, _| t.contains(Types::ACTION));
                if piles.is_empty() {
                    return Flow::Continue;
                }
                f.pc = 3;
                f.ask_pile(piles, false, Purpose::Trash)
            }
            Some(_) => {
                let actions: Vec<_> = g
                    .trash()
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, Types::ACTION))
                    .collect();
                if actions.is_empty() {
                    return Flow::Continue;
                }
                f.pc = 4;
                f.ask_cards(actions, 1, 1, Purpose::Gain)
            }
            None => Flow::done(),
        },
        3 => {
            f.pc = 2;
            match input.pile().and_then(|p| g.pile_top(p)) {
                Some(top) => Flow::Call(Frame::trash(f.player, vec![top])),
                None => Flow::Continue,
            }
        }
        _ => {
            f.pc = 2;
            match input.card() {
                Some(c) => Flow::Call(Frame::gain_card(f.player, c, Dest::Discard)),
                None => Flow::Continue,
            }
        }
    }
}

/// The players who pass a card to Masquerade, in turn order from `p`.
fn passers(g: &Game, p: PlayerId) -> Vec<PlayerId> {
    g.players_from(p)
        .into_iter()
        .filter(|&q| !g.hand(q).is_empty())
        .collect()
}

fn masquerade(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let passers = passers(g, f.player);
            match passers.get(f.s.i) {
                Some(&q) => {
                    f.pc = 1;
                    f.ask_cards_for(q, g.hand(q).to_vec(), 1, 1, Purpose::Pass)
                }
                None => {
                    f.pc = 2;
                    Flow::Continue
                }
            }
        }
        1 => {
            f.s.cards.extend(input.card());
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
        2 => {
            let picks = std::mem::take(&mut f.s.cards);
            let owners: Vec<PlayerId> = picks
                .iter()
                .map(|&c| g.zone(c).owner().expect("passed from a hand"))
                .collect();
            if picks.len() > 1 {
                for (k, &c) in picks.iter().enumerate() {
                    let to = owners[(k + 1) % owners.len()];
                    g.move_card(c, Zone::Hand(to), Pos::Top);
                    g.emit(Event::Moved {
                        player: owners[k],
                        card: None,
                        to: Zone::Hand(to),
                    });
                }
            }
            f.pc = 3;
            f.ask_cards(g.hand(f.player).to_vec(), 0, 1, Purpose::Trash)
        }
        3 => {
            f.pc = 4;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn mill(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if g.hand(f.player).is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Discard)
        }
        1 => {
            if !input.is_yes() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(g.hand(f.player).to_vec(), 2, 2, Purpose::Discard)
        }
        2 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        _ => {
            if f.s.n == 2 {
                g.add_coins(f.player, 2);
            }
            Flow::done()
        }
    }
}

fn mining_village(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if g.zone(f.this()) != Zone::InPlay(f.player) {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Trash)
        }
        1 => {
            if !input.is_yes() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, vec![f.this()]))
        }
        _ => {
            g.add_coins(f.player, 2);
            Flow::done()
        }
    }
}

fn minion(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                f.pc = 1;
                return f.ask_modes(
                    &[
                        "+$2",
                        "Discard your hand, +4 Cards; each other player with 5+ cards does too",
                    ],
                    1,
                    1,
                );
            }
            1 => {
                let m = input.take().map(mode_mask).unwrap_or_default();
                if m & 1 != 0 {
                    g.add_coins(f.player, 2);
                }
                if m & 2 == 0 {
                    return Flow::done();
                }
                f.pc = 2;
                return Flow::Call(Frame::discard(f.player, g.hand(f.player).to_vec()));
            }
            2 => {
                f.pc = 3;
                return Flow::Call(Frame::draw(f.player, 4));
            }
            3 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                if g.hand(target).len() >= 5 {
                    f.pc = 4;
                    return Flow::Call(Frame::discard(target, g.hand(target).to_vec()));
                }
                f.s.i += 1;
            }
            4 => {
                f.pc = 5;
                return Flow::Call(Frame::draw(f.targets[f.s.i], 4));
            }
            _ => {
                f.s.i += 1;
                f.pc = 3;
            }
        }
    }
}

fn nobles(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["+3 Cards", "+2 Actions"], 1, 1)
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            if f.s.m & 1 != 0 {
                return Flow::Call(Frame::draw(f.player, 3));
            }
            Flow::Continue
        }
        _ => {
            if f.s.m & 2 != 0 {
                g.turn.actions += 2;
            }
            Flow::done()
        }
    }
}

/// Reveal the top `n` cards, put those of `types` into the hand and the rest back in any
/// order (Patrol, Scout).
fn reveal_and_take(g: &mut Game, f: &mut CardFrame, input: Input, n: usize, types: Types) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, n))
        }
        1 => {
            let (take, rest): (Vec<_>, Vec<_>) =
                input.cards().into_iter().partition(|&c| g.is(c, types));
            for c in take {
                g.put_in_hand(c, f.player);
            }
            f.s.cards = rest;
            f.pc = 2;
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

fn pawn(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["+1 Card", "+1 Action", "+1 Buy", "+$1"], 2, 2)
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            if f.s.m & 1 != 0 {
                return Flow::Call(Frame::draw(f.player, 1));
            }
            Flow::Continue
        }
        _ => {
            if f.s.m & 2 != 0 {
                g.turn.actions += 1;
            }
            if f.s.m & 4 != 0 {
                g.turn.buys += 1;
            }
            if f.s.m & 8 != 0 {
                g.add_coins(f.player, 1);
            }
            Flow::done()
        }
    }
}

fn replace(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if let Some(flow) = trash_and_gain(g, f, input, 0, p, GainRule::UpTo(2), false) {
        return flow;
    }
    if f.pc == 5 {
        f.pc = 6;
        let Some(c) = f.s.pick else {
            return Flow::done();
        };
        let types = g.types(c);
        if types.intersects(Types::ACTION | Types::TREASURE) && g.zone(c) == Zone::Discard(p) {
            g.top_deck(c, p);
        }
        if !types.contains(Types::VICTORY) {
            return Flow::done();
        }
        f.s.i = 0;
    }
    let Some(&target) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    f.s.i += 1;
    match gain_from_supply(g, target, Card::Curse, Dest::Discard) {
        Some(frame) => Flow::Call(frame),
        None => Flow::done(),
    }
}

fn secret_passage(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Move)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            let n = g.player(f.player).deck.len() as i32;
            f.ask_number(0, n, Purpose::Move)
        }
        _ => {
            let c = f.s.pick.expect("set in step 1");
            let pos = input.number().unwrap_or(0).max(0) as usize;
            if g.zone(c) == Zone::Hand(f.player) {
                put_into_deck(g, c, f.player, pos);
            }
            Flow::done()
        }
    }
}

fn shanty_town(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    f.pc = 1;
    let cards = g.hand(f.player).iter().map(|&c| g.card(c)).collect();
    g.emit(Event::RevealedHand {
        player: f.player,
        cards,
    });
    if g.cards_in_hand_where(f.player, Types::ACTION).is_empty() {
        return Flow::Call(Frame::draw(f.player, 2));
    }
    Flow::done()
}

fn steward(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &["+2 Cards", "+$2", "Trash 2 cards from your hand"],
                1,
                1,
            )
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            if f.s.m & 1 != 0 {
                return Flow::Call(Frame::draw(f.player, 2));
            }
            Flow::Continue
        }
        2 => {
            if f.s.m & 2 != 0 {
                g.add_coins(f.player, 2);
            }
            if f.s.m & 4 == 0 {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_cards(g.hand(f.player).to_vec(), 2, 2, Purpose::Trash)
        }
        3 => {
            f.pc = 4;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn swindler(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
                let Some(c) = input.take().and_then(Input::card) else {
                    f.pc = 5;
                    continue;
                };
                f.s.cost = g.cost_of(c);
                f.pc = 2;
                return Flow::Call(Frame::trash(f.targets[f.s.i], vec![c]));
            }
            2 => {
                let cost = f.s.cost;
                let piles = g.supply_piles_where(|_, c| c == cost);
                if piles.is_empty() {
                    f.pc = 5;
                    continue;
                }
                f.pc = 3;
                return f.ask_pile(piles, false, Purpose::Gain);
            }
            3 => {
                f.pc = 5;
                if let Some(pile) = input.take().and_then(Input::pile) {
                    return Flow::Call(Frame::gain(f.targets[f.s.i], pile, Dest::Discard));
                }
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
        }
    }
}

fn torturer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let Some(&target) = f.targets.get(f.s.i) else {
                return Flow::done();
            };
            f.pc = 1;
            f.ask_numbered(
                target,
                &["Discard 2 cards", "Gain a Curse to your hand"],
                1,
                1,
                Purpose::Either,
            )
        }
        1 => {
            let target = f.targets[f.s.i];
            if chosen_modes(input).first() == Some(&0) {
                f.pc = 2;
                return f.ask_cards_for(target, g.hand(target).to_vec(), 2, 2, Purpose::Discard);
            }
            f.pc = 3;
            match gain_from_supply(g, target, Card::Curse, Dest::Hand) {
                Some(frame) => Flow::Call(frame),
                None => Flow::Continue,
            }
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()))
        }
        _ => {
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
    }
}

fn trading_post(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), 2, 2, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, chosen))
        }
        2 => {
            f.pc = 3;
            if f.s.n == 2
                && let Some(frame) = gain_from_supply(g, f.player, Card::Silver, Dest::Hand)
            {
                return Flow::Call(frame);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn wishing_well(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_name()
        }
        1 => {
            f.s.card = input.name();
            f.pc = 2;
            Flow::Call(Frame::reveal(f.player, 1))
        }
        _ => {
            if let Some(c) = input.card()
                && Some(g.card(c)) == f.s.card
            {
                g.put_in_hand(c, f.player);
            }
            Flow::done()
        }
    }
}

fn saboteur(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                if f.targets.get(f.s.i).is_none() {
                    return Flow::done();
                }
                f.pc = 1;
            }
            1 => {
                f.pc = 2;
                return Flow::Call(Frame::reveal(f.targets[f.s.i], 1));
            }
            2 => {
                let target = f.targets[f.s.i];
                let Some(c) = input.take().and_then(Input::card) else {
                    f.pc = 5;
                    continue;
                };
                g.set_aside_revealed(c, target);
                let cost = g.cost_of(c);
                if cost.coins < 3 {
                    f.s.cards.push(c);
                    f.pc = 1;
                    continue;
                }
                f.s.cost = cost;
                f.pc = 3;
                return Flow::Call(Frame::trash(target, vec![c]));
            }
            3 => {
                let piles = g.piles_costing_up_to(f.s.cost.minus_coins(2));
                f.pc = 4;
                if !piles.is_empty() {
                    return f.ask_pile_for(f.targets[f.s.i], piles, true, Purpose::Gain);
                }
            }
            4 => {
                f.pc = 5;
                if let Some(pile) = input.take().and_then(Input::pile) {
                    return Flow::Call(Frame::gain(f.targets[f.s.i], pile, Dest::Discard));
                }
            }
            5 => {
                f.pc = 6;
                let rest = std::mem::take(&mut f.s.cards);
                return Flow::Call(Frame::discard(f.targets[f.s.i], rest));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
        }
    }
}

fn secret_chamber(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            let n = hand.len();
            f.pc = 1;
            f.ask_cards(hand, 0, n, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        _ => {
            g.add_coins(f.player, f.s.n as u32);
            Flow::done()
        }
    }
}

fn secret_chamber_react(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            g.ext.intrigue.reveals += 1;
            if let Some(c) = f.this {
                g.show(c);
            }
            g.emit(Event::Revealed {
                player: f.player,
                card: Card::SecretChamber,
            });
            f.pc = 1;
            Flow::Call(Frame::draw(f.player, 2))
        }
        1 => {
            if f.s.n >= 2 || g.hand(f.player).is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(g.hand(f.player).to_vec(), 1, 1, Purpose::TopDeck)
        }
        _ => {
            for c in input.cards() {
                g.top_deck(c, f.player);
            }
            f.s.n += 1;
            f.pc = 1;
            Flow::Continue
        }
    }
}

fn tribute(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let left = g.left_of(f.player);
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(left, 2))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            Flow::Call(Frame::discard(left, f.s.cards.clone()))
        }
        2 => {
            let mut seen = vec![];
            let mut draw = 0;
            for &c in &f.s.cards {
                if seen.contains(&g.card(c)) {
                    continue;
                }
                seen.push(g.card(c));
                let types = g.types(c);
                if types.contains(Types::ACTION) {
                    g.turn.actions += 2;
                }
                if types.contains(Types::TREASURE) {
                    g.add_coins(f.player, 2);
                }
                if types.contains(Types::VICTORY) {
                    draw += 2;
                }
            }
            f.pc = 3;
            Flow::Call(Frame::draw(f.player, draw))
        }
        _ => Flow::done(),
    }
}
