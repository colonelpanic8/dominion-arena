//! Serde types for `dominion/agent-view@2`. Field names and shapes follow
//! `protocol/agent-view.schema.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "dominion/agent-view@2";

/// Cards by name, each with how many.
pub type CardCount = BTreeMap<String, u32>;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub schema: String,
    pub game: GameInfo,
    pub seat: Option<u8>,
    pub spectating: bool,
    pub turn: TurnView,
    pub you: Option<PlayerView>,
    pub opponents: Vec<PlayerView>,
    pub supply: Vec<PileView>,
    pub non_supply: Vec<PileView>,
    pub landscapes: Vec<LandscapeView>,
    pub trash: ZoneView,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertain: Vec<SharedUncertain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boons: Option<CardDeckView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hexes: Option<CardDeckView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lasting: Vec<LastingView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_turns: Vec<ExtraTurnView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<NoteView>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<String, i64>,
    pub decision: Option<AgentDecision>,
    pub since_last_decision: Vec<LogEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<GameResult>,
    #[serde(default)]
    pub kingdom: Vec<String>,
    #[serde(default)]
    pub landscapes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub players: Vec<ResultPlayer>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultPlayer {
    pub index: u8,
    pub name: String,
    pub vp: i32,
    pub rank: u32,
    pub turns: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub resigned: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_player: Option<String>,
    pub your_turn: bool,
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buys: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coins: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debt: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneView {
    pub count: u32,
    pub known: CardCount,
    pub unknown: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckView {
    pub count: u32,
    pub known: CardCount,
    pub unknown: u32,
    pub order_known: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Uncertain {
    pub zones: Vec<String>,
    pub count: u32,
    pub candidates: CardCount,
    pub outside: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedUncertain {
    pub zones: Vec<ZoneRef>,
    pub count: u32,
    pub candidates: CardCount,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pile: Option<String>,
    pub zone: String,
    pub count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owned {
    pub known: CardCount,
    pub unknown: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub coffers: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub villagers: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub favors: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub debt: u32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub vp_tokens: i32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pirate_ship: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub minus_card: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub minus_coin: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub journey_face_up: bool,
}

fn is_zero_i32(n: &i32) -> bool {
    *n == 0
}

impl Tokens {
    pub fn is_empty(&self) -> bool {
        *self == Tokens::default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerView {
    pub index: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vp: Option<i32>,
    pub hand: ZoneView,
    pub in_play: ZoneView,
    pub deck: DeckView,
    pub discard: ZoneView,
    pub other_zones: BTreeMap<String, ZoneView>,
    pub owned: Owned,
    pub uncertain: Vec<Uncertain>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unlocated: CardCount,
    #[serde(default, skip_serializing_if = "Tokens::is_empty")]
    pub tokens: Tokens,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adventure_tokens: BTreeMap<String, Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boons: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns_taken: Option<u32>,
    pub counters: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub coins: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub potions: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub debt: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PileTokens {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub embargo: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub trade_route: bool,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub vp: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub debt: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<PlayerToken>,
}

impl PileTokens {
    pub fn is_empty(&self) -> bool {
        *self == PileTokens::default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerToken {
    pub player: u8,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PileView {
    pub card: String,
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub kingdom: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "PileTokens::is_empty")]
    pub tokens: PileTokens,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traits: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LandscapeView {
    pub card: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vp: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardDeckView {
    pub count: u32,
    pub discard: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    pub card: String,
    pub holder: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastingView {
    pub card: String,
    pub owner: u8,
    pub until: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_player: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cards: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraTurnView {
    pub player: u8,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteView {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub count: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub seq: usize,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDecision {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    pub prompt: String,
    pub kind: String,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub ordered: bool,
    pub name_any: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    pub groups: Vec<DecisionGroup>,
    pub options: Vec<AgentOption>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionGroup {
    pub group: u32,
    pub kind: String,
    pub prompt: String,
    pub min: u32,
    pub max: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOption {
    pub index: u32,
    pub group: u32,
    pub action: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
}

/// An answer to a decision: option indices, or a card name when the decision allows
/// naming any card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indices: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Answer {
    pub fn indices(decision: impl Into<String>, indices: Vec<u32>) -> Answer {
        Answer {
            decision: decision.into(),
            indices: Some(indices),
            name: None,
        }
    }

    pub fn name(decision: impl Into<String>, name: impl Into<String>) -> Answer {
        Answer {
            decision: decision.into(),
            indices: None,
            name: Some(name.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub name: String,
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    pub text: String,
}
