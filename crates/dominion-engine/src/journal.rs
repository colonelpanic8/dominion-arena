//! An opt-in, ordered record of everything that changes on the table: what the
//! `dominion/events@1` stream is written from. [`Game::new_journaled`] turns it on.
//!
//! It holds the [`Trace`] entries (every card created, moved, mixed and shown) with the
//! rules' word for each move, the log entries in their place among them, and every
//! counter as it changes: counters are read whenever something is recorded, so a change
//! lies between the same two card moves it happened between.

use crate::game::{EndReason, ExtraLimit, ExtraTurn, Phase, Pos, Until};
use crate::{Card, CardId, Game, PileId, PlayerId, Trace, Zone};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// What kind of move a card made: the rules' word for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verb {
    Play,
    Draw,
    Discard,
    Gain,
    Trash,
    Topdeck,
    Bottomdeck,
    /// Into a deck under some of its cards.
    Insert,
    Reveal,
    Look,
    SetAside,
    Exile,
    /// Any other placement: into a hand, onto a mat, a deck into its discard pile.
    Put,
    /// To its pile.
    Return,
    /// One of an exchange's two moves.
    Exchange,
    Shuffle,
    /// From the top of a pile to its bottom.
    Rotate,
}

/// A counter the journal follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gauge {
    Actions,
    Buys,
    Coins,
    Potions,
    Coffers(PlayerId),
    Villagers(PlayerId),
    Favors(PlayerId),
    Debt(PlayerId),
    VpTokens(PlayerId),
    PirateShip(PlayerId),
    PileVp(PileId),
    PileDebt(PileId),
    Embargo(PileId),
    TradeRoute(PileId),
    TradeRouteMat,
    Sun,
    /// How many cards the turn's Clean-up draws.
    NextHand,
    /// 1 once the turn has bought something in this Buy phase (no more Treasures).
    Bought,
    /// Plays from the hand this turn, followed only in a turn that limits them (Voyage).
    HandPlays,
    /// Turns this player will skip (Lich).
    SkipTurns(PlayerId),
    /// Regular turns this player has skipped.
    TurnsSkipped(PlayerId),
}

/// What kind of turn a turn is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnKind {
    Regular,
    /// From `source`, the extra turn waiting with that `id`.
    Extra {
        source: Card,
        id: u32,
    },
    /// One of the round after the game has ended (Fleet).
    Final,
}

impl TurnKind {
    pub(crate) fn extra(e: &ExtraTurn) -> TurnKind {
        TurnKind::Extra {
            source: e.source,
            id: e.id,
        }
    }
}

/// What a turn forbids beyond the rules of every turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnLimits {
    /// No buying cards (Mission's turn); Events and Projects may be bought.
    pub no_card_buys: bool,
    /// At most this many plays from the hand (Voyage's turn).
    pub hand_plays: Option<u32>,
}

/// A [`crate::Lasting`] as the table shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectState {
    pub id: u32,
    pub card: Card,
    pub source: Option<CardId>,
    pub owner: PlayerId,
    /// When it ends, as its players are told ([`crate::View::lasting_ends`]).
    pub until: Until,
    /// Its card left play: it still works this turn and ends with it.
    pub ending: bool,
    pub keeps: Vec<CardId>,
    /// The cards of its data that are set aside.
    pub holds: Vec<CardId>,
    /// Tokens on its card (Garrison's).
    pub tokens: Option<u32>,
}

/// Why a lasting effect ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectEnd {
    /// An ability removed it: its own, resolving, as a rule.
    Resolved,
    /// Its `until` came.
    Expired,
    /// Its card had left play: it ended with the turn.
    LeftPlay,
    /// The extra turn it kept a card in play for began, was skipped or was lost.
    ExtraTurn,
}

impl Gauge {
    /// Whether it is the turn's: reset as a turn begins, gone between turns.
    pub fn of_turn(self) -> bool {
        matches!(
            self,
            Gauge::Actions
                | Gauge::Buys
                | Gauge::Coins
                | Gauge::Potions
                | Gauge::NextHand
                | Gauge::Bought
                | Gauge::HandPlays
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// A trace entry; `verb` is how its card moved, for a [`Trace::Moved`].
    Trace {
        entry: Trace,
        verb: Option<Verb>,
    },
    /// `player` plays `card`: its instructions are about to be followed. The move right
    /// before it, if its verb is [`Verb::Play`], put it into play.
    Played {
        player: PlayerId,
        card: CardId,
    },
    /// [`Game::log`]'s entry at `index` was logged here.
    Logged {
        index: usize,
    },
    /// A turn began: `player`'s `number`th, extra ones counted. Its counters are 1 Action,
    /// 1 Buy, $0, no Potions and a Clean-up drawing 5, and its phase [`Phase::Start`].
    Turn {
        player: PlayerId,
        number: u32,
        kind: TurnKind,
        limits: TurnLimits,
    },
    Phase(Phase),
    /// `gauge` changed from `was` to `now`.
    Gauge {
        gauge: Gauge,
        was: i64,
        now: i64,
    },
    /// The card the move right before put into a set-aside zone or `Looking` is face up.
    FaceUp(CardId),
    EffectBegan(EffectState),
    /// The effect with this id is now this.
    EffectChanged(EffectState),
    EffectEnded {
        id: u32,
        why: EffectEnd,
    },
    /// An extra turn is waiting: at the front of those waiting when `front`.
    ExtraTurnQueued {
        turn: ExtraTurn,
        front: bool,
    },
    /// The extra turn waiting with this id won't happen: its effect ended.
    ExtraTurnRemoved {
        id: u32,
    },
    /// A turn `player` would have taken doesn't happen: skipped, or `lost` as a third
    /// turn in a row.
    TurnSkipped {
        player: PlayerId,
        kind: TurnKind,
        lost: bool,
    },
    /// The turn is over: nobody's turn is in progress.
    TurnEnded {
        player: PlayerId,
    },
    /// The game will end, after the turns of `final_round` (Fleet), in that order.
    GameEnding {
        reason: EndReason,
        final_round: Vec<PlayerId>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Journal {
    steps: Vec<Step>,
    /// What the gauges read when last recorded, in [`Game::gauges`]' order.
    readings: Vec<(Gauge, i64)>,
    /// The lasting effects as last recorded.
    effects: Vec<EffectState>,
    /// The id the next extra turn queued gets.
    next_extra: u32,
    /// The extra turn the turn about to begin takes.
    taking: Option<u32>,
}

impl Game {
    /// Starts a game that records a journal (and a [`Trace`]), beginning with every card
    /// in it.
    pub fn new_journaled(config: crate::GameConfig) -> Result<Game, crate::SetupError> {
        let mut game = crate::setup::build(config)?;
        game.enable_trace();
        let steps = (game.trace().unwrap_or_default().iter().cloned())
            .map(|entry| Step::Trace { entry, verb: None })
            .collect();
        // What setup gave (starting Favors, tokens on piles) is journaled after the cards.
        game.journal = Some(Journal {
            steps,
            readings: (game.gauges().into_iter())
                .filter(|(g, _)| g.of_turn())
                .collect(),
            effects: vec![],
            next_extra: 1,
            taking: None,
        });
        game.sync_journal();
        game.start();
        Ok(game)
    }

    /// Takes the journal recorded so far, leaving it empty (and still recording).
    pub fn take_journal(&mut self) -> Vec<Step> {
        self.sync_journal();
        (self.journal.as_mut())
            .map(|j| std::mem::take(&mut j.steps))
            .unwrap_or_default()
    }

    /// Moves `id` as the rules' `verb`.
    pub(crate) fn move_as(&mut self, verb: Verb, id: CardId, to: Zone, pos: Pos) {
        self.verb = Some(verb);
        self.move_card(id, to, pos);
        self.verb = None;
    }

    pub(crate) fn journal_trace(&mut self, entry: &Trace) {
        if self.journal.is_none() {
            return;
        }
        self.sync_journal();
        let verb = match *entry {
            Trace::Moved {
                to, bottom, under, ..
            } => Some(self.verb.unwrap_or(match to {
                Zone::Deck(_) if bottom => Verb::Bottomdeck,
                Zone::Deck(_) if under > 0 => Verb::Insert,
                Zone::Deck(_) => Verb::Topdeck,
                Zone::Mat(_, crate::Mat::Exile) => Verb::Exile,
                Zone::SetAside(_) => Verb::SetAside,
                Zone::Looking(_) => Verb::Look,
                Zone::Pile(_) => Verb::Return,
                _ => Verb::Put,
            })),
            _ => None,
        };
        self.journal_push(Step::Trace {
            entry: entry.clone(),
            verb,
        });
    }

    /// Journals cards rearranged within `zone` with no trace entry of their own: each of
    /// `cards` in turn left it (from its top, when `top`) and came back at `pos`.
    pub(crate) fn journal_within(
        &mut self,
        verb: Verb,
        zone: Zone,
        cards: &[CardId],
        top: bool,
        pos: Pos,
    ) {
        if self.journal.is_none() {
            return;
        }
        self.sync_journal();
        for &card in cards {
            self.journal_push(Step::Trace {
                entry: Trace::Moved {
                    card,
                    from: zone,
                    to: zone,
                    top,
                    bottom: pos == Pos::Bottom,
                    under: match pos {
                        Pos::Under(n) => n,
                        _ => 0,
                    },
                },
                verb: Some(verb),
            });
        }
    }

    pub(crate) fn journal_played(&mut self, player: PlayerId, card: CardId) {
        if self.journal.is_some() {
            self.journal_push(Step::Played { player, card });
        }
    }

    pub(crate) fn journal_logged(&mut self, index: usize) {
        if self.journal.is_some() {
            self.sync_journal();
            self.journal_push(Step::Logged { index });
        }
    }

    /// Records the turn [`Game::turn`] was just made.
    pub(crate) fn journal_turn(&mut self) {
        let limits = self.turn_limits();
        let Some(journal) = &mut self.journal else {
            return;
        };
        let p = self.turn.player;
        let kind = match self.turn.extra {
            Some(source) => TurnKind::Extra {
                source,
                id: journal.taking.take().unwrap_or(0),
            },
            None if self.ending.is_some() => TurnKind::Final,
            None => TurnKind::Regular,
        };
        journal.steps.push(Step::Turn {
            player: p,
            number: self.players[p.index()].turns_taken + 1,
            kind,
            limits,
        });
        for (gauge, value) in &mut journal.readings {
            match gauge {
                Gauge::Actions | Gauge::Buys => *value = 1,
                Gauge::NextHand => *value = 5,
                Gauge::Coins | Gauge::Potions | Gauge::Bought | Gauge::HandPlays => *value = 0,
                _ => {}
            }
        }
    }

    /// What the turn in progress forbids beyond the rules of every turn.
    pub fn turn_limits(&self) -> TurnLimits {
        let extra = self.turn.extra;
        TurnLimits {
            no_card_buys: extra == Some(Card::Mission),
            hand_plays: (extra == Some(Card::Voyage))
                .then_some(crate::cards::allies::VOYAGE_HAND_PLAYS),
        }
    }

    /// Enters `phase` of the turn in progress.
    pub(crate) fn set_phase(&mut self, phase: Phase) {
        if self.turn.phase == phase {
            return;
        }
        self.sync_journal();
        self.turn.phase = phase;
        self.journal_push(Step::Phase(phase));
    }

    /// Ends the turn in progress: between turns, nobody's turn is.
    pub(crate) fn turn_over(&mut self) {
        self.sync_journal();
        self.turn.phase = Phase::End;
        let player = self.turn.player;
        if let Some(journal) = &mut self.journal {
            journal.steps.push(Step::TurnEnded { player });
            for (gauge, value) in &mut journal.readings {
                if gauge.of_turn() {
                    *value = 0;
                }
            }
        }
    }

    /// Queues an extra turn for `player` from `source`, at the front of those waiting if
    /// `front`.
    pub(crate) fn queue_extra_turn(
        &mut self,
        player: PlayerId,
        source: Card,
        limit: ExtraLimit,
        lasting: Option<u32>,
        front: bool,
    ) {
        let mut turn = ExtraTurn {
            player,
            source,
            limit,
            lasting,
            id: 0,
        };
        if let Some(journal) = &mut self.journal {
            turn.id = journal.next_extra;
            journal.next_extra += 1;
        }
        if self.journal.is_some() {
            self.sync_journal();
            self.journal_push(Step::ExtraTurnQueued {
                turn: turn.clone(),
                front,
            });
        }
        match front {
            true => self.extra_turns.push_front(turn),
            false => self.extra_turns.push_back(turn),
        }
    }

    /// Records that the extra turn with `id` begins next.
    pub(crate) fn journal_taking(&mut self, id: u32) {
        if let Some(journal) = &mut self.journal {
            journal.taking = Some(id);
        }
    }

    pub(crate) fn journal_extra_turn_removed(&mut self, id: u32) {
        self.journal_push(Step::ExtraTurnRemoved { id });
    }

    pub(crate) fn journal_skipped(&mut self, player: PlayerId, kind: TurnKind, lost: bool) {
        if self.journal.is_some() {
            self.sync_journal();
            self.journal_push(Step::TurnSkipped { player, kind, lost });
        }
    }

    pub(crate) fn journal_game_ending(&mut self) {
        if let (Some(_), Some(reason)) = (&self.journal, self.ending) {
            self.sync_journal();
            let final_round = self.final_round.iter().copied().collect();
            self.journal_push(Step::GameEnding {
                reason,
                final_round,
            });
        }
    }

    /// Records that lasting effect `id`, just removed, ended because of `why`. Sync the
    /// journal before removing it, so that what changed before is recorded before.
    pub(crate) fn journal_effect_end(&mut self, id: u32, why: EffectEnd) {
        if let Some(journal) = &mut self.journal {
            journal.effects.retain(|e| e.id != id);
            journal.steps.push(Step::EffectEnded { id, why });
        }
    }

    /// Records that `card`, just moved into a set-aside zone or `Looking`, is face up.
    pub(crate) fn journal_face_up(&mut self, card: CardId) {
        self.journal_push(Step::FaceUp(card));
    }

    /// Set once the game has ended, while the round after its end is played (Fleet).
    pub fn ending(&self) -> Option<EndReason> {
        self.ending
    }

    /// The turns of the round after the game's end still to be taken, in order.
    pub fn final_round(&self) -> &VecDeque<PlayerId> {
        &self.final_round
    }

    /// The lasting effects as the table shows them.
    pub fn effects(&self) -> Vec<EffectState> {
        let aside = |c: &CardId| matches!(self.zones[c.index()], Zone::SetAside(_));
        (self.lasting.iter())
            .map(|l| {
                let mut holds: Vec<CardId> = vec![];
                for &c in l.data.pick.iter().chain(&l.data.cards) {
                    if aside(&c) && !holds.contains(&c) {
                        holds.push(c);
                    }
                }
                EffectState {
                    id: l.id,
                    card: l.card,
                    source: l.source,
                    owner: l.owner,
                    until: crate::cards::lasting_ends(self, l),
                    ending: l.ending,
                    keeps: l.keeps.clone(),
                    holds,
                    tokens: crate::cards::lasting_tokens(l),
                }
            })
            .collect()
    }

    /// Records the gauges and the lasting effects that changed since they were last
    /// recorded.
    pub(crate) fn sync_journal(&mut self) {
        if self.journal.is_none() {
            return;
        }
        let effects = self.effects();
        let now = self.gauges();
        let journal = self.journal.as_mut().expect("checked above");
        for old in &journal.effects {
            if !effects.iter().any(|e| e.id == old.id) {
                journal.steps.push(Step::EffectEnded {
                    id: old.id,
                    why: EffectEnd::Resolved,
                });
            }
        }
        for e in &effects {
            match journal.effects.iter().find(|old| old.id == e.id) {
                None => journal.steps.push(Step::EffectBegan(e.clone())),
                Some(old) if old != e => journal.steps.push(Step::EffectChanged(e.clone())),
                Some(_) => {}
            }
        }
        journal.effects = effects;
        for &(gauge, value) in &now {
            let was = (journal.readings.iter())
                .find(|(g, _)| *g == gauge)
                .map_or(0, |&(_, v)| v);
            if was != value {
                journal.steps.push(Step::Gauge {
                    gauge,
                    was,
                    now: value,
                });
            }
        }
        journal.readings = now;
    }

    fn journal_push(&mut self, step: Step) {
        if let Some(journal) = &mut self.journal {
            journal.steps.push(step);
        }
    }

    fn gauges(&self) -> Vec<(Gauge, i64)> {
        let turn = &self.turn;
        // Between turns the turn's counters are gone.
        let of_turn = |v: i64| if turn.phase == Phase::End { 0 } else { v };
        let hand_plays = match self.turn_limits().hand_plays {
            Some(_) => i64::from(turn.hand_plays),
            None => 0,
        };
        let mut out = vec![
            (Gauge::Actions, of_turn(i64::from(turn.actions))),
            (Gauge::Buys, of_turn(i64::from(turn.buys))),
            (Gauge::Coins, of_turn(i64::from(turn.coins))),
            (Gauge::Potions, of_turn(i64::from(turn.potions))),
            (Gauge::NextHand, of_turn(turn.cleanup_draw as i64)),
            (Gauge::Bought, of_turn(i64::from(turn.bought))),
            (Gauge::HandPlays, of_turn(hand_plays)),
            (Gauge::TradeRouteMat, i64::from(self.trade_route_mat)),
            (
                Gauge::Sun,
                self.prophecy.map_or(0, |(_, suns)| i64::from(suns)),
            ),
        ];
        for (i, p) in self.players.iter().enumerate() {
            let id = PlayerId(i as u8);
            out.extend([
                (Gauge::Coffers(id), i64::from(p.coffers)),
                (Gauge::Villagers(id), i64::from(p.villagers)),
                (Gauge::Favors(id), i64::from(p.favors)),
                (Gauge::Debt(id), i64::from(p.debt)),
                (Gauge::VpTokens(id), i64::from(p.vp_tokens)),
                (Gauge::PirateShip(id), i64::from(p.pirate_ship)),
                (Gauge::SkipTurns(id), i64::from(p.skip_turns)),
                (Gauge::TurnsSkipped(id), i64::from(p.turns_skipped)),
            ]);
        }
        for (i, pile) in self.piles.iter().enumerate() {
            let id = PileId(i as u8);
            out.extend([
                (Gauge::PileVp(id), i64::from(pile.vp_tokens)),
                (Gauge::PileDebt(id), i64::from(pile.debt_tokens)),
                (Gauge::Embargo(id), i64::from(pile.embargo)),
                (Gauge::TradeRoute(id), i64::from(pile.trade_route)),
            ]);
        }
        out
    }
}
