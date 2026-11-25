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
    let choosing = matches!(d.kind, DecisionKind::Choose { .. });
    let (kind, story) = match (d.source?, d.purpose) {
        (Apothecary, P::TopDeck) if ordered => ("ORDER_CARDS", "APOTHECARY"),
        (Apprentice, P::Trash) => ("TRASH", "APPRENTICE"),
        (Golem, P::Play) => ("PLAY", "GOLEM"),
        (Herbalist, P::TopDeck) => ("TOPDECK", "HERBALIST"),
        (ScryingPool, P::Discard) if choosing => return Some(scrying_pool(view, d, c)),
        (Transmute, P::Trash) => ("TRASH", "TRANSMUTE"),
        (University, P::Gain) => ("GAIN", "UNIVERSITY"),
        _ => return None,
    };
    Some(Ok((kind, story, vec![])))
}

/// Keep or discard the revealed card: the player's own, or whose it is.
fn scrying_pool(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Result<Binding, ProjectionError> {
    let card = view
        .pending_reference()
        .and_then(|id| view.card(id))
        .ok_or_else(|| gap(d, "missing revealed card"))?;
    let whose = view
        .pending_subject()
        .ok_or_else(|| gap(d, "missing revealing player"))?;
    let cards = card_names(c, [card])?;
    Ok(if whose == d.player {
        ("CHOOSE_MODE", "SCRYING_POOL_YOU", vec![cards])
    } else {
        (
            "CHOOSE_MODE",
            "SCRYING_POOL_OTHER",
            vec![cards, LogArgument::Player(whose.0.into())],
        )
    })
}

pub(super) fn mode(_card: Card, _mode: u8) -> Option<&'static str> {
    None
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    match (card, ability) {
        (Card::Herbalist, 0) => Some("HERBALIST_ASK_FOR_DISCARD"),
        _ => None,
    }
}

pub(super) fn option_mode(card: Card, opt: Opt) -> Option<&'static str> {
    match (card, opt) {
        (Card::ScryingPool, Opt::Yes) => Some("SCRYING_POOL_DISCARD"),
        (Card::ScryingPool, Opt::No) => Some("SCRYING_POOL_TOPDECK"),
        _ => None,
    }
}

pub(super) fn option_button(_card: Card, _opt: Opt) -> Option<(&'static str, &'static str)> {
    None
}

pub(super) fn option_card(_view: &View<'_>, _d: &Decision, _opt: Opt) -> Option<CardId> {
    None
}

pub(super) fn ability_arguments(
    _view: &View<'_>,
    _t: &Trigger,
    _occ: Option<&Occasion>,
    _c: &WireCatalog,
) -> AbilityArguments {
    None
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Apothecary => &[
            ("QuestionIds", "APOTHECARY"),
            ("QuestionTypes", "ORDER_CARDS"),
        ],
        Apprentice => &[("QuestionIds", "APPRENTICE"), ("QuestionTypes", "TRASH")],
        Golem => &[("QuestionIds", "GOLEM"), ("QuestionTypes", "PLAY")],
        Herbalist => &[
            ("AbilityNames", "HERBALIST_ASK_FOR_DISCARD"),
            ("QuestionIds", "HERBALIST"),
            ("QuestionTypes", "TOPDECK"),
        ],
        ScryingPool => &[
            ("CardModes", "SCRYING_POOL_DISCARD"),
            ("CardModes", "SCRYING_POOL_TOPDECK"),
            ("QuestionIds", "SCRYING_POOL_OTHER"),
            ("QuestionIds", "SCRYING_POOL_YOU"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Transmute => &[("QuestionIds", "TRANSMUTE"), ("QuestionTypes", "TRASH")],
        University => &[("QuestionIds", "UNIVERSITY"), ("QuestionTypes", "GAIN")],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        Apothecary => &["potion-cost", "looking", "ordered-cards"],
        Apprentice => &["card-selection"],
        Transmute => &["potion-cost", "card-selection"],
        Familiar => &["potion-cost", "off-turn-attack"],
        Golem => &["potion-cost", "card-selection"],
        Herbalist => &["optional-ability", "card-selection"],
        PhilosophersStone | Vineyard => &["potion-cost"],
        ScryingPool => &["potion-cost", "off-turn-attack", "semantic-modes"],
        University => &["potion-cost", "pile-selection"],
        _ => return None,
    })
}
