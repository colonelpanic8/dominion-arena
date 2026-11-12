use crate::project::{self, Context};
use crate::text;
use dominion_engine::{Game, GameConfig, Knowledge, PlayerId, Response, SetupError};
use dominion_protocol::api::{PlayerTable, TableExtras};
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
}

/// A hosted game: the engine game, what each seat knows, decision ids and each seat's
/// log cursor.
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
        let mut knowledge = Knowledge::new(n);
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
        let view = self.game.view(seat);
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
        let (agent, targets) = project::decision(&self.game, &known, d, &open);
        let response =
            project::response(d, &agent, &targets, answer).map_err(AnswerError::Invalid)?;
        d.validate(&response)
            .map_err(|e| AnswerError::Invalid(e.to_string()))?;
        Ok(response)
    }

    /// Answers the open decision for `seat`.
    pub fn answer(&mut self, seat: PlayerId, answer: &Answer) -> Result<(), AnswerError> {
        let response = self.resolve(seat, answer)?;
        let log_len = self.game.log().len();
        self.game
            .respond(response)
            .map_err(|e| AnswerError::Invalid(e.to_string()))?;
        self.log_from[seat.index()] = log_len;
        self.knowledge.update(&mut self.game);
        self.seq += 1;
        Ok(())
    }
}
