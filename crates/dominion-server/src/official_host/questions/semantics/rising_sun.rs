//! Question bindings for this set's cards (see `semantics::SETS`).
use super::allies::names;
use super::{Binding, ChoiceShape, ProjectionError, WireCatalog, gap};
use dominion_engine::{Card, Decision, DecisionKind, Event, Opt, Purpose, Trigger, Types, View};
use dominion_online::questions::LogArgument;

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let choosing = matches!(d.kind, DecisionKind::Choose { .. });
    let (kind, story) = match (d.source?, d.purpose) {
        (Alley, P::Discard) => ("DISCARD", "ALLEY"),
        (Change, P::Trash) => ("TRASH", "CHANGE_TRASH"),
        (Change, P::Gain) => ("GAIN", "CHANGE_GAIN"),
        (Craftsman, P::Gain) => ("GAIN", "CRAFTSMAN"),
        (GoldMine, P::Gain) if choosing => ("GAIN", "GOLD_MINE"),
        (Ninja, P::Discard) => ("DISCARD", "NINJA"),
        (Samurai, P::Discard) => ("DISCARD", "SAMURAI"),
        (SnakeWitch, P::Reveal) if choosing => ("REVEAL", "SNAKE_WITCH"),
        (Tanuki, P::Trash) => ("TRASH", "TANUKI_TRASH"),
        (Tanuki, P::Gain) => ("GAIN", "TANUKI_GAIN"),
        (Amass, P::Gain) => ("GAIN", "AMASS"),
        (Continue, P::Gain) => ("GAIN", "CONTINUE"),
        (Credit, P::Gain) => ("GAIN", "CREDIT"),
        (Gather, P::Gain) => ("GAIN", "GATHER"),
        (Kintsugi, P::Trash) => ("TRASH", "KINTSUGI_TRASH"),
        (Kintsugi, P::Gain) => ("GAIN", "KINTSUGI_GAIN"),
        (Practice, P::Play) => ("PLAY", "PRACTICE"),
        (ReceiveTribute, P::Gain) => ("GAIN", receive_tribute_story(view, d)),
        (SeaTrade, P::Trash) => ("TRASH", "SEA_TRADE"),
        (Kitsune, P::Mode) => ("CHOOSE_MODE", "KITSUNE"),
        (MountainShrine, P::Trash) => ("TRASH", "MOUNTAIN_SHRINE"),
        (RiverShrine, P::Trash) => ("TRASH", "RIVER_SHRINE_TRASH"),
        (RiverShrine, P::Gain) => ("GAIN", "RIVER_SHRINE_GAIN"),
        (RusticVillage, P::Discard) if choosing => ("DISCARD", "RUSTIC_VILLAGE_CHOICE"),
        (RusticVillage, P::Discard) => ("DISCARD", "RUSTIC_VILLAGE_DISCARD"),
        (Sickness, P::Mode) => ("CHOOSE_MODE", "SICKNESS"),
        (Sickness, P::Discard) => ("DISCARD", "SICKNESS_DISCARD"),
        (Growth, P::Gain) => ("GAIN", "GROWTH"),
        (KindEmperor, P::Gain) => ("GAIN", "KIND_EMPEROR"),
        _ => return None,
    };
    Some(arguments(view, d, c, story).map(|args| (kind, story, args)))
}

fn arguments(
    view: &View<'_>,
    d: &Decision,
    _c: &WireCatalog,
    story: &str,
) -> Result<Vec<LogArgument>, ProjectionError> {
    let amount = |cost: dominion_engine::Cost| LogArgument::Amount {
        coins: cost.coins.into(),
        potions: cost.potions.into(),
        debt: cost.debt.into(),
    };
    Ok(match story {
        "TANUKI_GAIN" | "KINTSUGI_GAIN" => {
            let trashed = view
                .pending_reference()
                .and_then(|id| view.cost_of(id))
                .ok_or_else(|| gap(d, "missing visible trashed card"))?;
            vec![amount(trashed.plus_coins(2))]
        }
        "GROWTH" => {
            // Cheaper than the Treasure whose gain this answers.
            let treasure = view
                .log()
                .iter()
                .rev()
                .find_map(|e| match e {
                    Event::Gained { player, card, .. }
                        if *player == d.player && card.is(Types::TREASURE) =>
                    {
                        Some(*card)
                    }
                    _ => None,
                })
                .ok_or_else(|| gap(d, "missing gained Treasure"))?;
            vec![amount(view.cost(treasure))]
        }
        "GATHER" => {
            let DecisionKind::Pile { piles, .. } = &d.kind else {
                return Err(gap(d, "Gather shape"));
            };
            let pile = piles
                .first()
                .ok_or_else(|| gap(d, "Gather without piles"))?;
            vec![amount(view.cost(view.pile(*pile).card()))]
        }
        "SEA_TRADE" => match d.kind {
            DecisionKind::Cards { max, .. } => vec![LogArgument::Number(
                i32::try_from(max).map_err(|_| ProjectionError::Size("Sea Trade count"))?,
            )],
            _ => return Err(gap(d, "Sea Trade shape")),
        },
        _ => vec![],
    })
}

/// Receive Tribute gains up to three Actions one at a time; the story counts those left.
fn receive_tribute_story(view: &View<'_>, d: &Decision) -> &'static str {
    let log = view.log();
    let since = log
        .iter()
        .rposition(|e| {
            matches!(e, Event::BoughtLandscape { player, card: Card::ReceiveTribute }
                if *player == d.player)
        })
        .map_or(log.len(), |i| i + 1);
    let gained = log[since..]
        .iter()
        .filter(|e| {
            matches!(e, Event::Gained { player, card, .. }
            if *player == d.player && card.is(Types::ACTION))
        })
        .count();
    match gained {
        0 => "RECEIVE_TRIBUTE_FIRST",
        1 => "RECEIVE_TRIBUTE_SECOND",
        _ => "RECEIVE_TRIBUTE_THIRD",
    }
}

pub(super) fn choice_shape(
    view: &View<'_>,
    d: &Decision,
) -> Option<Result<ChoiceShape, ProjectionError>> {
    if !matches!(d.kind, DecisionKind::Choose { .. }) {
        return None;
    }
    match (d.source?, d.purpose) {
        (Card::GoldMine, Purpose::Gain) => Some(Ok(
            match view
                .piles()
                .find(|p| p.card() == Card::Gold)
                .and_then(|p| p.top())
            {
                Some(card) => ChoiceShape::Target {
                    card,
                    yes: Opt::Yes,
                    no: Opt::No,
                    decline: "NO_THANKS",
                },
                // The Debt still comes without a Gold.
                None => ChoiceShape::Buttons(vec![(Opt::Yes, "CONTINUE"), (Opt::No, "NO_THANKS")]),
            },
        )),
        (Card::RusticVillage, Purpose::Discard) => Some(Ok(ChoiceShape::Buttons(vec![
            (Opt::Yes, "CONTINUE"),
            (Opt::No, "DONT_DISCARD"),
        ]))),
        (Card::SnakeWitch, Purpose::Reveal) => Some(
            view.pending_source()
                .map(|card| ChoiceShape::Target {
                    card,
                    yes: Opt::Yes,
                    no: Opt::No,
                    decline: "SNAKE_WITCH_DONT_RETURN",
                })
                .ok_or_else(|| gap(d, "missing visible Snake Witch")),
        ),
        _ => None,
    }
}

pub(super) fn ability_arguments(
    view: &View<'_>,
    t: &Trigger,
    c: &WireCatalog,
) -> Option<Result<Vec<LogArgument>, ProjectionError>> {
    let missing =
        || ProjectionError::UnsupportedState(format!("{} ability argument", t.card.name()));
    let set_aside = || {
        let l = view
            .lasting()
            .into_iter()
            .find(|l| Some(l.id) == t.lasting)
            .ok_or_else(missing)?;
        let card = (l.data.cards.iter().copied())
            .chain(l.data.pick)
            .find_map(|id| view.card(id))
            .ok_or_else(missing)?;
        Ok(vec![names(c, card)?])
    };
    match (t.card, t.ability) {
        (Card::RapidExpansion, 0) => Some(set_aside()),
        (Card::RapidExpansion, 1) | (Card::Progress, 0) => Some(
            super::allies::last_gained(view, t.owner)
                .ok_or_else(missing)
                .and_then(|card| Ok(vec![names(c, card)?])),
        ),
        (Card::Daimyo, 1) => Some(
            view.log()
                .iter()
                .rev()
                .find_map(|e| match e {
                    Event::Played { player, card } if *player == t.owner => Some(*card),
                    _ => None,
                })
                .ok_or_else(missing)
                .and_then(|card| Ok(vec![names(c, card)?])),
        ),
        _ => None,
    }
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    let choices: &[&str] = match card {
        Card::Kitsune => &[
            "KITSUNE_COINS",
            "KITSUNE_ACTIONS",
            "KITSUNE_CURSE",
            "KITSUNE_SILVER",
        ],
        Card::Sickness => &["SICKNESS_CURSE", "SICKNESS_DISCARD"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    Some(match (card, ability) {
        (Card::Samurai, 1) => "SAMURAI",
        (Card::Daimyo, 1) => "DAIMYO_REPLAY",
        (Card::RiverShrine, 0) => "RIVER_SHRINE",
        (Card::Foresight, 0) => "FORESIGHT_RETURN_TO_HAND",
        (Card::KindEmperor, 0) => "KIND_EMPEROR",
        (Card::Sickness, 0) => "SICKNESS",
        (Card::BidingTime, 0) => "BIDING_TIME_RETURN_TO_HAND",
        (Card::BidingTime, 1) => "BIDING_TIME_SET_ASIDE_HAND",
        (Card::Panic, 0) => "PANIC_BUYS",
        (Card::Panic, 1) => "PANIC_RETURN",
        (Card::Enlightenment, 0) => "ENLIGHTENMENT_REPLACE_ABILITY",
        (Card::GoodHarvest, 0) => "GOOD_HARVEST",
        (Card::HarshWinter, 0) => "HARSH_WINTER",
        (Card::Progress, 0) => "PROGRESS",
        (Card::RapidExpansion, 0) => "RAPID_EXPANSION_PLAY_LATER",
        (Card::RapidExpansion, 1) => "RAPID_EXPANSION_SET_ASIDE",
        _ => return None,
    })
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Alley => &[("QuestionIds", "ALLEY"), ("QuestionTypes", "DISCARD")],
        Change => &[
            ("QuestionIds", "CHANGE_TRASH"),
            ("QuestionIds", "CHANGE_GAIN"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "GAIN"),
        ],
        Craftsman => &[("QuestionIds", "CRAFTSMAN"), ("QuestionTypes", "GAIN")],
        GoldMine => &[
            ("QuestionIds", "GOLD_MINE"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "BUTTON"),
            ("DeclineButtonIds", "NO_THANKS"),
            ("DeclineButtonIds", "CONTINUE"),
        ],
        Ninja => &[("QuestionIds", "NINJA"), ("QuestionTypes", "DISCARD")],
        Samurai => &[
            ("QuestionIds", "SAMURAI"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "SAMURAI"),
        ],
        SnakeWitch => &[
            ("QuestionIds", "SNAKE_WITCH"),
            ("QuestionTypes", "REVEAL"),
            ("DeclineButtonIds", "SNAKE_WITCH_DONT_RETURN"),
        ],
        Tanuki => &[
            ("QuestionIds", "TANUKI_TRASH"),
            ("QuestionIds", "TANUKI_GAIN"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "GAIN"),
        ],
        Daimyo => &[("AbilityNames", "DAIMYO_REPLAY")],
        Amass => &[("QuestionIds", "AMASS"), ("QuestionTypes", "GAIN")],
        Continue => &[("QuestionIds", "CONTINUE"), ("QuestionTypes", "GAIN")],
        Credit => &[("QuestionIds", "CREDIT"), ("QuestionTypes", "GAIN")],
        Gather => &[("QuestionIds", "GATHER"), ("QuestionTypes", "GAIN")],
        Kintsugi => &[
            ("QuestionIds", "KINTSUGI_TRASH"),
            ("QuestionIds", "KINTSUGI_GAIN"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "GAIN"),
        ],
        Practice => &[("QuestionIds", "PRACTICE"), ("QuestionTypes", "PLAY")],
        ReceiveTribute => &[
            ("QuestionIds", "RECEIVE_TRIBUTE_FIRST"),
            ("QuestionIds", "RECEIVE_TRIBUTE_SECOND"),
            ("QuestionIds", "RECEIVE_TRIBUTE_THIRD"),
            ("QuestionTypes", "GAIN"),
        ],
        SeaTrade => &[("QuestionIds", "SEA_TRADE"), ("QuestionTypes", "TRASH")],
        Kitsune => &[
            ("QuestionIds", "KITSUNE"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "KITSUNE_COINS"),
            ("CardModes", "KITSUNE_ACTIONS"),
            ("CardModes", "KITSUNE_CURSE"),
            ("CardModes", "KITSUNE_SILVER"),
        ],
        MountainShrine => &[
            ("QuestionIds", "MOUNTAIN_SHRINE"),
            ("QuestionTypes", "TRASH"),
        ],
        RiverShrine => &[
            ("QuestionIds", "RIVER_SHRINE_TRASH"),
            ("QuestionIds", "RIVER_SHRINE_GAIN"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "RIVER_SHRINE"),
        ],
        RusticVillage => &[
            ("QuestionIds", "RUSTIC_VILLAGE_CHOICE"),
            ("QuestionIds", "RUSTIC_VILLAGE_DISCARD"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "BUTTON"),
            ("DeclineButtonIds", "CONTINUE"),
            ("DeclineButtonIds", "DONT_DISCARD"),
        ],
        Sickness => &[
            ("QuestionIds", "SICKNESS"),
            ("QuestionIds", "SICKNESS_DISCARD"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "DISCARD"),
            ("CardModes", "SICKNESS_CURSE"),
            ("CardModes", "SICKNESS_DISCARD"),
            ("AbilityNames", "SICKNESS"),
        ],
        KindEmperor => &[
            ("QuestionIds", "KIND_EMPEROR"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "KIND_EMPEROR"),
        ],
        Foresight => &[("AbilityNames", "FORESIGHT_RETURN_TO_HAND")],
        BidingTime => &[
            ("AbilityNames", "BIDING_TIME_RETURN_TO_HAND"),
            ("AbilityNames", "BIDING_TIME_SET_ASIDE_HAND"),
        ],
        Enlightenment => &[("AbilityNames", "ENLIGHTENMENT_REPLACE_ABILITY")],
        GoodHarvest => &[("AbilityNames", "GOOD_HARVEST")],
        HarshWinter => &[("AbilityNames", "HARSH_WINTER")],
        Progress => &[("AbilityNames", "PROGRESS")],
        RapidExpansion => &[
            ("AbilityNames", "RAPID_EXPANSION_PLAY_LATER"),
            ("AbilityNames", "RAPID_EXPANSION_SET_ASIDE"),
        ],
        _ => &[],
    }
}

/// Cards the survey runs clean and `projection::late_sets` covers. Omens and Prophecies
/// wait for the Prophecy in the state projection; Riverboat for its set-aside card.
const ADMITTED: &[(Card, &[&str])] = &[
    (Card::Alley, &["shadow", "card-selection"]),
    (Card::Fishmonger, &["shadow"]),
    (
        Card::Ninja,
        &["shadow", "off-turn-attack", "card-selection"],
    ),
    (Card::Ronin, &["shadow"]),
    (
        Card::Tanuki,
        &[
            "shadow",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
    ),
    (Card::Change, &["debt", "card-selection", "pile-selection"]),
    (Card::Craftsman, &["debt", "pile-selection"]),
    (Card::GoldMine, &["debt", "optional-effect"]),
    (Card::ImperialEnvoy, &["debt"]),
    (Card::Litter, &["debt"]),
    (Card::RootCellar, &["debt"]),
    (
        Card::Samurai,
        &[
            "off-turn-attack",
            "card-selection",
            "duration-ability",
            "duration-reminder",
        ],
    ),
    (Card::SnakeWitch, &["optional-effect"]),
    (Card::Aristocrat, &[]),
    (Card::Artist, &[]),
    (Card::Daimyo, &["repeated-play", "duration-ability"]),
    (Card::Rice, &[]),
    (Card::Amass, &["event", "pile-selection"]),
    (Card::Continue, &["event", "pile-selection"]),
    (Card::Credit, &["event", "debt", "pile-selection"]),
    (Card::Foresight, &["event", "set-aside"]),
    (Card::Gather, &["event", "pile-selection"]),
    (
        Card::Kintsugi,
        &[
            "event",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
    ),
    (Card::Practice, &["event", "repeated-play", "optional-play"]),
    (Card::ReceiveTribute, &["event", "pile-selection"]),
    (Card::SeaTrade, &["event", "card-selection"]),
    (Card::BidingTime, &["prophecy", "set-aside"]),
    (Card::Enlightenment, &["prophecy"]),
    (Card::FlourishingTrade, &["prophecy", "dynamic-cost"]),
    (Card::GoodHarvest, &["prophecy"]),
    (Card::GreatLeader, &["prophecy"]),
    (Card::HarshWinter, &["prophecy", "debt"]),
    (Card::KindEmperor, &["prophecy", "pile-selection"]),
    (Card::Progress, &["prophecy"]),
    (Card::RapidExpansion, &["prophecy", "set-aside"]),
    (
        Card::Sickness,
        &["prophecy", "semantic-modes", "card-selection"],
    ),
];

/// Omens, once every Prophecy is carried: setup picks their Prophecy at random.
const OMENS: &[(Card, &[&str])] = &[
    (
        Card::Kitsune,
        &["prophecy", "semantic-modes", "off-turn-attack"],
    ),
    (Card::MountainShrine, &["prophecy", "card-selection"]),
    (Card::Poet, &["prophecy"]),
    (
        Card::RiverShrine,
        &["prophecy", "card-selection", "pile-selection"],
    ),
    (Card::RusticVillage, &["prophecy", "card-selection"]),
    (Card::TeaHouse, &["prophecy"]),
];

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    let find = |list: &[(Card, &'static [&'static str])]| {
        list.iter().find(|(c, _)| *c == card).map(|(_, f)| *f)
    };
    if let Some(features) = find(OMENS) {
        let mut prophecies = Card::ALL
            .iter()
            .filter(|c| c.kind() == dominion_engine::Kind::Prophecy);
        return prophecies
            .all(|&p| ADMITTED.iter().any(|(c, _)| *c == p))
            .then_some(features);
    }
    find(ADMITTED)
}
