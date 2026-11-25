//! Question bindings for this set's cards (see `semantics::SETS`).
use super::super::OptionShape;
use super::{Binding, ProjectionError, WireCatalog, gap};
use dominion_engine::{Card, Cost, Decision, DecisionKind, Purpose, Trigger, View};
use dominion_online::questions::{CardFrequency, LogArgument};

/// The engine's number for the Exile rule's ability on a gained card (`EXILE_DISCARD`).
const EXILE_DISCARD: u8 = 253;

fn names(c: &WireCatalog, card: Card) -> Result<LogArgument, ProjectionError> {
    Ok(LogArgument::CardNames(vec![CardFrequency {
        name: c.card(card)?,
        frequency: 1,
    }]))
}

fn amount(cost: Cost) -> LogArgument {
    LogArgument::Amount {
        coins: cost.coins.into(),
        potions: cost.potions.into(),
        debt: cost.debt.into(),
    }
}

fn cost_limit(view: &View<'_>, d: &Decision) -> Result<LogArgument, ProjectionError> {
    view.pending_cost_limit()
        .map(amount)
        .ok_or_else(|| gap(d, "missing explicit cost bound"))
}

/// The card a Way's ability (or Gamble) acts on, visible to everyone.
fn named(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
    id: Option<dominion_engine::CardId>,
) -> Result<LogArgument, ProjectionError> {
    let card = id
        .and_then(|id| view.card(id))
        .ok_or_else(|| gap(d, "missing visible referenced card"))?;
    names(c, card)
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
    let choose = matches!(d.kind, DecisionKind::Choose { .. });
    let (kind, story) = match (d.source?, d.purpose) {
        (WayOfTheGoat, P::Trash) if cards => ("TRASH", "WAY_OF_THE_GOAT"),
        (WayOfTheRat, P::Discard) if cards => ("DISCARD", "WAY_OF_THE_RAT"),
        (WayOfTheButterfly, P::Return) if choose => ("CHOOSE_MODE", "BUTTERFLY_RETURN"),
        (WayOfTheButterfly, P::Gain) if pile => ("GAIN", "BUTTERFLY_GAIN"),
        (Wayfarer, P::Gain) if choose => ("CHOOSE_MODE", "WAYFARER"),
        (Kiln, P::Gain) if choose => ("RESOLVE_ABILITY", "GAME_MAY_REACT_WITH"),
        (AnimalFair, P::Trash) if choose => ("COMPLEX_OR", "ANIMAL_FAIR_MAY_TRASH"),
        (AnimalFair, P::Trash) if cards => ("TRASH", "ANIMAL_FAIR_TRASH"),
        (Mastermind, P::Play) if cards => ("PLAY", "MASTERMIND"),
        (BountyHunter, P::Exile) if cards => ("EXILE", "BOUNTY_HUNTER"),
        (CamelTrain, P::Exile) if pile => ("EXILE", "CAMEL_TRAIN"),
        (Cardinal, P::Exile) if cards => ("EXILE", "CARDINAL_EXILE"),
        (Displace, P::Exile) if cards => ("EXILE", "DISPLACE_EXILE"),
        (Displace, P::Gain) if pile => ("GAIN", "DISPLACE_GAIN"),
        (Falconer, P::Gain) if pile => ("GAIN", "FALCONER"),
        (Goatherd, P::Trash) if cards => ("TRASH", "GOATHERD"),
        (Groom, P::Gain) if pile => ("GAIN", "GROOM"),
        (HuntingLodge, P::Discard) if choose => ("COMPLEX_OR", "HUNTING_LODGE"),
        (Hostelry, P::Discard) if cards => ("DISCARD", "HOSTELRY"),
        (Sanctuary, P::Exile) if cards => ("EXILE", "SANCTUARY"),
        (Scrap, P::Trash) if cards => ("TRASH", "SCRAP_TRASH"),
        (Scrap, P::Mode) => ("CHOOSE_MODE", "SCRAP_MODES"),
        (Barge, P::Either) => ("CHOOSE_MODE", "BARGE"),
        (VillageGreen, P::Either) => ("CHOOSE_MODE", "VILLAGE_GREEN"),
        (Banish, P::Exile) if cards => ("EXILE", "BANISH"),
        (Bargain, P::Gain) if pile => ("GAIN", "BARGAIN"),
        (Delay, P::SetAside) if cards => ("SET_ASIDE", "DELAY"),
        (Demand, P::Gain) if pile => ("GAIN", "DEMAND"),
        (Desperation, P::Gain) if choose => ("CHOOSE_MODE", "DESPERATION"),
        (Enhance, P::Trash) if cards => ("TRASH", "ENHANCE_TRASH"),
        (Enhance, P::Gain) if pile => ("GAIN", "ENHANCE_GAIN"),
        (Gamble, P::Play) if choose => ("PLAY", "GAMBLE"),
        (Invest, P::Exile) if pile => ("EXILE", "INVEST"),
        (March, P::Play) if cards => ("PLAY", "MARCH"),
        (Toil, P::Play) if cards => ("PLAY", "TOIL"),
        (Populate, P::Gain) if pile => ("GAIN", "POPULATE"),
        (Pursue, P::Name) => ("WISH", "PURSUE"),
        _ => return None,
    };
    let args = match story {
        "BUTTERFLY_RETURN" => vec![match named(view, d, c, view.pending_source()) {
            Ok(a) => a,
            Err(e) => return Some(Err(e)),
        }],
        "GAMBLE" => vec![match named(view, d, c, view.pending_reference()) {
            Ok(a) => a,
            Err(e) => return Some(Err(e)),
        }],
        "BUTTERFLY_GAIN" | "DISPLACE_GAIN" | "ENHANCE_GAIN" => match cost_limit(view, d) {
            Ok(a) => vec![a],
            Err(e) => return Some(Err(e)),
        },
        "GROOM" => vec![amount(Cost::coins(4))],
        "SCRAP_MODES" => match d.kind {
            DecisionKind::Choose { max, .. } => vec![LogArgument::Number(max as i32)],
            _ => return Some(Err(gap(d, "Scrap modes"))),
        },
        _ => vec![],
    };
    Some(Ok((kind, story, args)))
}

pub(super) fn option_shape(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<OptionShape, ProjectionError>> {
    use Card::*;
    Some(Ok(match (d.source?, d.purpose) {
        (WayOfTheButterfly, Purpose::Return) => {
            OptionShape::Modes(vec!["BUTTERFLY_RETURN_IT", "BUTTERFLY_DO_NOTHING"])
        }
        (Wayfarer, Purpose::Gain) => OptionShape::Mode("WAYFARER_SILVER", "DONT_GAIN"),
        (Desperation, Purpose::Gain) => OptionShape::Mode("DESPERATION_CURSE", "DONT_GAIN"),
        (HuntingLodge, Purpose::Discard) => {
            OptionShape::Button("DISCARD_ALL", "HUNTING_LODGE_KEEP")
        }
        (AnimalFair, Purpose::Trash) => OptionShape::Button("CONTINUE", "DONT_TRASH"),
        (Kiln, Purpose::Gain) => match view.pending_source() {
            Some(card) => match view.card(card).map(|k| names(c, k)) {
                Some(Ok(arg)) => OptionShape::Ability("KILN", card, vec![arg]),
                Some(Err(e)) => return Some(Err(e)),
                None => return Some(Err(gap(d, "Kiln's card"))),
            },
            None => return Some(Err(gap(d, "missing visible played card"))),
        },
        (Gamble, Purpose::Play) => match view.pending_reference() {
            Some(card) => OptionShape::Card(card, "DONT_PLAY"),
            None => return Some(Err(gap(d, "missing visible revealed card"))),
        },
        _ => return None,
    }))
}

pub(super) fn decline_button(d: &Decision) -> Option<&'static str> {
    use Card::*;
    Some(match (d.source?, d.purpose) {
        (Mastermind, Purpose::Play) => "MASTERMIND_DONT",
        (March, Purpose::Play) => "MARCH_DONT",
        (Toil, Purpose::Play) => "TOIL_DONT",
        (Delay, Purpose::SetAside) => "DELAY_DONT",
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        Barge => &["BARGE_NOW", "BARGE_LATER"],
        VillageGreen => &["VILLAGE_GREEN_NOW", "VILLAGE_GREEN_LATER"],
        Scrap => &[
            "SCRAP_CARD",
            "SCRAP_ACTION",
            "SCRAP_BUY",
            "SCRAP_COIN",
            "SCRAP_SILVER",
            "SCRAP_HORSE",
        ],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    if ability == EXILE_DISCARD {
        return Some("DISCARD_FROM_EXILE");
    }
    Some(match (card, ability) {
        (BlackCat, 1) => "BLACK_CAT_REACTION",
        (Sheepdog, 1) => "SHEEPDOG_REACTION",
        (Falconer, 1) => "FALCONER_REACTION",
        (VillageGreen, 1) => "VILLAGE_GREEN_PLAY",
        (Barge, 0) => "BARGE",
        (VillageGreen, 0) => "VILLAGE_GREEN_VILLAGE",
        (Gatekeeper, 0) => "GATEKEEPER_DURATION",
        (Gatekeeper, 1) => "GATEKEEPER_EXILE",
        (Mastermind, 0) => "MASTERMIND",
        (Delay, 0) => "DELAY_PLAY",
        (Reap, 0) => "REAP_PLAY",
        (WayOfTheTurtle, 0) => "TURTLE_PLAY",
        (CamelTrain, 2) => "EXILE_BY_NAME",
        (Cavalry, 2) => "CAVALRY",
        (Hostelry, 2) => "HOSTELRY",
        (Invest, 0) => "INVEST",
        (Livery, 1) => "LIVERY",
        (WayOfTheFrog, 0) => "FROG_TOPDECK",
        (WayOfTheSeal, 0) => "TOPDECK_REACTION",
        (WayOfTheSquirrel, 0) => "DRAW_FROM_SQUIRREL",
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
    let card = match symbol {
        "DISCARD_FROM_EXILE" => Some(t.card),
        "EXILE_BY_NAME" => Some(Card::Gold),
        "FROG_TOPDECK" => usize::try_from(t.data)
            .ok()
            .and_then(|i| Card::ALL.get(i).copied()),
        "GATEKEEPER_EXILE" | "TOPDECK_REACTION" => t.source.and_then(|id| view.card(id)),
        "DELAY_PLAY" | "TURTLE_PLAY" => view
            .lasting()
            .into_iter()
            .find(|l| Some(l.id) == t.lasting)
            .and_then(|l| l.data.pick)
            .and_then(|id| view.card(id)),
        _ => return None,
    };
    Some(
        card.ok_or_else(|| gap(d, &format!("{symbol} card")))
            .and_then(|card| Ok(vec![names(c, card)?])),
    )
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        AnimalFair => &[
            ("QuestionIds", "ANIMAL_FAIR_MAY_TRASH"),
            ("QuestionIds", "ANIMAL_FAIR_TRASH"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "BUTTON"),
            ("QuestionTypes", "TRASH"),
            ("DeclineButtonIds", "CONTINUE"),
            ("DeclineButtonIds", "DONT_TRASH"),
        ],
        Barge => &[
            ("AbilityNames", "BARGE"),
            ("CardModes", "BARGE_LATER"),
            ("CardModes", "BARGE_NOW"),
            ("QuestionIds", "BARGE"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        BlackCat => &[("AbilityNames", "BLACK_CAT_REACTION")],
        BountyHunter => &[("QuestionIds", "BOUNTY_HUNTER"), ("QuestionTypes", "EXILE")],
        CamelTrain => &[
            ("AbilityNames", "EXILE_BY_NAME"),
            ("QuestionIds", "CAMEL_TRAIN"),
            ("QuestionTypes", "EXILE"),
        ],
        Cardinal => &[
            ("QuestionIds", "CARDINAL_EXILE"),
            ("QuestionTypes", "EXILE"),
        ],
        Cavalry => &[("AbilityNames", "CAVALRY")],
        Displace => &[
            ("QuestionIds", "DISPLACE_EXILE"),
            ("QuestionIds", "DISPLACE_GAIN"),
            ("QuestionTypes", "EXILE"),
            ("QuestionTypes", "GAIN"),
        ],
        Falconer => &[
            ("AbilityNames", "FALCONER_REACTION"),
            ("QuestionIds", "FALCONER"),
            ("QuestionTypes", "GAIN"),
        ],
        Gatekeeper => &[
            ("AbilityNames", "GATEKEEPER_DURATION"),
            ("AbilityNames", "GATEKEEPER_EXILE"),
        ],
        Goatherd => &[("QuestionIds", "GOATHERD"), ("QuestionTypes", "TRASH")],
        Groom => &[("QuestionIds", "GROOM"), ("QuestionTypes", "GAIN")],
        Hostelry => &[
            ("AbilityNames", "HOSTELRY"),
            ("QuestionIds", "HOSTELRY"),
            ("QuestionTypes", "DISCARD"),
        ],
        HuntingLodge => &[
            ("QuestionIds", "HUNTING_LODGE"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "BUTTON"),
            ("DeclineButtonIds", "DISCARD_ALL"),
            ("DeclineButtonIds", "HUNTING_LODGE_KEEP"),
        ],
        Kiln => &[
            ("AbilityNames", "KILN"),
            ("DeclineButtonIds", "DONE_REACTING"),
            ("QuestionIds", "GAME_MAY_REACT_WITH"),
            ("QuestionTypes", "RESOLVE_ABILITY"),
        ],
        Livery => &[("AbilityNames", "LIVERY")],
        Mastermind => &[
            ("AbilityNames", "MASTERMIND"),
            ("DeclineButtonIds", "MASTERMIND_DONT"),
            ("QuestionIds", "MASTERMIND"),
            ("QuestionTypes", "PLAY"),
        ],
        Sanctuary => &[("QuestionIds", "SANCTUARY"), ("QuestionTypes", "EXILE")],
        Scrap => &[
            ("CardModes", "SCRAP_ACTION"),
            ("CardModes", "SCRAP_BUY"),
            ("CardModes", "SCRAP_CARD"),
            ("CardModes", "SCRAP_COIN"),
            ("CardModes", "SCRAP_HORSE"),
            ("CardModes", "SCRAP_SILVER"),
            ("QuestionIds", "SCRAP_MODES"),
            ("QuestionIds", "SCRAP_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "TRASH"),
        ],
        Sheepdog => &[("AbilityNames", "SHEEPDOG_REACTION")],
        VillageGreen => &[
            ("AbilityNames", "VILLAGE_GREEN_PLAY"),
            ("AbilityNames", "VILLAGE_GREEN_VILLAGE"),
            ("CardModes", "VILLAGE_GREEN_LATER"),
            ("CardModes", "VILLAGE_GREEN_NOW"),
            ("QuestionIds", "VILLAGE_GREEN"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Wayfarer => &[
            ("CardModes", "WAYFARER_SILVER"),
            ("DeclineButtonIds", "DONT_GAIN"),
            ("QuestionIds", "WAYFARER"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Banish => &[("QuestionIds", "BANISH"), ("QuestionTypes", "EXILE")],
        Bargain => &[("QuestionIds", "BARGAIN"), ("QuestionTypes", "GAIN")],
        Delay => &[
            ("AbilityNames", "DELAY_PLAY"),
            ("DeclineButtonIds", "DELAY_DONT"),
            ("QuestionIds", "DELAY"),
            ("QuestionTypes", "SET_ASIDE"),
        ],
        Demand => &[("QuestionIds", "DEMAND"), ("QuestionTypes", "GAIN")],
        Desperation => &[
            ("CardModes", "DESPERATION_CURSE"),
            ("DeclineButtonIds", "DONT_GAIN"),
            ("QuestionIds", "DESPERATION"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Enhance => &[
            ("QuestionIds", "ENHANCE_GAIN"),
            ("QuestionIds", "ENHANCE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Gamble => &[("QuestionIds", "GAMBLE"), ("QuestionTypes", "PLAY")],
        Invest => &[
            ("AbilityNames", "INVEST"),
            ("QuestionIds", "INVEST"),
            ("QuestionTypes", "EXILE"),
        ],
        March => &[
            ("DeclineButtonIds", "MARCH_DONT"),
            ("QuestionIds", "MARCH"),
            ("QuestionTypes", "PLAY"),
        ],
        Populate => &[("QuestionIds", "POPULATE"), ("QuestionTypes", "GAIN")],
        Pursue => &[("QuestionIds", "PURSUE"), ("QuestionTypes", "WISH")],
        Reap => &[("AbilityNames", "REAP_PLAY")],
        SeizeTheDay => &[("ExtraTurnTypes", "SEIZE_THE_DAY")],
        Toil => &[
            ("DeclineButtonIds", "TOIL_DONT"),
            ("QuestionIds", "TOIL"),
            ("QuestionTypes", "PLAY"),
        ],
        WayOfTheButterfly => &[
            ("CardModes", "BUTTERFLY_DO_NOTHING"),
            ("CardModes", "BUTTERFLY_RETURN_IT"),
            ("QuestionIds", "BUTTERFLY_GAIN"),
            ("QuestionIds", "BUTTERFLY_RETURN"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
        ],
        WayOfTheFrog => &[("AbilityNames", "FROG_TOPDECK")],
        WayOfTheGoat => &[
            ("QuestionIds", "WAY_OF_THE_GOAT"),
            ("QuestionTypes", "TRASH"),
        ],
        WayOfTheRat => &[
            ("QuestionIds", "WAY_OF_THE_RAT"),
            ("QuestionTypes", "DISCARD"),
        ],
        WayOfTheSeal => &[("AbilityNames", "TOPDECK_REACTION")],
        WayOfTheSquirrel => &[("AbilityNames", "DRAW_FROM_SQUIRREL")],
        WayOfTheTurtle => &[("AbilityNames", "TURTLE_PLAY")],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        AnimalFair => &["card-selection", "optional-effect"],
        Barge | VillageGreen => &["semantic-modes", "duration-reminder", "duration-ability"],
        BlackCat => &["off-turn-attack", "reaction-ability"],
        BountyHunter | Sanctuary => &["exile", "card-selection"],
        CamelTrain => &["exile", "pile-selection", "duration-ability"],
        Cardinal => &["exile", "off-turn-attack", "looking", "card-selection"],
        Cavalry => &["non-supply-pile", "duration-ability"],
        Coven => &["exile", "off-turn-attack"],
        Displace => &[
            "exile",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Falconer => &["pile-selection", "reaction-ability"],
        Fisherman | Destrier => &["dynamic-cost"],
        Gatekeeper => &[
            "exile",
            "off-turn-attack",
            "duration-reminder",
            "duration-ability",
        ],
        Goatherd => &["card-selection"],
        Groom => &["pile-selection", "non-supply-pile"],
        Horse | Paddock | Supplies => &["non-supply-pile"],
        Hostelry => &["non-supply-pile", "card-selection", "duration-ability"],
        HuntingLodge => &["optional-effect"],
        Kiln => &["optional-ability", "pile-selection"],
        Livery => &["non-supply-pile", "duration-ability"],
        Mastermind => &[
            "repeated-play",
            "card-selection",
            "duration-reminder",
            "duration-ability",
        ],
        Scrap => &["card-selection", "semantic-modes", "non-supply-pile"],
        Sheepdog => &["reaction-ability"],
        SnowyVillage => &[],
        Stockpile => &["exile"],
        Wayfarer => &["semantic-modes", "dynamic-cost"],
        Alliance | Commerce => &["event"],
        Banish => &["event", "exile", "card-selection"],
        Bargain | Demand => &["event", "pile-selection", "non-supply-pile"],
        Delay | Reap => &["event", "set-aside", "card-selection", "duration-ability"],
        Desperation => &["event", "semantic-modes"],
        Enclave => &["event", "exile"],
        Invest => &["event", "exile", "pile-selection", "duration-ability"],
        SeizeTheDay => &["event", "extra-turn"],
        Enhance => &[
            "event",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Gamble => &["event", "looking", "optional-play"],
        March => &["event", "discard-inspection", "optional-play"],
        Populate => &["event", "pile-selection"],
        Pursue => &["event", "name-card", "looking"],
        Ride | Stampede => &["event", "non-supply-pile"],
        Toil => &["event", "optional-play"],
        WayOfTheButterfly => &["way", "semantic-modes", "pile-selection"],
        WayOfTheCamel | WayOfTheWorm => &["way", "exile"],
        WayOfTheFrog | WayOfTheSquirrel | WayOfTheSeal => &["way", "duration-ability"],
        WayOfTheGoat | WayOfTheRat => &["way", "card-selection"],
        WayOfTheHorse => &["way", "non-supply-pile"],
        WayOfTheTurtle => &["way", "set-aside", "duration-ability"],
        WayOfTheChameleon | WayOfTheMole | WayOfTheMonkey | WayOfTheMule | WayOfTheOtter
        | WayOfTheOwl | WayOfTheOx | WayOfThePig | WayOfTheSheep => &["way"],
        _ => return None,
    })
}
