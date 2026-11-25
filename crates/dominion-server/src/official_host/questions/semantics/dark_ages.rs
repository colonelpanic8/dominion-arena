//! Question bindings for Dark Ages (see `semantics::SETS`).
use super::{Binding, Choice, ProjectionError, SetExtras, WireCatalog, card_names, cost_bound};
use dominion_engine::{Card, Decision, DecisionKind, Occasion, Opt, Purpose, Trigger, View};
use dominion_online::questions::LogArgument;

pub(super) const EXTRAS: SetExtras = SetExtras {
    choice,
    ability_arguments,
    decline,
};

fn knight(card: Card) -> bool {
    card.def().pile == Some(Card::Knights)
}

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let step = view.pending_step();
    let source = d.source?;
    let (kind, story) = match (source, d.purpose) {
        (Altar, P::Trash) => ("TRASH", "ALTAR_TRASH"),
        (Altar, P::Gain) => ("GAIN", "ALTAR_GAIN"),
        (Armory, P::Gain) => ("GAIN", "ARMORY"),
        (BandOfMisfits, P::Play) => ("IMPERSONATE", "BAND_OF_MISFITS"),
        (Catacombs, P::Mode) => ("CHOOSE_MODE", "CATACOMBS_MODE"),
        (Catacombs, P::Gain) => ("GAIN", "CATACOMBS_GAIN"),
        (Count, P::Mode) if step == Some(1) => ("CHOOSE_MODE", "COUNT_PAY"),
        (Count, P::Mode) => ("CHOOSE_MODE", "COUNT_PROFIT"),
        (Count, P::Discard) => ("DISCARD", "COUNT_DISCARD"),
        (Count, P::TopDeck) => ("TOPDECK", "COUNT_TOPDECK"),
        (Counterfeit, P::Play) => ("PLAY", "COUNTERFEIT"),
        (Cultist, P::Play) => ("PLAY", "CULTIST"),
        (DeathCart, P::Trash) => ("TRASH", "DEATH_CART"),
        (Forager, P::Trash) => ("TRASH", "FORAGER"),
        (Graverobber, P::Mode) => ("CHOOSE_MODE", "GRAVEROBBER_MODE"),
        (Graverobber, P::Gain) if matches!(d.kind, DecisionKind::Cards { .. }) => {
            ("GAIN", "GRAVEROBBER_GAIN")
        }
        (Graverobber, P::Gain) => ("GAIN", "GRAVEROBBER_GAIN_SUPPLY"),
        (Graverobber, P::Trash) => ("TRASH", "GRAVEROBBER_EXPAND"),
        (Hermit, P::Trash) => ("TRASH", "HERMIT_TRASH"),
        (Hermit, P::Gain) => ("GAIN", "HERMIT_GAIN"),
        (HuntingGrounds, P::Either) => ("CHOOSE_MODE", "HUNTING_GROUNDS"),
        (Ironmonger, P::Discard) => ("CHOOSE_MODE", "IRONMONGER"),
        (JunkDealer, P::Trash) => ("TRASH", "JUNK_DEALER"),
        (Mercenary, P::Trash) if matches!(d.kind, DecisionKind::Choose { .. }) => {
            ("COMPLEX_OR", "MERCENARY_MULTIPLE")
        }
        (Mercenary, P::Trash) => ("TRASH", "MERCENARY_MULTIPLE"),
        (Mercenary, P::Discard) => ("DISCARD", "MERCENARY_DISCARD"),
        (Mystic, P::Name) => ("WISH", "MYSTIC"),
        (Pillage, P::Discard) => ("DISCARD", "PILLAGE"),
        (Procession, P::Play) => ("PLAY", "PROCESSION_PLAY"),
        (Procession, P::Gain) => ("GAIN", "PROCESSION_GAIN"),
        (Rats, P::Trash) => ("TRASH", "RATS_TRASH_OTHER"),
        (Rebuild, P::Name) => ("WISH", "REBUILD_SKIP"),
        (Rebuild, P::Gain) => ("GAIN", "REBUILD_GAIN"),
        (Rogue, P::Gain) => ("GAIN", "ROGUE_GAIN"),
        (Rogue, P::Trash) => ("TRASH", "ROGUE_TRASH"),
        (Scavenger, P::Discard) => ("CHOOSE_MODE", "SCAVENGER"),
        (Scavenger, P::TopDeck) => ("TOPDECK", "SCAVENGER_TOPDECK"),
        (Squire, P::Mode) => ("CHOOSE_MODE", "SQUIRE_MODE"),
        (Squire, P::Gain) => ("GAIN", "SQUIRE_GAIN"),
        (Storeroom, P::Discard) if step == Some(1) => ("DISCARD", "STOREROOM_CARDS"),
        (Storeroom, P::Discard) => ("DISCARD", "STOREROOM_COINS"),
        (Survivors, P::Either) => ("COMPLEX_OR", "SURVIVORS"),
        (Survivors, P::TopDeck) if ordered => ("ORDER_CARDS", "SURVIVORS"),
        (Urchin, P::Discard) => ("DISCARD", "URCHIN"),
        (WanderingMinstrel, P::TopDeck) if ordered => ("ORDER_CARDS", "WANDERING_MINSTREL"),
        (DameAnna, P::Trash) if step == Some(1) => ("TRASH", "DAME_ANNA"),
        (DameNatalie, P::Gain) => ("GAIN", "DAME_NATALIE"),
        (SirMichael, P::Discard) => ("DISCARD", "SIR_MICHAEL"),
        (k, P::Trash) if knight(k) => ("TRASH", "KNIGHTS"),
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
        "CATACOMBS_GAIN" | "GRAVEROBBER_GAIN_SUPPLY" | "PROCESSION_GAIN" | "REBUILD_GAIN" => {
            vec![cost_bound(view, d)?]
        }
        "CATACOMBS_MODE" => vec![card_names(view, d, c, d.about.iter().copied())?],
        "IRONMONGER" => {
            let revealed = view
                .pending_reference()
                .ok_or_else(|| super::gap(d, "missing revealed card"))?;
            vec![card_names(view, d, c, [revealed])?]
        }
        "PILLAGE" | "ROGUE_TRASH" => vec![super::subject(view, d)?],
        _ => vec![],
    })
}

fn choice(view: &View<'_>, d: &Decision) -> Option<Choice> {
    let DecisionKind::Choose { options, .. } = &d.kind else {
        return None;
    };
    if d.source == Some(Card::Count) {
        // Count's two "choose one"s: which one is asking is its step.
        let modes: &[&str] = if view.pending_step() == Some(1) {
            &["COUNT_DISCARD", "COUNT_TOPDECK", "COUNT_GAIN_COPPER"]
        } else {
            &["COUNT_COINS", "COUNT_TRASH_HAND", "COUNT_GAIN_DUCHY"]
        };
        return options
            .iter()
            .map(|o| match o {
                Opt::Mode(m) => modes.get(usize::from(*m)).copied(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(Choice::Modes);
    }
    let yes_no =
        |yes, no| (options.as_slice() == [Opt::Yes, Opt::No]).then(|| Choice::Modes(vec![yes, no]));
    match d.source? {
        Card::Ironmonger => yes_no("IRONMONGER_DISCARD", "IRONMONGER_KEEP"),
        Card::Scavenger => yes_no("SCAVENGER_DISCARD", "SCAVENGER_KEEP"),
        Card::Mercenary if options.as_slice() == [Opt::Yes, Opt::No] => {
            Some(Choice::Buttons(vec!["CONTINUE", "DONT_TRASH"]))
        }
        Card::Survivors if options.as_slice() == [Opt::Mode(0), Opt::Mode(1)] => {
            Some(Choice::Buttons(vec!["DISCARD_ALL", "CONTINUE"]))
        }
        _ => None,
    }
}

fn decline(d: &Decision) -> Option<&'static str> {
    Some(match d.source? {
        Card::Counterfeit => "COUNTERFEIT_NO_TREASURE",
        Card::Cultist => "CULTIST_DONT_PLAY_ANOTHER",
        Card::Procession => "PROCESSION_DONT",
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        Catacombs => &["CATACOMBS_TAKE", "CATACOMBS_DISCARD"],
        Graverobber => &["GRAVEROBBER_GAIN", "GRAVEROBBER_TRASH"],
        HuntingGrounds => &["HUNTING_GROUNDS_DUCHY", "HUNTING_GROUNDS_ESTATES"],
        Squire => &["SQUIRE_ACTIONS", "SQUIRE_BUYS", "SQUIRE_SILVER"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    if ability != 0 {
        return None;
    }
    Some(match card {
        Cultist | Rats | OvergrownEstate => "DRAW_INSTRUCTIONS",
        Fortress => "FORTRESS",
        Squire => "SQUIRE_GAIN_ATTACK",
        Catacombs => "GAIN_CHEAPER",
        HuntingGrounds => "HUNTING_GROUNDS",
        Feodum => "FEODUM_GAIN_SILVERS",
        MarketSquare => "MARKET_SQUARE_REACTION",
        Hovel => "HOVEL_TRASH",
        Beggar => "BEGGAR_REACTION",
        Urchin => "URCHIN_TRANSFORM",
        DeathCart => "GAIN_TWO_RUINS",
        Hermit => "HERMIT",
        _ => return None,
    })
}

fn ability_arguments(
    _view: &View<'_>,
    t: &Trigger,
    _occasion: Option<&Occasion>,
    _c: &WireCatalog,
) -> Option<Result<Vec<LogArgument>, ProjectionError>> {
    let cards = match t.card {
        Card::Cultist => 3,
        Card::Rats | Card::OvergrownEstate => 1,
        _ => return None,
    };
    Some(Ok(vec![LogArgument::Number(cards)]))
}

macro_rules! symbols {
    ($($table:ident . $symbol:ident),* $(,)?) => {
        &[$((stringify!($table), stringify!($symbol))),*]
    };
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Altar => symbols![
            QuestionIds.ALTAR_TRASH,
            QuestionIds.ALTAR_GAIN,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
        ],
        Armory => symbols![QuestionIds.ARMORY, QuestionTypes.GAIN],
        BandOfMisfits => symbols![QuestionIds.BAND_OF_MISFITS, QuestionTypes.IMPERSONATE],
        Beggar => symbols![AbilityNames.BEGGAR_REACTION],
        Catacombs => symbols![
            QuestionIds.CATACOMBS_MODE,
            QuestionIds.CATACOMBS_GAIN,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.GAIN,
            CardModes.CATACOMBS_TAKE,
            CardModes.CATACOMBS_DISCARD,
            AbilityNames.GAIN_CHEAPER,
        ],
        Count => symbols![
            QuestionIds.COUNT_PAY,
            QuestionIds.COUNT_PROFIT,
            QuestionIds.COUNT_DISCARD,
            QuestionIds.COUNT_TOPDECK,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.DISCARD,
            QuestionTypes.TOPDECK,
            CardModes.COUNT_DISCARD,
            CardModes.COUNT_TOPDECK,
            CardModes.COUNT_GAIN_COPPER,
            CardModes.COUNT_COINS,
            CardModes.COUNT_TRASH_HAND,
            CardModes.COUNT_GAIN_DUCHY,
        ],
        Counterfeit => symbols![
            QuestionIds.COUNTERFEIT,
            QuestionTypes.PLAY,
            DeclineButtonIds.COUNTERFEIT_NO_TREASURE,
        ],
        Cultist => symbols![
            QuestionIds.CULTIST,
            QuestionTypes.PLAY,
            DeclineButtonIds.CULTIST_DONT_PLAY_ANOTHER,
            AbilityNames.DRAW_INSTRUCTIONS,
        ],
        DeathCart => symbols![
            QuestionIds.DEATH_CART,
            QuestionTypes.TRASH,
            AbilityNames.GAIN_TWO_RUINS,
        ],
        Feodum => symbols![AbilityNames.FEODUM_GAIN_SILVERS],
        Forager => symbols![QuestionIds.FORAGER, QuestionTypes.TRASH],
        Fortress => symbols![AbilityNames.FORTRESS],
        Graverobber => symbols![
            QuestionIds.GRAVEROBBER_MODE,
            QuestionIds.GRAVEROBBER_GAIN,
            QuestionIds.GRAVEROBBER_GAIN_SUPPLY,
            QuestionIds.GRAVEROBBER_EXPAND,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.GAIN,
            QuestionTypes.TRASH,
            CardModes.GRAVEROBBER_GAIN,
            CardModes.GRAVEROBBER_TRASH,
        ],
        Hermit => symbols![
            QuestionIds.HERMIT_TRASH,
            QuestionIds.HERMIT_GAIN,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
            AbilityNames.HERMIT,
        ],
        HuntingGrounds => symbols![
            QuestionIds.HUNTING_GROUNDS,
            QuestionTypes.CHOOSE_MODE,
            CardModes.HUNTING_GROUNDS_DUCHY,
            CardModes.HUNTING_GROUNDS_ESTATES,
            AbilityNames.HUNTING_GROUNDS,
        ],
        Ironmonger => symbols![
            QuestionIds.IRONMONGER,
            QuestionTypes.CHOOSE_MODE,
            CardModes.IRONMONGER_DISCARD,
            CardModes.IRONMONGER_KEEP,
        ],
        JunkDealer => symbols![QuestionIds.JUNK_DEALER, QuestionTypes.TRASH],
        MarketSquare => symbols![AbilityNames.MARKET_SQUARE_REACTION],
        Mercenary => symbols![
            QuestionIds.MERCENARY_MULTIPLE,
            QuestionIds.MERCENARY_DISCARD,
            QuestionTypes.COMPLEX_OR,
            QuestionTypes.BUTTON,
            QuestionTypes.TRASH,
            QuestionTypes.DISCARD,
            DeclineButtonIds.CONTINUE,
            DeclineButtonIds.DONT_TRASH,
        ],
        Mystic => symbols![QuestionIds.MYSTIC, QuestionTypes.WISH],
        Pillage => symbols![QuestionIds.PILLAGE, QuestionTypes.DISCARD],
        Procession => symbols![
            QuestionIds.PROCESSION_PLAY,
            QuestionIds.PROCESSION_GAIN,
            QuestionTypes.PLAY,
            QuestionTypes.GAIN,
            DeclineButtonIds.PROCESSION_DONT,
        ],
        Rats => symbols![
            QuestionIds.RATS_TRASH_OTHER,
            QuestionTypes.TRASH,
            AbilityNames.DRAW_INSTRUCTIONS,
        ],
        Rebuild => symbols![
            QuestionIds.REBUILD_SKIP,
            QuestionIds.REBUILD_GAIN,
            QuestionTypes.WISH,
            QuestionTypes.GAIN,
        ],
        Rogue => symbols![
            QuestionIds.ROGUE_GAIN,
            QuestionIds.ROGUE_TRASH,
            QuestionTypes.GAIN,
            QuestionTypes.TRASH,
        ],
        Scavenger => symbols![
            QuestionIds.SCAVENGER,
            QuestionIds.SCAVENGER_TOPDECK,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.TOPDECK,
            CardModes.SCAVENGER_DISCARD,
            CardModes.SCAVENGER_KEEP,
        ],
        Squire => symbols![
            QuestionIds.SQUIRE_MODE,
            QuestionIds.SQUIRE_GAIN,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.GAIN,
            CardModes.SQUIRE_ACTIONS,
            CardModes.SQUIRE_BUYS,
            CardModes.SQUIRE_SILVER,
            AbilityNames.SQUIRE_GAIN_ATTACK,
        ],
        Storeroom => symbols![
            QuestionIds.STOREROOM_CARDS,
            QuestionIds.STOREROOM_COINS,
            QuestionTypes.DISCARD,
        ],
        Survivors => symbols![
            QuestionIds.SURVIVORS,
            QuestionTypes.COMPLEX_OR,
            QuestionTypes.BUTTON,
            QuestionTypes.ORDER_CARDS,
            DeclineButtonIds.DISCARD_ALL,
            DeclineButtonIds.CONTINUE,
        ],
        Urchin => symbols![
            QuestionIds.URCHIN,
            QuestionTypes.DISCARD,
            AbilityNames.URCHIN_TRANSFORM,
        ],
        WanderingMinstrel => symbols![QuestionIds.WANDERING_MINSTREL, QuestionTypes.ORDER_CARDS,],
        DameAnna => symbols![
            QuestionIds.DAME_ANNA,
            QuestionIds.KNIGHTS,
            QuestionTypes.TRASH,
        ],
        DameNatalie => symbols![
            QuestionIds.DAME_NATALIE,
            QuestionIds.KNIGHTS,
            QuestionTypes.GAIN,
            QuestionTypes.TRASH,
        ],
        SirMichael => symbols![
            QuestionIds.SIR_MICHAEL,
            QuestionIds.KNIGHTS,
            QuestionTypes.DISCARD,
            QuestionTypes.TRASH,
        ],
        k if knight(k) => symbols![QuestionIds.KNIGHTS, QuestionTypes.TRASH],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        // Pillage: the attacker picks from another player's revealed hand, which the
        // projection can't place yet ("candidate zone association").
        Pillage => return None,
        BanditCamp | Feodum | Fortress | Madman | Marauder | PoorHouse | Sage | Vagrant | Ruins
        | Spoils | OvergrownEstate | Necropolis => &[],
        Altar | Hermit => &["card-selection", "pile-selection", "discard-inspection"],
        Armory | BandOfMisfits => &["pile-selection"],
        Beggar | MarketSquare | Hovel => &["reaction-ability"],
        Catacombs => &[
            "looking",
            "semantic-modes",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Count => &["semantic-modes", "card-selection"],
        Counterfeit | Cultist => &["optional-play", "repeated-play", "card-selection"],
        DeathCart | Forager | JunkDealer | Rats | Storeroom => &["card-selection"],
        Graverobber => &[
            "semantic-modes",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        HuntingGrounds | Squire => &["semantic-modes", "pile-selection"],
        Ironmonger => &["semantic-modes", "visible-reference"],
        Mercenary => &["optional-effect", "off-turn-attack", "card-selection"],
        Mystic => &["name-card"],
        Procession => &[
            "optional-play",
            "repeated-play",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Rebuild => &["name-card", "pile-selection", "explicit-cost-bound"],
        Rogue => &["off-turn-attack", "card-selection"],
        Scavenger => &["semantic-modes", "discard-inspection", "card-selection"],
        Survivors | WanderingMinstrel => &["looking", "ordered-cards", "optional-effect"],
        Urchin => &["off-turn-attack", "card-selection", "optional-ability"],
        Knights => &["off-turn-attack", "card-selection", "pile-selection"],
        k if knight(k) => &["off-turn-attack", "card-selection", "pile-selection"],
        _ => return None,
    })
}
