use std::collections::{BTreeMap, BTreeSet};

use dominion_engine::{Card, CardId, EndReason, Mat, Outcome, PlayerId, Types, View};
use dominion_online::{model, transport};

pub(crate) use super::identity::WireIdentity;
use super::{
    catalog::{FORCED_RESIGNATION, MANUAL_RESIGNATION, TIMED_OUT_RESIGNATION, WireCatalog, zone},
    identity::{HostGameContext, ProjectionRequest, ViewerRole},
    questions::{self, PreparedQuestion},
};
use crate::hub;
use dominion_protocol::PileMarker;
use dominion_protocol::lobby::ClockView;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProjectionError {
    #[error(transparent)]
    Compatibility(#[from] super::profile::CompatibilityGap),
    #[error(transparent)]
    UnsupportedSetup(#[from] super::profile::UnsupportedFeature),
    #[error("unsupported state: {0}")]
    UnsupportedState(String),
    #[error("missing pinned catalog symbol: {0}")]
    MissingSymbol(String),
    #[error("projection size limit: {0}")]
    Size(&'static str),
    #[error("inconsistent observation: {0}")]
    Inconsistent(&'static str),
}

pub(crate) struct ProjectedUpdate {
    pub version: u64,
    pub undos: u64,
    pub snapshot: model::Snapshot,
    pub question: Option<PreparedQuestion>,
    pub finished: Option<model::GameResult>,
    pub table: transport::TableDetails,
    /// Each player's clock, where the game has one.
    pub timers: Vec<transport::TimerUpdate>,
}

pub(crate) fn project(
    observation: hub::SeatObservation<'_>,
    context: &HostGameContext,
    catalog: &WireCatalog,
    request: ProjectionRequest,
) -> Result<ProjectedUpdate, ProjectionError> {
    build(observation, context, catalog, request)
}

pub(super) fn integer(value: usize, field: &'static str) -> Result<i32, ProjectionError> {
    if value > 65_536 {
        return Err(ProjectionError::Size(field));
    }
    i32::try_from(value).map_err(|_| ProjectionError::Size(field))
}

fn count(value: u32, field: &'static str) -> Result<i32, ProjectionError> {
    i32::try_from(value).map_err(|_| ProjectionError::Size(field))
}

struct VisibleZone {
    wire: model::Zone,
    slots: Vec<Option<CardId>>,
    /// Slots without a disclosed card whose name is public (a split pile's cards).
    named: Vec<(usize, Card)>,
    /// Its named slots are no physical cards (Boons, Hexes, landscapes): each is its own
    /// pile.
    standalone: bool,
}

/// A wire card: a disclosed engine card, or a zone's slot known only by name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Handle {
    Card(CardId),
    Named(usize, usize),
}

impl VisibleZone {
    fn new(kind: i32, owner: i32, pile_name: i32, slots: Vec<Option<CardId>>) -> Self {
        Self {
            wire: model::Zone {
                index: 0,
                kind,
                owner,
                pile_name,
                cards: vec![],
                created_by: -1,
                attached_trait: 0,
                is_visible: kind != zone::DRAW,
            },
            slots,
            named: vec![],
            standalone: false,
        }
    }

    /// A zone of cards that are no physical cards (Boons, Hexes, a landscape), each known
    /// by name, bottom first.
    fn named(kind: i32, pile_name: i32, cards: &[Card]) -> Self {
        let mut zone = Self::new(kind, -1, pile_name, vec![None; cards.len()]);
        zone.named = cards.iter().copied().enumerate().collect();
        zone.standalone = true;
        zone
    }
}

/// A setup note dominion.games shows only as a marker on its pile: the Bane, Obelisk's
/// pile, Ferryman's.
fn setup_marker(note: &dominion_engine::Note) -> Option<(dominion_engine::PileId, PileMarker)> {
    use dominion_engine::SetupRole::*;
    let marker = match (note.setup?, note.source) {
        (Bane, Card::YoungWitch) => PileMarker::Bane,
        (ChosenPile, Card::Obelisk) => PileMarker::Obelisk,
        (ExtraPile, Card::Ferryman) => PileMarker::Ferryman,
        _ => return None,
    };
    Some((note.pile?, marker))
}

fn build(
    observation: hub::SeatObservation<'_>,
    context: &HostGameContext,
    catalog: &WireCatalog,
    request: ProjectionRequest,
) -> Result<ProjectedUpdate, ProjectionError> {
    let ids = &context.wire_ids;
    let question_index = request.question_index;
    let reconnect = request.reconnect;
    let view = &observation.view;
    if context.table_options.timed {
        return Err(ProjectionError::UnsupportedState(
            "timed table options".into(),
        ));
    }
    table::validate(view)?;
    let players = view.num_players();
    if !matches!(context.viewer_role, ViewerRole::Player(p) if p < players && view.me() == Some(PlayerId(p as u8)))
    {
        return Err(ProjectionError::Inconsistent("viewer context"));
    }
    if ids.player_ids.len() != players
        || ids.player_ids.iter().any(|&id| id < 0)
        || ids.player_ids.iter().collect::<BTreeSet<_>>().len() != players
        || ids.game_id < 0
        || ids.table_id < 0
        || ids.game_id > 9_007_199_254_740_991
        || ids.table_id > 9_007_199_254_740_991
        || observation.names.len() != players
    {
        return Err(ProjectionError::Inconsistent("fixed-game identity"));
    }
    if observation.deciding.is_some_and(|p| p.index() >= players)
        || view.turn().player.index() >= players
    {
        return Err(ProjectionError::Inconsistent("player index"));
    }
    if observation.outcome.is_some()
        && (observation.decision_id.is_some()
            || observation.deciding.is_some()
            || view.pending().is_some())
    {
        return Err(ProjectionError::Inconsistent(
            "finished game has a decision",
        ));
    }
    if observation.ended.is_some() && observation.outcome.is_none() {
        return Err(ProjectionError::Inconsistent("end cause without result"));
    }
    if observation.outcome.is_none()
        && (view.pending().is_some() != (observation.deciding == view.me()))
    {
        return Err(ProjectionError::Inconsistent(
            "deciding seat and private question differ",
        ));
    }
    // Knowledge multisets deliberately do not identify physical slots in this profile.
    let _ = observation.known;
    let mut zones = Vec::new();
    for p in view.players() {
        let owner = i32::from(p.id().0);
        let hand = match p.hand() {
            Some(ids) => ids.iter().copied().map(Some).collect(),
            None => vec![None; p.hand_size()],
        };
        zones.push(VisibleZone::new(zone::HAND, owner, 0, hand));
        zones.push(VisibleZone::new(
            zone::DRAW,
            owner,
            0,
            vec![None; p.deck_size()],
        ));
        let mut discard = vec![None; p.discard_size()];
        if let Some(top) = p.discard_top() {
            *discard
                .first_mut()
                .ok_or(ProjectionError::Inconsistent("discard top without cards"))? = Some(top);
        }
        zones.push(VisibleZone::new(zone::DISCARD, owner, 0, discard));
        zones.push(VisibleZone::new(
            zone::PLAY,
            owner,
            0,
            p.in_play().iter().copied().map(Some).collect(),
        ));
    }
    for p in view.players() {
        let owner = i32::from(p.id().0);
        for (kind, size, known) in [
            (zone::REVEAL, p.looking_size(), p.looking()),
            (zone::SET_ASIDE, p.set_aside_size(), p.set_aside()),
            (
                10,
                p.mat_size(Mat::Tavern),
                p.mat(Mat::Tavern).unwrap_or(&[]).to_vec(),
            ),
            (
                11,
                p.mat_size(Mat::NativeVillage),
                p.mat(Mat::NativeVillage).unwrap_or(&[]).to_vec(),
            ),
            (
                12,
                p.mat_size(Mat::Island),
                p.mat(Mat::Island).unwrap_or(&[]).to_vec(),
            ),
            (
                21,
                p.mat_size(Mat::Exile),
                p.mat(Mat::Exile).unwrap_or(&[]).to_vec(),
            ),
        ] {
            if size == 0 {
                continue;
            }
            let mut slots: Vec<_> = known.into_iter().map(Some).collect();
            slots.resize(size, None);
            zones.push(VisibleZone::new(kind, owner, 0, slots));
        }
    }
    zones.push(VisibleZone::new(
        zone::TRASH,
        -1,
        0,
        view.trash().iter().copied().map(Some).collect(),
    ));
    // Searched discard/deck candidates are visible without disclosing the other slots.
    if let Some(d) = view.pending() {
        // A triggered ability's own card in a discard pile (Tunnel discarded with others).
        let triggers = view.pending_triggers().into_iter().filter_map(|t| {
            t.source.filter(|&id| {
                matches!(view.zone(id), Some(dominion_engine::Zone::Discard(p)) if p == d.player)
            })
        });
        let offered = d
            .about
            .iter()
            .copied()
            .chain(match &d.kind {
                dominion_engine::DecisionKind::Cards { candidates, .. } => candidates.clone(),
                _ => vec![],
            })
            .chain(triggers);
        let mut inspected = Vec::new();
        for id in offered {
            if zones.iter().any(|z| z.slots.contains(&Some(id))) || inspected.contains(&Some(id)) {
                continue;
            }
            let location = view
                .zone(id)
                .ok_or(ProjectionError::Inconsistent("undisclosed candidate"))?;
            let (owner, kind) = match location {
                dominion_engine::Zone::Discard(p) => (i32::from(p.0), zone::DISCARD),
                dominion_engine::Zone::Deck(p) => (i32::from(p.0), zone::DRAW),
                // Another player's hand they revealed (Pillage).
                dominion_engine::Zone::Hand(p) => (i32::from(p.0), zone::HAND),
                _ => {
                    return Err(ProjectionError::UnsupportedState(
                        "candidate zone association".into(),
                    ));
                }
            };
            let z = zones
                .iter_mut()
                .find(|z| z.wire.kind == kind && z.wire.owner == owner)
                .ok_or(ProjectionError::Inconsistent("candidate zone absent"))?;
            let slot =
                z.slots
                    .iter()
                    .position(Option::is_none)
                    .ok_or(ProjectionError::Inconsistent(
                        "candidate exceeds hidden zone size",
                    ))?;
            z.slots.remove(slot);
            inspected.push(Some(id));
        }
        if !inspected.is_empty() {
            zones.push(VisibleZone::new(23, i32::from(d.player.0), 0, inspected));
        }
    }
    if let Some(d) = view.pending()
        && let dominion_engine::DecisionKind::Cards {
            candidates,
            ordered: true,
            ..
        } = &d.kind
    {
        let candidate_set: BTreeSet<_> = candidates.iter().copied().collect();
        let containing: Vec<_> = zones
            .iter()
            .enumerate()
            .filter(|(_, z)| {
                z.slots
                    .iter()
                    .flatten()
                    .any(|id| candidate_set.contains(id))
            })
            .map(|(i, _)| i)
            .collect();
        if containing.len() != 1 || zones[containing[0]].wire.kind != 23 {
            return Err(ProjectionError::UnsupportedState(
                "ordered selection requires one reveal row".into(),
            ));
        }
        let mut bottom_first = candidates.iter().rev().copied();
        for slot in &mut zones[containing[0]].slots {
            if slot.is_some_and(|id| candidate_set.contains(&id)) {
                *slot = bottom_first.next();
            }
        }
    }
    let mut piles = view
        .piles()
        .map(|p| Ok((catalog.card(p.card())?, p)))
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    piles.sort_by_key(|x| x.0);
    let mut overrides = Vec::new();
    let mut markers = Vec::new();
    let mut pile_zones = BTreeMap::new();
    let mut held = Vec::new();
    for (name, pile) in &piles {
        let holder = table::holder(view, pile);
        if pile.removed() && holder.is_none() {
            // Out of the game (Divine Wind): their client has no zone for it.
            continue;
        }
        let kind = if let Some(kind) = holder {
            kind
        } else if !pile.in_supply() {
            zone::NON_SUPPLY
        } else if pile.kingdom() {
            zone::SUPPLY
        } else if pile.card().is(Types::TREASURE) {
            zone::TREASURE
        } else {
            zone::VICTORY
        };
        let mut slots = vec![None; pile.size()];
        if let Some(top) = pile.top() {
            // Bottom first, as their client stacks a zone's cards: the top is last.
            *slots
                .last_mut()
                .ok_or(ProjectionError::Inconsistent("pile top without cards"))? = Some(top);
        }
        let marker = pile.structure().and_then(|s| {
            let kind = match s.kind {
                dominion_engine::PileKind::Stacked => dominion_protocol::PileKind::Stacked,
                dominion_engine::PileKind::Rotating => dominion_protocol::PileKind::Rotating,
                dominion_engine::PileKind::Shuffled => dominion_protocol::PileKind::Shuffled,
            };
            dominion_protocol::pile_marker(kind, s.cards.len())
        });
        let cost = view.cost(pile.card());
        overrides.push((
            *name,
            model::Cost {
                coins: cost.coins.into(),
                potions: cost.potions.into(),
                debt: cost.debt.into(),
            },
        ));
        if holder.is_some() {
            held.push((pile.id(), VisibleZone::new(kind, -1, *name, slots)));
            continue;
        }
        let zone_index = integer(zones.len(), "zone index")?;
        pile_zones.insert(pile.id(), zone_index);
        let mut pile_markers: Vec<PileMarker> = view
            .notes()
            .iter()
            .filter_map(setup_marker)
            .filter(|(id, _)| *id == pile.id())
            .map(|(_, m)| m)
            .chain(marker)
            .collect();
        pile_markers.sort();
        for marker in pile_markers {
            markers.push(model::PileMarker {
                kind: super::catalog::pile_marker(marker),
                zone_index,
            });
        }
        let mut zone = VisibleZone::new(kind, -1, *name, slots);
        // Their client shadows a pile it knows holds different cards.
        if let Some(names) = pile.names().filter(|n| n.iter().any(|c| *c != n[0])) {
            if names.len() != pile.size() {
                return Err(ProjectionError::Inconsistent("pile names"));
            }
            let below = names.len() - 1;
            zone.named = names
                .into_iter()
                .skip(1)
                .enumerate()
                .map(|(i, card)| (below - 1 - i, card))
                .collect();
        }
        if let Some(&t) = pile.traits().first() {
            zone.wire.attached_trait = catalog.card(t)?;
        }
        zones.push(zone);
    }
    zones.extend(table::fate_zones(view, catalog)?);
    let mut landscape_zones = BTreeMap::new();
    for &landscape in view.landscapes() {
        let name = catalog.card(landscape)?;
        landscape_zones.insert(landscape, integer(zones.len(), "zone index")?);
        zones.push(VisibleZone::named(zone::LANDSCAPE, name, &[landscape]));
    }
    // Their client places a card a pile out of the game holds (Way of the Mouse's) on
    // its landscape, which must be drawn first.
    for (pile, zone) in held {
        pile_zones.insert(pile, integer(zones.len(), "zone index")?);
        zones.push(zone);
    }
    let mut grouped: BTreeMap<(i32, i32), Vec<Handle>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (zi, z) in zones.iter().enumerate() {
        integer(z.slots.len(), "zone size")?;
        for id in z.slots.iter().flatten() {
            if !seen.insert(*id) {
                return Err(ProjectionError::Inconsistent("card in multiple zones"));
            }
            let card = view.card(*id).ok_or(ProjectionError::Inconsistent(
                "undisclosed card in visible slot",
            ))?;
            let cost = view.cost(card);
            if view.cost_of(*id) != Some(cost) {
                return Err(ProjectionError::UnsupportedState(
                    "instance-specific cost override".into(),
                ));
            }
            overrides.push((
                catalog.card(card)?,
                model::Cost {
                    coins: cost.coins.into(),
                    potions: cost.potions.into(),
                    debt: cost.debt.into(),
                },
            ));
            let origin = view
                .origin_pile(*id)
                .map(|p| view.pile(p).card())
                .unwrap_or(card);
            grouped
                .entry((catalog.card(card)?, catalog.card(origin)?))
                .or_default()
                .push(Handle::Card(*id));
        }
        for &(slot, card) in &z.named {
            if z.slots.get(slot) != Some(&None) {
                return Err(ProjectionError::Inconsistent("named slot"));
            }
            let name = catalog.card(card)?;
            grouped
                .entry((name, if z.standalone { name } else { z.wire.pile_name }))
                .or_default()
                .push(Handle::Named(zi, slot));
        }
    }
    let mut handles = BTreeMap::new();
    let mut cards = Vec::new();
    let mut card_names = Vec::new();
    let mut card_counts = Vec::new();
    let mut pile_names = Vec::new();
    for ((name, origin), group) in grouped {
        card_names.push(name);
        pile_names.push(origin);
        card_counts.push(integer(group.len(), "allocation count")?);
        for handle in group {
            let id = integer(cards.len(), "allocated cards")?;
            handles.insert(handle, id);
            cards.push(model::Card {
                id,
                card_name: name,
                pile_name: origin,
            });
        }
    }
    integer(cards.len(), "allocated cards")?;
    let identities: BTreeMap<CardId, i32> = handles
        .iter()
        .filter_map(|(h, &id)| match h {
            Handle::Card(c) => Some((*c, id)),
            Handle::Named(..) => None,
        })
        .collect();
    let zones = zones
        .into_iter()
        .enumerate()
        .map(|(i, mut z)| {
            z.wire.index = integer(i, "zone index")?;
            z.wire.cards = z
                .slots
                .iter()
                .enumerate()
                .map(|(slot, id)| match id {
                    Some(id) => handles[&Handle::Card(*id)],
                    None => handles.get(&Handle::Named(i, slot)).copied().unwrap_or(-1),
                })
                .collect();
            if z.wire.kind == zone::HAND {
                z.wire.is_visible = Some(PlayerId(z.wire.owner as u8)) == view.me();
            }
            Ok(z.wire)
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    let turn = view.turn();
    let landscape_ids: BTreeMap<Card, i32> = landscape_zones
        .iter()
        .map(|(&card, &zone)| (card, handles[&Handle::Named(zone as usize, 0)]))
        .collect();
    for card in table::offstage(view) {
        let name = catalog.card(card)?;
        if !card_names.contains(&name) {
            card_names.push(name);
            pile_names.push(name);
            card_counts.push(0);
        }
    }
    let mut counters = [
        ("ACTIONS", turn.actions),
        ("COINS", turn.coins),
        ("POTIONS", turn.potions),
        ("BUYS", turn.buys),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (kind, value))| {
        Ok(model::Counter {
            index: integer(i, "counter index")?,
            kind: catalog.symbol("CounterNames", kind)?,
            owner: turn.player.0.into(),
            value: count(value, "turn counter")?,
            zone: -1,
            card: -1,
            is_visible: true,
        })
    })
    .collect::<Result<Vec<_>, ProjectionError>>()?;
    for p in view.players() {
        for (kind, value) in [
            ("COFFERS", i64::from(p.coffers())),
            ("VILLAGERS", i64::from(p.villagers())),
            ("DEBT", i64::from(p.debt())),
            ("VP_TOKENS", i64::from(p.vp_tokens())),
            ("FAVORS", i64::from(p.favors())),
            ("PIRATE_STRIKES", i64::from(p.pirate_ship())),
        ] {
            if value == 0 && catalog.symbol("CounterNames", kind).is_err() {
                continue;
            }
            counters.push(model::Counter {
                index: integer(counters.len(), "counter index")?,
                kind: catalog.symbol("CounterNames", kind)?,
                owner: i32::from(p.id().0),
                value: i32::try_from(value).map_err(|_| ProjectionError::Size("player counter"))?,
                zone: -1,
                card: -1,
                is_visible: true,
            });
        }
    }
    for c in table::counts(view, &pile_zones, &landscape_zones) {
        counters.push(model::Counter {
            index: integer(counters.len(), "table counter")?,
            kind: catalog.symbol("CounterNames", c.kind)?,
            owner: c.owner,
            value: c.value,
            zone: c.zone,
            card: -1,
            is_visible: true,
        });
    }
    let mut links: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    for lasting in view.lasting() {
        let Some(source) = lasting.source.and_then(|s| identities.get(&s).copied()) else {
            continue;
        };
        for id in lasting.data.cards.iter().copied().chain(lasting.data.pick) {
            if let Some(&linked) = identities.get(&id) {
                links.entry(source).or_default().insert(linked);
            }
        }
        if let Some(value) = view.lasting_tokens(&lasting) {
            counters.push(model::Counter {
                index: integer(counters.len(), "card counter")?,
                kind: catalog.symbol("CounterNames", "CARD_TOKENS")?,
                owner: i32::from(lasting.owner.0),
                value: count(value, "card tokens")?,
                zone: -1,
                card: source,
                is_visible: true,
            });
        }
    }
    // A Trait or Obelisk is drawn with a card of its pile: a card of that pile linked
    // to the landscape.
    let notes = view.notes();
    let chosen = view
        .piles()
        .flat_map(|p| p.traits().iter().map(move |&t| (p.id(), t)))
        .chain(notes.iter().filter_map(|n| {
            setup_marker(n)
                .filter(|(_, m)| *m == PileMarker::Obelisk)
                .map(|(pile, _)| (pile, n.source))
        }));
    for (pile, landscape) in chosen {
        let (Some(top), Some(&linked)) = (view.pile(pile).top(), landscape_ids.get(&landscape))
        else {
            return Err(ProjectionError::UnsupportedState(
                "a Trait or Obelisk without its pile's card".into(),
            ));
        };
        links.entry(identities[&top]).or_default().insert(linked);
    }
    // Their client draws Riverboat's card on its pile: the card linked to a Riverboat.
    let boxed = view
        .piles()
        .filter(|p| table::holder(view, p) == Some(zone::THE_BOX))
        .filter_map(|p| p.top());
    for card in boxed {
        let riverboat = identities
            .iter()
            .find(|(id, _)| view.card(**id) == Some(Card::Riverboat))
            .map(|(_, &handle)| handle);
        if let (Some(&card), Some(riverboat)) = (identities.get(&card), riverboat) {
            links.entry(card).or_default().insert(riverboat);
        }
    }
    let associations = links
        .into_iter()
        .map(|(card, linked_cards)| model::Association {
            card,
            linked_cards: linked_cards.into_iter().collect(),
        })
        .collect();
    let turn_number = view.player(turn.player).turns_taken();
    let tokens = table::tokens(view, catalog, &pile_zones, &zones)?;
    let snapshot = model::Snapshot {
        reconnect,
        game_id: ids.game_id,
        state: model::GameState {
            players: ids
                .player_ids
                .iter()
                .enumerate()
                .map(|(seat, &player_id)| {
                    Ok(model::Player {
                        player_id,
                        card_frequencies: vec![],
                        reminders: table::reminders(view, catalog, seat)?,
                        bought_projects: vec![],
                    })
                })
                .collect::<Result<_, ProjectionError>>()?,
            pile_names,
            card_names,
            card_counts,
            cards,
            zones,
            pile_markers: markers,
            tokens,
            counters,
            turn: model::TurnDescription {
                owner_id: turn.player.0.into(),
                controller_id: turn.player.0.into(),
                turn_number: count(turn_number, "turn number")?,
                turn_type: catalog.symbol("ExtraTurnTypes", table::turn_type(view)?)?,
            },
            cost_reduction: model::CostReduction {
                generic: 0,
                by_type: vec![],
                by_card: vec![],
                cost_override: overrides
                    .into_iter()
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            },
            temporary_effects: model::TemporaryEffects {
                renames: vec![],
                behaviour_changes: vec![],
                image_changes: vec![],
                card_type_changes: vec![],
            },
            associations,
            card_statuses: vec![],
        },
        autoplay_settings: vec![],
        table_options: model::TableOptions {
            players_see_spectator_chat: false,
            timer_settings: {
                let timer = catalog.timer(observation.clock.as_ref().map(|(rule, _)| *rule))?;
                model::TimerSettings {
                    settings: timer.parameters,
                    force_resign: timer.force_resign,
                    preset: timer.preset,
                }
            },
        },
        // Project tokens need the landscape zones to exist in the receiving model.
        events: view
            .players()
            .flat_map(|p| {
                p.projects().iter().map(move |&card| {
                    Ok(model::Event::ProjectBought(model::ProjectBought {
                        player: p.id().0.into(),
                        project: catalog.card(card)?,
                    }))
                })
            })
            .collect::<Result<_, ProjectionError>>()?,
    };
    let mut question = view
        .pending()
        .map(|decision| {
            let decision_id = observation
                .decision_id
                .ok_or(ProjectionError::Inconsistent("missing decision id"))?;
            questions::prepare(
                view,
                decision,
                decision_id,
                question_index,
                &identities,
                &landscape_ids,
                catalog,
            )
        })
        .transpose()?;
    if let Some(q) = &mut question {
        q.epoch = request.epoch;
    }
    let table = context.table_details.clone();
    let finished = observation
        .outcome
        .map(|outcome| result(outcome, observation.ended, ids, &piles))
        .transpose()?;
    let timers = match &observation.clock {
        Some((_, clock)) => timers(clock, ids)?,
        None => vec![],
    };
    Ok(ProjectedUpdate {
        version: observation.version,
        undos: observation.undos,
        snapshot,
        question,
        finished,
        table,
        timers,
    })
}

/// The client's timer for each player: their time, ticking for the player deciding.
fn timers(
    clock: &ClockView,
    ids: &WireIdentity,
) -> Result<Vec<transport::TimerUpdate>, ProjectionError> {
    if clock.left.len() != ids.player_ids.len() {
        return Err(ProjectionError::Inconsistent("clock seats"));
    }
    Ok((clock.left.iter().zip(&ids.player_ids).enumerate())
        .map(|(seat, (&left, &player))| transport::TimerUpdate {
            player,
            time: i64::try_from(left).unwrap_or(i64::MAX),
            running: clock.running == Some(seat as u8),
            free_time: 0,
            maximum: 0,
        })
        .collect())
}

fn result(
    outcome: &Outcome,
    ended: Option<&str>,
    ids: &WireIdentity,
    piles: &[(i32, dominion_engine::PileView<'_>)],
) -> Result<model::GameResult, ProjectionError> {
    let player_count = ids.player_ids.len();
    if outcome.scores.len() != player_count
        || outcome.turns.len() != player_count
        || outcome.winners.is_empty()
        || outcome
            .winners
            .iter()
            .chain(&outcome.resigned)
            .any(|p| p.index() >= player_count)
        || outcome.winners.iter().collect::<BTreeSet<_>>().len() != outcome.winners.len()
        || outcome.resigned.iter().collect::<BTreeSet<_>>().len() != outcome.resigned.len()
    {
        return Err(ProjectionError::Inconsistent("outcome players"));
    }
    if !matches!(
        outcome.reason,
        EndReason::ProvincesGone
            | EndReason::ColoniesGone
            | EndReason::PilesEmpty
            | EndReason::Resigned
    ) {
        return Err(ProjectionError::UnsupportedState(
            "nonstandard end condition".into(),
        ));
    }
    if (outcome.reason == EndReason::Resigned && outcome.resigned.len() != 1)
        || (outcome.reason != EndReason::Resigned && !outcome.resigned.is_empty())
        || (ended.is_some() && outcome.reason != EndReason::Resigned)
    {
        return Err(ProjectionError::Inconsistent("resignation outcome"));
    }
    if ended.is_some_and(|cause| !["resigned", "abandoned", "timeout"].contains(&cause)) {
        return Err(ProjectionError::UnsupportedState(
            "unknown end cause".into(),
        ));
    }
    let resigned = |p: usize| outcome.resigned.contains(&PlayerId(p as u8));
    let key = |p: usize| {
        (
            resigned(p),
            std::cmp::Reverse(outcome.scores[p]),
            outcome.turns[p],
        )
    };
    let mut players = Vec::new();
    for p in 0..player_count {
        let rank = 1 + (0..player_count).filter(|&q| key(q) < key(p)).count() as i32;
        if outcome.winners.contains(&PlayerId(p as u8)) != (rank == 1) {
            return Err(ProjectionError::Inconsistent(
                "outcome winners and ranking differ",
            ));
        }
        players.push(model::PlayerResult {
            player_id: ids.player_ids[p],
            rank,
            score: model::Score {
                total_points: outcome.scores[p],
                used_turns: count(outcome.turns[p], "result turns")?,
                parts: vec![],
            },
            card_names: vec![],
            resign_index: if resigned(p) { 0 } else { -1 },
            resignation_type: resigned(p).then_some(match ended {
                Some("abandoned") => FORCED_RESIGNATION,
                Some("timeout") => TIMED_OUT_RESIGNATION,
                _ => MANUAL_RESIGNATION,
            }),
        });
    }
    Ok(model::GameResult {
        table_id: ids.table_id,
        game_id: ids.game_id,
        rating_type: None,
        empty_piles: piles
            .iter()
            .filter(|(_, p)| p.size() == 0)
            .map(|(n, _)| *n)
            .collect(),
        players,
        auto_continue: false,
    })
}

#[path = "projection/table.rs"]
mod table;

#[cfg(test)]
#[path = "projection/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "projection/rules.rs"]
mod rules;

#[cfg(test)]
#[path = "projection/survey.rs"]
mod survey;
