//! Question bindings for this set's cards (see `semantics::SETS`).
use super::allies::{last_gained, names};
use super::{Binding, ChoiceShape, ProjectionError, WireCatalog, gap};
use dominion_engine::{Card, Decision, DecisionKind, Opt, Purpose, Trigger, View};
use dominion_online::questions::{CardFrequency, LogArgument};

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let choosing = matches!(d.kind, DecisionKind::Choose { .. });
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let (kind, story) = match (d.source?, d.purpose) {
        (Rope, P::Trash) => ("TRASH", "ROPE_TRASH"),
        (Patient, P::Play) => ("PLAY", "PATIENT_PLAY"),
        (Patient, P::SetAside) => ("SET_ASIDE", "PATIENT_SET_ASIDE"),
        (Prepare, P::Play) => ("PLAY", "PREPARE"),
        (CabinBoy, P::Mode) => ("CHOOSE_MODE", "CABIN_BOY_MODE"),
        (CabinBoy, P::Gain) => ("GAIN", "CABIN_BOY_GAIN"),
        (Quartermaster, P::Mode) => ("CHOOSE_MODE", "QUARTERMASTER_MODE"),
        (Quartermaster, P::Gain) => ("GAIN", "QUARTERMASTER_GAIN"),
        (Quartermaster, P::Move) => ("PUT_IN_HAND", "QUARTERMASTER_TAKE"),
        (SecludedShrine, P::Trash) => ("TRASH", "SECLUDED_SHRINE"),
        (Siren, P::Trash) => ("TRASH", "SIREN"),
        (Trickster, P::SetAside) => ("SET_ASIDE", "TRICKSTER_DISCARD"),
        (Avoid, P::Move) => ("DISCARD", "AVOID"),
        (Fated, P::TopDeck) if ordered => ("ORDER_CARDS", "SENTRY_TOPDECK"),
        (Fated, P::TopDeck) => ("TOPDECK", "FATED_TOPDECK"),
        (Fated, P::Move) if ordered => ("ORDER_CARDS", "ORDER_BOTTOMDECKED_CARDS"),
        (Fated, P::Move) => ("BOTTOMDECK", "FATED_BOTTOMDECK"),
        (Shaman, P::Gain) => ("GAIN", "SHAMAN_GAIN"),
        (Shaman, P::Trash) => ("TRASH", "SHAMAN_TRASH"),
        (Friendly, P::Discard) => ("DISCARD", "FRIENDLY"),
        (Shy, P::Discard) => ("DISCARD", "SHY"),
        (Inspiring, P::Play) => ("PLAY", "INSPIRING"),
        (Inherited, P::Other) if choosing => ("CHOOSE_MODE", "INHERITED"),
        (PrizeGoat, P::Trash) => ("TRASH", "PRIZE_GOAT"),
        (Pious, P::Trash) => ("TRASH", "PIOUS_TRASH"),
        (Hammer, P::Gain) => ("GAIN", "HAMMER"),
        (Enlarge, P::Trash) => ("TRASH", "ENLARGE_TRASH"),
        (Enlarge, P::Gain) => ("GAIN", "ENLARGE_GAIN"),
        (Cutthroat, P::Discard) => ("DISCARD", "CUTTHROAT_DISCARD"),
        (Sword, P::Discard) => ("DISCARD", "SWORD"),
        (Frigate, P::Discard) => ("DISCARD", "FRIGATE"),
        (Staff, P::Play) => ("PLAY", "STAFF"),
        (Gondola, P::Play) => ("PLAY", "GONDOLA_PLAY"),
        (Gondola, P::Either) => ("CHOOSE_MODE", "GONDOLA_MODE"),
        (Amphora, P::Either) => ("CHOOSE_MODE", "AMPHORA_MODE"),
        (Cage, P::SetAside) => ("SET_ASIDE", "CAGE_SET_ASIDE"),
        (Grotto, P::SetAside) => ("SET_ASIDE", "GROTTO_SET_ASIDE"),
        (Crucible, P::Trash) => ("TRASH", "CRUCIBLE"),
        (FortuneHunter, P::Play) => ("PLAY", "FORTUNE_HUNTER_PLAY"),
        (FortuneHunter, P::TopDeck) if ordered => ("ORDER_CARDS", "FORTUNE_HUNTER_TOPDECK"),
        (Mapmaker, P::Move) => ("PUT_IN_HAND", "MAPMAKER"),
        (Maroon, P::Trash) => ("TRASH", "MAROON"),
        (Tools, P::Gain) if choosing => ("GAIN", "TOOLS"),
        (Figurine, P::Discard) => ("DISCARD", "FIGURINE"),
        (FirstMate, P::Play) => ("PLAY", "FIRST_MATE"),
        (Pickaxe, P::Trash) => ("TRASH", "PICKAXE"),
        (Pilgrim, P::TopDeck) => ("TOPDECK", "PILGRIM_TOPDECK"),
        (SilverMine, P::Gain) => ("GAIN", "SILVER_MINE"),
        (KingsCache, P::Play) => ("PLAY", "KINGS_CACHE"),
        (Orb, P::Mode) => ("CHOOSE_MODE", "ORB"),
        (Orb, P::Play) => ("PLAY", "ORB"),
        (PuzzleBox, P::SetAside) => ("SET_ASIDE", "PUZZLE_BOX"),
        (Sextant, P::Discard) => ("DISCARD", "SEXTANT_DISCARD"),
        (Sextant, P::TopDeck) if ordered => ("ORDER_CARDS", "SEXTANT_TOPDECK"),
        (SpellScroll, P::Gain) => ("GAIN", "SPELL_SCROLL_GAIN"),
        (SpellScroll, P::Play) if choosing => ("CHOOSE_MODE", "SPELL_SCROLL_PLAY"),
        (Bury, P::Move) => ("BOTTOMDECK", "BURY"),
        (Peril, P::Trash) => ("TRASH", "PERIL"),
        (Foray, P::Discard) => ("DISCARD", "FORAY"),
        (Maelstrom, P::Trash) if d.player == view.turn().player => ("TRASH", "MAELSTROM_TRASH"),
        (Maelstrom, P::Trash) => ("TRASH", "MAELSTROM_OPPONENT_TRASH"),
        (Invasion, P::Play) => ("PLAY", "INVASION_PLAY"),
        (Invasion, P::Gain) => ("GAIN", "INVASION_GAIN"),
        (Prosper, P::Gain) => ("GAIN", "PROSPER"),
        _ => return None,
    };
    Some(arguments(view, d, c, story).map(|args| (kind, story, args)))
}

fn arguments(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
    story: &str,
) -> Result<Vec<LogArgument>, ProjectionError> {
    let amount = |cost: dominion_engine::Cost| LogArgument::Amount {
        coins: cost.coins.into(),
        potions: cost.potions.into(),
        debt: cost.debt.into(),
    };
    Ok(match story {
        "ENLARGE_GAIN" => {
            let trashed = view
                .pending_reference()
                .and_then(|id| view.cost_of(id))
                .ok_or_else(|| gap(d, "missing visible trashed card"))?;
            vec![amount(trashed.plus_coins(2))]
        }
        "SPELL_SCROLL_GAIN" => {
            let this = view
                .pending_source()
                .and_then(|id| view.cost_of(id))
                .ok_or_else(|| gap(d, "missing visible Spell Scroll"))?;
            vec![amount(this)]
        }
        "SPELL_SCROLL_PLAY" => {
            let card = view
                .pending_reference()
                .and_then(|id| view.card(id))
                .ok_or_else(|| gap(d, "missing visible gained card"))?;
            vec![names(c, card)?]
        }
        "MAELSTROM_OPPONENT_TRASH" => vec![LogArgument::Number(1)],
        "QUARTERMASTER_MODE" => {
            let this = view.pending_source();
            let held: Vec<Card> = view
                .lasting()
                .into_iter()
                .filter(|l| l.card == Card::Quartermaster && this.is_some() && l.source == this)
                .flat_map(|l| l.data.cards)
                .filter_map(|id| view.card(id))
                .collect();
            vec![frequencies(c, held)?]
        }
        _ => vec![],
    })
}

fn frequencies(c: &WireCatalog, cards: Vec<Card>) -> Result<LogArgument, ProjectionError> {
    let mut counts = std::collections::BTreeMap::new();
    for card in cards {
        *counts.entry(c.card(card)?).or_insert(0) += 1;
    }
    Ok(LogArgument::CardNames(
        counts
            .into_iter()
            .map(|(name, frequency)| CardFrequency { name, frequency })
            .collect(),
    ))
}

pub(super) fn choice_shape(
    view: &View<'_>,
    d: &Decision,
) -> Option<Result<ChoiceShape, ProjectionError>> {
    let DecisionKind::Choose { options, .. } = &d.kind else {
        return None;
    };
    match (d.source?, d.purpose) {
        (Card::Tools, Purpose::Gain) => Some(
            options
                .iter()
                .map(|&o| {
                    let Opt::Card(name) = o else {
                        return Err(gap(d, "Tools option"));
                    };
                    view.players()
                        .flat_map(|p| p.in_play().iter().copied())
                        .find(|&id| view.card(id) == Some(name))
                        .map(|id| (o, id))
                        .ok_or_else(|| gap(d, "Tools card not in play"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ChoiceShape::Cards),
        ),
        (Card::Inherited, Purpose::Other) => Some(
            options
                .iter()
                .map(|&o| match o {
                    Opt::Card(name) => Ok((o, name)),
                    _ => Err(gap(d, "Inherited option")),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ChoiceShape::Names),
        ),
        (Card::Orb, Purpose::Mode) => Some(Ok(ChoiceShape::Modes {
            options: vec![(Opt::Mode(0), "PLAY")],
            decline: Some((Opt::Mode(1), "ORB_DONT")),
        })),
        (Card::SpellScroll, Purpose::Play) => Some(Ok(ChoiceShape::Modes {
            options: vec![
                (Opt::Yes, "SPELL_SCROLL_PLAY"),
                (Opt::No, "SPELL_SCROLL_REFUSE"),
            ],
            decline: None,
        })),
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
    let held = || {
        let l = view
            .lasting()
            .into_iter()
            .find(|l| Some(l.id) == t.lasting)
            .ok_or_else(missing)?;
        let cards: Vec<Card> = (l.data.cards.iter().copied())
            .chain(l.data.pick)
            .filter_map(|id| view.card(id))
            .collect();
        if cards.is_empty() {
            return Err(missing());
        }
        Ok(vec![frequencies(c, cards)?])
    };
    let source = || {
        let card = t.source.and_then(|id| view.card(id)).ok_or_else(missing)?;
        Ok(vec![names(c, card)?])
    };
    let gained = || {
        let card = last_gained(view, t.owner).ok_or_else(missing)?;
        Ok(vec![names(c, card)?])
    };
    Some(match ability(t.card, t.ability)? {
        "CAGE_PUT_IN_HAND"
        | "GROTTO"
        | "TRICKSTER_RETURN_TO_HAND"
        | "DELIVER_RETURN"
        | "PUZZLE_BOX_RETURN"
        | "HASTY_PLAY"
        | "TIRELESS_RETURN" => held(),
        "HASTY_SET_ASIDE" | "TIRELESS_SET_ASIDE" | "RECKLESS_RETURN" => source(),
        "MINING_ROAD" | "DELIVER_SET_ASIDE" | "RUSH" | "MIRROR" | "TOPDECK_REACTION" => gained(),
        "FLAGSHIP_REPLAY" => {
            // The play it waits for: the most recent card its owner played.
            view.log()
                .iter()
                .rev()
                .find_map(|e| match e {
                    dominion_engine::Event::Played { player, card, .. } if *player == t.owner => {
                        Some(*card)
                    }
                    _ => None,
                })
                .ok_or_else(missing)
                .and_then(|card| Ok(vec![names(c, card)?]))
        }
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        CabinBoy => &["CABIN_BOY_COINS", "CABIN_BOY_GAIN"],
        Quartermaster => &["QUARTERMASTER_GAIN", "QUARTERMASTER_TAKE"],
        Gondola => &["GONDOLA_NOW", "GONDOLA_LATER"],
        Amphora => &["AMPHORA_NOW", "AMPHORA_LATER"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    // The engine's ability numbers for this set: START 0, WAITED 1, HELD 2, GAINED 3,
    // REACT 4, TRASHED 5, THIS_TURN 6, SHAMAN_GAIN 8, TRAIT 10, INHERIT 11.
    Some(match (card, ability) {
        (Gondola, 0) => "GONDOLA_DURATION",
        (Amphora, 0) => "AMPHORA",
        (BuriedTreasure, 0) => "BURIED_TREASURE_DURATION",
        (EndlessChalice, 0) => "ENDLESS_CHALICE",
        (Rope, 0) => "ROPE",
        (Crew, 0) => "CREW_DURATION",
        (Jewels, 0) => "JEWELS",
        (Enlarge, 0) => "ENLARGE",
        (Quartermaster, 0) => "QUARTERMASTER",
        (Hasty, 0) => "HASTY_PLAY",
        (Patient, 0) => "PATIENT_PLAY",
        (Prepare, 0) => "PREPARE",
        (CabinBoy, 0) => "CABIN_BOY",
        (Longship, 0) => "LONGSHIP_DURATION",
        (Figurehead, 0) => "FIGUREHEAD",
        (Stowaway, 0) => "STOWAWAY_DRAW",
        (Taskmaster, 0) => "TASKMASTER",
        (Grotto, 0) => "GROTTO",
        (Siren, 0) => "SIREN_DURATION",
        (Search, 1) => "SEARCH",
        (SecludedShrine, 1) => "SECLUDED_SHRINE",
        (Cage, 1) => "CAGE_TRASH",
        (Abundance, 1) => "ABUNDANCE",
        (Cutthroat, 1) => "CUTTHROAT_GAIN",
        (Flagship, 1) => "FLAGSHIP_REPLAY",
        (LandingParty, 1) => "LANDING_PARTY",
        (Cage, 2) => "CAGE_PUT_IN_HAND",
        (Trickster, 2) => "TRICKSTER_RETURN_TO_HAND",
        (Deliver, 2) => "DELIVER_RETURN",
        (PuzzleBox, 2) => "PUZZLE_BOX_RETURN",
        (Tireless, 2) => "TIRELESS_RETURN",
        (BuriedTreasure, 3) => "BURIED_TREASURE_PLAY",
        (Siren, 3) => "SIREN_GAIN",
        (Doubloons, 3) => "DOUBLOONS",
        (Gondola, 3) => "GONDOLA_PLAY",
        (WealthyVillage, 3) => "WEALTHY_VILLAGE",
        (Cursed, 3) => "CURSED_GAIN",
        (Hasty, 3) => "HASTY_SET_ASIDE",
        (Nearby, 3) => "NEARBY",
        (Rich, 3) => "RICH_GAIN",
        (Pious, 3) => "PIOUS",
        (Fawning, 3) => "FAWNING_GAIN",
        (Shield, 4) => "SHIELD_REACTION",
        (Mapmaker, 4) => "MAPMAKER",
        (Stowaway, 4) => "STOWAWAY_REACTION",
        (JewelledEgg, 5) => "JEWELLED_EGG_GAIN_LOOT",
        (Avoid, 6) => "AVOID",
        (Fated, 6) => "FATED",
        (Insignia, 6) => "TOPDECK_REACTION",
        (MiningRoad, 6) => "MINING_ROAD",
        (Deliver, 6) => "DELIVER_SET_ASIDE",
        (Rush, 6) => "RUSH",
        (Mirror, 6) => "MIRROR",
        (HarborVillage, 6) => "HARBOR_VILLAGE",
        (Frigate, 6) => "FRIGATE_DISCARD",
        (Trickster, 6) => "TRICKSTER_REACTION",
        (Shaman, 8) => "SHAMAN",
        (Shy, 10) => "SHY",
        (Friendly, 10) => "FRIENDLY_ABILITY",
        (Patient, 10) => "PATIENT_SET_ASIDE",
        (Inspiring, 10) => "INSPIRING",
        (Tireless, 10) => "TIRELESS_SET_ASIDE",
        (Reckless, 10) => "RECKLESS_RETURN",
        (Inherited, 11) => "INHERITED",
        _ => return None,
    })
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Rope => &[
            ("QuestionIds", "ROPE_TRASH"),
            ("QuestionTypes", "TRASH"),
            ("AbilityNames", "ROPE"),
        ],
        Patient => &[
            ("QuestionIds", "PATIENT_PLAY"),
            ("QuestionIds", "PATIENT_SET_ASIDE"),
            ("QuestionTypes", "PLAY"),
            ("QuestionTypes", "SET_ASIDE"),
            ("AbilityNames", "PATIENT_PLAY"),
            ("AbilityNames", "PATIENT_SET_ASIDE"),
        ],
        Prepare => &[
            ("QuestionIds", "PREPARE"),
            ("QuestionTypes", "PLAY"),
            ("AbilityNames", "PREPARE"),
        ],
        CabinBoy => &[
            ("QuestionIds", "CABIN_BOY_MODE"),
            ("QuestionIds", "CABIN_BOY_GAIN"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
            ("CardModes", "CABIN_BOY_COINS"),
            ("CardModes", "CABIN_BOY_GAIN"),
            ("AbilityNames", "CABIN_BOY"),
        ],
        Quartermaster => &[
            ("QuestionIds", "QUARTERMASTER_MODE"),
            ("QuestionIds", "QUARTERMASTER_GAIN"),
            ("QuestionIds", "QUARTERMASTER_TAKE"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "PUT_IN_HAND"),
            ("CardModes", "QUARTERMASTER_GAIN"),
            ("CardModes", "QUARTERMASTER_TAKE"),
            ("AbilityNames", "QUARTERMASTER"),
        ],
        SecludedShrine => &[
            ("QuestionIds", "SECLUDED_SHRINE"),
            ("QuestionTypes", "TRASH"),
            ("AbilityNames", "SECLUDED_SHRINE"),
        ],
        Siren => &[
            ("QuestionIds", "SIREN"),
            ("QuestionTypes", "TRASH"),
            ("AbilityNames", "SIREN_GAIN"),
            ("AbilityNames", "SIREN_DURATION"),
        ],
        Trickster => &[
            ("QuestionIds", "TRICKSTER_DISCARD"),
            ("QuestionTypes", "SET_ASIDE"),
            ("AbilityNames", "TRICKSTER_REACTION"),
            ("AbilityNames", "TRICKSTER_RETURN_TO_HAND"),
        ],
        Avoid => &[
            ("QuestionIds", "AVOID"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "AVOID"),
        ],
        Fated => &[
            ("QuestionIds", "FATED_TOPDECK"),
            ("QuestionIds", "FATED_BOTTOMDECK"),
            ("QuestionIds", "SENTRY_TOPDECK"),
            ("QuestionIds", "ORDER_BOTTOMDECKED_CARDS"),
            ("QuestionTypes", "TOPDECK"),
            ("QuestionTypes", "BOTTOMDECK"),
            ("QuestionTypes", "ORDER_CARDS"),
            ("AbilityNames", "FATED"),
        ],
        Shaman => &[
            ("QuestionIds", "SHAMAN_GAIN"),
            ("QuestionIds", "SHAMAN_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
            ("AbilityNames", "SHAMAN"),
        ],
        Friendly => &[
            ("QuestionIds", "FRIENDLY"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "FRIENDLY_ABILITY"),
        ],
        Shy => &[
            ("QuestionIds", "SHY"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "SHY"),
        ],
        Inspiring => &[
            ("QuestionIds", "INSPIRING"),
            ("QuestionTypes", "PLAY"),
            ("AbilityNames", "INSPIRING"),
        ],
        Inherited => &[
            ("QuestionIds", "INHERITED"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("AbilityNames", "INHERITED"),
        ],
        Pious => &[
            ("QuestionIds", "PIOUS_TRASH"),
            ("QuestionTypes", "TRASH"),
            ("AbilityNames", "PIOUS"),
        ],
        Enlarge => &[
            ("QuestionIds", "ENLARGE_TRASH"),
            ("QuestionIds", "ENLARGE_GAIN"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "ENLARGE"),
        ],
        Frigate => &[
            ("QuestionIds", "FRIGATE"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "FRIGATE_DISCARD"),
        ],
        Gondola => &[
            ("QuestionIds", "GONDOLA_PLAY"),
            ("QuestionIds", "GONDOLA_MODE"),
            ("QuestionTypes", "PLAY"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "GONDOLA_NOW"),
            ("CardModes", "GONDOLA_LATER"),
            ("AbilityNames", "GONDOLA_DURATION"),
            ("AbilityNames", "GONDOLA_PLAY"),
        ],
        Cage => &[
            ("QuestionIds", "CAGE_SET_ASIDE"),
            ("QuestionTypes", "SET_ASIDE"),
            ("AbilityNames", "CAGE_TRASH"),
            ("AbilityNames", "CAGE_PUT_IN_HAND"),
        ],
        Grotto => &[
            ("QuestionIds", "GROTTO_SET_ASIDE"),
            ("QuestionTypes", "SET_ASIDE"),
            ("AbilityNames", "GROTTO"),
        ],
        Crucible => &[("QuestionIds", "CRUCIBLE"), ("QuestionTypes", "TRASH")],
        FortuneHunter => &[
            ("QuestionIds", "FORTUNE_HUNTER_PLAY"),
            ("QuestionIds", "FORTUNE_HUNTER_TOPDECK"),
            ("QuestionTypes", "PLAY"),
            ("QuestionTypes", "ORDER_CARDS"),
        ],
        Mapmaker => &[
            ("QuestionIds", "MAPMAKER"),
            ("QuestionTypes", "PUT_IN_HAND"),
            ("AbilityNames", "MAPMAKER"),
        ],
        Maroon => &[("QuestionIds", "MAROON"), ("QuestionTypes", "TRASH")],
        Tools => &[("QuestionIds", "TOOLS"), ("QuestionTypes", "GAIN")],
        Figurine => &[("QuestionIds", "FIGURINE"), ("QuestionTypes", "DISCARD")],
        FirstMate => &[("QuestionIds", "FIRST_MATE"), ("QuestionTypes", "PLAY")],
        Pilgrim => &[
            ("QuestionIds", "PILGRIM_TOPDECK"),
            ("QuestionTypes", "TOPDECK"),
        ],
        SilverMine => &[("QuestionIds", "SILVER_MINE"), ("QuestionTypes", "GAIN")],
        KingsCache => &[("QuestionIds", "KINGS_CACHE"), ("QuestionTypes", "PLAY")],
        Bury => &[("QuestionIds", "BURY"), ("QuestionTypes", "BOTTOMDECK")],
        Maelstrom => &[
            ("QuestionIds", "MAELSTROM_TRASH"),
            ("QuestionIds", "MAELSTROM_OPPONENT_TRASH"),
            ("QuestionTypes", "TRASH"),
        ],
        Abundance => &[("AbilityNames", "ABUNDANCE")],
        BuriedTreasure => &[
            ("AbilityNames", "BURIED_TREASURE_DURATION"),
            ("AbilityNames", "BURIED_TREASURE_PLAY"),
        ],
        Crew => &[("AbilityNames", "CREW_DURATION")],
        Flagship => &[("AbilityNames", "FLAGSHIP_REPLAY")],
        HarborVillage => &[("AbilityNames", "HARBOR_VILLAGE")],
        LandingParty => &[("AbilityNames", "LANDING_PARTY")],
        Longship => &[("AbilityNames", "LONGSHIP_DURATION")],
        MiningRoad => &[("AbilityNames", "MINING_ROAD")],
        Stowaway => &[
            ("AbilityNames", "STOWAWAY_DRAW"),
            ("AbilityNames", "STOWAWAY_REACTION"),
        ],
        Taskmaster => &[("AbilityNames", "TASKMASTER")],
        Deliver => &[
            ("AbilityNames", "DELIVER_SET_ASIDE"),
            ("AbilityNames", "DELIVER_RETURN"),
        ],
        Rush => &[("AbilityNames", "RUSH")],
        Mirror => &[("AbilityNames", "MIRROR")],
        Journey => &[("ExtraTurnTypes", "JOURNEY")],
        Hasty => &[
            ("AbilityNames", "HASTY_SET_ASIDE"),
            ("AbilityNames", "HASTY_PLAY"),
        ],
        Nearby => &[("AbilityNames", "NEARBY")],
        Rich => &[("AbilityNames", "RICH_GAIN")],
        Fawning => &[("AbilityNames", "FAWNING_GAIN")],
        Tireless => &[
            ("AbilityNames", "TIRELESS_SET_ASIDE"),
            ("AbilityNames", "TIRELESS_RETURN"),
        ],
        Reckless => &[("AbilityNames", "RECKLESS_RETURN")],
        _ => &[],
    }
}

const DURATION: &[&str] = &["duration-ability", "duration-reminder"];
const TRAIT: &[&str] = &["trait", "optional-ability"];

/// Cards the survey runs clean and `projection::late_sets` covers.
const ADMITTED: &[(Card, &[&str])] = &[
    (
        Card::Cage,
        &["set-aside", "duration-ability", "duration-reminder"],
    ),
    (
        Card::Grotto,
        &["set-aside", "duration-ability", "duration-reminder"],
    ),
    (Card::Shaman, &["card-selection", "duration-ability"]),
    (
        Card::SecludedShrine,
        &["card-selection", "duration-ability", "duration-reminder"],
    ),
    (
        Card::Siren,
        &["card-selection", "duration-ability", "duration-reminder"],
    ),
    (
        Card::Stowaway,
        &["reaction-ability", "duration-ability", "duration-reminder"],
    ),
    (Card::Taskmaster, DURATION),
    (Card::Abundance, DURATION),
    (
        Card::CabinBoy,
        &[
            "semantic-modes",
            "pile-selection",
            "duration-ability",
            "duration-reminder",
        ],
    ),
    (Card::Crucible, &["card-selection"]),
    (
        Card::Flagship,
        &["repeated-play", "duration-ability", "duration-reminder"],
    ),
    (
        Card::FortuneHunter,
        &["looking", "optional-play", "ordered-cards"],
    ),
    (
        Card::Gondola,
        &[
            "semantic-modes",
            "optional-play",
            "duration-ability",
            "duration-reminder",
        ],
    ),
    (Card::HarborVillage, &["optional-ability"]),
    (Card::LandingParty, DURATION),
    (
        Card::Mapmaker,
        &["looking", "card-selection", "reaction-ability"],
    ),
    (Card::Maroon, &["card-selection"]),
    (
        Card::Rope,
        &["card-selection", "duration-ability", "duration-reminder"],
    ),
    (Card::SwampShacks, &[]),
    (Card::Tools, &["card-selection"]),
    (
        Card::BuriedTreasure,
        &["duration-ability", "duration-reminder", "optional-ability"],
    ),
    (Card::Crew, DURATION),
    (
        Card::Enlarge,
        &[
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
            "duration-ability",
            "duration-reminder",
        ],
    ),
    (Card::Figurine, &["card-selection"]),
    (Card::FirstMate, &["optional-play"]),
    (
        Card::Frigate,
        &["off-turn-attack", "card-selection", "duration-reminder"],
    ),
    (Card::Longship, DURATION),
    (Card::MiningRoad, &["optional-ability"]),
    (Card::Pendant, &[]),
    (Card::Pilgrim, &["card-selection"]),
    (
        Card::Quartermaster,
        &[
            "semantic-modes",
            "pile-selection",
            "set-aside",
            "duration-ability",
            "duration-reminder",
        ],
    ),
    (Card::SilverMine, &["pile-selection"]),
    (
        Card::Trickster,
        &["off-turn-attack", "set-aside", "optional-ability"],
    ),
    (Card::KingsCache, &["optional-play", "repeated-play"]),
    (Card::Avoid, &["event", "card-selection"]),
    (
        Card::Bury,
        &["event", "discard-inspection", "card-selection"],
    ),
    (Card::Deliver, &["event", "set-aside"]),
    (Card::Rush, &["event", "optional-ability"]),
    (Card::Launch, &["event"]),
    (Card::Mirror, &["event"]),
    (Card::Prepare, &["event", "set-aside", "duration-ability"]),
    (Card::Journey, &["event", "extra-turn"]),
    (
        Card::Maelstrom,
        &["event", "off-turn-attack", "card-selection"],
    ),
    (Card::Cheap, &["trait"]),
    (Card::Fated, &["trait", "optional-ability", "ordered-cards"]),
    (Card::Fawning, &["trait"]),
    (Card::Friendly, TRAIT),
    (Card::Hasty, &["trait", "set-aside", "duration-ability"]),
    (Card::Inherited, &["trait"]),
    (Card::Inspiring, TRAIT),
    (Card::Nearby, &["trait"]),
    (
        Card::Patient,
        &["trait", "optional-ability", "set-aside", "duration-ability"],
    ),
    (Card::Pious, TRAIT),
    (Card::Reckless, &["trait"]),
    (Card::Rich, &["trait"]),
    (Card::Shy, TRAIT),
    (Card::Tireless, &["trait", "set-aside"]),
    (
        Card::Cutthroat,
        &[
            "off-turn-attack",
            "card-selection",
            "duration-ability",
            "loot",
        ],
    ),
    (Card::JewelledEgg, &["loot"]),
    (Card::Pickaxe, &["card-selection", "loot"]),
    (Card::SackOfLoot, &["loot"]),
    (
        Card::Search,
        &["duration-ability", "duration-reminder", "loot"],
    ),
    (Card::WealthyVillage, &["loot"]),
    (Card::Peril, &["event", "card-selection", "loot"]),
    (Card::Foray, &["event", "card-selection", "loot"]),
    (Card::Looting, &["event", "loot"]),
    (
        Card::Invasion,
        &["event", "optional-play", "pile-selection", "loot"],
    ),
    (Card::Prosper, &["event", "pile-selection", "loot"]),
    (Card::Cursed, &["trait", "loot"]),
    (
        Card::Amphora,
        &[
            "loot",
            "semantic-modes",
            "duration-ability",
            "duration-reminder",
        ],
    ),
    (Card::Doubloons, &["loot"]),
    (
        Card::EndlessChalice,
        &["loot", "duration-ability", "duration-reminder"],
    ),
    (
        Card::Figurehead,
        &["loot", "duration-ability", "duration-reminder"],
    ),
    (Card::Hammer, &["loot", "pile-selection"]),
    (Card::Insignia, &["loot", "optional-ability"]),
    (
        Card::Jewels,
        &["loot", "duration-ability", "duration-reminder"],
    ),
    (Card::Orb, &["loot", "discard-inspection", "semantic-modes"]),
    (Card::PrizeGoat, &["loot", "card-selection"]),
    (Card::PuzzleBox, &["loot", "set-aside"]),
    (Card::Sextant, &["loot", "looking", "ordered-cards"]),
    (Card::Shield, &["loot", "reaction-ability"]),
    (
        Card::SpellScroll,
        &["loot", "pile-selection", "semantic-modes"],
    ),
    (Card::Staff, &["loot", "optional-play"]),
    (Card::Sword, &["loot", "off-turn-attack", "card-selection"]),
];

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    // The Loot pile is offered once each Loot is.
    if card == Card::Loot {
        let mut loots = Card::ALL.iter().filter(|c| c.def().pile == Some(card));
        return loots
            .all(|&c| requirements(c).is_some())
            .then_some(&["loot"]);
    }
    ADMITTED
        .iter()
        .find(|(c, _)| *c == card)
        .map(|(_, features)| *features)
}
