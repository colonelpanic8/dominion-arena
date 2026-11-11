//! Promotional cards: Black Market, Stash, Prince, Captain, Church, Dismantle, Envoy,
//! Governor, Marchland, Sauna / Avanto, Walled Village and the Event Summon.

use super::intrigue::{GainRule, trash_and_gain};
use super::{CardFrame, Hook, SetRules, due_durations, duration_trigger};
use crate::decision::{Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, From as GainFrom, Input, OVERPAY};
use crate::game::{Pos, Scratch, Until, Zone};
use crate::setup;
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
                        && l.data.pick.is_some_and(|c| g.zone(c) == Zone::SetAside(player))
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
                                .optional(),
                        );
                    }
                }
            }
            Occasion::Gain { player, card, .. } if g.card(card) == Card::Marchland => {
                out.push(Trigger::new(player, Card::Marchland, MARCHLAND_GAIN).on(card));
            }
            Occasion::WouldShuffle { player } => {
                if let Some(&stash) = g
                    .player(player)
                    .discard
                    .iter()
                    .find(|&&c| g.card(c) == Card::Stash)
                {
                    out.push(
                        Trigger::new(player, Card::Stash, STASH_SHUFFLE)
                            .on(stash)
                            .optional(),
                    );
                }
            }
            Occasion::StartCleanup { player } => {
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
                                .optional(),
                        );
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

    fn setup(g: &mut Game) {
        if g.supply_pile(Card::BlackMarket).is_none() {
            return;
        }
        let mut pool: Vec<Card> = Card::ALL
            .iter()
            .copied()
            .filter(|&c| black_market_eligible(g, c))
            .collect();
        g.shuffle(&mut pool);
        pool.truncate(BLACK_MARKET_SIZE);
        let pile = setup::add_mixed_pile(g, Card::BlackMarket, &pool, false);
        g.piles[pile.index()].hidden = true;
        g.ext.promos.black_market = Some(pile);
    }
}

/// Whether `card` can go in the Black Market deck: an unused Kingdom card that needs no
/// setup or components of its own (non-Supply cards, Boons, Hexes, Ruins, Heirlooms, ...),
/// which the deck does not provide.
fn black_market_eligible(g: &Game, card: Card) -> bool {
    const KEYWORDS: &[&str] = &[
        "Setup", "Heirloom", "Prize", "Reward", "Boon", "Hex", "Ruins", "Loot", "Favor", "Sun",
    ];
    let def = card.def();
    if card.kind() != Kind::Card || !def.kingdom || def.removed || g.home_pile(card).is_some() {
        return false;
    }
    let Some(cost) = def.cost else {
        return false;
    };
    if cost.potions > 0 && g.supply_pile(Card::Potion).is_none() {
        return false;
    }
    if card.is(Types::LOOTER
        | Types::FATE
        | Types::DOOM
        | Types::LIAISON
        | Types::OMEN
        | Types::TRAVELLER
        | Types::GATHERING)
    {
        return false;
    }
    let text = def.text;
    if KEYWORDS.iter().any(|k| mentions(text, k)) {
        return false;
    }
    !Card::ALL.iter().any(|&other| {
        let d = other.def();
        let component = (other.kind() == Kind::Card && !d.kingdom && !d.basic && d.pile.is_none())
            || matches!(other.kind(), Kind::Artifact | Kind::State);
        component && mentions(text, d.name)
    })
}

/// Whether `text` contains `word` as a whole word.
fn mentions(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
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

/// Whether `p` could pay for `id` right now.
fn affordable(g: &Game, p: PlayerId, id: CardId) -> bool {
    let cost = g.cost_of(id);
    g.player(p).debt == 0
        && cost.coins as u32 <= g.turn.coins
        && cost.potions as u32 <= g.turn.potions
        && super::can_buy(g, p, g.card(id))
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
                g.emit(Event::Revealed {
                    player: p,
                    card: g.card(c),
                });
            }
            f.s.cards = revealed;
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            let treasures = g.cards_in_hand_where(p, Types::TREASURE);
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
            let (debt, coins) = (g.player(p).debt, g.turn.coins);
            if debt > 0 && coins > 0 && f.s.m == 0 {
                f.s.m = 1;
                f.pc = 13;
                return f.ask_number(0, debt.min(coins) as i32, Purpose::Other);
            }
            let buyable: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|&c| g.zone(c) == Zone::Pile(bm) && affordable(g, p, c))
                .collect();
            if buyable.is_empty() {
                f.pc = 5;
                return Flow::Continue;
            }
            f.pc = 4;
            f.ask_cards(buyable, 0, 1, Purpose::Gain)
        }
        4 => match input.card() {
            Some(c) => {
                f.s.pick = Some(c);
                f.s.cards.retain(|&x| x != c);
                let cost = g.cost_of(c);
                g.turn.coins -= cost.coins as u32;
                g.turn.potions -= cost.potions as u32;
                g.players[p.index()].debt += cost.debt as u32;
                let kind = g.card(c);
                g.turn.bought_cards.push(kind);
                g.emit(Event::Bought {
                    player: p,
                    card: kind,
                });
                if kind.def().overpay && g.turn.coins > 0 {
                    f.pc = 10;
                    return f.ask_number(0, g.turn.coins as i32, Purpose::Overpay);
                }
                f.pc = 11;
                Flow::Continue
            }
            None => {
                f.pc = 5;
                Flow::Continue
            }
        },
        10 => {
            let n = input.number().unwrap_or(0);
            f.pc = 11;
            if n <= 0 {
                return Flow::Continue;
            }
            g.turn.coins -= n as u32;
            g.emit(Event::Overpaid {
                player: p,
                amount: n as u32,
            });
            let c = f.s.pick.expect("set in step 4");
            let mut child = CardFrame::new(g.card(c), Hook::Ability(OVERPAY), p, Some(c));
            child.data = n;
            Flow::Call(Frame::Card(Box::new(child)))
        }
        11 => {
            f.pc = 12;
            let card = f.s.pick.expect("set in step 4");
            Flow::Call(Frame::window(Occasion::Buy { player: p, card }))
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
                card: None,
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
        5 => {
            let rest: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|&c| g.zone(c) == Zone::Pile(bm))
                .collect();
            f.pc = 6;
            f.ask_order(rest, Purpose::Move)
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
                Some(top) => Flow::Call(Frame::play(f.player, top, false, vec![])),
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
                g.emit(Event::Moved {
                    player: f.player,
                    card: None,
                    to: Zone::SetAside(f.player),
                });
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
            for c in std::mem::take(&mut f.s.cards) {
                if g.zone(c) == Zone::Deck(f.player) {
                    g.put_in_hand(c, f.player);
                }
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
        f.s.m = match input.take().and_then(Input::opt) {
            Some(Opt::Mode(m)) => m as i32,
            _ => 0,
        };
        f.s.i = 0;
        f.pc = 10;
    }
    loop {
        let order = g.players_from(f.player);
        let Some(&who) = order.get(f.s.i) else {
            return Flow::done();
        };
        let mine = who == f.player;
        match f.s.m {
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
            if f.player == g.turn.player {
                g.turn.buys += 1;
            }
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
                .hand(f.player)
                .iter()
                .copied()
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

/// Stash: the discard pile is shuffled now, and each Stash in it is put where its owner
/// chooses among the shuffled cards.
fn stash_shuffle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let discard = &g.player(p).discard;
            f.s.n = discard.len() as i32;
            f.s.cards = discard
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::Stash)
                .collect();
            if f.s.n > 0 {
                g.shuffle_discard_into_deck(p);
            }
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            if f.s.i >= f.s.cards.len() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_number(0, f.s.n - 1, Purpose::Move)
        }
        _ => {
            let stash = f.s.cards[f.s.i];
            let shuffled = f.s.n as usize;
            let deck = &mut g.players[p.index()].deck;
            if let Some(i) = deck.iter().position(|&c| c == stash)
                && i < shuffled
            {
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
            let piles = g.supply_piles_where(|t, c| {
                t.contains(Types::ACTION) && c.at_most(Cost::coins(4))
            });
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
                && g.zone(c) == Zone::Discard(f.player)
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
