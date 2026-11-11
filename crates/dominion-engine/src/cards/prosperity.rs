//! Prosperity (2nd edition), its removed 1st-edition cards, Platinum and Colony.

use super::{Bonus, CardFrame, Hook, SetRules, mode_mask};
use crate::decision::{Decision, DecisionKind, Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input};
use crate::game::{Phase, Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Note, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {}

pub(crate) struct Rules;

const WATCHTOWER_REACT: u8 = 0;
const CLERK_START: u8 = 0;
const MINT_GAINED: u8 = 0;
const HOARD_GOLD: u8 = 0;
const COLLECTION_VP: u8 = 0;
const TIARA_TOPDECK: u8 = 0;
const ROYAL_SEAL_TOPDECK: u8 = 0;
const TALISMAN_COPY: u8 = 0;
const GOONS_VP: u8 = 0;

impl SetRules for Rules {
    fn notes(g: &Game, out: &mut Vec<Note>) {
        if setup::in_game(g, |c| c == Card::TradeRoute) {
            out.push(Note::new(Card::TradeRoute).count(g.trade_route_mat() as i32));
        }
        for l in g.lasting() {
            if let (Card::Contraband | Card::WarChest, Some(named)) = (l.card, l.data.card) {
                out.push(Note::new(l.card).player(l.owner).card(named));
            }
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Watchtower, Hook::Ability(WATCHTOWER_REACT)) => watchtower_react(g, f, input),
            (Card::Clerk, Hook::Ability(CLERK_START)) => {
                let this = f.this();
                if f.pc == 0 && g.zone(this) == Zone::Hand(f.player) {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::play(f.player, this, true, vec![])));
                }
                Flow::done()
            }
            (Card::Mint, Hook::Ability(MINT_GAINED)) => {
                if f.pc > 0 {
                    return Some(Flow::done());
                }
                f.pc = 1;
                let treasures: Vec<CardId> = g
                    .in_play(f.player)
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, Types::TREASURE) && !g.is(c, Types::DURATION))
                    .collect();
                Flow::Call(Frame::trash(f.player, treasures))
            }
            (Card::Hoard, Hook::Ability(HOARD_GOLD)) => match g.supply_pile(Card::Gold) {
                Some(gold) if f.pc == 0 => {
                    f.pc = 1;
                    Flow::Call(Frame::gain(f.player, gold, Dest::Discard))
                }
                _ => Flow::done(),
            },
            (Card::Collection, Hook::Ability(COLLECTION_VP))
            | (Card::Goons, Hook::Ability(GOONS_VP)) => {
                g.players[f.player.index()].vp_tokens += 1;
                Flow::done()
            }
            (Card::Tiara, Hook::Ability(TIARA_TOPDECK))
            | (Card::RoyalSeal, Hook::Ability(ROYAL_SEAL_TOPDECK)) => {
                let card = occ_card(f);
                if where_gained(g, f.player, card) {
                    top_deck(g, f.player, card);
                }
                Flow::done()
            }
            (Card::Talisman, Hook::Ability(TALISMAN_COPY)) => {
                let bought = occ_card(f);
                if f.pc > 0 {
                    return Some(Flow::done());
                }
                f.pc = 1;
                match another_copy_in_supply(g, g.printed(bought)) {
                    Some(copy) => Flow::Call(Frame::gain_card(f.player, copy, Dest::Discard)),
                    None => Flow::done(),
                }
            }
            (_, Hook::Ability(_)) => return None,
            (Card::Anvil, _) => anvil(g, f, input),
            (Card::Bank, _) => {
                let n = g
                    .in_play(f.player)
                    .iter()
                    .filter(|&&c| g.is(c, Types::TREASURE))
                    .count();
                g.add_coins(f.player, n as u32);
                Flow::done()
            }
            (Card::Bishop, _) => bishop(g, f, input),
            (Card::Charlatan, _) => each_target_gains(g, f, Card::Curse),
            (Card::City, _) => city(g, f),
            (Card::Clerk, _) => clerk(g, f, input),
            (Card::Collection | Card::Hoard | Card::Quarry, _) => {
                this_turn(g, f, Scratch::default());
                Flow::done()
            }
            (Card::CrystalBall, _) => crystal_ball(g, f, input),
            (Card::Expand, _) => upgrade(g, f, input, 3),
            (Card::Forge, _) => forge(g, f, input),
            (Card::Investment, _) => investment(g, f, input),
            (Card::KingsCourt, _) => play_times(g, f, input, 3, Types::ACTION),
            (Card::Magnate, _) => magnate(g, f),
            (Card::Mint, _) => mint(g, f, input),
            (Card::Monument, _) => {
                g.players[f.player.index()].vp_tokens += 1;
                Flow::done()
            }
            (Card::Rabble, _) => rabble(g, f, input),
            (Card::Tiara, _) => {
                if f.pc == 0 {
                    this_turn(g, f, Scratch::default());
                }
                play_times(g, f, input, 2, Types::TREASURE)
            }
            (Card::Vault, _) => vault(g, f, input),
            (Card::WarChest, _) => war_chest(g, f, input),
            (Card::Watchtower, _) => {
                if f.pc > 0 {
                    return Some(Flow::done());
                }
                f.pc = 1;
                Flow::Call(Frame::draw_to(f.player, 6))
            }
            (Card::Contraband, _) => contraband(g, f, input),
            (Card::CountingHouse, _) => counting_house(g, f, input),
            (Card::Goons, _) => discard_down_to(g, f, input, 3),
            (Card::Loan, _) => loan(g, f, input),
            (Card::Mountebank, _) => mountebank(g, f, input),
            (Card::TradeRoute, _) => trade_route(g, f, input),
            (Card::Venture, _) => venture(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        match *occ {
            Occasion::StartTurn { player } => {
                for &c in g.hand(player) {
                    if g.card(c) == Card::Clerk && super::can_play(g, player, c) {
                        out.push(Trigger::new(player, Card::Clerk, CLERK_START).on(c).optional());
                    }
                }
            }
            Occasion::Gain {
                player,
                card,
                bought,
            } => {
                // One trigger per effect, so each applies once however many come into
                // effect while the gain resolves.
                let lastings = |which: Card| {
                    g.lasting()
                        .iter()
                        .filter(move |l| l.card == which && l.owner == player)
                };
                let in_place = where_gained(g, player, card);
                if in_place
                    && let Some(&w) = g.hand(player).iter().find(|&&c| g.card(c) == Card::Watchtower)
                {
                    out.push(
                        Trigger::new(player, Card::Watchtower, WATCHTOWER_REACT)
                            .on(w)
                            .optional(),
                    );
                }
                if g.card(card) == Card::Mint {
                    out.push(Trigger::new(player, Card::Mint, MINT_GAINED).on(card));
                }
                if bought && g.is(card, Types::VICTORY) {
                    for l in lastings(Card::Hoard) {
                        out.push(Trigger::new(player, Card::Hoard, HOARD_GOLD).via_lasting(l.id));
                    }
                }
                if g.is(card, Types::ACTION) {
                    for l in lastings(Card::Collection) {
                        out.push(
                            Trigger::new(player, Card::Collection, COLLECTION_VP)
                                .via_lasting(l.id)
                                .quiet(),
                        );
                    }
                }
                if in_place && g.zone(card) != Zone::Deck(player) {
                    if lastings(Card::Tiara).next().is_some() {
                        out.push(Trigger::new(player, Card::Tiara, TIARA_TOPDECK).optional());
                    }
                    if let Some(&seal) = g
                        .in_play(player)
                        .iter()
                        .find(|&&c| g.card(c) == Card::RoyalSeal)
                    {
                        out.push(
                            Trigger::new(player, Card::RoyalSeal, ROYAL_SEAL_TOPDECK)
                                .on(seal)
                                .optional(),
                        );
                    }
                }
            }
            Occasion::Buy { player, card, cost } => {
                let copies = !g.is(card, Types::VICTORY) && cost.at_most(Cost::coins(4));
                for &c in g.in_play(player) {
                    match g.card(c) {
                        Card::Talisman if copies => {
                            out.push(Trigger::new(player, Card::Talisman, TALISMAN_COPY).on(c));
                        }
                        Card::Goons => {
                            out.push(Trigger::new(player, Card::Goons, GOONS_VP).on(c).quiet());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn modify_cost(g: &Game, card: Card, id: Option<CardId>, cost: &mut Cost) {
        let is_action = id.map_or(card.is(Types::ACTION), |id| g.is(id, Types::ACTION));
        if is_action {
            let quarries = g
                .lasting()
                .iter()
                .filter(|l| l.card == Card::Quarry)
                .count();
            *cost = cost.minus_coins((2 * quarries).min(255) as u8);
        }
        if card == Card::Peddler && g.turn().phase == Phase::Buy {
            let actions = g
                .in_play(g.turn().player)
                .iter()
                .filter(|&&c| g.is(c, Types::ACTION))
                .count();
            *cost = cost.minus_coins((2 * actions).min(255) as u8);
        }
    }

    fn modify_types(g: &Game, id: CardId, types: &mut Types) {
        if g.card(id) == Card::Curse && charlatan_in_game(g) {
            *types |= Types::TREASURE;
        }
    }

    fn modify_bonus(g: &Game, _p: PlayerId, id: CardId, bonus: &mut Bonus) {
        if g.card(id) == Card::Curse && charlatan_in_game(g) {
            bonus.coins += 1;
        }
    }

    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        let contraband = g.lasting().iter().any(|l| {
            l.card == Card::Contraband && l.owner == p && l.data.card == Some(card)
        });
        let grand_market_blocked = card == Card::GrandMarket
            && g.in_play(p).iter().any(|&c| g.card(c) == Card::Copper);
        !contraband && !grand_market_blocked
    }

    fn setup(g: &mut Game, s: &mut Setup) {
        if !s.joined(Card::TradeRoute) {
            return;
        }
        for i in 0..g.piles.len() {
            let pile = &g.piles[i];
            let victory = pile.in_supply
                && !pile.cards.is_empty()
                && pile.cards.iter().all(|&c| g.is(c, Types::VICTORY));
            g.piles[i].trade_route = victory;
        }
    }
}

/// "In games using this": a Charlatan exists, in the Supply or elsewhere (Black Market).
fn charlatan_in_game(g: &Game) -> bool {
    g.cards.contains(&Card::Charlatan)
}

/// The card a triggered ability's occasion is about.
pub(super) fn occ_card(f: &CardFrame) -> CardId {
    match f.occ {
        Some(
            Occasion::Gain { card, .. }
            | Occasion::WouldGain { card, .. }
            | Occasion::Buy { card, .. }
            | Occasion::Trash { card, .. },
        ) => card,
        _ => panic!("{} ability without a card occasion", f.card),
    }
}

/// Whether `card`, being gained by `p`, is still where it was gained to.
pub(super) fn where_gained(g: &Game, p: PlayerId, card: CardId) -> bool {
    g.gains.last().is_some_and(|st| st.card == card && st.player == p)
        && g.still_where_gained(card)
}

pub(super) fn top_deck(g: &mut Game, p: PlayerId, c: CardId) {
    g.move_card(c, Zone::Deck(p), Pos::Top);
    g.emit(Event::TopDecked { player: p });
}

/// A "this turn" effect of the card being played.
pub(super) fn this_turn(g: &mut Game, f: &CardFrame, data: Scratch) -> u32 {
    g.add_lasting(f.card, f.player, f.this, vec![], Until::EndOfTurn, data)
}

/// The top card of the Supply pile holding copies of `card`, if it is one.
pub(super) fn copy_in_supply(g: &Game, card: Card) -> Option<CardId> {
    let pile = g.home_pile(card).filter(|&p| g.pile(p).in_supply)?;
    g.pile_top(pile).filter(|&top| g.card(top) == card)
}

/// A copy of `card` in the Supply besides the one on top of its pile, which the buy in
/// progress gains: the card under it, if that is one too.
fn another_copy_in_supply(g: &Game, card: Card) -> Option<CardId> {
    let pile = g.home_pile(card).filter(|&p| g.pile(p).in_supply)?;
    match g.pile(pile).cards[..] {
        [.., under, top] if g.card(top) == card && g.card(under) == card => Some(under),
        _ => None,
    }
}

/// Supply piles whose top card passes `ok`.
pub(super) fn supply_piles(g: &Game, ok: impl Fn(CardId) -> bool) -> Vec<PileId> {
    (0..g.piles.len())
        .map(|i| PileId(i as u8))
        .filter(|&id| g.pile(id).in_supply && g.pile_top(id).is_some_and(&ok))
        .collect()
}

/// Steps `base` and `base + 1`: choose a Supply pile among `piles` and gain its top card.
/// Finishes after the gain (or if there is nothing to gain).
pub(super) fn choose_and_gain(
    f: &mut CardFrame,
    input: Input,
    base: u8,
    piles: impl FnOnce() -> Vec<PileId>,
    optional: bool,
    to: Dest,
) -> Flow {
    if f.pc == base {
        let piles = piles();
        if piles.is_empty() {
            return Flow::done();
        }
        f.pc = base + 1;
        return f.ask_pile(piles, optional, Purpose::Gain);
    }
    if f.pc == base + 1 {
        f.pc = base + 2;
        if let Some(pile) = input.pile() {
            return Flow::Call(Frame::gain(f.player, pile, to));
        }
    }
    Flow::done()
}

/// Asks `player` (another player than the card's) to name a card.
fn ask_name_for(f: &CardFrame, player: PlayerId) -> Flow {
    Flow::Ask(Decision {
        player,
        kind: DecisionKind::NameCard,
        purpose: Purpose::Name,
        source: Some(f.card),
        about: vec![],
    })
}

/// Asks `player` to order `cards`, top first.
pub(super) fn ask_order_for(f: &CardFrame, player: PlayerId, cards: Vec<CardId>) -> Flow {
    let n = cards.len();
    Flow::Ask(Decision {
        player,
        kind: DecisionKind::Cards {
            candidates: cards,
            min: n,
            max: n,
            ordered: true,
        },
        purpose: Purpose::TopDeck,
        source: Some(f.card),
        about: vec![],
    })
}

/// Puts `order` (top first) back on top of `p`'s deck.
pub(super) fn put_back(g: &mut Game, p: PlayerId, order: Vec<CardId>) {
    for &c in order.iter().rev() {
        if g.zone(c) == Zone::Deck(p) || g.zone(c) == Zone::SetAside(p) {
            g.move_card(c, Zone::Deck(p), Pos::Top);
        }
    }
}

/// Each target gains a `card` from the Supply, in turn order.
pub(super) fn each_target_gains(g: &mut Game, f: &mut CardFrame, card: Card) -> Flow {
    let Some(&target) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    f.s.i += 1;
    match g.supply_pile(card) {
        Some(pile) => Flow::Call(Frame::gain(target, pile, Dest::Discard)),
        None => Flow::done(),
    }
}

/// "You may play a card of `types` from your hand `n` times" (King's Court, Tiara).
fn play_times(g: &mut Game, f: &mut CardFrame, input: Input, n: i32, types: Types) -> Flow {
    match f.pc {
        0 => {
            let cards = g.playable(f.player, types);
            if cards.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(cards, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Continue
        }
        _ => {
            if f.s.n >= n {
                return Flow::done();
            }
            f.s.n += 1;
            let c = f.s.pick.expect("set in step 1");
            Flow::Call(Frame::play(f.player, c, f.s.n == 1, f.chain_for_child()))
        }
    }
}

/// Trash a card from your hand, then gain a card costing up to $`extra` more.
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
            choose_and_gain(
                f,
                input,
                2,
                || supply_piles(g, |c| g.cost_of(c).at_most(limit)),
                false,
                Dest::Discard,
            )
        }
    }
}

/// Each target discards down to `n` cards in hand.
fn discard_down_to(g: &mut Game, f: &mut CardFrame, input: Input, n: usize) -> Flow {
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

/// Reveal cards from the deck one at a time, setting them aside in `f.s.cards`, until
/// `stop` holds for the cards revealed so far (the last one is kept in `f.s.pick`) or the
/// deck runs out. Steps `base` and `base + 1`;
/// moves on to `base + 2` when done.
pub(super) fn reveal_until(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    base: u8,
    stop: impl Fn(&Game, &[CardId]) -> bool,
) -> Flow {
    if f.pc == base {
        f.pc = base + 1;
        return Flow::Call(Frame::reveal(f.player, 1));
    }
    let Some(c) = input.card() else {
        f.pc = base + 2;
        return Flow::Continue;
    };
    g.set_aside_revealed(c, f.player);
    f.s.cards.push(c);
    if stop(g, &f.s.cards) {
        f.s.pick = Some(c);
        f.pc = base + 2;
    } else {
        f.pc = base;
    }
    Flow::Continue
}

fn last_is_treasure(g: &Game, cards: &[CardId]) -> bool {
    cards.last().is_some_and(|&c| g.is(c, Types::TREASURE))
}

fn anvil(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let treasures = g.cards_in_hand_where(f.player, Types::TREASURE);
            if treasures.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(treasures, 0, 1, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        _ => choose_and_gain(
            f,
            input,
            2,
            || g.piles_costing_up_to(Cost::coins(4)),
            false,
            Dest::Discard,
        ),
    }
}

fn bishop(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                g.players[f.player.index()].vp_tokens += 1;
                let hand = g.hand(f.player).to_vec();
                f.pc = 3;
                if !hand.is_empty() {
                    f.pc = 1;
                    return f.ask_cards(hand, 1, 1, Purpose::Trash);
                }
            }
            1 => {
                let Some(c) = input.take().and_then(Input::card) else {
                    f.pc = 3;
                    continue;
                };
                f.s.cost = g.cost_of(c);
                f.pc = 2;
                return Flow::Call(Frame::trash(f.player, vec![c]));
            }
            2 => {
                g.players[f.player.index()].vp_tokens += (f.s.cost.coins / 2) as i32;
                f.pc = 3;
            }
            3 => {
                let Some(&other) = g.others(f.player).get(f.s.i) else {
                    return Flow::done();
                };
                let hand = g.hand(other).to_vec();
                if hand.is_empty() {
                    f.s.i += 1;
                    continue;
                }
                f.pc = 4;
                return f.ask_cards_for(other, hand, 0, 1, Purpose::Trash);
            }
            _ => {
                let other = g.others(f.player)[f.s.i];
                f.s.i += 1;
                f.pc = 3;
                return Flow::Call(Frame::trash(other, input.take().map(Input::cards).unwrap_or_default()));
            }
        }
    }
}

fn city(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    f.pc = 1;
    let empty = g.empty_supply_piles();
    if empty >= 2 {
        g.add_buys(f.player, 1);
        g.add_coins(f.player, 1);
    }
    if empty >= 1 {
        return Flow::Call(Frame::draw(f.player, 1));
    }
    Flow::done()
}

fn clerk(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            while let Some(&target) = f.targets.get(f.s.i) {
                let hand = g.hand(target).to_vec();
                if hand.len() >= 5 {
                    f.pc = 1;
                    return f.ask_cards_for(target, hand, 1, 1, Purpose::TopDeck);
                }
                f.s.i += 1;
            }
            Flow::done()
        }
        _ => {
            let target = f.targets[f.s.i];
            for c in input.cards() {
                top_deck(g, target, c);
            }
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
    }
}

fn crystal_ball(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            let mut options = vec![Opt::Mode(0), Opt::Mode(1)];
            let mut labels = vec![format!("Trash {}", g.card(c)), format!("Discard {}", g.card(c))];
            if g.is(c, Types::ACTION | Types::TREASURE) {
                options.push(Opt::Mode(2));
                labels.push(format!("Play {}", g.card(c)));
            }
            options.push(Opt::Mode(3));
            labels.push("Leave it".into());
            f.ask_choose_for(f.player, options, labels, 1, 1, Purpose::Either)
                .about(vec![c])
        }
        2 => {
            let c = f.s.pick.expect("set in step 1");
            f.pc = 3;
            match input.opt() {
                Some(Opt::Mode(0)) => Flow::Call(Frame::trash(f.player, vec![c])),
                Some(Opt::Mode(1)) => Flow::Call(Frame::discard(f.player, vec![c])),
                Some(Opt::Mode(2)) => Flow::Call(Frame::play(f.player, c, true, vec![])),
                _ => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn forge(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            let n = hand.len();
            f.pc = 1;
            f.ask_cards(hand, 0, n, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            f.s.n = chosen.iter().map(|&c| g.cost_of(c).coins as i32).sum();
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, chosen))
        }
        _ => {
            let total = Cost::coins(f.s.n.min(255) as u8);
            choose_and_gain(
                f,
                input,
                2,
                || supply_piles(g, |c| g.cost_of(c) == total),
                false,
                Dest::Discard,
            )
        }
    }
}

fn investment(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                f.pc = 2;
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        2 => {
            f.pc = 3;
            f.ask_modes(
                &[
                    "+$1",
                    "Trash this for +1 VP per differently named Treasure in hand",
                ],
                1,
                1,
            )
        }
        3 => {
            let m = mode_mask(input);
            if m & 1 != 0 {
                g.add_coins(f.player, 1);
            }
            if m & 2 == 0 {
                return Flow::done();
            }
            let this = f.this();
            if g.zone(this) != Zone::InPlay(f.player) {
                return Flow::done();
            }
            f.pc = 4;
            Flow::Call(Frame::trash(f.player, vec![this]))
        }
        _ => {
            let p = f.player;
            let cards: Vec<Card> = g.hand(p).iter().map(|&c| g.card(c)).collect();
            g.emit(Event::RevealedHand { player: p, cards });
            let mut names: Vec<Card> = g
                .hand(p)
                .iter()
                .filter(|&&c| g.is(c, Types::TREASURE))
                .map(|&c| g.card(c))
                .collect();
            names.sort();
            names.dedup();
            g.players[p.index()].vp_tokens += names.len() as i32;
            Flow::done()
        }
    }
}

fn magnate(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    f.pc = 1;
    let p = f.player;
    let cards: Vec<Card> = g.hand(p).iter().map(|&c| g.card(c)).collect();
    g.emit(Event::RevealedHand { player: p, cards });
    let n = g.cards_in_hand_where(p, Types::TREASURE).len();
    Flow::Call(Frame::draw(p, n))
}

fn mint(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let treasures = g.cards_in_hand_where(f.player, Types::TREASURE);
            if treasures.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(treasures, 0, 1, Purpose::Reveal)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let card = g.card(c);
            g.reveal(f.player, c);
            f.pc = 2;
            match copy_in_supply(g, card) {
                Some(copy) => Flow::Call(Frame::gain_card(f.player, copy, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn rabble(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                f.pc = 1;
                return Flow::Call(Frame::reveal(target, 3));
            }
            1 => {
                let target = f.targets[f.s.i];
                let (hits, rest): (Vec<_>, Vec<_>) = input
                    .take()
                    .map(Input::cards)
                    .unwrap_or_default()
                    .into_iter()
                    .partition(|&c| g.is(c, Types::ACTION | Types::TREASURE));
                f.s.cards = rest;
                f.pc = 2;
                return Flow::Call(Frame::discard(target, hits));
            }
            2 => {
                let target = f.targets[f.s.i];
                f.s.cards.retain(|&c| g.zone(c) == Zone::Deck(target));
                if f.s.cards.len() > 1 {
                    f.pc = 3;
                    return ask_order_for(f, target, f.s.cards.clone());
                }
                f.s.i += 1;
                f.pc = 0;
            }
            _ => {
                let target = f.targets[f.s.i];
                put_back(g, target, input.take().map(Input::cards).unwrap_or_default());
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
        }
    }
}

fn vault(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                let hand = g.hand(f.player).to_vec();
                let n = hand.len();
                f.pc = 1;
                return f.ask_cards(hand, 0, n, Purpose::Discard);
            }
            1 => {
                let chosen = input.take().map(Input::cards).unwrap_or_default();
                f.s.n = chosen.len() as i32;
                f.pc = 2;
                return Flow::Call(Frame::discard(f.player, chosen));
            }
            2 => {
                g.add_coins(f.player, f.s.n as u32);
                f.pc = 3;
            }
            3 => {
                let Some(&other) = g.others(f.player).get(f.s.i) else {
                    return Flow::done();
                };
                if g.hand(other).is_empty() {
                    f.s.i += 1;
                    continue;
                }
                f.pc = 4;
                return f.ask_yes_no_for(other, Purpose::Discard);
            }
            4 => {
                let other = g.others(f.player)[f.s.i];
                if !input.take().is_some_and(|i| i.is_yes()) {
                    f.s.i += 1;
                    f.pc = 3;
                    continue;
                }
                let hand = g.hand(other).to_vec();
                f.pc = 5;
                return f.ask_cards_for(other, hand, 2, 2, Purpose::Discard);
            }
            5 => {
                let other = g.others(f.player)[f.s.i];
                let chosen = input.take().map(Input::cards).unwrap_or_default();
                f.s.m = chosen.len() as i32;
                f.pc = 6;
                return Flow::Call(Frame::discard(other, chosen));
            }
            6 => {
                let other = g.others(f.player)[f.s.i];
                f.s.i += 1;
                f.pc = 3;
                if f.s.m == 2 {
                    return Flow::Call(Frame::draw(other, 1));
                }
            }
            _ => unreachable!(),
        }
    }
}

fn war_chest(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            ask_name_for(f, g.left_of(f.player))
        }
        1 => {
            let data = Scratch {
                card: input.name(),
                ..Scratch::default()
            };
            this_turn(g, f, data);
            f.pc = 2;
            Flow::Continue
        }
        _ => {
            let p = f.player;
            let named: Vec<Card> = g
                .lasting()
                .iter()
                .filter(|l| l.card == Card::WarChest && l.owner == p)
                .filter_map(|l| l.data.card)
                .collect();
            choose_and_gain(
                f,
                input,
                2,
                || {
                    supply_piles(g, |c| {
                        g.cost_of(c).at_most(Cost::coins(5)) && !named.contains(&g.card(c))
                    })
                },
                false,
                Dest::Discard,
            )
        }
    }
}

fn watchtower_react(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let card = occ_card(f);
    match f.pc {
        0 => {
            if !where_gained(g, f.player, card) {
                return Flow::done();
            }
            if let Some(c) = f.this {
                g.show(c);
            }
            g.emit(Event::Revealed {
                player: f.player,
                card: Card::Watchtower,
            });
            f.pc = 1;
            let name = g.card(card);
            f.ask_choose_for(
                f.player,
                vec![Opt::Mode(0), Opt::Mode(1)],
                vec![format!("Trash {name}"), format!("Put {name} onto your deck")],
                1,
                1,
                Purpose::Either,
            )
        }
        1 => {
            f.pc = 2;
            if !where_gained(g, f.player, card) {
                return Flow::done();
            }
            if input.opt() == Some(Opt::Mode(0)) {
                return Flow::Call(Frame::trash(f.player, vec![card]));
            }
            top_deck(g, f.player, card);
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn contraband(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            ask_name_for(f, g.left_of(f.player))
        }
        _ => {
            let data = Scratch {
                card: input.name(),
                ..Scratch::default()
            };
            this_turn(g, f, data);
            Flow::done()
        }
    }
}

fn counting_house(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let coppers: Vec<CardId> = g
                .player(p)
                .discard
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::Copper)
                .collect();
            let n = coppers.len();
            f.pc = 1;
            f.ask_cards(coppers, 0, n, Purpose::Move)
        }
        _ => {
            for c in input.cards() {
                g.reveal(p, c);
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
    }
}

fn loan(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        return reveal_until(g, f, input, 0, last_is_treasure);
    }
    match f.pc {
        2 => {
            let Some(c) = f.s.pick else {
                f.pc = 4;
                return Flow::Continue;
            };
            f.pc = 3;
            let name = g.card(c);
            f.ask_choose_for(
                f.player,
                vec![Opt::Mode(0), Opt::Mode(1)],
                vec![format!("Discard {name}"), format!("Trash {name}")],
                1,
                1,
                Purpose::Either,
            )
        }
        3 => {
            f.pc = 4;
            if input.opt() == Some(Opt::Mode(1)) {
                let c = f.s.pick.expect("set when revealed");
                f.s.cards.retain(|&x| x != c);
                return Flow::Call(Frame::trash(f.player, vec![c]));
            }
            Flow::Continue
        }
        4 => {
            f.pc = 5;
            Flow::Call(Frame::discard(f.player, std::mem::take(&mut f.s.cards)))
        }
        _ => Flow::done(),
    }
}

fn venture(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        return reveal_until(g, f, input, 0, last_is_treasure);
    }
    match f.pc {
        2 => {
            let pick = f.s.pick;
            let rest: Vec<CardId> = f.s.cards.iter().copied().filter(|&c| Some(c) != pick).collect();
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, rest))
        }
        3 => {
            f.pc = 4;
            match f.s.pick {
                Some(c) if g.zone(c) == Zone::SetAside(f.player) => {
                    Flow::Call(Frame::play(f.player, c, true, vec![]))
                }
                _ => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn mountebank(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        let Some(&target) = f.targets.get(f.s.i) else {
            return Flow::done();
        };
        let curse = g
            .hand(target)
            .iter()
            .copied()
            .find(|&c| g.card(c) == Card::Curse);
        match f.pc {
            0 => {
                if curse.is_some() {
                    f.pc = 1;
                    return f.ask_yes_no_for(target, Purpose::Discard);
                }
                f.pc = 2;
            }
            1 => {
                if input.take().is_some_and(|i| i.is_yes())
                    && let Some(c) = curse
                {
                    f.pc = 4;
                    return Flow::Call(Frame::discard(target, vec![c]));
                }
                f.pc = 2;
            }
            2 => {
                f.pc = 3;
                if let Some(pile) = g.supply_pile(Card::Curse) {
                    return Flow::Call(Frame::gain(target, pile, Dest::Discard));
                }
            }
            3 => {
                f.pc = 4;
                if let Some(pile) = g.supply_pile(Card::Copper) {
                    return Flow::Call(Frame::gain(target, pile, Dest::Discard));
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

fn trade_route(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            f.pc = 2;
            if hand.is_empty() {
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => {
            let n = g.trade_route_mat();
            g.add_coins(f.player, n);
            Flow::done()
        }
    }
}
