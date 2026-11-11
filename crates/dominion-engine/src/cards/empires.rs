//! Empires: its Kingdom cards, the five split piles, the Castles, the Events and the
//! Landmarks.
//!
//! Debt is generic (`Player::debt`, taken when a card or Event with Debt in its cost is
//! bought, paid with `Move::PayDebt`). VP and Debt tokens on piles are `Pile::vp_tokens`
//! and `Pile::debt_tokens`; VP tokens on Landmarks are this set's [`State`]. Durations
//! (Archive, Enchantress) and Donate resolve from a lasting effect at the start of their
//! owner's next turn. Overlord plays a card where it sits in the Supply, so every "this"
//! a card moves is checked to still be in play.

use super::{CardFrame, Hook, SetRules, due_durations, duration_trigger};
use crate::decision::{Decision, DecisionKind, Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input};
use crate::game::{Phase, Pos, Scratch, Until, Zone};
use crate::setup::add_mixed_pile;
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Plus, Set, Types};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// VP tokens on the Landmarks that hold them.
    landmark_vp: Vec<(Card, i32)>,
    /// Obelisk's chosen pile.
    obelisk: Option<PileId>,
    /// Someone has gained a Province (Mountain Pass).
    province_gained: bool,
}

pub(crate) struct Rules;

/// A Duration's start-of-turn effect, and Donate's.
const START: u8 = 0;
const ENCHANT: u8 = 1;
/// "When you gain this".
const GAINED: u8 = 0;
/// "When you trash this".
const TRASHED: u8 = 1;
const CAPITAL_DEBT: u8 = 0;
const CHARM_GAIN: u8 = 0;
const ENCAMPMENT_RETURN: u8 = 0;
const GROUNDSKEEPER_VP: u8 = 0;
/// A Landmark moves a VP token from the gained card's pile onto itself.
const MOVE: u8 = 0;
/// A Landmark gives its VP tokens (or some of them).
const TAKE: u8 = 1;
const MOUNTAIN_PASS_BID: u8 = 0;
const TOMB_VP: u8 = 0;
const TAX_DEBT: u8 = 0;
const ARENA_DISCARD: u8 = 0;

/// A helper's `next` that finishes the ability instead of continuing.
const DONE: u8 = u8::MAX;

/// The five split piles: top half, bottom half.
const SPLIT_PILES: [(Card, Card); 5] = [
    (Card::Catapult, Card::Rocks),
    (Card::Encampment, Card::Plunder),
    (Card::Gladiator, Card::Fortune),
    (Card::Patrician, Card::Emporium),
    (Card::Settlers, Card::BustlingVillage),
];

/// Landmarks that start with 6 VP per player and give 2 at a time.
const TOKEN_LANDMARKS: [Card; 6] = [
    Card::Arena,
    Card::Basilica,
    Card::Baths,
    Card::Battlefield,
    Card::Colonnade,
    Card::Labyrinth,
];

impl Game {
    /// The VP tokens on a Landmark.
    pub fn landmark_vp(&self, landmark: Card) -> i32 {
        self.ext
            .empires
            .landmark_vp
            .iter()
            .find(|(l, _)| *l == landmark)
            .map_or(0, |(_, n)| *n)
    }

    /// The pile Obelisk chose, if Obelisk is in the game.
    pub fn obelisk_pile(&self) -> Option<PileId> {
        self.ext.empires.obelisk
    }
}

impl SetRules for Rules {
    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::Archive, Hook::Ability(START)) => archive_start(g, f, input),
            (Card::Enchantress, Hook::Ability(START)) => {
                if f.pc == 0 {
                    f.end_lasting(g);
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, 2)));
                }
                Flow::done()
            }
            (Card::Enchantress, Hook::Ability(ENCHANT)) => enchant(g, f),
            (Card::Donate, Hook::Ability(START)) => donate(g, f, input),
            (Card::Rocks, Hook::Ability(GAINED | TRASHED)) => {
                if f.pc == 0 {
                    f.pc = 1;
                    let p = f.player;
                    let to = if p == g.turn.player && g.turn.phase == Phase::Buy {
                        Dest::DeckTop
                    } else {
                        Dest::Hand
                    };
                    if let Some(pile) = g.supply_pile(Card::Silver) {
                        return Some(Flow::Call(Frame::gain(p, pile, to)));
                    }
                }
                Flow::done()
            }
            (Card::CrumblingCastle, Hook::Ability(GAINED | TRASHED)) => {
                if f.pc == 0 {
                    f.pc = 1;
                    add_vp(g, f.player, 1);
                    if let Some(pile) = g.supply_pile(Card::Silver) {
                        return Some(Flow::Call(Frame::gain(f.player, pile, Dest::Discard)));
                    }
                }
                Flow::done()
            }
            (Card::Emporium, Hook::Ability(GAINED)) => {
                if actions_in_play(g, f.player) >= 5 {
                    add_vp(g, f.player, 2);
                }
                Flow::done()
            }
            (Card::Fortune, Hook::Ability(GAINED)) => {
                if f.pc == 0 {
                    f.s.m = 0;
                    f.s.i = 0;
                    f.pc = 1;
                }
                let n = gladiators_in_play(g, f.player);
                gain_copies(g, f, input, Card::Gold, n, Dest::Discard, DONE)
            }
            (Card::Forum, Hook::Ability(GAINED)) => {
                plus_buys(g, f.player, 1);
                Flow::done()
            }
            (Card::HauntedCastle, Hook::Ability(GAINED)) => haunted_castle(g, f, input),
            (Card::SprawlingCastle, Hook::Ability(GAINED)) => sprawling_castle(g, f, input),
            (Card::GrandCastle, Hook::Ability(GAINED)) => {
                let p = f.player;
                emit_hand(g, p);
                let in_hand = g.cards_in_hand_where(p, Types::VICTORY).len();
                let in_play: usize = (0..g.num_players())
                    .map(|i| {
                        let q = PlayerId(i as u8);
                        g.in_play(q)
                            .iter()
                            .filter(|&&c| g.is(c, Types::VICTORY))
                            .count()
                    })
                    .sum();
                add_vp(g, p, (in_hand + in_play) as i32);
                Flow::done()
            }
            (Card::Temple, Hook::Ability(GAINED)) => {
                if let Some(pile) = g.home_pile(Card::Temple) {
                    take_pile_vp(g, f.player, pile);
                }
                Flow::done()
            }
            (Card::Villa, Hook::Ability(GAINED)) => {
                villa_gained(g, f);
                Flow::done()
            }
            (Card::Capital, Hook::Ability(CAPITAL_DEBT)) => capital_debt(g, f, input),
            (Card::Charm, Hook::Ability(CHARM_GAIN)) => charm_gain(g, f, input),
            (Card::Encampment, Hook::Ability(ENCAMPMENT_RETURN)) => {
                if let Some(l) = f.end_lasting(g)
                    && let Some(c) = l.data.pick
                    && g.zone(c) == Zone::SetAside(l.owner)
                {
                    g.return_to_pile(c);
                }
                Flow::done()
            }
            (Card::Groundskeeper, Hook::Ability(GROUNDSKEEPER_VP)) => {
                add_vp(g, f.player, f.data);
                Flow::done()
            }
            (Card::Aqueduct | Card::DefiledShrine, Hook::Ability(MOVE)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ
                    && let Some(pile) = g.home_pile(g.printed(card))
                    && g.pile(pile).vp_tokens > 0
                {
                    g.piles[pile.index()].vp_tokens -= 1;
                    add_landmark_vp(g, f.card, 1);
                }
                Flow::done()
            }
            (Card::Aqueduct | Card::DefiledShrine, Hook::Ability(TAKE)) => {
                take_landmark_vp(g, f.player, f.card, i32::MAX);
                Flow::done()
            }
            (Card::Arena, Hook::Ability(ARENA_DISCARD)) => arena(g, f, input),
            (
                Card::Basilica | Card::Baths | Card::Battlefield | Card::Colonnade | Card::Labyrinth,
                Hook::Ability(TAKE),
            ) => {
                take_landmark_vp(g, f.player, f.card, 2);
                Flow::done()
            }
            (Card::MountainPass, Hook::Ability(MOUNTAIN_PASS_BID)) => mountain_pass(g, f, input),
            (Card::Tomb, Hook::Ability(TOMB_VP)) => {
                add_vp(g, f.player, 1);
                Flow::done()
            }
            (Card::Tax, Hook::Ability(TAX_DEBT)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ
                    && let Some(pile) = g.home_pile(g.printed(card))
                {
                    let debt = std::mem::take(&mut g.piles[pile.index()].debt_tokens);
                    g.players[f.player.index()].debt += debt.max(0) as u32;
                }
                Flow::done()
            }
            (_, Hook::Ability(_)) => return None,
            // Kingdom cards and their piles.
            (Card::Archive, _) => archive(g, f, input),
            (Card::BustlingVillage, _) => take_from_discard(g, f, input, Card::Settlers),
            (Card::Catapult, _) => catapult(g, f, input),
            (Card::ChariotRace, _) => chariot_race(g, f, input),
            (Card::Charm, _) => charm(g, f, input),
            (Card::CityQuarter, _) => {
                if f.pc == 0 {
                    emit_hand(g, f.player);
                    let n = g.cards_in_hand_where(f.player, Types::ACTION).len();
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, n)));
                }
                Flow::done()
            }
            (Card::Crown, _) => crown(g, f, input),
            (Card::Encampment, _) => encampment(g, f, input),
            (Card::Enchantress, _) => {
                let data = Scratch {
                    n: mask(&f.targets),
                    ..Scratch::default()
                };
                f.add_duration(g, data);
                Flow::done()
            }
            (Card::Engineer, _) => engineer(g, f, input),
            (Card::FarmersMarket, _) => farmers_market(g, f),
            (Card::Fortune, _) => {
                let p = f.player;
                let doubled = g.lasting().iter().any(|l| {
                    l.card == Card::Fortune && l.owner == p && l.until == Until::EndOfTurn
                });
                if p == g.turn.player && !doubled {
                    g.add_lasting(
                        Card::Fortune,
                        p,
                        f.this,
                        vec![],
                        Until::EndOfTurn,
                        Scratch::default(),
                    );
                    g.add_coins(p, g.turn.coins);
                }
                Flow::done()
            }
            (Card::Forum, _) => discard_from_hand(g, f, input, 2),
            (Card::Gladiator, _) => gladiator(g, f, input),
            (Card::Groundskeeper, _) => {
                g.add_lasting(
                    Card::Groundskeeper,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::Legionary, _) => legionary(g, f, input),
            (Card::OpulentCastle, _) => opulent_castle(g, f, input),
            (Card::Overlord, _) => overlord(g, f, input),
            (Card::Patrician, _) => patrician(g, f, input),
            (Card::Plunder, _) => {
                add_vp(g, f.player, 1);
                Flow::done()
            }
            (Card::RoyalBlacksmith, _) => {
                if f.pc == 0 {
                    let p = f.player;
                    emit_hand(g, p);
                    let coppers: Vec<CardId> = g
                        .hand(p)
                        .iter()
                        .copied()
                        .filter(|&c| g.card(c) == Card::Copper)
                        .collect();
                    f.pc = 1;
                    return Some(Flow::Call(Frame::discard(p, coppers)));
                }
                Flow::done()
            }
            (Card::Sacrifice, _) => sacrifice(g, f, input),
            (Card::Settlers, _) => take_from_discard(g, f, input, Card::Copper),
            (Card::SmallCastle, _) => small_castle(g, f, input),
            (Card::Temple, _) => temple(g, f, input),
            (Card::WildHunt, _) => wild_hunt(g, f, input),
            // Events.
            (Card::Advance, _) => advance(g, f, input),
            (Card::Annex, _) => annex(g, f, input),
            (Card::Banquet, _) => banquet(g, f, input),
            (Card::Conquest, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                }
                if f.pc == 1 {
                    return Some(gain_copies(
                        g,
                        f,
                        input,
                        Card::Silver,
                        2,
                        Dest::Discard,
                        2,
                    ));
                }
                let silvers = g.turn.gained.iter().filter(|&&c| c == Card::Silver).count();
                add_vp(g, f.player, silvers as i32);
                Flow::done()
            }
            (Card::Delve, _) => {
                if f.pc == 0 {
                    plus_buys(g, f.player, 1);
                    f.pc = 1;
                    if let Some(pile) = g.supply_pile(Card::Silver) {
                        return Some(Flow::Call(Frame::gain(f.player, pile, Dest::Discard)));
                    }
                }
                Flow::done()
            }
            (Card::Dominate, _) => match f.pc {
                0 => {
                    f.pc = 1;
                    match g.supply_pile(Card::Province) {
                        Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                        None => Flow::done(),
                    }
                }
                _ => {
                    if !input.cards().is_empty() {
                        add_vp(g, f.player, 9);
                    }
                    Flow::done()
                }
            },
            (Card::Donate, _) => {
                g.add_lasting(
                    Card::Donate,
                    f.player,
                    None,
                    vec![],
                    Until::Removed,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::Ritual, _) => ritual(g, f, input),
            (Card::SaltTheEarth, _) => salt_the_earth(g, f, input),
            (Card::Tax, _) => match f.pc {
                0 => {
                    let piles: Vec<PileId> = (0..g.piles().len())
                        .map(|i| PileId(i as u8))
                        .filter(|&id| g.pile(id).in_supply)
                        .collect();
                    f.pc = 1;
                    f.ask_pile(piles, false, Purpose::Other)
                }
                _ => {
                    if let Some(pile) = input.pile() {
                        g.piles[pile.index()].debt_tokens += 2;
                    }
                    Flow::done()
                }
            },
            (Card::Triumph, _) => match f.pc {
                0 => {
                    f.pc = 1;
                    match g.supply_pile(Card::Estate) {
                        Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                        None => Flow::done(),
                    }
                }
                _ => {
                    if !input.cards().is_empty() && f.player == g.turn.player {
                        let vp = g.turn.gained.len() as i32;
                        add_vp(g, f.player, vp);
                    }
                    Flow::done()
                }
            },
            (Card::Wedding, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                    add_vp(g, f.player, 1);
                    if let Some(pile) = g.supply_pile(Card::Gold) {
                        return Some(Flow::Call(Frame::gain(f.player, pile, Dest::Discard)));
                    }
                }
                Flow::done()
            }
            (Card::Windfall, _) => {
                if f.pc == 0 {
                    let pl = g.player(f.player);
                    if !pl.deck.is_empty() || !pl.discard.is_empty() {
                        return Some(Flow::done());
                    }
                    f.pc = 1;
                }
                gain_copies(g, f, input, Card::Gold, 3, Dest::Discard, DONE)
            }
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in due_durations(g, occ) {
            match l.card {
                Card::Archive | Card::Enchantress => out.push(duration_trigger(l, START)),
                Card::Donate => out.push(duration_trigger(l, START).first()),
                _ => {}
            }
        }
        match *occ {
            Occasion::Play { player, card } => enchant_triggers(g, player, card, out),
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            Occasion::Trash { player, card } => {
                let kind = g.card(card);
                if matches!(kind, Card::Rocks | Card::CrumblingCastle) {
                    out.push(Trigger::new(player, kind, TRASHED).on(card));
                }
                if g.has_landscape(Card::Tomb) {
                    out.push(Trigger::new(player, Card::Tomb, TOMB_VP).on(card));
                }
            }
            Occasion::StartBuyPhase { player } => {
                if g.has_landscape(Card::Arena)
                    && !g.cards_in_hand_where(player, Types::ACTION).is_empty()
                {
                    out.push(Trigger::new(player, Card::Arena, ARENA_DISCARD).optional());
                }
            }
            Occasion::StartCleanup { .. } => {
                for l in g.lasting() {
                    let set_aside = l
                        .data
                        .pick
                        .is_some_and(|c| g.zone(c) == Zone::SetAside(l.owner));
                    if l.card == Card::Encampment && set_aside {
                        out.push(
                            Trigger::new(l.owner, Card::Encampment, ENCAMPMENT_RETURN)
                                .via_lasting(l.id),
                        );
                    }
                }
            }
            Occasion::DiscardFromPlay { player } => {
                for &c in g.in_play(player) {
                    let kept = g.lasting().iter().any(|l| l.keeps.contains(&c));
                    if g.card(c) == Card::Capital && !kept {
                        out.push(Trigger::new(player, Card::Capital, CAPITAL_DEBT).on(c));
                    }
                }
            }
            Occasion::EndTurn { player } => {
                if g.has_landscape(Card::Baths)
                    && player == g.turn.player
                    && g.turn.gained.is_empty()
                    && g.landmark_vp(Card::Baths) > 0
                {
                    out.push(Trigger::new(player, Card::Baths, TAKE));
                }
            }
            _ => {}
        }
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        let castles = || {
            g.player(p)
                .owned()
                .filter(|&c| g.is(c, Types::CASTLE))
                .count() as i32
        };
        match card {
            Card::HumbleCastle => Some(castles()),
            Card::KingsCastle => Some(2 * castles()),
            _ => None,
        }
    }

    fn bonus_vp(g: &Game, p: PlayerId) -> i32 {
        let landmarks: Vec<Card> = g
            .landscapes()
            .iter()
            .copied()
            .filter(|l| l.set() == Set::Empires && l.kind() == Kind::Landmark)
            .collect();
        if landmarks.is_empty() {
            return 0;
        }
        let owned: Vec<CardId> = g.player(p).owned().collect();
        let mut counts: BTreeMap<Card, i32> = BTreeMap::new();
        for &c in &owned {
            *counts.entry(g.card(c)).or_default() += 1;
        }
        let count = |card: Card| counts.get(&card).copied().unwrap_or(0);
        let action_counts = || -> Vec<i32> {
            let mut by_name: BTreeMap<Card, i32> = BTreeMap::new();
            for &c in &owned {
                if g.is(c, Types::ACTION) {
                    *by_name.entry(g.card(c)).or_default() += 1;
                }
            }
            by_name.into_values().collect()
        };
        landmarks
            .into_iter()
            .map(|l| match l {
                Card::BanditFort => -2 * (count(Card::Silver) + count(Card::Gold)),
                Card::Fountain => {
                    if count(Card::Copper) >= 10 {
                        15
                    } else {
                        0
                    }
                }
                Card::Keep => {
                    let treasures: Vec<Card> = owned
                        .iter()
                        .filter(|&&c| g.is(c, Types::TREASURE))
                        .map(|&c| g.card(c))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let most = treasures.iter().filter(|&&t| {
                        g.others(p)
                            .into_iter()
                            .all(|q| g.count_owned(q, t) as i32 <= count(t))
                    });
                    5 * most.count() as i32
                }
                Card::Museum => 2 * counts.len() as i32,
                Card::Obelisk => match g.obelisk_pile() {
                    Some(pile) => {
                        let n = owned
                            .iter()
                            .filter(|&&c| g.home_pile(g.printed(c)) == Some(pile))
                            .count();
                        2 * n as i32
                    }
                    None => 0,
                },
                Card::Orchard => 4 * action_counts().into_iter().filter(|&n| n >= 3).count() as i32,
                Card::Palace => {
                    3 * count(Card::Copper)
                        .min(count(Card::Silver))
                        .min(count(Card::Gold))
                }
                Card::Tower => {
                    let n = owned
                        .iter()
                        .filter(|&&c| {
                            !g.is(c, Types::VICTORY)
                                && g.home_pile(g.printed(c)).is_some_and(|pile| {
                                    g.pile(pile).in_supply && g.pile(pile).cards.is_empty()
                                })
                        })
                        .count();
                    n as i32
                }
                Card::TriumphalArch => {
                    let mut v = action_counts();
                    v.sort_unstable_by(|a, b| b.cmp(a));
                    3 * v.get(1).copied().unwrap_or(0)
                }
                Card::Wall => -(owned.len() as i32 - 15).max(0),
                Card::WolfDen => -3 * counts.values().filter(|&&n| n == 1).count() as i32,
                _ => 0,
            })
            .sum()
    }

    fn build_kingdom_pile(g: &mut Game, card: Card) -> bool {
        if let Some(&(top, bottom)) = SPLIT_PILES.iter().find(|(top, _)| *top == card) {
            let mut cards = vec![top; 5];
            cards.extend([bottom; 5]);
            add_mixed_pile(g, top, &cards, true);
            return true;
        }
        if card != Card::Castles {
            return false;
        }
        let mut castles = Card::Castles.pile_members();
        if g.num_players() > 2 {
            castles.extend([
                Card::HumbleCastle,
                Card::SmallCastle,
                Card::OpulentCastle,
                Card::KingsCastle,
            ]);
        }
        castles.sort_by_key(|c| c.cost().unwrap_or_default().coins);
        add_mixed_pile(g, Card::Castles, &castles, true);
        true
    }

    fn setup(g: &mut Game) {
        let players = g.num_players() as i32;
        let supply: Vec<PileId> = (0..g.piles.len())
            .map(|i| PileId(i as u8))
            .filter(|&id| g.pile(id).in_supply)
            .collect();
        let action_piles: Vec<PileId> = supply
            .iter()
            .copied()
            .filter(|&id| g.pile_top(id).is_some_and(|top| g.is(top, Types::ACTION)))
            .collect();
        for l in g.landscapes.clone() {
            match l {
                _ if TOKEN_LANDMARKS.contains(&l) => {
                    g.ext.empires.landmark_vp.push((l, 6 * players));
                }
                Card::Aqueduct => {
                    g.ext.empires.landmark_vp.push((l, 0));
                    for card in [Card::Silver, Card::Gold] {
                        if let Some(pile) = g.supply_pile(card) {
                            g.piles[pile.index()].vp_tokens += 8;
                        }
                    }
                }
                Card::DefiledShrine => {
                    g.ext.empires.landmark_vp.push((l, 0));
                    for &pile in &action_piles {
                        let gathering = g.pile_top(pile).is_some_and(|t| g.is(t, Types::GATHERING));
                        if !gathering {
                            g.piles[pile.index()].vp_tokens += 2;
                        }
                    }
                }
                Card::Obelisk if !action_piles.is_empty() => {
                    let i = g.random_index(action_piles.len());
                    g.ext.empires.obelisk = Some(action_piles[i]);
                }
                Card::Tax => {
                    for &pile in &supply {
                        g.piles[pile.index()].debt_tokens += 1;
                    }
                }
                _ => {}
            }
        }
    }
}

fn gain_triggers(g: &Game, player: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let kind = g.card(card);
    let index = g
        .gains
        .iter()
        .rev()
        .find(|st| st.card == card)
        .and_then(|st| st.turn_index);
    let my_turn = player == g.turn.player;
    let buy_phase = my_turn && g.turn.phase == Phase::Buy;
    let own = match kind {
        Card::Rocks
        | Card::CrumblingCastle
        | Card::Forum
        | Card::SprawlingCastle
        | Card::GrandCastle
        | Card::Villa => true,
        Card::Emporium => actions_in_play(g, player) >= 5,
        Card::Fortune => gladiators_in_play(g, player) > 0,
        Card::HauntedCastle => my_turn,
        Card::Temple => g
            .home_pile(Card::Temple)
            .is_some_and(|pile| g.pile(pile).vp_tokens > 0),
        _ => false,
    };
    if own {
        out.push(Trigger::new(player, kind, GAINED).on(card));
    }
    for l in g.lasting() {
        if l.card == Card::Charm
            && l.owner == player
            && l.until == Until::EndOfTurn
            && index == Some(l.data.n as usize)
        {
            out.push(Trigger::new(player, Card::Charm, CHARM_GAIN).via_lasting(l.id));
        }
    }
    let victory = g.is(card, Types::VICTORY);
    let action = g.is(card, Types::ACTION);
    if victory {
        let n = g
            .lasting()
            .iter()
            .filter(|l| l.card == Card::Groundskeeper && l.owner == player)
            .count();
        if n > 0 {
            out.push(Trigger::new(player, Card::Groundskeeper, GROUNDSKEEPER_VP).data(n as i32));
        }
    }
    let pile_vp = g
        .home_pile(g.printed(card))
        .map_or(0, |pile| g.pile(pile).vp_tokens);
    let has = |l: Card| g.has_landscape(l);
    let tokens = |l: Card| g.landmark_vp(l) > 0;
    if has(Card::Aqueduct) {
        if g.is(card, Types::TREASURE) && pile_vp > 0 {
            out.push(Trigger::new(player, Card::Aqueduct, MOVE));
        }
        if victory && tokens(Card::Aqueduct) {
            out.push(Trigger::new(player, Card::Aqueduct, TAKE));
        }
    }
    if has(Card::DefiledShrine) {
        if action && pile_vp > 0 {
            out.push(Trigger::new(player, Card::DefiledShrine, MOVE));
        }
        if kind == Card::Curse && buy_phase && tokens(Card::DefiledShrine) {
            out.push(Trigger::new(player, Card::DefiledShrine, TAKE));
        }
    }
    if has(Card::Basilica) && buy_phase && g.turn.coins >= 2 && tokens(Card::Basilica) {
        out.push(Trigger::new(player, Card::Basilica, TAKE));
    }
    if has(Card::Battlefield) && victory && tokens(Card::Battlefield) {
        out.push(Trigger::new(player, Card::Battlefield, TAKE));
    }
    if has(Card::Colonnade) && action && buy_phase && tokens(Card::Colonnade) {
        let copy = g
            .in_play(player)
            .iter()
            .any(|&c| c != card && g.card(c) == kind);
        if copy {
            out.push(Trigger::new(player, Card::Colonnade, TAKE));
        }
    }
    if has(Card::Labyrinth) && index == Some(1) && tokens(Card::Labyrinth) {
        out.push(Trigger::new(player, Card::Labyrinth, TAKE));
    }
    if has(Card::MountainPass) && kind == Card::Province && !g.ext.empires.province_gained {
        out.push(Trigger::new(player, Card::MountainPass, MOUNTAIN_PASS_BID));
    }
    if has(Card::Tax)
        && buy_phase
        && g.home_pile(g.printed(card))
            .is_some_and(|pile| g.pile(pile).debt_tokens > 0)
    {
        out.push(Trigger::new(player, Card::Tax, TAX_DEBT));
    }
}

/// Enchantress: the first Action card each affected player plays on their turn.
fn enchant_triggers(g: &Game, player: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let first_action = player == g.turn.player
        && g.is(card, Types::ACTION)
        && g.turn.played.iter().filter(|c| c.is(Types::ACTION)).count() == 1;
    let open = g
        .plays
        .last()
        .is_some_and(|st| st.card == card && st.replaced.is_none() && !st.blocked);
    if !first_action || !open {
        return;
    }
    for l in g.lasting() {
        if l.card == Card::Enchantress
            && l.until == Until::Removed
            && l.owner != player
            && in_mask(l.data.n, player)
        {
            out.push(Trigger::new(l.owner, Card::Enchantress, ENCHANT).via_lasting(l.id));
        }
    }
}

fn enchant(g: &mut Game, f: &mut CardFrame) -> Flow {
    if let Some(Occasion::Play { card, .. }) = f.occ
        && let Some(st) = g.plays.last_mut()
        && st.card == card
        && st.replaced.is_none()
        && !st.blocked
    {
        st.replaced = Some(Plus {
            cards: 1,
            actions: 1,
            ..Plus::default()
        });
    }
    Flow::done()
}

// ---- Helpers ----

/// Continues the ability at `next`, or finishes it if `next` is [`DONE`].
fn goto(f: &mut CardFrame, next: u8) -> Flow {
    if next == DONE {
        return Flow::done();
    }
    f.pc = next;
    Flow::Continue
}

fn plus_actions(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.turn.actions += n;
    }
}

fn plus_buys(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.turn.buys += n;
    }
}

fn add_vp(g: &mut Game, p: PlayerId, n: i32) {
    g.players[p.index()].vp_tokens += n;
}

fn add_landmark_vp(g: &mut Game, landmark: Card, n: i32) {
    let vp = &mut g.ext.empires.landmark_vp;
    match vp.iter_mut().find(|(l, _)| *l == landmark) {
        Some((_, v)) => *v += n,
        None => vp.push((landmark, n)),
    }
}

/// Takes up to `n` VP tokens from a Landmark.
fn take_landmark_vp(g: &mut Game, p: PlayerId, landmark: Card, n: i32) {
    let n = n.min(g.landmark_vp(landmark)).max(0);
    add_landmark_vp(g, landmark, -n);
    add_vp(g, p, n);
}

fn take_pile_vp(g: &mut Game, p: PlayerId, pile: PileId) {
    let n = std::mem::take(&mut g.piles[pile.index()].vp_tokens);
    add_vp(g, p, n);
}

fn add_pile_vp(g: &mut Game, card: Card, n: i32) {
    if let Some(pile) = g.home_pile(card) {
        g.piles[pile.index()].vp_tokens += n;
    }
}

fn actions_in_play(g: &Game, p: PlayerId) -> usize {
    g.in_play(p)
        .iter()
        .filter(|&&c| g.is(c, Types::ACTION))
        .count()
}

fn gladiators_in_play(g: &Game, p: PlayerId) -> usize {
    g.in_play(p)
        .iter()
        .filter(|&&c| g.card(c) == Card::Gladiator)
        .count()
}

fn mask(players: &[PlayerId]) -> i32 {
    players.iter().fold(0, |m, p| m | 1 << p.0)
}

fn in_mask(mask: i32, p: PlayerId) -> bool {
    mask & (1 << p.0) != 0
}

fn emit_hand(g: &mut Game, p: PlayerId) {
    let cards = g.hand(p).iter().map(|&c| g.card(c)).collect();
    g.emit(Event::RevealedHand { player: p, cards });
}

fn emit_revealed(g: &mut Game, p: PlayerId, cards: &[CardId]) {
    for &c in cards {
        g.emit(Event::Revealed {
            player: p,
            card: g.card(c),
        });
    }
}

fn dest_zone(to: Dest, p: PlayerId) -> Zone {
    match to {
        Dest::Discard => Zone::Discard(p),
        Dest::Hand => Zone::Hand(p),
        Dest::DeckTop => Zone::Deck(p),
        Dest::SetAside => Zone::SetAside(p),
    }
}

/// Gains `n` copies of `card` from the Supply one at a time, counting the ones gained in
/// `f.s.m`, then continues at `next`. Start it with `f.s.i == 0` and `f.s.m == 0`.
fn gain_copies(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    card: Card,
    n: usize,
    to: Dest,
    next: u8,
) -> Flow {
    if f.s.i > 0 && !input.cards().is_empty() {
        f.s.m += 1;
    }
    if f.s.i < n
        && let Some(pile) = g.supply_pile(card)
    {
        f.s.i += 1;
        return Flow::Call(Frame::gain(f.player, pile, to));
    }
    f.s.i = 0;
    goto(f, next)
}

/// Asks to choose one of `piles` (computed by the caller when `f.pc == at`) and gains its
/// top card: asks at `at`, gains at `at + 1`, then puts the gained cards in `f.s.cards` and
/// goes on at `next`. With no pile to choose, goes straight on.
fn choose_and_gain(
    f: &mut CardFrame,
    input: Input,
    at: u8,
    piles: Vec<PileId>,
    to: Dest,
    next: u8,
) -> Flow {
    if f.pc == at {
        if piles.is_empty() {
            f.s.cards.clear();
            return goto(f, next);
        }
        f.pc = at + 1;
        return f.ask_pile(piles, false, Purpose::Gain);
    }
    if f.pc == at + 1 {
        f.pc = at + 2;
        if let Some(pile) = input.pile() {
            return Flow::Call(Frame::gain(f.player, pile, to));
        }
        f.s.cards.clear();
        return goto(f, next);
    }
    f.s.cards = input.cards();
    goto(f, next)
}

/// Discards exactly `n` cards from hand (fewer if the hand is smaller).
fn discard_from_hand(g: &mut Game, f: &mut CardFrame, input: Input, n: usize) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
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

/// Settlers / Bustling Village: you may reveal a `card` from your discard pile and put it
/// into your hand.
fn take_from_discard(g: &mut Game, f: &mut CardFrame, input: Input, card: Card) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let found: Vec<CardId> = g
                .player(p)
                .discard
                .iter()
                .copied()
                .filter(|&c| g.card(c) == card)
                .collect();
            if found.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(found, 0, 1, Purpose::Move)
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::Discard(p)
            {
                emit_revealed(g, p, &[c]);
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
    }
}

// ---- Kingdom cards ----

fn archive(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 3))
        }
        1 => {
            let cards = input.cards();
            for &c in &cards {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
                g.emit(Event::Moved {
                    player: p,
                    card: None,
                    to: Zone::SetAside(p),
                });
            }
            if cards.is_empty() {
                return Flow::done();
            }
            f.s.cards = cards.clone();
            f.pc = 2;
            f.ask_cards(cards, 1, 1, Purpose::Move)
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::SetAside(p)
            {
                g.put_in_hand(c, p);
            }
            let rest: Vec<CardId> = std::mem::take(&mut f.s.cards)
                .into_iter()
                .filter(|&c| g.zone(c) == Zone::SetAside(p))
                .collect();
            if !rest.is_empty() {
                let data = Scratch {
                    cards: rest,
                    ..Scratch::default()
                };
                f.add_duration(g, data);
            }
            Flow::done()
        }
    }
}

fn archive_start(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let Some(id) = f.lasting else {
        return Flow::done();
    };
    let held = |g: &Game| -> Vec<CardId> {
        g.lasting()
            .iter()
            .find(|l| l.id == id)
            .map(|l| l.data.cards.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|&c| g.zone(c) == Zone::SetAside(p))
            .collect()
    };
    match f.pc {
        0 => {
            let cards = held(g);
            if cards.is_empty() {
                f.end_lasting(g);
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(cards, 1, 1, Purpose::Move)
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::SetAside(p)
            {
                g.put_in_hand(c, p);
            }
            let rest = held(g);
            if rest.is_empty() {
                f.end_lasting(g);
            } else if let Some(l) = g.lasting_mut(id) {
                l.data.cards = rest;
            }
            Flow::done()
        }
    }
}

fn catapult(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    loop {
        match f.pc {
            0 => {
                let hand = g.hand(p).to_vec();
                if hand.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                return f.ask_cards(hand, 1, 1, Purpose::Trash);
            }
            1 => {
                let Some(c) = input.card() else {
                    return Flow::done();
                };
                f.s.pick = Some(c);
                f.pc = 2;
                return Flow::Call(Frame::trash(p, vec![c]));
            }
            2 => {
                let c = f.s.pick.expect("set in step 1");
                f.s.n = (g.cost_of(c).coins >= 3) as i32;
                f.s.m = g.is(c, Types::TREASURE) as i32;
                f.s.i = 0;
                f.pc = 3;
            }
            3 => {
                if f.s.n == 1
                    && let Some(&target) = f.targets.get(f.s.i)
                {
                    f.s.i += 1;
                    if let Some(pile) = g.supply_pile(Card::Curse) {
                        return Flow::Call(Frame::gain(target, pile, Dest::Discard));
                    }
                    continue;
                }
                f.s.i = 0;
                f.pc = 4;
            }
            4 => {
                if f.s.m == 0 {
                    return Flow::done();
                }
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                let hand = g.hand(target).to_vec();
                if hand.len() <= 3 {
                    f.s.i += 1;
                    continue;
                }
                let n = hand.len() - 3;
                f.pc = 5;
                return f.ask_cards_for(target, hand, n, n, Purpose::Discard);
            }
            5 => {
                f.pc = 6;
                let target = f.targets[f.s.i];
                return Flow::Call(Frame::discard(target, input.cards()));
            }
            _ => {
                f.s.i += 1;
                f.pc = 4;
            }
        }
    }
}

fn chariot_race(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::draw(p, 1))
        }
        1 => {
            f.s.pick = input.card();
            if let Some(c) = f.s.pick {
                emit_revealed(g, p, &[c]);
            }
            f.pc = 2;
            Flow::Call(Frame::reveal(g.left_of(p), 1))
        }
        _ => {
            if let (Some(mine), Some(theirs)) = (f.s.pick, input.card())
                && g.cost_of(theirs).less_than(g.cost_of(mine))
            {
                g.add_coins(p, 1);
                add_vp(g, p, 1);
            }
            Flow::done()
        }
    }
}

fn charm(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "+1 Buy and +$2",
                    "The next time you gain a card this turn, you may also gain a differently named card with the same cost",
                ],
                1,
                1,
            )
        }
        _ => {
            let p = f.player;
            if input.opt() == Some(Opt::Mode(0)) {
                plus_buys(g, p, 1);
                g.add_coins(p, 2);
            } else {
                let data = Scratch {
                    n: g.turn.gained.len() as i32,
                    ..Scratch::default()
                };
                g.add_lasting(Card::Charm, p, f.this, vec![], Until::EndOfTurn, data);
            }
            Flow::done()
        }
    }
}

fn charm_gain(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(Occasion::Gain { card, .. }) = f.occ else {
        return Flow::done();
    };
    match f.pc {
        0 => {
            let cost = g.cost_of(card);
            let name = g.card(card);
            let piles: Vec<PileId> = g.supply_piles_where(|_, c| c == cost)
                .into_iter()
                .filter(|&pile| g.pile_top(pile).is_some_and(|t| g.card(t) != name))
                .collect();
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, true, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn capital_debt(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.zone(f.this()) != Zone::InPlay(p) {
                return Flow::done();
            }
            g.players[p.index()].debt += 6;
            let coins = if p == g.turn.player { g.turn.coins } else { 0 };
            let max = coins.min(g.player(p).debt);
            if max == 0 {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_number(0, max as i32, Purpose::Other)
        }
        _ => {
            let n = input.number().unwrap_or(0).max(0) as u32;
            g.players[p.index()].debt -= n;
            g.turn.coins -= n;
            Flow::done()
        }
    }
}

fn crown(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let types = match g.turn.phase {
                Phase::Start | Phase::Action => Types::ACTION,
                Phase::Buy => Types::TREASURE,
                _ => return Flow::done(),
            };
            if p != g.turn.player {
                return Flow::done();
            }
            let candidates = g.cards_in_hand_where(p, types);
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
        _ => Flow::done(),
    }
}

fn encampment(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let shown: Vec<CardId> = g
                .hand(p)
                .iter()
                .copied()
                .filter(|&c| matches!(g.card(c), Card::Gold | Card::Plunder))
                .collect();
            f.pc = 1;
            if shown.is_empty() {
                return Flow::Continue;
            }
            f.ask_cards(shown, 0, 1, Purpose::Reveal)
        }
        _ => {
            let chosen = input.cards();
            if !chosen.is_empty() {
                emit_revealed(g, p, &chosen);
                return Flow::done();
            }
            let this = f.this();
            if g.zone(this) == Zone::InPlay(p) {
                g.set_aside(this, p);
                let data = Scratch {
                    pick: Some(this),
                    ..Scratch::default()
                };
                g.add_lasting(Card::Encampment, p, Some(this), vec![], Until::EndOfTurn, data);
            }
            Flow::done()
        }
    }
}

fn engineer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let up_to_4 = |g: &Game| g.piles_costing_up_to(Cost::coins(4));
    match f.pc {
        0..=2 => {
            let piles = if f.pc == 0 { up_to_4(g) } else { vec![] };
            choose_and_gain(f, input, 0, piles, Dest::Discard, 3)
        }
        3 => {
            if g.zone(f.this()) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 4;
            f.ask_yes_no(Purpose::Trash)
        }
        4 => {
            if !input.is_yes() {
                return Flow::done();
            }
            f.pc = 5;
            Flow::Call(Frame::trash(p, vec![f.this()]))
        }
        5 if g.zone(f.this()) != Zone::Trash => Flow::done(),
        _ => {
            let piles = if f.pc == 5 { up_to_4(g) } else { vec![] };
            choose_and_gain(f, input, 5, piles, Dest::Discard, DONE)
        }
    }
}

fn farmers_market(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let Some(pile) = g.home_pile(Card::FarmersMarket) else {
                return Flow::done();
            };
            if g.pile(pile).vp_tokens >= 4 {
                take_pile_vp(g, p, pile);
                let this = f.this();
                if g.zone(this) == Zone::InPlay(p) {
                    f.pc = 1;
                    return Flow::Call(Frame::trash(p, vec![this]));
                }
                return Flow::done();
            }
            g.piles[pile.index()].vp_tokens += 1;
            let n = g.pile(pile).vp_tokens.max(0) as u32;
            g.add_coins(p, n);
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn gladiator(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                f.pc = 3;
                return Flow::Continue;
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Reveal)
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 3;
                return Flow::Continue;
            };
            emit_revealed(g, p, &[c]);
            let name = g.card(c);
            let left = g.left_of(p);
            let copies: Vec<CardId> = g
                .hand(left)
                .iter()
                .copied()
                .filter(|&x| g.card(x) == name)
                .collect();
            if copies.is_empty() {
                f.pc = 3;
                return Flow::Continue;
            }
            f.pc = 2;
            f.ask_cards_for(left, copies, 0, 1, Purpose::Reveal)
        }
        2 => {
            let chosen = input.cards();
            if !chosen.is_empty() {
                let left = g.left_of(p);
                emit_revealed(g, left, &chosen);
                return Flow::done();
            }
            f.pc = 3;
            Flow::Continue
        }
        3 => {
            g.add_coins(p, 1);
            f.pc = 4;
            if let Some(pile) = g.home_pile(Card::Gladiator)
                && let Some(top) = g.pile_top(pile)
                && g.card(top) == Card::Gladiator
            {
                return Flow::Call(Frame::trash(p, vec![top]));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn haunted_castle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    loop {
        match f.pc {
            0 => {
                f.pc = 1;
                f.s.i = 0;
                if let Some(pile) = g.supply_pile(Card::Gold) {
                    return Flow::Call(Frame::gain(p, pile, Dest::Discard));
                }
            }
            1 => {
                let Some(&other) = g.others(p).get(f.s.i) else {
                    return Flow::done();
                };
                let hand = g.hand(other).to_vec();
                if hand.len() < 5 {
                    f.s.i += 1;
                    continue;
                }
                f.pc = 2;
                return f.ask_cards_for(other, hand, 2, 2, Purpose::TopDeck);
            }
            _ => {
                let other = g.others(p)[f.s.i];
                let chosen = input.cards();
                for &c in chosen.iter().rev() {
                    if g.zone(c) == Zone::Hand(other) {
                        g.move_card(c, Zone::Deck(other), Pos::Top);
                        g.emit(Event::Moved {
                            player: other,
                            card: None,
                            to: Zone::Deck(other),
                        });
                    }
                }
                f.s.i += 1;
                f.pc = 1;
                return Flow::Continue;
            }
        }
    }
}

fn legionary(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    loop {
        match f.pc {
            0 => {
                let golds: Vec<CardId> = g
                    .hand(p)
                    .iter()
                    .copied()
                    .filter(|&c| g.card(c) == Card::Gold)
                    .collect();
                if golds.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                return f.ask_cards(golds, 0, 1, Purpose::Reveal);
            }
            1 => {
                let chosen = input.cards();
                if chosen.is_empty() {
                    return Flow::done();
                }
                emit_revealed(g, p, &chosen);
                f.s.i = 0;
                f.pc = 2;
                return Flow::Continue;
            }
            2 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    return Flow::done();
                };
                let hand = g.hand(target).to_vec();
                if hand.len() > 2 {
                    let n = hand.len() - 2;
                    f.pc = 3;
                    return f.ask_cards_for(target, hand, n, n, Purpose::Discard);
                }
                f.pc = 4;
            }
            3 => {
                f.pc = 4;
                return Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()));
            }
            4 => {
                f.pc = 5;
                return Flow::Call(Frame::draw(f.targets[f.s.i], 1));
            }
            _ => {
                f.s.i += 1;
                f.pc = 2;
                return Flow::Continue;
            }
        }
    }
}

fn opulent_castle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let victory = g.cards_in_hand_where(p, Types::VICTORY);
            if victory.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            let n = victory.len();
            f.ask_cards(victory, 0, n, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            emit_revealed(g, p, &chosen);
            f.s.n = chosen.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        _ => {
            g.add_coins(p, 2 * f.s.n as u32);
            Flow::done()
        }
    }
}

fn overlord(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let piles = g.supply_piles_where(|t, cost| {
                t.contains(Types::ACTION)
                    && !t.intersects(Types::COMMAND | Types::DURATION)
                    && cost.at_most(Cost::coins(5))
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

fn patrician(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 1))
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::Deck(p)
                && g.cost_of(c).coins >= 5
            {
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
    }
}

fn sacrifice(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
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
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 => {
            let c = f.s.pick.expect("set in step 1");
            let types = g.types(c);
            if types.intersects(Types::TREASURE) {
                g.add_coins(p, 2);
            }
            if types.intersects(Types::VICTORY) {
                add_vp(g, p, 2);
            }
            if types.intersects(Types::ACTION) {
                plus_actions(g, p, 2);
                f.pc = 3;
                return Flow::Call(Frame::draw(p, 2));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn small_castle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let this = f.this();
            let mut candidates = g.cards_in_hand_where(p, Types::CASTLE);
            if g.zone(this) == Zone::InPlay(p) {
                candidates.insert(0, this);
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
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 => {
            f.pc = 3;
            let trashed = f.s.pick.is_some_and(|c| g.zone(c) == Zone::Trash);
            if trashed
                && let Some(pile) = g.supply_pile(Card::Castles)
                && g.pile_top(pile).is_some_and(|t| g.is(t, Types::CASTLE))
            {
                return Flow::Call(Frame::gain(p, pile, Dest::Discard));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn sprawling_castle(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["Gain a Duchy", "Gain 3 Estates"], 1, 1)
        }
        1 => {
            f.s.i = 0;
            f.s.m = 0;
            if input.opt() == Some(Opt::Mode(0)) {
                f.pc = 3;
                return match g.supply_pile(Card::Duchy) {
                    Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                    None => Flow::done(),
                };
            }
            f.pc = 2;
            gain_copies(g, f, Input::Start, Card::Estate, 3, Dest::Discard, DONE)
        }
        2 => gain_copies(g, f, input, Card::Estate, 3, Dest::Discard, DONE),
        _ => Flow::done(),
    }
}

fn temple(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            add_vp(g, p, 1);
            f.s.cards.clear();
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            let names: Vec<Card> = f.s.cards.iter().map(|&c| g.card(c)).collect();
            let candidates: Vec<CardId> = g
                .hand(p)
                .iter()
                .copied()
                .filter(|&c| !names.contains(&g.card(c)))
                .collect();
            if f.s.cards.len() == 3 || candidates.is_empty() {
                f.pc = 3;
                return Flow::Continue;
            }
            let min = usize::from(f.s.cards.is_empty());
            f.pc = 2;
            f.ask_cards(candidates, min, 1, Purpose::Trash)
        }
        2 => match input.card() {
            Some(c) => {
                f.s.cards.push(c);
                f.pc = 1;
                Flow::Continue
            }
            None => {
                f.pc = 3;
                Flow::Continue
            }
        },
        3 => {
            f.pc = 4;
            let chosen = std::mem::take(&mut f.s.cards);
            Flow::Call(Frame::trash(p, chosen))
        }
        _ => {
            add_pile_vp(g, Card::Temple, 1);
            Flow::done()
        }
    }
}

fn villa_gained(g: &mut Game, f: &mut CardFrame) {
    let p = f.player;
    let c = f.this();
    let landed = g
        .gains
        .iter()
        .rev()
        .find(|st| st.card == c)
        .map(|st| dest_zone(st.to, st.player));
    if landed == Some(g.zone(c)) {
        g.put_in_hand(c, p);
    }
    if p == g.turn.player {
        g.turn.actions += 1;
        if g.turn.phase == Phase::Buy {
            g.turn.return_to_action = true;
        }
    }
}

fn wild_hunt(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "+3 Cards and add 1 VP to the Wild Hunt pile",
                    "Gain an Estate and take the VP from the pile",
                ],
                1,
                1,
            )
        }
        1 => {
            if input.opt() == Some(Opt::Mode(0)) {
                f.pc = 2;
                return Flow::Call(Frame::draw(p, 3));
            }
            f.pc = 3;
            match g.supply_pile(Card::Estate) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        2 => {
            add_pile_vp(g, Card::WildHunt, 1);
            Flow::done()
        }
        _ => {
            if !input.cards().is_empty()
                && let Some(pile) = g.home_pile(Card::WildHunt)
            {
                take_pile_vp(g, p, pile);
            }
            Flow::done()
        }
    }
}

// ---- Events ----

fn advance(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.cards_in_hand_where(p, Types::ACTION);
            if actions.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::Trash)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 if !f.s.pick.is_some_and(|c| g.zone(c) == Zone::Trash) => Flow::done(),
        _ => {
            let piles = if f.pc == 2 {
                g.supply_piles_where(|t, c| {
                    t.intersects(Types::ACTION) && c.at_most(Cost::coins(6))
                })
            } else {
                vec![]
            };
            choose_and_gain(f, input, 2, piles, Dest::Discard, DONE)
        }
    }
}

fn annex(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let discard = g.player(p).discard.clone();
            f.pc = 1;
            f.ask_cards(discard, 0, 5, Purpose::Other)
        }
        1 => {
            f.s.cards = input.cards();
            for &c in &f.s.cards {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
            }
            let deck: Vec<CardId> = g.player(p).deck.clone();
            for c in deck {
                g.move_card(c, Zone::Discard(p), Pos::Top);
            }
            f.pc = 2;
            Flow::Call(Frame::shuffle(p))
        }
        2 => {
            for c in std::mem::take(&mut f.s.cards) {
                if g.zone(c) == Zone::SetAside(p) {
                    g.move_card(c, Zone::Discard(p), Pos::Top);
                }
            }
            f.pc = 3;
            match g.supply_pile(Card::Duchy) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn banquet(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.s.i = 0;
            f.s.m = 0;
            f.pc = 1;
            gain_copies(g, f, input, Card::Copper, 2, Dest::Discard, 2)
        }
        1 => gain_copies(g, f, input, Card::Copper, 2, Dest::Discard, 2),
        _ => {
            let piles = if f.pc == 2 {
                g.supply_piles_where(|t, c| {
                    !t.intersects(Types::VICTORY) && c.at_most(Cost::coins(5))
                })
            } else {
                vec![]
            };
            choose_and_gain(f, input, 2, piles, Dest::Discard, DONE)
        }
    }
}

fn donate(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.end_lasting(g);
            let pl = g.player(p);
            let all: Vec<CardId> = pl.deck.iter().chain(&pl.discard).copied().collect();
            for c in all {
                g.put_in_hand(c, p);
            }
            let hand = g.hand(p).to_vec();
            let n = hand.len();
            f.pc = 1;
            f.ask_cards(hand, 0, n, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        2 => {
            // Only the hand is shuffled into the deck; cards that reached the discard pile
            // while trashing stay there.
            f.s.cards = g.player(p).discard.clone();
            for &c in &f.s.cards {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
            }
            let pl = g.player(p);
            let rest: Vec<CardId> = pl.hand.iter().chain(&pl.deck).copied().collect();
            for c in rest {
                g.move_card(c, Zone::Discard(p), Pos::Top);
            }
            f.pc = 3;
            Flow::Call(Frame::shuffle(p))
        }
        3 => {
            for c in std::mem::take(&mut f.s.cards) {
                if g.zone(c) == Zone::SetAside(p) {
                    g.move_card(c, Zone::Discard(p), Pos::Top);
                }
            }
            f.pc = 4;
            Flow::Call(Frame::draw(p, 5))
        }
        _ => Flow::done(),
    }
}

fn ritual(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            match g.supply_pile(Card::Curse) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        1 => {
            let hand = g.hand(p).to_vec();
            if input.cards().is_empty() || hand.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(hand, 1, 1, Purpose::Trash)
        }
        2 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 3;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        _ => {
            let c = f.s.pick.expect("set in step 2");
            let vp = g.cost_of(c).coins as i32;
            add_vp(g, p, vp);
            Flow::done()
        }
    }
}

fn salt_the_earth(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            add_vp(g, p, 1);
            let piles = g.supply_piles_where(|t, _| t.intersects(Types::VICTORY));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            match input.pile().and_then(|pile| g.pile_top(pile)) {
                Some(top) => Flow::Call(Frame::trash(p, vec![top])),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

// ---- Landmarks ----

fn arena(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.cards_in_hand_where(p, Types::ACTION);
            if actions.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(actions, 1, 1, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        _ => {
            take_landmark_vp(g, p, Card::Arena, 2);
            Flow::done()
        }
    }
}

/// Each player bids once, from the gainer's left, ending with the gainer. The bid is a
/// number from the high bid (meaning pass) to 40.
fn mountain_pass(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let order: Vec<PlayerId> = g.others(p).into_iter().chain([p]).collect();
    match f.pc {
        0 => {
            g.ext.empires.province_gained = true;
            f.s.i = 0;
            f.s.n = 0;
            f.s.player = None;
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            let Some(&bidder) = order.get(f.s.i) else {
                if let Some(winner) = f.s.player {
                    add_vp(g, winner, 8);
                    g.players[winner.index()].debt += f.s.n as u32;
                }
                return Flow::done();
            };
            f.pc = 2;
            Flow::Ask(Decision {
                player: bidder,
                kind: DecisionKind::Number {
                    min: f.s.n,
                    max: 40,
                },
                purpose: Purpose::Other,
                source: Some(Card::MountainPass),
            })
        }
        _ => {
            let bid = input.number().unwrap_or(0);
            if bid > f.s.n {
                f.s.n = bid;
                f.s.player = Some(order[f.s.i]);
            }
            f.s.i += 1;
            f.pc = 1;
            Flow::Continue
        }
    }
}
