use super::super::{
    Host,
    identity::{self, HostGameContext, ViewerRole, WireIdentity},
};
use crate::{arena::Arena, hub::Seat};
use dominion_online::transport::{NamedId, TableDetails};
use dominion_protocol::lobby::UserKind;

/// Who a game is projected for.
pub(crate) enum Viewer<'a> {
    Seat(&'a Seat),
    /// Someone watching game `.0` without a seat, as `.1`.
    Spectator(&'a str, NamedId),
}

pub(crate) fn context(arena: &Arena, seat: &Seat, host: &Host) -> Result<HostGameContext, String> {
    viewer_context(arena, Viewer::Seat(seat), host)
}

pub(crate) fn viewer_context(
    arena: &Arena,
    viewer: Viewer<'_>,
    host: &Host,
) -> Result<HostGameContext, String> {
    let game = match &viewer {
        Viewer::Seat(seat) => seat.game.as_str(),
        Viewer::Spectator(game, _) => game,
    };
    let meta = arena.hub.meta(game).map_err(|e| e.to_string())?;
    let game_id = identity::number(game, 'g')?;
    let names = match &viewer {
        Viewer::Seat(seat) => arena.hub.project_seat(seat, |o| o.names.to_vec()),
        Viewer::Spectator(..) => arena.hub.project_watch(game, |o| o.names.to_vec()),
    }
    .map_err(|e| e.to_string())?;
    let player_ids = meta
        .owners
        .iter()
        .enumerate()
        .map(|(i, owner)| match owner {
            Some(id) => {
                let user = arena.account_ref(id).map_err(|e| e.to_string())?;
                identity::account(id, user.kind == UserKind::Bot)
            }
            None => identity::token(game_id, i),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if player_ids.len() != names.len() {
        return Err("game ownership and seats disagree".into());
    }
    let table = meta.table.as_ref().and_then(|id| arena.table(id).ok());
    let table_options = dominion_protocol::lobby::TableSpec {
        rated: meta.rated,
        timed: meta.timed,
        clock: meta.clock.as_ref().map(|c| c.rule),
        undo: meta.undo,
        ..super::rules::defaults()
    };
    let mut table_details = match table {
        Some(table) => super::rules::details(arena, &table, host)?,
        None => {
            let table_id = match &meta.table {
                Some(id) => identity::number(id, 't')?,
                None => (1_i64 << 52)
                    .checked_add(game_id)
                    .filter(|n| *n < (1_i64 << 53))
                    .ok_or("detached identity capacity exhausted")?,
            };
            TableDetails {
                table_id,
                host_id: player_ids[0],
                players: player_ids
                    .iter()
                    .zip(names)
                    .map(|(&id, name)| NamedId { id, name })
                    .collect(),
                ready: vec![false; player_ids.len()],
                bots: vec![],
                spectators: vec![],
                reconnecting: vec![],
                rules: vec![],
                division: String::new(),
                match_games: vec![],
                match_score: None,
                tournament: None,
            }
        }
    };
    let viewer_role = match viewer {
        Viewer::Seat(seat) => ViewerRole::Player(usize::from(seat.seat.0)),
        Viewer::Spectator(_, me) => {
            table_details.spectators.push(me);
            ViewerRole::Spectator
        }
    };
    Ok(HostGameContext {
        wire_ids: WireIdentity {
            game_id,
            table_id: table_details.table_id,
            player_ids,
        },
        table_details,
        table_options,
        viewer_role,
    })
}
