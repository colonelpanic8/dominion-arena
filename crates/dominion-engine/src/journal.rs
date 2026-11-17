//! An opt-in, ordered record of everything that changes on the table: what the
//! `dominion/events@1` stream is written from. [`Game::new_journaled`] turns it on.
//!
//! It holds the [`Trace`] entries (every card created, moved, mixed and shown) with the
//! rules' word for each move, the log entries in their place among them, and every
//! counter as it changes: counters are read whenever something is recorded, so a change
//! lies between the same two card moves it happened between.

use crate::game::{Phase, Pos};
use crate::{Card, CardId, Game, PileId, PlayerId, Trace, Zone};
use serde::{Deserialize, Serialize};

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
    /// A turn began: `player`'s `number`th, an extra one from `extra`. Its counters are 1
    /// Action, 1 Buy, $0 and no Potions, and its phase [`Phase::Start`].
    Turn {
        player: PlayerId,
        number: u32,
        extra: Option<Card>,
    },
    Phase(Phase),
    /// `gauge` changed from `was` to `now`.
    Gauge {
        gauge: Gauge,
        was: i64,
        now: i64,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Journal {
    steps: Vec<Step>,
    /// What the gauges read when last recorded, in [`Game::gauges`]' order.
    readings: Vec<(Gauge, i64)>,
    phase: Phase,
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
        let turn = |g: &Gauge| {
            matches!(
                g,
                Gauge::Actions | Gauge::Buys | Gauge::Coins | Gauge::Potions
            )
        };
        game.journal = Some(Journal {
            steps,
            readings: (game.gauges().into_iter())
                .filter(|(g, _)| turn(g))
                .collect(),
            phase: game.turn.phase,
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
        let Some(journal) = &mut self.journal else {
            return;
        };
        let p = self.turn.player;
        journal.steps.push(Step::Turn {
            player: p,
            number: self.players[p.index()].turns_taken + 1,
            extra: (self.turn.extra).or(self.ending.is_some().then_some(Card::Fleet)),
        });
        journal.phase = Phase::Start;
        for (gauge, value) in &mut journal.readings {
            match gauge {
                Gauge::Actions | Gauge::Buys => *value = 1,
                Gauge::Coins | Gauge::Potions => *value = 0,
                _ => {}
            }
        }
    }

    /// Records the gauges and the phase that changed since they were last recorded.
    pub(crate) fn sync_journal(&mut self) {
        if self.journal.is_none() {
            return;
        }
        let now = self.gauges();
        let phase = self.turn.phase;
        let journal = self.journal.as_mut().expect("checked above");
        if journal.phase != phase {
            journal.phase = phase;
            journal.steps.push(Step::Phase(phase));
        }
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
        let mut out = vec![
            (Gauge::Actions, i64::from(turn.actions)),
            (Gauge::Buys, i64::from(turn.buys)),
            (Gauge::Coins, i64::from(turn.coins)),
            (Gauge::Potions, i64::from(turn.potions)),
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
