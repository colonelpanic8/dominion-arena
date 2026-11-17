//! An opt-in, ordered record of everything that changes on the table: what the
//! `dominion/events@1` stream is written from. [`Game::new_journaled`] turns it on.
//!
//! It holds the [`Trace`] entries (every card created, moved, mixed and shown) with the
//! rules' word for each move, the log entries in their place among them, and every
//! counter as it changes: counters are read whenever something is recorded, so a change
//! lies between the same two card moves it happened between.
//!
//! It also says what caused what. An occurrence (a turn, a phase, a play, a buy, an
//! ability resolving, a gain, trash, discard or shuffle) is marked [`Step::Occurred`] right
//! after the step recording it, and stays open for as long as the frame it belongs to on
//! the effect stack; [`Step::Within`] says which open occurrence the steps after it happen
//! inside. Counters are read whenever one opens or closes, so a change is put inside the
//! occurrence it happened in.

use crate::game::{Phase, Pos};
use crate::{Card, CardId, Game, Occasion, PileId, PlayerId, Trace, Zone};
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
    /// The step before this one is occurrence `n` (numbered from 0 in the order marked).
    Occurred(u32),
    /// The steps from here on happen inside occurrence `n`, or inside none: before the
    /// first turn, between turns.
    Within(Option<u32>),
    /// A triggered ability of `card` begins to resolve for `player`, answering
    /// `occasion`. `origin` is the occurrence that made the lasting effect it comes from.
    Resolve {
        player: PlayerId,
        card: Card,
        occasion: Occasion,
        origin: Option<u32>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Journal {
    steps: Vec<Step>,
    /// What the gauges read when last recorded, in [`Game::gauges`]' order.
    readings: Vec<(Gauge, i64)>,
    phase: Phase,
    /// Occurrences marked so far.
    #[serde(default)]
    occurrences: u32,
    /// The occurrences open, innermost last.
    #[serde(default)]
    open: Vec<Open>,
    /// What the last [`Step::Within`] said.
    #[serde(default)]
    within: Option<u32>,
    /// The Turn frame's depth on the stack and the turn's occurrence.
    #[serde(default)]
    turn: Option<(usize, u32)>,
    /// Each lasting effect, by id, with the occurrence that made it.
    #[serde(default)]
    lastings: Vec<(u32, u32)>,
}

/// An occurrence open while the frame at `depth` on the stack is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Open {
    depth: usize,
    occ: u32,
    /// The card it is about, for occurrences that happen together (cards trashed at once).
    card: Option<CardId>,
}

impl Journal {
    fn innermost(&self) -> Option<u32> {
        self.open.last().map(|o| o.occ)
    }

    /// Pushes `step`, which happens inside `cause`.
    fn push(&mut self, step: Step, cause: Option<u32>) {
        if cause != self.within {
            self.within = cause;
            self.steps.push(Step::Within(cause));
        }
        self.steps.push(step);
    }

    /// Marks the step just pushed as an occurrence.
    fn occur(&mut self) -> u32 {
        let n = self.occurrences;
        self.occurrences += 1;
        self.steps.push(Step::Occurred(n));
        n
    }
}

impl Game {
    /// Starts a game that records a journal (and a [`Trace`]), beginning with every card
    /// in it.
    pub fn new_journaled(config: crate::GameConfig) -> Result<Game, crate::SetupError> {
        let mut game = crate::setup::build(config)?;
        game.enable_journal();
        game.start();
        Ok(game)
    }

    /// Starts journaling a game not yet started, beginning with every card in it.
    pub(crate) fn enable_journal(&mut self) {
        self.enable_trace();
        let steps = (self.trace().unwrap_or_default().iter().cloned())
            .map(|entry| Step::Trace { entry, verb: None })
            .collect();
        // What setup gave (starting Favors, tokens on piles) is journaled after the cards.
        let turn = |g: &Gauge| {
            matches!(
                g,
                Gauge::Actions | Gauge::Buys | Gauge::Coins | Gauge::Potions
            )
        };
        self.journal = Some(Journal {
            steps,
            readings: (self.gauges().into_iter())
                .filter(|(g, _)| turn(g))
                .collect(),
            phase: self.turn.phase,
            ..Journal::default()
        });
        self.sync_journal();
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

    /// Journals `player` playing `card`, an occurrence open until its frame finishes.
    pub(crate) fn journal_played(&mut self, player: PlayerId, card: CardId) {
        if self.journal.is_some() {
            self.sync_journal();
            self.journal_push(Step::Played { player, card });
            self.journal_opens(Some(card));
        }
    }

    pub(crate) fn journal_logged(&mut self, index: usize) {
        if self.journal.is_some() {
            self.sync_journal();
            self.journal_push(Step::Logged { index });
        }
    }

    /// Records the turn [`Game::turn`] was just made, for the Turn frame about to be
    /// called.
    pub(crate) fn journal_turn(&mut self) {
        let depth = self.stack.len() + 1;
        let Some(journal) = &mut self.journal else {
            return;
        };
        let p = self.turn.player;
        let cause = journal.innermost();
        journal.push(
            Step::Turn {
                player: p,
                number: self.players[p.index()].turns_taken + 1,
                extra: (self.turn.extra).or(self.ending.is_some().then_some(Card::Fleet)),
            },
            cause,
        );
        let occ = journal.occur();
        journal.open.push(Open {
            depth,
            occ,
            card: None,
        });
        journal.turn = Some((depth, occ));
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
            // A phase the turn moves on to is the turn's; one an effect returns to
            // (Villa) is that effect's.
            let cause = match (journal.turn, journal.open.last()) {
                (Some((depth, turn)), Some(o)) if o.depth == depth => Some(turn),
                _ => journal.innermost(),
            };
            journal.push(Step::Phase(phase), cause);
            let occ = journal.occur();
            if let Some((depth, _)) = journal.turn
                && let Some(o) = journal.open.iter_mut().find(|o| o.depth == depth)
            {
                o.occ = occ;
            }
        }
        let cause = journal.innermost();
        for &(gauge, value) in &now {
            let was = (journal.readings.iter())
                .find(|(g, _)| *g == gauge)
                .map_or(0, |&(_, v)| v);
            if was != value {
                journal.push(
                    Step::Gauge {
                        gauge,
                        was,
                        now: value,
                    },
                    cause,
                );
            }
        }
        journal.readings = now;
    }

    fn journal_push(&mut self, step: Step) {
        if let Some(journal) = &mut self.journal {
            let cause = journal.innermost();
            journal.push(step, cause);
        }
    }

    /// Moves the turn on to `phase`.
    pub(crate) fn set_phase(&mut self, phase: Phase) {
        self.sync_journal();
        self.turn.phase = phase;
        self.sync_journal();
    }

    /// Marks the step just journaled as an occurrence, returning its number.
    pub(crate) fn journal_occurred(&mut self) -> Option<u32> {
        self.journal.as_mut().map(Journal::occur)
    }

    /// Opens occurrence `occ`, about `card`, for the frame running now.
    pub(crate) fn journal_open(&mut self, occ: Option<u32>, card: Option<CardId>) {
        let depth = self.stack.len();
        if let (Some(journal), Some(occ)) = (&mut self.journal, occ) {
            journal.open.push(Open { depth, occ, card });
        }
    }

    /// Marks the step just journaled as an occurrence and opens it for the frame running
    /// now.
    pub(crate) fn journal_opens(&mut self, card: Option<CardId>) {
        let occ = self.journal_occurred();
        self.journal_open(occ, card);
    }

    /// Closes the occurrences of the frames at `depth` and above: the frame there has
    /// finished, or (for a window) the ability it ran has.
    pub(crate) fn journal_close(&mut self, depth: usize) {
        let open = |g: &Game| {
            (g.journal.as_ref()).is_some_and(|j| j.open.last().is_some_and(|o| o.depth >= depth))
        };
        if !open(self) {
            return;
        }
        // What changed was done inside them.
        self.sync_journal();
        let journal = self.journal.as_mut().expect("checked above");
        journal.open.retain(|o| o.depth < depth);
        if journal.turn.is_some_and(|(d, _)| d >= depth) {
            journal.turn = None;
        }
    }

    /// Journals a triggered ability beginning to resolve for the window running now, open
    /// until [`Game::journal_close`] is called for that window.
    pub(crate) fn journal_resolve(
        &mut self,
        player: PlayerId,
        card: Card,
        occasion: &Occasion,
        lasting: Option<u32>,
    ) {
        if self.journal.is_none() {
            return;
        }
        self.sync_journal();
        let journal = self.journal.as_mut().expect("checked above");
        // Of cards trashed or discarded together, the one it answers.
        let subject = match *occasion {
            Occasion::Trash { card, .. } | Occasion::Discard { card, .. } => Some(card),
            _ => None,
        };
        let depth = journal.open.last().map(|o| o.depth);
        let cause = (journal.open.iter().rev())
            .take_while(|o| Some(o.depth) == depth)
            .find(|o| subject.is_some() && o.card == subject)
            .map(|o| o.occ)
            .or(journal.innermost());
        let origin = lasting.and_then(|id| {
            (journal.lastings.iter())
                .find(|&&(l, _)| l == id)
                .map(|&(_, occ)| occ)
        });
        journal.push(
            Step::Resolve {
                player,
                card,
                occasion: occasion.clone(),
                origin,
            },
            cause,
        );
        self.journal_opens(None);
    }

    /// Records the occurrence lasting effect `id` comes from: the one `from` comes from,
    /// for a later part of it, else the one it was made inside.
    pub(crate) fn journal_lasting(&mut self, id: u32, from: Option<u32>) {
        let Some(journal) = &mut self.journal else {
            return;
        };
        let origin = from
            .and_then(|from| {
                (journal.lastings.iter())
                    .find(|&&(l, _)| l == from)
                    .map(|&(_, occ)| occ)
            })
            .or(journal.innermost());
        if let Some(occ) = origin {
            journal.lastings.push((id, occ));
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
