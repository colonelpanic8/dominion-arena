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
use crate::game::{EndReason, GainState, Phase, PlayState, Pos, Turn, Until, Zone};
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

#[derive(Debug, Default)]
pub(crate) enum Value {
    #[default]
    None,
    Cards(Vec<CardId>),
    Bool(bool),
}

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
        drawn: Vec<CardId>,
        started: bool,
        /// One draw shuffles at most once, even if its discard pile refills meanwhile.
        shuffled: bool,
    },
    Shuffle {
        player: PlayerId,
        pc: u8,
    },
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
        card: Option<CardId>,
        pc: u8,
    },
    Buy {
        player: PlayerId,
        pile: Option<PileId>,
        landscape: Option<Card>,
        card: Option<CardId>,
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
    /// Receive a Boon or Hex: resolve it, then discard it unless it was kept.
    Receive {
        player: PlayerId,
        card: Card,
        pc: u8,
    },
}

impl Frame {
    pub(crate) fn draw(player: PlayerId, count: usize) -> Frame {
        Frame::Draw {
            player,
            count,
            drawn: vec![],
            started: false,
            shuffled: false,
        }
    }

    /// The top `count` cards of a deck (top first), revealed to everyone.
    pub(crate) fn reveal(player: PlayerId, count: usize) -> Frame {
        Frame::TopCards {
            player,
            count,
            reveal: true,
            shuffled: false,
        }
    }

    /// The top `count` cards of a deck (top first), seen only by their owner.
    pub(crate) fn look(player: PlayerId, count: usize) -> Frame {
        Frame::TopCards {
            player,
            count,
            reveal: false,
            shuffled: false,
        }
    }

    pub(crate) fn shuffle(player: PlayerId) -> Frame {
        Frame::Shuffle { player, pc: 0 }
    }

    /// Gain the top card of `pile`.
    pub(crate) fn gain(player: PlayerId, pile: PileId, to: Dest) -> Frame {
        Frame::Gain {
            player,
            from: From::Pile(pile),
            to,
            bought: false,
            card: None,
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
            card: None,
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

    pub(crate) fn receive(player: PlayerId, card: Card) -> Frame {
        Frame::Receive {
            player,
            card,
            pc: 0,
        }
    }

    /// Play `card`, moving it into play first if `move_to_play`. `chain` lists the cards
    /// that are playing it (Throne Room, ...), which stay in play as long as it does.
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
            pc: 0,
        })
    }

    pub(crate) fn window(occ: Occasion) -> Frame {
        Window::open(occ)
    }

    pub(crate) fn resume(&mut self, g: &mut Game, input: Input) -> Flow {
        match self {
            Frame::Game { next, pc } => resume_game(g, next, pc),
            Frame::Turn { player, pc } => resume_turn(g, *player, pc, input),
            Frame::Play(f) => f.resume(g, input),
            Frame::Card(f) => cards::resume(g, f, input),
            Frame::Window(w) => w.resume(g, input),
            Frame::Draw {
                player,
                count,
                drawn,
                started,
                shuffled,
            } => {
                let p = *player;
                if !*started {
                    *started = true;
                    let pl = &mut g.players[p.index()];
                    if pl.minus_card && *count > 0 {
                        pl.minus_card = false;
                        *count -= 1;
                    }
                }
                while drawn.len() < *count {
                    let pl = &g.players[p.index()];
                    if pl.deck.is_empty() {
                        if pl.discard.is_empty() || *shuffled {
                            break;
                        }
                        *shuffled = true;
                        return Flow::Call(Frame::shuffle(p));
                    }
                    let top = *pl.deck.last().expect("deck is not empty");
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
            Frame::Shuffle { player, pc } => match pc {
                0 => {
                    *pc = 1;
                    Flow::Call(Frame::window(Occasion::WouldShuffle { player: *player }))
                }
                1 => {
                    let count = g.players[player.index()].discard.len();
                    if count == 0 {
                        return Flow::done();
                    }
                    g.shuffle_discard_into_deck(*player);
                    *pc = 2;
                    Flow::Call(Frame::window(Occasion::Shuffled {
                        player: *player,
                        count,
                    }))
                }
                _ => Flow::done(),
            },
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
                if *reveal {
                    for &c in &cards {
                        g.emit(Event::Revealed {
                            player: p,
                            card: g.card(c),
                        });
                    }
                }
                Flow::Done(Value::Cards(cards))
            }
            Frame::Gain {
                player,
                from,
                to,
                bought,
                card,
                pc,
            } => resume_gain(g, *player, *from, *to, *bought, card, pc),
            Frame::Buy {
                player,
                pile,
                landscape,
                card,
                pc,
            } => resume_buy(g, *player, *pile, *landscape, card, pc, input),
            Frame::Trash { player, cards, i } => {
                // Cards trashed together are all trashed before any "when trashed" ability.
                if *i == 0 {
                    for &c in cards.iter() {
                        let from = g.zone(c);
                        if !cards::trash_instead(g, *player, c) {
                            g.move_card(c, Zone::Trash, Pos::Top);
                        }
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
                } else if let Some(pos) = g.trashing.iter().rposition(|(c, _)| *c == cards[*i - 1])
                {
                    g.trashing.remove(pos);
                }
                let Some(&c) = cards.get(*i) else {
                    return Flow::Done(Value::Cards(std::mem::take(cards)));
                };
                *i += 1;
                Flow::Call(Frame::window(Occasion::Trash {
                    player: *player,
                    card: c,
                }))
            }
            Frame::Discard {
                player,
                cards,
                cleanup,
                i,
            } => {
                if *cleanup {
                    for &c in cards.iter() {
                        g.move_card(c, Zone::Discard(*player), Pos::Top);
                    }
                    return Flow::Done(Value::Cards(std::mem::take(cards)));
                }
                // Cards discarded together are all discarded before any reacts.
                if *i == 0 {
                    for &c in cards.iter() {
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
            Frame::Receive { player, card, pc } => match pc {
                0 => {
                    *pc = 1;
                    g.emit(Event::Received {
                        player: *player,
                        card: *card,
                    });
                    Flow::Call(Frame::Card(Box::new(CardFrame::new(
                        *card,
                        Hook::Main,
                        *player,
                        None,
                    ))))
                }
                _ => {
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
        }
    }
}

fn resume_game(g: &mut Game, next: &mut PlayerId, pc: &mut u8) -> Flow {
    match pc {
        0 => {
            let prev = (g.turn.player, g.turn.extra.is_some());
            let started = g.players.iter().any(|p| p.turns_taken > 0);
            while let Some(extra) = g.extra_turns.pop_front() {
                // Nobody takes more than two turns in a row.
                if (started && extra.player == prev.0 && prev.1) || g.skips_turn(extra.player, true)
                {
                    continue;
                }
                g.turn = Turn::new(extra.player);
                g.turn.controller = extra.controller;
                g.turn.extra = Some(extra.source);
                *pc = 1;
                return Flow::Call(Frame::Turn {
                    player: extra.player,
                    pc: 0,
                });
            }
            if g.ending.is_some() {
                let p = g
                    .final_round
                    .pop_front()
                    .expect("a final-round turn is due");
                g.turn = Turn::new(p);
                // Like extra turns, these don't count for the tiebreaker.
                g.players[p.index()].extra_turns_taken += 1;
            } else {
                while g.skips_turn(*next, false) {
                    *next = g.left_of(*next);
                }
                g.turn = Turn::new(*next);
                *next = g.left_of(*next);
            }
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
            g.lasting.retain(|l| l.until != Until::StartOfTurn(p));
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
                Input::Response(Response::Move(Move::PayDebt)) => {
                    g.players[p.index()].debt -= 1;
                    g.turn.coins -= 1;
                }
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
            if g.players[p.index()].debt > 0 && g.turn.coins > 0 {
                moves.push(Move::PayDebt);
            }
            moves.push(Move::EndPhase);
            ask_moves(p, Phase::Action, moves)
        }
        3 => {
            if g.turn.return_to_action {
                g.turn.return_to_action = false;
                *pc = 11;
                return Flow::Call(Frame::window(Occasion::EndBuyPhase { player: p }));
            }
            // An effect already returned to the Action phase itself (Continue, which plays
            // a card there).
            if g.turn.phase == Phase::Action {
                g.turn.bought = false;
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
                            pc: 0,
                        });
                    }
                    Move::BuyLandscape(card) => {
                        return Flow::Call(Frame::Buy {
                            player: p,
                            pile: None,
                            landscape: Some(card),
                            card: None,
                            pc: 0,
                        });
                    }
                    Move::Coffers => {
                        g.players[p.index()].coffers -= 1;
                        g.add_coins(p, 1);
                    }
                    Move::PayDebt => {
                        g.players[p.index()].debt -= 1;
                        g.turn.coins -= 1;
                    }
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
            if g.turn.can_buy_cards {
                for (i, pile) in g.piles.iter().enumerate() {
                    if let Some(&top) = pile.cards.last()
                        && pile.in_supply
                        && can_buy(g, p, g.card(top), g.cost_of(top))
                    {
                        moves.push(Move::Buy(PileId(i as u8)));
                    }
                }
            }
            for &l in &g.landscapes {
                if matches!(l.kind(), Kind::Event | Kind::Project) && can_buy(g, p, l, g.cost(l)) {
                    moves.push(Move::BuyLandscape(l));
                }
            }
            let pl = &g.players[p.index()];
            if pl.coffers > 0 {
                moves.push(Move::Coffers);
            }
            if pl.debt > 0 && g.turn.coins > 0 {
                moves.push(Move::PayDebt);
            }
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
            moves.push(Move::EndPhase);
            ask_moves(p, Phase::Night, moves)
        }
        6 => {
            *pc = 7;
            Flow::Call(Frame::window(Occasion::DiscardFromPlay { player: p }))
        }
        7 => {
            let kept: Vec<CardId> = g.lasting.iter().flat_map(|l| l.keeps.clone()).collect();
            // Cards other players played this turn (Guard Dog, ...) are discarded too.
            for other in g.others(p) {
                let stray: Vec<CardId> = g.players[other.index()]
                    .in_play
                    .iter()
                    .filter(|c| !kept.contains(c))
                    .copied()
                    .collect();
                for c in stray {
                    g.move_card(c, Zone::Discard(other), Pos::Top);
                }
            }
            let pl = &g.players[p.index()];
            let cards = pl
                .in_play
                .iter()
                .filter(|c| !kept.contains(c))
                .chain(&pl.hand)
                .copied()
                .collect();
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
        11 => {
            g.turn.phase = Phase::Action;
            g.turn.bought = false;
            *pc = 2;
            Flow::Continue
        }
        _ => {
            g.lasting.retain(|l| l.until != Until::EndOfTurn);
            let kept = std::mem::take(&mut g.players[p.index()].boons_kept);
            g.boons.discard.extend(kept);
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
    bought: bool,
    card: &mut Option<CardId>,
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
            g.gains.push(GainState {
                player,
                card: id,
                to,
                bought,
                cancelled: false,
                cost: g.cost_of(id),
                turn_index: None,
            });
            *pc = 1;
            Flow::Call(Frame::window(Occasion::WouldGain { player, card: id }))
        }
        1 => {
            let st = g.gains.last().expect("gain in progress").clone();
            if st.cancelled {
                g.gains.pop();
                return Flow::Done(Value::Cards(vec![]));
            }
            let zone = match st.to {
                Dest::Discard => Zone::Discard(st.player),
                Dest::Hand => Zone::Hand(st.player),
                Dest::DeckTop => Zone::Deck(st.player),
                Dest::SetAside => Zone::SetAside(st.player),
            };
            let cost = g.cost_of(st.card);
            g.gains.last_mut().expect("gain in progress").cost = cost;
            if let Some(pile) = g.home_pile(g.printed(st.card)) {
                let pile = &mut g.piles[pile.index()];
                if pile.trade_route {
                    pile.trade_route = false;
                    g.trade_route_mat += 1;
                }
            }
            g.move_card(st.card, zone, Pos::Top);
            let kind = g.card(st.card);
            if st.player == g.turn.player {
                g.gains.last_mut().expect("gain in progress").turn_index =
                    Some(g.turn.gained.len());
                g.turn.gained.push(kind);
                if g.turn.phase == Phase::Buy {
                    g.turn.gained_in_buy_phase.push(kind);
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
            g.gains.pop();
            Flow::Done(Value::Cards(card.iter().copied().collect()))
        }
    }
}

/// Overpay is resolved as the bought card's ability with this number.
pub(crate) const OVERPAY: u8 = 255;
/// Paying for a card another way than with its cost (Animal Fair) is offered as the card's
/// ability with this number, which finishes with `Value::Bool(true)` if it was paid that way.
pub(crate) const PAY_OTHERWISE: u8 = 254;

fn resume_buy(
    g: &mut Game,
    p: PlayerId,
    pile: Option<PileId>,
    landscape: Option<Card>,
    card: &mut Option<CardId>,
    pc: &mut u8,
    input: Input,
) -> Flow {
    if let Some(l) = landscape {
        return match pc {
            0 => {
                let cost = g.cost(l);
                pay(g, p, cost);
                g.emit(Event::BoughtLandscape { player: p, card: l });
                if l.kind() == Kind::Project {
                    g.players[p.index()].projects.push(l);
                }
                *pc = 1;
                Flow::Call(Frame::window(Occasion::BuyLandscape { player: p, card: l }))
            }
            1 => {
                *pc = 2;
                Flow::Call(Frame::Card(Box::new(CardFrame::new(
                    l,
                    Hook::Main,
                    p,
                    None,
                ))))
            }
            _ => Flow::done(),
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
            pay(g, p, cost);
            let kind = g.card(top);
            g.turn.bought_cards.push(kind);
            g.emit(Event::Bought {
                player: p,
                card: kind,
            });
            *pc = 2;
            if kind.def().overpay && (g.turn.coins > 0 || g.turn.potions > 0) {
                *pc = 1;
                return Flow::Ask(Decision {
                    player: p,
                    kind: DecisionKind::Number {
                        min: 0,
                        max: g.turn.coins as i32,
                    },
                    purpose: Purpose::Overpay,
                    source: Some(kind),
                });
            }
            Flow::Continue
        }
        1 => {
            let n = input.number().unwrap_or(0);
            *pc = 2;
            if n <= 0 {
                return Flow::Continue;
            }
            g.turn.coins -= n as u32;
            let top = card.expect("set in step 0");
            g.emit(Event::Overpaid {
                player: p,
                amount: n as u32,
            });
            let mut f = CardFrame::new(g.card(top), Hook::Ability(OVERPAY), p, Some(top));
            f.data = n;
            Flow::Call(Frame::Card(Box::new(f)))
        }
        2 => {
            *pc = 3;
            Flow::Call(Frame::window(Occasion::Buy {
                player: p,
                card: card.expect("set in step 0"),
            }))
        }
        3 => {
            *pc = 4;
            let top = card.expect("set in step 0");
            if g.zone(top) != Zone::Pile(pile) {
                return Flow::done();
            }
            Flow::Call(Frame::Gain {
                player: p,
                from: From::Card(top),
                to: Dest::Discard,
                bought: true,
                card: None,
                pc: 0,
            })
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
    pc: u8,
}

impl PlayFrame {
    fn resume(&mut self, g: &mut Game, input: Input) -> Flow {
        let (p, id) = (self.player, self.card);
        match self.pc {
            0 => {
                if self.move_to_play && g.zone(id) == Zone::Hand(p) && p == g.turn.player {
                    g.turn.hand_plays += 1;
                }
                if self.move_to_play && g.zone(id) != Zone::InPlay(p) {
                    g.move_card(id, Zone::InPlay(p), Pos::Top);
                }
                let card = g.card(id);
                g.emit(Event::Played { player: p, card });
                if p == g.turn.player {
                    g.turn.played.push(card);
                    g.turn.played_ids.push(id);
                }
                let behavior = cards::behavior(g, id).unwrap_or(card);
                g.plays.push(PlayState {
                    player: p,
                    card: id,
                    behavior,
                    way: None,
                    replaced: None,
                    blocked: false,
                });
                self.pc = 1;
                Flow::Call(Frame::window(Occasion::Play {
                    player: p,
                    card: id,
                }))
            }
            1 => {
                // Reactions to an Attack (Moat, ...) come before its player picks a Way.
                let st = g.plays.last().expect("play in progress");
                self.pc = 2;
                if st.replaced.is_none() && !st.blocked && g.is(id, Types::ATTACK) {
                    self.remaining = g.others(p);
                    self.pc = 3;
                }
                Flow::Continue
            }
            2 => {
                self.pc = 6;
                let st = g.plays.last().expect("play in progress");
                let ways: Vec<Card> = g
                    .landscapes
                    .iter()
                    .copied()
                    .filter(|&l| l.kind() == Kind::Way && cards::can_use_way(g, id, l))
                    .collect();
                // A Way can be used instead of a replaced effect (Enchantress, Enlightenment).
                if ways.is_empty() || st.blocked || !g.is(id, Types::ACTION) {
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
                    purpose: Purpose::Mode,
                    source: Some(g.card(id)),
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
                }
                self.pc = 4;
                Flow::Continue
            }
            4 => {
                let st = g.plays.last().expect("play in progress").clone();
                self.pc = 7;
                if st.blocked {
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
                // A Way's own +Cards, +Actions, ... replace the card's.
                let def = st.way.unwrap_or(st.behavior).def();
                let mut bonus = cards::Bonus {
                    plus: def.plus,
                    coins: def.coins as u32,
                    potions: def.potions as u32,
                };
                cards::modify_bonus(g, p, id, &mut bonus);
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
                g.add_coins(p, bonus.plus.coins as u32 + bonus.coins);
                self.pc = 5;
                Flow::Call(Frame::draw(p, bonus.plus.cards as usize))
            }
            5 => {
                let st = g.plays.last().expect("play in progress");
                let source = st.way.unwrap_or(st.behavior);
                let mut f = CardFrame::new(source, Hook::Main, p, Some(id));
                f.targets = std::mem::take(&mut self.targets);
                f.chain = self.chain.clone();
                self.pc = 7;
                Flow::Call(Frame::Card(Box::new(f)))
            }
            7 => {
                g.plays.pop();
                self.pc = 8;
                Flow::Call(Frame::window(Occasion::AfterPlay {
                    player: p,
                    card: id,
                }))
            }
            _ => Flow::done(),
        }
    }
}
