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
    match (t.card, t.ability) {
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

pub(super) fn mode(_card: Card, _mode: u8) -> Option<&'static str> {
    None
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    Some(match (card, ability) {
        (Card::Samurai, 1) => "SAMURAI",
        (Card::Daimyo, 1) => "DAIMYO_REPLAY",
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
];

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    ADMITTED
        .iter()
        .find(|(c, _)| *c == card)
        .map(|(_, features)| *features)
}
