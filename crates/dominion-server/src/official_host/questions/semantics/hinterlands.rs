//! Question bindings for this set's cards (see `semantics::SETS`).
use super::{
    AbilityArguments, Binding, ProjectionError, WireCatalog, amount, card_names, cost_limit, gap,
};
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
    let choosing = matches!(d.kind, DecisionKind::Choose { .. });
    let with_limit = |story| Some(cost_limit(view, d).map(|a| ("GAIN", story, vec![a])));
    let (kind, story) = match (d.source?, d.purpose) {
        (Berserker, P::Discard) => ("DISCARD", "BERSERKER_DISCARD"),
        (Berserker, P::Gain) => ("GAIN", "BERSERKER_GAIN"),
        (BorderVillage, P::Gain) => return with_limit("BORDER_VILLAGE"),
        (Cartographer, P::Discard) => ("DISCARD", "CARTOGRAPHER_DISCARD"),
        (Cartographer, P::TopDeck) if ordered => ("ORDER_CARDS", "CARTOGRAPHER_TOPDECK"),
        (Develop, P::Trash) => ("TRASH", "DEVELOP_TRASH"),
        (Develop, P::Gain) => return Some(develop(view, d)),
        (Duchess, P::Discard) if choosing => {
            return Some(about(view, d, c).map(|a| ("CHOOSE_MODE", "DUCHESS", vec![a])));
        }
        (Embassy, P::Discard) => ("DISCARD", "EMBASSY"),
        (Farmland, P::Trash) => ("TRASH", "FARMLAND_TRASH"),
        (Farmland, P::Gain) => return with_limit("FARMLAND_GAIN"),
        (Haggler, P::Gain) => return with_limit("HAGGLER"),
        (IllGottenGains, P::Gain) if choosing => ("GAIN", "ILL_GOTTEN_GAINS"),
        (Inn, P::Discard) => ("DISCARD", "INN_DISCARD"),
        (Inn, P::Move) => ("SHUFFLE_INTO_DECK", "INN_SHUFFLE"),
        (JackOfAllTrades, P::Discard) if choosing => {
            return Some(about(view, d, c).map(|a| ("CHOOSE_MODE", "JACK_DISCARD", vec![a])));
        }
        (JackOfAllTrades, P::Trash) => ("TRASH", "JACK_TRASH"),
        (Mandarin, P::TopDeck) if ordered => ("ORDER_CARDS", "MANDARIN_GAIN"),
        (Mandarin, P::TopDeck) => ("TOPDECK", "MANDARIN_PLAY"),
        (Margrave, P::Discard) => ("DISCARD", "MARGRAVE"),
        (NobleBrigand, P::Trash) => ("TRASH", "NOBLE_BRIGAND"),
        (Oasis, P::Discard) => ("DISCARD", "OASIS"),
        (Oracle, P::Discard) if choosing => return Some(oracle(view, d, c)),
        (Oracle, P::TopDeck) if ordered => ("ORDER_CARDS", "ORACLE_TOPDECK"),
        (Scheme, P::TopDeck) => ("TOPDECK", "SCHEME"),
        (Souk, P::Trash) => ("TRASH", "SOUK"),
        (SpiceMerchant, P::Trash) => ("TRASH", "SPICE_MERCHANT_TRASH"),
        (SpiceMerchant, P::Mode) => ("CHOOSE_MODE", "SPICE_MERCHANT_MODE"),
        (Stables, P::Discard) => ("DISCARD", "STABLES"),
        (Trader, P::Trash) => ("TRASH", "TRADER_TRASH"),
        (Weaver, P::Either) => ("CHOOSE_MODE", "WEAVER"),
        // "Gain a card costing up to $4.", word for word.
        (Weaver, P::Gain) => ("GAIN", "WORKSHOP"),
        (Wheelwright, P::Discard) => ("DISCARD", "WHEELWRIGHT_DISCARD"),
        (Wheelwright, P::Gain) => return with_limit("WHEELWRIGHT_GAIN"),
        (WitchsHut, P::Discard) => ("DISCARD", "WITCHS_HUT"),
        _ => return None,
    };
    Some(Ok((kind, story, vec![])))
}

/// The first gain names both costs left ($1 more, $1 less) when both are offered.
fn develop(view: &View<'_>, d: &Decision) -> Result<Binding, ProjectionError> {
    let DecisionKind::Pile { piles, .. } = &d.kind else {
        return Err(gap(d, "Develop gain shape"));
    };
    let mut costs = piles
        .iter()
        .map(|&p| view.pile(p).top().and_then(|id| view.cost_of(id)))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| gap(d, "Develop pile cost"))?;
    costs.sort_by_key(|c| (c.coins, c.potions, c.debt));
    costs.dedup();
    Ok(match costs.as_slice() {
        [cost] => ("GAIN", "DEVELOP_GAIN2", vec![amount(*cost)]),
        [down, up] => ("GAIN", "DEVELOP_GAIN1", vec![amount(*up), amount(*down)]),
        _ => return Err(gap(d, "Develop gain costs")),
    })
}

/// "You may discard CARDS0 from PLAYER1."
fn oracle(view: &View<'_>, d: &Decision, c: &WireCatalog) -> Result<Binding, ProjectionError> {
    let whose = view
        .pending_subject()
        .ok_or_else(|| gap(d, "missing revealing player"))?;
    let cards = view
        .player(whose)
        .looking()
        .into_iter()
        .map(|id| view.card(id))
        .collect::<Option<Vec<_>>>()
        .filter(|cards| !cards.is_empty())
        .ok_or_else(|| gap(d, "missing revealed cards"))?;
    Ok((
        "CHOOSE_MODE",
        "ORACLE_DISCARD",
        vec![card_names(c, cards)?, LogArgument::Player(whose.0.into())],
    ))
}

/// The one card the question is about, as CARDS0.
fn about(view: &View<'_>, d: &Decision, c: &WireCatalog) -> Result<LogArgument, ProjectionError> {
    let card = d
        .about
        .first()
        .and_then(|&id| view.card(id))
        .ok_or_else(|| gap(d, "missing card the question is about"))?;
    card_names(c, [card])
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    let choices: &[&str] = match card {
        Card::SpiceMerchant => &["SPICE_MERCHANT_CARDS", "SPICE_MERCHANT_COINS"],
        // Their client has no Weaver mode for the gain; Modify's reads "Gain a card".
        Card::Weaver => &["WEAVER_SILVERS", "MODIFY_GAIN"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (card, ability) {
        (Berserker, 0) => "BERSERKER",
        (BorderVillage, 0) => "GAIN_CHEAPER",
        (Cache, 0) => "GAIN_BY_NAME",
        (Cauldron, 0) => "CAULDRON_CURSING",
        (Duchess, 0) => "DUCHESS_MAY_GAIN",
        (Farmland, 0) => "FARMLAND",
        (FoolsGold, 0) => "FOOLS_GOLD_TRASH",
        (GuardDog, 0) => "GUARD_DOG_REACTION",
        (Haggler, 0) => "HAGGLER_ABILITY",
        (IllGottenGains, 0) => "CAULDRON_CURSING",
        (Inn, 0) => "INN_ABILITY",
        (Mandarin, 0) => "MANDARIN",
        (NobleBrigand, 0) => "NOBLE_BRIGAND",
        (NomadCamp, 0) => "PROGRESS",
        (Nomads, 0) => "NOMADS",
        (Scheme, 0) => "SCHEME_ASK_FOR_DISCARD",
        (Souk, 0) => "SOUK",
        (Trader, 0) => "TRADER_REACTION",
        (Trail, 1) => "TRAIL_REACTION",
        (Tunnel, 0) => "TUNNEL_REVEAL",
        (Weaver, 1) => "WEAVER_PLAY",
        _ => return None,
    })
}

pub(super) fn option_mode(card: Card, opt: Opt) -> Option<&'static str> {
    Some(match (card, opt) {
        (Card::Duchess, Opt::Yes) => "DUCHESS_DISCARD",
        (Card::Duchess, Opt::No) => "DUCHESS_KEEP",
        (Card::JackOfAllTrades, Opt::Yes) => "JACK_DISCARD",
        (Card::JackOfAllTrades, Opt::No) => "JACK_KEEP",
        (Card::Oracle, Opt::Yes) => "ORACLE_DISCARD",
        (Card::Oracle, Opt::No) => "ORACLE_TOPDECK",
        _ => return None,
    })
}

pub(super) fn option_button(_card: Card, _opt: Opt) -> Option<(&'static str, &'static str)> {
    None
}

pub(super) fn option_card(view: &View<'_>, d: &Decision, opt: Opt) -> Option<CardId> {
    match (d.source?, opt) {
        (Card::IllGottenGains, Opt::Yes) => view.pile(view.supply_pile(Card::Copper)?).top(),
        _ => None,
    }
}

pub(super) fn ability_arguments(
    view: &View<'_>,
    t: &Trigger,
    occ: Option<&Occasion>,
    c: &WireCatalog,
) -> AbilityArguments {
    match (t.card, t.ability, occ) {
        (Card::Trader, 0, Some(Occasion::Gain { card, .. })) => {
            Some(super::ability_card(view, t, Some(*card), c))
        }
        (Card::NomadCamp, 0, _) => Some(super::ability_card(view, t, t.source, c)),
        // "gain 2 Coppers (Cache)"
        (Card::Cache, 0, _) => Some(
            card_names(c, [Card::Copper, Card::Copper])
                .and_then(|coppers| Ok(vec![coppers, card_names(c, [Card::Cache])?])),
        ),
        _ => None,
    }
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Berserker => &[
            ("AbilityNames", "BERSERKER"),
            ("QuestionIds", "BERSERKER_DISCARD"),
            ("QuestionIds", "BERSERKER_GAIN"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "GAIN"),
        ],
        BorderVillage => &[
            ("AbilityNames", "GAIN_CHEAPER"),
            ("QuestionIds", "BORDER_VILLAGE"),
            ("QuestionTypes", "GAIN"),
        ],
        Cartographer => &[
            ("QuestionIds", "CARTOGRAPHER_DISCARD"),
            ("QuestionIds", "CARTOGRAPHER_TOPDECK"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "ORDER_CARDS"),
        ],
        Cache => &[("AbilityNames", "GAIN_BY_NAME")],
        Cauldron => &[("AbilityNames", "CAULDRON_CURSING")],
        Develop => &[
            ("QuestionIds", "DEVELOP_GAIN1"),
            ("QuestionIds", "DEVELOP_GAIN2"),
            ("QuestionIds", "DEVELOP_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Duchess => &[
            ("AbilityNames", "DUCHESS_MAY_GAIN"),
            ("CardModes", "DUCHESS_DISCARD"),
            ("CardModes", "DUCHESS_KEEP"),
            ("QuestionIds", "DUCHESS"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Embassy => &[("QuestionIds", "EMBASSY"), ("QuestionTypes", "DISCARD")],
        Farmland => &[
            ("AbilityNames", "FARMLAND"),
            ("QuestionIds", "FARMLAND_GAIN"),
            ("QuestionIds", "FARMLAND_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        FoolsGold => &[("AbilityNames", "FOOLS_GOLD_TRASH")],
        GuardDog => &[("AbilityNames", "GUARD_DOG_REACTION")],
        Haggler => &[
            ("AbilityNames", "HAGGLER_ABILITY"),
            ("QuestionIds", "HAGGLER"),
            ("QuestionTypes", "GAIN"),
        ],
        IllGottenGains => &[
            ("AbilityNames", "CAULDRON_CURSING"),
            ("QuestionIds", "ILL_GOTTEN_GAINS"),
            ("QuestionTypes", "GAIN"),
        ],
        Inn => &[
            ("AbilityNames", "INN_ABILITY"),
            ("QuestionIds", "INN_DISCARD"),
            ("QuestionIds", "INN_SHUFFLE"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "SHUFFLE_INTO_DECK"),
        ],
        JackOfAllTrades => &[
            ("CardModes", "JACK_DISCARD"),
            ("CardModes", "JACK_KEEP"),
            ("QuestionIds", "JACK_DISCARD"),
            ("QuestionIds", "JACK_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "TRASH"),
        ],
        Mandarin => &[
            ("AbilityNames", "MANDARIN"),
            ("QuestionIds", "MANDARIN_GAIN"),
            ("QuestionIds", "MANDARIN_PLAY"),
            ("QuestionTypes", "ORDER_CARDS"),
            ("QuestionTypes", "TOPDECK"),
        ],
        Margrave => &[("QuestionIds", "MARGRAVE"), ("QuestionTypes", "DISCARD")],
        NobleBrigand => &[
            ("AbilityNames", "NOBLE_BRIGAND"),
            ("QuestionIds", "NOBLE_BRIGAND"),
            ("QuestionTypes", "TRASH"),
        ],
        NomadCamp => &[("AbilityNames", "PROGRESS")],
        Nomads => &[("AbilityNames", "NOMADS")],
        Oasis => &[("QuestionIds", "OASIS"), ("QuestionTypes", "DISCARD")],
        Oracle => &[
            ("CardModes", "ORACLE_DISCARD"),
            ("CardModes", "ORACLE_TOPDECK"),
            ("QuestionIds", "ORACLE_DISCARD"),
            ("QuestionIds", "ORACLE_TOPDECK"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "ORDER_CARDS"),
        ],
        Scheme => &[
            ("AbilityNames", "SCHEME_ASK_FOR_DISCARD"),
            ("QuestionIds", "SCHEME"),
            ("QuestionTypes", "TOPDECK"),
        ],
        Souk => &[
            ("AbilityNames", "SOUK"),
            ("QuestionIds", "SOUK"),
            ("QuestionTypes", "TRASH"),
        ],
        SpiceMerchant => &[
            ("CardModes", "SPICE_MERCHANT_CARDS"),
            ("CardModes", "SPICE_MERCHANT_COINS"),
            ("QuestionIds", "SPICE_MERCHANT_MODE"),
            ("QuestionIds", "SPICE_MERCHANT_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "TRASH"),
        ],
        Stables => &[("QuestionIds", "STABLES"), ("QuestionTypes", "DISCARD")],
        Trader => &[
            ("AbilityNames", "TRADER_REACTION"),
            ("QuestionIds", "TRADER_TRASH"),
            ("QuestionTypes", "TRASH"),
        ],
        Trail => &[("AbilityNames", "TRAIL_REACTION")],
        Tunnel => &[("AbilityNames", "TUNNEL_REVEAL")],
        Weaver => &[
            ("AbilityNames", "WEAVER_PLAY"),
            ("CardModes", "MODIFY_GAIN"),
            ("CardModes", "WEAVER_SILVERS"),
            ("QuestionIds", "WEAVER"),
            ("QuestionIds", "WORKSHOP"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
        ],
        Wheelwright => &[
            ("QuestionIds", "WHEELWRIGHT_DISCARD"),
            ("QuestionIds", "WHEELWRIGHT_GAIN"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "GAIN"),
        ],
        WitchsHut => &[("QuestionIds", "WITCHS_HUT"), ("QuestionTypes", "DISCARD")],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        Berserker => &["off-turn-attack", "card-selection", "pile-selection"],
        BorderVillage | Haggler => &["pile-selection", "explicit-cost-bound"],
        Cache | Crossroads | NomadCamp | Nomads | SilkRoad => &[],
        Tunnel => &["reaction-ability"],
        Cartographer => &["looking", "card-selection", "ordered-cards"],
        Cauldron => &["off-turn-attack", "passive-lasting"],
        Develop => &["card-selection", "pile-selection", "explicit-cost-bound"],
        Duchess => &["looking", "semantic-modes", "optional-ability"],
        Embassy | Oasis | Stables | WitchsHut => &["card-selection"],
        Farmland | Wheelwright => &["card-selection", "pile-selection", "explicit-cost-bound"],
        FoolsGold | GuardDog | Trader => &["reaction-ability", "card-selection"],
        Highway => &["dynamic-cost"],
        IllGottenGains => &["optional-effect"],
        Inn => &["card-selection", "discard-inspection"],
        JackOfAllTrades => &["looking", "semantic-modes", "card-selection"],
        Margrave | NobleBrigand => &["off-turn-attack", "card-selection"],
        Oracle => &["off-turn-attack", "semantic-modes", "ordered-cards"],
        Scheme => &["optional-ability", "card-selection"],
        Souk | SpiceMerchant => &["card-selection", "semantic-modes"],
        Trail | Weaver => &["reaction-ability", "semantic-modes", "pile-selection"],
        _ => return None,
    })
}
