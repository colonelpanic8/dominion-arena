//! Dark Ages: its Kingdom cards, the Knights, Ruins, Shelters, Spoils, Madman and
//! Mercenary.

use super::{CardFrame, Hook, SetRules};
use crate::decision::Purpose;
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input};
use crate::game::{Pos, Scratch, Until, Zone};
use crate::setup::{add_mixed_pile, add_pile};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// How many cards the turn's player had gained this turn when their current Buy
    /// phase started: (player, their turn number, count). Kept only while a Hermit
    /// waits for the end of the Buy phase.
    buy_base: Option<(PlayerId, u32, usize)>,
}

pub(crate) struct Rules;

/// "When you trash this" (Cultist, Rats, Squire, ...).
const TRASHED: u8 = 0;
const MARKET_SQUARE_REACT: u8 = 0;
const HOVEL_REACT: u8 = 0;
const BEGGAR_REACT: u8 = 0;
const URCHIN_REACT: u8 = 0;
const DEATH_CART_GAINED: u8 = 0;
const HERMIT_BUY_START: u8 = 0;
const HERMIT_EXCHANGE: u8 = 1;

/// A helper's `next` that finishes the ability instead of continuing.
const DONE: u8 = u8::MAX;

/// Continues the ability at `next`, or finishes it if `next` is [`DONE`].
fn goto(f: &mut CardFrame, next: u8) -> Flow {
    if next == DONE {
        return Flow::done();
    }
    f.pc = next;
    Flow::Continue
}

fn has_trash_ability(card: Card) -> bool {
    matches!(
        card,
        Card::Cultist
            | Card::Rats
            | Card::Squire
            | Card::Catacombs
            | Card::Fortress
            | Card::HuntingGrounds
            | Card::Feodum
            | Card::OvergrownEstate
            | Card::SirVander
    )
}

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (c, Hook::Ability(TRASHED)) if has_trash_ability(c) => trashed(g, f, input),
            (Card::MarketSquare, Hook::Ability(MARKET_SQUARE_REACT)) => {
                market_square(g, f, input)
            }
            (Card::Hovel, Hook::Ability(HOVEL_REACT)) => {
                let this = f.this();
                if f.pc == 0 && g.zone(this) == Zone::Hand(f.player) {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::trash(f.player, vec![this])));
                }
                Flow::done()
            }
            (Card::Beggar, Hook::Ability(BEGGAR_REACT)) => beggar_react(g, f),
            (Card::Urchin, Hook::Ability(URCHIN_REACT)) => urchin_react(g, f),
            (Card::DeathCart, Hook::Ability(DEATH_CART_GAINED)) => {
                gain_copies(g, f, Card::Ruins, 2, Dest::Discard)
            }
            (Card::Hermit, Hook::Ability(HERMIT_BUY_START)) => {
                let p = f.player;
                g.ext.dark_ages.buy_base =
                    Some((p, g.player(p).turns_taken, g.turn.gained.len()));
                Flow::done()
            }
            (Card::Hermit, Hook::Ability(HERMIT_EXCHANGE)) => {
                hermit_exchange(g, f);
                Flow::done()
            }
            (_, Hook::Ability(_)) => return None,
            (Card::Altar, _) => altar(g, f, input),
            (Card::Armory, _) => gain_pile(g, f, input, &up_to(4), false, Dest::DeckTop, 0, DONE),
            (Card::BandOfMisfits, _) => band_of_misfits(g, f, input),
            (Card::BanditCamp, _) => gain_copies(g, f, Card::Spoils, 1, Dest::Discard),
            (Card::Beggar, _) => gain_copies(g, f, Card::Copper, 3, Dest::Hand),
            (Card::Catacombs, _) => catacombs(g, f, input),
            (Card::Count, _) => count(g, f, input),
            (Card::Counterfeit, _) => play_twice_and_trash(g, f, input, Types::TREASURE),
            (Card::Cultist, _) => cultist(g, f, input),
            (Card::DeathCart, _) => death_cart(g, f, input),
            (Card::Forager, _) => forager(g, f, input),
            (Card::Graverobber, _) => graverobber(g, f, input),
            (Card::Hermit, _) => hermit(g, f, input),
            (Card::Ironmonger, _) => ironmonger(g, f, input),
            (Card::JunkDealer, _) => trash_from_hand(g, f, input),
            (Card::Madman, _) => madman(g, f),
            (Card::Marauder, _) => marauder(g, f),
            (Card::Mercenary, _) => mercenary(g, f, input),
            (Card::Mystic, _) => mystic(g, f, input),
            (Card::Pillage, _) => pillage(g, f, input),
            (Card::PoorHouse, _) => {
                poor_house(g, f);
                Flow::done()
            }
            (Card::Procession, _) => play_twice_and_trash(g, f, input, Types::ACTION),
            (Card::Rats, _) => rats(g, f, input),
            (Card::Rebuild, _) => rebuild(g, f, input),
            (Card::Rogue, _) => rogue(g, f, input),
            (Card::Sage, _) => sage(g, f, input),
            (Card::Scavenger, _) => scavenger(g, f, input),
            (Card::Spoils, _) => {
                let this = f.this();
                if g.zone(this) == Zone::InPlay(f.player) {
                    g.return_to_pile(this);
                }
                Flow::done()
            }
            (Card::Squire, _) => squire(g, f, input),
            (Card::Storeroom, _) => storeroom(g, f, input),
            (Card::Survivors, _) => survivors(g, f, input),
            (Card::Urchin, _) => discard_down(g, f, input, 4, 0, DONE),
            (Card::Vagrant, _) => vagrant(g, f, input),
            (Card::WanderingMinstrel, _) => wandering_minstrel(g, f, input),
            (c, _) if c.def().pile == Some(Card::Knights) => knight(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        match *occ {
            Occasion::Trash { player, card } => {
                let kind = g.card(card);
                if has_trash_ability(kind) {
                    out.push(Trigger::new(player, kind, TRASHED).on(card));
                }
                if let Some(owner) = g.trashed_from(card).and_then(Zone::owner) {
                    for &c in g.hand(owner) {
                        if g.card(c) == Card::MarketSquare {
                            out.push(
                                Trigger::new(owner, Card::MarketSquare, MARKET_SQUARE_REACT)
                                    .on(c)
                                    .optional(),
                            );
                        }
                    }
                }
            }
            Occasion::Gain { player, card, .. } => {
                if g.card(card) == Card::DeathCart {
                    out.push(Trigger::new(player, Card::DeathCart, DEATH_CART_GAINED).on(card));
                }
                if g.is(card, Types::VICTORY) {
                    for &c in g.hand(player) {
                        if g.card(c) == Card::Hovel {
                            out.push(Trigger::new(player, Card::Hovel, HOVEL_REACT).on(c).optional());
                        }
                    }
                }
            }
            Occasion::Attack { target, .. } => {
                for &c in g.hand(target) {
                    if g.card(c) == Card::Beggar {
                        out.push(Trigger::new(target, Card::Beggar, BEGGAR_REACT).on(c).optional());
                    }
                }
            }
            Occasion::Play { player, card } => {
                if g.is(card, Types::ATTACK) {
                    for &c in g.in_play(player) {
                        if c != card && g.card(c) == Card::Urchin {
                            out.push(Trigger::new(player, Card::Urchin, URCHIN_REACT).on(c).optional());
                        }
                    }
                }
            }
            Occasion::StartBuyPhase { player } => {
                if hermits(g, player).next().is_some() {
                    out.push(Trigger::new(player, Card::Hermit, HERMIT_BUY_START).quiet());
                }
            }
            Occasion::EndBuyPhase { player } => {
                let base = g.ext.dark_ages.buy_base;
                let gained_none = base
                    == Some((player, g.player(player).turns_taken, g.turn.gained.len()));
                let madman_left = g
                    .pile_of(Card::Madman)
                    .is_some_and(|m| g.pile_top(m).is_some());
                if gained_none && madman_left {
                    let mut seen = vec![];
                    for (id, source) in hermits(g, player) {
                        // A Hermit played twice is still one card to exchange.
                        if g.zone(source) == Zone::InPlay(player) && !seen.contains(&source) {
                            seen.push(source);
                            out.push(
                                Trigger::new(player, Card::Hermit, HERMIT_EXCHANGE)
                                    .on(source)
                                    .via_lasting(id),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::Feodum).then(|| g.count_owned(p, Card::Silver) as i32 / 3)
    }

    fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
        match card {
            Card::Knights => {
                let mut knights = Card::Knights.pile_members();
                g.shuffle(&mut knights);
                add_mixed_pile(g, Card::Knights, &knights, true);
                true
            }
            Card::Rats => {
                add_pile(g, Card::Rats, 20, true);
                true
            }
            _ => false,
        }
    }

    fn setup_added_piles(g: &mut Game, _cards: &[Card]) {
        Self::setup(g);
    }

    fn setup(g: &mut Game) {
        let present = |g: &Game, pred: &dyn Fn(Card) -> bool| {
            g.piles
                .iter()
                .any(|pile| pile.cards.iter().any(|&c| pred(g.printed(c))))
        };
        if present(g, &|c| c.is(Types::LOOTER)) && g.pile_of(Card::Ruins).is_none() {
            let mut ruins: Vec<Card> = Card::Ruins
                .pile_members()
                .into_iter()
                .flat_map(|c| [c; 10])
                .collect();
            g.shuffle(&mut ruins);
            ruins.truncate(10 * (g.num_players() - 1));
            add_mixed_pile(g, Card::Ruins, &ruins, true);
        }
        let extra = [
            (
                Card::Spoils,
                15,
                &[Card::BanditCamp, Card::Marauder, Card::Pillage][..],
            ),
            (Card::Madman, 10, &[Card::Hermit][..]),
            (Card::Mercenary, 10, &[Card::Urchin][..]),
        ];
        for (card, size, users) in extra {
            if present(g, &|c| users.contains(&c)) && g.pile_of(card).is_none() {
                add_pile(g, card, size, false);
            }
        }
    }
}

/// The Hermit lastings `p` has this turn: (lasting id, the Hermit).
fn hermits(g: &Game, p: PlayerId) -> impl Iterator<Item = (u32, CardId)> + '_ {
    g.lasting()
        .iter()
        .filter(move |l| l.card == Card::Hermit && l.owner == p)
        .filter_map(|l| l.source.map(|s| (l.id, s)))
}

/// "Costing from $3 to $6": no Potion or Debt in it.
fn three_to_six(cost: Cost) -> bool {
    (3..=6).contains(&cost.coins) && cost.potions == 0 && cost.debt == 0
}

fn up_to(coins: u8) -> impl Fn(&Game) -> Vec<PileId> {
    move |g| g.piles_costing_up_to(Cost::coins(coins))
}

fn top_of(g: &Game, card: Card) -> Option<PileId> {
    g.pile_of(card).filter(|&p| g.pile_top(p).is_some())
}

/// Choose one of `piles(g)` and gain its top card to `dest`. Runs at pcs `base..base + 3`,
/// then continues at `next`.
#[allow(clippy::too_many_arguments)]
fn gain_pile(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    piles: &dyn Fn(&Game) -> Vec<PileId>,
    optional: bool,
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
            f.ask_pile(piles, optional, Purpose::Gain)
        }
        1 => match input.pile() {
            Some(pile) => {
                f.pc = base + 2;
                Flow::Call(Frame::gain(f.player, pile, dest))
            }
            None => {
                goto(f, next)
            }
        },
        _ => {
            goto(f, next)
        }
    }
}

/// Gain `n` copies of `card` from its pile, one at a time; done after.
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

/// Each target discards down to `n` cards, in turn order. Runs at pcs `base` and
/// `base + 1`, then continues at `next`.
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

/// Each target gains the top card of `card`'s pile, in turn order. Then continues at `next`.
fn each_target_gains(g: &mut Game, f: &mut CardFrame, card: Card, next: u8) -> Flow {
    if let Some(&target) = f.targets.get(f.s.i) {
        f.s.i += 1;
        if let Some(pile) = top_of(g, card) {
            return Flow::Call(Frame::gain(target, pile, Dest::Discard));
        }
        return Flow::Continue;
    }
    f.s.i = 0;
    goto(f, next)
}

/// The Knights' and Rogue's attack: each target reveals their top 2 cards, trashes one
/// costing from $3 to $6 and discards the rest. Sets `f.s.m` if a Knight was trashed.
/// Runs at pcs `base..base + 5`, then continues at `next`.
fn trashing_attack(g: &mut Game, f: &mut CardFrame, input: Input, base: u8, next: u8) -> Flow {
    match f.pc - base {
        0 => {
            let Some(&target) = f.targets.get(f.s.i) else {
                f.s.i = 0;
                return goto(f, next);
            };
            f.pc = base + 1;
            Flow::Call(Frame::reveal(target, 2))
        }
        1 => {
            let target = f.targets[f.s.i];
            let revealed = input.cards();
            // Held out of the deck, so cards drawn when one is trashed (Cultist) aren't these.
            for &c in &revealed {
                g.move_card(c, Zone::SetAside(target), Pos::Top);
            }
            let candidates: Vec<CardId> = revealed
                .iter()
                .copied()
                .filter(|&c| three_to_six(g.cost_of(c)))
                .collect();
            f.s.cards = revealed;
            if candidates.is_empty() {
                f.pc = base + 3;
                return Flow::Continue;
            }
            f.pc = base + 2;
            f.ask_cards_for(target, candidates, 1, 1, Purpose::Trash)
        }
        2 => {
            let target = f.targets[f.s.i];
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            if chosen.iter().any(|&c| g.is(c, Types::KNIGHT)) {
                f.s.m = 1;
            }
            f.pc = base + 3;
            Flow::Call(Frame::trash(target, chosen))
        }
        3 => {
            let target = f.targets[f.s.i];
            let rest: Vec<CardId> = std::mem::take(&mut f.s.cards)
                .into_iter()
                .filter(|&c| g.zone(c) == Zone::SetAside(target))
                .collect();
            f.pc = base + 4;
            Flow::Call(Frame::discard(target, rest))
        }
        _ => {
            f.s.i += 1;
            f.pc = base;
            Flow::Continue
        }
    }
}

fn trashed(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let this = f.this();
    match f.card {
        Card::Cultist if f.pc == 0 => {
            f.pc = 1;
            Flow::Call(Frame::draw(p, 3))
        }
        Card::Rats | Card::OvergrownEstate if f.pc == 0 => {
            f.pc = 1;
            Flow::Call(Frame::draw(p, 1))
        }
        Card::Fortress => {
            if g.zone(this) == Zone::Trash {
                g.put_in_hand(this, p);
                g.emit(Event::Moved {
                    player: p,
                    card: Some(Card::Fortress),
                    to: Zone::Hand(p),
                });
            }
            Flow::done()
        }
        Card::Squire => gain_pile(
            g,
            f,
            input,
            &|g| g.supply_piles_where(|t, _| t.contains(Types::ATTACK)),
            false,
            Dest::Discard,
            0,
            DONE,
        ),
        Card::Catacombs => {
            let cost = g.cost_of(this);
            gain_pile(
                g,
                f,
                input,
                &move |g| g.supply_piles_where(|_, c| c.less_than(cost)),
                false,
                Dest::Discard,
                0,
                DONE,
            )
        }
        Card::HuntingGrounds => match f.pc {
            0 => {
                f.pc = 1;
                f.ask_modes(&["Gain a Duchy", "Gain 3 Estates"], 1, 1)
            }
            1 => {
                f.s.m = match input.opt() {
                    Some(crate::Opt::Mode(1)) => 3,
                    _ => 1,
                };
                f.s.card = Some(if f.s.m == 3 {
                    Card::Estate
                } else {
                    Card::Duchy
                });
                f.pc = 2;
                Flow::Continue
            }
            _ => {
                let card = f.s.card.expect("set in step 1");
                gain_copies(g, f, card, f.s.m, Dest::Discard)
            }
        },
        Card::Feodum => gain_copies(g, f, Card::Silver, 3, Dest::Discard),
        Card::SirVander => gain_copies(g, f, Card::Gold, 1, Dest::Discard),
        _ => Flow::done(),
    }
}

fn market_square(g: &mut Game, f: &mut CardFrame, _input: Input) -> Flow {
    let (p, this) = (f.player, f.this());
    match f.pc {
        0 => {
            if g.zone(this) != Zone::Hand(p) {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::discard(p, vec![this]))
        }
        1 => {
            f.pc = 2;
            match top_of(g, Card::Gold).filter(|_| g.zone(this) == Zone::Discard(p)) {
                Some(gold) => Flow::Call(Frame::gain(p, gold, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn beggar_react(g: &mut Game, f: &mut CardFrame) -> Flow {
    let (p, this) = (f.player, f.this());
    match f.pc {
        0 => {
            if g.zone(this) != Zone::Hand(p) {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::discard(p, vec![this]))
        }
        1 | 2 => {
            if f.pc == 1 && g.zone(this) != Zone::Discard(p) {
                return Flow::done();
            }
            let dest = if f.pc == 1 {
                Dest::DeckTop
            } else {
                Dest::Discard
            };
            f.pc += 1;
            match top_of(g, Card::Silver) {
                Some(silver) => Flow::Call(Frame::gain(p, silver, dest)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn urchin_react(g: &mut Game, f: &mut CardFrame) -> Flow {
    let (p, this) = (f.player, f.this());
    match f.pc {
        0 => {
            if g.zone(this) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::trash(p, vec![this]))
        }
        1 => {
            f.pc = 2;
            match top_of(g, Card::Mercenary) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn hermit_exchange(g: &mut Game, f: &mut CardFrame) {
    let (p, this) = (f.player, f.this());
    let Some(madman) = top_of(g, Card::Madman) else {
        return;
    };
    if g.zone(this) != Zone::InPlay(p) || !g.return_to_pile(this) {
        return;
    }
    let top = g.pile_top(madman).expect("checked above");
    g.move_card(top, Zone::Discard(p), Pos::Top);
    g.emit(Event::Moved {
        player: p,
        card: Some(Card::Madman),
        to: Zone::Discard(p),
    });
}

fn altar(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), 1, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        2..=4 => gain_pile(g, f, input, &up_to(5), false, Dest::Discard, 2, 9),
        _ => Flow::done(),
    }
}

fn band_of_misfits(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let limit = g.cost_of(f.this());
            let piles = g.supply_piles_where(|t, cost| {
                t.contains(Types::ACTION)
                    && !t.intersects(Types::COMMAND | Types::DURATION)
                    && cost.less_than(limit)
            });
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Play)
        }
        1 => {
            f.pc = 2;
            match input.pile().and_then(|pile| g.pile_top(pile)) {
                Some(top) => Flow::Call(Frame::play(f.player, top, false, f.chain_for_child())),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn catacombs(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 3))
        }
        1 => {
            f.s.cards = input.cards();
            f.pc = 2;
            f.ask_modes(&["Put them into your hand", "Discard them and +3 Cards"], 1, 1)
        }
        2 => {
            let cards: Vec<CardId> = std::mem::take(&mut f.s.cards)
                .into_iter()
                .filter(|&c| g.zone(c) == Zone::Deck(p))
                .collect();
            if input.opt() == Some(crate::Opt::Mode(0)) {
                for c in cards {
                    g.put_in_hand(c, p);
                }
                return Flow::done();
            }
            f.pc = 3;
            Flow::Call(Frame::discard(p, cards))
        }
        3 => {
            f.pc = 4;
            Flow::Call(Frame::draw(p, 3))
        }
        _ => Flow::done(),
    }
}

fn count(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Discard 2 cards",
                    "Put a card from your hand onto your deck",
                    "Gain a Copper",
                ],
                1,
                1,
            )
        }
        1 => {
            let hand = g.hand(p).to_vec();
            match input.opt() {
                Some(crate::Opt::Mode(0)) => {
                    f.pc = 2;
                    f.ask_cards(hand, 2, 2, Purpose::Discard)
                }
                Some(crate::Opt::Mode(1)) => {
                    f.pc = 3;
                    f.ask_cards(hand, 1, 1, Purpose::TopDeck)
                }
                _ => {
                    f.pc = 4;
                    match top_of(g, Card::Copper) {
                        Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                        None => Flow::Continue,
                    }
                }
            }
        }
        2 => {
            f.pc = 4;
            Flow::Call(Frame::discard(p, input.cards()))
        }
        3 => {
            for c in input.cards() {
                g.top_deck(c, p);
            }
            f.pc = 4;
            Flow::Continue
        }
        4 => {
            f.pc = 5;
            f.ask_modes(&["+$3", "Trash your hand", "Gain a Duchy"], 1, 1)
        }
        5 => {
            f.pc = 6;
            match input.opt() {
                Some(crate::Opt::Mode(0)) => {
                    g.add_coins(p, 3);
                    Flow::done()
                }
                Some(crate::Opt::Mode(1)) => Flow::Call(Frame::trash(p, g.hand(p).to_vec())),
                _ => match top_of(g, Card::Duchy) {
                    Some(duchy) => Flow::Call(Frame::gain(p, duchy, Dest::Discard)),
                    None => Flow::done(),
                },
            }
        }
        _ => Flow::done(),
    }
}

/// Counterfeit and Procession: "You may play a non-Duration `types` card from your hand
/// twice. Trash it." Procession then gains an Action costing exactly $1 more.
fn play_twice_and_trash(g: &mut Game, f: &mut CardFrame, input: Input, types: Types) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let candidates: Vec<CardId> = g
                .playable(p, types)
                .into_iter()
                .filter(|&c| !g.is(c, Types::DURATION))
                .collect();
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(candidates, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::play(p, c, true, f.chain_for_child()))
        }
        2 => {
            f.pc = 3;
            let c = f.s.pick.expect("set in step 1");
            Flow::Call(Frame::play(p, c, false, f.chain_for_child()))
        }
        3 => {
            f.pc = 4;
            let c = f.s.pick.expect("set in step 1");
            if g.zone(c) == Zone::InPlay(p) {
                return Flow::Call(Frame::trash(p, vec![c]));
            }
            Flow::Continue
        }
        4..=6 if f.card == Card::Procession => {
            let c = f.s.pick.expect("set in step 1");
            let target = g.cost_of(c).plus_coins(1);
            gain_pile(
                g,
                f,
                input,
                &move |g| {
                    g.supply_piles_where(|t, cost| t.contains(Types::ACTION) && cost == target)
                },
                false,
                Dest::Discard,
                4,
                9,
            )
        }
        _ => Flow::done(),
    }
}

fn cultist(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => each_target_gains(g, f, Card::Ruins, 1),
        1 => {
            let cultists: Vec<CardId> = g
                .playable(p, Types::ACTION)
                .into_iter()
                .filter(|&c| g.card(c) == Card::Cultist)
                .collect();
            if cultists.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(cultists, 0, 1, Purpose::Play)
        }
        2 => {
            f.pc = 3;
            match input.card() {
                Some(c) => Flow::Call(Frame::play(p, c, true, vec![])),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn death_cart(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let (p, this) = (f.player, f.this());
    match f.pc {
        0 => {
            let mut candidates: Vec<CardId> = g.cards_in_hand_where(p, Types::ACTION);
            if g.zone(this) == Zone::InPlay(p) {
                candidates.insert(0, this);
            }
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(candidates, 0, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(p, chosen))
        }
        _ => {
            g.add_coins(p, 5);
            Flow::done()
        }
    }
}

fn forager(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(p).to_vec(), 1, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        _ => {
            let names: BTreeSet<Card> = g
                .trash()
                .iter()
                .filter(|&&c| g.is(c, Types::TREASURE))
                .map(|&c| g.card(c))
                .collect();
            g.add_coins(p, names.len() as u32);
            Flow::done()
        }
    }
}

fn trash_from_hand(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), 1, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn graverobber(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Gain a card from the trash costing from $3 to $6, onto your deck",
                    "Trash an Action card from your hand and gain a card costing up to $3 more",
                ],
                1,
                1,
            )
        }
        1 => {
            if input.opt() == Some(crate::Opt::Mode(1)) {
                let actions = g.cards_in_hand_where(p, Types::ACTION);
                if actions.is_empty() {
                    return Flow::done();
                }
                f.pc = 3;
                return f.ask_cards(actions, 1, 1, Purpose::Trash);
            }
            let cards: Vec<CardId> = g
                .trash()
                .iter()
                .copied()
                .filter(|&c| three_to_six(g.cost_of(c)))
                .collect();
            if cards.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(cards, 1, 1, Purpose::Gain)
        }
        2 => {
            f.pc = 9;
            match input.card() {
                Some(c) => Flow::Call(Frame::gain_card(p, c, Dest::DeckTop)),
                None => Flow::done(),
            }
        }
        3 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.cost = g.cost_of(c);
            f.pc = 4;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        4..=6 => {
            let limit = f.s.cost.plus_coins(3);
            gain_pile(
                g,
                f,
                input,
                &move |g| g.piles_costing_up_to(limit),
                false,
                Dest::Discard,
                4,
                9,
            )
        }
        _ => Flow::done(),
    }
}

fn hermit(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let pl = g.player(p);
            let candidates: Vec<CardId> = pl
                .discard
                .iter()
                .chain(&pl.hand)
                .copied()
                .filter(|&c| !g.is(c, Types::TREASURE))
                .collect();
            f.pc = 1;
            f.ask_cards(candidates, 0, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        2..=4 => gain_pile(g, f, input, &up_to(3), false, Dest::Discard, 2, 5),
        _ => {
            let stamp = (p, g.player(p).turns_taken);
            let fresh = g
                .ext
                .dark_ages
                .buy_base
                .is_some_and(|(q, t, _)| (q, t) == stamp);
            if g.turn.phase == crate::Phase::Buy && !fresh {
                g.ext.dark_ages.buy_base = Some((p, stamp.1, g.turn.gained.len()));
            }
            g.add_lasting(
                Card::Hermit,
                p,
                f.this,
                vec![],
                Until::EndOfTurn,
                Scratch::default(),
            );
            Flow::done()
        }
    }
}

fn ironmonger(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            f.ask_yes_no(Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            if input.is_yes() {
                let c = f.s.pick.expect("set in step 1");
                return Flow::Call(Frame::discard(p, vec![c]));
            }
            Flow::Continue
        }
        3 => {
            let c = f.s.pick.expect("set in step 1");
            let types = g.types(c);
            if types.contains(Types::ACTION) {
                g.turn.actions += 1;
            }
            if types.contains(Types::TREASURE) {
                g.add_coins(p, 1);
            }
            f.pc = 4;
            if types.contains(Types::VICTORY) {
                return Flow::Call(Frame::draw(p, 1));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn madman(g: &mut Game, f: &mut CardFrame) -> Flow {
    let (p, this) = (f.player, f.this());
    if f.pc == 0 && g.zone(this) == Zone::InPlay(p) && g.return_to_pile(this) {
        f.pc = 1;
        return Flow::Call(Frame::draw(p, g.hand(p).len()));
    }
    Flow::done()
}

fn marauder(g: &mut Game, f: &mut CardFrame) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            match top_of(g, Card::Spoils) {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        1 => each_target_gains(g, f, Card::Ruins, 2),
        _ => Flow::done(),
    }
}

fn mercenary(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.hand(p).is_empty() {
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
            f.ask_cards(g.hand(p).to_vec(), 2, 2, Purpose::Trash)
        }
        2 => {
            let chosen = input.cards();
            if chosen.len() < 2 {
                f.pc = 9;
                return Flow::Call(Frame::trash(p, chosen));
            }
            f.pc = 3;
            Flow::Call(Frame::trash(p, chosen))
        }
        3 => {
            f.pc = 4;
            Flow::Call(Frame::draw(p, 2))
        }
        4 => {
            g.add_coins(p, 2);
            f.pc = 5;
            Flow::Continue
        }
        5 | 6 => discard_down(g, f, input, 3, 5, 9),
        _ => Flow::done(),
    }
}

fn mystic(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
        _ => {
            if let Some(c) = input.card()
                && Some(g.card(c)) == f.s.card
                && g.zone(c) == Zone::Deck(p)
            {
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
    }
}

fn pillage(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let (p, this) = (f.player, f.this());
    match f.pc {
        0 => {
            if g.zone(this) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::trash(p, vec![this]))
        }
        1 => {
            if g.zone(this) != Zone::Trash {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Continue
        }
        2 => {
            if f.s.n < 2 {
                f.s.n += 1;
                if let Some(pile) = top_of(g, Card::Spoils) {
                    return Flow::Call(Frame::gain(p, pile, Dest::Discard));
                }
            }
            f.pc = 3;
            Flow::Continue
        }
        3 => {
            while let Some(&target) = f.targets.get(f.s.i) {
                let hand = g.hand(target).to_vec();
                if hand.len() >= 5 {
                    let cards = hand.iter().map(|&c| g.card(c)).collect();
                    g.emit(Event::RevealedHand {
                        player: target,
                        cards,
                    });
                    f.pc = 4;
                    return f.ask_cards(hand, 1, 1, Purpose::Discard);
                }
                f.s.i += 1;
            }
            Flow::done()
        }
        _ => {
            let target = f.targets[f.s.i];
            f.s.i += 1;
            f.pc = 3;
            Flow::Call(Frame::discard(target, input.cards()))
        }
    }
}

fn poor_house(g: &mut Game, f: &mut CardFrame) {
    let p = f.player;
    let cards = g.hand(p).iter().map(|&c| g.card(c)).collect();
    g.emit(Event::RevealedHand { player: p, cards });
    let treasures = g.cards_in_hand_where(p, Types::TREASURE).len() as u32;
    if p == g.turn.player {
        g.turn.coins -= treasures.min(g.turn.coins);
    }
}

fn rats(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            match top_of(g, Card::Rats) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        1 => {
            let others: Vec<CardId> = g
                .hand(p)
                .iter()
                .copied()
                .filter(|&c| g.card(c) != Card::Rats)
                .collect();
            if others.is_empty() {
                let cards = g.hand(p).iter().map(|&c| g.card(c)).collect();
                g.emit(Event::RevealedHand { player: p, cards });
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(others, 1, 1, Purpose::Trash)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        _ => Flow::done(),
    }
}

/// Reveal cards one at a time, holding them aside, until `stop` matches one. Runs at pcs
/// `base` and `base + 1`; the matching card (if any) is left in `f.s.pick` and the rest in
/// `f.s.cards`. Then continues at `next`.
fn reveal_until(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    stop: &dyn Fn(&Game, CardId) -> bool,
    base: u8,
    next: u8,
) -> Flow {
    let p = f.player;
    if f.pc == base {
        f.pc = base + 1;
        return Flow::Call(Frame::reveal(p, 1));
    }
    let Some(c) = input.card() else {
        return goto(f, next);
    };
    g.move_card(c, Zone::SetAside(p), Pos::Top);
    if stop(g, c) {
        f.s.pick = Some(c);
        f.pc = next;
    } else {
        f.s.cards.push(c);
        f.pc = base;
    }
    Flow::Continue
}

fn discard_held(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    let rest: Vec<CardId> = std::mem::take(&mut f.s.cards)
        .into_iter()
        .filter(|&c| g.zone(c) == Zone::SetAside(p))
        .collect();
    Flow::Call(Frame::discard(p, rest))
}

fn rebuild(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_name()
        }
        1 => {
            f.s.card = input.name();
            f.pc = 2;
            Flow::Continue
        }
        2 | 3 => {
            let named = f.s.card;
            reveal_until(
                g,
                f,
                input,
                &move |g, c| g.is(c, Types::VICTORY) && Some(g.card(c)) != named,
                2,
                4,
            )
        }
        4 => {
            f.pc = 5;
            discard_held(g, f)
        }
        5 => {
            let Some(c) = f.s.pick.filter(|&c| g.zone(c) == Zone::SetAside(p)) else {
                return Flow::done();
            };
            f.s.cost = g.cost_of(c);
            f.pc = 6;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        6..=8 => {
            let limit = f.s.cost.plus_coins(3);
            gain_pile(
                g,
                f,
                input,
                &move |g| {
                    g.supply_piles_where(|t, cost| t.contains(Types::VICTORY) && cost.at_most(limit))
                },
                false,
                Dest::Discard,
                6,
                9,
            )
        }
        _ => Flow::done(),
    }
}

fn sage(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 | 1 => reveal_until(g, f, input, &|g, c| g.cost_of(c).coins >= 3, 0, 2),
        2 => {
            if let Some(c) = f.s.pick.filter(|&c| g.zone(c) == Zone::SetAside(p)) {
                g.put_in_hand(c, p);
            }
            f.pc = 3;
            discard_held(g, f)
        }
        _ => Flow::done(),
    }
}

fn rogue(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let cards: Vec<CardId> = g
                .trash()
                .iter()
                .copied()
                .filter(|&c| three_to_six(g.cost_of(c)))
                .collect();
            if cards.is_empty() {
                f.pc = 10;
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(cards, 1, 1, Purpose::Gain)
        }
        1 => {
            f.pc = 20;
            match input.card() {
                Some(c) => Flow::Call(Frame::gain_card(f.player, c, Dest::Discard)),
                None => Flow::done(),
            }
        }
        10..=14 => trashing_attack(g, f, input, 10, 20),
        _ => Flow::done(),
    }
}

fn scavenger(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 2;
            if g.player(p).deck.is_empty() {
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Discard)
        }
        1 => {
            if input.is_yes() {
                let deck: Vec<CardId> = g.player(p).deck.iter().rev().copied().collect();
                for c in deck {
                    g.move_card(c, Zone::Discard(p), Pos::Top);
                }
            }
            f.pc = 2;
            Flow::Continue
        }
        2 => {
            let discard = g.player(p).discard.clone();
            f.pc = 3;
            f.ask_cards(discard, 1, 1, Purpose::TopDeck)
        }
        _ => {
            for c in input.cards() {
                g.top_deck(c, p);
            }
            Flow::done()
        }
    }
}

fn squire(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["+2 Actions", "+2 Buys", "Gain a Silver"], 1, 1)
        }
        1 => {
            f.pc = 2;
            match input.opt() {
                Some(crate::Opt::Mode(0)) => g.turn.actions += 2,
                Some(crate::Opt::Mode(1)) => g.turn.buys += 2,
                _ => {
                    if let Some(silver) = top_of(g, Card::Silver) {
                        return Flow::Call(Frame::gain(p, silver, Dest::Discard));
                    }
                }
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn storeroom(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 | 3 => {
            let hand = g.hand(p).to_vec();
            let n = hand.len();
            f.pc += 1;
            f.ask_cards(hand, 0, n, Purpose::Discard)
        }
        1 | 4 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc += 1;
            Flow::Call(Frame::discard(p, chosen))
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::draw(p, f.s.n as usize))
        }
        _ => {
            g.add_coins(p, f.s.n as u32);
            Flow::done()
        }
    }
}

fn survivors(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 2))
        }
        1 => {
            f.s.cards = input.cards();
            if f.s.cards.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_modes(&["Discard them", "Put them back in any order"], 1, 1)
        }
        2 => {
            let cards: Vec<CardId> = std::mem::take(&mut f.s.cards)
                .into_iter()
                .filter(|&c| g.zone(c) == Zone::Deck(p))
                .collect();
            if input.opt() == Some(crate::Opt::Mode(0)) {
                f.pc = 4;
                return Flow::Call(Frame::discard(p, cards));
            }
            f.pc = 3;
            f.ask_order(cards, Purpose::TopDeck)
        }
        3 => {
            for c in input.cards().into_iter().rev() {
                g.move_card(c, Zone::Deck(p), Pos::Top);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn vagrant(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 1))
        }
        _ => {
            if let Some(c) = input.card()
                && g.is(c, Types::CURSE | Types::RUINS | Types::SHELTER | Types::VICTORY)
            {
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
    }
}

fn wandering_minstrel(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 3))
        }
        1 => {
            let (actions, rest): (Vec<CardId>, Vec<CardId>) =
                input.cards().into_iter().partition(|&c| g.is(c, Types::ACTION));
            f.s.cards = actions;
            f.pc = 2;
            Flow::Call(Frame::discard(p, rest))
        }
        2 => {
            let actions: Vec<CardId> = std::mem::take(&mut f.s.cards)
                .into_iter()
                .filter(|&c| g.zone(c) == Zone::Deck(p))
                .collect();
            f.pc = 3;
            f.ask_order(actions, Purpose::TopDeck)
        }
        _ => {
            for c in input.cards().into_iter().rev() {
                g.move_card(c, Zone::Deck(p), Pos::Top);
            }
            Flow::done()
        }
    }
}

fn knight(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let (p, this) = (f.player, f.this());
    match f.pc {
        0 => {
            f.pc = 10;
            match f.card {
                Card::DameAnna => {
                    f.pc = 1;
                    f.ask_cards(g.hand(p).to_vec(), 0, 2, Purpose::Trash)
                }
                Card::DameNatalie => {
                    f.pc = 2;
                    Flow::Continue
                }
                Card::SirMichael => {
                    f.pc = 5;
                    Flow::Continue
                }
                _ => Flow::Continue,
            }
        }
        1 => {
            f.pc = 10;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        2..=4 => gain_pile(g, f, input, &up_to(3), true, Dest::Discard, 2, 10),
        5 | 6 => discard_down(g, f, input, 3, 5, 10),
        10..=14 => trashing_attack(g, f, input, 10, 20),
        20 => {
            f.pc = 21;
            if f.s.m == 1 && g.zone(this) == Zone::InPlay(p) {
                return Flow::Call(Frame::trash(p, vec![this]));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}
