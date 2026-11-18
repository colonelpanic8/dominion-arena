//! Ordered, resumable agent activity, separate from a game's public log.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptKind {
    /// The model's system prompt.
    System,
    /// Text given to the model as a user turn.
    Observation,
    Reasoning,
    Message,
    /// A tool call the model made: its name and arguments.
    Call,
    /// A friendly label for a game move the model made.
    Action,
    Game,
    /// What a tool call returned to the model.
    Result,
    Status,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptInput {
    pub attempt: String,
    pub offset: u64,
    pub time: u64,
    pub kind: TranscriptKind,
    /// Identifies a streamed block. A replacement supersedes its accumulated deltas.
    pub block: String,
    pub append: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub sequence: u64,
    pub seat: u8,
    pub agent: String,
    pub activity: TranscriptInput,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    pub entries: Vec<TranscriptEntry>,
    pub next: u64,
    pub more: bool,
}
