//! Menagerie: its Kingdom cards, the Horse pile, Exile, Events and Ways.
//!
//! Exile is each player's `Mat::Exile`; the cards on it are still theirs. When a player
//! gains a card, they may discard every copy of it from their Exile mat (the optional
//! [`EXILE_DISCARD`] trigger). Cards Exiled by Invest are listed in [`State::invested`]
//! while they stay on the mat.
//!
//! Ways run as the Way's `Hook::Main` with `f.this` = the card played; the engine applies
//! a Way's own vanilla bonuses. Way of the Chameleon plays the card's own ability as a
//! child frame and marks it in [`State::chameleon`]; [`Rules::after_step`] then exchanges
//! the +$ and +Cards that frame (and its "this turn" effects) give.
//!
//! Effects that play a card at the start of the next turn (Delay, Reap, Way of the Turtle)
//! and the Durations keep their card in `data.pick` of a lasting resolved by `START`.

use super::{CardFrame, Hook, SetRules, Step, due_durations, duration_trigger};
use crate::decision::{Opt, Purpose};
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input, PAY_OTHERWISE, Value};
use crate::game::{ExtraLimit, ExtraTurn, GainState, Mat, Phase, Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, PileId, PlayerId, Types};
use crate::Note;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// The card Way of the Mouse set aside (in its own non-Supply pile).
    pub mouse: Option<CardId>,
    /// Cards Exiled by Invest.
    pub invested: Vec<CardId>,
    /// Gains in progress (the card and its `GainState::since`, which no other gain of
    /// that card shares) during which the gainer discarded copies of the card from Exile.
    pub exile_discarded: Vec<(CardId, usize)>,
    /// Cards being played using Way of the Chameleon, innermost last.
    pub chameleon: Vec<CardId>,
    /// "This turn" effects made by plays using Way of the Chameleon.
    pub chameleon_lastings: Vec<u32>,
    /// Players who have bought Seize the Day.
    pub seized: Vec<PlayerId>,
    /// The turn Desperation was last bought in: its player and their turn number.
    pub desperation: Option<(PlayerId, u32)>,
}

pub(crate) struct Rules;

/// The Exile rule ("when you gain a card, you may discard its copies from Exile"), as an
/// ability of the gained card, whatever its set: `cards::resume` sends it to this module.
pub(crate) const EXILE_DISCARD: u8 = 253;

/// Durations and "at the start of your next turn" effects.
const START: u8 = 0;
/// Reactions and when-gain abilities of cards.
const REACT: u8 = 1;
const ON_GAIN: u8 = 2;
const GATEKEEPER_EXILE: u8 = 1;
const INVEST_DRAW: u8 = 0;
const KILN_COPY: u8 = 1;
const LIVERY_HORSE: u8 = 1;
const FROG_TOPDECK: u8 = 0;
const SEAL_TOPDECK: u8 = 0;
const SQUIRREL_DRAW: u8 = 0;

/// Cards and Events that gain Horses.
const HORSE_USERS: &[Card] = &[
    Card::Sleigh,
    Card::Supplies,
    Card::Scrap,
    Card::Cavalry,
    Card::Groom,
    Card::Hostelry,
    Card::Livery,
    Card::Paddock,
    Card::Ride,
    Card::Bargain,
    Card::Demand,
    Card::Stampede,
];

/// Lasting effects of the set that a `START` trigger resolves.
const STARTS: &[Card] = &[
    Card::Barge,
    Card::VillageGreen,
    Card::Gatekeeper,
    Card::Mastermind,
    Card::Delay,
    Card::Reap,
    Card::WayOfTheTurtle,
];

impl SetRules for Rules {
    fn notes(g: &Game, out: &mut Vec<Note>) {
        let s = &g.ext.menagerie;
        if let Some(c) = s.mouse {
            out.push(Note::new(Card::WayOfTheMouse).card(g.card(c)));
        }
        for &c in &s.invested {
            if let Zone::Mat(p, Mat::Exile) = g.zone(c) {
                out.push(Note::new(Card::Invest).player(p).card(g.card(c)));
            }
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (_, Hook::Ability(EXILE_DISCARD)) => exile_discard(g, f),
            (c, Hook::Ability(START)) if STARTS.contains(&c) => start(g, f, input),
            (Card::BlackCat | Card::Sheepdog | Card::Falconer, Hook::Ability(REACT)) => {
                play_from(g, f, Zone::Hand(f.player))
            }
            (Card::VillageGreen, Hook::Ability(REACT)) => play_from(g, f, Zone::Discard(f.player)),
            (Card::Sleigh, Hook::Ability(REACT)) => sleigh_react(g, f, input),
            (Card::CamelTrain, Hook::Ability(ON_GAIN)) => {
                exile_from_supply(g, f.player, Card::Gold);
                Flow::done()
            }
            (Card::Cavalry, Hook::Ability(ON_GAIN)) => cavalry_gain(g, f),
            (Card::Hostelry, Hook::Ability(ON_GAIN)) => hostelry_gain(g, f, input),
            (Card::Gatekeeper, Hook::Ability(GATEKEEPER_EXILE)) => {
                if let Some(Occasion::Gain { player, card, .. }) = f.occ
                    && still_where_gained(g, card)
                {
                    exile(g, player, card);
                }
                Flow::done()
            }
            (Card::Invest, Hook::Ability(INVEST_DRAW)) => draw_then_done(f, 0, 2),
            (Card::Kiln, Hook::Ability(KILN_COPY)) => kiln_copy(g, f, input),
            (Card::Livery, Hook::Ability(LIVERY_HORSE)) => gain_horses(g, f, 1, Dest::Discard),
            (Card::WayOfTheFrog, Hook::Ability(FROG_TOPDECK)) => {
                if let Some(l) = f.end_lasting(g)
                    && let Some(c) = l.source
                    && g.zone(c) == Zone::InPlay(f.player)
                {
                    g.top_deck(c, f.player);
                }
                Flow::done()
            }
            (Card::WayOfTheSeal, Hook::Ability(SEAL_TOPDECK)) => {
                if let Some(Occasion::Gain { card, .. }) = f.occ
                    && still_where_gained(g, card)
                {
                    g.top_deck(card, f.player);
                }
                Flow::done()
            }
            (Card::WayOfTheSquirrel, Hook::Ability(SQUIRREL_DRAW)) => {
                if f.pc == 0 {
                    f.end_lasting(g);
                }
                draw_then_done(f, 0, 2)
            }
            (Card::AnimalFair, Hook::Ability(PAY_OTHERWISE)) => animal_fair_pay(g, f, input),
            (_, Hook::Ability(_)) => return None,

            // ---- Kingdom cards ----
            (Card::AnimalFair, _) => {
                let n = g.empty_supply_piles() as u32;
                plus_buys(g, f.player, n);
                Flow::done()
            }
            (Card::Barge, _) => now_or_next_turn(g, f, input, 3),
            (Card::BlackCat, _) => black_cat(g, f),
            (Card::BountyHunter, _) => bounty_hunter(g, f, input),
            (Card::CamelTrain, _) => camel_train(g, f, input),
            (Card::Cardinal, _) => cardinal(g, f, input),
            (Card::Cavalry, _) => gain_horses(g, f, 2, Dest::Discard),
            (Card::Coven, _) => coven(g, f),
            (Card::Displace, _) => displace(g, f, input),
            (Card::Falconer, _) => falconer(g, f, input),
            (Card::Gatekeeper, _) => {
                let data = Scratch {
                    n: mask(&f.targets),
                    ..Scratch::default()
                };
                f.add_duration(g, data);
                Flow::done()
            }
            (Card::Goatherd, _) => goatherd(g, f, input),
            (Card::Groom, _) => groom(g, f, input),
            (Card::Horse, _) | (Card::WayOfTheHorse, _) => {
                return_this(g, f);
                Flow::done()
            }
            (Card::HuntingLodge, _) => hunting_lodge(g, f, input),
            (Card::Kiln, _) | (Card::Livery, _) => {
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
            (Card::Mastermind, _) => {
                f.add_duration(g, Scratch::default());
                Flow::done()
            }
            (Card::Paddock, _) => {
                if f.pc < 2 {
                    let flow = gain_horses(g, f, 2, Dest::Discard);
                    if !matches!(flow, Flow::Done(_)) {
                        return Some(flow);
                    }
                }
                let n = g.empty_supply_piles() as u32;
                plus_actions(g, f.player, n);
                Flow::done()
            }
            (Card::Sanctuary, _) => sanctuary(g, f, input),
            (Card::Scrap, _) => scrap(g, f, input),
            (Card::Sleigh, _) => gain_horses(g, f, 2, Dest::Discard),
            (Card::SnowyVillage, _) => {
                if f.player == g.turn.player {
                    g.turn.ignore_plus_actions = true;
                }
                Flow::done()
            }
            (Card::Stockpile, _) => {
                let this = f.this();
                if g.zone(this) == Zone::InPlay(f.player) {
                    exile(g, f.player, this);
                }
                Flow::done()
            }
            (Card::Supplies, _) => gain_horses(g, f, 1, Dest::DeckTop),
            (Card::VillageGreen, _) => now_or_next_turn(g, f, input, 1),
            (Card::Wayfarer, _) => may_gain(g, f, input, Card::Silver),

            // ---- Events ----
            (Card::Alliance, _) => alliance(g, f),
            (Card::Banish, _) => banish(g, f, input),
            (Card::Bargain, _) => bargain(g, f, input),
            (Card::Commerce, _) => commerce(g, f),
            (Card::Delay, _) => delay(g, f, input),
            (Card::Demand, _) => demand(g, f, input),
            (Card::Desperation, _) => desperation(g, f, input),
            (Card::Enclave, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                    if let Some(gold) = g.supply_pile(Card::Gold) {
                        return Some(Flow::Call(Frame::gain(f.player, gold, Dest::Discard)));
                    }
                }
                exile_from_supply(g, f.player, Card::Duchy);
                Flow::done()
            }
            (Card::Enhance, _) => enhance(g, f, input),
            (Card::Gamble, _) => gamble(g, f, input),
            (Card::Invest, _) => invest(g, f, input),
            (Card::March, _) => {
                let p = f.player;
                if f.pc == 0 {
                    for c in g.player(p).discard.clone() {
                        g.show_to(c, p);
                    }
                }
                let actions = cards_where(g, &g.player(p).discard, Types::ACTION);
                play_one_of(f, input, actions, true)
            }
            (Card::Populate, _) => populate(g, f, input),
            (Card::Pursue, _) => pursue(g, f, input),
            (Card::Reap, _) => reap(g, f, input),
            (Card::Ride, _) => gain_horses(g, f, 1, Dest::Discard),
            (Card::SeizeTheDay, _) => {
                let p = f.player;
                g.ext.menagerie.seized.push(p);
                g.extra_turns.push_back(ExtraTurn {
                    player: p,
                    source: Card::SeizeTheDay,
                    controller: p,
                    limit: ExtraLimit::None,
                });
                Flow::done()
            }
            (Card::Stampede, _) => {
                if f.pc == 0 && g.in_play(f.player).len() > 5 {
                    return Some(Flow::done());
                }
                gain_horses(g, f, 5, Dest::DeckTop)
            }
            (Card::Toil, _) => {
                if f.pc == 0 {
                    plus_buys(g, f.player, 1);
                }
                let actions = g.playable(f.player, Types::ACTION);
                play_one_of(f, input, actions, true)
            }
            (Card::Transport, _) => transport(g, f, input),

            // ---- Ways ----
            (Card::WayOfTheButterfly, _) => butterfly(g, f, input),
            (Card::WayOfTheCamel, _) => {
                exile_from_supply(g, f.player, Card::Gold);
                Flow::done()
            }
            (Card::WayOfTheChameleon, _) => chameleon(g, f),
            (Card::WayOfTheFrog | Card::WayOfTheSeal | Card::WayOfTheSquirrel, _) => {
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
            (Card::WayOfTheGoat, _) => match f.pc {
                0 => {
                    let hand = g.hand(f.player).to_vec();
                    f.pc = 1;
                    f.ask_cards(hand, 1, 1, Purpose::Trash)
                }
                1 => {
                    f.pc = 2;
                    Flow::Call(Frame::trash(f.player, input.cards()))
                }
                _ => Flow::done(),
            },
            (Card::WayOfTheMole, _) => match f.pc {
                0 => {
                    f.pc = 1;
                    Flow::Call(Frame::discard(f.player, g.hand(f.player).to_vec()))
                }
                _ => draw_then_done(f, 1, 3),
            },
            (Card::WayOfTheMouse, _) => {
                if f.pc == 0
                    && let Some(mouse) = g.ext.menagerie.mouse
                {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::play(
                        f.player,
                        mouse,
                        false,
                        f.chain_for_child(),
                    )));
                }
                Flow::done()
            }
            (Card::WayOfTheOwl, _) => {
                if f.pc == 0 {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::draw_to(f.player, 6)));
                }
                Flow::done()
            }
            (Card::WayOfTheRat, _) => way_of_the_rat(g, f, input),
            (Card::WayOfTheTurtle, _) => {
                let this = f.this();
                let p = f.player;
                if g.zone(this) == Zone::InPlay(p) {
                    left_play(g, this);
                    g.set_aside(this, p);
                    let data = Scratch {
                        pick: Some(this),
                        ..Scratch::default()
                    };
                    g.add_lasting(Card::WayOfTheTurtle, p, None, vec![], Until::Removed, data);
                }
                Flow::done()
            }
            (Card::WayOfTheWorm, _) => {
                exile_from_supply(g, f.player, Card::Estate);
                Flow::done()
            }
            // The other Ways are only their vanilla bonuses.
            (c, _) if c.kind() == Kind::Way => Flow::done(),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in due_durations(g, occ) {
            if STARTS.contains(&l.card) {
                let t = duration_trigger(l, START);
                out.push(match l.data.pick {
                    // Delay, Reap, Way of the Turtle: which set-aside card it plays.
                    Some(c) if l.source.is_none() => {
                        t.labelled(format!("{} ({})", l.card, g.card(c)))
                    }
                    _ => t,
                });
            }
        }
        match *occ {
            Occasion::Play { player, card } => {
                for l in g.lasting() {
                    if l.card == Card::Kiln && l.owner == player && l.until == Until::EndOfTurn {
                        out.push(Trigger::new(player, Card::Kiln, KILN_COPY).via_lasting(l.id).on(card));
                    }
                }
            }
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            Occasion::Discard { player, card } => {
                if g.card(card) == Card::VillageGreen
                    && g.zone(card) == Zone::Discard(player)
                    && g.turn.phase != Phase::Cleanup
                {
                    out.push(Trigger::new(player, Card::VillageGreen, REACT).on(card).optional());
                }
            }
            Occasion::DiscardFromPlay { player } => {
                for l in g.lasting() {
                    let Some(c) = l.source else { continue };
                    if l.card == Card::WayOfTheFrog
                        && l.owner == player
                        && g.leaving_play(player).contains(&c)
                    {
                        // Differently named cards go onto the deck in a chosen order.
                        let kind = g.card(c);
                        out.push(
                            Trigger::new(player, Card::WayOfTheFrog, FROG_TOPDECK)
                                .via_lasting(l.id)
                                .data(kind as i32)
                                .labelled(format!("Way of the Frog ({kind})")),
                        );
                    }
                }
            }
            Occasion::EndTurn { .. } => {
                for l in g.lasting() {
                    if l.card == Card::WayOfTheSquirrel {
                        out.push(
                            Trigger::new(l.owner, Card::WayOfTheSquirrel, SQUIRREL_DRAW)
                                .via_lasting(l.id),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn modify_cost(g: &Game, card: Card, _id: Option<CardId>, cost: &mut Cost) {
        let turn_player = g.turn_player();
        match card {
            Card::Fisherman if turn_player.is_some_and(|p| g.player(p).discard.is_empty()) => {
                *cost = cost.minus_coins(3);
            }
            Card::Destrier if turn_player.is_some() => {
                *cost = cost.minus_coins(g.turn.gained.len().min(255) as u8);
            }
            _ => {}
        }
    }

    fn fixed_cost(g: &Game, card: Card, _id: Option<CardId>) -> Option<Cost> {
        if card != Card::Wayfarer || g.turn_player().is_none() {
            return None;
        }
        let log = g.log();
        let start = log
            .iter()
            .rposition(|e| matches!(e, Event::TurnStart { .. }))
            .unwrap_or(0);
        log[start..].iter().rev().find_map(|e| match e {
            Event::Gained { card, .. } if *card != Card::Wayfarer => Some(g.cost(*card)),
            _ => None,
        })
    }

    fn pays_otherwise(g: &Game, p: PlayerId, card: Card) -> bool {
        card == Card::AnimalFair && !g.cards_in_hand_where(p, Types::ACTION).is_empty()
    }

    fn can_use_way(g: &Game, id: CardId, way: Card) -> bool {
        !(way == Card::WayOfTheMouse && g.ext.menagerie.mouse == Some(id))
    }

    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        let st = &g.ext.menagerie;
        match card {
            Card::SeizeTheDay => !st.seized.contains(&p),
            Card::Desperation => st.desperation != Some((p, g.player(p).turns_taken)),
            _ => true,
        }
    }

    fn after_step(g: &mut Game, f: &mut CardFrame, before: &Step, flow: &mut Flow) {
        if !exchanges(g, f) {
            return;
        }
        let p = f.player;
        let mut got = 0;
        if p == g.turn.player {
            got = g.turn.coins.saturating_sub(before.coins);
            g.turn.coins -= got;
            let pl = &mut g.players[p.index()];
            if before.minus_coin && !pl.minus_coin {
                pl.minus_coin = true;
                got += 1;
            }
        }
        if let Flow::Call(Frame::Draw { player, count, .. }) = flow
            && *player == p
            && says_plus_cards(f.card)
        {
            let n = std::mem::take(count);
            g.add_coins(p, n as u32);
        }
        if got > 0 {
            let rest = std::mem::replace(flow, Flow::Call(Frame::draw(p, got as usize)));
            f.hold(rest);
        }
    }

    fn ensure(g: &mut Game, s: &mut Setup) {
        if g.ext.menagerie.mouse.is_none() && g.landscapes.contains(&Card::WayOfTheMouse) {
            let candidates: Vec<Card> = Card::ALL
                .iter()
                .copied()
                .filter(|&c| {
                    let def = c.def();
                    c.kind() == Kind::Card
                        && def.kingdom
                        && def.pile.is_none()
                        && c.is(Types::ACTION)
                        && !c.is(Types::DURATION)
                        && matches!(def.cost, Some(cost) if cost == Cost::coins(2) || cost == Cost::coins(3))
                        && setup::unused(g, c)
                })
                .collect();
            if let Some(card) = s.choose(g, Card::WayOfTheMouse, candidates) {
                let pile = setup::add_pile(g, card, 1, false);
                g.ext.menagerie.mouse = g.pile_top(pile);
            }
        }
        if setup::in_game(g, |c| HORSE_USERS.contains(&c)) && g.pile_of(Card::Horse).is_none() {
            setup::add_pile(g, Card::Horse, 30, false);
        }
    }
}

// ---- Helpers ----

fn plus_actions(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player && !g.turn.ignore_plus_actions {
        g.turn.actions += n;
    }
}

fn plus_buys(g: &mut Game, p: PlayerId, n: u32) {
    if p == g.turn.player {
        g.turn.buys += n;
    }
}

fn mask(players: &[PlayerId]) -> i32 {
    players.iter().fold(0, |m, p| m | 1 << p.0)
}

fn in_mask(mask: i32, p: PlayerId) -> bool {
    mask & (1 << p.0) != 0
}

/// At step `pc`, draws `n` cards and finishes.
fn draw_then_done(f: &mut CardFrame, pc: u8, n: usize) -> Flow {
    if f.pc == pc {
        f.pc = pc + 1;
        return Flow::Call(Frame::draw(f.player, n));
    }
    Flow::done()
}

fn cards_where(g: &Game, cards: &[CardId], types: Types) -> Vec<CardId> {
    cards.iter().copied().filter(|&c| g.is(c, types)).collect()
}

/// The Supply pile whose top card is `card`.
fn supply_pile_topped_by(g: &Game, card: Card) -> Option<PileId> {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&id| g.pile(id).in_supply && g.pile_top(id).is_some_and(|t| g.card(t) == card))
}

/// The gain of `card` in progress.
fn gain_of(g: &Game, card: CardId) -> Option<&GainState> {
    g.gains.iter().rev().find(|st| st.card == card)
}

/// Whether a card being gained is still where it was gained to (the stop-moving rule).
fn still_where_gained(g: &Game, card: CardId) -> bool {
    g.still_where_gained(card)
}

/// Puts `id` on `p`'s Exile mat.
fn exile(g: &mut Game, p: PlayerId, id: CardId) {
    g.ext.menagerie.invested.retain(|&c| c != id);
    g.put_on_mat(id, p, Mat::Exile);
}

/// Exiles the top card of `card`'s Supply pile.
fn exile_from_supply(g: &mut Game, p: PlayerId, card: Card) -> Option<CardId> {
    let top = g.pile_top(g.supply_pile(card)?)?;
    exile(g, p, top);
    Some(top)
}

/// The copies of `card` on `p`'s Exile mat.
fn exiled_copies(g: &Game, p: PlayerId, card: Card) -> Vec<CardId> {
    g.player(p)
        .mat(Mat::Exile)
        .iter()
        .copied()
        .filter(|&c| g.card(c) == card)
        .collect()
}

/// Whether the text above the card's dividing line says "+N Card(s)" beyond its vanilla
/// bonus, so its drawing is +Cards rather than "draw" (Way of the Chameleon).
fn says_plus_cards(card: Card) -> bool {
    let text = card.text().split("\n---").next().unwrap_or("");
    let mut count = 0;
    let bytes = text.as_bytes();
    for (i, _) in text.match_indices('+') {
        let digits = bytes[i + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
        // Governor's is "+1 (+3) Cards".
        let rest = text[i + 1 + digits..].trim_start_matches(')');
        if digits > 0 && rest.starts_with(" Card") {
            count += 1;
        }
    }
    count > usize::from(card.def().plus.cards > 0)
}

/// Whether ability `f` is one whose +$ and +Cards Way of the Chameleon exchanges.
fn exchanges(g: &Game, f: &CardFrame) -> bool {
    let st = &g.ext.menagerie;
    if f.hook == Hook::Main && f.card.kind() == Kind::Card {
        return f.this.is_some_and(|id| st.chameleon.contains(&id));
    }
    f.lasting.is_some_and(|l| {
        st.chameleon_lastings.contains(&l) && g.lasting().iter().any(|x| x.id == l)
    })
}

/// A card that left play ends the Duration effects it made this turn.
fn left_play(g: &mut Game, id: CardId) {
    let card = g.card(id);
    g.lasting.retain(|l| {
        !(l.source == Some(id)
            && l.card == card
            && l.until == Until::Removed
            && l.turn == g.players[l.owner.index()].turns_taken)
    });
}

/// Returns the played card to its pile if it is in play (Horse, Way of the Horse).
fn return_this(g: &mut Game, f: &CardFrame) -> bool {
    let this = f.this();
    if g.zone(this) != Zone::InPlay(f.player) {
        return false;
    }
    left_play(g, this);
    g.return_to_pile(this)
}

/// Gains `n` Horses one at a time, then finishes. Uses `f.s.i`; leaves `f.pc` at 2.
fn gain_horses(g: &mut Game, f: &mut CardFrame, n: usize, to: Dest) -> Flow {
    f.pc = 1;
    if f.s.i < n
        && let Some(horses) = g.pile_of(Card::Horse)
        && g.pile_top(horses).is_some()
    {
        f.s.i += 1;
        return Flow::Call(Frame::gain(f.player, horses, to));
    }
    f.pc = 2;
    Flow::done()
}

/// "You may gain a `card`."
fn may_gain(g: &mut Game, f: &mut CardFrame, input: Input, card: Card) -> Flow {
    match f.pc {
        0 => {
            if g.supply_pile(card).and_then(|p| g.pile_top(p)).is_none() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Gain)
        }
        1 if input.is_yes() => {
            f.pc = 2;
            match g.supply_pile(card) {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

/// Asks for a Supply pile among `piles` and gains from it to `to`; steps `pc`, `pc + 1`.
fn gain_from(f: &mut CardFrame, input: Input, piles: Vec<PileId>, pc: u8, to: Dest) -> Flow {
    if f.pc == pc {
        if piles.is_empty() {
            f.pc = pc + 2;
            return Flow::Continue;
        }
        f.pc = pc + 1;
        return f.ask_pile(piles, false, Purpose::Gain);
    }
    f.pc = pc + 2;
    match input.pile() {
        Some(pile) => Flow::Call(Frame::gain(f.player, pile, to)),
        None => Flow::Continue,
    }
}

/// "You may play one of `cards`" (without using an Action); plays it from where it is.
fn play_one_of(
    f: &mut CardFrame,
    input: Input,
    cards: Vec<CardId>,
    optional: bool,
) -> Flow {
    match f.pc {
        0 => {
            if cards.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(cards, usize::from(!optional), 1, Purpose::Play)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.pc = 2;
            Flow::Call(Frame::play(f.player, c, true, vec![]))
        }
        _ => Flow::done(),
    }
}

/// A reaction that plays this card from `zone` if it is still there.
fn play_from(g: &mut Game, f: &mut CardFrame, zone: Zone) -> Flow {
    let this = f.this();
    let allowed = !matches!(zone, Zone::Hand(_)) || super::can_play(g, f.player, this);
    if f.pc == 0 && g.zone(this) == zone && allowed {
        f.pc = 1;
        if let Zone::Discard(p) = zone {
            g.reveal(p, this);
        }
        return Flow::Call(Frame::play(f.player, this, true, vec![]));
    }
    Flow::done()
}

/// The number of cards `p` trashed during their most recent finished turn.
fn trashed_last_turn(g: &Game, p: PlayerId) -> usize {
    let log = g.log();
    let is_start = |e: &Event| matches!(e, Event::TurnStart { .. });
    let mut end = log.len();
    if g.turn.player == p {
        end = log.iter().rposition(is_start).unwrap_or(0);
    }
    let Some(begin) = log[..end]
        .iter()
        .rposition(|e| matches!(e, Event::TurnStart { player, .. } if *player == p))
    else {
        return 0;
    };
    log[begin + 1..]
        .iter()
        .take_while(|e| !is_start(e))
        .filter(|e| matches!(e, Event::Trashed { player, .. } if *player == p))
        .count()
}

// ---- Triggers ----

fn gain_triggers(g: &Game, gainer: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let kind = g.card(card);
    let st = &g.ext.menagerie;
    let copies: Vec<CardId> = exiled_copies(g, gainer, kind)
        .into_iter()
        .filter(|&c| c != card)
        .collect();
    if !copies.is_empty() {
        out.push(
            Trigger::new(gainer, kind, EXILE_DISCARD)
                .on(card)
                .optional()
                .labelled(format!("Discard {kind} from Exile")),
        );
    }
    let in_place = still_where_gained(g, card);

    // Gatekeeper: only if the gainer had no Exiled copy when they gained it.
    let action_or_treasure = g.is(card, Types::ACTION | Types::TREASURE);
    let had_copies = gain_of(g, card)
        .is_some_and(|gain| st.exile_discarded.contains(&(card, gain.since)));
    if action_or_treasure && in_place && copies.is_empty() && !had_copies {
        for l in g.lasting() {
            let active = l.until == Until::Removed && g.player(l.owner).turns_taken == l.turn;
            if l.card == Card::Gatekeeper && active && l.owner != gainer && in_mask(l.data.n, gainer)
            {
                out.push(
                    Trigger::new(gainer, Card::Gatekeeper, GATEKEEPER_EXILE)
                        .via_lasting(l.id)
                        .on(card),
                );
            }
        }
    }

    for l in g.lasting() {
        if l.owner != gainer || l.until != Until::EndOfTurn {
            continue;
        }
        match l.card {
            Card::Livery => {
                if g.gain_cost(card).is_some_and(|c| c.coins >= 4) {
                    out.push(Trigger::new(gainer, Card::Livery, LIVERY_HORSE).via_lasting(l.id).on(card));
                }
            }
            Card::WayOfTheSeal if in_place => {
                out.push(
                    Trigger::new(gainer, Card::WayOfTheSeal, SEAL_TOPDECK)
                        .via_lasting(l.id)
                        .on(card)
                        .optional(),
                );
            }
            _ => {}
        }
    }

    for &c in &st.invested {
        if let Zone::Mat(owner, Mat::Exile) = g.zone(c)
            && owner != gainer
            && g.card(c) == kind
        {
            out.push(Trigger::new(owner, Card::Invest, INVEST_DRAW).on(c).data(card.0 as i32));
        }
    }

    match kind {
        Card::CamelTrain | Card::Cavalry | Card::Hostelry => {
            out.push(Trigger::new(gainer, kind, ON_GAIN).on(card));
        }
        _ => {}
    }

    let two_types = g.types(card).bits().count_ones() >= 2;
    for p in g.players_from(g.turn.player) {
        for &c in g.hand(p) {
            let react = match g.card(c) {
                Card::BlackCat => p != gainer && g.is(card, Types::VICTORY),
                Card::Sheepdog | Card::Sleigh => p == gainer,
                Card::Falconer => two_types,
                _ => false,
            };
            // Sleigh is discarded, not played.
            if react && (g.card(c) == Card::Sleigh || super::can_play(g, p, c)) {
                out.push(
                    Trigger::new(p, g.card(c), REACT)
                        .on(c)
                        .data(card.0 as i32)
                        .optional(),
                );
            }
        }
    }
}

fn exile_discard(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc > 0 {
        return Flow::done();
    }
    let Some(Occasion::Gain { card, .. }) = f.occ else {
        return Flow::done();
    };
    let p = f.player;
    let copies: Vec<CardId> = exiled_copies(g, p, g.card(card))
        .into_iter()
        .filter(|&c| c != card)
        .collect();
    let since = gain_of(g, card).map(|gain| gain.since);
    let st = &mut g.ext.menagerie;
    let gains = &g.gains;
    st.exile_discarded
        .retain(|&(c, since)| gains.iter().any(|s| s.card == c && s.since == since));
    st.exile_discarded.extend(since.map(|since| (card, since)));
    st.invested.retain(|c| !copies.contains(c));
    f.pc = 1;
    Flow::Call(Frame::discard(p, copies))
}

fn sleigh_react(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(Occasion::Gain { card, .. }) = f.occ else {
        return Flow::done();
    };
    let p = f.player;
    match f.pc {
        0 => {
            let this = f.this();
            if g.zone(this) != Zone::Hand(p) {
                return Flow::done();
            }
            f.pc = 1;
            Flow::Call(Frame::discard(p, vec![this]))
        }
        1 => {
            if !still_where_gained(g, card) {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_either(&["Put it into your hand", "Put it onto your deck"])
        }
        _ => {
            if still_where_gained(g, card) {
                if input.opt() == Some(Opt::Mode(0)) {
                    g.put_in_hand(card, p);
                } else {
                    g.top_deck(card, p);
                }
            }
            Flow::done()
        }
    }
}

fn cavalry_gain(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    if f.pc == 0 {
        f.pc = 1;
        return Flow::Call(Frame::draw(p, 2));
    }
    if f.pc == 1 {
        f.pc = 2;
        if p == g.turn.player {
            g.turn.buys += 1;
        }
        return Flow::Call(Frame::return_to_action(p));
    }
    Flow::done()
}

fn hostelry_gain(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let treasures = g.cards_in_hand_where(p, Types::TREASURE);
            if treasures.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            let n = treasures.len();
            f.ask_cards(treasures, 0, n, Purpose::Discard)
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
        _ => {
            if f.s.i < f.s.n as usize
                && let Some(horses) = g.pile_of(Card::Horse)
                && g.pile_top(horses).is_some()
            {
                f.s.i += 1;
                return Flow::Call(Frame::gain(p, horses, Dest::Discard));
            }
            Flow::done()
        }
    }
}

fn kiln_copy(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let Some(Occasion::Play { card, .. }) = f.occ else {
        return Flow::done();
    };
    match f.pc {
        0 => {
            f.end_lasting(g);
            if supply_pile_topped_by(g, g.card(card)).is_none() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Gain)
        }
        1 if input.is_yes() => {
            f.pc = 2;
            match supply_pile_topped_by(g, g.card(card)) {
                Some(pile) => Flow::Call(Frame::gain(f.player, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}

fn animal_fair_pay(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    let actions = g.cards_in_hand_where(p, Types::ACTION);
    match f.pc {
        0 => {
            if actions.is_empty() {
                return Flow::Done(Value::Bool(false));
            }
            let cost = g.cost_of(f.this());
            // Only the turn's player has $ to pay with (Black Market on another's turn).
            let affordable = p == g.turn.player
                && cost.coins as u32 <= g.turn.coins
                && cost.potions as u32 <= g.turn.potions;
            f.pc = 2;
            if affordable {
                f.pc = 1;
                return f.ask_yes_no(Purpose::Trash);
            }
            f.ask_cards(actions, 1, 1, Purpose::Trash)
        }
        1 => {
            if !input.is_yes() {
                return Flow::Done(Value::Bool(false));
            }
            f.pc = 2;
            f.ask_cards(actions, 1, 1, Purpose::Trash)
        }
        2 => {
            f.pc = 3;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        _ => Flow::Done(Value::Bool(true)),
    }
}

// ---- Start of turn ----

fn start(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        let Some(l) = f.end_lasting(g) else {
            return Flow::done();
        };
        f.s = l.data;
        f.pc = 1;
    }
    let p = f.player;
    match f.card {
        Card::Barge => {
            if f.pc == 1 {
                plus_buys(g, p, 1);
            }
            draw_then_done(f, 1, 3)
        }
        Card::VillageGreen => {
            if f.pc == 1 {
                plus_actions(g, p, 2);
            }
            draw_then_done(f, 1, 1)
        }
        Card::Gatekeeper => {
            g.add_coins(p, 3);
            Flow::done()
        }
        Card::Mastermind => match f.pc {
            1 => {
                let actions = g.playable(p, Types::ACTION);
                if actions.is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                f.ask_cards(actions, 0, 1, Purpose::Play)
            }
            2 => {
                let Some(c) = input.card() else {
                    return Flow::done();
                };
                f.s.pick = Some(c);
                f.pc = 3;
                Flow::Call(Frame::play(p, c, true, f.chain_for_child()))
            }
            3 | 4 => {
                f.pc += 1;
                let c = f.s.pick.expect("set in step 2");
                Flow::Call(Frame::play(p, c, false, f.chain_for_child()))
            }
            _ => Flow::done(),
        },
        Card::Delay | Card::Reap | Card::WayOfTheTurtle => {
            if f.pc == 1
                && let Some(c) = f.s.pick
                && g.zone(c) == Zone::SetAside(p)
            {
                f.pc = 2;
                return Flow::Call(Frame::play(p, c, true, vec![]));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

// ---- Kingdom cards ----

/// Barge, Village Green: "Either now or at the start of your next turn, +`cards` Cards and
/// (Barge) +1 Buy / (Village Green) +2 Actions."
fn now_or_next_turn(g: &mut Game, f: &mut CardFrame, input: Input, cards: usize) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_either(&["Now", "At the start of your next turn"])
        }
        1 => {
            if input.opt() == Some(Opt::Mode(1)) {
                f.add_duration(g, Scratch::default());
                return Flow::done();
            }
            if f.card == Card::Barge {
                plus_buys(g, p, 1);
            } else {
                plus_actions(g, p, 2);
            }
            f.pc = 2;
            Flow::Call(Frame::draw(p, cards))
        }
        _ => Flow::done(),
    }
}

fn black_cat(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.player == g.turn.player {
        return Flow::done();
    }
    let Some(&target) = f.targets.get(f.s.i) else {
        return Flow::done();
    };
    f.s.i += 1;
    match g.supply_pile(Card::Curse) {
        Some(curse) => Flow::Call(Frame::gain(target, curse, Dest::Discard)),
        None => Flow::done(),
    }
}

fn bounty_hunter(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Exile)
        }
        _ => {
            if let Some(c) = input.card() {
                let had = !exiled_copies(g, p, g.card(c)).is_empty();
                exile(g, p, c);
                if !had {
                    g.add_coins(p, 3);
                }
            }
            Flow::done()
        }
    }
}

fn camel_train(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let piles = g.supply_piles_where(|t, _| !t.intersects(Types::VICTORY));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Exile)
        }
        _ => {
            if let Some(pile) = input.pile()
                && let Some(top) = g.pile_top(pile)
            {
                exile(g, f.player, top);
            }
            Flow::done()
        }
    }
}

fn cardinal(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let mut input = Some(input);
    loop {
        let Some(&target) = f.targets.get(f.s.i) else {
            return Flow::done();
        };
        match f.pc {
            0 => {
                f.pc = 1;
                return Flow::Call(Frame::reveal(target, 2));
            }
            1 => {
                f.s.cards = input.take().map(Input::cards).unwrap_or_default();
                let eligible: Vec<CardId> = f
                    .s
                    .cards
                    .iter()
                    .copied()
                    .filter(|&c| {
                        let cost = g.cost_of(c);
                        (3..=6).contains(&cost.coins) && cost.potions == 0 && cost.debt == 0
                    })
                    .collect();
                if eligible.is_empty() {
                    f.pc = 3;
                    continue;
                }
                f.pc = 2;
                return f.ask_cards_for(target, eligible, 1, 1, Purpose::Exile);
            }
            2 => {
                for c in input.take().map(Input::cards).unwrap_or_default() {
                    if g.zone(c) == Zone::Deck(target) {
                        exile(g, target, c);
                    }
                }
                f.pc = 3;
            }
            3 => {
                let rest: Vec<CardId> = std::mem::take(&mut f.s.cards)
                    .into_iter()
                    .filter(|&c| g.zone(c) == Zone::Deck(target))
                    .collect();
                f.pc = 4;
                return Flow::Call(Frame::discard(target, rest));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
            }
        }
    }
}

fn coven(g: &mut Game, f: &mut CardFrame) -> Flow {
    while let Some(&target) = f.targets.get(f.s.i) {
        f.s.i += 1;
        if exile_from_supply(g, target, Card::Curse).is_none() {
            let curses = exiled_copies(g, target, Card::Curse);
            if !curses.is_empty() {
                return Flow::Call(Frame::discard(target, curses));
            }
        }
    }
    Flow::done()
}

fn displace(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 1, 1, Purpose::Exile)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let limit = g.cost_of(c).plus_coins(2);
            let name = g.card(c);
            exile(g, p, c);
            let piles: Vec<PileId> = g
                .piles_costing_up_to(limit)
                .into_iter()
                .filter(|&pile| g.pile_top(pile).is_some_and(|t| g.card(t) != name))
                .collect();
            f.pc = 2;
            gain_from(f, Input::Start, piles, 2, Dest::Discard)
        }
        2 | 3 => gain_from(f, input, vec![], 2, Dest::Discard),
        _ => Flow::done(),
    }
}

fn falconer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc == 0 {
        let limit = g.cost_of(f.this());
        let piles = g.supply_piles_where(|_, c| c.less_than(limit));
        return gain_from(f, input, piles, 0, Dest::Hand);
    }
    if f.pc == 1 {
        return gain_from(f, input, vec![], 0, Dest::Hand);
    }
    Flow::done()
}

fn goatherd(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            f.pc = 1;
            if hand.is_empty() {
                return Flow::Continue;
            }
            f.ask_cards(hand, 0, 1, Purpose::Trash)
        }
        1 => {
            f.pc = 2;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        2 => {
            let n = trashed_last_turn(g, g.right_of(p));
            f.pc = 3;
            Flow::Call(Frame::draw(p, n))
        }
        _ => Flow::done(),
    }
}

fn groom(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let piles = g.piles_costing_up_to(Cost::coins(4));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        2 => {
            // No bonus for a card that was not gained (a when-would-gain exchange).
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let t = g.types(c);
            f.s.n = i32::from(t.intersects(Types::ACTION))
                | i32::from(t.intersects(Types::TREASURE)) << 1
                | i32::from(t.intersects(Types::VICTORY)) << 2;
            f.pc = 3;
            if f.s.n & 1 != 0
                && let Some(horses) = g.pile_of(Card::Horse)
            {
                return Flow::Call(Frame::gain(p, horses, Dest::Discard));
            }
            Flow::Continue
        }
        3 => {
            f.pc = 4;
            if f.s.n & 2 != 0
                && let Some(silver) = g.supply_pile(Card::Silver)
            {
                return Flow::Call(Frame::gain(p, silver, Dest::Discard));
            }
            Flow::Continue
        }
        4 => {
            f.pc = 5;
            if f.s.n & 4 != 0 {
                plus_actions(g, p, 1);
                return Flow::Call(Frame::draw(p, 1));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn hunting_lodge(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_yes_no(Purpose::Discard)
        }
        1 => {
            if !input.is_yes() {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::discard(p, g.hand(p).to_vec()))
        }
        _ => draw_then_done(f, 2, 5),
    }
}

fn sanctuary(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            let hand = g.hand(f.player).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 0, 1, Purpose::Exile)
        }
        _ => {
            if let Some(c) = input.card() {
                exile(g, f.player, c);
            }
            Flow::done()
        }
    }
}

const SCRAP_MODES: [&str; 6] = [
    "+1 Card",
    "+1 Action",
    "+1 Buy",
    "+$1",
    "Gain a Silver",
    "Gain a Horse",
];

fn scrap(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            let n = (f.s.cost.coins as usize).min(6);
            f.pc = 3;
            f.ask_modes(&SCRAP_MODES, n, n)
        }
        3 => {
            f.s.n = input
                .options()
                .iter()
                .map(|o| match o {
                    Opt::Mode(i) => 1 << i,
                    _ => 0,
                })
                .sum();
            f.pc = 4;
            if f.s.n & 1 != 0 {
                return Flow::Call(Frame::draw(p, 1));
            }
            Flow::Continue
        }
        4 => {
            if f.s.n & 2 != 0 {
                plus_actions(g, p, 1);
            }
            if f.s.n & 4 != 0 {
                plus_buys(g, p, 1);
            }
            if f.s.n & 8 != 0 {
                g.add_coins(p, 1);
            }
            f.pc = 5;
            if f.s.n & 16 != 0
                && let Some(silver) = g.supply_pile(Card::Silver)
            {
                return Flow::Call(Frame::gain(p, silver, Dest::Discard));
            }
            Flow::Continue
        }
        5 => {
            f.pc = 6;
            if f.s.n & 32 != 0
                && let Some(horses) = g.pile_of(Card::Horse)
            {
                return Flow::Call(Frame::gain(p, horses, Dest::Discard));
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

// ---- Events ----

fn alliance(g: &mut Game, f: &mut CardFrame) -> Flow {
    const CARDS: [Card; 6] = [
        Card::Province,
        Card::Duchy,
        Card::Estate,
        Card::Gold,
        Card::Silver,
        Card::Copper,
    ];
    while let Some(&card) = CARDS.get(f.s.i) {
        f.s.i += 1;
        if let Some(pile) = g.supply_pile(card) {
            return Flow::Call(Frame::gain(f.player, pile, Dest::Discard));
        }
    }
    Flow::done()
}

fn banish(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(hand, 0, 1, Purpose::Exile)
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            let name = g.card(c);
            exile(g, p, c);
            let more: Vec<CardId> = g
                .hand(p)
                .iter()
                .copied()
                .filter(|&x| g.card(x) == name)
                .collect();
            if more.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            let n = more.len();
            f.ask_cards(more, 0, n, Purpose::Exile)
        }
        _ => {
            for c in input.cards() {
                exile(g, p, c);
            }
            Flow::done()
        }
    }
}

fn bargain(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        let piles = if f.pc == 0 {
            g.supply_piles_where(|t, c| !t.intersects(Types::VICTORY) && c.at_most(Cost::coins(5)))
        } else {
            vec![]
        };
        return gain_from(f, input, piles, 0, Dest::Discard);
    }
    let others = g.others(f.player);
    if let Some(&other) = others.get(f.s.i) {
        f.s.i += 1;
        if let Some(horses) = g.pile_of(Card::Horse) {
            return Flow::Call(Frame::gain(other, horses, Dest::Discard));
        }
        return Flow::Continue;
    }
    Flow::done()
}

fn commerce(g: &mut Game, f: &mut CardFrame) -> Flow {
    if f.pc == 0 {
        let mut names = g.turn.gained.clone();
        names.sort();
        names.dedup();
        f.s.n = if f.player == g.turn.player {
            names.len() as i32
        } else {
            0
        };
        f.pc = 1;
    }
    if (f.s.i as i32) < f.s.n
        && let Some(gold) = g.supply_pile(Card::Gold)
    {
        f.s.i += 1;
        return Flow::Call(Frame::gain(f.player, gold, Dest::Discard));
    }
    Flow::done()
}

fn delay(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let actions = g.cards_in_hand_where(p, Types::ACTION);
            if actions.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(actions, 0, 1, Purpose::SetAside)
        }
        _ => {
            if let Some(c) = input.card() {
                g.set_aside(c, p);
                let data = Scratch {
                    pick: Some(c),
                    ..Scratch::default()
                };
                g.add_lasting(Card::Delay, p, None, vec![], Until::Removed, data);
            }
            Flow::done()
        }
    }
}

fn demand(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    match f.pc {
        0 => {
            f.pc = 1;
            match g.pile_of(Card::Horse) {
                Some(horses) => Flow::Call(Frame::gain(f.player, horses, Dest::DeckTop)),
                None => Flow::Continue,
            }
        }
        1 => {
            let piles = g.piles_costing_up_to(Cost::coins(4));
            gain_from(f, Input::Start, piles, 1, Dest::DeckTop)
        }
        2 => gain_from(f, input, vec![], 1, Dest::DeckTop),
        _ => Flow::done(),
    }
}

fn desperation(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            g.ext.menagerie.desperation = Some((p, g.player(p).turns_taken));
            f.pc = 1;
            f.ask_yes_no(Purpose::Gain)
        }
        1 => {
            if !input.is_yes() {
                return Flow::done();
            }
            f.pc = 2;
            match g.supply_pile(Card::Curse) {
                Some(curse) => Flow::Call(Frame::gain(p, curse, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => {
            if !input.cards().is_empty() {
                plus_buys(g, p, 1);
                g.add_coins(p, 2);
            }
            Flow::done()
        }
    }
}

fn enhance(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let candidates: Vec<CardId> = g
                .hand(p)
                .iter()
                .copied()
                .filter(|&c| !g.is(c, Types::VICTORY))
                .collect();
            if candidates.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(candidates, 0, 1, Purpose::Trash)
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
            let piles = g.piles_costing_up_to(f.s.cost.plus_coins(2));
            gain_from(f, Input::Start, piles, 2, Dest::Discard)
        }
        3 => gain_from(f, input, vec![], 2, Dest::Discard),
        _ => Flow::done(),
    }
}

fn gamble(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            plus_buys(g, p, 1);
            f.pc = 1;
            Flow::Call(Frame::look(p, 1))
        }
        1 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::discard(p, vec![c]))
        }
        2 => {
            let c = f.s.pick.expect("set in step 1");
            if g.zone(c) != Zone::Discard(p) || !g.is(c, Types::ACTION | Types::TREASURE) {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_yes_no(Purpose::Play)
        }
        3 if input.is_yes() => {
            let c = f.s.pick.expect("set in step 1");
            f.pc = 4;
            if g.zone(c) != Zone::Discard(p) {
                return Flow::done();
            }
            Flow::Call(Frame::play(p, c, true, vec![]))
        }
        _ => Flow::done(),
    }
}

fn invest(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let piles = g.supply_piles_where(|t, _| t.intersects(Types::ACTION));
            if piles.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_pile(piles, false, Purpose::Exile)
        }
        1 => {
            let Some(top) = input.pile().and_then(|pile| g.pile_top(pile)) else {
                return Flow::done();
            };
            exile(g, p, top);
            g.ext.menagerie.invested.push(top);
            let name = g.card(top);
            // Every other player's Investment in a copy draws them 2 cards, in turn order.
            for q in g.others(p) {
                for &c in &g.ext.menagerie.invested {
                    if g.zone(c) == Zone::Mat(q, Mat::Exile) && g.card(c) == name {
                        f.s.cards.push(c);
                    }
                }
            }
            f.pc = 2;
            Flow::Continue
        }
        _ => {
            if f.s.i < f.s.cards.len() {
                let c = f.s.cards[f.s.i];
                f.s.i += 1;
                if let Zone::Mat(owner, _) = g.zone(c) {
                    return Flow::Call(Frame::draw(owner, 2));
                }
                return Flow::Continue;
            }
            Flow::done()
        }
    }
}

fn populate(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let done = |f: &CardFrame, pile: PileId| {
        let bit = pile.0 as u64;
        let mask = (f.s.n as u32 as u64) | ((f.s.m as u32 as u64) << 32);
        bit < 64 && mask & (1 << bit) != 0
    };
    if f.pc == 1 {
        if let Some(pile) = input.pile() {
            let bit = pile.0 as u64;
            let mut mask = (f.s.n as u32 as u64) | ((f.s.m as u32 as u64) << 32);
            if bit < 64 {
                mask |= 1 << bit;
            }
            f.s.n = mask as u32 as i32;
            f.s.m = (mask >> 32) as u32 as i32;
            f.pc = 0;
            return Flow::Call(Frame::gain(f.player, pile, Dest::Discard));
        }
        return Flow::done();
    }
    let piles: Vec<PileId> = (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .filter(|&id| {
            let pile = g.pile(id);
            // By the pile's card as printed: Enlightenment and Inheritance change cards,
            // not piles.
            pile.in_supply && pile.card.is(Types::ACTION) && !pile.cards.is_empty() && !done(f, id)
        })
        .collect();
    if piles.is_empty() {
        return Flow::done();
    }
    f.pc = 1;
    f.ask_pile(piles, false, Purpose::Gain)
}

fn pursue(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            plus_buys(g, p, 1);
            f.pc = 1;
            f.ask_name()
        }
        1 => {
            f.s.card = input.name();
            f.pc = 2;
            Flow::Call(Frame::reveal(p, 4))
        }
        2 => {
            let named = f.s.card;
            let (matches, rest): (Vec<CardId>, Vec<CardId>) = input
                .cards()
                .into_iter()
                .partition(|&c| Some(g.card(c)) == named);
            f.s.cards = matches;
            f.pc = 3;
            Flow::Call(Frame::discard(p, rest))
        }
        3 => {
            f.s.cards.retain(|&c| g.zone(c) == Zone::Deck(p));
            f.pc = 4;
            f.ask_order(f.s.cards.clone(), Purpose::TopDeck)
        }
        _ => {
            for c in input.cards().into_iter().rev() {
                if g.zone(c) == Zone::Deck(p) {
                    g.move_card(c, Zone::Deck(p), Pos::Top);
                }
            }
            Flow::done()
        }
    }
}

fn reap(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            match g.supply_pile(Card::Gold) {
                Some(gold) => Flow::Call(Frame::gain(p, gold, Dest::SetAside)),
                None => Flow::done(),
            }
        }
        _ => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::SetAside(p)
            {
                let data = Scratch {
                    pick: Some(c),
                    ..Scratch::default()
                };
                g.add_lasting(Card::Reap, p, None, vec![], Until::Removed, data);
            }
            Flow::done()
        }
    }
}

fn transport(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Exile an Action card from the Supply",
                    "Put an Action card you have in Exile onto your deck",
                ],
                1,
                1,
            )
        }
        1 => {
            if input.opt() == Some(Opt::Mode(0)) {
                let piles = g.supply_piles_where(|t, _| t.intersects(Types::ACTION));
                if piles.is_empty() {
                    return Flow::done();
                }
                f.pc = 2;
                return f.ask_pile(piles, false, Purpose::Exile);
            }
            let exiled = cards_where(g, g.player(p).mat(Mat::Exile), Types::ACTION);
            if exiled.is_empty() {
                return Flow::done();
            }
            f.pc = 3;
            f.ask_cards(exiled, 1, 1, Purpose::TopDeck)
        }
        2 => {
            if let Some(top) = input.pile().and_then(|pile| g.pile_top(pile)) {
                exile(g, p, top);
            }
            Flow::done()
        }
        _ => {
            if let Some(c) = input.card() {
                g.ext.menagerie.invested.retain(|&x| x != c);
                g.top_deck(c, p);
            }
            Flow::done()
        }
    }
}

// ---- Ways ----

fn butterfly(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let this = f.this();
    match f.pc {
        0 => {
            if g.zone(this) != Zone::InPlay(f.player) || g.home_pile(g.printed(this)).is_none() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_yes_no(Purpose::Return)
        }
        1 => {
            if !input.is_yes() || g.zone(this) != Zone::InPlay(f.player) {
                return Flow::done();
            }
            let target = g.cost_of(this).plus_coins(1);
            left_play(g, this);
            if !g.return_to_pile(this) {
                return Flow::done();
            }
            let piles = g.supply_piles_where(|_, c| c == target);
            f.pc = 2;
            gain_from(f, Input::Start, piles, 2, Dest::Discard)
        }
        2 | 3 => gain_from(f, input, vec![], 2, Dest::Discard),
        _ => Flow::done(),
    }
}

fn chameleon(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    let id = f.this();
    match f.pc {
        0 => {
            let play = g.plays.last().filter(|st| st.card == id);
            // The card's instructions do nothing (Highwayman).
            if play.is_some_and(|st| st.blocked) {
                return Flow::done();
            }
            // The card's instructions were replaced (Enlightenment, Enchantress): follow
            // the replacement, unchanged.
            if let Some(plus) = play.and_then(|st| st.replaced) {
                plus_actions(g, p, plus.actions as u32);
                plus_buys(g, p, plus.buys as u32);
                g.add_coins(p, plus.coins as u32);
                f.pc = 3;
                return Flow::Call(Frame::draw(p, plus.cards as usize));
            }
            let behavior = play.map_or(g.card(id), |st| st.behavior);
            // The card's instructions are followed, so as often as they would be (Reckless):
            // `f.s.i` runs are left, this one included.
            if f.s.i == 0 {
                f.s.i = 1 + super::extra_runs(g, p, id) as usize;
            }
            if f.s.card.replace(behavior).is_none()
                && let Some(first) = super::prelude(behavior, p, id)
            {
                return Flow::Call(first);
            }
            f.s.n = g.next_lasting as i32;
            let def = behavior.def();
            let mut bonus = super::Bonus {
                plus: def.plus,
                coins: def.coins as u32,
                potions: def.potions as u32,
            };
            super::modify_bonus(g, p, id, &mut bonus);
            let plus = bonus.plus;
            plus_actions(g, p, plus.actions as u32);
            plus_buys(g, p, plus.buys as u32);
            let pl = &mut g.players[p.index()];
            pl.villagers += plus.villagers as u32;
            pl.coffers += plus.coffers as u32;
            pl.favors += plus.favors as u32;
            if p == g.turn.player {
                g.turn.potions += bonus.potions;
            }
            g.add_coins(p, plus.cards as u32);
            f.pc = 1;
            Flow::Call(Frame::draw(p, plus.coins as usize + bonus.coins as usize))
        }
        1 => {
            g.ext.menagerie.chameleon.push(id);
            let behavior = f.s.card.expect("set in step 0");
            let mut child = CardFrame::new(behavior, Hook::Main, p, Some(id));
            child.targets = f.targets.clone();
            child.chain = f.chain.clone();
            f.pc = 2;
            Flow::Call(Frame::Card(Box::new(child)))
        }
        3 => Flow::done(),
        _ => {
            let st = &mut g.ext.menagerie;
            if let Some(i) = st.chameleon.iter().rposition(|&c| c == id) {
                st.chameleon.remove(i);
            }
            let behavior = f.s.card.expect("set in step 0");
            let from = f.s.n as u32;
            let lastings = &g.lasting;
            st.chameleon_lastings
                .retain(|l| lastings.iter().any(|x| x.id == *l));
            st.chameleon_lastings.extend(
                lastings
                    .iter()
                    .filter(|l| {
                        l.id >= from
                            && l.card == behavior
                            && l.source == Some(id)
                            && l.until == Until::EndOfTurn
                    })
                    .map(|l| l.id),
            );
            f.s.i -= 1;
            if f.s.i > 0 {
                f.s.card = None;
                f.pc = 0;
                return Flow::Continue;
            }
            Flow::done()
        }
    }
}

fn way_of_the_rat(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let treasures = g.cards_in_hand_where(p, Types::TREASURE);
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
            Flow::Call(Frame::discard(p, chosen))
        }
        2 => {
            f.pc = 3;
            match supply_pile_topped_by(g, g.card(f.this())) {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::done(),
            }
        }
        _ => Flow::done(),
    }
}
