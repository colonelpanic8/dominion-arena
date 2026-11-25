//! Question bindings for this set's cards (see `semantics::SETS`).
use super::{Binding, ChoiceShape, ProjectionError, WireCatalog, gap};
use dominion_engine::{
    Card, CardId, Decision, DecisionKind, Event, Opt, PlayerId, Purpose, Trigger, Types, View,
};
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
    let naming = matches!(d.kind, DecisionKind::NameCard);
    let (kind, story) = match (d.source?, d.purpose) {
        (Bauble, P::Mode) => ("CHOOSE_MODE", "BAUBLE"),
        (Sycophant, P::Discard) => ("DISCARD", "SYCOPHANT"),
        (Sentinel, P::Trash) => ("TRASH", "SENTINEL_TRASH"),
        (Sentinel, P::TopDeck) if ordered => ("ORDER_CARDS", "SENTINEL_TOPDECK"),
        (Broker, P::Trash) => ("TRASH", "BROKER_TRASH"),
        (Broker, P::Mode) => ("CHOOSE_MODE", "BROKER_MODE"),
        (Carpenter, P::Trash) => ("TRASH", "CARPENTER_TRASH"),
        (Carpenter, P::Gain) if view.pending_reference().is_some() => {
            ("GAIN", "CARPENTER_GAIN_UP_TO")
        }
        (Carpenter, P::Gain) => ("GAIN", "CARPENTER_GAIN"),
        (Courier, P::Play) => ("PLAY", "COURIER"),
        (Innkeeper, P::Mode) => ("CHOOSE_MODE", "INNKEEPER_MODE"),
        (Innkeeper, P::Discard) => ("DISCARD", "INNKEEPER_DISCARD"),
        (RoyalGalley, P::Play) => ("PLAY", "ROYAL_GALLEY"),
        (Town, P::Mode) => ("CHOOSE_MODE", "TOWN_MODE"),
        (Barbarian, P::Gain) => ("GAIN", "BARBARIAN"),
        (CapitalCity, P::Discard | P::Other) if choosing => ("CHOOSE_MODE", "CAPITAL_CITY_MODE"),
        (CapitalCity, P::Discard) => ("DISCARD", "CAPITAL_CITY_DISCARD"),
        (Contract, P::SetAside) => ("SET_ASIDE", "CONTRACT"),
        (Hunter, P::Move) => ("PUT_IN_HAND", hunter_story(view, d)),
        (Marquis, P::Discard) => ("DISCARD", "MARQUIS"),
        (Modify, P::Trash) => ("TRASH", "MODIFY_TRASH"),
        (Modify, P::Mode) => ("CHOOSE_MODE", "MODIFY_MODE"),
        (Modify, P::Gain) => ("GAIN", "MODIFY_GAIN"),
        (Specialist, P::Play) => ("PLAY", "SPECIALIST_PLAY"),
        (Specialist, P::Mode) => ("CHOOSE_MODE", "SPECIALIST_MODE"),
        (Swap, P::Return) => ("RETURN_TO_SUPPLY", "SWAP_RETURN"),
        (Swap, P::Gain) => ("GAIN", "SWAP_GAIN"),
        (TownCrier, P::Mode) => ("CHOOSE_MODE", "TOWN_CRIER"),
        (TownCrier, P::Other) if choosing => ("ROTATE", "MAY_ROTATE_TOWNSFOLK"),
        (Blacksmith, P::Mode) => ("CHOOSE_MODE", "BLACKSMITH"),
        (Miller, P::Move) => ("PUT_IN_HAND", "MILLER"),
        (Elder, P::Play) => ("PLAY", "ELDER"),
        (Acolyte, P::Trash) if choosing => ("TRASH", "ACOLYTE_SELF"),
        (Acolyte, P::Trash) => ("TRASH", "ACOLYTE"),
        (Sorceress, P::Name) if naming => ("WISH", "SORCERESS"),
        (Sibyl, P::TopDeck) => ("TOPDECK", "SIBYL_TOPDECK"),
        (Sibyl, P::Move) => ("BOTTOMDECK", "SIBYL_BOTTOMDECK"),
        (HerbGatherer, P::Play) => ("PLAY", "HERB_GATHERER_PLAY"),
        (HerbGatherer, P::Other) if choosing => ("ROTATE", "MAY_ROTATE_AUGURS"),
        (BattlePlan, P::Reveal) => ("REVEAL", "BATTLE_PLAN_REVEAL"),
        (Archer, P::Other) => ("REVEAL", "ARCHER_HIDE"),
        (Archer, P::Discard) => ("DISCARD", "ARCHER_DISCARD"),
        (BattlePlan, P::Other) => ("ROTATE", "BATTLE_PLAN_ROTATE"),
        (HillFort, P::Gain) => ("GAIN", "HILL_FORT_GAIN"),
        (HillFort, P::Mode) => ("CHOOSE_MODE", "HILL_FORT_MODE"),
        (Stronghold, P::Mode) => ("CHOOSE_MODE", "STRONGHOLD_MODE"),
        (Tent, P::Other) if choosing => ("ROTATE", "MAY_ROTATE_FORTS"),
        (OldMap, P::Discard) => ("DISCARD", "OLD_MAP_DISCARD"),
        (OldMap, P::Other) if choosing => ("ROTATE", "MAY_ROTATE_ODYSSEYS"),
        (SunkenTreasure, P::Gain) => ("GAIN", "SUNKEN_TREASURE"),
        (Student, P::Other) if choosing => ("ROTATE", "MAY_ROTATE_WIZARDS"),
        (Student, P::Trash) => ("TRASH", "STUDENT"),
        (Conjurer, P::Gain) => ("GAIN", "CONJURER"),
        (Sorcerer, P::Name) if naming => ("WISH", "SORCERER"),
        (Lich, P::Gain) => ("GAIN", "LICH"),
        (Importer, P::Gain) => ("GAIN", "IMPORTER"),
        (Skirmisher, P::Discard) => ("DISCARD", "SKIRMISHER"),
        (ArchitectsGuild, P::Gain) => ("GAIN", "ARCHITECTS_GUILD"),
        (CaveDwellers, P::Discard) => ("DISCARD", "CAVE_DWELLERS"),
        (CaveDwellers, P::Other) if choosing => ("CHOOSE_MODE", "CAVE_DWELLERS"),
        (DesertGuides, P::Other) if choosing => ("CHOOSE_MODE", "DESERT_GUIDES"),
        (CraftersGuild, P::Gain) => ("GAIN", "CRAFTERS_GUILD"),
        (ForestDwellers, P::Discard) => ("DISCARD", "FOREST_DWELLERS_DISCARD"),
        (ForestDwellers, P::TopDeck) if ordered => ("ORDER_CARDS", "FOREST_DWELLERS_TOPDECK"),
        (GangOfPickpockets, P::Other) if choosing => ("CHOOSE_MODE", "GANG_OF_PICKPOCKETS"),
        (GangOfPickpockets, P::Discard) => ("DISCARD", "GANG_OF_PICKPOCKETS"),
        (FamilyOfInventors, P::Other) => ("SELECT_ZONE", "FAMILY_OF_INVENTORS"),
        (MarketTowns, P::Play) => ("PLAY", "MARKET_TOWNS_FIRST"),
        (PeacefulCult, P::Trash) => ("TRASH", "PEACEFUL_CULT"),
        (WoodworkersGuild, P::Trash) => ("TRASH", "WOODWORKERS_GUILD_TRASH"),
        (WoodworkersGuild, P::Gain) => ("GAIN", "WOODWORKERS_GUILD_GAIN"),
        (CoastalHaven, P::SetAside) => ("SET_ASIDE", "COASTAL_HAVEN"),
        (OrderOfAstrologers, P::TopDeck) if ordered => ("ORDER_CARDS", "SENTRY_TOPDECK"),
        (OrderOfAstrologers, P::TopDeck) => ("TOPDECK", "ORDER_OF_ASTROLOGERS"),
        (OrderOfMasons, P::Move) => ("DISCARD", "ORDER_OF_MASONS"),
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
    Ok(match story {
        "CARPENTER_GAIN_UP_TO" | "MODIFY_GAIN" | "MODIFY_MODE" => {
            // Up to $2 more than the trashed card; nothing trashed ($0) when the hand was empty.
            let base = view
                .pending_reference()
                .and_then(|id| view.cost_of(id))
                .unwrap_or_default();
            let limit = base.plus_coins(2);
            vec![LogArgument::Amount {
                coins: limit.coins.into(),
                potions: limit.potions.into(),
                debt: limit.debt.into(),
            }]
        }
        "INNKEEPER_DISCARD" => match d.kind {
            DecisionKind::Cards { min, .. } => vec![LogArgument::Number(
                i32::try_from(min).map_err(|_| ProjectionError::Size("discard count"))?,
            )],
            _ => return Err(gap(d, "Innkeeper discard shape")),
        },
        "SPECIALIST_MODE" => {
            let card = view
                .pending_reference()
                .and_then(|id| view.card(id))
                .ok_or_else(|| gap(d, "missing visible played card"))?;
            vec![names(c, card)?]
        }
        "ARCHER_DISCARD" => {
            let DecisionKind::Cards { candidates, .. } = &d.kind else {
                return Err(gap(d, "Archer discard shape"));
            };
            let owner = candidates.iter().find_map(|&id| match view.zone(id) {
                Some(dominion_engine::Zone::Hand(p)) => Some(p),
                _ => None,
            });
            vec![LogArgument::Player(
                owner
                    .ok_or_else(|| gap(d, "missing revealed hand"))?
                    .0
                    .into(),
            )]
        }
        "SWAP_GAIN" => {
            let card =
                last_returned(view, d.player).ok_or_else(|| gap(d, "missing returned card"))?;
            vec![names(c, card)?]
        }
        _ => vec![],
    })
}

/// Hunter asks for an Action, then a Treasure, then a Victory card; each question's
/// candidates all have its type.
fn hunter_story(view: &View<'_>, d: &Decision) -> &'static str {
    let DecisionKind::Cards { candidates, .. } = &d.kind else {
        return "HUNTER_ACTION";
    };
    let shared = candidates
        .iter()
        .filter_map(|&id| view.types(id))
        .fold(Types::ACTION | Types::TREASURE | Types::VICTORY, |a, t| {
            a & t
        });
    if shared.contains(Types::ACTION) {
        "HUNTER_ACTION"
    } else if shared.contains(Types::TREASURE) {
        "HUNTER_TREASURE"
    } else {
        "HUNTER_VICTORY"
    }
}

pub(super) fn names(c: &WireCatalog, card: Card) -> Result<LogArgument, ProjectionError> {
    Ok(LogArgument::CardNames(vec![CardFrequency {
        name: c.card(card)?,
        frequency: 1,
    }]))
}

/// The card `player` most recently returned to a pile.
fn last_returned(view: &View<'_>, player: PlayerId) -> Option<Card> {
    view.log().iter().rev().find_map(|e| match e {
        Event::Moved {
            player: p,
            to: dominion_engine::Zone::Pile(_),
            cards,
            ..
        } if *p == player => cards.last().copied(),
        _ => None,
    })
}

/// The card `player` most recently gained.
pub(super) fn last_gained(view: &View<'_>, player: PlayerId) -> Option<Card> {
    view.log().iter().rev().find_map(|e| match e {
        Event::Gained {
            player: p, card, ..
        } if *p == player => Some(*card),
        _ => None,
    })
}

/// The top card of `randomizer`'s pile, to rotate.
fn pile_top(view: &View<'_>, randomizer: Card) -> Option<CardId> {
    view.piles().find(|p| p.card() == randomizer)?.top()
}

pub(super) fn choice_shape(
    view: &View<'_>,
    d: &Decision,
) -> Option<Result<ChoiceShape, ProjectionError>> {
    use Card::*;
    if !matches!(d.kind, DecisionKind::Choose { .. }) {
        return None;
    }
    let rotate = |randomizer: Card| {
        Some(
            pile_top(view, randomizer)
                .map(|card| ChoiceShape::Target {
                    card,
                    yes: Opt::Yes,
                    no: Opt::No,
                    decline: "DONT_ROTATE",
                })
                .ok_or_else(|| gap(d, "rotating an empty pile")),
        )
    };
    match (d.source?, d.purpose) {
        (TownCrier, Purpose::Other) => rotate(Townsfolk),
        (HerbGatherer, Purpose::Other) => rotate(Augurs),
        (Tent, Purpose::Other) => rotate(Forts),
        (OldMap, Purpose::Other) => rotate(Odysseys),
        (Student, Purpose::Other) => rotate(Wizards),
        (Acolyte, Purpose::Trash) => Some(
            view.pending_source()
                .map(|card| ChoiceShape::Target {
                    card,
                    yes: Opt::Yes,
                    no: Opt::No,
                    decline: "DONT_TRASH",
                })
                .ok_or_else(|| gap(d, "missing visible Acolyte")),
        ),
        (CapitalCity, purpose) => Some(Ok(ChoiceShape::Modes {
            options: vec![(
                Opt::Yes,
                if purpose == Purpose::Discard {
                    "CAPITAL_CITY_DISCARD"
                } else {
                    "CAPITAL_CITY_PAY"
                },
            )],
            decline: Some((Opt::No, "DONT")),
        })),
        (DesertGuides, _) => Some(Ok(ChoiceShape::Modes {
            options: vec![
                (Opt::Yes, "DESERT_GUIDES_NEW_HAND"),
                (Opt::No, "DESERT_GUIDES_KEEP"),
            ],
            decline: None,
        })),
        (CaveDwellers | GangOfPickpockets, _) => Some(Ok(ChoiceShape::Buttons(vec![
            (Opt::Yes, "SPEND_FAVOR"),
            (Opt::No, "CONTINUE"),
        ]))),
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
    let lasting = || {
        view.lasting()
            .into_iter()
            .find(|l| Some(l.id) == t.lasting)
            .ok_or_else(missing)
    };
    Some(match (t.card, t.ability) {
        (Card::Contract | Card::RoyalGalley, 0) => lasting().and_then(|l| {
            let card = l
                .data
                .pick
                .and_then(|id| view.card(id))
                .ok_or_else(missing)?;
            Ok(vec![names(c, card)?])
        }),
        (Card::Garrison, 0) => lasting().map(|l| vec![LogArgument::Number(l.data.n.max(0))]),
        (Card::CityState | Card::TrappersLodge, 0) | (Card::Bauble, 0) => {
            last_gained(view, t.owner)
                .ok_or_else(missing)
                .and_then(|card| Ok(vec![names(c, card)?]))
        }
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        Bauble => &[
            "BAUBLE_BUY",
            "BAUBLE_COIN",
            "BAUBLE_FAVOR",
            "BAUBLE_TOPDECK",
        ],
        Broker => &[
            "BROKER_CARDS",
            "BROKER_ACTIONS",
            "BROKER_COINS",
            "BROKER_FAVORS",
        ],
        Innkeeper => &["INNKEEPER_CARD", "INNKEEPER_3", "INNKEEPER_5_6"],
        Town => &["TOWN_VILLAGE", "TOWN_COINS_BUY"],
        Modify => &["MODIFY_CANTRIP", "MODIFY_GAIN"],
        Specialist => &["SPECIALIST_PLAY_AGAIN", "SPECIALIST_GAIN_COPY"],
        TownCrier => &[
            "TOWN_CRIER_COINS",
            "TOWN_CRIER_SILVER",
            "TOWN_CRIER_CANTRIP",
        ],
        Blacksmith => &["BLACKSMITH_OWL", "BLACKSMITH_OTTER", "BLACKSMITH_PIG"],
        HillFort => &["HILL_FORT_TO_HAND", "HILL_FORT_CANTRIP"],
        Stronghold => &["STRONGHOLD_COINS", "STRONGHOLD_CARDS"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (card, ability) {
        (Sycophant, 0) => "SYCOPHANT",
        (Lich, 0) => "LICH",
        (Highwayman, 0) => "HIGHWAYMAN_NEXT_TURN_ABILITY",
        (Highwayman, 1) => "HIGHWAYMAN_REPLACE_ABILITY",
        (Warlord, 0) => "WARLORD_ABILITY",
        (Stronghold, 0) => "STRONGHOLD_ABILITY",
        (Garrison, 0) => "GARRISON_REMOVE_ABILITY",
        (Garrison, 1) => "GARRISON_ADD_TOKEN",
        (Contract, 0) => "CONTRACT_ABILITY",
        (RoyalGalley, 0) => "ROYAL_GALLEY_PLAY",
        (Conjurer, 0) => "CONJURER",
        (Importer, 0) => "IMPORTER_ABILITY",
        (MerchantCamp, 0) => "MERCHANT_CAMP_TOPDECK",
        (Tent, 0) => "TENT_TOPDECK",
        (Territory, 0) => "TERRITORY_GAIN",
        (Galleria, 0) => "GALLERIA",
        (Guildmaster, 0) => "GUILDMASTER",
        (Skirmisher, 0) => "SKIRMISHER",
        (Bauble, 0) => "TOPDECK_REACTION",
        (CoastalHaven, 0 | 1) => "COASTAL_HAVEN",
        (ArchitectsGuild, 0) => "ARCHITECTS_GUILD",
        (CaveDwellers, 0) => "CAVE_DWELLERS",
        (CircleOfWitches, 0) => "CIRCLE_OF_WITCHES",
        (CityState, 0) => "CITY_STATE",
        (CraftersGuild, 0) => "CRAFTERS_GUILD",
        (DesertGuides, 0) => "DESERT_GUIDES",
        (FamilyOfInventors, 0) => "FAMILY_OF_INVENTORS",
        (FellowshipOfScribes, 0) => "FELLOWSHIP_OF_SCRIBES",
        (ForestDwellers, 0) => "FOREST_DWELLERS",
        (GangOfPickpockets, 0) => "GANG_OF_PICKPOCKETS",
        (IslandFolk, 0) => "ISLAND_FOLK",
        (LeagueOfBankers, 0) => "LEAGUE_OF_BANKERS",
        (LeagueOfShopkeepers, 0) => "LEAGUE_OF_SHOPKEEPERS",
        (MarketTowns, 0) => "MARKET_TOWNS",
        (MountainFolk, 0) => "MOUNTAIN_FOLK",
        (OrderOfAstrologers, 0) => "ORDER_OF_ASTROLOGERS",
        (OrderOfMasons, 0) => "ORDER_OF_MASONS",
        (PeacefulCult, 0) => "PEACEFUL_CULT",
        (TrappersLodge, 0) => "TRAPPERS_LODGE",
        (WoodworkersGuild, 0) => "WOODWORKERS_GUILD",
        _ => return None,
    })
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Bauble => &[
            ("QuestionIds", "BAUBLE"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "BAUBLE_BUY"),
            ("CardModes", "BAUBLE_COIN"),
            ("CardModes", "BAUBLE_FAVOR"),
            ("CardModes", "BAUBLE_TOPDECK"),
            ("AbilityNames", "TOPDECK_REACTION"),
        ],
        Sycophant => &[
            ("QuestionIds", "SYCOPHANT"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "SYCOPHANT"),
        ],
        Sentinel => &[
            ("QuestionIds", "SENTINEL_TRASH"),
            ("QuestionIds", "SENTINEL_TOPDECK"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "ORDER_CARDS"),
        ],
        Broker => &[
            ("QuestionIds", "BROKER_TRASH"),
            ("QuestionIds", "BROKER_MODE"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "BROKER_CARDS"),
            ("CardModes", "BROKER_ACTIONS"),
            ("CardModes", "BROKER_COINS"),
            ("CardModes", "BROKER_FAVORS"),
        ],
        Carpenter => &[
            ("QuestionIds", "CARPENTER_TRASH"),
            ("QuestionIds", "CARPENTER_GAIN"),
            ("QuestionIds", "CARPENTER_GAIN_UP_TO"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "GAIN"),
        ],
        Courier => &[("QuestionIds", "COURIER"), ("QuestionTypes", "PLAY")],
        Innkeeper => &[
            ("QuestionIds", "INNKEEPER_MODE"),
            ("QuestionIds", "INNKEEPER_DISCARD"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "DISCARD"),
            ("CardModes", "INNKEEPER_CARD"),
            ("CardModes", "INNKEEPER_3"),
            ("CardModes", "INNKEEPER_5_6"),
        ],
        RoyalGalley => &[
            ("QuestionIds", "ROYAL_GALLEY"),
            ("QuestionTypes", "PLAY"),
            ("AbilityNames", "ROYAL_GALLEY_PLAY"),
        ],
        Town => &[
            ("QuestionIds", "TOWN_MODE"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "TOWN_VILLAGE"),
            ("CardModes", "TOWN_COINS_BUY"),
        ],
        Barbarian => &[("QuestionIds", "BARBARIAN"), ("QuestionTypes", "GAIN")],
        CapitalCity => &[
            ("QuestionIds", "CAPITAL_CITY_MODE"),
            ("QuestionIds", "CAPITAL_CITY_DISCARD"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "DISCARD"),
            ("CardModes", "CAPITAL_CITY_DISCARD"),
            ("CardModes", "CAPITAL_CITY_PAY"),
            ("DeclineButtonIds", "DONT"),
        ],
        Contract => &[
            ("QuestionIds", "CONTRACT"),
            ("QuestionTypes", "SET_ASIDE"),
            ("AbilityNames", "CONTRACT_ABILITY"),
        ],
        Hunter => &[
            ("QuestionIds", "HUNTER_ACTION"),
            ("QuestionIds", "HUNTER_TREASURE"),
            ("QuestionIds", "HUNTER_VICTORY"),
            ("QuestionTypes", "PUT_IN_HAND"),
        ],
        Marquis => &[("QuestionIds", "MARQUIS"), ("QuestionTypes", "DISCARD")],
        Modify => &[
            ("QuestionIds", "MODIFY_TRASH"),
            ("QuestionIds", "MODIFY_MODE"),
            ("QuestionIds", "MODIFY_GAIN"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
            ("CardModes", "MODIFY_CANTRIP"),
            ("CardModes", "MODIFY_GAIN"),
        ],
        Specialist => &[
            ("QuestionIds", "SPECIALIST_PLAY"),
            ("QuestionIds", "SPECIALIST_MODE"),
            ("QuestionTypes", "PLAY"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "SPECIALIST_PLAY_AGAIN"),
            ("CardModes", "SPECIALIST_GAIN_COPY"),
        ],
        Swap => &[
            ("QuestionIds", "SWAP_RETURN"),
            ("QuestionIds", "SWAP_GAIN"),
            ("QuestionTypes", "RETURN_TO_SUPPLY"),
            ("QuestionTypes", "GAIN"),
        ],
        TownCrier => &[
            ("QuestionIds", "TOWN_CRIER"),
            ("QuestionIds", "MAY_ROTATE_TOWNSFOLK"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "ROTATE"),
            ("CardModes", "TOWN_CRIER_COINS"),
            ("CardModes", "TOWN_CRIER_SILVER"),
            ("CardModes", "TOWN_CRIER_CANTRIP"),
            ("DeclineButtonIds", "DONT_ROTATE"),
        ],
        Blacksmith => &[
            ("QuestionIds", "BLACKSMITH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "BLACKSMITH_OWL"),
            ("CardModes", "BLACKSMITH_OTTER"),
            ("CardModes", "BLACKSMITH_PIG"),
        ],
        Miller => &[("QuestionIds", "MILLER"), ("QuestionTypes", "PUT_IN_HAND")],
        Elder => &[("QuestionIds", "ELDER"), ("QuestionTypes", "PLAY")],
        Acolyte => &[
            ("QuestionIds", "ACOLYTE"),
            ("QuestionIds", "ACOLYTE_SELF"),
            ("QuestionTypes", "TRASH"),
            ("DeclineButtonIds", "DONT_TRASH"),
        ],
        Sorceress => &[("QuestionIds", "SORCERESS"), ("QuestionTypes", "WISH")],
        Sibyl => &[
            ("QuestionIds", "SIBYL_TOPDECK"),
            ("QuestionIds", "SIBYL_BOTTOMDECK"),
            ("QuestionTypes", "TOPDECK"),
            ("QuestionTypes", "BOTTOMDECK"),
        ],
        HerbGatherer => &[
            ("QuestionIds", "HERB_GATHERER_PLAY"),
            ("QuestionIds", "MAY_ROTATE_AUGURS"),
            ("QuestionTypes", "PLAY"),
            ("QuestionTypes", "ROTATE"),
            ("DeclineButtonIds", "DONT_ROTATE"),
        ],
        Archer => &[
            ("QuestionIds", "ARCHER_HIDE"),
            ("QuestionIds", "ARCHER_DISCARD"),
            ("QuestionTypes", "REVEAL"),
            ("QuestionTypes", "DISCARD"),
        ],
        BattlePlan => &[
            ("QuestionIds", "BATTLE_PLAN_REVEAL"),
            ("QuestionIds", "BATTLE_PLAN_ROTATE"),
            ("QuestionTypes", "REVEAL"),
            ("QuestionTypes", "ROTATE"),
        ],
        HillFort => &[
            ("QuestionIds", "HILL_FORT_GAIN"),
            ("QuestionIds", "HILL_FORT_MODE"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "HILL_FORT_TO_HAND"),
            ("CardModes", "HILL_FORT_CANTRIP"),
        ],
        Stronghold => &[
            ("QuestionIds", "STRONGHOLD_MODE"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "STRONGHOLD_COINS"),
            ("CardModes", "STRONGHOLD_CARDS"),
            ("AbilityNames", "STRONGHOLD_ABILITY"),
        ],
        Tent => &[
            ("QuestionIds", "MAY_ROTATE_FORTS"),
            ("QuestionTypes", "ROTATE"),
            ("DeclineButtonIds", "DONT_ROTATE"),
            ("AbilityNames", "TENT_TOPDECK"),
        ],
        Garrison => &[
            ("AbilityNames", "GARRISON_REMOVE_ABILITY"),
            ("AbilityNames", "GARRISON_ADD_TOKEN"),
            ("CounterNames", "CARD_TOKENS"),
        ],
        OldMap => &[
            ("QuestionIds", "OLD_MAP_DISCARD"),
            ("QuestionIds", "MAY_ROTATE_ODYSSEYS"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "ROTATE"),
            ("DeclineButtonIds", "DONT_ROTATE"),
        ],
        Voyage => &[("ExtraTurnTypes", "VOYAGE")],
        SunkenTreasure => &[
            ("QuestionIds", "SUNKEN_TREASURE"),
            ("QuestionTypes", "GAIN"),
        ],
        Student => &[
            ("QuestionIds", "STUDENT"),
            ("QuestionIds", "MAY_ROTATE_WIZARDS"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "ROTATE"),
            ("DeclineButtonIds", "DONT_ROTATE"),
        ],
        Conjurer => &[
            ("QuestionIds", "CONJURER"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "CONJURER"),
        ],
        Sorcerer => &[("QuestionIds", "SORCERER"), ("QuestionTypes", "WISH")],
        Lich => &[
            ("QuestionIds", "LICH"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "LICH"),
        ],
        Highwayman => &[
            ("AbilityNames", "HIGHWAYMAN_NEXT_TURN_ABILITY"),
            ("AbilityNames", "HIGHWAYMAN_REPLACE_ABILITY"),
        ],
        Warlord => &[("AbilityNames", "WARLORD_ABILITY")],
        Importer => &[
            ("QuestionIds", "IMPORTER"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "IMPORTER_ABILITY"),
        ],
        MerchantCamp => &[("AbilityNames", "MERCHANT_CAMP_TOPDECK")],
        Territory => &[("AbilityNames", "TERRITORY_GAIN")],
        Galleria => &[("AbilityNames", "GALLERIA")],
        Guildmaster => &[("AbilityNames", "GUILDMASTER")],
        Skirmisher => &[
            ("QuestionIds", "SKIRMISHER"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "SKIRMISHER"),
        ],
        ArchitectsGuild => &[
            ("QuestionIds", "ARCHITECTS_GUILD"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "ARCHITECTS_GUILD"),
        ],
        CaveDwellers => &[
            ("QuestionIds", "CAVE_DWELLERS"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "BUTTON"),
            ("DeclineButtonIds", "SPEND_FAVOR"),
            ("DeclineButtonIds", "CONTINUE"),
            ("AbilityNames", "CAVE_DWELLERS"),
        ],
        CircleOfWitches => &[("AbilityNames", "CIRCLE_OF_WITCHES")],
        CityState => &[("AbilityNames", "CITY_STATE")],
        CoastalHaven => &[
            ("QuestionIds", "COASTAL_HAVEN"),
            ("QuestionTypes", "SET_ASIDE"),
            ("AbilityNames", "COASTAL_HAVEN"),
        ],
        CraftersGuild => &[
            ("QuestionIds", "CRAFTERS_GUILD"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "CRAFTERS_GUILD"),
        ],
        DesertGuides => &[
            ("QuestionIds", "DESERT_GUIDES"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("CardModes", "DESERT_GUIDES_KEEP"),
            ("CardModes", "DESERT_GUIDES_NEW_HAND"),
            ("AbilityNames", "DESERT_GUIDES"),
        ],
        FamilyOfInventors => &[
            ("QuestionIds", "FAMILY_OF_INVENTORS"),
            ("QuestionTypes", "SELECT_ZONE"),
            ("AbilityNames", "FAMILY_OF_INVENTORS"),
        ],
        FellowshipOfScribes => &[("AbilityNames", "FELLOWSHIP_OF_SCRIBES")],
        ForestDwellers => &[
            ("QuestionIds", "FOREST_DWELLERS_DISCARD"),
            ("QuestionIds", "FOREST_DWELLERS_TOPDECK"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "ORDER_CARDS"),
            ("AbilityNames", "FOREST_DWELLERS"),
        ],
        GangOfPickpockets => &[
            ("QuestionIds", "GANG_OF_PICKPOCKETS"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "BUTTON"),
            ("DeclineButtonIds", "SPEND_FAVOR"),
            ("DeclineButtonIds", "CONTINUE"),
            ("AbilityNames", "GANG_OF_PICKPOCKETS"),
        ],
        IslandFolk => &[
            ("AbilityNames", "ISLAND_FOLK"),
            ("ExtraTurnTypes", "ISLAND_FOLK"),
        ],
        LeagueOfBankers => &[("AbilityNames", "LEAGUE_OF_BANKERS")],
        LeagueOfShopkeepers => &[("AbilityNames", "LEAGUE_OF_SHOPKEEPERS")],
        MarketTowns => &[
            ("QuestionIds", "MARKET_TOWNS_FIRST"),
            ("QuestionTypes", "PLAY"),
            ("AbilityNames", "MARKET_TOWNS"),
        ],
        MountainFolk => &[("AbilityNames", "MOUNTAIN_FOLK")],
        OrderOfAstrologers => &[
            ("QuestionIds", "ORDER_OF_ASTROLOGERS"),
            ("QuestionIds", "SENTRY_TOPDECK"),
            ("QuestionTypes", "TOPDECK"),
            ("QuestionTypes", "ORDER_CARDS"),
            ("AbilityNames", "ORDER_OF_ASTROLOGERS"),
        ],
        OrderOfMasons => &[
            ("QuestionIds", "ORDER_OF_MASONS"),
            ("QuestionTypes", "DISCARD"),
            ("AbilityNames", "ORDER_OF_MASONS"),
        ],
        PeacefulCult => &[
            ("QuestionIds", "PEACEFUL_CULT"),
            ("QuestionTypes", "TRASH"),
            ("AbilityNames", "PEACEFUL_CULT"),
        ],
        TrappersLodge => &[("AbilityNames", "TRAPPERS_LODGE")],
        WoodworkersGuild => &[
            ("QuestionIds", "WOODWORKERS_GUILD_TRASH"),
            ("QuestionIds", "WOODWORKERS_GUILD_GAIN"),
            ("QuestionTypes", "TRASH"),
            ("QuestionTypes", "GAIN"),
            ("AbilityNames", "WOODWORKERS_GUILD"),
        ],
        _ => &[],
    }
}

const CARDS: &[&str] = &["card-selection"];
const PILES: &[&str] = &["card-selection", "pile-selection"];
const MODES: &[&str] = &["semantic-modes", "card-selection", "pile-selection"];
const ALLY: &[&str] = &["favors", "duration-ability", "optional-ability"];

/// Cards the survey runs clean and `projection::late_sets` covers.
const ADMITTED: &[(Card, &[&str])] = &[
    (
        Card::Sentinel,
        &["looking", "card-selection", "ordered-cards"],
    ),
    (
        Card::Carpenter,
        &["card-selection", "pile-selection", "explicit-cost-bound"],
    ),
    (Card::Courier, &["discard-inspection", "optional-play"]),
    (Card::Innkeeper, MODES),
    (
        Card::RoyalGalley,
        &[
            "optional-play",
            "repeated-play",
            "duration-ability",
            "duration-reminder",
        ],
    ),
    (Card::Town, MODES),
    (Card::Barbarian, &["off-turn-attack", "pile-selection"]),
    (Card::CapitalCity, &["semantic-modes", "card-selection"]),
    (Card::Hunter, &["looking", "card-selection"]),
    (Card::Marquis, CARDS),
    (
        Card::Modify,
        &[
            "semantic-modes",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
    ),
    (
        Card::Specialist,
        &["semantic-modes", "optional-play", "visible-reference"],
    ),
    (Card::Swap, PILES),
    (
        Card::Skirmisher,
        &["off-turn-attack", "card-selection", "public-effects"],
    ),
    (
        Card::Highwayman,
        &["duration-ability", "duration-reminder", "public-effects"],
    ),
    (Card::Galleria, &["public-effects"]),
    (Card::MerchantCamp, &["optional-ability"]),
    (Card::TownCrier, &["semantic-modes", "rotate", "split-pile"]),
    (Card::Blacksmith, &["semantic-modes", "split-pile"]),
    (Card::Miller, &["looking", "card-selection", "split-pile"]),
    (
        Card::Elder,
        &["optional-play", "semantic-modes", "split-pile"],
    ),
    (
        Card::HerbGatherer,
        &[
            "discard-inspection",
            "optional-play",
            "rotate",
            "split-pile",
        ],
    ),
    (
        Card::Acolyte,
        &["card-selection", "optional-effect", "split-pile"],
    ),
    (Card::Sorceress, &["name-card", "split-pile"]),
    (Card::Sibyl, &["card-selection", "split-pile"]),
    (
        Card::BattlePlan,
        &["card-selection", "pile-selection", "rotate", "split-pile"],
    ),
    (
        Card::Archer,
        &["off-turn-attack", "card-selection", "split-pile"],
    ),
    (
        Card::Warlord,
        &["duration-ability", "duration-reminder", "split-pile"],
    ),
    (Card::Territory, &["optional-ability", "split-pile"]),
    (Card::Tent, &["rotate", "optional-ability", "split-pile"]),
    (
        Card::Garrison,
        &["duration-ability", "duration-reminder", "split-pile"],
    ),
    (
        Card::HillFort,
        &["semantic-modes", "pile-selection", "split-pile"],
    ),
    (
        Card::Stronghold,
        &[
            "semantic-modes",
            "duration-ability",
            "duration-reminder",
            "split-pile",
        ],
    ),
    (Card::OldMap, &["card-selection", "rotate", "split-pile"]),
    (Card::SunkenTreasure, &["pile-selection", "split-pile"]),
    (Card::DistantShore, &["split-pile"]),
    (
        Card::Conjurer,
        &[
            "pile-selection",
            "duration-ability",
            "duration-reminder",
            "split-pile",
        ],
    ),
    (
        Card::Sorcerer,
        &["name-card", "off-turn-attack", "split-pile"],
    ),
    (
        Card::ArchitectsGuild,
        &["favors", "optional-ability", "pile-selection"],
    ),
    (
        Card::CaveDwellers,
        &["favors", "optional-ability", "card-selection"],
    ),
    (Card::CircleOfWitches, ALLY),
    (Card::CityState, ALLY),
    (
        Card::CoastalHaven,
        &["favors", "optional-ability", "set-aside"],
    ),
    (
        Card::CraftersGuild,
        &["favors", "optional-ability", "pile-selection"],
    ),
    (
        Card::DesertGuides,
        &["favors", "optional-ability", "semantic-modes"],
    ),
    (Card::FellowshipOfScribes, ALLY),
    (
        Card::ForestDwellers,
        &["favors", "optional-ability", "looking", "ordered-cards"],
    ),
    (
        Card::GangOfPickpockets,
        &["favors", "duration-ability", "card-selection"],
    ),
    (Card::IslandFolk, &["favors", "optional-ability"]),
    (Card::LeagueOfBankers, ALLY),
    (Card::LeagueOfShopkeepers, ALLY),
    (
        Card::MarketTowns,
        &["favors", "optional-ability", "optional-play"],
    ),
    (Card::MountainFolk, ALLY),
    (
        Card::OrderOfAstrologers,
        &[
            "favors",
            "optional-ability",
            "card-selection",
            "ordered-cards",
        ],
    ),
    (
        Card::OrderOfMasons,
        &["favors", "optional-ability", "card-selection"],
    ),
    (
        Card::PeacefulCult,
        &["favors", "optional-ability", "card-selection"],
    ),
    (Card::PlateauShepherds, &["favors"]),
    (Card::TrappersLodge, ALLY),
    (
        Card::WoodworkersGuild,
        &[
            "favors",
            "optional-ability",
            "card-selection",
            "pile-selection",
        ],
    ),
];

fn admitted(card: Card) -> Option<&'static [&'static str]> {
    ADMITTED
        .iter()
        .find(|(c, _)| *c == card)
        .map(|(_, features)| *features)
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    if card.set() != dominion_engine::Set::Allies {
        return None;
    }
    // A split pile is offered once each of its cards is.
    if card.kind() == dominion_engine::Kind::Randomizer {
        let mut members = Card::ALL.iter().filter(|c| c.def().pile == Some(card));
        return members
            .all(|&c| requirements(c).is_some())
            .then_some(&["split-pile"]);
    }
    // A Liaison brings a random Ally the setup check never sees: every Ally must be carried.
    if card.is(Types::LIAISON) {
        let allies = Card::ALL
            .iter()
            .filter(|c| c.kind() == dominion_engine::Kind::Ally);
        if !allies.clone().all(|&a| admitted(a).is_some()) {
            return None;
        }
        return Some(&[
            "favors",
            "card-selection",
            "pile-selection",
            "semantic-modes",
        ]);
    }
    admitted(card)
}
