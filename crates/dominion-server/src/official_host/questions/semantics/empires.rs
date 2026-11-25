//! Question bindings for this set's cards (see `semantics::SETS`).
use super::super::OptionShape;
use super::{Binding, ProjectionError, WireCatalog, gap};
use dominion_engine::{
    Card, Cost, Decision, DecisionKind, Event, Phase, Purpose, Trigger, View, Zone,
};
use dominion_online::questions::{CardFrequency, LogArgument};
use std::collections::BTreeMap;

fn names(c: &WireCatalog, cards: &[Card]) -> Result<LogArgument, ProjectionError> {
    let mut counts = BTreeMap::new();
    for &card in cards {
        *counts.entry(c.card(card)?).or_insert(0) += 1;
    }
    Ok(LogArgument::CardNames(
        counts
            .into_iter()
            .map(|(name, frequency)| CardFrequency { name, frequency })
            .collect(),
    ))
}

fn amount(cost: Cost) -> LogArgument {
    LogArgument::Amount {
        coins: cost.coins.into(),
        potions: cost.potions.into(),
        debt: cost.debt.into(),
    }
}

/// Charm's gain: a card other than the one just gained, costing exactly what it did.
fn charm_arguments(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Result<Vec<LogArgument>, ProjectionError> {
    let gained = view
        .log()
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::Gained { player, card, .. } if *player == d.player => Some(*card),
            _ => None,
        })
        .ok_or_else(|| gap(d, "Charm without a gained card"))?;
    let DecisionKind::Pile { piles, .. } = &d.kind else {
        return Err(gap(d, "Charm gain shape"));
    };
    let cost = piles
        .first()
        .and_then(|&p| view.pile(p).top())
        .and_then(|top| view.cost_of(top))
        .ok_or_else(|| gap(d, "Charm cost"))?;
    Ok(vec![names(c, &[gained])?, amount(cost)])
}

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let pile = matches!(d.kind, DecisionKind::Pile { .. });
    let cards = matches!(d.kind, DecisionKind::Cards { .. });
    let number = matches!(d.kind, DecisionKind::Number { .. });
    let turn_player = d.player == view.turn().player;
    let (kind, story) = match (d.source?, d.purpose) {
        (Archive, P::Move) if cards => ("PUT_IN_HAND", "ARCHIVE"),
        (Catapult, P::Trash) if cards => ("TRASH", "CATAPULT_TRASH"),
        (Catapult, P::Discard) if cards => ("DISCARD", "CATAPULT_DISCARD"),
        (Charm, P::Mode) => ("CHOOSE_MODE", "CHARM_MODE"),
        (Charm, P::Gain) if pile => ("GAIN", "CHARM_GAIN"),
        (Capital, P::Other) if number => ("HOW_MUCH_REPAY", "GAME_REPAY_HOW_MUCH"),
        (Crown, P::Play) if cards && view.turn().phase == Phase::Buy => ("PLAY", "CROWN_TREASURE"),
        (Crown, P::Play) if cards => ("PLAY", "CROWN_ACTION"),
        (Encampment, P::Reveal) if cards => ("REVEAL", "ENCAMPMENT"),
        (Forum, P::Discard) if cards => ("DISCARD", "FORUM"),
        (Gladiator, P::Reveal) if cards && turn_player => ("REVEAL", "GLADIATOR_REVEAL"),
        (Gladiator, P::Reveal) if cards => ("REVEAL", "GLADIATOR_BLOCK"),
        (HauntedCastle, P::TopDeck) if cards => ("TOPDECK", "HAUNTED_CASTLE"),
        (Legionary, P::Reveal) if cards => ("REVEAL", "LEGIONARY_REVEAL"),
        (Legionary, P::Discard) if cards => ("DISCARD", "LEGIONARY_DISCARD"),
        (OpulentCastle, P::Discard) if cards => ("DISCARD", "OPULENT_CASTLE"),
        (Overlord, P::Play) if pile => ("PLAY", "OVERLORD"),
        (Sacrifice, P::Trash) if cards => ("TRASH", "SACRIFICE"),
        (SmallCastle, P::Trash) if cards => {
            let in_play = view
                .pending_source()
                .is_some_and(|id| view.zone(id) == Some(Zone::InPlay(d.player)));
            if in_play {
                ("TRASH", "SMALL_CASTLE_IN_PLAY")
            } else {
                ("TRASH", "SMALL_CASTLE_GONE")
            }
        }
        (SprawlingCastle, P::Either) => ("CHOOSE_MODE", "SPRAWLING_CASTLE"),
        (Temple, P::Trash) if cards => ("TRASH", "TEMPLE"),
        (WildHunt, P::Mode) => ("CHOOSE_MODE", "WILD_HUNT"),
        (Advance, P::Trash) if cards => ("TRASH", "ADVANCE_TRASH"),
        (Advance, P::Gain) if pile => ("GAIN", "ADVANCE_GAIN"),
        (Annex, P::Other) if cards => ("KEEP_IN_DISCARD", "ANNEX"),
        (Banquet, P::Gain) if pile => ("GAIN", "BANQUET"),
        (Donate, P::Trash) if cards => ("TRASH", "DONATE"),
        (Ritual, P::Trash) if cards => ("TRASH", "RITUAL"),
        (SaltTheEarth, P::Trash) if pile => ("TRASH", "SALT_THE_EARTH"),
        (Tax, P::Other) if pile => ("SELECT_ZONE", "TAX"),
        (Arena, P::Discard) if cards => ("DISCARD", "ARENA"),
        (MountainPass, P::Other) if number => match view.pending_subject() {
            Some(_) => ("BID", "MOUNTAIN_PASS_OVERBID"),
            None => ("BID", "MOUNTAIN_PASS_NEW"),
        },
        _ => return None,
    };
    let args = match story {
        "CHARM_GAIN" => match charm_arguments(view, d, c) {
            Ok(args) => args,
            Err(e) => return Some(Err(e)),
        },
        "GLADIATOR_BLOCK" => {
            let named = match &d.kind {
                DecisionKind::Cards { candidates, .. } => {
                    candidates.first().and_then(|&id| view.card(id))
                }
                _ => None,
            };
            match named.map(|card| names(c, &[card])) {
                Some(Ok(arg)) => vec![arg],
                Some(Err(e)) => return Some(Err(e)),
                None => return Some(Err(gap(d, "Gladiator's revealed card"))),
            }
        }
        "ARENA" => vec![LogArgument::Number(2)],
        "MOUNTAIN_PASS_OVERBID" => {
            let DecisionKind::Number { min, .. } = d.kind else {
                return Some(Err(gap(d, "Mountain Pass bid")));
            };
            let Some(bidder) = view.pending_subject() else {
                return Some(Err(gap(d, "Mountain Pass bidder")));
            };
            vec![
                LogArgument::Player(bidder.0.into()),
                LogArgument::Number(min),
            ]
        }
        _ => vec![],
    };
    Some(Ok((kind, story, args)))
}

pub(super) fn option_shape(
    _view: &View<'_>,
    _d: &Decision,
    _c: &WireCatalog,
) -> Option<Result<OptionShape, ProjectionError>> {
    None
}

pub(super) fn decline_button(d: &Decision) -> Option<&'static str> {
    use Card::*;
    Some(match (d.source?, d.purpose) {
        (Crown, Purpose::Play) => "CROWN_DONT",
        (Encampment, Purpose::Reveal) => "ENCAMPMENT_DONT_REVEAL",
        (Legionary, Purpose::Reveal) => "LEGIONARY_DONT_REVEAL",
        (Gladiator, Purpose::Reveal) => "GLADIATOR_DONT_REVEAL",
        (Annex, Purpose::Other) => "SHUFFLE_ALL",
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        Charm => &["CHARM_BUY_MONEY", "CHARM_COPY"],
        WildHunt => &["WILD_HUNT_CARDS", "WILD_HUNT_POINTS"],
        SprawlingCastle => &["SPRAWLING_CASTLE_DUCHY", "SPRAWLING_CASTLE_ESTATES"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (card, ability) {
        (Archive, 0) => "ARCHIVE_ABILITY",
        (Enchantress, 0) => "ENCHANTRESS_ABILITY",
        (Enchantress, 1) => "ENCHANTRESS_CANTRIP",
        (Donate, 0) => "DONATE",
        (Rocks, 0 | 1) => "ROCKS_GAINS_SILVER",
        (CrumblingCastle, 0 | 1) => "CRUMBLING_CASTLE",
        (Emporium, 0) => "EMPORIUM_GAIN_POINTS",
        (Fortune, 0) => "FORTUNE",
        (Forum, 0) => "FORUM",
        (HauntedCastle, 0) => "HAUNTED_CASTLE",
        (SprawlingCastle, 0) => "SPRAWLING_CASTLE",
        (GrandCastle, 0) => "GRAND_CASTLE",
        (Temple, 0) => "TEMPLE",
        (Villa, 0) => "VILLA",
        (Capital, 0) => "CAPITAL_TAKE_DEBT",
        (Charm, 0) => "CHARM",
        (Encampment, 0) => "ENCAMPMENT_RETURNED",
        (Aqueduct, 0) => "VP_TO_AQUEDUCT",
        (Aqueduct, 1) => "VP_FROM_AQUEDUCT",
        (DefiledShrine, 0) => "VP_TO_DEFILED_SHRINE",
        (DefiledShrine, 1) => "VP_FROM_DEFILED_SHRINE",
        (Basilica, 1) => "VP_FROM_BASILICA",
        (Baths, 1) => "VP_FROM_BATHS",
        (Battlefield, 1) => "VP_FROM_BATTLEFIELD",
        (Colonnade, 1) => "VP_FROM_COLONNADE",
        (Labyrinth, 1) => "VP_FROM_LABYRINTH",
        (Arena, 0) => "ARENA_DISCARD_ACTION",
        (MountainPass, 0) => "MOUNTAIN_PASS_BIDDING",
        (Tomb, 0) => "TOMB",
        (Tax, 0) => "TAX_RECEIVE",
        _ => return None,
    })
}

pub(super) fn ability_arguments(
    view: &View<'_>,
    d: &Decision,
    t: &Trigger,
    symbol: &str,
    c: &WireCatalog,
) -> Option<Result<Vec<LogArgument>, ProjectionError>> {
    if symbol != "ARCHIVE_ABILITY" {
        return None;
    }
    let cards: Vec<Card> = view
        .lasting()
        .into_iter()
        .find(|l| Some(l.id) == t.lasting)
        .map(|l| {
            l.data
                .cards
                .iter()
                .filter_map(|&id| view.card(id))
                .collect()
        })
        .unwrap_or_default();
    if cards.is_empty() {
        return Some(Err(gap(d, "Archive's set-aside cards")));
    }
    Some(names(c, &cards).map(|a| vec![a]))
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Archive => &[
            ("AbilityNames", "ARCHIVE_ABILITY"),
            ("QuestionIds", "ARCHIVE"),
            ("QuestionTypes", "PUT_IN_HAND"),
        ],
        Capital => &[
            ("AbilityNames", "CAPITAL_TAKE_DEBT"),
            ("QuestionIds", "GAME_REPAY_HOW_MUCH"),
            ("QuestionTypes", "HOW_MUCH_REPAY"),
        ],
        Castles | HumbleCastle | KingsCastle => &[],
        Catapult | Rocks => &[
            ("AbilityNames", "ROCKS_GAINS_SILVER"),
            ("QuestionIds", "CATAPULT_DISCARD"),
            ("QuestionIds", "CATAPULT_TRASH"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "TRASH"),
        ],
        Charm => &[
            ("AbilityNames", "CHARM"),
            ("CardModes", "CHARM_BUY_MONEY"),
            ("CardModes", "CHARM_COPY"),
            ("QuestionIds", "CHARM_GAIN"),
            ("QuestionIds", "CHARM_MODE"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
        ],
        Crown => &[
            ("DeclineButtonIds", "CROWN_DONT"),
            ("QuestionIds", "CROWN_ACTION"),
            ("QuestionIds", "CROWN_TREASURE"),
            ("QuestionTypes", "PLAY"),
        ],
        CrumblingCastle => &[("AbilityNames", "CRUMBLING_CASTLE")],
        Encampment | Plunder => &[
            ("AbilityNames", "ENCAMPMENT_RETURNED"),
            ("DeclineButtonIds", "ENCAMPMENT_DONT_REVEAL"),
            ("QuestionIds", "ENCAMPMENT"),
            ("QuestionTypes", "REVEAL"),
        ],
        Enchantress => &[
            ("AbilityNames", "ENCHANTRESS_ABILITY"),
            ("AbilityNames", "ENCHANTRESS_CANTRIP"),
        ],
        Forum => &[
            ("AbilityNames", "FORUM"),
            ("QuestionIds", "FORUM"),
            ("QuestionTypes", "DISCARD"),
        ],
        Gladiator | Fortune => &[
            ("AbilityNames", "FORTUNE"),
            ("DeclineButtonIds", "GLADIATOR_DONT_REVEAL"),
            ("QuestionIds", "GLADIATOR_BLOCK"),
            ("QuestionIds", "GLADIATOR_REVEAL"),
            ("QuestionTypes", "REVEAL"),
        ],
        GrandCastle => &[("AbilityNames", "GRAND_CASTLE")],
        HauntedCastle => &[
            ("AbilityNames", "HAUNTED_CASTLE"),
            ("QuestionIds", "HAUNTED_CASTLE"),
            ("QuestionTypes", "TOPDECK"),
        ],
        Legionary => &[
            ("DeclineButtonIds", "LEGIONARY_DONT_REVEAL"),
            ("QuestionIds", "LEGIONARY_DISCARD"),
            ("QuestionIds", "LEGIONARY_REVEAL"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "REVEAL"),
        ],
        OpulentCastle => &[
            ("QuestionIds", "OPULENT_CASTLE"),
            ("QuestionTypes", "DISCARD"),
        ],
        Overlord => &[("QuestionIds", "OVERLORD"), ("QuestionTypes", "PLAY")],
        Patrician | Emporium => &[("AbilityNames", "EMPORIUM_GAIN_POINTS")],
        Sacrifice => &[("QuestionIds", "SACRIFICE"), ("QuestionTypes", "TRASH")],
        SmallCastle => &[
            ("QuestionIds", "SMALL_CASTLE_GONE"),
            ("QuestionIds", "SMALL_CASTLE_IN_PLAY"),
            ("QuestionTypes", "TRASH"),
        ],
        SprawlingCastle => &[
            ("AbilityNames", "SPRAWLING_CASTLE"),
            ("CardModes", "SPRAWLING_CASTLE_DUCHY"),
            ("CardModes", "SPRAWLING_CASTLE_ESTATES"),
            ("QuestionIds", "SPRAWLING_CASTLE"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Temple => &[
            ("AbilityNames", "TEMPLE"),
            ("QuestionIds", "TEMPLE"),
            ("QuestionTypes", "TRASH"),
        ],
        Villa => &[("AbilityNames", "VILLA")],
        WildHunt => &[
            ("CardModes", "WILD_HUNT_CARDS"),
            ("CardModes", "WILD_HUNT_POINTS"),
            ("QuestionIds", "WILD_HUNT"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Advance => &[
            ("QuestionIds", "ADVANCE_GAIN"),
            ("QuestionIds", "ADVANCE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Annex => &[
            ("DeclineButtonIds", "SHUFFLE_ALL"),
            ("QuestionIds", "ANNEX"),
            ("QuestionTypes", "KEEP_IN_DISCARD"),
        ],
        Banquet => &[("QuestionIds", "BANQUET"), ("QuestionTypes", "GAIN")],
        Donate => &[
            ("AbilityNames", "DONATE"),
            ("QuestionIds", "DONATE"),
            ("QuestionTypes", "TRASH"),
        ],
        Ritual => &[("QuestionIds", "RITUAL"), ("QuestionTypes", "TRASH")],
        SaltTheEarth => &[
            ("QuestionIds", "SALT_THE_EARTH"),
            ("QuestionTypes", "TRASH"),
        ],
        Tax => &[
            ("AbilityNames", "TAX_RECEIVE"),
            ("QuestionIds", "TAX"),
            ("QuestionTypes", "SELECT_ZONE"),
        ],
        Aqueduct => &[
            ("AbilityNames", "VP_FROM_AQUEDUCT"),
            ("AbilityNames", "VP_TO_AQUEDUCT"),
        ],
        Arena => &[
            ("AbilityNames", "ARENA_DISCARD_ACTION"),
            ("QuestionIds", "ARENA"),
            ("QuestionTypes", "DISCARD"),
        ],
        Basilica => &[("AbilityNames", "VP_FROM_BASILICA")],
        Baths => &[("AbilityNames", "VP_FROM_BATHS")],
        Battlefield => &[("AbilityNames", "VP_FROM_BATTLEFIELD")],
        Colonnade => &[("AbilityNames", "VP_FROM_COLONNADE")],
        DefiledShrine => &[
            ("AbilityNames", "VP_FROM_DEFILED_SHRINE"),
            ("AbilityNames", "VP_TO_DEFILED_SHRINE"),
        ],
        Labyrinth => &[("AbilityNames", "VP_FROM_LABYRINTH")],
        MountainPass => &[
            ("AbilityNames", "MOUNTAIN_PASS_BIDDING"),
            ("QuestionIds", "MOUNTAIN_PASS_NEW"),
            ("QuestionIds", "MOUNTAIN_PASS_OVERBID"),
            ("QuestionTypes", "BID"),
        ],
        Tomb => &[("AbilityNames", "TOMB")],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        Archive => &[
            "card-selection",
            "set-aside",
            "duration-reminder",
            "duration-ability",
        ],
        Capital | CityQuarter | RoyalBlacksmith => &["debt"],
        Castles | HumbleCastle | KingsCastle => &["split-pile", "vp-tokens"],
        CrumblingCastle | GrandCastle => &["split-pile", "vp-tokens", "duration-ability"],
        SmallCastle | OpulentCastle | HauntedCastle => &["split-pile", "card-selection"],
        SprawlingCastle => &["split-pile", "semantic-modes", "duration-ability"],
        Catapult | Rocks => &["split-pile", "off-turn-attack", "card-selection"],
        ChariotRace | FarmersMarket | Groundskeeper => &["vp-tokens"],
        Charm => &["semantic-modes", "pile-selection", "duration-ability"],
        Crown => &["repeated-play", "card-selection"],
        Encampment | Plunder => &["split-pile", "set-aside", "card-selection", "vp-tokens"],
        Enchantress => &["public-effects", "duration-reminder", "duration-ability"],
        Forum => &["card-selection"],
        Gladiator | Fortune => &["split-pile", "card-selection"],
        Legionary => &["off-turn-attack", "card-selection"],
        Overlord => &["debt", "pile-selection", "repeated-play"],
        Patrician | Emporium => &["split-pile", "looking", "vp-tokens"],
        Sacrifice | Temple => &["card-selection", "vp-tokens"],
        Villa => &["duration-ability"],
        WildHunt => &["semantic-modes", "vp-tokens"],
        Advance => &["event", "card-selection", "pile-selection"],
        Annex => &["event", "debt", "discard-inspection", "card-selection"],
        Banquet => &["event", "pile-selection"],
        Conquest | Dominate => &["event", "vp-tokens"],
        Delve | Windfall => &["event"],
        Donate => &["event", "debt", "card-selection", "duration-ability"],
        Ritual => &["event", "card-selection", "vp-tokens"],
        SaltTheEarth => &["event", "pile-selection", "vp-tokens"],
        Triumph | Wedding => &["event", "debt", "vp-tokens"],
        BanditFort | Fountain | Keep | Museum | Obelisk | Orchard | Palace | Tower
        | TriumphalArch | Wall | WolfDen => &["landscape"],
        Tomb => &["landscape", "vp-tokens", "duration-ability"],
        Aqueduct | Basilica | Baths | Battlefield | Colonnade | DefiledShrine | Labyrinth => {
            &["landscape", "vp-tokens", "duration-ability"]
        }
        Arena => &[
            "landscape",
            "vp-tokens",
            "optional-ability",
            "card-selection",
        ],
        MountainPass => &["landscape", "debt", "vp-tokens"],
        _ => return None,
    })
}
