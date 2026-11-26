//! Checked, disjoint wire namespaces and owned projection context.
use dominion_online::transport::TableDetails;
use dominion_protocol::lobby::TableSpec;

const ACCOUNT_LIMIT: i64 = 1 << 29;
const TOKEN_BASE: i64 = 1 << 29;
const BOT_BASE: i64 = 1 << 30;
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

#[derive(Clone, Debug)]
pub(crate) struct WireIdentity {
    pub game_id: i64,
    pub table_id: i64,
    /// Engine seat order, which may differ from lobby seat order.
    pub player_ids: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewerRole {
    Player(usize),
    /// Watching without a seat: every player's held cards face up, no question.
    Spectator,
}

#[derive(Clone, Debug)]
pub(crate) struct HostGameContext {
    pub wire_ids: WireIdentity,
    pub table_details: TableDetails,
    pub viewer_role: ViewerRole,
    pub table_options: TableSpec,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProjectionRequest {
    pub question_index: i32,
    pub reconnect: bool,
    pub epoch: u64,
}

pub(crate) fn number(id: &str, prefix: char) -> Result<i64, String> {
    let value = id
        .strip_prefix(prefix)
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&n| n > 0 && n <= MAX_SAFE_INTEGER)
        .ok_or_else(|| "identity exceeds the pinned client's numeric capacity".to_string())?;
    if id != format!("{prefix}{value}") {
        return Err("noncanonical identity".into());
    }
    Ok(value)
}

pub(crate) fn canonical(value: i64, prefix: char) -> Result<String, String> {
    if !(1..=MAX_SAFE_INTEGER).contains(&value) {
        return Err("invalid wire identity".into());
    }
    Ok(format!("{prefix}{value}"))
}

pub(crate) fn account(id: &str, bot: bool) -> Result<i32, String> {
    let row = number(id, 'u')?;
    if row >= ACCOUNT_LIMIT {
        return Err("account identity capacity exhausted".into());
    }
    i32::try_from(row + if bot { BOT_BASE } else { 0 })
        .map_err(|_| "account identity overflow".into())
}

pub(crate) fn token(game: i64, seat: usize) -> Result<i32, String> {
    let slot = game
        .checked_mul(6)
        .and_then(|n| n.checked_add(seat as i64))
        .filter(|&n| n < ACCOUNT_LIMIT && seat < 6)
        .ok_or_else(|| "token identity capacity exhausted".to_string())?;
    i32::try_from(TOKEN_BASE + slot).map_err(|_| "token identity overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaces_are_checked_and_disjoint() {
        assert_eq!(number("g3", 'g').unwrap(), 3);
        for id in ["g0", "g-1", "g01", "t3", "g9007199254740992"] {
            assert!(number(id, 'g').is_err());
        }
        assert!(account("u536870912", false).is_err());
        assert!(account("u1", false).unwrap() < token(1, 0).unwrap());
        assert!(token(1, 5).unwrap() < account("u1", true).unwrap());
        assert_ne!(token(1, 0).unwrap(), token(2, 0).unwrap());
    }
}
