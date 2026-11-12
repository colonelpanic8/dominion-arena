//! The game page's model: one seat's latest view, the game's change counter and the log
//! accumulated across fetches, plus the rules for turning a selection of options into an
//! answer. Plain data, so it is tested natively against real games.

use crate::api::ApiError;
use crate::cards::CardDb;
use dominion_protocol::api::{Change, SeatView};
use dominion_protocol::{AgentDecision, AgentView, Answer, DecisionGroup, LogEntry};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameState {
    /// The latest view; its `since_last_decision` is whatever the response carried; read
    /// the whole log from `log`.
    pub view: Option<AgentView>,
    pub version: u64,
    /// The seat deciding as of `version`, if any.
    pub deciding: Option<u8>,
    /// What the table shows beside the view, as of `version`.
    pub table: dominion_protocol::api::TableExtras,
    /// Every log entry from seq 0, without gaps.
    pub log: Vec<LogEntry>,
    /// The message of the last failed call, until the next success.
    pub error: Option<String>,
    /// An answer is on its way.
    pub answering: bool,
}

/// What to do after applying a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    Done,
    /// The response's log started after the end of ours: fetch from [`GameState::log_from`].
    Refetch,
}

impl GameState {
    /// The `logFrom` to ask for next: the seq after the last one held.
    pub fn log_from(&self) -> usize {
        self.log.last().map_or(0, |e| e.seq + 1)
    }

    /// Appends entries in order, skipping those already held. Stops at a gap rather
    /// than leave one, returning false.
    pub fn merge_log(&mut self, entries: &[LogEntry]) -> bool {
        for e in entries {
            let next = self.log_from();
            if e.seq < next {
                continue;
            }
            if e.seq > next {
                return false;
            }
            self.log.push(e.clone());
        }
        true
    }

    /// Takes in a view response (a fetch, or the answer to an answer). A view older than
    /// the one held only contributes its log.
    pub fn apply(&mut self, sv: SeatView) -> Applied {
        let complete = self.merge_log(&sv.view.since_last_decision);
        if self.view.is_none() || sv.version >= self.version {
            self.version = sv.version;
            self.view = Some(sv.view);
            self.deciding = sv.deciding;
            self.table = sv.table;
        }
        self.error = None;
        if complete {
            Applied::Done
        } else {
            Applied::Refetch
        }
    }

    /// Whether a change event means the view must be fetched again.
    pub fn is_news(&self, change: Change) -> bool {
        self.view.is_none() || change.version != self.version
    }

    /// Records a failed answer; true if the view is stale and must be fetched again.
    pub fn answer_failed(&mut self, e: &ApiError) -> bool {
        self.answering = false;
        self.error = Some(e.message.clone());
        e.is_stale()
    }

    pub fn decision(&self) -> Option<&AgentDecision> {
        self.view.as_ref()?.decision.as_ref()
    }
}

fn bounds(d: &AgentDecision) -> (u32, u32) {
    (d.min.unwrap_or(1), d.max.unwrap_or(1))
}

fn group_of(d: &AgentDecision, index: u32) -> Option<u32> {
    d.options.iter().find(|o| o.index == index).map(|o| o.group)
}

fn group(d: &AgentDecision, g: u32) -> Option<&DecisionGroup> {
    d.groups.iter().find(|x| x.group == g)
}

/// A decision of exactly one pick, which a click on an option answers.
pub fn answers_on_click(d: &AgentDecision) -> bool {
    bounds(d) == (1, 1) && d.groups.iter().filter(|g| g.min > 0).count() <= 1
}

/// The options picked for a decision, in the order picked: for an `ordered` decision
/// that order is the answer's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    picks: Vec<u32>,
}

impl Selection {
    pub fn new(picks: Vec<u32>) -> Selection {
        Selection { picks }
    }

    pub fn picks(&self) -> &[u32] {
        &self.picks
    }

    pub fn position(&self, index: u32) -> Option<usize> {
        self.picks.iter().position(|&p| p == index)
    }

    pub fn clear(&mut self) {
        self.picks.clear();
    }

    fn in_group(&self, d: &AgentDecision, g: u32) -> usize {
        self.picks
            .iter()
            .filter(|&&p| group_of(d, p) == Some(g))
            .count()
    }

    /// Picks or unpicks an option. A full group (or decision) of one swaps its pick;
    /// a full one of more ignores further picks.
    pub fn toggle(&mut self, d: &AgentDecision, index: u32) {
        if let Some(i) = self.position(index) {
            self.picks.remove(i);
        } else {
            self.add(d, index);
        }
    }

    fn add(&mut self, d: &AgentDecision, index: u32) -> bool {
        let Some(g) = group_of(d, index) else {
            return false;
        };
        if let Some(gr) = group(d, g)
            && self.in_group(d, g) >= gr.max as usize
        {
            if gr.max != 1 {
                return false;
            }
            self.picks.retain(|&p| group_of(d, p) != Some(g));
        }
        let (_, max) = bounds(d);
        if self.picks.len() >= max as usize {
            if max != 1 {
                return false;
            }
            self.picks.clear();
        }
        self.picks.push(index);
        true
    }

    /// Picks the next unpicked option of a target's interchangeable `options`, under the
    /// same limits as [`Selection::toggle`]. False if nothing was picked.
    pub fn click(&mut self, d: &AgentDecision, options: &[u32]) -> bool {
        match options.iter().find(|&&o| self.position(o).is_none()) {
            Some(&o) => self.add(d, o),
            None => false,
        }
    }

    /// Unpicks the option of `options` picked last.
    pub fn unclick(&mut self, options: &[u32]) -> bool {
        match self.picks.iter().rposition(|p| options.contains(p)) {
            Some(i) => {
                self.picks.remove(i);
                true
            }
            None => false,
        }
    }

    /// How many of `options` are picked.
    pub fn count(&self, options: &[u32]) -> usize {
        self.picks.iter().filter(|p| options.contains(p)).count()
    }

    /// The positions in the answer of the picked ones among `options`, in answer order.
    pub fn positions(&self, options: &[u32]) -> Vec<usize> {
        (0..self.picks.len())
            .filter(|&i| options.contains(&self.picks[i]))
            .collect()
    }

    /// Moves the pick at position `from` to position `to`.
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.picks.len() && to < self.picks.len() {
            let p = self.picks.remove(from);
            self.picks.insert(to, p);
        }
    }

    pub fn is_legal(&self, d: &AgentDecision) -> bool {
        let (min, max) = bounds(d);
        let n = self.picks.len() as u32;
        let mut seen = std::collections::BTreeSet::new();
        (min..=max).contains(&n)
            && self
                .picks
                .iter()
                .all(|&p| seen.insert(p) && group_of(d, p).is_some())
            && d.groups.iter().all(|g| {
                let k = self.in_group(d, g.group) as u32;
                (g.min..=g.max).contains(&k)
            })
    }

    /// The answer this selection gives, if it is a legal one.
    pub fn answer(&self, d: &AgentDecision) -> Option<Answer> {
        self.is_legal(d)
            .then(|| Answer::indices(d.id.clone(), self.picks.clone()))
    }
}

/// The answer naming `input`, for a decision that takes any card's name.
pub fn name_answer(d: &AgentDecision, cards: &CardDb, input: &str) -> Option<Answer> {
    if !d.name_any {
        return None;
    }
    cards
        .card_name(input.trim())
        .map(|name| Answer::name(d.id.clone(), name))
}
