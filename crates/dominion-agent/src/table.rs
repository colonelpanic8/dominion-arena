use crate::events::Recorder;
use crate::project::{self, Context};
use crate::text;
use dominion_engine::{
    Card, CardId, DecisionKind, Game, GameConfig, Kind, Knowledge, Move, Opt, Phase, PlayerId,
    Purpose, Response, SetupError, Types, View,
};
use dominion_protocol::api::{InPlay, PlayerTable, TableExtras};
use dominion_protocol::events::{Event, History};
use dominion_protocol::{AgentView, Answer};
use serde::{Deserialize, Serialize};
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
    #[error("there is nothing to undo: no answer of yours can be taken back")]
    CannotUndo,
    /// The game no longer replays as it was played, so it can't go back.
    #[error("the game can't be undone: {0}")]
    Unreplayable(String),
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

/// One answer as the table took it, enough to replay it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Step {
    seat: PlayerId,
    /// The log's length before it, so that a replay that goes differently is caught.
    log: usize,
    act: Act,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Act {
    Answer {
        response: Response,
        way: Option<WayChoice>,
    },
    /// Autoplay Treasures.
    Treasures,
}

/// A hosted game: the engine game, what each seat and someone watching know, decision
/// ids and each seat's log cursor.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "Saved")]
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
    /// The seat that asked for each undo, in order.
    #[serde(default)]
    undone_by: Vec<PlayerId>,
    /// How many answers each undo took back (missing for undos saved before it was
    /// kept, which took back one).
    #[serde(default)]
    undone_answers: Vec<usize>,
    /// Per seat: how many undos there had been when they last answered.
    #[serde(default)]
    undos_seen: Vec<usize>,
    /// Every answer since the game began, to replay it to any earlier point.
    #[serde(default)]
    history: Vec<Step>,
    /// `history` holds every answer: a table saved before it was kept can't go back.
    #[serde(default)]
    complete: bool,
    /// Someone resigned: nothing before it can be undone.
    #[serde(default)]
    sealed: bool,
    /// Its `dominion/events@1` stream: none for a table that doesn't record (one saved
    /// before tables did).
    #[serde(rename = "recorder", skip_serializing_if = "Option::is_none")]
    recording: Option<Recording>,
    /// The last events recorded, until whoever keeps the stream says they are written.
    /// Saved, so that a table and its stream can always be brought back into step.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unwritten: Vec<Event>,
    /// Test hook: the recorder fails at this many records from now.
    #[serde(skip)]
    fault: Option<u32>,
}

/// A table's recording of its stream.
#[derive(Clone, Debug)]
enum Recording {
    On(Box<Recorder>),
    /// Stopped for good, and why: the game goes on, and its stream ends where it stopped.
    Stopped(String),
}

/// The version of what a recording table saves of its recorder and its game's journal.
/// Bump it whenever either changes what it keeps or what that means: a table saved with
/// another version stops recording when it is loaded rather than go on from state this
/// build would misread.
pub const RECORDING_VERSION: u32 = 2;

impl Serialize for Recording {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(2))?;
        match self {
            Recording::On(recorder) => {
                map.serialize_entry("version", &RECORDING_VERSION)?;
                map.serialize_entry("state", recorder)?;
            }
            Recording::Stopped(why) => map.serialize_entry("stopped", why)?,
        }
        map.end()
    }
}

impl Recording {
    /// A recording as saved: one of another version, or that this build can't read, has
    /// stopped.
    fn load(saved: serde_json::Value) -> Recording {
        if let Some(why) = saved.get("stopped").and_then(|w| w.as_str()) {
            return Recording::Stopped(why.into());
        }
        let Some(version) = saved.get("version").and_then(|v| v.as_u64()) else {
            return Recording::Stopped(
                "it was recorded by an earlier build, whose recording this one can't go on with"
                    .into(),
            );
        };
        if version != u64::from(RECORDING_VERSION) {
            return Recording::Stopped(format!(
                "it was recorded by a build whose recording is version {version}, which this \
                 one (version {RECORDING_VERSION}) can't go on with"
            ));
        }
        match Recorder::deserialize(&saved["state"]) {
            Ok(recorder) => Recording::On(Box::new(recorder)),
            Err(e) => Recording::Stopped(format!("its recorder can't be read: {e}")),
        }
    }
}

/// A table as saved. What only a recording table keeps is read leniently: what can't be
/// read stops its recording, never loses the game.
#[derive(Deserialize)]
struct Saved {
    id: String,
    game: serde_json::Value,
    knowledge: Knowledge,
    names: Vec<String>,
    seq: u64,
    log_from: Vec<usize>,
    #[serde(default)]
    ways: Vec<PendingWay>,
    #[serde(default)]
    undone_by: Vec<PlayerId>,
    #[serde(default)]
    undone_answers: Vec<usize>,
    #[serde(default)]
    undos_seen: Vec<usize>,
    #[serde(default)]
    history: Vec<Step>,
    #[serde(default)]
    complete: bool,
    #[serde(default)]
    sealed: bool,
    #[serde(default)]
    recorder: Option<serde_json::Value>,
    #[serde(default)]
    unwritten: Option<serde_json::Value>,
}

impl TryFrom<Saved> for Table {
    type Error = String;

    fn try_from(saved: Saved) -> Result<Table, String> {
        let mut recording = saved.recorder.map(Recording::load);
        let mut game = saved.game;
        let parsed = Game::deserialize(&game);
        let mut game = match parsed {
            Ok(game) => game,
            // A journal or chance this build can't read: the game without them.
            Err(e) => {
                let obj = game.as_object_mut().ok_or("the game is no object")?;
                if obj.remove("journal").is_none() & obj.remove("chance").is_none() {
                    return Err(e.to_string());
                }
                let game: Game = serde_json::from_value(game).map_err(|e| e.to_string())?;
                if let Some(Recording::On(_)) = recording {
                    recording = Some(Recording::Stopped(format!(
                        "its game's journal can't be read: {e}"
                    )));
                }
                game
            }
        };
        let mut unwritten = vec![];
        if let Some(Recording::On(_)) = recording {
            if !game.journaled() {
                recording = Some(Recording::Stopped("its game has no journal".into()));
            } else if let Some(saved) = saved.unwritten {
                match serde_json::from_value(saved) {
                    Ok(events) => unwritten = events,
                    Err(e) => {
                        recording = Some(Recording::Stopped(format!(
                            "its unwritten events can't be read: {e}"
                        )))
                    }
                }
            }
        }
        if !matches!(recording, Some(Recording::On(_))) {
            game.stop_journal();
        }
        Ok(Table {
            id: saved.id,
            game,
            knowledge: saved.knowledge,
            names: saved.names,
            seq: saved.seq,
            log_from: saved.log_from,
            ways: saved.ways,
            undone_by: saved.undone_by,
            undone_answers: saved.undone_answers,
            undos_seen: saved.undos_seen,
            history: saved.history,
            complete: saved.complete,
            sealed: saved.sealed,
            recording,
            unwritten,
            fault: None,
        })
    }
}

impl Table {
    /// Starts a game. `names` defaults to P1, P2, ... where missing.
    pub fn new(
        id: impl Into<String>,
        config: GameConfig,
        names: Vec<String>,
    ) -> Result<Table, SetupError> {
        Table::start(id.into(), config, names, false)
    }

    /// Starts a game that records its event stream ([`Table::unwritten`]).
    pub fn new_recorded(
        id: impl Into<String>,
        config: GameConfig,
        names: Vec<String>,
    ) -> Result<Table, SetupError> {
        Table::start(id.into(), config, names, true)
    }

    fn start(
        id: String,
        config: GameConfig,
        names: Vec<String>,
        record: bool,
    ) -> Result<Table, SetupError> {
        let mut game = match record {
            true => Game::new_journaled(config)?,
            false => Game::new_traced(config)?,
        };
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
        let mut table = Table {
            id,
            game,
            knowledge,
            names,
            seq: 1,
            log_from: vec![0; n],
            ways: vec![],
            undone_by: vec![],
            undone_answers: vec![],
            undos_seen: vec![0; n],
            history: vec![],
            complete: true,
            sealed: false,
            recording: record.then(|| Recording::On(Box::new(Recorder::new()))),
            unwritten: vec![],
            fault: None,
        };
        table.record();
        Ok(table)
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

    /// Tells `seat` which undos other seats asked for since `seat` last answered, and
    /// that the log it sees now replaces whatever it saw after them.
    pub fn undo_note(&self, seat: PlayerId) -> Option<String> {
        let seen = self.undos_seen.get(seat.index()).copied().unwrap_or(0);
        let mut counts: Vec<(PlayerId, usize)> = vec![];
        for (i, &p) in self.undone_by.iter().enumerate().skip(seen) {
            if p == seat {
                continue;
            }
            let n = self.undone_answers.get(i).copied().unwrap_or(1);
            match counts.iter_mut().find(|(q, _)| *q == p) {
                Some((_, total)) => *total += n,
                None => counts.push((p, n)),
            }
        }
        if counts.is_empty() {
            return None;
        }
        let who: Vec<String> = counts
            .iter()
            .map(|&(p, n)| {
                format!(
                    "{} undid {}",
                    self.names[p.index()],
                    dominion_engine::amount(n as i64, "answer", "answers")
                )
            })
            .collect();
        Some(format!(
            "Undo: since your last answer, {}. The game is as it was before what was taken \
             back; anything you saw after that point and no longer see in the log did not \
             happen. Decide from this view.",
            who.join(", and ")
        ))
    }

    /// How many answers the game has had (every seat's), counting from its start.
    pub fn answers(&self) -> usize {
        self.history.len()
    }

    /// Which of its histories the game has now: its answers and its undos, digested as
    /// the snapshot saves them. Two tables have the same one only if they took the same
    /// answers after the same undos, so it tells a stream built from this game as it
    /// stands from one built before an undo, however the game went on after it.
    pub fn history(&self) -> History {
        use sha2::{Digest, Sha256};
        use std::fmt::Write;
        #[derive(Serialize)]
        struct Kept<'a> {
            history: &'a [Step],
            undone_by: &'a [PlayerId],
            undone_answers: &'a [usize],
        }
        let bytes = serde_json::to_vec(&Kept {
            history: &self.history,
            undone_by: &self.undone_by,
            undone_answers: &self.undone_answers,
        })
        .expect("answers serialize");
        let digest = Sha256::digest(&bytes);
        History {
            answers: self.history.len() as u64,
            undos: self.undone_by.len() as u64,
            digest: digest.iter().fold(String::from("sha256:"), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            }),
        }
    }

    /// Whether the game can go back to an earlier point: it kept every answer, and
    /// nobody resigned.
    pub fn can_rewind(&self) -> bool {
        self.complete && !self.sealed
    }

    /// The number of `seat`'s last answer, counting every seat's from 0.
    pub fn last_answer(&self, seat: PlayerId) -> Option<usize> {
        self.history.iter().rposition(|s| s.seat == seat)
    }

    /// The number of the answer that led to log entry `seq`: undoing it removes the
    /// entry. None for entries from before the first answer.
    pub fn answer_for_log(&self, seq: usize) -> Option<usize> {
        self.history.iter().rposition(|s| s.log <= seq)
    }

    /// The seat that gave answer `answer`.
    pub fn answered_by(&self, answer: usize) -> Option<PlayerId> {
        self.history.get(answer).map(|s| s.seat)
    }

    /// Whether `seat` has an answer to take back.
    pub fn can_undo(&self, seat: PlayerId) -> bool {
        self.can_rewind() && self.last_answer(seat).is_some()
    }

    /// Takes back `seat`'s last answer, with every answer after it (other seats' too)
    /// and whatever followed automatically.
    pub fn undo(&mut self, seat: PlayerId) -> Result<(), AnswerError> {
        let answer = self.last_answer(seat).ok_or(AnswerError::CannotUndo)?;
        self.rewind(seat, answer)
    }

    /// Puts the game back as it was just before answer `answer`, replaying it from its
    /// start, for `by`, who asked.
    pub fn rewind(&mut self, by: PlayerId, answer: usize) -> Result<(), AnswerError> {
        if !self.can_rewind() || answer >= self.history.len() {
            return Err(AnswerError::CannotUndo);
        }
        let mut back = self.replayed(answer)?;
        // A decision id is never reused, so an answer sent for an undone one is stale.
        back.seq = self.seq + 1;
        back.undone_by = std::mem::take(&mut self.undone_by);
        back.undone_by.push(by);
        back.undone_answers = std::mem::take(&mut self.undone_answers);
        back.undone_answers.resize(back.undone_by.len() - 1, 1);
        back.undone_answers.push(self.history.len() - answer);
        back.undos_seen = std::mem::take(&mut self.undos_seen);
        if let Some(seen) = back.undos_seen.get_mut(by.index()) {
            *seen = back.undone_by.len();
        }
        *self = back;
        Ok(())
    }

    /// Whether the game can be replayed to any point: it kept every answer.
    pub fn replayable(&self) -> bool {
        self.complete
    }

    /// The game replayed from its start up to just before answer `answers`, going on from
    /// `from` when that is an earlier point of this game replayed so. It is only to look
    /// at: it never records, so whatever keeps this game's stream has nothing to write.
    pub fn replay(&self, answers: usize, from: Option<Table>) -> Result<Table, AnswerError> {
        if !self.complete || answers > self.history.len() {
            return Err(AnswerError::Unreplayable(
                "it doesn't keep that many answers".into(),
            ));
        }
        let from = from.filter(|t| {
            t.id == self.id
                && t.recording.is_none()
                && t.history.len() <= answers
                && self.history.starts_with(&t.history)
                && t.game.config() == self.game.config()
        });
        self.replayed_from(answers, from, false)
    }

    /// Each turn's start: how many answers there were when it was logged, whose turn it
    /// is and its number.
    pub fn turn_starts(&self) -> Vec<(usize, PlayerId, u32)> {
        (self.game.log().iter().enumerate())
            .filter_map(|(i, e)| match e {
                dominion_engine::Event::TurnStart { player, turn } => {
                    let answers = self.history.iter().filter(|s| s.log <= i).count();
                    Some((answers, *player, *turn))
                }
                _ => None,
            })
            .collect()
    }

    /// The game replayed from its start up to just before answer `answers`, to become the
    /// game: a recording table records it afresh, and one whose recording stopped stays
    /// stopped.
    fn replayed(&self, answers: usize) -> Result<Table, AnswerError> {
        let mut table = self.replayed_from(answers, None, self.records())?;
        if let Some(why) = self.recording_stopped() {
            table.recording = Some(Recording::Stopped(why.into()));
        }
        Ok(table)
    }

    /// Replays answers onto `from`, a table of this game that has taken a prefix of them
    /// and records as `record` says, or onto a new one.
    fn replayed_from(
        &self,
        answers: usize,
        from: Option<Table>,
        record: bool,
    ) -> Result<Table, AnswerError> {
        let fail = |why: String| AnswerError::Unreplayable(why);
        let mut table = match from {
            Some(t) => t,
            None => Table::start(
                self.id.clone(),
                self.game.config().clone(),
                self.names.clone(),
                record,
            )
            .map_err(|e| fail(e.to_string()))?,
        };
        let start = table.history.len();
        for (i, step) in self.history[..answers].iter().enumerate().skip(start) {
            if table.game.log().len() != step.log || table.deciding() != Some(step.seat) {
                return Err(fail(format!("answer {i} doesn't replay as it was given")));
            }
            let replayed = match &step.act {
                Act::Answer { response, way } => table
                    .respond_with(step.seat, response.clone(), *way)
                    .map(|_| ()),
                Act::Treasures => table.run_treasures(step.seat).map(|_| ()),
            };
            replayed.map_err(|e| fail(format!("answer {i}: {e}")))?;
            table.history.push(step.clone());
        }
        Ok(table)
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
                    in_play: p
                        .in_play()
                        .iter()
                        .filter_map(|&id| {
                            Some(InPlay {
                                card: name(id)?,
                                stays: view.stays_in_play(id),
                                holds: view.holds(id),
                            })
                        })
                        .collect(),
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

    /// The view of someone watching without a seat: every player's cards, with the
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
        self.record();
        self.ways.clear();
        self.sealed = true;
        Ok(())
    }

    /// The compact text rendering of `seat`'s view.
    pub fn text(&self, seat: PlayerId) -> String {
        text::render(&self.view(seat))
    }

    /// The engine response `answer` stands for, without playing it.
    pub fn resolve(&self, seat: PlayerId, answer: &Answer) -> Result<Response, AnswerError> {
        self.check_decision(seat, &answer.decision)?;
        let d = self.game.pending().expect("checked pending decision");
        let open = format!("d{}", self.seq);
        let known = self.knowledge.known(&self.game, seat);
        let (agent, targets) = project::decision(&self.game, &known, d, &open, &self.names);
        let response =
            project::response(d, &agent, &targets, answer).map_err(AnswerError::Invalid)?;
        d.validate(&response)
            .map_err(|e| AnswerError::Invalid(e.to_string()))?;
        Ok(response)
    }

    fn check_decision(&self, seat: PlayerId, decision: &str) -> Result<(), AnswerError> {
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
        Ok(())
    }

    /// Applies an engine response to the exact open decision, with normal knowledge,
    /// undo and pending Way bookkeeping.
    pub fn respond(
        &mut self,
        seat: PlayerId,
        decision: &str,
        response: Response,
    ) -> Result<(), AnswerError> {
        self.check_decision(seat, decision)?;
        self.game
            .pending()
            .expect("checked pending decision")
            .validate(&response)
            .map_err(|e| AnswerError::Invalid(e.to_string()))?;
        self.apply_response(seat, response, None).map(|_| ())
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
        self.apply_response(seat, response, way)
    }

    fn apply_response(
        &mut self,
        seat: PlayerId,
        response: Response,
        way: Option<WayChoice>,
    ) -> Result<Answered, AnswerError> {
        let log = self.game.log().len();
        let answered = self.respond_with(seat, response.clone(), way)?;
        self.history.push(Step {
            seat,
            log,
            act: Act::Answer { response, way },
        });
        Ok(answered)
    }

    fn respond_with(
        &mut self,
        seat: PlayerId,
        response: Response,
        way: Option<WayChoice>,
    ) -> Result<Answered, AnswerError> {
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
        self.answered(seat, log_len);
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
        Ok(self.answer_ways(seat))
    }

    /// Notes that `seat` answered when the log was `log_len` long.
    fn answered(&mut self, seat: PlayerId, log_len: usize) {
        self.log_from[seat.index()] = log_len;
        let undos = self.undone_by.len();
        if let Some(seen) = self.undos_seen.get_mut(seat.index()) {
            *seen = undos;
        }
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
        if self.treasure_plays(seat).is_empty() {
            return Err(AnswerError::Invalid(
                "no Treasures to play: the decision is not a Buy phase move offering one".into(),
            ));
        }
        let log = self.game.log().len();
        let played = self.run_treasures(seat)?;
        self.history.push(Step {
            seat,
            log,
            act: Act::Treasures,
        });
        Ok(played)
    }

    fn run_treasures(&mut self, seat: PlayerId) -> Result<u32, AnswerError> {
        let mut left = self.treasure_plays(seat);
        let log_len = self.game.log().len();
        let mut played = 0;
        while let Some(card) = self
            .treasure_plays(seat)
            .into_iter()
            .find(|c| left.contains(c))
        {
            left.retain(|&c| c != card);
            // Each play stands once made: a refused one ends the run there.
            match self.game_respond(Response::Move(Move::Play(card))) {
                Err(e) if played == 0 => return Err(e),
                Err(_) => break,
                Ok(()) => {}
            }
            self.answer_ways(seat);
            played += 1;
        }
        self.answered(seat, log_len);
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
    fn answer_ways(&mut self, seat: PlayerId) -> Answered {
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
            // Offered, so never refused; if it were, the question stays open.
            if self.game_respond(Response::option(w.choice.opt())).is_err() {
                self.ways.remove(i);
                break;
            }
            if w.seat == seat {
                answered.ways_answered += 1;
            }
        }
        answered
    }

    /// Answers the open decision: all or nothing, as the engine refuses an answer before
    /// changing anything, and recording can't fail.
    fn game_respond(&mut self, response: Response) -> Result<(), AnswerError> {
        self.game
            .respond(response)
            .map_err(|e| AnswerError::Invalid(e.to_string()))?;
        self.knowledge.update(&mut self.game);
        self.record();
        self.seq += 1;
        Ok(())
    }

    /// Turns what the game journaled since the last call into events. Every change to
    /// the game ends with it, so the journal never holds more than one change's steps.
    /// Recording is a side record: when the recorder fails, recording stops for good and
    /// the game goes on.
    fn record(&mut self) {
        let Some(Recording::On(recorder)) = &mut self.recording else {
            return;
        };
        let fault = match &mut self.fault {
            Some(0) => true,
            Some(n) => {
                *n -= 1;
                false
            }
            None => false,
        };
        let (game, names) = (&mut self.game, &self.names);
        let recorded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if fault {
                game.take_journal();
                return Err("a fault injected for a test".to_string());
            }
            recorder.record(game, names).map_err(|e| e.to_string())
        }));
        match recorded {
            Ok(Ok(events)) => self.unwritten.extend(events),
            Ok(Err(e)) => self.stop_recording(format!("its events can't be recorded: {e}")),
            Err(panic) => {
                let why = (panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic".into());
                self.stop_recording(format!("its recorder failed: {why}"))
            }
        }
    }

    /// Stops recording the stream for good, and says why: the game goes on, its stream
    /// ending where it was last written, and nothing more is recorded or written.
    pub fn stop_recording(&mut self, why: impl Into<String>) {
        if let Some(Recording::On(_)) = self.recording {
            self.recording = Some(Recording::Stopped(why.into()));
            self.unwritten.clear();
            self.game.stop_journal();
        }
    }

    /// Whether it records its stream now.
    pub fn records(&self) -> bool {
        matches!(self.recording, Some(Recording::On(_)))
    }

    /// Why its recording stopped, if it did: its stream is incomplete.
    pub fn recording_stopped(&self) -> Option<&str> {
        match &self.recording {
            Some(Recording::Stopped(why)) => Some(why),
            _ => None,
        }
    }

    /// Test hook: the recorder fails `after` records from now (0: the next).
    #[doc(hidden)]
    pub fn inject_recording_fault(&mut self, after: u32) {
        self.fault = Some(after);
    }

    /// What records its stream, while it does.
    pub fn recorder(&self) -> Option<&Recorder> {
        match &self.recording {
            Some(Recording::On(recorder)) => Some(recorder),
            _ => None,
        }
    }

    /// How many events the table has recorded, while it records its stream.
    pub fn recorded(&self) -> Option<u64> {
        self.recorder().map(|r| r.written())
    }

    /// How many of them have been written out: all but [`Table::unwritten`].
    pub fn written(&self) -> Option<u64> {
        Some(self.recorded()? - self.unwritten.len() as u64)
    }

    /// The events recorded since [`Table::mark_written`] was last called, in order. After
    /// an undo they are the whole stream of the game as it now stands, from event 0: the
    /// stream holds no undo, and what was taken back never happened.
    pub fn unwritten(&self) -> &[Event] {
        &self.unwritten
    }

    /// Notes that [`Table::unwritten`] have been written out.
    pub fn mark_written(&mut self) {
        self.unwritten.clear();
    }

    /// The table the recorded events fold to, while it records its stream.
    pub fn recorded_table(&self) -> Option<&dominion_protocol::events::Table> {
        self.recorder().map(Recorder::table)
    }

    /// Whether the table kept every answer since the game began.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// The table as it would be had it recorded its stream from the start: the game played
    /// again from its config with every answer it took, recording, and resigned as it was.
    /// The host's bookkeeping (decision ids, log cursors, undos) starts afresh. Whether
    /// it is the same game is for the caller to check.
    pub fn rerecorded(&self) -> Result<Table, Unreplayed> {
        let fail = |table: Option<Table>, answer, why: String| Unreplayed {
            table: table.map(Box::new),
            answer,
            why,
        };
        if !self.complete {
            return Err(fail(None, None, "it didn't keep every answer".into()));
        }
        let mut table = Table::start(
            self.id.clone(),
            self.game.config().clone(),
            self.names.clone(),
            true,
        )
        .map_err(|e| fail(None, None, format!("its setup fails: {e}")))?;
        for (i, step) in self.history.iter().enumerate() {
            if table.game.log().len() != step.log || table.deciding() != Some(step.seat) {
                let why = format!(
                    "{} was to answer after log entry {}; the game asks {} after {}",
                    step.seat,
                    step.log,
                    table.deciding().map_or("nobody".into(), |p| p.to_string()),
                    table.game.log().len(),
                );
                return Err(fail(Some(table), Some(i), why));
            }
            let replayed = match &step.act {
                Act::Answer { response, way } => table
                    .respond_with(step.seat, response.clone(), *way)
                    .map(|_| ()),
                Act::Treasures => table.run_treasures(step.seat).map(|_| ()),
            };
            if let Err(e) = replayed {
                return Err(fail(Some(table), Some(i), e.to_string()));
            }
            table.history.push(step.clone());
        }
        let resigned = self.game.outcome().map_or(&[][..], |o| &o.resigned[..]);
        for &seat in resigned {
            if let Err(e) = table.resign(seat) {
                return Err(fail(Some(table), None, format!("{seat} resigning: {e}")));
            }
        }
        Ok(table)
    }
}

/// Where [`Table::rerecorded`] stopped: the table as far as it got, the answer that
/// didn't replay (none: before the first, or after the last), and why.
#[derive(Debug)]
pub struct Unreplayed {
    pub table: Option<Box<Table>>,
    pub answer: Option<usize>,
    pub why: String,
}
