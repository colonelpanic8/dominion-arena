//! Question bindings for this set's cards (see `semantics::SETS`).
use super::{
    AbilityArguments, Binding, ProjectionError, WireCatalog, ability_card, cost_limit, names_of,
};
use dominion_engine::{Card, Decision, Purpose, Trigger, View};

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    _c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let (kind, story) = match (d.source?, d.purpose) {
        (Captain, P::Play) => ("PLAY", "CAPTAIN"),
        (Church, P::SetAside) => ("SET_ASIDE", "CHURCH_SET_ASIDE"),
        (Church, P::Trash) => ("TRASH", "CHURCH_TRASH"),
        (Dismantle, P::Trash) => ("TRASH", "DISMANTLE_TRASH"),
        (Dismantle, P::Gain) => {
            return Some(cost_limit(view, d).map(|a| ("GAIN", "DISMANTLE_GAIN", vec![a])));
        }
        (Envoy, P::Discard) => ("DISCARD", "ENVOY"),
        (Governor, P::Mode) => ("CHOOSE_MODE", "GOVERNOR_MODE"),
        (Governor, P::Trash) => ("TRASH", "GOVERNOR_MAY_TRASH"),
        (Governor, P::Gain) => {
            return Some(cost_limit(view, d).map(|a| ("GAIN", "GOVERNOR_GAIN", vec![a])));
        }
        (Marchland, P::Discard) => ("DISCARD", "MARCHLAND"),
        (Prince, P::SetAside) => ("SET_ASIDE", "PRINCE_MAY_SET_ASIDE"),
        (Sauna, P::Play) => ("PLAY", "SAUNA_CHAIN"),
        (Sauna, P::Trash) => ("TRASH", "SAUNA_MAY_TRASH"),
        (Avanto, P::Play) => ("PLAY", "AVANTO_CHAIN"),
        (Summon, P::Gain) => ("GAIN", "SUMMON"),
        _ => return None,
    };
    Some(Ok((kind, story, vec![])))
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    let choices: &[&str] = match card {
        Card::Governor => &["GOVERNOR_CARDS", "GOVERNOR_TREASURE", "GOVERNOR_REMODEL"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (card, ability) {
        (Captain, 0) => "CAPTAIN",
        (Church, 0) => "CHURCH",
        (Marchland, 0) => "MARCHLAND",
        (Prince, 0) => "PRINCE_PLAY",
        (Sauna, 0) => "SAUNA_TRASH_ABILITY",
        (Summon, 0) => "SUMMON_PLAY",
        (WalledVillage, 0) => "WALLED_VILLAGE_TOPDECK",
        _ => return None,
    })
}

pub(super) fn ability_arguments(view: &View<'_>, t: &Trigger, c: &WireCatalog) -> AbilityArguments {
    if !matches!(
        (t.card, t.ability),
        (Card::Church | Card::Prince | Card::Summon, 0)
    ) {
        return None;
    }
    let data = view
        .lasting()
        .into_iter()
        .find(|l| Some(l.id) == t.lasting)
        .map(|l| l.data);
    Some(match (t.card, data) {
        (Card::Church, Some(data)) => data
            .cards
            .iter()
            .map(|&id| view.card(id))
            .collect::<Option<Vec<_>>>()
            .ok_or(ProjectionError::Inconsistent("Church card not visible"))
            .and_then(|cards| Ok(vec![names_of(c, cards)?])),
        (_, data) => ability_card(t, data.and_then(|d| d.pick).and_then(|id| view.card(id)), c),
    })
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Captain => &[
            ("AbilityNames", "CAPTAIN"),
            ("QuestionIds", "CAPTAIN"),
            ("QuestionTypes", "PLAY"),
        ],
        Church => &[
            ("AbilityNames", "CHURCH"),
            ("QuestionIds", "CHURCH_SET_ASIDE"),
            ("QuestionIds", "CHURCH_TRASH"),
            ("QuestionTypes", "SET_ASIDE"),
            ("QuestionTypes", "TRASH"),
        ],
        Dismantle => &[
            ("QuestionIds", "DISMANTLE_GAIN"),
            ("QuestionIds", "DISMANTLE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Envoy => &[("QuestionIds", "ENVOY"), ("QuestionTypes", "DISCARD")],
        Governor => &[
            ("CardModes", "GOVERNOR_CARDS"),
            ("CardModes", "GOVERNOR_REMODEL"),
            ("CardModes", "GOVERNOR_TREASURE"),
            ("QuestionIds", "GOVERNOR_GAIN"),
            ("QuestionIds", "GOVERNOR_MAY_TRASH"),
            ("QuestionIds", "GOVERNOR_MODE"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Marchland => &[
            ("AbilityNames", "MARCHLAND"),
            ("QuestionIds", "MARCHLAND"),
            ("QuestionTypes", "DISCARD"),
        ],
        Prince => &[
            ("AbilityNames", "PRINCE_PLAY"),
            ("QuestionIds", "PRINCE_MAY_SET_ASIDE"),
            ("QuestionTypes", "SET_ASIDE"),
        ],
        Sauna | Avanto => &[
            ("AbilityNames", "SAUNA_TRASH_ABILITY"),
            ("QuestionIds", "AVANTO_CHAIN"),
            ("QuestionIds", "SAUNA_CHAIN"),
            ("QuestionIds", "SAUNA_MAY_TRASH"),
            ("QuestionTypes", "PLAY"),
            ("QuestionTypes", "TRASH"),
        ],
        Summon => &[
            ("AbilityNames", "SUMMON_PLAY"),
            ("QuestionIds", "SUMMON"),
            ("QuestionTypes", "GAIN"),
        ],
        WalledVillage => &[("AbilityNames", "WALLED_VILLAGE_TOPDECK")],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        Captain => &["pile-selection", "duration-reminder", "duration-ability"],
        Church => &[
            "card-selection",
            "set-aside",
            "duration-reminder",
            "duration-ability",
        ],
        Dismantle => &["card-selection", "pile-selection", "explicit-cost-bound"],
        Envoy => &["looking", "card-selection"],
        Governor => &[
            "semantic-modes",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Marchland => &["card-selection"],
        Prince => &["card-selection", "set-aside", "duration-ability"],
        Sauna | Avanto => &["split-pile", "card-selection", "optional-ability"],
        Summon => &["event", "pile-selection", "set-aside", "duration-ability"],
        WalledVillage => &["optional-ability"],
        _ => return None,
    })
}
