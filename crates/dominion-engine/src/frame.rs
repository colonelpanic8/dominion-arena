//! The effect stack.
//!
//! Everything that can be interrupted by a decision or a triggered ability runs as a
//! [`Frame`]: a small hand-written coroutine whose `pc` records where to resume. A frame
//! returns a [`Flow`] to ask a player something, call a child frame, or finish with a
//! [`Value`] that is handed to its parent as [`Input::Return`]. Because frames are plain
//! data, a game can be cloned or serialized mid-effect.

use crate::cards::{self, CardFrame, Hook};
use crate::decision::{Decision, DecisionKind, Move, Opt, Purpose, Response};
use crate::event::{Dest, Event};
use crate::game::{
    EndReason, ExtraLimit, ExtraTurn, GainState, Phase, PlayState, Pos, Turn, Until, Zone,
};
use crate::trigger::{Occasion, Window};
use crate::{Card, CardId, Game, Kind, PileId, PlayerId, Types};
use serde::{Deserialize, Serialize};

pub(crate) enum Input {
    Start,
    Return(Value),
    Response(Response),
}

impl Input {
    /// Cards chosen in a response, or returned by a child frame.
    pub(crate) fn cards(self) -> Vec<CardId> {
        match self {
            Input::Response(Response::Cards(v)) | Input::Return(Value::Cards(v)) => v,
            _ => vec![],
        }
    }

    pub(crate) fn card(self) -> Option<CardId> {
        self.cards().first().copied()
    }

    pub(crate) fn pile(self) -> Option<PileId> {
        match self {
            Input::Response(Response::Pile(p)) => p,
            _ => None,
        }
    }

    pub(crate) fn options(self) -> Vec<Opt> {
        match self {
            Input::Response(Response::Options(v)) => v,
            _ => vec![],
        }
    }

    pub(crate) fn opt(self) -> Option<Opt> {
        self.options().first().copied()
    }

    pub(crate) fn is_yes(&self) -> bool {
        match self {
            Input::Response(Response::Options(v)) => v.first() == Some(&Opt::Yes),
            Input::Return(Value::Bool(b)) => *b,
            _ => false,
        }
    }

    pub(crate) fn number(&self) -> Option<i32> {
        match self {
            Input::Response(Response::Number(n)) => Some(*n),
            _ => None,
        }
    }

    pub(crate) fn name(&self) -> Option<Card> {
        match self {
            Input::Response(Response::Name(c)) => Some(*c),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) enum Value {
    #[default]
    None,
    Cards(Vec<CardId>),
    Bool(bool),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Flow {
    Done(Value),
    /// Resume this frame again immediately with [`Input::Start`].
    Continue,
    Call(Frame),
    Ask(Decision),
}

impl Flow {
    pub(crate) fn done() -> Flow {
        Flow::Done(Value::None)
    }

    /// Marks the decision asked as being about `cards` (see [`Decision::about`]).
    pub(crate) fn about(mut self, cards: Vec<CardId>) -> Flow {
        if let Flow::Ask(d) = &mut self {
            d.about = cards;
        }
        self
    }

    /// Says in words what the decision asked asks (see [`Decision::question`]).
    pub(crate) fn question(mut self, text: impl Into<String>) -> Flow {
        if let Flow::Ask(d) = &mut self {
            d.question = Some(text.into());
        }
        self
    }
}

/// Where a gain takes its card from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum From {
    Pile(PileId),
    Card(CardId),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Frame {
    Game {
        next: PlayerId,
        pc: u8,
    },
    Turn {
        player: PlayerId,
        pc: u8,
    },
    Play(PlayFrame),
    Card(Box<CardFrame>),
    Window(Window),
    Draw {
        player: PlayerId,
        count: usize,
        /// Draw until the hand has this many cards instead of drawing `count`.
        #[serde(default)]
        to: Option<usize>,
        drawn: Vec<CardId>,
        started: bool,
        /// Whether it shuffled: once, before drawing, if the deck was short.
        shuffled: bool,
    },
    /// Shuffles a discard pile into (under) its deck, or a whole deck: the only way cards
    /// are shuffled.
    Shuffle {
        player: PlayerId,
        /// Shuffle the whole deck rather than the discard pile.
        deck: bool,
        count: usize,
        /// The shuffled cards put on the bottom (Shadow cards), for their owner to order.
        bottom: Vec<CardId>,
        pc: u8,
    },
    /// Takes the top cards of a deck into [`Zone::Looking`] (see [`Frame::look`]).
    TopCards {
        player: PlayerId,
        count: usize,
        reveal: bool,
        shuffled: bool,
    },
    Gain {
        player: PlayerId,
        from: From,
        to: Dest,
        bought: bool,
        /// For a bought card: which buy of the turn (see [`GainState::buy`]).
        #[serde(default)]
        buy: Option<usize>,
        card: Option<CardId>,
        /// Where `card` was when the gain began.
        #[serde(default)]
        at: Option<Zone>,
        pc: u8,
    },
    Buy {
        player: PlayerId,
        pile: Option<PileId>,
        landscape: Option<Card>,
        card: Option<CardId>,
        /// Which buy of the turn this is, once paid for: its position in
        /// `Turn::bought_cards`, from 1.
        #[serde(default)]
        index: usize,
        pc: u8,
    },
    Trash {
        player: PlayerId,
        cards: Vec<CardId>,
        i: usize,
    },
    Discard {
        player: PlayerId,
        cards: Vec<CardId>,
        cleanup: bool,
        i: usize,
    },
    /// Discards cards from play other than in Clean-up: "when you discard this from
    /// play", then an ordinary discard of those still there.
    DiscardFromPlay {
        player: PlayerId,
        cards: Vec<CardId>,
        pc: u8,
    },
    /// Offer to overpay for `card`, just bought: $, then Potions; then its overpay ability.
    Overpay {
        player: PlayerId,
        card: CardId,
        paid: crate::Cost,
        pc: u8,
    },
    /// Receive a Boon or Hex: resolve it, then discard it unless it was kept.
    Receive {
        player: PlayerId,
        card: Card,
        pc: u8,
    },
    /// "Return to your Action phase": ends `player`'s Buy phase, if it is on.
    ReturnToAction {
        player: PlayerId,
        pc: u8,
    },
}

impl Frame {
    pub(crate) fn draw(player: PlayerId, count: usize) -> Frame {
        Frame::Draw {
            player,
            count,
            to: None,
            drawn: vec![],
            started: false,
            shuffled: false,
        }
    }

    /// Draw until `player` has `size` cards in hand, or has no more cards to draw.
    pub(crate) fn draw_to(player: PlayerId, size: usize) -> Frame {
        Frame::Draw {
            player,
            count: 0,
            to: Some(size),
            drawn: vec![],
            started: false,
            shuffled: false,
        }
    }

    /// Reveals the top `count` cards of a deck to everyone, like [`Frame::look`].
    pub(crate) fn reveal(player: PlayerId, count: usize) -> Frame {
        Frame::TopCards {
            player,
            count,
            reveal: true,
            shuffled: false,
        }
    }

    /// Takes the top `count` cards of a deck (top first, shuffling first if it has fewer)
    /// for their owner to look at: they are in [`Zone::Looking`], off the deck, until the
    /// frame that called this finishes; then those it didn't move go back on top of the
    /// deck in their order, over anything put there meanwhile.
    pub(crate) fn look(player: PlayerId, count: usize) -> Frame {
        Frame::TopCards {
            player,
            count,
            reveal: false,
            shuffled: false,
        }
    }

    /// Shuffle `player`'s discard pile and put it under their deck.
    pub(crate) fn shuffle(player: PlayerId) -> Frame {
        Frame::Shuffle {
            player,
            deck: false,
            count: 0,
            bottom: vec![],
            pc: 0,
        }
    }

    /// Shuffle `player`'s deck.
    pub(crate) fn shuffle_deck(player: PlayerId) -> Frame {
        Frame::Shuffle {
            player,
            deck: true,
            count: 0,
            bottom: vec![],
            pc: 0,
        }
    }

    /// Gain the top card of `pile`.
    pub(crate) fn gain(player: PlayerId, pile: PileId, to: Dest) -> Frame {
        Frame::Gain {
            player,
            from: From::Pile(pile),
            to,
            bought: false,
            buy: None,
            card: None,
            at: None,
            pc: 0,
        }
    }

    /// Gain a specific card from wherever it is (the trash, a non-Supply pile, ...).
    pub(crate) fn gain_card(player: PlayerId, card: CardId, to: Dest) -> Frame {
        Frame::Gain {
            player,
            from: From::Card(card),
            to,
            bought: false,
            buy: None,
            card: None,
            at: None,
            pc: 0,
        }
    }

    pub(crate) fn trash(player: PlayerId, cards: Vec<CardId>) -> Frame {
        Frame::Trash {
            player,
            cards,
            i: 0,
        }
    }

    pub(crate) fn discard(player: PlayerId, cards: Vec<CardId>) -> Frame {
        Frame::Discard {
            player,
            cards,
            cleanup: false,
            i: 0,
        }
    }

    pub(crate) fn discard_from_play(player: PlayerId, cards: Vec<CardId>) -> Frame {
        Frame::DiscardFromPlay {
            player,
            cards,
            pc: 0,
        }
    }

    pub(crate) fn overpay(player: PlayerId, card: CardId) -> Frame {
        Frame::Overpay {
            player,
            card,
            paid: crate::Cost::ZERO,
            pc: 0,
        }
    }

    pub(crate) fn receive(player: PlayerId, card: Card) -> Frame {
        Frame::Receive {
            player,
            card,
            pc: 0,
        }
    }

    /// Return to the Action phase if it is `player`'s Buy phase, which ends first. The
    /// only way back there.
    pub(crate) fn return_to_action(player: PlayerId) -> Frame {
        Frame::ReturnToAction { player, pc: 0 }
    }

    /// Play `card`, moving it into play first if `move_to_play`. `chain` lists the cards
    /// that are playing it (Throne Room, ...), which stay in play as long as it does. A
    /// play from the hand that `SetRules::can_play` forbids does nothing.
    pub(crate) fn play(
        player: PlayerId,
        card: CardId,
        move_to_play: bool,
        chain: Vec<CardId>,
    ) -> Frame {
        Frame::Play(PlayFrame {
            player,
            card,
            move_to_play,
            chain,
            targets: vec![],
            remaining: vec![],
            again: 0,
            pc: 0,
        })
    }

    pub(crate) fn window(occ: Occasion) -> Frame {
        Window::open(occ)
    }

    pub(crate) fn resume(&mut self, g: &mut Game, input: Input) -> Flow {
        match self {
            Frame::Game { next, pc } => resume_game(g, next, pc, input),
            Frame::Turn { player, pc } => resume_turn(g, *player, pc, input),
            Frame::Play(f) => f.resume(g, input),
            Frame::Card(f) => cards::resume(g, f, input),
            Frame::Window(w) => w.resume(g, input),
            Frame::Draw {
                player,
                count,
                to,
                drawn,
                started,
                shuffled,
            } => {
                let (p, to) = (*player, *to);
                // Whether another card is to be drawn after `n` of `count`.
                let wants = |g: &Game, count: usize, n: usize| match to {
                    Some(size) => g.hand(p).len() < size,
                    None => n < count,
                };
                if !*started {
                    *started = true;
                    let mut token = 0;
                    if g.players[p.index()].minus_card && wants(g, *count, 0) {
                        g.players[p.index()].minus_card = false;
                        // The token replaces one draw of a fixed number; a draw up to a
                        // hand size still goes on to that size.
                        *count = count.saturating_sub(1);
                        token = 1;
                    }
                    // Short of cards, the discard pile is shuffled under the deck first,
                    // and then they are all drawn; the token counts as a card for that.
                    let needed = token
                        + match to {
                            Some(size) => size.saturating_sub(g.hand(p).len()),
                            None => *count,
                        };
                    let pl = &g.players[p.index()];
                    if pl.deck.len() < needed && !pl.discard.is_empty() {
                        *shuffled = true;
                        return Flow::Call(Frame::shuffle(p));
                    }
                }
                while wants(g, *count, drawn.len()) {
                    let Some(&top) = g.players[p.index()].deck.last() else {
                        break;
                    };
                    g.move_card(top, Zone::Hand(p), Pos::Top);
                    drawn.push(top);
                }
                if !drawn.is_empty() {
                    g.emit(Event::Drew {
                        player: p,
                        count: drawn.len(),
                    });
                }
                Flow::Done(Value::Cards(std::mem::take(drawn)))
            }
            Frame::Shuffle {
                player,
                deck,
                count,
                bottom,
                pc,
            } => resume_shuffle(g, *player, *deck, count, bottom, pc, input),
            Frame::TopCards {
                player,
                count,
                reveal,
                shuffled,
            } => {
                let p = *player;
                let pl = &g.players[p.index()];
                if pl.deck.len() < *count && !pl.discard.is_empty() && !*shuffled {
                    *shuffled = true;
                    return Flow::Call(Frame::shuffle(p));
                }
                let cards: Vec<CardId> = pl.deck.iter().rev().take(*count).copied().collect();
                if !*reveal && !cards.is_empty() {
                    g.emit(Event::Looked {
                        player: p,
                        count: cards.len(),
                    });
                }
                for &c in &cards {
                    // Revealed while still on the deck, so everyone sees which card it is.
                    if *reveal {
                        g.reveal(p, c);
                    }
                    // The look or reveal is its own log line.
                    g.move_unlogged(c, Zone::Looking(p));
                    if *reveal {
                        g.note_shown(c);
                        g.players[p.index()].face_up.push(c);
                    } else {
                        g.show_to(c, p);
                    }
                }
                if !cards.is_empty() {
                    g.holds.push(crate::game::Hold {
                        player: p,
                        cards: cards.clone(),
                        depth: g.stack.len(),
                    });
                }
                Flow::Done(Value::Cards(cards))
            }
            Frame::Gain {
                player,
                from,
                to,
                bought,
                buy,
                card,
                at,
                pc,
            } => resume_gain(g, *player, *from, *to, (*bought, *buy), (card, at), pc),
            Frame::Buy {
                player,
                pile,
                landscape,
                card,
                index,
                pc,
            } => resume_buy(g, *player, (*pile, *landscape), card, index, pc, input),
            Frame::Trash { player, cards, i } => {
                // Cards trashed together are all trashed before any "when trashed" ability,
                // and their abilities share one window.
                if *i > 0 {
                    for c in cards.iter() {
                        if let Some(pos) = g.trashing.iter().rposition(|(x, _)| x == c) {
                            g.trashing.remove(pos);
                        }
                    }
                    return Flow::Done(Value::Cards(std::mem::take(cards)));
                }
                *i = 1;
                for &c in cards.iter() {
                    let from = g.zone(c);
                    g.show(c);
                    g.move_card(c, Zone::Trash, Pos::Top);
                    g.trashing.push((c, from));
                    let card = g.card(c);
                    if *player == g.turn.player {
                        g.turn.trashed.push(card);
                    }
                    g.emit(Event::Trashed {
                        player: *player,
                        card,
                    });
                }
                let occs = cards
                    .iter()
                    .map(|&card| Occasion::Trash {
                        player: *player,
                        card,
                    })
                    .collect();
                Flow::Call(Window::open_all(occs))
            }
            Frame::Discard {
                player,
                cards,
                cleanup,
                i,
            } => {
                if *cleanup {
                    for &c in cards.iter() {
                        g.move_unlogged(c, Zone::Discard(*player));
                    }
                    return Flow::Done(Value::Cards(std::mem::take(cards)));
                }
                // Cards discarded together are all discarded before any reacts.
                if *i == 0 {
                    for &c in cards.iter() {
                        g.show(c);
                        g.move_card(c, Zone::Discard(*player), Pos::Top);
                        g.emit(Event::Discarded {
                            player: *player,
                            card: g.card(c),
                        });
                    }
                }
                let Some(&c) = cards.get(*i) else {
                    return Flow::Done(Value::Cards(std::mem::take(cards)));
                };
                *i += 1;
                Flow::Call(Frame::window(Occasion::Discard {
                    player: *player,
                    card: c,
                }))
            }
            Frame::DiscardFromPlay { player, cards, pc } => match pc {
                0 => {
                    *pc = 1;
                    g.leaving.push(cards.clone());
                    Flow::Call(Frame::window(Occasion::DiscardFromPlay { player: *player }))
                }
                1 => {
                    *pc = 2;
                    g.leaving.pop();
                    cards.retain(|&c| g.zone(c) == Zone::InPlay(*player));
                    Flow::Call(Frame::discard(*player, std::mem::take(cards)))
                }
                _ => Flow::Done(Value::Cards(input.cards())),
            },
            Frame::Overpay {
                player,
                card,
                paid,
                pc,
            } => resume_overpay(g, *player, *card, paid, pc, input),
            Frame::Receive { player, card, pc } => match pc {
                0 => {
                    *pc = 1;
                    g.emit(Event::Received {
                        player: *player,
                        card: *card,
                    });
                    g.begin_resolving(None);
                    Flow::Call(Frame::Card(Box::new(CardFrame::new(
                        *card,
                        Hook::Main,
                        *player,
                        None,
                    ))))
                }
                _ => {
                    g.end_resolving();
                    let kept = g.players[player.index()].boons_kept.contains(card);
                    if !kept {
                        match card.kind() {
                            Kind::Hex => g.hexes.discard.push(*card),
                            _ => g.boons.discard.push(*card),
                        }
                    }
                    Flow::done()
                }
            },
            Frame::ReturnToAction { player, pc } => {
                if *pc == 0 {
                    if *player != g.turn.player || g.turn.phase != Phase::Buy {
                        return Flow::done();
                    }
                    *pc = 1;
                    return Flow::Call(Frame::window(Occasion::EndBuyPhase { player: *player }));
                }
                let turn = &mut g.turn;
                turn.phase = Phase::Action;
                turn.bought = false;
                turn.earlier_buy_phase_gains = turn.gained_in_buy_phase.len();
                g.emit(Event::ReturnedToAction { player: *player });
                Flow::done()
            }
        }
    }
}

fn resume_shuffle(
    g: &mut Game,
    p: PlayerId,
    whole_deck: bool,
    count: &mut usize,
    bottom: &mut Vec<CardId>,
    pc: &mut u8,
    input: Input,
) -> Flow {
    match pc {
        0 => {
            *pc = 1;
            Flow::Call(Frame::window(Occasion::WouldShuffle { player: p }))
        }
        1 => {
            let pl = &mut g.players[p.index()];
            let mut cards = std::mem::take(if whole_deck {
                &mut pl.deck
            } else {
                &mut pl.discard
            });
            if cards.is_empty() {
                return Flow::done();
            }
            g.shuffle(&mut cards);
            let (mut under, rest): (Vec<CardId>, Vec<CardId>) = cards
                .iter()
                .partition(|&&c| cards::bottom_when_shuffled(g, p, c));
            *bottom = under.clone();
            under.extend(rest);
            for &c in &under {
                let from = g.zones[c.index()];
                g.zones[c.index()] = Zone::Deck(p);
                g.record(|| crate::Trace::Moved {
                    card: c,
                    from,
                    to: Zone::Deck(p),
                    top: false,
                });
            }
            let mixed = under.clone();
            g.mixed(mixed);
            *count = under.len();
            g.last_shuffle = under.clone();
            g.shuffle_placed.clear();
            let pl = &mut g.players[p.index()];
            under.append(&mut pl.deck);
            pl.deck = under;
            g.emit(Event::Shuffled { player: p });
            *pc = 3;
            if bottom.len() > 1 {
                *pc = 2;
                let n = bottom.len();
                return Flow::Ask(Decision {
                    player: p,
                    kind: DecisionKind::Cards {
                        candidates: bottom.iter().rev().copied().collect(),
                        min: n,
                        max: n,
                        ordered: true,
                    },
                    purpose: Purpose::Move,
                    source: None,
                    about: vec![],
                    question: Some("put on the bottom of your deck".into()),
                });
            }
            Flow::Continue
        }
        2 => {
            // Top first, so the last one is the bottom card of the deck.
            let order = input.cards();
            let deck = &mut g.players[p.index()].deck;
            for (i, c) in order.into_iter().rev().enumerate() {
                deck[i] = c;
            }
            *pc = 3;
            Flow::Continue
        }
        3 => {
            *pc = 4;
            Flow::Call(Frame::window(Occasion::Shuffled {
                player: p,
                count: *count,
            }))
        }
        _ => Flow::done(),
    }
}

/// Whether `extra` can't be the turn after the one that just ended.
fn fails(g: &Game, extra: &ExtraTurn) -> bool {
    match extra.limit {
        ExtraLimit::None => false,
        ExtraLimit::ThirdInARow => g.third_in_a_row(extra.player),
    }
}

fn resume_game(g: &mut Game, next: &mut PlayerId, pc: &mut u8, input: Input) -> Flow {
    match pc {
        0 | 2 => {
            let started = g.players.iter().any(|p| p.turns_taken > 0);
            let last = started.then_some(g.turn.player);
            let mut chosen = match input.opt() {
                Some(Opt::Card(source)) if *pc == 2 => Some(source),
                _ => None,
            };
            loop {
                // An extra turn that can't follow the turn that just ended is lost.
                let mut pending = std::mem::take(&mut g.extra_turns);
                pending.retain(|e| !fails(g, e));
                g.extra_turns = pending;
                // The player who just took a turn takes theirs first, then the others in
                // turn order; each chooses among their own.
                let Some(p) = g
                    .players_from(g.turn.player)
                    .into_iter()
                    .find(|&p| g.extra_turns.iter().any(|e| e.player == p))
                else {
                    break;
                };
                let mut sources: Vec<Card> = vec![];
                for e in g.extra_turns.iter().filter(|e| e.player == p) {
                    if !sources.contains(&e.source) {
                        sources.push(e.source);
                    }
                }
                let source = match chosen.take() {
                    Some(source) => source,
                    None if sources.len() > 1 => {
                        *pc = 2;
                        return Flow::Ask(Decision {
                            player: p,
                            kind: DecisionKind::Choose {
                                labels: sources
                                    .iter()
                                    .map(|s| format!("Take the extra turn from {s}"))
                                    .collect(),
                                options: sources.into_iter().map(Opt::Card).collect(),
                                min: 1,
                                max: 1,
                            },
                            purpose: Purpose::Either,
                            source: None,
                            about: vec![],
                            question: None,
                        });
                    }
                    None => sources[0],
                };
                let i = g
                    .extra_turns
                    .iter()
                    .position(|e| e.player == p && e.source == source)
                    .expect("the chosen extra turn is pending");
                g.extra_turns.remove(i);
                if g.skips_turn(p, true) {
                    // What kept its card in play for that turn (Voyage, Outpost) ends, so
                    // the card is discarded in the next Clean-up.
                    let until = Until::StartOfTurn(p);
                    g.lasting
                        .retain(|l| !(l.owner == p && l.card == source && l.until == until));
                    continue;
                }
                g.turn = Turn::new(p);
                g.turn.previous = last;
                g.turn.extra = Some(source);
                g.record_turn();
                *pc = 1;
                return Flow::Call(Frame::Turn { player: p, pc: 0 });
            }
            if let Some(reason) = g.ending {
                // The round after the game's end: a turn Lich skips is one of these too.
                let mut turn = g.final_round.pop_front();
                while let Some(p) = turn
                    && g.skips_turn(p, true)
                {
                    turn = g.final_round.pop_front();
                }
                let Some(p) = turn else {
                    g.finish(reason);
                    return Flow::done();
                };
                g.turn = Turn::new(p);
                g.turn.previous = last;
                // Like extra turns, these don't count for the tiebreaker.
                g.players[p.index()].extra_turns_taken += 1;
            } else {
                while g.skips_turn(*next, false) {
                    *next = g.left_of(*next);
                }
                g.turn = Turn::new(*next);
                g.turn.previous = last;
                *next = g.left_of(*next);
            }
            g.record_turn();
            *pc = 1;
            Flow::Call(Frame::Turn {
                player: g.turn.player,
                pc: 0,
            })
        }
        _ => {
            if g.ending.is_none()
                && let Some(reason) = g.game_over_reason()
            {
                g.ending = Some(reason);
                if reason != EndReason::TurnLimit {
                    let order = g.players_from(g.left_of(g.turn.player));
                    g.final_round = order
                        .into_iter()
                        .filter(|&p| cards::final_round(g, p))
                        .collect();
                }
            }
            if let Some(reason) = g.ending
                && g.final_round.is_empty()
            {
                g.finish(reason);
                return Flow::done();
            }
            *pc = 0;
            Flow::Continue
        }
    }
}

fn ask_moves(player: PlayerId, phase: Phase, moves: Vec<Move>) -> Flow {
    Flow::Ask(Decision {
        player,
        kind: DecisionKind::Turn { phase, moves },
        purpose: Purpose::Turn,
        source: None,
        about: vec![],
        question: None,
    })
}

/// Whether `p` may buy `card` (a card on top of a pile, an Event or a Project) now.
fn can_buy(g: &Game, p: PlayerId, card: Card, cost: crate::Cost) -> bool {
    let t = &g.turn;
    (t.buys > 0 || (t.actions > 0 && cards::actions_are_buys(g)))
        && g.players[p.index()].debt == 0
        && ((cost.coins as u32 <= t.coins && cost.potions as u32 <= t.potions)
            || cards::pays_otherwise(g, p, card))
        && cards::can_buy(g, p, card)
}

/// Adds spending Coffers and paying off Debt. Outside the Buy phase they are offered only
/// where they could matter: while an Action could be played (also after a Villager, or by
/// Market Towns at the start of the Buy phase), at Night while a Night card could, and
/// paying Debt at Night after what reads `$` at the end of the Buy phase (Wine Merchant).
fn push_token_moves(g: &Game, p: PlayerId, moves: &mut Vec<Move>) {
    let pl = &g.players[p.index()];
    let playable = match g.turn.phase {
        Phase::Action => !g.playable(p, Types::ACTION).is_empty(),
        Phase::Night => !g.playable(p, Types::NIGHT).is_empty(),
        _ => true,
    };
    if pl.coffers > 0 && playable {
        moves.push(Move::Coffers);
    }
    if pl.debt > 0 && g.turn.coins > 0 && (playable || g.turn.phase == Phase::Night) {
        moves.push(Move::PayDebt);
    }
}

fn spend_coffers(g: &mut Game, p: PlayerId) {
    g.players[p.index()].coffers -= 1;
    g.add_coins(p, 1);
}

fn pay_debt(g: &mut Game, p: PlayerId) {
    g.players[p.index()].debt -= 1;
    g.turn.coins -= 1;
}

fn resume_turn(g: &mut Game, player: PlayerId, pc: &mut u8, input: Input) -> Flow {
    let p = player;
    match pc {
        0 => {
            g.turn.phase = Phase::Start;
            let pl = &mut g.players[p.index()];
            pl.turns_taken += 1;
            if g.turn.extra.is_some() {
                pl.extra_turns_taken += 1;
            }
            let turn = pl.turns_taken;
            g.emit(Event::TurnStart { player: p, turn });
            *pc = 1;
            Flow::Call(Frame::window(Occasion::StartTurn { player: p }))
        }
        1 => {
            // One made during this start of turn lasts until the next turn.
            let turn = g.players[p.index()].turns_taken;
            g.lasting
                .retain(|l| l.until != Until::StartOfTurn(p) || (l.owner == p && l.turn == turn));
            g.turn.phase = Phase::Action;
            *pc = 2;
            Flow::Continue
        }
        2 => {
            match input {
                Input::Response(Response::Move(Move::Play(c))) => {
                    g.turn.actions -= 1;
                    return Flow::Call(Frame::play(p, c, true, vec![]));
                }
                Input::Response(Response::Move(Move::Villager)) => {
                    g.players[p.index()].villagers -= 1;
                    if !g.turn.ignore_plus_actions {
                        g.turn.actions += 1;
                    }
                }
                Input::Response(Response::Move(Move::Coffers)) => spend_coffers(g, p),
                Input::Response(Response::Move(Move::PayDebt)) => pay_debt(g, p),
                Input::Response(Response::Move(Move::EndPhase)) => {
                    g.turn.phase = Phase::Buy;
                    *pc = 3;
                    return Flow::Call(Frame::window(Occasion::StartBuyPhase { player: p }));
                }
                _ => {}
            }
            let mut moves: Vec<Move> = if g.turn.actions > 0 {
                g.playable(p, Types::ACTION)
                    .into_iter()
                    .map(Move::Play)
                    .collect()
            } else {
                vec![]
            };
            if g.players[p.index()].villagers > 0 {
                moves.push(Move::Villager);
            }
            push_token_moves(g, p, &mut moves);
            moves.push(Move::EndPhase);
            ask_moves(p, Phase::Action, moves)
        }
        3 => {
            // An effect returned to the Action phase (`Frame::return_to_action`).
            if g.turn.phase == Phase::Action {
                *pc = 2;
                return Flow::Continue;
            }
            if let Input::Response(Response::Move(m)) = input {
                match m {
                    Move::Play(c) => return Flow::Call(Frame::play(p, c, true, vec![])),
                    Move::Buy(pile) => {
                        return Flow::Call(Frame::Buy {
                            player: p,
                            pile: Some(pile),
                            landscape: None,
                            card: None,
                            index: 0,
                            pc: 0,
                        });
                    }
                    Move::BuyLandscape(card) => {
                        return Flow::Call(Frame::Buy {
                            player: p,
                            pile: None,
                            landscape: Some(card),
                            card: None,
                            index: 0,
                            pc: 0,
                        });
                    }
                    Move::Coffers => spend_coffers(g, p),
                    Move::PayDebt => pay_debt(g, p),
                    Move::EndPhase => {
                        *pc = 4;
                        return Flow::Call(Frame::window(Occasion::EndBuyPhase { player: p }));
                    }
                    Move::Villager => {}
                }
            }
            let mut moves = vec![];
            if !g.turn.bought {
                moves.extend(g.playable(p, Types::TREASURE).into_iter().map(Move::Play));
            }
            for (i, pile) in g.piles.iter().enumerate() {
                if let Some(&top) = pile.cards.last()
                    && pile.in_supply
                    && can_buy(g, p, g.card(top), g.cost_of(top))
                {
                    moves.push(Move::Buy(PileId(i as u8)));
                }
            }
            for &l in &g.landscapes {
                if matches!(l.kind(), Kind::Event | Kind::Project) && can_buy(g, p, l, g.cost(l)) {
                    moves.push(Move::BuyLandscape(l));
                }
            }
            push_token_moves(g, p, &mut moves);
            moves.push(Move::EndPhase);
            ask_moves(p, Phase::Buy, moves)
        }
        4 => {
            g.turn.phase = Phase::Night;
            *pc = 5;
            Flow::Call(Frame::window(Occasion::StartNight { player: p }))
        }
        5 => {
            match input {
                Input::Response(Response::Move(Move::Play(c))) => {
                    return Flow::Call(Frame::play(p, c, true, vec![]));
                }
                Input::Response(Response::Move(Move::Coffers)) => spend_coffers(g, p),
                Input::Response(Response::Move(Move::PayDebt)) => pay_debt(g, p),
                Input::Response(Response::Move(Move::EndPhase)) => {
                    g.turn.phase = Phase::Cleanup;
                    *pc = 6;
                    return Flow::Call(Frame::window(Occasion::StartCleanup { player: p }));
                }
                _ => {}
            }
            let mut moves: Vec<Move> = g
                .playable(p, Types::NIGHT)
                .into_iter()
                .map(Move::Play)
                .collect();
            push_token_moves(g, p, &mut moves);
            moves.push(Move::EndPhase);
            ask_moves(p, Phase::Night, moves)
        }
        6 => {
            *pc = 7;
            // Cards other players played this turn (Guard Dog, ...) are discarded too.
            let occs = g
                .players_from(p)
                .into_iter()
                .filter(|&q| q == p || !g.leaving_play(q).is_empty())
                .map(|player| Occasion::DiscardFromPlay { player })
                .collect();
            Flow::Call(Window::open_all(occs))
        }
        7 => {
            for other in g.others(p) {
                for c in g.leaving_play(other) {
                    g.move_unlogged(c, Zone::Discard(other));
                }
            }
            let mut cards = g.leaving_play(p);
            cards.extend(&g.players[p.index()].hand);
            *pc = 8;
            Flow::Call(Frame::Discard {
                player: p,
                cards,
                cleanup: true,
                i: 0,
            })
        }
        8 => {
            *pc = 9;
            Flow::Call(Frame::draw(p, g.turn.cleanup_draw))
        }
        9 => {
            *pc = 10;
            Flow::Call(Frame::window(Occasion::EndTurn { player: p }))
        }
        _ => {
            for l in std::mem::take(&mut g.lasting) {
                if l.until != Until::EndOfTurn && !l.ending {
                    g.lasting.push(l);
                } else if l.until == Until::StartOfTurn(l.owner)
                    && let Some(i) = g
                        .extra_turns
                        .iter()
                        .position(|t| t.player == l.owner && t.source == l.card)
                {
                    // The extra turn a Duration stayed in play for (Outpost, Voyage) is
                    // one of the things it no longer does.
                    g.extra_turns.remove(i);
                }
            }
            for pl in &mut g.players {
                g.boons.discard.append(&mut pl.boons_kept);
            }
            g.turn.phase = Phase::End;
            Flow::done()
        }
    }
}

fn resume_gain(
    g: &mut Game,
    player: PlayerId,
    from: From,
    to: Dest,
    (bought, buy): (bool, Option<usize>),
    (card, at): (&mut Option<CardId>, &mut Option<Zone>),
    pc: &mut u8,
) -> Flow {
    match pc {
        0 => {
            let id = match from {
                From::Pile(pile) => g.pile_top(pile),
                From::Card(c) => Some(c),
            };
            let Some(id) = id else {
                return Flow::Done(Value::Cards(vec![]));
            };
            *card = Some(id);
            *at = Some(g.zone(id));
            g.gains.push(GainState {
                player,
                card: id,
                to,
                bought,
                buy,
                cancelled: false,
                since: g.log.len(),
                cost: g.cost_of(id),
                phase: g.turn.phase,
                turn_index: None,
                buy_index: None,
            });
            *pc = 1;
            Flow::Call(Frame::window(Occasion::WouldGain { player, card: id }))
        }
        1 => {
            let expected = g.gains.last().expect("gain in progress").card;
            let now = moving(g, from, expected, *at);
            if g.gains.last().expect("gain in progress").cancelled || now.is_none() {
                g.gains.pop();
                *card = None;
                return Flow::Done(Value::Cards(vec![]));
            }
            let id = now.expect("checked above");
            *card = Some(id);
            g.gains.last_mut().expect("gain in progress").card = id;
            let st = g.gains.last().expect("gain in progress").clone();
            let zone = match st.to {
                Dest::Discard => Zone::Discard(st.player),
                Dest::Hand => Zone::Hand(st.player),
                Dest::DeckTop => Zone::Deck(st.player),
                Dest::SetAside => Zone::SetAside(st.player),
            };
            let cost = g.cost_of(st.card);
            let gain = g.gains.last_mut().expect("gain in progress");
            gain.cost = cost;
            gain.phase = g.turn.phase;
            if let Some(pile) = g.home_pile(g.printed(st.card)) {
                let pile = &mut g.piles[pile.index()];
                if pile.trade_route {
                    pile.trade_route = false;
                    g.trade_route_mat += 1;
                }
            }
            g.show(st.card);
            g.move_card(st.card, zone, Pos::Top);
            if let Zone::SetAside(p) = zone {
                g.players[p.index()].face_up.push(st.card);
            }
            let kind = g.card(st.card);
            if st.player == g.turn.player {
                let types = g.types(st.card);
                let gain = g.gains.last_mut().expect("gain in progress");
                gain.turn_index = Some(g.turn.gained.len());
                g.turn.gained.push(kind);
                g.turn.gained_types.push(types);
                if g.turn.phase == Phase::Buy {
                    gain.buy_index = Some(g.turn.gained_this_buy_phase().len());
                    g.turn.gained_in_buy_phase.push(kind);
                    g.turn.gained_in_buy_phase_types.push(types);
                }
            }
            g.emit(Event::Gained {
                player: st.player,
                card: kind,
                to: st.to,
            });
            *pc = 2;
            Flow::Call(Frame::window(Occasion::Gain {
                player: st.player,
                card: st.card,
                bought: st.bought,
            }))
        }
        _ => {
            g.last_gain = g.gains.pop();
            Flow::Done(Value::Cards(card.iter().copied().collect()))
        }
    }
}

/// The card a gain moves once its "would gain" window has closed. The card it began with,
/// if that is still where it was; otherwise, for a gain from a pile, the pile's top card
/// if it is a card of the same name (or any card of a pile named after a randomizer: "gain
/// a Loot" is its top card, whatever it is). Nothing if the pile has emptied or a specific
/// card has moved.
fn moving(g: &Game, from: From, expected: CardId, at: Option<Zone>) -> Option<CardId> {
    if Some(g.zone(expected)) == at {
        return Some(expected);
    }
    let From::Pile(pile) = from else {
        return None;
    };
    let top = g.pile_top(pile)?;
    let any = g.pile(pile).card.kind() == Kind::Randomizer;
    (any || g.printed(top) == g.printed(expected)).then_some(top)
}

pub(crate) use crate::cards::{OVERPAY, PAY_OTHERWISE};

fn resume_buy(
    g: &mut Game,
    p: PlayerId,
    (pile, landscape): (Option<PileId>, Option<Card>),
    card: &mut Option<CardId>,
    index: &mut usize,
    pc: &mut u8,
    input: Input,
) -> Flow {
    if let Some(l) = landscape {
        return match pc {
            0 => {
                let cost = g.cost(l);
                g.emit(Event::BoughtLandscape { player: p, card: l });
                pay(g, p, cost);
                if l.kind() == Kind::Project {
                    g.players[p.index()].projects.push(l);
                }
                *pc = 1;
                Flow::Call(Frame::window(Occasion::BuyLandscape { player: p, card: l }))
            }
            1 => {
                *pc = 2;
                g.begin_resolving(None);
                Flow::Call(Frame::Card(Box::new(CardFrame::new(
                    l,
                    Hook::Main,
                    p,
                    None,
                ))))
            }
            _ => {
                g.end_resolving();
                Flow::done()
            }
        };
    }
    let pile = pile.expect("a buy is of a pile or a landscape");
    match pc {
        0 => {
            let Some(top) = g.pile_top(pile) else {
                return Flow::done();
            };
            *card = Some(top);
            *pc = 5;
            let kind = g.card(top);
            if cards::pays_otherwise(g, p, kind) {
                return Flow::Call(Frame::Card(Box::new(CardFrame::new(
                    kind,
                    Hook::Ability(PAY_OTHERWISE),
                    p,
                    Some(top),
                ))));
            }
            Flow::Continue
        }
        5 => {
            let top = card.expect("set in step 0");
            let paid_otherwise = matches!(input, Input::Return(Value::Bool(true)));
            let cost = if paid_otherwise {
                crate::Cost::ZERO
            } else {
                g.cost_of(top)
            };
            let kind = g.card(top);
            g.emit(Event::Bought {
                player: p,
                card: kind,
            });
            pay(g, p, cost);
            g.turn.bought_cards.push(kind);
            *index = g.turn.bought_cards.len();
            *pc = 2;
            if kind.def().overpay {
                return Flow::Call(Frame::overpay(p, top));
            }
            Flow::Continue
        }
        2 => {
            *pc = 3;
            let card = card.expect("set in step 0");
            Flow::Call(Frame::window(Occasion::Buy {
                player: p,
                card,
                cost: g.cost_of(card),
            }))
        }
        3 => {
            *pc = 4;
            // What was bought is a card of that name from the pile, not one physical copy:
            // an on-buy ability may have gained the copy that was on top.
            let bought = g.printed(card.expect("set in step 0"));
            let top = g
                .pile_top(pile)
                .filter(|&c| !g.pile(pile).removed && g.card(c) == bought);
            if top.is_none() {
                return Flow::done();
            }
            Flow::Call(Frame::Gain {
                player: p,
                from: From::Pile(pile),
                to: Dest::Discard,
                bought: true,
                buy: Some(*index),
                card: None,
                at: None,
                pc: 0,
            })
        }
        _ => Flow::done(),
    }
}

fn resume_overpay(
    g: &mut Game,
    p: PlayerId,
    card: CardId,
    paid: &mut crate::Cost,
    pc: &mut u8,
    input: Input,
) -> Flow {
    let ask = |max: u32| {
        Flow::Ask(Decision {
            player: p,
            kind: DecisionKind::Number {
                min: 0,
                max: max as i32,
            },
            purpose: Purpose::Overpay,
            source: Some(g.card(card)),
            about: vec![],
            question: None,
        })
    };
    match pc {
        0 => {
            *pc = 1;
            if g.turn.coins > 0 {
                return ask(g.turn.coins);
            }
            Flow::Continue
        }
        1 => {
            paid.coins = input.number().unwrap_or(0).max(0) as u8;
            *pc = 2;
            if g.turn.potions > 0 {
                return ask(g.turn.potions);
            }
            Flow::Continue
        }
        2 => {
            paid.potions = input.number().unwrap_or(0).max(0) as u8;
            *pc = 3;
            if *paid == crate::Cost::ZERO {
                return Flow::done();
            }
            g.turn.coins -= paid.coins as u32;
            g.turn.potions -= paid.potions as u32;
            g.emit(Event::Overpaid {
                player: p,
                amount: paid.coins as u32,
                potions: paid.potions as u32,
            });
            let mut f = CardFrame::new(g.card(card), Hook::Ability(OVERPAY), p, Some(card));
            f.data = paid.coins as i32;
            f.s.cost = *paid;
            Flow::Call(Frame::Card(Box::new(f)))
        }
        _ => Flow::done(),
    }
}

fn pay(g: &mut Game, p: PlayerId, cost: crate::Cost) {
    let turn = &mut g.turn;
    turn.coins -= cost.coins as u32;
    turn.potions -= cost.potions as u32;
    if turn.buys > 0 {
        turn.buys -= 1;
    } else {
        turn.actions -= 1;
    }
    turn.bought = true;
    g.add_debt(p, cost.debt as u32);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PlayFrame {
    player: PlayerId,
    card: CardId,
    move_to_play: bool,
    chain: Vec<CardId>,
    targets: Vec<PlayerId>,
    remaining: Vec<PlayerId>,
    /// How many more times to follow the card's own instructions.
    again: u32,
    pc: u8,
}

impl PlayFrame {
    fn resume(&mut self, g: &mut Game, input: Input) -> Flow {
        let (p, id) = (self.player, self.card);
        match self.pc {
            0 => {
                if self.move_to_play && g.zone(id) == Zone::Hand(p) {
                    if !cards::can_play(g, p, id) {
                        return Flow::done();
                    }
                    if p == g.turn.player {
                        g.turn.hand_plays += 1;
                    }
                }
                let enters = self.move_to_play && g.zone(id) != Zone::InPlay(p);
                if enters {
                    g.move_card(id, Zone::InPlay(p), Pos::Top);
                }
                let card = g.card(id);
                let since = g.log.len();
                g.emit(Event::Played { player: p, card });
                let index = g.turn.played.len();
                g.turn.played.push(card);
                g.turn.played_by.push(p);
                g.turn.played_types.push(g.types(id));
                g.turn.played_ids.push(id);
                g.turn.stayed_in_play.push(g.zone(id) == Zone::InPlay(p));
                let behavior = cards::behavior(g, id).unwrap_or(card);
                g.plays.push(PlayState {
                    player: p,
                    card: id,
                    behavior,
                    way: None,
                    replaced: None,
                    blocked: false,
                    since,
                    gave_coins: false,
                    run_since: since,
                    tracker: self.chain.last().copied().filter(|_| !enters),
                    index,
                });
                self.pc = 1;
                Flow::Call(Frame::window(Occasion::Play {
                    player: p,
                    card: id,
                }))
            }
            1 => {
                // Reactions to an Attack (Moat, ...) come before its player picks a Way, and
                // answer the card being played, whatever it ends up doing.
                self.pc = 2;
                if g.is(id, Types::ATTACK) {
                    self.remaining = g.others_in_turn_order(p);
                    self.pc = 3;
                }
                Flow::Continue
            }
            2 => {
                self.pc = 6;
                let ways: Vec<Card> = g
                    .landscapes
                    .iter()
                    .copied()
                    .filter(|&l| l.kind() == Kind::Way && cards::can_use_way(g, id, l))
                    .collect();
                // A Way can be used instead of a replaced or blocked effect (Enchantress,
                // Enlightenment, Highwayman).
                if ways.is_empty() || !g.is(id, Types::ACTION) {
                    return Flow::Continue;
                }
                let mut options = vec![Opt::No];
                let mut labels = vec![format!("Play {}", g.card(id))];
                for w in ways {
                    options.push(Opt::Card(w));
                    labels.push(w.to_string());
                }
                Flow::Ask(Decision {
                    player: p,
                    kind: DecisionKind::Choose {
                        options,
                        labels,
                        min: 1,
                        max: 1,
                    },
                    purpose: Purpose::Way,
                    source: Some(g.card(id)),
                    about: vec![],
                    question: Some("play it as itself, or use a Way instead".into()),
                })
            }
            3 => {
                if let Input::Return(Value::Bool(unaffected)) = input {
                    let target = self.remaining.remove(0);
                    if !unaffected {
                        self.targets.push(target);
                    }
                }
                match self.remaining.first() {
                    Some(&target) => Flow::Call(Frame::window(Occasion::Attack {
                        attacker: p,
                        card: id,
                        target,
                    })),
                    None => {
                        self.pc = 2;
                        Flow::Continue
                    }
                }
            }
            6 => {
                if let Some(Opt::Card(way)) = input.opt() {
                    g.plays.last_mut().expect("play in progress").way = Some(way);
                    g.emit(Event::UsedWay {
                        player: p,
                        card: g.card(id),
                        way,
                    });
                }
                let st = g.plays.last().expect("play in progress");
                if st.way.is_none() && st.replaced.is_none() && !st.blocked {
                    self.again = cards::extra_runs(g, p, id);
                }
                self.pc = 4;
                Flow::Continue
            }
            4 => {
                let st = g.plays.last_mut().expect("play in progress");
                st.run_since = g.log.len();
                let st = st.clone();
                self.pc = 7;
                if st.blocked && st.way.is_none() {
                    return Flow::Continue;
                }
                let mine = p == g.turn.player;
                if let Some(plus) = st.replaced
                    && st.way.is_none()
                {
                    if mine {
                        if !g.turn.ignore_plus_actions {
                            g.turn.actions += plus.actions as u32;
                        }
                        g.turn.buys += plus.buys as u32;
                    }
                    g.add_coins(p, plus.coins as u32);
                    return Flow::Call(Frame::draw(p, plus.cards as usize));
                }
                self.pc = 10;
                if st.way.is_none()
                    && let Some(first) = cards::prelude(st.behavior, p, id)
                {
                    return Flow::Call(first);
                }
                Flow::Continue
            }
            10 => {
                let st = g.plays.last().expect("play in progress");
                let mine = p == g.turn.player;
                // A Way's own +Cards, +Actions, ... replace the card's.
                let def = st.way.unwrap_or(st.behavior).def();
                let mut bonus = cards::Bonus {
                    plus: def.plus,
                    coins: def.coins as u32,
                    potions: def.potions as u32,
                };
                if st.way.is_none() {
                    cards::modify_bonus(g, p, id, &mut bonus);
                }
                let pl = &mut g.players[p.index()];
                pl.villagers += bonus.plus.villagers as u32;
                pl.coffers += bonus.plus.coffers as u32;
                pl.favors += bonus.plus.favors as u32;
                if mine {
                    if !g.turn.ignore_plus_actions {
                        g.turn.actions += bonus.plus.actions as u32;
                    }
                    g.turn.buys += bonus.plus.buys as u32;
                    g.turn.potions += bonus.potions;
                }
                let had = g.player(p).coins_got;
                g.add_coins(p, bonus.plus.coins as u32 + bonus.coins);
                g.plays.last_mut().expect("play in progress").gave_coins |=
                    g.player(p).coins_got > had;
                self.pc = 5;
                Flow::Call(Frame::draw(p, bonus.plus.cards as usize))
            }
            5 => {
                let st = g.plays.last().expect("play in progress");
                let source = st.way.unwrap_or(st.behavior);
                let mut f = CardFrame::new(source, Hook::Main, p, Some(id));
                f.targets = self.targets.clone();
                f.chain = self.chain.clone();
                self.pc = 9;
                Flow::Call(Frame::Card(Box::new(f)))
            }
            9 => {
                self.pc = 7;
                if self.again > 0 {
                    self.again -= 1;
                    self.pc = 4;
                }
                Flow::Continue
            }
            7 => {
                self.pc = 8;
                Flow::Call(Frame::window(Occasion::AfterPlay {
                    player: p,
                    card: id,
                }))
            }
            _ => {
                g.plays.pop();
                Flow::done()
            }
        }
    }
}
