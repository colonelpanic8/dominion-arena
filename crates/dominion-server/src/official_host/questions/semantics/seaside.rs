//! Question bindings for this set's cards (see `semantics::SETS`).
use super::{AbilityArguments, Binding, ProjectionError, WireCatalog, card_names, gap};
use dominion_engine::{
    Card, CardId, Decision, DecisionKind, Occasion, Opt, Purpose, Trigger, View,
};
use dominion_online::questions::LogArgument;

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let (kind, story) = match (d.source?, d.purpose) {
        (Ambassador, P::Reveal) => ("REVEAL", "AMBASSADOR_REVEAL"),
        (Ambassador, P::Return) => return Some(ambassador(view, d, c)),
        (Blockade, P::Gain) => ("GAIN", "BLOCKADE"),
        (Explorer, P::Reveal) => ("REVEAL", "EXPLORER"),
        (Lookout, P::Trash) => ("TRASH", "LOOKOUT_TRASH"),
        (Lookout, P::Discard) => ("DISCARD", "LOOKOUT_DISCARD"),
        (Navigator, P::Either) => ("COMPLEX_OR", "NAVIGATOR"),
        (Navigator, P::TopDeck) if ordered => ("ORDER_CARDS", "NAVIGATOR"),
        (PearlDiver, P::TopDeck) => {
            let DecisionKind::Cards { candidates, .. } = &d.kind else {
                return None;
            };
            return Some(
                cards_argument(view, d, c, candidates)
                    .map(|arg| ("TOPDECK", "PEARL_DIVER", vec![arg])),
            );
        }
        (Pirate, P::Gain) => ("GAIN", "PIRATE"),
        (PirateShip, P::Mode) => {
            let coins = view.player(d.player).pirate_ship();
            let coins = i32::try_from(coins).unwrap_or(i32::MAX);
            return Some(Ok((
                "CHOOSE_MODE",
                "PIRATE_SHIP_MODE",
                vec![LogArgument::Amount {
                    coins,
                    potions: 0,
                    debt: 0,
                }],
            )));
        }
        (PirateShip, P::Trash) => ("TRASH", "PIRATE_SHIP_TRASH"),
        (Sailor, P::Trash) => ("TRASH", "SAILOR"),
        (Salvager, P::Trash) => ("TRASH", "SALVAGER"),
        (SeaWitch, P::Discard) => ("DISCARD", "SEA_WITCH"),
        (Smugglers, P::Gain) => {
            let DecisionKind::Choose { options, .. } = &d.kind else {
                return None;
            };
            let all = options.iter().all(|&o| option_card(view, d, o).is_some());
            (
                "GAIN",
                if all {
                    "SMUGGLERS_GAIN"
                } else {
                    "SMUGGLERS_MAY_GAIN"
                },
            )
        }
        (TidePools, P::Discard) => ("DISCARD", "TIDE_POOLS"),
        (Warehouse, P::Discard) => ("DISCARD", "WAREHOUSE"),
        _ => return None,
    };
    Some(Ok((kind, story, vec![])))
}

/// "You may return CARDS0" for one copy, "Return 0, 1 or CARDS0" (two of them) for more.
fn ambassador(view: &View<'_>, d: &Decision, c: &WireCatalog) -> Result<Binding, ProjectionError> {
    let DecisionKind::Cards {
        candidates, max, ..
    } = &d.kind
    else {
        return Err(gap(d, "Ambassador return shape"));
    };
    let card = candidates
        .first()
        .and_then(|&id| view.card(id))
        .ok_or_else(|| gap(d, "missing revealed card"))?;
    let story = if *max >= 2 {
        "AMBASSADOR_RETURN_TWO"
    } else {
        "AMBASSADOR_RETURN_ONE"
    };
    let copies = std::iter::repeat_n(card, (*max).max(1));
    Ok(("RETURN_TO_SUPPLY", story, vec![card_names(c, copies)?]))
}

fn cards_argument(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
    cards: &[CardId],
) -> Result<LogArgument, ProjectionError> {
    let names = cards
        .iter()
        .map(|&id| view.card(id))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| gap(d, "card argument not visible"))?;
    card_names(c, names)
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    let choices: &[&str] = match card {
        Card::PirateShip => &["PIRATE_SHIP_PROFIT", "PIRATE_SHIP_RAID"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (card, ability) {
        (Astrolabe, 0) => "ASTROLABE_ABILITY",
        (Blockade, 0) => "BLOCKADE_ABILITY",
        (Corsair, 0) => "CORSAIR_NEXT_TURN_ABILITY",
        (Corsair, 1) => "CORSAIR_TRASH",
        (Embargo, 0) => "EMBARGO_GAIN_CURSE",
        (FishingVillage, 0) => "FISHING_VILLAGE_ABILITY",
        (MerchantShip, 0) => "MERCHANT_SHIP_ABILITY",
        // "+1 Card", now or when the player to the right gains a card.
        (Monkey, 0 | 1) => "MONKEY_NEXT_TURN_ABILITY",
        (Pirate, 0) => "PIRATE_ABILITY",
        (Pirate, 1) => "PIRATE_REACTION",
        (Sailor, 0) => "SAILOR_NEXT_TURN",
        (Sailor, 1) => "SAILOR_REACTION",
        (SeaWitch, 0) => "SEA_WITCH_ABILITY",
        (Tactician, 0) => "TACTICIAN_ABILITY",
        (TidePools, 0) => "TIDE_POOLS_ABILITY",
        (Treasury, 0) => "TREASURY",
        (Wharf, 0) => "WHARF_ABILITY",
        _ => return None,
    })
}

pub(super) fn option_mode(_card: Card, _opt: Opt) -> Option<&'static str> {
    None
}

pub(super) fn option_button(card: Card, opt: Opt) -> Option<(&'static str, &'static str)> {
    match (card, opt) {
        (Card::Navigator, Opt::Mode(0)) => Some(("NAVIGATOR", "DISCARD_ALL")),
        (Card::Navigator, Opt::Mode(1)) => Some(("NAVIGATOR", "CONTINUE")),
        _ => None,
    }
}

pub(super) fn option_card(view: &View<'_>, d: &Decision, opt: Opt) -> Option<CardId> {
    match (d.source?, opt) {
        (Card::Smugglers, Opt::Card(card)) => view.pile(view.supply_pile(card)?).top(),
        _ => None,
    }
}

pub(super) fn ability_arguments(
    view: &View<'_>,
    t: &Trigger,
    occ: Option<&Occasion>,
    c: &WireCatalog,
) -> AbilityArguments {
    let card = match (t.card, t.ability, occ) {
        (Card::Blockade, 0, _) => view
            .lasting()
            .into_iter()
            .find(|l| Some(l.id) == t.lasting)
            .and_then(|l| l.data.pick),
        (Card::Corsair, 1, Some(Occasion::Play { card, .. })) => Some(*card),
        (Card::Sailor, 1, Some(Occasion::Gain { card, .. })) => Some(*card),
        _ => return None,
    };
    Some(super::ability_card(view, t, card, c))
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Ambassador => &[
            ("QuestionIds", "AMBASSADOR_RETURN_ONE"),
            ("QuestionIds", "AMBASSADOR_RETURN_TWO"),
            ("QuestionIds", "AMBASSADOR_REVEAL"),
            ("QuestionTypes", "RETURN_TO_SUPPLY"),
            ("QuestionTypes", "REVEAL"),
        ],
        Astrolabe => &[("AbilityNames", "ASTROLABE_ABILITY")],
        Blockade => &[
            ("AbilityNames", "BLOCKADE_ABILITY"),
            ("QuestionIds", "BLOCKADE"),
            ("QuestionTypes", "GAIN"),
        ],
        Corsair => &[
            ("AbilityNames", "CORSAIR_NEXT_TURN_ABILITY"),
            ("AbilityNames", "CORSAIR_TRASH"),
        ],
        Explorer => &[("QuestionIds", "EXPLORER"), ("QuestionTypes", "REVEAL")],
        FishingVillage => &[("AbilityNames", "FISHING_VILLAGE_ABILITY")],
        Lookout => &[
            ("QuestionIds", "LOOKOUT_DISCARD"),
            ("QuestionIds", "LOOKOUT_TRASH"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "TRASH"),
        ],
        MerchantShip => &[("AbilityNames", "MERCHANT_SHIP_ABILITY")],
        Monkey => &[("AbilityNames", "MONKEY_NEXT_TURN_ABILITY")],
        Navigator => &[
            ("DeclineButtonIds", "CONTINUE"),
            ("DeclineButtonIds", "DISCARD_ALL"),
            ("QuestionIds", "NAVIGATOR"),
            ("QuestionTypes", "BUTTON"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "ORDER_CARDS"),
        ],
        PearlDiver => &[("QuestionIds", "PEARL_DIVER"), ("QuestionTypes", "TOPDECK")],
        Pirate => &[
            ("AbilityNames", "PIRATE_ABILITY"),
            ("AbilityNames", "PIRATE_REACTION"),
            ("QuestionIds", "PIRATE"),
            ("QuestionTypes", "GAIN"),
        ],
        PirateShip => &[
            ("CardModes", "PIRATE_SHIP_PROFIT"),
            ("CardModes", "PIRATE_SHIP_RAID"),
            ("QuestionIds", "PIRATE_SHIP_MODE"),
            ("QuestionIds", "PIRATE_SHIP_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "TRASH"),
        ],
        Sailor => &[
            ("AbilityNames", "SAILOR_NEXT_TURN"),
            ("AbilityNames", "SAILOR_REACTION"),
            ("QuestionIds", "SAILOR"),
            ("QuestionTypes", "TRASH"),
        ],
        Salvager => &[("QuestionIds", "SALVAGER"), ("QuestionTypes", "TRASH")],
        SeaWitch => &[
            ("AbilityNames", "SEA_WITCH_ABILITY"),
            ("QuestionIds", "SEA_WITCH"),
            ("QuestionTypes", "DISCARD"),
        ],
        Smugglers => &[
            ("QuestionIds", "SMUGGLERS_GAIN"),
            ("QuestionIds", "SMUGGLERS_MAY_GAIN"),
            ("QuestionTypes", "GAIN"),
        ],
        Tactician => &[("AbilityNames", "TACTICIAN_ABILITY")],
        TidePools => &[
            ("AbilityNames", "TIDE_POOLS_ABILITY"),
            ("QuestionIds", "TIDE_POOLS"),
            ("QuestionTypes", "DISCARD"),
        ],
        Treasury => &[("AbilityNames", "TREASURY")],
        Warehouse => &[("QuestionIds", "WAREHOUSE"), ("QuestionTypes", "DISCARD")],
        Wharf => &[("AbilityNames", "WHARF_ABILITY")],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        Ambassador => &["off-turn-attack", "card-selection"],
        Astrolabe | FishingVillage | MerchantShip | Tactician | Wharf => {
            &["duration-reminder", "duration-ability"]
        }
        Blockade => &[
            "pile-selection",
            "set-aside",
            "duration-reminder",
            "duration-ability",
            "off-turn-attack",
        ],
        Corsair => &[
            "off-turn-attack",
            "duration-reminder",
            "duration-ability",
            "optional-ability",
        ],
        Cutpurse => &["off-turn-attack", "card-selection"],
        Explorer => &["card-selection"],
        Lookout => &["looking", "card-selection"],
        Monkey => &["duration-reminder", "duration-ability"],
        Navigator => &["looking", "ordered-cards", "optional-effect"],
        PearlDiver => &["card-selection"],
        Pirate => &[
            "reaction-ability",
            "pile-selection",
            "duration-reminder",
            "duration-ability",
        ],
        PirateShip => &["off-turn-attack", "semantic-modes", "card-selection"],
        Sailor => &[
            "card-selection",
            "duration-reminder",
            "duration-ability",
            "optional-ability",
        ],
        Salvager | Warehouse => &["card-selection"],
        SeaHag => &["off-turn-attack"],
        SeaWitch | TidePools => &["card-selection", "duration-reminder", "duration-ability"],
        Smugglers => &["card-selection"],
        TreasureMap => &["card-selection"],
        Treasury => &["optional-ability"],
        SeaChart => &[],
        _ => return None,
    })
}
