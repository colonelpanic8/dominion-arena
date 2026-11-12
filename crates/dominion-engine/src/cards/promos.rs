//! Promotional cards: Black Market, Stash, Prince, Captain, Church, Dismantle, Envoy,
//! Governor, Marchland, Sauna / Avanto, Walled Village and the Event Summon.

use super::intrigue::{GainRule, trash_and_gain};
use super::{CardFrame, Hook, SetRules, due_durations, duration_trigger, looking, mode_mask};
use crate::Note;
use crate::decision::Purpose;
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, From as GainFrom, Input, PAY_OTHERWISE, Value};
use crate::game::{Lasting, Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// The Black Market deck (not in the Supply), if Black Market is in the game.
    pub black_market: Option<PileId>,
}

pub(crate) struct Rules;

/// How many cards the Black Market deck is made of (at least 15 by the rules; Dominion
/// Online uses up to 60).
const BLACK_MARKET_SIZE: usize = 60;

const PRINCE_PLAY: u8 = 0;
const CAPTAIN_NEXT: u8 = 0;
const CHURCH_NEXT: u8 = 0;
const MARCHLAND_GAIN: u8 = 0;
const SAUNA_SILVER: u8 = 0;
const STASH_SHUFFLE: u8 = 0;
const SUMMON_PLAY: u8 = 0;
const WALLED_VILLAGE_TOPDECK: u8 = 0;

impl SetRules for Rules {
    fn lasting_ends(_g: &Game, l: &Lasting) -> Option<Until> {
        (l.card == Card::Prince).then_some(Until::Removed)
    }

    fn notes(g: &Game, out: &mut Vec<Note>) {
        // Which cards are in the Black Market deck is public, not their order.
        if let Some(pile) = g.ext.promos.black_market {
            let mut cards: Vec<Card> = g.pile(pile).cards.iter().map(|&c| g.card(c)).collect();
            cards.sort_by_key(|c| c.name());
            for chunk in cards.chunk_by(|a, b| a == b) {
                let note = Note::new(Card::BlackMarket).pile(pile).card(chunk[0]);
                out.push(note.count(chunk.len() as i32));
            }
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Prince, Hook::Ability(PRINCE_PLAY)) => prince_play(g, f),
            (Card::Captain, Hook::Ability(CAPTAIN_NEXT)) => {
                if f.pc == 0 {
                    f.end_lasting(g);
                }
                captain_play(g, f, input)
            }
            (Card::Church, Hook::Ability(CHURCH_NEXT)) => church_next(g, f, input),
            (Card::Marchland, Hook::Ability(MARCHLAND_GAIN)) => marchland_gain(g, f, input),
            (Card::Sauna, Hook::Ability(SAUNA_SILVER)) => trash_from_hand(g, f, input, 1),
            (Card::Stash, Hook::Ability(STASH_SHUFFLE)) => stash_shuffle(g, f, input),
            (Card::Summon, Hook::Ability(SUMMON_PLAY)) => summon_play(g, f),
            (Card::WalledVillage, Hook::Ability(WALLED_VILLAGE_TOPDECK)) => {
                let this = f.this();
                if g.zone(this) == Zone::InPlay(f.player) {
                    g.top_deck(this, f.player);
                }
                Flow::done()
            }
            (_, Hook::Ability(_)) => return None,
            (Card::BlackMarket, _) => black_market(g, f, input),
            (Card::Captain, _) => {
                if f.pc == 0 {
                    f.add_duration(g, Scratch::default());
                }
                captain_play(g, f, input)
            }
            (Card::Church, _) => church(g, f, input),
            (Card::Dismantle, _) => dismantle(g, f, input),
            (Card::Envoy, _) => envoy(g, f, input),
            (Card::Governor, _) => governor(g, f, input),
            (Card::Prince, _) => prince(g, f, input),
            (Card::Sauna, _) => sauna_avanto(g, f, input, Card::Avanto),
            (Card::Avanto, _) => sauna_avanto(g, f, input, Card::Sauna),
            (Card::Summon, _) => summon(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        match *occ {
            Occasion::StartTurn { player } => {
                // Prince plays its card at the start of every turn, even one that set it up.
                for l in g.lasting() {
                    if l.card == Card::Prince
                        && l.owner == player
                        && l.data
                            .pick
                            .is_some_and(|c| g.zone(c) == Zone::SetAside(player))
                    {
                        out.push(duration_trigger(l, PRINCE_PLAY));
                    }
                }
                for l in due_durations(g, occ) {
                    let ability = match l.card {
                        Card::Captain => CAPTAIN_NEXT,
                        Card::Church => CHURCH_NEXT,
                        Card::Summon => SUMMON_PLAY,
                        _ => continue,
                    };
                    out.push(duration_trigger(l, ability));
                }
            }
            Occasion::Play { player, card } if g.card(card) == Card::Silver => {
                if g.hand(player).is_empty() {
                    return;
                }
                for l in g.lasting() {
                    if l.card == Card::Sauna && l.owner == player {
                        out.push(
                            Trigger::new(player, Card::Sauna, SAUNA_SILVER)
                                .via_lasting(l.id)
                                .optional("trash a card from your hand"),
                        );
                    }
                }
            }
            Occasion::Gain { player, card, .. } if g.card(card) == Card::Marchland => {
                out.push(Trigger::new(player, Card::Marchland, MARCHLAND_GAIN).on(card));
            }
            Occasion::Shuffled { player, .. } => {
                if let Some(&stash) = g
                    .shuffled(player)
                    .iter()
                    .find(|&&c| g.card(c) == Card::Stash)
                {
                    out.push(
                        Trigger::new(player, Card::Stash, STASH_SHUFFLE)
                            .on(stash)
                            .optional("put it anywhere in your deck"),
                    );
                }
            }
            Occasion::StartCleanup { .. } => {
                // Whoever played it, on whoever's turn.
                for player in g.players_from(g.turn.player) {
                    let in_play = g.in_play(player);
                    for &c in in_play {
                        if g.card(c) != Card::WalledVillage {
                            continue;
                        }
                        let others = in_play
                            .iter()
                            .filter(|&&o| o != c && g.is(o, Types::ACTION))
                            .count();
                        if others <= 1 {
                            out.push(
                                Trigger::new(player, Card::WalledVillage, WALLED_VILLAGE_TOPDECK)
                                    .on(c)
                                    .optional("put it onto your deck"),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::Marchland).then(|| {
            let victory = g
                .player(p)
                .owned()
                .filter(|&c| g.is(c, Types::VICTORY))
                .count();
            victory as i32 / 3
        })
    }

    fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
        if card != Card::Sauna {
            return false;
        }
        let mut cards = vec![Card::Sauna; 5];
        cards.extend([Card::Avanto; 5]);
        setup::add_mixed_pile(g, Card::Sauna, &cards, true);
        true
    }

    fn ensure(g: &mut Game, s: &mut Setup) {
        if g.ext.promos.black_market.is_some() || !setup::in_game(g, |c| c == Card::BlackMarket) {
            return;
        }
        let legal: Vec<Card> = Card::ALL
            .iter()
            .copied()
            .filter(|&c| black_market_eligible(g, c))
            .collect();
        let deck = s.choose_or_random(g, Card::BlackMarket, legal, 1..=BLACK_MARKET_SIZE, |c| {
            c.def().kingdom && !c.is(Types::TRAVELLER | Types::GATHERING) && c != Card::Vampire
        });
        let home = |c: Card| c.def().pile.unwrap_or(c);
        if let Some((i, _)) = deck
            .iter()
            .enumerate()
            .find(|&(i, &c)| deck[..i].iter().any(|&d| home(d) == home(c)))
        {
            s.reject(Card::BlackMarket, deck[i]);
        }
        let pile = setup::add_mixed_pile(g, Card::BlackMarket, &deck, false);
        g.piles[pile.index()].hidden = true;
        g.piles[pile.index()].holder = true;
        g.ext.promos.black_market = Some(pile);
    }
}

/// Whether `card` can go in the Black Market deck: a card of an unused Kingdom pile, one per
/// pile (a Knight, a Castle, either half of a split pile). Setup adds whatever the deck's
/// cards use, as for any card in the game (the Potion pile, a Bane, Traveller piles), though
/// a card from the deck has no pile to exchange with or to hold a Gathering's VP. A random
/// deck takes only cards that name their pile, and leaves out Travellers, Gatherings and
/// Vampire, whose exchange or Gathering ability never works from there.
fn black_market_eligible(g: &Game, card: Card) -> bool {
    let def = card.def();
    card.kind() == Kind::Card
        && def.cost.is_some()
        && (def.kingdom || def.pile.is_some_and(|pile| pile.def().kingdom))
        && setup::unused(g, card)
}

fn gain_from_supply(g: &Game, p: PlayerId, card: Card, to: Dest) -> Option<Frame> {
    g.supply_pile(card).map(|pile| Frame::gain(p, pile, to))
}

/// "You may trash a card from your hand" (asked once the player has opted in).
fn trash_from_hand(g: &mut Game, f: &mut CardFrame, input: Input, min: usize) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), min, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

/// Whether `p` could buy `id` right now, paying its cost or some other way (Animal Fair).
fn affordable(g: &Game, p: PlayerId, id: CardId) -> bool {
    let (cost, kind) = (g.cost_of(id), g.card(id));
    let (coins, potions) = g.funds(p);
    g.player(p).debt == 0
        && ((cost.coins as u32 <= coins && cost.potions as u32 <= potions)
            || super::pays_otherwise(g, p, kind))
        && super::can_buy(g, p, kind)
}

fn black_market(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(bm) = g.ext.promos.black_market else {
        return Flow::done();
    };
    let p = f.player;
    match f.pc {
        0 => {
            let pile = &g.pile(bm).cards;
            let revealed: Vec<CardId> = pile.iter().rev().take(3).copied().collect();
            for &c in &revealed {
                g.reveal(p, c);
            }
            f.s.cards = revealed;
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            let treasures = g.playable(p, Types::TREASURE);
            if treasures.is_empty() {
                f.pc = 3;
                return Flow::Continue;
            }
            f.pc = 2;
            f.ask_cards(treasures, 0, 1, Purpose::Play)
        }
        2 => match input.card() {
            Some(c) => {
                f.pc = 1;
                Flow::Call(Frame::play(p, c, true, vec![]))
            }
            None => {
                f.pc = 3;
                Flow::Continue
            }
        },
        3 => {
            let in_deck = |g: &Game, c: CardId| g.zone(c) == Zone::Pile(bm);
            // Coffers can be spent at any time in the player's turn, and this is the only
            // moment between seeing these cards and paying (or overpaying) for one.
            let coffers = g.player(p).coffers;
            if f.s.m & 2 == 0
                && coffers > 0
                && g.turn_player() == Some(p)
                && f.s.cards.iter().any(|&c| in_deck(g, c))
            {
                f.s.m |= 2;
                f.pc = 14;
                return f
                    .ask_number(0, coffers as i32, Purpose::Other)
                    .question("spend how many Coffers first?");
            }
            let (debt, coins) = (g.player(p).debt, g.funds(p).0);
            if debt > 0 && coins > 0 && f.s.m & 1 == 0 {
                f.s.m |= 1;
                f.pc = 13;
                return f
                    .ask_number(0, debt.min(coins) as i32, Purpose::Other)
                    .question("pay off how much Debt first?");
            }
            let buyable: Vec<CardId> =
                f.s.cards
                    .iter()
                    .copied()
                    .filter(|&c| in_deck(g, c) && affordable(g, p, c))
                    .collect();
            if buyable.is_empty() {
                f.pc = 5;
                return Flow::Continue;
            }
            f.pc = 4;
            f.ask_cards(buyable, 0, 1, Purpose::Gain).question("buy")
        }
        4 => match input.card() {
            Some(c) => {
                f.s.pick = Some(c);
                f.s.cards.retain(|&x| x != c);
                f.pc = 10;
                let kind = g.card(c);
                if super::pays_otherwise(g, p, kind) {
                    let pay = CardFrame::new(kind, Hook::Ability(PAY_OTHERWISE), p, Some(c));
                    return Flow::Call(Frame::Card(Box::new(pay)));
                }
                Flow::Continue
            }
            None => {
                f.pc = 5;
                Flow::Continue
            }
        },
        10 => {
            let c = f.s.pick.expect("set in step 4");
            let cost = match input {
                Input::Return(Value::Bool(true)) => Cost::ZERO,
                _ => g.cost_of(c),
            };
            let kind = g.card(c);
            g.emit(Event::Bought {
                player: p,
                card: kind,
            });
            f.s.i = g.log().len();
            // Off-turn, a player has no $ or Potions to pay with (`affordable`), and their
            // purchase is none of the turn's.
            if p == g.turn.player {
                g.turn.coins -= cost.coins as u32;
                g.turn.potions -= cost.potions as u32;
                g.turn.bought_cards.push(kind);
            }
            g.add_debt(p, cost.debt as u32);
            f.pc = 11;
            if kind.def().overpay {
                return Flow::Call(Frame::overpay(p, c, f.s.i));
            }
            Flow::Continue
        }
        11 => {
            f.pc = 12;
            let card = f.s.pick.expect("set in step 4");
            let cost = g.cost_of(card);
            Flow::Call(Frame::window(Occasion::Buy {
                player: p,
                card,
                cost,
            }))
        }
        12 => {
            f.pc = 5;
            let c = f.s.pick.expect("set in step 4");
            if g.zone(c) != Zone::Pile(bm) {
                return Flow::Continue;
            }
            Flow::Call(Frame::Gain {
                player: p,
                from: GainFrom::Card(c),
                to: Dest::Discard,
                bought: true,
                buy: Some(f.s.i),
                card: None,
                at: None,
                pc: 0,
            })
        }
        13 => {
            let n = input.number().unwrap_or(0).max(0) as u32;
            g.players[p.index()].debt -= n;
            g.turn.coins -= n;
            f.pc = 3;
            Flow::Continue
        }
        14 => {
            let n = input.number().unwrap_or(0).max(0) as u32;
            g.players[p.index()].coffers -= n;
            g.get_coins(p, n);
            f.pc = 3;
            Flow::Continue
        }
        5 => {
            let rest: Vec<CardId> =
                f.s.cards
                    .iter()
                    .copied()
                    .filter(|&c| g.zone(c) == Zone::Pile(bm))
                    .collect();
            f.pc = 6;
            f.ask_order(rest, Purpose::Move)
                .question("put on the bottom of the Black Market deck")
        }
        _ => {
            for c in input.cards() {
                g.move_card(c, Zone::Pile(bm), Pos::Bottom);
            }
            Flow::done()
        }
    }
}

/// Captain: play a non-Duration, non-Command Action card from the Supply costing up to
/// $4, leaving it there.
fn captain_play(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let piles = g.supply_piles_where(|t, c| {
                t.contains(Types::ACTION)
                    && !t.intersects(Types::DURATION | Types::COMMAND)
                    && c.at_most(Cost::coins(4))
            });
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Play)
        }
        1 => {
            f.pc = 2;
            match input.pile().and_then(|p| g.pile_top(p)) {
                Some(top) => Flow::Call(Frame::play(f.player, top, false, f.chain_for_child())),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn church(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(f.player).to_vec(), 0, 3, Purpose::SetAside)
        }
        _ => {
            let chosen = input.cards();
            for &c in &chosen {
                g.move_card(c, Zone::SetAside(f.player), Pos::Top);
            }
            f.add_duration(
                g,
                Scratch {
                    cards: chosen,
                    ..Scratch::default()
                },
            );
            Flow::done()
        }
    }
}

fn church_next(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0
        && let Some(l) = f.end_lasting(g)
    {
        for c in l.data.cards {
            if g.zone(c) == Zone::SetAside(f.player) {
                g.put_in_hand(c, f.player);
            }
        }
    }
    trash_from_hand(g, f, input, 0)
}

fn dismantle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if let Some(flow) = trash_and_gain(g, f, input, 0, p, GainRule::Cheaper, false) {
        return flow;
    }
    if f.pc == 5 {
        f.pc = 6;
        if f.s.n == 1
            && f.s.cost.coins >= 1
            && let Some(frame) = gain_from_supply(g, p, Card::Gold, Dest::Discard)
        {
            return Flow::Call(frame);
        }
    }
    Flow::done()
}

fn envoy(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, 5))
        }
        1 => {
            f.s.cards = input.cards();
            if f.s.cards.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            let left = g.left_of(f.player);
            f.ask_cards_for(left, f.s.cards.clone(), 1, 1, Purpose::Discard)
        }
        2 => {
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        _ => {
            for c in looking(g, f.player, &std::mem::take(&mut f.s.cards)) {
                g.put_in_hand(c, f.player);
            }
            Flow::done()
        }
    }
}

fn governor(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    if f.pc == 0 {
        f.pc = 1;
        return f.ask_modes(
            &[
                "+3 Cards (others +1 Card)",
                "Gain a Gold (others a Silver)",
                "Trash a card, gain one costing exactly $2 more (others $1 more)",
            ],
            1,
            1,
        );
    }
    if f.pc == 1 {
        f.s.m = input.take().map(mode_mask).unwrap_or_default();
        f.s.i = 0;
        f.pc = 10;
    }
    loop {
        // The lowest mode left is the one resolving; it ends after every player.
        if f.s.m == 0 {
            return Flow::done();
        }
        let order = g.players_from(g.turn.player);
        let Some(&who) = order.get(f.s.i) else {
            f.s.m &= f.s.m - 1;
            f.s.i = 0;
            f.pc = 10;
            continue;
        };
        let mine = who == f.player;
        match f.s.m.trailing_zeros() {
            0 => {
                f.s.i += 1;
                return Flow::Call(Frame::draw(who, if mine { 3 } else { 1 }));
            }
            1 => {
                f.s.i += 1;
                let card = if mine { Card::Gold } else { Card::Silver };
                if let Some(frame) = gain_from_supply(g, who, card, Dest::Discard) {
                    return Flow::Call(frame);
                }
            }
            _ => {
                let rule = GainRule::Exactly(if mine { 2 } else { 1 });
                let input = input.take().unwrap_or(Input::Start);
                if let Some(flow) = trash_and_gain(g, f, input, 10, who, rule, true) {
                    return flow;
                }
                f.s.i += 1;
                f.pc = 10;
            }
        }
    }
}

fn marchland_gain(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            g.add_buys(f.player, 1);
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

fn prince(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let candidates: Vec<CardId> = g
                .hand(f.player)
                .iter()
                .copied()
                .filter(|&c| {
                    let t = g.types(c);
                    t.contains(Types::ACTION)
                        && !t.intersects(Types::DURATION | Types::COMMAND)
                        && g.cost_of(c).at_most(Cost::coins(4))
                })
                .collect();
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(candidates, 0, 1, Purpose::SetAside)
        }
        _ => {
            if let Some(c) = input.card() {
                g.set_aside(c, f.player);
                f.add_duration(
                    g,
                    Scratch {
                        pick: Some(c),
                        ..Scratch::default()
                    },
                );
            }
            Flow::done()
        }
    }
}

fn prince_play(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    f.pc = 1;
    let held = f
        .lasting
        .and_then(|id| g.lasting().iter().find(|l| l.id == id))
        .and_then(|l| l.data.pick);
    match held {
        Some(c) if g.zone(c) == Zone::SetAside(f.player) => {
            Flow::Call(Frame::play(f.player, c, false, vec![]))
        }
        _ => Flow::done(),
    }
}

/// Sauna and Avanto: "You may play a `other` from your hand."
fn sauna_avanto(g: &mut Game, f: &mut CardFrame, input: Input, other: Card) -> Flow {
    match f.pc {
        0 => {
            let candidates: Vec<CardId> = g
                .playable(f.player, Types::ACTION)
                .into_iter()
                .filter(|&c| g.card(c) == other)
                .collect();
            f.pc = 1;
            if candidates.is_empty() {
                return Flow::Continue;
            }
            f.ask_cards(candidates, 0, 1, Purpose::Play)
        }
        1 => {
            f.pc = 2;
            match input.card() {
                Some(c) => Flow::Call(Frame::play(f.player, c, true, vec![])),
                None => Flow::Continue,
            }
        }
        _ => {
            if f.card == Card::Sauna {
                g.add_lasting(
                    Card::Sauna,
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

/// Stash: each Stash just shuffled is put where its owner chooses among the shuffled cards
/// (0: on top of them), looking at the cards still on the deck above them.
fn stash_shuffle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let shuffled = g.shuffled(p).len();
    match f.pc {
        0 => {
            f.s.cards = g.player(p).deck[..shuffled]
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::Stash)
                .collect();
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            if f.s.i >= f.s.cards.len() || shuffled == 0 {
                return Flow::done();
            }
            f.pc = 2;
            let above = g.look_over_shuffled(p);
            let top = match g.player(p).deck.len() - shuffled {
                0 => "on top".to_string(),
                n => format!(
                    "on top of them, under the {} left on the deck",
                    crate::amount(n as i64, "card", "cards")
                ),
            };
            f.ask_number(0, shuffled as i32 - 1, Purpose::Move)
                .public_choice()
                .question(format!(
                    "how many of the shuffled cards go above Stash? 0 puts it {top}"
                ))
                .about(above)
        }
        _ => {
            let stash = f.s.cards[f.s.i];
            let deck = &mut g.players[p.index()].deck;
            if let Some(i) = deck[..shuffled].iter().position(|&c| c == stash) {
                deck.remove(i);
                let pos = (input.number().unwrap_or(0).max(0) as usize).min(shuffled - 1);
                deck.insert(shuffled - 1 - pos, stash);
            }
            f.s.i += 1;
            f.pc = 1;
            Flow::Continue
        }
    }
}

fn summon(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let piles =
                g.supply_piles_where(|t, c| t.contains(Types::ACTION) && c.at_most(Cost::coins(4)));
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
        _ => {
            if let Some(c) = input.card()
                && g.gained(f.player, c).is_some()
            {
                g.set_aside(c, f.player);
                g.add_lasting(
                    Card::Summon,
                    f.player,
                    None,
                    vec![],
                    Until::Removed,
                    Scratch {
                        pick: Some(c),
                        ..Scratch::default()
                    },
                );
            }
            Flow::done()
        }
    }
}

fn summon_play(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    f.pc = 1;
    let Some(l) = f.end_lasting(g) else {
        return Flow::done();
    };
    match l.data.pick {
        Some(c) if g.zone(c) == Zone::SetAside(f.player) => {
            Flow::Call(Frame::play(f.player, c, true, vec![]))
        }
        _ => Flow::done(),
    }
}
