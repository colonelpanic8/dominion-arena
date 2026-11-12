//! Renaissance: its Kingdom cards, Artifacts and Projects.
//!
//! Artifacts are in [`Game::artifacts`] with their holder; a Project's owners are the
//! players with it in [`crate::Player::projects`]. Start-of-turn Projects and Artifacts
//! resolve as `StartTurn` triggers, ordered with the turn's Durations. "Once per turn"
//! abilities that have been used (Innovation, Citadel, the Horn) leave an `EndOfTurn`
//! lasting named after them. Cargo Ship's "once this turn" and Improve's Clean-up ability
//! wait as `EndOfTurn` lastings of their own, one per play; Priest's and Inventor's
//! effects are `EndOfTurn` lastings too.

use super::{CardFrame, Hook, SetRules, due_durations, duration_trigger, mode_mask, take_mode};
use crate::decision::Purpose;
use crate::event::{Dest, Event};
use crate::frame::{Flow, Frame, Input};
use crate::game::{Pos, Scratch, Until, Zone};
use crate::setup::{self, Setup};
use crate::trigger::{Occasion, Trigger};
use crate::{Card, CardId, Cost, Game, Kind, Opt, PlayerId, Set, Types};
use crate::Note;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Coin tokens each player has on Sinister Plot.
    pub(crate) plot: Vec<u32>,
    /// Cards Sewers is trashing, which don't set Sewers off again.
    sewers: Vec<CardId>,
    /// Experiments gained by another Experiment, which don't come with another.
    experiments: Vec<CardId>,
}

pub(crate) struct Rules;

/// A Duration's start-of-turn effect.
const START: u8 = 0;
/// A Project's or Artifact's ability.
const EFFECT: u8 = 0;
/// "When you gain this".
const GAINED: u8 = 1;
/// "When you trash this".
const TRASHED: u8 = 2;
const CARGO_SET_ASIDE: u8 = 3;
const IMPROVE_TRASH: u8 = 3;
const PRIEST_COINS: u8 = 3;

/// Each Artifact and the Kingdom card that brings it into the game.
const ARTIFACTS: [(Card, Card); 5] = [
    (Card::Lantern, Card::BorderGuard),
    (Card::Horn, Card::BorderGuard),
    (Card::Flag, Card::FlagBearer),
    (Card::TreasureChest, Card::Swashbuckler),
    (Card::Key, Card::Treasurer),
];

impl SetRules for Rules {
    fn notes(g: &Game, out: &mut Vec<Note>) {
        for (i, &n) in g.ext.renaissance.plot.iter().enumerate() {
            let p = PlayerId(i as u8);
            if g.player(p).projects.contains(&Card::SinisterPlot) {
                out.push(Note::new(Card::SinisterPlot).player(p).count(n as i32));
            }
        }
        // The game has ended; the round after it is under way, with these players to go.
        if g.ending.is_some() && g.outcome().is_none() {
            out.push(Note::new(Card::Fleet));
            for &p in &g.final_round {
                out.push(Note::new(Card::Fleet).player(p));
            }
        }
    }

    fn run(g: &mut Game, f: &mut CardFrame, input: Input) -> Option<Flow> {
        Some(match (f.card, f.hook) {
            (Card::CargoShip | Card::Research, Hook::Ability(START)) => start(g, f),
            (Card::CargoShip, Hook::Ability(CARGO_SET_ASIDE)) => cargo_set_aside(g, f),
            (Card::Improve, Hook::Ability(IMPROVE_TRASH)) => improve_trash(g, f, input),
            (Card::Priest, Hook::Ability(PRIEST_COINS)) => {
                g.add_coins(f.player, 2);
                Flow::done()
            }
            (_, Hook::Ability(GAINED)) => gained(g, f, input),
            (_, Hook::Ability(TRASHED)) => {
                when_trashed_or_gained(g, f.card, f.player);
                Flow::done()
            }
            (c, Hook::Ability(EFFECT)) if c.kind() == Kind::Project => project(g, f, input),
            (c, Hook::Ability(EFFECT)) if c.kind() == Kind::Artifact => artifact(g, f),
            (_, Hook::Ability(_)) => return None,
            (Card::ActingTroupe, _) => {
                let this = f.this();
                if f.pc == 0 && g.zone(this) == Zone::InPlay(f.player) {
                    f.pc = 1;
                    return Some(Flow::Call(Frame::trash(f.player, vec![this])));
                }
                Flow::done()
            }
            (Card::BorderGuard, _) => border_guard(g, f, input),
            (Card::CargoShip, _) => {
                let data = Scratch {
                    cards: f.keeps(g),
                    ..Scratch::default()
                };
                g.add_lasting(
                    Card::CargoShip,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    data,
                );
                Flow::done()
            }
            (Card::Experiment, _) => {
                let this = f.this();
                if g.zone(this) == Zone::InPlay(f.player) {
                    g.return_to_pile(this);
                }
                Flow::done()
            }
            (Card::Hideout, _) => hideout(g, f, input),
            (Card::Improve, _) => {
                g.add_lasting(
                    Card::Improve,
                    f.player,
                    f.this,
                    vec![],
                    Until::EndOfTurn,
                    Scratch::default(),
                );
                Flow::done()
            }
            (Card::Inventor, _) => inventor(g, f, input),
            (Card::MountainVillage, _) => mountain_village(g, f, input),
            (Card::OldWitch, _) => old_witch(g, f, input),
            (Card::Priest, _) => priest(g, f, input),
            (Card::Recruiter, _) => recruiter(g, f, input),
            (Card::Research, _) => research(g, f, input),
            (Card::Scepter, _) => scepter(g, f, input),
            (Card::Scholar, _) => match f.pc {
                0 => {
                    f.pc = 1;
                    Flow::Call(Frame::discard(f.player, g.hand(f.player).to_vec()))
                }
                1 => {
                    f.pc = 2;
                    Flow::Call(Frame::draw(f.player, 7))
                }
                _ => Flow::done(),
            },
            (Card::Sculptor, _) => sculptor(g, f, input),
            (Card::Seer, _) => seer(g, f, input),
            (Card::Swashbuckler, _) => {
                let p = f.player;
                if !g.player(p).discard.is_empty() {
                    g.players[p.index()].coffers += 1;
                    if g.player(p).coffers >= 4 {
                        take_artifact(g, p, Card::TreasureChest);
                    }
                }
                Flow::done()
            }
            (Card::Treasurer, _) => treasurer(g, f, input),
            (Card::Villain, _) => villain(g, f, input),
            _ => return None,
        })
    }

    fn triggers(g: &Game, occ: &Occasion, out: &mut Vec<Trigger>) {
        for l in due_durations(g, occ) {
            if l.card.set() == Set::Renaissance {
                out.push(duration_trigger(l, START));
            }
        }
        match *occ {
            Occasion::StartTurn { player } => start_of_turn(g, player, out),
            Occasion::StartBuyPhase { player } => {
                if holds(g, player, Card::TreasureChest) {
                    out.push(Trigger::new(player, Card::TreasureChest, EFFECT));
                }
            }
            Occasion::EndBuyPhase { player } => {
                if has(g, player, Card::Exploration) && g.turn.gained_this_buy_phase().is_empty() {
                    out.push(Trigger::new(player, Card::Exploration, EFFECT).quiet());
                }
                if has(g, player, Card::Pageant) && g.turn.coins >= 1 {
                    out.push(Trigger::new(player, Card::Pageant, EFFECT).optional("pay $1 for +1 Coffers"));
                }
            }
            Occasion::StartCleanup { .. } => {
                // Whoever played it, on whoever's turn.
                for l in g.lasting() {
                    if l.card == Card::Improve
                        && l.until == Until::EndOfTurn
                        && !improvable(g, l.owner).is_empty()
                    {
                        out.push(
                            Trigger::new(l.owner, Card::Improve, IMPROVE_TRASH)
                                .via_lasting(l.id)
                                .optional("trash an Action card you would discard from play, to gain a card costing exactly $1 more"),
                        );
                    }
                }
            }
            Occasion::DiscardFromPlay { player } => {
                if holds(g, player, Card::Flag) && g.cleaning_up(player) {
                    out.push(Trigger::new(player, Card::Flag, EFFECT));
                }
                if holds(g, player, Card::Horn) && !used(g, Card::Horn, player) {
                    for c in g.leaving_play(player) {
                        if g.card(c) == Card::BorderGuard {
                            out.push(Trigger::new(player, Card::Horn, EFFECT).on(c).optional("put Border Guard onto your deck"));
                        }
                    }
                }
            }
            Occasion::AfterPlay { player, card } => {
                if player == g.turn.player
                    && has(g, player, Card::Citadel)
                    && !used(g, Card::Citadel, player)
                    && first_action_play(g, player, card)
                {
                    out.push(Trigger::new(player, Card::Citadel, EFFECT).on(card));
                }
            }
            Occasion::Gain { player, card, .. } => gain_triggers(g, player, card, out),
            Occasion::Trash { player, card } => trash_triggers(g, player, card, out),
            Occasion::Shuffled { player, .. }
                if has(g, player, Card::StarChart) && g.shuffled(player).len() >= 2 =>
            {
                out.push(Trigger::new(player, Card::StarChart, EFFECT).optional("pick one of the shuffled cards to go on top"));
            }
            _ => {}
        }
    }

    fn modify_cost(g: &Game, card: Card, _id: Option<CardId>, cost: &mut Cost) {
        if card.kind() != Kind::Card {
            return;
        }
        let canal = g.turn_player().is_some_and(|p| has(g, p, Card::Canal)) as usize;
        let inventors = g
            .lasting()
            .iter()
            .filter(|l| l.card == Card::Inventor && l.until == Until::EndOfTurn)
            .count();
        *cost = cost.minus_coins((canal + inventors).min(u8::MAX as usize) as u8);
    }

    fn modify_types(g: &Game, id: CardId, types: &mut Types) {
        if types.contains(Types::ACTION)
            && g.turn_player().is_some_and(|p| has(g, p, Card::Capitalism))
            && g.card(id).text().contains("+$")
        {
            *types |= Types::TREASURE;
        }
    }

    fn can_buy(g: &Game, p: PlayerId, card: Card) -> bool {
        let projects = &g.player(p).projects;
        card.kind() != Kind::Project || (projects.len() < 2 && !projects.contains(&card))
    }

    fn ensure(g: &mut Game, _s: &mut Setup) {
        g.ext.renaissance.plot.resize(g.num_players(), 0);
        for (artifact, card) in ARTIFACTS {
            if setup::in_game(g, |c| c == card) && !g.artifacts.iter().any(|(a, _)| *a == artifact) {
                g.artifacts.push((artifact, None));
            }
        }
    }

    fn on_event(g: &mut Game, event: &crate::Event) {
        if !g.in_action_phase() {
            return;
        }
        let (player, n) = match event {
            Event::Revealed { player, card } => (*player, (*card == Card::Patron) as u32),
            Event::RevealedHand { player, cards } => (
                *player,
                cards.iter().filter(|&&c| c == Card::Patron).count() as u32,
            ),
            _ => return,
        };
        g.players[player.index()].coffers += n;
    }

    fn final_round(g: &Game, p: PlayerId) -> bool {
        has(g, p, Card::Fleet)
    }
}

// ---- Helpers ----

/// Whether `p` has a cube on `project`.
fn has(g: &Game, p: PlayerId, project: Card) -> bool {
    g.player(p).projects.contains(&project)
}

fn holds(g: &Game, p: PlayerId, artifact: Card) -> bool {
    g.artifact_holder(artifact) == Some(p)
}

fn take_artifact(g: &mut Game, p: PlayerId, artifact: Card) {
    if let Some(entry) = g.artifacts.iter_mut().find(|(a, _)| *a == artifact) {
        entry.1 = Some(p);
        g.emit(Event::Received {
            player: p,
            card: artifact,
        });
    }
}

fn villagers(g: &mut Game, p: PlayerId, n: u32) {
    g.players[p.index()].villagers += n;
}

fn coffers(g: &mut Game, p: PlayerId, n: u32) {
    g.players[p.index()].coffers += n;
}

/// `p`'s "this turn" lastings of `card`.
fn this_turn(g: &Game, card: Card, p: PlayerId) -> impl Iterator<Item = &crate::Lasting> {
    g.lasting()
        .iter()
        .filter(move |l| l.card == card && l.owner == p && l.until == Until::EndOfTurn)
}

/// Whether `p` used their once-per-turn `card` ability this turn.
fn used(g: &Game, card: Card, p: PlayerId) -> bool {
    this_turn(g, card, p).next().is_some()
}

/// Whether the play of `card` being resolved is the turn's first play of an Action card,
/// however often that card has been played again since.
fn first_action_play(g: &Game, p: PlayerId, card: CardId) -> bool {
    let Some(st) = g.plays.last().filter(|st| st.card == card) else {
        return false;
    };
    let log = g.log();
    let start = log
        .iter()
        .rposition(|e| matches!(e, Event::TurnStart { .. }))
        .unwrap_or(0);
    let earlier = log[start..st.since.max(start)]
        .iter()
        .filter(|e| matches!(e, Event::Played { player, .. } if *player == p))
        .count();
    let first = g.turn.plays_by(p).position(|(_, t)| t.contains(Types::ACTION));
    first == Some(earlier)
}

fn mark_used(g: &mut Game, card: Card, p: PlayerId) {
    g.add_lasting(card, p, None, vec![], Until::EndOfTurn, Scratch::default());
}

/// Whether a lasting effect keeps `c` in play.
fn kept(g: &Game, c: CardId) -> bool {
    g.lasting().iter().any(|l| l.keeps.contains(&c))
}

/// Whether `card`, being gained, is still where it was gained to.
fn where_gained(g: &Game, card: CardId) -> bool {
    g.still_where_gained(card)
}

fn hand_where(g: &Game, p: PlayerId, pred: impl Fn(CardId) -> bool) -> Vec<CardId> {
    g.hand(p).iter().copied().filter(|&c| pred(c)).collect()
}

fn coppers_in_hand(g: &Game, p: PlayerId) -> Vec<CardId> {
    hand_where(g, p, |c| g.card(c) == Card::Copper)
}

/// Action cards `p` would discard from play this turn.
fn improvable(g: &Game, p: PlayerId) -> Vec<CardId> {
    g.in_play(p)
        .iter()
        .copied()
        .filter(|&c| g.is(c, Types::ACTION) && !kept(g, c))
        .collect()
}

/// Puts `c` from `p`'s hand on top of their deck, unseen.
fn top_deck_hidden(g: &mut Game, c: CardId, p: PlayerId) {
    g.move_card(c, Zone::Deck(p), Pos::Top);
    g.emit(Event::Moved {
        player: p,
        card: None,
        to: Zone::Deck(p),
    });
}

/// At step `pc`: asks `p` to trash one of their hand (if any), moving on to `pc + 1`
/// with the choice in `f.s.pick`. Returns `None` once the trash is underway or skipped.
fn trash_from_hand(g: &mut Game, f: &mut CardFrame, input: Input, pc: u8) -> Option<Flow> {
    if f.pc == pc {
        let hand = g.hand(f.player).to_vec();
        if hand.is_empty() {
            f.pc = pc + 2;
            return Some(Flow::Continue);
        }
        f.pc = pc + 1;
        return Some(f.ask_cards(hand, 1, 1, Purpose::Trash));
    }
    if f.pc == pc + 1 {
        let c = input.card();
        f.s.pick = c;
        if let Some(c) = c {
            f.s.cost = g.cost_of(c);
            f.s.m = g.is(c, Types::VICTORY) as i32;
        }
        f.pc = pc + 2;
        return Some(Flow::Call(Frame::trash(f.player, c.into_iter().collect())));
    }
    None
}

// ---- Triggers ----

fn start_of_turn(g: &Game, p: PlayerId, out: &mut Vec<Trigger>) {
    if holds(g, p, Card::Key) {
        out.push(Trigger::new(p, Card::Key, EFFECT));
    }
    for &project in &g.player(p).projects {
        let t = Trigger::new(p, project, EFFECT);
        match project {
            Card::Silos if !coppers_in_hand(g, p).is_empty() => out.push(t),
            Card::CropRotation if !g.cards_in_hand_where(p, Types::VICTORY).is_empty() => {
                out.push(t.optional("discard a Victory card for +2 Cards"))
            }
            Card::Fair => out.push(t.quiet()),
            Card::Barracks | Card::Cathedral | Card::CityGate | Card::SinisterPlot | Card::Piazza => {
                out.push(t)
            }
            _ => {}
        }
    }
}

fn gain_triggers(g: &Game, gainer: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    let gained = Trigger::new(gainer, g.card(card), GAINED).on(card);
    match g.card(card) {
        Card::Ducat if !coppers_in_hand(g, gainer).is_empty() => out.push(gained.optional("trash a Copper from your hand")),
        Card::Experiment if !g.ext.renaissance.experiments.contains(&card) => out.push(gained),
        Card::Lackeys | Card::Spices | Card::SilkMerchant | Card::FlagBearer => out.push(gained),
        _ => {}
    }
    if has(g, gainer, Card::Academy) && g.is(card, Types::ACTION) {
        out.push(Trigger::new(gainer, Card::Academy, EFFECT));
    }
    if has(g, gainer, Card::Guildhall) && g.is(card, Types::TREASURE) {
        out.push(Trigger::new(gainer, Card::Guildhall, EFFECT));
    }
    if g.is(card, Types::VICTORY) {
        for p in g.others(gainer) {
            if has(g, p, Card::RoadNetwork) {
                out.push(Trigger::new(p, Card::RoadNetwork, EFFECT));
            }
        }
    }
    if !where_gained(g, card) {
        return;
    }
    if gainer == g.turn.player
        && has(g, gainer, Card::Innovation)
        && !used(g, Card::Innovation, gainer)
        && g.is(card, Types::ACTION)
    {
        out.push(Trigger::new(gainer, Card::Innovation, EFFECT).optional(format!("play {}", g.card(card))));
    }
    if !where_gained(g, card) {
        return;
    }
    for l in this_turn(g, Card::CargoShip, gainer) {
        let mut t = Trigger::new(gainer, Card::CargoShip, CARGO_SET_ASIDE)
            .via_lasting(l.id)
            .optional(format!("set {} aside on Cargo Ship", g.card(card)));
        // Which Cargo Ship stays in play: one played by a Throne Room keeps that too.
        let by: Vec<String> = l.data.cards.iter().skip(1).map(|&c| g.card(c).to_string()).collect();
        if !by.is_empty() {
            t = t.labelled(format!("Cargo Ship played by {}", by.join(", ")));
        }
        out.push(t);
    }
}

fn trash_triggers(g: &Game, trasher: PlayerId, card: CardId, out: &mut Vec<Trigger>) {
    if matches!(g.card(card), Card::FlagBearer | Card::SilkMerchant) {
        out.push(Trigger::new(trasher, g.card(card), TRASHED).on(card));
    }
    for l in this_turn(g, Card::Priest, trasher) {
        out.push(Trigger::new(trasher, Card::Priest, PRIEST_COINS).via_lasting(l.id));
    }
    if has(g, trasher, Card::Sewers)
        && !g.ext.renaissance.sewers.contains(&card)
        && !g.hand(trasher).is_empty()
    {
        out.push(Trigger::new(trasher, Card::Sewers, EFFECT).optional("trash a card from your hand"));
    }
}

// ---- Triggered abilities ----

fn start(g: &mut Game, f: &mut CardFrame) -> Flow {
    let Some(l) = f.end_lasting(g) else {
        return Flow::done();
    };
    let p = f.player;
    for c in l.data.pick.into_iter().chain(l.data.cards) {
        if g.zone(c) == Zone::SetAside(p) {
            g.put_in_hand(c, p);
        }
    }
    Flow::done()
}

fn cargo_set_aside(g: &mut Game, f: &mut CardFrame) -> Flow {
    let Some(Occasion::Gain { card, .. }) = f.occ else {
        return Flow::done();
    };
    if !where_gained(g, card) {
        return Flow::done();
    }
    let Some(l) = f.end_lasting(g) else {
        return Flow::done();
    };
    let p = f.player;
    g.set_aside(card, p);
    let data = Scratch {
        pick: Some(card),
        ..Scratch::default()
    };
    g.continue_lasting(&l, l.data.cards.clone(), Until::Removed, data);
    Flow::done()
}

fn improve_trash(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.end_lasting(g);
            let candidates = improvable(g, p);
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
            f.s.cost = g.cost_of(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        2 => {
            let target = f.s.cost.plus_coins(1);
            let piles = g.supply_piles_where(|_, cost| cost == target);
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

/// "When you gain this".
fn gained(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.card {
        Card::Ducat => match f.pc {
            0 => {
                let coppers = coppers_in_hand(g, p);
                if coppers.is_empty() {
                    return Flow::done();
                }
                f.pc = 1;
                f.ask_cards(coppers, 1, 1, Purpose::Trash)
            }
            1 => {
                f.pc = 2;
                Flow::Call(Frame::trash(p, input.cards()))
            }
            _ => Flow::done(),
        },
        Card::Experiment => match f.pc {
            0 => {
                let Some(pile) = g.supply_pile(Card::Experiment) else {
                    return Flow::done();
                };
                let Some(top) = g.pile_top(pile) else {
                    return Flow::done();
                };
                g.ext.renaissance.experiments.push(top);
                f.s.pick = Some(top);
                f.pc = 1;
                Flow::Call(Frame::gain(p, pile, Dest::Discard))
            }
            _ => {
                let top = f.s.pick;
                g.ext.renaissance.experiments.retain(|&c| Some(c) != top);
                Flow::done()
            }
        },
        Card::Lackeys => {
            villagers(g, p, 2);
            Flow::done()
        }
        Card::Spices => {
            coffers(g, p, 2);
            Flow::done()
        }
        card => {
            when_trashed_or_gained(g, card, p);
            Flow::done()
        }
    }
}

/// Flag Bearer's and Silk Merchant's "when you gain or trash this".
fn when_trashed_or_gained(g: &mut Game, card: Card, p: PlayerId) {
    match card {
        Card::FlagBearer => take_artifact(g, p, Card::Flag),
        Card::SilkMerchant => {
            coffers(g, p, 1);
            villagers(g, p, 1);
        }
        _ => {}
    }
}

fn artifact(g: &mut Game, f: &mut CardFrame) -> Flow {
    let p = f.player;
    match f.card {
        Card::Flag => g.turn.cleanup_draw += 1,
        Card::Key => g.add_coins(p, 1),
        Card::TreasureChest => {
            if f.pc == 0
                && let Some(gold) = g.supply_pile(Card::Gold)
            {
                f.pc = 1;
                return Flow::Call(Frame::gain(p, gold, Dest::Discard));
            }
        }
        Card::Horn => {
            mark_used(g, Card::Horn, p);
            let c = f.this();
            if g.zone(c) == Zone::InPlay(p) {
                g.top_deck(c, p);
            }
        }
        _ => {}
    }
    Flow::done()
}

fn project(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.card {
        Card::Academy => villagers(g, p, 1),
        Card::Barracks => g.turn.actions += 1,
        Card::Fair => g.turn.buys += 1,
        Card::Guildhall => coffers(g, p, 1),
        Card::Exploration => {
            coffers(g, p, 1);
            villagers(g, p, 1);
        }
        Card::Pageant => {
            if g.turn.coins >= 1 {
                g.turn.coins -= 1;
                coffers(g, p, 1);
            }
        }
        Card::RoadNetwork if f.pc == 0 => {
            f.pc = 1;
            return Flow::Call(Frame::draw(p, 1));
        }
        Card::Cathedral => {
            if let Some(flow) = trash_from_hand(g, f, input, 0) {
                return flow;
            }
        }
        Card::CityGate => return city_gate(g, f, input),
        Card::Silos => return silos(g, f, input),
        Card::SinisterPlot => return sinister_plot(g, f, input),
        Card::Piazza => return piazza(g, f, input),
        Card::CropRotation => return crop_rotation(g, f, input),
        Card::Sewers => return sewers(g, f, input),
        Card::StarChart => return star_chart(g, f, input),
        Card::Innovation => {
            let Some(Occasion::Gain { card, .. }) = f.occ else {
                return Flow::done();
            };
            if f.pc == 0 && where_gained(g, card) {
                mark_used(g, Card::Innovation, p);
                f.pc = 1;
                return Flow::Call(Frame::play(p, card, true, vec![]));
            }
        }
        Card::Citadel if f.pc == 0 => {
            mark_used(g, Card::Citadel, p);
            f.pc = 1;
            return Flow::Call(Frame::play(p, f.this(), false, vec![]));
        }
        _ => {}
    }
    Flow::done()
}

fn city_gate(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::draw(p, 1))
        }
        1 => {
            let hand = g.hand(p).to_vec();
            if hand.is_empty() {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(hand, 1, 1, Purpose::TopDeck)
        }
        _ => {
            if let Some(c) = input.card() {
                top_deck_hidden(g, c, p);
            }
            Flow::done()
        }
    }
}

fn silos(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let coppers = coppers_in_hand(g, p);
            let n = coppers.len();
            f.pc = 1;
            f.ask_cards(coppers, 0, n, Purpose::Discard)
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
            Flow::Call(Frame::draw(p, f.s.n as usize))
        }
        _ => Flow::done(),
    }
}

fn sinister_plot(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let n = g.ext.renaissance.plot[p.index()];
            let remove = format!("Remove your {n} tokens for +{n} Cards");
            f.pc = 1;
            f.ask_either(&["Add a token", &remove])
        }
        1 => {
            let tokens = &mut g.ext.renaissance.plot[p.index()];
            if input.opt() == Some(Opt::Mode(1)) {
                let n = std::mem::take(tokens);
                f.pc = 2;
                return Flow::Call(Frame::draw(p, n as usize));
            }
            *tokens += 1;
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn piazza(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            if !g.is(c, Types::ACTION) || g.zone(c) != Zone::Deck(p) {
                return Flow::done();
            }
            f.pc = 2;
            Flow::Call(Frame::play(p, c, true, vec![]))
        }
        _ => Flow::done(),
    }
}

fn crop_rotation(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let victory = g.cards_in_hand_where(p, Types::VICTORY);
            if victory.is_empty() {
                return Flow::done();
            }
            f.pc = 1;
            f.ask_cards(victory, 1, 1, Purpose::Discard)
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
            Flow::Call(Frame::draw(p, 2))
        }
        _ => Flow::done(),
    }
}

fn sewers(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
            g.ext.renaissance.sewers.push(c);
            f.s.pick = Some(c);
            f.pc = 2;
            Flow::Call(Frame::trash(p, vec![c]))
        }
        _ => {
            let c = f.s.pick;
            g.ext.renaissance.sewers.retain(|&x| Some(x) != c);
            Flow::done()
        }
    }
}

/// Puts the chosen one of the cards just shuffled on top of them (under any cards still in
/// the deck).
fn star_chart(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let mut shuffled = g.shuffled(p).to_vec();
            shuffled.sort();
            f.pc = 1;
            f.ask_cards(shuffled, 1, 1, Purpose::TopDeck)
        }
        _ => {
            let count = g.shuffled(p).len();
            let deck = &mut g.players[p.index()].deck;
            if let Some(c) = input.card()
                && let Some(i) = deck[..count].iter().position(|&x| x == c)
            {
                deck.remove(i);
                deck.insert(count - 1, c);
            }
            Flow::done()
        }
    }
}

// ---- Kingdom cards ----

fn border_guard(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let n = if holds(g, p, Card::Lantern) { 3 } else { 2 };
            f.s.n = n;
            f.pc = 1;
            Flow::Call(Frame::reveal(p, n as usize))
        }
        1 => {
            let cards = input.cards();
            if cards.is_empty() {
                return Flow::done();
            }
            f.s.m = (cards.len() == f.s.n as usize && cards.iter().all(|&c| g.is(c, Types::ACTION)))
                as i32;
            f.s.cards = cards.clone();
            f.pc = 2;
            f.ask_cards(cards, 1, 1, Purpose::Move)
                .question("put into your hand")
        }
        2 => {
            let chosen = input.cards();
            for &c in &chosen {
                if g.zone(c) == Zone::Deck(p) {
                    g.put_in_hand(c, p);
                }
            }
            let rest: Vec<CardId> = f
                .s
                .cards
                .iter()
                .copied()
                .filter(|c| !chosen.contains(c) && g.zone(*c) == Zone::Deck(p))
                .collect();
            f.pc = 3;
            Flow::Call(Frame::discard(p, rest))
        }
        3 => {
            if f.s.m == 0 {
                return Flow::done();
            }
            f.pc = 4;
            f.ask_either(&["Take the Lantern", "Take the Horn"])
        }
        _ => {
            let artifact = match input.opt() {
                Some(Opt::Mode(1)) => Card::Horn,
                _ => Card::Lantern,
            };
            take_artifact(g, p, artifact);
            Flow::done()
        }
    }
}

fn hideout(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if let Some(flow) = trash_from_hand(g, f, input, 0) {
        return flow;
    }
    if f.pc == 2 && f.s.m == 1 {
        f.pc = 3;
        if let Some(curse) = g.supply_pile(Card::Curse) {
            return Flow::Call(Frame::gain(f.player, curse, Dest::Discard));
        }
    }
    Flow::done()
}

fn inventor(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            let piles = g.piles_costing_up_to(Cost::coins(4));
            f.pc = 1;
            if piles.is_empty() {
                return Flow::Continue;
            }
            f.ask_pile(piles, false, Purpose::Gain)
        }
        1 => {
            f.pc = 2;
            match input.pile() {
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Discard)),
                None => Flow::Continue,
            }
        }
        _ => {
            g.add_lasting(
                Card::Inventor,
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

fn mountain_village(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            g.look_through_discard(p);
            let discard = g.player(p).discard.clone();
            if discard.is_empty() {
                f.pc = 2;
                return Flow::Call(Frame::draw(p, 1));
            }
            f.pc = 1;
            f.ask_cards(discard, 1, 1, Purpose::Move)
                .question("put into your hand")
        }
        1 => {
            if let Some(c) = input.card()
                && g.zone(c) == Zone::Discard(p)
            {
                g.put_in_hand(c, p);
            }
            Flow::done()
        }
        _ => Flow::done(),
    }
}

fn old_witch(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    loop {
        let Some(&target) = f.targets.get(f.s.i) else {
            return Flow::done();
        };
        match f.pc {
            0 => {
                f.pc = 1;
                if let Some(curse) = g.supply_pile(Card::Curse) {
                    return Flow::Call(Frame::gain(target, curse, Dest::Discard));
                }
            }
            1 => {
                let curses = hand_where(g, target, |c| g.card(c) == Card::Curse);
                if curses.is_empty() {
                    f.pc = 3;
                    continue;
                }
                f.pc = 2;
                return f.ask_cards_for(target, curses, 0, 1, Purpose::Trash);
            }
            2 => {
                f.pc = 3;
                return Flow::Call(Frame::trash(target, input.cards()));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
            }
        }
    }
}

fn priest(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if let Some(flow) = trash_from_hand(g, f, input, 0) {
        return flow;
    }
    g.add_lasting(
        Card::Priest,
        f.player,
        f.this,
        vec![],
        Until::EndOfTurn,
        Scratch::default(),
    );
    Flow::done()
}

fn recruiter(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if let Some(flow) = trash_from_hand(g, f, input, 0) {
        return flow;
    }
    if f.s.pick.is_some() {
        villagers(g, f.player, f.s.cost.coins as u32);
    }
    Flow::done()
}

fn research(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    if f.pc < 2 {
        return trash_from_hand(g, f, input, 0).unwrap_or(Flow::Continue);
    }
    let p = f.player;
    match f.pc {
        2 => {
            let n = f.s.cost.coins as usize;
            if f.s.pick.is_none() || n == 0 {
                return Flow::done();
            }
            f.pc = 3;
            Flow::Call(Frame::look(p, n))
        }
        _ => {
            let cards = input.cards();
            if cards.is_empty() {
                return Flow::done();
            }
            for &c in &cards {
                g.move_card(c, Zone::SetAside(p), Pos::Top);
                g.emit(Event::Moved {
                    player: p,
                    card: None,
                    to: Zone::SetAside(p),
                });
            }
            let data = Scratch {
                cards,
                ..Scratch::default()
            };
            f.add_duration(g, data);
            Flow::done()
        }
    }
}

fn scepter(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(&["+$2", "Replay an Action card you played this turn"], 1, 1)
        }
        1 => {
            let m = mode_mask(input);
            if m & 1 != 0 {
                g.add_coins(p, 2);
            }
            if m & 2 == 0 {
                return Flow::done();
            }
            let mut candidates: Vec<CardId> = vec![];
            let played = g.turn.played_ids.iter().zip(&g.turn.stayed_in_play);
            for (&c, _) in played.filter(|(_, stayed)| **stayed) {
                let ok = g.zone(c) == Zone::InPlay(p)
                    && g.is(c, Types::ACTION)
                    && !g.is(c, Types::COMMAND)
                    && !candidates.contains(&c);
                if ok {
                    candidates.push(c);
                }
            }
            if candidates.is_empty() || p != g.turn.player {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_cards(candidates, 1, 1, Purpose::Play)
        }
        2 => {
            let Some(c) = input.card() else {
                return Flow::done();
            };
            f.pc = 3;
            Flow::Call(Frame::play(p, c, false, f.chain_for_child()))
        }
        _ => Flow::done(),
    }
}

fn sculptor(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
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
                Some(pile) => Flow::Call(Frame::gain(p, pile, Dest::Hand)),
                None => Flow::done(),
            }
        }
        _ => {
            if input.card().is_some_and(|c| g.is(c, Types::TREASURE)) {
                villagers(g, p, 1);
            }
            Flow::done()
        }
    }
}

fn seer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            Flow::Call(Frame::reveal(p, 3))
        }
        1 => {
            let mut rest = vec![];
            for c in input.cards() {
                let cost = g.cost_of(c);
                let cheap = (2..=4).contains(&cost.coins) && cost.potions == 0 && cost.debt == 0;
                if g.zone(c) != Zone::Deck(p) {
                    continue;
                }
                if cheap {
                    g.put_in_hand(c, p);
                } else {
                    rest.push(c);
                }
            }
            if rest.len() < 2 {
                return Flow::done();
            }
            f.pc = 2;
            f.ask_order(rest, Purpose::TopDeck)
        }
        _ => {
            for &c in input.cards().iter().rev() {
                g.move_card(c, Zone::Deck(p), Pos::Top);
            }
            Flow::done()
        }
    }
}

fn treasurer(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    let p = f.player;
    match f.pc {
        0 => {
            f.pc = 1;
            f.ask_modes(
                &[
                    "Trash a Treasure from your hand",
                    "Gain a Treasure from the trash to your hand",
                    "Take the Key",
                ],
                1,
                1,
            )
        }
        1 => {
            f.s.m = mode_mask(input);
            f.pc = 2;
            Flow::Continue
        }
        2 => match take_mode(f) {
            Some(0) => {
                let treasures = g.cards_in_hand_where(p, Types::TREASURE);
                if treasures.is_empty() {
                    return Flow::Continue;
                }
                f.pc = 3;
                f.ask_cards(treasures, 1, 1, Purpose::Trash)
            }
            Some(1) => {
                let treasures: Vec<CardId> = g
                    .trash()
                    .iter()
                    .copied()
                    .filter(|&c| g.is(c, Types::TREASURE))
                    .collect();
                if treasures.is_empty() {
                    return Flow::Continue;
                }
                f.pc = 4;
                f.ask_cards(treasures, 1, 1, Purpose::Gain)
            }
            Some(_) => {
                take_artifact(g, p, Card::Key);
                Flow::Continue
            }
            None => Flow::done(),
        },
        3 => {
            f.pc = 2;
            Flow::Call(Frame::trash(p, input.cards()))
        }
        _ => {
            f.pc = 2;
            match input.card() {
                Some(c) => Flow::Call(Frame::gain_card(p, c, Dest::Hand)),
                None => Flow::Continue,
            }
        }
    }
}

fn villain(g: &mut Game, f: &mut CardFrame, input: Input) -> Flow {
    loop {
        let Some(&target) = f.targets.get(f.s.i) else {
            return Flow::done();
        };
        match f.pc {
            0 => {
                if g.hand(target).len() < 5 {
                    f.s.i += 1;
                    continue;
                }
                let candidates = hand_where(g, target, |c| g.cost_of(c).coins >= 2);
                if candidates.is_empty() {
                    let cards = g.hand(target).iter().map(|&c| g.card(c)).collect();
                    g.emit(Event::RevealedHand {
                        player: target,
                        cards,
                    });
                    f.s.i += 1;
                    continue;
                }
                f.pc = 1;
                return f.ask_cards_for(target, candidates, 1, 1, Purpose::Discard);
            }
            1 => {
                f.pc = 2;
                return Flow::Call(Frame::discard(target, input.cards()));
            }
            _ => {
                f.s.i += 1;
                f.pc = 0;
            }
        }
    }
}
