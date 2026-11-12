//! What the lobby remembers between visits, in localStorage (conveniences: the table
//! itself is the server's, and nothing here is a setting).

use super::rules::TableRules;
use crate::platform;
use dominion_protocol::lobby::MatchRequest;
use serde::{Deserialize, Serialize};

const RULES: &str = "dominion.rules";
const LAST_KINGDOM: &str = "dominion.lastKingdom";
const HERE: &str = "dominion.here";
const SEARCH: &str = "dominion.search";
const ENTERED: &str = "dominion.entered";

/// The rules of the table hosted here last, to start the next one from.
pub fn rules() -> Option<TableRules> {
    platform::storage_get(RULES).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn set_rules(rules: &TableRules) {
    if let Ok(text) = serde_json::to_string(rules) {
        platform::storage_set(RULES, &text);
    }
}

/// The kingdom of the last game started from a table here, as kingdom text.
pub fn last_kingdom() -> Option<String> {
    platform::storage_get(LAST_KINGDOM).filter(|s| !s.is_empty())
}

pub fn set_last_kingdom(code: &str) {
    platform::storage_set(LAST_KINGDOM, code);
}

#[derive(Default, Serialize, Deserialize)]
struct Here {
    table: String,
    names: Vec<String>,
}

/// The token seats of `table` (by name) that its host plays at this screen; the others'
/// tokens are handed out.
pub fn here(table: &str) -> Vec<String> {
    platform::storage_get(HERE)
        .and_then(|s| serde_json::from_str::<Here>(&s).ok())
        .filter(|h| h.table == table)
        .map(|h| h.names)
        .unwrap_or_default()
}

pub fn set_here(table: &str, names: &[String]) {
    let here = Here {
        table: table.to_string(),
        names: names.to_vec(),
    };
    if let Ok(text) = serde_json::to_string(&here) {
        platform::storage_set(HERE, &text);
    }
}

/// What Automatch searched for last.
pub fn search() -> MatchRequest {
    platform::storage_get(SEARCH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn set_search(request: &MatchRequest) {
    if let Ok(text) = serde_json::to_string(request) {
        platform::storage_set(SEARCH, &text);
    }
}

/// Whether this tab has been taken to `game` when it started; says so from now on.
pub fn enter(game: &str) -> bool {
    let mut entered: Vec<String> = platform::session_get(ENTERED)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if entered.iter().any(|g| g == game) {
        return true;
    }
    entered.push(game.to_string());
    let keep = entered.len().saturating_sub(20);
    if let Ok(text) = serde_json::to_string(&entered[keep..]) {
        platform::session_set(ENTERED, &text);
    }
    false
}
