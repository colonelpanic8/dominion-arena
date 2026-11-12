//! Nocturne: its Kingdom cards, Heirlooms, Spirits, Wish, Bat, Zombies, Boons, Hexes and
//! States.
//!
//! Boons and Hexes are drawn from `Game::boons` / `Game::hexes`. A Boon or Hex is
//! received as its `Hook::Main`; [`Frame::receive`] discards it afterwards unless the
//! receiver keeps it until Clean-up (`Player::boons_kept`). Effects that receive one
//! Boon or Hex several times (Hex attacks, Sacred Grove, Pixie) or not at all from the
//! deck (Druid) resolve it with [`receive_raw`] and discard it themselves.
//!
//! States are in `Player::states`. Deluded and Envious become an `Until::EndOfTurn`
//! lasting of that State when returned at the start of the Buy phase. Night-Durations
//! follow the Seaside pattern ([`START`]).

use super::{
    Bonus, CardFrame, Hook, SetRules, due_durations, duration_trigger, looking, put_back,
};
use crate::decision::{Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, Value};
use crate::game::{Phase, Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup, add_pile};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Set, Types};
use crate::Note;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Druid's three set-aside Boons.
    pub druid: Vec<Card>,
}

pub(crate) struct Rules;

/// A Duration's start-of-turn effect.
const START: u8 = 0;
/// "This is gained to your hand (instead of your discard pile)."
const TO_HAND: u8 = 1;
const GUARDIAN_SHIELD: u8 = 2;
/// "When you gain this" (Blessed Village, Cemetery, Cursed Village, Skulk).
const GAINED: u8 = 0;
/// Blessed Village's Boon saved for the start of the next turn.
const LATER_BOON: u8 = 1;
const CHANGELING_EXCHANGE: u8 = 0;
const TRACKER_TOPDECK: u8 = 0;
const HOUND_SET_ASIDE: u8 = 0;
const HOUND_RETURN: u8 = 1;
const MIRROR_TRASHED: u8 = 0;
/// A card Necromancer played is trashed again: it is face up again.
const NECROMANCER_FACE_UP: u8 = 0;
const RIVER_DRAW: u8 = 0;
/// Lost in the Woods at the start of a turn; Deluded and Envious at the start of the Buy
/// phase.
const STATE_TRIGGER: u8 = 0;

/// A helper's `next` that finishes the ability instead of continuing.
const DONE: u8 = u8::MAX;

pub(crate) const BOONS: [Card; 12] = [
    Card::TheEarthsGift,
    Card::TheFieldsGift,
    Card::TheFlamesGift,
    Card::TheForestsGift,
    Card::TheMoonsGift,
    Card::TheMountainsGift,
    Card::TheRiversGift,
    Card::TheSeasGift,
    Card::TheSkysGift,
    Card::TheSunsGift,
    Card::TheSwampsGift,
    Card::TheWindsGift,
];

pub(crate) const HEXES: [Card; 12] = [
    Card::BadOmens,
    Card::Delusion,
    Card::Envy,
    Card::Famine,
    Card::Fear,
    Card::Greed,
    Card::Haunting,
    Card::Locusts,
    Card::Misery,
    Card::Plague,
    Card::Poverty,
    Card::War,
];

/// Each Kingdom card with an Heirloom, and its Heirloom.
const HEIRLOOMS: [(Card, Card); 7] = [
    (Card::Cemetery, Card::HauntedMirror),
    (Card::Fool, Card::LuckyCoin),
    (Card::Pixie, Card::Goat),
    (Card::Pooka, Card::CursedGold),
    (Card::SecretCave, Card::MagicLamp),
    (Card::Shepherd, Card::Pasture),
    (Card::Tracker, Card::Pouch),
];

const SPIRITS: [Card; 3] = [Card::WillOWisp, Card::Imp, Card::Ghost];

fn gains_to_hand(card: Card) -> bool {
    matches!(
        card,
        Card::DenOfSin | Card::GhostTown | Card::Guardian | Card::NightWatchman
    )
}

/// Boons kept in front of their receiver until Clean-up.
fn kept_boon(card: Card) -> bool {
    matches!(
        card,
        Card::TheFieldsGift | Card::TheForestsGift | Card::TheRiversGift
    )
}

impl SetRules for Rules {
    fn notes(g: &Game, out: &mut Vec<Note>) {
        for &boon in &g.ext.nocturne.druid {
            out.push(Note::new(Card::Druid).card(boon));
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (c, Hook::Ability(START)) if c.is(Types::DURATION) => start(g, f, input),
            (c, Hook::Ability(TO_HAND)) if gains_to_hand(c) => {
                let this = f.this();
                if let Some(st) = g.gains.last_mut()
                    && st.card == this
                    && st.to == Dest::Discard
                {
                    st.to = Dest::Hand;
                }
                Flow::done()
            }
            (Card::Guardian, Hook::Ability(GUARDIAN_SHIELD)) => Flow::Done(Value::Bool(true)),
            (Card::BlessedVillage, Hook::Ability(GAINED)) => blessed_village_gained(g, f, input),
            (Card::BlessedVillage, Hook::Ability(LATER_BOON)) => match f.end_lasting(g) {
                Some(l) if f.pc == 0 => {
                    f.pc = 1;
                    match l.data.card {
                        Some(boon) => Flow::Call(Frame::receive(f.player, boon)),
                        None => Flow::done(),
                    }
                }
                _ => Flow::done(),
            },
            (Card::Cemetery, Hook::Ability(GAINED)) => trash_from_hand(g, f, input, 0, 4),
            (Card::CursedVillage, Hook::Ability(GAINED)) => receive_hex(g, f, 0, DONE),
            (Card::Skulk, Hook::Ability(GAINED)) => gain_named(g, f, 0, Card::Gold, Dest::Discard, DONE),
            (Card::Changeling, Hook::Ability(CHANGELING_EXCHANGE)) => {
                let c = f.this();
                if where_gained(g, c) {
                    exchange(g, f.player, c, Card::Changeling);
                }
                Flow::done()
            }
            (Card::Tracker, Hook::Ability(TRACKER_TOPDECK)) => {
                let c = f.this();
                if where_gained(g, c) && g.zone(c) != Zone::Deck(f.player) {
                    g.top_deck(c, f.player);
                }
                Flow::done()
            }
            (Card::FaithfulHound, Hook::Ability(HOUND_SET_ASIDE)) => {
                let (c, p) = (f.this(), f.player);
                if g.zone(c) == Zone::Discard(p) {
                    g.set_aside(c, p);
                    let data = Scratch {
                        pick: Some(c),
                        ..Scratch::default()
                    };
                    g.add_lasting(Card::FaithfulHound, p, Some(c), vec![], Until::Removed, data);
                }
                Flow::done()
            }
            (Card::FaithfulHound, Hook::Ability(HOUND_RETURN)) => {
                if let Some(c) = f.end_lasting(g).and_then(|l| l.data.pick)
                    && g.zone(c) == Zone::SetAside(f.player)
                {
                    g.put_in_hand(c, f.player);
                }
                Flow::done()
            }
            (Card::HauntedMirror, Hook::Ability(MIRROR_TRASHED)) => haunted_mirror(g, f, input),
            (Card::Necromancer, Hook::Ability(NECROMANCER_FACE_UP)) => {
                f.end_lasting(g);
                Flow::done()
            }
            (Card::TheRiversGift, Hook::Ability(RIVER_DRAW)) => {
                if f.pc == 0 {
                    f.end_lasting(g);
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, 1)));
                }
                Flow::done()
            }
            (Card::LostInTheWoods, Hook::Ability(STATE_TRIGGER)) => lost_in_the_woods(g, f, input),
            (Card::Deluded | Card::Envious, Hook::Ability(STATE_TRIGGER)) => {
                let p = f.player;
                g.players[p.index()].states.retain(|&s| s != f.card);
                g.add_lasting(f.card, p, None, vec![], Until::EndOfTurn, Scratch::default());
                Flow::done()
            }
            (_, Hook::Ability(_)) => return None,
            (c, _) if c.kind() == Kind::Boon => boon(g, f, input),
            (c, _) if c.kind() == Kind::Hex => hex(g, f, input),
            (Card::Bard | Card::Tracker, _) => {
                if f.card == Card::Tracker && f.pc == 0 {
                    g.add_lasting(
                        Card::Tracker,
                        f.player,
                        f.this,
                        vec![],
                        Until::EndOfTurn,
                        Scratch::default(),
                    );
                }
                receive_boon(g, f, 0, DONE)
            }
            (Card::Bat, _) => bat(g, f, input),
            (Card::BlessedVillage, _) => Flow::done(),
            (Card::Changeling, _) => changeling(g, f, input),
            (Card::Cobbler | Card::DenOfSin | Card::GhostTown | Card::Guardian, _) => {
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Conclave | Card::Imp, _) => play_unique_action(g, f, input),
            (Card::Crypt, _) => crypt(g, f, input),
            (Card::CursedGold, _) => gain_named(g, f, 0, Card::Curse, Dest::Discard, DONE),
            (Card::CursedVillage, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw_to(f.player, 6)));
                }
                Flow::done()
            }
            (Card::DevilsWorkshop, _) => devils_workshop(g, f, input),
            (Card::Druid, _) => druid(g, f, input),
            (Card::Exorcist, _) => exorcist(g, f, input),
            (Card::Fool, _) => fool(g, f, input),
            (Card::Ghost, _) => ghost(g, f, input),
            (Card::Goat, _) => trash_from_hand(g, f, input, 0, 1),
            (Card::Idol, _) => idol(g, f),
            (Card::Leprechaun, _) => leprechaun(g, f),
            (Card::LuckyCoin, _) => gain_named(g, f, 0, Card::Silver, Dest::Discard, DONE),
            (Card::MagicLamp, _) => magic_lamp(g, f),
            (Card::Monastery, _) => monastery(g, f, input),
            (Card::Necromancer, _) => necromancer(g, f, input),
            (Card::NightWatchman, _) => sift(g, f, input, 0, 5),
            (Card::Pixie, _) => pixie(g, f, input),
            (Card::Pooka, _) => pooka(g, f, input),
            (Card::Raider, _) => raider(g, f, input),
            (Card::SacredGrove, _) => sacred_grove(g, f, input),
            (Card::SecretCave, _) => secret_cave(g, f, input),
            (Card::Shepherd, _) => shepherd(g, f, input),
            (Card::Skulk, _) => hex_others(g, f, 0, DONE),
            (Card::Tormentor, _) => tormentor(g, f),
            (Card::TragicHero, _) => tragic_hero(g, f, input),
            (Card::Vampire, _) => vampire(g, f, input),
            (Card::Werewolf, _) => {
                let night = g.turn.phase == Phase::Night && g.turn.player == f.player;
                if night {
                    return Some(hex_others(g, f, 0, DONE));
                }
                if f.pc == 0 {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw(f.player, 3)));
                }
                Flow::done()
            }
            (Card::WillOWisp, _) => will_o_wisp(g, f, input),
            (Card::Wish, _) => wish(g, f, input),
            (Card::ZombieApprentice, _) => zombie_apprentice(g, f, input),
            (Card::ZombieMason, _) => zombie_mason(g, f, input),
            (Card::ZombieSpy, _) => zombie_spy(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in due_durations(g, occ) {
            if l.card.set() != Set::Nocturne {
                continue;
            }
            if l.card.is(Types::DURATION) {
                out.push(duration_trigger(l, START));
            } else if l.card == Card::BlessedVillage {
                out.push(duration_trigger(l, LATER_BOON));
            }
        }
        match *occ {
            Occasion::StartTurn { player } => {
                if has_state(g, player, Card::LostInTheWoods) && !g.hand(player).is_empty() {
                    out.push(
                        Trigger::new(player, Card::LostInTheWoods, STATE_TRIGGER).optional("discard a card to receive a Boon"),
                    );
                }
            }
            Occasion::StartBuyPhase { player } => {
                for s in [Card::Deluded, Card::Envious] {
                    if has_state(g, player, s) {
                        out.push(Trigger::new(player, s, STATE_TRIGGER));
                    }
                }
            }
            Occasion::Attack {
                attacker, target, ..
            } => {
                let guarded = g.lasting().iter().any(|l| {
                    l.card == Card::Guardian && l.owner == target && l.until == Until::Removed
                });
                if guarded && attacker != target {
                    out.push(Trigger::new(target, Card::Guardian, GUARDIAN_SHIELD).protects());
                }
            }
            Occasion::WouldGain { player, card } => {
                let kind = g.card(card);
                let to_discard = g
                    .gains
                    .last()
                    .is_some_and(|st| st.card == card && st.to == Dest::Discard);
                if gains_to_hand(kind) && to_discard {
                    out.push(Trigger::new(player, kind, TO_HAND).on(card));
                }
            }
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            Occasion::Discard { player, card } => {
                if g.card(card) == Card::FaithfulHound
                    && g.zone(card) == Zone::Discard(player)
                    && g.turn.phase != Phase::Cleanup
                {
                    out.push(
                        Trigger::new(player, Card::FaithfulHound, HOUND_SET_ASIDE)
                            .on(card)
                            .optional("set it aside, to put it into your hand at end of turn"),
                    );
                }
            }
            Occasion::EndTurn { .. } => {
                for l in g.lasting() {
                    match l.card {
                        Card::FaithfulHound => out.push(
                            Trigger::new(l.owner, Card::FaithfulHound, HOUND_RETURN)
                                .via_lasting(l.id),
                        ),
                        Card::TheRiversGift => out.push(
                            Trigger::new(l.owner, Card::TheRiversGift, RIVER_DRAW)
                                .via_lasting(l.id),
                        ),
                        _ => {}
                    }
                }
            }
            Occasion::Trash { player, card } => {
                if g.card(card) == Card::HauntedMirror {
                    let owner = g
                        .trashed_from(card)
                        .and_then(Zone::owner)
                        .unwrap_or(player);
                    if !g.cards_in_hand_where(owner, Types::ACTION).is_empty() {
                        out.push(
                            Trigger::new(owner, Card::HauntedMirror, MIRROR_TRASHED)
                                .on(card)
                                .optional("discard an Action card to gain a Ghost"),
                        );
                    }
                }
                for l in g.lasting() {
                    if l.card == Card::Necromancer && l.data.pick == Some(card) {
                        out.push(
                            Trigger::new(l.owner, Card::Necromancer, NECROMANCER_FACE_UP)
                                .via_lasting(l.id),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn modify_bonus(g: &Game, p: PlayerId, id: CardId, bonus: &mut Bonus) {
        if matches!(g.card(id), Card::Silver | Card::Gold) && returned_state(g, p, Card::Envious)
        {
            bonus.coins = 1;
        }
    }

    fn vp(g: &Game, p: PlayerId, card: Card) -> Option<i32> {
        (card == Card::Pasture).then(|| g.count_owned(p, Card::Estate) as i32)
    }

    fn bonus_vp(g: &Game, p: PlayerId) -> i32 {
        g.player(p)
            .states
            .iter()
            .map(|s| match s {
                Card::Miserable => -2,
                Card::TwiceMiserable => -4,
                _ => 0,
            })
            .sum()
    }

    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        if card.kind() != Kind::Card || !returned_state(g, p, Card::Deluded) {
            return true;
        }
        // The copy on offer is in a pile, and every copy there has the same types.
        let mut copies = g.piles().iter().flat_map(|pile| &pile.cards);
        let copy = copies.find(|&&c| g.card(c) == card);
        !copy.map_or(card.is(Types::ACTION), |&c| g.is(c, Types::ACTION))
    }

    fn ensure(g: &mut Game, s: &mut Setup) {
        let uses = |g: &Game, cards: &[Card]| setup::in_game(g, |c| cards.contains(&c));
        let fate = setup::in_game(g, |c| c.is(Types::FATE));
        let doom = setup::in_game(g, |c| c.is(Types::DOOM));
        let (boons, druid) = (&g.boons, &g.ext.nocturne.druid);
        if fate && boons.draw.is_empty() && boons.discard.is_empty() && druid.is_empty() {
            let mut boons = BOONS.to_vec();
            g.shuffle(&mut boons);
            g.boons.draw = boons;
        }
        if g.ext.nocturne.druid.is_empty() && uses(g, &[Card::Druid]) {
            // The top 3 Boons: a deck too small for that (Druid joined mid-game) gives all
            // of its Boons, and the reshuffled discard pile the rest.
            let mut druid = vec![];
            if g.boons.draw.len() < 3 {
                druid = std::mem::take(&mut g.boons.draw);
                let mut boons = std::mem::take(&mut g.boons.discard);
                g.shuffle(&mut boons);
                g.boons.draw = boons;
            }
            let n = 3 - druid.len();
            druid.extend(s.choose_n(g, Card::Druid, g.boons.draw.clone(), n..=n));
            g.boons.draw.retain(|b| !druid.contains(b));
            g.ext.nocturne.druid = druid;
        }
        if doom && g.hexes.draw.is_empty() && g.hexes.discard.is_empty() {
            let mut hexes = HEXES.to_vec();
            g.shuffle(&mut hexes);
            g.hexes.draw = hexes;
        }
        let extra: [(Card, usize, bool); 5] = [
            (Card::WillOWisp, 12, fate || uses(g, &[Card::Exorcist])),
            (
                Card::Imp,
                13,
                uses(g, &[Card::DevilsWorkshop, Card::Tormentor, Card::Exorcist]),
            ),
            (Card::Ghost, 6, uses(g, &[Card::Exorcist, Card::Cemetery])),
            (Card::Wish, 12, uses(g, &[Card::Leprechaun, Card::SecretCave])),
            (Card::Bat, 10, uses(g, &[Card::Vampire])),
        ];
        for (card, size, needed) in extra {
            if needed && g.pile_of(card).is_none() {
                add_pile(g, card, size, false);
            }
        }
        if uses(g, &[Card::Necromancer]) && !uses(g, &[Card::ZombieApprentice]) {
            for z in [Card::ZombieApprentice, Card::ZombieMason, Card::ZombieSpy] {
                g.create(z, Zone::Trash);
            }
        }
    }

    fn modify_starting_deck(g: &Game, _p: PlayerId, deck: &mut Vec<Card>) {
        for (kingdom, heirloom) in HEIRLOOMS {
            if setup::in_game(g, |c| c == kingdom)
                && let Some(i) = deck.iter().position(|&c| c == Card::Copper)
            {
                deck[i] = heirloom;
            }
        }
    }
}

fn gain_triggers(g: &Game, player: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let kind = g.card(card);
    if matches!(
        kind,
        Card::BlessedVillage | Card::Cemetery | Card::CursedVillage | Card::Skulk
    ) {
        out.push(Trigger::new(player, kind, GAINED).on(card));
    }
    let changelings = g
        .pile_of(Card::Changeling)
        .is_some_and(|pile| g.pile_top(pile).is_some());
    if changelings
        && kind != Card::Changeling
        && g.gain_cost(card).unwrap_or_else(|| g.cost_of(card)).coins >= 3
        && g.home_pile(g.printed(card)).is_some()
        && where_gained(g, card)
    {
        out.push(
            Trigger::new(player, Card::Changeling, CHANGELING_EXCHANGE)
                .on(card)
                .optional(format!("exchange {} for a Changeling", g.card(card))),
        );
    }
    if where_gained(g, card) && g.zone(card) != Zone::Deck(player) {
        for l in g.lasting() {
            if l.card == Card::Tracker && l.owner == player && l.until == Until::EndOfTurn {
                out.push(
                    Trigger::new(player, Card::Tracker, TRACKER_TOPDECK)
                        .on(card)
                        .via_lasting(l.id)
                        .optional(format!("put {} onto your deck", g.card(card))),
                );
            }
        }
    }
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

/// Whether `card`, being gained, is still where it was gained to.
fn where_gained(g: &Game, card: CardId) -> bool {
    g.still_where_gained(card)
}

fn has_state(g: &Game, p: PlayerId, state: Card) -> bool {
    g.player(p).states.contains(&state)
}

/// Whether `p` returned Deluded or Envious (`state`) at the start of this Buy phase.
fn returned_state(g: &Game, p: PlayerId, state: Card) -> bool {
    g.lasting()
        .iter()
        .any(|l| l.card == state && l.owner == p && l.until == Until::EndOfTurn)
}

fn take_state(g: &mut Game, p: PlayerId, state: Card) {
    g.players[p.index()].states.push(state);
    g.emit(Event::Received {
        player: p,
        card: state,
    });
}

fn plus_actions(g: &mut Game, p: PlayerId, n: u32) {
    g.add_actions(p, n);
}

fn plus_buys(g: &mut Game, p: PlayerId, n: u32) {
    g.add_buys(p, n);
}

fn plus_coins(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.add_coins(p, n);
    }
}

fn emit_hand(g: &mut Game, p: PlayerId) {
    let cards = g.hand(p).iter().map(|&c| g.card(c)).collect();
    g.emit(Event::RevealedHand { player: p, cards });
}

/// The Supply pile whose top card is `card`.
fn supply_pile_topped_by(g: &Game, card: Card) -> Option<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&id| g.pile(id).in_supply && g.pile_top(id).is_some_and(|t| g.card(t) == card))
}

/// Exchanges `c` (returned to its pile) for the top card of `card`'s pile, which goes to
/// `p`'s discard pile. Does nothing unless both can move.
fn exchange(g: &mut Game, p: PlayerId, c: CardId, card: Card) -> bool {
    let Some(new) = g.pile_of(card).and_then(|pile| g.pile_top(pile)) else {
        return false;
    };
    if !g.return_to_pile(c) {
        return false;
    }
    g.move_card(new, Zone::Discard(p), Pos::Top);
    true
}

/// Gains a `card` from its pile (in the Supply or not) at step `at`, then goes to `next`.
fn gain_named(g: &mut Game, f: &mut CardFrame, at: u8, card: Card, to: Dest, next: u8) -> Flow {
    if f.pc != at {
        return goto(f, next);
    }
    match g.pile_of(card) {
        Some(pile) => {
            f.pc = next;
            Flow::Call(Frame::gain(f.player, pile, to))
        }
        None => goto(f, next),
    }
}

/// "Gain a card costing up to `limit`" at steps `at` and `at + 1`, then `next`.
fn gain_up_to(
    g: &mut Game,
    f: &mut CardFrame,
    input: Input,
    at: u8,
    limit: Cost,
    to: Dest,
    next: u8,
) -> Flow {
    if f.pc != at && f.pc != at + 1 {
        return goto(f, next);
    }
    if f.pc == at {
        let piles = g.piles_costing_up_to(limit);
        if piles.is_empty() {
            return goto(f, next);
        }
        f.pc = at + 1;
        return f.ask_pile(piles, false, Purpose::Gain);
    }
    match input.pile() {
        Some(pile) => {
            f.pc = next;
            Flow::Call(Frame::gain(f.player, pile, to))
        }
        None => goto(f, next),
    }
}

/// Trash up to `max` cards from hand at steps `at`..`at + 1`.
fn trash_from_hand(g: &mut Game, f: &mut CardFrame, input: Input, at: u8, max: usize) -> Flow {
    if f.pc == at {
        let hand = g.hand(f.player).to_vec();
        if hand.is_empty() {
            return Flow::done();
        }
        f.pc = at + 1;
        return f.ask_cards(hand, 0, max, Purpose::Trash);
    }
    if f.pc == at + 1 {
        let chosen = input.cards();
        if chosen.is_empty() {
            return Flow::done();
        }
        f.pc = at + 2;
        return Flow::Call(Frame::trash(f.player, chosen));
    }
    Flow::done()
}

/// Discard exactly `n` cards from hand (or all of them) at steps `at`..`at + 1`.
fn discard_n(g: &mut Game, f: &mut CardFrame, input: Input, at: u8, n: usize) -> Flow {
    if f.pc == at {
        let hand = g.hand(f.player).to_vec();
        let n = n.min(hand.len());
        if n == 0 {
            return Flow::done();
        }
        f.pc = at + 1;
        return f.ask_cards(hand, n, n, Purpose::Discard);
    }
    if f.pc == at + 1 {
        f.pc = at + 2;
        return Flow::Call(Frame::discard(f.player, input.cards()));
    }
    Flow::done()
}

/// Look at the top `n` cards, discard any number of them and put the rest back in any
/// order, at steps `at`..`at + 4`.
fn sift(g: &mut Game, f: &mut CardFrame, input: Input, at: u8, n: usize) -> Flow {
    let p = f.player;
    match f.pc - at {
        0 => {
            f.pc = at + 1;
            Flow::Call(Frame::look(p, n))
        }
        1 => {
            f.s.cards = input.cards();
            if f.s.cards.is_empty() {
                return Flow::done();
            }
            f.pc = at + 2;
            f.ask_cards(f.s.cards.clone(), 0, f.s.cards.len(), Purpose::Discard)
        }
        2 => {
            let chosen = input.cards();
            f.s.cards.retain(|c| !chosen.contains(c));
            f.pc = at + 3;
            Flow::Call(Frame::discard(p, chosen))
        }
        3 => {
            f.s.cards = looking(g, p, &f.s.cards);
            f.pc = at + 4;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        _ => {
            put_back(g, p, &input.cards());
            Flow::done()
        }
    }
}

/// Resolves a Boon or Hex for `p` without drawing or discarding it.
pub(crate) fn receive_raw(g: &mut Game, p: PlayerId, card: Card) -> Frame {
    g.emit(Event::Received { player: p, card });
    Frame::Card(Box::new(CardFrame::new(card, Hook::Main, p, None)))
}

/// Puts a Boon resolved with [`receive_raw`] on the Boons discard pile, unless someone
/// keeps it until Clean-up.
fn release_boon(g: &mut Game, boon: Card) {
    if !g.players.iter().any(|pl| pl.boons_kept.contains(&boon)) {
        g.boons.discard.push(boon);
    }
}

/// Receive the next Boon at step `at`, then go to `next`.
fn receive_boon(g: &mut Game, f: &mut CardFrame, at: u8, next: u8) -> Flow {
    if f.pc != at {
        return goto(f, next);
    }
    match g.draw_from(false) {
        Some(boon) => {
            f.pc = next;
            Flow::Call(Frame::receive(f.player, boon))
        }
        None => goto(f, next),
    }
}

/// Receive the next Hex at step `at`, then go to `next`.
fn receive_hex(g: &mut Game, f: &mut CardFrame, at: u8, next: u8) -> Flow {
    if f.pc != at {
        return goto(f, next);
    }
    match g.draw_from(true) {
        Some(hex) => {
            f.pc = next;
            Flow::Call(Frame::receive(f.player, hex))
        }
        None => goto(f, next),
    }
}

/// "Each other player receives the next Hex" at steps `at` and `at + 1`, then `next`:
/// one Hex, received by each affected player in turn order. No Hex is turned over if
/// nobody is affected.
fn hex_others(g: &mut Game, f: &mut CardFrame, at: u8, next: u8) -> Flow {
    if f.pc == at {
        if f.targets.is_empty() {
            return goto(f, next);
        }
        let Some(hex) = g.draw_from(true) else {
            return goto(f, next);
        };
        f.s.card = Some(hex);
        f.s.i = 0;
        f.pc = at + 1;
    }
    let hex = f.s.card.expect("drawn at the first step");
    match f.targets.get(f.s.i).copied() {
        Some(target) => {
            f.s.i += 1;
            Flow::Call(receive_raw(g, target, hex))
        }
        None => {
            g.hexes.discard.push(hex);
            f.s.card = None;
            f.s.i = 0;
            goto(f, next)
        }
    }
}

fn boon_mask(boons: &[Card]) -> i32 {
    boons.iter().fold(0, |m, b| {
        let i = BOONS.iter().position(|x| x == b).expect("a Boon");
        m | 1 << i
    })
}

fn boons_in(mask: i32) -> Vec<Card> {
    (0..BOONS.len())
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| BOONS[i])
        .collect()
}

fn ask_boon(f: &CardFrame, boons: &[Card]) -> Flow {
    let options = boons.iter().map(|&b| Opt::Card(b)).collect();
    let labels = boons.iter().map(|b| b.to_string()).collect();
    f.ask_choose_for(f.player, options, labels, 1, 1, Purpose::Either)
}

// ---- Boons ----

fn boon(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        f.pc = 1;
        let plus = f.card.def().plus;
        plus_actions(g, p, plus.actions as u32);
        plus_buys(g, p, plus.buys as u32);
        plus_coins(g, p, plus.coins as u32);
        // In front of its first receiver, whoever's turn it is.
        if kept_boon(f.card) && !g.players.iter().any(|pl| pl.boons_kept.contains(&f.card)) {
            g.players[p.index()].boons_kept.push(f.card);
        }
        if f.card == Card::TheRiversGift {
            g.add_lasting(
                Card::TheRiversGift,
                p,
                None,
                vec![],
                Until::EndOfTurn,
                Scratch::default(),
            );
        }
        if plus.cards > 0 {
            return Flow::Call(Frame::draw(p, plus.cards as usize));
        }
    }
    match f.card {
        Card::TheEarthsGift => match f.pc {
            1 => {
                let treasures = g.cards_in_hand_where(p, Types::TREASURE);
                if treasures.is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                f.ask_cards(treasures, 0, 1, Purpose::Discard)
            }
            2 => {
                let chosen = input.cards();
                if chosen.is_empty() {
                    return Flow::done();
                }
                f.pc = 3;
                Flow::Call(Frame::discard(p, chosen))
            }
            3 | 4 => gain_up_to(g, f, input, 3, Cost::coins(4), Dest::Discard, DONE),
            _ => Flow::done(),
        },
        Card::TheFlamesGift => trash_from_hand(g, f, input, 1, 1),
        Card::TheMoonsGift => match f.pc {
            1 => {
                g.look_through_discard(p);
                let discard = g.player(p).discard.clone();
                if discard.is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                f.ask_cards(discard, 0, 1, Purpose::TopDeck)
            }
            _ => {
                for c in input.cards() {
                    if g.zone(c) == Zone::Discard(p) {
                        g.top_deck(c, p);
                    }
                }
                Flow::done()
            }
        },
        Card::TheMountainsGift => gain_named(g, f, 1, Card::Silver, Dest::Discard, DONE),
        Card::TheSkysGift => match f.pc {
            1 => {
                if g.hand(p).is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                f.ask_yes_no(Purpose::Discard, "Discard 3 cards to gain a Gold")
            }
            2 => {
                if !input.is_yes() {
                    return Flow::done();
                }
                let hand = g.hand(p).to_vec();
                let n = hand.len().min(3);
                f.pc = 3;
                f.ask_cards(hand, n, n, Purpose::Discard)
            }
            3 => {
                let chosen = input.cards();
                f.s.n = chosen.len() as i32;
                f.pc = 4;
                Flow::Call(Frame::discard(p, chosen))
            }
            4 if f.s.n == 3 => gain_named(g, f, 4, Card::Gold, Dest::Discard, DONE),
            _ => Flow::done(),
        },
        Card::TheSunsGift => sift(g, f, input, 1, 4),
        Card::TheSwampsGift => gain_named(g, f, 1, Card::WillOWisp, Dest::Discard, DONE),
        Card::TheWindsGift => discard_n(g, f, input, 1, 2),
        _ => Flow::done(),
    }
}

// ---- Hexes ----

fn hex(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.card {
        Card::BadOmens => {
            let deck = g.player(p).deck.clone();
            for c in deck {
                g.move_card(c, Zone::Discard(p), Pos::Top);
            }
            g.look_through_discard(p);
            let coppers: Vec<CardId> = g
                .player(p)
                .discard
                .iter()
                .rev()
                .copied()
                .filter(|&c| g.card(c) == Card::Copper)
                .take(2)
                .collect();
            if coppers.len() < 2 {
                for c in g.player(p).discard.clone() {
                    g.reveal(p, c);
                }
            }
            // Everyone knows what they are.
            for c in coppers {
                g.show(c);
                g.top_deck(c, p);
            }
            Flow::done()
        }
        Card::Delusion | Card::Envy => {
            if !has_state(g, p, Card::Deluded) && !has_state(g, p, Card::Envious) {
                let state = if f.card == Card::Delusion {
                    Card::Deluded
                } else {
                    Card::Envious
                };
                take_state(g, p, state);
            }
            Flow::done()
        }
        Card::Famine => match f.pc {
            0 => {
                f.pc = 1;
                Flow::Call(Frame::reveal(p, 3))
            }
            1 => {
                f.s.cards = input.cards();
                let actions: Vec<CardId> = f
                    .s
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, Types::ACTION))
                    .collect();
                f.pc = 2;
                Flow::Call(Frame::discard(p, actions))
            }
            2 => {
                put_back(g, p, &std::mem::take(&mut f.s.cards));
                f.pc = 3;
                Flow::Call(Frame::shuffle_deck(p))
            }
            _ => Flow::done(),
        },
        Card::Fear => match f.pc {
            0 => {
                if g.hand(p).len() < 5 {
                    return Flow::done();
                }
                let candidates = g.cards_in_hand_where(p, Types::ACTION | Types::TREASURE);
                if candidates.is_empty() {
                    emit_hand(g, p);
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_cards(candidates, 1, 1, Purpose::Discard)
            }
            1 => {
                f.pc = 2;
                Flow::Call(Frame::discard(p, input.cards()))
            }
            _ => Flow::done(),
        },
        Card::Greed => gain_named(g, f, 0, Card::Copper, Dest::DeckTop, DONE),
        Card::Haunting => match f.pc {
            0 => {
                let hand = g.hand(p).to_vec();
                if hand.len() < 4 {
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_cards(hand, 1, 1, Purpose::TopDeck)
            }
            _ => {
                for c in input.cards() {
                    g.top_deck(c, p);
                }
                Flow::done()
            }
        },
        Card::Locusts => locusts(g, f, input),
        Card::Misery => {
            let states = &mut g.players[p.index()].states;
            if let Some(i) = states.iter().position(|&s| s == Card::Miserable) {
                states[i] = Card::TwiceMiserable;
                g.emit(Event::Received {
                    player: p,
                    card: Card::TwiceMiserable,
                });
            } else if !states.contains(&Card::TwiceMiserable) {
                take_state(g, p, Card::Miserable);
            }
            Flow::done()
        }
        Card::Plague => gain_named(g, f, 0, Card::Curse, Dest::Hand, DONE),
        Card::Poverty => {
            let n = g.hand(p).len().saturating_sub(3);
            discard_n(g, f, input, 0, n)
        }
        Card::War => war(g, f, input),
        _ => Flow::done(),
    }
}

fn locusts(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.s.cost = g.cost_of(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 => {
            let c = f.s.pick.expect("set in step 1");
            if matches!(g.card(c), Card::Copper | Card::Estate) {
                return gain_named(g, f, 2, Card::Curse, Dest::Discard, DONE);
            }
            let (types, limit) = (g.types(c), f.s.cost);
            let piles =
                g.supply_piles_where(|t, cost| t.intersects(types) && cost.less_than(limit));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn war(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 3;
                return Flow::Continue;
            };
            let cost = g.cost_of(c);
            if cost == Cost::coins(3) || cost == Cost::coins(4) {
                f.pc = 2;
                return Flow::Call(Frame::trash(p, vec![c]));
            }
            f.s.cards.push(c);
            f.pc = 0;
            Flow::Continue
        }
        2 => {
            f.pc = 3;
            Flow::Continue
        }
        3 => {
            f.pc = 4;
            let rest = looking(g, p, &std::mem::take(&mut f.s.cards));
            Flow::Call(Frame::discard(p, rest))
        }
        _ => Flow::done(),
    }
}

// ---- States ----

fn lost_in_the_woods(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Discard)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        2 => receive_boon(g, f, 2, DONE),
        _ => Flow::done(),
    }
}

// ---- Start of turn ----

fn start(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.card == Card::Crypt {
        return crypt_start(g, f, input);
    }
    if f.pc == 0 {
        let Some(l) = f.end_lasting(g) else {
            return Flow::done();
        };
        f.s = l.data;
        f.pc = 1;
    }
    let p = f.player;
    match f.card {
        Card::Cobbler => gain_up_to(g, f, input, 1, Cost::coins(4), Dest::Hand, DONE),
        Card::DenOfSin | Card::GhostTown => {
            if f.pc == 1 {
                f.pc = 2;
                let n = if f.card == Card::DenOfSin { 2 } else { 1 };
                if f.card == Card::GhostTown {
                    plus_actions(g, p, 1);
                }
                return Flow::Call(Frame::draw(p, n));
            }
            Flow::done()
        }
        Card::Guardian => {
            g.add_coins(p, 1);
            Flow::done()
        }
        Card::Raider | Card::SecretCave => {
            g.add_coins(p, 3);
            Flow::done()
        }
        Card::Ghost => {
            let Some(c) = f.s.pick else {
                return Flow::done();
            };
            match f.pc {
                1 if g.zone(c) == Zone::SetAside(p) => {
                    f.pc = 2;
                    Flow::Call(Frame::play(p, c, true, f.chain_for_child()))
                }
                2 => {
                    f.pc = 3;
                    Flow::Call(Frame::play(p, c, false, f.chain_for_child()))
                }
                _ => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn crypt_start(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let Some(id) = f.lasting else {
        return Flow::done();
    };
    let held = |g: &Game| -> Vec<CardId> {
        g.lasting()
            .iter()
            .find(|l| l.id == id)
            .map(|l| {
                l.data
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| g.zone(c) == Zone::SetAside(p))
                    .collect()
            })
            .unwrap_or_default()
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
                .question("put into your hand")
        }
        _ => {
            for c in input.cards() {
                if g.zone(c) == Zone::SetAside(p) {
                    g.put_in_hand(c, p);
                }
            }
            let left = held(g);
            if left.is_empty() {
                f.end_lasting(g);
            } else if let Some(l) = g.lasting_mut(id) {
                l.data.cards = left;
            }
            Flow::done()
        }
    }
}

// ---- Kingdom cards ----

fn bat(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 0, 2, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(f.player, chosen))
        }
        _ => {
            let this = f.this();
            if g.zone(this) == Zone::InPlay(f.player) {
                exchange(g, f.player, this, Card::Vampire);
            }
            Flow::done()
        }
    }
}

fn blessed_village_gained(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let Some(boon) = g.draw_from(false) else {
                return Flow::done();
            };
            g.emit(Event::Revealed {
                player: p,
                card: boon,
            });
            f.s.card = Some(boon);
            f.pc = 1;
f.ask_either(&[
                format!("Receive {boon} now"),
                format!("Receive {boon} at the start of your next turn"),
            ])
        }
        1 => {
            let boon = f.s.card.expect("set in step 0");
            if input.opt() == Some(Opt::Mode(1)) {
                let data = Scratch {
                    card: Some(boon),
                    ..Scratch::default()
                };
                g.add_lasting(Card::BlessedVillage, p, None, vec![], Until::Removed, data);
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::receive(p, boon))
        }
        _ => Flow::done(),
    }
}

fn changeling(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            let this = f.this();
            if g.zone(this) == Zone::InPlay(p) {
                return Flow::Call(Frame::trash(p, vec![this]));
            }
            Flow::Continue
        }
        1 => {
            let candidates: Vec<CardId> = g
                .in_play(p)
                .iter()
                .copied()
                .filter(|&c| supply_pile_topped_by(g, g.card(c)).is_some())
                .collect();
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(candidates, 1, 1, Purpose::Gain)
        }
        2 => {
            f.pc = 3;
            match input
                .card()
                .and_then(|c| supply_pile_topped_by(g, g.card(c)))
            {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

/// Conclave and Imp: play an Action from hand you have no copy of in play.
fn play_unique_action(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let in_play: Vec<Card> = g.in_play(p).iter().map(|&c| g.card(c)).collect();
            let candidates: Vec<CardId> = g
                .playable(p, Types::ACTION)
                .into_iter()
                .filter(|&c| !in_play.contains(&g.card(c)))
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
            f.pc = 2;
            Flow::Call(Frame::play(p, c, true, vec![]))
        }
        _ => {
            if f.card == Card::Conclave {
                plus_actions(g, p, 1);
            }
            Flow::done()
        }
    }
}

fn crypt(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let candidates: Vec<CardId> = g
                .in_play(p)
                .iter()
                .copied()
                .filter(|&c| g.is(c, Types::TREASURE) && !g.is(c, Types::DURATION))
                .collect();
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            let n = candidates.len();
            f.ask_cards(candidates, 0, n, Purpose::SetAside)
        }
        _ => {
            let chosen: Vec<CardId> = input
                .cards()
                .into_iter()
                .filter(|&c| g.zone(c) == Zone::InPlay(p))
                .collect();
            if chosen.is_empty() {
                return Flow::done();
            }
            for &c in &chosen {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
            }
            let data = Scratch {
                cards: chosen,
                ..Scratch::default()
            };
            f.add_duration(g, data);
            Flow::done()
        }
    }
}

fn devils_workshop(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let gained = if f.player == g.turn.player {
                g.turn.gained.len()
            } else {
                0
            };
            match gained {
                0 => gain_named(g, f, 0, Card::Gold, Dest::Discard, DONE),
                1 => goto(f, 1),
                _ => gain_named(g, f, 0, Card::Imp, Dest::Discard, DONE),
            }
        }
        1 | 2 => gain_up_to(g, f, input, 1, Cost::coins(4), Dest::Discard, DONE),
        _ => Flow::done(),
    }
}

fn druid(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let boons = g.ext.nocturne.druid.clone();
            if boons.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            ask_boon(f, &boons)
        }
        1 => {
            let Some(Opt::Card(boon)) = input.opt() else {
                return Flow::done();
            };
            f.s.card = Some(boon);
            f.pc = 2;
            Flow::Call(receive_raw(g, p, boon))
        }
        _ => {
            // Druid's Boons stay with Druid, even those normally kept until Clean-up.
            if let Some(boon) = f.s.card {
                g.players[p.index()].boons_kept.retain(|&b| b != boon);
            }
            Flow::done()
        }
    }
}

fn exorcist(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            f.s.cost = g.cost_of(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 => {
            let limit = f.s.cost;
            let piles: Vec<PileId> = SPIRITS
                .iter()
                .filter_map(|&s| g.pile_of(s))
                .filter(|&pile| {
                    g.pile_top(pile)
                        .is_some_and(|top| g.cost_of(top).less_than(limit))
                })
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
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn fool(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if has_state(g, p, Card::LostInTheWoods) {
                return Flow::done();
            }
            for pl in g.players.iter_mut() {
                pl.states.retain(|&s| s != Card::LostInTheWoods);
            }
            take_state(g, p, Card::LostInTheWoods);
            let boons: Vec<Card> = (0..3).filter_map(|_| g.draw_from(false)).collect();
            f.s.m = boon_mask(&boons);
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            let boons = boons_in(f.s.m);
            match boons.len() {
                0 => Flow::done(),
                1 => {
                    f.s.card = Some(boons[0]);
                    f.pc = 3;
                    Flow::Continue
                }
                _ => {
                    f.pc = 2;
                    ask_boon(f, &boons)
                }
            }
        }
        2 => {
            let Some(Opt::Card(boon)) = input.opt() else {
                return Flow::done();
            };
            f.s.card = Some(boon);
            f.pc = 3;
            Flow::Continue
        }
        _ => {
            let boon = f.s.card.take().expect("chosen");
            f.s.m &= !boon_mask(&[boon]);
            f.pc = 1;
            Flow::Call(Frame::receive(p, boon))
        }
    }
}

fn ghost(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                f.pc = 2;
                return Flow::Continue;
            };
            if g.is(c, Types::ACTION) {
                g.set_aside_revealed(c, p);
                g.emit(Event::SetAside {
                    player: p,
                    card: g.card(c),
                });
                f.s.pick = Some(c);
                f.pc = 2;
            } else {
                f.s.cards.push(c);
                f.pc = 0;
            }
            Flow::Continue
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::discard(p, std::mem::take(&mut f.s.cards)))
        }
        _ => {
            if let Some(c) = f.s.pick {
                let data = Scratch {
                    pick: Some(c),
                    ..Scratch::default()
                };
                f.add_duration(g, data);
            }
            Flow::done()
        }
    }
}

fn haunted_mirror(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
        2 => gain_named(g, f, 2, Card::Ghost, Dest::Discard, DONE),
        _ => Flow::done(),
    }
}

fn idol(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        let idols = g
            .in_play(p)
            .iter()
            .filter(|&&c| g.card(c) == Card::Idol)
            .count();
        if idols % 2 == 1 {
            return receive_boon(g, f, 0, DONE);
        }
        f.pc = 1;
    }
    if f.pc != 1 {
        return Flow::done();
    }
    let Some(&target) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    f.s.i += 1;
    match g.supply_pile(Card::Curse) {
        Some(pile) => Flow::Call(Frame::gain(target, pile, Dest::Discard)),
        None => Flow::done(),
    }
}

fn leprechaun(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            match g.supply_pile(Card::Gold) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        1 => {
            if g.in_play(p).len() == 7 {
                f.pc = 3;
                if let Some(pile) = g.pile_of(Card::Wish) {
                    return Flow::Call(Frame::gain(p, pile, Dest::Discard));
                }
                return Flow::done();
            }
            f.pc = 2;
            Flow::Continue
        }
        2 => receive_hex(g, f, 2, DONE),
        _ => Flow::done(),
    }
}

fn magic_lamp(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let names: Vec<Card> = g.in_play(p).iter().map(|&c| g.card(c)).collect();
            let singles = names
                .iter()
                .filter(|&&n| names.iter().filter(|&&m| m == n).count() == 1)
                .count();
            let this = f.this();
            if singles < 6 || g.zone(this) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::trash(p, vec![this]))
        }
        _ => {
            if f.s.i >= 3 {
                return Flow::done();
            }
            f.s.i += 1;
            match g.pile_of(Card::Wish) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
    }
}

fn monastery(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.s.n = if p == g.turn.player {
                g.turn.gained.len() as i32
            } else {
                0
            };
            f.pc = 1;
            Flow::Continue
        }
        1 => {
            if f.s.n <= 0 {
                return Flow::done();
            }
            let mut candidates = g.hand(p).to_vec();
            candidates.extend(
                g.in_play(p)
                    .iter()
                    .copied()
                    .filter(|&c| g.card(c) == Card::Copper),
            );
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(candidates, 0, 1, Purpose::Trash)
        }
        _ => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.s.n -= 1;
            f.pc = 1;
            Flow::Call(Frame::trash(p, chosen))
        }
    }
}

fn necromancer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let face_down: Vec<CardId> = g
                .lasting()
                .iter()
                .filter(|l| l.card == Card::Necromancer)
                .filter_map(|l| l.data.pick)
                .collect();
            let candidates: Vec<CardId> = g
                .trash()
                .iter()
                .copied()
                .filter(|&c| {
                    g.is(c, Types::ACTION)
                        && !g.is(c, Types::DURATION)
                        && !face_down.contains(&c)
                })
                .collect();
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(candidates, 1, 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let data = Scratch {
                pick: Some(c),
                ..Scratch::default()
            };
            g.add_lasting(Card::Necromancer, p, f.this, vec![], Until::EndOfTurn, data);
            f.pc = 2;
            Flow::Call(Frame::play(p, c, false, f.chain_for_child()))
        }
        _ => Flow::done(),
    }
}

fn pixie(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let Some(boon) = g.draw_from(false) else {
                return Flow::done();
            };
            g.emit(Event::Revealed {
                player: p,
                card: boon,
            });
            g.boons.discard.push(boon);
            f.s.card = Some(boon);
            if g.zone(f.this()) != Zone::InPlay(p) {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Trash, format!("Trash {} to receive {boon} twice", g.card(f.this())))
        }
        1 => {
            if !input.is_yes() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![f.this()]))
        }
        2 | 3 => {
            let boon = f.s.card.expect("set in step 0");
            if f.pc == 2 {
                let deck = &mut g.boons;
                if let Some(i) = deck.discard.iter().rposition(|&b| b == boon) {
                    deck.discard.remove(i);
                } else if let Some(i) = deck.draw.iter().position(|&b| b == boon) {
                    deck.draw.remove(i);
                }
            }
            f.pc += 1;
            Flow::Call(receive_raw(g, p, boon))
        }
        _ => {
            release_boon(g, f.s.card.expect("set in step 0"));
            Flow::done()
        }
    }
}

fn pooka(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let treasures: Vec<CardId> = g
                .cards_in_hand_where(p, Types::TREASURE)
                .into_iter()
                .filter(|&c| g.card(c) != Card::CursedGold)
                .collect();
            if treasures.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(treasures, 0, 1, Purpose::Trash)
        }
        1 => {
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(p, chosen))
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::draw(p, 4))
        }
        _ => Flow::done(),
    }
}

fn raider(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    loop {
        match f.pc {
            0 => {
                let Some(&target) = f.targets.get(f.s.i) else {
                    f.add_duration(g, Scratch::default());
                    return Flow::done();
                };
                if g.hand(target).len() < 5 {
                    f.s.i += 1;
                    continue;
                }
                let names: Vec<Card> = g.in_play(p).iter().map(|&c| g.card(c)).collect();
                let copies: Vec<CardId> = g
                    .hand(target)
                    .iter()
                    .copied()
                    .filter(|&c| names.contains(&g.card(c)))
                    .collect();
                if copies.is_empty() {
                    emit_hand(g, target);
                    f.s.i += 1;
                    continue;
                }
                f.pc = 1;
                return f.ask_cards_for(target, copies, 1, 1, Purpose::Discard);
            }
            1 => {
                f.pc = 2;
                return Flow::Call(Frame::discard(f.targets[f.s.i], input.cards()));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
            }
        }
    }
}

fn sacred_grove(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let Some(boon) = g.draw_from(false) else {
                return Flow::done();
            };
            f.s.card = Some(boon);
            f.pc = 1;
            Flow::Call(receive_raw(g, p, boon))
        }
        1 => {
            let boon = f.s.card.expect("set in step 0");
            let gives_coin = matches!(boon, Card::TheFieldsGift | Card::TheForestsGift);
            let others = g.others_in_turn_order(p);
            match others.get(f.s.i) {
                Some(&other) if !gives_coin => {
                    f.pc = 2;
                    f.ask_yes_no_for(other, Purpose::Other, format!("Receive {boon}"))
                }
                _ => {
                    release_boon(g, boon);
                    Flow::done()
                }
            }
        }
        _ => {
            let other = g.others_in_turn_order(p)[f.s.i];
            f.s.i += 1;
            f.pc = 1;
            if input.is_yes() {
                let boon = f.s.card.expect("set in step 0");
                return Flow::Call(receive_raw(g, other, boon));
            }
            Flow::Continue
        }
    }
}

fn secret_cave(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.hand(p).is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Discard, "Discard 3 cards for +$3 at the start of your next turn")
        }
        1 => {
            if !input.is_yes() {
                return Flow::done();
            }
            let hand = g.hand(p).to_vec();
            let n = hand.len().min(3);
            f.pc = 2;
            f.ask_cards(hand, n, n, Purpose::Discard)
        }
        2 => {
            let chosen = input.cards();
            f.s.n = chosen.len() as i32;
            f.pc = 3;
            Flow::Call(Frame::discard(p, chosen))
        }
        _ => {
            if f.s.n == 3 {
                f.add_duration(g, Scratch::default());
            }
            Flow::done()
        }
    }
}

fn shepherd(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            for &c in &chosen {
                g.reveal(p, c);
            }
            f.s.n = chosen.len() as i32;
            f.pc = 2;
            Flow::Call(Frame::discard(p, chosen))
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::draw(p, 2 * f.s.n as usize))
        }
        _ => Flow::done(),
    }
}

fn tormentor(g: &mut Game, f: &mut CardFrame) -> Flow {
    match f.pc {
        0 => {
            let others = g.in_play(f.player).iter().any(|&c| Some(c) != f.this);
            if !others {
                return gain_named(g, f, 0, Card::Imp, Dest::Discard, DONE);
            }
            goto(f, 1)
        }
        1 | 2 => hex_others(g, f, 1, DONE),
        _ => Flow::done(),
    }
}

fn tragic_hero(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            if g.hand(p).len() < 8 {
                return Flow::done();
            }
            f.pc = 1;
            let this = f.this();
            if g.zone(this) == Zone::InPlay(p) {
                return Flow::Call(Frame::trash(p, vec![this]));
            }
            Flow::Continue
        }
        1 => {
            let piles = g.supply_piles_where(|t, _| t.contains(Types::TREASURE));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        2 => {
            f.pc = 3;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn vampire(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 | 1 => hex_others(g, f, 0, 2),
        2 => {
            let piles: Vec<PileId> = g
                .piles_costing_up_to(Cost::coins(5))
                .into_iter()
                .filter(|&pile| g.pile_top(pile).is_some_and(|t| g.card(t) != Card::Vampire))
                .collect();
            if piles.is_empty() {
                f.pc = 4;
                return Flow::Continue;
            }
            f.pc = 3;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        _ => {
            let this = f.this();
            if g.zone(this) == Zone::InPlay(p) {
                exchange(g, p, this, Card::Bat);
            }
            Flow::done()
        }
    }
}

fn will_o_wisp(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 1))
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::Looking(p)
                && g.cost_of(c).at_most(Cost::coins(2))
            {
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
    }
}

fn wish(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        let this = f.this();
        if g.zone(this) != Zone::InPlay(p) || !g.return_to_pile(this) {
            return Flow::done();
        }
        f.pc = 1;
    }
    gain_up_to(g, f, input, 1, Cost::coins(6), Dest::Hand, DONE)
}

fn zombie_apprentice(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            let chosen = input.cards();
            if chosen.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::trash(p, chosen))
        }
        2 => {
            plus_actions(g, p, 1);
            f.pc = 3;
            Flow::Call(Frame::draw(p, 3))
        }
        _ => Flow::done(),
    }
}

fn zombie_mason(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.cost = g.cost_of(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 => {
            let piles = g.piles_costing_up_to(f.s.cost.plus_coins(1));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_pile(piles, true, Purpose::Gain)
        }
        3 => {
            f.pc = 4;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn zombie_spy(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::look(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            f.ask_yes_or(Purpose::Discard, format!("Discard {}", g.card(c)), "Put it back").about(vec![c])
        }
        2 => {
            let c = f.s.pick.expect("set in step 1");
            if input.is_yes() && g.zone(c) == Zone::Looking(p) {
                f.pc = 3;
                return Flow::Call(Frame::discard(p, vec![c]));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}
