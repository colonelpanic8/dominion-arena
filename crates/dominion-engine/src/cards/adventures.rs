//! Adventures: Durations, Reserve cards and the Tavern mat, Travellers, Events and the
//! Adventures tokens.

use super::{CardFrame, Hook, SetRules, due_durations, duration_trigger, mode_mask};
use crate::decision::{Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, Value};
use crate::game::{ExtraTurn, Mat, Phase, Pos, Scratch, Token, Until, Zone};
use crate::setup::add_pile;
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Ports being gained by another Port; they don't come with another.
    extra_ports: Vec<CardId>,
    /// Players who have bought Inheritance, and the card they set aside with it.
    pub inherited: Vec<(PlayerId, Option<CardId>)>,
}

pub(crate) struct Rules;

/// A Duration's (or Hireling's) start-of-turn effect.
const DURATION: u8 = 0;
/// Calling a Reserve card from the Tavern mat.
const CALL: u8 = 1;
/// Caravan Guard played in response to an Attack.
const REACT: u8 = 2;
/// Champion: Attacks don't affect its owner.
const PROTECT: u8 = 3;
/// "When you play a card, you first get ...": Champion and the pile tokens.
const PLAY_BONUS: u8 = 4;
/// When-gain abilities (Lost City, Port, Messenger).
const ON_GAIN: u8 = 5;
/// A Traveller's exchange when discarded from play.
const EXCHANGE: u8 = 6;
/// Haunted Woods / Swamp Hag hitting a player who gains a card they bought.
const VICTIM: u8 = 7;
/// The Trashing token.
const TRASHING: u8 = 8;
/// Travelling Fair's "you may put it onto your deck".
const FAIR: u8 = 9;
/// Save's card returning to the hand.
const SAVE_RETURN: u8 = 10;
/// Discarding Wine Merchant from the Tavern mat.
const WINE_DISCARD: u8 = 11;

const START_CALLS: [Card; 4] = [
    Card::Ratcatcher,
    Card::Guide,
    Card::Transmogrify,
    Card::Teacher,
];

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (_, Hook::Ability(PLAY_BONUS)) => play_bonus(g, f),
            (_, Hook::Ability(EXCHANGE)) => exchange(g, f),
            (_, Hook::Ability(DURATION)) => duration(g, f, input)?,
            (_, Hook::Ability(CALL)) => call(g, f, input)?,
            (_, Hook::Ability(ON_GAIN)) => on_gain(g, f, input)?,
            (_, Hook::Ability(VICTIM)) => victim(g, f, input),
            (Card::Champion, Hook::Ability(PROTECT)) => Flow::Done(Value::Bool(true)),
            (Card::CaravanGuard, Hook::Ability(REACT)) => match f.pc {
                0 => {
                    let this = f.this();
                    if g.zone(this) != Zone::Hand(f.player) {
                        return Some(Flow::done());
                    }
                    f.pc = 1;
                    Flow::Call(Frame::play(f.player, this, true, vec![]))
                }
                _ => Flow::done(),
            },
            (Card::Plan, Hook::Ability(TRASHING)) => match f.pc {
                0 => {
                    f.pc = 1;
                    f.ask_cards(g.hand(f.player).to_vec(), 0, 1, Purpose::Trash)
                }
                1 => {
                    f.pc = 2;
                    Flow::Call(Frame::trash(f.player, input.cards()))
                }
                _ => Flow::done(),
            },
            (Card::TravellingFair, Hook::Ability(FAIR)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ {
                    g.top_deck(card, f.player);
                }
                Flow::done()
            }
            (Card::Save, Hook::Ability(SAVE_RETURN)) => {
                let id = f.lasting.expect("from a lasting");
                let cards = g.lasting_mut(id).map(|l| std::mem::take(&mut l.data.cards));
                for c in cards.unwrap_or_default() {
                    if g.zone(c) == Zone::SetAside(f.player) {
                        g.put_in_hand(c, f.player);
                    }
                }
                Flow::done()
            }
            (Card::WineMerchant, Hook::Ability(WINE_DISCARD)) => match f.pc {
                0 => {
                    let this = f.this();
                    if g.zone(this) != Zone::Mat(f.player, Mat::Tavern) {
                        return Some(Flow::done());
                    }
                    f.pc = 1;
                    Flow::Call(Frame::discard(f.player, vec![this]))
                }
                _ => Flow::done(),
            },
            (_, Hook::Ability(_)) => return None,
            (Card::Inheritance, _) if f.this.is_some() => inherited_play(g, f),
            (
                Card::CoinOfTheRealm
                | Card::DistantLands
                | Card::Duplicate
                | Card::Guide
                | Card::Ratcatcher
                | Card::RoyalCarriage
                | Card::Teacher
                | Card::Transmogrify
                | Card::WineMerchant,
                _,
            ) => {
                to_tavern(g, f);
                Flow::done()
            }
            (Card::Amulet, _) => amulet(g, f, input),
            (Card::Artificer, _) => artificer(g, f, input),
            (Card::BridgeTroll, _) => bridge_troll(g, f),
            (Card::CaravanGuard | Card::Champion | Card::Hireling, _) => {
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Dungeon, _) => dungeon(g, f, input),
            (Card::Gear, _) => gear(g, f, input),
            (Card::Giant, _) => giant(g, f, input),
            (Card::HauntedWoods | Card::SwampHag, _) => {
                let mask = f.targets.iter().fold(0, |m, t| m | 1 << t.index());
                f.add_duration(
                    g,
                    Scratch {
                        i: mask,
                        ..Scratch::default()
                    },
                );
                Flow::done()
            }
            (Card::Magpie, _) => magpie(g, f, input),
            (Card::Messenger, _) => messenger(g, f, input),
            (Card::Miser, _) => miser(g, f, input),
            (Card::Ranger, _) => {
                if f.pc == 0 && flip_journey(g, f.player) {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, 5)));
                }
                Flow::done()
            }
            (Card::Raze, _) => raze(g, f, input),
            (Card::Relic, _) => {
                for &t in &f.targets {
                    g.players[t.index()].minus_card = true;
                    g.emit(Event::Received {
                        player: t,
                        card: Card::Relic,
                    });
                }
                Flow::done()
            }
            (Card::Storyteller, _) => storyteller(g, f, input),
            (Card::TreasureTrove, _) => gain_each(g, f, &[Card::Gold, Card::Copper]),
            (Card::TreasureHunter, _) => {
                if f.pc == 0 {
                    f.s.n = gains_on_last_turn(g, g.right_of(f.player)) as i32;
                    f.pc = 1;
                }
                if f.s.n <= 0 {
                    return Some(Flow::done());
                }
                f.s.n -= 1;
                match g.supply_pile(Card::Silver) {
                    Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                    None => Flow::done(),
                }
            }
            (Card::Warrior, _) => warrior(g, f, input),
            (Card::Hero, _) => match f.pc {
                0 => {
                    let piles = g.supply_piles_where(|t, _| t.intersects(Types::TREASURE));
                    if piles.is_empty() {
                        return Some(Flow::done());
                    }
                    f.pc = 1;
                    f.ask_pile(piles, false, Purpose::Gain)
                }
                1 => {
                    f.pc = 2;
                    gain_chosen(f, input.pile(), Dest::Discard)
                }
                _ => Flow::done(),
            },
            (Card::Soldier, _) => soldier(g, f, input),
            (Card::Fugitive, _) => match f.pc {
                0 => {
                    f.pc = 1;
                    f.ask_cards(g.hand(f.player).to_vec(), 1, 1, Purpose::Discard)
                }
                1 => {
                    f.pc = 2;
                    Flow::Call(Frame::discard(f.player, input.cards()))
                }
                _ => Flow::done(),
            },
            (Card::Disciple, _) => disciple(g, f, input),
            (Card::Alms, _) => {
                mark_bought(g, f);
                let treasures = g
                    .in_play(f.player)
                    .iter()
                    .any(|&c| g.is(c, Types::TREASURE));
                if treasures && f.pc == 0 {
                    return Some(Flow::done());
                }
                gain_up_to(g, f, input, Cost::coins(4), Types::all())
            }
            (Card::Borrow, _) => {
                mark_bought(g, f);
                add_buys(g, f.player, 1);
                let pl = &mut g.players[f.player.index()];
                if !pl.minus_card {
                    pl.minus_card = true;
                    g.add_coins(f.player, 1);
                }
                Flow::done()
            }
            (Card::Quest, _) => quest(g, f, input),
            (Card::Save, _) => save(g, f, input),
            (Card::ScoutingParty, _) => scouting_party(g, f, input),
            (Card::TravellingFair, _) => {
                add_buys(g, f.player, 2);
                g.add_lasting(
                    Card::TravellingFair,
                    f.player,
                    None,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::Bonfire, _) => match f.pc {
                0 => {
                    let coppers: Vec<CardId> = g
                        .in_play(f.player)
                        .iter()
                        .copied()
                        .filter(|&c| g.card(c) == Card::Copper)
                        .collect();
                    f.pc = 1;
                    f.ask_cards(coppers, 0, 2, Purpose::Trash)
                }
                1 => {
                    f.pc = 2;
                    Flow::Call(Frame::trash(f.player, input.cards()))
                }
                _ => Flow::done(),
            },
            (Card::Expedition, _) => {
                if f.player == g.turn.player {
                    g.turn.cleanup_draw += 2;
                }
                Flow::done()
            }
            (Card::Ferry, _) => move_token(g, f, input, Token::MinusCost),
            (Card::Plan, _) => move_token(g, f, input, Token::Trashing),
            (Card::LostArts, _) => move_token(g, f, input, Token::PlusAction),
            (Card::Training, _) => move_token(g, f, input, Token::PlusCoin),
            (Card::Pathfinding, _) => move_token(g, f, input, Token::PlusCard),
            (Card::Mission, _) => {
                g.extra_turns.push_back(ExtraTurn {
                    player: f.player,
                    source: Card::Mission,
                    controller: f.player,
                });
                Flow::done()
            }
            (Card::Pilgrimage, _) => pilgrimage(g, f, input),
            (Card::Ball, _) => {
                if f.pc == 0 {
                    g.players[f.player.index()].minus_coin = true;
                    f.s.n = 2;
                    f.pc = 1;
                }
                gain_up_to_repeat(g, f, input)
            }
            (Card::Raid, _) => {
                if f.pc == 0 {
                    f.s.n = g
                        .in_play(f.player)
                        .iter()
                        .filter(|&&c| g.card(c) == Card::Silver)
                        .count() as i32;
                    for o in g.others(f.player) {
                        g.players[o.index()].minus_card = true;
                        g.emit(Event::Received {
                            player: o,
                            card: Card::Raid,
                        });
                    }
                    f.pc = 1;
                }
                if f.s.n <= 0 {
                    return Some(Flow::done());
                }
                f.s.n -= 1;
                match g.supply_pile(Card::Silver) {
                    Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                    None => Flow::done(),
                }
            }
            (Card::Seaway, _) => match f.pc {
                0 => {
                    let piles = g.supply_piles_where(|t, cost| {
                        t.intersects(Types::ACTION) && cost.at_most(Cost::coins(4))
                    });
                    if piles.is_empty() {
                        return Some(Flow::done());
                    }
                    f.pc = 1;
                    f.ask_pile(piles, false, Purpose::Gain)
                }
                1 => {
                    let Some(pile) = input.pile() else {
                        return Some(Flow::done());
                    };
                    f.s.pile = Some(pile);
                    f.pc = 2;
                    Flow::Call(Frame::gain(f.player, pile, Dest::Discard))
                }
                _ => {
                    let pile = f.s.pile.expect("set in step 1");
                    place_token(g, f.player, Token::PlusBuy, pile);
                    Flow::done()
                }
            },
            (Card::Trade, _) => match f.pc {
                0 => {
                    f.pc = 1;
                    f.ask_cards(g.hand(f.player).to_vec(), 0, 2, Purpose::Trash)
                }
                1 => {
                    f.pc = 2;
                    Flow::Call(Frame::trash(f.player, input.cards()))
                }
                2 => {
                    f.s.n = input.cards().len() as i32;
                    f.pc = 3;
                    Flow::Continue
                }
                _ => {
                    if f.s.n <= 0 {
                        return Some(Flow::done());
                    }
                    f.s.n -= 1;
                    match g.supply_pile(Card::Silver) {
                        Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                        None => Flow::done(),
                    }
                }
            },
            (Card::Inheritance, _) => inheritance(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        match *occ {
            Occasion::StartTurn { player } => start_turn_triggers(g, occ, player, out),
            Occasion::Play { player, card } => {
                let (champions, tokens) = play_bonuses(g, player, card);
                if champions > 0 || !tokens.is_empty() {
                    let source = if champions > 0 {
                        Card::Champion
                    } else {
                        match tokens[0] {
                            Token::PlusCard => Card::Pathfinding,
                            Token::PlusAction => Card::LostArts,
                            Token::PlusBuy => Card::Seaway,
                            _ => Card::Training,
                        }
                    };
                    out.push(Trigger::new(player, source, PLAY_BONUS));
                }
            }
            Occasion::AfterPlay { player, card } => {
                if !g.is(card, Types::ACTION) {
                    return;
                }
                for &c in tavern(g, player) {
                    let replayable = g.zone(card) == Zone::InPlay(player);
                    match g.card(c) {
                        Card::CoinOfTheRealm => {
                            out.push(Trigger::new(player, Card::CoinOfTheRealm, CALL).on(c).optional())
                        }
                        Card::RoyalCarriage if replayable => {
                            out.push(Trigger::new(player, Card::RoyalCarriage, CALL).on(c).optional())
                        }
                        _ => {}
                    }
                }
            }
            Occasion::Attack { target, .. } => {
                if active(g, target, Card::Champion).next().is_some() {
                    out.push(Trigger::new(target, Card::Champion, PROTECT));
                }
                for &c in g.hand(target) {
                    if g.card(c) == Card::CaravanGuard {
                        out.push(
                            Trigger::new(target, Card::CaravanGuard, REACT)
                                .on(c)
                                .optional(),
                        );
                    }
                }
            }
            Occasion::Gain {
                player,
                card,
                bought,
            } => gain_triggers(g, player, card, bought, out),
            Occasion::EndBuyPhase { player } => {
                if g.turn.coins < 2 {
                    return;
                }
                for &c in tavern(g, player) {
                    if g.card(c) == Card::WineMerchant {
                        out.push(
                            Trigger::new(player, Card::WineMerchant, WINE_DISCARD)
                                .on(c)
                                .optional(),
                        );
                    }
                }
            }
            Occasion::DiscardFromPlay { player } => {
                let kept: Vec<CardId> = g.lasting().iter().flat_map(|l| l.keeps.clone()).collect();
                for &c in g.in_play(player) {
                    let printed = g.printed(c);
                    let Some(next) = upgrade(printed) else {
                        continue;
                    };
                    let available = g
                        .pile_of(next)
                        .is_some_and(|p| !g.pile(p).cards.is_empty());
                    if g.card(c) == printed && !kept.contains(&c) && available {
                        out.push(Trigger::new(player, printed, EXCHANGE).on(c).optional());
                    }
                }
            }
            Occasion::EndTurn { player } => {
                for l in g.lasting() {
                    let waiting = l
                        .data
                        .cards
                        .iter()
                        .any(|&c| g.zone(c) == Zone::SetAside(player));
                    if l.card == Card::Save && l.owner == player && waiting {
                        out.push(Trigger::new(player, Card::Save, SAVE_RETURN).via_lasting(l.id));
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
        let p = g.turn.player;
        let player = g.player(p);
        let trolls = g
            .lasting()
            .iter()
            .filter(|l| {
                l.card == Card::BridgeTroll
                    && l.owner == p
                    && (l.data.n == 1 || player.turns_taken > l.turn)
            })
            .count() as u8;
        *cost = cost.minus_coins(trolls);
        if let Some(&Some(pile)) = player.tokens.get(&Token::MinusCost)
            && g.home_pile(card) == Some(pile)
        {
            *cost = cost.minus_coins(2);
        }
    }

    fn modify_types(g: &Game, id: CardId, types: &mut Types) {
        if g.card(id) == Card::Estate && inherited_card(g, g.turn.player).is_some() {
            *types |= Types::ACTION | Types::COMMAND;
        }
    }

    fn behavior(g: &Game, id: CardId) -> Option<Card> {
        (g.card(id) == Card::Estate && inherited_card(g, g.turn.player).is_some())
            .then_some(Card::Inheritance)
    }

    fn vp(_g: &Game, _p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::DistantLands).then_some(0)
    }

    fn bonus_vp(g: &Game, p: PlayerId) -> i32 {
        4 * tavern(g, p)
            .iter()
            .filter(|&&c| g.card(c) == Card::DistantLands)
            .count() as i32
    }

    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        match card {
            Card::Alms | Card::Borrow | Card::Save | Card::Pilgrimage => !g
                .lasting()
                .iter()
                .any(|l| l.card == card && l.owner == p && l.until == Until::EndOfTurn),
            Card::Inheritance => g.ext.adventures.inherited.iter().all(|&(q, _)| q != p),
            _ => !(card.kind() == Kind::Card && g.turn.extra == Some(Card::Mission)),
        }
    }

    fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
        if card != Card::Port {
            return false;
        }
        add_pile(g, Card::Port, 12, true);
        true
    }

    fn setup(g: &mut Game) {
        for p in &mut g.players {
            p.journey_face_up = true;
        }
        let starts: Vec<Card> = [Card::Page, Card::Peasant]
            .into_iter()
            .filter(|&c| g.supply_pile(c).is_some())
            .collect();
        Self::setup_added_piles(g, &starts);
    }

    fn setup_added_piles(g: &mut Game, cards: &[Card]) {
        let lines = [
            (
                Card::Page,
                [
                    Card::TreasureHunter,
                    Card::Warrior,
                    Card::Hero,
                    Card::Champion,
                ],
            ),
            (
                Card::Peasant,
                [Card::Soldier, Card::Fugitive, Card::Disciple, Card::Teacher],
            ),
        ];
        for (start, line) in lines {
            if cards.contains(&start) && g.pile_of(line[0]).is_none() {
                for card in line {
                    add_pile(g, card, 5, false);
                }
            }
        }
    }
}

// ---- Shared helpers ----

fn tavern(g: &Game, p: PlayerId) -> &[CardId] {
    g.player(p).mat(Mat::Tavern)
}

/// "Put this on your Tavern mat", if it is still in play.
fn to_tavern(g: &mut Game, f: &CardFrame) {
    let this = f.this();
    if g.zone(this) == Zone::InPlay(f.player) {
        g.put_on_mat(this, f.player, Mat::Tavern);
    }
}

/// Calls a Reserve card: moves it from the Tavern mat into play. Returns whether it was
/// still there to call.
fn call_from_tavern(g: &mut Game, f: &CardFrame) -> bool {
    let this = f.this();
    if g.zone(this) != Zone::Mat(f.player, Mat::Tavern) {
        return false;
    }
    g.move_card(this, Zone::InPlay(f.player), Pos::Top);
    g.emit(Event::Moved {
        player: f.player,
        card: Some(g.card(this)),
        to: Zone::InPlay(f.player),
    });
    true
}

fn add_buys(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.turn.buys += n;
    }
}

fn add_actions(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.turn.actions += n;
    }
}

/// Turns `p`'s Journey token over; returns whether it is now face up.
fn flip_journey(g: &mut Game, p: PlayerId) -> bool {
    let pl = &mut g.players[p.index()];
    pl.journey_face_up = !pl.journey_face_up;
    pl.journey_face_up
}

/// Lastings of `card` owned by `p` whose card is still in play (Champion, Hireling).
fn active(g: &Game, p: PlayerId, card: Card) -> impl Iterator<Item = &crate::Lasting> {
    g.lasting().iter().filter(move |l| {
        l.card == card
            && l.owner == p
            && l.source.is_some_and(|s| g.zone(s) == Zone::InPlay(p))
    })
}

/// The Supply pile a copy of `card` can be gained from.
fn copy_pile(g: &Game, card: Card) -> Option<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&id| {
            let pile = g.pile(id);
            pile.in_supply && pile.cards.last().is_some_and(|&top| g.card(top) == card)
        })
}

fn upgrade(card: Card) -> Option<Card> {
    Some(match card {
        Card::Page => Card::TreasureHunter,
        Card::TreasureHunter => Card::Warrior,
        Card::Warrior => Card::Hero,
        Card::Hero => Card::Champion,
        Card::Peasant => Card::Soldier,
        Card::Soldier => Card::Fugitive,
        Card::Fugitive => Card::Disciple,
        Card::Disciple => Card::Teacher,
        _ => return None,
    })
}

/// Supply piles that hold Action cards (or would, for an empty pile). Inheritance's
/// Estates don't make the Estate pile one.
fn action_supply_piles(g: &Game) -> Vec<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .filter(|&id| {
            let pile = g.pile(id);
            pile.in_supply
                && if pile.cards.is_empty() {
                    pile.card
                        .pile_members()
                        .iter()
                        .any(|c| c.is(Types::ACTION))
                } else {
                    pile.cards.iter().any(|&c| g.card(c).is(Types::ACTION))
                }
        })
        .collect()
}

/// Moves `p`'s `token` to `pile`.
fn place_token(g: &mut Game, p: PlayerId, token: Token, pile: PileId) {
    if let Some(Some(old)) = g.players[p.index()].tokens.insert(token, Some(pile)) {
        g.piles[old.index()]
            .tokens
            .retain(|&(o, t)| (o, t) != (p, token));
    }
    g.piles[pile.index()].tokens.push((p, token));
}

/// The card `p` set aside with Inheritance (under their Estate token).
fn inherited_card(g: &Game, p: PlayerId) -> Option<CardId> {
    g.ext
        .adventures
        .inherited
        .iter()
        .find(|&&(q, _)| q == p)
        .and_then(|&(_, c)| c)
}

/// How many cards `p` gained on their most recent turn.
fn gains_on_last_turn(g: &Game, p: PlayerId) -> usize {
    let log = g.log();
    let Some(start) = log
        .iter()
        .rposition(|e| matches!(e, Event::TurnStart { player, .. } if *player == p))
    else {
        return 0;
    };
    log[start + 1..]
        .iter()
        .take_while(|e| !matches!(e, Event::TurnStart { .. }))
        .filter(|e| matches!(e, Event::Gained { player, .. } if *player == p))
        .count()
}

/// Marks a "once per turn" Event as bought this turn.
fn mark_bought(g: &mut Game, f: &CardFrame) {
    if f.pc == 0 {
        g.add_lasting(
            f.card,
            f.player,
            None,
            vec![],
            Until::EndOfTurn,
            Scratch::default(),
        );
    }
}

fn gain_chosen(f: &CardFrame, pile: Option<PileId>, to: Dest) -> Flow {
    match pile {
        Some(pile) => Flow::Call(Frame::gain(f.player, pile, to)),
        None => Flow::done(),
    }
}

/// "Gain a card costing up to `limit`" (with one of `types`), from step 0.
fn gain_up_to(g: &mut Game, f: &mut CardFrame, input: Input, limit: Cost, types: Types) -> Flow {
    match f.pc {
        0 => {
            let piles = g.supply_piles_where(|t, cost| cost.at_most(limit) && t.intersects(types));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            gain_chosen(f, input.pile(), Dest::Discard)
        }
        _ => Flow::done(),
    }
}

/// Ball: gain `f.s.n` more cards costing up to $4, one at a time.
fn gain_up_to_repeat(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        1 => {
            if f.s.n <= 0 {
                return Flow::done();
            }
            f.s.n -= 1;
            let piles = g.piles_costing_up_to(Cost::coins(4));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        2 => {
            f.pc = 3;
            gain_chosen(f, input.pile(), Dest::Discard)
        }
        _ => {
            f.pc = 1;
            Flow::Continue
        }
    }
}

/// Gains one of each of `cards` from the Supply, in order.
fn gain_each(g: &mut Game, f: &mut CardFrame, cards: &[Card]) -> Flow {
    while let Some(&card) = cards.get(f.s.i) {
        f.s.i += 1;
        if let Some(pile) = g.supply_pile(card) {
            return Flow::Call(Frame::gain(f.player, pile, Dest::Discard));
        }
    }
    Flow::done()
}

/// Puts `cards` on top of `p`'s deck, `cards[0]` ending on top.
fn top_deck_ordered(g: &mut Game, p: PlayerId, cards: &[CardId]) {
    for &c in cards.iter().rev() {
        g.move_card(c, Zone::Deck(p), Pos::Top);
    }
}

// ---- Triggers ----

fn start_turn_triggers(g: &Game, occ: &Occasion, player: PlayerId, out: &mut Vec<Trigger>) {
    for l in due_durations(g, occ) {
        let fires = match l.card {
            Card::Amulet
            | Card::CaravanGuard
            | Card::Dungeon
            | Card::Gear
            | Card::HauntedWoods
            | Card::SwampHag => true,
            Card::BridgeTroll => l.data.n == 0,
            Card::Hireling => l.source.is_some_and(|s| g.zone(s) == Zone::InPlay(player)),
            _ => false,
        };
        if fires {
            out.push(duration_trigger(l, DURATION));
        }
    }
    for &c in tavern(g, player) {
        let card = g.card(c);
        if START_CALLS.contains(&card) {
            out.push(Trigger::new(player, card, CALL).on(c).optional());
        }
    }
}

/// What `p` first gets for playing `card`: the number of Champion effects (for Actions)
/// and `p`'s bonus tokens on the card's pile.
fn play_bonuses(g: &Game, p: PlayerId, card: CardId) -> (usize, Vec<Token>) {
    let champions = if g.is(card, Types::ACTION) {
        active(g, p, Card::Champion).count()
    } else {
        0
    };
    let tokens = g
        .home_pile(g.printed(card))
        .map(|pile| {
            g.pile(pile)
                .tokens
                .iter()
                .filter(|&&(o, t)| {
                    o == p
                        && matches!(
                            t,
                            Token::PlusCard | Token::PlusAction | Token::PlusBuy | Token::PlusCoin
                        )
                })
                .map(|&(_, t)| t)
                .collect()
        })
        .unwrap_or_default();
    (champions, tokens)
}

fn gain_triggers(g: &Game, player: PlayerId, card: CardId, bought: bool, out: &mut Vec<Trigger>) {
    let kind = g.card(card);
    let on_gain = match kind {
        Card::LostCity => true,
        Card::Port => !g.ext.adventures.extra_ports.contains(&card),
        Card::Messenger => {
            let t = &g.turn;
            player == t.player && t.phase == Phase::Buy && t.gained_in_buy_phase.len() == 1
        }
        _ => false,
    };
    if on_gain {
        out.push(Trigger::new(player, kind, ON_GAIN).on(card));
    }
    let copyable = g.cost_of(card).at_most(Cost::coins(6)) && copy_pile(g, kind).is_some();
    for &c in tavern(g, player) {
        if copyable && g.card(c) == Card::Duplicate {
            out.push(Trigger::new(player, Card::Duplicate, CALL).on(c).optional());
        }
    }
    let pl = g.player(player);
    if let Some(&Some(pile)) = pl.tokens.get(&Token::Trashing)
        && g.home_pile(g.printed(card)) == Some(pile)
        && !pl.hand.is_empty()
    {
        out.push(Trigger::new(player, Card::Plan, TRASHING));
    }
    let fair = g
        .lasting()
        .iter()
        .any(|l| l.card == Card::TravellingFair && l.owner == player);
    let movable = matches!(g.zone(card), Zone::Discard(p) | Zone::Hand(p) if p == player);
    if fair && movable && player == g.turn.player {
        out.push(Trigger::new(player, Card::TravellingFair, FAIR).optional());
    }
    if bought {
        for l in g.lasting() {
            let hits = matches!(l.card, Card::HauntedWoods | Card::SwampHag)
                && l.until == Until::Removed
                && l.owner != player
                && l.data.i & (1 << player.index()) != 0;
            if hits {
                out.push(Trigger::new(player, l.card, VICTIM).via_lasting(l.id));
            }
        }
    }
}

// ---- Abilities ----

fn play_bonus(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    let Some(Occasion::Play { card, .. }) = f.occ else {
        return Flow::done();
    };
    let p = f.player;
    let (champions, tokens) = play_bonuses(g, p, card);
    let mut cards = 0;
    add_actions(g, p, champions as u32);
    for t in tokens {
        match t {
            Token::PlusCard => cards += 1,
            Token::PlusAction => add_actions(g, p, 1),
            Token::PlusBuy => add_buys(g, p, 1),
            _ => g.add_coins(p, 1),
        }
    }
    f.pc = 1;
    Flow::Call(Frame::draw(p, cards))
}

fn exchange(g: &mut Game, f: &mut CardFrame) -> Flow {
    let this = f.this();
    let p = f.player;
    let next = upgrade(g.printed(this)).expect("only Travellers exchange");
    let top = g.pile_of(next).and_then(|pile| g.pile_top(pile));
    if g.zone(this) != Zone::InPlay(p) {
        return Flow::done();
    }
    let Some(top) = top else {
        return Flow::done();
    };
    g.return_to_pile(this);
    g.move_card(top, Zone::Discard(p), Pos::Top);
    g.emit(Event::Moved {
        player: p,
        card: Some(next),
        to: Zone::Discard(p),
    });
    Flow::done()
}

fn duration(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
    let p = f.player;
    let id = f.lasting.expect("a Duration's start-of-turn effect has a lasting");
    Some(match f.card {
        Card::Amulet => amulet(g, f, input),
        Card::Dungeon => dungeon(g, f, input),
        Card::BridgeTroll => {
            add_buys(g, p, 1);
            if let Some(l) = g.lasting_mut(id) {
                l.keeps.clear();
                l.until = Until::EndOfTurn;
                l.data.n = 1;
            }
            Flow::done()
        }
        Card::CaravanGuard => {
            f.end_lasting(g);
            g.add_coins(p, 1);
            Flow::done()
        }
        Card::SwampHag => {
            f.end_lasting(g);
            g.add_coins(p, 3);
            Flow::done()
        }
        Card::HauntedWoods | Card::Hireling => {
            if f.pc > 0 {
                return Some(Flow::done());
            }
            f.pc = 1;
            let n = if f.card == Card::Hireling {
                1
            } else {
                f.end_lasting(g);
                3
            };
            Flow::Call(Frame::draw(p, n))
        }
        Card::Gear => {
            let cards = f.end_lasting(g).map(|l| l.data.cards);
            for c in cards.unwrap_or_default() {
                if g.zone(c) == Zone::SetAside(p) {
                    g.put_in_hand(c, p);
                }
            }
            Flow::done()
        }
        _ => return None,
    })
}

fn call(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
    if f.pc == 0 {
        if !call_from_tavern(g, f) {
            return Some(Flow::done());
        }
        f.pc = 1;
    }
    let p = f.player;
    Some(match f.card {
        Card::CoinOfTheRealm => {
            add_actions(g, p, 2);
            Flow::done()
        }
        Card::RoyalCarriage => match f.pc {
            1 => {
                let Some(Occasion::AfterPlay { card, .. }) = f.occ else {
                    return Some(Flow::done());
                };
                f.pc = 2;
                if g.zone(card) != Zone::InPlay(p) {
                    return Some(Flow::done());
                }
                Flow::Call(Frame::play(p, card, false, f.chain_for_child()))
            }
            _ => Flow::done(),
        },
        Card::Duplicate => match f.pc {
            1 => {
                let Some(Occasion::Gain { card, .. }) = f.occ else {
                    return Some(Flow::done());
                };
                f.pc = 2;
                gain_chosen(f, copy_pile(g, g.card(card)), Dest::Discard)
            }
            _ => Flow::done(),
        },
        Card::Ratcatcher => match f.pc {
            1 => {
                f.pc = 2;
                f.ask_cards(g.hand(p).to_vec(), 1, 1, Purpose::Trash)
            }
            2 => {
                f.pc = 3;
                Flow::Call(Frame::trash(p, input.cards()))
            }
            _ => Flow::done(),
        },
        Card::Guide => match f.pc {
            1 => {
                f.pc = 2;
                Flow::Call(Frame::discard(p, g.hand(p).to_vec()))
            }
            2 => {
                f.pc = 3;
                Flow::Call(Frame::draw(p, 5))
            }
            _ => Flow::done(),
        },
        Card::Transmogrify => match f.pc {
            1 => {
                let hand = g.hand(p).to_vec();
                if hand.is_empty() {
                    return Some(Flow::done());
                }
                f.pc = 2;
                f.ask_cards(hand, 1, 1, Purpose::Trash)
            }
            2 => {
                let Some(c) = input.card() else {
                    return Some(Flow::done());
                };
                f.s.cost = g.cost_of(c);
                f.pc = 3;
                Flow::Call(Frame::trash(p, vec![c]))
            }
            3 => {
                let piles = g.piles_costing_up_to(f.s.cost.plus_coins(1));
                if piles.is_empty() {
                    return Some(Flow::done());
                }
                f.pc = 4;
                f.ask_pile(piles, false, Purpose::Gain)
            }
            4 => {
                f.pc = 5;
                gain_chosen(f, input.pile(), Dest::Hand)
            }
            _ => Flow::done(),
        },
        Card::Teacher => match f.pc {
            1 => {
                f.pc = 2;
                f.ask_either(&["+1 Card", "+1 Action", "+1 Buy", "+$1"])
            }
            2 => {
                let token = match input.opt() {
                    Some(Opt::Mode(0)) => Token::PlusCard,
                    Some(Opt::Mode(1)) => Token::PlusAction,
                    Some(Opt::Mode(2)) => Token::PlusBuy,
                    _ => Token::PlusCoin,
                };
                f.s.n = token as i32;
                let piles: Vec<PileId> = action_supply_piles(g)
                    .into_iter()
                    .filter(|&pile| g.pile(pile).tokens.iter().all(|&(o, _)| o != p))
                    .collect();
                if piles.is_empty() {
                    return Some(Flow::done());
                }
                f.pc = 3;
                f.ask_pile(piles, false, Purpose::Other)
            }
            _ => {
                let token = [
                    Token::PlusCard,
                    Token::PlusAction,
                    Token::PlusBuy,
                    Token::PlusCoin,
                ]
                .into_iter()
                .find(|&t| t as i32 == f.s.n)
                .expect("set in step 2");
                if let Some(pile) = input.pile() {
                    place_token(g, p, token, pile);
                }
                Flow::done()
            }
        },
        _ => return None,
    })
}

fn on_gain(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
    let p = f.player;
    Some(match f.card {
        Card::LostCity => match g.others(p).get(f.s.i) {
            Some(&o) => {
                f.s.i += 1;
                Flow::Call(Frame::draw(o, 1))
            }
            None => Flow::done(),
        },
        Card::Port => match f.pc {
            0 => {
                let Some(top) = g.supply_pile(Card::Port).and_then(|pile| g.pile_top(pile)) else {
                    return Some(Flow::done());
                };
                g.ext.adventures.extra_ports.push(top);
                f.s.pick = Some(top);
                f.pc = 1;
                Flow::Call(Frame::gain_card(p, top, Dest::Discard))
            }
            _ => {
                let top = f.s.pick.expect("set in step 0");
                g.ext.adventures.extra_ports.retain(|&c| c != top);
                Flow::done()
            }
        },
        Card::Messenger => match f.pc {
            0 => {
                let piles = g.piles_costing_up_to(Cost::coins(4));
                if piles.is_empty() {
                    return Some(Flow::done());
                }
                f.pc = 1;
                f.ask_pile(piles, false, Purpose::Gain)
            }
            1 => {
                let Some(pile) = input.pile() else {
                    return Some(Flow::done());
                };
                f.s.card = g.pile_top(pile).map(|c| g.card(c));
                f.pc = 2;
                Flow::Call(Frame::gain(p, pile, Dest::Discard))
            }
            _ => {
                let Some(card) = f.s.card else {
                    return Some(Flow::done());
                };
                while let Some(&o) = g.others(p).get(f.s.i) {
                    f.s.i += 1;
                    if let Some(pile) = copy_pile(g, card) {
                        return Some(Flow::Call(Frame::gain(o, pile, Dest::Discard)));
                    }
                }
                Flow::done()
            }
        },
        _ => return None,
    })
}

fn victim(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match (f.card, f.pc) {
        (Card::SwampHag, 0) => {
            f.pc = 1;
            match g.supply_pile(Card::Curse) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        (Card::HauntedWoods, 0) => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_order(hand, Purpose::TopDeck)
        }
        (Card::HauntedWoods, 1) => {
            let cards: Vec<CardId> = input
                .cards()
                .into_iter()
                .filter(|&c| g.zone(c) == Zone::Hand(p))
                .collect();
            top_deck_ordered(g, p, &cards);
            Flow::done()
        }
        _ => Flow::done(),
    }
}

// ---- Kingdom cards ----

fn amulet(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if f.hook == Hook::Main {
                f.add_duration(g, Scratch::default());
            } else {
                f.end_lasting(g);
            }
            f.pc = 1;
            f.ask_modes(&["+$1", "Trash a card from your hand", "Gain a Silver"], 1, 1)
        }
        1 => {
            f.s.m = mode_mask(input);
            if f.s.m & 1 != 0 {
                g.add_coins(p, 1);
            }
            if f.s.m & 2 == 0 {
                f.pc = 3;
                return Flow::Continue;
            }
            f.pc = 2;
            f.ask_cards(g.hand(p).to_vec(), 1, 1, Purpose::Trash)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        3 => {
            f.pc = 4;
            if f.s.m & 4 != 0
                && let Some(pile) = g.supply_pile(Card::Silver)
            {
                return Flow::Call(Frame::gain(p, pile, Dest::Discard));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn artificer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            let n = hand.len();
            f.pc = 1;
            f.ask_cards(hand, 0, n, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        2 => {
            let exact = Cost::coins(f.s.n.min(255) as u8);
            let piles = g.supply_piles_where(|_, cost| cost == exact);
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, true, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            gain_chosen(f, input.pile(), Dest::DeckTop)
        }
        _ => Flow::done(),
    }
}

fn bridge_troll(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    for &t in &f.targets {
        g.players[t.index()].minus_coin = true;
        g.emit(Event::Received {
            player: t,
            card: Card::BridgeTroll,
        });
    }
    add_buys(g, p, 1);
    f.add_duration(g, Scratch::default());
    g.add_lasting(
        Card::BridgeTroll,
        p,
        f.this,
        vec![],
        Until::EndOfTurn,
        Scratch {
            n: 1,
            ..Scratch::default()
        },
    );
    Flow::done()
}

fn dungeon(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if f.hook == Hook::Main {
                f.add_duration(g, Scratch::default());
            } else {
                f.end_lasting(g);
            }
            f.pc = 1;
            Flow::Call(Frame::draw(p, 2))
        }
        1 => {
            let hand = g.hand(p).to_vec();
            let n = hand.len().min(2);
            f.pc = 2;
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(p, input.cards()))
        }
        _ => Flow::done(),
    }
}

fn gear(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_cards(g.hand(p).to_vec(), 0, 2, Purpose::SetAside)
        }
        _ => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            for &c in &chosen {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
                g.emit(Event::Moved {
                    player: p,
                    card: None,
                    to: Zone::SetAside(p),
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

fn giant(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let mut input = Some(input);
    loop {
        match f.pc {
            0 => {
                if !flip_journey(g, p) {
                    g.add_coins(p, 1);
                    return Flow::done();
                }
                g.add_coins(p, 5);
                f.pc = 1;
            }
            1 => {
                let Some(&t) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                f.pc = 2;
                return Flow::Call(Frame::reveal(t, 1));
            }
            2 => {
                let t = f.targets[f.s.i];
                let revealed = input.take().and_then(Input::card);
                let Some(c) = revealed else {
                    f.pc = 4;
                    continue;
                };
                let cost = g.cost_of(c);
                if cost.potions == 0 && cost.debt == 0 && (3..=6).contains(&cost.coins) {
                    f.pc = 5;
                    return Flow::Call(Frame::trash(t, vec![c]));
                }
                f.pc = 4;
                return Flow::Call(Frame::discard(t, vec![c]));
            }
            4 => {
                f.pc = 5;
                if let Some(pile) = g.supply_pile(Card::Curse) {
                    return Flow::Call(Frame::gain(f.targets[f.s.i], pile, Dest::Discard));
                }
            }
            _ => {
                f.s.i += 1;
                f.pc = 1;
            }
        }
    }
}

fn magpie(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            if g.is(c, Types::TREASURE) {
                g.put_in_hand(c, p);
            }
            f.pc = 2;
            if g.is(c, Types::ACTION | Types::VICTORY)
                && let Some(pile) = g.supply_pile(Card::Magpie)
            {
                return Flow::Call(Frame::gain(p, pile, Dest::Discard));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn messenger(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.player(p).deck.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Discard)
        }
        _ => {
            if input.is_yes() {
                let deck: Vec<CardId> = g.player(p).deck.iter().rev().copied().collect();
                for c in deck {
                    g.move_card(c, Zone::Discard(p), Pos::Top);
                }
                g.emit(Event::Moved {
                    player: p,
                    card: None,
                    to: Zone::Discard(p),
                });
            }
            Flow::done()
        }
    }
}

fn miser(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Put a Copper from your hand onto your Tavern mat",
                    "+$1 per Copper on your Tavern mat",
                ],
                1,
                1,
            )
        }
        _ => {
            let m = mode_mask(input);
            if m & 1 != 0
                && let Some(&c) = g.hand(p).iter().find(|&&c| g.card(c) == Card::Copper)
            {
                g.put_on_mat(c, p, Mat::Tavern);
            }
            if m & 2 != 0 {
                let n = tavern(g, p)
                    .iter()
                    .filter(|&&c| g.card(c) == Card::Copper)
                    .count();
                g.add_coins(p, n as u32);
            }
            Flow::done()
        }
    }
}

fn raze(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let mut candidates = g.hand(p).to_vec();
            let this = f.this();
            if g.zone(this) == Zone::InPlay(p) {
                candidates.push(this);
            }
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(candidates, 1, 1, Purpose::Trash)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.n = g.cost_of(c).coins as i32;
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 => {
            if f.s.n <= 0 {
                return Flow::done();
            }
            f.pc = 3;
            Flow::Call(Frame::look(p, f.s.n as usize))
        }
        3 => {
            f.s.cards = input.cards();
            if f.s.cards.is_empty() {
                return Flow::done();
            }
            f.pc = 4;
            f.ask_cards(f.s.cards.clone(), 1, 1, Purpose::Move)
        }
        4 => {
            let chosen = input.cards();
            for &c in &chosen {
                g.put_in_hand(c, p);
            }
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = 5;
            Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)))
        }
        _ => Flow::done(),
    }
}

fn storyteller(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let treasures = g.playable(p, Types::TREASURE);
            if f.s.n >= 3 || treasures.is_empty() {
                f.pc = 2;
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(treasures, 0, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 2;
                return Flow::Continue;
            };
            f.s.n += 1;
            f.pc = 0;
            Flow::Call(Frame::play(p, c, true, vec![]))
        }
        2 => {
            let mut paid = 0;
            if p == g.turn.player {
                paid = std::mem::take(&mut g.turn.coins) as usize;
            }
            f.pc = 3;
            Flow::Call(Frame::draw(p, 1 + paid))
        }
        _ => Flow::done(),
    }
}

fn warrior(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.s.m = g
                .in_play(p)
                .iter()
                .filter(|&&c| g.is(c, Types::TRAVELLER))
                .count() as i32;
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            let Some(&t) = f.targets.get(f.s.i) else {
                return Flow::done();
            };
            if f.s.n >= f.s.m {
                f.s.i += 1;
                f.s.n = 0;
                return Flow::Continue;
            }
            f.s.n += 1;
            f.pc = 2;
            Flow::Call(Frame::look(t, 1))
        }
        2 => {
            let Some(c) = input.card() else {
                f.s.i += 1;
                f.s.n = 0;
                f.pc = 1;
                return Flow::Continue;
            };
            f.s.pick = Some(c);
            f.pc = 3;
            Flow::Call(Frame::discard(f.targets[f.s.i], vec![c]))
        }
        _ => {
            let t = f.targets[f.s.i];
            let c = f.s.pick.take().expect("set in step 2");
            let cost = g.cost_of(c);
            f.pc = 1;
            if g.zone(c) == Zone::Discard(t)
                && cost.potions == 0
                && cost.debt == 0
                && (3..=4).contains(&cost.coins)
            {
                return Flow::Call(Frame::trash(t, vec![c]));
            }
            Flow::Continue
        }
    }
}

fn soldier(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let this = f.this();
            let others = g
                .in_play(p)
                .iter()
                .filter(|&&c| c != this && g.is(c, Types::ATTACK))
                .count();
            g.add_coins(p, others as u32);
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            while let Some(&t) = f.targets.get(f.s.i) {
                let hand = g.hand(t).to_vec();
                if hand.len() >= 4 {
                    f.pc = 2;
                    return f.ask_cards_for(t, hand, 1, 1, Purpose::Discard);
                }
                f.s.i += 1;
            }
            Flow::done()
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()))
        }
        _ => {
            f.s.i += 1;
            f.pc = 1;
            Flow::Continue
        }
    }
}

fn disciple(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.playable(p, Types::ACTION);
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
            f.s.card = Some(g.card(c));
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
            let card = f.s.card.expect("set in step 1");
            gain_chosen(f, copy_pile(g, card), Dest::Discard)
        }
        _ => Flow::done(),
    }
}

// ---- Events ----

fn quest(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_numbered(
                p,
                &["Discard an Attack", "Discard two Curses", "Discard six cards"],
                0,
                1,
                Purpose::Either,
            )
        }
        1 => {
            let (candidates, n) = match input.opt() {
                Some(Opt::Mode(0)) => (g.cards_in_hand_where(p, Types::ATTACK), 1),
                Some(Opt::Mode(1)) => (g.cards_in_hand_where(p, Types::CURSE), 2),
                Some(Opt::Mode(2)) => (g.hand(p).to_vec(), 6),
                _ => return Flow::done(),
            };
            if candidates.is_empty() {
                return Flow::done();
            }
            f.s.m = n;
            f.pc = 2;
            f.ask_cards(candidates, n as usize, n as usize, Purpose::Discard)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(p, input.cards()))
        }
        3 => {
            f.pc = 4;
            if input.cards().len() as i32 == f.s.m
                && let Some(pile) = g.supply_pile(Card::Gold)
            {
                return Flow::Call(Frame::gain(p, pile, Dest::Discard));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn save(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            add_buys(g, p, 1);
            f.pc = 1;
            f.ask_cards(g.hand(p).to_vec(), 1, 1, Purpose::SetAside)
        }
        _ => {
            let chosen = input.cards();
            for &c in &chosen {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
                g.emit(Event::Moved {
                    player: p,
                    card: None,
                    to: Zone::SetAside(p),
                });
            }
            g.add_lasting(
                Card::Save,
                p,
                None,
                vec![],
                Until::EndOfTurn,
                Scratch {
                    cards: chosen,
                    ..Scratch::default()
                },
            );
            Flow::done()
        }
    }
}

fn scouting_party(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            add_buys(g, p, 1);
            f.pc = 1;
            Flow::Call(Frame::look(p, 5))
        }
        1 => {
            f.s.cards = input.cards();
            let n = f.s.cards.len().min(3);
            f.pc = 2;
            f.ask_cards(f.s.cards.clone(), n, n, Purpose::Discard)
        }
        2 => {
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = 3;
            Flow::Call(Frame::discard(p, chosen))
        }
        3 => {
            f.s.cards.retain(|&c| g.zone(c) == Zone::Deck(p));
            if f.s.cards.len() < 2 {
                return Flow::done();
            }
            f.pc = 4;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        _ => {
            top_deck_ordered(g, p, &input.cards());
            Flow::done()
        }
    }
}

fn move_token(g: &mut Game, f: &mut CardFrame, input: Input, token: Token) -> Flow {
    match f.pc {
        0 => {
            let piles = action_supply_piles(g);
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Other)
        }
        _ => {
            if let Some(pile) = input.pile() {
                place_token(g, f.player, token, pile);
            }
            Flow::done()
        }
    }
}

fn pilgrimage(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            mark_bought(g, f);
            if !flip_journey(g, p) {
                return Flow::done();
            }
            let mut names = vec![];
            let mut candidates = vec![];
            for &c in g.in_play(p) {
                if !names.contains(&g.card(c)) {
                    names.push(g.card(c));
                    candidates.push(c);
                }
            }
            f.pc = 1;
            f.ask_cards(candidates, 0, 3, Purpose::Gain)
        }
        1 => {
            f.s.cards = input.cards();
            f.s.i = 0;
            f.pc = 2;
            Flow::Continue
        }
        _ => {
            while let Some(&c) = f.s.cards.get(f.s.i) {
                f.s.i += 1;
                if let Some(pile) = copy_pile(g, g.card(c)) {
                    return Flow::Call(Frame::gain(p, pile, Dest::Discard));
                }
            }
            Flow::done()
        }
    }
}

fn inheritance(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if inherited_card(g, p).is_none() {
                g.ext.adventures.inherited.push((p, None));
            }
            let piles = g.supply_piles_where(|t, cost| {
                t.contains(Types::ACTION)
                    && !t.intersects(Types::COMMAND | Types::DURATION)
                    && cost.at_most(Cost::coins(4))
            });
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::SetAside)
        }
        _ => {
            let Some(top) = input.pile().and_then(|pile| g.pile_top(pile)) else {
                return Flow::done();
            };
            // Set aside with its owner's cards, so that it counts when scoring.
            g.set_aside(top, p);
            g.players[p.index()].tokens.insert(Token::Estate, None);
            let st = &mut g.ext.adventures.inherited;
            st.retain(|&(q, _)| q != p);
            st.push((p, Some(top)));
            Flow::done()
        }
    }
}

/// An Estate played by the player with the Estate token: plays the card with the token,
/// leaving it there.
fn inherited_play(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    f.pc = 1;
    let Some(card) = inherited_card(g, f.player) else {
        return Flow::done();
    };
    if g.zone(card) != Zone::SetAside(f.player) {
        return Flow::done();
    }
    Flow::Call(Frame::play(f.player, card, false, f.chain_for_child()))
}
