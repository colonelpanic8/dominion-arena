//! The seat tokens this browser holds, by game, in localStorage. A browser may hold
//! several seats of one game (hot-seat play).

use crate::platform;
use dominion_protocol::api::SeatToken;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const KEY: &str = "dominion.seats";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldSeat {
    pub seat: u8,
    pub name: String,
    pub token: String,
}

fn load() -> BTreeMap<String, Vec<HeldSeat>> {
    platform::storage_get(KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(all: &BTreeMap<String, Vec<HeldSeat>>) {
    if let Ok(s) = serde_json::to_string(all) {
        platform::storage_set(KEY, &s);
    }
}

/// Every game this browser holds a seat in, with those seats.
pub fn all() -> BTreeMap<String, Vec<HeldSeat>> {
    load()
}

/// The seats held in `game`, in seat order.
pub fn seats(game: &str) -> Vec<HeldSeat> {
    load().remove(game).unwrap_or_default()
}

/// Remembers a seat, replacing what was held for the same seat.
pub fn hold(game: &str, seat: HeldSeat) {
    let mut all = load();
    let seats = all.entry(game.to_string()).or_default();
    seats.retain(|s| s.seat != seat.seat);
    seats.push(seat);
    seats.sort_by_key(|s| s.seat);
    save(&all);
}

/// Forgets a seat (its token was refused).
pub fn forget(game: &str, token: &str) {
    let mut all = load();
    if let Some(seats) = all.get_mut(game) {
        seats.retain(|s| s.token != token);
        if seats.is_empty() {
            all.remove(game);
        }
    }
    save(&all);
}

const LINKS: &str = "dominion.links";

fn load_links() -> BTreeMap<String, Vec<SeatToken>> {
    platform::storage_get(LINKS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Remembers the tokens of `game`'s seats played elsewhere, to hand out.
pub fn give_out(game: &str, links: &[SeatToken]) {
    if links.is_empty() {
        return;
    }
    let mut all = load_links();
    all.insert(game.to_string(), links.to_vec());
    if let Ok(s) = serde_json::to_string(&all) {
        platform::storage_set(LINKS, &s);
    }
}

/// The tokens of `game`'s seats this browser created and hands out.
pub fn links(game: &str) -> Vec<SeatToken> {
    load_links().remove(game).unwrap_or_default()
}
