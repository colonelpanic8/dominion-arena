//! Cornucopia & Guilds (2nd edition), and the removed 1st-edition Cornucopia and Guilds
//! cards with Tournament's Prizes.

use super::{CardFrame, Hook, SetRules};
use crate::decision::{Decision, DecisionKind, Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, OVERPAY, Value};
use crate::game::{Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup, add_pile};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, Phase, PileId, PlayerId, Set, Types};
use crate::Note;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Young Witch's Bane pile.
    pub bane: Option<PileId>,
    /// The pile set aside for Ferryman.
    pub ferryman: Option<PileId>,
}

pub(crate) struct Rules;

/// Resolves an overpay when the overpaid-for card is gained.
const OVERPAID: u8 = 0;
const FARMHANDS_SET_ASIDE: u8 = 1;
const FARMHANDS_PLAY: u8 = 2;
const FERRYMAN_GAIN: u8 = 0;
const FOOTPAD_DRAW: u8 = 0;
const JOUST_DISCARD: u8 = 0;
const MERCHANT_GUILD_COFFERS: u8 = 0;
const HORSE_TRADERS_SET_ASIDE: u8 = 0;
const HORSE_TRADERS_RETURN: u8 = 1;

const REWARDS: [Card; 6] = [
    Card::Coronet,
    Card::Courser,
    Card::Demesne,
    Card::Housecarl,
    Card::HugeTurnip,
    Card::Renown,
];
const PRIZES: [Card; 5] = [
    Card::BagOfGold,
    Card::Diadem,
    Card::Followers,
    Card::Princess,
    Card::TrustySteed,
];

/// Overpaying happens on buying; the overpay abilities happen when the card is gained.
const OVERPAY_CARDS: [Card; 6] = [
    Card::Farrier,
    Card::Herald,
    Card::Infirmary,
    Card::Stonemason,
    Card::Doctor,
    Card::Masterpiece,
];

impl SetRules for Rules {
    fn notes(g: &Game, out: &mut Vec<Note>) {
        let s = &g.ext.cornucopia_guilds;
        if let Some(pile) = s.bane {
            out.push(Note::new(Card::YoungWitch).pile(pile));
        }
        if let Some(pile) = s.ferryman {
            out.push(Note::new(Card::Ferryman).pile(pile));
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (c, Hook::Ability(OVERPAY)) if OVERPAY_CARDS.contains(&c) => {
                let data = Scratch {
                    n: f.data,
                    cost: f.s.cost,
                    i: g.turn.bought_cards.len(),
                    ..Scratch::default()
                };
                g.add_lasting(c, f.player, f.this, vec![], Until::EndOfTurn, data);
                Flow::done()
            }
            (c, Hook::Ability(OVERPAID)) if OVERPAY_CARDS.contains(&c) => {
                overpaid(g, f, input)
            }
            (Card::Farmhands, Hook::Ability(FARMHANDS_SET_ASIDE)) => farmhands(g, f, input),
            (Card::Farmhands, Hook::Ability(FARMHANDS_PLAY)) => {
                play_set_aside(g, f)
            }
            (Card::Ferryman, Hook::Ability(FERRYMAN_GAIN)) => match g.ext.cornucopia_guilds.ferryman
            {
                Some(pile) if f.pc == 0 => {
                    f.pc = 1;
                    Flow::Call(Frame::gain(f.player, pile, Dest::Discard))
                }
                _ => Flow::done(),
            },
            (Card::Footpad, Hook::Ability(FOOTPAD_DRAW)) => {
                if f.pc == 0 {
                    f.pc = 1;
                    Flow::Call(Frame::draw(f.player, 1))
                } else {
                    Flow::done()
                }
            }
            (Card::Joust, Hook::Ability(JOUST_DISCARD)) => {
                let id = f.lasting.expect("from a lasting");
                if let Some(l) = g.remove_lasting(id)
                    && let Some(c) = l.data.pick
                    && g.zone(c) == Zone::SetAside(l.owner)
                {
                    g.move_card(c, Zone::Discard(l.owner), Pos::Top);
                }
                Flow::done()
            }
            (Card::MerchantGuild, Hook::Ability(MERCHANT_GUILD_COFFERS)) => {
                let n = g.turn.gained_this_buy_phase().len();
                g.players[f.player.index()].coffers += n as u32;
                Flow::done()
            }
            (Card::HorseTraders, Hook::Ability(HORSE_TRADERS_SET_ASIDE)) => {
                let c = f.this();
                if g.zone(c) == Zone::Hand(f.player) {
                    g.set_aside(c, f.player);
                    let data = Scratch {
                        pick: Some(c),
                        ..Scratch::default()
                    };
                    g.add_lasting(
                        Card::HorseTraders,
                        f.player,
                        Some(c),
                        vec![],
                        Until::Removed,
                        data,
                    );
                }
                Flow::Done(Value::Bool(false))
            }
            (Card::HorseTraders, Hook::Ability(HORSE_TRADERS_RETURN)) => match f.pc {
                0 => {
                    f.pc = 1;
                    Flow::Call(Frame::draw(f.player, 1))
                }
                _ => {
                    let l = g.remove_lasting(f.lasting.expect("from a lasting"));
                    if let Some(c) = l.and_then(|l| l.data.pick)
                        && g.zone(c) == Zone::SetAside(f.player)
                    {
                        g.put_in_hand(c, f.player);
                    }
                    Flow::done()
                }
            },
            (_, Hook::Ability(_)) => return None,
            (Card::Advisor, _) => advisor(g, f, input),
            (Card::Butcher, _) => butcher(g, f, input),
            (Card::Carnival, _) => carnival(g, f, input),
            (Card::Ferryman, _) => discard_n(g, f, input, 1),
            (Card::Footpad, _) => discard_down(g, f, input, 3, 0).unwrap_or_else(Flow::done),
            (Card::Hamlet, _) => hamlet(g, f, input),
            (Card::Herald, _) => herald(g, f, input),
            (Card::HornOfPlenty, _) => horn_of_plenty(g, f, input),
            (Card::HuntingParty, _) => hunting_party(g, f, input),
            (Card::Infirmary, _) => trash_optional(g, f, input),
            (Card::Jester, _) => jester(g, f, input),
            (Card::Journeyman, _) => journeyman(g, f, input),
            (Card::Joust, _) => joust(g, f, input),
            (Card::Menagerie, _) => {
                if f.pc == 0 {
                    reveal_hand(g, f.player);
                    let hand = g.hand(f.player);
                    let n = if distinct_names(g, hand.iter().copied()) == hand.len() {
                        3
                    } else {
                        1
                    };
                    f.pc = 1;
                    Flow::Call(Frame::draw(f.player, n))
                } else {
                    Flow::done()
                }
            }
            (Card::MerchantGuild, _) | (Card::Renown, _) | (Card::Princess, _) => {
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
            (Card::Plaza, _) => plaza(g, f, input),
            (Card::Remake, _) => remake(g, f, input),
            (Card::Shop, _) => shop(g, f, input),
            (Card::Soothsayer, _) => soothsayer(g, f, input),
            (Card::Stonemason, _) => stonemason(g, f, input),
            (Card::YoungWitch, _) => young_witch(g, f, input),
            (Card::Coronet, _) => coronet(g, f, input),
            (Card::Courser, _) => courser(g, f, input, false),
            (Card::TrustySteed, _) => courser(g, f, input, true),
            (Card::Demesne, _) => gain_one(g, f, Card::Gold, Dest::Discard),
            (Card::Housecarl, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                    let actions: Vec<CardId> = g
                        .in_play(f.player)
                        .iter()
                        .copied()
                        .filter(|&c| g.is(c, Types::ACTION))
                        .collect();
                    Flow::Call(Frame::draw(f.player, distinct_names(g, actions)))
                } else {
                    Flow::done()
                }
            }
            (Card::HugeTurnip, _) => {
                let n = g.player(f.player).coffers;
                g.add_coins(f.player, n);
                Flow::done()
            }
            (Card::BagOfGold, _) => gain_one(g, f, Card::Gold, Dest::DeckTop),
            (Card::Diadem, _) => {
                g.add_coins(f.player, g.turn.actions);
                Flow::done()
            }
            (Card::Followers, _) => followers(g, f, input),
            (Card::Doctor, _) => doctor(g, f, input),
            (Card::FarmingVillage, _) => farming_village(g, f, input),
            (Card::FortuneTeller, _) => fortune_teller(g, f, input),
            (Card::Harvest, _) => harvest(g, f, input),
            (Card::HorseTraders, _) => discard_n(g, f, input, 2),
            (Card::Taxman, _) => taxman(g, f, input),
            (Card::Tournament, _) => tournament(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        match *occ {
            Occasion::Gain {
                player,
                card,
                bought,
            } => {
                for l in g.lasting() {
                    // The gain of the buy that was overpaid for (`data.i`: which buy of the
                    // turn), whichever copy it takes, and only by the player who overpaid
                    // (not a possessor gaining it).
                    if bought
                        && l.card == g.card(card)
                        && l.data.i == g.turn.bought_cards.len()
                        && l.owner == player
                        && OVERPAY_CARDS.contains(&l.card)
                    {
                        out.push(Trigger::new(player, l.card, OVERPAID).on(card).via_lasting(l.id));
                    }
                }
                match g.card(card) {
                    Card::Farmhands
                        if !g
                            .cards_in_hand_where(player, Types::ACTION | Types::TREASURE)
                            .is_empty() =>
                    {
                        out.push(
                            Trigger::new(player, Card::Farmhands, FARMHANDS_SET_ASIDE)
                                .on(card)
                                .optional(),
                        );
                    }
                    Card::Ferryman if g.ext.cornucopia_guilds.ferryman.is_some() => {
                        out.push(Trigger::new(player, Card::Ferryman, FERRYMAN_GAIN).on(card));
                    }
                    _ => {}
                }
                // "In games using this": wherever its cards are (the Black Market deck).
                if g.gain_phase(card).is_some_and(Phase::is_action)
                    && setup::in_game(g, |c| c == Card::Footpad)
                {
                    out.push(Trigger::new(player, Card::Footpad, FOOTPAD_DRAW));
                }
            }
            Occasion::StartTurn { player } => {
                for l in g.lasting() {
                    let ability = match l.card {
                        Card::Farmhands => FARMHANDS_PLAY,
                        Card::HorseTraders => HORSE_TRADERS_RETURN,
                        _ => continue,
                    };
                    // "Your next turn": not the turn the card was set aside on.
                    if l.owner == player && l.turn < g.player(player).turns_taken {
                        out.push(Trigger::new(player, l.card, ability).via_lasting(l.id));
                    }
                }
            }
            Occasion::DiscardFromPlay { player } if g.cleaning_up(player) => {
                for l in g.lasting() {
                    if l.card == Card::Joust {
                        out.push(Trigger::new(l.owner, Card::Joust, JOUST_DISCARD).via_lasting(l.id));
                    }
                }
            }
            Occasion::EndBuyPhase { player } => {
                for l in g.lasting() {
                    if l.card == Card::MerchantGuild && l.owner == player {
                        out.push(
                            Trigger::new(player, Card::MerchantGuild, MERCHANT_GUILD_COFFERS)
                                .via_lasting(l.id),
                        );
                    }
                }
            }
            Occasion::Attack { target, .. } => {
                for &c in g.hand(target) {
                    if g.card(c) == Card::HorseTraders {
                        out.push(
                            Trigger::new(target, Card::HorseTraders, HORSE_TRADERS_SET_ASIDE)
                                .on(c)
                                .optional(),
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
        let n: u8 = g
            .lasting()
            .iter()
            .filter(|l| matches!(l.card, Card::Renown | Card::Princess))
            .map(|_| 2)
            .sum();
        *cost = cost.minus_coins(n);
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        match card {
            Card::Fairgrounds => Some(2 * (distinct_names(g, g.player(p).owned()) / 5) as i32),
            Card::Demesne => Some(g.count_owned(p, Card::Gold) as i32),
            _ => None,
        }
    }

    fn ensure(g: &mut Game, s: &mut Setup) {
        let st = &g.ext.cornucopia_guilds;
        if st.bane.is_none()
            && setup::in_game(g, |c| c == Card::YoungWitch)
            && let Some(bane) = choose_unused_costing(g, s, Card::YoungWitch, 2, 3)
        {
            g.ext.cornucopia_guilds.bane = Some(setup::add_kingdom_pile(g, bane));
        }
        let st = &g.ext.cornucopia_guilds;
        if st.ferryman.is_none()
            && setup::in_game(g, |c| c == Card::Ferryman)
            && let Some(card) = choose_unused_costing(g, s, Card::Ferryman, 3, 4)
        {
            let pile = setup::add_kingdom_pile(g, card);
            g.piles[pile.index()].in_supply = false;
            g.ext.cornucopia_guilds.ferryman = Some(pile);
        }
        if setup::in_game(g, |c| c == Card::Tournament) && g.pile_of(PRIZES[0]).is_none() {
            for prize in PRIZES {
                add_pile(g, prize, 1, false);
            }
        }
        if setup::in_game(g, |c| c == Card::Joust) && g.pile_of(REWARDS[0]).is_none() {
            let n = if g.num_players() == 2 { 1 } else { 2 };
            for reward in REWARDS {
                add_pile(g, reward, n, false);
            }
        }
    }

    fn setup(g: &mut Game, s: &mut Setup) {
        if s.joined(Card::Baker) {
            for p in &mut g.players {
                p.coffers += 1;
            }
        }
    }
}

/// The unused Kingdom card costing $`lo` to $`hi` that `by` adds (Young Witch's Bane,
/// Ferryman's pile): any when configured, at random one from the sets in use.
fn choose_unused_costing(g: &mut Game, s: &mut Setup, by: Card, lo: u8, hi: u8) -> Option<Card> {
    let candidates = Card::kingdom_cards()
        .filter(|&c| {
            setup::unused(g, c)
                && c.cost().is_some_and(|k| {
                    k.potions == 0 && k.debt == 0 && (lo..=hi).contains(&k.coins)
                })
        })
        .collect();
    let sets: Vec<Set> = g.config.kingdom.iter().map(|c| c.set()).collect();
    s.choose_or_random(g, by, candidates, |c| sets.contains(&c.set()))
}

// ---- Helpers shared with other sets ----

/// The number of differently named cards among `ids`.
pub(crate) fn distinct_names(g: &Game, ids: impl IntoIterator<Item = CardId>) -> usize {
    let mut names: Vec<Card> = ids.into_iter().map(|c| g.card(c)).collect();
    names.sort();
    names.dedup();
    names.len()
}

/// A Supply pile a copy of `card` can be gained from (its top card is `card`).
pub(crate) fn copy_pile(g: &Game, card: Card) -> Option<PileId> {
    (0..g.piles.len()).map(|i| PileId(i as u8)).find(|&p| {
        g.pile(p).in_supply && g.pile_top(p).is_some_and(|t| g.card(t) == card)
    })
}

/// Whether a card being gained is still where it was gained to.
pub(crate) fn still_gained(g: &Game, card: CardId) -> bool {
    g.still_where_gained(card)
}

pub(crate) fn reveal_hand(g: &mut Game, p: PlayerId) {
    let cards = g.hand(p).iter().map(|&c| g.card(c)).collect();
    g.emit(Event::RevealedHand { player: p, cards });
}

/// Asks `player` to choose exactly `n` of `candidates` in order, top first.
pub(crate) fn ask_ordered(
    f: &CardFrame,
    player: PlayerId,
    candidates: Vec<CardId>,
    n: usize,
    purpose: Purpose,
) -> Flow {
    let n = n.min(candidates.len());
    Flow::Ask(Decision {
        player,
        kind: DecisionKind::Cards {
            candidates,
            min: n,
            max: n,
            ordered: true,
        },
        purpose,
        source: Some(f.card),
    })
}

/// Puts `cards` (listed top first) onto `p`'s deck without revealing them.
pub(crate) fn put_back(g: &mut Game, p: PlayerId, cards: &[CardId]) {
    for &c in cards.iter().rev() {
        g.move_card(c, Zone::Deck(p), Pos::Top);
    }
}

/// Gains a `card` from the Supply (if there is one) and finishes.
pub(crate) fn gain_one(g: &mut Game, f: &mut CardFrame, card: Card, to: Dest) -> Flow {
    if f.pc != 0 {
        return Flow::done();
    }
    f.pc = 1;
    match g.supply_pile(card) {
        Some(pile) => Flow::Call(Frame::gain(f.player, pile, to)),
        None => Flow::done(),
    }
}

/// Gains `f.data` copies of `card`, one at a time.
fn gain_n(g: &mut Game, f: &mut CardFrame, card: Card) -> Flow {
    if f.s.n >= f.data {
        return Flow::done();
    }
    f.s.n += 1;
    match g.supply_pile(card) {
        Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
        None => Flow::done(),
    }
}

/// Each of `players` from `f.s.i` on gains a `card`, one per call; `None` when all have.
pub(crate) fn each_gains(
    g: &Game,
    f: &mut CardFrame,
    players: &[PlayerId],
    card: Card,
) -> Option<Flow> {
    let &t = players.get(f.s.i)?;
    f.s.i += 1;
    Some(match g.supply_pile(card) {
        Some(pile) => Flow::Call(Frame::gain(t, pile, Dest::Discard)),
        None => Flow::Continue,
    })
}

/// Each of `f.targets` from `f.s.i` on discards down to `n` cards in hand. Uses pcs `at`
/// to `at + 2`; `None` when everyone has.
pub(crate) fn discard_down(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    n: usize,
    at: u8,
) -> Option<Flow> {
    if f.pc == at + 1 {
        f.pc = at + 2;
        return Some(Flow::Call(Frame::discard(f.targets[f.s.i], input.cards())));
    }
    if f.pc == at + 2 {
        f.s.i += 1;
    }
    f.pc = at;
    while let Some(&t) = f.targets.get(f.s.i) {
        let hand = g.hand(t).to_vec();
        if hand.len() > n {
            let k = hand.len() - n;
            f.pc = at + 1;
            return Some(f.ask_cards_for(t, hand, k, k, Purpose::Discard));
        }
        f.s.i += 1;
    }
    None
}

/// "Discard `n` cards."
pub(crate) fn discard_n(g: &mut Game, f: &mut CardFrame, input: Input, n: usize) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

/// "You may trash a card from your hand."
fn trash_optional(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, 0, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, input.cards()))
        }
        _ => Flow::done(),
    }
}

/// Reveals cards from `who`'s deck one at a time, setting aside each that doesn't match
/// `pred` in `f.s.cards`, until one matches. Uses pcs `at` and `at + 1`. `Ok(found)` when
/// done (`None` if the cards ran out); the match is left on top of the deck.
pub(crate) fn reveal_until(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    who: PlayerId,
    pred: impl Fn(&Game, CardId) -> bool,
    at: u8,
) -> Result<Option<CardId>, Flow> {
    if f.pc == at {
        f.pc = at + 1;
        return Err(Flow::Call(Frame::reveal(who, 1)));
    }
    match input.card() {
        None => Ok(None),
        Some(c) if pred(g, c) => Ok(Some(c)),
        Some(c) => {
            g.set_aside_revealed(c, who);
            f.s.cards.push(c);
            f.pc = at;
            Err(Flow::Continue)
        }
    }
}

/// Plays the card a lasting set aside, if it still is.
fn play_set_aside(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc != 0 {
        return Flow::done();
    }
    f.pc = 1;
    let l = g.remove_lasting(f.lasting.expect("from a lasting"));
    match l.and_then(|l| l.data.pick) {
        Some(c) if g.zone(c) == Zone::SetAside(f.player) => {
            Flow::Call(Frame::play(f.player, c, true, vec![]))
        }
        _ => Flow::done(),
    }
}

// ---- Card scripts ----

fn overpaid(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if let Some(id) = f.lasting.take() {
        let l = g.remove_lasting(id).expect("lasting exists");
        f.data = l.data.n;
        f.s.cost = l.data.cost;
    }
    let n = f.data;
    match f.card {
        Card::Masterpiece => gain_n(g, f, Card::Silver),
        Card::Doctor => doctor_overpay(g, f, input),
        Card::Farrier => {
            if f.player == g.turn.player {
                g.turn.cleanup_draw += n as usize;
            }
            Flow::done()
        }
        Card::Herald => match f.pc {
            0 => {
                let discard = g.player(f.player).discard.clone();
                if discard.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_cards(discard, n as usize, n as usize, Purpose::TopDeck)
            }
            1 => {
                let chosen = input.cards();
                f.pc = 2;
                f.ask_order(chosen, Purpose::TopDeck)
            }
            _ => {
                for &c in input.cards().iter().rev() {
                    g.top_deck(c, f.player);
                }
                Flow::done()
            }
        },
        Card::Infirmary => {
            let this = f.this();
            // The first play puts it into play from where it was gained.
            let first = f.s.i == 0;
            if f.s.i >= n as usize || (first && !still_gained(g, this)) {
                return Flow::done();
            }
            f.s.i += 1;
            Flow::Call(Frame::play(f.player, this, first, vec![]))
        }
        _ => {
            // Stonemason: gain 2 Actions each costing exactly the amount overpaid.
            match f.pc {
                0 => {
                    if f.s.i >= 2 {
                        return Flow::done();
                    }
                    f.s.i += 1;
                    let exact = f.s.cost;
                    let piles =
                        g.supply_piles_where(|t, c| c == exact && t.intersects(Types::ACTION));
                    if piles.is_empty() {
                        return Flow::done();
                    }
                    f.pc = 1;
                    f.ask_pile(piles, false, Purpose::Gain)
                }
                _ => {
                    f.pc = 0;
                    match input.pile() {
                        Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                        None => Flow::Continue,
                    }
                }
            }
        }
    }
}

fn farmhands(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let cands = g.cards_in_hand_where(f.player, Types::ACTION | Types::TREASURE);
            f.ask_cards(cands, 1, 1, Purpose::SetAside)
        }
        _ => {
            if let Some(c) = input.card() {
                g.set_aside(c, f.player);
                let data = Scratch {
                    pick: Some(c),
                    ..Scratch::default()
                };
                g.add_lasting(Card::Farmhands, f.player, f.this, vec![], Until::Removed, data);
            }
            Flow::done()
        }
    }
}

fn advisor(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, 3))
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

fn butcher(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, 0, 1, Purpose::Trash)
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
            let coffers = g.player(f.player).coffers.min(255) as u8;
            let limit = f.s.cost.plus_coins(coffers);
            let piles = g.piles_costing_up_to(limit);
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        3 => {
            let Some(pile) = input.pile() else {
                return Flow::done();
            };
            let top = g.pile_top(pile).expect("offered piles are not empty");
            let spend = g.cost_of(top).coins.saturating_sub(f.s.cost.coins);
            g.players[f.player.index()].coffers -= spend as u32;
            f.pc = 4;
            Flow::Call(Frame::gain(f.player, pile, Dest::Discard))
        }
        _ => Flow::done(),
    }
}

fn carnival(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, 4))
        }
        1 => {
            let mut names = vec![];
            let mut rest = vec![];
            for c in input.cards() {
                if names.contains(&g.card(c)) {
                    rest.push(c);
                } else {
                    names.push(g.card(c));
                    g.put_in_hand(c, f.player);
                }
            }
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, rest))
        }
        _ => Flow::done(),
    }
}

fn hamlet(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 | 3 => {
            f.pc += 1;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, 0, 1, Purpose::Discard)
        }
        1 | 4 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                f.pc += 2;
                return Flow::Continue;
            }
            f.pc += 1;
            Flow::Call(Frame::discard(f.player, chosen))
        }
        2 => {
            g.add_actions(f.player, 1);
            f.pc = 3;
            Flow::Continue
        }
        5 => {
            g.add_buys(f.player, 1);
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn herald(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, 1))
        }
        1 => {
            f.pc = 2;
            match input.card() {
                Some(c) if g.is(c, Types::ACTION) => {
                    Flow::Call(Frame::play(f.player, c, true, vec![]))
                }
                _ => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn horn_of_plenty(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let n = distinct_names(g, g.in_play(f.player).to_vec());
            let piles = g.piles_costing_up_to(Cost::coins(n.min(255) as u8));
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
        2 => {
            let gained = input.card();
            let this = f.this();
            f.pc = 3;
            if gained.is_some_and(|c| g.is(c, Types::VICTORY))
                && g.zone(this) == Zone::InPlay(f.player)
            {
                return Flow::Call(Frame::trash(f.player, vec![this]));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn hunting_party(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        reveal_hand(g, f.player);
        f.pc = 1;
    }
    if f.pc <= 2 {
        let p = f.player;
        let names: Vec<Card> = g.hand(p).iter().map(|&c| g.card(c)).collect();
        match reveal_until(g, f, input, p, |g, c| !names.contains(&g.card(c)), 1) {
            Err(flow) => return flow,
            Ok(found) => {
                if let Some(c) = found {
                    g.put_in_hand(c, p);
                }
                f.pc = 3;
                return Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)));
            }
        }
    }
    Flow::done()
}

fn jester(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        let Some(&t) = f.targets.get(f.s.i) else {
            return Flow::done();
        };
        match f.pc {
            0 => {
                f.pc = 1;
                return Flow::Call(Frame::reveal(t, 1));
            }
            1 => {
                let Some(c) = input.take().and_then(Input::card) else {
                    f.pc = 4;
                    continue;
                };
                f.s.pick = Some(c);
                f.pc = 2;
                return Flow::Call(Frame::discard(t, vec![c]));
            }
            2 => {
                let c = f.s.pick.expect("set in step 1");
                if g.is(c, Types::VICTORY) {
                    f.pc = 4;
                    if let Some(curse) = g.supply_pile(Card::Curse) {
                        return Flow::Call(Frame::gain(t, curse, Dest::Discard));
                    }
                    continue;
                }
                let Some(pile) = copy_pile(g, g.card(c)) else {
                    f.pc = 4;
                    continue;
                };
                f.s.pile = Some(pile);
                f.pc = 3;
                return f.ask_choose_for(
                    f.player,
                    vec![Opt::Player(t), Opt::Player(f.player)],
                    vec![format!("{t} gains it"), "You gain it".into()],
                    1,
                    1,
                    Purpose::Gain,
                );
            }
            3 => {
                f.pc = 4;
                let who = match input.take().and_then(Input::opt) {
                    Some(Opt::Player(q)) => q,
                    _ => t,
                };
                return Flow::Call(Frame::gain(who, f.s.pile.expect("set"), Dest::Discard));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
                return Flow::Continue;
            }
        }
    }
}

fn journeyman(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            let p = f.player;
            let named = f.s.card;
            if f.pc == 2 && f.s.n >= 3 {
                f.pc = 4;
                return Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)));
            }
            match reveal_until(g, f, input, p, |g, c| Some(g.card(c)) != named, 2) {
                Err(flow) => flow,
                Ok(Some(c)) => {
                    g.put_in_hand(c, p);
                    f.s.n += 1;
                    f.pc = 2;
                    Flow::Continue
                }
                Ok(None) => {
                    f.pc = 4;
                    Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)))
                }
            }
        }
        _ => Flow::done(),
    }
}

fn joust(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let provinces: Vec<CardId> = g
                .hand(f.player)
                .iter()
                .copied()
                .filter(|&c| g.card(c) == Card::Province)
                .collect();
            if provinces.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(provinces, 0, 1, Purpose::SetAside)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            g.set_aside(c, f.player);
            let data = Scratch {
                pick: Some(c),
                ..Scratch::default()
            };
            g.add_lasting(Card::Joust, f.player, f.this, vec![], Until::Removed, data);
            let piles: Vec<PileId> = REWARDS
                .iter()
                .filter_map(|&r| g.pile_of(r))
                .filter(|&p| !g.pile(p).cards.is_empty())
                .collect();
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        2 => {
            f.pc = 3;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Hand)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn plaza(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
        _ => {
            g.players[f.player.index()].coffers += 1;
            Flow::done()
        }
    }
}

fn remake(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if f.s.n >= 2 || g.hand(f.player).is_empty() {
                return Flow::done();
            }
            f.s.n += 1;
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
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
        2 => {
            let exact = f.s.cost.plus_coins(1);
            let piles = g.supply_piles_where(|_, c| c == exact);
            f.pc = 0;
            if piles.is_empty() {
                return Flow::Continue;
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        _ => {
            f.pc = 0;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
    }
}

fn shop(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let in_play: Vec<Card> = g.in_play(f.player).iter().map(|&c| g.card(c)).collect();
            let cands: Vec<CardId> = g
                .playable(f.player, Types::ACTION)
                .into_iter()
                .filter(|&c| !in_play.contains(&g.card(c)))
                .collect();
            f.pc = 1;
            f.ask_cards(cands, 0, 1, Purpose::Play)
        }
        1 => {
            f.pc = 2;
            match input.card() {
                Some(c) => Flow::Call(Frame::play(f.player, c, true, vec![])),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn soothsayer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            match g.supply_pile(Card::Gold) {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        1 => {
            let Some(&t) = f.targets.get(f.s.i) else {
                return Flow::done();
            };
            let Some(curse) = g.supply_pile(Card::Curse) else {
                return Flow::done();
            };
            f.pc = 2;
            Flow::Call(Frame::gain(t, curse, Dest::Discard))
        }
        2 => {
            let t = f.targets[f.s.i];
            f.pc = 3;
            if input.cards().is_empty() {
                return Flow::Continue;
            }
            Flow::Call(Frame::draw(t, 1))
        }
        _ => {
            f.s.i += 1;
            f.pc = 1;
            Flow::Continue
        }
    }
}

fn stonemason(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
        2 => {
            if f.s.n >= 2 {
                return Flow::done();
            }
            f.s.n += 1;
            let limit = f.s.cost;
            let piles = g.supply_piles_where(|_, c| c.less_than(limit));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        _ => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
    }
}

fn young_witch(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let hand = g.hand(f.player).to_vec();
            f.ask_cards(hand, 2, 2, Purpose::Discard)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, input.cards()))
        }
        2 => {
            let Some(&t) = f.targets.get(f.s.i) else {
                return Flow::done();
            };
            let bane = g.ext.cornucopia_guilds.bane;
            let banes: Vec<CardId> = g
                .hand(t)
                .iter()
                .copied()
                .filter(|&c| bane.is_some() && g.origin_pile(g.card(c)) == bane)
                .collect();
            if banes.is_empty() {
                f.pc = 4;
                return Flow::Continue;
            }
            f.s.pick = Some(banes[0]);
            f.pc = 3;
            f.ask_yes_no_for(t, Purpose::Reveal)
        }
        3 => {
            let t = f.targets[f.s.i];
            if input.is_yes() {
                let c = f.s.pick.expect("set in step 2");
                g.reveal(t, c);
                f.s.i += 1;
                f.pc = 2;
                return Flow::Continue;
            }
            f.pc = 4;
            Flow::Continue
        }
        _ => {
            let t = f.targets[f.s.i];
            f.s.i += 1;
            f.pc = 2;
            match g.supply_pile(Card::Curse) {
                Some(curse) => Flow::Call(Frame::gain(t, curse, Dest::Discard)),
                None => Flow::Continue,
            }
        }
    }
}

/// Plays a non-Reward card of `types` from hand twice; pcs `at` to `at + 2`. `None` when
/// done.
fn play_twice(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    types: Types,
    at: u8,
) -> Option<Flow> {
    if f.pc == at {
        let cands: Vec<CardId> = g
            .playable(f.player, types)
            .into_iter()
            .filter(|&c| !g.is(c, Types::REWARD))
            .collect();
        f.pc = at + 1;
        return Some(f.ask_cards(cands, 0, 1, Purpose::Play));
    }
    if f.pc == at + 1 {
        let c = input.card()?;
        f.s.pick = Some(c);
        f.pc = at + 2;
        return Some(Flow::Call(Frame::play(
            f.player,
            c,
            true,
            f.chain_for_child(),
        )));
    }
    if f.pc == at + 2 {
        f.pc = at + 3;
        let c = f.s.pick.expect("set");
        return Some(Flow::Call(Frame::play(
            f.player,
            c,
            false,
            f.chain_for_child(),
        )));
    }
    None
}

fn coronet(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc <= 3 {
        if let Some(flow) = play_twice(g, f, input, Types::ACTION, 0) {
            return flow;
        }
        f.pc = 4;
        return Flow::Continue;
    }
    play_twice(g, f, input, Types::TREASURE, 4).unwrap_or_else(Flow::done)
}

/// Courser and Trusty Steed: two different options, done in the order listed.
fn courser(g: &mut Game, f: &mut CardFrame, input: Input, steed: bool) -> Flow {
    if f.pc == 0 {
        f.pc = 1;
        let last = if steed {
            "Gain 4 Silvers and put your deck into your discard pile"
        } else {
            "Gain 4 Silvers"
        };
        return f.ask_modes(&["+2 Cards", "+2 Actions", "+$2", last], 2, 2);
    }
    if f.pc == 1 {
        for o in input.options() {
            if let Opt::Mode(m) = o {
                f.s.m |= 1 << m;
            }
        }
        f.pc = 2;
        if f.s.m & 1 != 0 {
            return Flow::Call(Frame::draw(f.player, 2));
        }
    }
    if f.pc == 2 {
        if f.s.m & 2 != 0 {
            g.add_actions(f.player, 2);
        }
        if f.s.m & 4 != 0 {
            g.add_coins(f.player, 2);
        }
        if f.s.m & 8 == 0 {
            return Flow::done();
        }
        f.pc = 3;
    }
    if f.s.n < 4 {
        f.s.n += 1;
        if let Some(pile) = g.supply_pile(Card::Silver) {
            return Flow::Call(Frame::gain(f.player, pile, Dest::Discard));
        }
    }
    if steed {
        let p = f.player;
        let deck: Vec<CardId> = g.player(p).deck.iter().rev().copied().collect();
        for c in deck {
            g.move_card(c, Zone::Discard(p), Pos::Top);
        }
    }
    Flow::done()
}

fn followers(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            match g.supply_pile(Card::Estate) {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        // Each other player in turn gains a Curse, then discards.
        1 => {
            let Some(&t) = f.targets.get(f.s.i) else {
                return Flow::done();
            };
            f.pc = 2;
            match g.supply_pile(Card::Curse) {
                Some(pile) => Flow::Call(Frame::gain(t, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        2 => {
            let t = f.targets[f.s.i];
            let hand = g.hand(t).to_vec();
            if hand.len() <= 3 {
                f.pc = 4;
                return Flow::Continue;
            }
            let n = hand.len() - 3;
            f.pc = 3;
            f.ask_cards_for(t, hand, n, n, Purpose::Discard)
        }
        3 => {
            f.pc = 4;
            Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()))
        }
        _ => {
            f.s.i += 1;
            f.pc = 1;
            Flow::Continue
        }
    }
}

fn doctor(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_name()
        }
        1 => {
            f.s.card = input.name();
            f.pc = 2;
            Flow::Call(Frame::reveal(f.player, 3))
        }
        2 => {
            let named = f.s.card;
            let (matches, rest): (Vec<_>, Vec<_>) =
                input.cards().into_iter().partition(|&c| Some(g.card(c)) == named);
            f.s.cards = rest;
            f.pc = 3;
            Flow::Call(Frame::trash(f.player, matches))
        }
        3 => {
            let p = f.player;
            f.s.cards.retain(|&c| g.zone(c) == Zone::Deck(p));
            f.pc = 4;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        _ => {
            put_back(g, f.player, &input.cards());
            Flow::done()
        }
    }
}

fn doctor_overpay(_g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            if f.s.n >= f.data {
                return Flow::done();
            }
            f.s.n += 1;
            f.pc = 1;
            Flow::Call(Frame::look(f.player, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            f.ask_either(&["Trash it", "Discard it", "Put it back"])
        }
        _ => {
            let c = f.s.pick.expect("set in step 1");
            f.pc = 0;
            match input.opt() {
                Some(Opt::Mode(0)) => Flow::Call(Frame::trash(f.player, vec![c])),
                Some(Opt::Mode(1)) => Flow::Call(Frame::discard(f.player, vec![c])),
                _ => Flow::Continue,
            }
        }
    }
}

fn farming_village(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc <= 1 {
        let p = f.player;
        match reveal_until(g, f, input, p, |g, c| g.is(c, Types::TREASURE | Types::ACTION), 0)
        {
            Err(flow) => return flow,
            Ok(found) => {
                if let Some(c) = found {
                    g.put_in_hand(c, p);
                }
                f.pc = 2;
                return Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)));
            }
        }
    }
    Flow::done()
}

fn fortune_teller(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(&t) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    if f.pc <= 1 {
        match reveal_until(g, f, input, t, |g, c| g.is(c, Types::VICTORY | Types::CURSE), 0) {
            Err(flow) => return flow,
            Ok(_) => {
                f.pc = 2;
                return Flow::Call(Frame::discard(t, std::mem::take(&mut f.s.cards)));
            }
        }
    }
    f.s.i += 1;
    f.pc = 0;
    Flow::Continue
}

fn harvest(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(f.player, 4))
        }
        1 => {
            let cards = input.cards();
            f.s.n = distinct_names(g, cards.iter().copied()) as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(f.player, cards))
        }
        _ => {
            g.add_coins(f.player, f.s.n as u32);
            Flow::done()
        }
    }
}

fn taxman(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            let treasures = g.cards_in_hand_where(f.player, Types::TREASURE);
            f.ask_cards(treasures, 0, 1, Purpose::Trash)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.cost = g.cost_of(c);
            f.s.card = Some(g.card(c));
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, vec![c]))
        }
        2 => {
            let Some(&t) = f.targets.get(f.s.i) else {
                f.pc = 4;
                return Flow::Continue;
            };
            if g.hand(t).len() < 5 {
                f.s.i += 1;
                return Flow::Continue;
            }
            let copies: Vec<CardId> = g
                .hand(t)
                .iter()
                .copied()
                .filter(|&c| Some(g.card(c)) == f.s.card)
                .collect();
            if copies.is_empty() {
                reveal_hand(g, t);
                f.s.i += 1;
                return Flow::Continue;
            }
            f.pc = 3;
            Flow::Call(Frame::discard(t, vec![copies[0]]))
        }
        3 => {
            f.s.i += 1;
            f.pc = 2;
            Flow::Continue
        }
        4 => {
            let limit = f.s.cost.plus_coins(3);
            let piles =
                g.supply_piles_where(|t, c| c.at_most(limit) && t.intersects(Types::TREASURE));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 5;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        5 => {
            f.pc = 6;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::DeckTop)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn tournament(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let players = g.players_from(f.player);
    match f.pc {
        0 => {
            while let Some(&q) = players.get(f.s.i) {
                let has = g.hand(q).iter().any(|&c| g.card(c) == Card::Province);
                if has {
                    f.pc = 1;
                    return f.ask_yes_no_for(q, Purpose::Reveal);
                }
                f.s.i += 1;
            }
            f.pc = 2;
            Flow::Continue
        }
        1 => {
            let q = players[f.s.i];
            if input.is_yes() {
                let province = *g
                    .hand(q)
                    .iter()
                    .find(|&&c| g.card(c) == Card::Province)
                    .expect("checked in step 0");
                g.reveal(q, province);
                if q == f.player {
                    f.s.pick = Some(province);
                } else {
                    f.s.m = 1;
                }
            }
            f.s.i += 1;
            f.pc = 0;
            Flow::Continue
        }
        2 => {
            let Some(c) = f.s.pick else {
                f.pc = 5;
                return Flow::Continue;
            };
            f.pc = 3;
            Flow::Call(Frame::discard(f.player, vec![c]))
        }
        3 => {
            let mut piles: Vec<PileId> = PRIZES
                .iter()
                .filter_map(|&r| g.pile_of(r))
                .filter(|&p| !g.pile(p).cards.is_empty())
                .collect();
            piles.extend(g.supply_pile(Card::Duchy));
            f.pc = 4;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        4 => {
            f.pc = 5;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::DeckTop)),
                None => Flow::Continue,
            }
        }
        5 => {
            f.pc = 6;
            if f.s.m != 0 {
                return Flow::done();
            }
            g.add_coins(f.player, 1);
            Flow::Call(Frame::draw(f.player, 1))
        }
        _ => Flow::done(),
    }
}
