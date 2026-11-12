//! Hinterlands (2nd edition) and its removed 1st-edition cards.

use super::cornucopia_guilds::{
    ask_ordered, discard_down, discard_n, each_gains, gain_one, reveal_hand, still_gained,
};
use super::{CardFrame, Hook, SetRules, looking, mode_mask, names, put_back, whose};
use crate::decision::{Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input};
use crate::game::{Phase, Pos, Scratch, Until, Zone};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {}

pub(crate) struct Rules;

/// "When you gain this" (and Nomads' "or trash").
const GAINED: u8 = 0;
/// Trail's and Weaver's "you may play it".
const REACT_PLAY: u8 = 1;
const TUNNEL_GOLD: u8 = 0;
const FOOLS_GOLD_TRASH: u8 = 0;
const GUARD_DOG_PLAY: u8 = 0;
const TRADER_EXCHANGE: u8 = 0;
const HAGGLER_GAIN: u8 = 0;
const CAULDRON_CURSE: u8 = 0;
const SCHEME_TOPDECK: u8 = 0;
const DUCHESS_GAIN: u8 = 0;
const NOMAD_CAMP_TO_DECK: u8 = 0;
const NOBLE_BRIGAND_BUY: u8 = 0;

/// Cards with a mandatory "when you gain this" ability.
const ON_GAIN: [Card; 10] = [
    Card::BorderVillage,
    Card::Farmland,
    Card::Inn,
    Card::Nomads,
    Card::Souk,
    Card::Cache,
    Card::Embassy,
    Card::IllGottenGains,
    Card::Mandarin,
    Card::Berserker,
];

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Berserker, Hook::Ability(GAINED)) => {
                let this = f.this();
                if f.pc == 0 && still_gained(g, this) {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::play(f.player, this, true, vec![])));
                }
                Flow::done()
            }
            (Card::BorderVillage, Hook::Ability(GAINED)) => {
                let cost = g.cost_of(f.this());
                gain_where(g, f, input, Dest::Discard, |_, c| c.less_than(cost))
            }
            (Card::Farmland, Hook::Ability(GAINED)) => farmland(g, f, input),
            (Card::Inn, Hook::Ability(GAINED)) => inn_gained(g, f, input),
            (Card::Nomads, Hook::Ability(GAINED)) => {
                g.add_coins(f.player, 2);
                Flow::done()
            }
            (Card::Souk, Hook::Ability(GAINED)) => match f.pc {
                0 => {
                    f.pc = 1;
                    let hand = g.hand(f.player).to_vec();
                    f.ask_cards(hand, 0, 2, Purpose::Trash)
                }
                1 => {
                    f.pc = 2;
                    Flow::Call(Frame::trash(f.player, input.cards()))
                }
                _ => Flow::done(),
            },
            (Card::Cache, Hook::Ability(GAINED)) => {
                if f.s.n >= 2 {
                    return Some(Flow::done());
                }
                f.s.n += 1;
                match g.supply_pile(Card::Copper) {
                    Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                    None => Flow::done(),
                }
            }
            (Card::Embassy, Hook::Ability(GAINED)) => {
                let others = g.others_in_turn_order(f.player);
                each_gains(g, f, &others, Card::Silver).unwrap_or_else(Flow::done)
            }
            (Card::IllGottenGains, Hook::Ability(GAINED)) => {
                let others = g.others_in_turn_order(f.player);
                each_gains(g, f, &others, Card::Curse).unwrap_or_else(Flow::done)
            }
            (Card::Mandarin, Hook::Ability(GAINED)) => match f.pc {
                0 => {
                    let treasures: Vec<CardId> = g
                        .in_play(f.player)
                        .iter()
                        .copied()
                        .filter(|&c| g.is(c, Types::TREASURE))
                        .collect();
                    f.pc = 1;
                    f.ask_order(treasures, Purpose::TopDeck)
                }
                _ => {
                    for c in input.cards().into_iter().rev() {
                        g.move_card(c, Zone::Deck(f.player), Pos::Top);
                    }
                    Flow::done()
                }
            },
            (Card::Trail | Card::Weaver, Hook::Ability(REACT_PLAY)) => {
                if f.pc == 0 {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::play(f.player, f.this(), true, vec![])));
                }
                Flow::done()
            }
            (Card::Tunnel, Hook::Ability(TUNNEL_GOLD)) => {
                if f.pc == 0 {
                    if let Some(c) = f.this {
                        g.show(c);
                    }
                    g.emit(Event::Revealed {
                        player: f.player,
                        card: Card::Tunnel,
                    });
                }
                gain_one(g, f, Card::Gold, Dest::Discard)
            }
            (Card::FoolsGold, Hook::Ability(FOOLS_GOLD_TRASH)) => match f.pc {
                0 => {
                    f.pc = 1;
                    let this = f.this();
                    if g.zone(this) != Zone::Hand(f.player) {
                        return Some(Flow::done());
                    }
                    Flow::Call(Frame::trash(f.player, vec![this]))
                }
                1 => {
                    f.pc = 2;
                    match g.supply_pile(Card::Gold) {
                        Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::DeckTop)),
                        None => Flow::done(),
                    }
                }
                _ => Flow::done(),
            },
            (Card::GuardDog, Hook::Ability(GUARD_DOG_PLAY)) => {
                let this = f.this();
                if f.pc == 0 && g.zone(this) == Zone::Hand(f.player) {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::play(f.player, this, true, vec![])));
                }
                Flow::done()
            }
            (Card::Trader, Hook::Ability(TRADER_EXCHANGE)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ
                    && still_gained(g, card)
                    && let Some(silvers) = g.supply_pile(Card::Silver)
                    && let Some(silver) = g.pile_top(silvers)
                {
                    if let Some(c) = f.this {
                        g.show(c);
                    }
                    g.emit(Event::Revealed {
                        player: f.player,
                        card: Card::Trader,
                    });
                    g.exchange(f.player, card, silver);
                }
                Flow::done()
            }
            (Card::Haggler, Hook::Ability(HAGGLER_GAIN)) => {
                let Some(Occasion::Gain { card, .. }) = f.occ else {
                    return Some(Flow::done());
                };
                if f.pc == 0 {
                    f.s.cost = g.cost_of(card);
                }
                let limit = f.s.cost;
                gain_where(g, f, input, Dest::Discard, |t, c| {
                    c.less_than(limit) && !t.intersects(Types::VICTORY)
                })
            }
            (Card::Cauldron, Hook::Ability(CAULDRON_CURSE)) => {
                if f.pc == 0 {
                    let l = g.remove_lasting(f.lasting.expect("from a lasting"));
                    let mask = l.map_or(0, |l| l.data.n);
                    f.targets = g
                        .others_in_turn_order(f.player)
                        .into_iter()
                        .filter(|t| mask & (1 << t.0) != 0)
                        .collect();
                    f.pc = 1;
                }
                let targets = f.targets.clone();
                each_gains(g, f, &targets, Card::Curse).unwrap_or_else(Flow::done)
            }
            (Card::Scheme, Hook::Ability(SCHEME_TOPDECK)) => match f.pc {
                0 => {
                    f.pc = 1;
                    let cands = scheme_candidates(g, f.player);
                    f.ask_cards(cands, 1, 1, Purpose::TopDeck)
                }
                _ => {
                    if let Some(c) = input.card()
                        && g.zone(c) == Zone::InPlay(f.player)
                    {
                        g.top_deck(c, f.player);
                    }
                    Flow::done()
                }
            },
            (Card::Duchess, Hook::Ability(DUCHESS_GAIN)) => {
                gain_one(g, f, Card::Duchess, Dest::Discard)
            }
            (Card::NomadCamp, Hook::Ability(NOMAD_CAMP_TO_DECK)) => {
                if let Some(st) = g.gains.last_mut()
                    && st.card == f.this()
                    && st.to == Dest::Discard
                {
                    st.to = Dest::DeckTop;
                }
                Flow::done()
            }
            (Card::NobleBrigand, Hook::Ability(NOBLE_BRIGAND_BUY)) => {
                if f.targets.is_empty() {
                    f.targets = g.others_in_turn_order(f.player);
                }
                noble_brigand(g, f, input)
            }
            (_, Hook::Ability(_)) => return None,
            (Card::Berserker, _) => berserker(g, f, input),
            (Card::Cartographer, _) => cartographer(g, f, input),
            (Card::Cauldron, _) => {
                let mask = f.targets.iter().fold(0, |m, t| m | (1 << t.0));
                // Action gains already finished: a gain still resolving can be the third.
                let resolving = g
                    .gains
                    .iter()
                    .filter(|s| s.player == f.player && g.is(s.card, Types::ACTION))
                    .count();
                let data = Scratch {
                    n: mask,
                    m: (actions_gained(g) - resolving.min(actions_gained(g))) as i32,
                    ..Scratch::default()
                };
                g.add_lasting(Card::Cauldron, f.player, f.this, vec![], Until::EndOfTurn, data);
                Flow::done()
            }
            (Card::Crossroads, _) => match f.pc {
                0 => {
                    reveal_hand(g, f.player);
                    let n = g.cards_in_hand_where(f.player, Types::VICTORY).len();
                    f.pc = 1;
                    Flow::Call(Frame::draw(f.player, n))
                }
                _ => {
                    if first_play_of(g, f, Card::Crossroads) {
                        g.add_actions(f.player, 3);
                    }
                    Flow::done()
                }
            },
            (Card::Develop, _) => develop(g, f, input),
            (Card::FoolsGold, _) => {
                let first = first_play_of(g, f, Card::FoolsGold);
                g.add_coins(f.player, if first { 1 } else { 4 });
                Flow::done()
            }
            (Card::GuardDog, _) => {
                if f.pc == 0 && g.hand(f.player).len() <= 5 {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, 2)));
                }
                Flow::done()
            }
            (Card::Haggler | Card::Highway | Card::Scheme, _) => {
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
            (Card::Inn, _) => discard_n(g, f, input, 2),
            (Card::JackOfAllTrades, _) => jack_of_all_trades(g, f, input),
            (Card::Margrave, _) => margrave(g, f, input),
            (Card::Oasis, _) => discard_n(g, f, input, 1),
            (Card::Souk, _) => {
                let lose = (g.hand(f.player).len() as u32).min(g.turn.coins);
                if f.player == g.turn.player {
                    g.turn.coins -= lose;
                }
                Flow::done()
            }
            (Card::SpiceMerchant, _) => spice_merchant(g, f, input),
            (Card::Stables, _) => stables(g, f, input),
            (Card::Trader, _) => trader(g, f, input),
            (Card::Weaver, _) => weaver(g, f, input),
            (Card::Wheelwright, _) => wheelwright(g, f, input),
            (Card::WitchsHut, _) => witchs_hut(g, f, input),
            (Card::Duchess, _) => duchess(g, f, input),
            (Card::Embassy, _) => discard_n(g, f, input, 3),
            (Card::IllGottenGains, _) => match f.pc {
                0 => {
                    if g.supply_pile(Card::Copper).is_none() {
                        return Some(Flow::done());
                    }
                    f.pc = 1;
                    f.ask_yes_no(Purpose::Gain, "Gain a Copper to your hand")
                }
                1 if input.is_yes() => {
                    f.pc = 2;
                    let pile = g.supply_pile(Card::Copper).expect("checked");
                    Flow::Call(Frame::gain(f.player, pile, Dest::Hand))
                }
                _ => Flow::done(),
            },
            (Card::Mandarin, _) => match f.pc {
                0 => {
                    f.pc = 1;
                    let hand = g.hand(f.player).to_vec();
                    f.ask_cards(hand, 1, 1, Purpose::TopDeck)
                }
                _ => {
                    for c in input.cards() {
                        g.move_card(c, Zone::Deck(f.player), Pos::Top);
                    }
                    Flow::done()
                }
            },
            (Card::NobleBrigand, _) => noble_brigand(g, f, input),
            (Card::Oracle, _) => oracle(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        let not_cleanup = g.turn.phase != Phase::Cleanup;
        match *occ {
            Occasion::Gain {
                player,
                card,
                bought,
            } => {
                let kind = g.card(card);
                if ON_GAIN.contains(&kind) {
                    let applies = kind != Card::Berserker
                        || (still_gained(g, card)
                            && g.in_play(player).iter().any(|&c| g.is(c, Types::ACTION)));
                    if applies {
                        out.push(Trigger::new(player, kind, GAINED).on(card));
                    }
                }
                if kind == Card::Trail && not_cleanup && still_gained(g, card) {
                    out.push(Trigger::new(player, kind, REACT_PLAY).on(card).optional("play it"));
                }
                if kind == Card::Duchy
                    && g
                        .supply_pile(Card::Duchess)
                        .is_some_and(|p| !g.pile(p).cards.is_empty())
                {
                    out.push(Trigger::new(player, Card::Duchess, DUCHESS_GAIN).optional("gain a Duchess"));
                }
                if kind == Card::Province {
                    for q in g.others(player) {
                        for &c in g.hand(q) {
                            if g.card(c) == Card::FoolsGold {
                                out.push(
                                    Trigger::new(q, Card::FoolsGold, FOOLS_GOLD_TRASH)
                                        .on(c)
                                        .optional("trash it from your hand to gain a Gold onto your deck"),
                                );
                            }
                        }
                    }
                }
                let exchangeable = still_gained(g, card)
                    && g.home_pile(g.printed(card)).is_some()
                    && g
                        .supply_pile(Card::Silver)
                        .is_some_and(|p| !g.pile(p).cards.is_empty());
                if exchangeable {
                    for &c in g.hand(player) {
                        if g.card(c) == Card::Trader {
                            out.push(
                                Trigger::new(player, Card::Trader, TRADER_EXCHANGE)
                                    .on(c)
                                    .optional(format!("reveal it to exchange {} for a Silver", g.card(card))),
                            );
                        }
                    }
                }
                // Fires once the third Action is gained, even if resolving its other
                // when-gain abilities gained more Actions first.
                let third_action = player == g.turn.player
                    && g.is(card, Types::ACTION)
                    && actions_gained(g) >= 3;
                for l in g.lasting() {
                    if l.owner != player {
                        continue;
                    }
                    if l.card == Card::Haggler && bought {
                        out.push(Trigger::new(player, l.card, HAGGLER_GAIN).via_lasting(l.id));
                    }
                    if l.card == Card::Cauldron && third_action && l.data.m < 3 {
                        out.push(Trigger::new(player, l.card, CAULDRON_CURSE).via_lasting(l.id));
                    }
                }
            }
            Occasion::WouldGain { player, card } => {
                // Only instead of to the discard pile.
                let to_discard = g
                    .gains
                    .last()
                    .is_some_and(|st| st.card == card && st.to == Dest::Discard);
                if g.card(card) == Card::NomadCamp && to_discard {
                    out.push(Trigger::new(player, Card::NomadCamp, NOMAD_CAMP_TO_DECK).on(card));
                }
            }
            Occasion::Trash { player, card } => match g.card(card) {
                Card::Nomads => out.push(Trigger::new(player, Card::Nomads, GAINED).on(card)),
                Card::Trail if not_cleanup && g.zone(card) == Zone::Trash => {
                    out.push(Trigger::new(player, Card::Trail, REACT_PLAY).on(card).optional("play it"));
                }
                _ => {}
            },
            Occasion::Discard { player, card } => {
                if !not_cleanup || g.zone(card) != Zone::Discard(player) {
                    return;
                }
                match g.card(card) {
                    Card::Tunnel => {
                        out.push(Trigger::new(player, Card::Tunnel, TUNNEL_GOLD).on(card).optional("reveal it to gain a Gold"));
                    }
                    k @ (Card::Trail | Card::Weaver) => {
                        out.push(Trigger::new(player, k, REACT_PLAY).on(card).optional("play it"));
                    }
                    _ => {}
                }
            }
            Occasion::Buy { player, card, .. } => {
                if g.card(card) == Card::NobleBrigand {
                    out.push(Trigger::new(player, Card::NobleBrigand, NOBLE_BRIGAND_BUY).on(card));
                }
            }
            Occasion::Attack { target, .. } => {
                for &c in g.hand(target) {
                    if g.card(c) == Card::GuardDog && super::can_play(g, target, c) {
                        out.push(
                            Trigger::new(target, Card::GuardDog, GUARD_DOG_PLAY)
                                .on(c)
                                .optional("play it from your hand"),
                        );
                    }
                }
            }
            Occasion::DiscardFromPlay { player } => {
                if scheme_candidates(g, player).is_empty() {
                    return;
                }
                for l in g.lasting() {
                    if l.card == Card::Scheme && l.owner == player {
                        out.push(
                            Trigger::new(player, Card::Scheme, SCHEME_TOPDECK)
                                .via_lasting(l.id)
                                .optional("put an Action card you discard from play onto your deck"),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn modify_cost(g: &Game, card: Card, _id: Option<CardId>, cost: &mut Cost) {
        if card.kind() != Kind::Card {
            return;
        }
        let n = g
            .lasting()
            .iter()
            .filter(|l| l.card == Card::Highway)
            .count();
        *cost = cost.minus_coins(n.min(255) as u8);
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::SilkRoad).then(|| {
            let victory = g
                .player(p)
                .owned()
                .filter(|&c| g.is(c, Types::VICTORY))
                .count();
            victory as i32 / 4
        })
    }
}

/// "If this is the first time you played a `name` this turn": this play is of a card named
/// `name`, and none of its player's earlier plays this turn was.
fn first_play_of(g: &Game, f: &CardFrame, name: Card) -> bool {
    f.this.is_some_and(|c| {
        g.card(c) == name && g.plays_before(f.player, c).all(|(played, _)| played != name)
    })
}

/// Action cards the turn's player has gained this turn.
fn actions_gained(g: &Game) -> usize {
    g.turn.gained_of(Types::ACTION)
}

/// The Action cards `p` is discarding from play.
fn scheme_candidates(g: &Game, p: PlayerId) -> Vec<CardId> {
    g.leaving_play(p)
        .into_iter()
        .filter(|&c| g.is(c, Types::ACTION))
        .collect()
}

/// Gains a card from a Supply pile whose top card satisfies `pred` (mandatory if any).
fn gain_where(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    to: Dest,
    pred: impl Fn(Types, Cost) -> bool,
) -> Flow {
    match f.pc {
        0 => {
            let piles = g.supply_piles_where(pred);
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

fn berserker(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        if f.pc == 0 {
            f.s.cost = g.cost_of(f.this());
        }
        let limit = f.s.cost;
        let flow = gain_where(g, f, input, Dest::Discard, |_, c| c.less_than(limit));
        if !matches!(flow, Flow::Done(_)) {
            return flow;
        }
        f.pc = 2;
        return Flow::Continue;
    }
    if f.pc == 2 {
        f.pc = 3;
        return Flow::Continue;
    }
    discard_down(g, f, input, 3, 3).unwrap_or_else(Flow::done)
}

fn cartographer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(f.player, 4))
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
            Flow::Call(Frame::discard(f.player, chosen))
        }
        3 => {
            f.s.cards = looking(g, f.player, &f.s.cards);
            f.pc = 4;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        _ => {
            put_back(g, f.player, &input.cards());
            Flow::done()
        }
    }
}

fn develop(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            let (up, down) = develop_costs(g, f);
            let piles = g.supply_piles_where(|_, c| Some(c) == up || Some(c) == down);
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        _ => {
            let Some(pile) = input.pile() else {
                return Flow::done();
            };
            let top = g.pile_top(pile).expect("offered piles are not empty");
            let (up, _) = develop_costs(g, f);
            f.s.m |= if Some(g.cost_of(top)) == up { 1 } else { 2 };
            f.pc = 2;
            Flow::Call(Frame::gain(f.player, pile, Dest::DeckTop))
        }
    }
}

/// The costs Develop still has to gain: $1 more and $1 less than the trashed card costs
/// now.
fn develop_costs(g: &Game, f: &CardFrame) -> (Option<Cost>, Option<Cost>) {
    let cost = g.cost_of(f.s.pick.expect("set when trashed"));
    let up = (f.s.m & 1 == 0).then(|| cost.plus_coins(1));
    let down = (f.s.m & 2 == 0 && cost.coins >= 1).then(|| cost.minus_coins(1));
    (up, down)
}

fn farmland(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            let exact = g.cost_of(f.s.pick.expect("set in step 1")).plus_coins(2);
            let piles: Vec<PileId> = g
                .supply_piles_where(|_, c| c == exact)
                .into_iter()
                .filter(|&p| g.pile_top(p).is_some_and(|t| g.card(t) != Card::Farmland))
                .collect();
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn inn_gained(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let p = f.player;
            g.look_through_discard(p);
            let actions: Vec<CardId> = g
                .player(p)
                .discard
                .iter()
                .copied()
                .filter(|&c| g.is(c, Types::ACTION))
                .collect();
            f.pc = 1;
            let n = actions.len();
            f.ask_cards(actions, 0, n, Purpose::Move)
                .question("shuffle into your deck")
        }
        1 => {
            let p = f.player;
            for c in input.cards() {
                g.reveal(p, c);
                g.move_card(c, Zone::Deck(p), Pos::Top);
            }
            f.pc = 2;
            Flow::Call(Frame::shuffle_deck(p))
        }
        _ => Flow::done(),
    }
}

fn jack_of_all_trades(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            match g.supply_pile(Card::Silver) {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::look(f.player, 1))
        }
        2 => {
            f.s.pick = input.card();
            if f.s.pick.is_none() {
                f.pc = 4;
                return Flow::Continue;
            }
            f.pc = 3;
            f.ask_yes_or(Purpose::Discard, format!("Discard {}", g.card(f.s.pick.expect("set above"))), "Put it back").about(f.s.pick.into_iter().collect())
        }
        3 => {
            f.pc = 4;
            let c = f.s.pick.expect("set in step 2");
            if input.is_yes() {
                return Flow::Call(Frame::discard(f.player, vec![c]));
            }
            put_back(g, f.player, &[c]);
            Flow::Continue
        }
        4 => {
            f.pc = 5;
            Flow::Call(Frame::draw_to(f.player, 5))
        }
        5 => {
            f.pc = 6;
            let cands: Vec<CardId> = g
                .hand(f.player)
                .iter()
                .copied()
                .filter(|&c| !g.is(c, Types::TREASURE))
                .collect();
            f.ask_cards(cands, 0, 1, Purpose::Trash)
        }
        6 => {
            f.pc = 7;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn margrave(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(&t) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::draw(t, 1))
        }
        1 => {
            let hand = g.hand(t).to_vec();
            if hand.len() <= 3 {
                f.pc = 3;
                return Flow::Continue;
            }
            let n = hand.len() - 3;
            f.pc = 2;
            f.ask_cards_for(t, hand, n, n, Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(t, input.cards()))
        }
        _ => {
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
    }
}

fn spice_merchant(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let treasures = g.cards_in_hand_where(f.player, Types::TREASURE);
            f.ask_cards(treasures, 0, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, chosen))
        }
        2 => {
            f.pc = 3;
            f.ask_modes(&["+2 Cards and +1 Action", "+1 Buy and +$2"], 1, 1)
        }
        3 => {
            f.s.m = mode_mask(input);
            f.pc = 4;
            if f.s.m & 1 != 0 {
                g.add_actions(f.player, 1);
                return Flow::Call(Frame::draw(f.player, 2));
            }
            Flow::Continue
        }
        _ => {
            if f.s.m & 2 != 0 {
                g.add_buys(f.player, 1);
                g.add_coins(f.player, 2);
            }
            Flow::done()
        }
    }
}

fn stables(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let treasures = g.cards_in_hand_where(f.player, Types::TREASURE);
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
        2 => {
            f.pc = 3;
            g.add_actions(f.player, 1);
            Flow::Call(Frame::draw(f.player, 3))
        }
        _ => Flow::done(),
    }
}

fn trader(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            f.s.n = g.cost_of(f.s.pick.expect("set in step 1")).coins as i32;
            f.pc = 3;
            Flow::Continue
        }
        _ => {
            if f.s.n <= 0 {
                return Flow::done();
            }
            f.s.n -= 1;
            match g.supply_pile(Card::Silver) {
                Some(pile) if !g.pile(pile).cards.is_empty() => {
                    Flow::Call(Frame::gain(f.player, pile, Dest::Discard))
                }
                _ => Flow::done(),
            }
        }
    }
}

fn weaver(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_either(&["Gain two Silvers", "Gain a card costing up to $4"])
        }
        1 => {
            if input.opt() == Some(Opt::Mode(1)) {
                f.pc = 10;
                return Flow::Continue;
            }
            f.pc = 2;
            Flow::Continue
        }
        2 | 3 => {
            f.pc += 1;
            match g.supply_pile(Card::Silver) {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        4 => Flow::done(),
        _ => {
            f.pc -= 10;
            let flow = gain_where(g, f, input, Dest::Discard, |_, c| {
                c.at_most(Cost::coins(4))
            });
            f.pc += 10;
            flow
        }
    }
}

fn wheelwright(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, 0, 1, Purpose::Discard)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, vec![c]))
        }
        _ => {
            f.pc -= 2;
            let limit = g.cost_of(f.s.pick.expect("set in step 1"));
            let flow = gain_where(g, f, input, Dest::Discard, |t, c| {
                c.at_most(limit) && t.intersects(Types::ACTION)
            });
            f.pc += 2;
            flow
        }
    }
}

fn witchs_hut(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, 2, 2, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            for &c in &chosen {
                g.reveal(f.player, c);
            }
            f.s.n = (chosen.len() == 2 && chosen.iter().all(|&c| g.is(c, Types::ACTION))) as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        _ => {
            if f.s.n == 0 {
                return Flow::done();
            }
            let targets = f.targets.clone();
            each_gains(g, f, &targets, Card::Curse).unwrap_or_else(Flow::done)
        }
    }
}

fn duchess(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let players = g.players_from(g.turn.player);
    let Some(&q) = players.get(f.s.i) else {
        return Flow::done();
    };
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(q, 1))
        }
        1 => {
            f.s.pick = input.card();
            if f.s.pick.is_none() {
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
            f.pc = 2;
            f.ask_yes_or_for(q, Purpose::Discard, format!("Discard {}", g.card(f.s.pick.expect("set above"))), "Put it back")
                .about(f.s.pick.into_iter().collect())
        }
        _ => {
            f.s.i += 1;
            f.pc = 0;
            let c = f.s.pick.expect("set in step 1");
            if input.is_yes() {
                return Flow::Call(Frame::discard(q, vec![c]));
            }
            put_back(g, q, &[c]);
            Flow::Continue
        }
    }
}

fn noble_brigand(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                let Some(&t) = f.targets.get(f.s.i) else {
                    f.s.i = 0;
                    f.pc = 6;
                    continue;
                };
                f.pc = 1;
                return Flow::Call(Frame::reveal(t, 2));
            }
            1 => {
                let revealed = input.take().map(Input::cards).unwrap_or_default();
                let m = f.s.m as usize;
                f.s.cards.truncate(m);
                f.s.cards.extend(&revealed);
                f.s.n = revealed.iter().any(|&c| g.is(c, Types::TREASURE)) as i32;
                f.s.pick = None;
                let loot: Vec<CardId> = revealed
                    .into_iter()
                    .filter(|&c| matches!(g.card(c), Card::Silver | Card::Gold))
                    .collect();
                if loot.is_empty() {
                    f.pc = 3;
                    continue;
                }
                f.pc = 2;
                return f.ask_cards(loot, 1, 1, Purpose::Trash);
            }
            2 => {
                let c = input.take().and_then(Input::card);
                f.s.pick = c;
                f.pc = 3;
                return Flow::Call(Frame::trash(f.targets[f.s.i], c.into_iter().collect()));
            }
            3 => {
                let t = f.targets[f.s.i];
                let m = f.s.m as usize;
                let revealed = f.s.cards.split_off(m);
                if let Some(c) = f.s.pick.take() {
                    f.s.cards.push(c);
                    f.s.m += 1;
                }
                let rest = looking(g, t, &revealed);
                f.pc = 4;
                return Flow::Call(Frame::discard(t, rest));
            }
            4 => {
                f.pc = 5;
                if f.s.n == 0
                    && let Some(pile) = g.supply_pile(Card::Copper)
                {
                    return Flow::Call(Frame::gain(f.targets[f.s.i], pile, Dest::Discard));
                }
            }
            5 => {
                f.s.i += 1;
                f.pc = 0;
            }
            _ => {
                let Some(&c) = f.s.cards.get(f.s.i) else {
                    return Flow::done();
                };
                f.s.i += 1;
                if g.zone(c) == Zone::Trash {
                    return Flow::Call(Frame::gain_card(f.player, c, Dest::Discard));
                }
            }
        }
    }
}

fn oracle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let victims: Vec<PlayerId> = std::iter::once(f.player)
        .chain(f.targets.iter().copied())
        .collect();
    let Some(&q) = victims.get(f.s.i) else {
        if f.pc != 9 {
            f.pc = 9;
            return Flow::Call(Frame::draw(f.player, 2));
        }
        return Flow::done();
    };
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(q, 2))
        }
        1 => {
            f.s.cards = input.cards();
            if f.s.cards.is_empty() {
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
            f.pc = 2;
            f.ask_yes_or(Purpose::Discard, format!("Discard {} {}", whose(f.player, q), names(g, &f.s.cards)), "Put them back")
        }
        2 => {
            let cards = std::mem::take(&mut f.s.cards);
            if input.is_yes() {
                f.pc = 4;
                return Flow::Call(Frame::discard(q, cards));
            }
            f.pc = 3;
            let n = cards.len();
            ask_ordered(f, q, cards, n, Purpose::TopDeck)
        }
        3 => {
            put_back(g, q, &input.cards());
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
        _ => {
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
    }
}
