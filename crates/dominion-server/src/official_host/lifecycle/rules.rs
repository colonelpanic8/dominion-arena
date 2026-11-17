use super::super::{Host, identity};
use crate::arena::{self, Arena};
use dominion_engine::Card;
use dominion_online::transport::{
    self, Group, NamedId, RequiredSlot, RuleValue, SpecialKingdom, TableRule,
};
use dominion_protocol::lobby::{SeatKind, TableInfo, TableSpec, UserKind};

pub(super) fn defaults() -> TableSpec {
    TableSpec {
        rated: false,
        timed: false,
        undo: dominion_protocol::lobby::UndoRule::Off,
        colonies: Some(false),
        shelters: Some(false),
        slots: Some(vec![]),
        ..TableSpec::default()
    }
}

const EXPANSIONS: &[&[&str]] = &[
    &["Promos"],
    &["Base"],
    &["Intrigue"],
    &["Seaside"],
    &["Alchemy"],
    &["Prosperity"],
    &["Cornucopia & Guilds"],
    &["Hinterlands"],
    &["Dark Ages"],
    &[],
    &["Adventures"],
    &["Empires"],
    &["Nocturne"],
    &["Renaissance"],
    &["Menagerie"],
    &[],
    &["Allies"],
    &["Plunder"],
    &["Rising Sun"],
    &[],
];
fn bad<T>(message: &str) -> arena::Result<T> {
    Err(arena::Error::BadRequest(message.into()))
}
fn name(host: &Host, ordinal: i32) -> arena::Result<Card> {
    let card = usize::try_from(ordinal)
        .ok()
        .and_then(|i| host.catalog.catalog.cards.get(i));
    let card = card.and_then(|c| Card::ALL.iter().copied().find(|x| x.name() == c.name));
    card.ok_or_else(|| arena::Error::BadRequest("card is absent from Arena's rules catalog".into()))
}
fn ternary(value: i32) -> arena::Result<Option<bool>> {
    match value {
        0 => Ok(Some(false)),
        1 => Ok(Some(true)),
        2 => Ok(None),
        _ => bad("invalid special kingdom flag"),
    }
}

pub(super) fn apply(
    host: &Host,
    spec: &mut TableSpec,
    rule: &TableRule,
    seats: usize,
) -> arena::Result<()> {
    match (&rule.value, rule.id) {
        (RuleValue::Required(slots), 2) => {
            let mut kingdom = vec![];
            let mut landscapes = vec![];
            let mut from_sets = vec![];
            for slot in slots {
                match slot {
                    RequiredSlot::Card(id) => {
                        let card = name(host, *id)?;
                        if card.kind().is_landscape() {
                            landscapes.push(card.name().into());
                        } else {
                            kingdom.push(card.name().into());
                        }
                    }
                    RequiredSlot::Expansion(id) => {
                        let names = EXPANSIONS
                            .get(*id as usize)
                            .filter(|names| names.len() == 1)
                            .ok_or_else(|| {
                                arena::Error::BadRequest(
                                    "combined or unsupported expansion slot".into(),
                                )
                            })?;
                        from_sets.push(names[0].into());
                    }
                    RequiredSlot::LandscapeTypes(_) => {
                        return bad("random landscape type slots are not mapped");
                    }
                }
            }
            spec.kingdom = kingdom;
            spec.landscapes = landscapes;
            spec.from_sets = from_sets;
        }
        (RuleValue::Ints(ids), 3) => {
            if ids.contains(&15) {
                return bad("random removed-edition selection is not mapped");
            }
            let mut sets = vec![];
            for id in ids {
                for set in *EXPANSIONS
                    .get(*id as usize)
                    .ok_or_else(|| arena::Error::BadRequest("unknown expansion".into()))?
                {
                    if !sets.iter().any(|s| s == set) {
                        sets.push((*set).to_string());
                    }
                }
            }
            if sets.is_empty() {
                return bad("at least one released expansion is required");
            }
            spec.sets = sets;
        }
        (RuleValue::Special(s), 11) => {
            if !s.setup.is_empty() || !s.trait_targets.is_empty() {
                return bad("explicit setup choices are not mapped to table rules");
            }
            spec.colonies = ternary(s.colonies)?;
            spec.shelters = ternary(s.shelters)?;
        }
        (RuleValue::Boolean(value), 19) => spec.rated = *value,
        (RuleValue::Int(value), 15) if *value == 0 => {
            spec.undo = dominion_protocol::lobby::UndoRule::Off
        }
        (RuleValue::Timer(timer), 16) => {
            spec.clock = host
                .catalog
                .clock(timer)
                .map_err(arena::Error::BadRequest)?;
            spec.timed = false;
        }
        (RuleValue::Group(Group::Everybody), 5) => spec.watchable = true,
        (RuleValue::Group(Group::Nobody), 5) => spec.watchable = false,
        (RuleValue::Boolean(false), 13) => {}
        (RuleValue::Ints(ids), 12) if ids.is_empty() => spec.respect_banned = false,
        (RuleValue::PlayerOrder(order), 14) if order.is_empty() => spec.randomize_order = true,
        (RuleValue::Seed { enabled: false, .. }, 8) => {}
        (RuleValue::Int(value), 9 | 10) if *value == seats as i32 => {}
        (RuleValue::Ints(cards), 1) if cards.is_empty() => {}
        (RuleValue::Boolean(false), 0 | 20 | 23) => {}
        (RuleValue::Group(Group::Everybody), 4) => {}
        (RuleValue::Group(Group::Nobody), 17) => {}
        (RuleValue::Double(value), 6) if *value <= 0.0 => {}
        (RuleValue::Double(value), 7) if *value >= 100.0 => {}
        (RuleValue::Ints(values), 21) if values.is_empty() => {}
        (RuleValue::Text(value), 24) if value.is_empty() => {}
        _ => return bad("this official table rule has no equivalent Arena policy"),
    }
    Ok(())
}

pub(super) fn details(
    arena: &Arena,
    info: &TableInfo,
    host: &Host,
) -> Result<transport::TableDetails, String> {
    let (host_account, kind) = arena.account_id(&info.host).map_err(|e| e.to_string())?;
    let host_id = identity::account(&host_account, kind == UserKind::Bot)?;
    let mut players = vec![];
    let mut ready = vec![];
    let mut bots = vec![];
    for seat in &info.seats {
        if let Some(user) = &seat.user {
            let (id, kind) = arena.account_id(&user.name).map_err(|e| e.to_string())?;
            let named = NamedId {
                id: identity::account(&id, kind == UserKind::Bot)?,
                name: user.name.clone(),
            };
            if seat.kind == SeatKind::Agent {
                bots.push(named);
            } else {
                players.push(named);
                ready.push(seat.ready);
            }
        }
    }
    let card_ids = |names: &[String]| -> Result<Vec<RequiredSlot>, String> {
        names
            .iter()
            .map(|name| {
                host.catalog
                    .catalog
                    .ordinal(name)
                    .map(RequiredSlot::Card)
                    .ok_or_else(|| format!("missing pinned card {name}"))
            })
            .collect()
    };
    let mut required = card_ids(&info.spec.kingdom)?;
    required.extend(card_ids(&info.spec.landscapes)?);
    for set in &info.spec.from_sets {
        let id = EXPANSIONS
            .iter()
            .position(|names| names.contains(&set.as_str()))
            .ok_or_else(|| format!("missing pinned expansion {set}"))?;
        required.push(RequiredSlot::Expansion(id as i32));
    }
    let expansions = EXPANSIONS
        .iter()
        .enumerate()
        .filter(|(_, sets)| {
            sets.iter()
                .any(|set| info.spec.sets.is_empty() || info.spec.sets.iter().any(|s| s == set))
        })
        .map(|(i, _)| i as i32)
        .collect();
    let flag = |v: Option<bool>| v.map_or(2, i32::from);
    let rule = |id, value| (TableRule { id, value }, false);
    let rules = vec![
        rule(2, RuleValue::Required(required)),
        rule(3, RuleValue::Ints(expansions)),
        rule(9, RuleValue::Int(info.seats.len() as i32)),
        rule(10, RuleValue::Int(info.seats.len() as i32)),
        rule(
            11,
            RuleValue::Special(SpecialKingdom {
                colonies: flag(info.spec.colonies),
                shelters: flag(info.spec.shelters),
                setup: vec![],
                trait_targets: vec![],
            }),
        ),
        rule(13, RuleValue::Boolean(false)),
        rule(
            15,
            RuleValue::Int(match info.spec.undo {
                dominion_protocol::lobby::UndoRule::Off => 0,
                dominion_protocol::lobby::UndoRule::Ask => 1,
                dominion_protocol::lobby::UndoRule::Free => 2,
            }),
        ),
        rule(19, RuleValue::Boolean(info.spec.rated)),
        rule(
            5,
            RuleValue::Group(if info.spec.watchable {
                Group::Everybody
            } else {
                Group::Nobody
            }),
        ),
    ];
    let mut rules = rules;
    if !info.spec.timed || info.spec.clock.is_some() {
        let timer = host
            .catalog
            .timer(info.spec.clock)
            .map_err(|e| e.to_string())?;
        rules.push(rule(16, RuleValue::Timer(timer)));
    }
    Ok(transport::TableDetails {
        table_id: identity::number(&info.id, 't')?,
        host_id,
        players,
        ready,
        bots,
        spectators: vec![],
        reconnecting: vec![],
        rules,
        division: String::new(),
        match_games: vec![],
        match_score: None,
        tournament: None,
    })
}

pub(super) fn summary(arena: &Arena, info: &TableInfo) -> Result<transport::TableSummary, String> {
    let (id, kind) = arena.account_id(&info.host).map_err(|e| e.to_string())?;
    Ok(transport::TableSummary {
        table_id: identity::number(&info.id, 't')?,
        host: NamedId {
            id: identity::account(&id, kind == UserKind::Bot)?,
            name: info.host.clone(),
        },
        players: info
            .seats
            .iter()
            .filter(|s| s.kind == SeatKind::Player)
            .count() as i32,
        bots: info
            .seats
            .iter()
            .filter(|s| s.kind == SeatKind::Agent)
            .count() as i32,
        spectators: 0,
        min_players: info.seats.len() as i32,
        max_players: info.seats.len() as i32,
        observable: false,
        joinable: info.game.is_none() && info.seats.iter().any(|s| s.kind == SeatKind::Open),
        status: if info.game.is_some() {
            2
        } else if info.last_game.is_some() {
            1
        } else {
            0
        },
        start_time: info
            .game
            .as_ref()
            .and_then(|id| arena.hub.meta(id).ok())
            .map(|m| m.started as i64),
    })
}
