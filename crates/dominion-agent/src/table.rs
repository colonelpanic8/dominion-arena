use crate::project::{self, Context};
use crate::text;
use dominion_engine::{
    Card, CardId, DecisionKind, Game, GameConfig, Kind, Knowledge, Move, Opt, Phase, PlayerId,
    Purpose, Response, SetupError, Types, View,
};
use dominion_protocol::api::{PlayerTable, TableExtras};
use dominion_protocol::{AgentView, Answer};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AnswerError {
    #[error("the game is over")]
    GameOver,
    #[error("{seat} is not deciding now ({deciding} is, decision {open})")]
    NotYourDecision {
        seat: PlayerId,
        deciding: PlayerId,
        open: String,
    },
    #[error("decision {given} is not open; the open decision is {open}")]
    StaleDecision { given: String, open: String },
    #[error("{0}")]
    Invalid(String),
    #[error("there is nothing to undo: another seat has answered since your last answer")]
    CannotUndo,
}

/// An answer to the Way question ("play it as itself, or use a Way instead") given in
/// advance, with the answer whose play would ask it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WayChoice {
    /// Play the card as itself.
    None,
    Way(Card),
}

impl WayChoice {
    /// `none`, or a Way's name (ignoring case, spaces and punctuation).
    pub fn parse(s: &str) -> Result<WayChoice, String> {
        if s.trim().eq_ignore_ascii_case("none") {
            return Ok(WayChoice::None);
        }
        Card::from_name(s)
            .filter(|c| c.kind() == Kind::Way)
            .map(WayChoice::Way)
            .ok_or_else(|| format!("way {s:?} is neither \"none\" nor the name of a Way"))
    }

    fn opt(self) -> Opt {
        match self {
            WayChoice::None => Opt::No,
            WayChoice::Way(w) => Opt::Card(w),
        }
    }
}

/// What a pre-answered Way did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Answered {
    /// Way questions answered with it.
    pub ways_answered: u32,
    /// Its Way, when a Way question came up that doesn't offer it; that question is open.
    pub way_not_offered: Option<Card>,
}

impl Answered {
    /// A line for the agent when its pre-answer was not used.
    pub fn note(&self) -> Option<String> {
        self.way_not_offered
            .map(|w| format!("{w} is not offered for this play, so the Way question is still open"))
    }
}

/// A Way pre-answer waiting for the Way questions of `card`'s play: kept while `seat` is
/// asked nothing else and the play goes on, while other seats decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PendingWay {
    seat: PlayerId,
    card: CardId,
    choice: WayChoice,
}

/// A hosted game: the engine game, what each seat and someone watching know, decision
/// ids and each seat's log cursor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    id: String,
    game: Game,
    knowledge: Knowledge,
    names: Vec<String>,
    /// Bumped whenever a new decision opens; the open decision's id is `d<seq>`.
    seq: u64,
    /// Per seat: the log length when they last answered.
    log_from: Vec<usize>,
    /// At most one per seat.
    #[serde(default)]
    ways: Vec<PendingWay>,
    /// The seat that took back each answer undone, in order.
    #[serde(default)]
    undone_by: Vec<PlayerId>,
    /// Per seat: how many answers had been undone when they last answered.
    #[serde(default)]
    undos_seen: Vec<usize>,
    /// The seat that answered last and the table before that answer, while no other
    /// seat has answered since. Not saved: a restarted server can't undo.
    #[serde(skip)]
    undo: Option<(PlayerId, Arc<Table>)>,
}

impl Table {
    /// Starts a game. `names` defaults to P1, P2, ... where missing.
    pub fn new(
        id: impl Into<String>,
        config: GameConfig,
        names: Vec<String>,
    ) -> Result<Table, SetupError> {
        let mut game = Game::new_traced(config)?;
        let n = game.num_players();
        let mut knowledge = Knowledge::with_watcher(n);
        knowledge.update(&mut game);
        let names = (0..n)
            .map(|i| {
                names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| PlayerId(i as u8).to_string())
            })
            .collect();
        Ok(Table {
            id: id.into(),
            game,
            knowledge,
            names,
            seq: 1,
            log_from: vec![0; n],
            ways: vec![],
            undone_by: vec![],
            undos_seen: vec![0; n],
            undo: None,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn game(&self) -> &Game {
        &self.game
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn knowledge(&self) -> &Knowledge {
        &self.knowledge
    }

    /// How many answers have been undone: the log may have changed behind any earlier
    /// count.
    pub fn undos(&self) -> u64 {
        self.undone_by.len() as u64
    }

    /// Tells `seat` which answers other seats took back since `seat` last answered, and
    /// that the log it sees now replaces whatever it saw after them.
    pub fn undo_note(&self, seat: PlayerId) -> Option<String> {
        let seen = self.undos_seen.get(seat.index()).copied().unwrap_or(0);
        let mut counts: Vec<(PlayerId, usize)> = vec![];
        for &p in self.undone_by.iter().skip(seen).filter(|&&p| p != seat) {
            match counts.iter_mut().find(|(q, _)| *q == p) {
                Some((_, n)) => *n += 1,
                None => counts.push((p, 1)),
            }
        }
        if counts.is_empty() {
            return None;
        }
        let who: Vec<String> = counts
            .iter()
            .map(|&(p, n)| {
                let answers = if n == 1 {
                    "their last answer".to_string()
                } else {
                    format!("their last {n} answers")
                };
                format!("{} took back {answers}", self.names[p.index()])
            })
            .collect();
        Some(format!(
            "Undo: since your last answer, {}. The game is as it was before what was taken \
             back; anything you saw after that point and no longer see in the log did not \
             happen. Decide from this view.",
            who.join(", and ")
        ))
    }

    /// Whether `seat` may undo: it answered last, and no other seat has answered since.
    pub fn can_undo(&self, seat: PlayerId) -> bool {
        self.undo.as_ref().is_some_and(|(s, _)| *s == seat)
    }

    /// Takes back `seat`'s last answer, with whatever followed it automatically. Its
    /// earlier answers can be taken back in turn, back to the last answer of another
    /// seat. False if `seat` can't undo.
    pub fn undo(&mut self, seat: PlayerId) -> bool {
        let Some((_, before)) = self.undo.take_if(|(s, _)| *s == seat) else {
            return false;
        };
        let seq = self.seq;
        let mut undone_by = std::mem::take(&mut self.undone_by);
        undone_by.push(seat);
        *self = Arc::unwrap_or_clone(before);
        // A decision id is never reused, so an answer sent for the undone one is stale.
        self.seq = seq + 1;
        if let Some(seen) = self.undos_seen.get_mut(seat.index()) {
            *seen = undone_by.len();
        }
        self.undone_by = undone_by;
        true
    }

    pub fn is_over(&self) -> bool {
        self.game.outcome().is_some()
    }

    /// The seat that must decide now, if any.
    pub fn deciding(&self) -> Option<PlayerId> {
        self.game.pending().map(|d| d.player)
    }

    /// The open decision's id, if any.
    pub fn decision_id(&self) -> Option<String> {
        self.game.pending().map(|_| format!("d{}", self.seq))
    }

    pub fn view(&self, seat: PlayerId) -> AgentView {
        self.view_from(seat, self.log_from[seat.index()])
    }

    /// `seat`'s view with the log from sequence number `log_from` on, instead of from
    /// the seat's last answer.
    pub fn view_from(&self, seat: PlayerId, log_from: usize) -> AgentView {
        let id = self.decision_id();
        let known = self.knowledge.known(&self.game, seat);
        project::view(
            &self.game,
            &known,
            seat,
            &Context {
                game_id: Some(&self.id),
                names: &self.names,
                decision_id: id.as_deref(),
                log_from,
            },
        )
    }

    /// What a graphical table shows beside `seat`'s view (play order, discard tops),
    /// read through its engine view so that only what it may see is named.
    pub fn extras(&self, seat: PlayerId) -> TableExtras {
        Self::extras_of(self.game.view(seat))
    }

    fn extras_of(view: View<'_>) -> TableExtras {
        let name = |id| view.card(id).map(|c| c.name().to_string());
        TableExtras {
            players: view
                .players()
                .map(|p| PlayerTable {
                    seat: p.id().0,
                    in_play: p.in_play().iter().filter_map(|&id| name(id)).collect(),
                    discard_top: p.discard_top().and_then(name),
                })
                .collect(),
        }
    }

    /// Whether the table keeps what someone watching knows (one saved before it did
    /// doesn't).
    pub fn watchable(&self) -> bool {
        self.knowledge.watches()
    }

    /// The view of someone watching without a seat: what every player may know, with the
    /// log from `log_from` on and no decision. `None` unless [`Table::watchable`].
    pub fn watch_view(&self, log_from: usize) -> Option<AgentView> {
        let known = self.knowledge.watched(&self.game)?;
        Some(project::watch_view(
            &self.game,
            &known,
            &Context {
                game_id: Some(&self.id),
                names: &self.names,
                decision_id: None,
                log_from,
            },
        ))
    }

    /// [`Table::extras`] for someone watching.
    pub fn watch_extras(&self) -> Option<TableExtras> {
        self.watchable().then(|| Self::extras_of(self.game.watch()))
    }

    /// Ends the game with `seat` resigning, for good: nothing before it can be undone.
    pub fn resign(&mut self, seat: PlayerId) -> Result<(), AnswerError> {
        if self.is_over() {
            return Err(AnswerError::GameOver);
        }
        if seat.index() >= self.names.len() {
            return Err(AnswerError::Invalid(format!("there is no seat {}", seat.0)));
        }
        self.game.resign(seat);
        self.ways.clear();
        self.undo = None;
        Ok(())
    }

    /// The compact text rendering of `seat`'s view.
    pub fn text(&self, seat: PlayerId) -> String {
        text::render(&self.view(seat))
    }

    /// The engine response `answer` stands for, without playing it.
    pub fn resolve(&self, seat: PlayerId, answer: &Answer) -> Result<Response, AnswerError> {
        let Some(d) = self.game.pending() else {
            return Err(AnswerError::GameOver);
        };
        let open = format!("d{}", self.seq);
        if d.player != seat {
            return Err(AnswerError::NotYourDecision {
                seat,
                deciding: d.player,
                open,
            });
        }
        if answer.decision != open {
            return Err(AnswerError::StaleDecision {
                given: answer.decision.clone(),
                open,
            });
        }
        let known = self.knowledge.known(&self.game, seat);
        let (agent, targets) = project::decision(&self.game, &known, d, &open, &self.names);
        let response =
            project::response(d, &agent, &targets, answer).map_err(AnswerError::Invalid)?;
        d.validate(&response)
            .map_err(|e| AnswerError::Invalid(e.to_string()))?;
        Ok(response)
    }

    /// Answers the open decision for `seat`.
    pub fn answer(&mut self, seat: PlayerId, answer: &Answer) -> Result<(), AnswerError> {
        self.answer_with(seat, answer, None).map(|_| ())
    }

    /// Answers the open decision for `seat`, then, with `way`, answers the Way questions
    /// of the card the answer plays and of its consecutive replays, also when other seats
    /// are asked something first (Reactions to an Attack). A Way that isn't in the game
    /// rejects the answer; one in the game but not offered for a play leaves that question
    /// open.
    pub fn answer_with(
        &mut self,
        seat: PlayerId,
        answer: &Answer,
        way: Option<WayChoice>,
    ) -> Result<Answered, AnswerError> {
        let response = self.resolve(seat, answer)?;
        let mut before = self.clone();
        if !before.can_undo(seat) {
            before.undo = None;
        }
        if let Some(WayChoice::Way(w)) = way
            && !self.game.landscapes().contains(&w)
        {
            let ways: Vec<&str> = self
                .game
                .landscapes()
                .iter()
                .filter(|l| l.kind() == Kind::Way)
                .map(|l| l.name())
                .collect();
            return Err(AnswerError::Invalid(match ways.as_slice() {
                [] => format!("{w} is not in this game, which has no Ways"),
                _ => format!("{w} is not in this game (its Ways: {})", ways.join(", ")),
            }));
        }
        let log_len = self.game.log().len();
        // The card whose Way questions it answers: the one played, a replayed one, or the
        // one an ability starts playing (a Reaction that plays itself).
        let played = match response {
            Response::Move(Move::Play(card)) => Some(card),
            _ => self.game.way_card(),
        };
        self.game_respond(response)?;
        self.log_from[seat.index()] = log_len;
        let undos = self.undone_by.len();
        if let Some(seen) = self.undos_seen.get_mut(seat.index()) {
            *seen = undos;
        }
        if let Some(choice) = way {
            let card = played.or_else(|| {
                (self.game.plays_in_progress())
                    .find(|&(p, _, since)| p == seat && since >= log_len)
                    .map(|(_, card, _)| card)
            });
            self.ways.retain(|w| w.seat != seat);
            self.ways
                .extend(card.map(|card| PendingWay { seat, card, choice }));
        }
        let answered = self.answer_ways(seat)?;
        self.undo = Some((seat, Arc::new(before)));
        Ok(answered)
    }

    /// "Autoplay Treasures": plays, one after another, the Treasures `seat` could play in
    /// the open Buy phase decision `decision`, while the seat is asked for a move in that
    /// phase. Cards that became playable meanwhile (drawn by Figurine) are left alone.
    /// One undo takes the whole run back. Returns how many were played.
    pub fn play_treasures(&mut self, seat: PlayerId, decision: &str) -> Result<u32, AnswerError> {
        let Some(d) = self.game.pending() else {
            return Err(AnswerError::GameOver);
        };
        let open = format!("d{}", self.seq);
        if d.player != seat {
            return Err(AnswerError::NotYourDecision {
                seat,
                deciding: d.player,
                open,
            });
        }
        if decision != open {
            return Err(AnswerError::StaleDecision {
                given: decision.to_string(),
                open,
            });
        }
        let mut left = self.treasure_plays(seat);
        if left.is_empty() {
            return Err(AnswerError::Invalid(
                "no Treasures to play: the decision is not a Buy phase move offering one".into(),
            ));
        }
        let mut before = self.clone();
        if !before.can_undo(seat) {
            before.undo = None;
        }
        let log_len = self.game.log().len();
        let mut played = 0;
        while let Some(card) = self
            .treasure_plays(seat)
            .into_iter()
            .find(|c| left.contains(c))
        {
            left.retain(|&c| c != card);
            self.game_respond(Response::Move(Move::Play(card)))?;
            self.answer_ways(seat)?;
            played += 1;
        }
        self.log_from[seat.index()] = log_len;
        let undos = self.undone_by.len();
        if let Some(seen) = self.undos_seen.get_mut(seat.index()) {
            *seen = undos;
        }
        self.undo = Some((seat, Arc::new(before)));
        Ok(played)
    }

    /// The Treasures `seat`'s open Buy phase decision offers to play.
    fn treasure_plays(&self, seat: PlayerId) -> Vec<CardId> {
        match self.game.pending() {
            Some(d) if d.player == seat => match &d.kind {
                DecisionKind::Turn {
                    phase: Phase::Buy,
                    moves,
                } => moves
                    .iter()
                    .filter_map(|m| match *m {
                        Move::Play(c) if self.game.is(c, Types::TREASURE) => Some(c),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            },
            _ => vec![],
        }
    }

    /// Answers the Way questions the pending pre-answers cover, dropping each once its
    /// seat is asked anything else or its card's play is over. What `seat`'s did.
    fn answer_ways(&mut self, seat: PlayerId) -> Result<Answered, AnswerError> {
        let mut answered = Answered::default();
        loop {
            let game = &self.game;
            self.ways.retain(|w| {
                game.pending().is_some()
                    && (game.plays_in_progress()).any(|(p, c, _)| p == w.seat && c == w.card)
            });
            let Some(d) = self.game.pending() else {
                break;
            };
            let Some(i) = self.ways.iter().position(|w| w.seat == d.player) else {
                break;
            };
            let w = self.ways[i];
            let offered = match &d.kind {
                DecisionKind::Choose { options, .. }
                    if d.purpose == Purpose::Way && self.game.way_card() == Some(w.card) =>
                {
                    Some(options.contains(&w.choice.opt()))
                }
                _ => None,
            };
            if offered != Some(true) {
                if let (Some(false), WayChoice::Way(card), true) =
                    (offered, w.choice, w.seat == seat)
                {
                    answered.way_not_offered = Some(card);
                }
                self.ways.remove(i);
                break;
            }
            self.game_respond(Response::option(w.choice.opt()))?;
            if w.seat == seat {
                answered.ways_answered += 1;
            }
        }
        Ok(answered)
    }

    fn game_respond(&mut self, response: Response) -> Result<(), AnswerError> {
        self.game
            .respond(response)
            .map_err(|e| AnswerError::Invalid(e.to_string()))?;
        self.knowledge.update(&mut self.game);
        self.seq += 1;
        Ok(())
    }
}
