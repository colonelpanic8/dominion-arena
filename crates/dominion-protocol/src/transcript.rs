//! Ordered, resumable agent activity, separate from a game's public log.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptKind {
    Observation,
    Reasoning,
    Message,
    Action,
    Game,
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
