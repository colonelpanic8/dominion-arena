//! Question bindings for this set's cards (see `semantics::SETS`).
use super::super::OptionShape;
use super::{Binding, ProjectionError, WireCatalog, gap};
use dominion_engine::{Card, Cost, Decision, DecisionKind, Event, Purpose, Trigger, View};
use dominion_online::questions::{CardFrequency, LogArgument};
use std::collections::BTreeMap;

/// One CardNames argument naming `cards`, counted.
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

/// The card `player` gained last: what a "when you gain" ability is about.
fn last_gained(view: &View<'_>, player: dominion_engine::PlayerId) -> Option<Card> {
    view.log().iter().rev().find_map(|e| match e {
        Event::Gained {
            player: p, card, ..
        } if *p == player => Some(*card),
        _ => None,
    })
}

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    _c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let pile = matches!(d.kind, DecisionKind::Pile { .. });
    let cards = matches!(d.kind, DecisionKind::Cards { .. });
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let (kind, story) = match (d.source?, d.purpose) {
        (Improve, P::Trash) if cards => ("TRASH", "IMPROVE_TRASH"),
        (Improve, P::Gain) if pile => ("GAIN", "IMPROVE_GAIN"),
        (Ducat, P::Trash) if cards => ("TRASH", "DUCAT"),
        (CityGate, P::TopDeck) if cards => ("TOPDECK", "CITY_GATE"),
        (Silos, P::Discard) if cards => ("DISCARD", "SILOS"),
        (SinisterPlot, P::Either) => ("CHOOSE_MODE", "SINISTER_PLOT"),
        (CropRotation, P::Discard) if cards => ("DISCARD", "CROP_ROTATION"),
        (Sewers, P::Trash) if cards => ("TRASH", "SEWERS"),
        (StarChart, P::TopDeck) if cards && !ordered => ("TOPDECK", "STAR_CHART"),
        (BorderGuard, P::Move) if cards => ("PUT_IN_HAND", "BORDER_GUARD_PUT_IN_HAND"),
        (BorderGuard, P::Either) => ("CHOOSE_MODE", "BORDER_GUARD_MODE"),
        (Hideout, P::Trash) if cards => ("TRASH", "HIDEOUT"),
        // Inventor has no prompt of its own; Workshop's says the same.
        (Inventor, P::Gain) if pile => ("GAIN", "WORKSHOP"),
        (MountainVillage, P::Move) if cards => ("PUT_IN_HAND", "MOUNTAIN_VILLAGE"),
        (OldWitch, P::Trash) if cards => ("TRASH", "OLD_WITCH"),
        (Priest, P::Trash) if cards => ("TRASH", "PRIEST"),
        (Research, P::Trash) if cards => ("TRASH", "RESEARCH_TRASH"),
        (Sculptor, P::Gain) if pile => ("GAIN", "SCULPTOR"),
        (Seer, P::TopDeck) if ordered => ("ORDER_CARDS", "SEER"),
        (Treasurer, P::Mode) => ("CHOOSE_MODE", "TREASURER_MODE"),
        (Treasurer, P::Trash) if cards => ("TRASH", "TREASURER_TRASH"),
        (Treasurer, P::Gain) if cards => ("GAIN", "TREASURER_GAIN"),
        (Villain, P::Discard) if cards => ("DISCARD", "VILLAIN"),
        _ => return None,
    };
    let args = match story {
        "IMPROVE_GAIN" => match view.pending_cost_limit() {
            Some(cost) => vec![amount(cost)],
            None => return Some(Err(gap(d, "missing explicit cost bound"))),
        },
        "SEWERS" => vec![LogArgument::Number(1)],
        "SINISTER_PLOT" => {
            let tokens = view
                .notes()
                .into_iter()
                .find(|n| n.source == SinisterPlot && n.player == Some(d.player))
                .map_or(0, |n| n.count);
            vec![LogArgument::Number(tokens)]
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

pub(super) fn decline_button(_d: &Decision) -> Option<&'static str> {
    None
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        SinisterPlot => &["SINISTER_PLOT_ADD", "SINISTER_PLOT_DRAW"],
        BorderGuard => &["BORDER_GUARD_LANTERN", "BORDER_GUARD_HORN"],
        Treasurer => &["TREASURER_TRASH", "TREASURER_GAIN", "TREASURER_KEY"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (card, ability) {
        (CargoShip, 0) => "CARGO_SHIP_RETURN",
        (Research, 0) => "RESEARCH",
        (CargoShip, 3) => "CARGO_SHIP_SET_ASIDE",
        (Improve, 3) => "IMPROVE",
        (Priest, 3) => "PRIEST",
        (Ducat, 1) => "DUCAT",
        (Lackeys, 1) => "LACKEYS",
        (Spices, 1) => "SPICES",
        (SilkMerchant, 1 | 2) => "SILK_MERCHANT",
        (FlagBearer, 1 | 2) => "FLAG_BEARER",
        (TreasureChest, 0) => "TREASURE_CHEST",
        (Key, 0) => "KEY",
        (Horn, 0) => "HORN_TOPDECK",
        (Exploration, 0) => "EXPLORATION",
        (Pageant, 0) => "PAGEANT",
        (Citadel, 0) => "CITADEL",
        (StarChart, 0) => "STAR_CHART",
        (Silos, 0) => "SILOS",
        (CropRotation, 0) => "CROP_ROTATION",
        (Fair, 0) => "FAIR",
        (Barracks, 0) => "BARRACKS",
        (CityGate, 0) => "CITY_GATE",
        (SinisterPlot, 0) => "SINISTER_PLOT",
        (Piazza, 0) => "PIAZZA",
        (Academy, 0) => "ACADEMY",
        (Guildhall, 0) => "GUILDHALL",
        (Innovation, 0) => "INNOVATION",
        (Sewers, 0) => "SEWERS_MAY_TRASH",
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
    let lasting = || view.lasting().into_iter().find(|l| Some(l.id) == t.lasting);
    let cards: Vec<Card> = match symbol {
        "CARGO_SHIP_RETURN" => lasting()
            .and_then(|l| l.data.pick)
            .and_then(|id| view.card(id))
            .into_iter()
            .collect(),
        "RESEARCH" => lasting()
            .map(|l| {
                l.data
                    .cards
                    .iter()
                    .filter_map(|&id| view.card(id))
                    .collect()
            })
            .unwrap_or_default(),
        "CITADEL" => t.source.and_then(|id| view.card(id)).into_iter().collect(),
        "CARGO_SHIP_SET_ASIDE" | "INNOVATION" => last_gained(view, t.owner).into_iter().collect(),
        _ => return None,
    };
    if cards.is_empty() {
        return Some(Err(gap(d, &format!("{symbol} cards"))));
    }
    Some(names(c, &cards).map(|a| vec![a]))
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Academy => &[("AbilityNames", "ACADEMY")],
        Barracks => &[("AbilityNames", "BARRACKS")],
        BorderGuard => &[
            ("CardModes", "BORDER_GUARD_HORN"),
            ("CardModes", "BORDER_GUARD_LANTERN"),
            ("QuestionIds", "BORDER_GUARD_MODE"),
            ("QuestionIds", "BORDER_GUARD_PUT_IN_HAND"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "PUT_IN_HAND"),
        ],
        CargoShip => &[
            ("AbilityNames", "CARGO_SHIP_RETURN"),
            ("AbilityNames", "CARGO_SHIP_SET_ASIDE"),
        ],
        Citadel => &[("AbilityNames", "CITADEL")],
        Fleet => &[("ExtraTurnTypes", "FLEET")],
        Horn => &[("AbilityNames", "HORN_TOPDECK")],
        Key => &[("AbilityNames", "KEY")],
        TreasureChest | Swashbuckler => &[("AbilityNames", "TREASURE_CHEST")],
        CityGate => &[
            ("AbilityNames", "CITY_GATE"),
            ("QuestionIds", "CITY_GATE"),
            ("QuestionTypes", "TOPDECK"),
        ],
        CropRotation => &[
            ("AbilityNames", "CROP_ROTATION"),
            ("QuestionIds", "CROP_ROTATION"),
            ("QuestionTypes", "DISCARD"),
        ],
        Ducat => &[
            ("AbilityNames", "DUCAT"),
            ("QuestionIds", "DUCAT"),
            ("QuestionTypes", "TRASH"),
        ],
        Exploration => &[("AbilityNames", "EXPLORATION")],
        Fair => &[("AbilityNames", "FAIR")],
        FlagBearer => &[("AbilityNames", "FLAG_BEARER")],
        Guildhall => &[("AbilityNames", "GUILDHALL")],
        Hideout => &[("QuestionIds", "HIDEOUT"), ("QuestionTypes", "TRASH")],
        Improve => &[
            ("AbilityNames", "IMPROVE"),
            ("QuestionIds", "IMPROVE_GAIN"),
            ("QuestionIds", "IMPROVE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Innovation => &[("AbilityNames", "INNOVATION")],
        Inventor => &[("QuestionIds", "WORKSHOP"), ("QuestionTypes", "GAIN")],
        Lackeys => &[("AbilityNames", "LACKEYS")],
        MountainVillage => &[
            ("QuestionIds", "MOUNTAIN_VILLAGE"),
            ("QuestionTypes", "PUT_IN_HAND"),
        ],
        OldWitch => &[("QuestionIds", "OLD_WITCH"), ("QuestionTypes", "TRASH")],
        Pageant => &[("AbilityNames", "PAGEANT")],
        Piazza => &[("AbilityNames", "PIAZZA")],
        Priest => &[
            ("AbilityNames", "PRIEST"),
            ("QuestionIds", "PRIEST"),
            ("QuestionTypes", "TRASH"),
        ],
        Research => &[
            ("AbilityNames", "RESEARCH"),
            ("QuestionIds", "RESEARCH_TRASH"),
            ("QuestionTypes", "TRASH"),
        ],
        Sculptor => &[("QuestionIds", "SCULPTOR"), ("QuestionTypes", "GAIN")],
        Seer => &[("QuestionIds", "SEER"), ("QuestionTypes", "ORDER_CARDS")],
        Sewers => &[
            ("AbilityNames", "SEWERS_MAY_TRASH"),
            ("QuestionIds", "SEWERS"),
            ("QuestionTypes", "TRASH"),
        ],
        SilkMerchant => &[("AbilityNames", "SILK_MERCHANT")],
        Silos => &[
            ("AbilityNames", "SILOS"),
            ("QuestionIds", "SILOS"),
            ("QuestionTypes", "DISCARD"),
        ],
        SinisterPlot => &[
            ("AbilityNames", "SINISTER_PLOT"),
            ("CardModes", "SINISTER_PLOT_ADD"),
            ("CardModes", "SINISTER_PLOT_DRAW"),
            ("QuestionIds", "SINISTER_PLOT"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Spices => &[("AbilityNames", "SPICES")],
        StarChart => &[
            ("AbilityNames", "STAR_CHART"),
            ("QuestionIds", "STAR_CHART"),
            ("QuestionTypes", "TOPDECK"),
        ],
        Treasurer => &[
            ("CardModes", "TREASURER_GAIN"),
            ("CardModes", "TREASURER_KEY"),
            ("CardModes", "TREASURER_TRASH"),
            ("QuestionIds", "TREASURER_GAIN"),
            ("QuestionIds", "TREASURER_MODE"),
            ("QuestionIds", "TREASURER_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Villain => &[("QuestionIds", "VILLAIN"), ("QuestionTypes", "DISCARD")],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        ActingTroupe => &["villagers"],
        CargoShip => &[
            "set-aside",
            "duration-reminder",
            "duration-ability",
            "optional-ability",
        ],
        Ducat => &["coffers", "optional-ability", "card-selection"],
        Hideout => &["card-selection"],
        Improve => &[
            "optional-ability",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Inventor => &["pile-selection", "dynamic-cost"],
        Lackeys => &["villagers", "duration-ability"],
        MountainVillage => &["discard-inspection", "card-selection"],
        OldWitch => &["off-turn-attack", "card-selection"],
        Patron => &["villagers", "coffers"],
        Priest => &["card-selection", "duration-ability"],
        Research => &[
            "card-selection",
            "set-aside",
            "duration-reminder",
            "duration-ability",
        ],
        Scholar => &[],
        Sculptor => &["pile-selection", "villagers"],
        Seer => &["looking", "ordered-cards"],
        SilkMerchant => &["coffers", "villagers", "duration-ability"],
        Spices => &["coffers", "duration-ability"],
        Villain => &["coffers", "off-turn-attack", "card-selection"],
        BorderGuard => &["looking", "card-selection", "semantic-modes"],
        Swashbuckler => &["coffers"],
        Treasurer => &["semantic-modes", "card-selection"],
        Horn | Key | TreasureChest => &["duration-ability"],
        Lantern => &[],
        SinisterPlot => &["project", "semantic-modes", "duration-ability"],
        Fleet => &["project"],
        Academy => &["project", "villagers", "duration-ability"],
        Barracks | Fair => &["project", "duration-ability"],
        Canal => &["project", "dynamic-cost"],
        Capitalism => &["project"],
        Citadel => &["project", "repeated-play", "duration-ability"],
        CityGate => &["project", "card-selection", "duration-ability"],
        CropRotation | Sewers | Silos => &["project", "optional-ability", "card-selection"],
        Exploration => &["project", "coffers", "villagers", "duration-ability"],
        Guildhall => &["project", "coffers", "duration-ability"],
        Innovation => &["project", "optional-ability"],
        Pageant => &["project", "coffers", "optional-ability"],
        Piazza => &["project", "looking", "duration-ability"],
        StarChart => &["project", "optional-ability", "card-selection", "looking"],
        _ => return None,
    })
}
