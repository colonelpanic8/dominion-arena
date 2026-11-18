//! A game as this client was told it: what its reducer was fed, in order, so that a
//! reducer fed the same again shows the game at any point of it.
use serde::{Deserialize, Serialize};

use crate::{Result, model, questions::Question, reducer::Reducer};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Input {
    /// Who the client is in game `game`, under official client `client`, and the
    /// players' names known so far. Written again whenever the client starts recording a
    /// game it was already recording (a reconnect after a restart).
    Begin {
        game: i64,
        client: String,
        player: i32,
        names: Vec<(i32, String)>,
    },
    Names {
        names: Vec<(i32, String)>,
    },
    Snapshot {
        snapshot: Box<model::Snapshot>,
    },
    Event {
        event: model::Event,
    },
    Question {
        question: Box<Question>,
    },
    Log {
        change: model::LogChange,
    },
    Finish {
        result: model::GameResult,
    },
    /// The open question was answered or withdrawn.
    Cleared,
    /// The end of what arrived together: one frame from the official server, or an
    /// answer.
    Frame,
}

impl Reducer {
    /// Takes `input` as it was taken when it was recorded.
    pub fn feed(&mut self, input: &Input) -> Result<()> {
        match input {
            Input::Begin { player, names, .. } => {
                self.player_id = *player;
                self.names.extend(names.iter().cloned());
            }
            Input::Names { names } => self.names.extend(names.iter().cloned()),
            Input::Snapshot { snapshot } => self.replace(model::Snapshot::clone(snapshot))?,
            Input::Event { event } => self.apply(event.clone())?,
            Input::Question { question } => self.ask(Question::clone(question))?,
            Input::Log { change } => self.log(change)?,
            Input::Finish { result } => self.finish(result)?,
            Input::Cleared => self.invalidate(),
            Input::Frame => {}
        }
        Ok(())
    }
}
