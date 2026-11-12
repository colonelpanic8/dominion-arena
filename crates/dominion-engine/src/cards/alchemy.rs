//! Alchemy, with the Potion.

use super::prosperity::{
    choose_and_gain, each_target_gains, reveal_until, supply_piles, this_turn, top_deck,
};
use super::{CardFrame, Hook, SetRules, put_back, whose};
use crate::decision::Purpose;
use crate::event::Dest;
use crate::frame::{Flow, Frame, Input};
use crate::game::{Scratch, Zone};
use crate::trigger::{Occasion, Trigger};
use crate::setup::{self, Setup};
use crate::{Card, CardId, Cost, Game, Kind, PlayerId, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {}

pub(crate) struct Rules;

const ALCHEMIST_TOPDECK: u8 = 0;
const HERBALIST_TOPDECK: u8 = 0;

fn potion_in_play(g: &Game, p: PlayerId) -> bool {
    g.in_play(p).iter().any(|&c| g.card(c) == Card::Potion)
}

/// The Treasures `p` is discarding from play.
fn treasures_leaving_play(g: &Game, p: PlayerId) -> Vec<CardId> {
    g.leaving_play(p)
        .into_iter()
        .filter(|&c| g.is(c, Types::TREASURE))
        .collect()
}

impl SetRules for Rules {
    fn ensure(g: &mut Game, _s: &mut Setup) {
        let potions = |c: Card| c.kind() == Kind::Card && c.cost().is_some_and(|c| c.potions > 0);
        if setup::in_game(g, potions) && g.supply_pile(Card::Potion).is_none() {
            setup::add_pile(g, Card::Potion, 16, true);
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Alchemist, Hook::Ability(ALCHEMIST_TOPDECK)) => {
                let this = f.this();
                if g.zone(this) == Zone::InPlay(f.player) && potion_in_play(g, f.player) {
                    top_deck(g, f.player, this);
                }
                Flow::done()
            }
            (Card::Herbalist, Hook::Ability(HERBALIST_TOPDECK)) => match f.pc {
                0 => {
                    let treasures = treasures_leaving_play(g, f.player);
                    f.pc = 1;
                    f.ask_cards(treasures, 0, 1, Purpose::TopDeck)
                }
                _ => {
                    if let Some(id) = f.lasting {
                        g.remove_lasting(id);
                    }
                    for c in input.cards() {
                        if g.zone(c) == Zone::InPlay(f.player) {
                            top_deck(g, f.player, c);
                        }
                    }
                    Flow::done()
                }
            },
            (_, Hook::Ability(_)) => return None,
            (Card::Alchemist | Card::Herbalist, _) => {
                this_turn(g, f, Scratch::default());
                Flow::done()
            }
            (Card::Apothecary, _) => apothecary(g, f, input),
            (Card::Apprentice, _) => apprentice(g, f, input),
            (Card::Familiar, _) => each_target_gains(g, f, Card::Curse),
            (Card::Golem, _) => golem(g, f, input),
            (Card::PhilosophersStone, _) => {
                let pl = g.player(f.player);
                let n = (pl.deck.len() + pl.discard.len()) / 5;
                g.add_coins(f.player, n as u32);
                Flow::done()
            }

            (Card::ScryingPool, _) => scrying_pool(g, f, input),
            (Card::Transmute, _) => transmute(g, f, input),
            (Card::University, _) => choose_and_gain(
                f,
                input,
                0,
                || {
                    supply_piles(g, |c| {
                        g.is(c, Types::ACTION) && g.cost_of(c).at_most(Cost::coins(5))
                    })
                },
                true,
                Dest::Discard,
            ),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        match *occ {
            Occasion::StartCleanup { .. } => {
                // Whoever played it, on whoever's turn.
                for l in g.lasting() {
                    let player = l.owner;
                    if l.card == Card::Alchemist
                        && let Some(src) = l.source
                        && g.zone(src) == Zone::InPlay(player)
                        && potion_in_play(g, player)
                    {
                        out.push(
                            Trigger::new(player, Card::Alchemist, ALCHEMIST_TOPDECK)
                                .on(src)
                                .via_lasting(l.id)
                                .optional("put it onto your deck"),
                        );
                    }
                }
            }
            Occasion::DiscardFromPlay { player } => {
                if treasures_leaving_play(g, player).is_empty() {
                    return;
                }
                for l in g.lasting() {
                    if l.card == Card::Herbalist && l.owner == player {
                        out.push(
                            Trigger::new(player, Card::Herbalist, HERBALIST_TOPDECK)
                                .via_lasting(l.id)
                                .optional("put one of your Treasures from play onto your deck"),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::Vineyard).then(|| {
            let actions = g
                .player(p)
                .owned()
                .filter(|&c| g.is(c, Types::ACTION))
                .count();
            actions as i32 / 3
        })
    }
}

fn apothecary(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, 4))
        }
        1 => {
            let (take, rest): (Vec<_>, Vec<_>) = input
                .cards()
                .into_iter()
                .partition(|&c| matches!(g.card(c), Card::Copper | Card::Potion));
            for c in take {
                g.put_in_hand(c, f.player);
            }
            if rest.len() < 2 {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_order(rest, Purpose::TopDeck)
        }
        _ => {
            put_back(g, f.player, &input.cards());
            Flow::done()
        }
    }
}

fn apprentice(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, vec![c]))
        }
        2 => {
            let cost = g.cost_of(f.s.pick.expect("set in step 1"));
            let n = cost.coins as usize + if cost.potions > 0 { 2 } else { 0 };
            f.pc = 3;
            Flow::Call(Frame::draw(f.player, n))
        }
        _ => Flow::done(),
    }
}

fn golem_target(g: &Game, c: CardId) -> bool {
    g.is(c, Types::ACTION) && g.card(c) != Card::Golem
}

fn golem(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        return reveal_until(g, f, input, 0, |g, cards| {
            cards.iter().filter(|&&c| golem_target(g, c)).count() == 2
        });
    }
    match f.pc {
        2 => {
            let (actions, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut f.s.cards)
                .into_iter()
                .partition(|&c| golem_target(g, c));
            f.s.cards = actions;
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, rest))
        }
        3 => {
            let cards = &f.s.cards;
            if cards.len() == 2 && g.card(cards[0]) != g.card(cards[1]) {
                f.pc = 4;
                return f.ask_cards(cards.clone(), 1, 1, Purpose::Play);
            }
            f.pc = 5;
            Flow::Continue
        }
        4 => {
            if let Some(first) = input.card()
                && f.s.cards[0] != first
            {
                f.s.cards.reverse();
            }
            f.pc = 5;
            Flow::Continue
        }
        _ => {
            let Some(&c) = f.s.cards.get(f.s.i) else {
                return Flow::done();
            };
            f.s.i += 1;
            if !g.held(f.player, c) {
                return Flow::Continue;
            }
            Flow::Call(Frame::play(f.player, c, true, vec![]))
        }
    }
}

fn scrying_pool(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc >= 3 {
        if f.pc < 5 {
            return reveal_until(g, f, input, 3, |g, cards| {
                cards.last().is_some_and(|&c| !g.is(c, Types::ACTION))
            });
        }
        for c in std::mem::take(&mut f.s.cards) {
            if g.held(f.player, c) {
                g.put_in_hand(c, f.player);
            }
        }
        return Flow::done();
    }
    let victims: Vec<PlayerId> = std::iter::once(f.player)
        .chain(f.targets.iter().copied())
        .collect();
    match f.pc {
        0 => {
            let Some(&victim) = victims.get(f.s.i) else {
                f.pc = 3;
                return Flow::Continue;
            };
            f.pc = 1;
            Flow::Call(Frame::reveal(victim, 1))
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
            let victim = victims[f.s.i];
            f.s.i += 1;
            f.pc = 0;
            let c = f.s.pick.take().expect("set in step 1");
            if input.is_yes() {
                return Flow::Call(Frame::discard(victim, vec![c]));
            }
            put_back(g, victim, &[c]);
            Flow::Continue
        }
    }
}

const TRANSMUTE: [(Types, Card); 3] = [
    (Types::ACTION, Card::Duchy),
    (Types::TREASURE, Card::Transmute),
    (Types::VICTORY, Card::Gold),
];

fn transmute(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            let types = g.types(c);
            for (i, (t, _)) in TRANSMUTE.iter().enumerate() {
                if types.intersects(*t) {
                    f.s.n |= 1 << i;
                }
            }
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, vec![c]))
        }
        _ => {
            while f.s.i < TRANSMUTE.len() {
                let (_, card) = TRANSMUTE[f.s.i];
                let wanted = f.s.n & (1 << f.s.i) != 0;
                f.s.i += 1;
                if wanted && let Some(pile) = g.supply_pile(card) {
                    return Flow::Call(Frame::gain(f.player, pile, Dest::Discard));
                }
            }
            Flow::done()
        }
    }
}
