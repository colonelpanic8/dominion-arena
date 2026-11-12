//! The table this browser sits at, and what the lobby remembers between visits, in
//! localStorage (a convenience: nothing here is a setting).

use super::rules::TableRules;
use crate::platform;
use dominion_protocol::api::SeatToken;
use serde::{Deserialize, Serialize};

const TABLE: &str = "dominion.table";
const NAME: &str = "dominion.name";
const LAST_KINGDOM: &str = "dominion.lastKingdom";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyTable {
    pub rules: TableRules,
    /// The game it started last.
    #[serde(default)]
    pub game: Option<String>,
    /// That game's seats played elsewhere, with their tokens to hand out.
    #[serde(default)]
    pub links: Vec<SeatToken>,
}

pub fn load() -> Option<MyTable> {
    platform::storage_get(TABLE).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn save(table: Option<&MyTable>) {
    let text = table
        .and_then(|t| serde_json::to_string(t).ok())
        .unwrap_or_default();
    platform::storage_set(TABLE, &text);
}

/// The name you played under last.
pub fn name() -> String {
    platform::storage_get(NAME).unwrap_or_default()
}

pub fn set_name(name: &str) {
    platform::storage_set(NAME, name.trim());
}

/// The kingdom of the last game started here, as kingdom text.
pub fn last_kingdom() -> Option<String> {
    platform::storage_get(LAST_KINGDOM).filter(|s| !s.is_empty())
}

pub fn set_last_kingdom(code: &str) {
    platform::storage_set(LAST_KINGDOM, code);
}
